//! OpenEXR's Huffman codes for 16-bit values (PIZ chunks, and DWA's DCT coefficients): a table
//! of code lengths (runs of unused values packed short), canonical codes from them, and the
//! highest value used standing for "the value before, this many more times".

/// Code lengths are below this.
const MOST_BITS: usize = 58;
/// Codes this long or shorter are found in one look.
const LOOK: u32 = 14;
const SHORT_ZERO_RUN: u64 = 59;
const LONG_ZERO_RUN: u64 = 63;
const SHORTEST_LONG_RUN: usize = 2 + LONG_ZERO_RUN as usize - SHORT_ZERO_RUN as usize;

struct Bits<'a> {
    b: &'a [u8],
    at: usize,
    c: u64,
    lc: u32,
}

impl Bits<'_> {
    fn byte(&mut self) -> Result<(), String> {
        let v = *self.b.get(self.at).ok_or("Huffman codes cut short")?;
        self.at += 1;
        self.c = self.c << 8 | v as u64;
        self.lc += 8;
        Ok(())
    }

    fn get(&mut self, n: u32) -> Result<u64, String> {
        while self.lc < n {
            self.byte()?;
        }
        self.lc -= n;
        Ok(self.c >> self.lc & ((1 << n) - 1))
    }
}

fn le_u32(b: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(b.get(at..at + 4).ok_or("Huffman codes cut short")?.try_into().expect("four bytes")))
}

/// What one look at the next [`LOOK`] bits finds: a short code's value and length, or the
/// long codes starting with them.
#[derive(Clone, Default)]
struct Entry {
    len: u32,
    value: u32,
    long: Vec<u32>,
}

/// `n` values unpacked from `data`, OpenEXR's Huffman-coded block.
pub(crate) fn decode(data: &[u8], n: usize) -> Result<Vec<u16>, String> {
    if data.is_empty() {
        return if n == 0 { Ok(Vec::new()) } else { Err("Huffman codes missing".into()) };
    }
    let (lo, hi) = (le_u32(data, 0)? as usize, le_u32(data, 4)? as usize);
    let bits = le_u32(data, 12)? as usize;
    if lo > 0x10000 || hi > 0x10000 || lo > hi {
        return Err("a Huffman table of values out of range".into());
    }
    // the code lengths of values lo..=hi
    let mut codes = vec![0u64; 0x10001];
    let mut r = Bits { b: data, at: 20, c: 0, lc: 0 };
    let mut i = lo;
    while i <= hi {
        let l = r.get(6)?;
        if l == LONG_ZERO_RUN {
            let run = r.get(8)? as usize + SHORTEST_LONG_RUN;
            if i + run > hi + 1 {
                return Err("a Huffman table too long".into());
            }
            i += run;
        } else if l >= SHORT_ZERO_RUN {
            let run = (l - SHORT_ZERO_RUN) as usize + 2;
            if i + run > hi + 1 {
                return Err("a Huffman table too long".into());
            }
            i += run;
        } else {
            codes[i] = l;
            i += 1;
        }
    }
    // canonical codes: the longest first, counting up within each length
    let mut count = [0u64; MOST_BITS + 1];
    for &l in &codes {
        count[l as usize] += 1;
    }
    let mut c = 0u64;
    for l in (1..=MOST_BITS).rev() {
        let next = (c + count[l]) >> 1;
        count[l] = c;
        c = next;
    }
    for code in codes.iter_mut() {
        let l = *code;
        if l > 0 {
            *code = l | count[l as usize] << 6;
            count[l as usize] += 1;
        }
    }
    // the table for one look
    let mut table = vec![Entry::default(); 1 << LOOK];
    for (v, &code) in codes.iter().enumerate().take(hi + 1).skip(lo) {
        let (c, l) = (code >> 6, (code & 63) as u32);
        if l == 0 {
            continue;
        }
        if c >> l != 0 {
            return Err("a Huffman table with a code too big for its length".into());
        }
        if l > LOOK {
            let e = &mut table[(c >> (l - LOOK)) as usize];
            if e.len != 0 {
                return Err("a Huffman table with codes that clash".into());
            }
            e.long.push(v as u32);
        } else {
            let first = (c << (LOOK - l)) as usize;
            for e in &mut table[first..first + (1 << (LOOK - l))] {
                if e.len != 0 || !e.long.is_empty() {
                    return Err("a Huffman table with codes that clash".into());
                }
                e.len = l;
                e.value = v as u32;
            }
        }
    }
    // the codes
    let start = r.at;
    let end = start + bits.div_ceil(8);
    if end > data.len() {
        return Err("Huffman codes cut short".into());
    }
    let mut out: Vec<u16> = Vec::with_capacity(n);
    let rle = hi as u32;
    let mut b = Bits { b: &data[..end], at: start, c: 0, lc: 0 };
    let put = |v: u32, b: &mut Bits, out: &mut Vec<u16>| -> Result<(), String> {
        if v == rle {
            if b.lc < 8 {
                b.byte()?;
            }
            b.lc -= 8;
            let times = (b.c >> b.lc) as u8 as usize;
            let last = *out.last().ok_or("a run of the value before the first")?;
            if out.len() + times > n {
                return Err("Huffman codes for more values than there are".into());
            }
            out.extend(std::iter::repeat_n(last, times));
        } else {
            if out.len() >= n {
                return Err("Huffman codes for more values than there are".into());
            }
            out.push(v as u16);
        }
        Ok(())
    };
    while b.at < end {
        b.byte()?;
        while b.lc >= LOOK {
            let e = &table[(b.c >> (b.lc - LOOK)) as usize & ((1 << LOOK) - 1)];
            if e.len > 0 {
                b.lc -= e.len;
                put(e.value, &mut b, &mut out)?;
                continue;
            }
            if e.long.is_empty() {
                return Err("a Huffman code that is not in its table".into());
            }
            let mut found = false;
            for &v in &e.long {
                let code = codes[v as usize];
                let l = (code & 63) as u32;
                while b.lc < l && b.at < end {
                    b.byte()?;
                }
                if b.lc >= l && code >> 6 == (b.c >> (b.lc - l)) & ((1 << l) - 1) {
                    b.lc -= l;
                    put(v, &mut b, &mut out)?;
                    found = true;
                    break;
                }
            }
            if !found {
                return Err("a Huffman code that is not in its table".into());
            }
        }
    }
    // the last few codes, shorter than a look
    let spare = ((8 - bits as i64).rem_euclid(8)) as u32;
    b.c >>= spare;
    b.lc = b.lc.checked_sub(spare).ok_or("Huffman codes cut short")?;
    while b.lc > 0 {
        let e = &table[(b.c << (LOOK - b.lc)) as usize & ((1 << LOOK) - 1)];
        if e.len == 0 || e.len > b.lc {
            return Err("a Huffman code that is not in its table".into());
        }
        b.lc -= e.len;
        put(e.value, &mut b, &mut out)?;
    }
    if out.len() != n {
        return Err(format!("Huffman codes for {} values, not {n}", out.len()));
    }
    Ok(out)
}
