//! A flat surface (a sign, a screen, a wall, a page) followed through footage as a whole, the way
//! a planar tracker follows it: the many features found between its four corners followed from
//! one frame to the next (`klt`), how the surface moved fitted to them in perspective, ignoring
//! those that moved otherwise (`fit::robust`), and the corners moved by that — so a corner may lie
//! off the picture, or on a plain part with nothing to follow. The features are found afresh on
//! each frame inside the corners as they are then, so the surface may turn far from how it began.
//!
//! Points are the layer's pixels; an image pixel is a layer point plus the frame's offset.

use veymelo_raster::Image;

use crate::fit::{Model, robust};
use crate::klt::{Gray, corners, follow};

/// How few features followed are too few to say how the surface moved.
pub const FEWEST: usize = 8;

/// A surface being followed: the frame it was last seen on, and its corners there.
pub struct Planar {
    last: Gray,
    corners: [[f64; 2]; 4],
}

/// Whether `p` lies inside the four-sided `quad` (its corners in order round it, either way).
pub fn inside(quad: &[[f64; 2]; 4], p: [f64; 2]) -> bool {
    let mut odd = false;
    for i in 0..4 {
        let (a, b) = (quad[i], quad[(i + 1) % 4]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            odd = !odd;
        }
    }
    odd
}

impl Planar {
    /// A surface with corners `corners` (upper left, upper right, lower left, lower right, as a
    /// perspective corner pin has them) on frame `img`, whose layer (0, 0) is at `offset` in it.
    pub fn new(img: &Image, offset: [f64; 2], corners: [[f64; 2]; 4]) -> Planar {
        Planar { last: Gray::new(img, offset, 3), corners }
    }

    /// The corners in their order round the surface (the lower ones the other way round).
    fn round(&self) -> [[f64; 2]; 4] {
        let c = self.corners;
        [c[0], c[1], c[3], c[2]]
    }

    /// The next frame: where the corners are on it, and how many of the features followed
    /// agreed (0 to 1); none where too few could be followed (the corners then stay as they were).
    pub fn step(&mut self, img: &Image, offset: [f64; 2]) -> Option<([[f64; 2]; 4], f64)> {
        let now = Gray::new(img, offset, 3);
        let was = &self.last;
        let quad = self.round();
        let apart = (was.width().max(was.height()) as f64 / 120.0).max(3.0);
        let found: Vec<[f64; 2]> = corners(was, 4000, apart, 0.002).into_iter().filter(|p| inside(&quad, was.to_layer(*p))).take(600).collect();
        let pairs: Vec<([f64; 2], [f64; 2])> = found.iter().zip(follow(was, &now, &found)).filter_map(|(p, q)| Some((was.to_layer(*p), now.to_layer(q?)))).collect();
        self.last = now;
        if pairs.len() < FEWEST {
            return None;
        }
        let (map, agreed) = robust(Model::Perspective, &pairs, 1.5 * self.last.step)?;
        self.corners = self.corners.map(|c| map.apply(c));
        Some((self.corners, agreed as f64 / pairs.len() as f64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame `w` × `h` of a textured card (its corners `quad`, upper left, upper right, lower
    /// left, lower right) over a plain grey ground: the card's own pattern mapped onto it.
    fn frame(w: usize, h: usize, quad: [[f64; 2]; 4]) -> Image {
        let back = crate::solve::Warp::perspective([quad[0], quad[1], quad[3], quad[2]], [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]).unwrap();
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let uv = back.apply([x as f64 + 0.5, y as f64 + 0.5]);
                let v = if (0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]) {
                    // blobs of light and dark, like print
                    let (u, v) = (uv[0] * 12.0, uv[1] * 9.0);
                    let s = (u * 1.7).sin() * (v * 2.3).cos() + (u * 0.6 + v * 1.1).sin() * 0.7 + ((u * 3.1).cos() * (v * 2.7).sin()) * 0.4;
                    (0.5 + 0.25 * s) as f32
                } else {
                    0.5
                };
                img.px[y * w + x] = [v, v, v, 1.0];
            }
        }
        img
    }

    #[test]
    fn a_card_is_followed_as_it_turns_in_perspective() {
        let at = |k: f64| -> [[f64; 2]; 4] {
            // the card slides right, its right side swinging away
            let s = 8.0 * k;
            [[150.0 + s, 100.0 + 0.5 * k], [420.0 + s - 2.0 * k, 110.0 + 2.0 * k], [160.0 + s, 300.0], [410.0 + s - 2.0 * k, 290.0 - 2.0 * k]]
        };
        let first = frame(600, 400, at(0.0));
        let mut p = Planar::new(&first, [0.0; 2], at(0.0));
        let mut last = at(0.0);
        for k in 1..=10 {
            let img = frame(600, 400, at(k as f64));
            let (c, sure) = p.step(&img, [0.0; 2]).expect("followed");
            assert!(sure > 0.8, "frame {k}: {sure}");
            last = c;
        }
        // its corners where they went, to within a pixel after ten frames
        for (c, want) in last.iter().zip(at(10.0)) {
            assert!((c[0] - want[0]).hypot(c[1] - want[1]) < 1.0, "{c:?} {want:?}");
        }
    }

    #[test]
    fn a_point_is_inside_a_four_sided_shape() {
        let q = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert!(inside(&q, [5.0, 5.0]) && !inside(&q, [11.0, 5.0]) && !inside(&q, [5.0, -1.0]));
    }
}
