//! How the picture moved between two frames (optical flow): a grid of vectors, each found by
//! matching the block of pixels round it in the first frame against the second, coarse to fine
//! (halved copies first, so big moves are found), finished to a part of a pixel.

use crate::par;

/// Where each block went: vectors `step` pixels apart, `cols` × `rows` of them, the first
/// centred at (`step` / 2, `step` / 2), and how sure each is across and down, 0 to 1 (a flat
/// block matches anywhere: 0; a line standing up says how far it went across, not down).
#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    pub cols: usize,
    pub rows: usize,
    pub step: f64,
    pub v: Vec<[f32; 2]>,
    pub sure: Vec<[f32; 2]>,
}

impl Flow {
    /// The move at a pixel (its middle at .5), between the vectors round it.
    pub fn at(&self, x: f64, y: f64) -> [f32; 2] {
        if self.v.is_empty() {
            return [0.0; 2];
        }
        let (fx, fy) = ((x / self.step - 0.5).clamp(0.0, (self.cols - 1) as f64), (y / self.step - 0.5).clamp(0.0, (self.rows - 1) as f64));
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.cols - 1), (y0 + 1).min(self.rows - 1));
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let g = |i: usize, j: usize| self.v[j * self.cols + i];
        let l = |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        l(l(g(x0, y0), g(x1, y0), tx), l(g(x0, y1), g(x1, y1), tx), ty)
    }
}

/// A grey picture.
#[derive(Clone, Debug)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub v: Vec<f32>,
}

impl Plane {
    /// Its value between pixels (middles at .5), its edge going on.
    fn at(&self, x: f64, y: f64) -> f32 {
        let (fx, fy) = ((x - 0.5).clamp(0.0, (self.w - 1) as f64), (y - 0.5).clamp(0.0, (self.h - 1) as f64));
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let g = |i: usize, j: usize| self.v[j * self.w + i];
        let top = g(x0, y0) + (g(x1, y0) - g(x0, y0)) * tx;
        let bottom = g(x0, y1) + (g(x1, y1) - g(x0, y1)) * tx;
        top + (bottom - top) * ty
    }

    fn get(&self, x: i64, y: i64) -> f32 {
        self.v[(y.clamp(0, self.h as i64 - 1) as usize) * self.w + x.clamp(0, self.w as i64 - 1) as usize]
    }

    /// Half the size, each pixel the mean of four.
    fn half(&self) -> Plane {
        let (w, h) = (self.w.div_ceil(2).max(1), self.h.div_ceil(2).max(1));
        let mut v = vec![0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (2 * x as i64, 2 * y as i64);
                v[y * w + x] = (self.get(sx, sy) + self.get(sx + 1, sy) + self.get(sx, sy + 1) + self.get(sx + 1, sy + 1)) * 0.25;
            }
        }
        Plane { w, h, v }
    }
}

/// How unlike the block `size` across at `p` in `a` is to the one at `p + d` in `b` (the mean
/// squared difference, the blocks' own mean brightness taken off when `level_out`).
fn unlike(a: &Plane, b: &Plane, p: (f64, f64), d: (f64, f64), size: usize) -> f32 {
    let half = size as f64 * 0.5;
    let (mut s, mut n) = (0f32, 0f32);
    for j in 0..size {
        for i in 0..size {
            let (x, y) = (p.0 - half + i as f64 + 0.5, p.1 - half + j as f64 + 0.5);
            let e = a.at(x, y) - b.at(x + d.0, y + d.1);
            s += e * e;
            n += 1.0;
        }
    }
    s / n.max(1.0)
}

/// The flow from `a` to `b` (grey pictures the same size): a vector every `step` pixels, each
/// matching a block `window` across, finding moves up to about `reach` pixels.
pub fn measure(a: &Plane, b: &Plane, step: usize, window: usize, reach: usize) -> Flow {
    let step = step.max(1);
    let (cols, rows) = (a.w.div_ceil(step).max(1), a.h.div_ceil(step).max(1));
    // halved copies until the reach is a few pixels at the coarsest, keeping enough picture there
    let mut pyr = vec![(a.clone(), b.clone())];
    while pyr.len() < 7 {
        let (pa, pb) = &pyr[pyr.len() - 1];
        if (reach >> (pyr.len() - 1)) <= 4 || pa.w.min(pa.h) < 48 {
            break;
        }
        pyr.push((pa.half(), pb.half()));
    }
    let top = pyr.len() - 1;
    let mut v = vec![[0f32; 2]; cols * rows];
    let mut sure = vec![[0f32; 2]; cols * rows];
    for level in (0..=top).rev() {
        let (pa, pb) = &pyr[level];
        let k = (1usize << level) as f64;
        let size = (window as f64 / k).round().max(5.0) as usize;
        // search the whole reach at the coarsest, just round the best guess below it
        let r: i64 = if level == top { ((reach as f64 / k).ceil() as i64 + 1).clamp(2, 16) } else { 2 };
        let guesses = v.clone();
        let mut found = vec![([0f32; 2], [0f32; 2]); cols * rows];
        par::rows(&mut found, cols, |j, row| {
            for (i, out) in row.iter_mut().enumerate() {
                let p = (((i as f64 + 0.5) * step as f64) / k, ((j as f64 + 0.5) * step as f64) / k);
                // its own guess, or a neighbour's if that fits better (so a wrong one is put right)
                let mut g = guesses[j * cols + i];
                if level < top {
                    let fit = |c: [f32; 2]| unlike(pa, pb, p, (c[0] as f64 / k, c[1] as f64 / k), size);
                    let mut best = fit(g);
                    for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                        let (ni, nj) = (i as i64 + di, j as i64 + dj);
                        if ni >= 0 && nj >= 0 && (ni as usize) < cols && (nj as usize) < rows {
                            let c = guesses[nj as usize * cols + ni as usize];
                            let e = fit(c);
                            if e < best {
                                (best, g) = (e, c);
                            }
                        }
                    }
                }
                let g = (g[0] as f64 / k, g[1] as f64 / k);
                let (mut best, mut at) = (f32::MAX, (0i64, 0i64));
                for dy in -r..=r {
                    for dx in -r..=r {
                        let c = unlike(pa, pb, p, (g.0 + dx as f64, g.1 + dy as f64), size) + 1e-6 * (dx * dx + dy * dy) as f32;
                        if c < best {
                            (best, at) = (c, (dx, dy));
                        }
                    }
                }
                let (mut fx, mut fy) = (g.0 + at.0 as f64, g.1 + at.1 as f64);
                let mut sure = [0f32; 2];
                if level == 0 {
                    let cost = |dx: f64, dy: f64| unlike(pa, pb, p, (fx + dx, fy + dy), size) as f64;
                    let c0 = unlike(pa, pb, p, (fx, fy), size) as f64;
                    let (cl, cr, cu, cd) = (cost(-1.0, 0.0), cost(1.0, 0.0), cost(0.0, -1.0), cost(0.0, 1.0));
                    // how sharply the cost rises either side, for how well the block matches at
                    // all: sure where it rises well clear of what cannot be matched (noise)
                    let rise = |m: f64, p: f64| {
                        let r = (m + p - 2.0 * c0).max(0.0);
                        // the cost at the parabola's low point, between the whole pixels
                        let low = if r > 1e-12 { (c0 - (m - p) * (m - p) / (8.0 * r)).max(0.0) } else { c0.max(0.0) };
                        (r / (r + low + 1e-5)) as f32
                    };
                    sure = [rise(cl, cr), rise(cu, cd)];
                    if c0 > 1e-9 {
                        // a part of a pixel: the low point of a parabola through the costs either side
                        let sub = |m: f64, z: f64, p: f64| {
                            let den = m - 2.0 * z + p;
                            if den > 1e-12 { ((m - p) / (2.0 * den)).clamp(-0.5, 0.5) } else { 0.0 }
                        };
                        fx += sub(cl, c0, cr);
                        fy += sub(cu, c0, cd);
                    }
                }
                *out = ([(fx * k) as f32, (fy * k) as f32], sure);
            }
        });
        for (k, (m, s)) in found.into_iter().enumerate() {
            v[k] = m;
            sure[k] = s;
        }
    }
    Flow { cols, rows, step: step as f64, v, sure }
}

/// A move for every pixel, and whether the picture there says it (a flat patch looks the same
/// whichever way it went: its move is only a guess from round it).
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub w: usize,
    pub h: usize,
    pub v: Vec<[f32; 2]>,
    pub sure: Vec<bool>,
}

impl Field {
    fn index(&self, x: f64, y: f64) -> usize {
        let i = (x.floor() as i64).clamp(0, self.w as i64 - 1) as usize;
        let j = (y.floor() as i64).clamp(0, self.h as i64 - 1) as usize;
        j * self.w + i
    }

    /// The move of the pixel a point is in (past the edge, the edge pixel's).
    pub fn at(&self, x: f64, y: f64) -> [f32; 2] {
        self.v[self.index(x, y)]
    }

    /// Whether the picture says the move of the pixel a point is in.
    pub fn sure_at(&self, x: f64, y: f64) -> bool {
        self.sure[self.index(x, y)]
    }
}

/// Each pixel's own move from `a` to `b` (a picture's channels, one or up to three): of the moves `raw` found round it (each as found, and
/// as far as it is sure of it with the rest from `smooth`, the same blocks smoothed) and the
/// smoothed one there, the one under which a small patch round the pixel matches best (the
/// nearest the smoothed one when they match alike). The patch is 7 by 7 pixels `spacing` apart
/// (more apart in a picture drawn bigger, to cover as much of it). Its pixels count by how like the middle one
/// they look, so what lies across an edge (another thing, moving otherwise) hardly counts, and
/// no one pixel counts past a point (it may be hidden in `b`, matching nothing): a move stops at
/// the edge of what moves instead of fading out across a block. With `only`, just the pixels it
/// marks (the rest left still).
pub fn per_pixel(a: &[&Plane], b: &[&Plane], raw: &Flow, smooth: &Flow, spacing: usize, only: Option<&[bool]>) -> Field {
    const R: i64 = 3;
    const SIDE: usize = (2 * R + 1) as usize;
    let gap = spacing.max(1) as i64;
    // how much a patch pixel counts for how far it is from the middle
    let near: Vec<f32> = (0..SIDE * SIDE).map(|i| (-(((i % SIDE) as i64 - R).pow(2) + ((i / SIDE) as i64 - R).pow(2)) as f32 / (2 * R * R) as f32).exp()).collect();
    let (w, h, nc) = (a[0].w, a[0].h, a.len().min(b.len()).min(3));
    let mut v = vec![([0f32; 2], false); w * h];
    par::rows(&mut v, w, |y, row| {
        let mut moves: Vec<[f32; 2]> = Vec::with_capacity(19);
        let mut support = [(0i64, 0i64, [0f32; 3], 0f32); SIDE * SIDE];
        for (x, out) in row.iter_mut().enumerate() {
            if only.is_some_and(|o| !o[y * w + x]) {
                continue;
            }
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            let s = smooth.at(cx, cy);
            moves.clear();
            moves.push(s);
            let (gi, gj) = ((cx / raw.step - 0.5).round() as i64, (cy / raw.step - 0.5).round() as i64);
            for dj in -1..=1 {
                for di in -1..=1 {
                    let (i, j) = (gi + di, gj + dj);
                    if i >= 0 && j >= 0 && (i as usize) < raw.cols && (j as usize) < raw.rows {
                        // a block's move, and the same where it is sure of it (across or down) and
                        // the smoothed elsewhere
                        let k = j as usize * raw.cols + i as usize;
                        let (r, m, sure) = (raw.v[k], smooth.v[k], raw.sure[k]);
                        for c in [r, [if sure[0] >= 0.25 { r[0] } else { m[0] }, if sure[1] >= 0.25 { r[1] } else { m[1] }]] {
                            // moves within half a pixel of one already there are as good as it
                            if moves.iter().all(|q| (q[0] - c[0]).abs().max((q[1] - c[1]).abs()) > 0.5) {
                                moves.push(c);
                            }
                        }
                    }
                }
            }
            if moves.len() == 1 {
                // all round agree; sure if the picture is not flat here
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        for c in 0..nc {
                            let q = a[c].get(x as i64 + dx * gap, y as i64 + dy * gap);
                            (lo[c], hi[c]) = (lo[c].min(q), hi[c].max(q));
                        }
                    }
                }
                *out = (s, (0..nc).any(|c| hi[c] - lo[c] > 0.02));
                continue;
            }
            // the patch: where, its values, and how much it counts
            let mut mid = [0f32; 3];
            for (m, p) in mid.iter_mut().zip(&a[..nc]) {
                *m = p.get(x as i64, y as i64);
            }
            let mut n = 0;
            for dy in -R..=R {
                for dx in -R..=R {
                    let (ix, iy) = (x as i64 + dx * gap, y as i64 + dy * gap);
                    let mut q = [0f32; 3];
                    let mut unlike = 0f32;
                    for c in 0..nc {
                        q[c] = a[c].get(ix, iy);
                        unlike = unlike.max((q[c] - mid[c]).abs());
                    }
                    support[n] = (ix, iy, q, near[n] * (-unlike / 0.06).exp());
                    n += 1;
                }
            }
            let total: f32 = support[..n].iter().map(|p| p.3).sum();
            // sure if what looks like the middle pixel round it is not all one flat colour
            let spread = (0..nc).any(|c| {
                let mean = support[..n].iter().map(|p| p.3 * p.2[c]).sum::<f32>() / total;
                support[..n].iter().map(|p| p.3 * (p.2[c] - mean).powi(2)).sum::<f32>() / total > 2e-4
            });
            // how unlike the patch is to `b` under a move (stopping once past `bound`), on the
            // patch's every other pixel or all of it
            let cost = |c: [f32; 2], every: usize, bound: f32| -> f32 {
                let lean = 1e-8 * ((c[0] - s[0]).powi(2) + (c[1] - s[1]).powi(2));
                // the move's whole pixels and the same four weights between them for every sample
                let (fx, fy) = (c[0].floor(), c[1].floor());
                let (tx, ty) = (c[0] - fx, c[1] - fy);
                let (ox, oy) = (fx as i64, fy as i64);
                let wt = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
                let (mut e, mut t) = (0f32, 0f32);
                for (i, &(ix, iy, q, k)) in support[..n].iter().enumerate() {
                    if every > 1 && (!(i % SIDE).is_multiple_of(every) || !(i / SIDE).is_multiple_of(every)) {
                        continue;
                    }
                    let (x0, y0) = (ix + ox, iy + oy);
                    let inside = x0 >= 0 && y0 >= 0 && x0 + 1 < w as i64 && y0 + 1 < h as i64;
                    for ch in 0..nc {
                        let p = b[ch];
                        let v = if inside {
                            let at = y0 as usize * w + x0 as usize;
                            p.v[at] * wt[0] + p.v[at + 1] * wt[1] + p.v[at + w] * wt[2] + p.v[at + w + 1] * wt[3]
                        } else {
                            p.get(x0, y0) * wt[0] + p.get(x0 + 1, y0) * wt[1] + p.get(x0, y0 + 1) * wt[2] + p.get(x0 + 1, y0 + 1) * wt[3]
                        };
                        let d = q[ch] - v;
                        e += k * (d * d).min(0.0025);
                    }
                    t += k;
                    if every == 1 && e / total + lean >= bound {
                        return f32::MAX;
                    }
                }
                e / t.max(1e-12) + lean
            };
            // a rough look at every move, then a close one at the best few
            let mut rough = [(f32::MAX, [0f32; 2]); 19];
            for (r, c) in rough.iter_mut().zip(&moves) {
                *r = (cost(*c, 3, f32::MAX), *c);
            }
            let rough = &mut rough[..moves.len()];
            rough.sort_unstable_by(|p, q| p.0.total_cmp(&q.0));
            let mut best = (f32::MAX, s);
            for &(_, c) in rough.iter().take(3) {
                let e = cost(c, 1, best.0);
                if e < best.0 {
                    best = (e, c);
                }
            }
            *out = (best.1, spread);
        }
    });
    Field { w, h, v: v.iter().map(|p| p.0).collect(), sure: v.iter().map(|p| p.1).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture of soft random blobs (not repeating), moved by (dx, dy).
    fn texture(w: usize, h: usize, dx: f64, dy: f64) -> Plane {
        // a random number on every whole point 6 apart, smoothly between
        let cell = |i: i64, j: i64| {
            let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
            x ^= x >> 31;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x ^= x >> 29;
            (x >> 40) as f64 / (1u64 << 24) as f64
        };
        let noise = |x: f64, y: f64| {
            let (gx, gy) = (x / 6.0, y / 6.0);
            let (i, j) = (gx.floor() as i64, gy.floor() as i64);
            let (tx, ty) = (gx - i as f64, gy - j as f64);
            let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
            let top = cell(i, j) + (cell(i + 1, j) - cell(i, j)) * sx;
            let bottom = cell(i, j + 1) + (cell(i + 1, j + 1) - cell(i, j + 1)) * sx;
            top + (bottom - top) * sy
        };
        let v = (0..w * h).map(|i| noise((i % w) as f64 + 0.5 - dx, (i / w) as f64 + 0.5 - dy) as f32).collect();
        Plane { w, h, v }
    }

    #[test]
    fn finds_a_move() {
        let (a, b) = (texture(96, 64, 0.0, 0.0), texture(96, 64, 3.4, -2.2));
        let f = measure(&a, &b, 8, 9, 8);
        let m = f.at(48.0, 32.0);
        assert!((m[0] - 3.4).abs() < 0.3 && (m[1] + 2.2).abs() < 0.3, "{m:?}");
        let far = measure(&a, &texture(96, 64, 14.0, 9.0), 8, 9, 24);
        let m = far.at(48.0, 32.0);
        assert!((m[0] - 14.0).abs() < 0.5 && (m[1] - 9.0).abs() < 0.5, "a big move found coarse to fine: {m:?}");
        let still = measure(&a, &a, 8, 9, 8);
        assert!(still.v.iter().all(|v| v[0].abs() < 0.05 && v[1].abs() < 0.05));
        let mut low: Vec<f32> = f.sure.iter().map(|s| s[0].min(s[1])).collect();
        low.sort_by(f32::total_cmp);
        assert!(low[low.len() / 2] > 0.6 && low[low.len() / 20] > 0.05, "blobs mostly say where they went: {:?}", &low[..12]);
    }

    #[test]
    fn knows_how_sure_it_is() {
        // grey everywhere but a line standing up at x = 40, moved 3 to the right
        let line = |dx: f64| Plane { w: 96, h: 64, v: (0..96 * 64).map(|i| if ((i % 96) as f64 - 40.0 - dx).abs() < 1.5 { 0.9 } else { 0.3 }).collect() };
        let f = measure(&line(0.0), &line(3.0), 8, 9, 8);
        let on = f.sure[3 * f.cols + 5];
        assert!(on[0] > 0.9 && on[1] < 0.05, "on the line, sure across only: {on:?}");
        let flat = f.sure[3 * f.cols + 10];
        assert!(flat[0] < 0.05 && flat[1] < 0.05, "away from it, sure of nothing: {flat:?}");
    }

    #[test]
    fn each_pixel_its_own_move() {
        // the right half moves 5 to the right, the left half stays
        let (still, moved) = (texture(96, 64, 0.0, 0.0), texture(96, 64, 5.0, 0.0));
        let a = texture(96, 64, 0.0, 0.0);
        let mut b = a.clone();
        for y in 0..64 {
            for x in 0..96 {
                b.v[y * 96 + x] = if x >= 50 { moved.v[y * 96 + x] } else { still.v[y * 96 + x] };
            }
        }
        let raw = measure(&a, &b, 16, 9, 8);
        let f = per_pixel(&[&a], &[&b], &raw, &raw, 1, None);
        // a block's width either side of the edge, the move is one or the other, not between
        let (left, right) = (f.at(40.5, 32.5), f.at(60.5, 32.5));
        assert!(left[0].abs() < 0.3 && (right[0] - 5.0).abs() < 0.3, "{left:?} {right:?}");
        let between = f.v.iter().filter(|v| v[0] > 1.0 && v[0] < 4.0).count();
        assert!(between < f.v.len() / 50, "few pixels half moved: {between}");
    }
}
