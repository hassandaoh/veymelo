//! Inflate (RFC 1951): deflated data read back, written from the RFC. A Huffman code is read a
//! bit at a time against how many codes each length has; canonical codes of one length are
//! consecutive, so that count and the symbols in code order are all a table needs.
//!
//! Every read is checked: data that ends early, codes that lead nowhere, repeats reaching
//! back before the start or output past a limit are errors, never a crash.

use crate::deflate::{DISTANCES, LENGTHS, ORDER};

/// The longest a code can be.
const MAX_BITS: usize = 15;

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    have: u32,
}

impl Bits<'_> {
    fn need(&mut self, n: u32) -> Result<u32, String> {
        while self.have < n {
            let b = *self.data.get(self.pos).ok_or("the deflated data ends early")?;
            self.pos += 1;
            self.buf |= u64::from(b) << self.have;
            self.have += 8;
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.have -= n;
        Ok(v)
    }

    /// To the next whole byte (the bits held are what is left of the byte being read).
    fn align(&mut self) {
        self.buf = 0;
        self.have = 0;
    }
}

/// A canonical Huffman code: how many codes of each length, and the symbols in code order.
struct Huffman {
    count: [u16; MAX_BITS + 1],
    symbol: Vec<u16>,
}

impl Huffman {
    /// The code with these lengths (0: the symbol is not used). An incomplete code is
    /// allowed (a lone distance code is); one with too many codes is not.
    fn new(lengths: &[u8]) -> Result<Huffman, String> {
        let mut count = [0u16; MAX_BITS + 1];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut left: i32 = 1;
        for &c in &count[1..] {
            left = left * 2 - i32::from(c);
            if left < 0 {
                return Err("a Huffman code with too many codes".into());
            }
        }
        let mut offs = [0u16; MAX_BITS + 2];
        for len in 1..=MAX_BITS {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        count[0] = 0;
        Ok(Huffman { count, symbol })
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=MAX_BITS {
            code |= bits.need(1)? as i32;
            let count = i32::from(self.count[len]);
            if code - first < count {
                return self.symbol.get((index + code - first) as usize).copied().ok_or_else(|| "a bad Huffman code".to_string());
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("a code that is in no table".into())
    }
}

fn fixed() -> (Huffman, Huffman) {
    let mut lit = [8u8; 288];
    lit[144..256].fill(9);
    lit[256..280].fill(7);
    // the fixed codes are complete, so these cannot fail
    let lit = Huffman::new(&lit).unwrap_or(Huffman { count: [0; MAX_BITS + 1], symbol: Vec::new() });
    let dist = Huffman::new(&[5u8; 30]).unwrap_or(Huffman { count: [0; MAX_BITS + 1], symbol: Vec::new() });
    (lit, dist)
}

fn dynamic(bits: &mut Bits) -> Result<(Huffman, Huffman), String> {
    let nlen = bits.need(5)? as usize + 257;
    let ndist = bits.need(5)? as usize + 1;
    let ncode = bits.need(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err("too many codes in a block's header".into());
    }
    let mut cl = [0u8; 19];
    for &s in &ORDER[..ncode] {
        cl[s] = bits.need(3)? as u8;
    }
    let cl = Huffman::new(&cl)?;
    let mut lengths = Vec::with_capacity(nlen + ndist);
    while lengths.len() < nlen + ndist {
        let (len, times) = match cl.decode(bits)? {
            s @ 0..=15 => (s as u8, 1),
            16 => (*lengths.last().ok_or("a repeat with nothing before it")?, 3 + bits.need(2)? as usize),
            17 => (0, 3 + bits.need(3)? as usize),
            _ => (0, 11 + bits.need(7)? as usize),
        };
        if lengths.len() + times > nlen + ndist {
            return Err("code lengths run past the end".into());
        }
        lengths.extend(std::iter::repeat_n(len, times));
    }
    if lengths[256] == 0 {
        return Err("a block with no end code".into());
    }
    Ok((Huffman::new(&lengths[..nlen])?, Huffman::new(&lengths[nlen..])?))
}

/// `data` (raw deflate) inflated.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    inflate_up_to(data, usize::MAX)
}

/// `data` inflated, as long as it comes to no more than `limit` bytes.
pub fn inflate_up_to(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut bits = Bits { data, pos: 0, buf: 0, have: 0 };
    let mut out: Vec<u8> = Vec::new();
    let too_big = || format!("it inflates to more than {limit} bytes");
    loop {
        let last = bits.need(1)? == 1;
        match bits.need(2)? {
            0 => {
                bits.align();
                let at = bits.pos;
                let head = data.get(at..at + 4).ok_or("a stored block's length is cut off")?;
                let (n, nn) = (u16::from_le_bytes([head[0], head[1]]), u16::from_le_bytes([head[2], head[3]]));
                if n != !nn {
                    return Err("a stored block's length does not check".into());
                }
                let body = data.get(at + 4..at + 4 + n as usize).ok_or("a stored block is cut off")?;
                if out.len() + body.len() > limit {
                    return Err(too_big());
                }
                out.extend_from_slice(body);
                bits.pos = at + 4 + n as usize;
            }
            kind @ (1 | 2) => {
                let (lit, dist) = if kind == 1 { fixed() } else { dynamic(&mut bits)? };
                loop {
                    let s = lit.decode(&mut bits)?;
                    if s < 256 {
                        if out.len() >= limit {
                            return Err(too_big());
                        }
                        out.push(s as u8);
                        continue;
                    }
                    if s == 256 {
                        break;
                    }
                    let &(_, extra, base) = LENGTHS.get(usize::from(s - 257)).ok_or("a bad length code")?;
                    let len = usize::from(base) + bits.need(u32::from(extra))? as usize;
                    let d = dist.decode(&mut bits)?;
                    let &(extra, base) = DISTANCES.get(usize::from(d)).ok_or("a bad distance code")?;
                    let back = usize::from(base) + bits.need(u32::from(extra))? as usize;
                    if back > out.len() {
                        return Err("a repeat reaches back before the start".into());
                    }
                    if out.len() + len > limit {
                        return Err(too_big());
                    }
                    let from = out.len() - back;
                    for k in 0..len {
                        out.push(out[from + k]);
                    }
                }
            }
            _ => return Err("a block of an unknown kind".into()),
        }
        if last {
            return Ok(out);
        }
    }
}

/// A zlib stream (RFC 1950) inflated, its header and check sum checked.
pub fn unzlib(data: &[u8]) -> Result<Vec<u8>, String> {
    let (&cmf, &flg) = (data.first().ok_or("an empty zlib stream")?, data.get(1).ok_or("a zlib stream with no header")?);
    if cmf & 0x0F != 8 || (u16::from(cmf) << 8 | u16::from(flg)) % 31 != 0 {
        return Err("not a zlib stream".into());
    }
    if flg & 0x20 != 0 {
        return Err("a zlib stream that needs a dictionary".into());
    }
    let body = data.get(2..data.len().saturating_sub(4)).ok_or("a zlib stream cut off")?;
    let out = inflate(body)?;
    let want = data.get(data.len() - 4..).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    if want != Some(crate::adler32(&out)) {
        return Err("a zlib stream whose check sum does not match".into());
    }
    Ok(out)
}
