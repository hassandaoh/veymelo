//! Light bouncing from surface to surface in a 3D scene (global illumination), found on the
//! surfaces themselves so it is smooth and the same from frame to frame: at points spread over
//! each flat layer (a grid over its picture) and at each corner of each model, the light coming
//! back from all round is gathered (rays spread over the half sphere the point faces, each
//! meeting the nearest surface and taking the light it is lit by), and a pixel takes what the
//! points round it gathered, between them. Each bounce after the first gathers what the one
//! before it made of the surfaces too.

use veymelo_geom::{Vec3, vec3};

/// A point light is gathered at: where it is in the world, which way its surface faces (toward
/// the camera, for a flat layer), and the flat layer it is on (`usize::MAX`: a model's).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Probe {
    pub at: Vec3,
    pub n: Vec3,
    pub own: usize,
}

/// The points over a flat layer: a grid `nx` × `ny` of cells `du` × `dv` (its own units) from
/// (`u0`, `v0`), a point at each cell's middle, the first of them `first` in the scene's list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Grid {
    pub u0: f64,
    pub v0: f64,
    pub du: f64,
    pub dv: f64,
    pub nx: usize,
    pub ny: usize,
    pub first: usize,
}

/// How many cells a flat layer's grid has along its longer side (at most).
pub(crate) const CELLS: usize = 40;

impl Grid {
    /// A grid over the rectangle from (`u0`, `v0`), `w` × `h` (its own units), its first point
    /// `first`.
    pub fn over(u0: f64, v0: f64, w: f64, h: f64, first: usize) -> Grid {
        Grid::over_cells(u0, v0, w, h, first, CELLS)
    }

    /// The same with `cells` cells along its longer side (at most).
    pub fn over_cells(u0: f64, v0: f64, w: f64, h: f64, first: usize, cells: usize) -> Grid {
        let cell = (w.max(h) / cells as f64).max(1e-6);
        let (nx, ny) = (((w / cell).ceil() as usize).clamp(1, cells), ((h / cell).ceil() as usize).clamp(1, cells));
        Grid { u0, v0, du: w / nx as f64, dv: h / ny as f64, nx, ny, first }
    }

    /// Its points: where each is in its own units, row by row.
    pub fn points(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        (0..self.ny).flat_map(move |j| (0..self.nx).map(move |i| (self.u0 + (i as f64 + 0.5) * self.du, self.v0 + (j as f64 + 0.5) * self.dv)))
    }

    /// The light its points gathered (`light`, all the scene's) at its point (u, v), between
    /// them (held to the grid's edge past it).
    pub fn at(&self, light: &[[f32; 3]], u: f64, v: f64) -> [f32; 3] {
        let x = ((u - self.u0) / self.du - 0.5).clamp(0.0, (self.nx - 1) as f64);
        let y = ((v - self.v0) / self.dv - 0.5).clamp(0.0, (self.ny - 1) as f64);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.nx - 1), (y0 + 1).min(self.ny - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let get = |i: usize, j: usize| light.get(self.first + j * self.nx + i).copied().unwrap_or([0.0; 3]);
        let (a, b, c, d) = (get(x0, y0), get(x1, y0), get(x0, y1), get(x1, y1));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * fx;
            top + (c[k] + (d[k] - c[k]) * fx - top) * fy
        })
    }
}

/// How many rays a point gathers along.
pub(crate) const RAYS: usize = 128;

/// What the bounce's turns are made from (apart from the shadows', the occlusion's and the
/// mirrors').
pub(crate) const SEED: u32 = 0x27D4_EB2F;

/// The light gathered over each grid of `grids` (in `light`) smoothed, each point's the mean of
/// those round it (1 2 1 across and down), so what is left of each point's own rays' chance is
/// spread out.
pub(crate) fn smooth(light: &mut [[f32; 3]], grids: &[Grid]) {
    for g in grids {
        let before = light[g.first..g.first + g.nx * g.ny].to_vec();
        let at = |i: isize, j: isize| before[(j.clamp(0, g.ny as isize - 1) as usize) * g.nx + i.clamp(0, g.nx as isize - 1) as usize];
        for j in 0..g.ny as isize {
            for i in 0..g.nx as isize {
                let mut sum = [0f32; 3];
                for (dj, wj) in [(-1, 1.0), (0, 2.0), (1, 1.0)] {
                    for (di, wi) in [(-1, 1.0), (0, 2.0), (1, 1.0)] {
                        let c = at(i + di, j + dj);
                        (0..3).for_each(|k| sum[k] += c[k] * wi * wj / 16.0);
                    }
                }
                light[g.first + j as usize * g.nx + i as usize] = sum;
            }
        }
    }
}

/// The ways a point at `q` facing `n` gathers along: spread evenly over the half sphere it
/// faces, more toward its top (by the cosine, as light falls: a sunflower on the disc beneath,
/// lifted), the pattern turned a little from point to point.
pub(crate) fn rays(q: Vec3, n: Vec3) -> impl Iterator<Item = Vec3> {
    let side = if n.x.abs() < 0.9 { vec3(1.0, 0.0, 0.0) } else { vec3(0.0, 1.0, 0.0) };
    let a = n.cross(side).normalize();
    let b = n.cross(a);
    let turn = veymelo_raster::noise((q.x * 7.0) as usize, (q.y * 7.0) as usize, (q.z as i64 as u32) ^ SEED) as f64 * std::f64::consts::TAU;
    (0..RAYS).map(move |k| {
        let r = ((k as f64 + 0.5) / RAYS as f64).sqrt();
        let ang = k as f64 * 2.399_963 + turn;
        a * (r * ang.cos()) + b * (r * ang.sin()) + n * (1.0 - r * r).max(0.0).sqrt()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_reads_between_its_points() {
        let g = Grid::over(0.0, 0.0, 100.0, 50.0, 3);
        assert_eq!((g.nx, g.ny), (40, 20));
        let mut light = vec![[9.0; 3]; 3];
        light.extend((0..g.nx * g.ny).map(|i| [(i % g.nx) as f32, 0.0, 0.0]));
        // at a point, its own; halfway to the next, half of each; past the edge, the edge's
        assert_eq!(g.at(&light, g.du * 0.5, g.dv * 0.5)[0], 0.0);
        assert!((g.at(&light, g.du * 1.0, g.dv * 0.5)[0] - 0.5).abs() < 1e-6);
        assert_eq!(g.at(&light, -50.0, 10.0)[0], 0.0);
        assert_eq!(g.at(&light, 500.0, 10.0)[0], 39.0);
        assert_eq!(g.points().count(), g.nx * g.ny);
    }

    #[test]
    fn rays_lean_toward_the_way_a_point_faces() {
        let n = vec3(0.0, -1.0, 0.0);
        let all: Vec<Vec3> = rays(vec3(3.0, 4.0, 5.0), n).collect();
        assert_eq!(all.len(), RAYS);
        assert!(all.iter().all(|d| d.dot(n) > 0.0 && (d.length() - 1.0).abs() < 1e-9));
        // the mean of the cosine is two thirds over a half sphere weighed by it
        let mean = all.iter().map(|d| d.dot(n)).sum::<f64>() / RAYS as f64;
        assert!((mean - 2.0 / 3.0).abs() < 0.02, "{mean}");
    }
}
