//! Distortions, part two: pushed about by noise or by another layer (Turbulent Displace,
//! Displacement Map), lenses (Optics Compensation, Magnify, Lens), the layer bent on a grid or
//! between curves (Mesh Warp, Bezier Warp), bent along a line (Bend), a page turning (Page Turn),
//! and tiles (Tiler, Griddler).
//!
//! Sizes and places are in layer units, so a preview bends the same as the full picture.

use std::f64::consts::{FRAC_PI_2, PI};

use veymelo_geom::{Rect, Vec2, vec2};
use veymelo_raster::{Px, Sampling};

use crate::color::rgb3;
use crate::defs::{angle, choice, color, effect, flag, layer, num, pct, point, px, ranged};
use crate::distort::{rect_px, remap};
use crate::generate::MODE_OF;
use crate::puppet::{Piece, draw};
use crate::util::{hsl, luma, map_px, premul, unpremul};
use crate::{Buf, Ctx, Fit, Spec, Take};

const CATEGORY: &str = "distort";

fn at(b: &Buf, p: Vec2) -> (f64, f64) {
    let q = b.pixel(p);
    (q.x, q.y)
}

const KINDS: &[&str] = &[
    "turbulent",
    "bulge",
    "twist",
    "turbulentSmoother",
    "bulgeSmoother",
    "twistSmoother",
    "verticalDisplacement",
    "horizontalDisplacement",
    "crossDisplacement",
];
const PINS: &[&str] = &[
    "none",
    "pinAll",
    "pinHorizontal",
    "pinVertical",
    "pinLeft",
    "pinTop",
    "pinRight",
    "pinBottom",
    "pinAllLocked",
    "pinHorizontalLocked",
    "pinVerticalLocked",
    "pinLeftLocked",
    "pinTopLocked",
    "pinRightLocked",
    "pinBottomLocked",
];

effect!(
    TURBULENT_DEF,
    "turbulentDisplace",
    "Turbulent Displace",
    "the picture pushed about by smooth noise: wavering, bulging or swirling; turn the evolution to animate it",
    [
        choice("displacement", "Displacement", KINDS, 0),
        ranged("amount", "Amount", 50.0, -1000.0, 1000.0),
        ranged("size", "Size", 100.0, 2.0, 4000.0).doc("how big the swirls are"),
        point("offset", "Offset (Turbulence)"),
        ranged("complexity", "Complexity", 1.0, 1.0, 10.0).doc("finer and finer detail on top"),
        angle("evolution", "Evolution", 0.0).doc("each turn is a new pattern, smoothly"),
        flag("cycleEvolution", "Cycle Evolution", false),
        num("cycle", "Cycle (in Revolutions)", 1.0).range(1.0, 1000.0).doc("the pattern comes back after this many turns"),
        num("randomSeed", "Random Seed", 0.0).range(0.0, 1_000_000.0),
        choice("pinning", "Pinning", PINS, 1).doc("which edges stay put; locked ones also take nothing from past the edges"),
        flag("resizeLayer", "Resize Layer", false).doc("room for the picture to be pushed past the layer's edges"),
    ]
);
pub const TURBULENT: Spec = Spec::new(&TURBULENT_DEF, CATEGORY, turbulent).points(&[("offset", [0.5, 0.5])]);

fn turbulent(ctx: &Ctx, mut b: Buf) -> Buf {
    let amount = ctx.num("amount", 50.0) * b.scale;
    if amount.abs() < 1e-9 {
        return b;
    }
    let kind = ctx.choice("displacement");
    let size = (ctx.num("size", 100.0) * b.scale).max(1.0);
    if ctx.flag("resizeLayer") && !ctx.adjustment {
        b.pad(amount.abs().ceil() as usize + 1);
    }
    let off = at(&b, ctx.point("offset"));
    let octaves = ctx.num("complexity", 1.0).clamp(1.0, 10.0);
    let z = ctx.num("evolution", 0.0) / 360.0;
    let period = ctx.flag("cycleEvolution").then(|| ctx.num("cycle", 1.0).round().max(1.0) as i64);
    let pinning = ctx.choice("pinning");
    let locked = pinning >= 8;
    let pin = if locked { pinning - 7 } else { pinning };
    let falloff = if (3..=5).contains(&kind) { 0.3 } else { 0.5 };
    let (lx, ly, lw, lh) = rect_px(ctx, &b);
    // how far in from a held edge the push is back to full
    let edge = (lw.min(lh) * if locked { 0.25 } else { 0.1 }).max(1.0);
    let seed = crate::noise::mix(&[0x7d15, ctx.num("randomSeed", 0.0).max(0.0) as u64]);
    let noise = |u: f64, v: f64, k: u64| crate::noise::fractal(u, v, z, seed ^ k, octaves, falloff, period);
    b.img = remap(&b.img, false, |x, y| {
        let (u, v) = ((x - off.0) / size, (y - off.1) / size);
        let (dx, dy) = match kind {
            // along the slope of the noise (out of its hollows) or round it
            1 | 2 | 4 | 5 => {
                let e = 0.05;
                let gx = (noise(u + e, v, 0) - noise(u - e, v, 0)) / (2.0 * e);
                let gy = (noise(u, v + e, 0) - noise(u, v - e, 0)) / (2.0 * e);
                let k = amount * 0.125;
                if kind == 1 || kind == 4 { (gx * k, gy * k) } else { (-gy * k, gx * k) }
            }
            6 => (0.0, noise(u, v, 17) * amount * 0.5),
            7 => (noise(u, v, 0) * amount * 0.5, 0.0),
            8 => {
                let a = noise(u, v, 0) * amount * 0.5;
                (a, a)
            }
            _ => (noise(u, v, 0) * amount * 0.5, noise(u, v, 17) * amount * 0.5),
        };
        // measured from the outermost pixels' middles, so held edges stay exactly
        let (l, r) = (((x - lx - 0.5) / edge).clamp(0.0, 1.0), ((lx + lw - 0.5 - x) / edge).clamp(0.0, 1.0));
        let (t, bt) = (((y - ly - 0.5) / edge).clamp(0.0, 1.0), ((ly + lh - 0.5 - y) / edge).clamp(0.0, 1.0));
        let free = match pin {
            1 => l.min(r).min(t).min(bt),
            2 => t.min(bt),
            3 => l.min(r),
            4 => l,
            5 => t,
            6 => r,
            7 => bt,
            _ => 1.0,
        };
        let (sx, sy) = (x + dx * free, y + dy * free);
        if locked { (sx.clamp(lx + 0.5, lx + lw - 0.5), sy.clamp(ly + 0.5, ly + lh - 0.5)) } else { (sx, sy) }
    });
    b
}

/// What a map's pixel says, from 0 to 1: a channel of it, all the way, half way, or nothing.
const SOURCES: &[&str] = &["red", "green", "blue", "alpha", "luminance", "hue", "lightness", "saturation", "full", "half", "off"];

/// A map pixel's value by `source` (None: off).
fn pick(source: u32, p: Px) -> Option<f64> {
    let (c, a) = unpremul(p);
    let (h, s, l) = hsl(c);
    Some(match source {
        0 => c[0],
        1 => c[1],
        2 => c[2],
        3 => a,
        4 => luma(c),
        5 => h,
        6 => l,
        7 => s,
        8 => 1.0,
        9 => 0.5,
        _ => return None,
    } as f64)
}

effect!(
    DISPLACEMENT_DEF,
    "displacementMap",
    "Displacement Map",
    "the picture pushed across and down by how bright another layer (or this one) is",
    [
        layer("displacementMapLayer", "Displacement Map Layer").doc("none: the layer itself"),
        choice("useForHorizontalDisplacement", "Use For Horizontal Displacement", SOURCES, 0),
        ranged("maxHorizontalDisplacement", "Max Horizontal Displacement", 5.0, -32_000.0, 32_000.0),
        choice("useForVerticalDisplacement", "Use For Vertical Displacement", SOURCES, 1),
        ranged("maxVerticalDisplacement", "Max Vertical Displacement", 5.0, -32_000.0, 32_000.0),
        choice("displacementMapBehavior", "Displacement Map Behavior", &["centerMap", "stretchMapToFit", "tileMap"], 0),
        flag("wrapPixelsAround", "Wrap Pixels Around", false),
        flag("expandOutput", "Expand Output", true),
    ]
);
pub const DISPLACEMENT: Spec = Spec::new(&DISPLACEMENT_DEF, CATEGORY, displacement);

fn displacement(ctx: &Ctx, mut b: Buf) -> Buf {
    let (hs, vs) = (ctx.choice("useForHorizontalDisplacement"), ctx.choice("useForVerticalDisplacement"));
    let (mh, mv) = (ctx.num("maxHorizontalDisplacement", 5.0) * b.scale, ctx.num("maxVerticalDisplacement", 5.0) * b.scale);
    let wrap = ctx.flag("wrapPixelsAround");
    if ctx.flag("expandOutput") && !wrap && !ctx.adjustment {
        // room for the pushes that can happen (none from half or off)
        let reach = |s: u32, most: f64| if s >= 9 { 0.0 } else { most.abs() };
        b.pad(reach(hs, mh).max(reach(vs, mv)).ceil().min(4096.0) as usize);
    }
    let fit = [Fit::Centre, Fit::Stretch, Fit::Tile][ctx.choice("displacementMapBehavior").min(2) as usize];
    // the map as the layer is before its masks and effects (or this layer as it is now)
    let (map, lands) = match ctx.layer_placed("displacementMapLayer", &b, Take::Source, fit) {
        Some((img, r)) => (img, (fit == Fit::Centre).then_some(r)),
        None => (b.img.clone(), None),
    };
    let src = b.img.clone();
    let (lx, ly, lw, lh) = rect_px(ctx, &b);
    map_px(&mut b.img, |x, y, p| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        // past a centred map that is smaller than the layer nothing moves
        if lands.is_some_and(|r| px < r.x0 || py < r.y0 || px >= r.x1 || py >= r.y1) {
            return p;
        }
        let m = map.px[y * map.width + x];
        let push = |s: u32, most: f64| pick(s, m).map_or(0.0, |v| (v - 0.5) * 2.0 * most);
        let (dx, dy) = (push(hs, mh), push(vs, mv));
        if dx == 0.0 && dy == 0.0 {
            return p;
        }
        let (sx, sy) = (px + dx, py + dy);
        if wrap {
            src.sample(Sampling::Bilinear, lx + (sx - lx).rem_euclid(lw), ly + (sy - ly).rem_euclid(lh))
        } else {
            src.sample(Sampling::Bilinear, sx, sy)
        }
    });
    b
}

effect!(
    OPTICS_DEF,
    "opticsCompensation",
    "Optics Compensation",
    "a wide lens's barrel bulge put in, or (reversed) taken out",
    [
        ranged("fieldOfView", "Field Of View (FOV)", 0.0, 0.0, 180.0),
        flag("reverseLensDistortion", "Reverse Lens Distortion", false),
        choice("fovOrientation", "FOV Orientation", &["horizontal", "vertical", "diagonal"], 0),
        point("viewCenter", "View Center"),
        flag("optimalPixels", "Optimal Pixels (Invalidates Resize)", false).doc("scaled so the whole picture fits, or fills with no empty corners"),
        choice("resize", "Resize", &["off", "max2X", "max4X", "unlimited"], 0).doc("with Reverse: room for the stretched corners"),
    ]
);
pub const OPTICS: Spec = Spec::new(&OPTICS_DEF, CATEGORY, optics).points(&[("viewCenter", [0.5, 0.5])]);

fn optics(ctx: &Ctx, mut b: Buf) -> Buf {
    let fov = ctx.num("fieldOfView", 0.0).clamp(0.0, 179.0).to_radians();
    if fov < 1e-6 {
        return b;
    }
    let reverse = ctx.flag("reverseLensDistortion");
    let (lx, ly, lw, lh) = rect_px(ctx, &b);
    // the field of view spans this far from the middle
    let span = match ctx.choice("fovOrientation") {
        1 => lh * 0.5,
        2 => lw.hypot(lh) * 0.5,
        _ => lw * 0.5,
    }
    .max(1.0);
    let half = fov * 0.5;
    let focal = span / half.tan();
    let optimal = ctx.flag("optimalPixels");
    let corner_reach = |c: (f64, f64)| [(lx, ly), (lx + lw, ly), (lx, ly + lh), (lx + lw, ly + lh)].iter().map(|p| (p.0 - c.0).hypot(p.1 - c.1)).fold(1.0, f64::max);
    let resize = if optimal { 0 } else { ctx.choice("resize") };
    if reverse && resize > 0 && !ctx.adjustment {
        let rc = corner_reach(at(&b, ctx.point("viewCenter")));
        let th = rc / span * half;
        let most = lw.max(lh) * [0.0, 0.5, 1.5, 3.5][resize.min(3) as usize];
        let grow = if th < FRAC_PI_2 - 1e-3 { (focal * th.tan() - rc).max(0.0) } else { most };
        b.pad(grow.min(most).min(4096.0).ceil() as usize);
    }
    let c = at(&b, ctx.point("viewCenter"));
    let fit = if optimal {
        let rc = corner_reach(c);
        if reverse { (focal * (rc / span * half).min(FRAC_PI_2 - 1e-3).tan() / rc).max(1.0) } else { ((rc / focal).atan() / half * span / rc).min(1.0) }
    } else {
        1.0
    };
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let r = dx.hypot(dy) * fit;
        if r < 1e-9 {
            return (x, y);
        }
        let from = if reverse {
            (r / focal).atan() / half * span
        } else {
            let th = r / span * half;
            if th >= FRAC_PI_2 - 1e-4 {
                // past the edge of what the lens sees
                return (-1e9, -1e9);
            }
            focal * th.tan()
        };
        let k = from / r * fit;
        (c.0 + dx * k, c.1 + dy * k)
    });
    b
}

const MAGNIFY_MODES: [&str; 18] = [
    "none", "normal", "add", "multiply", "screen", "overlay", "softLight", "hardLight", "colorDodge", "colorBurn", "darken", "lighten", "difference", "exclusion", "hue", "saturation", "color", "luminosity",
];

effect!(
    MAGNIFY_DEF,
    "magnify",
    "Magnify",
    "a magnifying glass over part of the layer",
    [
        choice("shape", "Shape", &["circle", "square"], 0),
        point("center", "Center"),
        ranged("magnification", "Magnification", 200.0, 1.0, 10_000.0),
        choice("link", "Link", &["none", "sizeToMagnification", "sizeAndFeatherToMagnification"], 0),
        px("size", "Size", 100.0, 4000.0),
        px("feather", "Feather", 0.0, 4000.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        choice("scaling", "Scaling", &["standard", "soft", "scatter"], 0).doc("standard: sharp pixels; soft: smooth; scatter: grainy"),
        choice("blendingMode", "Blending Mode", &MAGNIFY_MODES, 1).doc("none: only the glass"),
        flag("resizeLayer", "Resize Layer", false).doc("the glass may reach past the layer's edges"),
    ]
);
pub const MAGNIFY: Spec = Spec::new(&MAGNIFY_DEF, CATEGORY, magnify).points(&[("center", [0.5, 0.5])]);

fn magnify(ctx: &Ctx, mut b: Buf) -> Buf {
    let square = ctx.choice("shape") == 1;
    let k = (ctx.num("magnification", 200.0) / 100.0).max(0.01);
    let link = ctx.choice("link");
    let mut size = ctx.num("size", 100.0) * b.scale;
    let mut feather = ctx.num("feather", 0.0) * b.scale;
    if link >= 1 {
        size *= k;
    }
    if link == 2 {
        feather *= k;
    }
    let opacity = (ctx.num("opacity", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let scaling = ctx.choice("scaling");
    let mode = ctx.choice("blendingMode").min(17);
    if ctx.flag("resizeLayer") && !ctx.adjustment {
        let c = at(&b, ctx.point("center"));
        let (lx, ly, lw, lh) = rect_px(ctx, &b);
        let over = [lx - (c.0 - size), (c.0 + size) - (lx + lw), ly - (c.1 - size), (c.1 + size) - (ly + lh)].into_iter().fold(0.0, f64::max);
        b.pad(over.min(4096.0).ceil() as usize);
    }
    let c = at(&b, ctx.point("center"));
    let src = b.img.clone();
    let seed = ctx.seed;
    map_px(&mut b.img, |x, y, p| {
        let (dx, dy) = (x as f64 + 0.5 - c.0, y as f64 + 0.5 - c.1);
        let d = if square { dx.abs().max(dy.abs()) } else { dx.hypot(dy) };
        let inside = if feather > 0.01 {
            let t = ((size - d) / feather).clamp(0.0, 1.0);
            (t * t * (3.0 - 2.0 * t)) as f32
        } else {
            (size - d + 0.5).clamp(0.0, 1.0) as f32
        } * opacity;
        if inside <= 0.0 {
            return if mode == 0 { [0.0; 4] } else { p };
        }
        let (sx, sy) = (c.0 + dx / k, c.1 + dy / k);
        let seen = match scaling {
            1 => src.sample(Sampling::Bicubic, sx, sy),
            2 => {
                // somewhere in the enlarged pixel, at random
                let j = |n: u64| (crate::noise::unit(crate::noise::mix(&[seed, x as u64, y as u64, n])) - 0.5) * (1.0 - 1.0 / k).max(0.0);
                src.sample(Sampling::Bilinear, sx.floor() + 0.5 + j(1), sy.floor() + 0.5 + j(2))
            }
            _ => src.sample(Sampling::Nearest, sx, sy),
        };
        match mode {
            0 => seen.map(|v| v * inside),
            1 => [0, 1, 2, 3].map(|i| p[i] + (seen[i] - p[i]) * inside),
            m => veymelo_color::blend(MODE_OF[m as usize], p, seen.map(|v| v * inside), 0.5),
        }
    });
    b
}

effect!(
    MESH_WARP_DEF,
    "meshWarp",
    "Mesh Warp",
    "a grid over the layer whose points can be moved, the picture bending smoothly with them",
    [
        ranged("rows", "Rows", 7.0, 1.0, 31.0),
        ranged("columns", "Columns", 7.0, 1.0, 31.0),
        ranged("quality", "Quality", 8.0, 1.0, 10.0),
        veymelo_project::PropDef::new("distortionMesh", "Distortion Mesh", veymelo_project::Kind::Text, veymelo_keyframe::Value::Text(String::new()))
            .doc("how far each grid point moved (layer units), row by row from the top left: \"dx,dy dx,dy …\", (rows + 1) × (columns + 1) of them; empty: none moved"),
    ]
);
pub const MESH_WARP: Spec = Spec::new(&MESH_WARP_DEF, CATEGORY, mesh_warp);

/// The pairs of numbers in a mesh's text.
pub fn mesh_moves(text: &str) -> Vec<[f64; 2]> {
    text.split(|c: char| c.is_whitespace() || c == ';')
        .filter(|t| !t.is_empty())
        .filter_map(|t| {
            let (a, b) = t.split_once(',')?;
            Some([a.trim().parse().ok()?, b.trim().parse().ok()?])
        })
        .collect()
}

/// A Catmull-Rom curve's weights at `t` for four points in a row (the curve runs from the
/// second to the third).
fn spline(t: f64) -> [f64; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2]
}

/// The picture drawn through a grid of `nx` × `ny` cells: point `i` (row by row) of `from`
/// (layer units) goes to point `i` of `to`. The buffer grows to hold where it goes.
fn through_grid(ctx: &Ctx, b: &mut Buf, nx: usize, ny: usize, from: &[Vec2], to: &[Vec2]) {
    let before = b.clone();
    if !ctx.adjustment {
        let mut r = Rect::new(to[0].x, to[0].y, to[0].x, to[0].y);
        for p in to {
            r.include(*p);
        }
        b.cover(r.intersect(&before.area().inflate(8192.0 / b.scale)));
    }
    let (s, o) = (b.scale, b.origin);
    let px = |p: Vec2| (p - o) * s;
    let idx = |i: usize, j: usize| j * (nx + 1) + i;
    let mut pieces = Vec::with_capacity(nx * ny * 2);
    for j in 0..ny {
        for i in 0..nx {
            let q = [idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1)];
            for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                pieces.push(Piece::new(t.map(|k| before.pixel(from[k])), t.map(|k| px(to[k]))));
            }
        }
    }
    b.img = draw(&before.img, &pieces, b.img.width, b.img.height);
}

fn mesh_warp(ctx: &Ctx, mut b: Buf) -> Buf {
    let rows = ctx.num("rows", 7.0).round().clamp(1.0, 31.0) as usize;
    let cols = ctx.num("columns", 7.0).round().clamp(1.0, 31.0) as usize;
    let q = ctx.num("quality", 8.0).round().clamp(1.0, 10.0) as usize;
    let text = ctx.value_in(ctx.effect, "distortionMesh").map(|v| v.as_text().to_string()).unwrap_or_default();
    let moves = mesh_moves(&text);
    if moves.is_empty() || moves.iter().all(|m| m[0] == 0.0 && m[1] == 0.0) {
        return b;
    }
    if moves.len() != (rows + 1) * (cols + 1) {
        ctx.note(format!("the distortion mesh has {} points; {} rows and {} columns need {}", moves.len(), rows, cols, (rows + 1) * (cols + 1)));
        return b;
    }
    let r = ctx.bounds;
    // a grid point's move, the grid's edges going on as they are
    let moved = |i: i64, j: i64| moves[(j.clamp(0, rows as i64) as usize) * (cols + 1) + i.clamp(0, cols as i64) as usize];
    let (nx, ny) = (cols * q, rows * q);
    let (mut from, mut to) = (Vec::with_capacity((nx + 1) * (ny + 1)), Vec::with_capacity((nx + 1) * (ny + 1)));
    for gj in 0..=ny {
        for gi in 0..=nx {
            let (ci, cj) = ((gi / q).min(cols - 1), (gj / q).min(rows - 1));
            let (wu, wv) = (spline((gi - ci * q) as f64 / q as f64), spline((gj - cj * q) as f64 / q as f64));
            // smoothly between the moves of the sixteen grid points round it
            let mut d = [0.0; 2];
            for (b2, vy) in wv.iter().enumerate() {
                for (a2, ux) in wu.iter().enumerate() {
                    let m = moved(ci as i64 - 1 + a2 as i64, cj as i64 - 1 + b2 as i64);
                    d[0] += m[0] * ux * vy;
                    d[1] += m[1] * ux * vy;
                }
            }
            let p = vec2(r.x0 + gi as f64 / nx as f64 * r.width(), r.y0 + gj as f64 / ny as f64 * r.height());
            from.push(p);
            to.push(p + vec2(d[0], d[1]));
        }
    }
    through_grid(ctx, &mut b, nx, ny, &from, &to);
    b
}

effect!(
    BEZIER_WARP_DEF,
    "bezierWarp",
    "Bezier Warp",
    "the layer's four sides as curves, each with two handles, the picture bent to fit inside them",
    [
        point("topLeftVertex", "Top Left Vertex"),
        point("topLeftTangent", "Top Left Tangent"),
        point("topRightTangent", "Top Right Tangent"),
        point("rightTopVertex", "Right Top Vertex"),
        point("rightTopTangent", "Right Top Tangent"),
        point("rightBottomTangent", "Right Bottom Tangent"),
        point("bottomRightVertex", "Bottom Right Vertex"),
        point("bottomRightTangent", "Bottom Right Tangent"),
        point("bottomLeftTangent", "Bottom Left Tangent"),
        point("leftBottomVertex", "Left Bottom Vertex"),
        point("leftBottomTangent", "Left Bottom Tangent"),
        point("leftTopTangent", "Left Top Tangent"),
        ranged("quality", "Quality", 8.0, 1.0, 10.0),
    ]
);
pub const BEZIER_WARP: Spec = Spec::new(&BEZIER_WARP_DEF, CATEGORY, bezier_warp).points(&[
    ("topLeftVertex", [0.0, 0.0]),
    ("topLeftTangent", [1.0 / 3.0, 0.0]),
    ("topRightTangent", [2.0 / 3.0, 0.0]),
    ("rightTopVertex", [1.0, 0.0]),
    ("rightTopTangent", [1.0, 1.0 / 3.0]),
    ("rightBottomTangent", [1.0, 2.0 / 3.0]),
    ("bottomRightVertex", [1.0, 1.0]),
    ("bottomRightTangent", [2.0 / 3.0, 1.0]),
    ("bottomLeftTangent", [1.0 / 3.0, 1.0]),
    ("leftBottomVertex", [0.0, 1.0]),
    ("leftBottomTangent", [0.0, 2.0 / 3.0]),
    ("leftTopTangent", [0.0, 1.0 / 3.0]),
]);

fn cubic(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f64) -> Vec2 {
    let u = 1.0 - t;
    p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t)
}

fn bezier_warp(ctx: &Ctx, mut b: Buf) -> Buf {
    let p = |id: &str| ctx.point(id);
    let (tl, tr, br, bl) = (p("topLeftVertex"), p("rightTopVertex"), p("bottomRightVertex"), p("leftBottomVertex"));
    let top = |t: f64| cubic(tl, p("topLeftTangent"), p("topRightTangent"), tr, t);
    let bottom = |t: f64| cubic(bl, p("bottomLeftTangent"), p("bottomRightTangent"), br, t);
    let left = |t: f64| cubic(tl, p("leftTopTangent"), p("leftBottomTangent"), bl, t);
    let right = |t: f64| cubic(tr, p("rightTopTangent"), p("rightBottomTangent"), br, t);
    let n = ctx.num("quality", 8.0).round().clamp(1.0, 10.0) as usize * 4;
    let r = ctx.bounds;
    let (mut from, mut to) = (Vec::with_capacity((n + 1) * (n + 1)), Vec::with_capacity((n + 1) * (n + 1)));
    for j in 0..=n {
        let v = j as f64 / n as f64;
        let (l, rr) = (left(v), right(v));
        for i in 0..=n {
            let u = i as f64 / n as f64;
            // a patch that has the four curves for sides: each pair of opposite sides blended,
            // less the corners blended (counted twice)
            let corners = tl * ((1.0 - u) * (1.0 - v)) + tr * (u * (1.0 - v)) + bl * ((1.0 - u) * v) + br * (u * v);
            to.push(top(u) * (1.0 - v) + bottom(u) * v + l * (1.0 - u) + rr * u - corners);
            from.push(vec2(r.x0 + u * r.width(), r.y0 + v * r.height()));
        }
    }
    through_grid(ctx, &mut b, n, n, &from, &to);
    b
}

effect!(
    BEND_DEF,
    "bend",
    "Bend",
    "the layer bent into an arc between two points, like a strip of paper",
    [
        angle("bend", "Bend", 0.0),
        point("start", "Start"),
        point("end", "End"),
        choice("renderPrestart", "Render Prestart", &["none", "static"], 1).doc("what is before the start: nothing, or as it was"),
    ]
);
pub const BEND: Spec = Spec::new(&BEND_DEF, CATEGORY, bend).points(&[("start", [0.5, 0.2]), ("end", [0.5, 0.8])]);

fn bend(ctx: &Ctx, mut b: Buf) -> Buf {
    let mut turn = ctx.num("bend", 0.0).to_radians();
    if turn.abs() < 1e-6 {
        return b;
    }
    let (s, e) = (at(&b, ctx.point("start")), at(&b, ctx.point("end")));
    let len = (e.0 - s.0).hypot(e.1 - s.1);
    if len < 1e-6 {
        return b;
    }
    let keep_before = ctx.choice("renderPrestart") == 1;
    // along the strip and across it, toward the side it bends to
    let along = ((e.0 - s.0) / len, (e.1 - s.1) / len);
    let mut across = (-along.1, along.0);
    if turn < 0.0 {
        turn = -turn;
        across = (-across.0, -across.1);
    }
    let radius = len / turn;
    // where the end goes, and which way the strip runs on from there
    let end = (radius * turn.sin(), radius * (1.0 - turn.cos()));
    let (on, side) = ((turn.cos(), turn.sin()), (-turn.sin(), turn.cos()));
    let src = b.img.clone();
    let from = |u: f64, w: f64| (s.0 + along.0 * u + across.0 * w, s.1 + along.1 * u + across.1 * w);
    map_px(&mut b.img, |x, y, _| {
        let (vx, vy) = (x as f64 + 0.5 - s.0, y as f64 + 0.5 - s.1);
        let (lu, lw) = (vx * along.0 + vy * along.1, vx * across.0 + vy * across.1);
        // what could show here: the arc, the straight part after it, what is before the start
        let mut ways: [Option<(f64, f64)>; 3] = [None; 3];
        let (dx, dy) = (lu, lw - radius);
        let a = dx.atan2(-dy);
        if (0.0..=turn).contains(&a) {
            ways[0] = Some((radius * a, radius - dx.hypot(dy)));
        }
        let (ex, ey) = (lu - end.0, lw - end.1);
        let past = ex * on.0 + ey * on.1;
        if past >= 0.0 {
            ways[1] = Some((len + past, ex * side.0 + ey * side.1));
        }
        if lu < 0.0 && keep_before {
            ways[2] = Some((lu, lw));
        }
        for (u, w) in ways.into_iter().flatten() {
            let (sx, sy) = from(u, w);
            let c = src.sample(Sampling::Bilinear, sx, sy);
            if c[3] > 1e-4 {
                return c;
            }
        }
        [0.0; 4]
    });
    b
}

effect!(
    LENS_DEF,
    "lens",
    "Lens",
    "a round lens: the picture seen through a ball, nothing round it",
    [point("center", "Center"), pct("size", "Size", 50.0, 0.0, 400.0).doc("100: from the middle to the layer's edge"), ranged("convergence", "Convergence", 0.0, -100.0, 100.0),]
);
pub const LENS: Spec = Spec::new(&LENS_DEF, CATEGORY, lens).points(&[("center", [0.5, 0.5])]);

fn lens(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = at(&b, ctx.point("center"));
    let (_, _, lw, lh) = rect_px(ctx, &b);
    let r = (ctx.num("size", 50.0) / 100.0 * lw.max(lh) * 0.5).max(0.5);
    let k = 2f64.powf(ctx.num("convergence", 0.0) / 50.0);
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (dx, dy) = (x as f64 + 0.5 - c.0, y as f64 + 0.5 - c.1);
        let d = dx.hypot(dy);
        let edge = (r - d + 0.5).clamp(0.0, 1.0) as f32;
        if edge <= 0.0 {
            return [0.0; 4];
        }
        let g = if d < 1e-9 { 0.0 } else { ((d / r).min(1.0).asin() / FRAC_PI_2).powf(k) * r / d };
        src.sample(Sampling::Bilinear, c.0 + dx * g, c.1 + dy * g).map(|v| v * edge)
    });
    b
}

effect!(
    PAGE_TURN_DEF,
    "pageTurn",
    "Page Turn",
    "the layer's corner curling over like a page being turned, its back showing",
    [
        point("foldPosition", "Fold Position"),
        angle("foldDirection", "Fold Direction", 315.0),
        px("foldRadius", "Fold Radius", 20.0, 1000.0),
        angle("lightDirection", "Light Direction", -45.0),
        choice("render", "Render", &["frontAndBackPage", "backPage", "frontPage"], 0),
        pct("backOpacity", "Back Opacity", 75.0, 0.0, 100.0).doc("how much of the picture shows through the back"),
        color("paperColor", "Paper Color", 1.0, 1.0, 1.0),
    ]
);
pub const PAGE_TURN: Spec = Spec::new(&PAGE_TURN_DEF, CATEGORY, page_turn).points(&[("foldPosition", [0.7, 0.7])]);

fn page_turn(ctx: &Ctx, mut b: Buf) -> Buf {
    let f = at(&b, ctx.point("foldPosition"));
    let dir = ctx.num("foldDirection", 315.0).to_radians();
    let r = (ctx.num("foldRadius", 20.0) * b.scale).max(0.5);
    let light = ctx.num("lightDirection", -45.0).to_radians();
    let render = ctx.choice("render");
    let see = (ctx.num("backOpacity", 75.0) / 100.0).clamp(0.0, 1.0) as f32;
    let paper = rgb3(ctx.color("paperColor"));
    // the curl lies on this side of the fold line
    let n = (-dir.sin(), dir.cos());
    let lit = (n.0 * light.sin() - n.1 * light.cos()) as f32;
    let (front, back) = (render != 1, render != 2);
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let d = (px - f.0) * n.0 + (py - f.1) * n.1;
        if d > r {
            return [0.0; 4];
        }
        // what could show here, top first: how far along the page it is (unrolled), whether
        // it is the back, and how lit
        let mut ways: [Option<(f64, bool, f32)>; 4] = [None; 4];
        if d <= 0.0 {
            ways[0] = Some((PI * r - d, true, 0.9));
            ways[3] = Some((d, false, 1.0));
        } else {
            let a = (d / r).clamp(-1.0, 1.0).asin();
            ways[1] = Some((PI * r - r * a, true, 0.6 + 0.4 * a.cos() as f32));
            ways[2] = Some((r * a, false, 1.0 - 0.4 * (a / FRAC_PI_2) as f32 * (0.5 + 0.5 * lit)));
        }
        for (u, is_back, shade) in ways.into_iter().flatten() {
            if (is_back && !back) || (!is_back && !front) {
                continue;
            }
            let k = u - d;
            let c = src.sample(Sampling::Bilinear, px + k * n.0, py + k * n.1);
            if c[3] <= 1e-4 {
                continue;
            }
            let (col, a) = unpremul(c);
            let col = if is_back { [0, 1, 2].map(|i| paper[i] + (col[i] - paper[i]) * see) } else { col };
            return premul(col.map(|v| v * shade), a);
        }
        [0.0; 4]
    });
    b
}

effect!(
    TILER_DEF,
    "tiler",
    "Tiler",
    "the layer made smaller and repeated in tiles to fill it",
    [ranged("scale", "Scale", 50.0, 1.0, 10_000.0), point("center", "Center"), pct("blendWithOriginal", "Blend w. Original", 0.0, 0.0, 100.0),]
);
pub const TILER: Spec = Spec::new(&TILER_DEF, CATEGORY, tiler).points(&[("center", [0.5, 0.5])]);

fn tiler(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = (ctx.num("scale", 50.0) / 100.0).max(0.001);
    let c = at(&b, ctx.point("center"));
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    let (x0, y0, w, h) = rect_px(ctx, &b);
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, p| {
        let (qx, qy) = (c.0 + (x as f64 + 0.5 - c.0) / k, c.1 + (y as f64 + 0.5 - c.1) / k);
        let (sx, sy) = (x0 + (qx - x0).rem_euclid(w), y0 + (qy - y0).rem_euclid(h));
        let t = src.sample(Sampling::Bilinear, sx.clamp(x0 + 0.5, x0 + w - 0.5), sy.clamp(y0 + 0.5, y0 + h - 0.5));
        [0, 1, 2, 3].map(|i| t[i] + (p[i] - t[i]) * keep)
    });
    b
}

effect!(
    GRIDDLER_DEF,
    "griddler",
    "Griddler",
    "the layer cut into square tiles, each scaled and turned about its own middle",
    [
        ranged("horizontalScale", "Horizontal Scale", 100.0, 1.0, 1000.0),
        ranged("verticalScale", "Vertical Scale", 100.0, 1.0, 1000.0),
        ranged("tileSize", "Tile Size", 10.0, 0.5, 100.0).doc("percent of the layer's width"),
        angle("rotation", "Rotation", 0.0),
        flag("cutTiles", "Cut Tiles", false).doc("gaps between the tiles"),
    ]
);
pub const GRIDDLER: Spec = Spec::new(&GRIDDLER_DEF, CATEGORY, griddler);

fn griddler(ctx: &Ctx, mut b: Buf) -> Buf {
    let (hs, vs) = ((ctx.num("horizontalScale", 100.0) / 100.0).max(0.001), (ctx.num("verticalScale", 100.0) / 100.0).max(0.001));
    let turn = ctx.num("rotation", 0.0).to_radians();
    let cut = ctx.flag("cutTiles");
    let (x0, y0, lw, _) = rect_px(ctx, &b);
    let t = (ctx.num("tileSize", 10.0) / 100.0 * lw).max(1.0);
    if (hs - 1.0).abs() < 1e-9 && (vs - 1.0).abs() < 1e-9 && turn.abs() < 1e-9 && !cut {
        return b;
    }
    let (s, c) = turn.sin_cos();
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let (cx, cy) = (x0 + (((px - x0) / t).floor() + 0.5) * t, y0 + (((py - y0) / t).floor() + 0.5) * t);
        let (lx, ly) = (px - cx, py - cy);
        // a gap of a tenth of a tile between them, its edges smooth
        let keep = if cut { (0.45 * t - lx.abs().max(ly.abs()) + 0.5).clamp(0.0, 1.0) as f32 } else { 1.0 };
        if keep <= 0.0 {
            return [0.0; 4];
        }
        let (rx, ry) = (lx * c + ly * s, -lx * s + ly * c);
        src.sample(Sampling::Bilinear, cx + rx / hs, cy + ry / vs).map(|v| v * keep)
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 11] = [TURBULENT, DISPLACEMENT, OPTICS, MAGNIFY, MESH_WARP, BEZIER_WARP, BEND, LENS, PAGE_TURN, TILER, GRIDDLER];

#[cfg(test)]
mod tests;
