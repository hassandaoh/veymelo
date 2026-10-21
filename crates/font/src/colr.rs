//! Colour glyphs drawn from a graph of paints (`COLR` version 1): outlines that clip what is
//! painted inside them; solid colours and linear, radial and sweep gradients, their colours from
//! the font's palette (or the text's own); transforms of what is under them; and two paintings
//! combined by one of the ways of compositing and blending. Read here into a [`Paint`] tree
//! (shared parts read again where each is used, a part that would hold itself left out), at this
//! instance of a variable font (its alphas, stops, points and transforms moved by the deltas of
//! the table's variation store).

use crate::{Font, byte_at, fixed_at, i16_at, u16_at, u32_at};

/// A colour of the font's palette (straight sRGB RGBA), or the text's own (none), and how much
/// of it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ink {
    pub colour: Option<[f32; 4]>,
    pub alpha: f64,
}

/// How a gradient carries on past its first and last stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extend {
    /// The end colours carry on.
    Pad,
    /// The stops again and again.
    Repeat,
    /// The stops back and forth.
    Reflect,
}

/// A gradient's colours: its stops, in their order along it (those at one place in the order the
/// font gives them), and how it carries on past them.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorLine {
    pub extend: Extend,
    pub stops: Vec<(f64, Ink)>,
}

/// A paint of a colour glyph, in font units (y up), angles counter-clockwise.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    /// Nothing (a paint the font got wrong, or one that would hold itself).
    Nothing,
    /// Each painted over those before it.
    Layers(Vec<Paint>),
    Solid(Ink),
    /// Colours along `p0` to `p1`, the same along lines parallel to `p0`–`p2`.
    Linear { line: ColorLine, p0: [f64; 2], p1: [f64; 2], p2: [f64; 2] },
    /// Colours over the circles from (`c0`, `r0`) to (`c1`, `r1`) and on past them.
    Radial { line: ColorLine, c0: [f64; 2], r0: f64, c1: [f64; 2], r1: f64 },
    /// Colours round `centre` from `start` to `end` degrees.
    Sweep { line: ColorLine, centre: [f64; 2], start: f64, end: f64 },
    /// `paint` only inside the outline of `glyph`.
    Glyph { glyph: u16, paint: Box<Paint> },
    /// `paint` moved by `m` (`[xx, yx, xy, yy, dx, dy]`: x' = xx·x + xy·y + dx, y' = yx·x + yy·y + dy).
    Transform { m: [f64; 6], paint: Box<Paint> },
    /// `source` put on `backdrop` the way `mode` says (the font's numbers: 0 clear … 3 over …
    /// 12 plus, 13 screen … 23 multiply, 24 hue … 27 luminosity).
    Composite { source: Box<Paint>, mode: u8, backdrop: Box<Paint> },
}

/// A colour glyph of paints: its root, and the box it is painted in (`[least x, least y, most x,
/// most y]`, font units).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorGlyph {
    pub paint: Paint,
    pub bounds: [f64; 4],
}

/// The most paints read for one glyph (a font that shares its parts too often stops there).
const MOST: usize = 200_000;

/// An offset of three bytes.
fn u24_at(d: &[u8], at: usize) -> usize {
    (u16_at(d, at) as usize) << 8 | byte_at(d, at + 2) as usize
}

/// Matrix `a` after `b` (what `b` moves, then moved by `a`).
pub fn times(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

/// `m` about `c` instead of the origin.
fn about(m: [f64; 6], c: [f64; 2]) -> [f64; 6] {
    times([1.0, 0.0, 0.0, 1.0, c[0], c[1]], times(m, [1.0, 0.0, 0.0, 1.0, -c[0], -c[1]]))
}

struct Reader<'a> {
    font: &'a Font,
    d: &'a [u8],
    bases: usize,
    layers: usize,
    map: usize,
    store: usize,
    read: usize,
}

impl Reader<'_> {
    /// How much variable value `k` of those from `base` moves at this instance, in the units its
    /// field is stored in.
    fn delta(&self, base: u32, k: u32) -> f64 {
        if self.store == 0 || base == u32::MAX || !self.font.varied() {
            return 0.0;
        }
        let Some(i) = base.checked_add(k) else { return 0.0 };
        let (outer, inner) = if self.map == 0 { ((i >> 16) as usize, (i & 0xFFFF) as usize) } else { self.font.delta_set(self.map, i as usize) };
        self.font.item_delta(self.store, outer, inner)
    }

    /// The paint table glyph `g`'s colour starts at.
    fn base(&self, g: u16) -> Option<usize> {
        let (d, at) = (self.d, self.bases);
        if at == 0 {
            return None;
        }
        let (mut lo, mut hi) = (0usize, u32_at(d, at) as usize);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let r = at + 4 + 6 * mid;
            match u16_at(d, r).cmp(&g) {
                std::cmp::Ordering::Equal => return Some(at + u32_at(d, r + 2) as usize),
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    fn ink(&self, index: u16, alpha: f64) -> Ink {
        Ink { colour: if index == 0xFFFF { None } else { Some(self.font.palette(index).unwrap_or([0.0; 4])) }, alpha }
    }

    fn line(&self, at: usize, var: bool) -> ColorLine {
        let d = self.d;
        let extend = match byte_at(d, at) {
            1 => Extend::Repeat,
            2 => Extend::Reflect,
            _ => Extend::Pad,
        };
        let size = if var { 10 } else { 6 };
        let mut stops: Vec<(f64, Ink)> = (0..u16_at(d, at + 1) as usize)
            .map(|i| {
                let s = at + 3 + size * i;
                let base = if var { u32_at(d, s + 6) } else { u32::MAX };
                let offset = (i16_at(d, s) as f64 + self.delta(base, 0)) / 16384.0;
                let alpha = (i16_at(d, s + 4) as f64 + self.delta(base, 1)) / 16384.0;
                (offset, self.ink(u16_at(d, s + 2), alpha))
            })
            .collect();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        ColorLine { extend, stops }
    }

    /// The paint at `at`, `within` the paints it is part of (none of which it may be).
    fn paint(&mut self, at: usize, within: &mut Vec<usize>) -> Paint {
        self.read += 1;
        if within.contains(&at) || within.len() > 64 || self.read > MOST || at >= self.d.len() {
            return Paint::Nothing;
        }
        within.push(at);
        let p = self.read_paint(at, within);
        within.pop();
        p
    }

    fn read_paint(&mut self, at: usize, within: &mut Vec<usize>) -> Paint {
        let d = self.d;
        let format = byte_at(d, at);
        let var = format % 2 == 1 && (3..=31).contains(&format);
        let child = |r: &mut Self, within: &mut Vec<usize>| Box::new(r.paint(at + u24_at(d, at + 1), within));
        // the variable values from `base`, each added to its field: whole units, 2.14 or 16.16
        let base_at = |k: usize| if var { u32_at(d, at + k) } else { u32::MAX };
        let fword = |r: &Self, off: usize, base: u32, k: u32| i16_at(d, at + off) as f64 + r.delta(base, k);
        let f2 = |r: &Self, off: usize, base: u32, k: u32| (i16_at(d, at + off) as f64 + r.delta(base, k)) / 16384.0;
        match format {
            1 => {
                let (n, first) = (byte_at(d, at + 1) as usize, u32_at(d, at + 2) as usize);
                let count = if self.layers == 0 { 0 } else { u32_at(d, self.layers) as usize };
                let mut kids = Vec::with_capacity(n);
                for i in first..(first + n).min(count) {
                    let p = self.layers + u32_at(d, self.layers + 4 + 4 * i) as usize;
                    kids.push(self.paint(p, within));
                }
                Paint::Layers(kids)
            }
            2 | 3 => {
                let base = base_at(5);
                Paint::Solid(self.ink(u16_at(d, at + 1), f2(self, 3, base, 0)))
            }
            4 | 5 => {
                let (line, base) = (self.line(at + u24_at(d, at + 1), var), base_at(16));
                let p = |k: u32| [fword(self, 4 + 4 * k as usize, base, 2 * k), fword(self, 6 + 4 * k as usize, base, 2 * k + 1)];
                Paint::Linear { p0: p(0), p1: p(1), p2: p(2), line }
            }
            6 | 7 => {
                let (line, base) = (self.line(at + u24_at(d, at + 1), var), base_at(16));
                let r = |off: usize, k: u32| u16_at(d, at + off) as f64 + self.delta(base, k);
                Paint::Radial { c0: [fword(self, 4, base, 0), fword(self, 6, base, 1)], r0: r(8, 2), c1: [fword(self, 10, base, 3), fword(self, 12, base, 4)], r1: r(14, 5), line }
            }
            8 | 9 => {
                let (line, base) = (self.line(at + u24_at(d, at + 1), var), base_at(12));
                // stored less one, in halves of a turn
                Paint::Sweep { centre: [fword(self, 4, base, 0), fword(self, 6, base, 1)], start: (f2(self, 8, base, 2) + 1.0) * 180.0, end: (f2(self, 10, base, 3) + 1.0) * 180.0, line }
            }
            10 => Paint::Glyph { paint: child(self, within), glyph: u16_at(d, at + 4) },
            11 => match self.base(u16_at(d, at + 1)) {
                Some(p) => self.paint(p, within),
                None => Paint::Nothing,
            },
            12 | 13 => {
                let t = at + u24_at(d, at + 4);
                let base = if var { u32_at(d, t + 24) } else { u32::MAX };
                let m = std::array::from_fn(|k| fixed_at(d, t + 4 * k) + self.delta(base, k as u32) / 65536.0);
                Paint::Transform { m, paint: child(self, within) }
            }
            14 | 15 => {
                let base = base_at(8);
                Paint::Transform { m: [1.0, 0.0, 0.0, 1.0, fword(self, 4, base, 0), fword(self, 6, base, 1)], paint: child(self, within) }
            }
            16..=23 => {
                let uniform = format >= 20;
                let centred = matches!(format, 18 | 19 | 22 | 23);
                let n = if uniform { 1 } else { 2 };
                let base = base_at(4 + 2 * n + if centred { 4 } else { 0 });
                let sx = f2(self, 4, base, 0);
                let sy = if uniform { sx } else { f2(self, 6, base, 1) };
                let m = [sx, 0.0, 0.0, sy, 0.0, 0.0];
                let m = if centred { about(m, [fword(self, 4 + 2 * n, base, n as u32), fword(self, 6 + 2 * n, base, n as u32 + 1)]) } else { m };
                Paint::Transform { m, paint: child(self, within) }
            }
            24..=27 => {
                let centred = format >= 26;
                let base = base_at(if centred { 10 } else { 6 });
                let (s, c) = (f2(self, 4, base, 0) * std::f64::consts::PI).sin_cos();
                let m = [c, s, -s, c, 0.0, 0.0];
                let m = if centred { about(m, [fword(self, 6, base, 1), fword(self, 8, base, 2)]) } else { m };
                Paint::Transform { m, paint: child(self, within) }
            }
            28..=31 => {
                let centred = format >= 30;
                let base = base_at(if centred { 12 } else { 8 });
                let (x, y) = (f2(self, 4, base, 0) * std::f64::consts::PI, f2(self, 6, base, 1) * std::f64::consts::PI);
                let m = [1.0, y.tan(), -x.tan(), 1.0, 0.0, 0.0];
                let m = if centred { about(m, [fword(self, 8, base, 2), fword(self, 10, base, 3)]) } else { m };
                Paint::Transform { m, paint: child(self, within) }
            }
            32 => {
                let source = Box::new(self.paint(at + u24_at(d, at + 1), within));
                let mode = byte_at(d, at + 4);
                let backdrop = Box::new(self.paint(at + u24_at(d, at + 5), within));
                Paint::Composite { source, mode: if mode <= 27 { mode } else { 0 }, backdrop }
            }
            _ => Paint::Nothing,
        }
    }

    /// The clip box the font gives glyph `g`, rounded outward.
    fn clip(&self, clips: usize, g: u16) -> Option<[f64; 4]> {
        let d = self.d;
        if clips == 0 {
            return None;
        }
        let (mut lo, mut hi) = (0usize, u32_at(d, clips + 1) as usize);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let r = clips + 5 + 7 * mid;
            if g < u16_at(d, r) {
                hi = mid;
            } else if g > u16_at(d, r + 2) {
                lo = mid + 1;
            } else {
                let b = clips + u24_at(d, r + 4);
                let base = if byte_at(d, b) == 2 { u32_at(d, b + 9) } else { u32::MAX };
                let v = |k: usize| i16_at(d, b + 1 + 2 * k) as f64 + self.delta(base, k as u32);
                return Some([v(0).floor(), v(1).floor(), v(2).ceil(), v(3).ceil()]);
            }
        }
        None
    }
}

/// Boxes `[least x, least y, most x, most y]`: none, painting everywhere.
type Reach = Option<[f64; 4]>;

const EMPTY: [f64; 4] = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];

fn union(a: Reach, b: Reach) -> Reach {
    Some([a?[0].min(b?[0]), a?[1].min(b?[1]), a?[2].max(b?[2]), a?[3].max(b?[3])])
}

fn meet(a: Reach, b: Reach) -> Reach {
    match (a, b) {
        (Some(a), Some(b)) => Some([a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]),
        (a, None) => a,
        (None, b) => b,
    }
}

impl Paint {
    /// Where the paint can paint, its parts moved by `m`: none, everywhere.
    pub fn reach(&self, font: &Font, m: [f64; 6]) -> Reach {
        match self {
            Paint::Nothing => Some(EMPTY),
            Paint::Layers(kids) => kids.iter().try_fold(EMPTY, |r, k| union(Some(r), k.reach(font, m))),
            Paint::Solid(_) | Paint::Linear { .. } | Paint::Radial { .. } | Paint::Sweep { .. } => None,
            Paint::Glyph { glyph, .. } => Some(font.bounds_placed(*glyph, &|x, y| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])).unwrap_or(EMPTY)),
            Paint::Transform { m: t, paint } => paint.reach(font, times(m, *t)),
            Paint::Composite { source, mode, backdrop } => {
                let (s, b) = (source.reach(font, m), backdrop.reach(font, m));
                match mode {
                    0 => Some(EMPTY),
                    1 | 7 => s,
                    2 | 8 => b,
                    5 | 6 => meet(s, b),
                    _ => union(s, b),
                }
            }
        }
    }
}

impl Font {
    /// Glyph `g`'s colour as a graph of paints (`COLR` version 1), with the box it is painted in
    /// (the font's clip box, else where its outlines reach). None when it has none, or when what
    /// it paints has no bounds.
    pub fn color_glyph(&self, g: u16) -> Option<ColorGlyph> {
        let (d, t) = (self.data(), self.colr);
        if t == 0 || u16_at(d, t) < 1 {
            return None;
        }
        let at = |off: usize| match u32_at(d, t + off) as usize {
            0 => 0,
            o => t + o,
        };
        let mut r = Reader { font: self, d, bases: at(14), layers: at(18), map: at(26), store: at(30), read: 0 };
        let root = r.base(g)?;
        let paint = r.paint(root, &mut Vec::new());
        let bounds = r.clip(at(22), g).or_else(|| paint.reach(self, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]))?;
        Some(ColorGlyph { paint, bounds })
    }
}

#[cfg(test)]
mod tests;
