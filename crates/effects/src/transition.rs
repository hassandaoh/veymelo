//! Transitions: the layer taken away as the completion goes from 0 to 100 — in blocks, by a
//! gradient, through an iris, along a line, round a point, in blinds, behind glass, in a grid, by
//! an image's brightness, between jaws, with a flash of light, in sweeping lines, stretched away,
//! twisted over, or warped into another layer.
//!
//! At 0 the layer is as it was. Angles go clockwise from up. Sizes are in layer units.

use std::f64::consts::{PI, TAU};

use veymelo_raster::{Image, Px, Sampling};

use crate::color::rgb3;
use crate::defs::{angle, choice, color, effect, flag, layer, pct, point, ranged};
use crate::distort::{rect_px, remap};
use crate::matte::soften;
use crate::util::{hsl, luma, map_px, unpremul};
use crate::{Buf, Ctx, Fit, Spec, Take};

const CATEGORY: &str = "transition";

fn mix4(a: Px, b: Px, t: f32) -> Px {
    [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How far along the transition is, 0 to 1.
fn done(ctx: &Ctx, id: &str) -> f64 {
    (ctx.num(id, 0.0) / 100.0).clamp(0.0, 1.0)
}

/// The layer's rectangle's corners in buffer pixels.
fn corners(ctx: &Ctx, b: &Buf) -> [(f64, f64); 4] {
    let (x0, y0, w, h) = rect_px(ctx, b);
    [(x0, y0), (x0 + w, y0), (x0, y0 + h), (x0 + w, y0 + h)]
}

/// The farthest of the layer's corners from `c` by `measure`.
fn reach(ctx: &Ctx, b: &Buf, c: (f64, f64), measure: impl Fn(f64, f64) -> f64) -> f64 {
    corners(ctx, b).iter().map(|(x, y)| measure(x - c.0, y - c.1)).fold(1e-6, f64::max)
}

effect!(
    BLOCK_DEF,
    "blockDissolve",
    "Block Dissolve",
    "the layer gone in random blocks",
    [
        pct("transitionCompletion", "Transition Completion", 0.0, 0.0, 100.0),
        ranged("blockWidth", "Block Width", 1.0, 1.0, 4000.0),
        ranged("blockHeight", "Block Height", 1.0, 1.0, 4000.0),
        ranged("feather", "Feather", 0.0, 0.0, 400.0),
        flag("softEdges", "Soft Edges (Best Quality)", true),
    ]
);
pub const BLOCK: Spec = Spec::new(&BLOCK_DEF, CATEGORY, block_dissolve);

fn block_dissolve(ctx: &Ctx, mut b: Buf) -> Buf {
    let d = done(ctx, "transitionCompletion") as f32;
    if d <= 0.0 {
        return b;
    }
    let (bw, bh) = (ctx.num("blockWidth", 1.0).max(1e-3), ctx.num("blockHeight", 1.0).max(1e-3));
    let feather = ctx.num("feather", 0.0) * b.scale;
    let seed = ctx.seed;
    let subs: &[f64] = if ctx.flag("softEdges") { &[0.125, 0.375, 0.625, 0.875] } else { &[0.5] };
    let (o, s) = (b.origin, b.scale);
    // a block (layer units) stays while its number is past the completion
    let stays = |x: f64, y: f64| {
        let (lx, ly) = (o.x + x / s, o.y + y / s);
        crate::noise::unit(crate::noise::mix(&[seed, (lx / bw).floor() as i64 as u64, (ly / bh).floor() as i64 as u64])) as f32 >= d
    };
    let (w, h) = (b.img.width, b.img.height);
    let n = (subs.len() * subs.len()) as f32;
    let mut keep: Vec<f32> = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f64, (i / w) as f64);
            subs.iter().flat_map(|sy| subs.iter().map(move |sx| (*sx, *sy))).filter(|(sx, sy)| stays(x + sx, y + sy)).count() as f32 / n
        })
        .collect();
    if feather > 0.0 {
        keep = soften(&keep, w, h, feather * 0.5);
    }
    for (p, k) in b.img.px.iter_mut().zip(&keep) {
        *p = p.map(|v| v * k);
    }
    b
}

effect!(
    GRADIENT_WIPE_DEF,
    "gradientWipe",
    "Gradient Wipe",
    "the layer gone darkest parts first, by a gradient layer's brightness (or its own)",
    [
        pct("transitionCompletion", "Transition Completion", 0.0, 0.0, 100.0),
        pct("transitionSoftness", "Transition Softness", 0.0, 0.0, 100.0),
        layer("gradientLayer", "Gradient Layer").doc("none: this layer"),
        choice("gradientPlacement", "Gradient Placement", &["tileGradient", "centerGradient", "stretchGradientToFit"], 2),
        flag("invertGradient", "Invert Gradient", false),
    ]
);
pub const GRADIENT_WIPE: Spec = Spec::new(&GRADIENT_WIPE_DEF, CATEGORY, gradient_wipe);

fn gradient_wipe(ctx: &Ctx, mut b: Buf) -> Buf {
    let d = done(ctx, "transitionCompletion") as f32;
    if d <= 0.0 {
        return b;
    }
    let soft = (ctx.num("transitionSoftness", 0.0) / 100.0) as f32;
    let flip = ctx.flag("invertGradient");
    let fit = [Fit::Tile, Fit::Centre, Fit::Stretch][ctx.choice("gradientPlacement").min(2) as usize];
    let grad = ctx.layer_placed("gradientLayer", &b, Take::Source, fit).map_or_else(|| b.img.clone(), |(img, _)| img);
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let mut t = luma(unpremul(grad.px[y * w + x]).0).clamp(0.0, 1.0);
        if flip {
            t = 1.0 - t;
        }
        let k = if d >= 1.0 {
            0.0
        } else if soft > 0.0 {
            ((t - (d * (1.0 + soft) - soft)) / soft).clamp(0.0, 1.0)
        } else if t >= d {
            1.0
        } else {
            0.0
        };
        p.map(|v| v * k)
    });
    b
}

effect!(
    IRIS_DEF,
    "irisWipe",
    "Iris Wipe",
    "only what is inside a polygon (or star) round a point kept: open it from 0 to wipe the layer on",
    [
        point("irisCenter", "Iris Center"),
        ranged("irisPoints", "Iris Points", 6.0, 6.0, 32.0),
        ranged("outerRadius", "Outer Radius", 0.0, 0.0, 32_000.0),
        flag("useInnerRadius", "Use Inner Radius", false).doc("a star: its points at the outer radius, between them at the inner"),
        ranged("innerRadius", "Inner Radius", 0.0, 0.0, 32_000.0),
        angle("rotation", "Rotation", 0.0),
        ranged("feather", "Feather", 0.0, 0.0, 1000.0),
    ]
);
pub const IRIS: Spec = Spec::new(&IRIS_DEF, CATEGORY, iris_wipe).points(&[("irisCenter", [0.5, 0.5])]);

fn iris_wipe(ctx: &Ctx, mut b: Buf) -> Buf {
    let outer = ctx.num("outerRadius", 0.0).max(0.0) * b.scale;
    if outer <= 0.0 {
        return b;
    }
    let n = ctx.num("irisPoints", 6.0).round().clamp(3.0, 64.0) as usize;
    let star = ctx.flag("useInnerRadius");
    let inner = if star { ctx.num("innerRadius", 0.0).max(0.0) * b.scale } else { outer };
    let turn = ctx.num("rotation", 0.0).to_radians();
    let feather = (ctx.num("feather", 0.0) * b.scale).max(1.0);
    let c = b.pixel(ctx.point("irisCenter"));
    let m = if star { n * 2 } else { n };
    let step = TAU / m as f64;
    let verts: Vec<(f64, f64)> = (0..m)
        .map(|i| {
            let r = if star && i % 2 == 1 { inner } else { outer };
            let a = turn + i as f64 * step;
            (r * a.sin(), -r * a.cos())
        })
        .collect();
    let cross = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;
    map_px(&mut b.img, |x, y, p| {
        let d = (x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
        let r = d.0.hypot(d.1);
        // how far out the polygon's edge is this way
        let edge = if r < 1e-9 {
            outer.min(inner)
        } else {
            let a = (d.0.atan2(-d.1) - turn).rem_euclid(TAU);
            let i = ((a / step).floor() as usize).min(m - 1);
            let (vi, vj) = (verts[i], verts[(i + 1) % m]);
            let e = (vj.0 - vi.0, vj.1 - vi.1);
            let den = cross((d.0 / r, d.1 / r), e);
            if den.abs() < 1e-12 { outer } else { cross(vi, e) / den }
        };
        let k = ((edge - r) / feather + 0.5).clamp(0.0, 1.0) as f32;
        p.map(|v| v * k)
    });
    b
}

effect!(
    LINEAR_WIPE_DEF,
    "linearWipe",
    "Linear Wipe",
    "the layer wiped away in a straight line",
    [pct("transitionCompletion", "Transition Completion", 0.0, 0.0, 100.0), angle("wipeAngle", "Wipe Angle", 90.0).doc("which way the wipe goes (90: left to right)"), ranged("feather", "Feather", 0.0, 0.0, 32_000.0),]
);
pub const LINEAR_WIPE: Spec = Spec::new(&LINEAR_WIPE_DEF, CATEGORY, linear_wipe);

fn linear_wipe(ctx: &Ctx, mut b: Buf) -> Buf {
    let d = done(ctx, "transitionCompletion");
    if d <= 0.0 {
        return b;
    }
    let a = ctx.num("wipeAngle", 90.0).to_radians();
    let feather = (ctx.num("feather", 0.0) * b.scale).max(1.0);
    let (dx, dy) = (a.sin(), -a.cos());
    let along: Vec<f64> = corners(ctx, &b).iter().map(|(x, y)| x * dx + y * dy).collect();
    let (lo, hi) = (along.iter().cloned().fold(f64::INFINITY, f64::min), along.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
    // the edge runs from before the first corner to past the last
    let edge = lo - feather + (hi - lo + 2.0 * feather) * d;
    map_px(&mut b.img, |x, y, p| {
        let k = (((x as f64 + 0.5) * dx + (y as f64 + 0.5) * dy - edge) / feather + 0.5).clamp(0.0, 1.0) as f32;
        p.map(|v| v * k)
    });
    b
}

effect!(
    RADIAL_WIPE_DEF,
    "radialWipe",
    "Radial Wipe",
    "the layer wiped away round a point, like a clock's hand",
    [
        pct("transitionCompletion", "Transition Completion", 0.0, 0.0, 100.0),
        angle("startAngle", "Start Angle", 0.0),
        point("wipeCenter", "Wipe Center"),
        choice("wipe", "Wipe", &["clockwise", "counterclockwise", "both"], 0),
        ranged("feather", "Feather", 0.0, 0.0, 360.0),
    ]
);
pub const RADIAL_WIPE: Spec = Spec::new(&RADIAL_WIPE_DEF, CATEGORY, radial_wipe).points(&[("wipeCenter", [0.5, 0.5])]);

fn radial_wipe(ctx: &Ctx, mut b: Buf) -> Buf {
    let d = done(ctx, "transitionCompletion");
    if d <= 0.0 {
        return b;
    }
    let start = ctx.num("startAngle", 0.0);
    let c = b.pixel(ctx.point("wipeCenter"));
    let way = ctx.choice("wipe");
    // the soft edge as wide as asked, in pixels, at any distance
    let feather = (ctx.num("feather", 0.0) * b.scale).max(1.0);
    map_px(&mut b.img, |x, y, p| {
        let (dx, dy) = (x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
        let r = dx.hypot(dy).max(0.5);
        let a = (dx.atan2(-dy).to_degrees() - start).rem_euclid(360.0);
        let (a, edge) = match way {
            1 => (360.0 - a, d * 360.0),
            2 => (a.min(360.0 - a), d * 180.0),
            _ => (a, d * 360.0),
        };
        let k = if d >= 1.0 { 0.0 } else { (((a - edge).to_radians() * r) / feather + 0.5).clamp(0.0, 1.0) as f32 };
        p.map(|v| v * k)
    });
    b
}

effect!(
    VENETIAN_DEF,
    "venetianBlinds",
    "Venetian Blinds",
    "the layer gone in stripes, like blinds closing",
    [
        pct("transitionCompletion", "Transition Completion", 0.0, 0.0, 100.0),
        angle("direction", "Direction", 90.0),
        ranged("width", "Width", 30.0, 2.0, 1000.0),
        ranged("feather", "Feather", 0.0, 0.0, 100.0),
    ]
);
pub const VENETIAN: Spec = Spec::new(&VENETIAN_DEF, CATEGORY, venetian);

fn venetian(ctx: &Ctx, mut b: Buf) -> Buf {
    let d = done(ctx, "transitionCompletion");
    if d <= 0.0 {
        return b;
    }
    let a = ctx.num("direction", 90.0).to_radians();
    let width = ctx.num("width", 30.0).max(1.0);
    let feather = (ctx.num("feather", 0.0) * b.scale).max(1.0);
    let (dx, dy) = (a.cos(), a.sin());
    let (o, s) = (b.origin, b.scale);
    map_px(&mut b.img, |x, y, p| {
        // across the stripes, in layer units
        let (lx, ly) = (o.x + (x as f64 + 0.5) / s, o.y + (y as f64 + 0.5) / s);
        let u = (lx * dx + ly * dy).rem_euclid(width);
        let k = if d >= 1.0 { 0.0 } else { (((u - d * width) * s) / feather + 0.5).clamp(0.0, 1.0) as f32 };
        p.map(|v| v * k)
    });
    b
}

/// A layer setting's layer laid over this one at its own size, if one is named.
fn revealed(ctx: &Ctx, b: &Buf, id: &str) -> Option<Image> {
    ctx.layer_over(id, b, Take::All, false)
}

const GRADIENT_FROM: &[&str] = &["red", "green", "blue", "alpha", "luminance", "lightness", "saturation"];

/// A channel of a picture, weighed by its alpha, as a plane.
fn plane_of(img: &Image, ch: u32) -> Vec<f32> {
    img.px
        .iter()
        .map(|p| {
            let (c, a) = unpremul(*p);
            let (_, s, l) = hsl(c);
            a * match ch {
                0 => c[0],
                1 => c[1],
                2 => c[2],
                3 => 1.0,
                5 => l,
                6 => s,
                _ => luma(c),
            }
        })
        .collect()
}

/// How much of a pixel with gradient value `g` has gone at completion `c`, over a soft band
/// `w` wide: none at 0, all at 1.
fn gone(g: f32, c: f32, w: f32) -> f32 {
    let w = w.max(1e-3);
    let edge = c * (1.0 + 2.0 * w) - w;
    1.0 - smoothstep(edge - w, edge + w, g)
}

/// A plane's slope at a pixel.
fn slope(p: &[f32], w: usize, h: usize, x: usize, y: usize) -> (f32, f32) {
    let at = |x: i64, y: i64| p[(y.clamp(0, h as i64 - 1) as usize) * w + x.clamp(0, w as i64 - 1) as usize];
    let (xi, yi) = (x as i64, y as i64);
    ((at(xi + 1, yi) - at(xi - 1, yi)) * 0.5, (at(xi, yi + 1) - at(xi, yi - 1)) * 0.5)
}

effect!(
    GLASS_WIPE_DEF,
    "glassWipe",
    "Glass Wipe",
    "the layer melting away by a gradient, the edge bending the picture like glass, another layer showing through",
    [
        pct("completion", "Completion", 0.0, 0.0, 100.0),
        layer("layerToReveal", "Layer to Reveal"),
        layer("gradientLayer", "Gradient Layer").doc("none: this layer"),
        pct("softness", "Softness", 10.0, 0.0, 100.0),
        ranged("displacementAmount", "Displacement Amount", 10.0, -100.0, 100.0),
    ]
);
pub const GLASS_WIPE: Spec = Spec::new(&GLASS_WIPE_DEF, CATEGORY, glass_wipe);

fn glass_wipe(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = done(ctx, "completion") as f32;
    if c <= 0.0 {
        return b;
    }
    let (w, h) = (b.img.width, b.img.height);
    let gimg = ctx.layer_over("gradientLayer", &b, Take::All, true).unwrap_or_else(|| b.img.clone());
    let mut g = plane_of(&gimg, 4);
    let s = ctx.num("softness", 10.0) * b.scale * 0.2;
    if s > 0.05 {
        g = soften(&g, w, h, s);
    }
    let band_w = (ctx.num("softness", 10.0) / 100.0) as f32 * 0.5 + 0.01;
    let push = (ctx.num("displacementAmount", 10.0) * b.scale * 40.0) as f32;
    let shown = revealed(ctx, &b, "layerToReveal");
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let t = gone(g[y * w + x], c, band_w);
        let band = 4.0 * t * (1.0 - t);
        let (gx, gy) = slope(&g, w, h, x, y);
        let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
        let (ox, oy) = ((gx * push * band) as f64, (gy * push * band) as f64);
        let a = if band > 1e-4 { src.sample(Sampling::Bilinear, fx + ox, fy + oy) } else { src.px[y * w + x] };
        let r = match &shown {
            Some(r) if band > 1e-4 => r.sample(Sampling::Bilinear, fx - ox, fy - oy),
            Some(r) => r.px[y * w + x],
            None => [0.0; 4],
        };
        mix4(a, r, t)
    });
    b
}

effect!(
    GRID_WIPE_DEF,
    "gridWipe",
    "Grid Wipe",
    "the layer gone in a grid of shrinking squares, spreading from a point",
    [
        pct("completion", "Completion", 0.0, 0.0, 100.0),
        point("center", "Center"),
        angle("rotation", "Rotation", 0.0),
        ranged("border", "Border", 0.0, 0.0, 100.0),
        ranged("tiles", "Tiles", 10.0, 1.0, 500.0),
        choice("shape", "Shape", &["doors", "radial", "rectangular"], 1),
        flag("reverseTransition", "Reverse Transition", false),
    ]
);
pub const GRID_WIPE: Spec = Spec::new(&GRID_WIPE_DEF, CATEGORY, grid_wipe).points(&[("center", [0.5, 0.5])]);

fn grid_wipe(ctx: &Ctx, mut b: Buf) -> Buf {
    let d = done(ctx, "completion");
    if d <= 0.0 {
        return b;
    }
    let c = b.pixel(ctx.point("center"));
    let (s, co) = ctx.num("rotation", 0.0).to_radians().sin_cos();
    let border = (ctx.num("border", 0.0) * b.scale).max(1.0);
    let tiles = ctx.num("tiles", 10.0).round().clamp(1.0, 500.0);
    let shape = ctx.choice("shape");
    let reverse = ctx.flag("reverseTransition");
    let (_, _, w, _) = rect_px(ctx, &b);
    let cell = (w / tiles).max(1.0);
    let measure = |dx: f64, dy: f64| match shape {
        0 => dx.abs(),
        2 => dx.abs().max(dy.abs()),
        _ => dx.hypot(dy),
    };
    let far = reach(ctx, &b, (c.x, c.y), |dx, dy| measure(dx * co + dy * s, -dx * s + dy * co));
    let spread = 0.25;
    map_px(&mut b.img, |x, y, p| {
        let (dx, dy) = (x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
        let (qx, qy) = (dx * co + dy * s, -dx * s + dy * co);
        let (cx, cy) = (((qx / cell).floor() + 0.5) * cell, ((qy / cell).floor() + 0.5) * cell);
        let mut order = (measure(cx, cy) / far).clamp(0.0, 1.0);
        if reverse {
            order = 1.0 - order;
        }
        // how far this tile has shrunk
        let t = ((d * (1.0 + spread) - order) / spread).clamp(0.0, 1.0);
        if t <= 0.0 {
            return p;
        }
        let half = cell * 0.5 * (1.0 - t);
        let m = (qx - cx).abs().max((qy - cy).abs());
        let k = ((half - m) / border + 0.5).clamp(0.0, 1.0) as f32;
        p.map(|v| v * k)
    });
    b
}

effect!(
    IMAGE_WIPE_DEF,
    "imageWipe",
    "Image Wipe",
    "the layer gone by a channel of another layer (or its own), its darkest parts first",
    [
        pct("completion", "Completion", 0.0, 0.0, 100.0),
        pct("borderSoftness", "Border Softness", 0.0, 0.0, 100.0),
        flag("autoSoftness", "Auto Softness", true),
        layer("layer", "Layer").doc("none: this layer"),
        choice("property", "Property", GRADIENT_FROM, 4),
        ranged("blur", "Blur", 0.0, 0.0, 200.0),
        flag("inverseGradient", "Inverse Gradient", false),
    ]
);
pub const IMAGE_WIPE: Spec = Spec::new(&IMAGE_WIPE_DEF, CATEGORY, image_wipe);

fn image_wipe(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = done(ctx, "completion") as f32;
    if c <= 0.0 {
        return b;
    }
    let (w, h) = (b.img.width, b.img.height);
    let gimg = ctx.layer_over("layer", &b, Take::All, true).unwrap_or_else(|| b.img.clone());
    let mut g = plane_of(&gimg, ctx.choice("property"));
    let blur = ctx.num("blur", 0.0) * b.scale * 0.5;
    if blur > 0.05 {
        g = soften(&g, w, h, blur);
    }
    if ctx.flag("inverseGradient") {
        for v in &mut g {
            *v = 1.0 - *v;
        }
    }
    let mut soft = (ctx.num("borderSoftness", 0.0) / 100.0) as f32 * 0.5;
    if ctx.flag("autoSoftness") {
        soft = soft.max(0.02);
    }
    for (p, gv) in b.img.px.iter_mut().zip(&g) {
        let k = 1.0 - gone(*gv, c, soft);
        *p = p.map(|v| v * k);
    }
    b
}

effect!(
    JAWS_DEF,
    "jaws",
    "Jaws",
    "the layer split along a toothed line, the halves pulled apart",
    [
        pct("completion", "Completion", 0.0, 0.0, 100.0),
        point("center", "Center"),
        angle("direction", "Direction", 0.0),
        pct("height", "Height", 50.0, 0.0, 100.0).doc("how deep the teeth are, of their width"),
        ranged("width", "Width", 50.0, 1.0, 1000.0),
        choice("shape", "Shape", &["spikes", "roboJaw", "block", "waves"], 0),
    ]
);
pub const JAWS: Spec = Spec::new(&JAWS_DEF, CATEGORY, jaws).points(&[("center", [0.5, 0.5])]);

fn jaws(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = done(ctx, "completion");
    if c <= 0.0 {
        return b;
    }
    let p0 = b.pixel(ctx.point("center"));
    let (sn, cs) = ctx.num("direction", 0.0).to_radians().sin_cos();
    let width = (ctx.num("width", 50.0) * b.scale).max(1.0);
    let amp = ctx.num("height", 50.0) / 100.0 * width * 0.5;
    let shape = ctx.choice("shape");
    // far enough at the end that both halves, teeth and all, have left
    let far = reach(ctx, &b, (p0.x, p0.y), |dx, dy| (-dx * sn + dy * cs).abs());
    let d = c * (far + 2.0 * amp + 2.0);
    let tooth = move |u: f64| -> f64 {
        let ph = (u / width).rem_euclid(1.0);
        match shape {
            0 => amp * (1.0 - 4.0 * (ph - 0.5).abs()),
            1 => amp * ((1.0 - 4.0 * (ph - 0.5).abs()) * 2.0).clamp(-1.0, 1.0),
            2 => {
                if ph < 0.5 {
                    amp
                } else {
                    -amp
                }
            }
            _ => amp * (ph * TAU).sin(),
        }
    };
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (dx, dy) = (x as f64 + 0.5 - p0.x, y as f64 + 0.5 - p0.y);
        // along the cut, and across it
        let (u, v) = (dx * cs + dy * sn, -dx * sn + dy * cs);
        let f = tooth(u);
        let from = |vs: f64| src.sample(Sampling::Bilinear, u * cs - vs * sn + p0.x, u * sn + vs * cs + p0.y);
        // the top half moved up by d, the bottom down, each with a soft edge at its teeth
        let top = ((f - (v + d)) + 0.5).clamp(0.0, 1.0) as f32;
        let bottom = (((v - d) - f) + 0.5).clamp(0.0, 1.0) as f32;
        let mut out = [0.0; 4];
        if top > 0.0 {
            out = from(v + d).map(|q| q * top);
        }
        if bottom > 0.0 {
            let q = from(v - d).map(|q| q * bottom);
            out = [0, 1, 2, 3].map(|i| out[i] + q[i]);
        }
        out
    });
    b
}

effect!(
    LIGHT_WIPE_DEF,
    "lightWipe",
    "Light Wipe",
    "the layer wiped away behind a bright edge of light",
    [
        pct("completion", "Completion", 0.0, 0.0, 100.0),
        point("center", "Center"),
        ranged("intensity", "Intensity", 100.0, 0.0, 1000.0),
        choice("shape", "Shape", &["doors", "round", "square"], 1),
        angle("direction", "Direction", 0.0),
        flag("colorFromSource", "Color from Source", false),
        color("color", "Color", 1.0, 1.0, 1.0),
        flag("reverseTransition", "Reverse Transition", false),
    ]
);
pub const LIGHT_WIPE: Spec = Spec::new(&LIGHT_WIPE_DEF, CATEGORY, light_wipe).points(&[("center", [0.5, 0.5])]);

fn light_wipe(ctx: &Ctx, mut b: Buf) -> Buf {
    let d = done(ctx, "completion");
    if d <= 0.0 {
        return b;
    }
    let c = b.pixel(ctx.point("center"));
    let strength = (ctx.num("intensity", 100.0) / 100.0) as f32;
    let shape = ctx.choice("shape");
    let (s, co) = ctx.num("direction", 0.0).to_radians().sin_cos();
    let own = ctx.flag("colorFromSource");
    let col = rgb3(ctx.color("color"));
    let reverse = ctx.flag("reverseTransition");
    let measure = move |dx: f64, dy: f64| {
        let (qx, qy) = (dx * co + dy * s, -dx * s + dy * co);
        match shape {
            0 => qx.abs(),
            2 => qx.abs().max(qy.abs()),
            _ => qx.hypot(qy),
        }
    };
    let far = reach(ctx, &b, (c.x, c.y), measure);
    let edge = if reverse { (1.0 - d) * far } else { d * far };
    let band = (far * 0.06).max(2.0);
    // the light comes up at the start and goes at the end
    let ramp = (d * 20.0).min(1.0) as f32 * ((1.0 - d) * 20.0).min(1.0) as f32;
    map_px(&mut b.img, |x, y, p| {
        let m = measure(x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
        let cover = (if reverse { edge - m } else { m - edge } + 0.5).clamp(0.0, 1.0) as f32;
        if cover <= 0.0 {
            return [0.0; 4];
        }
        let g = (-((m - edge) / band).powi(2)).exp() as f32 * strength * ramp;
        let (sc, a) = unpremul(p);
        let lc = if own { sc } else { col };
        [p[0] + lc[0] * g * a, p[1] + lc[1] * g * a, p[2] + lc[2] * g * a, p[3]].map(|v| v * cover)
    });
    b
}

effect!(
    LINE_SWEEP_DEF,
    "lineSweep",
    "Line Sweep",
    "the layer swept away in strips, each a little behind the last",
    [
        pct("completion", "Completion", 0.0, 0.0, 100.0),
        angle("direction", "Direction", 0.0),
        ranged("thickness", "Thickness", 20.0, 1.0, 500.0),
        ranged("slant", "Slant", 20.0, -500.0, 500.0),
        flag("flipDirection", "Flip Direction", false),
    ]
);
pub const LINE_SWEEP: Spec = Spec::new(&LINE_SWEEP_DEF, CATEGORY, line_sweep);

fn line_sweep(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = done(ctx, "completion");
    if c <= 0.0 {
        return b;
    }
    let s = b.scale;
    let (sn, cs) = ctx.num("direction", 0.0).to_radians().sin_cos();
    let thick = (ctx.num("thickness", 20.0) * s).max(1.0);
    let slant = ctx.num("slant", 20.0) * s;
    let flip = ctx.flag("flipDirection");
    let along = |x: f64, y: f64| (x * cs + y * sn, -x * sn + y * cs);
    let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for (x, y) in corners(ctx, &b) {
        let (u, v) = along(x, y);
        (u0, u1, v0, v1) = (u0.min(u), u1.max(u), v0.min(v), v1.max(v));
    }
    let strips = ((v1 - v0) / thick).ceil().max(1.0);
    let total = (u1 - u0) + slant.abs() * strips + 1.0;
    map_px(&mut b.img, |x, y, p| {
        let (u, v) = along(x as f64 + 0.5, y as f64 + 0.5);
        let k = ((v - v0) / thick).floor();
        let k = if flip { strips - 1.0 - k } else { k };
        let late = if slant >= 0.0 { k } else { strips - 1.0 - k };
        let front = u0 + c * total - slant.abs() * late;
        let keep = (u - front + 0.5).clamp(0.0, 1.0) as f32;
        p.map(|q| q * keep)
    });
    b
}

effect!(
    RADIAL_SCALE_DEF,
    "radialScaleWipe",
    "Radial Scale Wipe",
    "a hole opening at a point, the picture stretched away round it",
    [pct("completion", "Completion", 0.0, 0.0, 100.0), point("center", "Center"), flag("reverseTransition", "Reverse Transition", false),]
);
pub const RADIAL_SCALE: Spec = Spec::new(&RADIAL_SCALE_DEF, CATEGORY, radial_scale_wipe).points(&[("center", [0.5, 0.5])]);

fn radial_scale_wipe(ctx: &Ctx, mut b: Buf) -> Buf {
    let d = done(ctx, "completion");
    if d <= 0.0 {
        return b;
    }
    let c = b.pixel(ctx.point("center"));
    let reverse = ctx.flag("reverseTransition");
    let far = reach(ctx, &b, (c.x, c.y), f64::hypot);
    let hole = d * far;
    let src = b.img.clone();
    b.img = remap(&src, false, |x, y| {
        let (dx, dy) = (x - c.x, y - c.y);
        let r = dx.hypot(dy);
        let gone = (-1e9, -1e9);
        if reverse {
            // the whole picture shrinking into a disc
            let lim = (1.0 - d) * far;
            if r >= lim || lim <= 1e-9 {
                return gone;
            }
            let k = far / lim;
            (c.x + dx * k, c.y + dy * k)
        } else {
            if r <= hole || far - hole <= 1e-9 {
                return gone;
            }
            let from = (r - hole) * far / (far - hole);
            (c.x + dx / r * from, c.y + dy / r * from)
        }
    });
    b
}

effect!(
    SCALE_WIPE_DEF,
    "scaleWipe",
    "Scale Wipe",
    "the picture past a line stretched out one way",
    [ranged("stretch", "Stretch", 0.0, 0.0, 1000.0), point("center", "Center"), angle("direction", "Direction", 90.0),]
);
pub const SCALE_WIPE: Spec = Spec::new(&SCALE_WIPE_DEF, CATEGORY, scale_wipe).points(&[("center", [0.5, 0.5])]);

fn scale_wipe(ctx: &Ctx, mut b: Buf) -> Buf {
    let stretch = ctx.num("stretch", 0.0).max(0.0);
    if stretch <= 0.0 {
        return b;
    }
    let c = b.pixel(ctx.point("center"));
    let a = ctx.num("direction", 90.0).to_radians();
    let dir = (a.sin(), -a.cos());
    let src = b.img.clone();
    b.img = remap(&src, true, |x, y| {
        let u = (x - c.x) * dir.0 + (y - c.y) * dir.1;
        if u <= 0.0 {
            return (x, y);
        }
        let back = u - u / (1.0 + stretch);
        (x - dir.0 * back, y - dir.1 * back)
    });
    b
}

effect!(
    TWISTER_DEF,
    "twister",
    "Twister",
    "the layer twisted over like a ribbon about an axis, its back (or another layer) coming round",
    [
        pct("completion", "Completion", 0.0, 0.0, 100.0),
        layer("backside", "Backside").doc("none: this layer, from behind"),
        flag("shading", "Shading", true),
        point("center", "Center"),
        angle("axis", "Axis", 45.0),
    ]
);
pub const TWISTER: Spec = Spec::new(&TWISTER_DEF, CATEGORY, twister).points(&[("center", [0.5, 0.5])]);

fn twister(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = done(ctx, "completion");
    if c <= 0.0 {
        return b;
    }
    let p0 = b.pixel(ctx.point("center"));
    let (sn, cs) = ctx.num("axis", 45.0).to_radians().sin_cos();
    let shading = ctx.flag("shading");
    let half = reach(ctx, &b, (p0.x, p0.y), f64::hypot);
    let src = b.img.clone();
    let back = revealed(ctx, &b, "backside");
    map_px(&mut b.img, |x, y, _| {
        let (dx, dy) = (x as f64 + 0.5 - p0.x, y as f64 + 0.5 - p0.y);
        let (u, v) = (dx * cs + dy * sn, -dx * sn + dy * cs);
        // the turn travels along the axis
        let along = ((u / half) * 0.5 + 0.5).clamp(0.0, 1.0);
        let k = (PI * (c * 2.0 - along).clamp(0.0, 1.0)).cos();
        if k.abs() < 1e-4 {
            return [0.0; 4];
        }
        let vs = v / k;
        let (sx, sy) = (u * cs - vs * sn + p0.x, u * sn + vs * cs + p0.y);
        let mut o: Px = if k > 0.0 {
            src.sample(Sampling::Bilinear, sx, sy)
        } else {
            match &back {
                // a back layer reads the right way round once fully over
                Some(bk) => bk.sample(Sampling::Bilinear, u * cs + vs * sn + p0.x, u * sn - vs * cs + p0.y),
                None => src.sample(Sampling::Bilinear, sx, sy),
            }
        };
        if shading {
            let l = (0.4 + 0.6 * k.abs()) as f32;
            o = [o[0] * l, o[1] * l, o[2] * l, o[3]];
        }
        o
    });
    b
}

effect!(
    WARP_REVEAL_DEF,
    "warpReveal",
    "Warp Reveal",
    "the layer warped away by a reactor layer's brightness, another warping in to take its place",
    [
        pct("completion", "Completion", 0.0, 0.0, 100.0),
        layer("layerToReveal", "Layer to Reveal"),
        layer("reactorLayer", "Reactor Layer").doc("none: this layer"),
        choice("reactor", "Reactor", &["brightness", "contrastDifferences", "localDifferences"], 0),
        ranged("smoothness", "Smoothness", 10.0, 0.0, 200.0),
        ranged("warpAmount", "Warp Amount", 50.0, -1000.0, 1000.0),
        choice("warpDirection", "Warp Direction", &["joint", "opposing", "twisting"], 0),
        pct("blendSpan", "Blend Span", 10.0, 0.0, 100.0),
    ]
);
pub const WARP_REVEAL: Spec = Spec::new(&WARP_REVEAL_DEF, CATEGORY, warp_reveal);

fn warp_reveal(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = done(ctx, "completion") as f32;
    if c <= 0.0 {
        return b;
    }
    let (w, h) = (b.img.width, b.img.height);
    let rimg = ctx.layer_over("reactorLayer", &b, Take::All, true).unwrap_or_else(|| b.img.clone());
    let mut m = plane_of(&rimg, 4);
    let sm = ctx.num("smoothness", 10.0) * b.scale * 0.5;
    if sm > 0.05 {
        m = soften(&m, w, h, sm);
    }
    m = match ctx.choice("reactor") {
        // how far from what is round it
        1 => {
            let mean = soften(&m, w, h, 8.0 * b.scale);
            m.iter().zip(&mean).map(|(a, b)| ((a - b).abs() * 4.0).min(1.0)).collect()
        }
        // how steep
        2 => (0..w * h)
            .map(|i| {
                let (gx, gy) = slope(&m, w, h, i % w, i / w);
                (gx.hypot(gy) * 8.0).min(1.0)
            })
            .collect(),
        _ => m,
    };
    let amount = (ctx.num("warpAmount", 50.0) * b.scale) as f32 * 20.0;
    let way = ctx.choice("warpDirection");
    let span = (ctx.num("blendSpan", 10.0) / 100.0) as f32 * 0.5;
    let shown = revealed(ctx, &b, "layerToReveal");
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let q = gone(m[y * w + x], c, span);
        let (gx, gy) = slope(&m, w, h, x, y);
        let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
        let (ox, oy) = ((gx * amount * q) as f64, (gy * amount * q) as f64);
        let (ix, iy) = ((gx * amount * (1.0 - q)) as f64, (gy * amount * (1.0 - q)) as f64);
        let (ix, iy) = match way {
            1 => (-ix, -iy),
            2 => (-iy, ix),
            _ => (ix, iy),
        };
        let out = if q < 1.0 { src.sample(Sampling::Bilinear, fx + ox, fy + oy) } else { [0.0; 4] };
        let inc = match &shown {
            Some(r) if q > 0.0 => r.sample(Sampling::Bilinear, fx + ix, fy + iy),
            _ => [0.0; 4],
        };
        mix4(out, inc, q)
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 16] = [BLOCK, GRADIENT_WIPE, IRIS, LINEAR_WIPE, RADIAL_WIPE, VENETIAN, GLASS_WIPE, GRID_WIPE, IMAGE_WIPE, JAWS, LIGHT_WIPE, LINE_SWEEP, RADIAL_SCALE, SCALE_WIPE, TWISTER, WARP_REVEAL];

#[cfg(test)]
mod tests;
