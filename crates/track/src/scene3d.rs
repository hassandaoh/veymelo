//! A moving camera worked out from footage (structure from motion), the way a 3D camera tracker
//! does it: features followed across the frames (`Follower`), then where the camera was on each
//! frame and where the followed points are in the world (`solve`).
//!
//! 1. Features (Shi–Tomasi corners) are followed from frame to frame by pyramidal Lucas–Kanade,
//!    checked by following back; those that do not move as the scene's geometry allows (a
//!    fundamental matrix most agree with, RANSAC, Sampson distance) are let go; new ones are
//!    found where there are few.
//! 2. Two frames far enough apart that the picture changed in depth (not as one flat picture
//!    would: a homography explains it worse) give how the camera moved between them: the
//!    essential matrix of their shared points (eight-point, RANSAC, made to have two equal
//!    singular values), the one of its four turns and moves that puts the points in front of
//!    both cameras; their shared points are triangulated.
//! 3. Each other frame, outward from those, is placed from where the frame next to it was:
//!    Gauss–Newton on how far the points it sees land from where they are seen (Huber-weighted,
//!    so a bad point counts less); points seen from two placed frames far enough apart are then
//!    triangulated.
//! 4. All of it refined together, round after round: each camera again from all its points, each
//!    point again from all its cameras.
//!
//! A shot where the camera only turns (a tripod pan) has no depth to find: the turns come from
//! the homographies between frames, the points are put on a sphere round the camera. Where the
//! angle of view is not given, it is found by solving with several and keeping the one whose
//! points land nearest where they are seen.
//!
//! Points are in the frame's pixels, x right, y down; a camera looks down its +z, so a point
//! (x, y, z) in front of it is seen at (cx + f x / z, cy + f y / z).

use veymelo_raster::Image;

use crate::fit::{Model, robust};
use crate::klt::{Gray, corners, follow};

pub type M3 = [[f64; 3]; 3];
pub type V3 = [f64; 3];

const I3: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn mul(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn tr(a: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| a[j][i]))
}

fn mv(a: &M3, v: &V3) -> V3 {
    std::array::from_fn(|i| (0..3).map(|k| a[i][k] * v[k]).sum())
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

fn det(a: &M3) -> f64 {
    dot(a[0], cross(a[1], a[2]))
}

/// The turn about `w` (its length the angle, radians).
pub fn rodrigues(w: V3) -> M3 {
    let th = norm(w);
    if th < 1e-12 {
        return [[1.0, -w[2], w[1]], [w[2], 1.0, -w[0]], [-w[1], w[0], 1.0]];
    }
    let k = w.map(|v| v / th);
    let (s, c) = th.sin_cos();
    let kx: M3 = [[0.0, -k[2], k[1]], [k[2], 0.0, -k[0]], [-k[1], k[0], 0.0]];
    let kk = mul(&kx, &kx);
    std::array::from_fn(|i| std::array::from_fn(|j| I3[i][j] + s * kx[i][j] + (1.0 - c) * kk[i][j]))
}

/// A symmetric matrix's eigenvalues (smallest first) and eigenvectors (as columns' order: the
/// i-th returned vector goes with the i-th value), by Jacobi turns.
pub fn eigen(a: &[Vec<f64>]) -> (Vec<f64>, Vec<Vec<f64>>) {
    let n = a.len();
    let mut m: Vec<Vec<f64>> = a.to_vec();
    let mut v: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect();
    for _ in 0..100 {
        let off: f64 = (0..n).flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j))).map(|(i, j)| m[i][j] * m[i][j]).sum();
        if off < 1e-30 {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                if m[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (m[q][q] - m[p][p]) / (2.0 * m[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for row in m.iter_mut() {
                    let (mkp, mkq) = (row[p], row[q]);
                    row[p] = c * mkp - s * mkq;
                    row[q] = s * mkp + c * mkq;
                }
                let (upper, lower) = m.split_at_mut(q);
                for (mpk, mqk) in upper[p].iter_mut().zip(lower[0].iter_mut()) {
                    let (a, b) = (*mpk, *mqk);
                    *mpk = c * a - s * b;
                    *mqk = s * a + c * b;
                }
                for row in v.iter_mut() {
                    let (vkp, vkq) = (row[p], row[q]);
                    row[p] = c * vkp - s * vkq;
                    row[q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| m[i][i].total_cmp(&m[j][j]));
    (order.iter().map(|&i| m[i][i]).collect(), order.iter().map(|&i| (0..n).map(|k| v[k][i]).collect()).collect())
}

/// The singular value decomposition of a 3 × 3 matrix: U, the values (largest first), V.
pub fn svd3(a: &M3) -> (M3, V3, M3) {
    let ata = mul(&tr(a), a);
    let (vals, vecs) = eigen(&ata.iter().map(|r| r.to_vec()).collect::<Vec<_>>());
    // largest first
    let v: Vec<V3> = (0..3).rev().map(|i| [vecs[i][0], vecs[i][1], vecs[i][2]]).collect();
    let s: V3 = std::array::from_fn(|i| vals[2 - i].max(0.0).sqrt());
    let mut u: Vec<V3> = Vec::new();
    for vi in v.iter().take(2) {
        let av = mv(a, vi);
        let l = norm(av);
        u.push(if l > 1e-300 { av.map(|x| x / l) } else { [0.0; 3] });
    }
    if norm(u[1]) < 0.5 {
        let side = if u[0][0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let c = cross(u[0], side);
        u[1] = c.map(|x| x / norm(c));
    }
    u.push(cross(u[0], u[1]));
    let mut vv = v.clone();
    // v's third made right-handed to match
    if dot(cross(vv[0], vv[1]), vv[2]) < 0.0 {
        vv[2] = vv[2].map(|x| -x);
    }
    let um: M3 = std::array::from_fn(|i| std::array::from_fn(|j| u[j][i]));
    let vm: M3 = std::array::from_fn(|i| std::array::from_fn(|j| vv[j][i]));
    (um, s, vm)
}

/// The turn nearest a matrix.
fn nearest_turn(a: &M3) -> M3 {
    let (u, _, v) = svd3(a);
    let mut r = mul(&u, &tr(&v));
    if det(&r) < 0.0 {
        let mut u2 = u;
        for row in u2.iter_mut() {
            row[2] = -row[2];
        }
        r = mul(&u2, &tr(&v));
    }
    r
}

/// The fundamental (or, for points already divided by the focal length, essential) matrix most
/// like `x'ᵀ F x = 0` for `pairs` (x, x'), by the eight-point method on points moved and scaled
/// about their middle (Hartley's normalisation).
pub fn eight_point(pairs: &[([f64; 2], [f64; 2])]) -> Option<M3> {
    if pairs.len() < 8 {
        return None;
    }
    let norming = |pts: Vec<[f64; 2]>| -> M3 {
        let n = pts.len() as f64;
        let c = [pts.iter().map(|p| p[0]).sum::<f64>() / n, pts.iter().map(|p| p[1]).sum::<f64>() / n];
        let d = pts.iter().map(|p| (p[0] - c[0]).hypot(p[1] - c[1])).sum::<f64>() / n;
        let s = if d > 1e-12 { std::f64::consts::SQRT_2 / d } else { 1.0 };
        [[s, 0.0, -s * c[0]], [0.0, s, -s * c[1]], [0.0, 0.0, 1.0]]
    };
    let (t1, t2) = (norming(pairs.iter().map(|p| p.0).collect()), norming(pairs.iter().map(|p| p.1).collect()));
    let mut ata = vec![vec![0.0; 9]; 9];
    for (a, b) in pairs {
        let x = mv(&t1, &[a[0], a[1], 1.0]);
        let y = mv(&t2, &[b[0], b[1], 1.0]);
        let row = [y[0] * x[0], y[0] * x[1], y[0], y[1] * x[0], y[1] * x[1], y[1], x[0], x[1], 1.0];
        for i in 0..9 {
            for j in 0..9 {
                ata[i][j] += row[i] * row[j];
            }
        }
    }
    let (_, vecs) = eigen(&ata);
    let f = &vecs[0];
    let fm: M3 = [[f[0], f[1], f[2]], [f[3], f[4], f[5]], [f[6], f[7], f[8]]];
    // rank two
    let (u, s, v) = svd3(&fm);
    let d: M3 = [[s[0], 0.0, 0.0], [0.0, s[1], 0.0], [0.0, 0.0, 0.0]];
    let f2 = mul(&mul(&u, &d), &tr(&v));
    Some(mul(&mul(&tr(&t2), &f2), &t1))
}

/// How far a pair is from agreeing with F (Sampson's distance, in the points' units).
fn sampson(f: &M3, a: [f64; 2], b: [f64; 2]) -> f64 {
    let x = [a[0], a[1], 1.0];
    let y = [b[0], b[1], 1.0];
    let fx = mv(f, &x);
    let fty = mv(&tr(f), &y);
    let e = dot(y, fx);
    let d = fx[0] * fx[0] + fx[1] * fx[1] + fty[0] * fty[0] + fty[1] * fty[1];
    if d < 1e-300 { f64::INFINITY } else { (e * e / d).sqrt() }
}

/// Numbers that look random, the same each time.
struct Dice(u64);

impl Dice {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// The fundamental (or essential) matrix most pairs agree with (within `tol`), refined on those,
/// and which they are.
pub fn robust_fundamental(pairs: &[([f64; 2], [f64; 2])], tol: f64) -> Option<(M3, Vec<usize>)> {
    if pairs.len() < 8 {
        return None;
    }
    let mut dice = Dice(0x9E37_79B9_7F4A_7C15 ^ pairs.len() as u64);
    let agree = |f: &M3| -> Vec<usize> { (0..pairs.len()).filter(|&i| sampson(f, pairs[i].0, pairs[i].1) <= tol).collect() };
    let mut best: Vec<usize> = Vec::new();
    for _ in 0..300 {
        let mut pick: Vec<usize> = Vec::new();
        while pick.len() < 8 {
            let k = dice.below(pairs.len());
            if !pick.contains(&k) {
                pick.push(k);
            }
        }
        let Some(f) = eight_point(&pick.iter().map(|&i| pairs[i]).collect::<Vec<_>>()) else { continue };
        let a = agree(&f);
        if a.len() > best.len() {
            best = a;
            if best.len() * 10 >= pairs.len() * 9 {
                break;
            }
        }
    }
    let f = eight_point(&best.iter().map(|&i| pairs[i]).collect::<Vec<_>>())?;
    let a = agree(&f);
    Some((f, a))
}

/// A feature followed across frames: where it was seen on each frame of its run (the frame
/// pixels).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub first: usize,
    pub at: Vec<[f64; 2]>,
}

impl Track {
    pub fn on(&self, frame: usize) -> Option<[f64; 2]> {
        frame.checked_sub(self.first).and_then(|i| self.at.get(i)).copied()
    }

    pub fn last(&self) -> usize {
        self.first + self.at.len() - 1
    }
}

/// Features followed through frames given one after another.
pub struct Follower {
    last: Option<Gray>,
    /// The tracks still followed, and where each is in the last frame (its pixels).
    alive: Vec<(usize, [f64; 2])>,
    pub tracks: Vec<Track>,
    frame: usize,
    /// About how many it keeps following.
    pub most: usize,
}

impl Default for Follower {
    fn default() -> Follower {
        Follower { last: None, alive: Vec::new(), tracks: Vec::new(), frame: 0, most: 400 }
    }
}

impl Follower {
    /// The next frame (its layer (0, 0) at `offset` in it).
    pub fn add(&mut self, img: &Image, offset: [f64; 2]) {
        let now = Gray::new(img, offset, 3);
        let apart = (now.width().max(now.height()) as f64 / 45.0).max(5.0);
        if let Some(was) = &self.last {
            let from: Vec<[f64; 2]> = self.alive.iter().map(|a| a.1).collect();
            let to = follow(was, &now, &from);
            let moved: Vec<(usize, [f64; 2], [f64; 2])> = self.alive.iter().zip(to).filter_map(|((id, p), q)| Some((*id, *p, q?))).collect();
            // those that move as the scene's geometry allows
            let pairs: Vec<([f64; 2], [f64; 2])> = moved.iter().map(|(_, p, q)| (was.to_layer(*p), now.to_layer(*q))).collect();
            let keep: Vec<bool> = match robust_fundamental(&pairs, 1.25 * now.step) {
                Some((_, ok)) => {
                    let mut k = vec![false; pairs.len()];
                    ok.iter().for_each(|&i| k[i] = true);
                    k
                }
                None => vec![true; pairs.len()],
            };
            self.alive = moved.iter().zip(keep).filter(|(_, k)| *k).map(|((id, _, q), _)| (*id, *q)).collect();
            for (id, q) in &self.alive {
                self.tracks[*id].at.push(now.to_layer(*q));
            }
        }
        // more where there are few
        if self.alive.len() < self.most {
            for c in corners(&now, self.most * 2, apart, 0.01) {
                if self.alive.len() >= self.most {
                    break;
                }
                if self.alive.iter().any(|(_, p)| (p[0] - c[0]).hypot(p[1] - c[1]) < apart) {
                    continue;
                }
                self.tracks.push(Track { first: self.frame, at: vec![now.to_layer(c)] });
                self.alive.push((self.tracks.len() - 1, c));
            }
        }
        self.last = Some(now);
        self.frame += 1;
    }

    /// The tracks seen on three frames or more.
    pub fn finish(self) -> Vec<Track> {
        self.tracks.into_iter().filter(|t| t.at.len() >= 3).collect()
    }
}

/// How a shot was solved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Solve {
    /// The camera moved through the scene.
    Typical,
    /// The camera only turned.
    Tripod,
}

/// A camera on a frame: its turn (world to camera) and move (`x_cam = r x + t`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub r: M3,
    pub t: V3,
}

impl Pose {
    pub const ORIGIN: Pose = Pose { r: I3, t: [0.0; 3] };

    /// Where the camera is in the world.
    pub fn center(&self) -> V3 {
        mv(&tr(&self.r), &self.t).map(|v| -v)
    }

    pub fn to_camera(&self, x: V3) -> V3 {
        add(mv(&self.r, &x), self.t)
    }
}

/// A shot solved: the focal length (frame pixels), the frame's middle, each frame's camera
/// (none: not placed), each track's point in the world (none: not placed), how far the points
/// land from where they are seen on average (pixels), and how it was solved.
#[derive(Clone, Debug, PartialEq)]
pub struct Solution {
    pub focal: f64,
    pub center: [f64; 2],
    pub cameras: Vec<Option<Pose>>,
    pub points: Vec<Option<V3>>,
    pub error: f64,
    pub how: Solve,
}

impl Solution {
    /// Where world point `x` is seen on frame `k` (none: behind or not placed).
    pub fn seen(&self, k: usize, x: V3) -> Option<[f64; 2]> {
        let p = self.cameras.get(k)?.as_ref()?.to_camera(x);
        (p[2] > 1e-9).then(|| [self.center[0] + self.focal * p[0] / p[2], self.center[1] + self.focal * p[1] / p[2]])
    }
}

/// How the shot is to be solved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The angle of view across (degrees; none: found).
    pub angle: Option<f64>,
    /// How (none: whichever fits).
    pub how: Option<Solve>,
}

/// The middle of the frame, from the frames' size.
fn frame_middle(size: [f64; 4]) -> [f64; 2] {
    [(size[0] + size[2]) / 2.0, (size[1] + size[3]) / 2.0]
}

/// A point seen at pixel `p` as a way from the camera (divided by the focal length).
fn ray(p: [f64; 2], c: [f64; 2], f: f64) -> [f64; 2] {
    [(p[0] - c[0]) / f, (p[1] - c[1]) / f]
}

/// The point seen from cameras `views` (each its pose and where it is seen, as a way), by the
/// linear method (DLT), and refined: none where it is behind one or they are too near alike.
fn triangulate(views: &[(Pose, [f64; 2])]) -> Option<V3> {
    let mut ata = vec![vec![0.0; 4]; 4];
    for (p, x) in views {
        let rows = [[x[0] * p.r[2][0] - p.r[0][0], x[0] * p.r[2][1] - p.r[0][1], x[0] * p.r[2][2] - p.r[0][2], x[0] * p.t[2] - p.t[0]], [x[1] * p.r[2][0] - p.r[1][0], x[1] * p.r[2][1] - p.r[1][1], x[1] * p.r[2][2] - p.r[1][2], x[1] * p.t[2] - p.t[1]]];
        for row in rows {
            for i in 0..4 {
                for j in 0..4 {
                    ata[i][j] += row[i] * row[j];
                }
            }
        }
    }
    let (_, vecs) = eigen(&ata);
    let h = &vecs[0];
    if h[3].abs() < 1e-12 {
        return None;
    }
    let x = [h[0] / h[3], h[1] / h[3], h[2] / h[3]];
    let x = refine_point(x, views, 5);
    views.iter().all(|(p, _)| p.to_camera(x)[2] > 1e-6).then_some(x)
}

/// Huber's weight for a residual `r` (beyond `k` it counts less).
fn huber(r: f64, k: f64) -> f64 {
    if r <= k { 1.0 } else { k / r }
}

/// A point moved to where it is seen best from `views` (Gauss–Newton).
fn refine_point(mut x: V3, views: &[(Pose, [f64; 2])], rounds: usize) -> V3 {
    for _ in 0..rounds {
        let mut h = [[0.0; 3]; 3];
        let mut g = [0.0; 3];
        for (p, obs) in views {
            let c = p.to_camera(x);
            if c[2] <= 1e-9 {
                continue;
            }
            let (iz, iz2) = (1.0 / c[2], 1.0 / (c[2] * c[2]));
            let e = [c[0] * iz - obs[0], c[1] * iz - obs[1]];
            let dpi = [[iz, 0.0, -c[0] * iz2], [0.0, iz, -c[1] * iz2]];
            let j: [[f64; 3]; 2] = std::array::from_fn(|r| std::array::from_fn(|k| (0..3).map(|m| dpi[r][m] * p.r[m][k]).sum()));
            for r in 0..2 {
                for a in 0..3 {
                    g[a] += j[r][a] * e[r];
                    for b in 0..3 {
                        h[a][b] += j[r][a] * j[r][b];
                    }
                }
            }
        }
        for (i, row) in h.iter_mut().enumerate() {
            row[i] += 1e-12 + row[i] * 1e-6;
        }
        let Some(d) = crate::solve::linear(h, g) else { break };
        x = sub(x, d);
        if norm(d) < 1e-12 * (1.0 + norm(x)) {
            break;
        }
    }
    x
}

/// A camera moved and turned to where the points `seen` (each its place in the world and where it
/// is seen, as a way) land best (Gauss–Newton, Huber-weighted beyond `k`), from `pose`.
fn refine_pose(mut pose: Pose, seen: &[(V3, [f64; 2])], k: f64, rounds: usize) -> Pose {
    for _ in 0..rounds {
        let mut h = [[0.0; 6]; 6];
        let mut g = [0.0; 6];
        for (x, obs) in seen {
            let rx = mv(&pose.r, x);
            let c = add(rx, pose.t);
            if c[2] <= 1e-9 {
                continue;
            }
            let (iz, iz2) = (1.0 / c[2], 1.0 / (c[2] * c[2]));
            let e = [c[0] * iz - obs[0], c[1] * iz - obs[1]];
            let w = huber(e[0].hypot(e[1]), k);
            let dpi = [[iz, 0.0, -c[0] * iz2], [0.0, iz, -c[1] * iz2]];
            // d c / d ω = −[R x]×, d c / d t = I
            let skew: M3 = [[0.0, rx[2], -rx[1]], [-rx[2], 0.0, rx[0]], [rx[1], -rx[0], 0.0]];
            let mut j = [[0.0; 6]; 2];
            for r in 0..2 {
                for a in 0..3 {
                    j[r][a] = (0..3).map(|m| dpi[r][m] * skew[m][a]).sum();
                    j[r][3 + a] = dpi[r][a];
                }
            }
            for r in 0..2 {
                for a in 0..6 {
                    g[a] += w * j[r][a] * e[r];
                    for b in 0..6 {
                        h[a][b] += w * j[r][a] * j[r][b];
                    }
                }
            }
        }
        for (i, row) in h.iter_mut().enumerate() {
            row[i] += 1e-12 + row[i] * 1e-6;
        }
        let Some(d) = crate::solve::linear(h, g) else { break };
        pose.r = mul(&rodrigues([-d[0], -d[1], -d[2]]), &pose.r);
        pose.t = sub(pose.t, [d[3], d[4], d[5]]);
        if d.iter().map(|v| v * v).sum::<f64>().sqrt() < 1e-12 {
            break;
        }
    }
    pose
}

/// A turn alone (the camera's middle held at the world's) to where the ways `seen` (each the
/// way in the world and where it is seen) land best.
fn refine_turn(mut r: M3, seen: &[(V3, [f64; 2])], rounds: usize) -> M3 {
    for _ in 0..rounds {
        let mut h = [[0.0; 3]; 3];
        let mut g = [0.0; 3];
        for (x, obs) in seen {
            let c = mv(&r, x);
            if c[2] <= 1e-9 {
                continue;
            }
            let (iz, iz2) = (1.0 / c[2], 1.0 / (c[2] * c[2]));
            let e = [c[0] * iz - obs[0], c[1] * iz - obs[1]];
            let dpi = [[iz, 0.0, -c[0] * iz2], [0.0, iz, -c[1] * iz2]];
            let skew: M3 = [[0.0, c[2], -c[1]], [-c[2], 0.0, c[0]], [c[1], -c[0], 0.0]];
            let j: [[f64; 3]; 2] = std::array::from_fn(|row| std::array::from_fn(|a| (0..3).map(|m| dpi[row][m] * skew[m][a]).sum()));
            for row in 0..2 {
                for a in 0..3 {
                    g[a] += j[row][a] * e[row];
                    for b in 0..3 {
                        h[a][b] += j[row][a] * j[row][b];
                    }
                }
            }
        }
        for (i, row) in h.iter_mut().enumerate() {
            row[i] += 1e-12 + row[i] * 1e-6;
        }
        let Some(d) = crate::solve::linear(h, g) else { break };
        r = mul(&rodrigues([-d[0], -d[1], -d[2]]), &r);
    }
    r
}

/// The frames' size: x0, y0, x1, y1 (the layer's pixels).
pub type Frame = [f64; 4];

/// Shot `tracks` (over `frames` frames covering `size`) solved, its angle of view found when
/// not given: the solution whose points land nearest where they are seen.
pub fn solve(tracks: &[Track], frames: usize, size: Frame, s: &Settings) -> Option<Solution> {
    let width = size[2] - size[0];
    let focal_of = |deg: f64| width / 2.0 / (deg.to_radians() / 2.0).tan();
    if let Some(a) = s.angle {
        return solve_with(tracks, frames, size, focal_of(a), s.how);
    }
    // a few angles, then nearer round the best (an angle too wide or too narrow bends the scene
    // so its points land further from where they are seen)
    let mut tried: Vec<(f64, Solution)> = Vec::new();
    for deg in [20.0, 30.0, 40.0, 50.0, 60.0, 75.0, 90.0] {
        if let Some(sol) = solve_with(tracks, frames, size, focal_of(deg), s.how) {
            tried.push((deg, sol));
        }
    }
    let score = |s: &Solution| s.error * (1.0 + 4.0 * (1.0 - s.cameras.iter().flatten().count() as f64 / frames.max(1) as f64));
    let (mut lo, mut hi) = {
        let best = tried.iter().min_by(|a, b| score(&a.1).total_cmp(&score(&b.1)))?.0;
        (best * 0.7, best * 1.3)
    };
    for _ in 0..6 {
        let (a, b) = (lo + (hi - lo) * 0.382, lo + (hi - lo) * 0.618);
        let (sa, sb) = (solve_with(tracks, frames, size, focal_of(a), s.how), solve_with(tracks, frames, size, focal_of(b), s.how));
        let (ea, eb) = (sa.as_ref().map_or(f64::INFINITY, score), sb.as_ref().map_or(f64::INFINITY, score));
        if let Some(sa) = sa {
            tried.push((a, sa));
        }
        if let Some(sb) = sb {
            tried.push((b, sb));
        }
        if ea < eb { hi = b } else { lo = a }
    }
    tried.into_iter().min_by(|a, b| score(&a.1).total_cmp(&score(&b.1))).map(|t| t.1)
}

/// Shot `tracks` solved with focal length `f` (frame pixels).
pub fn solve_with(tracks: &[Track], frames: usize, size: Frame, f: f64, how: Option<Solve>) -> Option<Solution> {
    let c = frame_middle(size);
    let seen = |t: &Track, k: usize| t.on(k).map(|p| ray(p, c, f));
    // the first frame, and a later one far enough off in depth (the most its shared points
    // move unlike one flat picture)
    let a = (0..frames).find(|&k| tracks.iter().filter(|t| t.on(k).is_some()).count() >= 30)?;
    let flat_tol = 1.0;
    let mut pick: Option<(usize, f64)> = None;
    let mut flattest = 1.0f64;
    for b in a + 2..frames.min(a + 80) {
        let pairs: Vec<([f64; 2], [f64; 2])> = tracks.iter().filter_map(|t| Some((t.on(a)?, t.on(b)?))).collect();
        if pairs.len() < 30 {
            break;
        }
        let flat = robust(Model::Perspective, &pairs, flat_tol).map_or(0.0, |(_, n)| n as f64 / pairs.len() as f64);
        flattest = flattest.min(flat);
        let spread = {
            let mut d: Vec<f64> = pairs.iter().map(|(p, q)| (p[0] - q[0]).hypot(p[1] - q[1])).collect();
            d.sort_by(f64::total_cmp);
            d[d.len() / 2]
        };
        // far enough apart, unlike one flat picture; the more so the better
        let worth = (1.0 - flat) * spread.min(0.15 * (size[2] - size[0]));
        if pick.is_none_or(|p| worth > p.1) {
            pick = Some((b, worth));
        }
        if flat < 0.6 && spread > 0.08 * (size[2] - size[0]) {
            break;
        }
    }
    let tripod = match how {
        Some(Solve::Tripod) => true,
        Some(Solve::Typical) => false,
        None => flattest > 0.9,
    };
    if tripod {
        return solve_tripod(tracks, frames, size, f);
    }
    let b = pick?.0;
    let pairs: Vec<(usize, [f64; 2], [f64; 2])> = tracks.iter().enumerate().filter_map(|(i, t)| Some((i, seen(t, a)?, seen(t, b)?))).collect();
    let (e, inliers) = robust_fundamental(&pairs.iter().map(|p| (p.1, p.2)).collect::<Vec<_>>(), 1.5 / f)?;
    // made an essential matrix (two equal singular values, the third none)
    let (u, _, v) = svd3(&e);
    let w: M3 = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    let fix = |m: M3| if det(&m) < 0.0 { m.map(|r| r.map(|x| -x)) } else { m };
    let (r1, r2) = (fix(mul(&mul(&u, &w), &tr(&v))), fix(mul(&mul(&u, &tr(&w)), &tr(&v))));
    let t0 = [u[0][2], u[1][2], u[2][2]];
    let mut best: Option<(Pose, usize)> = None;
    for (r, t) in [(r1, t0), (r1, t0.map(|x| -x)), (r2, t0), (r2, t0.map(|x| -x))] {
        let pb = Pose { r, t };
        let front = inliers.iter().filter(|&&i| triangulate(&[(Pose::ORIGIN, pairs[i].1), (pb, pairs[i].2)]).is_some()).count();
        if best.as_ref().is_none_or(|bst| front > bst.1) {
            best = Some((pb, front));
        }
    }
    let (pose_b, _) = best?;
    let mut cameras: Vec<Option<Pose>> = vec![None; frames];
    cameras[a] = Some(Pose::ORIGIN);
    cameras[b] = Some(pose_b);
    let mut points: Vec<Option<V3>> = vec![None; tracks.len()];
    let place_points = |cameras: &[Option<Pose>], points: &mut Vec<Option<V3>>| {
        for (i, t) in tracks.iter().enumerate() {
            if points[i].is_some() {
                continue;
            }
            let views: Vec<(Pose, [f64; 2])> = (t.first..=t.last()).filter_map(|k| Some((cameras[k]?, seen(t, k)?))).collect();
            if views.len() < 2 {
                continue;
            }
            // seen from far enough apart (a degree at least between the ways)
            let (p0, p1) = (views[0], views[views.len() - 1]);
            let way = |p: &Pose, x: [f64; 2]| {
                let d = mv(&tr(&p.r), &[x[0], x[1], 1.0]);
                d.map(|v| v / norm(d))
            };
            if dot(way(&p0.0, p0.1), way(&p1.0, p1.1)) > (1.0f64).to_radians().cos() {
                continue;
            }
            if let Some(x) = triangulate(&views) {
                // where it lands from each, near where it is seen
                let ok = views.iter().all(|(p, o)| {
                    let q = p.to_camera(x);
                    (q[0] / q[2] - o[0]).hypot(q[1] / q[2] - o[1]) * f < 4.0
                });
                if ok {
                    points[i] = Some(x);
                }
            }
        }
    };
    place_points(&cameras, &mut points);
    // each other frame from its neighbour, forward then back
    let order: Vec<usize> = (a + 1..frames).filter(|&k| k != b).chain((0..a).rev()).collect();
    for k in order {
        let from = if k > a { (a..k).rev().find(|&j| cameras[j].is_some()) } else { (k + 1..frames).find(|&j| cameras[j].is_some()) };
        let Some(start) = from.and_then(|j| cameras[j]) else { continue };
        let obs: Vec<(V3, [f64; 2])> = tracks.iter().zip(&points).filter_map(|(t, p)| Some(((*p)?, seen(t, k)?))).collect();
        if obs.len() < 8 {
            continue;
        }
        let mut pose = refine_pose(start, &obs, 2.0 / f, 15);
        // let go of those that land far, then again
        let near: Vec<(V3, [f64; 2])> = obs
            .iter()
            .filter(|(x, o)| {
                let q = pose.to_camera(*x);
                q[2] > 0.0 && (q[0] / q[2] - o[0]).hypot(q[1] / q[2] - o[1]) * f < 3.0
            })
            .copied()
            .collect();
        if near.len() >= 8 {
            pose = refine_pose(pose, &near, 2.0 / f, 10);
        }
        cameras[k] = Some(pose);
        place_points(&cameras, &mut points);
    }
    // all of it again, round after round
    for _ in 0..8 {
        for (k, slot) in cameras.iter_mut().enumerate() {
            let Some(pose) = *slot else { continue };
            let obs: Vec<(V3, [f64; 2])> = tracks.iter().zip(&points).filter_map(|(t, p)| Some(((*p)?, seen(t, k)?))).collect();
            if obs.len() >= 8 {
                *slot = Some(refine_pose(pose, &obs, 2.0 / f, 5));
            }
        }
        for (i, t) in tracks.iter().enumerate() {
            let Some(x) = points[i] else { continue };
            let views: Vec<(Pose, [f64; 2])> = (t.first..=t.last()).filter_map(|k| Some((cameras[k]?, seen(t, k)?))).collect();
            points[i] = Some(refine_point(x, &views, 3));
        }
    }
    // let go of points that still land far
    for (i, t) in tracks.iter().enumerate() {
        let Some(x) = points[i] else { continue };
        let worst = (t.first..=t.last()).filter_map(|k| Some((cameras[k]?, seen(t, k)?))).map(|(p, o)| {
            let q = p.to_camera(x);
            if q[2] <= 0.0 { f64::INFINITY } else { (q[0] / q[2] - o[0]).hypot(q[1] / q[2] - o[1]) * f }
        }).fold(0.0, f64::max);
        if worst > 5.0 {
            points[i] = None;
        }
    }
    let mut sol = Solution { focal: f, center: c, cameras, points, error: 0.0, how: Solve::Typical };
    sol.error = mean_error(&sol, tracks);
    Some(sol)
}

/// How far the solution's points land from where they are seen, on average (pixels).
pub fn mean_error(s: &Solution, tracks: &[Track]) -> f64 {
    let (mut sum, mut n) = (0.0, 0usize);
    for (t, p) in tracks.iter().zip(&s.points) {
        let Some(x) = p else { continue };
        for k in t.first..=t.last() {
            let (Some(q), Some(o)) = (s.seen(k, *x), t.on(k)) else { continue };
            sum += (q[0] - o[0]).hypot(q[1] - o[1]);
            n += 1;
        }
    }
    if n == 0 { f64::INFINITY } else { sum / n as f64 }
}

/// A shot where the camera only turns: each frame's turn from the homography to the frame
/// before (the camera's turn, seen through its lens), refined; the points put a unit away.
fn solve_tripod(tracks: &[Track], frames: usize, size: Frame, f: f64) -> Option<Solution> {
    let c = frame_middle(size);
    let k: M3 = [[f, 0.0, c[0]], [0.0, f, c[1]], [0.0, 0.0, 1.0]];
    let kinv: M3 = [[1.0 / f, 0.0, -c[0] / f], [0.0, 1.0 / f, -c[1] / f], [0.0, 0.0, 1.0]];
    let mut cameras: Vec<Option<Pose>> = vec![None; frames];
    let first = (0..frames).find(|&k| tracks.iter().any(|t| t.on(k).is_some()))?;
    cameras[first] = Some(Pose::ORIGIN);
    for j in first + 1..frames {
        // from the nearest frame before it that is placed and shares enough with it
        let from = (first..j).rev().find_map(|i| {
            let r = cameras[i]?.r;
            let pairs: Vec<([f64; 2], [f64; 2])> = tracks.iter().filter_map(|t| Some((t.on(i)?, t.on(j)?))).collect();
            (pairs.len() >= 8).then_some((r, pairs))
        });
        let Some((r, pairs)) = from else { continue };
        let Some((h, _)) = robust(Model::Perspective, &pairs, 1.5) else { continue };
        let step = nearest_turn(&mul(&mul(&kinv, &h.0), &k));
        cameras[j] = Some(Pose { r: mul(&step, &r), t: [0.0; 3] });
    }
    // each point a unit away along its way from where it was first seen
    let mut points: Vec<Option<V3>> = vec![None; tracks.len()];
    for (i, t) in tracks.iter().enumerate() {
        let Some(p) = cameras[t.first] else { continue };
        let x = ray(t.at[0], c, f);
        let d = mv(&tr(&p.r), &[x[0], x[1], 1.0]);
        points[i] = Some(d.map(|v| v / norm(d)));
    }
    for _ in 0..4 {
        for (k, slot) in cameras.iter_mut().enumerate() {
            let Some(p) = *slot else { continue };
            let obs: Vec<(V3, [f64; 2])> = tracks.iter().zip(&points).filter_map(|(t, x)| Some(((*x)?, ray(t.on(k)?, c, f)))).collect();
            if obs.len() >= 4 {
                *slot = Some(Pose { r: refine_turn(p.r, &obs, 5), t: [0.0; 3] });
            }
        }
        for (i, t) in tracks.iter().enumerate() {
            // the mean of the ways it is seen along
            let mut d = [0.0; 3];
            for (k, cam) in cameras.iter().enumerate().take(t.last() + 1).skip(t.first) {
                let (Some(p), Some(o)) = (cam, t.on(k)) else { continue };
                let x = ray(o, c, f);
                let w = mv(&tr(&p.r), &[x[0], x[1], 1.0]);
                d = add(d, w.map(|v| v / norm(w)));
            }
            if norm(d) > 1e-12 {
                points[i] = Some(d.map(|v| v / norm(d)));
            }
        }
    }
    let mut sol = Solution { focal: f, center: c, cameras, points, error: 0.0, how: Solve::Tripod };
    sol.error = mean_error(&sol, tracks);
    Some(sol)
}

/// As text to keep: a first line of the lens, the frames' first time and the time between
/// them, the error and how it was solved, then a line a camera (`c`, its frame, its turn as a
/// quaternion x y z w, its move) and a line a point (`p`, x y z).
pub fn to_text(s: &Solution, start: f64, step: f64) -> String {
    let n = |v: f64| {
        let t = format!("{v:.6}");
        let t = t.trim_end_matches('0').trim_end_matches('.').to_string();
        if t == "-0" { "0".to_string() } else { t }
    };
    let mut out = format!("camera3d 1 {} {} {} {} {} {} {}\n", n(s.focal), n(s.center[0]), n(s.center[1]), n(start), n(step), n(s.error), if s.how == Solve::Tripod { "tripod" } else { "typical" });
    for (k, c) in s.cameras.iter().enumerate() {
        if let Some(p) = c {
            let q = quat(&p.r);
            out.push_str(&format!("c {k} {} {} {} {} {} {} {}\n", n(q[0]), n(q[1]), n(q[2]), n(q[3]), n(p.t[0]), n(p.t[1]), n(p.t[2])));
        }
    }
    for x in s.points.iter().flatten() {
        out.push_str(&format!("p {} {} {}\n", n(x[0]), n(x[1]), n(x[2])));
    }
    out
}

/// A solution kept as text, and its frames' first time and the time between them.
pub fn from_text(text: &str) -> Option<(Solution, f64, f64)> {
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next()?.strip_prefix("camera3d 1 ")?.split_whitespace().collect();
    let [f, cx, cy, start, step, error, how] = head[..] else { return None };
    let num = |s: &str| s.parse::<f64>().ok();
    let mut cams: Vec<(usize, Pose)> = Vec::new();
    let mut points = Vec::new();
    for line in lines {
        let v: Vec<&str> = line.split_whitespace().collect();
        match v.first() {
            Some(&"c") if v.len() == 9 => {
                let k: usize = v[1].parse().ok()?;
                let q = [num(v[2])?, num(v[3])?, num(v[4])?, num(v[5])?];
                cams.push((k, Pose { r: from_quat(q), t: [num(v[6])?, num(v[7])?, num(v[8])?] }));
            }
            Some(&"p") if v.len() == 4 => points.push(Some([num(v[1])?, num(v[2])?, num(v[3])?])),
            None => {}
            _ => return None,
        }
    }
    let frames = cams.iter().map(|c| c.0 + 1).max().unwrap_or(0);
    let mut cameras = vec![None; frames];
    for (k, p) in cams {
        cameras[k] = Some(p);
    }
    let sol = Solution { focal: num(f)?, center: [num(cx)?, num(cy)?], cameras, points, error: num(error)?, how: if how == "tripod" { Solve::Tripod } else { Solve::Typical } };
    Some((sol, num(start)?, num(step)?))
}

/// A turn as a quaternion (x, y, z, w).
pub fn quat(r: &M3) -> [f64; 4] {
    let t = r[0][0] + r[1][1] + r[2][2];
    let q = if t > 0.0 {
        let s = (t + 1.0).sqrt() * 2.0;
        [(r[2][1] - r[1][2]) / s, (r[0][2] - r[2][0]) / s, (r[1][0] - r[0][1]) / s, 0.25 * s]
    } else if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        let s = (1.0 + r[0][0] - r[1][1] - r[2][2]).sqrt() * 2.0;
        [0.25 * s, (r[0][1] + r[1][0]) / s, (r[0][2] + r[2][0]) / s, (r[2][1] - r[1][2]) / s]
    } else if r[1][1] > r[2][2] {
        let s = (1.0 + r[1][1] - r[0][0] - r[2][2]).sqrt() * 2.0;
        [(r[0][1] + r[1][0]) / s, 0.25 * s, (r[1][2] + r[2][1]) / s, (r[0][2] - r[2][0]) / s]
    } else {
        let s = (1.0 + r[2][2] - r[0][0] - r[1][1]).sqrt() * 2.0;
        [(r[0][2] + r[2][0]) / s, (r[1][2] + r[2][1]) / s, 0.25 * s, (r[1][0] - r[0][1]) / s]
    };
    let l = (q.iter().map(|v| v * v).sum::<f64>()).sqrt();
    q.map(|v| v / l)
}

/// A quaternion (x, y, z, w) as a turn.
pub fn from_quat(q: [f64; 4]) -> M3 {
    let l = (q.iter().map(|v| v * v).sum::<f64>()).sqrt().max(1e-300);
    let [x, y, z, w] = q.map(|v| v / l);
    [[1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)], [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)], [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)]]
}

#[cfg(test)]
mod tests;
