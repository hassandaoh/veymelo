//! Caustics: the light glass bends, landing where it is bent to (the bright patch a glass ball
//! focuses on the floor, the ripples a glass shape throws).
//!
//! From each light that throws shadows a grid of beams is sent at each glass model that casts
//! them (over the circle round it, as the light sees it). Each beam's four edge rays are
//! followed: bent going into the glass and coming out by its index of refraction (reflected
//! whole inside where they cannot come out), let through as much as the glass lets its shadow
//! through and as much as is not reflected off its surface (the more, the more slant), through the flat layers that cast shadows as much as they let light through, and
//! stopped by a model that is not glass. Where they cross a flat layer that takes light after
//! going through glass, the beam's light is spread over the patch its four rays land on (a beam
//! whose rays do not go alike, at a model's outline, gives a quarter at each ray's landing),
//! gathered in a grid over the part of the layer it lands on, and smoothed.
//!
//! A beam carries what the light lets fall on a surface facing it where the beam is, times the
//! beam's width there; a light that does not fall off lights as much at any distance, so a
//! point light's beam carries more the farther it goes as it widens, and a pane of glass lets
//! through what its shadow would. With caustics on, a glass model's shadow on the flat layers
//! lets no light straight through (it is here instead).

use veymelo_geom::{Vec3, vec3};
use veymelo_project::LightKind;

use super::{GLASS_STEP, Scene, glass_at, through_glass};
use crate::bounce::Grid;
use crate::scene;

/// How many beams across (and down) each light sends at each glass model.
const BEAMS: usize = 160;

/// How many cells a caustic's grid has along its longer side (at most).
const CELLS: usize = 200;

/// The most surfaces a ray is followed through (glass and flat layers).
const MOST_CROSSINGS: usize = 32;

/// How many cells wide a beam's patch is spread over by points across it (wider: over the
/// cells it covers).
const SPREAD: f64 = 6.0;

/// How far behind the glass a parallel light's rays start (as far as anything in its way can
/// be).
const FAR: f64 = 1e6;

/// The light bent onto a flat layer: the grid over the part of it the light lands on (its first
/// cell 0), and the light at each cell.
pub(super) struct Caustic {
    pub plane: usize,
    pub grid: Grid,
    pub light: Vec<[f32; 3]>,
}

/// Where a ray lands on a flat layer that takes light, after going through glass.
#[derive(Clone, Copy)]
struct Landing {
    plane: usize,
    /// The glass surfaces it went through to get there, as a number (rays alike went alike).
    way: u64,
    /// Where on the layer (its own units).
    u: f64,
    v: f64,
    /// How much of the light it set out with it carries there (per colour): what the glass and
    /// layers on its way let through, times the light's reach and spread there.
    carries: [f64; 3],
}

/// A beam's light on a flat layer: spread over the patch its four rays land on (in order round
/// it), or at one point.
enum Patch {
    Quad([(f64, f64); 4], [f64; 3]),
    Point((f64, f64), [f64; 3]),
}

impl Patch {
    fn points(&self) -> &[(f64, f64)] {
        match self {
            Patch::Quad(c, _) => c,
            Patch::Point(p, _) => std::slice::from_ref(p),
        }
    }
}

impl Scene<'_> {
    /// The light the glass models bend onto each flat layer that takes light, `amount` of it.
    pub(super) fn caustics(&self, amount: f32) -> Vec<Caustic> {
        let receives: Vec<bool> = self.planes.iter().map(|p| p.material.lit && p.material.accepts == 1 && p.material.casts != 2).collect();
        if amount <= 0.0 || !receives.iter().any(|&r| r) || !self.casting.glass.iter().any(Option::is_some) {
            return Vec::new();
        }
        let trees = self.casting.trees();
        let mut patches: Vec<Vec<Patch>> = (0..self.planes.len()).map(|_| Vec::new()).collect();
        for l in self.lights.iter().filter(|l| l.kind != LightKind::Ambient && l.shadows && l.darkness > 0.0) {
            for (k, tree) in trees.iter().enumerate() {
                if self.casting.glass[k].is_none() {
                    continue;
                }
                let Some((lo, hi)) = tree.bounds() else { continue };
                let (c, r) = ((lo + hi) * 0.5, (hi - lo).length() * 0.5 * 1.01 + 1.0);
                let parallel = l.kind == LightKind::Parallel;
                let (d0, dist) = if parallel {
                    (l.dir.normalize(), FAR)
                } else {
                    let d = c - l.at;
                    (d.normalize(), d.length())
                };
                if !parallel && dist <= r {
                    continue;
                }
                let side = if d0.x.abs() < 0.9 { vec3(1.0, 0.0, 0.0) } else { vec3(0.0, 1.0, 0.0) };
                let a = d0.cross(side).normalize();
                let b = d0.cross(a);
                let step = 2.0 * r / BEAMS as f64;
                let corner = |i: usize, j: usize| c + a * (-r + step * i as f64) + b * (-r + step * j as f64);
                // each edge ray followed (row after row of them, on every core)
                let n = BEAMS + 1;
                let rays: Vec<Vec<Landing>> = veymelo_raster::par::map(n * n, |at| {
                    let s = corner(at % n, at / n);
                    let (from, dir) = if parallel { (s - d0 * FAR, d0) } else { (l.at, (s - l.at).normalize()) };
                    // a point light's reach where the ray is, `went` along it from the light, and
                    // its spread there (its beam as wide as it is far); a parallel light's, as it is
                    let carry = |went: f64| if parallel { l.reach(from + dir * went) } else { l.reach(l.at + dir * went) * went * went };
                    self.follow(from, dir, &receives, &carry)
                });
                // each beam's light: the light's colour, as much as its shadow takes away, times
                // how wide the beam is where it sets out (a point light's: the part of all round
                // it the beam is, as seen across its middle)
                for j in 0..BEAMS {
                    for i in 0..BEAMS {
                        let mid = corner(i, j) + (a + b) * (step * 0.5);
                        let wide = if parallel {
                            step * step
                        } else {
                            let cos = dist / (mid - l.at).length();
                            step * step * cos * cos * cos / (dist * dist)
                        };
                        let k = wide * l.darkness * amount as f64;
                        let light: [f64; 3] = std::array::from_fn(|ch| l.color[ch] as f64 * k);
                        let four = [&rays[j * n + i], &rays[j * n + i + 1], &rays[(j + 1) * n + i + 1], &rays[(j + 1) * n + i]];
                        let alike = four.iter().all(|r| r.len() == four[0].len() && r.iter().zip(four[0].iter()).all(|(x, y)| x.plane == y.plane && x.way == y.way));
                        if alike {
                            for m in 0..four[0].len() {
                                let at = four.map(|r| (r[m].u, r[m].v));
                                let carries: [f64; 3] = std::array::from_fn(|ch| four.iter().map(|r| r[m].carries[ch]).sum::<f64>() / 4.0);
                                patches[four[0][m].plane].push(Patch::Quad(at, std::array::from_fn(|ch| light[ch] * carries[ch])));
                            }
                        } else {
                            for ray in four {
                                for land in ray {
                                    patches[land.plane].push(Patch::Point((land.u, land.v), std::array::from_fn(|ch| light[ch] * land.carries[ch] / 4.0)));
                                }
                            }
                        }
                    }
                }
            }
        }
        patches.into_iter().enumerate().filter(|(_, p)| !p.is_empty()).filter_map(|(i, p)| self.gathered(i, &p)).collect()
    }

    /// Where the ray from `from` along `dir` lands on the flat layers that take light
    /// (`receives`) after going through glass, and what it carries there (`carry`: the light's
    /// reach and spread a way along it).
    fn follow(&self, from: Vec3, dir: Vec3, receives: &[bool], carry: &dyn Fn(f64) -> f64) -> Vec<Landing> {
        let trees = self.casting.trees();
        let mut out = Vec::new();
        let (mut q, mut dir) = (from, dir);
        let mut left = [1f64; 3];
        let (mut went, mut way, mut near) = (0.0, 0u64, 1e-6);
        for _ in 0..MOST_CROSSINGS {
            // the nearest model surface
            let mut hit: Option<(f64, usize, u32, f64, f64)> = None;
            for (k, tree) in trees.iter().enumerate() {
                if let Some((d, which, u, v)) = tree.nearest(q, dir, near, hit.map_or(f64::INFINITY, |h| h.0)) {
                    hit = Some((d, k, which, u, v));
                }
            }
            // the flat layers crossed before it, nearest first: lit by it (after glass), and
            // letting through as much as they let light through
            let until = hit.map_or(f64::INFINITY, |h| h.0);
            let mut crossed: Vec<(f64, usize)> = self
                .planes
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    let facing = dir.dot(p.normal);
                    if facing.abs() < 1e-12 || (!receives[i] && p.material.casts == 0) {
                        return None;
                    }
                    let s = (p.origin - q).dot(p.normal) / facing;
                    (s > 1e-6 && s < until).then_some((s, i))
                })
                .collect();
            crossed.sort_by(|x, y| x.0.total_cmp(&y.0));
            for (s, i) in crossed {
                let p = &self.planes[i];
                let at = p.unworld.apply(q + dir * s);
                let (u0, v0) = (p.own.origin.x, p.own.origin.y);
                let (w, h) = (p.own.img.width as f64 / p.own.scale, p.own.img.height as f64 / p.own.scale);
                if way != 0 && receives[i] && at.x >= u0 && at.x <= u0 + w && at.y >= v0 && at.y <= v0 + h {
                    let k = carry(went + s);
                    out.push(Landing { plane: i, way, u: at.x, v: at.y, carries: left.map(|c| c * k) });
                }
                if p.material.casts != 0 {
                    let t = p.shadow_texel(at.x, at.y);
                    let a = t[3].clamp(0.0, 1.0);
                    if a > 0.0 {
                        let tr = p.material.transmission;
                        (0..3).for_each(|c| left[c] *= (1.0 - a + a * tr * (t[c] / a)) as f64);
                    }
                }
            }
            if left.iter().all(|&c| c <= 0.0) {
                break;
            }
            // a model not glass stops it; glass bends it, going in or coming out
            let Some((d, k, which, u, v)) = hit else { break };
            let Some(g) = self.casting.glass[k] else { break };
            let (n, rgb) = glass_at(&self.faces[g.first + which as usize], u, v);
            (0..3).for_each(|c| left[c] *= through_glass(g.clear, g.rolloff, n, dir, rgb[c]) as f64);
            let going_in = n.dot(dir) < 0.0;
            let facing = if going_in { n } else { -n };
            let eta = if going_in { 1.0 / g.ior as f64 } else { g.ior as f64 };
            q += dir * d;
            went += d;
            dir = match scene::refract(dir, facing, eta) {
                Some(bent) => {
                    let kept = 1.0 - reflected(-facing.dot(dir), -facing.dot(bent), eta);
                    left = left.map(|c| c * kept);
                    bent
                }
                None => (dir - facing * (2.0 * facing.dot(dir))).normalize(),
            };
            way = way.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2 * k as u64 + going_in as u64 + 1);
            near = GLASS_STEP;
        }
        out
    }

    /// The light of `patches` on flat layer `i`, gathered in a grid over where they land (past
    /// a few far strays, if the most land in far less), smoothed: what falls on each cell, for
    /// its size.
    fn gathered(&self, i: usize, patches: &[Patch]) -> Option<Caustic> {
        let p = &self.planes[i];
        let mut us: Vec<f64> = patches.iter().flat_map(|q| q.points().iter().map(|c| c.0)).collect();
        let mut vs: Vec<f64> = patches.iter().flat_map(|q| q.points().iter().map(|c| c.1)).collect();
        us.sort_by(f64::total_cmp);
        vs.sort_by(f64::total_cmp);
        let span = |s: &[f64]| {
            let (lo, hi) = (s[0], s[s.len() - 1]);
            let k = s.len() / 200;
            let (a, b) = (s[k], s[s.len() - 1 - k]);
            if (hi - lo) > 3.0 * (b - a) { (a, b) } else { (lo, hi) }
        };
        let ((u0, u1), (v0, v1)) = (span(&us), span(&vs));
        let size = (u1 - u0).max(v1 - v0);
        if size.is_nan() || size <= 1e-9 {
            return None;
        }
        // a ring of cells round it with none landing, so it reads none past its edge
        let margin = size / (CELLS - 4) as f64 * 2.0;
        let grid = Grid::over_cells(u0 - margin, v0 - margin, u1 - u0 + 2.0 * margin, v1 - v0 + 2.0 * margin, 0, CELLS);
        let mut sum = vec![[0f64; 3]; grid.nx * grid.ny];
        let put = |sum: &mut [[f64; 3]], u: f64, v: f64, light: [f64; 3]| {
            let x = (u - grid.u0) / grid.du - 0.5;
            let y = (v - grid.v0) / grid.dv - 0.5;
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = (x - x0, y - y0);
            for (dx, dy, w) in [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)] {
                let (cx, cy) = (x0 as i64 + dx, y0 as i64 + dy);
                if cx < 0 || cy < 0 || cx >= grid.nx as i64 || cy >= grid.ny as i64 {
                    continue;
                }
                let cell = &mut sum[cy as usize * grid.nx + cx as usize];
                (0..3).for_each(|c| cell[c] += light[c] * w);
            }
        };
        for patch in patches {
            match patch {
                Patch::Point((u, v), light) => put(&mut sum, *u, *v, *light),
                // spread over it: a small one by points across it, two each cell it is wide; a big
                // one evenly over the cells whose middles it covers (its two halves each by its
                // size)
                Patch::Quad(at, light) => {
                    let (lo_u, hi_u) = at.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), c| (a.min(c.0), b.max(c.0)));
                    let (lo_v, hi_v) = at.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), c| (a.min(c.1), b.max(c.1)));
                    let cells = ((hi_u - lo_u) / grid.du).max((hi_v - lo_v) / grid.dv);
                    if cells > SPREAD {
                        let cell = |c: (f64, f64)| ((c.0 - grid.u0) / grid.du, (c.1 - grid.v0) / grid.dv);
                        let corners = at.map(cell);
                        let halves = [[corners[0], corners[1], corners[2]], [corners[0], corners[2], corners[3]]];
                        let size = |t: &[(f64, f64); 3]| ((t[1].0 - t[0].0) * (t[2].1 - t[0].1) - (t[2].0 - t[0].0) * (t[1].1 - t[0].1)).abs();
                        let whole = size(&halves[0]) + size(&halves[1]);
                        for t in &halves {
                            let share = if whole > 0.0 { size(t) / whole } else { 0.5 };
                            let covered = covered(t, grid.nx, grid.ny);
                            if covered.is_empty() {
                                let mid = ((t[0].0 + t[1].0 + t[2].0) / 3.0, (t[0].1 + t[1].1 + t[2].1) / 3.0);
                                put(&mut sum, grid.u0 + mid.0 * grid.du, grid.v0 + mid.1 * grid.dv, light.map(|c| c * share));
                                continue;
                            }
                            let each = light.map(|c| c * share / covered.len() as f64);
                            for (cx, cy) in covered {
                                let cell = &mut sum[cy * grid.nx + cx];
                                (0..3).for_each(|c| cell[c] += each[c]);
                            }
                        }
                        continue;
                    }
                    let s = ((cells * 2.0).ceil() as usize).clamp(1, (SPREAD * 2.0) as usize);
                    let each = light.map(|c| c / (s * s) as f64);
                    for j in 0..s {
                        for k in 0..s {
                            let (a, b) = ((k as f64 + 0.5) / s as f64, (j as f64 + 0.5) / s as f64);
                            let lerp = |x: (f64, f64), y: (f64, f64), t: f64| (x.0 + (y.0 - x.0) * t, x.1 + (y.1 - x.1) * t);
                            let (top, bottom) = (lerp(at[0], at[1], a), lerp(at[3], at[2], a));
                            let (u, v) = lerp(top, bottom, b);
                            put(&mut sum, u, v, each);
                        }
                    }
                }
            }
        }
        // what falls on each cell, for its size in the world
        let area = p.across.cross(p.down).length() * grid.du * grid.dv;
        let mut light: Vec<[f32; 3]> = sum.iter().map(|c| c.map(|v| (v / area) as f32)).collect();
        crate::bounce::smooth(&mut light, &[grid]);
        for j in 0..grid.ny {
            for k in 0..grid.nx {
                if j == 0 || k == 0 || j == grid.ny - 1 || k == grid.nx - 1 {
                    light[j * grid.nx + k] = [0.0; 3];
                }
            }
        }
        Some(Caustic { plane: i, grid, light })
    }
}

/// How much of the light crossing a surface is reflected off it (Fresnel's, for light of every
/// way of shaking): `cos_in` and `cos_out` the cosines of the ways in and out from its normal,
/// `eta` how much slower the stuff it comes from is than the one it goes into.
fn reflected(cos_in: f64, cos_out: f64, eta: f64) -> f64 {
    let (a, b) = (cos_in.abs(), cos_out.abs());
    let s = (eta * a - b) / (eta * a + b);
    let p = (eta * b - a) / (eta * b + a);
    ((s * s + p * p) / 2.0).clamp(0.0, 1.0)
}

/// The cells of a grid `nx` × `ny` whose middles triangle `t` (in cells) covers.
fn covered(t: &[(f64, f64); 3], nx: usize, ny: usize) -> Vec<(usize, usize)> {
    let lo = |k: usize| t.iter().map(|p| if k == 0 { p.0 } else { p.1 }).fold(f64::INFINITY, f64::min);
    let hi = |k: usize| t.iter().map(|p| if k == 0 { p.0 } else { p.1 }).fold(f64::NEG_INFINITY, f64::max);
    let (x0, x1) = (((lo(0) - 0.5).ceil().max(0.0)) as usize, ((hi(0) - 0.5).floor().min(nx as f64 - 1.0)).max(-1.0));
    let (y0, y1) = (((lo(1) - 0.5).ceil().max(0.0)) as usize, ((hi(1) - 0.5).floor().min(ny as f64 - 1.0)).max(-1.0));
    let side = |a: (f64, f64), b: (f64, f64), p: (f64, f64)| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    let mut out = Vec::new();
    if x1 < 0.0 || y1 < 0.0 {
        return out;
    }
    for y in y0..=y1 as usize {
        for x in x0..=x1 as usize {
            let p = (x as f64 + 0.5, y as f64 + 0.5);
            let (a, b, c) = (side(t[0], t[1], p), side(t[1], t[2], p), side(t[2], t[0], p));
            if (a >= 0.0 && b >= 0.0 && c >= 0.0) || (a <= 0.0 && b <= 0.0 && c <= 0.0) {
                out.push((x, y));
            }
        }
    }
    out
}
