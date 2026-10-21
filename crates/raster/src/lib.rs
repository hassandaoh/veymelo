//! Pictures we draw ourselves.
//!
//! - [`Image`]: premultiplied floating-point RGBA for compositing, drawn onto another through a
//!   transform ([`place`]), added up for motion blur ([`accumulate`]) or mixed whole
//!   ([`blend_onto`]).
//! - [`Canvas`]: RGB bytes for the app's window, with paths filled with smooth edges
//!   (`veymelo_path` finds how much of each pixel they cover), lines, dots and soft shadows.

pub mod blur;
pub mod filter;
pub mod flow;
pub mod gpu;
mod image;
pub mod par;
pub mod passes;
mod pixels;

pub use image::{Image, Px, Sampling, accumulate, blend_onto, noise, place, reach};
pub use pixels::Pixels;

use veymelo_path::{cover, Path};

/// A colour, 0 to 255 per channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// This colour, `k` of the way (0 to 1) to `other`.
    pub fn mix(self, other: Rgb, k: f64) -> Rgb {
        let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * k.clamp(0.0, 1.0)).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
}

/// A picture, `width` × `height` pixels, four bytes each (red, green, blue, unused).
#[derive(Clone)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Canvas {
    pub fn new(width: usize, height: usize, fill: Rgb) -> Canvas {
        Canvas { width, height, pixels: [fill.0, fill.1, fill.2, 255].repeat(width * height) }
    }

    /// Draws a line `width` pixels thick through `points` (back to the first when `closed`),
    /// in `colour`, `alpha` opaque. Each piece is a thin band; where bands overlap at a bend
    /// they count once.
    pub fn stroke(&mut self, points: &[(f64, f64)], closed: bool, width: f64, colour: Rgb, alpha: f32) {
        let mut path = Path::default();
        let half = width / 2.0;
        let n = points.len();
        let pieces = if closed { n } else { n.saturating_sub(1) };
        for i in 0..pieces {
            let (a, b) = (points[i], points[(i + 1) % n]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = dx.hypot(dy);
            if len < 1e-9 {
                continue;
            }
            // a little longer than the piece, so the bands meet at a bend
            let (ux, uy) = (dx / len * half * 0.5, dy / len * half * 0.5);
            let (nx, ny) = (-dy / len * half, dx / len * half);
            path.move_to(a.0 - ux + nx, a.1 - uy + ny);
            path.line_to(b.0 + ux + nx, b.1 + uy + ny);
            path.line_to(b.0 + ux - nx, b.1 + uy - ny);
            path.line_to(a.0 - ux - nx, a.1 - uy - ny);
            path.close();
        }
        self.fill_faded(&path, colour, alpha);
    }

    /// Mixes `colour` into the pixel at (`x`, `y`) by `amount` (0 to 1).
    fn mix(&mut self, x: usize, y: usize, colour: Rgb, amount: f32) {
        let p = (y * self.width + x) * 4;
        for (v, c) in self.pixels[p..p + 3].iter_mut().zip([colour.0, colour.1, colour.2]) {
            *v = (*v as f32 + (c as f32 - *v as f32) * amount).round() as u8;
        }
    }

    /// Fills the inside of `path` with `colour`, `alpha` of the way opaque (0 to 1).
    pub fn fill_faded(&mut self, path: &Path, colour: Rgb, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let Some(m) = cover(path, self.width, self.height) else { return };
        let alpha = alpha.min(1.0);
        for row in 0..m.height {
            for col in 0..m.width {
                let c = m.cover[row * m.width + col] * alpha;
                if c > 0.0 {
                    self.mix(m.left + col, m.top + row, colour, c);
                }
            }
        }
    }

    /// A soft shadow of a rectangle with round corners (`x`, `y`, `w`, `h`, corner `r`,
    /// pixels): darkest under it, fading out over about `blur` pixels from its edge.
    #[allow(clippy::too_many_arguments)]
    pub fn shadow(&mut self, x: f64, y: f64, w: f64, h: f64, r: f64, blur: f64, colour: Rgb, alpha: f32) {
        if alpha <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let r = r.min(w / 2.0).min(h / 2.0);
        let sigma = (blur / 2.0).max(0.5);
        let (cx, cy, hx, hy) = (x + w / 2.0, y + h / 2.0, w / 2.0 - r, h / 2.0 - r);
        let pad = blur * 1.6;
        let (x0, y0) = ((x - pad).floor().max(0.0) as usize, (y - pad).floor().max(0.0) as usize);
        let (x1, y1) = (((x + w + pad).ceil().max(0.0) as usize).min(self.width), ((y + h + pad).ceil().max(0.0) as usize).min(self.height));
        for py in y0..y1 {
            for px in x0..x1 {
                // how far the pixel's middle is outside the shape (negative inside)
                let (qx, qy) = (((px as f64 + 0.5) - cx).abs() - hx, ((py as f64 + 0.5) - cy).abs() - hy);
                let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
                // a smooth step from inside to outside, about as wide as a blur of `sigma`
                let a = 1.0 / (1.0 + (1.7 * d / sigma).exp());
                if a > 0.002 {
                    self.mix(px, py, colour, a as f32 * alpha);
                }
            }
        }
    }

    /// Puts `dot` down with its middle on the pixel (`x`, `y`), in `colour`, `alpha` opaque.
    pub fn stamp(&mut self, dot: &Dot, x: i64, y: i64, colour: Rgb, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let half = (dot.size / 2) as i64;
        for dy in 0..dot.size as i64 {
            let py = y + dy - half;
            if py < 0 || py >= self.height as i64 {
                continue;
            }
            for dx in 0..dot.size as i64 {
                let px = x + dx - half;
                let c = dot.cover[(dy as usize) * dot.size + dx as usize];
                if px >= 0 && px < self.width as i64 && c > 0.0 {
                    self.mix(px as usize, py as usize, colour, c * alpha);
                }
            }
        }
    }
}

/// A small round dot, ready to be put down many times: how much of each pixel of its square
/// it covers.
pub struct Dot {
    size: usize,
    cover: Vec<f32>,
}

impl Dot {
    /// A dot `radius` pixels across from its middle, which sits (`fx`, `fy`) of a pixel right of
    /// and below the middle of a pixel (0 to 1), so dots can sit between pixels.
    pub fn new(radius: f64, fx: f64, fy: f64) -> Dot {
        let half = radius.ceil() as usize + 2;
        let size = 2 * half + 1;
        let mut cover = vec![0f32; size * size];
        // four by four samples a pixel
        for y in 0..size {
            for x in 0..size {
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let px = x as f64 + (sx as f64 + 0.5) / 4.0 - half as f64 - 0.5 - fx;
                        let py = y as f64 + (sy as f64 + 0.5) / 4.0 - half as f64 - 0.5 - fy;
                        if px * px + py * py <= radius * radius {
                            hits += 1;
                        }
                    }
                }
                cover[y * size + x] = hits as f32 / 16.0;
            }
        }
        Dot { size, cover }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, side: f64) -> Path {
        let mut p = Path::default();
        p.move_to(x, y);
        p.line_to(x + side, y);
        p.line_to(x + side, y + side);
        p.line_to(x, y + side);
        p.close();
        p
    }

    #[test]
    fn a_square_on_whole_pixels_is_solid_inside_and_clean_outside() {
        let mut c = Canvas::new(10, 10, Rgb(255, 255, 255));
        c.fill_faded(&square(2.0, 2.0, 4.0), Rgb(0, 0, 0), 1.0);
        let at = |x: usize, y: usize| c.pixels[(y * 10 + x) * 4];
        assert_eq!(at(3, 3), 0);
        assert_eq!(at(1, 3), 255);
        assert_eq!(at(3, 6), 255);
    }

    #[test]
    fn half_a_pixel_covered_is_half_grey() {
        let mut c = Canvas::new(4, 4, Rgb(255, 255, 255));
        c.fill_faded(&square(1.5, 1.0, 2.0), Rgb(0, 0, 0), 1.0);
        let at = |x: usize, y: usize| c.pixels[(y * 4 + x) * 4];
        assert!((at(1, 1) as i32 - 128).abs() <= 1, "{}", at(1, 1));
        assert_eq!(at(2, 1), 0);
    }

    #[test]
    fn a_line_is_as_thick_as_asked() {
        let mut c = Canvas::new(10, 10, Rgb(255, 255, 255));
        c.stroke(&[(1.0, 5.0), (9.0, 5.0)], false, 2.0, Rgb(0, 0, 0), 1.0);
        let at = |x: usize, y: usize| c.pixels[(y * 10 + x) * 4];
        assert_eq!(at(5, 4), 0);
        assert_eq!(at(5, 5), 0);
        assert_eq!(at(5, 3), 255);
        assert_eq!(at(5, 6), 255);
    }
}
