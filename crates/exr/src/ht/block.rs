//! An HT code-block's cleanup pass decoded (ITU-T T.814): three streams in its bytes — the MEL
//! stream (run lengths of quads with nothing in them) read forward from where the MEL and VLC
//! part starts, the VLC stream (each quad's significance and exponent offsets, by context) read
//! backward from the end, and the MagSgn stream (each significant sample's magnitude bits and
//! sign) read forward from the start — each with its own rule for the bits stuffed after a byte
//! of all ones. Samples come out sign and magnitude, the magnitude's top at bit 62 of a word (the
//! sign at 63), as the bit-planes the pass codes put them.

use super::tables::{FIRST, LATER, Row};

/// The MEL coder's exponents, by state.
const EXP: [u32; 13] = [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 4, 5];

/// The VLC table of a set of rows as lookups: context (bits 7–9) and the next seven bits.
/// Each entry: codeword length (bits 0–2), `u_off` (3), `rho` (4–7), `e_1` (8–11), `e_k` (12–15).
fn lookup(rows: &[Row]) -> Vec<u16> {
    let mut t = vec![0u16; 1024];
    for (i, slot) in t.iter_mut().enumerate() {
        let (cwd, cq) = (i & 0x7F, i >> 7);
        if let Some(r) = rows.iter().find(|r| r[0] as usize == cq && r[5] as usize == cwd & ((1 << r[6]) - 1)) {
            *slot = (r[6] as u16) | (r[2] as u16) << 3 | (r[1] as u16) << 4 | (r[4] as u16) << 8 | (r[3] as u16) << 12;
        }
    }
    t
}

fn tables() -> &'static (Vec<u16>, Vec<u16>) {
    static T: std::sync::OnceLock<(Vec<u16>, Vec<u16>)> = std::sync::OnceLock::new();
    T.get_or_init(|| (lookup(&FIRST), lookup(&LATER)))
}

/// The MEL stream: bits from the top of each byte, a byte after one of all ones giving seven;
/// its last byte's low four bits taken as ones, and ones past its end.
struct Mel<'a> {
    d: &'a [u8],
    pos: usize,
    end: usize,
    byte: u8,
    left: u32,
    after_ff: bool,
    k: usize,
}

impl Mel<'_> {
    fn bit(&mut self) -> u32 {
        if self.left == 0 {
            let mut b = if self.pos < self.end { self.d[self.pos] } else { 0xFF };
            if self.pos + 1 == self.end {
                b |= 0x0F;
            }
            self.pos += 1;
            self.left = if self.after_ff { 7 } else { 8 };
            self.after_ff = b == 0xFF;
            self.byte = b;
        }
        self.left -= 1;
        (self.byte >> self.left) as u32 & 1
    }

    /// The next run: twice the zero events in it, and one more when it ends with an event of one.
    fn run(&mut self) -> i32 {
        let e = EXP[self.k];
        if self.bit() == 1 {
            self.k = (self.k + 1).min(12);
            (((1u32 << e) - 1) << 1) as i32
        } else {
            let mut v = 0u32;
            for _ in 0..e {
                v = v << 1 | self.bit();
            }
            self.k = self.k.saturating_sub(1);
            (v << 1 | 1) as i32
        }
    }
}

/// Bits taken from the bottom of each byte, gathered in a word.
struct Bits {
    acc: u64,
    n: u32,
}

/// The VLC stream: read backward from the byte before the last, its top four bits first.
struct Vlc<'a> {
    d: &'a [u8],
    /// The next byte to read, and how many are left.
    pos: usize,
    left: usize,
    after: bool,
    bits: Bits,
}

impl Vlc<'_> {
    fn new(d: &[u8], lcup: usize, scup: usize) -> Vlc<'_> {
        let first = d[lcup - 2];
        let top = (first >> 4) as u64;
        let n = if top & 7 == 7 { 3 } else { 4 };
        Vlc { d, pos: lcup - 3, left: scup - 2, after: first > 0x8F, bits: Bits { acc: top, n } }
    }

    fn fill(&mut self) {
        while self.bits.n <= 56 {
            let b = if self.left > 0 {
                let b = self.d[self.pos];
                self.left -= 1;
                self.pos = self.pos.wrapping_sub(1);
                b
            } else {
                0
            };
            let take = if self.after && b & 0x7F == 0x7F { 7 } else { 8 };
            self.bits.acc |= ((b as u64) & ((1 << take) - 1)) << self.bits.n;
            self.bits.n += take;
            self.after = b > 0x8F;
        }
    }

    fn peek(&mut self) -> u64 {
        if self.bits.n < 32 {
            self.fill();
        }
        self.bits.acc
    }

    fn advance(&mut self, n: u32) {
        self.peek();
        self.bits.acc >>= n;
        self.bits.n -= n;
    }

    fn read(&mut self, n: u32) -> u32 {
        let v = (self.peek() & ((1u64 << n) - 1)) as u32;
        self.advance(n);
        v
    }
}

/// A stream read forward (MagSgn): a byte after one of all ones gives seven bits; past its end,
/// `fill` bytes.
pub(crate) struct Forward<'a> {
    d: &'a [u8],
    pos: usize,
    end: usize,
    fill: u8,
    after_ff: bool,
    bits: Bits,
}

impl Forward<'_> {
    pub(crate) fn new(d: &[u8], start: usize, end: usize, fill: u8) -> Forward<'_> {
        Forward { d, pos: start, end, fill, after_ff: false, bits: Bits { acc: 0, n: 0 } }
    }

    fn top_up(&mut self) {
        while self.bits.n <= 56 {
            let b = if self.pos < self.end { self.d[self.pos] } else { self.fill };
            self.pos += 1;
            let take = if self.after_ff { 7 } else { 8 };
            self.bits.acc |= ((b as u64) & ((1 << take) - 1)) << self.bits.n;
            self.bits.n += take;
            self.after_ff = b == 0xFF;
        }
    }

    /// The next `n` bits (up to 56), taken.
    pub(crate) fn read(&mut self, n: u32) -> u64 {
        if n == 0 {
            return 0;
        }
        if self.bits.n < n {
            self.top_up();
        }
        let v = self.bits.acc & ((1u64 << n) - 1);
        self.bits.acc >>= n;
        self.bits.n -= n;
        v
    }
}

/// The next MEL event (true: one), from the run being read (twice its zeros left, and one when
/// it ends with a one), a new run read when it is done.
fn event(run: &mut i32, mel: &mut Mel) -> bool {
    *run -= 2;
    let one = *run == -1;
    if *run < 0 {
        *run = mel.run();
    }
    one
}

/// A U-VLC prefix from the next bits: its value and how many suffix bits follow.
fn prefix(v: &mut Vlc) -> (u32, u32) {
    let b = v.peek();
    if b & 1 == 1 {
        v.advance(1);
        (1, 0)
    } else if b & 2 == 2 {
        v.advance(2);
        (2, 0)
    } else if b & 4 == 4 {
        v.advance(3);
        (3, 1)
    } else {
        v.advance(3);
        (5, 5)
    }
}

/// A quad's `u`: its prefix value and suffix, and the extension that follows the largest
/// suffixes (read after both quads' suffixes).
fn extend(v: &mut Vlc, u: &mut [u32; 2], suffix: [Option<u32>; 2]) {
    for k in 0..2 {
        if suffix[k].is_some_and(|s| s >= 28) {
            u[k] += 4 * v.read(4);
        }
    }
}

/// The cleanup pass of a code-block `w` × `h`: its coded bytes (the cleanup segment is the
/// first `lcup`), the number of most significant bit-planes missing; each sample, row by row,
/// as sign (bit 63) and magnitude.
pub(crate) fn cleanup(d: &[u8], lcup: usize, missing: u32, w: usize, h: usize) -> Result<Vec<u64>, String> {
    if lcup < 2 || lcup > d.len() {
        return Err("an HT code-block too short".into());
    }
    let scup = ((d[lcup - 1] as usize) << 4) + (d[lcup - 2] & 0xF) as usize;
    if scup < 2 || scup > lcup || scup > 4079 {
        return Err("an HT code-block whose MEL and VLC length is wrong".into());
    }
    if missing >= 60 {
        return Err("an HT code-block missing too many bit-planes".into());
    }
    let p = 62 - missing;
    let (vlc0, vlc1) = tables();
    let mut mel = Mel { d, pos: lcup - scup, end: lcup - 1, byte: 0, left: 0, after_ff: false, k: 0 };
    let mut vlc = Vlc::new(d, lcup, scup);
    let mut ms = Forward::new(d, 0, lcup - scup, 0xFF);
    let (qw, qh) = (w.div_ceil(2), h.div_ceil(2));
    // each quad's VLC entry and u, row by row (a quad more each side for its neighbours)
    let stride = qw + 3;
    let mut inf = vec![0u16; stride * qh];
    let mut us = vec![0u32; stride * qh];
    let mut run = mel.run();
    for r in 0..qh {
        let row = r * stride;
        let above = if r == 0 { None } else { Some((r - 1) * stride) };
        let table = if r == 0 { vlc0 } else { vlc1 };
        let mut cq: usize = 0;
        let mut q = 0;
        while q < qw {
            if let Some(a) = above {
                cq |= ((inf[a + q] & 0xA0) << 2) as usize | ((inf[a + q + 1] & 0x20) << 4) as usize;
            }
            let mut t0 = table[cq | (vlc.peek() & 0x7F) as usize];
            if cq == 0 && !event(&mut run, &mut mel) {
                t0 = 0;
            }
            inf[row + q] = t0;
            vlc.advance((t0 & 7) as u32);
            cq = match above {
                None => (((t0 & 0x10) << 3) | ((t0 & 0xE0) << 2)) as usize,
                Some(a) => (((t0 & 0x40) << 2) | ((t0 & 0x80) << 1) | (inf[a + q] & 0x80) | ((inf[a + q + 1] & 0xA0) << 2) | ((inf[a + q + 2] & 0x20) << 4)) as usize,
            };
            let mut t1 = 0;
            if q + 1 < qw {
                t1 = table[cq | (vlc.peek() & 0x7F) as usize];
                if cq == 0 && !event(&mut run, &mut mel) {
                    t1 = 0;
                }
                inf[row + q + 1] = t1;
                vlc.advance((t1 & 7) as u32);
            }
            cq = match above {
                None => (((t1 & 0x10) << 3) | ((t1 & 0xE0) << 2)) as usize,
                Some(a) => (((t1 & 0x40) << 2) | ((t1 & 0x80) << 1) | (inf[a + q + 1] & 0x80)) as usize,
            };
            // the pair's exponent offsets
            let (off0, off1) = (t0 & 8 != 0, t1 & 8 != 0);
            let mut u = [0u32; 2];
            let mut suffix = [None, None];
            if off0 && off1 {
                if r == 0 && event(&mut run, &mut mel) {
                    let ((p0, s0), (p1, s1)) = (prefix(&mut vlc), prefix(&mut vlc));
                    let (x0, x1) = (vlc.read(s0), vlc.read(s1));
                    u = [2 + p0 + x0, 2 + p1 + x1];
                    suffix = [(s0 == 5).then_some(x0), (s1 == 5).then_some(x1)];
                } else {
                    let (p0, s0) = prefix(&mut vlc);
                    if r == 0 && p0 > 2 {
                        let one = vlc.read(1);
                        let x0 = vlc.read(s0);
                        u = [p0 + x0, 1 + one];
                        suffix = [(s0 == 5).then_some(x0), None];
                    } else {
                        let (p1, s1) = prefix(&mut vlc);
                        let (x0, x1) = (vlc.read(s0), vlc.read(s1));
                        u = [p0 + x0, p1 + x1];
                        suffix = [(s0 == 5).then_some(x0), (s1 == 5).then_some(x1)];
                    }
                }
            } else if off0 || off1 {
                let k = if off0 { 0 } else { 1 };
                let (pf, s) = prefix(&mut vlc);
                let x = vlc.read(s);
                u[k] = pf + x;
                suffix[k] = (s == 5).then_some(x);
            }
            extend(&mut vlc, &mut u, suffix);
            // the first row's exponent bound is one more than u
            let base = if r == 0 { 1 } else { 0 };
            us[row + q] = u[0] + base;
            if q + 1 < qw {
                us[row + q + 1] = u[1] + base;
            }
            q += 2;
        }
    }
    // the magnitudes and signs, row by row; each quad's exponent bound from the row above
    let mut out = vec![0u64; w * h];
    let mut vp = vec![0u32; qw + 2];
    let mut next = vec![0u32; qw + 2];
    for r in 0..qh {
        let row = r * stride;
        let mut carry = 0u32;
        next.iter_mut().for_each(|v| *v = 0);
        for q in 0..qw {
            let t = inf[row + q];
            let rho = (t >> 4) & 0xF;
            let kappa = if r == 0 {
                0
            } else if rho.count_ones() >= 2 {
                31 - (vp[q] | vp[q + 1] | 2).leading_zeros()
            } else {
                1
            };
            let bound = us[row + q] + kappa;
            if rho != 0 && bound > missing + 2 {
                return Err("an HT code-block with more bit-planes than it says".into());
            }
            let mut v = [0u32; 4];
            for (b, vb) in v.iter_mut().enumerate() {
                let (x, y) = (2 * q + (b >> 1), 2 * r + (b & 1));
                if rho >> b & 1 == 0 {
                    continue;
                }
                let ek = (t >> (12 + b)) as u32 & 1;
                let e1 = (t >> (8 + b)) as u64 & 1;
                let m = bound - ek;
                let bits = ms.read(m);
                let sign = bits & 1;
                let mut val = bits | (e1 << m) | 1;
                *vb = val.min(u32::MAX as u64) as u32;
                val = (sign << 63) | ((val + 2) << (p - 1));
                if x < w && y < h {
                    out[y * w + x] = val;
                }
            }
            next[q] = carry | v[1];
            carry = v[3];
        }
        next[qw] = carry;
        std::mem::swap(&mut vp, &mut next);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_context_is_a_whole_prefix_code() {
        for rows in [&FIRST[..], &LATER[..]] {
            for cq in 0..8 {
                let mine: Vec<&Row> = rows.iter().filter(|r| r[0] == cq).collect();
                // the lengths fill the code exactly, and no codeword begins another
                let kraft: f64 = mine.iter().map(|r| 0.5f64.powi(r[6] as i32)).sum();
                assert!((kraft - 1.0).abs() < 1e-12, "context {cq}: {kraft}");
                for (i, a) in mine.iter().enumerate() {
                    for b in &mine[i + 1..] {
                        let n = a[6].min(b[6]);
                        assert_ne!(a[5] & ((1 << n) - 1), b[5] & ((1 << n) - 1), "{a:?} {b:?}");
                    }
                }
            }
        }
        // every lookup finds a codeword
        let (a, b) = tables();
        assert!(a.iter().chain(b.iter()).all(|e| e & 7 != 0));
    }
}
