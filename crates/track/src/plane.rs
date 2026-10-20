//! The tracker's working pictures: a crop of a frame in the channel it compares (colour, its
//! brightness or its saturation), softened or sharpened if asked, halved again and again for a
//! pyramid, and read between pixels.
//!
//! A pixel `i` covers `[i, i + 1)`, its middle at `i + 0.5`; a halved plane keeps that, so a
//! point at `p` on one level is at `p / 2` on the next.

use veymelo_raster::Image;

/// What the tracker compares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channel {
    /// The three colours.
    Rgb,
    /// Brightness.
    #[default]
    Luminance,
    /// How strong the colour is.
    Saturation,
}

impl Channel {
    pub const IDS: [&'static str; 3] = ["rgb", "luminance", "saturation"];

    pub fn from_index(i: u32) -> Channel {
        [Channel::Rgb, Channel::Luminance, Channel::Saturation][(i as usize).min(2)]
    }

    /// How many numbers a pixel has.
    pub fn depth(self) -> usize {
        if self == Channel::Rgb { 3 } else { 1 }
    }
}

/// Pixels of one or three numbers each, row after row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub depth: usize,
    pub v: Vec<f32>,
}

impl Plane {
    pub fn new(w: usize, h: usize, depth: usize) -> Plane {
        Plane { w, h, depth, v: vec![0.0; w * h * depth] }
    }

    /// Channel `c` of pixel (`x`, `y`), the edge pixels standing in outside.
    #[inline]
    pub fn at(&self, x: isize, y: isize, c: usize) -> f32 {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.v[(y * self.w + x) * self.depth + c]
    }

    /// The plane at (`x`, `y`), between pixels, into `out`.
    #[inline]
    pub fn read(&self, x: f64, y: f64, out: &mut [f32; 3]) {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
        let (x0, y0) = (x0 as isize, y0 as isize);
        if x0 >= 0 && y0 >= 0 && (x0 as usize) + 1 < self.w && (y0 as usize) + 1 < self.h {
            let d = self.depth;
            let a = (y0 as usize * self.w + x0 as usize) * d;
            let b = a + self.w * d;
            for (c, o) in out.iter_mut().enumerate().take(d) {
                let top = self.v[a + c] + (self.v[a + d + c] - self.v[a + c]) * tx;
                let bottom = self.v[b + c] + (self.v[b + d + c] - self.v[b + c]) * tx;
                *o = top + (bottom - top) * ty;
            }
        } else {
            for (c, o) in out.iter_mut().enumerate().take(self.depth) {
                let top = self.at(x0, y0, c) + (self.at(x0 + 1, y0, c) - self.at(x0, y0, c)) * tx;
                let bottom = self.at(x0, y0 + 1, c) + (self.at(x0 + 1, y0 + 1, c) - self.at(x0, y0 + 1, c)) * tx;
                *o = top + (bottom - top) * ty;
            }
        }
    }

    /// `w` × `h` pixels of `img` from (`x0`, `y0`) in `channel` (its edge pixels outside it).
    pub fn crop(img: &Image, channel: Channel, x0: i64, y0: i64, w: usize, h: usize) -> Plane {
        let mut p = Plane::new(w, h, channel.depth());
        if img.width == 0 || img.height == 0 {
            return p;
        }
        for y in 0..h {
            let sy = (y0 + y as i64).clamp(0, img.height as i64 - 1) as usize;
            for x in 0..w {
                let sx = (x0 + x as i64).clamp(0, img.width as i64 - 1) as usize;
                let px = img.px[sy * img.width + sx];
                let i = (y * w + x) * p.depth;
                match channel {
                    Channel::Rgb => p.v[i..i + 3].copy_from_slice(&px[..3]),
                    Channel::Luminance => p.v[i] = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2],
                    Channel::Saturation => {
                        let (hi, lo) = (px[0].max(px[1]).max(px[2]), px[0].min(px[1]).min(px[2]));
                        p.v[i] = if hi > 1e-6 { (hi - lo) / hi } else { 0.0 };
                    }
                }
            }
        }
        p
    }

    /// Softened by a Gaussian of `sigma` pixels, across then down.
    pub fn blurred(&self, sigma: f64) -> Plane {
        if sigma <= 0.05 {
            return self.clone();
        }
        let r = (sigma * 3.0).ceil() as isize;
        let mut k: Vec<f32> = (-r..=r).map(|i| (-((i * i) as f64) / (2.0 * sigma * sigma)).exp() as f32).collect();
        let total: f32 = k.iter().sum();
        k.iter_mut().for_each(|w| *w /= total);
        let pass = |from: &Plane, across: bool| {
            let mut out = Plane::new(from.w, from.h, from.depth);
            for y in 0..from.h {
                for x in 0..from.w {
                    for c in 0..from.depth {
                        let mut sum = 0.0;
                        for (j, w) in k.iter().enumerate() {
                            let o = j as isize - r;
                            sum += w * if across { from.at(x as isize + o, y as isize, c) } else { from.at(x as isize, y as isize + o, c) };
                        }
                        out.v[(y * from.w + x) * from.depth + c] = sum;
                    }
                }
            }
            out
        };
        pass(&pass(self, true), false)
    }

    /// Edges brought out: twice what a little softening takes away, added back.
    pub fn sharpened(&self) -> Plane {
        let soft = self.blurred(1.5);
        let mut out = self.clone();
        for (o, s) in out.v.iter_mut().zip(&soft.v) {
            *o += 2.0 * (*o - s);
        }
        out
    }

    /// Half as wide and high, each pixel the mean of four.
    pub fn halved(&self) -> Plane {
        let (w, h) = (self.w.div_ceil(2).max(1), self.h.div_ceil(2).max(1));
        let mut out = Plane::new(w, h, self.depth);
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (2 * x as isize, 2 * y as isize);
                for c in 0..self.depth {
                    out.v[(y * w + x) * self.depth + c] = 0.25 * (self.at(sx, sy, c) + self.at(sx + 1, sy, c) + self.at(sx, sy + 1, c) + self.at(sx + 1, sy + 1, c));
                }
            }
        }
        out
    }

    /// How fast it changes across and down at each pixel.
    pub fn slopes(&self) -> (Plane, Plane) {
        let (mut gx, mut gy) = (Plane::new(self.w, self.h, self.depth), Plane::new(self.w, self.h, self.depth));
        for y in 0..self.h as isize {
            for x in 0..self.w as isize {
                for c in 0..self.depth {
                    let i = (y as usize * self.w + x as usize) * self.depth + c;
                    gx.v[i] = 0.5 * (self.at(x + 1, y, c) - self.at(x - 1, y, c));
                    gy.v[i] = 0.5 * (self.at(x, y + 1, c) - self.at(x, y - 1, c));
                }
            }
        }
        (gx, gy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_between_pixels_and_on_halves() {
        let mut p = Plane::new(8, 8, 1);
        for y in 0..8 {
            for x in 0..8 {
                p.v[y * 8 + x] = x as f32 + 10.0 * y as f32;
            }
        }
        let mut o = [0.0; 3];
        p.read(2.5, 3.5, &mut o);
        assert_eq!(o[0], 32.0, "a pixel's middle");
        p.read(3.0, 3.5, &mut o);
        assert!((o[0] - 32.5).abs() < 1e-6, "half way to the next");
        // a ramp keeps its value at the same point of the halved plane
        let (mut half, mut full) = ([0.0; 3], [0.0; 3]);
        p.halved().read(1.5, 1.5, &mut half);
        p.read(3.0, 3.0, &mut full);
        assert!((half[0] - full[0]).abs() < 1e-5, "{} {}", half[0], full[0]);
    }

    #[test]
    fn channels_compared() {
        let img = Image::filled(2, 2, [1.0, 0.5, 0.0, 1.0]);
        assert_eq!(Plane::crop(&img, Channel::Rgb, 0, 0, 2, 2).v[..3], [1.0, 0.5, 0.0]);
        assert!((Plane::crop(&img, Channel::Luminance, 0, 0, 2, 2).v[0] - (0.2126 + 0.3576)).abs() < 1e-6);
        assert_eq!(Plane::crop(&img, Channel::Saturation, 0, 0, 2, 2).v[0], 1.0);
        // outside, the edge
        assert_eq!(Plane::crop(&img, Channel::Rgb, -5, -5, 1, 1).v[..3], [1.0, 0.5, 0.0]);
    }
}
