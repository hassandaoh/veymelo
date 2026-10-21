//! Blur and sharpen: Gaussian, box, along a direction, round or toward a centre (and zoomed or
//! turned five ways), per channel, by a map, keeping edges (bilateral, smart), crossed, by the
//! picture's own shape (vector), through a lens (bokeh), sharpening, and a shaken camera's blur
//! taken away with sharper neighbouring frames.
//!
//! Blurriness is given in layer pixels and is about twice the Gaussian's deviation, as in
//! compositing tools; the blur is drawn at the buffer's pixels to a unit, so a preview blurs
//! like the full picture. Without Repeat Edge Pixels a blur spreads past the layer's edges into
//! clear pixels; under an adjustment layer the picture's edges repeat.

use veymelo_project::{PropDef, Unit};
use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px, Sampling, par};

use crate::defs::{choice, effect, flag, layer, num, point, px};
use crate::util::{luma, luma_px, map_px, premul, tidy, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "blur";
const DIMENSIONS: &[&str] = &["both", "horizontal", "vertical"];
const REPEAT: PropDef = flag("repeatEdge", "Repeat Edge Pixels", false).doc("blur as if the edge pixels went on, so the edges stay sharp and solid");
const DIMS: PropDef = choice("dimensions", "Blur Dimensions", DIMENSIONS, 0).doc("blur across and down, or one of them");

/// A blurriness in layer pixels as a Gaussian deviation in buffer pixels.
fn deviation(blurriness: f64, b: &Buf) -> f64 {
    blurriness.max(0.0) * 0.5 * b.scale
}

/// Across and down, by the Blur Dimensions setting.
fn axes(dimensions: u32) -> (f64, f64) {
    match dimensions {
        1 => (1.0, 0.0),
        2 => (0.0, 1.0),
        _ => (1.0, 1.0),
    }
}

/// What lies past the edges, and room made for the blur to spread into when that is nothing.
fn edges(ctx: &Ctx, b: &mut Buf, repeat: bool, reach: f64) -> Edge {
    if repeat || ctx.adjustment {
        Edge::Repeat
    } else {
        b.pad(reach.ceil() as usize + 1);
        Edge::Clear
    }
}

effect!(GAUSSIAN_DEF, "gaussianBlur", "Gaussian Blur", "blurs evenly, softly", [px("blurriness", "Blurriness", 0.0, 3000.0).doc("how far it spreads"), DIMS, REPEAT]);
pub const GAUSSIAN: Spec = Spec::new(&GAUSSIAN_DEF, CATEGORY, gaussian);

fn gaussian(ctx: &Ctx, mut b: Buf) -> Buf {
    let s = deviation(ctx.num("blurriness", 0.0), &b);
    if s < 0.01 {
        return b;
    }
    let (kx, ky) = axes(ctx.choice("dimensions"));
    let e = edges(ctx, &mut b, ctx.flag("repeatEdge"), 4.0 * s);
    b.img = filter::gaussian(&b.img, s * kx, s * ky, e);
    b
}

effect!(
    FAST_BOX_DEF,
    "fastBoxBlur",
    "Fast Box Blur",
    "blurs with boxes: once is square and hard, three times is close to Gaussian",
    [
        px("radius", "Blur Radius", 0.0, 3000.0),
        num("iterations", "Iterations", 3.0).range(1.0, 50.0).doc("how many times the box is run"),
        DIMS,
        REPEAT,
    ]
);
pub const FAST_BOX: Spec = Spec::new(&FAST_BOX_DEF, CATEGORY, fast_box);

fn fast_box(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = ctx.num("radius", 0.0).max(0.0) * b.scale;
    if r < 0.01 {
        return b;
    }
    let n = ctx.num("iterations", 3.0).round().clamp(1.0, 50.0) as usize;
    let (kx, ky) = axes(ctx.choice("dimensions"));
    let e = edges(ctx, &mut b, ctx.flag("repeatEdge"), r * n as f64);
    let width = 2.0 * r + 1.0;
    b.img = filter::boxes(&b.img, if kx > 0.0 { width } else { 0.0 }, if ky > 0.0 { width } else { 0.0 }, n, e);
    b
}

effect!(
    DIRECTIONAL_DEF,
    "directionalBlur",
    "Directional Blur",
    "blurs along one direction, as if moving fast",
    [num("direction", "Direction", 0.0).unit(Unit::Degrees).doc("0 is up and down, turning clockwise"), px("length", "Blur Length", 0.0, 1000.0)]
);
pub const DIRECTIONAL: Spec = Spec::new(&DIRECTIONAL_DEF, CATEGORY, directional);

fn directional(ctx: &Ctx, mut b: Buf) -> Buf {
    let len = ctx.num("length", 0.0).max(0.0) * b.scale;
    if len < 0.5 {
        return b;
    }
    let a = ctx.num("direction", 0.0).to_radians();
    if !ctx.adjustment {
        b.pad((len / 2.0).ceil() as usize + 1);
    }
    b.img = filter::along(&b.img, (a.sin(), -a.cos()), len);
    b
}

effect!(
    RADIAL_DEF,
    "radialBlur",
    "Radial Blur",
    "blurs round a centre (spin) or toward it (zoom)",
    [
        num("amount", "Amount", 10.0).range(0.0, 100.0).doc("degrees of spin, or % of the distance for zoom"),
        point("center", "Center"),
        choice("type", "Type", &["spin", "zoom"], 0),
        choice("antialiasing", "Antialiasing (Best Quality)", &["low", "high"], 1).doc("kept for files from elsewhere: the blur is always smooth"),
    ]
);
pub const RADIAL: Spec = Spec::new(&RADIAL_DEF, CATEGORY, radial).points(&[("center", [0.5, 0.5])]);

fn radial(ctx: &Ctx, mut b: Buf) -> Buf {
    let amount = ctx.num("amount", 10.0).max(0.0);
    if amount <= 0.0 {
        return b;
    }
    let c = b.pixel(ctx.point("center"));
    let zoom = ctx.choice("type") == 1;
    b.img = filter::round(&b.img, (c.x, c.y), if zoom { amount / 100.0 } else { amount }, zoom);
    b
}

/// The picture with the difference from its blur added back `k` times (where the difference is
/// at least `threshold` of 255).
fn unsharp(img: &Image, sigma: f64, k: f32, threshold: f32) -> Image {
    let soft = filter::gaussian(img, sigma, sigma, Edge::Repeat);
    let mut out = img.clone();
    map_px(&mut out, |x, y, p| {
        let q = soft.px[y * img.width + x];
        let mut o = p;
        for c in 0..3 {
            let d = p[c] - q[c];
            if d.abs() * 255.0 >= threshold {
                o[c] = (p[c] + d * k).max(0.0);
            }
        }
        o
    });
    out
}

effect!(SHARPEN_DEF, "sharpen", "Sharpen", "makes edges crisper", [num("amount", "Sharpen Amount", 0.0).range(0.0, 500.0)]);
pub const SHARPEN: Spec = Spec::new(&SHARPEN_DEF, CATEGORY, sharpen);

fn sharpen(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = ctx.num("amount", 0.0).max(0.0) as f32 / 50.0;
    if k > 0.0 {
        b.img = unsharp(&b.img, b.scale.max(0.2), k, 0.0);
    }
    b
}

effect!(
    UNSHARP_DEF,
    "unsharpMask",
    "Unsharp Mask",
    "sharpens by adding back the difference from a blur",
    [
        num("amount", "Amount", 50.0).range(0.0, 500.0).unit(Unit::Percent),
        num("radius", "Radius", 1.0).range(0.1, 500.0).unit(Unit::Pixels).doc("how wide the edges it sharpens are"),
        num("threshold", "Threshold", 0.0).range(0.0, 255.0).doc("smaller differences than this (of 255) are left alone"),
    ]
);
pub const UNSHARP: Spec = Spec::new(&UNSHARP_DEF, CATEGORY, unsharp_mask);

fn unsharp_mask(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = ctx.num("amount", 50.0).max(0.0) as f32 / 100.0;
    if k > 0.0 {
        b.img = unsharp(&b.img, (ctx.num("radius", 1.0) * b.scale).max(0.1), k, ctx.num("threshold", 0.0) as f32);
    }
    b
}

effect!(
    CHANNEL_DEF,
    "channelBlur",
    "Channel Blur",
    "blurs red, green, blue and alpha each by its own amount",
    [
        px("redBlurriness", "Red Blurriness", 0.0, 3000.0),
        px("greenBlurriness", "Green Blurriness", 0.0, 3000.0),
        px("blueBlurriness", "Blue Blurriness", 0.0, 3000.0),
        px("alphaBlurriness", "Alpha Blurriness", 0.0, 3000.0),
        REPEAT,
        DIMS,
    ]
);
pub const CHANNEL: Spec = Spec::new(&CHANNEL_DEF, CATEGORY, channel);

fn channel(ctx: &Ctx, mut b: Buf) -> Buf {
    let sig = ["redBlurriness", "greenBlurriness", "blueBlurriness", "alphaBlurriness"].map(|id| deviation(ctx.num(id, 0.0), &b));
    if sig.iter().all(|s| *s < 0.01) {
        return b;
    }
    let (kx, ky) = axes(ctx.choice("dimensions"));
    let e = edges(ctx, &mut b, ctx.flag("repeatEdge"), 4.0 * sig.iter().copied().fold(0.0, f64::max));
    // each deviation blurs the picture once; a colour is its blurred value over the alpha blurred alike
    let mut blurred: Vec<(f64, Image)> = Vec::new();
    for s in sig {
        if !blurred.iter().any(|(t, _)| *t == s) {
            let img = if s < 0.01 { b.img.clone() } else { filter::gaussian(&b.img, s * kx, s * ky, e) };
            blurred.push((s, img));
        }
    }
    let with = |s: f64| &blurred.iter().find(|(t, _)| *t == s).expect("made").1;
    let (r, g, bl, a) = (with(sig[0]), with(sig[1]), with(sig[2]), with(sig[3]));
    map_px(&mut b.img, |x, y, _| {
        let i = y * r.width + x;
        let straight = |img: &Image, c: usize| if img.px[i][3] > 1e-6 { img.px[i][c] / img.px[i][3] } else { 0.0 };
        premul([straight(r, 0), straight(g, 1), straight(bl, 2)], a.px[i][3].clamp(0.0, 1.0))
    });
    b
}

effect!(
    COMPOUND_DEF,
    "compoundBlur",
    "Compound Blur",
    "blurs more where a map layer is brighter",
    [
        layer("blurLayer", "Blur Layer").doc("the map (none: the layer itself)"),
        px("maximumBlur", "Maximum Blur", 20.0, 1000.0).doc("the blur where the map is white"),
        flag("stretchMapToFit", "Stretch Map to Fit", true).doc("the map is stretched over the layer (else centred at its own size)"),
        flag("invertBlur", "Invert Blur", false),
    ]
);
pub const COMPOUND: Spec = Spec::new(&COMPOUND_DEF, CATEGORY, compound);

fn compound(ctx: &Ctx, mut b: Buf) -> Buf {
    let most = deviation(ctx.num("maximumBlur", 20.0), &b);
    if most < 0.01 {
        return b;
    }
    let e = edges(ctx, &mut b, false, 4.0 * most);
    let map = ctx.layer_over("blurLayer", &b, Take::All, ctx.flag("stretchMapToFit")).unwrap_or_else(|| b.img.clone());
    let invert = ctx.flag("invertBlur");
    // blurs at even steps up to the most; each pixel takes the two its map value falls between
    let steps = 6usize;
    let levels: Vec<Image> = (0..steps).map(|k| if k == 0 { b.img.clone() } else { filter::gaussian(&b.img, most * k as f64 / (steps - 1) as f64, most * k as f64 / (steps - 1) as f64, e) }).collect();
    map_px(&mut b.img, |x, y, _| {
        let i = y * map.width + x;
        let m = luma_px(map.px[i]).clamp(0.0, 1.0);
        let t = if invert { 1.0 - m } else { m } * (steps - 1) as f32;
        let k = (t.floor() as usize).min(steps - 2);
        let f = t - k as f32;
        let (p, q) = (levels[k].px[i], levels[k + 1].px[i]);
        [0, 1, 2, 3].map(|c| p[c] + (q[c] - p[c]) * f)
    });
    b
}

/// The mean of the neighbours within `r` pixels, each counted by how near it is and how like the
/// middle it is: a bell in both (bilateral), or a hard limit on the difference in brightness (smart).
fn keeping_edges(src: &Image, r: f64, range: f32, hard: bool) -> Image {
    keeping_edges_then(src, r, range, hard, None).unwrap_or_else(|| keeping_edges_here(src, r, range, hard))
}

/// [`keeping_edges`] on the GPU, then (if asked) a step after it (None: leave it to the
/// processor). The neighbours (each a step across and down, and how near it is) are listed here
/// as the processor goes through them.
fn keeping_edges_then(src: &Image, r: f64, range: f32, hard: bool, then: Option<veymelo_raster::gpu::Op>) -> Option<Image> {
    use veymelo_raster::gpu;
    if !gpu::wanted(src.px.len()) {
        return None;
    }
    let reach = r.ceil() as i64;
    let step = ((r / 12.0).ceil() as i64).max(1);
    let (near2, like2) = (2.0 * (r / 2.0).max(0.5).powi(2), 2.0 * range.max(1e-3).powi(2));
    let mut near: Vec<[f32; 4]> = Vec::new();
    let mut dy = -reach;
    while dy <= reach {
        let mut dx = -reach;
        while dx <= reach {
            let d2 = (dx * dx + dy * dy) as f64;
            if d2 <= r * r + 0.5 {
                near.push([dx as f32, dy as f32, (-d2 / near2) as f32, 0.0]);
            }
            dx += step;
        }
        dy += step;
    }
    let p = GpuKeeping { w: src.width as u32, h: src.height as u32, n: near.len() as u32, hard: hard as u32, grey: 0, range, like2, pad: 0.0 };
    let kernel = gpu::Kernel { source: gpu::program(&[gpu::COMMON, include_str!("blur.metal")]), name: "keeping_edges" };
    let mut ops = vec![gpu::Op::new(kernel, &p, [src.width, src.height]).with(&near)];
    ops.extend(then);
    gpu::apply(src, &ops)
}

/// Bilateral and Smart Blur's settings as the GPU takes them (blur.metal's Keeping).
#[repr(C)]
#[derive(Clone, Copy)]
struct GpuKeeping {
    w: u32,
    h: u32,
    n: u32,
    hard: u32,
    grey: u32,
    range: f32,
    like2: f32,
    pad: f32,
}

unsafe impl veymelo_raster::gpu::Plain for GpuKeeping {}

/// Smart Blur's edges as the GPU takes them (blur.metal's SmartEdges).
#[repr(C)]
#[derive(Clone, Copy)]
struct GpuSmartEdges {
    w: u32,
    h: u32,
    mode: u32,
    limit: f32,
}

unsafe impl veymelo_raster::gpu::Plain for GpuSmartEdges {}

fn keeping_edges_here(src: &Image, r: f64, range: f32, hard: bool) -> Image {
    let reach = r.ceil() as i64;
    // past a dozen pixels every few neighbours stand for the rest
    let step = ((r / 12.0).ceil() as i64).max(1);
    let (near2, like2) = (2.0 * (r / 2.0).max(0.5).powi(2), 2.0 * range.max(1e-3).powi(2));
    let (w, h) = (src.width as i64, src.height as i64);
    let mut out = Image::new(src.width, src.height);
    let ow = out.width;
    par::rows(&mut out.px, ow, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let c = src.px[y * src.width + x];
            let cl = luma_px(c);
            let (mut acc, mut total) = ([0f32; 4], 0f32);
            let mut dy = -reach;
            while dy <= reach {
                let mut dx = -reach;
                while dx <= reach {
                    let d2 = (dx * dx + dy * dy) as f64;
                    if d2 <= r * r + 0.5 {
                        let q = src.px[((y as i64 + dy).clamp(0, h - 1) * w + (x as i64 + dx).clamp(0, w - 1)) as usize];
                        let k = if hard {
                            if (luma_px(q) - cl).abs() <= range { 1.0 } else { 0.0 }
                        } else {
                            let diff: f32 = (0..4).map(|i| (q[i] - c[i]).powi(2)).sum();
                            ((-d2 / near2) as f32 - diff / like2).exp()
                        };
                        if k > 0.0 {
                            for i in 0..4 {
                                acc[i] += q[i] * k;
                            }
                            total += k;
                        }
                    }
                    dx += step;
                }
                dy += step;
            }
            *o = if total > 0.0 { acc.map(|v| v / total) } else { c };
        }
    });
    out
}

effect!(
    BILATERAL_DEF,
    "bilateralBlur",
    "Bilateral Blur",
    "smooths flat areas, keeping edges sharp",
    [
        px("radius", "Radius", 5.0, 200.0),
        num("threshold", "Threshold", 10.0).range(0.0, 100.0).doc("how different neighbours may be and still be blurred in"),
        flag("colorize", "Colorize", true).doc("off: the result is grey"),
    ]
);
pub const BILATERAL: Spec = Spec::new(&BILATERAL_DEF, CATEGORY, bilateral);

fn bilateral(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = ctx.num("radius", 5.0).max(0.0) * b.scale;
    if r < 0.5 {
        return b;
    }
    b.img = keeping_edges(&b.img, r, ctx.num("threshold", 10.0) as f32 / 100.0, false);
    if !ctx.flag("colorize") {
        map_px(&mut b.img, |_, _, p| {
            let (c, a) = unpremul(p);
            premul([luma(c); 3], a)
        });
    }
    b
}

/// How strongly the brightness changes at each pixel (Sobel).
fn edge_strength(img: &Image) -> Vec<f32> {
    let (w, h) = (img.width as i64, img.height as i64);
    let l: Vec<f32> = img.px.iter().map(|p| luma_px(*p)).collect();
    let at = |x: i64, y: i64| l[(y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize];
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let gx = at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x - 1, y) - at(x - 1, y + 1);
            let gy = at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x, y - 1) - at(x + 1, y - 1);
            (gx * gx + gy * gy).sqrt() * 0.25
        })
        .collect()
}

effect!(
    SMART_DEF,
    "smartBlur",
    "Smart Blur",
    "blurs only between colours that are alike, or shows the edges it keeps",
    [
        px("radius", "Radius", 3.0, 100.0),
        num("threshold", "Threshold", 25.0).range(0.0, 100.0).doc("how different in brightness neighbours may be and still be blurred in"),
        choice("mode", "Mode", &["normal", "edgeOnly", "overlayEdge"], 0).doc("the blur, its edges in white on black, or the blur with its edges in white"),
    ]
);
pub const SMART: Spec = Spec::new(&SMART_DEF, CATEGORY, smart);

fn smart(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = ctx.num("radius", 3.0).max(0.0) * b.scale;
    let range = ctx.num("threshold", 25.0) as f32 / 100.0;
    let mode = ctx.choice("mode");
    if r >= 0.5 {
        // the blur and (if asked) its edges, on the GPU in one go
        let then = (mode != 0).then(|| {
            use veymelo_raster::gpu;
            let p = GpuSmartEdges { w: b.img.width as u32, h: b.img.height as u32, mode, limit: range.max(0.01) * 0.5 };
            gpu::Op::new(gpu::Kernel { source: gpu::program(&[gpu::COMMON, include_str!("blur.metal")]), name: "smart_edges" }, &p, [b.img.width, b.img.height])
        });
        if let Some(img) = keeping_edges_then(&b.img, r, range, true, then) {
            b.img = img;
            return b;
        }
    }
    let blurred = if r >= 0.5 { keeping_edges(&b.img, r, range, true) } else { b.img.clone() };
    if mode == 0 {
        b.img = blurred;
        return b;
    }
    let strength = edge_strength(&blurred);
    let limit = range.max(0.01) * 0.5;
    b.img = blurred;
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let on = strength[y * w + x] > limit;
        match (mode, on) {
            (1, _) => premul([if on { 1.0 } else { 0.0 }; 3], p[3]),
            (_, true) => premul([1.0; 3], p[3]),
            _ => p,
        }
    });
    b
}

effect!(
    CROSS_DEF,
    "crossBlur",
    "Cross Blur",
    "blurs across and down apart, then puts the two together",
    [px("radiusX", "Radius X", 10.0, 500.0), px("radiusY", "Radius Y", 10.0, 500.0), choice("transferMode", "Transfer Mode", &["blend", "add", "screen", "lighten"], 0)]
);
pub const CROSS: Spec = Spec::new(&CROSS_DEF, CATEGORY, cross);

fn cross(ctx: &Ctx, mut b: Buf) -> Buf {
    let (rx, ry) = (ctx.num("radiusX", 10.0).max(0.0) * b.scale, ctx.num("radiusY", 10.0).max(0.0) * b.scale);
    if rx < 0.01 && ry < 0.01 {
        return b;
    }
    let e = edges(ctx, &mut b, false, 3.0 * rx.max(ry));
    let across = filter::boxes(&b.img, 2.0 * rx + 1.0, 0.0, 3, e);
    let down = filter::boxes(&b.img, 0.0, 2.0 * ry + 1.0, 3, e);
    let mode = ctx.choice("transferMode");
    map_px(&mut b.img, |x, y, _| {
        let (p, q) = (across.px[y * across.width + x], down.px[y * across.width + x]);
        tidy([0, 1, 2, 3].map(|c| match mode {
            1 => p[c] + q[c],
            2 => p[c] + q[c] - p[c] * q[c],
            3 => p[c].max(q[c]),
            _ => (p[c] + q[c]) * 0.5,
        }))
    });
    b
}

effect!(
    RADIAL_FAST_DEF,
    "radialFastBlur",
    "Radial Fast Blur",
    "streaks out from a centre, softly or keeping the brightest or darkest",
    [
        point("center", "Center"),
        num("amount", "Amount", 50.0).range(0.0, 100.0).doc("how long the streaks are, as % of half the distance to the centre"),
        choice("zoom", "Zoom", &["standard", "brightest", "darkest"], 0),
    ]
);
pub const RADIAL_FAST: Spec = Spec::new(&RADIAL_FAST_DEF, CATEGORY, radial_fast).points(&[("center", [0.5, 0.5])]);

fn radial_fast(ctx: &Ctx, mut b: Buf) -> Buf {
    let s = (ctx.num("amount", 50.0) / 100.0).clamp(0.0, 1.0) * 0.5;
    if s <= 0.0 {
        return b;
    }
    let c = b.pixel(ctx.point("center"));
    let mode = ctx.choice("zoom");
    if veymelo_raster::gpu::wanted(b.img.px.len()) {
        use veymelo_raster::gpu::{self, split};
        let p = GpuFast { w: b.img.width as u32, h: b.img.height as u32, mode, pad: 0, cx: split(c.x), cy: split(c.y), s: split(s), sf: s as f32 };
        let kernel = gpu::Kernel { source: gpu::program(&[gpu::COMMON, include_str!("blur.metal")]), name: "radial_fast" };
        if let Some(img) = gpu::apply(&b.img, &[gpu::Op::new(kernel, &p, [b.img.width, b.img.height])]) {
            b.img = img;
            return b;
        }
    }
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (dx, dy) = (x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
        let n = ((dx.hypot(dy) * s).ceil() as usize + 1).clamp(2, 256);
        let mut acc = match mode {
            1 => [f32::MIN; 4],
            2 => [f32::MAX; 4],
            _ => [0.0; 4],
        };
        for i in 0..n {
            let k = 1.0 - s * i as f64 / (n - 1) as f64;
            let q = src.sample(Sampling::Bilinear, c.x + dx * k, c.y + dy * k);
            for ch in 0..4 {
                acc[ch] = match mode {
                    1 => acc[ch].max(q[ch]),
                    2 => acc[ch].min(q[ch]),
                    _ => acc[ch] + q[ch] / n as f32,
                };
            }
        }
        let a = acc[3].clamp(0.0, 1.0);
        // the brightest or darkest of each channel may come from different pixels: kept within the alpha
        if mode == 0 { tidy(acc) } else { [acc[0].clamp(0.0, a), acc[1].clamp(0.0, a), acc[2].clamp(0.0, a), a] }
    });
    b
}

/// Zoom Blur's settings as the GPU takes them (blur.metal's Zoom).
#[repr(C)]
#[derive(Clone, Copy)]
struct GpuZoom {
    w: u32,
    h: u32,
    kind: u32,
    pad: u32,
    cx: [f32; 2],
    cy: [f32; 2],
    k: [f32; 2],
    zoom: f32,
    arc: f32,
}

unsafe impl veymelo_raster::gpu::Plain for GpuZoom {}

/// Radial Fast Blur's settings as the GPU takes them (blur.metal's Fast).
#[repr(C)]
#[derive(Clone, Copy)]
struct GpuFast {
    w: u32,
    h: u32,
    mode: u32,
    pad: u32,
    cx: [f32; 2],
    cy: [f32; 2],
    s: [f32; 2],
    sf: f32,
}

unsafe impl veymelo_raster::gpu::Plain for GpuFast {}

const MAP_PROPERTIES: &[&str] = &["red", "green", "blue", "alpha", "luminance", "lightness", "hue", "saturation"];

effect!(
    VECTOR_DEF,
    "vectorBlur",
    "Vector Blur",
    "blurs along the lines of a map's brightness (the layer's own by default)",
    [
        choice("type", "Type", &["natural", "constantLength", "perpendicular", "directionCenter", "directionFading"], 0),
        px("amount", "Amount", 10.0, 500.0),
        num("angleOffset", "Angle Offset", 0.0).unit(Unit::Degrees),
        num("ridgeSmoothness", "Ridge Smoothness", 1.0).range(0.0, 100.0).doc("how gently the strength of the blur follows the map"),
        layer("vectorMap", "Vector Map").doc("the map (none: the layer itself)"),
        choice("property", "Property", MAP_PROPERTIES, 4).doc("what of the map is read"),
        num("mapSoftness", "Map Softness", 15.0).range(0.0, 100.0).doc("how much the map is blurred first"),
    ]
);
pub const VECTOR: Spec = Spec::new(&VECTOR_DEF, CATEGORY, vector);

/// One property of a premultiplied pixel, 0 to 1.
fn property(p: Px, which: u32) -> f32 {
    let (c, a) = unpremul(p);
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    match which {
        0 => p[0],
        1 => p[1],
        2 => p[2],
        3 => a,
        5 => (mx + mn) / 2.0 * a,
        6 => veymelo_color::rgb_to_hsl(c[0] as f64, c[1] as f64, c[2] as f64).0 as f32 / 360.0,
        7 => veymelo_color::rgb_to_hsl(c[0] as f64, c[1] as f64, c[2] as f64).1 as f32,
        _ => luma_px(p),
    }
}

fn vector(ctx: &Ctx, mut b: Buf) -> Buf {
    let amount = ctx.num("amount", 10.0).max(0.0) * b.scale;
    if amount < 0.5 {
        return b;
    }
    let kind = ctx.choice("type");
    let offset = ctx.num("angleOffset", 0.0).to_radians();
    let ridge = 1.0 / (1.0 + ctx.num("ridgeSmoothness", 1.0).max(0.0));
    let soft = deviation(ctx.num("mapSoftness", 15.0), &b);
    let which = ctx.choice("property");
    let source = ctx.layer_over("vectorMap", &b, Take::All, true).unwrap_or_else(|| b.img.clone());
    let mut plane = source.clone();
    map_px(&mut plane, |_, _, p| [property(p, which); 4]);
    let map: Vec<f32> = filter::gaussian(&plane, soft, soft, Edge::Repeat).px.iter().map(|p| p[0]).collect();
    let (w, h) = (b.img.width as i64, b.img.height as i64);
    let at = |x: i64, y: i64| map[(y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize] as f64;
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, p| {
        let (x, y) = (x as i64, y as i64);
        let (gx, gy) = ((at(x + 1, y) - at(x - 1, y)) * 0.5, (at(x, y + 1) - at(x, y - 1)) * 0.5);
        let m = at(x, y);
        let slope = gx.hypot(gy);
        let (angle, len) = match kind {
            0..=2 => {
                if slope < 1e-6 {
                    return p;
                }
                let g = gy.atan2(gx);
                let base = if kind == 2 { g } else { g + std::f64::consts::FRAC_PI_2 };
                (base + offset, if kind == 1 { amount } else { amount * (slope * 10.0 * ridge.max(0.01) * 2.0).min(1.0) })
            }
            3 => (m * std::f64::consts::TAU + offset, amount),
            _ => (m * std::f64::consts::TAU + offset, amount * m),
        };
        if len < 0.5 {
            return p;
        }
        let (dx, dy) = (angle.cos() * len, angle.sin() * len);
        let n = ((len * 2.0).ceil() as usize).clamp(2, 128);
        let mut acc = [0f32; 4];
        for i in 0..n {
            let t = i as f64 / (n - 1) as f64 - 0.5;
            let (sx, sy) = ((x as f64 + 0.5 + dx * t).clamp(0.5, w as f64 - 0.5), (y as f64 + 0.5 + dy * t).clamp(0.5, h as f64 - 0.5));
            let q = src.sample(Sampling::Bilinear, sx, sy);
            for c in 0..4 {
                acc[c] += q[c] / n as f32;
            }
        }
        acc
    });
    b
}

/// The rows of an iris: (row, first, last) pixel offsets.
type Rows = Vec<(i64, i64, i64)>;

const IRIS_SHAPES: &[&str] = &["triangle", "square", "pentagon", "hexagon", "heptagon", "octagon", "nonagon", "decagon", "circle"];

effect!(
    LENS_DEF,
    "cameraLensBlur",
    "Camera Lens Blur",
    "blurs as an out-of-focus lens does: bright points open into the iris's shape",
    [
        px("blurRadius", "Blur Radius", 10.0, 500.0),
        choice("irisShape", "Iris Shape", IRIS_SHAPES, 3),
        num("irisRoundness", "Iris Roundness", 0.0).range(0.0, 100.0).unit(Unit::Percent),
        num("irisAspectRatio", "Iris Aspect Ratio", 1.0).range(0.01, 100.0).doc("above 1 wider, below 1 narrower"),
        num("irisRotation", "Iris Rotation", 0.0).unit(Unit::Degrees),
        num("diffractionFringe", "Diffraction Fringe", 0.0).range(0.0, 500.0).doc("how much brighter the iris's rim is: 100 a natural halo, 500 a ring"),
        layer("blurMapLayer", "Blur Map").doc("a map of how far each part is from focus (none: all blurred alike)"),
        choice("blurMapChannel", "Blur Map Channel", &["luminance", "alpha", "color"], 0),
        choice("blurMapPlacement", "Blur Map Placement", &["center", "stretch"], 1),
        num("blurFocalDistance", "Blur Focal Distance", 0.0).range(0.0, 255.0).doc("the map value that is in focus"),
        flag("invertBlurMap", "Invert Blur Map", false),
        num("highlightGain", "Highlight Gain", 0.0).range(0.0, 100.0).doc("how much brighter highlights bloom"),
        num("highlightThreshold", "Highlight Threshold", 255.0).range(0.0, 255.0).doc("how bright a pixel must be to count as a highlight"),
        num("highlightSaturation", "Highlight Saturation", 100.0).range(0.0, 100.0).doc("100 keeps highlights' colour, 0 makes their bloom white"),
        REPEAT,
        flag("useLinear", "Use Linear Working Space", false).doc("mix in linear light, as light does"),
    ]
);
pub const LENS: Spec = Spec::new(&LENS_DEF, CATEGORY, lens);

/// The rows of an iris of radius `r`.
fn iris(r: f64, sides: u32, rotation: f64, roundness: f64, aspect: f64) -> Rows {
    let reach = r.ceil() as i64;
    let rot = rotation.to_radians();
    let corners: Vec<(f64, f64)> = (0..sides).map(|k| {
        let a = rot - std::f64::consts::FRAC_PI_2 + k as f64 * std::f64::consts::TAU / sides.max(3) as f64;
        (r * a.cos(), r * a.sin())
    }).collect();
    let mut rows = Vec::new();
    for dy in -reach..=reach {
        let y = dy as f64;
        let circle = (y.abs() <= r).then(|| (r * r - y * y).max(0.0).sqrt());
        let polygon = if sides >= 3 {
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for k in 0..corners.len() {
                let ((x0, y0), (x1, y1)) = (corners[k], corners[(k + 1) % corners.len()]);
                if (y0 - y) * (y1 - y) <= 0.0 && (y1 - y0).abs() > 1e-12 {
                    let x = x0 + (y - y0) / (y1 - y0) * (x1 - x0);
                    (lo, hi) = (lo.min(x), hi.max(x));
                }
            }
            (lo <= hi).then_some((lo, hi))
        } else {
            circle.map(|c| (-c, c))
        };
        let span = match (polygon, circle) {
            (Some((lo, hi)), Some(c)) => Some((lo + (-c - lo) * roundness, hi + (c - hi) * roundness)),
            (Some(s), None) => Some(s),
            (None, Some(c)) if roundness > 0.5 => Some((-c, c)),
            _ => None,
        };
        if let Some((lo, hi)) = span {
            let (l, h) = ((lo * aspect).round() as i64, (hi * aspect).round() as i64);
            if l <= h {
                rows.push((dy, l, h));
            }
        }
    }
    if rows.is_empty() {
        rows.push((0, 0, 0));
    }
    rows
}

fn lens(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = ctx.num("blurRadius", 10.0).max(0.0) * b.scale;
    if r < 0.5 {
        return b;
    }
    let shape = ctx.choice("irisShape");
    let sides = if shape >= 8 { 0 } else { shape + 3 };
    let roundness = (ctx.num("irisRoundness", 0.0) / 100.0).clamp(0.0, 1.0);
    let aspect = ctx.num("irisAspectRatio", 1.0).max(0.01);
    let rotation = ctx.num("irisRotation", 0.0);
    let repeat = ctx.flag("repeatEdge") || ctx.adjustment;
    if !repeat {
        b.pad((r * aspect.max(1.0)).ceil() as usize + 1);
    }
    let linear = ctx.flag("useLinear");
    // highlights pushed up before the blur so they bloom; saturation keeps their colour or whitens the gain
    let threshold = ctx.num("highlightThreshold", 255.0) as f32 / 255.0;
    let gain = 1.0 + ctx.num("highlightGain", 0.0).max(0.0) as f32 / 100.0 * 4.0;
    let saturation = (ctx.num("highlightSaturation", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let mut src = b.img.clone();
    map_px(&mut src, |_, _, p| {
        let (mut c, a) = unpremul(p);
        if linear {
            c = c.map(|v| veymelo_color::srgb_to_linear(v as f64) as f32);
        }
        if gain > 1.0 && a > 0.0 && luma(c) >= threshold {
            let top = c[0].max(c[1]).max(c[2]);
            c = c.map(|v| v + (v + (top - v) * (1.0 - saturation)) * (gain - 1.0));
        }
        premul(c, a)
    });
    let (w, h) = (src.width, src.height);
    // sums along each row, so a run of pixels is one subtraction
    let sums: Vec<Vec<[f64; 4]>> = (0..h)
        .map(|y| {
            let mut s = vec![[0f64; 4]; w + 1];
            for x in 0..w {
                let p = src.px[y * w + x];
                for c in 0..4 {
                    s[x + 1][c] = s[x][c] + p[c] as f64;
                }
            }
            s
        })
        .collect();
    let total = |rows: &[(i64, i64, i64)], x: usize, y: usize| -> [f64; 4] {
        let mut acc = [0f64; 4];
        for &(dy, l, rr) in rows {
            let mut yy = y as i64 + dy;
            if yy < 0 || yy >= h as i64 {
                if !repeat {
                    continue;
                }
                yy = yy.clamp(0, h as i64 - 1);
            }
            let s = &sums[yy as usize];
            let (mut a, mut z) = (x as i64 + l, x as i64 + rr);
            if repeat {
                // past the ends: the end pixels, as many times as the run reaches past
                if a < 0 {
                    let first = src.px[yy as usize * w];
                    let n = (-a).min(z - a + 1) as f64;
                    for c in 0..4 {
                        acc[c] += first[c] as f64 * n;
                    }
                    a = 0;
                }
                if z >= w as i64 {
                    let last = src.px[yy as usize * w + w - 1];
                    let n = (z - (w as i64 - 1)).min(z - a + 1) as f64;
                    for c in 0..4 {
                        acc[c] += last[c] as f64 * n;
                    }
                    z = w as i64 - 1;
                }
            }
            let (a, z) = (a.max(0), z.min(w as i64 - 1));
            if a <= z {
                for c in 0..4 {
                    acc[c] += s[z as usize + 1][c] - s[a as usize][c];
                }
            }
        }
        acc
    };
    let count = |rows: &[(i64, i64, i64)]| rows.iter().map(|(_, l, r)| (r - l + 1) as f64).sum::<f64>();
    // the rim (outer fifth of the iris) takes more of the light with the fringe
    let fringe = (ctx.num("diffractionFringe", 0.0) / 500.0).clamp(0.0, 1.0);
    let (inner_w, rim_w) = (1.0 - fringe, 1.0 + 4.0 * fringe);
    let irises = |radius: f64| {
        let outer = iris(radius, sides, rotation, roundness, aspect);
        let inner = if fringe > 0.0 && radius * 0.8 >= 1.0 { iris(radius * 0.8, sides, rotation, roundness, aspect) } else { Vec::new() };
        (outer, inner)
    };
    let blur_at = |ir: &(Rows, Rows), x: usize, y: usize| -> Px {
        let (outer, inner) = ir;
        let acc: [f64; 4] = if inner.is_empty() {
            total(outer, x, y).map(|v| v / count(outer).max(1.0))
        } else {
            let (so, si) = (total(outer, x, y), total(inner, x, y));
            let (no, ni) = (count(outer), count(inner));
            let norm = (inner_w * ni + rim_w * (no - ni)).max(1e-9);
            [0, 1, 2, 3].map(|c| (inner_w * si[c] + rim_w * (so[c] - si[c])) / norm)
        };
        let mut p = acc.map(|v| v as f32);
        p[3] = p[3].clamp(0.0, 1.0);
        if linear {
            let (c, a) = unpremul(p);
            p = premul(c.map(|v| veymelo_color::linear_to_srgb(v.max(0.0) as f64) as f32), a);
        }
        p
    };
    // with a map, each pixel's radius is how far its map value is from the focal distance
    let map: Option<Vec<f32>> = ctx.layer_over("blurMapLayer", &b, Take::All, ctx.choice("blurMapPlacement") == 1).map(|img| {
        let ch = ctx.choice("blurMapChannel");
        let invert = ctx.flag("invertBlurMap");
        let focal = (ctx.num("blurFocalDistance", 0.0) / 255.0).clamp(0.0, 1.0) as f32;
        let span = focal.max(1.0 - focal).max(1e-3);
        img.px
            .iter()
            .map(|p| {
                let (c, a) = unpremul(*p);
                let v = match ch {
                    1 => a,
                    2 => c[0].max(c[1]).max(c[2]) * a,
                    _ => luma(c) * a,
                };
                let v = if invert { 1.0 - v } else { v };
                ((v - focal).abs() / span).clamp(0.0, 1.0)
            })
            .collect()
    });
    let mut out = Image::new(w, h);
    match &map {
        None => {
            let ir = irises(r);
            par::rows(&mut out.px, w, |y, row| {
                for (x, o) in row.iter_mut().enumerate() {
                    *o = blur_at(&ir, x, y);
                }
            });
        }
        Some(m) => {
            // radii at even steps; each pixel mixes the two its map value falls between
            let steps = (r.ceil() as usize).clamp(1, 8);
            let all: Vec<_> = (0..=steps).map(|k| irises(r * k as f64 / steps as f64)).collect();
            let plain = &b.img;
            par::rows(&mut out.px, w, |y, row| {
                for (x, o) in row.iter_mut().enumerate() {
                    let f = m[y * w + x] * steps as f32;
                    let k = (f.floor() as usize).min(steps);
                    let t = f - k as f32;
                    let at = |k: usize| if k == 0 || r * k as f64 / (steps as f64) < 0.5 { plain.px[y * w + x] } else { blur_at(&all[k], x, y) };
                    let p = at(k);
                    *o = if t > 1e-4 && k < steps {
                        let q = at(k + 1);
                        [0, 1, 2, 3].map(|c| p[c] + (q[c] - p[c]) * t)
                    } else {
                        p
                    };
                }
            });
        }
    }
    b.img = out;
    b
}

effect!(FLICKER_DEF, "reduceInterlaceFlicker", "Reduce Interlace Flicker", "softens up and down, against flicker on interlaced screens", [num("softness", "Softness", 0.0).range(0.0, 100.0)]);
pub const FLICKER: Spec = Spec::new(&FLICKER_DEF, "obsolete", flicker);

fn flicker(ctx: &Ctx, mut b: Buf) -> Buf {
    let s = ctx.num("softness", 0.0).max(0.0) * b.scale;
    if s >= 0.01 {
        b.img = filter::gaussian(&b.img, 0.0, s, Edge::Repeat);
    }
    b
}

/// Every effect here.
pub const ALL: [Spec; 17] = [GAUSSIAN, FAST_BOX, DIRECTIONAL, RADIAL, SHARPEN, UNSHARP, CHANNEL, COMPOUND, BILATERAL, SMART, CROSS, RADIAL_FAST, VECTOR, LENS, FLICKER, ZOOM, DEBLUR];

const ZOOM_TYPES: &[&str] = &["straightZoom", "fadingZoom", "centered", "rotate", "rotateFading"];

effect!(
    ZOOM_DEF,
    "zoomBlur",
    "Zoom Blur",
    "streaks toward a centre (evenly, fading, or both ways about each point) or round it (evenly or fading)",
    [
        choice("type", "Type", ZOOM_TYPES, 1).doc("toward the centre evenly or fading; centred: both ways about each point; turned round the centre evenly or fading"),
        num("amount", "Amount", 10.0).range(-100.0, 100.0).doc("how far: % of the distance to the centre (zooms), or degrees of turn"),
        num("quality", "Quality", 50.0).range(1.0, 100.0).doc("how many samples along each streak"),
        point("center", "Center"),
    ]
);
pub const ZOOM: Spec = Spec::new(&ZOOM_DEF, CATEGORY, zoom_blur).points(&[("center", [0.5, 0.5])]);

fn zoom_blur(ctx: &Ctx, mut b: Buf) -> Buf {
    let amount = ctx.num("amount", 10.0);
    if amount == 0.0 {
        return b;
    }
    let kind = ctx.choice("type");
    let quality = ctx.num("quality", 50.0).clamp(1.0, 100.0);
    let c = b.pixel(ctx.point("center"));
    let src = b.img.clone();
    // zooms: a part of the distance to the centre; turns: radians of arc
    let zoom = (amount / 100.0).clamp(-1.0, 1.0) * 0.5;
    let arc = amount.to_radians() * 0.25;
    if veymelo_raster::gpu::wanted(b.img.px.len()) {
        use veymelo_raster::gpu::{self, split};
        let k = if kind >= 3 { arc.abs() } else { zoom.abs() } * quality / 50.0;
        let p = GpuZoom { w: b.img.width as u32, h: b.img.height as u32, kind, pad: 0, cx: split(c.x), cy: split(c.y), k: split(k), zoom: zoom as f32, arc: arc as f32 };
        let kernel = gpu::Kernel { source: gpu::program(&[gpu::COMMON, include_str!("blur.metal")]), name: "zoom_blur" };
        if let Some(img) = gpu::apply(&b.img, &[gpu::Op::new(kernel, &p, [b.img.width, b.img.height])]) {
            b.img = img;
            return b;
        }
    }
    map_px(&mut b.img, |x, y, p| {
        let (dx, dy) = (x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
        let r = dx.hypot(dy);
        let span = r * if kind >= 3 { arc.abs() } else { zoom.abs() };
        let n = ((span * quality / 50.0).ceil() as usize).clamp(1, 256);
        if n <= 1 {
            return p;
        }
        let (mut acc, mut sum) = ([0.0f32; 4], 0.0f32);
        for i in 0..n {
            let t = i as f64 / (n - 1) as f64;
            let (sx, sy, w) = match kind {
                // toward the centre, evenly
                0 => (c.x + dx * (1.0 - zoom * t), c.y + dy * (1.0 - zoom * t), 1.0),
                // toward the centre, fading along the streak
                1 => (c.x + dx * (1.0 - zoom * t), c.y + dy * (1.0 - zoom * t), (1.0 - t) as f32 + 0.05),
                // both ways about the point
                2 => (c.x + dx * (1.0 + zoom * (t - 0.5)), c.y + dy * (1.0 + zoom * (t - 0.5)), 1.0),
                // turned round the centre, evenly or fading to the arc's ends
                _ => {
                    let (sa, ca) = (arc * (t - 0.5)).sin_cos();
                    let w = if kind == 4 { (1.0 - (2.0 * t - 1.0).abs()) as f32 + 0.05 } else { 1.0 };
                    (c.x + dx * ca - dy * sa, c.y + dx * sa + dy * ca, w)
                }
            };
            let q = src.sample(Sampling::Bilinear, sx, sy);
            (0..4).for_each(|k| acc[k] += q[k] * w);
            sum += w;
        }
        acc.map(|v| v / sum)
    });
    b
}

effect!(
    DEBLUR_DEF,
    "cameraShakeDeblur",
    "Camera-Shake Deblur",
    "takes away the blur of a shaking camera: where a neighbouring frame is clearly sharper, its picture there, lined up with this one, stands in",
    [
        num("blurDuration", "Blur Duration", 2.0).range(1.0, 10.0).doc("how many frames either side are looked at"),
        choice("deblurMethod", "Deblur Method", &["highQuality", "standard"], 0).doc("standard lines the sharper frames up over half the distance"),
        num("strength", "Strength", 100.0).range(100.0, 200.0).unit(Unit::Percent).doc("past 100 %: pushed further from the blurred picture"),
        num("shakeSensitivity", "Shake Sensitivity", 50.0).range(0.0, 100.0).doc("how much sharper a neighbour must be (50: by the threshold; 100: any sharper; 0: by twice it)"),
        num("searchRange", "Search Range", 8.0).range(0.0, 64.0).unit(Unit::Pixels).doc("how far a sharper frame is shifted to line it up"),
        num("patchSize", "Patch Size", 32.0).range(8.0, 256.0).unit(Unit::Pixels).doc("the pieces the frame is judged in"),
        num("threshold", "Threshold", 20.0).range(0.0, 500.0).unit(Unit::Percent).doc("how much sharper, in %, a neighbour's piece must be"),
        flag("showBlurryFrames", "Show Blurry Frames", false).doc("tint red where sharper frames stand in"),
    ]
);
pub const DEBLUR: Spec = Spec::new(&DEBLUR_DEF, CATEGORY, deblur);

/// A picture's brightness, pixel by pixel.
struct Bright {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

impl Bright {
    fn of(img: &Image) -> Bright {
        Bright { w: img.width, h: img.height, v: img.px.iter().map(|p| luma_px(*p)).collect() }
    }

    fn at(&self, x: i64, y: i64) -> f32 {
        self.v[y.clamp(0, self.h as i64 - 1) as usize * self.w + x.clamp(0, self.w as i64 - 1) as usize]
    }

    /// How sharp a rectangle is: the mean of its brightness's squared steps.
    fn sharpness(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> f64 {
        let (mut sum, mut n) = (0.0f64, 0usize);
        for y in y0..y1.saturating_sub(1) {
            for x in x0..x1.saturating_sub(1) {
                let v = self.at(x as i64, y as i64);
                let (gx, gy) = (self.at(x as i64 + 1, y as i64) - v, self.at(x as i64, y as i64 + 1) - v);
                sum += (gx * gx + gy * gy) as f64;
                n += 1;
            }
        }
        if n == 0 { 0.0 } else { sum / n as f64 }
    }
}

/// The whole-pixel shift, within `r` either way, that best lines `other` up with `this` on a
/// rectangle (the least squared differences; the smallest shift on a tie).
fn line_up(this: &Bright, other: &Bright, (x0, y0, x1, y1): (usize, usize, usize, usize), r: i64) -> (i64, i64) {
    let step = ((x1 - x0).max(y1 - y0) / 24).max(1);
    let mut best = (f64::INFINITY, 0, 0);
    for dy in -r..=r {
        for dx in -r..=r {
            let mut sum = 0.0f64;
            for y in (y0..y1).step_by(step) {
                for x in (x0..x1).step_by(step) {
                    let d = this.at(x as i64, y as i64) - other.at(x as i64 + dx, y as i64 + dy);
                    sum += (d * d) as f64;
                }
            }
            let sum = sum * (1.0 + 1e-9 * (dx * dx + dy * dy) as f64);
            if sum < best.0 {
                best = (sum, dx, dy);
            }
        }
    }
    (best.1, best.2)
}

fn deblur(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(host) = ctx.host else { return b };
    let (w, h) = (b.img.width, b.img.height);
    if w == 0 || h == 0 {
        return b;
    }
    let span = ctx.num("blurDuration", 2.0).clamp(1.0, 10.0).round() as i64;
    let mut search = (ctx.num("searchRange", 8.0).max(0.0) * b.scale).round() as i64;
    if ctx.choice("deblurMethod") == 1 {
        search /= 2;
    }
    let patch = (ctx.num("patchSize", 32.0).max(8.0) * b.scale).round().max(4.0) as usize;
    let sensitivity = ctx.num("shakeSensitivity", 50.0).clamp(0.0, 100.0);
    let needed = 1.0 + ctx.num("threshold", 20.0).max(0.0) / 100.0 * (100.0 - sensitivity) / 50.0;
    let strength = (ctx.num("strength", 100.0) / 100.0).clamp(1.0, 2.0) as f32;
    let show = ctx.flag("showBlurryFrames");
    // the frames either side, as this buffer lays its pixels out
    let mut near: Vec<(Image, Bright)> = Vec::new();
    for k in (-span..=span).filter(|k| *k != 0) {
        let Some(nb) = host.this_at(ctx.time + veymelo_time::Tick::from_seconds(k as f64 * ctx.frame.seconds()), ctx.index) else { continue };
        let img = if nb.img.width == w && nb.img.height == h && nb.origin == b.origin && nb.scale == b.scale {
            nb.img
        } else {
            let mut img = Image::new(w, h);
            let to = |x: usize, y: usize| {
                let at = b.to_layer().apply(veymelo_geom::vec2(x as f64 + 0.5, y as f64 + 0.5));
                nb.img.sample(Sampling::Bilinear, (at.x - nb.origin.x) * nb.scale, (at.y - nb.origin.y) * nb.scale)
            };
            map_px(&mut img, |x, y, _| to(x, y));
            img
        };
        let bright = Bright::of(&img);
        near.push((img, bright));
    }
    if near.is_empty() {
        return b;
    }
    let this = Bright::of(&b.img);
    let (nx, ny) = (w.div_ceil(patch), h.div_ceil(patch));
    // each patch: the sharper neighbour standing in, and its shift; or none
    let mut choice: Vec<Option<(usize, i64, i64)>> = vec![None; nx * ny];
    par::chunks(&mut choice, nx, |py, row| {
        for (px, out) in row.iter_mut().enumerate() {
            let rect = (px * patch, py * patch, ((px + 1) * patch).min(w), ((py + 1) * patch).min(h));
            let own = this.sharpness(rect.0, rect.1, rect.2, rect.3);
            let best = near.iter().enumerate().map(|(k, (_, l))| (l.sharpness(rect.0, rect.1, rect.2, rect.3), k)).filter(|(s, _)| *s > own * needed).max_by(|a, b| a.0.total_cmp(&b.0));
            *out = best.map(|(_, k)| {
                let (dx, dy) = line_up(&this, &near[k].1, rect, search);
                (k, dx, dy)
            });
        }
    });
    if choice.iter().all(Option::is_none) {
        return b;
    }
    let src = b.img.clone();
    let pick = |i: usize, x: usize, y: usize| -> Px {
        match choice[i] {
            None => src.px[y * w + x],
            Some((k, dx, dy)) => near[k].0.get(x as i64 + dx, y as i64 + dy),
        }
    };
    // the patches' choices blended between their middles, so no seams show
    map_px(&mut b.img, |x, y, p| {
        let fy = ((y as f64 + 0.5) / patch as f64 - 0.5).clamp(0.0, (ny - 1) as f64);
        let fx = ((x as f64 + 0.5) / patch as f64 - 0.5).clamp(0.0, (nx - 1) as f64);
        let (x0, y0, tx, ty) = (fx.floor() as usize, fy.floor() as usize, (fx - fx.floor()) as f32, (fy - fy.floor()) as f32);
        let (x1, y1) = ((x0 + 1).min(nx - 1), (y0 + 1).min(ny - 1));
        let ids = [y0 * nx + x0, y0 * nx + x1, y1 * nx + x0, y1 * nx + x1];
        if ids.iter().all(|&i| choice[i].is_none()) {
            return p;
        }
        let weights = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
        let mut acc = [0.0f32; 4];
        for (&i, wgt) in ids.iter().zip(weights) {
            let q = pick(i, x, y);
            (0..4).for_each(|c| acc[c] += q[c] * wgt);
        }
        if strength != 1.0 {
            // further from the blurred picture than what stands in
            (0..4).for_each(|c| acc[c] = p[c] + (acc[c] - p[c]) * strength);
            acc[3] = acc[3].clamp(0.0, 1.0);
            (0..3).for_each(|c| acc[c] = acc[c].max(0.0));
        }
        if show {
            acc[0] = (acc[0] + 0.3 * acc[3]).min(acc[3].max(acc[0]));
        }
        acc
    });
    b
}

#[cfg(test)]
mod tests;
