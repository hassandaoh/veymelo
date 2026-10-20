//! Bending a mesh as little as it can be bent: "as-rigid-as-possible" shape manipulation, after
//! Igarashi, Moscovich and Hughes (SIGGRAPH 2005). Two least-squares solves:
//!
//! 1. Each triangle's third corner is written in the frame of each of its sides
//!    (`v_k = v_i + x·(v_j − v_i) + y·turn90(v_j − v_i)`); keeping those relations as well as
//!    can be, with the pinned corners where the pins are, lets every triangle turn and grow or
//!    shrink evenly, but not shear.
//! 2. Each triangle of that is matched with its own rest shape turned to fit it best; the corners
//!    are then placed so every side is as near as can be to its matched triangle's. That takes
//!    away the growing and shrinking. Repeating this step turns the parts that turn further.
//!
//! Stiffer triangles (starch) count for more. Sides can be given lengths and directions of their
//! own (bend and advanced pins turn and scale about themselves). Both systems are symmetric and
//! positive definite (a very small pull toward a first guess keeps a loosely pinned mesh from
//! drifting) and are solved by conjugate gradients.

use std::collections::BTreeMap;

/// A side `i → j` that should be `to`, as strongly as `weight`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Side {
    pub i: usize,
    pub j: usize,
    pub to: [f64; 2],
    pub weight: f64,
}

/// What holds the mesh: corners at places, sides at lengths and directions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Holds {
    pub at: Vec<(usize, [f64; 2])>,
    pub sides: Vec<Side>,
}

/// A symmetric matrix of few numbers, by row.
struct Sparse {
    rows: Vec<BTreeMap<usize, f64>>,
}

impl Sparse {
    fn new(n: usize) -> Sparse {
        Sparse { rows: vec![BTreeMap::new(); n] }
    }

    fn add(&mut self, i: usize, j: usize, v: f64) {
        *self.rows[i].entry(j).or_insert(0.0) += v;
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Solve `m x = b` for the unknowns that are `free`, the others staying as they are in `x`
/// (conjugate gradients, each step scaled by the diagonal, from `x` as it is).
fn solve(m: &Sparse, b: &[f64], x: &mut [f64], free: &[bool]) {
    let n = b.len();
    let vars: Vec<usize> = (0..n).filter(|i| free[*i]).collect();
    if vars.is_empty() {
        return;
    }
    let mut slot = vec![usize::MAX; n];
    for (k, i) in vars.iter().enumerate() {
        slot[*i] = k;
    }
    // the system for the free unknowns alone, what is held moved to the right side
    let size = vars.len();
    let (mut start, mut col, mut val) = (vec![0usize], Vec::new(), Vec::new());
    let mut diag = vec![1.0; size];
    let mut rhs = vec![0.0; size];
    for (k, &i) in vars.iter().enumerate() {
        let mut r = b[i];
        for (&j, &v) in &m.rows[i] {
            if free[j] {
                col.push(slot[j]);
                val.push(v);
                if j == i && v.abs() > 1e-300 {
                    diag[k] = v;
                }
            } else {
                r -= v * x[j];
            }
        }
        rhs[k] = r;
        start.push(col.len());
    }
    let times = |p: &[f64], out: &mut [f64]| {
        for (r, o) in out.iter_mut().enumerate() {
            *o = (start[r]..start[r + 1]).map(|k| val[k] * p[col[k]]).sum();
        }
    };
    let mut xs: Vec<f64> = vars.iter().map(|i| x[*i]).collect();
    let mut q = vec![0.0; size];
    times(&xs, &mut q);
    let mut r: Vec<f64> = rhs.iter().zip(&q).map(|(b, a)| b - a).collect();
    let goal = 1e-13 * dot(&rhs, &rhs).sqrt().max(1e-30);
    let mut z: Vec<f64> = r.iter().zip(&diag).map(|(r, d)| r / d).collect();
    let mut p = z.clone();
    let mut rz = dot(&r, &z);
    for _ in 0..4 * size + 50 {
        if dot(&r, &r).sqrt() <= goal {
            break;
        }
        times(&p, &mut q);
        let pq = dot(&p, &q);
        if pq.abs() < 1e-300 {
            break;
        }
        let step = rz / pq;
        for k in 0..size {
            xs[k] += step * p[k];
            r[k] -= step * q[k];
            z[k] = r[k] / diag[k];
        }
        let rz_next = dot(&r, &z);
        let keep = rz_next / rz;
        rz = rz_next;
        for k in 0..size {
            p[k] = z[k] + keep * p[k];
        }
    }
    for (k, i) in vars.iter().enumerate() {
        x[*i] = xs[k];
    }
}

/// The pull toward the first guess, for a mesh whose sides are about one unit.
const PULL: f64 = 1e-9;

/// The mesh `rest` (triangles `tris`, each as stiff as its `weights`, 1 being usual) bent as
/// little as it can be while `holds` hold, with `refine` more rounds of step 2. Where its
/// corners go.
pub fn bend(rest: &[[f64; 2]], tris: &[[usize; 3]], weights: &[f64], holds: &Holds, refine: usize) -> Vec<[f64; 2]> {
    let n = rest.len();
    if n == 0 {
        return Vec::new();
    }
    // first guess: moved as the held corners moved, on average
    let mut shift = [0.0, 0.0];
    for (i, p) in &holds.at {
        shift = [shift[0] + p[0] - rest[*i][0], shift[1] + p[1] - rest[*i][1]];
    }
    if !holds.at.is_empty() {
        let k = holds.at.len() as f64;
        shift = [shift[0] / k, shift[1] / k];
    }
    let guess: Vec<[f64; 2]> = rest.iter().map(|p| [p[0] + shift[0], p[1] + shift[1]]).collect();
    // one corner held and nothing else: any turn about it costs nothing, so the move is the answer
    if holds.at.len() == 1 && holds.sides.is_empty() {
        return guess;
    }
    let mut held = vec![false; n];
    for (i, _) in &holds.at {
        held[*i] = true;
    }
    let weight = |k: usize| weights.get(k).copied().unwrap_or(1.0);
    let mean_side2 = if tris.is_empty() { 1.0 } else { tris.iter().map(|t| (rest[t[0]][0] - rest[t[1]][0]).powi(2) + (rest[t[0]][1] - rest[t[1]][1]).powi(2)).sum::<f64>() / tris.len() as f64 };
    let pull = PULL * mean_side2.max(1e-12).recip().min(1.0);

    // step 1: x and y of every corner together (x0, y0, x1, y1…)
    let mut m = Sparse::new(2 * n);
    let mut b = vec![0.0; 2 * n];
    for (k, t) in tris.iter().enumerate() {
        let w = weight(k);
        for (i, j, l) in [(t[0], t[1], t[2]), (t[1], t[2], t[0]), (t[2], t[0], t[1])] {
            let e = [rest[j][0] - rest[i][0], rest[j][1] - rest[i][1]];
            let d = [rest[l][0] - rest[i][0], rest[l][1] - rest[i][1]];
            let ee = e[0] * e[0] + e[1] * e[1];
            if ee <= 1e-18 {
                continue;
            }
            // l's place in the frame of side i → j
            let (x, y) = ((d[0] * e[0] + d[1] * e[1]) / ee, (d[1] * e[0] - d[0] * e[1]) / ee);
            // the error v_l − v_i − x·(v_j − v_i) − y·turn90(v_j − v_i), as 2×2 blocks per corner
            let parts = [(l, [[1.0, 0.0], [0.0, 1.0]]), (i, [[x - 1.0, -y], [y, x - 1.0]]), (j, [[-x, y], [-y, -x]])];
            for (va, ba) in &parts {
                for (vb, bb) in &parts {
                    // ba transposed times bb
                    for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                        let v = ba[0][r] * bb[0][c] + ba[1][r] * bb[1][c];
                        if v != 0.0 {
                            m.add(2 * va + r, 2 * vb + c, w * v);
                        }
                    }
                }
            }
        }
    }
    for s in &holds.sides {
        for c in 0..2 {
            m.add(2 * s.i + c, 2 * s.i + c, s.weight);
            m.add(2 * s.j + c, 2 * s.j + c, s.weight);
            m.add(2 * s.i + c, 2 * s.j + c, -s.weight);
            m.add(2 * s.j + c, 2 * s.i + c, -s.weight);
            b[2 * s.j + c] += s.weight * s.to[c];
            b[2 * s.i + c] -= s.weight * s.to[c];
        }
    }
    for i in 0..n {
        for c in 0..2 {
            m.add(2 * i + c, 2 * i + c, pull);
            b[2 * i + c] += pull * guess[i][c];
        }
    }
    let mut x: Vec<f64> = guess.iter().flat_map(|p| [p[0], p[1]]).collect();
    for (i, p) in &holds.at {
        x[2 * i] = p[0];
        x[2 * i + 1] = p[1];
    }
    let free2: Vec<bool> = (0..2 * n).map(|k| !held[k / 2]).collect();
    solve(&m, &b, &mut x, &free2);
    let mut now: Vec<[f64; 2]> = (0..n).map(|i| [x[2 * i], x[2 * i + 1]]).collect();

    // step 2: x and y apart, every side toward its triangle turned to fit
    let mut l = Sparse::new(n);
    let mut join = |i: usize, j: usize, w: f64| {
        l.add(i, i, w);
        l.add(j, j, w);
        l.add(i, j, -w);
        l.add(j, i, -w);
    };
    for (k, t) in tris.iter().enumerate() {
        for e in 0..3 {
            join(t[e], t[(e + 1) % 3], weight(k));
        }
    }
    for s in &holds.sides {
        join(s.i, s.j, s.weight);
    }
    for i in 0..n {
        l.add(i, i, pull);
    }
    let free: Vec<bool> = (0..n).map(|i| !held[i]).collect();
    for _ in 0..=refine {
        let (mut bx, mut by) = (vec![0.0; n], vec![0.0; n]);
        for (k, t) in tris.iter().enumerate() {
            let w = weight(k);
            let fit = turned_to_fit([rest[t[0]], rest[t[1]], rest[t[2]]], [now[t[0]], now[t[1]], now[t[2]]]);
            for e in 0..3 {
                let (a, c) = (e, (e + 1) % 3);
                let (i, j) = (t[a], t[c]);
                let d = [fit[c][0] - fit[a][0], fit[c][1] - fit[a][1]];
                bx[j] += w * d[0];
                bx[i] -= w * d[0];
                by[j] += w * d[1];
                by[i] -= w * d[1];
            }
        }
        for s in &holds.sides {
            bx[s.j] += s.weight * s.to[0];
            bx[s.i] -= s.weight * s.to[0];
            by[s.j] += s.weight * s.to[1];
            by[s.i] -= s.weight * s.to[1];
        }
        for i in 0..n {
            bx[i] += pull * guess[i][0];
            by[i] += pull * guess[i][1];
        }
        let mut xs: Vec<f64> = now.iter().map(|p| p[0]).collect();
        let mut ys: Vec<f64> = now.iter().map(|p| p[1]).collect();
        for (i, p) in &holds.at {
            xs[*i] = p[0];
            ys[*i] = p[1];
        }
        solve(&l, &bx, &mut xs, &free);
        solve(&l, &by, &mut ys, &free);
        now = (0..n).map(|i| [xs[i], ys[i]]).collect();
    }
    now
}

/// Triangle `rest` turned about its middle and moved onto `now` as nearly as it can be.
pub fn turned_to_fit(rest: [[f64; 2]; 3], now: [[f64; 2]; 3]) -> [[f64; 2]; 3] {
    let mid = |t: &[[f64; 2]; 3]| [(t[0][0] + t[1][0] + t[2][0]) / 3.0, (t[0][1] + t[1][1] + t[2][1]) / 3.0];
    let (mr, mn) = (mid(&rest), mid(&now));
    let (mut same, mut across) = (0.0, 0.0);
    for k in 0..3 {
        let p = [rest[k][0] - mr[0], rest[k][1] - mr[1]];
        let q = [now[k][0] - mn[0], now[k][1] - mn[1]];
        same += p[0] * q[0] + p[1] * q[1];
        across += p[0] * q[1] - p[1] * q[0];
    }
    let (s, c) = across.atan2(same).sin_cos();
    rest.map(|v| {
        let p = [v[0] - mr[0], v[1] - mr[1]];
        [mn[0] + p[0] * c - p[1] * s, mn[1] + p[0] * s + p[1] * c]
    })
}
