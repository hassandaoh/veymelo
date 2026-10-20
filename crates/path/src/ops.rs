//! What can be done to shapes before they are painted: round their corners, grow or shrink
//! them, pucker or bloat them, twist them, zig-zag their edges, wiggle them over time.

use crate::shape::{Cubic, Sub, Vertex, arc_handle, cubic_at, cubic_direction, cubic_length, t_at_length};
use crate::Path;
use crate::stroke::{Cap, Join, Stroke, stroke};

fn add(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn scale(a: [f64; 2], k: f64) -> [f64; 2] {
    [a[0] * k, a[1] * k]
}

fn len(a: [f64; 2]) -> f64 {
    a[0].hypot(a[1])
}

fn unit(a: [f64; 2]) -> [f64; 2] {
    let l = len(a);
    if l > 0.0 { scale(a, 1.0 / l) } else { [0.0; 2] }
}

/// Round every sharp corner (a point with no tangents) with a circle of `radius`, as big as the
/// edges beside it allow.
pub fn round_corners(shapes: &[Sub], radius: f64) -> Vec<Sub> {
    if radius <= 0.0 {
        return shapes.to_vec();
    }
    shapes
        .iter()
        .map(|s| {
            let n = s.v.len();
            let mut out = Vec::with_capacity(n * 2);
            for k in 0..n {
                let v = s.v[k];
                let end = !s.closed && (k == 0 || k + 1 == n);
                if end || v.i != [0.0; 2] || v.o != [0.0; 2] || n < 3 {
                    out.push(v);
                    continue;
                }
                // the directions the edges leave the corner in, toward their next control point
                let prev = &s.v[(k + n - 1) % n];
                let next = &s.v[(k + 1) % n];
                let toward_prev = if prev.o != [0.0; 2] { add(prev.p, prev.o) } else { prev.p };
                let toward_next = if next.i != [0.0; 2] { add(next.p, next.i) } else { next.p };
                let (u1, u2) = (unit(sub(toward_prev, v.p)), unit(sub(toward_next, v.p)));
                let cos = (u1[0] * u2[0] + u1[1] * u2[1]).clamp(-1.0, 1.0);
                let corner = cos.acos();
                if corner < 1e-6 || (std::f64::consts::PI - corner) < 1e-6 {
                    out.push(v);
                    continue;
                }
                // how far along each edge the circle touches, held to half the shorter edge
                let room = len(sub(prev.p, v.p)).min(len(sub(next.p, v.p))) / 2.0;
                let d = (radius / (corner / 2.0).tan()).min(room);
                let r = d * (corner / 2.0).tan();
                let h = arc_handle(std::f64::consts::PI - corner) * r;
                let (p1, p2) = (add(v.p, scale(u1, d)), add(v.p, scale(u2, d)));
                out.push(Vertex { p: p1, i: [0.0; 2], o: scale(u1, -h) });
                out.push(Vertex { p: p2, i: scale(u2, -h), o: [0.0; 2] });
            }
            Sub { v: out, closed: s.closed }
        })
        .collect()
}

/// Each run flattened into its points.
fn points(s: &Sub) -> Vec<[f64; 2]> {
    let mut p = Path::default();
    s.flatten_into(&mut p);
    let mut pts: Vec<[f64; 2]> = p.contours().first().map(|c| c.iter().map(|q| [q.0, q.1]).collect()).unwrap_or_default();
    if s.closed && pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    pts.dedup_by(|a, b| len(sub(*a, *b)) < 1e-9);
    pts
}

fn area(pts: &[[f64; 2]]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| pts[i][0] * pts[(i + 1) % n][1] - pts[(i + 1) % n][0] * pts[i][1]).sum::<f64>() / 2.0
}

/// Grow (or below zero, shrink) closed shapes by `amount`, their corners joined by `join`;
/// `copies` of them, each a further `amount` out, starting `copy_offset` amounts further. An
/// open run becomes the outline round it.
pub fn offset(shapes: &[Sub], amount: f64, join: Join, miter_limit: f64, copies: f64, copy_offset: f64) -> Vec<Sub> {
    let copies = copies.max(0.0).floor() as usize;
    let mut out = Vec::new();
    for c in 0..copies.max(1) {
        let d = amount * (c as f64 + 1.0 + copy_offset);
        for s in shapes {
            if !s.closed {
                let outline = stroke(std::slice::from_ref(s), &Stroke { width: 2.0 * d.abs(), cap: Cap::Butt, join, miter_limit, ..Stroke::default() });
                out.extend(outline.contours().iter().map(|c| Sub::polygon(&c.iter().map(|p| [p.0, p.1]).collect::<Vec<_>>(), true)));
                continue;
            }
            let pts = points(s);
            if pts.len() < 3 || d == 0.0 {
                out.push(s.clone());
                continue;
            }
            out.push(Sub::polygon(&offset_polygon(&pts, d, join, miter_limit), true));
        }
    }
    out
}

/// A polygon grown by `d` (out for d above zero).
fn offset_polygon(pts: &[[f64; 2]], d: f64, join: Join, miter_limit: f64) -> Vec<[f64; 2]> {
    let n = pts.len();
    // the outward side: left of the edges when the polygon winds the negative way
    let out_side = if area(pts) > 0.0 { -1.0 } else { 1.0 };
    let normal = |k: usize| {
        let e = unit(sub(pts[(k + 1) % n], pts[k]));
        scale([-e[1], e[0]], out_side * d)
    };
    let mut res = Vec::with_capacity(n * 2);
    for k in 0..n {
        let (n0, n1) = (normal((k + n - 1) % n), normal(k));
        let v = pts[k];
        let (a, b) = (add(v, n0), add(v, n1));
        let (e0, e1) = (unit(sub(v, pts[(k + n - 1) % n])), unit(sub(pts[(k + 1) % n], v)));
        let cross = e0[0] * e1[1] - e0[1] * e1[0];
        // a corner the offset goes round (convex for growing): joined; else where the edges meet
        let goes_round = cross * out_side * d.signum() < 0.0;
        let turn = cross.atan2(e0[0] * e1[0] + e0[1] * e1[1]).abs();
        if turn < 1e-6 {
            res.push(b);
            continue;
        }
        if !goes_round {
            res.push(meet(a, e0, b, e1).unwrap_or(b));
            continue;
        }
        match join {
            Join::Bevel => {
                res.push(a);
                res.push(b);
            }
            Join::Miter => {
                let reach = 1.0 / (turn / 2.0).cos();
                if reach <= miter_limit {
                    res.push(meet(a, e0, b, e1).unwrap_or(b));
                } else {
                    res.push(a);
                    res.push(b);
                }
            }
            Join::Round => {
                let steps = ((turn / 0.2).ceil() as usize).max(2);
                let (s0, s1) = (n0[1].atan2(n0[0]), n1[1].atan2(n1[0]));
                let mut sweep = s1 - s0;
                while sweep > std::f64::consts::PI {
                    sweep -= std::f64::consts::TAU;
                }
                while sweep < -std::f64::consts::PI {
                    sweep += std::f64::consts::TAU;
                }
                for i in 0..=steps {
                    let ang = s0 + sweep * i as f64 / steps as f64;
                    res.push([v[0] + d.abs() * ang.cos(), v[1] + d.abs() * ang.sin()]);
                }
            }
        }
    }
    res
}

/// Where the line through `a` along `da` meets the line through `b` along `db`.
fn meet(a: [f64; 2], da: [f64; 2], b: [f64; 2], db: [f64; 2]) -> Option<[f64; 2]> {
    let den = da[0] * db[1] - da[1] * db[0];
    if den.abs() < 1e-12 {
        return None;
    }
    let t = ((b[0] - a[0]) * db[1] - (b[1] - a[1]) * db[0]) / den;
    Some(add(a, scale(da, t)))
}

/// Pull points toward each run's centre and push their tangents away (`amount` above zero,
/// in %: bloat), or the other way (below zero: pucker).
pub fn pucker_bloat(shapes: &[Sub], amount: f64) -> Vec<Sub> {
    let k = amount / 100.0;
    shapes
        .iter()
        .map(|s| {
            let c = s.centre();
            let toward = |p: [f64; 2], f: f64| add(p, scale(sub(c, p), f));
            let v = s
                .v
                .iter()
                .map(|x| {
                    let p = toward(x.p, k);
                    Vertex { p, i: sub(toward(add(x.p, x.i), -k), p), o: sub(toward(add(x.p, x.o), -k), p) }
                })
                .collect();
            Sub { v, closed: s.closed }
        })
        .collect()
}

/// Turn shapes about `centre` by `angle` degrees at the centre, less and less out to their
/// farthest point (which does not turn).
pub fn twist(shapes: &[Sub], angle: f64, centre: [f64; 2]) -> Vec<Sub> {
    if angle == 0.0 {
        return shapes.to_vec();
    }
    // fine enough to bend: no straight piece longer than two pixels
    let dense: Vec<(Vec<[f64; 2]>, bool)> = shapes
        .iter()
        .map(|s| {
            let pts = points(s);
            let n = pts.len();
            let mut out = Vec::new();
            let segs = if s.closed { n } else { n.saturating_sub(1) };
            for k in 0..segs {
                let (a, b) = (pts[k], pts[(k + 1) % n]);
                let steps = ((len(sub(b, a)) / 2.0).ceil() as usize).clamp(1, 400);
                for j in 0..steps {
                    out.push(add(a, scale(sub(b, a), j as f64 / steps as f64)));
                }
            }
            if !s.closed && let Some(&l) = pts.last() {
                out.push(l);
            }
            (out, s.closed)
        })
        .collect();
    let far = dense.iter().flat_map(|(p, _)| p.iter()).map(|p| len(sub(*p, centre))).fold(0.0, f64::max);
    if far <= 0.0 {
        return shapes.to_vec();
    }
    dense
        .into_iter()
        .map(|(pts, closed)| {
            let turned: Vec<[f64; 2]> = pts
                .iter()
                .map(|p| {
                    let r = sub(*p, centre);
                    let a = (angle * (1.0 - len(r) / far)).to_radians();
                    let (s, c) = a.sin_cos();
                    add(centre, [r[0] * c - r[1] * s, r[0] * s + r[1] * c])
                })
                .collect();
            Sub::polygon(&turned, closed)
        })
        .collect()
}

/// Points along each curve at even lengths: `per` pieces to a curve. Each with its position and
/// direction.
fn along(s: &Sub, per: usize) -> Vec<([f64; 2], [f64; 2], f64)> {
    let mut out = Vec::new();
    for k in 0..s.segments() {
        let c: Cubic = s.segment(k);
        let l = cubic_length(&c, 0.0, 1.0);
        for j in 0..per {
            let t = t_at_length(&c, l * j as f64 / per as f64);
            out.push((cubic_at(&c, t), cubic_direction(&c, t), l / per as f64));
        }
    }
    if !s.closed && let Some(last) = s.v.last() {
        let c = s.segment(s.segments() - 1);
        out.push((last.p, cubic_direction(&c, 1.0), 0.0));
    }
    out
}

/// Zig-zag the edges: `ridges` ridges to each curve, out and in by `size`, sharp or `smooth`.
pub fn zigzag(shapes: &[Sub], size: f64, ridges: f64, smooth: bool) -> Vec<Sub> {
    let per = ridges.max(0.0).round() as usize + 1;
    shapes
        .iter()
        .filter(|s| s.segments() > 0)
        .map(|s| {
            let pts = along(s, per);
            let v = pts
                .iter()
                .enumerate()
                .map(|(j, (p, d, step))| {
                    let side = if j % 2 == 0 { 1.0 } else { -1.0 };
                    let q = add(*p, scale([-d[1], d[0]], size * side));
                    if smooth {
                        let h = scale(*d, step / 3.0);
                        Vertex { p: q, i: scale(h, -1.0), o: h }
                    } else {
                        Vertex { p: q, i: [0.0; 2], o: [0.0; 2] }
                    }
                })
                .collect();
            Sub { v, closed: s.closed }
        })
        .collect()
}

/// How Wiggle Paths moves points.
#[derive(Clone, Copy, Debug)]
pub struct Wiggle {
    /// How far a point moves, at most.
    pub size: f64,
    /// Points to each curve.
    pub detail: f64,
    pub smooth: bool,
    /// Wiggles a second.
    pub speed: f64,
    /// How alike the points move, in % (100: all together).
    pub correlation: f64,
    /// Where in its wiggling it starts, degrees.
    pub phase: f64,
    pub seed: f64,
}

fn hash(a: u64, b: u64, c: u64) -> f64 {
    let mut h = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ c.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 29;
    (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// A smooth wander from -1 to 1 for point `j` at time `t` (in wiggles), the same every time.
fn wander(seed: u64, j: u64, t: f64) -> [f64; 2] {
    let (k, f) = (t.floor(), t - t.floor());
    let s = f * f * (3.0 - 2.0 * f);
    let at = |i: f64, axis: u64| hash(seed, j * 2 + axis, i as i64 as u64);
    [at(k, 0) + (at(k + 1.0, 0) - at(k, 0)) * s, at(k, 1) + (at(k + 1.0, 1) - at(k, 1)) * s]
}

/// Move points about over time (`time` in seconds), each its own way or, with correlation, more
/// alike.
pub fn wiggle(shapes: &[Sub], w: &Wiggle, time: f64) -> Vec<Sub> {
    let per = w.detail.max(0.0).round() as usize + 1;
    let t = time * w.speed + w.phase / 360.0;
    let seed = w.seed.to_bits() ^ 0x5DEE_CE66;
    let c = (w.correlation / 100.0).clamp(0.0, 1.0);
    let common = wander(seed, u32::MAX as u64, t);
    shapes
        .iter()
        .filter(|s| s.segments() > 0)
        .map(|s| {
            let pts = along(s, per);
            let v = pts
                .iter()
                .enumerate()
                .map(|(j, (p, d, step))| {
                    let own = wander(seed, j as u64, t);
                    let m = [common[0] * c + own[0] * (1.0 - c), common[1] * c + own[1] * (1.0 - c)];
                    let q = add(*p, scale(m, w.size));
                    if w.smooth {
                        let h = scale(*d, step / 3.0);
                        Vertex { p: q, i: scale(h, -1.0), o: h }
                    } else {
                        Vertex { p: q, i: [0.0; 2], o: [0.0; 2] }
                    }
                })
                .collect();
            Sub { v, closed: s.closed }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fill::{Rule, fill};
    use crate::shape::{flatten, rect};

    fn area(shapes: &[Sub]) -> f32 {
        fill(&flatten(shapes), Rule::NonZero, 600, 600).map_or(0.0, |m| m.cover.iter().sum())
    }

    #[test]
    fn rounded_corners_take_off_the_corners() {
        let sq = [rect([200.0, 200.0], [300.0, 300.0], 0.0)];
        let r = round_corners(&sq, 20.0);
        assert_eq!(r[0].v.len(), 8);
        let want = 200.0 * 200.0 - (4.0 - std::f32::consts::PI) * 400.0;
        assert!((area(&r) - want).abs() < 2.0, "{} vs {want}", area(&r));
        // the radius is held to half the shortest edge
        let big = round_corners(&sq, 500.0);
        assert!((area(&big) - std::f32::consts::PI * 10000.0).abs() < 40.0, "a circle: {}", area(&big));
    }

    #[test]
    fn offsets_grow_and_shrink() {
        let sq = [rect([200.0, 200.0], [300.0, 300.0], 0.0)];
        assert!((area(&offset(&sq, 10.0, Join::Miter, 4.0, 1.0, 0.0)) - 220.0 * 220.0).abs() < 2.0);
        assert!((area(&offset(&sq, -10.0, Join::Miter, 4.0, 1.0, 0.0)) - 180.0 * 180.0).abs() < 2.0);
        let rounded = area(&offset(&sq, 10.0, Join::Round, 4.0, 1.0, 0.0));
        let want = 200.0 * 200.0 + 4.0 * 200.0 * 10.0 + std::f32::consts::PI * 100.0;
        assert!((rounded - want).abs() < 6.0, "{rounded} vs {want}");
        // the other way round makes no difference
        let back = [sq[0].reversed()];
        assert!((area(&offset(&back, 10.0, Join::Miter, 4.0, 1.0, 0.0)) - 220.0 * 220.0).abs() < 2.0);
        assert_eq!(offset(&sq, 10.0, Join::Miter, 4.0, 3.0, 0.0).len(), 3);
    }

    #[test]
    fn bloat_and_pucker() {
        let sq = [rect([200.0, 200.0], [300.0, 300.0], 0.0)];
        let bloated = pucker_bloat(&sq, 50.0);
        assert_eq!(bloated[0].v[0].p, [350.0, 250.0], "the corners go halfway in");
        assert!(bloated[0].v[0].o != [0.0; 2], "and the edges bulge out");
        assert!(area(&pucker_bloat(&sq, -50.0)) < area(&sq));
    }

    #[test]
    fn twist_keeps_the_far_points_and_turns_the_rest() {
        let line = [Sub::polygon(&[[300.0, 300.0], [400.0, 300.0]], false)];
        let t = twist(&line, 90.0, [300.0, 300.0]);
        let first = t[0].v[0].p;
        let last = t[0].v.last().unwrap().p;
        assert!((first[0] - 300.0).abs() < 1e-9 && (last[0] - 400.0).abs() < 1e-9, "the centre and the farthest point stay");
        let mid = t[0].v[t[0].v.len() / 2].p;
        assert!(mid[1] > 300.0 + 10.0, "the middle turned: {mid:?}");
    }

    #[test]
    fn zigzag_ridges() {
        let line = [Sub::polygon(&[[0.0, 0.0], [100.0, 0.0]], false)];
        let z = zigzag(&line, 10.0, 4.0, false);
        assert_eq!(z[0].v.len(), 6, "five pieces: six points");
        assert_eq!(z[0].v[0].p, [0.0, 10.0]);
        let p = z[0].v[1].p;
        assert!((p[0] - 20.0).abs() < 1e-9 && (p[1] + 10.0).abs() < 1e-9, "{p:?}");
    }

    #[test]
    fn wiggles_move_and_stay_the_same_every_time() {
        let sq = [rect([200.0, 200.0], [300.0, 300.0], 0.0)];
        let w = Wiggle { size: 10.0, detail: 3.0, smooth: true, speed: 2.0, correlation: 50.0, phase: 0.0, seed: 1.0 };
        let (a, b) = (wiggle(&sq, &w, 0.3), wiggle(&sq, &w, 0.3));
        assert_eq!(a, b);
        assert_ne!(a, wiggle(&sq, &w, 0.7), "it moves over time");
        let moved = a[0].v.iter().map(|v| v.p).zip(along(&sq[0], 4).iter().map(|x| x.0)).map(|(p, q)| len(sub(p, q))).fold(0.0, f64::max);
        assert!(moved > 0.0 && moved <= 10.0 * std::f64::consts::SQRT_2 + 1e-9, "{moved}");
    }
}
