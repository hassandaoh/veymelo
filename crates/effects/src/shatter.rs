//! Shatter: the layer broken into pieces — bricks, squares, hexagons, triangles, planks, shards
//! of glass, panes, rings, jigsaw pieces or a map's colours — that fly apart in 3D where a force
//! touches them (and where a gradient lets them), tumbling, falling, slowed by the air, given
//! depth and lit; or the pieces seen as lines.
//!
//! A piece breaks the first time one of the two force spheres reaches it (the forces may move);
//! from then its flight is worked out from the time alone.

use std::f64::consts::{PI, TAU};

use veymelo_raster::Image;
use veymelo_time::Tick;

use crate::defs::{angle, choice, color, flag, layer, pct, point, ranged};
use crate::scene3d::{self, Camera, Light, M3, Piece, Placed, scene_effect};
use crate::sprites::{Shape, Sprite, draw, rand, rand2};
use crate::util::{luma, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const PATTERNS: &[&str] = &["bricks", "concentric", "glass", "glassPanes", "hexagons", "planks", "puzzle", "squares", "triangles1", "triangles2", "custom"];
const TEXTURE_MODES: &[&str] = &["color", "layer", "tintedLayer"];

scene_effect!(
    SHATTER_DEF,
    "shatter",
    "Shatter",
    "the layer broken into pieces of a pattern that fly apart in 3D where a force reaches them",
    z: 2.0, light: 1.0, ambient: 0.25, diffuse: 0.75,
    [
        choice("view", "View", &["rendered", "wireframeFrontView", "wireframe", "wireframeFrontViewForces", "wireframeForces"], 0),
        choice("render", "Render", &["all", "layer", "pieces"], 0).doc("layer: only what has not broken; pieces: only what has"),
        choice("pattern", "Pattern", PATTERNS, 0),
        layer("customShatterMap", "Custom Shatter Map").doc("for the custom pattern: each colour of it one piece"),
        flag("whiteTilesFixed", "White Tiles Fixed", false).doc("the custom map's white stays put"),
        ranged("repetitions", "Repetitions", 10.0, 1.0, 500.0).doc("how many pieces across the layer"),
        angle("direction", "Direction", 0.0),
        point("origin", "Origin"),
        ranged("extrusionDepth", "Extrusion Depth", 0.0, 0.0, 1.0).doc("how thick the pieces are, in pieces"),
        point("force1Position", "Force 1 Position"),
        ranged("force1Depth", "Force 1 Depth", 0.1, -1.0, 1.0),
        ranged("force1Radius", "Force 1 Radius", 0.4, 0.0, 4.0).doc("in layer widths"),
        ranged("force1Strength", "Force 1 Strength", 5.0, -20.0, 20.0),
        point("force2Position", "Force 2 Position"),
        ranged("force2Depth", "Force 2 Depth", 0.1, -1.0, 1.0),
        ranged("force2Radius", "Force 2 Radius", 0.0, 0.0, 4.0),
        ranged("force2Strength", "Force 2 Strength", 5.0, -20.0, 20.0),
        pct("shatterThreshold", "Shatter Threshold", 0.0, 0.0, 100.0).doc("with a gradient layer: the brightest this much of it may break"),
        layer("gradientLayer", "Gradient Layer"),
        flag("invertGradient", "Invert Gradient", false),
        ranged("rotationSpeed", "Rotation Speed", 0.2, 0.0, 5.0),
        choice("tumbleAxis", "Tumble Axis", &["free", "none", "x", "y", "z", "xy", "xz", "yz"], 0),
        ranged("randomness", "Randomness", 0.1, 0.0, 1.0),
        ranged("viscosity", "Viscosity", 0.1, 0.0, 1.0),
        pct("massVariance", "Mass Variance", 30.0, 0.0, 100.0),
        ranged("gravity", "Gravity", 3.0, 0.0, 100.0),
        angle("gravityDirection", "Gravity Direction", 180.0),
        ranged("gravityInclination", "Gravity Inclination", 0.0, -90.0, 90.0),
        color("textureColor", "Color", 1.0, 1.0, 1.0),
        ranged("opacity", "Opacity", 1.0, 0.0, 1.0),
        choice("frontMode", "Front Mode", TEXTURE_MODES, 1),
        layer("frontLayer", "Front Layer").doc("none: the layer"),
        choice("sideMode", "Side Mode", TEXTURE_MODES, 0),
        layer("sideLayer", "Side Layer"),
        choice("backMode", "Back Mode", TEXTURE_MODES, 1),
        layer("backLayer", "Back Layer"),
        ranged("randomSeed", "Random Seed", 0.0, 0.0, 10_000.0),
    ]
);
pub const SHATTER: Spec = Spec::new(&SHATTER_DEF, "simulation", shatter).points(&[
    ("xyPosition", [0.5, 0.5]),
    ("upperLeftCorner", [0.0, 0.0]),
    ("upperRightCorner", [1.0, 0.0]),
    ("lowerLeftCorner", [0.0, 1.0]),
    ("lowerRightCorner", [1.0, 1.0]),
    ("lightPosition", [0.5, 0.5]),
    ("origin", [0.5, 0.5]),
    ("force1Position", [0.5, 0.5]),
    ("force2Position", [0.25, 0.25]),
]);

type Poly = Vec<[f64; 2]>;

/// A random number for a lattice point (i, j).
fn at(i: i64, j: i64, k: u64, seed: u64) -> f64 {
    rand(i as u64 ^ ((j as u64) << 32), k, seed) as f64
}

/// The part of `poly` on the side of the line through `p` facing away from `n` (n · (x − p) ≤ 0).
fn clip(poly: &[[f64; 2]], p: [f64; 2], n: [f64; 2]) -> Poly {
    let side = |q: [f64; 2]| (q[0] - p[0]) * n[0] + (q[1] - p[1]) * n[1];
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (sa, sb) = (side(a), side(b));
        if sa <= 0.0 {
            out.push(a);
        }
        if (sa <= 0.0) != (sb <= 0.0) {
            let t = sa / (sa - sb);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    out
}

/// A jigsaw edge from `a` to `b`, its tab bulging to the left (`out`) or right.
fn tab_edge(a: [f64; 2], b: [f64; 2], out: bool) -> Poly {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (nx, ny) = if out { (-dy, dx) } else { (dy, -dx) };
    let point = |t: f64, h: f64| [a[0] + dx * t + nx * h, a[1] + dy * t + ny * h];
    let mut pts = vec![a, point(0.38, 0.0), point(0.36, 0.08)];
    // the knob: round, its neck narrower
    for k in 0..=10 {
        let th = PI * 1.25 - k as f64 / 10.0 * PI * 1.5;
        pts.push(point(0.5 + 0.13 * th.cos(), 0.18 + 0.13 * th.sin()));
    }
    pts.extend([point(0.64, 0.08), point(0.62, 0.0)]);
    pts
}

/// The pattern's pieces (in pattern space, `cell` a piece's size) covering x0..x1, y0..y1.
fn pattern(kind: u32, cell: f64, x0: f64, y0: f64, x1: f64, y1: f64, seed: u64) -> Vec<Poly> {
    let (i0, i1) = ((x0 / cell).floor() as i64 - 1, (x1 / cell).ceil() as i64 + 1);
    let (j0, j1) = ((y0 / cell).floor() as i64 - 1, (y1 / cell).ceil() as i64 + 1);
    let square = |x: f64, y: f64, w: f64, h: f64| vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    let mut out = Vec::new();
    match kind {
        // bricks: rows half a piece high, every other one moved half a piece
        0 => {
            let h = cell * 0.5;
            for r in ((y0 / h).floor() as i64 - 1)..=((y1 / h).ceil() as i64 + 1) {
                let sh = if r.rem_euclid(2) == 1 { cell * 0.5 } else { 0.0 };
                for i in i0..=i1 {
                    out.push(square(i as f64 * cell + sh, r as f64 * h, cell, h));
                }
            }
        }
        // rings round the origin, each cut into pieces about as long as wide
        1 => {
            let far = [x0.abs(), x1.abs()].iter().fold(0f64, |m, v| m.max(*v)).hypot([y0.abs(), y1.abs()].iter().fold(0f64, |m, v| m.max(*v)));
            for ring in 0..((far / cell).ceil() as i64 + 1) {
                let (r0, r1) = (ring as f64 * cell, (ring + 1) as f64 * cell);
                let count = ((TAU * (r0 + r1) * 0.5 / cell).round() as i64).max(if ring == 0 { 1 } else { 3 });
                let turn = at(ring, 0, 3, seed) * TAU;
                for k in 0..count {
                    let (a0, a1) = (turn + k as f64 / count as f64 * TAU, turn + (k + 1) as f64 / count as f64 * TAU);
                    let steps = ((a1 - a0) * r1 / (cell * 0.25)).ceil().max(2.0) as usize;
                    let mut p: Poly = (0..=steps).map(|s| {
                        let a = a0 + (a1 - a0) * s as f64 / steps as f64;
                        [r1 * a.cos(), r1 * a.sin()]
                    }).collect();
                    if ring > 0 {
                        p.extend((0..=steps).rev().map(|s| {
                            let a = a0 + (a1 - a0) * s as f64 / steps as f64;
                            [r0 * a.cos(), r0 * a.sin()]
                        }));
                    } else {
                        p.push([0.0, 0.0]);
                    }
                    out.push(p);
                }
            }
        }
        // shards of glass: the cells of points scattered about a grid, each the area nearer
        // its point than any other
        2 => {
            let site = |i: i64, j: i64| [(i as f64 + 0.15 + 0.7 * at(i, j, 1, seed)) * cell, (j as f64 + 0.15 + 0.7 * at(i, j, 2, seed)) * cell];
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let c = site(i, j);
                    let mut p = square(c[0] - 2.0 * cell, c[1] - 2.0 * cell, 4.0 * cell, 4.0 * cell);
                    for dj in -2..=2 {
                        for di in -2..=2 {
                            if di == 0 && dj == 0 {
                                continue;
                            }
                            let o = site(i + di, j + dj);
                            p = clip(&p, [(c[0] + o[0]) * 0.5, (c[1] + o[1]) * 0.5], [o[0] - c[0], o[1] - c[1]]);
                        }
                    }
                    if p.len() >= 3 {
                        out.push(p);
                    }
                }
            }
        }
        // panes: blocks of two by two pieces, each cut across or down once or twice
        3 => {
            for j in (j0.div_euclid(2) * 2..=j1).step_by(2) {
                for i in (i0.div_euclid(2) * 2..=i1).step_by(2) {
                    let (x, y, s2) = (i as f64 * cell, j as f64 * cell, 2.0 * cell);
                    let cut = 0.3 + 0.4 * at(i, j, 4, seed);
                    if at(i, j, 5, seed) < 0.5 {
                        let c2 = 0.3 + 0.4 * at(i, j, 6, seed);
                        out.push(square(x, y, s2 * cut, s2));
                        out.push(square(x + s2 * cut, y, s2 * (1.0 - cut), s2 * c2));
                        out.push(square(x + s2 * cut, y + s2 * c2, s2 * (1.0 - cut), s2 * (1.0 - c2)));
                    } else {
                        let c2 = 0.3 + 0.4 * at(i, j, 6, seed);
                        out.push(square(x, y, s2, s2 * cut));
                        out.push(square(x, y + s2 * cut, s2 * c2, s2 * (1.0 - cut)));
                        out.push(square(x + s2 * c2, y + s2 * cut, s2 * (1.0 - c2), s2 * (1.0 - cut)));
                    }
                }
            }
        }
        4 => {
            let r = cell / 3f64.sqrt();
            let (dx, dy) = (1.5 * r, 3f64.sqrt() * r);
            for ci in ((x0 / dx).floor() as i64 - 1)..=((x1 / dx).ceil() as i64 + 1) {
                for rj in ((y0 / dy).floor() as i64 - 1)..=((y1 / dy).ceil() as i64 + 1) {
                    let (cx, cy) = (ci as f64 * dx, (rj as f64 + if ci.rem_euclid(2) == 1 { 0.5 } else { 0.0 }) * dy);
                    out.push((0..6).map(|k| {
                        let a = k as f64 * PI / 3.0;
                        [cx + r * a.cos(), cy + r * a.sin()]
                    }).collect());
                }
            }
        }
        // planks: rows a third of a piece high, of boards two to four pieces long
        5 => {
            let h = cell / 3.0;
            for r in ((y0 / h).floor() as i64 - 1)..=((y1 / h).ceil() as i64 + 1) {
                let mut x = x0 - 4.0 * cell - at(r, 0, 7, seed) * 3.0 * cell;
                let mut k = 0;
                while x < x1 + cell {
                    let w = cell * (2.0 + 2.0 * at(r, k, 8, seed));
                    out.push(square(x, r as f64 * h, w, h));
                    x += w;
                    k += 1;
                }
            }
        }
        // jigsaw pieces: each inner edge with a knob, out of one piece and into its neighbour
        6 => {
            let knob = |i: i64, j: i64, k: u64| at(i, j, k, seed) < 0.5;
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (x, y) = (i as f64 * cell, j as f64 * cell);
                    let (tl, tr, br, bl) = ([x, y], [x + cell, y], [x + cell, y + cell], [x, y + cell]);
                    // the edge shared with the piece above is that piece's bottom, the other way
                    let mut p = Vec::new();
                    p.extend(tab_edge(tl, tr, !knob(i, j, 10)));
                    p.extend(tab_edge(tr, br, knob(i + 1, j, 11)));
                    p.extend(tab_edge(br, bl, knob(i, j + 1, 10)));
                    p.extend(tab_edge(bl, tl, !knob(i, j, 11)));
                    out.push(p);
                }
            }
        }
        // triangles: squares cut along one diagonal (the first pattern alternating it)
        8 | 9 => {
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (x, y) = (i as f64 * cell, j as f64 * cell);
                    if kind == 8 && (i + j).rem_euclid(2) == 0 {
                        out.push(vec![[x, y], [x + cell, y], [x, y + cell]]);
                        out.push(vec![[x + cell, y], [x + cell, y + cell], [x, y + cell]]);
                    } else {
                        out.push(vec![[x, y], [x + cell, y], [x + cell, y + cell]]);
                        out.push(vec![[x, y], [x + cell, y + cell], [x, y + cell]]);
                    }
                }
            }
        }
        _ => {
            for j in j0..=j1 {
                for i in i0..=i1 {
                    out.push(square(i as f64 * cell, j as f64 * cell, cell, cell));
                }
            }
        }
    }
    out
}

/// The flight's two factors with the air slowing it by `k`: ∫e^(−ks)ds and ∫∫e^(−ks)ds du from 0
/// to `t`.
fn slowed(k: f64, t: f64) -> (f64, f64) {
    if k < 1e-6 {
        (t, 0.5 * t * t)
    } else {
        let e = (-k * t).exp();
        ((1.0 - e) / k, t / k - (1.0 - e) / (k * k))
    }
}

/// A force sphere at a moment (layer units): middle, radius and strength.
#[derive(Clone, Copy)]
struct Force {
    at: [f64; 3],
    radius: f64,
    strength: f64,
}

fn forces(ctx: &Ctx, t: f64, width: f64) -> [Force; 2] {
    let tt = Tick::from_seconds(t);
    let num = |id: &str, d: f64| ctx.value_at(id, tt).map_or(d, |v| v.as_f64());
    let pt = |id: &str| ctx.value_at(id, tt).map_or([0.0; 2], |v| v.as_vec2());
    [1, 2].map(|k| {
        let p = pt(&format!("force{k}Position"));
        Force { at: [p[0], p[1], num(&format!("force{k}Depth"), 0.1) * width], radius: num(&format!("force{k}Radius"), if k == 1 { 0.4 } else { 0.0 }).max(0.0) * width, strength: num(&format!("force{k}Strength"), 5.0) }
    })
}

/// A face's texture: one colour, a layer, or a layer tinted by the colour.
struct Face {
    mode: u32,
    img: Option<Image>,
}

impl Face {
    fn of(ctx: &Ctx, b: &Buf, mode: &str, layer: &str) -> Face {
        let mode = ctx.choice(mode);
        Face { mode, img: (mode != 0).then(|| ctx.layer_over(layer, b, Take::All, true).unwrap_or_else(|| b.img.clone())) }
    }
}

/// A custom map colour's key (pieces of one colour move as one; none where it is clear).
fn colour_key(p: [f32; 4]) -> u32 {
    let (c, a) = unpremul(p);
    if a < 0.5 {
        return u32::MAX;
    }
    let q = |v: f32| ((v.clamp(0.0, 1.0) * 15.0).round() as u32) & 15;
    (q(c[0]) << 8) | (q(c[1]) << 4) | q(c[2])
}

fn shatter(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = ctx.bounds;
    let lw = r.width();
    let s = b.scale;
    let t = ctx.time.seconds().max(0.0);
    let kind = ctx.choice("pattern").min(PATTERNS.len() as u32 - 1);
    let custom = kind as usize == PATTERNS.len() - 1;
    let cell = lw / ctx.num("repetitions", 10.0).clamp(1.0, 500.0);
    let dir = ctx.num("direction", 0.0).to_radians();
    let origin = ctx.point("origin");
    let seed = crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 0.0) as u64, 37]);
    let unit = lw * 0.1;
    let (sd, cd) = dir.sin_cos();
    let to_layer = |q: [f64; 2]| [origin.x + q[0] * cd - q[1] * sd, origin.y + q[0] * sd + q[1] * cd];
    // the layer's corners in pattern space, for the pattern's reach
    let corners = [[r.x0, r.y0], [r.x1, r.y0], [r.x1, r.y1], [r.x0, r.y1]].map(|c| {
        let (dx, dy) = (c[0] - origin.x, c[1] - origin.y);
        [dx * cd + dy * sd, -dx * sd + dy * cd]
    });
    let lo = [corners.iter().map(|c| c[0]).fold(f64::MAX, f64::min), corners.iter().map(|c| c[1]).fold(f64::MAX, f64::min)];
    let hi = [corners.iter().map(|c| c[0]).fold(f64::MIN, f64::max), corners.iter().map(|c| c[1]).fold(f64::MIN, f64::max)];
    let polys: Vec<Poly> = pattern(if custom { 7 } else { kind }, cell, lo[0], lo[1], hi[0], hi[1], seed).into_iter().map(|p| p.into_iter().map(to_layer).collect()).collect();
    let centre = |p: &Poly| [p.iter().map(|v| v[0]).sum::<f64>() / p.len() as f64, p.iter().map(|v| v[1]).sum::<f64>() / p.len() as f64];
    // the custom map: the cells of one colour together
    let map = custom.then(|| ctx.layer_over("customShatterMap", &b, Take::All, true).unwrap_or_else(|| b.img.clone()));
    let keys: Vec<u32> = match &map {
        Some(m) => polys
            .iter()
            .map(|p| {
                let c = centre(p);
                let at = b.pixel(veymelo_geom::vec2(c[0], c[1]));
                colour_key(m.sample(veymelo_raster::Sampling::Bilinear, at.x, at.y))
            })
            .collect(),
        None => (0..polys.len() as u32).collect(),
    };
    let mut groups: std::collections::HashMap<u32, ([f64; 2], f64)> = std::collections::HashMap::new();
    for (p, k) in polys.iter().zip(&keys) {
        let c = centre(p);
        let e = groups.entry(*k).or_insert(([0.0; 2], 0.0));
        (e.0[0], e.0[1], e.1) = (e.0[0] + c[0], e.0[1] + c[1], e.1 + 1.0);
    }
    // when each group breaks: the first moment a force reaches it (and the gradient lets it)
    let gradient = ctx.layer_over("gradientLayer", &b, Take::All, true);
    let invert = ctx.flag("invertGradient");
    let fd = ctx.frame.seconds().max(1e-3);
    let moments: Vec<f64> = {
        let n = ((t / fd).ceil() as usize).min(1200);
        (0..=n).map(|k| if n == 0 { 0.0 } else { t * k as f64 / n as f64 }).collect()
    };
    let samples: Vec<([Force; 2], f64)> = moments.iter().map(|&m| (forces(ctx, m, lw), ctx.value_at("shatterThreshold", Tick::from_seconds(m)).map_or(0.0, |v| v.as_f64()) / 100.0)).collect();
    let white_fixed = ctx.flag("whiteTilesFixed");
    let breaks = |key: u32, c: [f64; 2]| -> Option<(f64, [f64; 3])> {
        if key == u32::MAX || (custom && white_fixed && key == 0xfff) {
            return None;
        }
        let level = gradient.as_ref().map(|g| {
            let at = b.pixel(veymelo_geom::vec2(c[0], c[1]));
            let (cc, _) = unpremul(g.sample(veymelo_raster::Sampling::Bilinear, at.x, at.y));
            let l = luma(cc).clamp(0.0, 1.0) as f64;
            if invert { 1.0 - l } else { l }
        });
        for (k, (fs, threshold)) in samples.iter().enumerate() {
            let mut push = [0.0f64; 3];
            let mut hit = false;
            for f in fs {
                if f.radius <= 0.0 {
                    continue;
                }
                let d = [c[0] - f.at[0], c[1] - f.at[1], -f.at[2]];
                let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if dist < f.radius {
                    hit = true;
                    let k = f.strength * (1.0 - dist / f.radius) * unit / dist.max(1e-6);
                    push = [0, 1, 2].map(|q| push[q] + d[q] * k);
                }
            }
            let allowed = level.is_none_or(|l| *threshold > 0.0 && l >= 1.0 - threshold);
            if hit && allowed {
                return Some((moments[k], push));
            }
        }
        None
    };
    let (rot_speed, tumble, randomness) = (ctx.num("rotationSpeed", 0.2), ctx.choice("tumbleAxis"), ctx.num("randomness", 0.1));
    let k_air = ctx.num("viscosity", 0.1).max(0.0) * 5.0;
    let mass_var = ctx.num("massVariance", 30.0) / 100.0;
    let (g, gdir, ginc) = (ctx.num("gravity", 3.0), ctx.num("gravityDirection", 180.0).to_radians(), ctx.num("gravityInclination", 0.0).to_radians());
    let fall = [gdir.sin() * ginc.cos() * g * unit, -gdir.cos() * ginc.cos() * g * unit, ginc.sin() * g * unit];
    let render = ctx.choice("render");
    let view = ctx.choice("view");
    let cam = Camera::of(ctx, &b);
    let (eye, light) = (cam.eye(), Light::of(ctx, &b));
    let tc = ctx.color("textureColor");
    let tint_c = [tc.r as f32, tc.g as f32, tc.b as f32];
    let opacity = ctx.num("opacity", 1.0).clamp(0.0, 1.0) as f32;
    let (front, side, back) = (Face::of(ctx, &b, "frontMode", "frontLayer"), Face::of(ctx, &b, "sideMode", "sideLayer"), Face::of(ctx, &b, "backMode", "backLayer"));
    let thick = ctx.num("extrusionDepth", 0.0).max(0.0) * cell * s;
    let mut breaking: std::collections::HashMap<u32, Option<(f64, [f64; 3])>> = std::collections::HashMap::new();
    let mut placed: Vec<Placed> = Vec::new();
    let mut lines: Vec<Sprite> = Vec::new();
    for (poly, &key) in polys.iter().zip(&keys) {
        let (mn, mx) = (poly.iter().fold([f64::MAX; 2], |m, v| [m[0].min(v[0]), m[1].min(v[1])]), poly.iter().fold([f64::MIN; 2], |m, v| [m[0].max(v[0]), m[1].max(v[1])]));
        if mx[0] < r.x0 || mx[1] < r.y0 || mn[0] > r.x1 || mn[1] > r.y1 || key == u32::MAX {
            continue;
        }
        let (sum, count) = groups[&key];
        let gc = [sum[0] / count, sum[1] / count];
        let broke = *breaking.entry(key).or_insert_with(|| breaks(key, gc));
        if (broke.is_some() && render == 1) || (broke.is_none() && render == 2) {
            continue;
        }
        let i = key as u64;
        let mut at3 = [gc[0], gc[1], 0.0];
        let mut turn = scene3d::IDENTITY;
        let moving = broke.is_some_and(|(when, _)| t > when);
        if let Some((when, push)) = broke.filter(|_| moving) {
            let since = t - when;
            let mass = 1.0 + mass_var * rand2(i, 1, seed) as f64;
            let v: [f64; 3] = [0, 1, 2].map(|q| push[q] / mass + randomness * unit * rand2(i, 2 + q as u64, seed) as f64 * 2.0);
            let (f1, f2) = slowed(k_air, since);
            at3 = [0, 1, 2].map(|q| at3[q] + v[q] * f1 + fall[q] * f2);
            let axis = match tumble {
                1 => [0.0; 3],
                2 => [1.0, 0.0, 0.0],
                3 => [0.0, 1.0, 0.0],
                4 => [0.0, 0.0, 1.0],
                5 => [rand2(i, 6, seed) as f64, rand2(i, 7, seed) as f64, 0.0],
                6 => [rand2(i, 6, seed) as f64, 0.0, rand2(i, 8, seed) as f64],
                7 => [0.0, rand2(i, 7, seed) as f64, rand2(i, 8, seed) as f64],
                _ => [rand2(i, 6, seed) as f64, rand2(i, 7, seed) as f64, rand2(i, 8, seed) as f64],
            };
            let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() / unit;
            let spin = rot_speed * TAU * (0.5 + rand(i, 9, seed) as f64) * speed.min(10.0);
            turn = scene3d::about(axis, spin * f1);
        }
        // the piece's middle: its place in the group, turned with it
        let c = centre(poly);
        let off = scene3d::apply(&turn, [c[0] - gc[0], c[1] - gc[1], 0.0]);
        let world = [at3[0] + off[0], at3[1] + off[1], at3[2] + off[2]];
        let wpx = b.pixel(veymelo_geom::vec2(world[0], world[1]));
        let pos = [wpx.x, wpx.y, world[2] * s];
        let outline: Vec<[f32; 2]> = poly.iter().map(|v| {
            let p = b.pixel(veymelo_geom::vec2(v[0], v[1]));
            [p.x as f32, p.y as f32]
        }).collect();
        let cpx = b.pixel(veymelo_geom::vec2(c[0], c[1]));
        let rest = [cpx.x as f32, cpx.y as f32];
        if view != 0 {
            // as lines: at rest (front views), or as they fly
            let animated = view == 2 || view == 4;
            let pts: Vec<[f32; 2]> = outline.iter().map(|v| {
                if !animated {
                    return *v;
                }
                let q = scene3d::apply(&turn, [(v[0] - rest[0]) as f64, (v[1] - rest[1]) as f64, 0.0]);
                cam.project([pos[0] + q[0], pos[1] + q[1], pos[2] + q[2]]).map_or(*v, |p| [p[0] as f32, p[1] as f32])
            }).collect();
            for e in 0..pts.len() {
                let (a, z) = (pts[e], pts[(e + 1) % pts.len()]);
                lines.push(Sprite::new(z[0], z[1], 0.5, [0.75, 0.85, 1.0, 1.0], Shape::Line { dx: z[0] - a[0], dy: z[1] - a[1] }));
            }
            continue;
        }
        let piece = Piece { outline: outline.clone(), rest, turn, at: pos, alpha: opacity, flat: None, mirror: 0 };
        let Some(mut p) = piece.place(&cam) else { continue };
        p = p.lit(&light, eye);
        let face = if p.back { &back } else { &front };
        p = match face.mode {
            0 => p.flat(tint_c),
            2 => p.tinted(tint_c),
            _ => p,
        };
        // the sides of a thick piece in flight: one for each edge, back from the face
        if moving && thick > 0.0 {
            let nb = [turn[0][2], turn[1][2], turn[2][2]];
            for e in 0..outline.len() {
                let (a, z) = (outline[e], outline[(e + 1) % outline.len()]);
                let ed = [(z[0] - a[0]) as f64, (z[1] - a[1]) as f64];
                let len = ed[0].hypot(ed[1]);
                if len < 1e-6 {
                    continue;
                }
                let along: [f64; 3] = [0, 1, 2].map(|q| (turn[q][0] * ed[0] + turn[q][1] * ed[1]) / len);
                let third = [along[1] * nb[2] - along[2] * nb[1], along[2] * nb[0] - along[0] * nb[2], along[0] * nb[1] - along[1] * nb[0]];
                let side_turn: M3 = [0, 1, 2].map(|q| [along[q], nb[q], third[q]]);
                let mid = [((a[0] + z[0]) * 0.5 - rest[0]) as f64, ((a[1] + z[1]) * 0.5 - rest[1]) as f64];
                let m3: [f64; 3] = [0, 1, 2].map(|q| pos[q] + turn[q][0] * mid[0] + turn[q][1] * mid[1] + nb[q] * thick * 0.5);
                let (hl, hd) = ((len * 0.5) as f32, (thick * 0.5) as f32);
                let cs = [(a[0] + z[0]) * 0.5, (a[1] + z[1]) * 0.5];
                let quad = vec![[cs[0] - hl, cs[1] - hd], [cs[0] + hl, cs[1] - hd], [cs[0] + hl, cs[1] + hd], [cs[0] - hl, cs[1] + hd]];
                let colour = match (&side.img, side.mode) {
                    (Some(img), m) if m != 0 => {
                        let (cc, _) = unpremul(img.sample(veymelo_raster::Sampling::Bilinear, cs[0] as f64, cs[1] as f64));
                        if m == 2 { [0, 1, 2].map(|q| cc[q] * tint_c[q]) } else { cc }
                    }
                    _ => tint_c,
                };
                if let Some(sp) = (Piece { outline: quad, rest: cs, turn: side_turn, at: m3, alpha: opacity, flat: Some(colour), mirror: 0 }).place(&cam) {
                    placed.push(sp.lit(&light, eye));
                }
            }
        }
        placed.push(p);
    }
    let mut out = Image::new(b.img.width, b.img.height);
    if view == 0 {
        let fallback = b.img.clone();
        let (fi, bi) = (front.img.as_ref().unwrap_or(&fallback), back.img.as_ref().or(front.img.as_ref()).unwrap_or(&fallback));
        scene3d::draw(&mut out, &mut placed, fi, Some(bi));
    } else {
        if view >= 3 {
            for f in forces(ctx, t, lw) {
                let c = b.pixel(veymelo_geom::vec2(f.at[0], f.at[1]));
                let rad = f.radius * s;
                for k in 0..64 {
                    let (a0, a1) = (k as f64 / 64.0 * TAU, (k + 1) as f64 / 64.0 * TAU);
                    let (p0, p1) = ([c.x + rad * a0.cos(), c.y + rad * a0.sin()], [c.x + rad * a1.cos(), c.y + rad * a1.sin()]);
                    lines.push(Sprite::new(p1[0] as f32, p1[1] as f32, 0.5, [1.0, 0.3, 0.3, 1.0], Shape::Line { dx: (p1[0] - p0[0]) as f32, dy: (p1[1] - p0[1]) as f32 }));
                }
            }
        }
        draw(&mut out, &lines, false);
    }
    b.img = out;
    b
}

#[cfg(test)]
mod tests;
