//! Generators, part one: what draws something new on the layer: ramps (Gradient Ramp,
//! 4-Color Gradient), patterns (Checkerboard, Grid, Cell Pattern), shapes (Circle, Ellipse,
//! Beam), and light (Lens Flare, Light Rays, Light Burst, Light Sweep).
//!
//! Sizes and places are in layer units, so a preview draws the same as the full picture.

use std::f64::consts::PI;

use veymelo_color::BlendMode;
use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Px, Sampling};

use crate::color::rgb3;
use crate::defs::{angle, choice, color, effect, flag, num, pct, point, px, ranged};
use crate::util::{luma, map_px, mix3, premul, unpremul};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "generate";

/// How a generated pattern is put with the layer: alone (none), or by a blend mode.
pub(crate) const MODES: [&str; 20] = [
    "none", "normal", "add", "multiply", "screen", "overlay", "softLight", "hardLight", "colorDodge", "colorBurn", "darken", "lighten", "difference", "exclusion", "hue", "saturation",
    "color", "luminosity", "stencilAlpha", "silhouetteAlpha",
];
pub(crate) const MODE_OF: [BlendMode; 20] = [
    BlendMode::Normal,
    BlendMode::Normal,
    BlendMode::Add,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
    BlendMode::StencilAlpha,
    BlendMode::SilhouetteAlpha,
];

/// A generated pixel `g` put with the layer's `o` by mode `m` (0: the pattern alone).
pub(crate) fn put_with(o: Px, g: Px, m: u32) -> Px {
    if m == 0 { g } else { veymelo_color::blend(MODE_OF[(m as usize).min(19)], o, g, 0.5) }
}

/// `src` over `dst`, both premultiplied.
fn over(dst: Px, src: Px) -> Px {
    let k = 1.0 - src[3];
    [src[0] + dst[0] * k, src[1] + dst[1] * k, src[2] + dst[2] * k, src[3] + dst[3] * k]
}

/// Light (a colour, and how much it covers) added onto a premultiplied pixel.
fn add_light(p: Px, l: [f32; 3], cover: f32) -> Px {
    let cover = cover.clamp(0.0, 1.0);
    [p[0] + l[0], p[1] + l[1], p[2] + l[2], (p[3] + cover * (1.0 - p[3])).clamp(0.0, 1.0)]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A number from 0 to 1 for a pixel and seed, the same every time.
fn grain(x: usize, y: usize, seed: u64) -> f32 {
    crate::noise::unit(crate::noise::mix(&[x as u64, y as u64, seed])) as f32
}

effect!(
    GRADIENT_RAMP_DEF,
    "gradientRamp",
    "Gradient Ramp",
    "fills the layer's shape with a ramp from one colour to another, straight or round",
    [
        point("startOfRamp", "Start of Ramp"),
        color("startColor", "Start Color", 0.0, 0.0, 0.0),
        point("endOfRamp", "End of Ramp"),
        color("endColor", "End Color", 1.0, 1.0, 1.0),
        choice("rampShape", "Ramp Shape", &["linear", "radial"], 0),
        ranged("rampScatter", "Ramp Scatter", 0.0, 0.0, 512.0).doc("grain against banding"),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
    ]
);
pub const GRADIENT_RAMP: Spec = Spec::new(&GRADIENT_RAMP_DEF, CATEGORY, gradient_ramp).points(&[("startOfRamp", [0.5, 0.0]), ("endOfRamp", [0.5, 1.0])]);

fn gradient_ramp(ctx: &Ctx, mut b: Buf) -> Buf {
    let (s, e) = (ctx.point("startOfRamp"), ctx.point("endOfRamp"));
    let (c0, c1) = (rgb3(ctx.color("startColor")), rgb3(ctx.color("endColor")));
    let radial = ctx.choice("rampShape") == 1;
    let keep = ctx.num("blendWithOriginal", 0.0) as f32 / 100.0;
    let scatter = ctx.num("rampScatter", 0.0) as f32 / 512.0;
    let (dx, dy) = (e.x - s.x, e.y - s.y);
    let len2 = (dx * dx + dy * dy).max(1e-9);
    let seed = ctx.seed;
    let g = b.clone_geometry();
    map_px(&mut b.img, |x, y, p| {
        let (lx, ly) = g.at(x, y);
        let (vx, vy) = (lx - s.x, ly - s.y);
        let mut t = if radial { ((vx * vx + vy * vy) / len2).sqrt() } else { (vx * dx + vy * dy) / len2 } as f32;
        if scatter > 0.0 {
            t += (grain(x, y, seed) - 0.5) * scatter;
        }
        // the ramp fills only the layer's shape: it keeps the layer's alpha
        let g = premul(mix3(c0, c1, t.clamp(0.0, 1.0)), p[3]);
        [0, 1, 2, 3].map(|i| p[i] * keep + g[i] * (1.0 - keep))
    });
    b
}

effect!(
    FOUR_COLOR_DEF,
    "fourColorGradient",
    "4-Color Gradient",
    "a smooth mix of four colours, each strongest at its own point",
    [
        point("point1", "Point 1"),
        color("color1", "Color 1", 1.0, 1.0, 0.0),
        point("point2", "Point 2"),
        color("color2", "Color 2", 0.0, 1.0, 0.0),
        point("point3", "Point 3"),
        color("color3", "Color 3", 1.0, 0.0, 1.0),
        point("point4", "Point 4"),
        color("color4", "Color 4", 0.0, 0.0, 1.0),
        ranged("blend", "Blend", 100.0, 1.0, 1000.0).doc("how far each colour spreads into the others"),
        pct("jitter", "Jitter", 0.0, 0.0, 100.0).doc("grain against banding"),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        choice("blendingMode", "Blending Mode", &MODES, 0),
    ]
);
pub const FOUR_COLOR: Spec = Spec::new(&FOUR_COLOR_DEF, CATEGORY, four_color).points(&[("point1", [0.1, 0.1]), ("point2", [0.9, 0.1]), ("point3", [0.1, 0.9]), ("point4", [0.9, 0.9])]);

fn four_color(ctx: &Ctx, mut b: Buf) -> Buf {
    let pts: Vec<((f64, f64), [f32; 3])> = (1..=4).map(|i| {
        let p = ctx.point(&format!("point{i}"));
        ((p.x, p.y), rgb3(ctx.color(&format!("color{i}"))))
    }).collect();
    let spread = ctx.num("blend", 100.0).max(1.0);
    let jitter = (ctx.num("jitter", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    let op = ctx.num("opacity", 100.0) as f32 / 100.0;
    let mode = ctx.choice("blendingMode");
    let seed = ctx.seed;
    let g = b.clone_geometry();
    map_px(&mut b.img, |x, y, p| {
        let (lx, ly) = g.at(x, y);
        let (mut acc, mut total) = ([0f64; 3], 0.0);
        for ((qx, qy), c) in &pts {
            let w = 1.0 / (((lx - qx).powi(2) + (ly - qy).powi(2)) / (spread * 100.0) + 1e-6).powf(1.5);
            for i in 0..3 {
                acc[i] += c[i] as f64 * w;
            }
            total += w;
        }
        let mut c = acc.map(|v| (v / total) as f32);
        if jitter > 0.0 {
            let n = (grain(x, y, seed) - 0.5) * jitter * (8.0 / 255.0);
            c = c.map(|v| v + n);
        }
        // alone it fills the layer's shape; blended it covers all of the layer
        let a = if mode == 0 { op * p[3] } else { op };
        put_with(p, premul(c, a), mode)
    });
    b
}

/// Where a buffer's pixels are in the layer, kept apart from its picture (for closures that
/// change the picture).
struct Geometry {
    origin: (f64, f64),
    scale: f64,
}

impl Geometry {
    fn at(&self, x: usize, y: usize) -> (f64, f64) {
        (self.origin.0 + (x as f64 + 0.5) / self.scale, self.origin.1 + (y as f64 + 0.5) / self.scale)
    }
}

impl Buf {
    fn clone_geometry(&self) -> Geometry {
        Geometry { origin: (self.origin.x, self.origin.y), scale: self.scale }
    }
}

const SIZE_FROM: &[&str] = &["cornerPoint", "widthSlider", "widthAndHeightSliders"];

/// The size of a pattern's cells in layer units.
fn cell_size(ctx: &Ctx, anchor: (f64, f64)) -> (f64, f64) {
    let (w, h) = match ctx.choice("sizeFrom") {
        0 => {
            let c = ctx.point("corner");
            ((c.x - anchor.0).abs(), (c.y - anchor.1).abs())
        }
        1 => (ctx.num("width", 64.0), ctx.num("width", 64.0)),
        _ => (ctx.num("width", 64.0), ctx.num("height", 64.0)),
    };
    (w.max(1e-3), h.max(1e-3))
}

effect!(
    CHECKERBOARD_DEF,
    "checkerboard",
    "Checkerboard",
    "squares of a colour and nothing, like a chessboard",
    [
        point("anchor", "Anchor"),
        choice("sizeFrom", "Size From", SIZE_FROM, 1),
        point("corner", "Corner"),
        px("width", "Width", 64.0, 4000.0),
        px("height", "Height", 64.0, 4000.0),
        px("featherWidth", "Feather Width", 0.0, 1000.0),
        px("featherHeight", "Feather Height", 0.0, 1000.0),
        color("color", "Color", 1.0, 1.0, 1.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        choice("blendingMode", "Blending Mode", &MODES, 0),
    ]
);
pub const CHECKERBOARD: Spec = Spec::new(&CHECKERBOARD_DEF, CATEGORY, checkerboard).points(&[("corner", [0.1, 0.1])]);

fn checkerboard(ctx: &Ctx, mut b: Buf) -> Buf {
    let a = ctx.point("anchor");
    let anchor = (a.x, a.y);
    let (w, h) = cell_size(ctx, anchor);
    // the feather in layer units, never less than half a pixel
    let px_size = 1.0 / b.scale;
    let (fw, fh) = (ctx.num("featherWidth", 0.0).max(px_size), ctx.num("featherHeight", 0.0).max(px_size));
    let c = rgb3(ctx.color("color"));
    let op = ctx.num("opacity", 100.0) as f32 / 100.0;
    let mode = ctx.choice("blendingMode");
    // how far into its cell a point is (in feathers, up to 1), signed by the cell's colour
    let side = |p: f64, size: f64, f: f64| -> f64 {
        let u = p / size;
        let fr = u - u.floor();
        let d = fr.min(1.0 - fr) * size;
        let s = if (u.floor() as i64).rem_euclid(2) == 0 { 1.0 } else { -1.0 };
        s * (d / (f * 0.5)).min(1.0)
    };
    let g = b.clone_geometry();
    map_px(&mut b.img, |x, y, p| {
        let (lx, ly) = g.at(x, y);
        let k = ((0.5 + 0.5 * side(lx - anchor.0, w, fw) * side(ly - anchor.1, h, fh)) as f32).clamp(0.0, 1.0) * op;
        put_with(p, premul(c, k), mode)
    });
    b
}

effect!(
    GRID_DEF,
    "grid",
    "Grid",
    "lines in a grid",
    [
        point("anchor", "Anchor"),
        choice("sizeFrom", "Size From", SIZE_FROM, 1),
        point("corner", "Corner"),
        px("width", "Width", 64.0, 4000.0),
        px("height", "Height", 64.0, 4000.0),
        px("border", "Border", 2.0, 400.0).doc("the lines' width"),
        px("featherWidth", "Feather Width", 0.0, 1000.0),
        px("featherHeight", "Feather Height", 0.0, 1000.0),
        flag("invertGrid", "Invert Grid", false),
        color("color", "Color", 1.0, 1.0, 1.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        choice("blendingMode", "Blending Mode", &MODES, 0),
    ]
);
pub const GRID: Spec = Spec::new(&GRID_DEF, CATEGORY, grid).points(&[("corner", [0.1, 0.1])]);

fn grid(ctx: &Ctx, mut b: Buf) -> Buf {
    let a = ctx.point("anchor");
    let anchor = (a.x, a.y);
    let (w, h) = cell_size(ctx, anchor);
    let border = ctx.num("border", 2.0).max(0.0);
    let px_size = 1.0 / b.scale;
    let (fw, fh) = (ctx.num("featherWidth", 0.0).max(px_size), ctx.num("featherHeight", 0.0).max(px_size));
    let invert = ctx.flag("invertGrid");
    let c = rgb3(ctx.color("color"));
    let op = ctx.num("opacity", 100.0) as f32 / 100.0;
    let mode = ctx.choice("blendingMode");
    let g = b.clone_geometry();
    map_px(&mut b.img, |x, y, p| {
        let (lx, ly) = g.at(x, y);
        let (gx, gy) = ((lx - anchor.0).rem_euclid(w), (ly - anchor.1).rem_euclid(h));
        let (dx, dy) = (gx.min(w - gx), gy.min(h - gy));
        let on = if border <= 0.0 { 0.0 } else { ((border / 2.0 - dx) / fw + 0.5).clamp(0.0, 1.0).max(((border / 2.0 - dy) / fh + 0.5).clamp(0.0, 1.0)) as f32 };
        let k = if invert { 1.0 - on } else { on } * op;
        put_with(p, premul(c, k), mode)
    });
    b
}

effect!(
    CIRCLE_DEF,
    "circle",
    "Circle",
    "a disc, or a ring",
    [
        point("center", "Center"),
        px("radius", "Radius", 75.0, 4000.0),
        choice("edge", "Edge", &["none", "edgeRadius", "thickness", "thicknessTimesRadius", "thicknessAndFeatherTimesRadius"], 0).doc("none: a disc; the others make a ring"),
        px("edgeRadius", "Edge Radius", 0.0, 4000.0),
        num("thickness", "Thickness", 10.0).range(0.0, 4000.0).doc("in pixels, or % of the radius"),
        num("featherOuterEdge", "Feather Outer Edge", 0.0).range(0.0, 1000.0),
        num("featherInnerEdge", "Feather Inner Edge", 0.0).range(0.0, 1000.0),
        flag("invertCircle", "Invert Circle", false),
        color("color", "Color", 1.0, 1.0, 1.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        choice("blendingMode", "Blending Mode", &MODES, 0),
    ]
);
pub const CIRCLE: Spec = Spec::new(&CIRCLE_DEF, CATEGORY, circle).points(&[("center", [0.5, 0.5])]);

fn circle(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = ctx.point("center");
    let r = ctx.num("radius", 75.0);
    let thick = ctx.num("thickness", 10.0).max(0.0);
    let (mut f_out, mut f_in) = (ctx.num("featherOuterEdge", 0.0).max(0.0), ctx.num("featherInnerEdge", 0.0).max(0.0));
    let inner = match ctx.choice("edge") {
        1 => Some(ctx.num("edgeRadius", 0.0)),
        2 => Some(r - thick),
        3 => Some(r - thick / 100.0 * r),
        4 => {
            (f_out, f_in) = (f_out / 100.0 * r, f_in / 100.0 * r);
            Some(r - thick / 100.0 * r)
        }
        _ => None,
    };
    let (outer, inner) = match inner {
        Some(i) => (r.max(i), Some(r.min(i).max(0.0))),
        None => (r, None),
    };
    // at least half a pixel of softness, for a smooth edge
    let half = 0.5 / b.scale;
    let (f_out, f_in) = (f_out.max(half * 2.0), f_in.max(half * 2.0));
    let col = rgb3(ctx.color("color"));
    let op = ctx.num("opacity", 100.0) as f32 / 100.0;
    let invert = ctx.flag("invertCircle");
    let mode = ctx.choice("blendingMode");
    let g = b.clone_geometry();
    map_px(&mut b.img, |x, y, p| {
        let (lx, ly) = g.at(x, y);
        let d = (lx - c.x).hypot(ly - c.y);
        let mut k = ((outer - d) / f_out + 0.5).clamp(0.0, 1.0) as f32;
        if let Some(i) = inner {
            k *= ((d - i) / f_in + 0.5).clamp(0.0, 1.0) as f32;
        }
        let k = if invert { 1.0 - k } else { k } * op;
        put_with(p, premul(col, k), mode)
    });
    b
}

effect!(
    ELLIPSE_DEF,
    "ellipse",
    "Ellipse",
    "a glowing ring, its colour fading from inside to outside",
    [
        point("center", "Center"),
        px("width", "Width", 100.0, 30000.0),
        px("height", "Height", 100.0, 30000.0),
        px("thickness", "Thickness", 10.0, 3000.0),
        pct("softness", "Softness", 0.0, 0.0, 100.0),
        color("insideColor", "Inside Color", 1.0, 1.0, 1.0),
        color("outsideColor", "Outside Color", 0.0, 0.6, 1.0),
        flag("compositeOnOriginal", "Composite On Original", false),
    ]
);
pub const ELLIPSE: Spec = Spec::new(&ELLIPSE_DEF, CATEGORY, ellipse).points(&[("center", [0.5, 0.5])]);

fn ellipse(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = ctx.point("center");
    let (rx, ry) = ((ctx.num("width", 100.0) * 0.5).max(1e-3), (ctx.num("height", 100.0) * 0.5).max(1e-3));
    let half = (ctx.num("thickness", 10.0) * 0.5).max(0.25 / b.scale);
    let soft = (ctx.num("softness", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    let (inside, outside) = (rgb3(ctx.color("insideColor")), rgb3(ctx.color("outsideColor")));
    let composite = ctx.flag("compositeOnOriginal");
    let (g, s) = (b.clone_geometry(), b.scale);
    map_px(&mut b.img, |x, y, p| {
        let (lx, ly) = g.at(x, y);
        let (dx, dy) = (lx - c.x, ly - c.y);
        // the distance to the ellipse, to first order
        let q = ((dx / rx).powi(2) + (dy / ry).powi(2)).sqrt();
        let dist = if q < 1e-9 {
            rx.min(ry)
        } else {
            let (gx, gy) = (dx / (rx * rx) / q, dy / (ry * ry) / q);
            ((q - 1.0) / gx.hypot(gy).max(1e-9)).abs()
        };
        let t = (dist / half) as f32;
        let a = if soft > 1e-3 { 1.0 - smoothstep(1.0 - soft, 1.0, t) } else { ((half - dist) * s + 0.5).clamp(0.0, 1.0) as f32 };
        let gpx = premul(mix3(inside, outside, smoothstep(0.0, 1.0, t)), a);
        if composite { over(p, gpx) } else { gpx }
    });
    b
}

effect!(
    BEAM_DEF,
    "beam",
    "Beam",
    "a bolt of light travelling from one point to another",
    [
        point("startingPoint", "Starting Point"),
        point("endingPoint", "Ending Point"),
        pct("length", "Length", 25.0, 0.0, 100.0).doc("of the way between the points"),
        pct("time", "Time", 0.0, 0.0, 100.0).doc("how far along it has travelled (animate it)"),
        px("startingThickness", "Starting Thickness", 8.0, 1000.0),
        px("endingThickness", "Ending Thickness", 8.0, 1000.0),
        pct("softness", "Softness", 20.0, 0.0, 100.0),
        color("insideColor", "Inside Color", 1.0, 1.0, 1.0),
        color("outsideColor", "Outside Color", 0.4, 0.4, 1.0),
        flag("perspective3d", "3D Perspective", true).doc("on: the thickness follows where the beam is between the points; off: its own tail and head"),
        flag("compositeOnOriginal", "Composite On Original", true),
    ]
);
pub const BEAM: Spec = Spec::new(&BEAM_DEF, CATEGORY, beam).points(&[("startingPoint", [0.25, 0.5]), ("endingPoint", [0.75, 0.5])]);

fn beam(ctx: &Ctx, mut b: Buf) -> Buf {
    let (s, e) = (ctx.point("startingPoint"), ctx.point("endingPoint"));
    let len = (ctx.num("length", 25.0) / 100.0).clamp(0.0, 1.0);
    let time = (ctx.num("time", 0.0) / 100.0).clamp(0.0, 1.0);
    let (st, et) = (ctx.num("startingThickness", 8.0), ctx.num("endingThickness", 8.0));
    let soft = (ctx.num("softness", 20.0) / 100.0).clamp(0.0, 1.0) as f32;
    let (inside, outside) = (rgb3(ctx.color("insideColor")), rgb3(ctx.color("outsideColor")));
    let composite = ctx.flag("compositeOnOriginal");
    let perspective = ctx.flag("perspective3d");
    let (t0, t1) = (time * (1.0 - len), time * (1.0 - len) + len);
    let (dx, dy) = (e.x - s.x, e.y - s.y);
    let l2 = (dx * dx + dy * dy).max(1e-9);
    let (g, sc) = (b.clone_geometry(), b.scale);
    map_px(&mut b.img, |x, y, p| {
        let (lx, ly) = g.at(x, y);
        let tc = (((lx - s.x) * dx + (ly - s.y) * dy) / l2).clamp(t0, t1);
        let dist = (lx - (s.x + dx * tc)).hypot(ly - (s.y + dy * tc));
        let tk = if perspective || t1 - t0 < 1e-9 { tc } else { (tc - t0) / (t1 - t0) };
        let half = ((st + (et - st) * tk) * 0.5).max(0.25 / sc);
        let tt = (dist / half) as f32;
        let a = if len <= 0.0 {
            0.0
        } else if soft > 1e-3 {
            1.0 - smoothstep(1.0 - soft, 1.0, tt)
        } else {
            ((half - dist) * sc + 0.5).clamp(0.0, 1.0) as f32
        };
        let gpx = premul(mix3(inside, outside, smoothstep(0.0, 1.0, tt)), a);
        if composite { over(p, gpx) } else { gpx }
    });
    b
}

effect!(
    LENS_FLARE_DEF,
    "lensFlare",
    "Lens Flare",
    "the glare a bright light makes in a camera lens: a bright core, streaks, a halo and ghosts along a line through the middle",
    [
        point("flareCenter", "Flare Center"),
        pct("flareBrightness", "Flare Brightness", 100.0, 0.0, 300.0),
        choice("lensType", "Lens Type", &["zoom50To300", "prime35", "prime105"], 0),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
    ]
);
pub const LENS_FLARE: Spec = Spec::new(&LENS_FLARE_DEF, CATEGORY, lens_flare).points(&[("flareCenter", [0.25, 0.25])]);

/// A ghost of a lens flare: how far along the line through the middle (0 at the light, 1 at the
/// middle), its size (of the diagonal), colour and strength, and whether it is a ring.
struct Ghost(f64, f64, [f32; 3], f32, bool);

/// What a lens's flare is made of: the ghosts its pairs of glass surfaces throw (a zoom has many
/// elements, a prime few), the blades of its iris (one streak a blade), how tight its core is and
/// how far out its halo (of the diagonal; the longer the lens, the tighter the core and the wider
/// the halo).
struct Lens {
    ghosts: usize,
    blades: f64,
    core: f64,
    halo: f64,
    seed: u64,
}

fn lens_of(kind: u32) -> Lens {
    match kind {
        // a 35 mm prime
        1 => Lens { ghosts: 4, blades: 6.0, core: 0.036, halo: 0.17, seed: 35 },
        // a 105 mm prime
        2 => Lens { ghosts: 3, blades: 10.0, core: 0.022, halo: 0.3, seed: 105 },
        // a 50 to 300 mm zoom
        _ => Lens { ghosts: 7, blades: 8.0, core: 0.03, halo: 0.24, seed: 50 },
    }
}

/// The ghosts of `lens`: spread along the line through the middle, past it too; those farther
/// from the middle bigger (more out of focus) and those smaller brighter (their light less
/// spread); each tinted by the coating it bounced off (hues a golden turn apart, so neighbours
/// differ); the big ones some of them rings.
fn ghosts(lens: &Lens) -> Vec<Ghost> {
    let u = |k: usize, what: u64| crate::noise::unit(crate::noise::mix(&[lens.seed, k as u64, what]));
    (0..lens.ghosts)
        .map(|k| {
            let along = 0.25 + 1.85 * (k as f64 + 0.5) / lens.ghosts as f64 + 0.12 * (u(k, 0) - 0.5);
            let size = 0.012 + 0.07 * u(k, 1).powf(1.3) * (0.5 + 0.5 * (along - 1.0).abs());
            let hue = (0.07 + 0.618_034 * k as f64 + 0.1 * u(k, 2)).fract();
            let colour = crate::util::rgb(hue as f32, 1.0, 0.75);
            let strength = 0.06 + 0.22 * (0.025 / (size + 0.005)).min(1.0);
            Ghost(along, size, colour, strength as f32, size > 0.045 && u(k, 3) > 0.4)
        })
        .collect()
}

fn lens_flare(ctx: &Ctx, mut b: Buf) -> Buf {
    let f = ctx.point("flareCenter");
    let bright = ctx.num("flareBrightness", 100.0) as f32 / 100.0;
    let lens = ctx.choice("lensType");
    let keep = ctx.num("blendWithOriginal", 0.0) as f32 / 100.0;
    let r = ctx.bounds;
    let (mid, diag) = (r.center(), r.width().hypot(r.height()).max(1.0));
    let lens = lens_of(lens);
    let gs = ghosts(&lens);
    let (core_k, rays, halo_r) = (lens.core, lens.blades, lens.halo);
    let g = b.clone_geometry();
    map_px(&mut b.img, |x, y, p| {
        let (lx, ly) = g.at(x, y);
        let (dx, dy) = (lx - f.x, ly - f.y);
        let d = dx.hypot(dy) / diag;
        let core = 1.5 * (-(d / core_k).powi(2)).exp() + 0.25 * (-d / 0.12).exp();
        let streak = (rays * 0.5 * dy.atan2(dx)).cos().abs().powi(40) * (-d / 0.3).exp() * 0.35;
        let halo = ((-((d - halo_r) / 0.012).powi(2)).exp() * 0.12) as f32;
        let base = (core + streak) as f32;
        let mut l = [base + halo * 0.6, base * 0.95 + halo * 0.8, base * 0.85 + halo];
        for Ghost(t, gr, gc, k, ring) in &gs {
            let (gx, gy) = (f.x + (mid.x - f.x) * t, f.y + (mid.y - f.y) * t);
            let dd = (lx - gx).hypot(ly - gy) / diag;
            let v = if *ring { (-((dd - gr) / (gr * 0.12)).powi(2)).exp() as f32 } else { smoothstep(*gr as f32, *gr as f32 * 0.8, dd as f32) };
            for i in 0..3 {
                l[i] += gc[i] * k * v;
            }
        }
        let l = l.map(|v| v * bright);
        let o = add_light(p, l, l[0].max(l[1]).max(l[2]).min(1.0));
        [0, 1, 2, 3].map(|i| o[i] + (p[i] - o[i]) * keep)
    });
    b
}

const CELL_PATTERNS: &[&str] = &[
    "bubbles", "crystals", "plates", "staticPlates", "crystallize", "pillow", "crystalsHq", "platesHq", "staticPlatesHq", "crystallizeHq", "mixedCrystalsHq", "tubular",
];

effect!(
    CELL_PATTERN_DEF,
    "cellPattern",
    "Cell Pattern",
    "cells, like bubbles, crystals or plates, that can move (evolution)",
    [
        choice("cellPattern", "Cell Pattern", CELL_PATTERNS, 0).doc("the HQ ones are smoother at the cells' edges"),
        flag("invert", "Invert", false),
        ranged("contrast", "Contrast", 100.0, 0.0, 10000.0),
        choice("overflow", "Overflow", &["clip", "softClamp", "wrapBack"], 0),
        ranged("disperse", "Disperse", 1.0, 0.0, 1.5).doc("how far from a grid the cells wander"),
        px("size", "Size", 60.0, 4000.0),
        point("offset", "Offset"),
        flag("enableTiling", "Enable Tiling", false),
        ranged("cellsHorizontal", "Cells Horizontal", 16.0, 1.0, 1000.0),
        ranged("cellsVertical", "Cells Vertical", 16.0, 1.0, 1000.0),
        angle("evolution", "Evolution", 0.0).doc("animate it to make the cells move"),
        flag("cycleEvolution", "Cycle Evolution", false),
        ranged("cycle", "Cycle (in Revolutions)", 1.0, 1.0, 1000.0),
        ranged("randomSeed", "Random Seed", 0.0, 0.0, 100_000.0),
    ]
);
pub const CELL_PATTERN: Spec = Spec::new(&CELL_PATTERN_DEF, CATEGORY, cell_pattern).points(&[("offset", [0.5, 0.5])]);

/// The nearest and second nearest cell points to (u, v) (in cells), and the nearest one's tone.
#[allow(clippy::too_many_arguments)]
fn cells(u: f64, v: f64, disperse: f64, evo: f64, seed: u64, tile: Option<(i64, i64)>, reach: i64) -> (f64, f64, f32) {
    let (cx, cy) = (u.floor() as i64, v.floor() as i64);
    let (mut f1, mut f2, mut tone) = (f64::INFINITY, f64::INFINITY, 0f32);
    for j in -reach..=reach {
        for i in -reach..=reach {
            let (gx, gy) = (cx + i, cy + j);
            let (tx, ty) = match tile {
                Some((nx, ny)) => (gx.rem_euclid(nx), gy.rem_euclid(ny)),
                None => (gx, gy),
            };
            let h = |k: u64| crate::noise::unit(crate::noise::mix(&[tx as u64, ty as u64, seed, k]));
            let ang = 2.0 * PI * (h(2) + evo);
            let (px, py) = (gx as f64 + 0.5 + (h(0) - 0.5) * disperse + 0.15 * disperse * ang.cos(), gy as f64 + 0.5 + (h(1) - 0.5) * disperse + 0.15 * disperse * ang.sin());
            let d = (px - u).hypot(py - v);
            if d < f1 {
                (f2, f1, tone) = (f1, d, h(3) as f32);
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    (f1, f2, tone)
}

fn cell_pattern(ctx: &Ctx, mut b: Buf) -> Buf {
    let which = ctx.choice("cellPattern");
    let hq = (6..=10).contains(&which);
    let shape = match which {
        0..=5 => which,
        6 => 1,
        7 => 2,
        8 => 3,
        9 => 4,
        10 => 6,
        _ => 8,
    };
    let invert = ctx.flag("invert");
    let contrast = ctx.num("contrast", 100.0) as f32 / 100.0;
    let overflow = ctx.choice("overflow");
    let disperse = ctx.num("disperse", 1.0).clamp(0.0, 1.5);
    let size = ctx.num("size", 60.0).max(1e-3);
    let off = ctx.point("offset");
    let mut evo = ctx.num("evolution", 0.0) / 360.0;
    if ctx.flag("cycleEvolution") {
        evo = evo.rem_euclid(ctx.num("cycle", 1.0).round().max(1.0));
    }
    let seed = crate::noise::mix(&[ctx.num("randomSeed", 0.0) as i64 as u64, 0xCE11]);
    let tile = ctx.flag("enableTiling").then(|| (ctx.num("cellsHorizontal", 16.0).round().max(1.0) as i64, ctx.num("cellsVertical", 16.0).round().max(1.0) as i64));
    let subs: Vec<f64> = if hq { (0..4).map(|k| (k as f64 + 0.5) / 4.0).collect() } else { vec![0.5] };
    let reach = if hq { 2 } else { 1 };
    let g = b.clone_geometry();
    map_px(&mut b.img, |x, y, _| {
        let mut total = 0f32;
        for sy in &subs {
            for sx in &subs {
                let (lx, ly) = (g.origin.0 + (x as f64 + sx) / g.scale, g.origin.1 + (y as f64 + sy) / g.scale);
                let (f1, f2, tone) = cells((lx - off.x) / size, (ly - off.y) / size, disperse, evo, seed, tile, reach);
                let (f1, f2) = (f1 as f32, f2 as f32);
                let e = f2 - f1;
                total += match shape {
                    0 => (1.0 - f1 * 1.3).clamp(0.0, 1.0),
                    1 => tone * 0.8 + 0.2 * (1.0 - f1).clamp(0.0, 1.0),
                    2 => (e * 2.0).clamp(0.0, 1.0),
                    3 => tone * smoothstep(0.0, 0.06, e),
                    4 => tone,
                    5 => (1.0 - f1 * f1 * 2.0).clamp(0.0, 1.0),
                    6 => tone * (e * 3.0).clamp(0.0, 1.0),
                    _ => 1.0 - ((e - 0.15).abs() * 5.0).clamp(0.0, 1.0),
                };
            }
        }
        let v = (total / (subs.len() * subs.len()) as f32 - 0.5) * contrast + 0.5;
        let v = match overflow {
            1 => 0.5 + 0.5 * ((v - 0.5) * 2.0).tanh(),
            2 => {
                let t = v.rem_euclid(2.0);
                if t > 1.0 { 2.0 - t } else { t }
            }
            _ => v.clamp(0.0, 1.0),
        };
        let v = if invert { 1.0 - v } else { v };
        [v, v, v, 1.0]
    });
    b
}

effect!(
    LIGHT_RAYS_DEF,
    "lightRays",
    "Light Rays",
    "rays of light streaming out from a centre, coloured by what they pass through",
    [
        pct("intensity", "Intensity", 100.0, 0.0, 600.0),
        point("center", "Center"),
        px("radius", "Radius", 40.0, 4000.0).doc("how much round the centre the light comes from"),
        pct("warpSoftness", "Warp Softness", 50.0, 0.0, 400.0),
        choice("shape", "Shape", &["round", "square"], 0),
        flag("colorFromSource", "Color from Source", true),
        color("color", "Color", 1.0, 1.0, 1.0),
        choice("transferMode", "Transfer Mode", &["add", "lighten", "screen", "none"], 0),
    ]
);
pub const LIGHT_RAYS: Spec = Spec::new(&LIGHT_RAYS_DEF, CATEGORY, light_rays).points(&[("center", [0.5, 0.5])]);

/// How many steps a ray of light is gathered over.
const RAY_STEPS: usize = 48;

fn light_rays(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = b.pixel(ctx.point("center"));
    let k = (ctx.num("intensity", 100.0) / 100.0 * 0.5).clamp(0.0, 0.95);
    let gain = (ctx.num("intensity", 100.0) / 100.0 * 2.0) as f32;
    let rad = (ctx.num("radius", 40.0) * b.scale).max(0.5);
    let soft = (ctx.num("warpSoftness", 50.0) / 50.0).max(0.0);
    let square = ctx.choice("shape") == 1;
    let from_source = ctx.flag("colorFromSource");
    let col = rgb3(ctx.color("color"));
    let mode = ctx.choice("transferMode");
    let src = b.img.clone();
    let near = |x: f64, y: f64| {
        let (dx, dy) = ((x - c.x).abs(), (y - c.y).abs());
        let d = if square { dx.max(dy) } else { dx.hypot(dy) };
        1.0 - smoothstep(rad as f32, (rad * (1.0 + soft)) as f32 + 1e-3, d as f32)
    };
    map_px(&mut b.img, |x, y, p| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let mut acc = [0f32; 4];
        for i in 0..RAY_STEPS {
            let s = 1.0 - k * i as f64 / RAY_STEPS as f64;
            let (qx, qy) = (c.x + (px - c.x) * s, c.y + (py - c.y) * s);
            let m = near(qx, qy);
            if m > 0.0 {
                let v = src.sample(Sampling::Bilinear, qx, qy);
                for j in 0..4 {
                    acc[j] += v[j] * m;
                }
            }
        }
        let mut ray = acc.map(|v| v / RAY_STEPS as f32 * gain);
        if !from_source {
            let l = luma(unpremul(ray).0) * ray[3];
            ray = [col[0] * l, col[1] * l, col[2] * l, ray[3]];
        }
        let la = ray[3].min(1.0);
        match mode {
            1 => [p[0].max(ray[0]), p[1].max(ray[1]), p[2].max(ray[2]), p[3].max(la)],
            2 => {
                let s = |a: f32, b: f32| a + b - a * b;
                [s(p[0], ray[0]), s(p[1], ray[1]), s(p[2], ray[2]), s(p[3], la)]
            }
            3 => [ray[0], ray[1], ray[2], la],
            _ => add_light(p, [ray[0], ray[1], ray[2]], la),
        }
    });
    b
}

effect!(
    LIGHT_BURST_DEF,
    "lightBurst",
    "Light Burst",
    "streaks the picture out from a centre, as if it were bursting with light",
    [
        point("center", "Center"),
        pct("intensity", "Intensity", 100.0, 0.0, 600.0),
        pct("rayLength", "Ray Length", 50.0, 0.0, 100.0),
        choice("burst", "Burst", &["straight", "fade", "center"], 0).doc("straight keeps the brightest along each ray; fade and center mix along it"),
        flag("setColor", "Set Color", false),
        color("color", "Color", 1.0, 1.0, 1.0),
    ]
);
pub const LIGHT_BURST: Spec = Spec::new(&LIGHT_BURST_DEF, CATEGORY, light_burst).points(&[("center", [0.5, 0.5])]);

fn light_burst(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = b.pixel(ctx.point("center"));
    let intensity = ctx.num("intensity", 100.0) as f32 / 100.0;
    let k = (ctx.num("rayLength", 50.0) / 100.0).clamp(0.0, 1.0);
    let mode = ctx.choice("burst");
    let set = ctx.flag("setColor").then(|| rgb3(ctx.color("color")));
    if k <= 0.0 && (intensity - 1.0).abs() < 1e-6 && set.is_none() {
        return b;
    }
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let (mut acc, mut total, mut best, mut best_l) = ([0f32; 4], 0f32, [0f32; 4], -1f32);
        for i in 0..RAY_STEPS {
            let f = i as f64 / RAY_STEPS as f64;
            let s = 1.0 - k * f;
            let v = src.sample(Sampling::Bilinear, c.x + (px - c.x) * s, c.y + (py - c.y) * s);
            match mode {
                0 => {
                    let l = luma_of(v) + v[3] * 1e-3;
                    if l > best_l {
                        (best_l, best) = (l, v);
                    }
                }
                1 => {
                    let w = 1.0 - f as f32;
                    for j in 0..4 {
                        acc[j] += v[j] * w;
                    }
                    total += w;
                }
                _ => {
                    for j in 0..4 {
                        acc[j] += v[j];
                    }
                    total += 1.0;
                }
            }
        }
        let mut o = if mode == 0 { best } else { acc.map(|v| v / total.max(1e-6)) };
        if let Some(col) = set {
            let (sc, a) = unpremul(o);
            let l = luma(sc) * a;
            o = [col[0] * l, col[1] * l, col[2] * l, a];
        }
        [o[0] * intensity, o[1] * intensity, o[2] * intensity, (o[3] * intensity).clamp(0.0, 1.0)]
    });
    b
}

fn luma_of(p: Px) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

effect!(
    LIGHT_SWEEP_DEF,
    "lightSweep",
    "Light Sweep",
    "a band of light passing over the layer, brightest at its edges",
    [
        point("center", "Center"),
        angle("direction", "Direction", -30.0),
        choice("shape", "Shape", &["linear", "smooth", "sharp"], 1),
        px("width", "Width", 50.0, 4000.0),
        pct("sweepIntensity", "Sweep Intensity", 50.0, 0.0, 500.0),
        pct("edgeIntensity", "Edge Intensity", 100.0, 0.0, 500.0),
        px("edgeThickness", "Edge Thickness", 1.0, 100.0),
        color("lightColor", "Light Color", 1.0, 1.0, 1.0),
        choice("lightReception", "Light Reception", &["add", "composite", "cutout"], 0),
    ]
);
pub const LIGHT_SWEEP: Spec = Spec::new(&LIGHT_SWEEP_DEF, CATEGORY, light_sweep).points(&[("center", [0.5, 0.5])]);

fn light_sweep(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = b.pixel(ctx.point("center"));
    let dir = ctx.num("direction", -30.0).to_radians();
    let shape = ctx.choice("shape");
    let half = (ctx.num("width", 50.0) * b.scale * 0.5).max(0.5);
    let (sweep, edge_i) = (ctx.num("sweepIntensity", 50.0) as f32 / 100.0, ctx.num("edgeIntensity", 100.0) as f32 / 100.0);
    let edge_t = ctx.num("edgeThickness", 1.0) * b.scale;
    let lc = rgb3(ctx.color("lightColor"));
    let mode = ctx.choice("lightReception");
    let (nx, ny) = (dir.cos(), dir.sin());
    let mut alpha = b.img.clone();
    map_px(&mut alpha, |_, _, p| [p[3]; 4]);
    let soft = if edge_t > 0.05 { filter::gaussian(&alpha, edge_t * 0.5, edge_t * 0.5, Edge::Repeat) } else { alpha };
    let (w, h) = (soft.width as i64, soft.height as i64);
    let a_at = |x: i64, y: i64| soft.px[(y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize][0];
    map_px(&mut b.img, |x, y, p| {
        let d = ((x as f64 + 0.5 - c.x) * nx + (y as f64 + 0.5 - c.y) * ny).abs();
        let band = match shape {
            0 => (1.0 - d / half).max(0.0) as f32,
            2 => (half - d + 0.5).clamp(0.0, 1.0) as f32,
            _ => smoothstep(half as f32, 0.0, d as f32),
        };
        if band <= 0.0 {
            return if mode == 2 { [0.0; 4] } else { p };
        }
        let (xi, yi) = (x as i64, y as i64);
        let edge = ((a_at(xi + 1, yi) - a_at(xi - 1, yi)).hypot(a_at(xi, yi + 1) - a_at(xi, yi - 1)) * 2.0).min(1.0);
        let light = band * sweep + band * edge * edge_i;
        let a = p[3];
        match mode {
            1 => [p[0] + lc[0] * light, p[1] + lc[1] * light, p[2] + lc[2] * light, (a + (band * sweep).min(1.0) * (1.0 - a)).min(1.0)],
            2 => {
                let la = (light * a).min(1.0);
                premul(lc, la)
            }
            _ => [p[0] + lc[0] * light * a, p[1] + lc[1] * light * a, p[2] + lc[2] * light * a, a],
        }
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 12] = [GRADIENT_RAMP, FOUR_COLOR, CHECKERBOARD, GRID, CIRCLE, ELLIPSE, BEAM, LENS_FLARE, CELL_PATTERN, LIGHT_RAYS, LIGHT_BURST, LIGHT_SWEEP];

#[cfg(test)]
mod tests;
