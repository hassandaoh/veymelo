//! How each part of the picture moved between two frames (optical flow): points on a grid over
//! the first followed into the second (`klt::follow`, kept only where following back lands where
//! it began), the gaps where none could be followed filled from the neighbours, each point then
//! the median of those round it (so one bad match does not show), and read between the points.
//!
//! Points are the layer's pixels; an image pixel is a layer point plus the frame's offset.

use crate::klt::{Gray, follow};

/// How the picture moved, on a grid of points `every` layer pixels apart from `origin`.
#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    pub origin: [f64; 2],
    pub every: f64,
    pub w: usize,
    pub h: usize,
    /// Each grid point's move, row by row.
    pub moves: Vec<[f64; 2]>,
}

impl Flow {
    /// How the picture moved from `from` to `to`, followed at points `spacing` of their pixels
    /// apart.
    pub fn between(from: &Gray, to: &Gray, spacing: usize) -> Flow {
        let spacing = spacing.max(2);
        let (w, h) = (from.width().div_ceil(spacing).max(1), from.height().div_ceil(spacing).max(1));
        let at = |i: usize, j: usize| [(i * spacing) as f64 + spacing as f64 / 2.0, (j * spacing) as f64 + spacing as f64 / 2.0];
        let points: Vec<[f64; 2]> = (0..h).flat_map(|j| (0..w).map(move |i| at(i, j))).collect();
        let found = follow(from, to, &points);
        let mut moves: Vec<Option<[f64; 2]>> = points
            .iter()
            .zip(&found)
            .map(|(p, q)| {
                let (a, b) = (from.to_layer(*p), to.to_layer((*q)?));
                Some([b[0] - a[0], b[1] - a[1]])
            })
            .collect();
        // gaps filled from what is round them, nearer first
        let all_missing = moves.iter().all(Option::is_none);
        if all_missing {
            moves.iter_mut().for_each(|m| *m = Some([0.0; 2]));
        }
        while moves.iter().any(Option::is_none) {
            let before = moves.clone();
            for j in 0..h {
                for i in 0..w {
                    if before[j * w + i].is_some() {
                        continue;
                    }
                    let near: Vec<[f64; 2]> = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)]
                        .iter()
                        .filter_map(|(di, dj)| {
                            let (x, y) = (i as isize + di, j as isize + dj);
                            (x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h).then(|| before[y as usize * w + x as usize]).flatten()
                        })
                        .collect();
                    if !near.is_empty() {
                        let n = near.len() as f64;
                        moves[j * w + i] = Some([near.iter().map(|m| m[0]).sum::<f64>() / n, near.iter().map(|m| m[1]).sum::<f64>() / n]);
                    }
                }
            }
        }
        let filled: Vec<[f64; 2]> = moves.into_iter().map(|m| m.unwrap_or([0.0; 2])).collect();
        // each the median of the nine round it, across and down apart
        let median = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let smooth: Vec<[f64; 2]> = (0..w * h)
            .map(|k| {
                let (i, j) = ((k % w) as isize, (k / w) as isize);
                let (mut xs, mut ys) = (Vec::with_capacity(9), Vec::with_capacity(9));
                for dj in -1..=1 {
                    for di in -1..=1 {
                        let (x, y) = ((i + di).clamp(0, w as isize - 1) as usize, (j + dj).clamp(0, h as isize - 1) as usize);
                        xs.push(filled[y * w + x][0]);
                        ys.push(filled[y * w + x][1]);
                    }
                }
                [median(&mut xs), median(&mut ys)]
            })
            .collect();
        let first = from.to_layer(at(0, 0));
        Flow { origin: first, every: spacing as f64 * from.step, w, h, moves: smooth }
    }

    /// How the picture moved at layer point `p`, between the grid's points (held at its edges).
    pub fn at(&self, p: [f64; 2]) -> [f64; 2] {
        let x = ((p[0] - self.origin[0]) / self.every).clamp(0.0, (self.w - 1) as f64);
        let y = ((p[1] - self.origin[1]) / self.every).clamp(0.0, (self.h - 1) as f64);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let m = |i: usize, j: usize| self.moves[j * self.w + i];
        std::array::from_fn(|c| {
            let top = m(x0, y0)[c] + (m(x1, y0)[c] - m(x0, y0)[c]) * fx;
            let bottom = m(x0, y1)[c] + (m(x1, y1)[c] - m(x0, y1)[c]) * fx;
            top + (bottom - top) * fy
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veymelo_raster::Image;

    /// A noisy picture `w` × `h` whose left half is moved by `left` and right half by `right`.
    fn picture(w: usize, h: usize, left: [f64; 2], right: [f64; 2]) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let d = if x < w / 2 { left } else { right };
                let (u, v) = ((x as f64 - d[0]) * 0.21, (y as f64 - d[1]) * 0.17);
                let s = (u.sin() * v.cos() + (u * 0.37 + v * 0.53).sin() * 0.8 + ((u * 1.7).cos() * (v * 1.3).sin()) * 0.5) as f32;
                img.px[y * w + x] = [0.5 + 0.2 * s, 0.5 + 0.2 * s, 0.5 + 0.2 * s, 1.0];
            }
        }
        img
    }

    #[test]
    fn each_half_moves_its_own_way() {
        let a = Gray::new(&picture(320, 200, [0.0; 2], [0.0; 2]), [0.0; 2], 3);
        let b = Gray::new(&picture(320, 200, [3.0, 0.0], [-2.0, 1.5]), [0.0; 2], 3);
        let f = Flow::between(&a, &b, 8);
        let l = f.at([60.0, 100.0]);
        let r = f.at([260.0, 100.0]);
        assert!((l[0] - 3.0).abs() < 0.2 && l[1].abs() < 0.2, "{l:?}");
        assert!((r[0] + 2.0).abs() < 0.2 && (r[1] - 1.5).abs() < 0.2, "{r:?}");
    }
}
