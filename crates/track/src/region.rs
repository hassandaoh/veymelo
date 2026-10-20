//! What is inside an outline followed from frame to frame (a mask tracker): corners found inside
//! it, followed into the next frame, and how most of them moved fitted (moved; turned and scaled
//! too; slanted too; or in perspective), the outline taken along. Corners are found afresh inside
//! the moved outline each frame, so what turns its back or leaves is let go.

use veymelo_raster::Image;

use crate::fit::{Model, robust};
use crate::klt::{Gray, corners, follow};
use crate::solve::Warp;

/// Whether `p` is inside the outline `poly` (the even-odd rule).
pub fn inside(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut odd = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            odd = !odd;
        }
    }
    odd
}

/// Follows what is inside an outline.
pub struct Region {
    last: Gray,
    /// The outline as it is now, in the layer's pixels.
    pub outline: Vec<[f64; 2]>,
    model: Model,
}

impl Region {
    /// What is inside `outline` (the layer's pixels) on `img` (whose layer (0, 0) is at `offset`).
    pub fn new(img: &Image, offset: [f64; 2], outline: Vec<[f64; 2]>, model: Model) -> Region {
        Region { last: Gray::new(img, offset, 3), outline, model }
    }

    /// Corners inside the outline as it is, in the last frame's pixels.
    fn features(&self) -> Vec<[f64; 2]> {
        let g = &self.last;
        let apart = (g.width().max(g.height()) as f64 / 80.0).max(4.0);
        corners(g, 3000, apart, 0.001).into_iter().filter(|p| inside(&self.outline, g.to_layer(*p))).take(400).collect()
    }

    /// How what is inside moved into the next frame, and how many corners agreed; the outline
    /// moved with it. None when too little inside can be followed (the outline stays).
    pub fn step(&mut self, img: &Image, offset: [f64; 2]) -> Option<(Warp, usize)> {
        let now = Gray::new(img, offset, 3);
        let points = self.features();
        let pairs: Vec<([f64; 2], [f64; 2])> = points.iter().zip(follow(&self.last, &now, &points)).filter_map(|(p, q)| Some((self.last.to_layer(*p), now.to_layer(q?)))).collect();
        let tolerance = 1.5 * self.last.step;
        self.last = now;
        let (warp, agreed) = robust(self.model, &pairs, tolerance).filter(|(_, n)| *n >= 6)?;
        self.outline = self.outline.iter().map(|p| warp.apply(*p)).collect();
        Some((warp, agreed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_or_out() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert!(inside(&square, [5.0, 5.0]) && !inside(&square, [15.0, 5.0]) && !inside(&square, [5.0, -1.0]));
    }

    #[test]
    fn a_patch_inside_an_outline_followed_as_it_turns() {
        // a textured disc on a flat ground, moving and turning
        let disc = |k: usize, w: usize, h: usize| {
            let (cx, cy, turn) = (120.0 + 6.0 * k as f64, 100.0 + 2.0 * k as f64, (3.0 * k as f64).to_radians());
            let mut img = Image::filled(w, h, [0.3, 0.3, 0.3, 1.0]);
            for y in 0..h {
                for x in 0..w {
                    let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                    if dx.hypot(dy) < 50.0 {
                        let (u, v) = (dx * turn.cos() + dy * turn.sin(), -dx * turn.sin() + dy * turn.cos());
                        let t = (0.5 + 0.2 * (u * 0.31).sin() * (v * 0.23).cos() + 0.15 * ((u + v) * 0.17).sin()) as f32;
                        img.px[y * w + x] = [t, t, t, 1.0];
                    }
                }
            }
            img
        };
        let outline = vec![[90.0, 70.0], [150.0, 70.0], [150.0, 130.0], [90.0, 130.0]];
        let mut region = Region::new(&disc(0, 320, 240), [0.0; 2], outline, Model::Similar);
        for k in 1..8 {
            let (_, agreed) = region.step(&disc(k, 320, 240), [0.0; 2]).unwrap_or_else(|| panic!("frame {k}"));
            assert!(agreed > 20, "frame {k}: {agreed}");
        }
        // the outline's first corner went round the disc's middle with it
        let (cx, cy, turn) = (120.0 + 42.0, 100.0 + 14.0, 21f64.to_radians());
        let want = [cx + (-30.0) * turn.cos() - (-30.0) * turn.sin(), cy + (-30.0) * turn.sin() + (-30.0) * turn.cos()];
        let got = region.outline[0];
        assert!((got[0] - want[0]).hypot(got[1] - want[1]) < 1.0, "{got:?} {want:?}");
    }
}
