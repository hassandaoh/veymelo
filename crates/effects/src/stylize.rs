//! Stylize, part one: light spilling from the bright parts (Glow), blocks and steps (Mosaic,
//! Posterize, Threshold), edges and relief (Find Edges, Emboss, Color Emboss), a drawn look
//! (Cartoon), worn edges (Roughen Edges), scattered pixels (Scatter), flashes (Strobe Light) and
//! a texture pressed in (Texturize).
//!
//! Sizes and places are in layer units, so a preview looks the same as the full picture.

use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px, Sampling};

use crate::color::rgb3;
use crate::defs::{angle, choice, color, effect, flag, layer, num, pct, point, px, ranged};
use crate::distort::rect_px;
use crate::generate::MODE_OF;
use crate::grade::Curve;
use crate::util::{luma, map_px, premul, unpremul};
use crate::{Buf, Ctx, Fit, Spec, Take};

const CATEGORY: &str = "stylize";

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn mix4(a: Px, b: Px, t: f32) -> Px {
    [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t)
}

const OPERATIONS: [&str; 17] = [
    "normal", "add", "multiply", "screen", "overlay", "softLight", "hardLight", "colorDodge", "colorBurn", "darken", "lighten", "difference", "exclusion", "hue", "saturation", "color", "luminosity",
];

effect!(
    GLOW_DEF,
    "glow",
    "Glow",
    "light spilling out from the bright parts of the layer",
    [
        choice("glowBasedOn", "Glow Based On", &["alphaChannel", "colorChannels"], 1),
        pct("glowThreshold", "Glow Threshold", 60.0, 0.0, 100.0).doc("only what is brighter than this glows"),
        px("glowRadius", "Glow Radius", 10.0, 1000.0),
        ranged("glowIntensity", "Glow Intensity", 1.0, 0.0, 255.0),
        choice("compositeOriginal", "Composite Original", &["onTop", "behind", "none"], 0).doc("the layer over its glow, behind it, or the glow alone"),
        choice("glowOperation", "Glow Operation", &OPERATIONS, 1),
        choice("glowColors", "Glow Colors", &["originalColors", "aAndBColors", "arbitraryMap"], 0),
        veymelo_project::PropDef::new("arbitraryMap", "Arbitrary Map", veymelo_project::Kind::Text, veymelo_keyframe::Value::Text(String::new()))
            .doc("the glow's colour from how bright it is: red, green and blue curves \"x,y x,y … | … | …\" (empty: straight)"),
        choice("colorLooping", "Color Looping", &["sawtoothAToB", "sawtoothBToA", "triangleABA", "triangleBAB"], 2),
        ranged("colorLoops", "Color Loops", 1.0, 1.0, 127.0),
        angle("colorPhase", "Color Phase", 0.0),
        pct("aAndBMidpoint", "A & B Midpoint", 50.0, 0.0, 100.0),
        color("colorA", "Color A", 1.0, 1.0, 1.0),
        color("colorB", "Color B", 0.0, 0.0, 0.0),
        choice("glowDimensions", "Glow Dimensions", &["horizontalAndVertical", "horizontal", "vertical"], 0),
    ]
);
pub const GLOW: Spec = Spec::new(&GLOW_DEF, CATEGORY, glow);

/// Where between colour A (0) and B (1) brightness `l` falls: `loops` times round from `phase`
/// (turns), as `looping` says, `mid` going to the middle.
pub fn a_to_b(l: f32, looping: u32, loops: f32, phase: f32, mid: f32) -> f32 {
    let u = l.clamp(0.0, 1.0) * loops.max(0.0) + phase;
    let mut t = u - u.floor();
    if u > 0.0 && t <= 0.0 {
        t = 1.0;
    }
    let tri = 1.0 - (2.0 * t - 1.0).abs();
    let t = match looping {
        0 => t,
        1 => 1.0 - t,
        3 => 1.0 - tri,
        _ => tri,
    };
    t.max(0.0).powf(0.5f32.ln() / mid.clamp(0.01, 0.99).ln())
}

fn glow(ctx: &Ctx, mut b: Buf) -> Buf {
    let threshold = (ctx.num("glowThreshold", 60.0) / 100.0) as f32;
    let radius = ctx.num("glowRadius", 10.0) * b.scale;
    let intensity = ctx.num("glowIntensity", 1.0) as f32;
    let from_alpha = ctx.choice("glowBasedOn") == 0;
    let colours = ctx.choice("glowColors");
    let map: Option<Vec<Option<Curve>>> = (colours == 2).then(|| {
        let text = ctx.value_in(ctx.effect, "arbitraryMap").map(|v| v.as_text().to_string()).unwrap_or_default();
        let mut parts = text.split('|');
        (0..3).map(|_| parts.next().and_then(Curve::parse)).collect()
    });
    let looping = ctx.choice("colorLooping");
    let loops = ctx.num("colorLoops", 1.0) as f32;
    let phase = (ctx.num("colorPhase", 0.0) / 360.0) as f32;
    let mid = (ctx.num("aAndBMidpoint", 50.0) / 100.0) as f32;
    let (ca, cb) = (rgb3(ctx.color("colorA")), rgb3(ctx.color("colorB")));
    let (kx, ky) = match ctx.choice("glowDimensions") {
        1 => (1.0, 0.0),
        2 => (0.0, 1.0),
        _ => (1.0, 1.0),
    };
    if !ctx.adjustment {
        b.pad((radius * 1.5).ceil() as usize + 2);
    }
    // what glows: the bright parts, in their own colours or the chosen ones
    let mut bright = b.img.clone();
    map_px(&mut bright, |_, _, p| {
        let (c, a) = unpremul(p);
        if a <= 0.0 {
            return [0.0; 4];
        }
        let l = if from_alpha { a } else { luma(c) };
        let k = ((l - threshold) / (1.0 - threshold).max(1e-3)).clamp(0.0, 1.0);
        let col = match &map {
            Some(curves) => {
                let t = a_to_b(l, looping, loops, phase, mid);
                [0, 1, 2].map(|i| curves[i].as_ref().map_or(t, |cv| cv.at(t)))
            }
            None if colours == 1 => mix3(ca, cb, a_to_b(l, looping, loops, phase, mid)),
            None => return p.map(|v| v * k),
        };
        premul(col, k * a)
    });
    let s = (radius / 2.0).max(0.5);
    let spread = filter::gaussian(&bright, s * kx, s * ky, Edge::Clear);
    let how = ctx.choice("compositeOriginal");
    let op = MODE_OF[ctx.choice("glowOperation").min(16) as usize + 1];
    map_px(&mut b.img, |x, y, o| {
        let g = spread.px[y * spread.width + x].map(|v| v * intensity);
        let mut out = match how {
            // only where the layer is not
            1 => {
                let k = 1.0 - o[3];
                [0, 1, 2, 3].map(|c| o[c] + g[c] * k)
            }
            2 => g,
            _ if op != veymelo_color::BlendMode::Add => veymelo_color::blend(op, o, g, 0.0),
            _ => [o[0] + g[0], o[1] + g[1], o[2] + g[2], (o[3] + g[3] * (1.0 - o[3])).min(1.0)],
        };
        out[3] = out[3].clamp(0.0, 1.0);
        out
    });
    b
}

effect!(
    MOSAIC_DEF,
    "mosaic",
    "Mosaic",
    "the layer as a grid of blocks, each one colour",
    [
        ranged("horizontalBlocks", "Horizontal Blocks", 10.0, 1.0, 4000.0),
        ranged("verticalBlocks", "Vertical Blocks", 10.0, 1.0, 4000.0),
        flag("sharpColors", "Sharp Colors", false).doc("each block the colour at its middle, not the mean of it"),
    ]
);
pub const MOSAIC: Spec = Spec::new(&MOSAIC_DEF, CATEGORY, mosaic);

fn mosaic(ctx: &Ctx, mut b: Buf) -> Buf {
    let (lx, ly, lw, lh) = rect_px(ctx, &b);
    let (nx, ny) = (ctx.num("horizontalBlocks", 10.0).round().max(1.0) as usize, ctx.num("verticalBlocks", 10.0).round().max(1.0) as usize);
    let (cw, ch) = (lw / nx as f64, lh / ny as f64);
    let (w, h) = (b.img.width, b.img.height);
    // which block a pixel is in (None past the layer)
    let block = |x: usize, y: usize| -> Option<(usize, usize)> {
        let (px, py) = (x as f64 + 0.5 - lx, y as f64 + 0.5 - ly);
        if px < 0.0 || py < 0.0 || px >= lw || py >= lh {
            return None;
        }
        Some((((px / cw) as usize).min(nx - 1), ((py / ch) as usize).min(ny - 1)))
    };
    let src = b.img.clone();
    if ctx.flag("sharpColors") {
        map_px(&mut b.img, |x, y, p| match block(x, y) {
            Some((i, j)) => src.sample(Sampling::Nearest, lx + (i as f64 + 0.5) * cw, ly + (j as f64 + 0.5) * ch),
            None => p,
        });
        return b;
    }
    let mut sums = vec![[0f64; 5]; nx * ny];
    for y in 0..h {
        for x in 0..w {
            if let Some((i, j)) = block(x, y) {
                let (p, s) = (src.px[y * w + x], &mut sums[j * nx + i]);
                for c in 0..4 {
                    s[c] += p[c] as f64;
                }
                s[4] += 1.0;
            }
        }
    }
    map_px(&mut b.img, |x, y, p| match block(x, y) {
        Some((i, j)) => {
            let s = sums[j * nx + i];
            [0, 1, 2, 3].map(|c| (s[c] / s[4].max(1.0)) as f32)
        }
        None => p,
    });
    b
}

effect!(POSTERIZE_DEF, "posterize", "Posterize", "each channel in so many steps", [ranged("level", "Level", 6.0, 2.0, 255.0),]);
pub const POSTERIZE: Spec = Spec::new(&POSTERIZE_DEF, CATEGORY, posterize);

fn posterize(ctx: &Ctx, mut b: Buf) -> Buf {
    let steps = ctx.num("level", 6.0).round().clamp(2.0, 255.0) as f32 - 1.0;
    crate::util::map_straight(&mut b.img, |c| c.map(|v| (v.clamp(0.0, 1.0) * steps).round() / steps));
    b
}

effect!(THRESHOLD_DEF, "threshold", "Threshold", "white where the layer is brighter than the level, black where not", [ranged("level", "Level", 128.0, 0.0, 255.0),]);
pub const THRESHOLD: Spec = Spec::new(&THRESHOLD_DEF, CATEGORY, threshold);

fn threshold(ctx: &Ctx, mut b: Buf) -> Buf {
    let t = (ctx.num("level", 128.0) / 255.0) as f32;
    crate::util::map_straight(&mut b.img, |c| [if luma(c) >= t { 1.0 } else { 0.0 }; 3]);
    b
}

/// The pixel's brightness, its edge pixels going on past the picture.
fn bright_at(img: &Image, x: i64, y: i64) -> f32 {
    let p = img.get(x.clamp(0, img.width as i64 - 1), y.clamp(0, img.height as i64 - 1));
    luma(unpremul(p).0)
}

effect!(
    FIND_EDGES_DEF,
    "findEdges",
    "Find Edges",
    "the edges in the picture drawn dark on white (or, inverted, light on black)",
    [flag("invert", "Invert", false), pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),]
);
pub const FIND_EDGES: Spec = Spec::new(&FIND_EDGES_DEF, CATEGORY, find_edges);

fn find_edges(ctx: &Ctx, mut b: Buf) -> Buf {
    let invert = ctx.flag("invert");
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0) as f32;
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, p| {
        let l = |dx: i64, dy: i64| bright_at(&src, x as i64 + dx, y as i64 + dy);
        let gx = l(1, -1) + 2.0 * l(1, 0) + l(1, 1) - l(-1, -1) - 2.0 * l(-1, 0) - l(-1, 1);
        let gy = l(-1, 1) + 2.0 * l(0, 1) + l(1, 1) - l(-1, -1) - 2.0 * l(0, -1) - l(1, -1);
        let e = gx.hypot(gy).min(1.0);
        let v = if invert { e } else { 1.0 - e };
        mix4([v * p[3], v * p[3], v * p[3], p[3]], p, keep)
    });
    b
}

effect!(
    EMBOSS_DEF,
    "emboss",
    "Emboss",
    "the picture pressed into grey metal, lit from a direction",
    [
        angle("direction", "Direction", 45.0),
        ranged("relief", "Relief", 1.5, 0.0, 10.0),
        ranged("contrast", "Contrast", 100.0, 0.0, 500.0),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
    ]
);
pub const EMBOSS: Spec = Spec::new(&EMBOSS_DEF, CATEGORY, emboss);

effect!(
    COLOR_EMBOSS_DEF,
    "colorEmboss",
    "Color Emboss",
    "the picture pressed in, lit from a direction, keeping its colours",
    [
        angle("direction", "Direction", 45.0),
        ranged("relief", "Relief", 1.5, 0.0, 10.0),
        ranged("contrast", "Contrast", 100.0, 0.0, 500.0),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
    ]
);
pub const COLOR_EMBOSS: Spec = Spec::new(&COLOR_EMBOSS_DEF, CATEGORY, color_emboss);

/// A step of `relief` along `direction`, in the buffer's pixels.
fn relief_step(ctx: &Ctx, scale: f64) -> (f64, f64) {
    let a = ctx.num("direction", 45.0).to_radians();
    let r = ctx.num("relief", 1.5) * scale;
    (a.cos() * r, -a.sin() * r)
}

/// The picture's straight colours a step toward the light and a step away from it, its edge
/// pixels going on. Brighter is higher: a slope rising away from the light faces it.
fn relief_pair(src: &Image, x: usize, y: usize, (dx, dy): (f64, f64)) -> ([f32; 3], [f32; 3]) {
    let (w, h) = (src.width as f64, src.height as f64);
    let at = |px: f64, py: f64| unpremul(src.sample(Sampling::Bilinear, px.clamp(0.5, w - 0.5), py.clamp(0.5, h - 0.5))).0;
    (at(x as f64 + 0.5 + dx, y as f64 + 0.5 + dy), at(x as f64 + 0.5 - dx, y as f64 + 0.5 - dy))
}

fn emboss(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = (ctx.num("contrast", 100.0) / 100.0) as f32;
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0) as f32;
    let src = b.img.clone();
    let step = relief_step(ctx, b.scale);
    map_px(&mut b.img, |x, y, p| {
        let (toward, away) = relief_pair(&src, x, y, step);
        let v = (0.5 + (luma(away) - luma(toward)) * k).clamp(0.0, 1.0);
        mix4([v * p[3], v * p[3], v * p[3], p[3]], p, keep)
    });
    b
}

fn color_emboss(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = (ctx.num("contrast", 100.0) / 100.0) as f32;
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0) as f32;
    let src = b.img.clone();
    let step = relief_step(ctx, b.scale);
    map_px(&mut b.img, |x, y, p| {
        let (c, al) = unpremul(p);
        if al <= 0.0 {
            return p;
        }
        let (toward, away) = relief_pair(&src, x, y, step);
        let e = [0, 1, 2].map(|i| (c[i] + (away[i] - toward[i]) * k).max(0.0));
        premul(mix3(e, c, keep), al)
    });
    b
}

effect!(
    CARTOON_DEF,
    "cartoon",
    "Cartoon",
    "the picture as a drawing: flat smoothed colours in steps, with inked edges",
    [
        choice("render", "Render", &["fill", "edges", "fillAndEdges"], 2),
        px("detailRadius", "Detail Radius", 8.0, 50.0).doc("how far it smooths, keeping edges"),
        pct("detailThreshold", "Detail Threshold", 10.0, 0.0, 100.0).doc("how big a change counts as an edge to keep"),
        ranged("shadingSteps", "Shading Steps", 8.0, 1.0, 64.0),
        pct("shadingSmoothness", "Shading Smoothness", 70.0, 0.0, 100.0),
        ranged("edgeThreshold", "Edge Threshold", 1.5, 0.0, 5.0),
        ranged("edgeWidth", "Edge Width", 1.5, 0.0, 10.0),
        pct("edgeSoftness", "Edge Softness", 60.0, 0.0, 100.0),
        pct("edgeOpacity", "Edge Opacity", 100.0, 0.0, 100.0),
        ranged("edgeEnhancement", "Edge Enhancement", 0.0, -100.0, 100.0).doc("above 0 the changes are made sharper, below softer"),
        pct("edgeBlackLevel", "Edge Black Level", 0.0, 0.0, 100.0),
        ranged("edgeContrast", "Edge Contrast", 0.5, 0.0, 1.0),
    ]
);
pub const CARTOON: Spec = Spec::new(&CARTOON_DEF, CATEGORY, cartoon);

/// The picture smoothed but for its edges: each pixel a straight function of the brightness
/// round it, fitted over a square `r` each way (a guided filter, guided by the brightness), so
/// flat parts flatten and edges bigger than `eps` stay.
fn smooth_keeping_edges(img: &Image, r: usize, eps: f32) -> Image {
    let (w, h) = (img.width, img.height);
    let size = (2 * r + 1) as f64;
    let mean = |m: &Image| filter::boxes(m, size, size, 1, Edge::Repeat);
    let guide: Vec<f32> = img.px.iter().map(|p| luma(unpremul(*p).0) * p[3]).collect();
    // the brightness and its square, then each channel and it times the brightness
    let mut one = Image::new(w, h);
    let mut two = Image::new(w, h);
    let mut three = Image::new(w, h);
    for (k, p) in img.px.iter().enumerate() {
        let g = guide[k];
        one.px[k] = [g, g * g, p[3], 0.0];
        two.px[k] = [p[0], p[1], p[2], 0.0];
        three.px[k] = [p[0] * g, p[1] * g, p[2] * g, 0.0];
    }
    let (m1, m2, m3) = (mean(&one), mean(&two), mean(&three));
    let mut ka = Image::new(w, h);
    let mut kb = Image::new(w, h);
    for k in 0..w * h {
        let (mg, mgg) = (m1.px[k][0], m1.px[k][1]);
        let var = (mgg - mg * mg).max(0.0);
        for c in 0..3 {
            let a = (m3.px[k][c] - mg * m2.px[k][c]) / (var + eps);
            ka.px[k][c] = a;
            kb.px[k][c] = m2.px[k][c] - a * mg;
        }
    }
    let (ma, mb) = (mean(&ka), mean(&kb));
    let mut out = img.clone();
    for (k, p) in out.px.iter_mut().enumerate() {
        for (c, v) in p.iter_mut().take(3).enumerate() {
            *v = (ma.px[k][c] * guide[k] + mb.px[k][c]).max(0.0);
        }
    }
    out
}

fn cartoon(ctx: &Ctx, mut b: Buf) -> Buf {
    let render = ctx.choice("render");
    let r = (ctx.num("detailRadius", 8.0) * b.scale).round().max(0.0) as usize;
    let detail = (ctx.num("detailThreshold", 10.0) / 100.0) as f32;
    let steps = ctx.num("shadingSteps", 8.0).max(1.0) as f32;
    let smooth = (ctx.num("shadingSmoothness", 70.0) / 100.0) as f32;
    let et = ctx.num("edgeThreshold", 1.5) as f32;
    let ew = ctx.num("edgeWidth", 1.5) * b.scale;
    let es = (ctx.num("edgeSoftness", 60.0) / 100.0) as f32;
    let eo = (ctx.num("edgeOpacity", 100.0) / 100.0) as f32;
    let black = (ctx.num("edgeBlackLevel", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    let contrast = ((ctx.num("edgeContrast", 0.5).clamp(0.0, 0.999) * std::f64::consts::FRAC_PI_2).tan() as f32).min(100.0);
    let grey = move |e: f32| (((1.0 - e) - black).abs() - 0.5) * contrast + 0.5;
    let mut flat = if r > 0 { smooth_keeping_edges(&b.img, r, detail * detail * 0.25 + 1e-5) } else { b.img.clone() };
    let enhance = ctx.num("edgeEnhancement", 0.0) / 100.0;
    if enhance < 0.0 {
        let s = 1.5 * b.scale;
        let soft = filter::gaussian(&flat, s, s, Edge::Repeat);
        let k = (-enhance) as f32;
        for (p, q) in flat.px.iter_mut().zip(&soft.px) {
            *p = mix4(*p, *q, k);
        }
    } else if enhance > 0.0 {
        // each pixel taken from the side of the nearest edge it belongs to (a shock filter step)
        let (w, h) = (flat.width, flat.height);
        let l: Vec<f32> = flat.px.iter().map(|p| luma(unpremul(*p).0)).collect();
        let g = |x: i64, y: i64| l[(y.clamp(0, h as i64 - 1) as usize) * w + x.clamp(0, w as i64 - 1) as usize];
        let s = enhance * 2.0 * b.scale;
        let src = flat.clone();
        map_px(&mut flat, |x, y, p| {
            let (x, y) = (x as i64, y as i64);
            let (gx, gy) = ((g(x + 1, y) - g(x - 1, y)) * 0.5, (g(x, y + 1) - g(x, y - 1)) * 0.5);
            let lap = g(x + 1, y) + g(x - 1, y) + g(x, y + 1) + g(x, y - 1) - 4.0 * g(x, y);
            let m = gx.hypot(gy);
            if m < 1e-4 || lap.abs() < 1e-5 {
                return p;
            }
            let k = -lap.signum() as f64 * s / m as f64;
            src.sample(Sampling::Bilinear, (x as f64 + 0.5 + gx as f64 * k).clamp(0.5, w as f64 - 0.5), (y as f64 + 0.5 + gy as f64 * k).clamp(0.5, h as f64 - 0.5))
        });
    }
    let (w, h) = (flat.width, flat.height);
    let edges: Option<Vec<f32>> = (render >= 1).then(|| {
        let l: Vec<f32> = flat.px.iter().map(|p| luma(unpremul(*p).0)).collect();
        let g = |x: i64, y: i64| l[(y.clamp(0, h as i64 - 1) as usize) * w + x.clamp(0, w as i64 - 1) as usize];
        let (t1, t0) = (et * 0.2, et * 0.2 * (1.0 - es * 0.9));
        let e: Vec<f32> = (0..w * h)
            .map(|k| {
                let (x, y) = ((k % w) as i64, (k / w) as i64);
                let gx = g(x + 1, y - 1) + 2.0 * g(x + 1, y) + g(x + 1, y + 1) - g(x - 1, y - 1) - 2.0 * g(x - 1, y) - g(x - 1, y + 1);
                let gy = g(x - 1, y + 1) + 2.0 * g(x, y + 1) + g(x + 1, y + 1) - g(x - 1, y - 1) - 2.0 * g(x, y - 1) - g(x + 1, y - 1);
                smoothstep(t0, t1.max(t0 + 1e-4), gx.hypot(gy))
            })
            .collect();
        if ew > 1.0 {
            // thickened: each pixel the strongest edge within the width
            let reach = ((ew - 1.0) * 0.5).ceil() as i64;
            let rr = ((ew - 1.0) * 0.5) as f32;
            (0..w * h)
                .map(|k| {
                    let (x, y) = ((k % w) as i64, (k / w) as i64);
                    let mut m = 0f32;
                    for dy in -reach..=reach {
                        for dx in -reach..=reach {
                            let d = ((dx * dx + dy * dy) as f32).sqrt();
                            let fall = (rr - d + 1.0).clamp(0.0, 1.0);
                            if fall > 0.0 {
                                let (nx, ny) = (x + dx, y + dy);
                                if nx >= 0 && ny >= 0 && nx < w as i64 && ny < h as i64 {
                                    m = m.max(e[ny as usize * w + nx as usize] * fall);
                                }
                            }
                        }
                    }
                    m
                })
                .collect()
        } else {
            e.iter().map(|v| v * ew.max(0.0) as f32).collect()
        }
    });
    map_px(&mut b.img, |x, y, p| {
        let a = p[3];
        if a <= 0.0 {
            return p;
        }
        let k = y * w + x;
        let (c, _) = unpremul(flat.px[k]);
        let fill = {
            let l = luma(c);
            let t = l.max(0.0) * steps;
            let f = t - t.floor();
            let f2 = if smooth <= 0.0 { if f >= 0.5 { 1.0 } else { 0.0 } } else { smoothstep(0.5 - smooth * 0.5, 0.5 + smooth * 0.5, f) };
            let lq = (t.floor() + f2) / steps;
            if l > 1e-4 { c.map(|v| v * lq / l) } else { [lq; 3] }
        };
        let g = grey(edges.as_ref().map_or(0.0, |e| e[k] * eo)).clamp(0.0, 1.0);
        premul(
            match render {
                0 => fill,
                1 => [g; 3],
                _ => fill.map(|v| v * g),
            },
            a,
        )
    });
    b
}

const EDGE_TYPES: &[&str] = &["roughen", "roughenColor", "cut", "spiky", "rusty", "rustyColor", "photocopy", "photocopyColor"];

effect!(
    ROUGHEN_DEF,
    "roughenEdges",
    "Roughen Edges",
    "the layer's edges worn rough, eaten away, rusty or photocopied",
    [
        choice("edgeType", "Edge Type", EDGE_TYPES, 0),
        color("edgeColor", "Edge Color", 0.55, 0.3, 0.1).doc("for the colour types"),
        px("border", "Border", 8.0, 500.0).doc("how far in from the edge it reaches"),
        ranged("edgeSharpness", "Edge Sharpness", 1.0, 0.0, 10.0),
        ranged("fractalInfluence", "Fractal Influence", 1.0, 0.0, 1.0),
        ranged("scale", "Scale", 100.0, 10.0, 1000.0),
        ranged("stretchWidthOrHeight", "Stretch Width or Height", 0.0, -10.0, 10.0),
        point("offset", "Offset (Turbulence)"),
        ranged("complexity", "Complexity", 2.0, 1.0, 10.0),
        angle("evolution", "Evolution", 0.0),
        flag("cycleEvolution", "Cycle Evolution", false),
        num("cycle", "Cycle (in Revolutions)", 1.0).range(1.0, 1000.0),
        num("randomSeed", "Random Seed", 0.0).range(0.0, 1_000_000.0),
    ]
);
pub const ROUGHEN: Spec = Spec::new(&ROUGHEN_DEF, CATEGORY, roughen);

fn roughen(ctx: &Ctx, mut b: Buf) -> Buf {
    let kind = ctx.choice("edgeType");
    let edge_c = rgb3(ctx.color("edgeColor"));
    let border = ctx.num("border", 8.0) * b.scale;
    let sharp = ctx.num("edgeSharpness", 1.0).max(0.01) as f32;
    let influence = ctx.num("fractalInfluence", 1.0) as f32;
    let size = (ctx.num("scale", 100.0) * b.scale * 0.1).max(0.5);
    let off = b.pixel(ctx.point("offset"));
    let octaves = ctx.num("complexity", 2.0);
    let z = ctx.num("evolution", 0.0) / 360.0;
    let st = ctx.num("stretchWidthOrHeight", 0.0);
    let (sx, sy) = (1.0 + st.max(0.0), 1.0 + (-st).max(0.0));
    let period = ctx.flag("cycleEvolution").then(|| ctx.num("cycle", 1.0).round().max(1.0) as i64);
    // how fine, how strong and how sharp the roughness is, and whether coloured, by type
    let (fine, strong, sharper, coloured) = match kind {
        1 => (1.0, 1.0, 1.0, true),
        2 => (1.0, 1.0, 3.0, false),
        3 => (3.0, 1.0, 1.5, false),
        4 => (1.5, 1.5, 1.0, false),
        5 => (1.5, 1.5, 1.0, true),
        6 => (0.6, 1.2, 2.5, false),
        7 => (0.6, 1.2, 2.5, true),
        _ => (1.0, 1.0, 1.0, false),
    };
    let extra = if kind == 4 || kind == 5 { 1.0 } else { 0.0 };
    let (w, h) = (b.img.width, b.img.height);
    let seed = crate::noise::mix(&[0x40f1, ctx.num("randomSeed", 0.0).max(0.0) as u64]);
    if veymelo_raster::gpu::wanted(w * h)
        && let Some(oct) = crate::noise::gpu_octaves(octaves + extra, 0.5)
    {
        use crate::noise::split;
        use veymelo_raster::gpu;
        let p = GpuRoughen {
            w: w as u32,
            h: h as u32,
            coloured: coloured as u32,
            pad: 0,
            off_x: split(off.x),
            off_y: split(off.y),
            kx: split(fine / (size * sx)),
            ky: split(fine / (size * sy)),
            influence,
            strong,
            sharp: sharp * sharper,
            edge_r: edge_c[0],
            edge_g: edge_c[1],
            edge_b: edge_c[2],
            seed,
            z: crate::noise::gpu_depth(z, period),
        };
        let source = gpu::program(&[gpu::COMMON, include_str!("noise.metal"), include_str!("stylize.metal")]);
        let mut ops = vec![gpu::Op::new(gpu::Kernel { source, name: "alpha_of" }, &(w as u32), [w, h])];
        if border > 0.05 {
            ops.extend(gpu::gaussian_ops(w, h, border * 0.5, border * 0.5, Edge::Clear));
        }
        ops.push(gpu::Op { original: true, ..gpu::Op::new(gpu::Kernel { source, name: "roughen" }, &p, [w, h]).with(&[oct]) });
        if let Some(img) = gpu::apply(&b.img, &ops) {
            b.img = img;
            return b;
        }
    }
    // how far in from the edge: the alpha softened over the border
    let mut alpha = Image::new(w, h);
    for (o, p) in alpha.px.iter_mut().zip(&b.img.px) {
        *o = [p[3]; 4];
    }
    let depth = if border > 0.05 { filter::gaussian(&alpha, border * 0.5, border * 0.5, Edge::Clear) } else { alpha };
    map_px(&mut b.img, |x, y, p| {
        if p[3] <= 0.0 {
            return p;
        }
        let dv = depth.px[y * w + x][0];
        let (u, v) = ((x as f64 + 0.5 - off.x) / (size * sx) * fine, (y as f64 + 0.5 - off.y) / (size * sy) * fine);
        let n = (crate::noise::fractal(u, v, z, seed, octaves + extra, 0.5, period) * 0.5) as f32;
        let s = dv - 0.5 + n * influence * strong * 0.5;
        let m = (s * sharp * sharper * 4.0 + 0.5).clamp(0.0, 1.0);
        let (mut c, a) = unpremul(p);
        if coloured {
            c = mix3(c, edge_c, 1.0 - smoothstep(0.5, 0.9, dv + n * influence * 0.5));
        }
        premul(c, a * m)
    });
    b
}

/// Roughen Edges' settings as the GPU takes them (stylize.metal's Roughen).
#[repr(C)]
#[derive(Clone, Copy)]
struct GpuRoughen {
    w: u32,
    h: u32,
    coloured: u32,
    pad: u32,
    off_x: [f32; 2],
    off_y: [f32; 2],
    kx: [f32; 2],
    ky: [f32; 2],
    influence: f32,
    strong: f32,
    sharp: f32,
    edge_r: f32,
    edge_g: f32,
    edge_b: f32,
    seed: u64,
    z: crate::noise::GpuDepth,
}

unsafe impl veymelo_raster::gpu::Plain for GpuRoughen {}

effect!(
    SCATTER_DEF,
    "scatter",
    "Scatter",
    "the layer's pixels scattered a little at random, like frosted glass",
    [
        ranged("scatterAmount", "Scatter Amount", 0.0, 0.0, 127.0),
        choice("grain", "Grain", &["both", "horizontal", "vertical"], 0),
        flag("scatterRandomness", "Scatter Randomness", false).doc("scattered afresh every frame"),
    ]
);
pub const SCATTER: Spec = Spec::new(&SCATTER_DEF, CATEGORY, scatter);

fn scatter(ctx: &Ctx, mut b: Buf) -> Buf {
    let amount = ctx.num("scatterAmount", 0.0) * b.scale;
    if amount <= 0.0 {
        return b;
    }
    let grain = ctx.choice("grain");
    let seed = if ctx.flag("scatterRandomness") { crate::noise::mix(&[ctx.seed, ctx.time.seconds().to_bits()]) } else { ctx.seed };
    let src = b.img.clone();
    // pixels of the layer (its units) so a preview scatters alike
    let (s, o) = (b.scale, b.origin);
    map_px(&mut b.img, |x, y, _| {
        let (lx, ly) = ((o.x + (x as f64 + 0.5) / s).floor() as i64 as u64, (o.y + (y as f64 + 0.5) / s).floor() as i64 as u64);
        let rnd = |k: u64| crate::noise::unit(crate::noise::mix(&[seed, lx, ly, k])) * 2.0 - 1.0;
        let dx = if grain == 2 { 0.0 } else { rnd(0) * amount };
        let dy = if grain == 1 { 0.0 } else { rnd(1) * amount };
        src.sample(Sampling::Nearest, x as f64 + 0.5 + dx, y as f64 + 0.5 + dy)
    });
    b
}

effect!(
    STROBE_DEF,
    "strobeLight",
    "Strobe Light",
    "the layer flashing a colour (or turning see-through) at times, regularly or at random",
    [
        color("strobeColor", "Strobe Color", 1.0, 1.0, 1.0),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
        ranged("strobeDuration", "Strobe Duration (secs)", 0.05, 0.0, 30.0),
        ranged("strobePeriod", "Strobe Period (secs)", 1.0, 0.0, 30.0).doc("from the start of one flash to the next"),
        pct("randomStrobeProbability", "Random Strobe Probability", 0.0, 0.0, 100.0),
        choice("strobe", "Strobe", &["operatesOnColorOnly", "makesLayerTransparent"], 0),
        choice("strobeOperator", "Strobe Operator", &["copy", "add", "subtract", "multiply", "difference", "screen", "lighten", "darken"], 0),
    ]
);
pub const STROBE: Spec = Spec::new(&STROBE_DEF, CATEGORY, strobe);

/// Whether the strobe is flashing at `t` (seconds): in the first `duration` of each `period`, or
/// at random in a whole period with chance `chance`.
pub fn flashing(t: f64, duration: f64, period: f64, chance: f64, seed: u64) -> bool {
    if period <= 0.0 {
        return false;
    }
    let k = (t / period).floor();
    t - k * period < duration || (chance > 0.0 && crate::noise::unit(crate::noise::mix(&[seed, k as i64 as u64])) < chance)
}

fn strobe(ctx: &Ctx, mut b: Buf) -> Buf {
    let on = flashing(ctx.time.seconds(), ctx.num("strobeDuration", 0.05), ctx.num("strobePeriod", 1.0), ctx.num("randomStrobeProbability", 0.0) / 100.0, ctx.seed);
    if !on {
        return b;
    }
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    if ctx.choice("strobe") == 1 {
        for p in &mut b.img.px {
            *p = p.map(|v| v * keep);
        }
        return b;
    }
    let s = rgb3(ctx.color("strobeColor"));
    let op = ctx.choice("strobeOperator");
    crate::util::map_straight(&mut b.img, |c| {
        let o = [0, 1, 2].map(|i| {
            let (v, k) = (c[i], s[i]);
            match op {
                1 => v + k,
                2 => (v - k).max(0.0),
                3 => v * k,
                4 => (v - k).abs(),
                5 => 1.0 - (1.0 - v) * (1.0 - k),
                6 => v.max(k),
                7 => v.min(k),
                _ => k,
            }
        });
        mix3(o, c, keep)
    });
    b
}

effect!(
    TEXTURIZE_DEF,
    "texturize",
    "Texturize",
    "another layer's texture pressed into this one, lit from a direction",
    [
        layer("textureLayer", "Texture Layer"),
        angle("lightDirection", "Light Direction", 135.0),
        ranged("textureContrast", "Texture Contrast", 1.0, 0.0, 2.0),
        choice("texturePlacement", "Texture Placement", &["tileTexture", "centerTexture", "stretchTextureToFit"], 0),
    ]
);
pub const TEXTURIZE: Spec = Spec::new(&TEXTURIZE_DEF, CATEGORY, texturize);

fn texturize(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = ctx.num("textureContrast", 1.0) as f32;
    if k == 0.0 {
        return b;
    }
    let fit = [Fit::Tile, Fit::Centre, Fit::Stretch][ctx.choice("texturePlacement").min(2) as usize];
    let Some((tex, _)) = ctx.layer_placed("textureLayer", &b, Take::All, fit) else { return b };
    let a = ctx.num("lightDirection", 135.0).to_radians();
    let (lx, ly) = (a.cos() as f32, -a.sin() as f32);
    let (w, h) = (tex.width, tex.height);
    let mut plane = Image::new(w, h);
    for (o, p) in plane.px.iter_mut().zip(&tex.px) {
        *o = [luma(unpremul(*p).0) * p[3]; 4];
    }
    let s = 0.7 * b.scale;
    let height: Vec<f32> = filter::gaussian(&plane, s, s, Edge::Repeat).px.iter().map(|p| p[0]).collect();
    let at = |x: i64, y: i64| height[(y.clamp(0, h as i64 - 1) as usize) * w + x.clamp(0, w as i64 - 1) as usize];
    map_px(&mut b.img, |x, y, p| {
        let (x, y) = (x as i64, y as i64);
        let (gx, gy) = ((at(x + 1, y) - at(x - 1, y)) * 0.5, (at(x, y + 1) - at(x, y - 1)) * 0.5);
        let shade = (1.0 - k * 4.0 * (gx * lx + gy * ly)).max(0.0);
        [p[0] * shade, p[1] * shade, p[2] * shade, p[3]]
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 12] = [GLOW, MOSAIC, POSTERIZE, THRESHOLD, FIND_EDGES, EMBOSS, COLOR_EMBOSS, CARTOON, ROUGHEN, SCATTER, STROBE, TEXTURIZE];

#[cfg(test)]
mod tests;
