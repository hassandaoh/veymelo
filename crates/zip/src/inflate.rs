//! Inflate (RFC 1951): deflated data read back. The codes are read from lookup tables: the
//! next bits, as many as the shorter codes take, name either a symbol (with how long its code
//! is) or a second table for the codes that are longer, which the bits after them index.
//!
//! Every read is checked: data that ends early, codes that lead nowhere, repeats reaching
//! back before the start or output past a limit are errors, never a crash.

use crate::deflate::{DISTANCES, LENGTHS, ORDER};

/// The longest a code can be.
const MAX_BITS: usize = 15;
/// How many bits the first table is indexed by (shorter for a code whose codes are all shorter).
const FIRST: u32 = 10;

/// The bits of the data, first bit first: a word of them kept ahead of where reading is.
struct Bits<'a> {
    data: &'a [u8],
    /// The next byte not yet in the word.
    pos: usize,
    word: u64,
    have: u32,
}

impl Bits<'_> {
    /// Bytes into the word while there is room for a whole one.
    fn fill(&mut self) {
        while self.have <= 56 {
            let Some(&b) = self.data.get(self.pos) else { break };
            self.word |= u64::from(b) << self.have;
            self.pos += 1;
            self.have += 8;
        }
    }

    /// The next `n` bits (up to 32) without taking them; past the end they read as zeros.
    fn peek(&mut self, n: u32) -> u32 {
        if self.have < n {
            self.fill();
        }
        (self.word & ((1u64 << n) - 1)) as u32
    }

    /// Take `n` bits that have been looked at.
    fn take(&mut self, n: u32) -> Result<(), String> {
        if n > self.have {
            return Err("the deflated data ends early".into());
        }
        self.word >>= n;
        self.have -= n;
        Ok(())
    }

    /// The next `n` bits (up to 32), taken.
    fn need(&mut self, n: u32) -> Result<u32, String> {
        let v = self.peek(n);
        self.take(n)?;
        Ok(v)
    }

    /// To the next whole byte: what is left of the byte being read is dropped, and the whole
    /// bytes still in the word are given back to be read as they are.
    fn align(&mut self) {
        let rest = self.have % 8;
        self.word >>= rest;
        self.have -= rest;
        self.pos -= (self.have / 8) as usize;
        self.word = 0;
        self.have = 0;
    }
}

/// A table entry: a symbol and how many bits its code takes from here, or (`LINK` set) where a
/// second table starts and how many bits index it. Zero: no code begins with these bits.
const LINK: u32 = 1 << 5;

fn leaf(symbol: u16, bits: u32) -> u32 {
    u32::from(symbol) << 16 | bits
}

/// A Huffman code made into tables for reading.
struct Huffman {
    /// The first table, then the second ones.
    table: Vec<u32>,
    first: u32,
}

impl Huffman {
    /// The code with these lengths (0: the symbol is not used), its codes given as RFC 1951 says
    /// (shorter codes first, each length's in the order of their symbols). An incomplete code
    /// is allowed (a lone distance code is); one with too many codes is not.
    fn new(lengths: &[u8]) -> Result<Huffman, String> {
        let mut count = [0u32; MAX_BITS + 1];
        for &l in lengths {
            count[l as usize] += 1;
        }
        count[0] = 0;
        let mut room: i64 = 1;
        for &c in &count[1..] {
            room = room * 2 - i64::from(c);
            if room < 0 {
                return Err("a Huffman code with too many codes".into());
            }
        }
        let mut next = [0u32; MAX_BITS + 2];
        for len in 1..=MAX_BITS {
            next[len + 1] = (next[len] + count[len]) << 1;
        }
        // each used symbol's code, its bits turned round to the order they arrive in
        let mut codes: Vec<(u32, u32, u16)> = vec![];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                let l = u32::from(l);
                let code = next[l as usize];
                next[l as usize] += 1;
                codes.push((code.reverse_bits() >> (32 - l), l, s as u16));
            }
        }
        let longest = codes.iter().map(|c| c.1).max().unwrap_or(1);
        let first = longest.min(FIRST);
        let mut table = vec![0u32; 1 << first];
        // the short codes: every index whose low bits are the code
        for &(code, len, s) in codes.iter().filter(|c| c.1 <= first) {
            let mut i = code as usize;
            while i < table.len() {
                table[i] = leaf(s, len);
                i += 1 << len;
            }
        }
        // the long ones: a second table for each first-table index they begin with, as wide as
        // the longest of them needs
        let mut long: Vec<_> = codes.iter().filter(|c| c.1 > first).copied().collect();
        long.sort_by_key(|c| c.0 & ((1 << first) - 1));
        let mut k = 0;
        while k < long.len() {
            let head = long[k].0 & ((1 << first) - 1);
            let end = k + long[k..].iter().take_while(|c| c.0 & ((1 << first) - 1) == head).count();
            let wide = long[k..end].iter().map(|c| c.1 - first).max().unwrap_or(1);
            let at = table.len();
            table.resize(at + (1 << wide), 0);
            table[head as usize] = (at as u32) << 16 | LINK | wide;
            for &(code, len, s) in &long[k..end] {
                let rest = len - first;
                let mut i = (code >> first) as usize;
                while i < 1 << wide {
                    table[at + i] = leaf(s, rest);
                    i += 1 << rest;
                }
            }
            k = end;
        }
        Ok(Huffman { table, first })
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, String> {
        let e = self.table[bits.peek(self.first) as usize];
        if e & LINK == 0 {
            if e == 0 {
                return Err("a code that is in no table".into());
            }
            bits.take(e & 0x1F)?;
            return Ok((e >> 16) as u16);
        }
        bits.take(self.first)?;
        let wide = e & 0x1F;
        let e = self.table[(e >> 16) as usize + bits.peek(wide) as usize];
        if e == 0 {
            return Err("a code that is in no table".into());
        }
        bits.take(e & 0x1F)?;
        Ok((e >> 16) as u16)
    }
}

fn fixed() -> (Huffman, Huffman) {
    let mut lit = [8u8; 288];
    lit[144..256].fill(9);
    lit[256..280].fill(7);
    // the fixed codes are complete, so these cannot fail
    let lit = Huffman::new(&lit).unwrap_or(Huffman { table: vec![0], first: 0 });
    let dist = Huffman::new(&[5u8; 30]).unwrap_or(Huffman { table: vec![0], first: 0 });
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
    let mut bits = Bits { data, pos: 0, word: 0, have: 0 };
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
                    if back >= len && len >= 32 {
                        out.extend_from_within(from..from + len);
                    } else {
                        // short, or reaching into what it is making: a byte at a time
                        for k in 0..len {
                            out.push(out[from + k]);
                        }
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
