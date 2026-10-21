//! Outlines of OpenType fonts with PostScript outlines: the `CFF ` table (and `CFF2`, its
//! variable form). The table's indexes and dictionaries are read once; a glyph's outline comes
//! from running its Type 2 charstring (with the font's shared and private subroutines; in a
//! CID-keyed font, the private ones of the font dictionary the glyph belongs to), cubic curves
//! drawn as they come. In `CFF2`, `blend` mixes each value with its deltas for the instance.

use crate::Pen;

/// An INDEX: where each piece of data starts and ends.
#[derive(Clone, Debug, Default)]
pub(crate) struct Index {
    spans: Vec<(usize, usize)>,
}

impl Index {
    pub(crate) fn get(&self, i: usize) -> Option<(usize, usize)> {
        self.spans.get(i).copied()
    }

    pub(crate) fn len(&self) -> usize {
        self.spans.len()
    }
}

pub(crate) fn u8_at(d: &[u8], at: usize) -> usize {
    d.get(at).copied().unwrap_or(0) as usize
}

pub(crate) fn uint(d: &[u8], at: usize, size: usize) -> usize {
    (0..size).fold(0, |v, k| (v << 8) | u8_at(d, at + k))
}

/// An INDEX at `at`; where it ends. CFF2's has a 32-bit count.
pub(crate) fn read_index(d: &[u8], at: usize, long_count: bool) -> (Index, usize) {
    let (count, mut p) = if long_count { (uint(d, at, 4), at + 4) } else { (uint(d, at, 2), at + 2) };
    if count == 0 {
        return (Index::default(), p);
    }
    let size = u8_at(d, p);
    p += 1;
    let offsets: Vec<usize> = (0..=count).map(|k| uint(d, p + k * size, size)).collect();
    let base = p + (count + 1) * size - 1;
    let spans = offsets.windows(2).map(|w| (base + w[0], base + w[1])).collect();
    (Index { spans }, base + offsets[count])
}

/// A DICT's operators and their operands.
pub(crate) fn read_dict(d: &[u8], from: usize, to: usize) -> Vec<(u16, Vec<f64>)> {
    let mut out = Vec::new();
    let mut operands = Vec::new();
    let mut p = from;
    while p < to.min(d.len()) {
        let b = d[p] as usize;
        match b {
            0..=21 => {
                let op = if b == 12 {
                    p += 1;
                    1200 + u8_at(d, p) as u16
                } else {
                    b as u16
                };
                out.push((op, std::mem::take(&mut operands)));
                p += 1;
            }
            28 => {
                operands.push(uint(d, p + 1, 2) as i16 as f64);
                p += 3;
            }
            29 => {
                operands.push(uint(d, p + 1, 4) as i32 as f64);
                p += 5;
            }
            30 => {
                // a real number in packed decimal
                let mut s = String::new();
                p += 1;
                'digits: loop {
                    let byte = u8_at(d, p);
                    p += 1;
                    for nib in [byte >> 4, byte & 15] {
                        match nib {
                            0..=9 => s.push((b'0' + nib as u8) as char),
                            0xA => s.push('.'),
                            0xB => s.push('E'),
                            0xC => s.push_str("E-"),
                            0xE => s.push('-'),
                            0xF => break 'digits,
                            _ => {}
                        }
                    }
                    if p >= d.len() {
                        break;
                    }
                }
                operands.push(s.parse().unwrap_or(0.0));
            }
            32..=246 => {
                operands.push(b as f64 - 139.0);
                p += 1;
            }
            247..=250 => {
                operands.push((b as f64 - 247.0) * 256.0 + u8_at(d, p + 1) as f64 + 108.0);
                p += 2;
            }
            251..=254 => {
                operands.push(-(b as f64 - 251.0) * 256.0 - u8_at(d, p + 1) as f64 - 108.0);
                p += 2;
            }
            _ => p += 1,
        }
    }
    out
}

pub(crate) fn dict_get(dict: &[(u16, Vec<f64>)], op: u16) -> Option<&[f64]> {
    dict.iter().find(|(o, _)| *o == op).map(|(_, v)| v.as_slice())
}

/// A font dictionary's private part: its subroutines and widths.
#[derive(Clone, Debug, Default)]
struct Private {
    subrs: Index,
    /// The item variation data a CFF2 private dictionary picks.
    vsindex: usize,
}

/// A `CFF ` or `CFF2` table, read.
#[derive(Clone, Debug, Default)]
pub(crate) struct Cff {
    v2: bool,
    charstrings: Index,
    global: Index,
    privates: Vec<Private>,
    /// Each glyph's font dictionary (CID-keyed fonts and CFF2); empty: all the first.
    fd_select: Vec<u8>,
    /// CFF2: where the variation store is (0: none).
    vstore: usize,
}

fn read_private(d: &[u8], table: usize, size: usize, offset: usize, v2: bool) -> Private {
    let start = table + offset;
    let dict = read_dict(d, start, start + size);
    let subrs = dict_get(&dict, 19).and_then(|v| v.first()).map(|o| read_index(d, start + *o as usize, v2).0).unwrap_or_default();
    let vsindex = dict_get(&dict, 22).and_then(|v| v.first()).map_or(0, |v| *v as usize);
    Private { subrs, vsindex }
}

fn read_fd_select(d: &[u8], at: usize, glyphs: usize) -> Vec<u8> {
    let mut out = vec![0u8; glyphs];
    match u8_at(d, at) {
        0 => {
            for (g, slot) in out.iter_mut().enumerate() {
                *slot = u8_at(d, at + 1 + g) as u8;
            }
        }
        3 | 4 => {
            let long = u8_at(d, at) == 4;
            let (n, mut p) = if long { (uint(d, at + 1, 4), at + 5) } else { (uint(d, at + 1, 2), at + 3) };
            let (gsize, fsize) = if long { (4, 2) } else { (2, 1) };
            for _ in 0..n {
                let first = uint(d, p, gsize);
                let fd = uint(d, p + gsize, fsize);
                let next = uint(d, p + gsize + fsize, gsize);
                for slot in out.iter_mut().take(next.min(glyphs)).skip(first) {
                    *slot = fd as u8;
                }
                p += gsize + fsize;
            }
        }
        _ => {}
    }
    out
}

impl Cff {
    /// The table at `at` (`v2`: a CFF2 table), for a font of `glyphs` glyphs.
    pub(crate) fn read(d: &[u8], at: usize, v2: bool, glyphs: usize) -> Option<Cff> {
        if at >= d.len() {
            return None;
        }
        let header = u8_at(d, at + 2);
        let (top, global_at) = if v2 {
            let len = uint(d, at + 3, 2);
            let top = read_dict(d, at + header, at + header + len);
            (top, at + header + len)
        } else {
            let (_names, p) = read_index(d, at + header, false);
            let (tops, p) = read_index(d, p, false);
            let (_strings, p) = read_index(d, p, false);
            let (a, b) = tops.get(0)?;
            (read_dict(d, a, b), p)
        };
        let (global, _) = read_index(d, global_at, v2);
        let charstrings = read_index(d, at + *dict_get(&top, 17)?.first()? as usize, v2).0;
        let mut privates = Vec::new();
        let mut fd_select = Vec::new();
        if let Some(fd_array) = dict_get(&top, 1236).and_then(|v| v.first()) {
            let (fds, _) = read_index(d, at + *fd_array as usize, v2);
            for k in 0..fds.len() {
                let (a, b) = fds.get(k)?;
                let fd = read_dict(d, a, b);
                match dict_get(&fd, 18) {
                    Some([size, off]) => privates.push(read_private(d, at, *size as usize, *off as usize, v2)),
                    _ => privates.push(Private::default()),
                }
            }
            if let Some(sel) = dict_get(&top, 1237).and_then(|v| v.first()) {
                fd_select = read_fd_select(d, at + *sel as usize, glyphs);
            }
        } else if let Some([size, off]) = dict_get(&top, 18) {
            privates.push(read_private(d, at, *size as usize, *off as usize, v2));
        }
        if privates.is_empty() {
            privates.push(Private::default());
        }
        let vstore = if v2 { dict_get(&top, 24).and_then(|v| v.first()).map_or(0, |o| at + *o as usize) } else { 0 };
        Some(Cff { v2, charstrings, global, privates, fd_select, vstore })
    }

    /// How many regions item variation data `vsindex` of the store mixes (CFF2).
    fn regions(&self, d: &[u8], vsindex: usize) -> usize {
        if self.vstore == 0 {
            return 0;
        }
        // the store comes after a 16-bit length
        let store = self.vstore + 2;
        let count = uint(d, store + 6, 2);
        if vsindex >= count {
            return 0;
        }
        let data = store + uint(d, store + 8 + 4 * vsindex, 4);
        uint(d, data + 4, 2)
    }

    /// The scalars of item variation data `vsindex`'s regions at the instance (CFF2).
    fn scalars(&self, d: &[u8], vsindex: usize, region_scalar: &dyn Fn(usize, usize) -> f64) -> Vec<f64> {
        let n = self.regions(d, vsindex);
        if n == 0 {
            return Vec::new();
        }
        let store = self.vstore + 2;
        let list = store + uint(d, store + 2, 4);
        let data = store + uint(d, store + 8 + 4 * vsindex, 4);
        (0..n).map(|k| region_scalar(list, uint(d, data + 6 + 2 * k, 2))).collect()
    }

    /// Glyph `g`'s outline into `pen`, through `place`. `region_scalar(list, region)` says how
    /// much a region of a variation store counts at the instance.
    pub(crate) fn outline(&self, d: &[u8], g: u16, pen: &mut dyn Pen, place: &dyn Fn(f64, f64) -> (f64, f64), region_scalar: &dyn Fn(usize, usize) -> f64) {
        let Some((a, b)) = self.charstrings.get(g as usize) else { return };
        let fd = self.fd_select.get(g as usize).copied().unwrap_or(0) as usize;
        let private = self.privates.get(fd).or(self.privates.first()).cloned().unwrap_or_default();
        let mut run = Run { cff: self, d, private: &private, pen, place, stack: Vec::new(), x: 0.0, y: 0.0, open: false, hints: 0, width_done: self.v2, depth: 0, scalars: Vec::new(), vsindex: private.vsindex, region_scalar };
        if self.v2 {
            run.scalars = self.scalars(d, run.vsindex, region_scalar);
        }
        run.exec(a, b);
        if run.open {
            run.pen.close();
        }
    }
}

fn bias(n: usize) -> i64 {
    if n < 1240 {
        107
    } else if n < 33900 {
        1131
    } else {
        32768
    }
}

/// A charstring being run.
struct Run<'a> {
    cff: &'a Cff,
    d: &'a [u8],
    private: &'a Private,
    pen: &'a mut dyn Pen,
    place: &'a dyn Fn(f64, f64) -> (f64, f64),
    stack: Vec<f64>,
    x: f64,
    y: f64,
    open: bool,
    /// Stem hints seen (their count sizes a hint mask).
    hints: usize,
    /// The advance width (an odd first operand) is passed over once.
    width_done: bool,
    depth: u32,
    scalars: Vec<f64>,
    vsindex: usize,
    region_scalar: &'a dyn Fn(usize, usize) -> f64,
}

impl Run<'_> {
    fn move_to(&mut self, dx: f64, dy: f64) {
        if self.open {
            self.pen.close();
        }
        self.x += dx;
        self.y += dy;
        let (px, py) = (self.place)(self.x, self.y);
        self.pen.move_to(px, py);
        self.open = true;
    }

    fn line_to(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        let (px, py) = (self.place)(self.x, self.y);
        self.pen.line_to(px, py);
    }

    #[allow(clippy::too_many_arguments)]
    fn curve_to(&mut self, dxa: f64, dya: f64, dxb: f64, dyb: f64, dxc: f64, dyc: f64) {
        let (ax, ay) = (self.x + dxa, self.y + dya);
        let (bx, by) = (ax + dxb, ay + dyb);
        let (cx, cy) = (bx + dxc, by + dyc);
        let (pa, pb, pc) = ((self.place)(ax, ay), (self.place)(bx, by), (self.place)(cx, cy));
        self.pen.cubic_to(pa.0, pa.1, pb.0, pb.1, pc.0, pc.1);
        (self.x, self.y) = (cx, cy);
    }

    /// Pass over the width an odd count of operands starts with (CFF only).
    fn width(&mut self, even: bool) {
        if !self.width_done {
            if (self.stack.len() % 2 == 1) == even && !self.stack.is_empty() {
                self.stack.remove(0);
            }
            self.width_done = true;
        }
    }

    fn stems(&mut self) {
        self.width(true);
        self.hints += self.stack.len() / 2;
        self.stack.clear();
    }

    /// Run the charstring at `a..b`. False when it ended the glyph.
    fn exec(&mut self, a: usize, b: usize) -> bool {
        let d = self.d;
        let mut p = a;
        while p < b {
            let op = d[p] as usize;
            p += 1;
            match op {
                32..=246 => self.stack.push(op as f64 - 139.0),
                247..=250 => {
                    self.stack.push((op as f64 - 247.0) * 256.0 + u8_at(d, p) as f64 + 108.0);
                    p += 1;
                }
                251..=254 => {
                    self.stack.push(-(op as f64 - 251.0) * 256.0 - u8_at(d, p) as f64 - 108.0);
                    p += 1;
                }
                28 => {
                    self.stack.push(uint(d, p, 2) as i16 as f64);
                    p += 2;
                }
                255 => {
                    self.stack.push(uint(d, p, 4) as i32 as f64 / 65536.0);
                    p += 4;
                }
                1 | 3 | 18 | 23 => self.stems(),
                19 | 20 => {
                    // a hint mask: stems given just before it count, then the mask's bytes
                    if !self.stack.is_empty() {
                        self.stems();
                    }
                    self.width_done = true;
                    p += self.hints.div_ceil(8);
                }
                21 => {
                    self.width(true);
                    let n = self.stack.len();
                    let (dx, dy) = (self.stack.get(n.wrapping_sub(2)).copied().unwrap_or(0.0), self.stack.last().copied().unwrap_or(0.0));
                    self.move_to(dx, dy);
                    self.stack.clear();
                }
                22 | 4 => {
                    self.width(false);
                    let v = self.stack.last().copied().unwrap_or(0.0);
                    if op == 22 { self.move_to(v, 0.0) } else { self.move_to(0.0, v) }
                    self.stack.clear();
                }
                5 => {
                    let s = std::mem::take(&mut self.stack);
                    for c in s.chunks(2) {
                        if c.len() == 2 {
                            self.line_to(c[0], c[1]);
                        }
                    }
                }
                6 | 7 => {
                    let s = std::mem::take(&mut self.stack);
                    let mut horizontal = op == 6;
                    for v in s {
                        if horizontal { self.line_to(v, 0.0) } else { self.line_to(0.0, v) }
                        horizontal = !horizontal;
                    }
                }
                8 => {
                    let s = std::mem::take(&mut self.stack);
                    for c in s.chunks(6) {
                        if c.len() == 6 {
                            self.curve_to(c[0], c[1], c[2], c[3], c[4], c[5]);
                        }
                    }
                }
                24 => {
                    // curves, then a line
                    let s = std::mem::take(&mut self.stack);
                    let n = s.len().saturating_sub(2);
                    for c in s[..n].chunks(6) {
                        if c.len() == 6 {
                            self.curve_to(c[0], c[1], c[2], c[3], c[4], c[5]);
                        }
                    }
                    if s.len() >= 2 {
                        self.line_to(s[n], s[n + 1]);
                    }
                }
                25 => {
                    // lines, then a curve
                    let s = std::mem::take(&mut self.stack);
                    let n = s.len().saturating_sub(6);
                    for c in s[..n].chunks(2) {
                        if c.len() == 2 {
                            self.line_to(c[0], c[1]);
                        }
                    }
                    if s.len() >= 6 {
                        self.curve_to(s[n], s[n + 1], s[n + 2], s[n + 3], s[n + 4], s[n + 5]);
                    }
                }
                26 | 27 => {
                    // vvcurveto, hhcurveto
                    let s = std::mem::take(&mut self.stack);
                    let mut k = 0;
                    let mut first = if s.len() % 4 == 1 {
                        k = 1;
                        s[0]
                    } else {
                        0.0
                    };
                    while k + 4 <= s.len() {
                        if op == 26 {
                            self.curve_to(first, s[k], s[k + 1], s[k + 2], 0.0, s[k + 3]);
                        } else {
                            self.curve_to(s[k], first, s[k + 1], s[k + 2], s[k + 3], 0.0);
                        }
                        first = 0.0;
                        k += 4;
                    }
                }
                30 | 31 => {
                    // vhcurveto, hvcurveto: the curves turn between vertical and horizontal
                    let s = std::mem::take(&mut self.stack);
                    let mut horizontal = op == 31;
                    let mut k = 0;
                    while k + 4 <= s.len() {
                        let last = if s.len() - (k + 4) == 1 { s[k + 4] } else { 0.0 };
                        if horizontal {
                            self.curve_to(s[k], 0.0, s[k + 1], s[k + 2], last, s[k + 3]);
                        } else {
                            self.curve_to(0.0, s[k], s[k + 1], s[k + 2], s[k + 3], last);
                        }
                        horizontal = !horizontal;
                        k += 4;
                    }
                }
                10 | 29 => {
                    let Some(n) = self.stack.pop() else { continue };
                    let subrs = if op == 10 { &self.private.subrs } else { &self.cff.global };
                    let i = n as i64 + bias(subrs.len());
                    if self.depth > 10 || i < 0 {
                        continue;
                    }
                    let Some((sa, sb)) = subrs.get(i as usize) else { continue };
                    self.depth += 1;
                    let go_on = self.exec(sa, sb);
                    self.depth -= 1;
                    if !go_on {
                        return false;
                    }
                }
                11 => return true,
                14 => {
                    self.width(true);
                    if self.open {
                        self.pen.close();
                        self.open = false;
                    }
                    return false;
                }
                15 => {
                    // CFF2: which variation data blends use
                    if let Some(v) = self.stack.pop() {
                        self.vsindex = v as usize;
                        self.scalars = self.cff.scalars(self.d, self.vsindex, self.region_scalar);
                    }
                }
                16 => {
                    // CFF2: values mixed with their deltas
                    let Some(n) = self.stack.pop() else { continue };
                    let n = n as usize;
                    let k = self.scalars.len();
                    let total = n * (k + 1);
                    if self.stack.len() < total {
                        self.stack.clear();
                        continue;
                    }
                    let base = self.stack.len() - total;
                    for i in 0..n {
                        let mut v = self.stack[base + i];
                        for (j, s) in self.scalars.iter().enumerate() {
                            v += self.stack[base + n + i * k + j] * s;
                        }
                        self.stack[base + i] = v;
                    }
                    self.stack.truncate(base + n);
                }
                12 => {
                    let op2 = u8_at(d, p);
                    p += 1;
                    let s = std::mem::take(&mut self.stack);
                    let g = |k: usize| s.get(k).copied().unwrap_or(0.0);
                    match op2 {
                        35 => {
                            // flex
                            self.curve_to(g(0), g(1), g(2), g(3), g(4), g(5));
                            self.curve_to(g(6), g(7), g(8), g(9), g(10), g(11));
                        }
                        34 => {
                            // hflex
                            let y0 = self.y;
                            self.curve_to(g(0), 0.0, g(1), g(2), g(3), 0.0);
                            let back = y0 - self.y;
                            self.curve_to(g(4), 0.0, g(5), back, g(6), 0.0);
                        }
                        36 => {
                            // hflex1
                            let y0 = self.y;
                            self.curve_to(g(0), g(1), g(2), g(3), g(4), 0.0);
                            let back = y0 - (self.y + g(7));
                            self.curve_to(g(5), 0.0, g(6), g(7), g(8), back);
                        }
                        37 => {
                            // flex1: the last point by whichever way moved more
                            let (x0, y0) = (self.x, self.y);
                            let dx: f64 = g(0) + g(2) + g(4) + g(6) + g(8);
                            let dy: f64 = g(1) + g(3) + g(5) + g(7) + g(9);
                            self.curve_to(g(0), g(1), g(2), g(3), g(4), g(5));
                            let (lx, ly) = if dx.abs() > dy.abs() { (g(10), y0 - (self.y + g(7) + g(9))) } else { (x0 - (self.x + g(6) + g(8)), g(10)) };
                            self.curve_to(g(6), g(7), g(8), g(9), lx, ly);
                        }
                        _ => {}
                    }
                }
                _ => self.stack.clear(),
            }
        }
        true
    }
}

#[cfg(test)]
mod tests;
