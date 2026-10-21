//! Pictures for compositing: premultiplied RGBA in floating point (colours may go past white
//! while working), and how one is drawn onto another through a transform.

use veymelo_color::{BlendMode, Rgba, blend};
use veymelo_geom::{Mat3, Rect, vec2};

use crate::par;

/// A pixel: red, green, blue, each times alpha, and alpha.
pub type Px = [f32; 4];

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub px: Vec<Px>,
}

impl Image {
    /// A clear picture.
    pub fn new(width: usize, height: usize) -> Image {
        Image { width, height, px: vec![[0.0; 4]; width * height] }
    }

    pub fn filled(width: usize, height: usize, p: Px) -> Image {
        Image { width, height, px: vec![p; width * height] }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The pixel at (`x`, `y`); clear outside.
    #[inline]
    pub fn get(&self, x: i64, y: i64) -> Px {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 { [0.0; 4] } else { self.px[y as usize * self.width + x as usize] }
    }

    /// The colour at (`x`, `y`), not premultiplied.
    pub fn color_at(&self, x: usize, y: usize) -> Rgba {
        let [r, g, b, a] = self.px[y * self.width + x];
        if a <= 0.0 { Rgba::CLEAR } else { Rgba { r: (r / a) as f64, g: (g / a) as f64, b: (b / a) as f64, a: a as f64 } }
    }

    /// The picture sampled at (`x`, `y`) (pixel centres are at .5).
    #[inline]
    pub fn sample(&self, how: Sampling, x: f64, y: f64) -> Px {
        match how {
            Sampling::Nearest => self.get(x.floor() as i64, y.floor() as i64),
            Sampling::Bilinear => self.bilinear(x, y),
            Sampling::Bicubic => self.bicubic(x, y),
        }
    }

    fn bilinear(&self, x: f64, y: f64) -> Px {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let (a, b, c, d) = (self.get(x0, y0), self.get(x0 + 1, y0), self.get(x0, y0 + 1), self.get(x0 + 1, y0 + 1));
        let mut o = [0.0; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * tx;
            let bottom = c[i] + (d[i] - c[i]) * tx;
            o[i] = top + (bottom - top) * ty;
        }
        o
    }

    /// Catmull-Rom: sharper than bilinear; its overshoot is kept from going below zero.
    fn bicubic(&self, x: f64, y: f64) -> Px {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let weights = |t: f64| {
            let (t2, t3) = (t * t, t * t * t);
            [(-0.5 * t3 + t2 - 0.5 * t) as f32, (1.5 * t3 - 2.5 * t2 + 1.0) as f32, (-1.5 * t3 + 2.0 * t2 + 0.5 * t) as f32, (0.5 * t3 - 0.5 * t2) as f32]
        };
        let (wx, wy) = (weights(fx - x0), weights(fy - y0));
        let mut o = [0.0f32; 4];
        for (j, wyj) in wy.iter().enumerate() {
            for (i, wxi) in wx.iter().enumerate() {
                let p = self.get(x0 as i64 - 1 + i as i64, y0 as i64 - 1 + j as i64);
                let k = wxi * wyj;
                for c in 0..4 {
                    o[c] += p[c] * k;
                }
            }
        }
        let a = o[3].clamp(0.0, f32::MAX);
        [o[0].clamp(0.0, f32::MAX), o[1].clamp(0.0, f32::MAX), o[2].clamp(0.0, f32::MAX), a]
    }

    /// Half the size, each pixel the mean of four (for drawing much smaller without shimmer).
    pub fn half(&self) -> Image {
        let (w, h) = (self.width.div_ceil(2).max(1), self.height.div_ceil(2).max(1));
        let mut out = Image::new(w, h);
        par::rows(&mut out.px, w, |y, row| {
            for (x, p) in row.iter_mut().enumerate() {
                let mut acc = [0.0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let q = self.get((2 * x + dx) as i64, (2 * y + dy) as i64);
                    for c in 0..4 {
                        acc[c] += q[c];
                    }
                }
                *p = acc.map(|v| v * 0.25);
            }
        });
        out
    }

    /// Every channel held to 0…1 and colour within alpha (working in 8 or 16 bits).
    pub fn clamp(&mut self) {
        crate::par::chunks(&mut self.px, 1 << 14, |_, part| {
            for p in part {
                p[3] = p[3].clamp(0.0, 1.0);
                for c in 0..3 {
                    p[c] = p[c].clamp(0.0, p[3]);
                }
            }
        });
    }

    /// Within columns and rows `area` (x0, y0, x1, y1): every channel held as `clamp` holds it,
    /// then rounded as `quantize` rounds it (what the two do to the whole picture, there only).
    pub fn settle_in(&mut self, levels: f32, area: [usize; 4]) {
        let [x0, y0, x1, y1] = [area[0].min(self.width), area[1].min(self.height), area[2].min(self.width), area[3].min(self.height)];
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let w = self.width;
        crate::par::rows(&mut self.px[y0 * w..y1 * w], w, |_, row| {
            for p in &mut row[x0..x1] {
                p[3] = p[3].clamp(0.0, 1.0);
                for c in 0..3 {
                    p[c] = p[c].clamp(0.0, p[3]);
                }
                for c in p.iter_mut() {
                    *c = (*c * levels).round() / levels;
                }
            }
        });
    }

    /// Every channel rounded to `levels` steps (255 for 8 bits).
    pub fn quantize(&mut self, levels: f32) {
        crate::par::chunks(&mut self.px, 1 << 14, |_, part| {
            for p in part {
                for c in p.iter_mut() {
                    *c = (*c * levels).round() / levels;
                }
            }
        });
    }

    /// Eight bits a channel, not premultiplied.
    pub fn to_rgba8(&self) -> Vec<u8> {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut out = Vec::with_capacity(self.px.len() * 4);
        for &[r, g, b, a] in &self.px {
            if a <= 0.0 {
                out.extend([0, 0, 0, 0]);
            } else if a == 1.0 {
                // (a colour over one is itself)
                out.extend([byte(r), byte(g), byte(b), 255]);
            } else {
                out.extend([byte(r / a), byte(g / a), byte(b / a), byte(a)]);
            }
        }
        out
    }

    /// From eight bits a channel, not premultiplied.
    pub fn from_rgba8(width: usize, height: usize, rgba: &[u8]) -> Image {
        let px = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| {
                let a = c[3] as f32 / 255.0;
                [c[0] as f32 / 255.0 * a, c[1] as f32 / 255.0 * a, c[2] as f32 / 255.0 * a, a]
            })
            .collect();
        Image { width, height, px }
    }

    /// Change every colour (not premultiplied), keeping alpha.
    pub fn map_colors(&mut self, f: impl Fn(f32) -> f32 + Sync) {
        par::rows(&mut self.px, self.width, |_, row| {
            for p in row {
                if p[3] > 0.0 {
                    for c in 0..3 {
                        p[c] = f(p[c] / p[3]) * p[3];
                    }
                }
            }
        });
    }
}

/// How a picture is sampled between its pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sampling {
    /// The nearest pixel (draft).
    Nearest,
    #[default]
    Bilinear,
    Bicubic,
}

/// A number from 0 to 1 for each pixel and seed, the same every time (for Dissolve).
#[inline]
pub fn noise(x: usize, y: usize, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343) ^ (y as u32).wrapping_mul(0xD816_3841) ^ seed.wrapping_mul(0xCB1A_B31F);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// The columns and rows (x0, y0, x1, y1) of a `width` × `height` picture that `src` can reach
/// through `m`, with a pixel to spare for the filter (all of it for a perspective that may wrap
/// around): all that drawing it there can change.
pub fn reach(width: usize, height: usize, src: &Image, m: &Mat3) -> [usize; 4] {
    let corners = Rect::from_size(src.width as f64, src.height as f64).corners();
    let behind = !m.is_affine() && corners.iter().any(|c| m.0[2][0] * c.x + m.0[2][1] * c.y + m.0[2][2] <= 0.0);
    if behind {
        return [0, 0, width, height];
    }
    let r = m.map_rect(&Rect::from_size(src.width as f64, src.height as f64));
    let clamp = |v: f64, hi: usize| v.clamp(0.0, hi as f64) as usize;
    [clamp(r.x0.floor() - 2.0, width), clamp(r.y0.floor() - 2.0, height), clamp(r.x1.ceil() + 2.0, width), clamp(r.y1.ceil() + 2.0, height)]
}

/// Visit each pixel of `dst` that `src` drawn through `m` (its pixels to `dst`'s) lands on, with
/// the sample there. Much smaller drawings sample a halved copy, so nothing shimmers.
fn warp(dst: &mut Image, src: &Image, m: &Mat3, how: Sampling, everywhere: bool, f: impl Fn(&mut Px, Px, usize, usize) + Sync) {
    if src.is_empty() || dst.is_empty() {
        return;
    }
    let Some(mut inv) = m.inverse() else { return };
    let mut halved;
    let mut img = src;
    let mut scale = if m.is_affine() { m.mean_scale() } else { 1.0 };
    if scale < 0.5 {
        let mut f = 1.0;
        halved = src.half();
        f *= 0.5;
        scale *= 2.0;
        while scale < 0.5 && halved.width > 1 && halved.height > 1 {
            halved = halved.half();
            f *= 0.5;
            scale *= 2.0;
        }
        img = &halved;
        inv = Mat3::scale(vec2(f, f)) * inv;
    }
    let [x0, y0, x1, y1] = if everywhere { [0, 0, dst.width, dst.height] } else { reach(dst.width, dst.height, src, m) };
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let (iw, ih) = (img.width as f64, img.height as f64);
    let width = dst.width;
    let affine = inv.is_affine();
    let [r0, r1, r2] = inv.0;
    // how many of the picture's pixels one of `dst`'s spans, across and down: the same everywhere
    // unless in perspective
    let even = (r0[0].hypot(r0[1]), r1[0].hypot(r1[1]));
    par::rows(&mut dst.px[y0 * width..y1 * width], width, |row_y, row| {
        let y = y0 + row_y;
        for (x, d) in row.iter_mut().enumerate().take(x1).skip(x0) {
            let q = vec2(x as f64 + 0.5, y as f64 + 0.5);
            let p = inv.apply(q);
            let (gx, gy) = if affine {
                even
            } else {
                let w = r2[0] * q.x + r2[1] * q.y + r2[2];
                ((r0[0] - p.x * r2[0]).hypot(r0[1] - p.x * r2[1]) / w.abs(), (r1[0] - p.y * r2[0]).hypot(r1[1] - p.y * r2[1]) / w.abs())
            };
            // how much of the pixel the picture covers (its edges as sharp as the drawing is
            // big), and its colour there, its edge pixels going on
            let side = |from: f64, to: f64, g: f64| {
                let g = g.max(1e-12);
                ((from / g + 0.5).clamp(0.0, 1.0) + (to / g + 0.5).clamp(0.0, 1.0) - 1.0).clamp(0.0, 1.0)
            };
            let cover = if how == Sampling::Nearest {
                // a draft: no part pixels
                if p.x >= 0.0 && p.y >= 0.0 && p.x < iw && p.y < ih { 1.0 } else { 0.0 }
            } else {
                (side(p.x, iw - p.x, gx) * side(p.y, ih - p.y, gy)) as f32
            };
            let s = if cover <= 0.0 {
                [0.0; 4]
            } else {
                let s = img.sample(how, p.x.clamp(0.5, iw - 0.5), p.y.clamp(0.5, ih - 0.5));
                if cover < 1.0 { s.map(|v| v * cover) } else { s }
            };
            if s[3] > 0.0 || everywhere {
                f(d, s, x, y);
            }
        }
    });
}

/// Draw `src` onto `dst` through `m` (from `src`'s pixels to `dst`'s), mixed with `mode` at
/// `opacity`. `seed` picks the Dissolve pattern.
pub fn place(dst: &mut Image, src: &Image, m: &Mat3, how: Sampling, mode: BlendMode, opacity: f32, seed: u32) {
    if opacity <= 0.0 && !mode.is_stencil() {
        return;
    }
    warp(dst, src, m, how, mode.is_stencil(), |d, s, x, y| {
        let s = if opacity < 1.0 { s.map(|v| v * opacity) } else { s };
        *d = blend(mode, *d, s, noise(x, y, seed));
    });
}

/// Add `src` drawn through `m`, times `weight`, to `dst` (the pictures of a motion blur add up).
pub fn accumulate(dst: &mut Image, src: &Image, m: &Mat3, how: Sampling, weight: f32) {
    warp(dst, src, m, how, false, |d, s, _, _| {
        for c in 0..4 {
            d[c] += s[c] * weight;
        }
    });
}

/// Mix `src` onto `dst`, the same size, with `mode` at `opacity`.
pub fn blend_onto(dst: &mut Image, src: &Image, mode: BlendMode, opacity: f32, seed: u32) {
    let width = dst.width;
    let stencil = mode.is_stencil();
    par::rows(&mut dst.px, width, |y, row| {
        for (x, d) in row.iter_mut().enumerate() {
            let s = src.px[y * width + x];
            if s[3] <= 0.0 && !stencil {
                continue;
            }
            let s = if opacity < 1.0 { s.map(|v| v * opacity) } else { s };
            *d = blend(mode, *d, s, noise(x, y, seed));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(n: usize) -> Image {
        Image::filled(n, n, [1.0, 0.0, 0.0, 1.0])
    }

    #[test]
    fn moving_by_whole_pixels_copies() {
        let mut dst = Image::new(10, 10);
        place(&mut dst, &square(4), &Mat3::translate(vec2(3.0, 2.0)), Sampling::Bilinear, BlendMode::Normal, 1.0, 0);
        assert_eq!(dst.get(3, 2), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(dst.get(6, 5), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(dst.get(2, 2)[3], 0.0);
        assert_eq!(dst.get(7, 5)[3], 0.0);
        let total: f32 = dst.px.iter().map(|p| p[3]).sum();
        assert!((total - 16.0).abs() < 1e-4);
    }

    #[test]
    fn half_a_pixel_over_spreads_the_edge() {
        let mut dst = Image::new(10, 10);
        place(&mut dst, &square(4), &Mat3::translate(vec2(3.5, 2.0)), Sampling::Bilinear, BlendMode::Normal, 1.0, 0);
        assert!((dst.get(3, 3)[3] - 0.5).abs() < 1e-5);
        assert!((dst.get(7, 3)[3] - 0.5).abs() < 1e-5);
        let total: f32 = dst.px.iter().map(|p| p[3]).sum();
        assert!((total - 16.0).abs() < 1e-3, "nothing is lost: {total}");
    }

    #[test]
    fn much_smaller_keeps_the_average() {
        // a fine checkerboard drawn at a tenth of its size is grey, not a shimmer of whole pixels
        let mut check = Image::new(200, 200);
        for y in 0..200 {
            for x in 0..200 {
                if (x + y) % 2 == 0 {
                    check.px[y * 200 + x] = [1.0, 1.0, 1.0, 1.0];
                }
            }
        }
        let mut dst = Image::new(20, 20);
        place(&mut dst, &check, &Mat3::scale(vec2(0.1, 0.1)), Sampling::Bilinear, BlendMode::Normal, 1.0, 0);
        for y in 2..18 {
            for x in 2..18 {
                assert!((dst.get(x, y)[3] - 0.5).abs() < 0.05, "{x} {y}: {:?}", dst.get(x, y));
            }
        }
    }

    #[test]
    fn stencils_reach_everywhere() {
        let mut dst = Image::filled(8, 8, [0.5, 0.5, 0.5, 1.0]);
        place(&mut dst, &square(2), &Mat3::translate(vec2(3.0, 3.0)), Sampling::Bilinear, BlendMode::StencilAlpha, 1.0, 0);
        assert_eq!(dst.get(0, 0), [0.0; 4], "outside the layer, nothing is left");
        assert_eq!(dst.get(3, 3), [0.5, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn motion_pictures_add_up() {
        let mut acc = Image::new(10, 4);
        for i in 0..4 {
            accumulate(&mut acc, &square(2), &Mat3::translate(vec2(i as f64, 1.0)), Sampling::Bilinear, 0.25);
        }
        assert!((acc.get(0, 1)[3] - 0.25).abs() < 1e-5);
        assert!((acc.get(2, 1)[3] - 0.5).abs() < 1e-5);
        let total: f32 = acc.px.iter().map(|p| p[3]).sum();
        assert!((total - 4.0).abs() < 1e-4);
    }

    #[test]
    fn noise_is_even_and_steady() {
        let n: Vec<f32> = (0..10_000).map(|i| noise(i % 100, i / 100, 7)).collect();
        let mean = n.iter().sum::<f32>() / n.len() as f32;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
        assert!(n.iter().all(|v| (0.0..1.0).contains(v)));
        assert_eq!(noise(3, 4, 7), noise(3, 4, 7));
        assert_ne!(noise(3, 4, 7), noise(3, 4, 8));
    }

    #[test]
    fn eight_bits_and_back() {
        let img = Image::from_rgba8(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]);
        assert_eq!(img.to_rgba8(), [255, 0, 0, 255, 0, 0, 255, 128]);
    }
}
