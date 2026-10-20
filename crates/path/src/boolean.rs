//! Merging shapes: their union, the first less the others, what they share, or where an odd
//! number of them are. The shapes are flattened; every edge is cut where it crosses another, and
//! an edge is kept where the result is inside on one side of it and outside on the other, turned
//! so the inside is on its left. The kept edges are then linked into outlines.

use crate::Path;
use crate::shape::Sub;

/// How shapes are merged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Merge {
    /// All of them as one shape, as they are (their fill rule decides the inside).
    #[default]
    Merge,
    /// Their union.
    Add,
    /// The first, less all the others.
    Subtract,
    /// What all of them share.
    Intersect,
    /// Where an odd number of them are.
    Exclude,
}

type P = [f64; 2];

/// One shape as closed polygons.
fn polygons(shape: &[Sub]) -> Vec<Vec<P>> {
    let mut path = Path::default();
    for s in shape {
        // an open run is closed for merging, as a fill would close it
        let closed = Sub { v: s.v.clone(), closed: true };
        closed.flatten_into(&mut path);
    }
    path.contours()
        .iter()
        .map(|c| {
            let mut pts: Vec<P> = c.iter().map(|p| [p.0, p.1]).collect();
            if pts.len() > 1 && pts.first() == pts.last() {
                pts.pop();
            }
            pts.dedup();
            pts
        })
        .filter(|c| c.len() >= 3)
        .collect()
}

/// How many times the polygons wind round `p` (crossings of a ray to the right, by direction).
fn winding(polys: &[Vec<P>], p: P) -> i32 {
    let mut w = 0;
    for poly in polys {
        let n = poly.len();
        for k in 0..n {
            let (a, b) = (poly[k], poly[(k + 1) % n]);
            if (a[1] <= p[1]) != (b[1] <= p[1]) {
                let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                if x > p[0] {
                    w += if b[1] > a[1] { 1 } else { -1 };
                }
            }
        }
    }
    w
}

/// Where segments `a0`-`a1` and `b0`-`b1` cross, as fractions along each (inside both).
fn cross(a0: P, a1: P, b0: P, b1: P) -> Option<(f64, f64)> {
    let (r, s) = ([a1[0] - a0[0], a1[1] - a0[1]], [b1[0] - b0[0], b1[1] - b0[1]]);
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-14 {
        return None;
    }
    let q = [b0[0] - a0[0], b0[1] - a0[1]];
    let t = (q[0] * s[1] - q[1] * s[0]) / den;
    let u = (q[0] * r[1] - q[1] * r[0]) / den;
    let eps = 1e-12;
    (t > eps && t < 1.0 - eps && u > -eps && u < 1.0 + eps || u > eps && u < 1.0 - eps && t > -eps && t < 1.0 + eps).then_some((t, u))
}

/// Merge shapes (each a list of runs, in order) by `mode`.
pub fn merge(shapes: &[Vec<Sub>], mode: Merge) -> Vec<Sub> {
    if mode == Merge::Merge {
        return shapes.iter().flatten().cloned().collect();
    }
    let inputs: Vec<Vec<Vec<P>>> = shapes.iter().map(|s| polygons(s)).collect();
    if inputs.iter().all(|p| p.is_empty()) {
        return Vec::new();
    }
    let inside = |p: P| -> bool {
        let ins: Vec<bool> = inputs.iter().map(|polys| winding(polys, p) != 0).collect();
        match mode {
            Merge::Add | Merge::Merge => ins.iter().any(|&x| x),
            Merge::Subtract => ins.first().copied().unwrap_or(false) && !ins[1..].iter().any(|&x| x),
            Merge::Intersect => ins.iter().all(|&x| x),
            Merge::Exclude => ins.iter().filter(|&&x| x).count() % 2 == 1,
        }
    };
    // every edge, cut where any other crosses it
    let mut edges: Vec<(P, P)> = Vec::new();
    for polys in &inputs {
        for poly in polys {
            let n = poly.len();
            for k in 0..n {
                edges.push((poly[k], poly[(k + 1) % n]));
            }
        }
    }
    let boxes: Vec<[f64; 4]> = edges.iter().map(|(a, b)| [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]).collect();
    let mut cuts: Vec<Vec<f64>> = vec![Vec::new(); edges.len()];
    for i in 0..edges.len() {
        for j in i + 1..edges.len() {
            let (bi, bj) = (boxes[i], boxes[j]);
            if bi[2] < bj[0] || bj[2] < bi[0] || bi[3] < bj[1] || bj[3] < bi[1] {
                continue;
            }
            if let Some((t, u)) = cross(edges[i].0, edges[i].1, edges[j].0, edges[j].1) {
                cuts[i].push(t);
                cuts[j].push(u);
            }
        }
    }
    let size = boxes.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |s, b| [s[0].min(b[0]), s[1].min(b[1]), s[2].max(b[2]), s[3].max(b[3])]);
    let eps = ((size[2] - size[0]).max(size[3] - size[1]) * 1e-7).max(1e-9);
    let mut kept: Vec<(P, P)> = Vec::new();
    for (k, (a, b)) in edges.iter().enumerate() {
        let mut ts: Vec<f64> = cuts[k].iter().copied().filter(|t| *t > 0.0 && *t < 1.0).collect();
        ts.push(0.0);
        ts.push(1.0);
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-12);
        for w in ts.windows(2) {
            let at = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let (p, q) = (at(w[0]), at(w[1]));
            let d = [q[0] - p[0], q[1] - p[1]];
            let l = d[0].hypot(d[1]);
            if l < 1e-12 {
                continue;
            }
            let m = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
            let nrm = [-d[1] / l * eps, d[0] / l * eps];
            let left = inside([m[0] + nrm[0], m[1] + nrm[1]]);
            let right = inside([m[0] - nrm[0], m[1] - nrm[1]]);
            if left != right {
                kept.push(if left { (p, q) } else { (q, p) });
            }
        }
    }
    // the same edge from two shapes is one edge
    let key = |p: P| ((p[0] / eps).round() as i64, (p[1] / eps).round() as i64);
    kept.sort_by(|x, y| key(x.0).cmp(&key(y.0)).then(key(x.1).cmp(&key(y.1))));
    kept.dedup_by(|x, y| key(x.0) == key(y.0) && key(x.1) == key(y.1));
    link(kept, eps)
}

/// Edges linked end to start into closed outlines.
fn link(edges: Vec<(P, P)>, eps: f64) -> Vec<Sub> {
    let snap = |p: P| ((p[0] / (eps * 100.0)).round() as i64, (p[1] / (eps * 100.0)).round() as i64);
    let mut from: std::collections::HashMap<(i64, i64), Vec<usize>> = std::collections::HashMap::new();
    for (i, (a, _)) in edges.iter().enumerate() {
        from.entry(snap(*a)).or_default().push(i);
    }
    let mut used = vec![false; edges.len()];
    let mut out = Vec::new();
    for start in 0..edges.len() {
        if used[start] {
            continue;
        }
        let mut loop_pts = vec![edges[start].0];
        let mut at = start;
        used[at] = true;
        loop {
            let end = edges[at].1;
            if snap(end) == snap(edges[start].0) {
                break;
            }
            loop_pts.push(end);
            let Some(next) = from.get(&snap(end)).and_then(|c| c.iter().copied().find(|&i| !used[i])) else { break };
            used[next] = true;
            at = next;
        }
        if loop_pts.len() >= 3 {
            out.push(Sub::polygon(&loop_pts, true));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fill::{Rule, fill};
    use crate::shape::{ellipse, flatten, rect};

    fn area(shapes: &[Sub]) -> f32 {
        fill(&flatten(shapes), Rule::NonZero, 400, 400).map_or(0.0, |m| m.cover.iter().sum())
    }

    fn two() -> Vec<Vec<Sub>> {
        // two 100 × 100 squares overlapping in a 50 × 100 strip
        vec![vec![rect([100.0, 100.0], [100.0, 100.0], 0.0)], vec![rect([100.0, 100.0], [150.0, 100.0], 0.0)]]
    }

    #[test]
    fn squares_merged_each_way() {
        let s = two();
        assert!((area(&merge(&s, Merge::Add)) - 15000.0).abs() < 2.0, "{}", area(&merge(&s, Merge::Add)));
        assert!((area(&merge(&s, Merge::Subtract)) - 5000.0).abs() < 2.0, "{}", area(&merge(&s, Merge::Subtract)));
        assert!((area(&merge(&s, Merge::Intersect)) - 5000.0).abs() < 2.0, "{}", area(&merge(&s, Merge::Intersect)));
        assert!((area(&merge(&s, Merge::Exclude)) - 10000.0).abs() < 2.0, "{}", area(&merge(&s, Merge::Exclude)));
        assert_eq!(merge(&s, Merge::Merge).len(), 2, "merge keeps them as they are");
        assert_eq!(merge(&s, Merge::Add).len(), 1, "one outline round both");
    }

    #[test]
    fn a_hole_and_curves() {
        let ring = vec![vec![ellipse([200.0, 200.0], [200.0, 200.0])], vec![ellipse([100.0, 100.0], [200.0, 200.0])]];
        let r = merge(&ring, Merge::Subtract);
        let want = std::f32::consts::PI * (100.0f32.powi(2) - 50.0f32.powi(2));
        assert!((area(&r) - want).abs() / want < 0.005, "{} vs {want}", area(&r));
        assert_eq!(r.len(), 2, "an outline and a hole");
        // apart, nothing is shared
        let apart = vec![vec![rect([10.0, 10.0], [20.0, 20.0], 0.0)], vec![rect([10.0, 10.0], [80.0, 80.0], 0.0)]];
        assert!(merge(&apart, Merge::Intersect).is_empty());
        assert!((area(&merge(&apart, Merge::Add)) - 200.0).abs() < 1.0);
    }

    #[test]
    fn edges_lying_on_each_other() {
        // side by side, sharing an edge: their union is one rectangle
        let side = vec![vec![rect([100.0, 100.0], [100.0, 100.0], 0.0)], vec![rect([100.0, 100.0], [200.0, 100.0], 0.0)]];
        let u = merge(&side, Merge::Add);
        assert!((area(&u) - 20000.0).abs() < 2.0, "{}", area(&u));
        assert_eq!(u.len(), 1);
        // the same square twice: once
        let same = vec![vec![rect([100.0, 100.0], [100.0, 100.0], 0.0)], vec![rect([100.0, 100.0], [100.0, 100.0], 0.0)]];
        assert!((area(&merge(&same, Merge::Add)) - 10000.0).abs() < 2.0);
        assert!(merge(&same, Merge::Exclude).is_empty());
    }
}
