//! Smart mask interpolation: two outlines given the same number of vertices, matched vertex to
//! vertex, and the shapes between them.
//!
//! Vertices are added by cutting the outlines' pieces (so neither outline changes), spread by
//! length so the n-th vertex of each sits as far round its outline. The second outline's
//! starting vertex and direction are chosen so matched vertices sit alike, measured with both
//! shapes moved to their centres and sized alike (unless the first vertices are to match).
//! Between them, each vertex moves with the turn, growth and travel that best take the first
//! shape onto the second (Procrustes), so a shape that turns, turns instead of folding through
//! itself; what is left over goes straight. Bending resistance weighs that whole-shape motion
//! against plain straight lines.

use veymelo_keyframe::ShapePath;

use crate::bezier::{P, add, length, lerp, reversed, starting_at, sub, with_vertices};

/// How shapes are matched and moved between.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Morph {
    /// Vertices move in straight lines.
    pub linear: bool,
    /// 0 to 100: how much the shape keeps itself, turning and growing as a whole.
    pub bending_resistance: f64,
    /// Pixels between vertices once they are added (none: only as many as the larger has).
    pub spacing: Option<f64>,
    /// The two first vertices match.
    pub first_vertices_match: bool,
    /// Outlines with as many vertices as each other are matched as they are.
    pub one_to_one: bool,
}

impl Default for Morph {
    fn default() -> Morph {
        Morph { linear: false, bending_resistance: 50.0, spacing: Some(15.0), first_vertices_match: false, one_to_one: false }
    }
}

/// The middle of points and how far they spread from it.
fn frame(v: &[P]) -> (P, f64) {
    let n = v.len().max(1) as f64;
    let c = [v.iter().map(|p| p[0]).sum::<f64>() / n, v.iter().map(|p| p[1]).sum::<f64>() / n];
    let spread = (v.iter().map(|p| (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sum::<f64>() / n).sqrt().max(1e-9);
    (c, spread)
}

/// How unlike two point lists are with the second started at `k`, each about its middle and
/// at its own size.
fn cost(a: &[P], b: &[P], k: usize) -> f64 {
    let ((ca, ra), (cb, rb)) = (frame(a), frame(b));
    let n = a.len();
    (0..n)
        .map(|i| {
            let p = [(a[i][0] - ca[0]) / ra, (a[i][1] - ca[1]) / ra];
            let q = b[(i + k) % n];
            let q = [(q[0] - cb[0]) / rb, (q[1] - cb[1]) / rb];
            (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)
        })
        .sum()
}

/// The two outlines with as many vertices as each other, vertex `i` of one matched to vertex
/// `i` of the other.
pub fn correspond(a: &ShapePath, b: &ShapePath, m: &Morph) -> (ShapePath, ShapePath) {
    if a.points.is_empty() || b.points.is_empty() {
        return (a.clone(), b.clone());
    }
    if m.one_to_one && a.points.len() == b.points.len() && a.closed == b.closed {
        return (a.clone(), b.clone());
    }
    let mut n = a.points.len().max(b.points.len());
    if let Some(s) = m.spacing.filter(|s| *s > 0.5) {
        n = n.max((length(a).max(length(b)) / s).round() as usize).min(1000);
    }
    let (aa, bb) = (with_vertices(a, n), with_vertices(b, n));
    // the start and direction of the second that match best
    let mut best = (f64::INFINITY, bb.clone());
    for flip in [false, true] {
        let cand = if flip { reversed(&bb) } else { bb.clone() };
        let ks: Vec<usize> = if aa.closed && cand.closed && !m.first_vertices_match { (0..n).collect() } else { vec![0] };
        for k in ks {
            let c = cost(&aa.points, &cand.points, k);
            if c < best.0 - 1e-12 {
                best = (c, starting_at(&cand, k));
            }
        }
    }
    (aa, best.1)
}

/// The turn (radians) and growth that best take points `a` onto `b`, about their middles.
fn procrustes(a: &[P], b: &[P]) -> (f64, f64) {
    let ((ca, ra), (cb, rb)) = (frame(a), frame(b));
    let (mut dot, mut cross) = (0.0, 0.0);
    for (p, q) in a.iter().zip(b) {
        let (p, q) = (sub(*p, ca), sub(*q, cb));
        dot += p[0] * q[0] + p[1] * q[1];
        cross += p[0] * q[1] - p[1] * q[0];
    }
    (cross.atan2(dot), rb / ra)
}

fn turn(v: P, angle: f64, k: f64) -> P {
    let (s, c) = angle.sin_cos();
    [(v[0] * c - v[1] * s) * k, (v[0] * s + v[1] * c) * k]
}

/// The shape `u` of the way (0 to 1) from `a` to `b`, which [`correspond`] matched.
pub fn between(a: &ShapePath, b: &ShapePath, u: f64, m: &Morph) -> ShapePath {
    let straight = |pa: &[P], pb: &[P]| -> Vec<P> { pa.iter().zip(pb).map(|(p, q)| lerp(*p, *q, u)).collect() };
    let lines = ShapePath { points: straight(&a.points, &b.points), in_tangents: straight(&a.in_tangents, &b.in_tangents), out_tangents: straight(&a.out_tangents, &b.out_tangents), closed: a.closed, feather: veymelo_keyframe::Feathers::NONE };
    let w = (m.bending_resistance / 100.0).clamp(0.0, 1.0);
    if m.linear || w == 0.0 || a.points.len() != b.points.len() || a.points.len() < 2 {
        return lines;
    }
    let ((ca, _), (cb, _)) = (frame(&a.points), frame(&b.points));
    let (angle, growth) = procrustes(&a.points, &b.points);
    // the whole shape turned, grown and moved part way, and what is left over gone straight
    let rigid = |p: P, t: f64| add(lerp(ca, cb, t), turn(sub(p, ca), angle * t, growth.powf(t)));
    let rigid_v = |v: P, t: f64| turn(v, angle * t, growth.powf(t));
    let pts: Vec<P> = a.points.iter().zip(&b.points).map(|(p, q)| add(rigid(*p, u), sub(lerp([0.0; 2], *q, u), lerp([0.0; 2], rigid(*p, 1.0), u)))).collect();
    let tan = |ta: &[P], tb: &[P]| -> Vec<P> { ta.iter().zip(tb).map(|(p, q)| add(rigid_v(*p, u), sub(lerp([0.0; 2], *q, u), lerp([0.0; 2], rigid_v(*p, 1.0), u)))).collect() };
    let whole = ShapePath { points: pts, in_tangents: tan(&a.in_tangents, &b.in_tangents), out_tangents: tan(&a.out_tangents, &b.out_tangents), closed: a.closed, feather: veymelo_keyframe::Feathers::NONE };
    let mix = |x: &[P], y: &[P]| -> Vec<P> { x.iter().zip(y).map(|(p, q)| lerp(*q, *p, w)).collect() };
    ShapePath { points: mix(&whole.points, &lines.points), in_tangents: mix(&whole.in_tangents, &lines.in_tangents), out_tangents: mix(&whole.out_tangents, &lines.out_tangents), closed: a.closed, feather: veymelo_keyframe::Feathers::NONE }
}
