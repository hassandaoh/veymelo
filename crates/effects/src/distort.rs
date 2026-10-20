//! Distortions, part one: each pixel taken from somewhere else of the layer (Twirl, Spherize,
//! Bulge, Wave Warp, Ripple, Mirror, Offset, Polar Coordinates), and the layer moved whole
//! (Transform, Corner Pin), which may reach past its rectangle.
//!
//! Sizes and places are in layer units, so a preview bends the same as the full picture.

use std::f64::consts::{FRAC_PI_2, TAU};

use veymelo_geom::{Mat3, Rect, Vec2, vec2};
use veymelo_raster::{Image, Sampling};
use veymelo_time::Tick;

use crate::defs::{angle, choice, effect, flag, pct, point, px, ranged};
use crate::util::map_px;
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "distort";

/// Each pixel taken from the point `from(x, y)` says (pixel middles at .5), between pixels
/// mixed; past the edges clear, or (`repeat`) the edge pixels going on.
pub(crate) fn remap(img: &Image, repeat: bool, from: impl Fn(f64, f64) -> (f64, f64) + Sync) -> Image {
    let (w, h) = (img.width as f64, img.height as f64);
    let mut out = Image::new(img.width, img.height);
    map_px(&mut out, |x, y, _| {
        let (sx, sy) = from(x as f64 + 0.5, y as f64 + 0.5);
        if repeat { img.sample(Sampling::Bilinear, sx.clamp(0.5, w - 0.5), sy.clamp(0.5, h - 0.5)) } else { img.sample(Sampling::Bilinear, sx, sy) }
    });
    out
}

/// The layer's rectangle in the buffer's pixels: left, top, width, height.
pub(crate) fn rect_px(ctx: &Ctx, b: &Buf) -> (f64, f64, f64, f64) {
    let (a, z) = (b.pixel(vec2(ctx.bounds.x0, ctx.bounds.y0)), b.pixel(vec2(ctx.bounds.x1, ctx.bounds.y1)));
    (a.x, a.y, (z.x - a.x).max(1.0), (z.y - a.y).max(1.0))
}

fn at(b: &Buf, p: Vec2) -> (f64, f64) {
    let q = b.pixel(p);
    (q.x, q.y)
}

effect!(
    TWIRL_DEF,
    "twirl",
    "Twirl",
    "turns the picture round a point, most at the middle and less toward the edge of the twirl",
    [angle("angle", "Angle", 0.0), pct("twirlRadius", "Twirl Radius", 30.0, 0.0, 100.0).doc("of the layer's longer side: 50 reaches its edges"), point("twirlCenter", "Twirl Center"),]
);
pub const TWIRL: Spec = Spec::new(&TWIRL_DEF, CATEGORY, twirl).points(&[("twirlCenter", [0.5, 0.5])]);

fn twirl(ctx: &Ctx, mut b: Buf) -> Buf {
    let turn = ctx.num("angle", 0.0).to_radians();
    let r = (ctx.num("twirlRadius", 30.0) / 100.0 * ctx.bounds.width().max(ctx.bounds.height()) * b.scale).max(1.0);
    if turn == 0.0 {
        return b;
    }
    let c = at(&b, ctx.point("twirlCenter"));
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let d = dx.hypot(dy);
        if d >= r {
            return (x, y);
        }
        let k = 1.0 - d / r;
        let (s, co) = (-turn * k * k).sin_cos();
        (c.0 + dx * co - dy * s, c.1 + dx * s + dy * co)
    });
    b
}

effect!(
    SPHERIZE_DEF,
    "spherize",
    "Spherize",
    "the picture wrapped round a ball",
    [px("radius", "Radius", 0.0, 2500.0), point("centerOfSphere", "Center of Sphere"),]
);
pub const SPHERIZE: Spec = Spec::new(&SPHERIZE_DEF, CATEGORY, spherize).points(&[("centerOfSphere", [0.5, 0.5])]);

fn spherize(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = ctx.num("radius", 0.0) * b.scale;
    if r < 1.0 {
        return b;
    }
    let c = at(&b, ctx.point("centerOfSphere"));
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let d = dx.hypot(dy) / r;
        if d >= 1.0 || d == 0.0 {
            return (x, y);
        }
        // a point of the ball seen from straight on came from as far round its surface
        let k = d.asin() / FRAC_PI_2 / d;
        (c.0 + dx * k, c.1 + dy * k)
    });
    b
}

effect!(
    BULGE_DEF,
    "bulge",
    "Bulge",
    "an oval of the picture swelling out (or sinking in, below 0)",
    [
        px("horizontalRadius", "Horizontal Radius", 50.0, 4000.0),
        px("verticalRadius", "Vertical Radius", 50.0, 4000.0),
        point("bulgeCenter", "Bulge Center"),
        ranged("bulgeHeight", "Bulge Height", 1.0, -4.0, 4.0),
        pct("taperRadius", "Taper Radius", 0.0, 0.0, 100.0).doc("higher: steeper in the middle, flatter at the sides"),
        flag("pinAllEdges", "Pin All Edges", false).doc("the layer's edges stay put"),
    ]
);
pub const BULGE: Spec = Spec::new(&BULGE_DEF, CATEGORY, bulge).points(&[("bulgeCenter", [0.5, 0.5])]);

fn bulge(ctx: &Ctx, mut b: Buf) -> Buf {
    let (rx, ry) = ((ctx.num("horizontalRadius", 50.0) * b.scale).max(1.0), (ctx.num("verticalRadius", 50.0) * b.scale).max(1.0));
    let c = at(&b, ctx.point("bulgeCenter"));
    let height = ctx.num("bulgeHeight", 1.0);
    let taper = 1.0 + ctx.num("taperRadius", 0.0).max(0.0) / 100.0 * 3.0;
    let pin = ctx.flag("pinAllEdges");
    let (lx, ly, lw, lh) = rect_px(ctx, &b);
    let edge = (lw.min(lh) * 0.1).max(1.0);
    if height == 0.0 {
        return b;
    }
    b.img = remap(&b.img, false, |x, y| {
        let (u, v) = ((x - c.0) / rx, (y - c.1) / ry);
        let d2 = u * u + v * v;
        if d2 >= 1.0 {
            return (x, y);
        }
        let mut lift = height * (1.0 - d2).powf(taper) * 0.5;
        if pin {
            let ex = ((x - lx - 0.5).min(lx + lw - 0.5 - x) / edge).clamp(0.0, 1.0);
            let ey = ((y - ly - 0.5).min(ly + lh - 0.5 - y) / edge).clamp(0.0, 1.0);
            lift *= ex.min(ey);
        }
        let k = 1.0 - lift;
        (c.0 + u * rx * k, c.1 + v * ry * k)
    });
    b
}

const WAVE_TYPES: &[&str] = &["sine", "square", "triangle", "sawtooth", "circle", "semicircle", "uncircle", "noise", "smoothNoise"];
const PINNING: &[&str] = &["none", "allEdges", "center", "leftEdge", "topEdge", "rightEdge", "bottomEdge", "horizontalEdges", "verticalEdges"];

effect!(
    WAVE_WARP_DEF,
    "waveWarp",
    "Wave Warp",
    "waves running across the layer, pushing the picture to and fro",
    [
        choice("waveType", "Wave Type", WAVE_TYPES, 0),
        ranged("waveHeight", "Wave Height", 10.0, -1000.0, 1000.0),
        ranged("waveWidth", "Wave Width", 40.0, 1.0, 10_000.0),
        angle("direction", "Direction", 90.0).doc("which way they run (90: to the right, pushing up and down)"),
        ranged("waveSpeed", "Wave Speed", 1.0, -100.0, 100.0).doc("waves a second"),
        choice("pinning", "Pinning", PINNING, 0).doc("which edges stay put"),
        angle("phase", "Phase", 0.0),
    ]
);
pub const WAVE_WARP: Spec = Spec::new(&WAVE_WARP_DEF, CATEGORY, wave_warp);

/// A wave of `kind` at `t` (radians), from -1 to 1. The noise kinds come from `seed`.
fn wave(kind: u32, t: f64, seed: u64) -> f64 {
    let f = t.rem_euclid(TAU) / TAU;
    // half a circle over 0 to 1: 0 at its ends, 1 in its middle
    let arc = |u: f64| (1.0 - (2.0 * u - 1.0).powi(2)).max(0.0).sqrt();
    let level = |i: f64| crate::noise::unit(crate::noise::mix(&[seed, i as i64 as u64])) * 2.0 - 1.0;
    match kind {
        1 => {
            if f < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        2 => 1.0 - 4.0 * (f - 0.5).abs(),
        3 => 2.0 * f - 1.0,
        4 => {
            if f < 0.5 {
                arc(f * 2.0)
            } else {
                -arc(f * 2.0 - 1.0)
            }
        }
        5 => 2.0 * arc((f * 2.0).fract()) - 1.0,
        6 => 1.0 - 2.0 * arc((f * 2.0).fract()),
        // a level of its own every half wave, jumped to or eased into
        7 => level((t / TAU * 2.0).floor()),
        8 => {
            let u = t / TAU * 2.0;
            let (i, k) = (u.floor(), u - u.floor());
            let s = k * k * (3.0 - 2.0 * k);
            level(i) + (level(i + 1.0) - level(i)) * s
        }
        _ => t.sin(),
    }
}

/// How free a pixel is to move (0: held, 1: free) with `pin`, at (u, v) across the layer
/// (from 0 to 1).
fn freedom(pin: u32, u: f64, v: f64) -> f64 {
    let across = (v.min(1.0 - v) * 2.0).clamp(0.0, 1.0);
    let down = (u.min(1.0 - u) * 2.0).clamp(0.0, 1.0);
    match pin {
        1 => across.min(down),
        2 => ((u - 0.5).hypot(v - 0.5) * 2.0).min(1.0),
        3 => u,
        4 => v,
        5 => 1.0 - u,
        6 => 1.0 - v,
        7 => across,
        8 => down,
        _ => 1.0,
    }
}

fn wave_warp(ctx: &Ctx, mut b: Buf) -> Buf {
    let height = ctx.num("waveHeight", 10.0) * b.scale;
    let width = (ctx.num("waveWidth", 40.0) * b.scale).max(1.0);
    let phase = ctx.num("phase", 0.0).to_radians() + ctx.time.seconds() * ctx.num("waveSpeed", 1.0) * TAU;
    let (kind, pin) = (ctx.choice("waveType"), ctx.choice("pinning"));
    let dir = ctx.num("direction", 90.0).to_radians();
    // straight along an axis exactly, so a square wave's edge does not lean by a rounding error
    let exact = |v: f64| if v.abs() < 1e-12 { 0.0 } else { v };
    let (dx, dy) = (exact(dir.sin()), exact(-dir.cos()));
    if !ctx.adjustment {
        b.pad(height.abs().ceil() as usize + 1);
    }
    let (lx, ly, lw, lh) = rect_px(ctx, &b);
    let seed = ctx.seed;
    b.img = remap(&b.img, false, |x, y| {
        // measured between the outermost pixels' middles, so held edges stay exactly
        let (u, v) = (((x - lx - 0.5) / (lw - 1.0).max(1.0)).clamp(0.0, 1.0), ((y - ly - 0.5) / (lh - 1.0).max(1.0)).clamp(0.0, 1.0));
        let push = wave(kind, (x * dx + y * dy) / width * TAU + phase, seed) * height * freedom(pin, u, v);
        // across the way the waves run
        (x + dy * push, y - dx * push)
    });
    b
}

effect!(
    RIPPLE_DEF,
    "ripple",
    "Ripple",
    "rings spreading from a point, as on water",
    [
        pct("radius", "Radius", 50.0, 0.0, 100.0).doc("100: from the middle of the layer to its edge"),
        point("centerOfRipple", "Center of Ripple"),
        choice("typeOfConversion", "Type of Conversion", &["asymmetric", "symmetric"], 0).doc("asymmetric ones also push sideways"),
        ranged("waveSpeed", "Wave Speed", 1.0, -15.0, 15.0),
        ranged("waveWidth", "Wave Width", 20.0, 1.0, 1000.0),
        ranged("waveHeight", "Wave Height", 10.0, 0.0, 400.0),
        angle("ripplePhase", "Ripple Phase", 0.0),
    ]
);
pub const RIPPLE: Spec = Spec::new(&RIPPLE_DEF, CATEGORY, ripple).points(&[("centerOfRipple", [0.5, 0.5])]);

fn ripple(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = (ctx.num("radius", 50.0) / 100.0 * ctx.bounds.width().max(ctx.bounds.height()) * 0.5 * b.scale).max(1.0);
    let sideways = ctx.choice("typeOfConversion") == 0;
    let c = at(&b, ctx.point("centerOfRipple"));
    let width = (ctx.num("waveWidth", 20.0) * b.scale).max(1.0);
    let height = ctx.num("waveHeight", 10.0) * b.scale;
    let phase = ctx.num("ripplePhase", 0.0).to_radians() - ctx.time.seconds() * ctx.num("waveSpeed", 1.0) * TAU;
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let d = dx.hypot(dy);
        if d >= r || d == 0.0 {
            return (x, y);
        }
        let fall = 1.0 - d / r;
        let a = d / width * TAU + phase;
        let out = a.sin() * height * fall;
        let side = if sideways { a.cos() * height * fall * 0.5 } else { 0.0 };
        let (ux, uy) = (dx / d, dy / d);
        (x + ux * out - uy * side, y + uy * out + ux * side)
    });
    b
}

effect!(
    MIRROR_DEF,
    "mirror",
    "Mirror",
    "one side of a line reflected onto the other",
    [point("reflectionCenter", "Reflection Center"), angle("reflectionAngle", "Reflection Angle", 0.0).doc("0: the left side shows on the right too"),]
);
pub const MIRROR: Spec = Spec::new(&MIRROR_DEF, CATEGORY, mirror).points(&[("reflectionCenter", [0.5, 0.5])]);

fn mirror(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = at(&b, ctx.point("reflectionCenter"));
    let (s, co) = ctx.num("reflectionAngle", 0.0).to_radians().sin_cos();
    b.img = remap(&b.img, false, |x, y| {
        let d = (x - c.0) * co + (y - c.1) * s;
        if d > 0.0 { (x - 2.0 * d * co, y - 2.0 * d * s) } else { (x, y) }
    });
    b
}

effect!(
    OFFSET_DEF,
    "offset",
    "Offset",
    "the picture slid across the layer, what goes off one side coming back on the other",
    [point("shiftCenterTo", "Shift Center To"), pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),]
);
pub const OFFSET: Spec = Spec::new(&OFFSET_DEF, CATEGORY, offset).points(&[("shiftCenterTo", [0.5, 0.5])]);

fn offset(ctx: &Ctx, mut b: Buf) -> Buf {
    let (lx, ly, lw, lh) = rect_px(ctx, &b);
    let to = at(&b, ctx.point("shiftCenterTo"));
    let (sx, sy) = (to.0 - (lx + lw * 0.5), to.1 - (ly + lh * 0.5));
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    let before = b.img.clone();
    b.img = remap(&b.img, false, |x, y| {
        // within the layer's rectangle only, round and round
        if x < lx || y < ly || x >= lx + lw || y >= ly + lh {
            return (x, y);
        }
        (lx + (x - lx - sx).rem_euclid(lw), ly + (y - ly - sy).rem_euclid(lh))
    });
    if keep > 0.0 {
        for (p, q) in b.img.px.iter_mut().zip(&before.px) {
            for k in 0..4 {
                p[k] += (q[k] - p[k]) * keep;
            }
        }
    }
    b
}

effect!(
    POLAR_DEF,
    "polarCoordinates",
    "Polar Coordinates",
    "the picture bent round into a circle (its top in the middle), or a circle unrolled",
    [pct("interpolation", "Interpolation", 0.0, 0.0, 100.0).doc("how far it goes"), choice("typeOfConversion", "Type of Conversion", &["rectToPolar", "polarToRect"], 0),]
);
pub const POLAR: Spec = Spec::new(&POLAR_DEF, CATEGORY, polar);

fn polar(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = (ctx.num("interpolation", 0.0) / 100.0).clamp(0.0, 1.0);
    if k == 0.0 {
        return b;
    }
    let into_circle = ctx.choice("typeOfConversion") == 0;
    let (lx, ly, lw, lh) = rect_px(ctx, &b);
    let (cx, cy) = (lx + lw * 0.5, ly + lh * 0.5);
    let reach = (lw.min(lh) * 0.5).max(1.0);
    b.img = remap(&b.img, true, |x, y| {
        let (px, py) = if into_circle {
            // round from the top clockwise is across the picture; out from the middle is down it
            let (dx, dy) = (x - cx, y - cy);
            let a = dx.atan2(-dy).rem_euclid(TAU);
            (lx + a / TAU * lw, ly + dx.hypot(dy) / reach * lh)
        } else {
            let a = (x - lx) / lw * TAU;
            let r = (y - ly) / lh * reach;
            (cx + r * a.sin(), cy - r * a.cos())
        };
        (x + (px - x) * k, y + (py - y) * k)
    });
    b
}

effect!(
    TRANSFORM_DEF,
    "transform",
    "Transform",
    "the layer moved, scaled, skewed, turned and faded inside its effects, with a motion blur of its own",
    [
        point("anchorPoint", "Anchor Point"),
        point("position", "Position"),
        flag("uniformScale", "Uniform Scale", true),
        ranged("scaleHeight", "Scale Height", 100.0, -10_000.0, 10_000.0),
        ranged("scaleWidth", "Scale Width", 100.0, -10_000.0, 10_000.0),
        ranged("skew", "Skew", 0.0, -70.0, 70.0),
        angle("skewAxis", "Skew Axis", 0.0),
        angle("rotation", "Rotation", 0.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        flag("useCompositionsShutterAngle", "Use Composition's Shutter Angle", true).doc("blurred as the composition's motion blur is (when it is on for the layer)"),
        ranged("shutterAngle", "Shutter Angle", 0.0, 0.0, 720.0),
        choice("sampling", "Sampling", &["bilinear", "bicubic"], 0),
    ]
);
pub const TRANSFORM: Spec = Spec::new(&TRANSFORM_DEF, CATEGORY, transform).points(&[("anchorPoint", [0.5, 0.5]), ("position", [0.5, 0.5])]);

/// The Transform settings at layer time `t` as a matrix of the layer's units, and its opacity.
fn transform_at(ctx: &Ctx, t: Tick) -> (Mat3, f32) {
    let n = |id: &str, d: f64| ctx.value_at(id, t).map_or(d, |v| v.as_f64());
    let p = |id: &str| ctx.value_at(id, t).map_or(vec2(0.0, 0.0), |v| {
        let [x, y] = v.as_vec2();
        vec2(x, y)
    });
    let sh = n("scaleHeight", 100.0);
    let sw = if ctx.value_at("uniformScale", t).is_none_or(|v| v.as_bool()) { sh } else { n("scaleWidth", 100.0) };
    let m = Mat3::translate(p("position"))
        * Mat3::rotate_deg(n("rotation", 0.0))
        * Mat3::skew_deg(n("skew", 0.0), n("skewAxis", 0.0))
        * Mat3::scale(vec2(sw / 100.0, sh / 100.0))
        * Mat3::translate(-p("anchorPoint"));
    (m, (n("opacity", 100.0) / 100.0).clamp(0.0, 1.0) as f32)
}

/// The buffer made big enough for what the matrices `ms` (of layer units) take its picture to
/// (under an adjustment layer it stays the composition), and the old picture.
fn grown(ctx: &Ctx, b: &mut Buf, ms: &[Mat3]) -> Image {
    let old = b.img.clone();
    if !ctx.adjustment {
        let area = b.area();
        let mut reach = area;
        for m in ms {
            reach = reach.union(&m.map_rect(&area));
        }
        b.cover(reach.intersect(&area.inflate(8192.0 / b.scale)));
    }
    b.img = Image::new(b.img.width, b.img.height);
    old
}

/// From the old buffer's pixels to the new one's, through `m` (of layer units).
fn through(old: &Buf, new: &Buf, m: Mat3) -> Mat3 {
    Mat3::scale(vec2(new.scale, new.scale)) * Mat3::translate(-new.origin) * m * old.to_layer()
}

fn transform(ctx: &Ctx, mut b: Buf) -> Buf {
    let how = if ctx.choice("sampling") == 1 { Sampling::Bicubic } else { Sampling::Bilinear };
    // the shutter: open this many degrees of a frame, from this many, in at least so many pictures
    let shutter = if ctx.flag("useCompositionsShutterAngle") {
        ctx.shutter
    } else {
        let a = ctx.num("shutterAngle", 0.0).clamp(0.0, 720.0);
        (a > 0.0).then_some((a, -a * 0.5, 16))
    };
    let frame = ctx.frame.seconds();
    let moments: Vec<(Mat3, f32)> = match shutter {
        Some((angle, phase, least)) if angle > 0.0 && ctx.read_at.is_some() => {
            let when = |f: f64| ctx.time + Tick::from_seconds((phase + angle * f) / 360.0 * frame);
            // as many pictures as the farthest corner moves pixels, so the blur is smooth
            let (first, last) = (transform_at(ctx, when(0.0)).0, transform_at(ctx, when(1.0)).0);
            let corners = b.area().corners();
            let travel = corners.iter().map(|c| (first.apply(*c) - last.apply(*c)).length() * b.scale).fold(0.0, f64::max);
            let n = (travel.ceil() as u32).clamp(least.max(2), 128) as usize;
            (0..n).map(|i| transform_at(ctx, when(i as f64 / (n - 1) as f64))).collect()
        }
        _ => vec![transform_at(ctx, ctx.time)],
    };
    let before = b.clone();
    let ms: Vec<Mat3> = moments.iter().map(|m| m.0).collect();
    let old = grown(ctx, &mut b, &ms);
    let weight = 1.0 / moments.len() as f32;
    for (m, opacity) in &moments {
        let into = through(&before, &b, *m);
        if moments.len() == 1 {
            veymelo_raster::place(&mut b.img, &old, &into, how, veymelo_color::BlendMode::Normal, *opacity, 0);
        } else {
            veymelo_raster::accumulate(&mut b.img, &old, &into, how, weight * opacity);
        }
    }
    b
}

effect!(
    CORNER_PIN_DEF,
    "cornerPin",
    "Corner Pin",
    "the layer's four corners pinned anywhere, the picture stretched between them in perspective",
    [point("upperLeft", "Upper Left"), point("upperRight", "Upper Right"), point("lowerLeft", "Lower Left"), point("lowerRight", "Lower Right"),]
);
pub const CORNER_PIN: Spec = Spec::new(&CORNER_PIN_DEF, CATEGORY, corner_pin).points(&[("upperLeft", [0.0, 0.0]), ("upperRight", [1.0, 0.0]), ("lowerLeft", [0.0, 1.0]), ("lowerRight", [1.0, 1.0])]);

fn corner_pin(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = ctx.bounds;
    let from = [vec2(r.x0, r.y0), vec2(r.x1, r.y0), vec2(r.x1, r.y1), vec2(r.x0, r.y1)];
    let to = [ctx.point("upperLeft"), ctx.point("upperRight"), ctx.point("lowerRight"), ctx.point("lowerLeft")];
    let Some(m) = Mat3::quad_to_quad(from, to) else { return b };
    // pins crossed over or in a line: nothing to stretch the picture across
    let turn = |a: Vec2, o: Vec2, c: Vec2| (a - o).cross(c - o);
    let signs = [turn(to[3], to[0], to[1]), turn(to[0], to[1], to[2]), turn(to[1], to[2], to[3]), turn(to[2], to[3], to[0])];
    if !(signs.iter().all(|s| *s > 0.0) || signs.iter().all(|s| *s < 0.0)) {
        b.img = Image::new(b.img.width, b.img.height);
        return b;
    }
    // the buffer grown to hold the pins (not a long way past the layer)
    let before = b.clone();
    if !ctx.adjustment {
        let pins = to.iter().fold(Rect::new(to[0].x, to[0].y, to[0].x, to[0].y), |mut a, p| {
            a.include(*p);
            a
        });
        b.cover(pins.intersect(&before.area().inflate(8192.0 / b.scale)));
    }
    b.img = Image::new(b.img.width, b.img.height);
    // only the layer's rectangle is stretched (the buffer may hold more round it)
    let (a, z) = (before.pixel(from[0]), before.pixel(from[2]));
    let (x0, y0) = (a.x.round().max(0.0) as usize, a.y.round().max(0.0) as usize);
    let (x1, y1) = ((z.x.round().max(0.0) as usize).min(before.img.width), (z.y.round().max(0.0) as usize).min(before.img.height));
    if x0 >= x1 || y0 >= y1 {
        return b;
    }
    let mut src = Image::new(x1 - x0, y1 - y0);
    for y in y0..y1 {
        src.px[(y - y0) * src.width..(y - y0 + 1) * src.width].copy_from_slice(&before.img.px[y * before.img.width + x0..y * before.img.width + x1]);
    }
    let into = through(&before, &b, m) * Mat3::translate(vec2(x0 as f64, y0 as f64));
    veymelo_raster::place(&mut b.img, &src, &into, Sampling::Bilinear, veymelo_color::BlendMode::Normal, 1.0, 0);
    b
}

/// Every effect here.
pub const ALL: [Spec; 10] = [TWIRL, SPHERIZE, BULGE, WAVE_WARP, RIPPLE, MIRROR, OFFSET, POLAR, TRANSFORM, CORNER_PIN];

#[cfg(test)]
mod tests;
