//! Strokes: the outline a line of some width leaves along shapes, with its ends (caps), its
//! corners (joins) and dashes. The outline is made of pieces (a quad for each straight piece, a
//! join at each corner, a cap at each end) all wound the same way, so filling them by the
//! non-zero rule gives their union. A stroke may also thin toward its ends (taper) and swell
//! and thin along its length (wave): then it is made of many short quads, each end as wide as
//! the line is there, rounded where it turns.

use crate::Path;
use crate::shape::Sub;

/// How a stroke ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cap {
    /// Square, at the end.
    #[default]
    Butt,
    Round,
    /// Square, half the width past the end.
    Square,
}

/// How a stroke turns a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Join {
    /// Pointed, unless the point would reach past the miter limit (then bevelled).
    #[default]
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub width: f64,
    pub cap: Cap,
    pub join: Join,
    /// How far a miter may reach, in stroke widths.
    pub miter_limit: f64,
    /// Lengths of dash, gap, dash, gap… (none: a solid line).
    pub dashes: Vec<f64>,
    /// How far into the dashes the line starts.
    pub dash_offset: f64,
    /// Thinner toward its ends.
    pub taper: Option<Taper>,
    /// Swelling and thinning along it.
    pub wave: Option<Wave>,
}

impl Default for Stroke {
    fn default() -> Stroke {
        Stroke { width: 1.0, cap: Cap::Butt, join: Join::Miter, miter_limit: 4.0, dashes: Vec::new(), dash_offset: 0.0, taper: None, wave: None }
    }
}

/// A stroke thinning toward its start and end: over these lengths (in units, or as parts of
/// each line's length when `in_parts`), down to these parts of its width (0: to a point), easing
/// from straight (0) to a round bulge (1).
#[derive(Clone, Debug, PartialEq)]
pub struct Taper {
    pub in_parts: bool,
    pub start_length: f64,
    pub end_length: f64,
    pub start_width: f64,
    pub end_width: f64,
    pub start_ease: f64,
    pub end_ease: f64,
}

/// A stroke swelling and thinning along its length: its width going between whole and
/// `1 - amount` of itself, so many times along each line (`cycles`) or every `wavelength` units,
/// starting `phase` degrees into a wave.
#[derive(Clone, Debug, PartialEq)]
pub struct Wave {
    pub amount: f64,
    pub cycles: Option<f64>,
    pub wavelength: f64,
    pub phase: f64,
}

impl Taper {
    fn acts(&self) -> bool {
        (self.start_length > 0.0 && self.start_width < 1.0) || (self.end_length > 0.0 && self.end_width < 1.0)
    }
}

impl Wave {
    fn acts(&self) -> bool {
        self.amount > 0.0 && self.cycles.map_or(self.wavelength > 0.0, |c| c > 0.0)
    }
}

/// From a tip (`u` 0) to the whole width (1): straight with no ease, a quarter circle with all.
fn eased(u: f64, ease: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    let round = (1.0 - (1.0 - u) * (1.0 - u)).max(0.0).sqrt();
    u + (round - u) * ease.clamp(0.0, 1.0)
}

/// The part of its width a stroke has `at` units along a line `length` long.
pub fn width_at(taper: Option<&Taper>, wave: Option<&Wave>, at: f64, length: f64) -> f64 {
    let mut k: f64 = 1.0;
    if let Some(t) = taper.filter(|t| t.acts()) {
        let (mut a, mut b) = if t.in_parts { (t.start_length * length, t.end_length * length) } else { (t.start_length, t.end_length) };
        (a, b) = (a.max(0.0), b.max(0.0));
        // tapers longer than the line together share it, each its part
        if a + b > length && a + b > 0.0 {
            let f = length / (a + b);
            (a, b) = (a * f, b * f);
        }
        let (sw, ew) = (t.start_width.clamp(0.0, 1.0), t.end_width.clamp(0.0, 1.0));
        if a > 0.0 && at < a {
            k = k.min(sw + (1.0 - sw) * eased(at / a, t.start_ease));
        }
        if b > 0.0 && at > length - b {
            k = k.min(ew + (1.0 - ew) * eased((length - at) / b, t.end_ease));
        }
    }
    if let Some(w) = wave.filter(|w| w.acts()) {
        let wavelength = w.cycles.map_or(w.wavelength, |c| length / c);
        if wavelength > 0.0 {
            let angle = std::f64::consts::TAU * at / wavelength + w.phase.to_radians();
            k *= 1.0 - w.amount.clamp(0.0, 1.0) * (0.5 - 0.5 * angle.cos());
        }
    }
    k
}

/// The stretches `(from, to)` along a line `length` long that dashes leave drawn.
fn dash_stretches(pattern: &[f64], offset: f64, length: f64) -> Vec<(f64, f64)> {
    let total: f64 = pattern.iter().sum();
    if pattern.is_empty() || total <= 1e-9 || pattern.iter().any(|d| *d < 0.0) {
        return vec![(0.0, length)];
    }
    let mut at = offset.rem_euclid(total);
    let mut k = 0;
    while at >= pattern[k] {
        at -= pattern[k];
        k = (k + 1) % pattern.len();
    }
    let (mut s, mut out) = (0.0, Vec::new());
    let mut left = pattern[k] - at;
    while s < length && out.len() < 1_000_000 {
        let e = (s + left).min(length);
        if k % 2 == 0 {
            out.push((s, e));
        }
        s += left;
        k = (k + 1) % pattern.len();
        left = pattern[k];
    }
    out
}

/// A line stroked thinner and wider along it: quads from each point to the next, each end as
/// wide as the stroke is there, a disc where it turns, and its ends capped.
fn varying(out: &mut Path, pts: &[P], closed: bool, s: &Stroke) {
    let hw = s.width.abs() / 2.0;
    let mut line: Vec<P> = pts.to_vec();
    if closed && let Some(&f) = pts.first() {
        line.push(f);
    }
    // fine enough for the width to change smoothly
    let rough: f64 = line.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum();
    if rough <= 0.0 {
        return;
    }
    let step = (rough / 4000.0).max(0.25);
    let mut fine = vec![line[0]];
    for w in line.windows(2) {
        let n = ((w[1].0 - w[0].0).hypot(w[1].1 - w[0].1) / step).ceil().max(1.0) as usize;
        for k in 1..=n {
            let f = k as f64 / n as f64;
            fine.push((w[0].0 + (w[1].0 - w[0].0) * f, w[0].1 + (w[1].1 - w[0].1) * f));
        }
    }
    let mut along = vec![0.0];
    for w in fine.windows(2) {
        along.push(along.last().unwrap_or(&0.0) + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
    }
    let length = *along.last().unwrap_or(&0.0);
    let width = |at: f64| hw * width_at(s.taper.as_ref(), s.wave.as_ref(), at, length);
    let point_at = |at: f64| -> (P, usize) {
        let i = along.partition_point(|x| *x <= at).clamp(1, fine.len() - 1);
        let (a, b) = (along[i - 1], along[i]);
        let f = if b > a { ((at - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
        ((fine[i - 1].0 + (fine[i].0 - fine[i - 1].0) * f, fine[i - 1].1 + (fine[i].1 - fine[i - 1].1) * f), i)
    };
    let dashed = !s.dashes.is_empty() && s.dashes.iter().sum::<f64>() > 0.0;
    let stretches = if dashed { dash_stretches(&s.dashes, s.dash_offset, length) } else { vec![(0.0, length)] };
    let whole = stretches.len() == 1 && stretches[0].0 <= 0.0 && stretches[0].1 >= length;
    for (a, b) in stretches {
        let ((pa, ia), (pb, ib)) = (point_at(a), point_at(b));
        let mut piece: Vec<(P, f64)> = vec![(pa, a)];
        piece.extend((ia..ib).map(|i| (fine[i], along[i])));
        piece.push((pb, b));
        piece.dedup_by(|x, y| (x.0.0 - y.0.0).hypot(x.0.1 - y.0.1) < 1e-9);
        if piece.len() < 2 {
            if s.cap == Cap::Round {
                put(out, disc(pa, width(a)));
            }
            continue;
        }
        let dir = |i: usize| {
            let (p, q) = (piece[i].0, piece[i + 1].0);
            let l = (q.0 - p.0).hypot(q.1 - p.1).max(1e-12);
            ((q.0 - p.0) / l, (q.1 - p.1) / l)
        };
        for i in 0..piece.len() - 1 {
            let d = dir(i);
            let ((p, sp), (q, sq)) = (piece[i], piece[i + 1]);
            let (wp, wq) = (width(sp), width(sq));
            put(out, vec![(p.0 - d.1 * wp, p.1 + d.0 * wp), (q.0 - d.1 * wq, q.1 + d.0 * wq), (q.0 + d.1 * wq, q.1 - d.0 * wq), (p.0 + d.1 * wp, p.1 - d.0 * wp)]);
            // where it turns: rounded, or for the little turns along a curve the wedges between
            // the two quads filled
            if i > 0 {
                let e = dir(i - 1);
                if e.0 * d.0 + e.1 * d.1 < 0.995 {
                    put(out, disc(p, wp));
                } else {
                    put(out, vec![p, (p.0 - e.1 * wp, p.1 + e.0 * wp), (p.0 - d.1 * wp, p.1 + d.0 * wp)]);
                    put(out, vec![p, (p.0 + e.1 * wp, p.1 - e.0 * wp), (p.0 + d.1 * wp, p.1 - d.0 * wp)]);
                }
            }
        }
        if whole && closed {
            put(out, disc(piece[0].0, width(0.0)));
            continue;
        }
        let last = piece.len() - 1;
        let d0 = dir(0);
        let d1 = dir(last - 1);
        for (p, w, d) in [(piece[0].0, width(piece[0].1), (-d0.0, -d0.1)), (piece[last].0, width(piece[last].1), d1)] {
            match s.cap {
                Cap::Butt => {}
                Cap::Round => put(out, disc(p, w)),
                Cap::Square => {
                    let (nn, e) = ((-d.1 * w, d.0 * w), (p.0 + d.0 * w, p.1 + d.1 * w));
                    put(out, vec![(p.0 + nn.0, p.1 + nn.1), (e.0 + nn.0, e.1 + nn.1), (e.0 - nn.0, e.1 - nn.1), (p.0 - nn.0, p.1 - nn.1)]);
                }
            }
        }
    }
}

type P = (f64, f64);

/// A run of shapes flattened: its points, and whether it closes.
fn polyline(s: &Sub) -> (Vec<P>, bool) {
    let mut p = Path::default();
    s.flatten_into(&mut p);
    let mut pts = p.contours().first().cloned().unwrap_or_default();
    if s.closed && pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    pts.dedup_by(|a, b| (a.0 - b.0).hypot(a.1 - b.1) < 1e-9);
    (pts, s.closed)
}

/// The pieces of a polyline that dashes leave.
fn dash(pts: &[P], closed: bool, pattern: &[f64], offset: f64) -> Vec<Vec<P>> {
    let total: f64 = pattern.iter().sum();
    if pattern.is_empty() || total <= 0.0 || pattern.iter().any(|d| *d < 0.0) {
        return vec![if closed { pts.iter().chain(pts.first()).copied().collect() } else { pts.to_vec() }];
    }
    let mut line: Vec<P> = pts.to_vec();
    if closed && let Some(&f) = pts.first() {
        line.push(f);
    }
    // where in the pattern the line starts
    let mut at = offset.rem_euclid(total);
    let mut k = 0;
    while at >= pattern[k] {
        at -= pattern[k];
        k = (k + 1) % pattern.len();
    }
    let mut left = pattern[k] - at;
    let mut on = k % 2 == 0;
    let mut out = Vec::new();
    let mut piece: Vec<P> = if on { vec![line[0]] } else { Vec::new() };
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mut len = (b.0 - a.0).hypot(b.1 - a.1);
        let mut from = a;
        while len > 0.0 {
            let step = left.min(len);
            let f = step / len;
            let to = (from.0 + (b.0 - from.0) * f, from.1 + (b.1 - from.1) * f);
            if on {
                piece.push(to);
            }
            len -= step;
            left -= step;
            from = to;
            if left <= 1e-12 {
                if on && piece.len() > 1 {
                    out.push(std::mem::take(&mut piece));
                }
                piece.clear();
                k = (k + 1) % pattern.len();
                left = pattern[k];
                on = k % 2 == 0;
                if on {
                    piece.push(from);
                }
            }
        }
    }
    if on && piece.len() > 1 {
        out.push(piece);
    }
    out
}

fn signed_area(poly: &[P]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| poly[i].0 * poly[(i + 1) % n].1 - poly[(i + 1) % n].0 * poly[i].1).sum::<f64>() / 2.0
}

/// Add a polygon wound the positive way.
fn put(out: &mut Path, mut poly: Vec<P>) {
    if poly.len() < 3 {
        return;
    }
    if signed_area(&poly) < 0.0 {
        poly.reverse();
    }
    out.move_to(poly[0].0, poly[0].1);
    for p in &poly[1..] {
        out.line_to(p.0, p.1);
    }
    out.close();
}

fn disc(c: P, r: f64) -> Vec<P> {
    // enough sides that no edge strays more than a twentieth of a pixel from the circle
    let n = ((std::f64::consts::PI / (1.0 - 0.05 / r.max(0.06)).clamp(-1.0, 1.0).acos()).ceil() as usize).clamp(8, 512);
    (0..n).map(|k| {
        let a = std::f64::consts::TAU * k as f64 / n as f64;
        (c.0 + r * a.cos(), c.1 + r * a.sin())
    }).collect()
}

/// The outline of `shapes` stroked with `s`: polygons to fill by the non-zero rule.
pub fn stroke(shapes: &[Sub], s: &Stroke) -> Path {
    let mut out = Path::default();
    let hw = s.width.abs() / 2.0;
    if hw <= 0.0 {
        return out;
    }
    let varies = s.taper.as_ref().is_some_and(Taper::acts) || s.wave.as_ref().is_some_and(Wave::acts);
    for sub in shapes {
        let (pts, closed) = polyline(sub);
        if pts.is_empty() {
            continue;
        }
        if varies {
            varying(&mut out, &pts, closed, s);
            continue;
        }
        let dashed = !s.dashes.is_empty() && s.dashes.iter().sum::<f64>() > 0.0;
        let pieces: Vec<(Vec<P>, bool)> = if dashed { dash(&pts, closed, &s.dashes, s.dash_offset).into_iter().map(|p| (p, false)).collect() } else { vec![(pts, closed)] };
        for (line, closed) in pieces {
            outline(&mut out, &line, closed, hw, s);
        }
    }
    out
}

fn outline(out: &mut Path, pts: &[P], closed: bool, hw: f64, s: &Stroke) {
    let n = pts.len();
    if n == 1 || (n == 2 && pts[0] == pts[1]) {
        // a dot: only round and square ends show it
        match s.cap {
            Cap::Round => put(out, disc(pts[0], hw)),
            Cap::Square => {
                let p = pts[0];
                put(out, vec![(p.0 - hw, p.1 - hw), (p.0 + hw, p.1 - hw), (p.0 + hw, p.1 + hw), (p.0 - hw, p.1 + hw)]);
            }
            Cap::Butt => {}
        }
        return;
    }
    let segs = if closed { n } else { n - 1 };
    let dir = |k: usize| {
        let (a, b) = (pts[k % n], pts[(k + 1) % n]);
        let len = (b.0 - a.0).hypot(b.1 - a.1).max(1e-12);
        ((b.0 - a.0) / len, (b.1 - a.1) / len)
    };
    for k in 0..segs {
        let (a, b) = (pts[k], pts[(k + 1) % n]);
        let d = dir(k);
        let nn = (-d.1 * hw, d.0 * hw);
        put(out, vec![(a.0 + nn.0, a.1 + nn.1), (b.0 + nn.0, b.1 + nn.1), (b.0 - nn.0, b.1 - nn.1), (a.0 - nn.0, a.1 - nn.1)]);
    }
    // corners
    let corners: Vec<usize> = if closed { (0..n).collect() } else { (1..n - 1).collect() };
    for k in corners {
        let (d1, d2) = (dir((k + n - 1) % n), dir(k));
        let cross = d1.0 * d2.1 - d1.1 * d2.0;
        let dot = d1.0 * d2.0 + d1.1 * d2.1;
        if cross.abs() < 1e-12 && dot > 0.0 {
            continue;
        }
        let v = pts[k];
        // the outside of the turn
        let side = if cross > 0.0 { -1.0 } else { 1.0 };
        let o1 = (-d1.1 * hw * side, d1.0 * hw * side);
        let o2 = (-d2.1 * hw * side, d2.0 * hw * side);
        let turn = cross.atan2(dot).abs();
        // a flattened curve turns a little at every point: keep it smooth whatever the join
        let join = if turn < 0.2 { Join::Miter } else { s.join };
        match join {
            Join::Round => put(out, disc(v, hw)),
            Join::Bevel => put(out, vec![v, (v.0 + o1.0, v.1 + o1.1), (v.0 + o2.0, v.1 + o2.1)]),
            Join::Miter => {
                // the point reaches 1 / cos(half the turn) half-widths out
                let reach = 1.0 / (turn / 2.0).cos();
                let limit = if turn < 0.2 { f64::INFINITY } else { s.miter_limit };
                if reach <= limit && reach.is_finite() {
                    let m = ((o1.0 + o2.0) / 2.0, (o1.1 + o2.1) / 2.0);
                    let ml = m.0.hypot(m.1).max(1e-12);
                    let tip = (v.0 + m.0 / ml * hw * reach, v.1 + m.1 / ml * hw * reach);
                    put(out, vec![v, (v.0 + o1.0, v.1 + o1.1), tip, (v.0 + o2.0, v.1 + o2.1)]);
                } else {
                    put(out, vec![v, (v.0 + o1.0, v.1 + o1.1), (v.0 + o2.0, v.1 + o2.1)]);
                }
            }
        }
    }
    if !closed {
        for (p, d) in [(pts[0], { let d = dir(0); (-d.0, -d.1) }), (pts[n - 1], dir(n - 2))] {
            match s.cap {
                Cap::Butt => {}
                Cap::Round => put(out, disc(p, hw)),
                Cap::Square => {
                    let nn = (-d.1 * hw, d.0 * hw);
                    let e = (p.0 + d.0 * hw, p.1 + d.1 * hw);
                    put(out, vec![(p.0 + nn.0, p.1 + nn.1), (e.0 + nn.0, e.1 + nn.1), (e.0 - nn.0, e.1 - nn.1), (p.0 - nn.0, p.1 - nn.1)]);
                }
            }
        }
    }
}

/// The length of each dash piece of a line, for checking.
#[cfg(test)]
fn piece_lengths(pts: &[P], closed: bool, pattern: &[f64], offset: f64) -> Vec<f64> {
    dash(pts, closed, pattern, offset).iter().map(|p| p.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fill::{Rule, fill};

    fn area(p: &Path) -> f32 {
        fill(p, Rule::NonZero, 400, 400).map_or(0.0, |m| m.cover.iter().sum())
    }

    #[test]
    fn a_line_with_each_cap() {
        let line = [Sub::polygon(&[[100.0, 100.0], [200.0, 100.0]], false)];
        let with = |cap| area(&stroke(&line, &Stroke { width: 10.0, cap, ..Stroke::default() }));
        assert!((with(Cap::Butt) - 1000.0).abs() < 0.5, "{}", with(Cap::Butt));
        assert!((with(Cap::Square) - 1100.0).abs() < 0.5);
        let round = with(Cap::Round);
        assert!((round - (1000.0 + std::f32::consts::PI * 25.0)).abs() < 1.0, "{round}");
    }

    #[test]
    fn a_tapered_line_thins_to_points() {
        // 100 long, 10 wide, thinning straight to points over its first and last quarters:
        // a quad of 50 × 10 between two triangles of 25 × 10
        let line = [Sub::polygon(&[[100.0, 100.0], [200.0, 100.0]], false)];
        let taper = Taper { in_parts: true, start_length: 0.25, end_length: 0.25, start_width: 0.0, end_width: 0.0, start_ease: 0.0, end_ease: 0.0 };
        let a = area(&stroke(&line, &Stroke { width: 10.0, taper: Some(taper.clone()), ..Stroke::default() }));
        assert!((a - 750.0).abs() < 3.0, "{a}");
        // in pixels, and only at the start: half of it thins
        let px = Taper { in_parts: false, start_length: 50.0, end_length: 0.0, ..taper.clone() };
        let b = area(&stroke(&line, &Stroke { width: 10.0, taper: Some(px), ..Stroke::default() }));
        assert!((b - 750.0).abs() < 3.0, "{b}");
        // eased: a round bulge holds more
        let round = Taper { start_ease: 1.0, end_ease: 1.0, ..taper.clone() };
        let c = area(&stroke(&line, &Stroke { width: 10.0, taper: Some(round), ..Stroke::default() }));
        assert!(c > a + 50.0, "{c} against {a}");
        // tapers longer than the line share it
        let long = Taper { start_length: 1.0, end_length: 1.0, ..taper };
        let d = area(&stroke(&line, &Stroke { width: 10.0, taper: Some(long), ..Stroke::default() }));
        assert!((d - 500.0).abs() < 3.0, "a diamond: {d}");
    }

    #[test]
    fn a_waved_line_swells_and_thins() {
        let line = [Sub::polygon(&[[100.0, 100.0], [300.0, 100.0]], false)];
        // between whole and none of 10 wide: on average half
        let wave = Wave { amount: 1.0, cycles: Some(4.0), wavelength: 0.0, phase: 0.0 };
        let a = area(&stroke(&line, &Stroke { width: 10.0, wave: Some(wave.clone()), ..Stroke::default() }));
        assert!((a - 1000.0).abs() < 10.0, "{a}");
        // where a wave is thinnest the line is nothing: across at a quarter of a wave in
        let outline = stroke(&line, &Stroke { width: 10.0, wave: Some(Wave { cycles: Some(2.0), ..wave }), ..Stroke::default() });
        let m = fill(&outline, Rule::NonZero, 400, 400).unwrap();
        let at = |x: usize, y: usize| if x >= m.left && x < m.left + m.width && y >= m.top && y < m.top + m.height { m.at(x, y) } else { 0.0 };
        assert!(at(150, 99) < 0.05, "thinnest half a wave in: {}", at(150, 99));
        assert!(at(200, 96) > 0.9, "widest a whole wave in");
        // with dashes, each dash as wide as the line is where it is
        let dashed = Stroke { width: 10.0, dashes: vec![20.0, 20.0], wave: Some(Wave { amount: 0.5, cycles: Some(1.0), wavelength: 0.0, phase: 0.0 }), ..Stroke::default() };
        let b = area(&stroke(&line, &dashed));
        assert!(b > 600.0 && b < 900.0, "{b}");
    }

    #[test]
    fn corners_by_join() {
        // a right angle: the two sides overlap in a 5 × 5 square inside the corner; outside it a
        // miter fills the corner square, a bevel half of it, round a quarter circle
        let corner = [Sub::polygon(&[[100.0, 100.0], [200.0, 100.0], [200.0, 200.0]], false)];
        let with = |join| area(&stroke(&corner, &Stroke { width: 10.0, join, ..Stroke::default() }));
        let sides = 100.0 * 10.0 * 2.0 - 25.0;
        assert!((with(Join::Miter) - (sides + 25.0)).abs() < 0.6, "{}", with(Join::Miter));
        assert!((with(Join::Bevel) - (sides + 12.5)).abs() < 0.6, "{}", with(Join::Bevel));
        assert!((with(Join::Round) - (sides + std::f32::consts::PI * 25.0 / 4.0)).abs() < 0.8, "{}", with(Join::Round));
        // a sharp turn past the miter limit is bevelled
        let sharp = [Sub::polygon(&[[100.0, 100.0], [200.0, 100.0], [100.0, 110.0]], false)];
        let tip = stroke(&sharp, &Stroke { width: 10.0, ..Stroke::default() });
        let (_, _, right, _) = tip.extent().unwrap();
        assert!(right < 206.0, "no long spike: {right}");
    }

    #[test]
    fn a_closed_square_outline() {
        let square = [crate::shape::rect([100.0, 100.0], [200.0, 200.0], 0.0)];
        let a = area(&stroke(&square, &Stroke { width: 10.0, ..Stroke::default() }));
        assert!((a - (110.0 * 110.0 - 90.0 * 90.0)).abs() < 1.0, "a ring: {a}");
    }

    #[test]
    fn dashes() {
        let line = [(0.0, 0.0), (100.0, 0.0)];
        assert_eq!(piece_lengths(&line, false, &[10.0, 5.0], 0.0).len(), 7);
        let lens = piece_lengths(&line, false, &[10.0, 5.0], 3.0);
        assert!((lens[0] - 7.0).abs() < 1e-9, "starting 3 into the first dash: {lens:?}");
        let sq = [Sub::polygon(&[[100.0, 100.0], [200.0, 100.0]], false)];
        let a = area(&stroke(&sq, &Stroke { width: 4.0, dashes: vec![10.0, 10.0], ..Stroke::default() }));
        assert!((a - 200.0).abs() < 0.5, "half of the line: {a}");
    }

    #[test]
    fn a_curve_strokes_smoothly() {
        let circle = [crate::shape::ellipse([200.0, 200.0], [200.0, 200.0])];
        let a = area(&stroke(&circle, &Stroke { width: 10.0, join: Join::Bevel, ..Stroke::default() }));
        let want = std::f32::consts::PI * (105.0f32.powi(2) - 95.0f32.powi(2));
        assert!((a - want).abs() / want < 0.005, "{a} vs {want}");
    }
}
