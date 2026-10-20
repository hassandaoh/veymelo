//! Stylize, part two: painted (Brush Strokes), mirrored and tiled (Kaleidoscope, Repeat Tile,
//! Motion Tile, Hex Tile), darkened toward the edges (Vignette), cut to black and white (Channel
//! Threshold, Threshold RGB), loaded block by block (Block Load), burnt (Burn Film), made of
//! glass or plastic (Glass, Plastic), and recoloured from itself (Smoothie).
//!
//! Sizes and places are in layer units, so a preview looks the same as the full picture.

use std::f64::consts::TAU;

use veymelo_geom::vec2;
use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px, Sampling};

use crate::color::rgb3;
use crate::defs::{angle, choice, color, effect, flag, layer, num, pct, point, px, ranged};
use crate::distort::rect_px;
use crate::util::{hsl, luma, map_px, premul, unpremul};
use crate::{Buf, Ctx, Spec, Take};

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

/// Sampled with the picture's edge pixels going on past it.
fn held(img: &Image, x: f64, y: f64) -> Px {
    img.sample(Sampling::Bilinear, x.clamp(0.5, img.width as f64 - 0.5), y.clamp(0.5, img.height as f64 - 0.5))
}

effect!(
    BRUSH_DEF,
    "brushStrokes",
    "Brush Strokes",
    "the picture repainted in short brush strokes",
    [
        angle("strokeAngle", "Stroke Angle", 135.0),
        px("brushSize", "Brush Size", 2.0, 5.0),
        px("strokeLength", "Stroke Length", 4.0, 40.0),
        ranged("strokeDensity", "Stroke Density", 1.0, 0.0, 2.0),
        ranged("strokeRandomness", "Stroke Randomness", 1.0, 0.0, 2.0),
        choice("paintSurface", "Paint Surface", &["paintOnOriginalImage", "paintOnTransparent", "paintOnWhite", "paintOnBlack"], 0),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
    ]
);
pub const BRUSH: Spec = Spec::new(&BRUSH_DEF, CATEGORY, brush_strokes);

fn brush_strokes(ctx: &Ctx, mut b: Buf) -> Buf {
    let angle = ctx.num("strokeAngle", 135.0);
    let size = (ctx.num("brushSize", 2.0) * b.scale).max(0.5);
    let len = ctx.num("strokeLength", 4.0) * b.scale;
    let density = ctx.num("strokeDensity", 1.0) as f32;
    let wild = ctx.num("strokeRandomness", 1.0);
    let surface = ctx.choice("paintSurface");
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0) as f32;
    let seed = ctx.seed;
    // strokes in cells of the layer (its units), so a preview paints alike
    let cell = (size * 2.0).max(1.0) / b.scale;
    let (s, o) = (b.scale, b.origin);
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, p| {
        let (lx, ly) = (o.x + (x as f64 + 0.5) / s, o.y + (y as f64 + 0.5) / s);
        let (cx, cy) = ((lx / cell).floor() as i64 as u64, (ly / cell).floor() as i64 as u64);
        let rnd = |k: u64| crate::noise::unit(crate::noise::mix(&[seed, cx, cy, k]));
        let a = (angle + (rnd(0) - 0.5) * wild * 60.0).to_radians();
        let l = len * (0.5 + rnd(1));
        let (dx, dy) = (a.sin(), -a.cos());
        let shift = (rnd(2) - 0.5) * size;
        let (bx, by) = (x as f64 + 0.5 - dy * shift, y as f64 + 0.5 + dx * shift);
        let n = l.ceil().clamp(1.0, 32.0) as usize;
        let mut acc = [0f32; 4];
        for i in 0..n {
            let t = if n == 1 { 0.0 } else { i as f64 / (n - 1) as f64 - 0.5 } * l;
            let q = held(&src, bx + dx * t, by + dy * t);
            for c in 0..4 {
                acc[c] += q[c];
            }
        }
        let stroke = acc.map(|v| v / n as f32);
        let m = if (rnd(3) as f32) < density.min(1.0) { 1.0 } else { 0.0 };
        let painted = match surface {
            1 => stroke.map(|v| v * m),
            2 | 3 => {
                let under: Px = if surface == 2 { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] };
                let k = 1.0 - stroke[3] * m;
                [0, 1, 2, 3].map(|c| stroke[c] * m + under[c] * k)
            }
            _ => mix4(p, stroke, m),
        };
        mix4(painted, p, keep)
    });
    b
}

effect!(
    KALEIDOSCOPE_DEF,
    "kaleidoscope",
    "Kaleidoscope",
    "the picture round a point cut into wedges and mirrored, as in a kaleidoscope",
    [
        point("center", "Center"),
        ranged("size", "Size", 100.0, 1.0, 1000.0),
        choice("mirroring", "Mirroring", &["flower", "starlish", "unfold", "wheel", "triangles"], 0).doc("6, 8, 4, 12 (turned, not mirrored) or 3 wedges"),
        angle("rotation", "Rotation", 0.0),
    ]
);
pub const KALEIDOSCOPE: Spec = Spec::new(&KALEIDOSCOPE_DEF, CATEGORY, kaleidoscope).points(&[("center", [0.5, 0.5])]);

fn kaleidoscope(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = b.pixel(ctx.point("center"));
    let k = ctx.num("size", 100.0).max(1.0) / 100.0;
    let mode = ctx.choice("mirroring").min(4);
    let turn = ctx.num("rotation", 0.0).to_radians();
    let wedge = TAU / [6.0, 8.0, 4.0, 12.0, 3.0][mode as usize];
    let mirror = mode != 3;
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (dx, dy) = (x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
        let r = dx.hypot(dy) / k;
        let mut a = (dy.atan2(dx) - turn).rem_euclid(wedge);
        if mirror && a > wedge * 0.5 {
            a = wedge - a;
        }
        held(&src, c.x + r * (a + turn).cos(), c.y + r * (a + turn).sin())
    });
    b
}

effect!(
    REPEAT_DEF,
    "repeatTile",
    "Repeat Tile",
    "the layer repeated past its edges, plainly or mirrored",
    [
        px("expandRight", "Expand Right", 0.0, 4000.0),
        px("expandLeft", "Expand Left", 0.0, 4000.0),
        px("expandDown", "Expand Down", 0.0, 4000.0),
        px("expandUp", "Expand Up", 0.0, 4000.0),
        choice("tiling", "Tiling", &["repeat", "unfold", "checkerFlipH", "checkerFlipV", "checkerFlip"], 0),
    ]
);
pub const REPEAT: Spec = Spec::new(&REPEAT_DEF, CATEGORY, repeat_tile);

fn repeat_tile(ctx: &Ctx, b: Buf) -> Buf {
    let n = |id: &str| ctx.num(id, 0.0).max(0.0);
    let (r, l, d, u) = (n("expandRight"), n("expandLeft"), n("expandDown"), n("expandUp"));
    if ctx.adjustment || (r + l + d + u) * b.scale < 0.5 {
        return b;
    }
    let mode = ctx.choice("tiling");
    let lr = ctx.bounds;
    let (lw, lh) = (lr.width(), lr.height());
    if lw * b.scale < 1.0 || lh * b.scale < 1.0 {
        return b;
    }
    let s = b.scale;
    // the new buffer: the layer and as much more as asked, the same pixels to a unit
    let origin = vec2(lr.x0 - l, lr.y0 - u);
    let (w, h) = ((((lw + l + r) * s).round() as usize).clamp(1, 16_384), (((lh + u + d) * s).round() as usize).clamp(1, 16_384));
    let mut out = Image::new(w, h);
    let (lx, ly, pw, ph) = rect_px(ctx, &b);
    map_px(&mut out, |x, y, _| {
        let (fx, fy) = (origin.x + (x as f64 + 0.5) / s - lr.x0, origin.y + (y as f64 + 0.5) / s - lr.y0);
        let (i, j) = ((fx / lw).floor() as i64, (fy / lh).floor() as i64);
        let (mut tx, mut ty) = (fx - i as f64 * lw, fy - j as f64 * lh);
        let odd = (i + j).rem_euclid(2) == 1;
        let (flip_x, flip_y) = match mode {
            1 => (i.rem_euclid(2) == 1, j.rem_euclid(2) == 1),
            2 => (odd, false),
            3 => (false, odd),
            4 => (odd, odd),
            _ => (false, false),
        };
        if flip_x {
            tx = lw - tx;
        }
        if flip_y {
            ty = lh - ty;
        }
        let (sx, sy) = (lx + tx * s, ly + ty * s);
        b.img.sample(Sampling::Bilinear, sx.clamp(lx + 0.5, lx + pw - 0.5), sy.clamp(ly + 0.5, ly + ph - 0.5))
    });
    Buf { img: out, origin, scale: s }
}

effect!(
    MOTION_TILE_DEF,
    "motionTile",
    "Motion Tile",
    "the layer as tiles of itself, across a bigger (or smaller) area, mirrored or shifted",
    [
        point("tileCenter", "Tile Center"),
        pct("tileWidth", "Tile Width", 100.0, 1.0, 1000.0),
        pct("tileHeight", "Tile Height", 100.0, 1.0, 1000.0),
        pct("outputWidth", "Output Width", 100.0, 1.0, 1000.0),
        pct("outputHeight", "Output Height", 100.0, 1.0, 1000.0),
        flag("mirrorEdges", "Mirror Edges", false),
        angle("phase", "Phase", 0.0).doc("each column (or row) slid on by this much of a tile"),
        flag("horizontalPhaseShift", "Horizontal Phase Shift", false),
    ]
);
pub const MOTION_TILE: Spec = Spec::new(&MOTION_TILE_DEF, CATEGORY, motion_tile).points(&[("tileCenter", [0.5, 0.5])]);

fn motion_tile(ctx: &Ctx, b: Buf) -> Buf {
    let lr = ctx.bounds;
    let (lw, lh) = (lr.width(), lr.height());
    if lw <= 0.0 || lh <= 0.0 {
        return b;
    }
    let tc = ctx.point("tileCenter");
    let (tw, th) = ((lw * ctx.num("tileWidth", 100.0) / 100.0).max(0.01), (lh * ctx.num("tileHeight", 100.0) / 100.0).max(0.01));
    let mirror = ctx.flag("mirrorEdges");
    let phase = ctx.num("phase", 0.0) / 360.0;
    let across = ctx.flag("horizontalPhaseShift");
    let s = b.scale;
    let (w, h, origin) = if ctx.adjustment {
        (b.img.width, b.img.height, b.origin)
    } else {
        let (ow, oh) = ((lw * ctx.num("outputWidth", 100.0) / 100.0).max(1.0 / s), (lh * ctx.num("outputHeight", 100.0) / 100.0).max(1.0 / s));
        let c = lr.center();
        (((ow * s).round() as usize).clamp(1, 16_384), ((oh * s).round() as usize).clamp(1, 16_384), vec2(c.x - ow * 0.5, c.y - oh * 0.5))
    };
    let (lx, ly, pw, ph) = rect_px(ctx, &b);
    let mut out = Image::new(w, h);
    map_px(&mut out, |x, y, _| {
        let (u, v) = (origin.x + (x as f64 + 0.5) / s, origin.y + (y as f64 + 0.5) / s);
        let (mut tx, mut ty) = ((u - tc.x) / tw + 0.5, (v - tc.y) / th + 0.5);
        if across {
            tx += phase * ty.floor();
        } else {
            ty += phase * tx.floor();
        }
        let (i, j) = (tx.floor(), ty.floor());
        let (mut fx, mut fy) = (tx - i, ty - j);
        if mirror {
            if (i as i64).rem_euclid(2) == 1 {
                fx = 1.0 - fx;
            }
            if (j as i64).rem_euclid(2) == 1 {
                fy = 1.0 - fy;
            }
        }
        b.img.sample(Sampling::Bilinear, (lx + fx * pw).clamp(lx + 0.5, lx + pw - 0.5), (ly + fy * ph).clamp(ly + 0.5, ly + ph - 0.5))
    });
    Buf { img: out, origin, scale: s }
}

effect!(
    VIGNETTE_DEF,
    "vignette",
    "Vignette",
    "the picture darker toward its edges, as a lens makes it (lighter, below 0)",
    [
        ranged("amount", "Amount", 50.0, -100.0, 100.0),
        ranged("angleOfView", "Angle of View", 60.0, 0.0, 179.0),
        point("center", "Center"),
        pct("pinHighlights", "Pin Highlights", 0.0, 0.0, 100.0).doc("the bright parts darken less"),
    ]
);
pub const VIGNETTE: Spec = Spec::new(&VIGNETTE_DEF, CATEGORY, vignette).points(&[("center", [0.5, 0.5])]);

fn vignette(ctx: &Ctx, mut b: Buf) -> Buf {
    let amount = (ctx.num("amount", 50.0) / 100.0) as f32;
    let fov = ctx.num("angleOfView", 60.0).clamp(0.0, 179.0).to_radians();
    if amount == 0.0 || fov <= 0.0 {
        return b;
    }
    let c = b.pixel(ctx.point("center"));
    let pin = (ctx.num("pinHighlights", 0.0) / 100.0) as f32;
    let half = ctx.bounds.width().hypot(ctx.bounds.height()) * b.scale * 0.5;
    let dist = half.max(1.0) / (fov * 0.5).tan();
    map_px(&mut b.img, |x, y, p| {
        let (col, a) = unpremul(p);
        if a <= 0.0 {
            return p;
        }
        // the lens's falloff: the cosine to the fourth power of the angle off its middle
        let r = (x as f64 + 0.5 - c.x).hypot(y as f64 + 0.5 - c.y) / dist;
        let f = (1.0 / (1.0 + r * r)).powi(2) as f32;
        let mut k = 1.0 - amount * (1.0 - f);
        if pin > 0.0 {
            k += (1.0 - k) * pin * luma(col).clamp(0.0, 1.0);
        }
        premul(col.map(|v| (v * k).max(0.0)), a)
    });
    b
}

effect!(
    CHANNEL_THRESHOLD_DEF,
    "channelThreshold",
    "Channel Threshold",
    "the layer cut to black and white by its brightness, each colour, its saturation or its alpha",
    [
        ranged("threshold", "Threshold", 127.0, 0.0, 255.0),
        choice("channel", "Channel", &["luminance", "rgb", "saturation", "alpha"], 0),
        flag("invert", "Invert", false),
        pct("blendWithOriginal", "Blend w. Original", 0.0, 0.0, 100.0),
    ]
);
pub const CHANNEL_THRESHOLD: Spec = Spec::new(&CHANNEL_THRESHOLD_DEF, CATEGORY, channel_threshold);

fn channel_threshold(ctx: &Ctx, mut b: Buf) -> Buf {
    let t = (ctx.num("threshold", 127.0) / 255.0) as f32;
    let ch = ctx.choice("channel");
    let invert = ctx.flag("invert");
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0) as f32;
    let cut = |v: f32| if (v >= t) != invert { 1.0 } else { 0.0 };
    map_px(&mut b.img, |_, _, p| {
        let (c, a) = unpremul(p);
        if a <= 0.0 {
            return p;
        }
        let (o, na) = match ch {
            1 => (c.map(cut), a),
            2 => ([cut(hsl(c).1); 3], a),
            3 => (c, cut(a)),
            _ => ([cut(luma(c)); 3], a),
        };
        premul(mix3(o, c, keep), na + (a - na) * keep)
    });
    b
}

effect!(
    THRESHOLD_RGB_DEF,
    "thresholdRgb",
    "Threshold RGB",
    "each colour channel cut to on or off at a level of its own",
    [
        ranged("redThreshold", "Red Threshold", 127.0, 0.0, 255.0),
        ranged("greenThreshold", "Green Threshold", 127.0, 0.0, 255.0),
        ranged("blueThreshold", "Blue Threshold", 127.0, 0.0, 255.0),
        flag("invertRedChannel", "Invert Red Channel", false),
        flag("invertGreenChannel", "Invert Green Channel", false),
        flag("invertBlueChannel", "Invert Blue Channel", false),
        pct("blendWithOriginal", "Blend w. Original", 0.0, 0.0, 100.0),
    ]
);
pub const THRESHOLD_RGB: Spec = Spec::new(&THRESHOLD_RGB_DEF, CATEGORY, threshold_rgb);

fn threshold_rgb(ctx: &Ctx, mut b: Buf) -> Buf {
    let t = [ctx.num("redThreshold", 127.0), ctx.num("greenThreshold", 127.0), ctx.num("blueThreshold", 127.0)].map(|v| (v / 255.0) as f32);
    let inv = [ctx.flag("invertRedChannel"), ctx.flag("invertGreenChannel"), ctx.flag("invertBlueChannel")];
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0) as f32;
    crate::util::map_straight(&mut b.img, |c| mix3([0, 1, 2].map(|i| if (c[i] >= t[i]) != inv[i] { 1.0 } else { 0.0 }), c, keep));
    b
}

effect!(
    BLOCK_LOAD_DEF,
    "blockLoad",
    "Block Load",
    "the picture coming in as it would loading: big blocks, then smaller and smaller (animate the completion)",
    [
        pct("completion", "Completion", 0.0, 0.0, 100.0),
        flag("scanlines", "Scanlines", true).doc("each finer level comes in from the top down"),
        flag("smoothing", "Smoothing", true),
        flag("startCleared", "Start Cleared", true),
        flag("bilinear", "Bilinear", false).doc("blocks blended into each other"),
    ]
);
pub const BLOCK_LOAD: Spec = Spec::new(&BLOCK_LOAD_DEF, CATEGORY, block_load);

/// The picture as blocks `s` pixels square, each its mean: (blocks across, down, their colours).
fn blocks(img: &Image, s: usize) -> (usize, usize, Vec<Px>) {
    let (w, h) = (img.width, img.height);
    let (gw, gh) = (w.div_ceil(s).max(1), h.div_ceil(s).max(1));
    let mut out = vec![[0f32; 4]; gw * gh];
    for (k, o) in out.iter_mut().enumerate() {
        let (bx, by) = (k % gw, k / gw);
        let (mut acc, mut n) = ([0f32; 4], 0.0f32);
        for y in by * s..((by + 1) * s).min(h) {
            for x in bx * s..((bx + 1) * s).min(w) {
                let p = img.px[y * w + x];
                for c in 0..4 {
                    acc[c] += p[c];
                }
                n += 1.0;
            }
        }
        if n > 0.0 {
            *o = acc.map(|v| v / n);
        }
    }
    (gw, gh, out)
}

fn block_of(g: &(usize, usize, Vec<Px>), s: usize, x: usize, y: usize, blend: bool) -> Px {
    let (gw, gh, d) = g;
    if !blend || s == 1 {
        return d[(y / s).min(gh - 1) * gw + (x / s).min(gw - 1)];
    }
    let (fx, fy) = ((x as f32 + 0.5) / s as f32 - 0.5, (y as f32 + 0.5) / s as f32 - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let gx = |v: f32| (v as i64).clamp(0, *gw as i64 - 1) as usize;
    let gy = |v: f32| (v as i64).clamp(0, *gh as i64 - 1) as usize;
    let at = |i: f32, j: f32| d[gy(j) * gw + gx(i)];
    mix4(mix4(at(x0, y0), at(x0 + 1.0, y0), tx), mix4(at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0), tx), ty)
}

fn block_load(ctx: &Ctx, mut b: Buf) -> Buf {
    let done = (ctx.num("completion", 0.0) / 100.0).clamp(0.0, 1.0);
    if done >= 1.0 {
        return b;
    }
    let (scan, smooth, cleared, blend) = (ctx.flag("scanlines"), ctx.flag("smoothing"), ctx.flag("startCleared"), ctx.flag("bilinear"));
    let (w, h) = (b.img.width, b.img.height);
    let levels = (w.max(h).max(2) as f64).log2().ceil() as u32;
    // level k (blocks 2^(levels - k) across) is coming in while the completion is in its stretch
    let p = done * (levels + 1) as f64;
    let level = p.floor() as u32;
    let part = (p - level as f64) as f32;
    let size = |k: u32| 1usize << (levels - k.min(levels));
    let s_new = size(level);
    let new = blocks(&b.img, s_new);
    let old = (level > 0).then(|| (size(level - 1), blocks(&b.img, size(level - 1))));
    let rows = h.div_ceil(s_new) as f32;
    let soft = if smooth { 1.5f32 } else { 0.0 };
    let mut out = Image::new(w, h);
    map_px(&mut out, |x, y, _| {
        // how much of the new level is in on this row
        let t = if scan {
            let front = part * (rows + soft) - soft * 0.5;
            let r = (y / s_new) as f32 + 0.5;
            if soft > 0.0 { smoothstep(r - soft * 0.5, r + soft * 0.5, front) } else if r < front { 1.0 } else { 0.0 }
        } else if smooth {
            part
        } else {
            0.0
        };
        let before = match &old {
            Some((s, g)) => block_of(g, *s, x, y, blend),
            None if cleared => [0.0; 4],
            None => block_of(&new, s_new, x, y, blend),
        };
        if t <= 0.0 { before } else { mix4(before, block_of(&new, s_new, x, y, blend), t.min(1.0)) }
    });
    b.img = out;
    b
}

effect!(
    BURN_FILM_DEF,
    "burnFilm",
    "Burn Film",
    "the picture burning away like film stuck in a projector (animate the burn)",
    [pct("burn", "Burn", 0.0, 0.0, 100.0), point("center", "Center"), num("randomSeed", "Random Seed", 0.0).range(0.0, 1_000_000.0),]
);
pub const BURN_FILM: Spec = Spec::new(&BURN_FILM_DEF, CATEGORY, burn_film).points(&[("center", [0.5, 0.5])]);

fn burn_film(ctx: &Ctx, mut b: Buf) -> Buf {
    let burn = (ctx.num("burn", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    if burn <= 0.0 {
        return b;
    }
    let c = ctx.point("center");
    let seed = crate::noise::mix(&[0xb0b, ctx.num("randomSeed", 0.0).max(0.0) as u64]);
    let r = ctx.bounds;
    let diag = r.width().hypot(r.height()).max(1.0);
    // the burnt edge: holes past the line, a hot rim at it and a charred ring just inside
    let line = 1.0 - burn * 1.25;
    let band = 0.035 * (burn * 10.0).min(1.0);
    let (s, o) = (b.scale, b.origin);
    map_px(&mut b.img, |x, y, p| {
        let (lx, ly) = (o.x + (x as f64 + 0.5) / s, o.y + (y as f64 + 0.5) / s);
        let d = ((lx - c.x).hypot(ly - c.y) / diag) as f32;
        let n = (crate::noise::fractal(lx / 60.0, ly / 60.0, 0.0, seed, 4.0, 0.5, None) * 0.5 + 0.5) as f32;
        // it burns first near the middle and where the noise is high
        let f = n * 0.65 + (1.0 - d.min(1.0)) * 0.35;
        if f > line + band {
            return [0.0; 4];
        }
        if f <= line - band * 2.0 {
            return p;
        }
        let (col, a) = unpremul(p);
        // charred dark toward the hole, glowing orange right at it
        let k = smoothstep(line - band * 2.0, line, f);
        let hot = smoothstep(line, line + band, f);
        let char = 1.0 - k * 0.85;
        let c2 = col.map(|v| v * char);
        premul([c2[0] + (1.0 - c2[0]) * hot * 0.9, c2[1] + (0.45 - c2[1]) * hot * 0.9, c2[2] * (1.0 - hot)], a * (1.0 - hot * 0.6))
    });
    b
}

const BUMP_FROM: &[&str] = &["red", "green", "blue", "alpha", "luminance", "lightness"];

/// How high the surface is (0 to 1) over the buffer: a channel of the bump layer (or this one),
/// softened.
fn heights(ctx: &Ctx, b: &Buf, from: &str) -> Vec<f32> {
    let src = ctx.layer_over(from, b, Take::All, true).unwrap_or_else(|| b.img.clone());
    let ch = ctx.choice("property");
    let mut plane = Image::new(src.width, src.height);
    for (o, p) in plane.px.iter_mut().zip(&src.px) {
        let (c, a) = unpremul(*p);
        let v = match ch {
            0 => c[0] * a,
            1 => c[1] * a,
            2 => c[2] * a,
            3 => a,
            4 => luma(c) * a,
            _ => hsl(c).2 * a,
        };
        *o = [v; 4];
    }
    let soft = ctx.num("softness", 10.0) * b.scale * 0.5;
    if soft > 0.05 {
        plane = filter::gaussian(&plane, soft, soft, Edge::Repeat);
    }
    plane.px.iter().map(|p| p[0]).collect()
}

/// The light on a glass or plastic surface.
struct Light {
    strength: f32,
    colour: [f32; 3],
    point: Option<(f64, f64, f32)>,
    toward: [f32; 3],
    ambient: f32,
    diffuse: f32,
    specular: f32,
    shine: f32,
    metal: f32,
}

impl Light {
    fn of(ctx: &Ctx, b: &Buf) -> Light {
        let lift = ctx.num("lightHeight", 50.0).clamp(-100.0, 100.0);
        let up = (lift / 100.0 * 90.0).to_radians();
        let az = ctx.num("lightDirection", -45.0).to_radians();
        let p = b.pixel(ctx.point("lightPosition"));
        let reach = (lift / 100.0 * ctx.bounds.width().max(ctx.bounds.height()) * b.scale * 0.5) as f32;
        Light {
            strength: (ctx.num("lightIntensity", 100.0) / 100.0) as f32,
            colour: rgb3(ctx.color("lightColor")),
            point: (ctx.choice("lightType") == 1).then_some((p.x, p.y, reach.max(1.0))),
            toward: [(az.sin() * up.cos()) as f32, (-az.cos() * up.cos()) as f32, up.sin() as f32],
            ambient: (ctx.num("ambient", 75.0) / 100.0) as f32,
            diffuse: (ctx.num("diffuse", 32.0) / 100.0) as f32,
            specular: (ctx.num("specular", 26.0) / 100.0) as f32,
            shine: 1.0 / ctx.num("roughness", 0.05).clamp(0.001, 1.0) as f32,
            metal: (ctx.num("metal", 100.0) / 100.0) as f32,
        }
    }

    fn to_light(&self, x: f64, y: f64) -> [f32; 3] {
        match self.point {
            Some((px, py, h)) => {
                let v = [(px - x) as f32, (py - y) as f32, h];
                let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
                v.map(|c| c / l)
            }
            None => self.toward,
        }
    }

    /// The colour `c` lit on a surface facing `n`: a flat surface keeps its colour, slopes to
    /// the light brighten, slopes away darken, and the shine goes on top.
    fn shade(&self, c: [f32; 3], n: [f32; 3], l: [f32; 3]) -> [f32; 3] {
        let lit_n = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0);
        let flat = self.ambient + self.diffuse * l[2].max(0.0);
        let lit = if flat > 1e-4 { (self.ambient + self.diffuse * lit_n) / flat } else { 1.0 };
        let lit = 1.0 + (lit - 1.0) * self.strength;
        let half = [l[0], l[1], l[2] + 1.0];
        let hl = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt().max(1e-6);
        let gloss = self.specular * self.strength * ((n[0] * half[0] + n[1] * half[1] + n[2] * half[2]) / hl).max(0.0).powf(self.shine);
        [0, 1, 2].map(|i| {
            let shine_c = self.colour[i] * (1.0 - self.metal) + c[i] * self.colour[i] * self.metal;
            c[i] * lit * (1.0 + (self.colour[i] - 1.0) * self.strength.min(1.0)) + gloss * shine_c
        })
    }
}

/// Which way the surface faces at a pixel, its heights `k` times steeper.
fn facing(h: &[f32], w: usize, hh: usize, x: usize, y: usize, k: f32) -> [f32; 3] {
    let at = |x: i64, y: i64| h[(y.clamp(0, hh as i64 - 1) as usize) * w + x.clamp(0, w as i64 - 1) as usize];
    let (xi, yi) = (x as i64, y as i64);
    let (gx, gy) = ((at(xi + 1, yi) - at(xi - 1, yi)) * 0.5 * k, (at(xi, yi + 1) - at(xi, yi - 1)) * 0.5 * k);
    let l = (gx * gx + gy * gy + 1.0).sqrt();
    [-gx / l, -gy / l, 1.0 / l]
}

macro_rules! lit_surface {
    ($def:ident, $id:literal, $name:literal, $doc:literal, $layer:literal, $layer_name:literal, [$($own:expr),*], $amb:expr, $dif:expr, $spec:expr, $metal:expr) => {
        effect!(
            $def,
            $id,
            $name,
            $doc,
            [
                layer($layer, $layer_name).doc("none: the layer itself"),
                choice("property", "Property", BUMP_FROM, 3),
                px("softness", "Softness", 10.0, 200.0),
                $($own,)*
                ranged("lightIntensity", "Light Intensity", 100.0, 0.0, 400.0),
                color("lightColor", "Light Color", 1.0, 1.0, 1.0),
                choice("lightType", "Light Type", &["distantLight", "pointLight"], 0),
                ranged("lightHeight", "Light Height", 50.0, -100.0, 100.0),
                point("lightPosition", "Light Position"),
                angle("lightDirection", "Light Direction", -45.0),
                ranged("ambient", "Ambient", $amb, 0.0, 200.0),
                ranged("diffuse", "Diffuse", $dif, 0.0, 200.0),
                ranged("specular", "Specular", $spec, 0.0, 200.0),
                ranged("roughness", "Roughness", 0.05, 0.001, 0.5),
                pct("metal", "Metal", $metal, 0.0, 100.0),
            ]
        );
    };
}

lit_surface!(
    GLASS_DEF,
    "glass",
    "Glass",
    "the layer seen through bumpy glass: bent and lit by a bump layer's heights",
    "bumpMap",
    "Bump Map",
    [ranged("height", "Height", 50.0, -100.0, 100.0), ranged("displacement", "Displacement", 100.0, -500.0, 500.0)],
    75.0,
    32.0,
    26.0,
    100.0
);
pub const GLASS: Spec = Spec::new(&GLASS_DEF, CATEGORY, glass).points(&[("lightPosition", [0.25, 0.25])]);

fn glass(ctx: &Ctx, mut b: Buf) -> Buf {
    let h = heights(ctx, &b, "bumpMap");
    let height = (ctx.num("height", 50.0) / 100.0) as f32;
    let bend = ctx.num("displacement", 100.0) * b.scale * 0.25;
    let light = Light::of(ctx, &b);
    let (w, hh) = (b.img.width, b.img.height);
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, p| {
        if p[3] <= 0.0 {
            return [0.0; 4];
        }
        let n = facing(&h, w, hh, x, y, height * 25.0);
        let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
        let seen = if bend != 0.0 && height != 0.0 { held(&src, fx + n[0] as f64 * bend, fy + n[1] as f64 * bend) } else { p };
        let c = if seen[3] > 0.0 { unpremul(seen).0 } else { unpremul(p).0 };
        premul(light.shade(c, n, light.to_light(fx, fy)), p[3])
    });
    b
}

lit_surface!(
    PLASTIC_DEF,
    "plastic",
    "Plastic",
    "the layer moulded in shiny plastic, shaped and lit by a bump layer's heights",
    "bumpLayer",
    "Bump Layer",
    [ranged("height", "Height", 25.0, -100.0, 100.0), pct("cutMin", "Cut Min", 0.0, 0.0, 100.0), pct("cutMax", "Cut Max", 100.0, 0.0, 100.0)],
    100.0,
    70.0,
    50.0,
    50.0
);
pub const PLASTIC: Spec = Spec::new(&PLASTIC_DEF, CATEGORY, plastic).points(&[("lightPosition", [0.25, 0.25])]);

fn plastic(ctx: &Ctx, mut b: Buf) -> Buf {
    let mut h = heights(ctx, &b, "bumpLayer");
    let (lo, hi) = ((ctx.num("cutMin", 0.0) / 100.0) as f32, (ctx.num("cutMax", 100.0) / 100.0) as f32);
    if lo > 0.0 || hi < 1.0 {
        for v in &mut h {
            *v = v.clamp(lo.min(hi), hi.max(lo));
        }
    }
    let k = (ctx.num("height", 25.0) / 100.0 * 25.0) as f32;
    let light = Light::of(ctx, &b);
    let (w, hh) = (b.img.width, b.img.height);
    map_px(&mut b.img, |x, y, p| {
        if p[3] <= 0.0 {
            return p;
        }
        let (c, a) = unpremul(p);
        premul(light.shade(c, facing(&h, w, hh, x, y, k), light.to_light(x as f64 + 0.5, y as f64 + 0.5)), a)
    });
    b
}

effect!(
    HEX_TILE_DEF,
    "hexTile",
    "Hex Tile",
    "the picture round a point repeated in hexagons over the layer",
    [
        choice("render", "Render", &["normal", "mirrored", "rotated"], 0),
        px("radius", "Radius", 50.0, 4000.0),
        point("center", "Center"),
        angle("rotate", "Rotate", 0.0),
        pct("smearing", "Smearing", 0.0, 0.0, 100.0),
    ]
);
pub const HEX_TILE: Spec = Spec::new(&HEX_TILE_DEF, CATEGORY, hex_tile).points(&[("center", [0.5, 0.5])]);

/// The hexagon (pointy at the top, `r` from its middle to a corner) a point is in: its middle and
/// which one it is.
fn hexagon(x: f64, y: f64, r: f64) -> ((f64, f64), (i64, i64)) {
    let q = (3f64.sqrt() / 3.0 * x - y / 3.0) / r;
    let rr = (2.0 / 3.0 * y) / r;
    let (cx, cz) = (q, rr);
    let cy = -cx - cz;
    let (mut rx, ry, mut rz) = (cx.round(), cy.round(), cz.round());
    let (dx, dy, dz) = ((rx - cx).abs(), (ry - cy).abs(), (rz - cz).abs());
    if dx > dy && dx > dz {
        rx = -ry - rz;
    } else if dy <= dz {
        rz = -rx - ry;
    }
    ((r * 3f64.sqrt() * (rx + rz / 2.0), r * 1.5 * rz), (rx as i64, rz as i64))
}

fn hex_tile(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = (ctx.num("radius", 50.0) * b.scale).max(0.5);
    let c = b.pixel(ctx.point("center"));
    let (sn, cs) = ctx.num("rotate", 0.0).to_radians().sin_cos();
    let smear = (ctx.num("smearing", 0.0) / 100.0).clamp(0.0, 1.0);
    let mode = ctx.choice("render");
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let (dx, dy) = (x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
        let (u, v) = (dx * cs + dy * sn, -dx * sn + dy * cs);
        let ((hx, hy), (qi, ri)) = hexagon(u, v, r);
        let (mut lu, mut lv) = (u - hx, v - hy);
        let odd = (qi + ri).rem_euclid(2) == 1;
        if odd && mode >= 1 {
            lu = -lu;
        }
        if odd && mode == 2 {
            lv = -lv;
        }
        // every hexagon shows the one round the centre
        let (tx, ty) = (c.x + lu * cs - lv * sn, c.y + lu * sn + lv * cs);
        held(&src, tx + (x as f64 + 0.5 - tx) * smear, ty + (y as f64 + 0.5 - ty) * smear)
    });
    b
}

effect!(
    SMOOTHIE_DEF,
    "smoothie",
    "Smoothie",
    "the layer recoloured with the colours found along a line across it, flowing by a layer's brightness",
    [
        layer("flowLayer", "Flow Layer").doc("none: the layer itself"),
        choice("property", "Property", &["red", "green", "blue", "alpha", "luminance", "lightness", "hue", "saturation"], 4),
        px("smoothness", "Smoothness", 20.0, 200.0),
        point("sampleA", "Sample A"),
        point("sampleB", "Sample B"),
        angle("phase", "Phase", 0.0),
        choice("colorLoop", "Color Loop", &["1x", "2x", "3x", "4x"], 0),
    ]
);
pub const SMOOTHIE: Spec = Spec::new(&SMOOTHIE_DEF, CATEGORY, smoothie).points(&[("sampleA", [0.25, 0.5]), ("sampleB", [0.75, 0.5])]);

fn smoothie(ctx: &Ctx, mut b: Buf) -> Buf {
    let flow = ctx.layer_over("flowLayer", &b, Take::All, true).unwrap_or_else(|| b.img.clone());
    let ch = ctx.choice("property");
    let mut plane = Image::new(flow.width, flow.height);
    for (o, p) in plane.px.iter_mut().zip(&flow.px) {
        let (c, a) = unpremul(*p);
        let (h, s, l) = hsl(c);
        *o = [match ch {
            0 => c[0],
            1 => c[1],
            2 => c[2],
            3 => a,
            5 => l,
            6 => h,
            7 => s,
            _ => luma(c),
        }; 4];
    }
    let soft = ctx.num("smoothness", 20.0) * b.scale * 0.5;
    if soft > 0.05 {
        plane = filter::gaussian(&plane, soft, soft, Edge::Repeat);
    }
    let (a, z) = (b.pixel(ctx.point("sampleA")), b.pixel(ctx.point("sampleB")));
    // the colours along A to B
    const N: usize = 256;
    let palette: Vec<[f32; 3]> = (0..N)
        .map(|i| {
            let t = i as f64 / (N - 1) as f64;
            unpremul(held(&b.img, a.x + (z.x - a.x) * t, a.y + (z.y - a.y) * t)).0
        })
        .collect();
    let phase = (ctx.num("phase", 0.0) / 360.0) as f32;
    let loops = (ctx.choice("colorLoop").min(3) + 1) as f32;
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        if p[3] <= 0.0 {
            return p;
        }
        let t = (plane.px[y * w + x][0] * loops + phase).rem_euclid(1.0);
        // there and back, so it loops without a seam
        let t = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
        let f = t * (N - 1) as f32;
        let i = (f as usize).min(N - 2);
        premul(mix3(palette[i], palette[i + 1], f - i as f32), p[3])
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 13] = [BRUSH, KALEIDOSCOPE, REPEAT, MOTION_TILE, VIGNETTE, CHANNEL_THRESHOLD, THRESHOLD_RGB, BLOCK_LOAD, BURN_FILM, GLASS, PLASTIC, HEX_TILE, SMOOTHIE];

#[cfg(test)]
mod tests;
