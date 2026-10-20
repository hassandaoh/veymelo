//! Distortions, part four: the picture flowing toward or away from two knots (Flow Motion), and
//! pushed about by brush strokes (Liquify).
//!
//! Liquify keeps its strokes as text, one a line, in its Distortion Mesh setting: they are what
//! the effect is, written by the `liquify.stroke` command (or by hand), and they animate with hold
//! keys. Played back, they build a mesh of how far each point of the layer moved; the same strokes
//! always build the same mesh, which is kept for the next frame.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use veymelo_raster::{Px, Sampling};

use crate::color::rgb3;
use crate::defs::{choice, color, effect, flag, num, point, ranged};
use crate::draw::inside;
use crate::util::map_px;
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "distort";

effect!(
    FLOW_DEF,
    "flowMotion",
    "Flow Motion",
    "the picture flowing out of one knot and into another (or the other way, below 0)",
    [
        point("knot1", "Knot 1"),
        ranged("amount1", "Amount 1", 50.0, -400.0, 100.0),
        point("knot2", "Knot 2"),
        ranged("amount2", "Amount 2", -50.0, -400.0, 100.0),
        flag("tileEdges", "Tile Edges", false).doc("what is pulled in from past the edges is the layer over again"),
        choice("antialiasing", "Antialiasing", &["low", "medium", "high"], 1),
        ranged("falloff", "Falloff", 2.0, 0.01, 10.0).doc("higher: the flow keeps closer to the knots"),
    ]
);
pub const FLOW: Spec = Spec::new(&FLOW_DEF, CATEGORY, flow).points(&[("knot1", [0.25, 0.5]), ("knot2", [0.75, 0.5])]);

fn flow(ctx: &Ctx, mut b: Buf) -> Buf {
    let knots = [(ctx.point("knot1"), ctx.num("amount1", 50.0) / 100.0), (ctx.point("knot2"), ctx.num("amount2", -50.0) / 100.0)];
    if knots.iter().all(|k| k.1 == 0.0) {
        return b;
    }
    let tile = ctx.flag("tileEdges");
    let n = [1, 2, 3][ctx.choice("antialiasing").min(2) as usize];
    let falloff = ctx.num("falloff", 2.0).max(0.01);
    let r = ctx.bounds;
    let reach = r.width().hypot(r.height()) * 0.5;
    // where a point of the layer (its units) takes its colour from
    let from = |x: f64, y: f64| {
        let (mut x, mut y) = (x, y);
        for (k, a) in knots {
            if a == 0.0 {
                continue;
            }
            let (dx, dy) = (x - k.x, y - k.y);
            let w = (1.0 - (dx.hypot(dy) / reach).min(1.0)).powf(falloff);
            // above 0 the picture near the knot comes from nearer it (it flows out); below, from further
            let s = (1.0 - a.clamp(-4.0, 0.95) * w).max(0.02);
            (x, y) = (k.x + dx * s, k.y + dy * s);
        }
        if tile { (r.x0 + (x - r.x0).rem_euclid(r.width().max(1e-9)), r.y0 + (y - r.y0).rem_euclid(r.height().max(1e-9))) } else { (x, y) }
    };
    let src = b.img.clone();
    let (s, o) = (b.scale, b.origin);
    let (a, z) = (b.pixel(veymelo_geom::vec2(r.x0, r.y0)), b.pixel(veymelo_geom::vec2(r.x1, r.y1)));
    let edge = (a.x.max(0.0), a.y.max(0.0), z.x.min(src.width as f64), z.y.min(src.height as f64));
    map_px(&mut b.img, |x, y, _| {
        let mut acc = [0f32; 4];
        for j in 0..n {
            for i in 0..n {
                let (px, py) = (x as f64 + (i as f64 + 0.5) / n as f64, y as f64 + (j as f64 + 0.5) / n as f64);
                let (lx, ly) = from(o.x + px / s, o.y + py / s);
                let (mut sx, mut sy) = ((lx - o.x) * s, (ly - o.y) * s);
                if tile {
                    // tiles meet without a seam: their edge pixels go on
                    (sx, sy) = (sx.clamp(edge.0 + 0.5, edge.2 - 0.5), sy.clamp(edge.1 + 0.5, edge.3 - 0.5));
                }
                let q = src.sample(Sampling::Bilinear, sx, sy);
                for k in 0..4 {
                    acc[k] += q[k];
                }
            }
        }
        acc.map(|v| v / (n * n) as f32)
    });
    b
}

/// Liquify's brushes, by the names the strokes use.
pub const TOOLS: &[&str] = &["warp", "turbulence", "twirlClockwise", "twirlCounterclockwise", "pucker", "bloat", "shiftPixels", "reflection", "clone", "reconstruction", "freeze", "thaw"];
const RECONSTRUCTION: &[&str] = &["revert", "displace", "amplitwist", "affine"];

effect!(
    LIQUIFY_DEF,
    "liquify",
    "Liquify",
    "the picture pushed, twirled, puckered or bloated by brush strokes (made with the liquify.stroke command)",
    [
        choice("tool", "Tool", TOOLS, 0).doc("the brush the next stroke uses"),
        ranged("brushSize", "Brush Size", 64.0, 1.0, 600.0),
        ranged("brushPressure", "Brush Pressure", 50.0, 1.0, 100.0),
        num("freezeAreaMask", "Freeze Area Mask", 0.0).range(0.0, 1000.0).doc("a mask (from 1) inside which nothing moves; 0: none"),
        ranged("turbulentJitter", "Turbulent Jitter", 70.0, 1.0, 100.0),
        point("cloneOffset", "Clone Offset").doc("for the clone brush: where it copies from, from where it paints"),
        choice("reconstructionMode", "Reconstruction Mode", RECONSTRUCTION, 0).doc("what the reconstruction brush restores toward: nothing moved, or how the start of the stroke moved"),
        flag("viewFreezeAreaMask", "View Freeze Area Mask", false),
        flag("viewMesh", "View Mesh", false),
        choice("meshSize", "Mesh Size", &["small", "medium", "large"], 1),
        color("meshColor", "Mesh Color", 0.5, 0.5, 0.5),
        veymelo_project::PropDef::new("distortionMesh", "Distortion Mesh", veymelo_project::Kind::Text, veymelo_keyframe::Value::Text(String::new()))
            .doc("the strokes, one a line: \"tool size pressure jitter cloneX cloneY [mode] x,y x,y …\" in the layer's units"),
        point("distortionMeshOffset", "Distortion Mesh Offset"),
        ranged("distortionPercentage", "Distortion Percentage", 100.0, 0.0, 200.0),
        choice("distortionDisplayQuality", "Distortion Display Quality", &["draft", "smooth"], 1),
    ]
);
pub const LIQUIFY: Spec = Spec::new(&LIQUIFY_DEF, CATEGORY, liquify);

/// One brush stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub tool: usize,
    /// Across (layer units), how hard (1 to 100), and the turbulence's jitter (1 to 100).
    pub size: f64,
    pub pressure: f64,
    pub jitter: f64,
    /// For the clone brush: where it copies from, from where it paints.
    pub clone_offset: [f64; 2],
    /// For the reconstruction brush: what it restores toward (of [`RECONSTRUCTION`]).
    pub mode: usize,
    pub points: Vec<[f64; 2]>,
}

impl Stroke {
    /// As a line of the Distortion Mesh text.
    pub fn line(&self) -> String {
        let mut s = format!("{} {} {} {} {} {}", TOOLS[self.tool], self.size, self.pressure, self.jitter, self.clone_offset[0], self.clone_offset[1]);
        if self.mode != 0 {
            s.push(' ');
            s.push_str(RECONSTRUCTION[self.mode]);
        }
        for p in &self.points {
            s.push_str(&format!(" {},{}", p[0], p[1]));
        }
        s
    }

    pub fn parse(line: &str) -> Option<Stroke> {
        let mut words = line.split_whitespace().peekable();
        let name = words.next()?;
        let tool = TOOLS.iter().position(|t| t.eq_ignore_ascii_case(name))?;
        let mut num = || -> Option<f64> { words.next()?.parse().ok() };
        let (size, pressure, jitter, cx, cy) = (num()?, num()?, num()?, num()?, num()?);
        let mode = match words.peek().and_then(|w| RECONSTRUCTION.iter().position(|m| m.eq_ignore_ascii_case(w))) {
            Some(m) => {
                words.next();
                m
            }
            None => 0,
        };
        let points: Vec<[f64; 2]> = words
            .filter_map(|t| {
                let (a, b) = t.split_once(',')?;
                Some([a.parse().ok()?, b.parse().ok()?])
            })
            .collect();
        (!points.is_empty()).then_some(Stroke { tool, size, pressure, jitter, clone_offset: [cx, cy], mode, points })
    }
}

/// The strokes in a Distortion Mesh text (lines that are not strokes are passed over).
pub fn strokes(text: &str) -> Vec<Stroke> {
    text.lines().filter_map(Stroke::parse).collect()
}

/// How far each point of a grid over the layer moved (from where its colour comes, layer
/// units), and how frozen it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub nx: usize,
    pub ny: usize,
    /// The gap between grid points, and the layer's corner the grid starts at.
    pub cell: f64,
    pub corner: [f64; 2],
    pub d: Vec<[f64; 2]>,
    pub frozen: Vec<f64>,
}

impl Mesh {
    fn new(corner: [f64; 2], w: f64, h: f64, cell: f64) -> Mesh {
        let (nx, ny) = ((w / cell).ceil() as usize + 1, (h / cell).ceil() as usize + 1);
        Mesh { nx, ny, cell, corner, d: vec![[0.0; 2]; nx * ny], frozen: vec![0.0; nx * ny] }
    }

    fn get(&self, i: i64, j: i64) -> [f64; 2] {
        self.d[(j.clamp(0, self.ny as i64 - 1) as usize) * self.nx + i.clamp(0, self.nx as i64 - 1) as usize]
    }

    /// How far the point (layer units) moved: in straight lines between grid points, or
    /// (`smooth`) along curves through them. Nothing moves off the grid.
    pub fn at(&self, x: f64, y: f64, smooth: bool) -> [f64; 2] {
        let (fx, fy) = ((x - self.corner[0]) / self.cell, (y - self.corner[1]) / self.cell);
        if fx < 0.0 || fy < 0.0 || fx > (self.nx - 1) as f64 || fy > (self.ny - 1) as f64 {
            return [0.0; 2];
        }
        let (i, j) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        if smooth {
            let w = |t: f64| {
                let (t2, t3) = (t * t, t * t * t);
                [-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2]
            };
            let (wx, wy) = (w(tx), w(ty));
            let mut out = [0.0; 2];
            for (b, vy) in wy.iter().enumerate() {
                for (a, ux) in wx.iter().enumerate() {
                    let m = self.get(i - 1 + a as i64, j - 1 + b as i64);
                    out[0] += m[0] * ux * vy;
                    out[1] += m[1] * ux * vy;
                }
            }
            return out;
        }
        let l = |a: [f64; 2], b: [f64; 2], t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        l(l(self.get(i, j), self.get(i + 1, j), tx), l(self.get(i, j + 1), self.get(i + 1, j + 1), tx), ty)
    }

    /// How the move changes across the point: the move's slope along x and along y.
    fn slope(&self, x: f64, y: f64) -> [[f64; 2]; 2] {
        let e = self.cell;
        let (a, b, c, d) = (self.at(x + e, y, false), self.at(x - e, y, false), self.at(x, y + e, false), self.at(x, y - e, false));
        [[(a[0] - b[0]) / (2.0 * e), (a[1] - b[1]) / (2.0 * e)], [(c[0] - d[0]) / (2.0 * e), (c[1] - d[1]) / (2.0 * e)]]
    }
}

/// The strokes played back over a layer with rectangle `corner` + `w` × `h`; `frozen` says which
/// points the freeze mask holds.
pub fn build(strokes: &[Stroke], corner: [f64; 2], w: f64, h: f64, frozen: &dyn Fn(f64, f64) -> bool) -> Mesh {
    let cell = (w.max(h) / 128.0).clamp(2.0, 16.0);
    let mut m = Mesh::new(corner, w, h, cell);
    for (k, f) in m.frozen.iter_mut().enumerate() {
        if frozen(corner[0] + (k % m.nx) as f64 * cell, corner[1] + (k / m.nx) as f64 * cell) {
            *f = 1.0;
        }
    }
    for (si, s) in strokes.iter().enumerate() {
        let r = (s.size * 0.5).max(1.0);
        let pressure = (s.pressure / 100.0).clamp(0.0, 1.0);
        let tool = TOOLS[s.tool];
        // dabs every quarter of the brush along the stroke, each knowing which way the brush moved
        let mut dabs: Vec<([f64; 2], [f64; 2])> = Vec::new();
        if s.points.len() == 1 {
            dabs.push((s.points[0], [0.0; 2]));
        }
        for pair in s.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let n = (((b[0] - a[0]).hypot(b[1] - a[1]) / (r * 0.25)).ceil() as usize).max(1);
            let step = [(b[0] - a[0]) / n as f64, (b[1] - a[1]) / n as f64];
            for i in 1..=n {
                dabs.push(([a[0] + step[0] * i as f64, a[1] + step[1] * i as f64], step));
            }
        }
        // what reconstruction restores toward: how the stroke's start moved, carried on
        let start = s.points[0];
        let (d0, slope) = (m.at(start[0], start[1], false), m.slope(start[0], start[1]));
        let keep = match s.mode {
            // only how far it moved
            1 => [[0.0; 2]; 2],
            // turned and scaled alike, the nearest to how it bent
            2 => {
                let (a, b) = ((slope[0][0] + slope[1][1]) * 0.5, (slope[0][1] - slope[1][0]) * 0.5);
                [[a, b], [-b, a]]
            }
            3 => slope,
            _ => [[0.0; 2]; 2],
        };
        let toward = |x: f64, y: f64| -> [f64; 2] {
            if s.mode == 0 {
                return [0.0; 2];
            }
            let (dx, dy) = (x - start[0], y - start[1]);
            [d0[0] + keep[0][0] * dx + keep[1][0] * dy, d0[1] + keep[0][1] * dx + keep[1][1] * dy]
        };
        for (di, (c, mv)) in dabs.iter().enumerate() {
            let gx = (((c[0] - r - corner[0]) / cell).floor().max(0.0) as usize, (((c[0] + r - corner[0]) / cell).ceil().max(0.0) as usize).min(m.nx - 1));
            let gy = (((c[1] - r - corner[1]) / cell).floor().max(0.0) as usize, (((c[1] + r - corner[1]) / cell).ceil().max(0.0) as usize).min(m.ny - 1));
            let before = m.d.clone();
            for j in gy.0..=gy.1 {
                for i in gx.0..=gx.1 {
                    let (x, y) = (corner[0] + i as f64 * cell, corner[1] + j as f64 * cell);
                    let (dx, dy) = (x - c[0], y - c[1]);
                    let dist = dx.hypot(dy);
                    if dist >= r {
                        continue;
                    }
                    let k = j * m.nx + i;
                    let t = 1.0 - dist / r;
                    let fall = t * t * (3.0 - 2.0 * t);
                    if tool == "freeze" || tool == "thaw" {
                        let v = pressure * fall;
                        m.frozen[k] = if tool == "freeze" { m.frozen[k].max(v) } else { (m.frozen[k] - v).max(0.0) };
                        continue;
                    }
                    let w = pressure * fall * (1.0 - m.frozen[k]) * 0.5;
                    if w <= 0.0 {
                        continue;
                    }
                    let d = &mut m.d[k];
                    match tool {
                        "warp" => {
                            d[0] -= mv[0] * w * 2.0;
                            d[1] -= mv[1] * w * 2.0;
                        }
                        // pushed along, and shaken by smooth noise about half the brush across
                        // that changes from dab to dab
                        "turbulence" => {
                            let j2 = s.jitter / 100.0 * r * 0.25;
                            let shake = |n: u64| crate::noise::smooth(x / (r * 0.5), y / (r * 0.5), di as f64 * 0.2, crate::noise::mix(&[si as u64, n]), None);
                            d[0] += shake(0) * j2 * w - mv[0] * w;
                            d[1] += shake(1) * j2 * w - mv[1] * w;
                        }
                        "twirlClockwise" | "twirlCounterclockwise" => {
                            let (sn, cs) = ((if tool == "twirlClockwise" { -0.08 } else { 0.08 }) * w).sin_cos();
                            d[0] += dx * cs - dy * sn - dx;
                            d[1] += dx * sn + dy * cs - dy;
                        }
                        "pucker" => {
                            d[0] += dx * 0.05 * w;
                            d[1] += dy * 0.05 * w;
                        }
                        "bloat" => {
                            d[0] -= dx * 0.05 * w;
                            d[1] -= dy * 0.05 * w;
                        }
                        // across the stroke, to its left
                        "shiftPixels" => {
                            d[0] -= mv[1] * w * 2.0;
                            d[1] += mv[0] * w * 2.0;
                        }
                        // what is on the other side of the stroke's line
                        "reflection" => {
                            let l = mv[0].hypot(mv[1]);
                            if l > 1e-9 {
                                let n = [-mv[1] / l, mv[0] / l];
                                let q = dx * n[0] + dy * n[1];
                                d[0] += (-2.0 * q * n[0] - d[0]) * w;
                                d[1] += (-2.0 * q * n[1] - d[1]) * w;
                            }
                        }
                        // the moves found at the clone's source
                        "clone" => {
                            let (fx, fy) = (((x + s.clone_offset[0] - corner[0]) / cell).round(), ((y + s.clone_offset[1] - corner[1]) / cell).round());
                            let src = before[(fy.clamp(0.0, (m.ny - 1) as f64) as usize) * m.nx + fx.clamp(0.0, (m.nx - 1) as f64) as usize];
                            d[0] += (src[0] + s.clone_offset[0] - d[0]) * w;
                            d[1] += (src[1] + s.clone_offset[1] - d[1]) * w;
                        }
                        _ => {
                            let to = toward(x, y);
                            d[0] += (to[0] - d[0]) * w;
                            d[1] += (to[1] - d[1]) * w;
                        }
                    }
                }
            }
        }
    }
    m
}

type Meshes = Mutex<HashMap<u64, Arc<Mesh>>>;

/// The mesh an instance's strokes make (kept, for the next frame and the next look).
pub fn mesh_of(ctx: &Ctx) -> Arc<Mesh> {
    let text = ctx.value_in(ctx.effect, "distortionMesh").map(|v| v.as_text().to_string()).unwrap_or_default();
    let mask = ctx.mask(ctx.num("freezeAreaMask", 0.0).round().max(0.0) as usize).cloned();
    let r = ctx.bounds;
    let mut key = vec![crate::noise::mix(&text.bytes().map(u64::from).collect::<Vec<_>>()), r.x0.to_bits(), r.y0.to_bits(), r.x1.to_bits(), r.y1.to_bits()];
    if let Some(m) = &mask {
        key.push(u64::from(m.inverted));
        key.extend(m.points.iter().flat_map(|p| [p[0].to_bits(), p[1].to_bits()]));
    }
    let key = crate::noise::mix(&key);
    static KEPT: OnceLock<Meshes> = OnceLock::new();
    let kept = KEPT.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(m) = kept.lock().ok().and_then(|k| k.get(&key).cloned()) {
        return m;
    }
    let held = |x: f64, y: f64| mask.as_ref().is_some_and(|m| inside(&m.points, [x, y]) != m.inverted);
    let m = Arc::new(build(&strokes(&text), [r.x0, r.y0], r.width().max(1.0), r.height().max(1.0), &held));
    if let Ok(mut k) = kept.lock() {
        if k.len() > 32 {
            k.clear();
        }
        k.insert(key, m.clone());
    }
    m
}

fn liquify(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = ctx.num("distortionPercentage", 100.0) / 100.0;
    let (show_frozen, show_mesh) = (ctx.flag("viewFreezeAreaMask"), ctx.flag("viewMesh"));
    let empty = ctx.value_in(ctx.effect, "distortionMesh").is_none_or(|v| v.as_text().trim().is_empty());
    if empty && !show_frozen && !show_mesh {
        return b;
    }
    let mesh = mesh_of(ctx);
    let off = ctx.point("distortionMeshOffset");
    let smooth = ctx.choice("distortionDisplayQuality") == 1;
    let line_c = rgb3(ctx.color("meshColor"));
    let every = mesh.cell * [2.0, 4.0, 8.0][ctx.choice("meshSize").min(2) as usize];
    let src = b.img.clone();
    let (s, o) = (b.scale, b.origin);
    let line = 0.6 / s.max(1e-6);
    map_px(&mut b.img, |x, y, p| {
        let (lx, ly) = (o.x + (x as f64 + 0.5) / s, o.y + (y as f64 + 0.5) / s);
        let d = mesh.at(lx - off.x, ly - off.y, smooth);
        let mut c: Px = if d == [0.0; 2] { p } else { src.sample(Sampling::Bilinear, (lx + d[0] * k - o.x) * s, (ly + d[1] * k - o.y) * s) };
        if show_frozen {
            let (i, j) = (((lx - off.x - mesh.corner[0]) / mesh.cell).round(), ((ly - off.y - mesh.corner[1]) / mesh.cell).round());
            let f = mesh.frozen[(j.clamp(0.0, (mesh.ny - 1) as f64) as usize) * mesh.nx + i.clamp(0.0, (mesh.nx - 1) as f64) as usize] as f32;
            if f > 0.0 {
                c[0] = c[0] * (1.0 - 0.5 * f) + 0.5 * f;
                c[3] = c[3].max(0.5 * f);
            }
        }
        if show_mesh {
            // the grid's lines, bent with it
            let (gx, gy) = ((lx + d[0] * k - off.x - mesh.corner[0]).rem_euclid(every), (ly + d[1] * k - off.y - mesh.corner[1]).rem_euclid(every));
            if gx < line || gy < line {
                c = [line_c[0], line_c[1], line_c[2], 1.0];
            }
        }
        c
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 2] = [FLOW, LIQUIFY];

#[cfg(test)]
mod tests;
