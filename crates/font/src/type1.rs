//! Type 1 fonts (PostScript's own, as PDF files and `.pfa`/`.pfb` files hold them): the clear part
//! read for the font's matrix and encoding, the rest unlocked (`eexec`) for its subroutines and
//! charstrings, each unlocked again, and a glyph's outline from running its charstring (moves,
//! lines and curves, flex drawn as its two curves, subroutines, accented characters made of two).

use std::collections::HashMap;

use crate::Pen;
use crate::names::{Encoding, encoded};

/// A Type 1 font, read.
pub(crate) struct Type1 {
    pub matrix: [f64; 6],
    /// Each code's glyph name (none: the font uses Standard's).
    pub encoding: Option<Vec<Option<String>>>,
    pub names: Vec<String>,
    pub by_name: HashMap<String, usize>,
    charstrings: Vec<Vec<u8>>,
    subrs: Vec<Vec<u8>>,
}

/// Unlock `data` (key `r`), leaving out the first `skip` bytes.
fn decrypt(data: &[u8], mut r: u16, skip: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for &c in data {
        out.push(c ^ (r >> 8) as u8);
        r = (c as u16).wrapping_add(r).wrapping_mul(52845).wrapping_add(22719);
    }
    out.split_off(skip.min(out.len()))
}

fn white(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | 0)
}

/// Where `what` is in `d` from `from`.
fn find(d: &[u8], what: &[u8], from: usize) -> Option<usize> {
    d.get(from..)?.windows(what.len()).position(|w| w == what).map(|p| p + from)
}

/// Tokens of PostScript text read one at a time (names with their slash, numbers, words; strings
/// and procedures left as their brackets).
struct Tokens<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Tokens<'a> {
    fn next(&mut self) -> Option<&'a [u8]> {
        let d = self.d;
        while self.p < d.len() && white(d[self.p]) {
            self.p += 1;
        }
        if self.p < d.len() && d[self.p] == b'%' {
            while self.p < d.len() && d[self.p] != b'\n' && d[self.p] != b'\r' {
                self.p += 1;
            }
            return self.next();
        }
        let start = self.p;
        if start >= d.len() {
            return None;
        }
        if matches!(d[start], b'[' | b']' | b'{' | b'}' | b'(' | b')' | b'<' | b'>') {
            self.p += 1;
            return Some(&d[start..start + 1]);
        }
        self.p += 1;
        while self.p < d.len() && !white(d[self.p]) && !matches!(d[self.p], b'[' | b']' | b'{' | b'}' | b'(' | b')' | b'<' | b'>' | b'/' | b'%') {
            self.p += 1;
        }
        Some(&d[start..self.p])
    }

    fn number(&mut self) -> Option<f64> {
        std::str::from_utf8(self.next()?).ok()?.parse().ok()
    }

    /// `len` bytes of binary data after the token that reads them (one space after it).
    fn binary(&mut self, len: usize) -> Option<&'a [u8]> {
        self.next()?;
        let start = self.p + 1;
        let out = self.d.get(start..start + len)?;
        self.p = start + len;
        Some(out)
    }
}

impl Type1 {
    /// A Type 1 font from its file's bytes (`.pfb` segments, or the clear text then the locked
    /// part, binary or in hex).
    pub fn read(bytes: &[u8]) -> Option<Type1> {
        // a .pfb: segments of text (1) and binary (2)
        let joined;
        let bytes = if bytes.first() == Some(&0x80) {
            let mut out = Vec::new();
            let mut p = 0;
            while p + 6 <= bytes.len() && bytes[p] == 0x80 && bytes[p + 1] != 3 {
                let len = u32::from_le_bytes([bytes[p + 2], bytes[p + 3], bytes[p + 4], bytes[p + 5]]) as usize;
                out.extend(bytes.get(p + 6..p + 6 + len)?);
                p += 6 + len;
            }
            joined = out;
            &joined[..]
        } else {
            bytes
        };
        let at = find(bytes, b"eexec", 0)?;
        let clear = &bytes[..at];
        let mut start = at + 5;
        while start < bytes.len() && white(bytes[start]) {
            start += 1;
        }
        let locked = &bytes[start..];
        // in hex when its first four are hex digits
        let raw: Vec<u8> = if locked.len() >= 4 && locked[..4].iter().all(u8::is_ascii_hexdigit) {
            let digits: Vec<u8> = locked.iter().copied().filter(u8::is_ascii_hexdigit).collect();
            digits.chunks(2).map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap_or("0"), 16).unwrap_or(0)).collect()
        } else {
            locked.to_vec()
        };
        let private = decrypt(&raw, 55665, 4);
        let mut font = Type1 { matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0], encoding: None, names: Vec::new(), by_name: HashMap::new(), charstrings: Vec::new(), subrs: Vec::new() };
        // the clear part: the matrix and the encoding
        if let Some(m) = find(clear, b"/FontMatrix", 0) {
            let mut t = Tokens { d: clear, p: m + 11 };
            if t.next() == Some(b"[") {
                let v: Vec<f64> = (0..6).filter_map(|_| t.number()).collect();
                if let [a, b, c, d, e, f] = v[..] {
                    font.matrix = [a, b, c, d, e, f];
                }
            }
        }
        if let Some(e) = find(clear, b"/Encoding", 0) {
            let mut t = Tokens { d: clear, p: e + 9 };
            if t.next() != Some(b"StandardEncoding") {
                let mut codes: Vec<Option<String>> = vec![None; 256];
                // dup code /name put, until the encoding is defined
                let mut last: [&[u8]; 3] = [b"", b"", b""];
                while let Some(tok) = t.next() {
                    if tok == b"def" || tok == b"readonly" {
                        break;
                    }
                    if tok == b"put" && last[0] == b"dup"
                        && let Some(code) = std::str::from_utf8(last[1]).ok().and_then(|s| s.parse::<usize>().ok())
                        && let Some(name) = last[2].strip_prefix(b"/")
                        && code < 256
                    {
                        codes[code] = Some(String::from_utf8_lossy(name).into_owned());
                    }
                    last = [last[1], last[2], tok];
                }
                font.encoding = Some(codes);
            }
        }
        // the locked part: subroutines and charstrings, each locked again
        let len_iv = find(&private, b"/lenIV", 0).and_then(|p| Tokens { d: &private, p: p + 6 }.number()).unwrap_or(4.0) as i64;
        let unlock = |s: &[u8]| if len_iv < 0 { s.to_vec() } else { decrypt(s, 4330, len_iv as usize) };
        if let Some(p) = find(&private, b"/Subrs", 0) {
            let mut t = Tokens { d: &private, p: p + 6 };
            let n = t.number().unwrap_or(0.0) as usize;
            font.subrs = vec![Vec::new(); n.min(65536)];
            while let Some(tok) = t.next() {
                if tok != b"dup" {
                    if tok == b"index" || tok == b"/CharStrings" || tok == b"end" {
                        break;
                    }
                    continue;
                }
                let (Some(i), Some(len)) = (t.number(), t.number()) else { break };
                let Some(s) = t.binary(len as usize) else { break };
                if let Some(slot) = font.subrs.get_mut(i as usize) {
                    *slot = unlock(s);
                }
            }
        }
        let p = find(&private, b"/CharStrings", 0)?;
        let mut t = Tokens { d: &private, p: p + 12 };
        while let Some(tok) = t.next() {
            if tok == b"end" && !font.names.is_empty() {
                break;
            }
            let Some(name) = tok.strip_prefix(b"/") else { continue };
            let save = t.p;
            let Some(len) = t.number() else {
                t.p = save;
                continue;
            };
            let Some(s) = t.binary(len as usize) else { break };
            let name = String::from_utf8_lossy(name).into_owned();
            font.by_name.insert(name.clone(), font.names.len());
            font.names.push(name);
            font.charstrings.push(unlock(s));
        }
        (!font.charstrings.is_empty()).then_some(font)
    }

    /// Glyph `g`'s outline into `pen` through `place`, and how far it moves the pen.
    pub fn outline(&self, g: usize, pen: &mut dyn Pen, place: &dyn Fn(f64, f64) -> (f64, f64)) -> Option<f64> {
        let cs = self.charstrings.get(g)?;
        let mut run = Run { font: self, pen, place, stack: Vec::new(), ps: Vec::new(), x: 0.0, y: 0.0, at: (0.0, 0.0), sbx: 0.0, width: 0.0, open: false, flex: None, done: false, depth: 0 };
        run.exec(cs);
        if run.open {
            run.pen.close();
        }
        Some(run.width)
    }
}

struct Run<'a, 'p> {
    font: &'a Type1,
    pen: &'p mut dyn Pen,
    place: &'p dyn Fn(f64, f64) -> (f64, f64),
    stack: Vec<f64>,
    /// What the PostScript procedures (`callothersubr`) leave for `pop`.
    ps: Vec<f64>,
    x: f64,
    y: f64,
    /// Where drawing is offset (an accent's place).
    at: (f64, f64),
    sbx: f64,
    width: f64,
    open: bool,
    /// Flex's points, collected while it is on.
    flex: Option<Vec<(f64, f64)>>,
    done: bool,
    depth: u32,
}

impl Run<'_, '_> {
    fn point(&self, x: f64, y: f64) -> (f64, f64) {
        (self.place)(x + self.at.0, y + self.at.1)
    }

    fn move_by(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        if let Some(f) = &mut self.flex {
            f.push((self.x, self.y));
            return;
        }
        if self.open {
            self.pen.close();
            self.open = false;
        }
    }

    fn begin(&mut self) {
        if !self.open {
            let (px, py) = self.point(self.x, self.y);
            self.pen.move_to(px, py);
            self.open = true;
        }
    }

    fn line_by(&mut self, dx: f64, dy: f64) {
        self.begin();
        self.x += dx;
        self.y += dy;
        let (px, py) = self.point(self.x, self.y);
        self.pen.line_to(px, py);
    }

    fn curve_to(&mut self, a: (f64, f64), b: (f64, f64), c: (f64, f64)) {
        self.begin();
        let (pa, pb, pc) = (self.point(a.0, a.1), self.point(b.0, b.1), self.point(c.0, c.1));
        self.pen.cubic_to(pa.0, pa.1, pb.0, pb.1, pc.0, pc.1);
        (self.x, self.y) = c;
    }

    fn curve_by(&mut self, d: [f64; 6]) {
        let a = (self.x + d[0], self.y + d[1]);
        let b = (a.0 + d[2], a.1 + d[3]);
        let c = (b.0 + d[4], b.1 + d[5]);
        self.curve_to(a, b, c);
    }

    /// Accented character `base` with `accent` placed (`adx`, `ady`) from it (`asb` the accent's
    /// own left side), both by their codes in Standard.
    fn seac(&mut self, asb: f64, adx: f64, ady: f64, base: u8, accent: u8) {
        let glyph = |code: u8| encoded(Encoding::Standard, code).and_then(|n| self.font.by_name.get(n).copied());
        let (Some(b), Some(a)) = (glyph(base), glyph(accent)) else { return };
        let (width, sbx) = (self.width, self.sbx);
        for (g, at) in [(b, (0.0, 0.0)), (a, (adx - asb + sbx, ady))] {
            let mut sub = Run { font: self.font, pen: &mut *self.pen, place: self.place, stack: Vec::new(), ps: Vec::new(), x: 0.0, y: 0.0, at, sbx: 0.0, width: 0.0, open: false, flex: None, done: false, depth: self.depth + 1 };
            if sub.depth < 4 {
                sub.exec(&self.font.charstrings[g]);
            }
            if sub.open {
                sub.pen.close();
            }
        }
        self.width = width;
        self.done = true;
    }

    fn exec(&mut self, cs: &[u8]) {
        let mut p = 0;
        while p < cs.len() && !self.done {
            let b = cs[p];
            p += 1;
            match b {
                32..=246 => self.stack.push(b as f64 - 139.0),
                247..=250 => {
                    self.stack.push((b as f64 - 247.0) * 256.0 + *cs.get(p).unwrap_or(&0) as f64 + 108.0);
                    p += 1;
                }
                251..=254 => {
                    self.stack.push(-(b as f64 - 251.0) * 256.0 - *cs.get(p).unwrap_or(&0) as f64 - 108.0);
                    p += 1;
                }
                255 => {
                    let v = cs.get(p..p + 4).map_or(0, |q| i32::from_be_bytes([q[0], q[1], q[2], q[3]]));
                    self.stack.push(v as f64);
                    p += 4;
                }
                _ => {
                    let op = if b == 12 {
                        p += 1;
                        1200 + *cs.get(p - 1).unwrap_or(&0) as u16
                    } else {
                        b as u16
                    };
                    self.op(op);
                }
            }
        }
    }

    fn op(&mut self, op: u16) {
        let s = std::mem::take(&mut self.stack);
        let a = |k: usize| s.get(k).copied().unwrap_or(0.0);
        match op {
            // the side bearing and width: where drawing starts, how far it moves the pen
            13 => {
                (self.sbx, self.width) = (a(0), a(1));
                (self.x, self.y) = (a(0), 0.0);
            }
            1207 => {
                (self.sbx, self.width) = (a(0), a(2));
                (self.x, self.y) = (a(0), a(1));
            }
            21 => self.move_by(a(0), a(1)),
            22 => self.move_by(a(0), 0.0),
            4 => self.move_by(0.0, a(0)),
            5 => self.line_by(a(0), a(1)),
            6 => self.line_by(a(0), 0.0),
            7 => self.line_by(0.0, a(0)),
            8 => self.curve_by([a(0), a(1), a(2), a(3), a(4), a(5)]),
            30 => self.curve_by([0.0, a(0), a(1), a(2), a(3), 0.0]),
            31 => self.curve_by([a(0), 0.0, a(1), a(2), 0.0, a(3)]),
            9 => {
                if self.open {
                    self.pen.close();
                    self.open = false;
                }
            }
            14 => {
                if self.open {
                    self.pen.close();
                    self.open = false;
                }
                self.done = true;
            }
            10 => {
                let mut s = s;
                let n = s.pop().unwrap_or(-1.0);
                self.stack = s;
                if self.depth < 20
                    && n >= 0.0
                    && let Some(sub) = self.font.subrs.get(n as usize)
                {
                    self.depth += 1;
                    self.exec(sub);
                    self.depth -= 1;
                }
            }
            // return: back to the charstring that called
            11 => {
                self.stack = s;
            }
            1206 => self.seac(a(0), a(1), a(2), a(3) as u8, a(4) as u8),
            1212 => {
                let mut s = s;
                let d = s.pop().unwrap_or(1.0);
                let n = s.pop().unwrap_or(0.0);
                s.push(if d == 0.0 { 0.0 } else { n / d });
                self.stack = s;
            }
            1216 => {
                // a PostScript procedure: flex (1 starts, 2 marks, 0 ends) or anything else (its
                // arguments left for pop)
                let mut s = s;
                let other = s.pop().unwrap_or(-1.0) as i64;
                let n = (s.pop().unwrap_or(0.0).max(0.0) as usize).min(s.len());
                let args = s.split_off(s.len() - n);
                self.stack = s;
                match other {
                    1 => {
                        self.flex = Some(vec![(self.x, self.y)]);
                        self.ps.clear();
                    }
                    2 => {}
                    0 => {
                        if let Some(f) = self.flex.take() {
                            // the reference point, then two curves of three points each
                            if f.len() >= 8 {
                                let (a1, b1, c1, a2, b2, c2) = (f[2], f[3], f[4], f[5], f[6], f[7]);
                                let start = f[0];
                                (self.x, self.y) = start;
                                self.curve_to(a1, b1, c1);
                                self.curve_to(a2, b2, c2);
                            } else if let Some(&last) = f.last() {
                                (self.x, self.y) = f[0];
                                self.line_by(last.0 - f[0].0, last.1 - f[0].1);
                            }
                        }
                        self.ps = vec![self.y, self.x];
                    }
                    _ => self.ps = args.into_iter().rev().collect(),
                }
            }
            1217 => {
                let mut s = s;
                s.push(self.ps.pop().unwrap_or(0.0));
                self.stack = s;
            }
            1233 => {
                (self.x, self.y) = (a(0), a(1));
            }
            // hints and dot sections: nothing drawn
            _ => {}
        }
    }
}
