//! Filters over whole pictures (premultiplied, four channels): Gaussian and box blurs, blurs
//! along a direction, round a centre or toward it. Every width may be fractional and changes the
//! picture smoothly, so a blur that grows over time grows evenly.

use crate::{Image, Px, par};

/// What lies past a picture's edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Nothing: a blur fades out at the edges (pad the picture first to let it spread).
    Clear,
    /// The edge pixels, going on forever.
    Repeat,
}

/// Something done to a line of pixels (a row or a column).
type LineFn = dyn Fn(&mut [Px]) + Sync;

/// The picture turned on its diagonal (rows become columns).
fn turned(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let mut out = Image::new(h, w);
    for y in 0..h {
        for x in 0..w {
            out.px[x * h + y] = img.px[y * w + x];
        }
    }
    out
}

/// Run `f` on every row, then (when `columns`) on every column.
fn each_line(img: &mut Image, rows: Option<&LineFn>, columns: Option<&LineFn>) {
    if let Some(f) = rows {
        let w = img.width;
        par::rows(&mut img.px, w, |_, row| f(row));
    }
    if let Some(f) = columns {
        let mut t = turned(img);
        let h = t.width;
        par::rows(&mut t.px, h, |_, col| f(col));
        *img = turned(&t);
    }
}

/// A line blurred by a box `width` pixels wide (fractional ends count for the part they cover).
fn box_line(line: &mut [Px], width: f64, edge: Edge) {
    let n = line.len();
    if n == 0 || width <= 1.0 + 1e-9 {
        return;
    }
    let reach = (width / 2.0).ceil() as i64 + 1;
    // sums from the start, over the line with `reach` more on each side
    let at = |j: i64| -> Px {
        match edge {
            Edge::Clear if j < 0 || j >= n as i64 => [0.0; 4],
            _ => line[j.clamp(0, n as i64 - 1) as usize],
        }
    };
    let total = n + 2 * reach as usize;
    let mut sum = vec![[0f64; 4]; total + 1];
    for k in 0..total {
        let p = at(k as i64 - reach);
        for c in 0..4 {
            sum[k + 1][c] = sum[k][c] + p[c] as f64;
        }
    }
    let run = |a: i64, b: i64, c: usize| -> f64 {
        let (a, b) = ((a + reach).clamp(0, total as i64) as usize, (b + reach).clamp(0, total as i64) as usize);
        if b > a { sum[b][c] - sum[a][c] } else { 0.0 }
    };
    let out: Vec<Px> = (0..n as i64)
        .map(|i| {
            let (a, b) = (i as f64 + 0.5 - width / 2.0, i as f64 + 0.5 + width / 2.0);
            let (fa, fb) = (a.floor(), b.floor());
            let (pa, pb) = (at(fa as i64), at(fb as i64));
            let mut o = [0f32; 4];
            for c in 0..4 {
                let v = (fa + 1.0 - a) * pa[c] as f64 + run(fa as i64 + 1, fb as i64, c) + (b - fb) * pb[c] as f64;
                o[c] = (v / width) as f32;
            }
            o
        })
        .collect();
    line.copy_from_slice(&out);
}

/// A line blurred by a Gaussian of deviation `sigma` (the exact bell, cut at four deviations).
fn bell_line(line: &mut [Px], weights: &[f32], edge: Edge) {
    let n = line.len() as i64;
    let r = weights.len() as i64 / 2;
    let src = line.to_vec();
    for i in 0..n {
        let mut o = [0f32; 4];
        for (k, w) in weights.iter().enumerate() {
            let j = i + k as i64 - r;
            let p = match edge {
                Edge::Clear if j < 0 || j >= n => continue,
                _ => src[j.clamp(0, n - 1) as usize],
            };
            for c in 0..4 {
                o[c] += p[c] * w;
            }
        }
        line[i as usize] = o;
    }
}

/// The weights of a Gaussian of deviation `sigma`, summing to one.
fn bell(sigma: f64) -> Vec<f32> {
    let r = (sigma * 4.0).ceil().max(1.0) as i64;
    // each weight is the bell's area over its pixel, so a small deviation is not lost between samples
    let cdf = |x: f64| 0.5 * (1.0 + erf(x / (sigma * std::f64::consts::SQRT_2)));
    let w: Vec<f64> = (-r..=r).map(|k| cdf(k as f64 + 0.5) - cdf(k as f64 - 0.5)).collect();
    let total: f64 = w.iter().sum();
    w.iter().map(|v| (v / total) as f32).collect()
}

#[cfg(test)]
pub(crate) fn bell_weights(sigma: f64) -> Vec<f32> {
    bell(sigma)
}

/// The error function (Abramowitz and Stegun 7.1.26, good to 1.5e-7).
fn erf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let y = 1.0 - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t + 0.254_829_592) * t * (-x * x).exp();
    if x < 0.0 { -y } else { y }
}

/// Above this deviation a Gaussian is three boxes (the same to the eye, and much quicker).
const EXACT_MOST: f64 = 12.0;

fn gauss_line(sigma: f64, edge: Edge) -> Box<LineFn> {
    if sigma <= EXACT_MOST {
        let w = bell(sigma);
        Box::new(move |l: &mut [Px]| bell_line(l, &w, edge))
    } else {
        // three boxes of width 2σ have the bell's spread
        let width = 2.0 * sigma;
        Box::new(move |l: &mut [Px]| {
            for _ in 0..3 {
                box_line(l, width, edge);
            }
        })
    }
}

/// The picture blurred by a Gaussian of deviations `sx` across and `sy` down, in pixels.
pub fn gaussian(img: &Image, sx: f64, sy: f64, edge: Edge) -> Image {
    if let Some(out) = crate::gpu::gaussian(img, sx, sy, edge, &|sigma| (sigma <= EXACT_MOST).then(|| bell(sigma))) {
        return out;
    }
    let mut out = img.clone();
    let rows = (sx > 0.01).then(|| gauss_line(sx, edge));
    let cols = (sy > 0.01).then(|| gauss_line(sy, edge));
    each_line(&mut out, rows.as_deref(), cols.as_deref());
    out
}

/// The picture blurred `passes` times by boxes `wx` and `wy` pixels wide.
pub fn boxes(img: &Image, wx: f64, wy: f64, passes: usize, edge: Edge) -> Image {
    if let Some(out) = crate::gpu::boxes(img, wx, wy, passes, edge) {
        return out;
    }
    let mut out = img.clone();
    let line = |width: f64| -> Box<LineFn> {
        Box::new(move |l: &mut [Px]| {
            for _ in 0..passes {
                box_line(l, width, edge);
            }
        })
    };
    let rows = (wx > 1.0).then(|| line(wx));
    let cols = (wy > 1.0).then(|| line(wy));
    each_line(&mut out, rows.as_deref(), cols.as_deref());
    out
}

/// Each pixel the mean of the picture along a line through it: `dir` (a unit step in pixels)
/// times every distance from -`length`/2 to `length`/2.
pub fn along(img: &Image, dir: (f64, f64), length: f64) -> Image {
    if length < 0.5 {
        return img.clone();
    }
    let n = (length.ceil() as usize * 2 + 1).clamp(3, 1025);
    if let Some(out) = crate::gpu::along(img, dir, length, n) {
        return out;
    }
    let mut out = Image::new(img.width, img.height);
    let w = img.width;
    par::rows(&mut out.px, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let mut acc = [0f32; 4];
            for i in 0..n {
                let t = (i as f64 / (n - 1) as f64 - 0.5) * length;
                let p = img.sample(crate::Sampling::Bilinear, x as f64 + 0.5 + dir.0 * t, y as f64 + 0.5 + dir.1 * t);
                for c in 0..4 {
                    acc[c] += p[c];
                }
            }
            *px = acc.map(|v| v / n as f32);
        }
    });
    out
}

/// Each pixel the mean of the picture round `centre` (turned from -`degrees`/2 to `degrees`/2)
/// or, with `zoom`, along the line to it (scaled from 1 - `degrees`/2 to 1 + `degrees`/2 of its
/// distance). As many samples as the farthest of them moves pixels, so long blurs stay smooth.
pub fn round(img: &Image, centre: (f64, f64), amount: f64, zoom: bool) -> Image {
    if amount.abs() < 1e-9 {
        return img.clone();
    }
    if let Some(out) = crate::gpu::round(img, centre, amount, zoom) {
        return out;
    }
    let mut out = Image::new(img.width, img.height);
    let w = img.width;
    par::rows(&mut out.px, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let (vx, vy) = (x as f64 + 0.5 - centre.0, y as f64 + 0.5 - centre.1);
            let travel = if zoom { vx.hypot(vy) * amount.abs() } else { vx.hypot(vy) * amount.abs().to_radians() };
            let n = (travel.ceil() as usize + 1).clamp(2, 512);
            let mut acc = [0f32; 4];
            for i in 0..n {
                let t = i as f64 / (n - 1) as f64 - 0.5;
                let (sx, sy) = if zoom {
                    let k = 1.0 + t * amount;
                    (centre.0 + vx * k, centre.1 + vy * k)
                } else {
                    let (s, c) = (t * amount).to_radians().sin_cos();
                    (centre.0 + vx * c - vy * s, centre.1 + vx * s + vy * c)
                };
                let p = img.sample(crate::Sampling::Bilinear, sx, sy);
                for c in 0..4 {
                    acc[c] += p[c];
                }
            }
            *px = acc.map(|v| v / n as f32);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot() -> Image {
        let mut img = Image::new(64, 64);
        img.px[32 * 64 + 32] = [1.0; 4];
        img
    }

    fn mass(img: &Image) -> f32 {
        img.px.iter().map(|p| p[3]).sum()
    }

    /// The spread of a blurred dot (in the middle of the picture) across.
    fn spread(img: &Image) -> f64 {
        let (mut m, mut v) = (0.0, 0.0);
        let mid = (img.width / 2) as f64 + 0.5;
        for (i, p) in img.px.iter().enumerate() {
            let x = (i % img.width) as f64 + 0.5 - mid;
            m += p[3] as f64;
            v += p[3] as f64 * x * x;
        }
        (v / m).sqrt()
    }

    #[test]
    fn blurs_keep_what_there_is_and_spread_it_as_asked() {
        // room enough round the dot for the widest blur
        let mut big = Image::new(256, 256);
        big.px[128 * 256 + 128] = [1.0; 4];
        for s in [0.6, 2.0, 7.5, 20.0] {
            let b = gaussian(&big, s, s, Edge::Clear);
            assert!((mass(&b) - 1.0).abs() < 2e-3, "σ {s}: {}", mass(&b));
            assert!((spread(&b) - s).abs() < 0.12 * s.max(1.0), "σ {s}: spread {}", spread(&b));
        }
        let flat = Image::filled(20, 10, [0.5, 0.5, 0.5, 1.0]);
        let b = gaussian(&flat, 3.0, 30.0, Edge::Repeat);
        assert!(b.px.iter().all(|p| (p[0] - 0.5).abs() < 1e-5 && (p[3] - 1.0).abs() < 1e-5));
        let edge = gaussian(&flat, 3.0, 0.0, Edge::Clear);
        assert!(edge.px[0][3] < 0.75 && edge.px[10][3] > 0.99, "fades at the edge");
    }

    #[test]
    fn a_growing_blur_grows_evenly() {
        // the centre of a blurred dot falls steadily as the blur grows, with no steps
        let mut last = f32::MAX;
        for k in 0..60 {
            let s = 0.5 + k as f64 * 0.5;
            let c = gaussian(&dot(), s, s, Edge::Clear).px[32 * 64 + 32][3];
            assert!(c < last, "σ {s}");
            last = c;
        }
        let b = boxes(&dot(), 5.0, 5.0, 3, Edge::Clear);
        assert!((mass(&b) - 1.0).abs() < 1e-3);
        assert!(boxes(&dot(), 5.5, 5.5, 3, Edge::Clear).px[32 * 64 + 32][3] < b.px[32 * 64 + 32][3]);
    }

    #[test]
    fn along_a_direction_and_round_a_centre() {
        let b = along(&dot(), (1.0, 0.0), 10.0);
        assert!(b.px[32 * 64 + 36][3] > 0.0 && b.px[36 * 64 + 32][3] == 0.0, "only across");
        let spun = round(&dot(), (32.0, 22.5), 90.0, false);
        assert!(spun.px[(32 - 3) * 64 + 32 + 7][3] > 0.0, "carried round the centre");
        let zoomed = round(&dot(), (22.5, 32.5), 0.5, true);
        assert!(zoomed.px[32 * 64 + 34][3] > 0.0 && zoomed.px[32 * 64 + 31][3] > 0.0 && zoomed.px[32 * 64 + 40][3] == 0.0);
    }
}
