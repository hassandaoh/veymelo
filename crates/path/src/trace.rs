//! Auto-trace: the outlines of a field (a layer's alpha, a colour or its brightness) as
//! Bezier paths.
//!
//! 1. Marching squares over the samples at pixel centres, each crossing placed where the field
//!    passes the threshold along the cell's edge (a saddle settled by the cell's mean), on a
//!    field ringed with "outside" so every outline closes.
//! 2. Douglas–Peucker: each outline kept within the tolerance with as few vertices as it needs.
//! 3. A cubic fitted to the samples between each two kept vertices (least squares with the end
//!    directions fixed, its parameters refined by Newton steps, as Graphics Gems describes); a
//!    vertex's direction is the straight one at corner roundness 0 and the smooth one (from the
//!    vertex before to the one after) at 1, between them blended.
//!
//! Outlines smaller than the least area go; each says how many others it lies in (an odd
//! number: a hole).

use std::collections::HashMap;

use crate::bezier::{P, dist, signed_area, sub};

/// How a field is traced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trace {
    /// Inside where the field is at least this (0 to 1).
    pub threshold: f32,
    /// How far (pixels) the simplified outline may stray.
    pub tolerance: f64,
    /// Outlines enclosing less (square pixels) go.
    pub min_area: f64,
    /// 0: straight lines and sharp corners; 1: smooth all round.
    pub corner_roundness: f64,
}

impl Default for Trace {
    fn default() -> Trace {
        Trace { threshold: 0.5, tolerance: 1.0, min_area: 10.0, corner_roundness: 0.5 }
    }
}

/// An outline traced: its vertices and their tangents (relative to them), how many outlines
/// it lies in, and the area it encloses (above zero: clockwise on screen).
#[derive(Clone, Debug, PartialEq)]
pub struct Traced {
    pub points: Vec<P>,
    pub in_tangents: Vec<P>,
    pub out_tangents: Vec<P>,
    pub depth: usize,
    pub area: f64,
}

/// Trace `field` (`w` × `h`, a row at a time; sample (x, y) at the pixel's centre).
pub fn trace(field: &[f32], w: usize, h: usize, o: &Trace) -> Vec<Traced> {
    if w == 0 || h == 0 || field.len() < w * h {
        return Vec::new();
    }
    let loops: Vec<(Vec<P>, Vec<P>, f64)> = contours(field, w, h, o.threshold)
        .into_iter()
        .filter_map(|raw| {
            let a = signed_area(&raw);
            if a.abs() < o.min_area.max(0.0) || raw.len() < 3 {
                return None;
            }
            let simple: Vec<P> = simplified(&raw, o.tolerance.max(0.01)).into_iter().map(|i| raw[i]).collect();
            (simple.len() >= 3).then_some((raw, simple, a))
        })
        .collect();
    let depths: Vec<usize> = (0..loops.len()).map(|i| (0..loops.len()).filter(|&j| j != i && loops[j].2.abs() > loops[i].2.abs() && inside(loops[i].1[0], &loops[j].1)).count()).collect();
    let mut out: Vec<Traced> = loops
        .into_iter()
        .zip(depths)
        .map(|((raw, simple, area), depth)| {
            let (points, in_tangents, out_tangents) = fitted(&raw, &simple, o.corner_roundness.clamp(0.0, 1.0));
            Traced { points, in_tangents, out_tangents, depth, area }
        })
        .collect();
    // outlines first, then holes, islands in them…; the biggest first
    out.sort_by(|a, b| a.depth.cmp(&b.depth).then(b.area.abs().total_cmp(&a.area.abs())));
    out
}

/// Marching squares: closed outlines in pixel coordinates.
fn contours(field: &[f32], w: usize, h: usize, thr: f32) -> Vec<Vec<P>> {
    let gw = w + 2;
    // the grid ringed with outside: grid (i, j) is pixel (i − 1, j − 1)
    let val = |i: usize, j: usize| -> f32 { if i == 0 || j == 0 || i > w || j > h { f32::NEG_INFINITY } else { field[(j - 1) * w + (i - 1)] } };
    let ins = |i: usize, j: usize| val(i, j) >= thr;
    let pos = |i: usize, j: usize| [i as f64 - 0.5, j as f64 - 0.5];
    let crossing = |a: (usize, usize), b: (usize, usize)| -> P {
        let (va, vb) = (val(a.0, a.1), val(b.0, b.1));
        let t = if va.is_finite() && vb.is_finite() && (vb - va).abs() > 1e-12 {
            ((thr - va) / (vb - va)).clamp(0.0, 1.0) as f64
        } else if va.is_finite() {
            // the ring: half a pixel out
            if va >= thr { 0.5 } else { 0.0 }
        } else if vb >= thr {
            0.5
        } else {
            1.0
        };
        let (pa, pb) = (pos(a.0, a.1), pos(b.0, b.1));
        [pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t]
    };
    let across = |i: usize, j: usize| 2 * (j * gw + i);
    let down = |i: usize, j: usize| 2 * (j * gw + i) + 1;
    let mut next: HashMap<usize, usize> = HashMap::new();
    let mut point: HashMap<usize, P> = HashMap::new();
    for j in 0..h + 1 {
        for i in 0..w + 1 {
            let (tl, tr, br, bl) = (ins(i, j), ins(i + 1, j), ins(i + 1, j + 1), ins(i, j + 1));
            let code = (tl as u8) << 3 | (tr as u8) << 2 | (br as u8) << 1 | bl as u8;
            if code == 0 || code == 15 {
                continue;
            }
            let (top, bottom, left, right) = (across(i, j), across(i, j + 1), down(i, j), down(i + 1, j));
            let mean = || (val(i, j) + val(i + 1, j) + val(i + 1, j + 1) + val(i, j + 1)) / 4.0;
            let mut seg = |a: usize, b: usize| {
                next.insert(a, b);
            };
            match code {
                1 => seg(left, bottom),
                2 => seg(bottom, right),
                3 => seg(left, right),
                4 => seg(right, top),
                5 => {
                    if mean() >= thr {
                        seg(left, top);
                        seg(right, bottom);
                    } else {
                        seg(left, bottom);
                        seg(right, top);
                    }
                }
                6 => seg(bottom, top),
                7 => seg(left, top),
                8 => seg(top, left),
                9 => seg(top, bottom),
                10 => {
                    if mean() >= thr {
                        seg(top, right);
                        seg(bottom, left);
                    } else {
                        seg(top, left);
                        seg(bottom, right);
                    }
                }
                11 => seg(top, right),
                12 => seg(right, left),
                13 => seg(right, bottom),
                14 => seg(bottom, left),
                _ => {}
            }
            for (e, a, b) in [(top, (i, j), (i + 1, j)), (bottom, (i, j + 1), (i + 1, j + 1)), (left, (i, j), (i, j + 1)), (right, (i + 1, j), (i + 1, j + 1))] {
                if ins(a.0, a.1) != ins(b.0, b.1) {
                    point.entry(e).or_insert_with(|| crossing(a, b));
                }
            }
        }
    }
    let mut starts: Vec<usize> = next.keys().copied().collect();
    starts.sort_unstable();
    let mut out = Vec::new();
    for start in starts {
        if !next.contains_key(&start) {
            continue;
        }
        let mut poly = Vec::new();
        let mut e = start;
        while let Some(n) = next.remove(&e) {
            if let Some(p) = point.get(&e) {
                poly.push(*p);
            }
            e = n;
            if e == start {
                break;
            }
        }
        if poly.len() >= 3 {
            out.push(poly);
        }
    }
    out
}

fn inside(p: P, poly: &[P]) -> bool {
    let mut odd = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            odd = !odd;
        }
        j = i;
    }
    odd
}

fn from_segment(p: P, a: P, b: P) -> f64 {
    let d = sub(b, a);
    let l2 = d[0] * d[0] + d[1] * d[1];
    let t = if l2 > 0.0 { (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
    dist(p, [a[0] + t * d[0], a[1] + t * d[1]])
}

/// Douglas–Peucker on `p[lo..=hi]`: the indices kept between them.
fn keep_between(p: &[P], lo: usize, hi: usize, tol: f64, keep: &mut Vec<usize>) {
    if hi <= lo + 1 {
        return;
    }
    let (mut worst, mut at) = (0.0, lo);
    for k in lo + 1..hi {
        let d = from_segment(p[k], p[lo], p[hi]);
        if d > worst {
            (worst, at) = (d, k);
        }
    }
    if worst > tol {
        keep_between(p, lo, at, tol, keep);
        keep.push(at);
        keep_between(p, at, hi, tol, keep);
    }
}

/// Douglas–Peucker on a closed outline: the indices kept, in order.
fn simplified(p: &[P], tol: f64) -> Vec<usize> {
    let n = p.len();
    // split at the first point and the one farthest from it
    let far = (0..n).max_by(|&a, &b| dist(p[a], p[0]).total_cmp(&dist(p[b], p[0]))).unwrap_or(0);
    let mut ring = p.to_vec();
    ring.push(p[0]);
    let mut keep = vec![0];
    keep_between(&ring, 0, far, tol, &mut keep);
    keep.push(far);
    keep_between(&ring, far, n, tol, &mut keep);
    keep.sort_unstable();
    keep.dedup();
    keep.retain(|&i| i < n);
    keep
}

fn unit(v: P) -> Option<P> {
    let l = v[0].hypot(v[1]);
    (l > 1e-12 && l.is_finite()).then(|| [v[0] / l, v[1] / l])
}

/// Cubics through the kept vertices of an outline (`raw` holds all its samples): its vertices
/// and tangents.
fn fitted(raw: &[P], simple: &[P], round: f64) -> (Vec<P>, Vec<P>, Vec<P>) {
    let n = simple.len();
    let (mut ins, mut outs) = (vec![[0.0; 2]; n], vec![[0.0; 2]; n]);
    if round <= 1e-6 {
        return (simple.to_vec(), ins, outs);
    }
    let idx: Vec<usize> = simple.iter().map(|s| raw.iter().position(|r| r == s).unwrap_or(0)).collect();
    let dirs: Vec<(P, P)> = (0..n)
        .map(|i| {
            let (prev, cur, next) = (simple[(i + n - 1) % n], simple[i], simple[(i + 1) % n]);
            let smooth = unit(sub(next, prev)).unwrap_or([1.0, 0.0]);
            let (out_chord, in_chord) = (unit(sub(next, cur)).unwrap_or(smooth), unit(sub(cur, prev)).unwrap_or(smooth));
            let blend = |c: P| unit([c[0] * (1.0 - round) + smooth[0] * round, c[1] * (1.0 - round) + smooth[1] * round]).unwrap_or(c);
            (blend(out_chord), blend(in_chord))
        })
        .collect();
    for i in 0..n {
        let j = (i + 1) % n;
        let (a, b) = (idx[i], idx[j]);
        let pts: Vec<P> = if b > a { raw[a..=b].to_vec() } else { raw[a..].iter().chain(raw[..=b].iter()).copied().collect() };
        let chord = dist(simple[i], simple[j]);
        let (t0, t1) = (dirs[i].0, dirs[j].1);
        let (l0, l1) = if pts.len() >= 3 {
            let c = crate::fit::fit(&pts, t0, t1);
            let most = chord * 0.5 * round.max(0.34);
            (((c[1][0] - c[0][0]) * t0[0] + (c[1][1] - c[0][1]) * t0[1]).clamp(0.0, most), ((c[3][0] - c[2][0]) * t1[0] + (c[3][1] - c[2][1]) * t1[1]).clamp(0.0, most))
        } else {
            (chord / 3.0 * round, chord / 3.0 * round)
        };
        outs[i] = [t0[0] * l0, t0[1] * l0];
        ins[j] = [-t1[0] * l1, -t1[1] * l1];
    }
    (simple.to_vec(), ins, outs)
}
