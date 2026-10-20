//! Distortions, part three: bent along an axis (Bender, Slant), a picture's light turned to
//! glossy blobs (Blobbylize), pinned by its corners (Power Pin), rings and pulls (Ripple Pulse,
//! Smudge, Split, Split 2), reshaped by masks (Smear, Reshape), the old twirl, the fifteen warp
//! styles (Warp), and made bigger keeping its detail (Detail-preserving Upscale).
//!
//! Sizes and places are in layer units, so a preview bends the same as the full picture.

use std::f64::consts::{PI, TAU};

use veymelo_geom::{Mat3, Vec2, vec2};
use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px, Sampling};

use crate::color::rgb3;
use crate::defs::{angle, choice, color, effect, flag, layer, num, pct, point, px, ranged};
use crate::distort::remap;
use crate::draw::{distance, even, inside, length, point_at};
use crate::util::{hsl, luma, map_px, premul, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "distort";

fn at(b: &Buf, p: Vec2) -> (f64, f64) {
    let q = b.pixel(p);
    (q.x, q.y)
}

effect!(
    BENDER_DEF,
    "bender",
    "Bender",
    "the layer bent sideways along a line from its base to its top, like a reed in the wind",
    [
        ranged("amount", "Amount", 0.0, -200.0, 200.0),
        choice("style", "Style", &["bend", "marilyn", "sharp", "boxer"], 0).doc("bend: a smooth curve; marilyn: billowing in the middle; sharp: leaning straight; boxer: an S"),
        flag("adjustToDistance", "Adjust To Distance", false).doc("how far it bends goes by the line's length, not the layer's height"),
        point("top", "Top"),
        point("base", "Base"),
    ]
);
pub const BENDER: Spec = Spec::new(&BENDER_DEF, CATEGORY, bender).points(&[("top", [0.5, 0.0]), ("base", [0.5, 1.0])]);

fn bender(ctx: &Ctx, mut b: Buf) -> Buf {
    let amount = ctx.num("amount", 0.0) / 100.0;
    if amount == 0.0 {
        return b;
    }
    let style = ctx.choice("style");
    let line = (ctx.point("top") - ctx.point("base")) * b.scale;
    let len = line.length().max(1e-6);
    let up = (line.x / len, line.y / len);
    // to the right of the line when it points up
    let side = (-up.1, up.0);
    let reach = if ctx.flag("adjustToDistance") { len } else { ctx.bounds.height() * b.scale };
    // how far along the line (0 at the base, 1 at the top) leans
    let lean = move |t: f64| {
        let t = t.clamp(0.0, 1.0);
        match style {
            1 => 0.5 * (PI * t).sin() * t,
            2 => 0.5 * t,
            3 => 0.25 * (1.0 - (PI * t).cos()) - 0.1 * (TAU * t).sin(),
            _ => 0.5 * t * t,
        }
    };
    if !ctx.adjustment {
        b.pad((amount.abs() * reach * 0.55).ceil().min(4096.0) as usize);
    }
    let base = at(&b, ctx.point("base"));
    b.img = remap(&b.img, false, |x, y| {
        let t = ((x - base.0) * up.0 + (y - base.1) * up.1) / len;
        let d = amount * reach * lean(t);
        (x - side.0 * d, y - side.1 * d)
    });
    b
}

/// A light shining from `direction` (0: from the top, turning clockwise) at `height` (0: from
/// the side, 100: from straight in front), as a direction to it.
fn distant(direction: f64, height: f64) -> [f64; 3] {
    let up = (height / 100.0).clamp(0.0, 1.0) * PI * 0.5;
    let d = direction.to_radians();
    [d.sin() * up.cos(), -d.cos() * up.cos(), up.sin()]
}

fn unit3(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

const CHANNELS: &[&str] = &["red", "green", "blue", "alpha", "luminance", "hue", "lightness", "saturation"];

/// A pixel's value by `channel` of [`CHANNELS`], from 0 to 1 (colours weighed by how much is there).
fn channel(ch: u32, p: Px) -> f32 {
    let (c, a) = unpremul(p);
    let (h, s, l) = hsl(c);
    match ch {
        0 => c[0] * a,
        1 => c[1] * a,
        2 => c[2] * a,
        3 => a,
        5 => h * a,
        6 => l * a,
        7 => s * a,
        _ => luma(c) * a,
    }
}

effect!(
    BLOBBYLIZE_DEF,
    "blobbylize",
    "Blobbylize",
    "the layer melted into glossy blobs, shaped by how bright (or solid) a layer is and lit",
    [
        layer("blobLayer", "Blob Layer").doc("none: the layer itself"),
        choice("property", "Property", CHANNELS, 3),
        px("softness", "Softness", 6.0, 200.0),
        pct("cutAway", "Cut Away", 0.0, 0.0, 100.0),
        ranged("lightIntensity", "Light Intensity", 100.0, 0.0, 400.0),
        color("lightColor", "Light Color", 1.0, 1.0, 1.0),
        choice("lightType", "Light Type", &["distantLight", "pointLight"], 0),
        pct("lightHeight", "Light Height", 50.0, 0.0, 100.0),
        point("lightPosition", "Light Position"),
        angle("lightDirection", "Light Direction", -45.0),
        ranged("ambient", "Ambient", 20.0, 0.0, 200.0),
        ranged("diffuse", "Diffuse", 70.0, 0.0, 200.0),
        ranged("specular", "Specular", 40.0, 0.0, 200.0),
        ranged("roughness", "Roughness", 0.05, 0.001, 0.5),
        pct("metal", "Metal", 100.0, 0.0, 100.0).doc("the shine takes the layer's colour"),
    ]
);
pub const BLOBBYLIZE: Spec = Spec::new(&BLOBBYLIZE_DEF, CATEGORY, blobbylize).points(&[("lightPosition", [0.25, 0.25])]);

fn blobbylize(ctx: &Ctx, mut b: Buf) -> Buf {
    let soft = ctx.num("softness", 6.0).max(0.0) * b.scale;
    let cut = (ctx.num("cutAway", 0.0) / 100.0).clamp(0.0, 0.99) as f32;
    let blob = ctx.layer_over("blobLayer", &b, Take::All, true).unwrap_or_else(|| b.img.clone());
    let ch = ctx.choice("property");
    let (w, h) = (b.img.width, b.img.height);
    let mut plane = Image::new(w, h);
    for (o, p) in plane.px.iter_mut().zip(&blob.px) {
        *o = [channel(ch, *p); 4];
    }
    // the heights: the channel softened into round slopes
    let height: Vec<f32> = if soft > 0.0 { filter::gaussian(&plane, soft, soft, Edge::Clear).px.iter().map(|p| p[0]).collect() } else { plane.px.iter().map(|p| p[0]).collect() };
    let hat = |x: i64, y: i64| height[(y.clamp(0, h as i64 - 1) as usize) * w + x.clamp(0, w as i64 - 1) as usize] as f64;
    let strength = ctx.num("lightIntensity", 100.0) / 100.0;
    let light_c = rgb3(ctx.color("lightColor"));
    let point_light = ctx.choice("lightType") == 1;
    let lp = at(&b, ctx.point("lightPosition"));
    let lift = ctx.num("lightHeight", 50.0);
    let towards = distant(ctx.num("lightDirection", -45.0), lift);
    let (ambient, diffuse, specular) = (ctx.num("ambient", 20.0) / 100.0, ctx.num("diffuse", 70.0) / 100.0, ctx.num("specular", 40.0) / 100.0);
    let shine = (1.0 / ctx.num("roughness", 0.05).clamp(0.001, 1.0)).min(500.0);
    let metal = (ctx.num("metal", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let depth = (soft * 0.5).max(1.0);
    let span = (w.max(h) as f64).max(1.0);
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (xi, yi) = (x as i64, y as i64);
        let (gx, gy) = ((hat(xi + 1, yi) - hat(xi - 1, yi)) * 0.5, (hat(xi, yi + 1) - hat(xi, yi - 1)) * 0.5);
        // seen through the blob's surface, bent down its slopes
        let s = src.sample(Sampling::Bilinear, x as f64 + 0.5 - gx * depth * 2.0, y as f64 + 0.5 - gy * depth * 2.0);
        let (c, a) = unpremul(s);
        let matte = ((height[y * w + x] - cut) / ((1.0 - cut) * 0.5).max(1e-3)).clamp(0.0, 1.0);
        let alpha = a * matte;
        if alpha <= 0.0 {
            return [0.0; 4];
        }
        let n = unit3([-gx * depth, -gy * depth, 1.0]);
        let l = if point_light { unit3([lp.0 - x as f64, lp.1 - y as f64, lift / 100.0 * span]) } else { towards };
        let lit = dot3(n, l).max(0.0);
        let half = unit3([l[0], l[1], l[2] + 1.0]);
        let gloss = dot3(n, half).max(0.0).powf(shine) * specular;
        let out = [0, 1, 2].map(|i| {
            let light = (ambient + diffuse * lit * strength) as f32 * light_c[i];
            let shine_c = (1.0 - metal) * light_c[i] + metal * c[i] * light_c[i];
            c[i] * light + (gloss * strength) as f32 * shine_c
        });
        premul(out, alpha)
    });
    b
}

effect!(
    POWER_PIN_DEF,
    "powerPin",
    "Power Pin",
    "the layer's corners pinned anywhere, in perspective or not, or (unstretched) a pinned part of it stretched back to fill it",
    [
        point("topLeft", "Top Left"),
        point("topRight", "Top Right"),
        point("bottomLeft", "Bottom Left"),
        point("bottomRight", "Bottom Right"),
        pct("perspective", "Perspective", 100.0, 0.0, 100.0).doc("0: stretched evenly between the pins"),
        flag("unstretch", "Unstretch", false),
        ranged("expandTop", "Top", 0.0, -100.0, 1000.0).doc("the part pinned reaches this percent past the layer's top"),
        ranged("expandLeft", "Left", 0.0, -100.0, 1000.0),
        ranged("expandRight", "Right", 0.0, -100.0, 1000.0),
        ranged("expandBottom", "Bottom", 0.0, -100.0, 1000.0),
    ]
);
pub const POWER_PIN: Spec =
    Spec::new(&POWER_PIN_DEF, CATEGORY, power_pin).points(&[("topLeft", [0.0, 0.0]), ("topRight", [1.0, 0.0]), ("bottomLeft", [0.0, 1.0]), ("bottomRight", [1.0, 1.0])]);

fn power_pin(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = ctx.bounds;
    let (lw, lh) = (r.width().max(1.0), r.height().max(1.0));
    let n = |id: &str| ctx.num(id, 0.0) / 100.0;
    // the part of the layer that is pinned (layer units)
    let (x0, y0, x1, y1) = (r.x0 - n("expandLeft") * lw, r.y0 - n("expandTop") * lh, r.x1 + n("expandRight") * lw, r.y1 + n("expandBottom") * lh);
    let pins = [ctx.point("topLeft"), ctx.point("topRight"), ctx.point("bottomRight"), ctx.point("bottomLeft")];
    let unstretch = ctx.flag("unstretch");
    if !ctx.adjustment && !unstretch {
        let mut reach = b.area();
        for p in &pins {
            reach.include(*p);
        }
        b.cover(reach.intersect(&b.area().inflate(8192.0 / b.scale)));
    }
    let q = pins.map(|p| b.pixel(p));
    let k = (ctx.num("perspective", 100.0) / 100.0).clamp(0.0, 1.0);
    let ahead = Mat3::square_to_quad(q);
    let Some(back) = ahead.inverse() else { return b };
    // evenly between the pins (no perspective)
    let even_at = move |u: f64, v: f64| {
        let top = q[0] + (q[1] - q[0]) * u;
        let bottom = q[3] + (q[2] - q[3]) * u;
        top + (bottom - top) * v
    };
    let src = b.img.clone();
    let (s, o) = (b.scale, b.origin);
    map_px(&mut b.img, |x, y, _| {
        let p = vec2(x as f64 + 0.5, y as f64 + 0.5);
        let inside = |u: f64, v: f64| (-1e-9..=1.0 + 1e-9).contains(&u) && (-1e-9..=1.0 + 1e-9).contains(&v);
        if unstretch {
            // the layer's rectangle filled with what lies between the pins
            let lp = o + p / s;
            let (u, v) = ((lp.x - x0) / (x1 - x0), (lp.y - y0) / (y1 - y0));
            if !inside(u, v) {
                return [0.0; 4];
            }
            let (m, e) = (ahead.apply(vec2(u, v)), even_at(u, v));
            let f = e + (m - e) * k;
            return src.sample(Sampling::Bilinear, f.x, f.y);
        }
        let m = back.apply(p);
        let (mut u, mut v) = (m.x, m.y);
        if k < 1.0 {
            // where the even stretch puts this point, found step by step from the perspective's
            let (mut eu, mut ev) = (u, v);
            for _ in 0..10 {
                let f = even_at(eu, ev);
                let (rx, ry) = (f.x - p.x, f.y - p.y);
                if rx.abs() + ry.abs() < 1e-6 {
                    break;
                }
                let e = 1e-4;
                let (fa, fb) = (even_at(eu + e, ev), even_at(eu, ev + e));
                let (j00, j10, j01, j11) = ((fa.x - f.x) / e, (fa.y - f.y) / e, (fb.x - f.x) / e, (fb.y - f.y) / e);
                let det = j00 * j11 - j01 * j10;
                if det.abs() < 1e-12 {
                    break;
                }
                eu -= (j11 * rx - j01 * ry) / det;
                ev -= (-j10 * rx + j00 * ry) / det;
            }
            u = eu + (u - eu) * k;
            v = ev + (v - ev) * k;
        }
        if !inside(u, v) {
            return [0.0; 4];
        }
        let lp = vec2(x0 + u * (x1 - x0), y0 + v * (y1 - y0));
        let f = (lp - o) * s;
        src.sample(Sampling::Bilinear, f.x, f.y)
    });
    b
}

effect!(
    RIPPLE_PULSE_DEF,
    "ripplePulse",
    "Ripple Pulse",
    "a ring of ripples running out from a point while the pulse level is up (animate it)",
    [
        point("center", "Center"),
        ranged("pulseLevel", "Pulse Level (animate)", 0.0, -1000.0, 1000.0),
        ranged("timeSpan", "Time Span (sec)", 1.0, 0.01, 100.0).doc("how long a ripple lasts"),
        ranged("amplitude", "Amplitude", 100.0, -1000.0, 1000.0),
        flag("renderBumpAsWell", "Render Bump As Well", false).doc("the ripples' slopes lit and shaded"),
    ]
);
pub const RIPPLE_PULSE: Spec = Spec::new(&RIPPLE_PULSE_DEF, CATEGORY, ripple_pulse).points(&[("center", [0.5, 0.5])]);

fn ripple_pulse(ctx: &Ctx, mut b: Buf) -> Buf {
    let span = ctx.num("timeSpan", 1.0).max(0.01);
    let t = ctx.time.seconds().max(0.0);
    let size = ctx.bounds.width().max(ctx.bounds.height()) * b.scale;
    // the ring crosses half the layer in one time span
    let speed = size * 0.5 / span;
    let wavelength = (size * 0.06).max(2.0);
    let amp = ctx.num("amplitude", 100.0) / 100.0;
    // the ripples at a distance were made when the front was there: as high as the pulse
    // level was then (read at steps back over the time span)
    const STEPS: usize = 64;
    let levels: Vec<f64> = (0..=STEPS)
        .map(|i| {
            let age = span * i as f64 / STEPS as f64;
            if age > t { 0.0 } else { ctx.value_at("pulseLevel", veymelo_time::Tick::from_seconds(t - age)).map_or(0.0, |v| v.as_f64()) / 100.0 }
        })
        .collect();
    if amp == 0.0 || levels.iter().all(|l| *l == 0.0) {
        return b;
    }
    let height = amp * wavelength * 0.5;
    let wave = |r: f64| -> f64 {
        let age = r / speed;
        if age >= span {
            return 0.0;
        }
        let k = age / span * STEPS as f64;
        let (i, f) = (k.floor() as usize, k - k.floor());
        let level = levels[i] + (levels[(i + 1).min(STEPS)] - levels[i]) * f;
        // running outward, fading as they go
        (TAU * (r - speed * t) / wavelength).sin() * level * (1.0 - age / span)
    };
    let c = at(&b, ctx.point("center"));
    let bump = ctx.flag("renderBumpAsWell");
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (dx, dy) = (x as f64 + 0.5 - c.0, y as f64 + 0.5 - c.1);
        let r = dx.hypot(dy);
        let d = wave(r) * height;
        let (ux, uy) = if r > 1e-9 { (dx / r, dy / r) } else { (0.0, 0.0) };
        let mut s = src.sample(Sampling::Bilinear, x as f64 + 0.5 - ux * d, y as f64 + 0.5 - uy * d);
        if bump && d != 0.0 {
            let slope = (wave(r + 0.5) - wave(r - 0.5)) * height;
            let k = (1.0 - slope * 0.5).clamp(0.0, 2.0) as f32;
            for v in s.iter_mut().take(3) {
                *v *= k;
            }
        }
        s
    });
    b
}

effect!(
    SLANT_DEF,
    "slant",
    "Slant",
    "the layer leaned over from its floor (and taller or shorter)",
    [
        ranged("slant", "Slant", 0.0, -1000.0, 1000.0),
        flag("stretching", "Stretching", false).doc("narrower as it leans, so it keeps its length"),
        ranged("height", "Height", 100.0, -1000.0, 1000.0),
        point("floor", "Floor"),
    ]
);
pub const SLANT: Spec = Spec::new(&SLANT_DEF, CATEGORY, slant).points(&[("floor", [0.5, 1.0])]);

fn slant(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = ctx.num("slant", 0.0) / 100.0;
    let height = ctx.num("height", 100.0) / 100.0;
    if k == 0.0 && (height - 1.0).abs() < 1e-12 {
        return b;
    }
    let narrow = if ctx.flag("stretching") { 1.0 / (1.0 + k * k).sqrt() } else { 1.0 };
    let tall = if height.abs() < 1e-6 { 1e6 } else { 1.0 / height };
    if !ctx.adjustment {
        let lh = ctx.bounds.height() * b.scale;
        b.pad(((height - 1.0).max(0.0) * lh).max((k * lh * height.max(1.0)).abs()).ceil().min(4096.0) as usize);
    }
    let floor = at(&b, ctx.point("floor"));
    b.img = remap(&b.img, false, |x, y| (floor.0 + (x - floor.0 - (floor.1 - y) * k) / narrow, floor.1 + (y - floor.1) * tall));
    b
}

effect!(
    SMUDGE_DEF,
    "smudge",
    "Smudge",
    "the picture pulled from one point toward another, as with a finger",
    [point("from", "From"), point("to", "To"), ranged("reach", "Reach", 50.0, -100.0, 100.0), px("radius", "Radius", 50.0, 4000.0),]
);
pub const SMUDGE: Spec = Spec::new(&SMUDGE_DEF, CATEGORY, smudge).points(&[("from", [0.35, 0.5]), ("to", [0.65, 0.5])]);

fn smudge(ctx: &Ctx, mut b: Buf) -> Buf {
    let reach = ctx.num("reach", 50.0) / 100.0;
    let (f0, t0) = (ctx.point("from"), ctx.point("to"));
    let moved = ((t0 - f0).length() * reach.abs() * b.scale).ceil();
    if !ctx.adjustment && moved > 0.0 {
        b.pad(moved.min(4096.0) as usize);
    }
    let (from, to) = (at(&b, f0), at(&b, t0));
    let radius = (ctx.num("radius", 50.0) * b.scale).max(0.5);
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let l2 = dx * dx + dy * dy;
    if l2 < 1e-12 || reach == 0.0 {
        return b;
    }
    b.img = remap(&b.img, false, |x, y| {
        let t = (((x - from.0) * dx + (y - from.1) * dy) / l2).clamp(0.0, 1.0);
        let (px, py) = (from.0 + dx * t, from.1 + dy * t);
        let w = (1.0 - (x - px).hypot(y - py) / radius).max(0.0);
        let k = reach * w * w * (3.0 - 2.0 * w) * t;
        (x - dx * k, y - dy * k)
    });
    b
}

effect!(
    SPLIT_DEF,
    "split",
    "Split",
    "the layer split open along a line between two points, like a cut pulled apart",
    [point("pointA", "Point A"), point("pointB", "Point B"), ranged("split", "Split", 50.0, 0.0, 1000.0),]
);
pub const SPLIT: Spec = Spec::new(&SPLIT_DEF, CATEGORY, split).points(&[("pointA", [0.25, 0.5]), ("pointB", [0.75, 0.5])]);

effect!(
    SPLIT2_DEF,
    "split2",
    "Split 2",
    "the layer split open along a line, each side by its own amount",
    [point("pointA", "Point A"), point("pointB", "Point B"), ranged("split1", "Split 1", 50.0, 0.0, 1000.0), ranged("split2", "Split 2", 50.0, 0.0, 1000.0),]
);
pub const SPLIT2: Spec = Spec::new(&SPLIT2_DEF, CATEGORY, split2).points(&[("pointA", [0.25, 0.5]), ("pointB", [0.75, 0.5])]);

fn split(ctx: &Ctx, b: Buf) -> Buf {
    let s = ctx.num("split", 50.0) / 100.0;
    split_open(ctx, b, s, s)
}

fn split2(ctx: &Ctx, b: Buf) -> Buf {
    split_open(ctx, b, ctx.num("split1", 50.0) / 100.0, ctx.num("split2", 50.0) / 100.0)
}

/// Split open between the points, the left side (looking from A to B) by `left`, the right
/// by `right`.
fn split_open(ctx: &Ctx, mut b: Buf, left: f64, right: f64) -> Buf {
    if left == 0.0 && right == 0.0 {
        return b;
    }
    let (a, c) = (at(&b, ctx.point("pointA")), at(&b, ctx.point("pointB")));
    let len = (c.0 - a.0).hypot(c.1 - a.1);
    if len < 1e-6 {
        return b;
    }
    let (ux, uy) = ((c.0 - a.0) / len, (c.1 - a.1) / len);
    let (nx, ny) = (-uy, ux);
    let reach = len * 0.5;
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let (rx, ry) = (px - a.0, py - a.1);
        let t = (rx * ux + ry * uy) / len;
        let s = rx * nx + ry * ny;
        if !(0.0..=1.0).contains(&t) {
            return src.sample(Sampling::Bilinear, px, py);
        }
        let open = if s >= 0.0 { left } else { right } * reach * (PI * t).sin();
        let ad = s.abs();
        // in the gap a soft edge, a pixel wide
        let keep = (ad - open + 0.5).clamp(0.0, 1.0) as f32;
        if keep <= 0.0 {
            return [0.0; 4];
        }
        // the sides pushed apart, less and less over `reach` past the gap
        let push = open * (1.0 - (ad - open) / reach).max(0.0);
        let ns = s.signum() * (ad - push).max(0.0);
        src.sample(Sampling::Bilinear, px + nx * (ns - s), py + ny * (ns - s)).map(|v| v * keep)
    });
    b
}

/// How freely the picture moves away from the mask's outline: a stiffer one moves less far off.
const ELASTICITY: &[&str] = &["stiff", "lessStiff", "belowNormal", "absolutelyNormal", "aboveAverage", "loose", "liquid", "superFluid"];

fn elastic(i: u32) -> f64 {
    [1.0, 1.25, 1.5, 2.0, 2.5, 3.0, 3.5, 4.5][(i as usize).min(7)]
}

/// Mask number `id` of the layer (0: none) as an outline in the buffer's pixels, and whether
/// it is closed.
fn mask_px(ctx: &Ctx, b: &Buf, id: &str) -> Option<(Vec<[f64; 2]>, bool)> {
    let m = ctx.mask(ctx.num(id, 0.0).round().max(0.0) as usize)?;
    if m.points.len() < 2 {
        return None;
    }
    Some((
        m.points
            .iter()
            .map(|q| {
                let p = b.pixel(vec2(q[0], q[1]));
                [p.x, p.y]
            })
            .collect(),
        m.closed,
    ))
}

effect!(
    SMEAR_DEF,
    "smear",
    "Smear",
    "the part of the picture inside one mask moved, turned or scaled, the picture round it stretching to follow inside a boundary mask",
    [
        num("sourceMask", "Source Mask", 0.0).range(0.0, 1000.0).doc("which mask (from 1; 0: none)"),
        num("boundaryMask", "Boundary Mask", 0.0).range(0.0, 1000.0),
        point("maskOffset", "Mask Offset").doc("how far the source moves (layer units)"),
        angle("maskRotation", "Mask Rotation", 0.0),
        ranged("maskScale", "Mask Scale", 100.0, 1.0, 1000.0),
        pct("percent", "Percent", 100.0, 0.0, 100.0),
        choice("elasticity", "Elasticity", ELASTICITY, 3),
        choice("interpolationMethod", "Interpolation Method", &["discrete", "linear", "smooth"], 1),
    ]
);
pub const SMEAR: Spec = Spec::new(&SMEAR_DEF, CATEGORY, smear);

fn smear(ctx: &Ctx, mut b: Buf) -> Buf {
    let (Some((source, _)), Some((bound, _))) = (mask_px(ctx, &b, "sourceMask"), mask_px(ctx, &b, "boundaryMask")) else { return b };
    let k = ctx.num("percent", 100.0) / 100.0;
    if k == 0.0 {
        return b;
    }
    let off = ctx.point("maskOffset");
    let (ox, oy) = (off.x * b.scale, off.y * b.scale);
    let (sn, cs) = ctx.num("maskRotation", 0.0).to_radians().sin_cos();
    let scale = (ctx.num("maskScale", 100.0) / 100.0).max(1e-3);
    let stiff = elastic(ctx.choice("elasticity"));
    let smooth = ctx.choice("interpolationMethod") == 2;
    let n = source.len().max(1) as f64;
    let c = source.iter().fold([0.0, 0.0], |a, q| [a[0] + q[0] / n, a[1] + q[1] / n]);
    let moved: Vec<[f64; 2]> = source
        .iter()
        .map(|q| {
            let (dx, dy) = ((q[0] - c[0]) * scale, (q[1] - c[1]) * scale);
            [c[0] + dx * cs - dy * sn + ox, c[1] + dx * sn + dy * cs + oy]
        })
        .collect();
    // where a point of the moved source came from
    let back = |x: f64, y: f64| {
        let (dx, dy) = (x - ox - c[0], y - oy - c[1]);
        (c[0] + (dx * cs + dy * sn) / scale, c[1] + (-dx * sn + dy * cs) / scale)
    };
    b.img = remap(&b.img, false, |x, y| {
        if !inside(&bound, [x, y]) {
            return (x, y);
        }
        // all the way inside the moved source, less and less out to the boundary
        let mut w = if inside(&moved, [x, y]) {
            1.0
        } else {
            let (dm, db) = (distance(&moved, true, [x, y]), distance(&bound, true, [x, y]));
            (db / (db + dm).max(1e-9)).powf(1.0 / stiff)
        };
        if smooth {
            w = w * w * (3.0 - 2.0 * w);
        }
        let (fx, fy) = back(x, y);
        let t = (w * k).clamp(0.0, 1.0);
        (x + (fx - x) * t, y + (fy - y) * t)
    });
    b
}

effect!(
    RESHAPE_DEF,
    "reshape",
    "Reshape",
    "the picture inside one mask's outline reshaped to fit another's, within a boundary mask",
    [
        num("sourceMask", "Source Mask", 0.0).range(0.0, 1000.0).doc("which mask (from 1; 0: none)"),
        num("destinationMask", "Destination Mask", 0.0).range(0.0, 1000.0),
        num("boundaryMask", "Boundary Mask", 0.0).range(0.0, 1000.0).doc("0: the whole layer moves"),
        pct("percent", "Percent", 100.0, 0.0, 100.0),
        choice("elasticity", "Elasticity", ELASTICITY, 0),
        choice("interpolationMethod", "Interpolation Method", &["discrete", "linear", "smooth"], 1),
        veymelo_project::PropDef::new("correspondencePoints", "Correspondence Points", veymelo_project::Kind::Text, veymelo_keyframe::Value::Text(String::new()))
            .doc("which point of the source outline goes to which of the destination's, as fractions round them: \"0,0 0.25,0.3 …\"; empty: matched by itself"),
    ]
);
pub const RESHAPE: Spec = Spec::new(&RESHAPE_DEF, CATEGORY, reshape);

/// The pairs "source,destination" of fractions round the outlines, in the destination's order.
fn pairs_of(text: &str) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = text
        .split_whitespace()
        .filter_map(|t| {
            let (a, b) = t.split_once(',')?;
            Some((a.trim().parse::<f64>().ok()?.rem_euclid(1.0), b.trim().parse::<f64>().ok()?.rem_euclid(1.0)))
        })
        .collect();
    v.sort_by(|a, b| a.1.total_cmp(&b.1));
    v
}

/// The fraction round the source outline that goes to fraction `u` round the destination's,
/// evenly between the pairs.
fn source_fraction(pairs: &[(f64, f64)], u: f64, closed: bool) -> f64 {
    let n = pairs.len();
    if n == 1 {
        return (pairs[0].0 + (u - pairs[0].1)).rem_euclid(1.0);
    }
    if !closed {
        let ends: Vec<(f64, f64)> = std::iter::once((0.0, 0.0)).chain(pairs.iter().copied()).chain(std::iter::once((1.0, 1.0))).collect();
        let k = ends.iter().rposition(|p| p.1 <= u).unwrap_or(0).min(ends.len() - 2);
        let (a, b) = (ends[k], ends[k + 1]);
        let t = if b.1 > a.1 { (u - a.1) / (b.1 - a.1) } else { 0.0 };
        return (a.0 + (b.0 - a.0) * t).clamp(0.0, 1.0);
    }
    let k = pairs.iter().rposition(|p| p.1 <= u).unwrap_or(n - 1);
    let (a, b) = (pairs[k], pairs[(k + 1) % n]);
    let du = (b.1 - a.1).rem_euclid(1.0).max(1e-9);
    let t = (u - a.1).rem_euclid(1.0) / du;
    (a.0 + (b.0 - a.0).rem_euclid(1.0) * t).rem_euclid(1.0)
}

/// How many points round each outline Reshape matches.
const MATCHED: usize = 128;

fn reshape(ctx: &Ctx, mut b: Buf) -> Buf {
    let (Some((sm, sc)), Some((dm, dc))) = (mask_px(ctx, &b, "sourceMask"), mask_px(ctx, &b, "destinationMask")) else { return b };
    let k = ctx.num("percent", 100.0) / 100.0;
    if k == 0.0 {
        return b;
    }
    let bound = mask_px(ctx, &b, "boundaryMask").map(|m| m.0);
    let stiff = elastic(ctx.choice("elasticity"));
    let smooth = ctx.choice("interpolationMethod") == 2;
    let (s, d) = (even(&sm, sc, MATCHED), even(&dm, dc, MATCHED));
    let text = ctx.value_in(ctx.effect, "correspondencePoints").map(|v| v.as_text().to_string()).unwrap_or_default();
    let pairs = pairs_of(&text);
    // without pairs, the source outline turned round to fit the destination's best
    let turn = if pairs.is_empty() && sc && dc {
        let cost = |r: usize| (0..MATCHED).map(|i| (s[(i + r) % MATCHED][0] - d[i][0]).powi(2) + (s[(i + r) % MATCHED][1] - d[i][1]).powi(2)).sum::<f64>();
        (0..MATCHED).min_by(|a, c| cost(*a).total_cmp(&cost(*c))).unwrap_or(0)
    } else {
        0
    };
    let slen = length(&sm, sc);
    // the pulls are softened over about the gap between points, so the outline does not
    // show them one by one
    let gap2 = (length(&dm, dc) / MATCHED as f64).powi(2);
    // from each destination point back to its source point
    let back: Vec<[f64; 2]> = (0..MATCHED)
        .map(|i| {
            let q = if pairs.is_empty() { s[(i + turn) % MATCHED] } else { point_at(&sm, sc, source_fraction(&pairs, i as f64 / MATCHED as f64, sc && dc) * slen).0 };
            [q[0] - d[i][0], q[1] - d[i][1]]
        })
        .collect();
    b.img = remap(&b.img, false, |x, y| {
        if let Some(bd) = &bound
            && !inside(bd, [x, y])
        {
            return (x, y);
        }
        // the destination points' pulls, nearer ones counting much more
        let (mut wx, mut wy, mut ws, mut nearest) = (0.0, 0.0, 0.0, f64::INFINITY);
        for (p, m) in d.iter().zip(&back) {
            let d2 = (x - p[0]).powi(2) + (y - p[1]).powi(2);
            nearest = nearest.min(d2);
            let w = 1.0 / (d2 + gap2 + 1e-6).powf(stiff * 0.5);
            wx += w * m[0];
            wy += w * m[1];
            ws += w;
        }
        let mut fade = 1.0;
        if let Some(bd) = &bound {
            let db = distance(bd, true, [x, y]);
            fade = db / (db + nearest.sqrt()).max(1e-9);
            if smooth {
                fade = fade * fade * (3.0 - 2.0 * fade);
            }
        }
        let t = fade * k / ws;
        (x + wx * t, y + wy * t)
    });
    b
}

effect!(
    TWIRL_LEGACY_DEF,
    "twirlLegacy",
    "Twirl (Legacy)",
    "the old twirl: turning less and less in a straight line out to its edge",
    [angle("angle", "Angle", 0.0), pct("twirlRadius", "Twirl Radius", 30.0, 0.0, 100.0).doc("of half the layer's shorter side"), point("twirlCenter", "Twirl Center"),]
);
pub const TWIRL_LEGACY: Spec = Spec::new(&TWIRL_LEGACY_DEF, CATEGORY, twirl_legacy).points(&[("twirlCenter", [0.5, 0.5])]);

fn twirl_legacy(ctx: &Ctx, mut b: Buf) -> Buf {
    let turn = ctx.num("angle", 0.0).to_radians();
    if turn == 0.0 {
        return b;
    }
    let r = (ctx.num("twirlRadius", 30.0) / 100.0 * ctx.bounds.width().min(ctx.bounds.height()) * 0.5 * b.scale).max(1.0);
    let c = at(&b, ctx.point("twirlCenter"));
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let d = dx.hypot(dy);
        if d >= r {
            return (x, y);
        }
        let (s, co) = (-turn * (1.0 - d / r)).sin_cos();
        (c.0 + dx * co - dy * s, c.1 + dx * s + dy * co)
    });
    b
}

const WARP_STYLES: &[&str] = &["arc", "arcLower", "arcUpper", "arch", "bulge", "shellLower", "shellUpper", "flag", "wave", "fish", "rise", "fisheye", "inflate", "squeeze", "twist"];

effect!(
    WARP_DEF,
    "warp",
    "Warp",
    "the layer bent into one of fifteen shapes: arcs, a flag, a wave, a fish, a fisheye…",
    [
        choice("warpStyle", "Warp Style", WARP_STYLES, 0),
        choice("warpAxis", "Warp Axis", &["horizontal", "vertical"], 0),
        ranged("bend", "Bend", 0.0, -100.0, 100.0),
        ranged("horizontalDistortion", "Horizontal Distortion", 0.0, -100.0, 100.0),
        ranged("verticalDistortion", "Vertical Distortion", 0.0, -100.0, 100.0),
    ]
);
pub const WARP: Spec = Spec::new(&WARP_DEF, CATEGORY, warp);

/// Where `style` bent by `k` (-1 to 1) takes a point (u along the axis, v across it, both -1
/// to 1 over the layer).
fn warp_style(style: u32, k: f64, u: f64, v: f64) -> (f64, f64) {
    let bell = 1.0 - u * u;
    match style {
        0 => (u * (1.0 - 0.3 * k * v), v - k * bell),
        1 => (u, v - k * bell * (v + 1.0) * 0.5),
        2 => (u, v - k * bell * (1.0 - v) * 0.5),
        3 => (u, v - k * bell),
        4 => (u, v * (1.0 + k * bell)),
        5 => (u, v + k * bell * bell * (v + 1.0) * 0.5),
        6 => (u, v - k * bell * bell * (1.0 - v) * 0.5),
        7 => (u, v - 0.5 * k * (PI * u).sin()),
        8 => (u, v - 0.5 * k * (PI * u + 0.5 * PI * v).sin()),
        9 => (u, v * (1.0 + 0.5 * k * (PI * u).sin())),
        10 => (u, v - 0.5 * k * (0.5 * PI * u).sin()),
        11 => {
            // pushed out from the middle, less toward the rim, never folding over
            let r = u.hypot(v);
            if r >= 1.0 {
                (u, v)
            } else {
                let f = 1.0 + k * (1.0 - r);
                (u * f, v * f)
            }
        }
        12 => (u * (1.0 + 0.5 * k * (1.0 - v * v)), v * (1.0 + 0.5 * k * bell)),
        13 => (u * (1.0 - 0.5 * k * (1.0 - v * v)), v * (1.0 + 0.5 * k * bell)),
        _ => {
            let r = u.hypot(v);
            if r >= 1.0 {
                (u, v)
            } else {
                let (s, c) = (k * PI * (1.0 - r)).sin_cos();
                (u * c - v * s, u * s + v * c)
            }
        }
    }
}

fn warp(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = ctx.num("bend", 0.0) / 100.0;
    let (hd, vd) = (ctx.num("horizontalDistortion", 0.0) / 100.0, ctx.num("verticalDistortion", 0.0) / 100.0);
    if k == 0.0 && hd == 0.0 && vd == 0.0 {
        return b;
    }
    let style = ctx.choice("warpStyle");
    let vertical = ctx.choice("warpAxis") == 1;
    let r = ctx.bounds;
    let (hx, hy) = ((r.width() * b.scale * 0.5).max(1e-6), (r.height() * b.scale * 0.5).max(1e-6));
    if !ctx.adjustment {
        b.pad(((k.abs() + hd.abs() + vd.abs()) * 0.6 * hx.max(hy)).ceil().min(4096.0) as usize + 1);
    }
    let c = at(&b, r.center());
    let ahead = move |x: f64, y: f64| -> (f64, f64) {
        let (u0, v0) = ((x - c.0) / hx, (y - c.1) / hy);
        let (u, v) = if vertical { (v0, u0) } else { (u0, v0) };
        let (wu, wv) = warp_style(style, k, u, v);
        let (mut x1, mut y1) = if vertical { (wv, wu) } else { (wu, wv) };
        // bigger toward one side, as if seen at a slant
        y1 *= 1.0 + 0.5 * hd * x1;
        x1 *= 1.0 + 0.5 * vd * y1;
        (c.0 + x1 * hx, c.1 + y1 * hy)
    };
    b.img = remap(&b.img, false, |x, y| {
        // where the warp takes from, found step by step
        let (mut sx, mut sy) = (x, y);
        for _ in 0..12 {
            let (fx, fy) = ahead(sx, sy);
            let (rx, ry) = (fx - x, fy - y);
            if rx.abs() + ry.abs() < 1e-7 {
                break;
            }
            let e = 0.01;
            let ((ax, ay), (bx, by)) = (ahead(sx + e, sy), ahead(sx, sy + e));
            let (j00, j10, j01, j11) = ((ax - fx) / e, (ay - fy) / e, (bx - fx) / e, (by - fy) / e);
            let det = j00 * j11 - j01 * j10;
            if det.abs() < 1e-9 {
                return (-1e9, -1e9);
            }
            sx -= (j11 * rx - j01 * ry) / det;
            sy -= (-j10 * rx + j00 * ry) / det;
        }
        let (fx, fy) = ahead(sx, sy);
        if (fx - x).abs() + (fy - y).abs() > 0.5 { (-1e9, -1e9) } else { (sx, sy) }
    });
    b
}

effect!(
    UPSCALE_DEF,
    "detailPreservingUpscale",
    "Detail-preserving Upscale",
    "the layer made bigger with its edges and detail kept sharp",
    [
        ranged("scale", "Scale", 100.0, 100.0, 1000.0),
        pct("reduceNoise", "Reduce Noise", 0.0, 0.0, 100.0),
        pct("detail", "Detail", 20.0, 0.0, 100.0),
        choice("alpha", "Alpha", &["bilinear", "bicubic"], 1),
    ]
);
pub const UPSCALE: Spec = Spec::new(&UPSCALE_DEF, CATEGORY, upscale);

fn upscale(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = (ctx.num("scale", 100.0) / 100.0).clamp(1.0, 10.0);
    if (k - 1.0).abs() < 1e-9 {
        return b;
    }
    let detail = (ctx.num("detail", 20.0) / 100.0) as f32;
    let noise = ctx.num("reduceNoise", 0.0) / 100.0;
    let src = if noise > 0.0 { filter::gaussian(&b.img, noise * 1.5, noise * 1.5, Edge::Repeat) } else { b.img.clone() };
    let bilinear_alpha = ctx.choice("alpha") == 0;
    let (sw, sh) = (src.width as f64, src.height as f64);
    // smooth and sharp between pixels; the picture's own edge pixels go on, so its edges stay
    let up = |x: f64, y: f64| -> Px {
        let (x, y) = (x.clamp(0.5, sw - 0.5), y.clamp(0.5, sh - 0.5));
        let p = src.sample(Sampling::Bicubic, x, y);
        if !bilinear_alpha {
            return p;
        }
        let a = src.sample(Sampling::Bilinear, x, y)[3];
        let f = if p[3] > 1e-6 { a / p[3] } else { 0.0 };
        [p[0] * f, p[1] * f, p[2] * f, a]
    };
    let centre = b.pixel(ctx.bounds.center());
    let mut out = if ctx.adjustment {
        // in place, about the layer's middle
        let mut o = Image::new(src.width, src.height);
        map_px(&mut o, |x, y, _| {
            let (sx, sy) = (centre.x + (x as f64 + 0.5 - centre.x) / k, centre.y + (y as f64 + 0.5 - centre.y) / k);
            if sx < 0.0 || sy < 0.0 || sx > sw || sy > sh { [0.0; 4] } else { up(sx, sy) }
        });
        o
    } else {
        // a bigger buffer over more of the layer, the same pixels to a unit
        let (w, h) = (((sw * k).round() as usize).clamp(1, 16_384), ((sh * k).round() as usize).clamp(1, 16_384));
        let (kx, ky) = (w as f64 / sw, h as f64 / sh);
        let mut o = Image::new(w, h);
        map_px(&mut o, |x, y, _| up((x as f64 + 0.5) / kx, (y as f64 + 0.5) / ky));
        let c = ctx.bounds.center();
        b.origin = vec2(c.x - (c.x - b.origin.x) * kx, c.y - (c.y - b.origin.y) * ky);
        o
    };
    if detail > 0.0 {
        let soft = filter::gaussian(&out, 0.6 * k, 0.6 * k, Edge::Repeat);
        for (p, q) in out.px.iter_mut().zip(&soft.px) {
            let a = p[3];
            for c in 0..3 {
                p[c] = (p[c] + (p[c] - q[c]) * detail * 1.5).clamp(0.0, a.max(p[c]));
            }
        }
    }
    b.img = out;
    b
}

/// Every effect here.
pub const ALL: [Spec; 13] = [BENDER, BLOBBYLIZE, POWER_PIN, RIPPLE_PULSE, SLANT, SMUDGE, SPLIT, SPLIT2, SMEAR, RESHAPE, TWIRL_LEGACY, WARP, UPSCALE];

#[cfg(test)]
mod tests;
