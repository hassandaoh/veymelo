//! Many small features followed from one frame to the next, for working out how the whole
//! picture moved: corners found where the picture changes both ways (the smaller eigenvalue of
//! the structure tensor, as Shi and Tomasi pick them), spread over the frame, each followed by
//! pyramidal Lucas–Kanade (Bouguet's scheme: coarse to fine, a few Gauss–Newton steps on each
//! level) and kept only when following it back lands where it began.

use veymelo_raster::Image;

use crate::plane::{Channel, Plane};

/// A frame's brightness at a size worth analysing, and its pyramid.
pub struct Gray {
    /// How many frame pixels one of its pixels stands for.
    pub step: f64,
    /// Where the frame's layer (0, 0) is in the frame's pixels.
    pub offset: [f64; 2],
    levels: Vec<Plane>,
    slopes: Vec<(Plane, Plane)>,
}

/// About how big the frames are looked at (the longer side), however big they are.
pub const LOOK_SIZE: f64 = 640.0;

impl Gray {
    /// `img` (whose layer (0, 0) is at `offset`) brought to about [`LOOK_SIZE`] across, its
    /// brightness, in `levels` + 1 levels.
    pub fn new(img: &Image, offset: [f64; 2], levels: usize) -> Gray {
        let step = (img.width.max(img.height) as f64 / LOOK_SIZE).max(1.0);
        let (w, h) = (((img.width as f64) / step).round().max(1.0) as usize, ((img.height as f64) / step).round().max(1.0) as usize);
        let full = Plane::crop(img, Channel::Luminance, 0, 0, img.width, img.height);
        let mut base = Plane::new(w, h, 1);
        // each of its pixels the mean of the frame's under it (a box as wide as the step)
        let r = (step / 2.0).max(0.5);
        for y in 0..h {
            for x in 0..w {
                let (cx, cy) = ((x as f64 + 0.5) * step, (y as f64 + 0.5) * step);
                let (x0, x1) = ((cx - r).floor() as isize, (cx + r).ceil() as isize);
                let (y0, y1) = ((cy - r).floor() as isize, (cy + r).ceil() as isize);
                let (mut sum, mut n) = (0.0, 0.0);
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        sum += full.at(sx, sy, 0);
                        n += 1.0;
                    }
                }
                base.v[y * w + x] = sum / n;
            }
        }
        let mut all = vec![base.blurred(0.7)];
        for _ in 0..levels {
            let next = all.last().map(Plane::halved).unwrap_or_default();
            all.push(next);
        }
        let slopes = all.iter().map(Plane::slopes).collect();
        Gray { step, offset, levels: all, slopes }
    }

    /// From its pixels to the layer's.
    pub fn to_layer(&self, p: [f64; 2]) -> [f64; 2] {
        [p[0] * self.step - self.offset[0], p[1] * self.step - self.offset[1]]
    }

    pub fn width(&self) -> usize {
        self.levels[0].w
    }

    pub fn height(&self) -> usize {
        self.levels[0].h
    }
}

/// Up to `most` corners of `g` (in its pixels), strongest first, none nearer than `apart`
/// pixels to a stronger one, and none weaker than `quality` of the strongest.
pub fn corners(g: &Gray, most: usize, apart: f64, quality: f64) -> Vec<[f64; 2]> {
    let (p, (gx, gy)) = (&g.levels[0], &g.slopes[0]);
    let (w, h) = (p.w, p.h);
    let mut strength = vec![0.0f32; w * h];
    let edge = 4;
    for y in edge..h.saturating_sub(edge) {
        for x in edge..w.saturating_sub(edge) {
            let (mut a, mut b, mut c) = (0.0f32, 0.0f32, 0.0f32);
            for dy in -2isize..=2 {
                for dx in -2isize..=2 {
                    let i = (y as isize + dy) as usize * w + (x as isize + dx) as usize;
                    let (u, v) = (gx.v[i], gy.v[i]);
                    (a, b, c) = (a + u * u, b + u * v, c + v * v);
                }
            }
            // the smaller eigenvalue of [[a, b], [b, c]]
            strength[y * w + x] = (a + c) / 2.0 - (((a - c) / 2.0).powi(2) + b * b).sqrt();
        }
    }
    let top = strength.iter().copied().fold(0.0f32, f32::max);
    if top <= 0.0 {
        return Vec::new();
    }
    let floor = top * quality as f32;
    let mut found: Vec<(f32, usize)> = (0..w * h)
        .filter(|&i| {
            let s = strength[i];
            if s < floor {
                return false;
            }
            // the strongest round itself
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            (-1..=1).all(|dy| (-1..=1).all(|dx| (dx == 0 && dy == 0) || strength[((y + dy).clamp(0, h as isize - 1) as usize) * w + (x + dx).clamp(0, w as isize - 1) as usize] <= s))
        })
        .map(|i| (strength[i], i))
        .collect();
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    // a grid of cells `apart` wide, to keep them apart quickly
    let cell = apart.max(1.0);
    let (cw, ch) = ((w as f64 / cell).ceil() as usize + 1, (h as f64 / cell).ceil() as usize + 1);
    let mut grid: Vec<Vec<[f64; 2]>> = vec![Vec::new(); cw * ch];
    let mut out = Vec::new();
    for (_, i) in found {
        let q = [(i % w) as f64 + 0.5, (i / w) as f64 + 0.5];
        let (gxc, gyc) = ((q[0] / cell) as usize, (q[1] / cell) as usize);
        let near = (gyc.saturating_sub(1)..=(gyc + 1).min(ch - 1)).any(|yy| (gxc.saturating_sub(1)..=(gxc + 1).min(cw - 1)).any(|xx| grid[yy * cw + xx].iter().any(|o| (o[0] - q[0]).hypot(o[1] - q[1]) < apart)));
        if !near {
            grid[gyc * cw + gxc].push(q);
            out.push(q);
            if out.len() >= most {
                break;
            }
        }
    }
    out
}

/// Where the point `p` of `from` is in `to` (their pixels), by pyramidal Lucas–Kanade with a
/// window `r` pixels either side; none when it cannot be followed.
fn follow_one(from: &Gray, to: &Gray, p: [f64; 2], r: i32) -> Option<[f64; 2]> {
    let top = from.levels.len() - 1;
    let mut guess = [0.0f64; 2];
    let mut v = [0.0f32; 3];
    let mut q = [0.0f32; 3];
    for k in (0..=top).rev() {
        let s = (1u32 << k) as f64;
        let (a, b) = (&from.levels[k], &to.levels[k]);
        let (gx, gy) = &from.slopes[k];
        let pk = [p[0] / s, p[1] / s];
        // the window's slopes, fixed for this level
        let (mut sxx, mut sxy, mut syy) = (0.0f64, 0.0f64, 0.0f64);
        let n = (2 * r + 1) as usize;
        let mut window = Vec::with_capacity(n * n);
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (pk[0] + dx as f64, pk[1] + dy as f64);
                a.read(x, y, &mut v);
                let mut ix = [0.0f32; 3];
                let mut iy = [0.0f32; 3];
                gx.read(x, y, &mut ix);
                gy.read(x, y, &mut iy);
                let (ix, iy) = (ix[0] as f64, iy[0] as f64);
                (sxx, sxy, syy) = (sxx + ix * ix, sxy + ix * iy, syy + iy * iy);
                window.push((v[0] as f64, ix, iy));
            }
        }
        let det = sxx * syy - sxy * sxy;
        let small = (sxx + syy) / 2.0 - (((sxx - syy) / 2.0).powi(2) + sxy * sxy).sqrt();
        if det.abs() < 1e-12 || small / ((n * n) as f64) < 1e-7 {
            return None;
        }
        let mut d = [0.0f64; 2];
        for _ in 0..20 {
            let (mut bx, mut by) = (0.0f64, 0.0f64);
            let mut i = 0;
            for dy in -r..=r {
                for dx in -r..=r {
                    let (x, y) = (pk[0] + guess[0] + d[0] + dx as f64, pk[1] + guess[1] + d[1] + dy as f64);
                    b.read(x, y, &mut q);
                    let (val, ix, iy) = window[i];
                    let e = val - q[0] as f64;
                    (bx, by) = (bx + e * ix, by + e * iy);
                    i += 1;
                }
            }
            let step = [(syy * bx - sxy * by) / det, (sxx * by - sxy * bx) / det];
            d = [d[0] + step[0], d[1] + step[1]];
            if step[0].hypot(step[1]) < 0.01 {
                break;
            }
        }
        guess = if k > 0 { [(guess[0] + d[0]) * 2.0, (guess[1] + d[1]) * 2.0] } else { [guess[0] + d[0], guess[1] + d[1]] };
    }
    let at = [p[0] + guess[0], p[1] + guess[1]];
    let inside = at[0] >= 1.0 && at[1] >= 1.0 && at[0] < to.width() as f64 - 1.0 && at[1] < to.height() as f64 - 1.0;
    (inside && at[0].is_finite() && at[1].is_finite()).then_some(at)
}

/// Each of `points` of `from` followed into `to` (their pixels), kept where following it back
/// lands within a pixel of where it began.
pub fn follow(from: &Gray, to: &Gray, points: &[[f64; 2]]) -> Vec<Option<[f64; 2]>> {
    let r = 7;
    let one = |p: [f64; 2]| {
        let there = follow_one(from, to, p, r)?;
        let back = follow_one(to, from, there, r)?;
        ((back[0] - p[0]).hypot(back[1] - p[1]) < 1.0).then_some(there)
    };
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(points.len().max(1));
    if threads < 2 || points.len() < 64 {
        return points.iter().map(|p| one(*p)).collect();
    }
    let per = points.len().div_ceil(threads);
    std::thread::scope(|s| {
        let one = &one;
        let parts: Vec<_> = points.chunks(per).map(|c| s.spawn(move || c.iter().map(|p| one(*p)).collect::<Vec<_>>())).collect();
        parts.into_iter().flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
    })
}
