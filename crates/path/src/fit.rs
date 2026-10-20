//! A cubic fitted to points by least squares, its end directions given: the handle lengths
//! solved from the normal equations, then each point's parameter refined by Newton steps
//! toward the curve while the fit improves (the method Graphics Gems describes).

use crate::bezier::{P, add, at, dist, mul, sub};

const ROUNDS: usize = 6;

fn chord_params(pts: &[P]) -> Vec<f64> {
    let mut u = vec![0.0];
    let mut acc = 0.0;
    for w in pts.windows(2) {
        acc += dist(w[0], w[1]);
        u.push(acc);
    }
    if acc > 0.0 {
        u.iter_mut().for_each(|x| *x /= acc);
    }
    u
}

fn bernstein(t: f64) -> [f64; 4] {
    let s = 1.0 - t;
    [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t]
}

fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

/// The best cubic for `pts` at parameters `u`, leaving along `t0` and arriving along `t1`.
fn solve(pts: &[P], u: &[f64], t0: P, t1: P) -> [P; 4] {
    let (p0, p3) = (pts[0], pts[pts.len() - 1]);
    let back = mul(t1, -1.0);
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (p, &t) in pts.iter().zip(u) {
        let b = bernstein(t);
        let (a0, a1) = (mul(t0, b[1]), mul(back, b[2]));
        c00 += dot(a0, a0);
        c01 += dot(a0, a1);
        c11 += dot(a1, a1);
        let rest = sub(*p, add(mul(p0, b[0] + b[1]), mul(p3, b[2] + b[3])));
        x0 += dot(a0, rest);
        x1 += dot(a1, rest);
    }
    let det = c00 * c11 - c01 * c01;
    let seg = dist(p0, p3);
    let (mut l, mut r) = if det.abs() > 1e-12 { ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det) } else { (seg / 3.0, seg / 3.0) };
    if !l.is_finite() || !r.is_finite() || l < 1e-6 * seg || r < 1e-6 * seg {
        (l, r) = (seg / 3.0, seg / 3.0);
    }
    // no wild handles on nearly straight data
    let cap = seg * 4.0 + 1e-9;
    [p0, add(p0, mul(t0, l.min(cap))), add(p3, mul(back, r.min(cap))), p3]
}

fn worst(pts: &[P], c: &[P; 4], u: &[f64]) -> f64 {
    pts.iter().zip(u).map(|(p, &t)| dist(at(c, t), *p)).fold(0.0, f64::max)
}

/// The parameter on `c` nearer to `p` than `t`, by one Newton step.
fn nearer(c: &[P; 4], p: P, t: f64) -> f64 {
    let d1 = [mul(sub(c[1], c[0]), 3.0), mul(sub(c[2], c[1]), 3.0), mul(sub(c[3], c[2]), 3.0)];
    let d2 = [mul(sub(d1[1], d1[0]), 2.0), mul(sub(d1[2], d1[1]), 2.0)];
    let s = 1.0 - t;
    let q1 = add(add(mul(d1[0], s * s), mul(d1[1], 2.0 * s * t)), mul(d1[2], t * t));
    let q2 = add(mul(d2[0], s), mul(d2[1], t));
    let q = sub(at(c, t), p);
    let den = dot(q1, q1) + dot(q, q2);
    if den.abs() < 1e-12 {
        return t;
    }
    let nt = t - dot(q, q1) / den;
    if nt.is_finite() { nt.clamp(0.0, 1.0) } else { t }
}

/// The cubic through the first and last of `pts` that best follows the rest, leaving along
/// `t0` and arriving along `t1` (unit directions, both pointing the way the curve goes).
pub fn fit(pts: &[P], t0: P, t1: P) -> [P; 4] {
    let mut u = chord_params(pts);
    let mut best = solve(pts, &u, t0, t1);
    let mut err = worst(pts, &best, &u);
    for _ in 0..ROUNDS {
        for (i, ui) in u.iter_mut().enumerate() {
            *ui = nearer(&best, pts[i], *ui);
        }
        let c = solve(pts, &u, t0, t1);
        let e = worst(pts, &c, &u);
        if e < err {
            (best, err) = (c, e);
        } else {
            break;
        }
    }
    best
}
