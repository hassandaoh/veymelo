//! Paths: closed outlines made of lines and curves (flattened into short lines as they are
//! drawn), and how much of each pixel a path covers. A pixel's share is exact: every edge adds
//! the signed area it leaves in each cell it crosses, and summing along the row gives how much
//! of each pixel is inside.

pub mod bezier;
pub mod boolean;
pub mod caps;
pub mod feather;
pub mod fit;
pub mod morph;
pub mod trace;
pub mod fill;
pub mod ops;
pub mod shape;
pub mod stroke;

pub use boolean::{Merge, merge};
pub use fill::{Rule, fill};
pub use shape::{Sub, Vertex};
pub use stroke::{Cap, Join, Stroke, Taper, Wave, stroke};

/// Closed outlines made of straight pieces, in pixels. Curves are flattened as they are added,
/// in pieces short enough that they stray under a fortieth of a pixel (straight pieces inside
/// a curve cut a little area off it: at this, a circle of radius 20 loses about a tenth of a
/// percent).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    contours: Vec<Vec<(f64, f64)>>,
}

/// How far a flattened curve may stray from the true one, in pixels.
const TOLERANCE: f64 = 0.025;

impl Path {
    pub fn move_to(&mut self, x: f64, y: f64) {
        self.contours.push(vec![(x, y)]);
    }

    pub fn line_to(&mut self, x: f64, y: f64) {
        match self.contours.last_mut() {
            Some(c) => c.push((x, y)),
            None => self.move_to(x, y),
        }
    }

    fn here(&self) -> Option<(f64, f64)> {
        self.contours.last().and_then(|c| c.last()).copied()
    }

    /// A quadratic curve from where the path is, bent toward (`cx`, `cy`), to (`x`, `y`).
    pub fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
        let Some((x0, y0)) = self.here() else {
            return self.move_to(x, y);
        };
        let bend = ((x0 - 2.0 * cx + x).hypot(y0 - 2.0 * cy + y)) / 4.0;
        let n = ((bend / TOLERANCE).sqrt().ceil() as usize).clamp(1, 128);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            self.line_to(u * u * x0 + 2.0 * u * t * cx + t * t * x, u * u * y0 + 2.0 * u * t * cy + t * t * y);
        }
    }

    /// A cubic curve from where the path is, pulled toward (`ax`, `ay`) then (`bx`, `by`), to
    /// (`x`, `y`).
    pub fn cubic_to(&mut self, ax: f64, ay: f64, bx: f64, by: f64, x: f64, y: f64) {
        let Some((x0, y0)) = self.here() else {
            return self.move_to(x, y);
        };
        // how far the curve bends from its chord bounds how many pieces it needs
        let d1 = (x0 - 2.0 * ax + bx).hypot(y0 - 2.0 * ay + by);
        let d2 = (ax - 2.0 * bx + x).hypot(ay - 2.0 * by + y);
        let bend = 0.75 * d1.max(d2);
        let n = ((bend / TOLERANCE).sqrt().ceil() as usize).clamp(1, 256);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.line_to(a * x0 + b * ax + c * bx + d * x, a * y0 + b * ay + c * by + d * y);
        }
    }

    pub fn close(&mut self) {
        if let Some(c) = self.contours.last_mut()
            && let (Some(&a), Some(&b)) = (c.first(), c.last())
            && a != b
        {
            c.push(a);
        }
    }

    /// The outlines as points (each closed one ends where it starts).
    pub fn contours(&self) -> &[Vec<(f64, f64)>] {
        &self.contours
    }

    /// The outline of a rectangle with round corners of radius `r`, as points going round
    /// (for drawing it as a line).
    pub fn rounded_points(x: f64, y: f64, w: f64, h: f64, r: f64) -> Vec<(f64, f64)> {
        let mut p = Path::rounded(x, y, w, h, r);
        p.contours.pop().unwrap_or_default()
    }

    /// A rectangle with round corners of radius `r` (the corners are quarter circles, each
    /// made of two curves).
    pub fn rounded(x: f64, y: f64, w: f64, h: f64, r: f64) -> Path {
        let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
        let mut p = Path::default();
        // each corner from one side's end to the next side's start, about its centre
        let corners = [(x + w - r, y + r, -90.0f64), (x + w - r, y + h - r, 0.0), (x + r, y + h - r, 90.0), (x + r, y + r, 180.0)];
        let reach = r / 22.5f64.to_radians().cos();
        for (i, &(cx, cy, from)) in corners.iter().enumerate() {
            let at = |deg: f64, len: f64| (cx + len * deg.to_radians().cos(), cy + len * deg.to_radians().sin());
            let start = at(from, r);
            if i == 0 {
                p.move_to(start.0, start.1);
            } else {
                p.line_to(start.0, start.1);
            }
            for half in 0..2 {
                let a = from + 45.0 * half as f64;
                let (c, e) = (at(a + 22.5, reach), at(a + 45.0, r));
                p.quad_to(c.0, c.1, e.0, e.1);
            }
        }
        p.close();
        p
    }

    /// The box round the path: left, top, right, bottom.
    pub fn extent(&self) -> Option<(f64, f64, f64, f64)> {
        let mut pts = self.contours.iter().flatten();
        let &(x, y) = pts.next()?;
        Some(pts.fold((x, y, x, y), |(a, b, c, d), &(x, y)| (a.min(x), b.min(y), c.max(x), d.max(y))))
    }
}

/// How much of each pixel of a box a path covers (0 to 1), the box's top left at (`left`,
/// `top`) in whole pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub left: usize,
    pub top: usize,
    pub width: usize,
    pub height: usize,
    pub cover: Vec<f32>,
}

impl Mask {
    /// The cover of the pixel (`x`, `y`) of the picture (0 outside the box).
    pub fn at(&self, x: usize, y: usize) -> f32 {
        if x < self.left || y < self.top || x >= self.left + self.width || y >= self.top + self.height {
            return 0.0;
        }
        self.cover[(y - self.top) * self.width + (x - self.left)]
    }
}

/// How much of each pixel of a `width` × `height` picture `path` covers, counting a place
/// inside once however many outlines wind round it. None when it covers nothing.
pub fn cover(path: &Path, width: usize, height: usize) -> Option<Mask> {
    let (x0, y0, x1, y1) = path.extent()?;
    // the cells the path can touch, two spare columns at the right for what spills over
    let left = (x0.floor().max(0.0)) as usize;
    let top = (y0.floor().max(0.0)) as usize;
    let right = (x1.ceil().max(0.0) as usize).min(width);
    let bottom = (y1.ceil().max(0.0) as usize).min(height);
    if left >= right || top >= bottom {
        return None;
    }
    let (w, h) = (right - left, bottom - top);
    let stride = w + 2;
    let mut acc = vec![0f32; stride * h];
    for c in &path.contours {
        for pair in c.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            edge(&mut acc, stride, w, h, a.0 - left as f64, a.1 - top as f64, b.0 - left as f64, b.1 - top as f64);
        }
    }
    let mut cover = vec![0f32; w * h];
    for row in 0..h {
        let mut sum = 0f32;
        for col in 0..w {
            sum += acc[row * stride + col];
            cover[row * w + col] = sum.abs().min(1.0);
        }
    }
    Some(Mask { left, top, width: w, height: h, cover })
}

/// Adds what the edge from (`x0`, `y0`) to (`x1`, `y1`) leaves in each cell it crosses: the
/// signed area to its right, so that summing along a row gives how much of each pixel is
/// inside. Points left of the box count at its left edge.
#[allow(clippy::too_many_arguments)]
fn edge(acc: &mut [f32], stride: usize, w: usize, h: usize, x0: f64, y0: f64, x1: f64, y1: f64) {
    if y0 == y1 {
        return;
    }
    let (dir, (xa, ya), (xb, yb)) = if y0 < y1 { (1.0, (x0, y0), (x1, y1)) } else { (-1.0, (x1, y1), (x0, y0)) };
    let slope = (xb - xa) / (yb - ya);
    let top = ya.max(0.0);
    let bottom = yb.min(h as f64);
    if top >= bottom {
        return;
    }
    let mut x = xa + (top - ya) * slope;
    let mut y = top;
    let limit = w as f64 + 0.999;
    while y < bottom {
        let row = y.floor() as usize;
        let next_y = ((row + 1) as f64).min(bottom);
        let dy = next_y - y;
        let next_x = x + slope * dy;
        let d = (dy * dir) as f32;
        let line = row * stride;
        let (l, r) = if x < next_x { (x, next_x) } else { (next_x, x) };
        let (l, r) = (l.clamp(0.0, limit), r.clamp(0.0, limit));
        let lf = l.floor();
        let li = lf as usize;
        let ri = r.ceil() as usize;
        if ri <= li + 1 {
            // inside one cell: its share is how far right of the edge's middle the cell's end is
            let mid = ((l + r) / 2.0 - lf) as f32;
            acc[line + li] += d * (1.0 - mid);
            acc[line + li + 1] += d * mid;
        } else {
            // across several cells: a triangle in the first, a band in each middle one, the
            // rest in the last
            let s = (1.0 / (r - l)) as f32;
            let lfrac = (l - lf) as f32;
            let a0 = 0.5 * s * (1.0 - lfrac) * (1.0 - lfrac);
            let rfrac = (r - ri as f64 + 1.0) as f32;
            let am = 0.5 * s * rfrac * rfrac;
            acc[line + li] += d * a0;
            if ri == li + 2 {
                acc[line + li + 1] += d * (1.0 - a0 - am);
            } else {
                let a1 = s * (1.5 - lfrac);
                acc[line + li + 1] += d * (a1 - a0);
                for cell in li + 2..ri - 1 {
                    acc[line + cell] += d * s;
                }
                let a2 = a1 + (ri - li - 3) as f32 * s;
                acc[line + ri - 1] += d * (1.0 - a2 - am);
            }
            acc[line + ri] += d * am;
        }
        x = next_x;
        y = next_y;
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
    fn a_square_on_whole_pixels_covers_them_and_nothing_else() {
        let m = cover(&square(2.0, 2.0, 4.0), 10, 10).unwrap();
        assert_eq!(m.at(3, 3), 1.0);
        assert_eq!(m.at(5, 5), 1.0);
        assert_eq!(m.at(1, 3), 0.0);
        assert_eq!(m.at(6, 3), 0.0);
        assert_eq!(m.at(3, 6), 0.0);
    }

    #[test]
    fn half_a_pixel_covered_is_half() {
        let m = cover(&square(1.5, 1.0, 2.0), 4, 4).unwrap();
        assert!((m.at(1, 1) - 0.5).abs() < 1e-6, "{}", m.at(1, 1));
        assert_eq!(m.at(2, 1), 1.0);
        assert!((m.at(3, 1) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_shape_past_the_edges_is_cut_off_cleanly() {
        let m = cover(&square(-3.0, -3.0, 6.0), 6, 6).unwrap();
        assert_eq!(m.at(0, 0), 1.0);
        assert_eq!(m.at(2, 2), 1.0);
        assert_eq!(m.at(3, 3), 0.0);
        assert!(cover(&square(-30.0, -30.0, 6.0), 6, 6).is_none());
    }

    #[test]
    fn a_circle_of_curves_covers_about_pi_r_squared() {
        // four cubic quarter circles, radius 20, centre (25, 25)
        let (c, r, k) = (25.0, 20.0, 0.552_284_75 * 20.0);
        let mut p = Path::default();
        p.move_to(c + r, c);
        p.cubic_to(c + r, c + k, c + k, c + r, c, c + r);
        p.cubic_to(c - k, c + r, c - r, c + k, c - r, c);
        p.cubic_to(c - r, c - k, c - k, c - r, c, c - r);
        p.cubic_to(c + k, c - r, c + r, c - k, c + r, c);
        p.close();
        let m = cover(&p, 50, 50).unwrap();
        let area: f32 = m.cover.iter().sum();
        let want = std::f32::consts::PI * 400.0;
        assert!((area - want).abs() / want < 0.002, "{area} vs {want}");
    }
}

#[cfg(test)]
mod tools_tests;
