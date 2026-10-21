//! Shadings: axial and radial ones as gradients (their colours along them taken from their
//! function at its joins, or often enough to follow a curve), the others as meshes of shaded
//! triangles — free-form and lattice triangles as they are, Coons and tensor patches cut into a
//! grid of small ones, function-based shadings sampled over their domain.

use veymelo_svg::{Gradient, GradientKind, Spread};

use crate::colour::Space;
use crate::file::File;
use crate::function::Function;
use crate::object::{Dict, Obj};

/// A point and its colour (straight sRGB with alpha).
pub type Corner = ([f64; 2], [f64; 4]);

pub type Triangle = [Corner; 3];

/// A patch's 16 points (a 4 × 4 grid) and its corners' colours.
type Patch = ([[f64; 2]; 16], [[f64; 4]; 4]);

pub struct Shading {
    pub kind: i64,
    space: Space,
    function: Option<Function>,
    dict: Dict,
    stream: Option<std::sync::Arc<crate::object::Stream>>,
    pub bbox: Option<[f64; 4]>,
}

fn nums(o: Option<Obj>) -> Vec<f64> {
    o.and_then(|o| o.as_array().map(|a| a.iter().filter_map(Obj::as_f64).collect())).unwrap_or_default()
}

impl Shading {
    pub fn read(file: &File, o: &Obj, resources: &Dict) -> Option<Shading> {
        let o = file.resolve(o);
        let dict = o.as_dict()?.clone();
        let kind = file.at(&dict, "ShadingType")?.as_i64()?;
        let space = Space::read(file, dict.get("ColorSpace").unwrap_or(&Obj::Name("DeviceGray".into())), resources);
        let function = dict.get("Function").and_then(|f| Function::read(file, f));
        let stream = match &o {
            Obj::Stream(s) => Some(s.clone()),
            _ => None,
        };
        let bbox = crate::file::rect(file.at(&dict, "BBox").as_ref());
        Some(Shading { kind, space, function, dict, stream, bbox })
    }

    /// A colour from the shading's inputs (through its function when it has one), straight
    /// sRGB with alpha.
    fn colour(&self, input: &[f64]) -> [f64; 4] {
        let comps = match &self.function {
            Some(f) => f.eval(input),
            None => input.to_vec(),
        };
        match self.space.rgb(&comps) {
            Some([r, g, b]) => [r, g, b, 1.0],
            None => [0.0; 4],
        }
    }

    /// An axial or radial shading as a gradient in its own space.
    pub fn gradient(&self, file: &File) -> Option<Gradient> {
        let coords = nums(file.at(&self.dict, "Coords"));
        let domain = nums(file.at(&self.dict, "Domain"));
        let (t0, t1) = (domain.first().copied().unwrap_or(0.0), domain.get(1).copied().unwrap_or(1.0));
        let extend: Vec<bool> = file.at(&self.dict, "Extend").and_then(|e| e.as_array().map(|a| a.iter().filter_map(Obj::as_bool).collect())).unwrap_or_default();
        let kind = match (self.kind, coords.len()) {
            (2, 4..) => GradientKind::Linear { x1: coords[0], y1: coords[1], x2: coords[2], y2: coords[3] },
            (3, 6..) => GradientKind::Circles { x0: coords[0], y0: coords[1], r0: coords[2], x1: coords[3], y1: coords[4], r1: coords[5] },
            _ => return None,
        };
        // the colours: at the function's joins when it changes in straight lines, else often
        let f = self.function.as_ref()?;
        let mut ts: Vec<f64> = if f.straight() {
            let mut v = vec![0.0, 1.0];
            v.extend(f.joins().iter().map(|&j| (j - t0) / (t1 - t0)).filter(|u| (0.0..=1.0).contains(u)));
            v
        } else {
            (0..=64).map(|k| k as f64 / 64.0).collect()
        };
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let stops = ts.iter().map(|&u| (u, self.colour(&[t0 + u * (t1 - t0)]))).collect();
        Some(Gradient { kind, stops, box_units: false, transform: veymelo_geom::Mat3::IDENTITY, spread: Spread::Pad, ends: [extend.first().copied().unwrap_or(false), extend.get(1).copied().unwrap_or(false)] })
    }

    /// The other shadings as triangles in their own space.
    pub fn triangles(&self, file: &File) -> Vec<Triangle> {
        match self.kind {
            1 => self.function_based(file),
            4..=7 => self.mesh(file),
            _ => Vec::new(),
        }
    }

    fn function_based(&self, file: &File) -> Vec<Triangle> {
        let d = nums(file.at(&self.dict, "Domain"));
        let (x0, x1, y0, y1) = (d.first().copied().unwrap_or(0.0), d.get(1).copied().unwrap_or(1.0), d.get(2).copied().unwrap_or(0.0), d.get(3).copied().unwrap_or(1.0));
        let m = nums(file.at(&self.dict, "Matrix"));
        let m = if m.len() == 6 { [m[0], m[1], m[2], m[3], m[4], m[5]] } else { [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] };
        let n = 48;
        let at = |i: usize, j: usize| {
            let (x, y) = (x0 + (x1 - x0) * i as f64 / n as f64, y0 + (y1 - y0) * j as f64 / n as f64);
            ([m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]], self.colour(&[x, y]))
        };
        let grid: Vec<Vec<Corner>> = (0..=n).map(|j| (0..=n).map(|i| at(i, j)).collect()).collect();
        let mut out = Vec::with_capacity(2 * n * n);
        for j in 0..n {
            for i in 0..n {
                out.push([grid[j][i], grid[j][i + 1], grid[j + 1][i + 1]]);
                out.push([grid[j][i], grid[j + 1][i + 1], grid[j + 1][i]]);
            }
        }
        out
    }

    fn mesh(&self, file: &File) -> Vec<Triangle> {
        let Some(s) = &self.stream else { return Vec::new() };
        let Ok(dec) = file.decoded(s) else { return Vec::new() };
        let data = dec.data;
        let int = |k: &str| file.at(&self.dict, k).and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 32) as u32;
        let (bpcoord, bpcomp, bpflag) = (int("BitsPerCoordinate"), int("BitsPerComponent"), int("BitsPerFlag"));
        let decode = nums(file.at(&self.dict, "Decode"));
        let ncomp = if self.function.is_some() { 1 } else { self.space.components() };
        if decode.len() < 4 + 2 * ncomp || bpcoord == 0 || bpcomp == 0 {
            return Vec::new();
        }
        let mut bits = Bits { d: &data, p: 0 };
        let read_point = |b: &mut Bits| -> Option<[f64; 2]> {
            let max = ((1u64 << bpcoord) - 1) as f64;
            let x = b.read(bpcoord)? as f64 / max;
            let y = b.read(bpcoord)? as f64 / max;
            Some([decode[0] + x * (decode[1] - decode[0]), decode[2] + y * (decode[3] - decode[2])])
        };
        let read_colour = |b: &mut Bits| -> Option<[f64; 4]> {
            let max = ((1u64 << bpcomp) - 1) as f64;
            let mut c = Vec::with_capacity(ncomp);
            for k in 0..ncomp {
                let v = b.read(bpcomp)? as f64 / max;
                c.push(decode[4 + 2 * k] + v * (decode[5 + 2 * k] - decode[4 + 2 * k]));
            }
            Some(self.colour(&c))
        };
        let mut out = Vec::new();
        match self.kind {
            4 => {
                let mut prev: Vec<Corner> = Vec::new();
                while out.len() < 1_000_000 {
                    let Some(flag) = bits.read(bpflag) else { break };
                    let (Some(p), Some(c)) = (read_point(&mut bits), read_colour(&mut bits)) else { break };
                    bits.align();
                    let v = (p, c);
                    match flag {
                        0 => {
                            prev.push(v);
                            if prev.len() == 3 {
                                out.push([prev[0], prev[1], prev[2]]);
                            } else if prev.len() > 3 {
                                prev = vec![v];
                            }
                        }
                        1 if prev.len() >= 3 => {
                            let n = prev.len();
                            let t = [prev[n - 2], prev[n - 1], v];
                            out.push(t);
                            prev = t.to_vec();
                        }
                        2 if prev.len() >= 3 => {
                            let n = prev.len();
                            let t = [prev[n - 3], prev[n - 1], v];
                            out.push(t);
                            prev = t.to_vec();
                        }
                        _ => {}
                    }
                }
            }
            5 => {
                let per_row = file.at(&self.dict, "VerticesPerRow").and_then(|v| v.as_i64()).unwrap_or(2).max(2) as usize;
                let mut rows: Vec<Vec<Corner>> = Vec::new();
                'all: while rows.len() < 100_000 {
                    let mut row = Vec::with_capacity(per_row);
                    for _ in 0..per_row {
                        let (Some(p), Some(c)) = (read_point(&mut bits), read_colour(&mut bits)) else { break 'all };
                        row.push((p, c));
                    }
                    rows.push(row);
                }
                for r in rows.windows(2) {
                    for i in 0..per_row - 1 {
                        out.push([r[0][i], r[0][i + 1], r[1][i + 1]]);
                        out.push([r[0][i], r[1][i + 1], r[1][i]]);
                    }
                }
            }
            _ => {
                // patches: 12 points (Coons) or 16 (tensor), 4 colours; a flag reuses an edge of
                // the patch before
                let tensor = self.kind == 7;
                let mut last: Option<Patch> = None;
                while out.len() < 2_000_000 {
                    let Some(flag) = bits.read(bpflag) else { break };
                    let (np, nc) = if flag == 0 { (if tensor { 16 } else { 12 }, 4) } else { (if tensor { 12 } else { 8 }, 2) };
                    let mut pts = Vec::with_capacity(np);
                    for _ in 0..np {
                        match read_point(&mut bits) {
                            Some(p) => pts.push(p),
                            None => break,
                        }
                    }
                    let mut cols = Vec::with_capacity(nc);
                    for _ in 0..nc {
                        match read_colour(&mut bits) {
                            Some(c) => cols.push(c),
                            None => break,
                        }
                    }
                    bits.align();
                    if pts.len() < np || cols.len() < nc {
                        break;
                    }
                    let Some((p, c)) = patch(flag, &pts, &cols, last.as_ref(), tensor) else { continue };
                    out.extend(patch_triangles(&p, &c));
                    last = Some((p, c));
                }
            }
        }
        out
    }
}

struct Bits<'a> {
    d: &'a [u8],
    p: usize,
}

impl Bits<'_> {
    fn read(&mut self, n: u32) -> Option<u64> {
        if n == 0 {
            return Some(0);
        }
        if self.p + n as usize > self.d.len() * 8 {
            return None;
        }
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 1) | ((self.d[self.p / 8] >> (7 - self.p % 8)) & 1) as u64;
            self.p += 1;
        }
        Some(v)
    }

    fn align(&mut self) {
        self.p = self.p.div_ceil(8) * 8;
    }
}

/// The 16 points of a patch in a 4 × 4 grid (row by row from its first corner) and its corners'
/// colours, from what was read (`flag` 1 to 3 taking an edge of the one before).
fn patch(flag: u64, pts: &[[f64; 2]], cols: &[[f64; 4]], last: Option<&Patch>, tensor: bool) -> Option<Patch> {
    // the boundary in the order the file gives it: 12 points round the patch
    let (edge, edge_cols): (Vec<[f64; 2]>, Vec<[f64; 4]>) = if flag == 0 {
        (pts[..12].to_vec(), cols.to_vec())
    } else {
        let (lp, lc) = last?;
        let ring = ring_of(lp);
        // the shared edge of the patch before (its points 3–6, 6–9, 9–12 round it)
        let (k, ca, cb) = match flag {
            1 => (3, 1, 2),
            2 => (6, 2, 3),
            _ => (9, 3, 0),
        };
        let mut e = vec![ring[k % 12], ring[(k + 1) % 12], ring[(k + 2) % 12], ring[(k + 3) % 12]];
        e.extend(&pts[..8]);
        (e, vec![lc[ca], lc[cb], cols[0], cols[1]])
    };
    // grid: p[row*4 + col], the boundary round it from (0,0) up the first column
    let mut g = [[0.0; 2]; 16];
    let ring_at = [(0, 0), (0, 1), (0, 2), (0, 3), (1, 3), (2, 3), (3, 3), (3, 2), (3, 1), (3, 0), (2, 0), (1, 0)];
    for (k, &(i, j)) in ring_at.iter().enumerate() {
        g[i * 4 + j] = edge[k];
    }
    let inner = [(1, 1), (1, 2), (2, 2), (2, 1)];
    if tensor && flag == 0 && pts.len() >= 16 {
        for (k, &(i, j)) in inner.iter().enumerate() {
            g[i * 4 + j] = pts[12 + k];
        }
    } else if tensor && flag != 0 && pts.len() >= 12 {
        for (k, &(i, j)) in inner.iter().enumerate() {
            g[i * 4 + j] = pts[8 + k];
        }
    } else {
        // Coons: the inner points from the boundary (the tensor form of a Coons patch)
        let p = |i: usize, j: usize| g[i * 4 + j];
        let lin = |a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], e: [f64; 2], f: [f64; 2], h: [f64; 2], k: [f64; 2]| -> [f64; 2] {
            std::array::from_fn(|t| (-4.0 * a[t] + 6.0 * (b[t] + c[t]) - 2.0 * (d[t] + e[t]) + 3.0 * (f[t] + h[t]) - k[t]) / 9.0)
        };
        let p11 = lin(p(0, 0), p(0, 1), p(1, 0), p(0, 3), p(3, 0), p(3, 1), p(1, 3), p(3, 3));
        let p12 = lin(p(0, 3), p(0, 2), p(1, 3), p(0, 0), p(3, 3), p(3, 2), p(1, 0), p(3, 0));
        let p21 = lin(p(3, 0), p(3, 1), p(2, 0), p(3, 3), p(0, 0), p(0, 1), p(2, 3), p(0, 3));
        let p22 = lin(p(3, 3), p(3, 2), p(2, 3), p(3, 0), p(0, 3), p(0, 2), p(2, 0), p(0, 0));
        g[5] = p11;
        g[6] = p12;
        g[10] = p22;
        g[9] = p21;
    }
    // corners' colours: (0,0), (0,3), (3,3), (3,0)
    Some((g, [edge_cols[0], edge_cols[1], edge_cols[2], edge_cols[3]]))
}

/// A patch's 12 boundary points in the order a file gives them.
fn ring_of(g: &[[f64; 2]; 16]) -> [[f64; 2]; 12] {
    let ring_at = [(0, 0), (0, 1), (0, 2), (0, 3), (1, 3), (2, 3), (3, 3), (3, 2), (3, 1), (3, 0), (2, 0), (1, 0)];
    std::array::from_fn(|k| g[ring_at[k].0 * 4 + ring_at[k].1])
}

/// A patch cut into a grid of triangles, each corner's colour between the patch's four.
fn patch_triangles(g: &[[f64; 2]; 16], c: &[[f64; 4]; 4]) -> Vec<Triangle> {
    let bern = |t: f64| [(1.0 - t).powi(3), 3.0 * t * (1.0 - t).powi(2), 3.0 * t * t * (1.0 - t), t.powi(3)];
    let n = 12;
    let at = |u: f64, v: f64| -> Corner {
        let (bu, bv) = (bern(u), bern(v));
        let mut p = [0.0; 2];
        for i in 0..4 {
            for j in 0..4 {
                let w = bv[i] * bu[j];
                p[0] += w * g[i * 4 + j][0];
                p[1] += w * g[i * 4 + j][1];
            }
        }
        // corners: (u, v) = (0,0) c0, (1,0) c1, (1,1) c2, (0,1) c3
        let col = std::array::from_fn(|k| (1.0 - u) * (1.0 - v) * c[0][k] + u * (1.0 - v) * c[1][k] + u * v * c[2][k] + (1.0 - u) * v * c[3][k]);
        (p, col)
    };
    let grid: Vec<Vec<Corner>> = (0..=n).map(|j| (0..=n).map(|i| at(i as f64 / n as f64, j as f64 / n as f64)).collect()).collect();
    let mut out = Vec::with_capacity(2 * n * n);
    for j in 0..n {
        for i in 0..n {
            out.push([grid[j][i], grid[j][i + 1], grid[j + 1][i + 1]]);
            out.push([grid[j][i], grid[j + 1][i + 1], grid[j + 1][i]]);
        }
    }
    out
}
