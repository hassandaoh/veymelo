//! Shapes as compositing tools keep them: runs of points, each with tangents in and out
//! (relative to the point), open or closed. Between two points is a cubic curve (a straight
//! line when both tangents are zero).

use crate::Path;

/// A point of a shape and its tangents, relative to it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub p: [f64; 2],
    pub i: [f64; 2],
    pub o: [f64; 2],
}

impl Vertex {
    pub fn at(x: f64, y: f64) -> Vertex {
        Vertex { p: [x, y], i: [0.0; 2], o: [0.0; 2] }
    }
}

/// One run of points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sub {
    pub v: Vec<Vertex>,
    pub closed: bool,
}

/// A cubic's four points.
pub type Cubic = [[f64; 2]; 4];

fn add(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

impl Sub {
    /// Straight lines through `points`.
    pub fn polygon(points: &[[f64; 2]], closed: bool) -> Sub {
        Sub { v: points.iter().map(|p| Vertex::at(p[0], p[1])).collect(), closed }
    }

    /// How many curves it has (a closed one has one more, back to its start).
    pub fn segments(&self) -> usize {
        match (self.v.len(), self.closed) {
            (0, _) => 0,
            (n, true) => n,
            (n, false) => n - 1,
        }
    }

    /// Curve `k`, from point `k` to the next.
    pub fn segment(&self, k: usize) -> Cubic {
        let a = &self.v[k];
        let b = &self.v[(k + 1) % self.v.len()];
        [a.p, add(a.p, a.o), add(b.p, b.i), b.p]
    }

    /// The same shape run the other way.
    pub fn reversed(&self) -> Sub {
        let mut v: Vec<Vertex> = self.v.iter().rev().map(|x| Vertex { p: x.p, i: x.o, o: x.i }).collect();
        if self.closed && !v.is_empty() {
            // a closed run keeps its first point first
            v.rotate_right(1);
        }
        Sub { v, closed: self.closed }
    }

    /// Every point moved by `f` (an affine map keeps the curves exact).
    pub fn map(&self, f: &dyn Fn([f64; 2]) -> [f64; 2]) -> Sub {
        let v = self
            .v
            .iter()
            .map(|x| {
                let p = f(x.p);
                Vertex { p, i: sub(f(add(x.p, x.i)), p), o: sub(f(add(x.p, x.o)), p) }
            })
            .collect();
        Sub { v, closed: self.closed }
    }

    /// Its curves as straight pieces, added to `path` (a closed one closed).
    pub fn flatten_into(&self, path: &mut Path) {
        let Some(first) = self.v.first() else { return };
        path.move_to(first.p[0], first.p[1]);
        for k in 0..self.segments() {
            let [s, a, b, e] = self.segment(k);
            if a == s && b == e {
                path.line_to(e[0], e[1]);
            } else {
                path.cubic_to(a[0], a[1], b[0], b[1], e[0], e[1]);
            }
        }
        if self.closed {
            path.close();
        }
    }

    /// Its length along its curves.
    pub fn length(&self) -> f64 {
        (0..self.segments()).map(|k| cubic_length(&self.segment(k), 0.0, 1.0)).sum()
    }

    /// The part from `from` to `to` along it (lengths, `from` < `to`), as an open run.
    pub fn cut(&self, from: f64, to: f64) -> Sub {
        let mut out = Sub::default();
        let mut walked = 0.0;
        for k in 0..self.segments() {
            let c = self.segment(k);
            let len = cubic_length(&c, 0.0, 1.0);
            let (a, b) = (walked, walked + len);
            walked = b;
            if b < from || a > to || len <= 0.0 {
                continue;
            }
            let t0 = if from > a { t_at_length(&c, from - a) } else { 0.0 };
            let t1 = if to < b { t_at_length(&c, to - a) } else { 1.0 };
            let part = sub_cubic(&c, t0, t1);
            if out.v.is_empty() {
                out.v.push(Vertex { p: part[0], i: [0.0; 2], o: sub(part[1], part[0]) });
            } else if let Some(last) = out.v.last_mut() {
                last.o = sub(part[1], part[0]);
            }
            out.v.push(Vertex { p: part[3], i: sub(part[2], part[3]), o: [0.0; 2] });
        }
        out
    }

    /// The smallest box round its points and tangents: left, top, right, bottom.
    pub fn extent(&self) -> Option<[f64; 4]> {
        let mut pts = self.v.iter().flat_map(|x| [x.p, add(x.p, x.i), add(x.p, x.o)]);
        let p = pts.next()?;
        Some(pts.fold([p[0], p[1], p[0], p[1]], |e, q| [e[0].min(q[0]), e[1].min(q[1]), e[2].max(q[0]), e[3].max(q[1])]))
    }

    /// The mean of its points (a centre to work about).
    pub fn centre(&self) -> [f64; 2] {
        let n = self.v.len().max(1) as f64;
        let s = self.v.iter().fold([0.0, 0.0], |s, x| add(s, x.p));
        [s[0] / n, s[1] / n]
    }
}

/// Shapes as straight pieces.
pub fn flatten(shapes: &[Sub]) -> Path {
    let mut p = Path::default();
    for s in shapes {
        s.flatten_into(&mut p);
    }
    p
}

/// The point on a cubic at `t`.
pub fn cubic_at(c: &Cubic, t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let (a, b, d, e) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [a * c[0][0] + b * c[1][0] + d * c[2][0] + e * c[3][0], a * c[0][1] + b * c[1][1] + d * c[2][1] + e * c[3][1]]
}

/// A cubic's direction (and speed) at `t`.
pub fn cubic_tangent(c: &Cubic, t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let d = |k: usize| 3.0 * u * u * (c[1][k] - c[0][k]) + 6.0 * u * t * (c[2][k] - c[1][k]) + 3.0 * t * t * (c[3][k] - c[2][k]);
    [d(0), d(1)]
}

/// Which way a cubic goes at `t`, as a unit vector. Where its speed is zero (an end whose
/// handle sits on it), the way toward the next control point, or along its chord.
pub fn cubic_direction(c: &Cubic, t: f64) -> [f64; 2] {
    let unit = |d: [f64; 2]| {
        let l = d[0].hypot(d[1]);
        (l > 1e-9).then(|| [d[0] / l, d[1] / l])
    };
    let towards = if t < 0.5 { sub(c[2], c[0]) } else { sub(c[3], c[1]) };
    unit(cubic_tangent(c, t)).or_else(|| unit(towards)).or_else(|| unit(sub(c[3], c[0]))).unwrap_or([0.0; 2])
}

/// Gauss-Legendre: five points and their weights on -1…1.
const GAUSS: [(f64, f64); 5] = [
    (0.0, 0.568_888_888_888_888_9),
    (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
    (0.906_179_845_938_664, 0.236_926_885_056_189_1),
];

/// A cubic's length from `t0` to `t1`, in eight pieces of five-point Gauss-Legendre each.
pub fn cubic_length(c: &Cubic, t0: f64, t1: f64) -> f64 {
    const PIECES: usize = 8;
    let h = (t1 - t0) / PIECES as f64;
    (0..PIECES)
        .map(|k| {
            let (m, half) = (t0 + h * (k as f64 + 0.5), h / 2.0);
            GAUSS
                .iter()
                .map(|(x, w)| {
                    let d = cubic_tangent(c, m + half * x);
                    w * d[0].hypot(d[1])
                })
                .sum::<f64>()
                * half
        })
        .sum()
}

/// Where along a cubic (its t) the length `s` from its start is.
pub fn t_at_length(c: &Cubic, s: f64) -> f64 {
    let total = cubic_length(c, 0.0, 1.0);
    if total.is_nan() || total <= 0.0 || s <= 0.0 {
        return 0.0;
    }
    if s >= total {
        return 1.0;
    }
    let mut t = s / total;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..30 {
        let err = cubic_length(c, 0.0, t) - s;
        if err.abs() < 1e-9 * total.max(1.0) {
            break;
        }
        if err > 0.0 {
            hi = t;
        } else {
            lo = t;
        }
        let d = cubic_tangent(c, t);
        let speed = d[0].hypot(d[1]);
        let next = if speed > 1e-12 { t - err / speed } else { 0.5 * (lo + hi) };
        t = if next > lo && next < hi { next } else { 0.5 * (lo + hi) };
    }
    t
}

/// The part of a cubic from `t0` to `t1`.
pub fn sub_cubic(c: &Cubic, t0: f64, t1: f64) -> Cubic {
    // the part from t0 on, then the first part of that
    let right = split(c, t0).1;
    if t1 >= 1.0 {
        return right;
    }
    let u = if t0 < 1.0 { (t1 - t0) / (1.0 - t0) } else { 0.0 };
    split(&right, u).0
}

/// A cubic cut in two at `t`.
pub fn split(c: &Cubic, t: f64) -> (Cubic, Cubic) {
    let ab = lerp(c[0], c[1], t);
    let bc = lerp(c[1], c[2], t);
    let cd = lerp(c[2], c[3], t);
    let abc = lerp(ab, bc, t);
    let bcd = lerp(bc, cd, t);
    let m = lerp(abc, bcd, t);
    ([c[0], ab, abc, m], [m, bcd, cd, c[3]])
}

/// The handle length that makes a cubic follow a circle through `sweep` radians, for radius 1.
pub fn arc_handle(sweep: f64) -> f64 {
    4.0 / 3.0 * (sweep / 4.0).tan()
}

/// A rectangle of `size` centred at `centre`, its corners rounded by `round` (a radius, held to
/// half the shorter side). It starts at the top right and goes clockwise, as such paths do.
pub fn rect(size: [f64; 2], centre: [f64; 2], round: f64) -> Sub {
    let (w, h) = (size[0].abs() / 2.0, size[1].abs() / 2.0);
    let (x0, y0, x1, y1) = (centre[0] - w, centre[1] - h, centre[0] + w, centre[1] + h);
    let r = round.max(0.0).min(w).min(h);
    if r <= 0.0 {
        return Sub::polygon(&[[x1, y0], [x1, y1], [x0, y1], [x0, y0]], true);
    }
    let k = r * arc_handle(std::f64::consts::FRAC_PI_2);
    let v = |x: f64, y: f64, i: [f64; 2], o: [f64; 2]| Vertex { p: [x, y], i, o };
    Sub {
        v: vec![
            v(x1, y0 + r, [0.0, -k], [0.0; 2]),
            v(x1, y1 - r, [0.0; 2], [0.0, k]),
            v(x1 - r, y1, [k, 0.0], [0.0; 2]),
            v(x0 + r, y1, [0.0; 2], [-k, 0.0]),
            v(x0, y1 - r, [0.0, k], [0.0; 2]),
            v(x0, y0 + r, [0.0; 2], [0.0, -k]),
            v(x0 + r, y0, [-k, 0.0], [0.0; 2]),
            v(x1 - r, y0, [0.0; 2], [k, 0.0]),
        ],
        closed: true,
    }
}

/// An ellipse of `size` centred at `centre`: four curves from the top, clockwise.
pub fn ellipse(size: [f64; 2], centre: [f64; 2]) -> Sub {
    let (rx, ry) = (size[0].abs() / 2.0, size[1].abs() / 2.0);
    let k = arc_handle(std::f64::consts::FRAC_PI_2);
    let (kx, ky) = (rx * k, ry * k);
    let [cx, cy] = centre;
    Sub {
        v: vec![
            Vertex { p: [cx, cy - ry], i: [-kx, 0.0], o: [kx, 0.0] },
            Vertex { p: [cx + rx, cy], i: [0.0, -ky], o: [0.0, ky] },
            Vertex { p: [cx, cy + ry], i: [kx, 0.0], o: [-kx, 0.0] },
            Vertex { p: [cx - rx, cy], i: [0.0, ky], o: [0.0, -ky] },
        ],
        closed: true,
    }
}

/// What a polystar is.
#[derive(Clone, Copy, Debug)]
pub struct Polystar {
    /// A star (points in and out) or a polygon.
    pub star: bool,
    /// How many points (a fraction makes the last one part-way).
    pub points: f64,
    pub centre: [f64; 2],
    /// Turned clockwise, degrees; the first point is straight up.
    pub rotation: f64,
    pub inner_radius: f64,
    pub outer_radius: f64,
    /// How round the inner and outer points are, in %: at 100 a polygon is a circle.
    pub inner_roundness: f64,
    pub outer_roundness: f64,
}

/// A star or polygon, from its top point, clockwise.
pub fn polystar(s: &Polystar) -> Sub {
    let n = s.points.max(1.0);
    let whole = n.floor() as usize;
    let count = if s.star { whole * 2 } else { whole };
    if count < 2 {
        return Sub::default();
    }
    let step = std::f64::consts::TAU / if s.star { n * 2.0 } else { n };
    let mut v = Vec::with_capacity(count);
    for k in 0..count {
        let outer = !s.star || k % 2 == 0;
        let (r, round) = if outer { (s.outer_radius, s.outer_roundness) } else { (s.inner_radius, s.inner_roundness) };
        let a = (s.rotation - 90.0).to_radians() + step * k as f64;
        let p = [s.centre[0] + r * a.cos(), s.centre[1] + r * a.sin()];
        // round points get handles along the circle through them
        let h = r * arc_handle(step) * round / 100.0;
        let tangent = [-a.sin() * h, a.cos() * h];
        v.push(Vertex { p, i: [-tangent[0], -tangent[1]], o: tangent });
    }
    Sub { v, closed: true }
}

/// Of each run, or of all of them in a row (`one_by_one`), the part from `start` to `end` (in
/// %, either way round), turned on by `offset` degrees (360 is the whole length). A closed run
/// is followed round past its start.
pub fn trim(shapes: &[Sub], start: f64, end: f64, offset: f64, one_by_one: bool) -> Vec<Sub> {
    trim_indexed(shapes, start, end, offset, one_by_one).into_iter().map(|(_, s)| s).collect()
}

/// [`trim`], each piece with the place in `shapes` of the run it came from.
pub fn trim_indexed(shapes: &[Sub], start: f64, end: f64, offset: f64, one_by_one: bool) -> Vec<(usize, Sub)> {
    let (mut a, mut b) = ((start / 100.0).clamp(0.0, 1.0), (end / 100.0).clamp(0.0, 1.0));
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    if b - a >= 1.0 && offset.rem_euclid(360.0) == 0.0 {
        return shapes.iter().cloned().enumerate().collect();
    }
    if b - a <= 0.0 {
        return Vec::new();
    }
    let o = offset / 360.0;
    let (a, b) = (a + o, b + o);
    // the part [a, b] of a loop of length 1, as pieces of [0, 1]
    let wrap = |a: f64, b: f64| -> Vec<(f64, f64)> {
        let shift = a.floor();
        let (a, b) = (a - shift, b - shift);
        if b <= 1.0 { vec![(a, b)] } else { vec![(a, 1.0), (0.0, b - 1.0)] }
    };
    let mut out = Vec::new();
    if !one_by_one {
        for (i, s) in shapes.iter().enumerate() {
            let len = s.length();
            let pieces = wrap(a, b);
            if s.closed && pieces.len() == 2 {
                // round past the start: one piece
                let mut first = s.cut(pieces[0].0 * len, len);
                join(&mut first, s.cut(0.0, pieces[1].1 * len));
                out.push((i, first));
            } else {
                out.extend(pieces.into_iter().map(|(x, y)| (i, s.cut(x * len, y * len))));
            }
        }
    } else {
        let lens: Vec<f64> = shapes.iter().map(Sub::length).collect();
        let total: f64 = lens.iter().sum();
        for (x, y) in wrap(a, b) {
            let (from, to) = (x * total, y * total);
            let mut at = 0.0;
            for (i, (s, len)) in shapes.iter().zip(&lens).enumerate() {
                let (s0, s1) = (at, at + len);
                at = s1;
                if s1 <= from || s0 >= to {
                    continue;
                }
                out.push((i, s.cut((from - s0).max(0.0), (to - s0).min(*len))));
            }
        }
    }
    out.retain(|(_, s)| s.v.len() > 1);
    out
}

/// Add `next` to the end of `run` (it starts where `run` ends).
fn join(run: &mut Sub, next: Sub) {
    let mut it = next.v.into_iter();
    if let (Some(first), Some(last)) = (it.next(), run.v.last_mut()) {
        last.o = first.o;
    }
    run.v.extend(it);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn lengths_of_lines_and_circles() {
        assert!(close(Sub::polygon(&[[0.0, 0.0], [30.0, 40.0]], false).length(), 50.0, 1e-9));
        assert!(close(rect([10.0, 20.0], [0.0, 0.0], 0.0).length(), 60.0, 1e-9));
        let circle = ellipse([200.0, 200.0], [0.0, 0.0]);
        // four cubics make a circle to within about 0.03 % of its radius
        assert!(close(circle.length(), std::f64::consts::TAU * 100.0, 0.1), "{}", circle.length());
    }

    #[test]
    fn rect_and_ellipse_start_where_they_should() {
        let r = rect([100.0, 50.0], [0.0, 0.0], 0.0);
        assert_eq!(r.v[0].p, [50.0, -25.0], "top right");
        assert_eq!(r.v[1].p, [50.0, 25.0], "then down: clockwise");
        let e = ellipse([100.0, 50.0], [10.0, 0.0]);
        assert_eq!(e.v[0].p, [10.0, -25.0], "the top");
        let round = rect([100.0, 50.0], [0.0, 0.0], 80.0);
        assert_eq!(round.v.len(), 8);
        assert!(close(round.v[0].p[1], 0.0, 1e-9), "the roundness is held to half the shorter side");
    }

    #[test]
    fn polystars() {
        let s = polystar(&Polystar { star: true, points: 5.0, centre: [0.0, 0.0], rotation: 0.0, inner_radius: 50.0, outer_radius: 100.0, inner_roundness: 0.0, outer_roundness: 0.0 });
        assert_eq!(s.v.len(), 10);
        assert!(close(s.v[0].p[0], 0.0, 1e-9) && close(s.v[0].p[1], -100.0, 1e-9), "the first point straight up");
        let round = polystar(&Polystar { star: false, points: 6.0, centre: [0.0, 0.0], rotation: 0.0, inner_radius: 0.0, outer_radius: 100.0, inner_roundness: 0.0, outer_roundness: 100.0 });
        // at 100 % a polygon is a circle
        for k in 0..6 {
            let c = round.segment(k);
            for t in [0.25, 0.5, 0.75] {
                let p = cubic_at(&c, t);
                assert!(close(p[0].hypot(p[1]), 100.0, 0.05), "{p:?}");
            }
        }
    }

    #[test]
    fn cutting_by_length() {
        let line = Sub::polygon(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]], false);
        let part = line.cut(50.0, 150.0);
        assert_eq!(part.v.iter().map(|v| v.p).collect::<Vec<_>>(), vec![[50.0, 0.0], [100.0, 0.0], [100.0, 50.0]]);
        let circle = ellipse([200.0, 200.0], [0.0, 0.0]);
        let quarter = circle.cut(0.0, circle.length() / 4.0);
        let end = quarter.v.last().unwrap().p;
        assert!(close(end[0], 100.0, 0.01) && close(end[1], 0.0, 0.01), "a quarter round from the top is at the right: {end:?}");
    }

    #[test]
    fn trimming() {
        let square = rect([100.0, 100.0], [0.0, 0.0], 0.0);
        let half = trim(std::slice::from_ref(&square), 0.0, 50.0, 0.0, false);
        assert_eq!(half.len(), 1);
        assert!(close(half[0].length(), 200.0, 1e-6));
        // turned by a quarter turn, the half starts a side later
        let turned = trim(std::slice::from_ref(&square), 0.0, 50.0, 90.0, false);
        assert_eq!(turned[0].v[0].p, [50.0, 50.0]);
        // past the start of a closed run, one piece
        let round = trim(std::slice::from_ref(&square), 75.0, 100.0, 90.0, false);
        assert_eq!(round.len(), 1);
        assert!(close(round[0].length(), 100.0, 1e-6));
        // either way round, and nothing between equal ends
        assert!(close(trim(std::slice::from_ref(&square), 60.0, 10.0, 0.0, false)[0].length(), 200.0, 1e-6));
        assert!(trim(std::slice::from_ref(&square), 30.0, 30.0, 0.0, false).is_empty());
        // one by one: two lines as one length
        let two = [Sub::polygon(&[[0.0, 0.0], [100.0, 0.0]], false), Sub::polygon(&[[0.0, 10.0], [100.0, 10.0]], false)];
        let first = trim(&two, 0.0, 50.0, 0.0, true);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].v.last().unwrap().p, [100.0, 0.0]);
        let each = trim(&two, 0.0, 50.0, 0.0, false);
        assert_eq!(each.len(), 2, "simultaneously: half of each");
    }

    #[test]
    fn reversed_and_moved() {
        let tri = Sub::polygon(&[[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]], true);
        let r = tri.reversed();
        assert_eq!(r.v.iter().map(|v| v.p).collect::<Vec<_>>(), vec![[0.0, 0.0], [0.0, 10.0], [10.0, 0.0]]);
        let e = ellipse([10.0, 10.0], [0.0, 0.0]).map(&|p| [p[0] * 2.0 + 5.0, p[1]]);
        assert_eq!(e.v[1].p, [15.0, 0.0]);
        assert_eq!(e.v[0].o, [2.0 * 5.0 * arc_handle(std::f64::consts::FRAC_PI_2), 0.0]);
    }
}
