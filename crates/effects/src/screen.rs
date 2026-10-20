//! Screen Keyer: a whole green- or blue-screen keyer.
//!
//! Colours are first made neutral by the alpha bias (and despill bias): each channel scaled so
//! that colour becomes grey. A pixel's screen difference is the screen's strongest channel less a
//! balance of the other two; the matte is `1 − gain × difference ÷ the screen colour's
//! difference`. That screen matte is clipped (with rollback giving back edge detail lost to it),
//! despotted, shrunk or grown and softened, then joined with the inside and outside masks and the
//! layer's own alpha. The subject is the picture with the screen colour taken out as much as it
//! shows through, then despilled; where clipping or the inside mask made it more solid than the
//! screen said, its colour can come from the source or a replace colour; and the subject and its
//! edges can be colour corrected.

use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px};

use crate::color::rgb3;
use crate::defs::{angle, choice, color, effect, flag, num, pct, ranged};
use crate::draw::inside;
use crate::matte::{rounded_extreme, soften};
use crate::util::{luma, premul, rgb, unpremul};
use crate::{Buf, Ctx, Spec};

const VIEWS: &[&str] = &[
    "source",
    "sourceAlpha",
    "correctedSource",
    "colourCorrectionEdges",
    "screenMatte",
    "insideMask",
    "outsideMask",
    "combinedMatte",
    "status",
    "intermediateResult",
    "finalResult",
];
const REPLACE: &[&str] = &["none", "source", "hardColour", "softColour"];
const SUPPRESS: &[&str] = &["none", "red", "green", "blue", "cyan", "magenta", "yellow"];
const CROP: &[&str] = &["colour", "repeat", "reflect", "wrap"];

effect!(
    SCREEN_KEYER_DEF,
    "screenKeyer",
    "Screen Keyer",
    "a green or blue screen keyed out: the screen matte cleaned, masks inside and out, the spill taken off, the subject and its edges colour corrected",
    [
        choice("view", "View", VIEWS, 10),
        flag("unpremultiplyResult", "Unpremultiply Result", true),
        color("screenColour", "Screen Colour", 0.0, 1.0, 0.0),
        ranged("screenGain", "Screen Gain", 100.0, 0.0, 200.0).doc("higher: more of the screen keyed out"),
        pct("screenBalance", "Screen Balance", 50.0, 0.0, 100.0),
        color("despillBias", "Despill Bias", 0.5, 0.5, 0.5),
        color("alphaBias", "Alpha Bias", 0.5, 0.5, 0.5),
        flag("lockBiasesTogether", "Lock Biases Together", true),
        ranged("screenPreBlur", "Screen Pre-blur", 0.0, 0.0, 100.0),
        pct("clipBlack", "Clip Black", 0.0, 0.0, 100.0),
        pct("clipWhite", "Clip White", 100.0, 0.0, 100.0),
        ranged("clipRollback", "Clip Rollback", 0.0, 0.0, 100.0),
        ranged("screenShrinkGrow", "Screen Shrink/Grow", 0.0, -100.0, 100.0),
        ranged("screenSoftness", "Screen Softness", 0.0, 0.0, 100.0),
        ranged("screenDespotBlack", "Screen Despot Black", 0.0, 0.0, 100.0).doc("small holes in the matte filled"),
        ranged("screenDespotWhite", "Screen Despot White", 0.0, 0.0, 100.0).doc("small specks in the screen cleared"),
        choice("replaceMethod", "Replace Method", REPLACE, 3),
        color("replaceColour", "Replace Colour", 0.5, 0.5, 0.5),
        num("insideMask", "Inside Mask", 0.0).range(0.0, 1000.0).doc("a mask (from 1) kept solid; 0: none"),
        ranged("insideMaskSoftness", "Inside Mask Softness", 0.0, 0.0, 100.0),
        flag("invertInsideMask", "Invert Inside Mask", false),
        choice("insideReplaceMethod", "Inside Replace Method", REPLACE, 1),
        color("insideReplaceColour", "Inside Replace Colour", 0.5, 0.5, 0.5),
        choice("sourceAlpha", "Source Alpha", &["ignore", "addToInsideMask", "normal"], 2),
        num("outsideMask", "Outside Mask", 0.0).range(0.0, 1000.0).doc("a mask (from 1) keyed out whatever is in it; 0: none"),
        ranged("outsideMaskSoftness", "Outside Mask Softness", 0.0, 0.0, 100.0),
        flag("invertOutsideMask", "Invert Outside Mask", false),
        flag("enableColourCorrection", "Enable Colour Correction", false),
        ranged("saturation", "Saturation", 100.0, 0.0, 400.0),
        ranged("contrast", "Contrast", 0.0, -100.0, 100.0),
        ranged("brightness", "Brightness", 0.0, -100.0, 100.0),
        choice("colourSuppression", "Colour Suppression", SUPPRESS, 0),
        pct("suppressionBalance", "Suppression Balance", 50.0, 0.0, 100.0),
        pct("suppressionAmount", "Suppression Amount", 0.0, 0.0, 100.0),
        angle("colourBalanceHue", "Colour Balance Hue", 0.0),
        pct("colourBalanceSaturation", "Colour Balance Saturation", 0.0, 0.0, 100.0),
        flag("enableEdgeColourCorrection", "Enable Edge Colour Correction", false),
        pct("edgeHardness", "Edge Hardness", 0.0, 0.0, 100.0),
        ranged("edgeSoftness", "Edge Softness", 0.0, 0.0, 100.0),
        ranged("edgeGrow", "Edge Grow", 1.0, 0.0, 100.0),
        ranged("edgeSaturation", "Edge Saturation", 100.0, 0.0, 400.0),
        ranged("edgeContrast", "Edge Contrast", 0.0, -100.0, 100.0),
        ranged("edgeBrightness", "Edge Brightness", 0.0, -100.0, 100.0),
        choice("edgeColourSuppression", "Edge Colour Suppression", SUPPRESS, 0),
        pct("edgeSuppressionBalance", "Edge Suppression Balance", 50.0, 0.0, 100.0),
        pct("edgeSuppressionAmount", "Edge Suppression Amount", 0.0, 0.0, 100.0),
        angle("edgeColourBalanceHue", "Edge Colour Balance Hue", 0.0),
        pct("edgeColourBalanceSaturation", "Edge Colour Balance Saturation", 0.0, 0.0, 100.0),
        choice("xMethod", "X Method", CROP, 0),
        choice("yMethod", "Y Method", CROP, 0),
        color("edgeColour", "Edge Colour", 0.0, 0.0, 0.0),
        pct("edgeColourAlpha", "Edge Colour Alpha", 0.0, 0.0, 100.0),
        pct("cropLeft", "Crop Left", 0.0, 0.0, 100.0),
        pct("cropRight", "Crop Right", 0.0, 0.0, 100.0),
        pct("cropTop", "Crop Top", 0.0, 0.0, 100.0),
        pct("cropBottom", "Crop Bottom", 0.0, 0.0, 100.0),
    ]
);
pub const SCREEN_KEYER: Spec = Spec::new(&SCREEN_KEYER_DEF, "keying", screen_keyer);

/// The screen's strongest channel, and the other two (the stronger of them first).
fn primary(s: [f32; 3]) -> (usize, usize, usize) {
    let pi = if s[1] >= s[0] && s[1] >= s[2] {
        1
    } else if s[2] >= s[0] {
        2
    } else {
        0
    };
    let (a, b) = match pi {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    if s[a] >= s[b] { (pi, a, b) } else { (pi, b, a) }
}

/// The strongest channel less a balance of the other two.
pub fn screen_difference(c: [f32; 3], pi: usize, o1: usize, o2: usize, balance: f32) -> f32 {
    c[pi] - (balance * c[o1] + (1.0 - balance) * c[o2])
}

/// Each channel scaled so `bias` becomes grey (a grey bias changes nothing).
fn neutral(c: [f32; 3], bias: [f32; 3]) -> [f32; 3] {
    let l = luma(bias).max(1e-4);
    [0, 1, 2].map(|i| c[i] * l / bias[i].max(1e-4))
}

fn correct(c: [f32; 3], sat: f32, contrast: f32, bright: f32) -> [f32; 3] {
    let l = luma(c);
    c.map(|v| l + (v - l) * sat).map(|v| ((v - 0.5) * contrast + 0.5) * bright)
}

/// A primary's (or a secondary's) excess taken off by `amount`.
fn suppress(mut c: [f32; 3], kind: u32, balance: f32, amount: f32) -> [f32; 3] {
    if kind == 0 || amount <= 0.0 {
        return c;
    }
    if kind <= 3 {
        let k = kind as usize - 1;
        let (a, b) = ((k + 1) % 3, (k + 2) % 3);
        let limit = c[a] * balance + c[b] * (1.0 - balance);
        c[k] -= (c[k] - limit).max(0.0) * amount;
    } else {
        // cyan is green and blue over red; magenta red and blue over green; yellow red and green over blue
        let third = (kind - 4) as usize;
        let (a, b) = ((third + 1) % 3, (third + 2) % 3);
        let excess = (c[a].min(c[b]) - c[third]).max(0.0) * amount;
        c[a] -= excess;
        c[b] -= excess;
    }
    c
}

/// A colour wheel's push: toward the hue, by the saturation.
fn tint(hue: f64, sat: f64) -> [f32; 3] {
    let s = (sat / 100.0) as f32;
    if s <= 0.0 {
        return [0.0; 3];
    }
    let c = rgb((hue / 360.0).rem_euclid(1.0) as f32, 1.0, 0.5);
    let l = luma(c);
    c.map(|v| (v - l) * s * 0.5)
}

fn tinted(c: [f32; 3], t: [f32; 3]) -> [f32; 3] {
    [c[0] + t[0], c[1] + t[1], c[2] + t[2]].map(|v| v.max(0.0))
}

/// Where a mask covers (softened, inverted as asked), or None.
fn mask_cover(ctx: &Ctx, b: &Buf, id: &str, soft: f64, flip: bool) -> Option<Vec<f32>> {
    let m = ctx.mask(ctx.num(id, 0.0).round().max(0.0) as usize)?;
    let pts: Vec<[f64; 2]> = m
        .points
        .iter()
        .map(|q| {
            let p = b.pixel(veymelo_geom::vec2(q[0], q[1]));
            [p.x, p.y]
        })
        .collect();
    let (w, h) = (b.img.width, b.img.height);
    let mut cover: Vec<f32> = (0..w * h).map(|i| if inside(&pts, [(i % w) as f64 + 0.5, (i / w) as f64 + 0.5]) != m.inverted { 1.0 } else { 0.0 }).collect();
    let s = soft * b.scale;
    if s > 0.0 {
        cover = soften(&cover, w, h, s * 0.5);
    }
    if flip {
        for v in &mut cover {
            *v = 1.0 - *v;
        }
    }
    Some(cover)
}

/// The picture with its edges cropped: past them the edge colour, or the picture repeated,
/// reflected or wrapped.
fn cropped_source(ctx: &Ctx, b: &Buf) -> Image {
    let crops = ["cropLeft", "cropRight", "cropTop", "cropBottom"].map(|k| ctx.num(k, 0.0) / 100.0);
    if crops.iter().all(|c| *c <= 0.0) {
        return b.img.clone();
    }
    let r = ctx.bounds;
    let (a, z) = (b.pixel(veymelo_geom::vec2(r.x0, r.y0)), b.pixel(veymelo_geom::vec2(r.x1, r.y1)));
    let (lw, lh) = (z.x - a.x, z.y - a.y);
    let (x0, x1, y0, y1) = (a.x + crops[0] * lw, a.x + (1.0 - crops[1]) * lw, a.y + crops[2] * lh, a.y + (1.0 - crops[3]) * lh);
    let methods = [ctx.choice("xMethod"), ctx.choice("yMethod")];
    let ea = (ctx.num("edgeColourAlpha", 0.0) / 100.0) as f32;
    if methods == [0, 0] && ea <= 0.0 {
        // see-through edge colour: the borders are cut from the result instead
        return b.img.clone();
    }
    let edge = premul(rgb3(ctx.color("edgeColour")), ea);
    let fold = |v: f64, lo: f64, hi: f64, m: u32| -> Option<f64> {
        let span = (hi - lo).max(1.0);
        if v >= lo && v < hi {
            return Some(v);
        }
        match m {
            1 => Some(v.clamp(lo, hi - 0.5)),
            2 => {
                let t = (v - lo).rem_euclid(2.0 * span);
                Some(lo + if t >= span { 2.0 * span - t - 0.5 } else { t })
            }
            3 => Some(lo + (v - lo).rem_euclid(span)),
            _ => None,
        }
    };
    let (w, h) = (b.img.width, b.img.height);
    let mut out = Image::new(w, h);
    for (i, p) in out.px.iter_mut().enumerate() {
        let (fx, fy) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
        *p = match (fold(fx, x0, x1, methods[0]), fold(fy, y0, y1, methods[1])) {
            (Some(sx), Some(sy)) => b.img.get(sx.floor() as i64, sy.floor() as i64),
            _ => edge,
        };
    }
    out
}

fn screen_keyer(ctx: &Ctx, mut b: Buf) -> Buf {
    let view = ctx.choice("view");
    if view == 0 {
        return b;
    }
    let screen = rgb3(ctx.color("screenColour"));
    let (pi, o1, o2) = primary(screen);
    let gain = (ctx.num("screenGain", 100.0) / 100.0) as f32;
    let bal = (ctx.num("screenBalance", 50.0) / 100.0) as f32;
    let dbias = rgb3(ctx.color("despillBias"));
    let abias = if ctx.flag("lockBiasesTogether") { dbias } else { rgb3(ctx.color("alphaBias")) };
    let sd = screen_difference(neutral(screen, abias), pi, o1, o2, bal).max(1e-3);
    let (cb, cw) = ((ctx.num("clipBlack", 0.0) / 100.0) as f32, (ctx.num("clipWhite", 100.0) / 100.0) as f32);
    let px = |id: &str| ctx.num(id, 0.0) * b.scale;
    let (rollback, preblur, grow, soft) = (px("clipRollback"), px("screenPreBlur"), px("screenShrinkGrow"), px("screenSoftness"));
    let (despot_b, despot_w) = (px("screenDespotBlack"), px("screenDespotWhite"));
    let (w, h) = (b.img.width, b.img.height);
    let src = cropped_source(ctx, &b);
    let keyed = if preblur > 0.0 { filter::gaussian(&src, preblur * 0.5, preblur * 0.5, Edge::Repeat) } else { src.clone() };
    // how solid the subject is by the screen alone
    let raw: Vec<f32> = keyed
        .px
        .iter()
        .map(|p| {
            let (c, _) = unpremul(*p);
            (1.0 - gain * screen_difference(neutral(c, abias), pi, o1, o2, bal) / sd).clamp(0.0, 1.0)
        })
        .collect();
    let mut m: Vec<f32> = raw.iter().map(|v| ((v - cb) / (cw - cb).max(1e-4)).clamp(0.0, 1.0)).collect();
    if rollback > 0.0 {
        // near the clipped edge, the detail clipping took away given back
        let (grown, shrunk) = (rounded_extreme(&m, w, h, rollback, true), rounded_extreme(&m, w, h, rollback, false));
        for i in 0..w * h {
            if grown[i] > shrunk[i] + 1e-4 {
                m[i] = m[i].max(raw[i]).min(grown[i]);
            }
        }
    }
    if despot_b > 0.0 {
        m = rounded_extreme(&rounded_extreme(&m, w, h, despot_b, true), w, h, despot_b, false);
    }
    if despot_w > 0.0 {
        m = rounded_extreme(&rounded_extreme(&m, w, h, despot_w, false), w, h, despot_w, true);
    }
    if grow.abs() > 1e-6 {
        m = rounded_extreme(&m, w, h, grow.abs(), grow > 0.0);
    }
    if soft > 0.0 {
        m = soften(&m, w, h, soft * 0.5);
    }
    let screen_matte = m;
    let inner = mask_cover(ctx, &b, "insideMask", ctx.num("insideMaskSoftness", 0.0), ctx.flag("invertInsideMask"));
    let outer = mask_cover(ctx, &b, "outsideMask", ctx.num("outsideMaskSoftness", 0.0), ctx.flag("invertOutsideMask"));
    let alpha_mode = ctx.choice("sourceAlpha");
    let combined: Vec<f32> = (0..w * h)
        .map(|i| {
            let sa = src.px[i][3].clamp(0.0, 1.0);
            let ins = inner.as_ref().map_or(0.0, |p| p[i]);
            let ins = if alpha_mode == 1 { ins.max(sa) } else { ins };
            let mut a = screen_matte[i].max(ins);
            if let Some(o) = &outer {
                a *= 1.0 - o[i];
            }
            if alpha_mode == 2 {
                a *= sa;
            }
            a.clamp(0.0, 1.0)
        })
        .collect();
    let edges: Option<Vec<f32>> = ctx.flag("enableEdgeColourCorrection").then(|| {
        let grow = px("edgeGrow").max(0.0);
        let hard = (ctx.num("edgeHardness", 0.0) / 100.0) as f32;
        let esoft = px("edgeSoftness");
        // where the matte is part see-through, grown and softened
        let mut band: Vec<f32> = combined.iter().map(|a| if *a > 1e-3 && *a < 0.999 { 1.0 } else { 0.0 }).collect();
        if grow > 0.0 {
            band = rounded_extreme(&band, w, h, grow, true);
        }
        if esoft > 0.0 {
            band = soften(&band, w, h, esoft * 0.5);
        }
        band.iter().map(|v| (v * (1.0 + hard * 4.0)).min(1.0)).collect()
    });
    let (replace, rc) = (ctx.choice("replaceMethod"), rgb3(ctx.color("replaceColour")));
    let (in_replace, in_rc) = (ctx.choice("insideReplaceMethod"), rgb3(ctx.color("insideReplaceColour")));
    let fg_cc = ctx.flag("enableColourCorrection");
    let (fs, fc, fb) = ((ctx.num("saturation", 100.0) / 100.0) as f32, (ctx.num("contrast", 0.0) / 100.0) as f32 + 1.0, (ctx.num("brightness", 0.0) / 100.0) as f32 + 1.0);
    let (es, ec, eb) = ((ctx.num("edgeSaturation", 100.0) / 100.0) as f32, (ctx.num("edgeContrast", 0.0) / 100.0) as f32 + 1.0, (ctx.num("edgeBrightness", 0.0) / 100.0) as f32 + 1.0);
    let fg_sup = (ctx.choice("colourSuppression"), (ctx.num("suppressionBalance", 50.0) / 100.0) as f32, (ctx.num("suppressionAmount", 0.0) / 100.0) as f32);
    let edge_sup = (ctx.choice("edgeColourSuppression"), (ctx.num("edgeSuppressionBalance", 50.0) / 100.0) as f32, (ctx.num("edgeSuppressionAmount", 0.0) / 100.0) as f32);
    let fg_tint = tint(ctx.num("colourBalanceHue", 0.0), ctx.num("colourBalanceSaturation", 0.0));
    let edge_tint = tint(ctx.num("edgeColourBalanceHue", 0.0), ctx.num("edgeColourBalanceSaturation", 0.0));
    let unpremultiply = ctx.flag("unpremultiplyResult");
    let crops = ["cropLeft", "cropRight", "cropTop", "cropBottom"].map(|k| ctx.num(k, 0.0) / 100.0);
    let cut_borders = ctx.num("edgeColourAlpha", 0.0) <= 0.0 && ctx.choice("xMethod") == 0 && ctx.choice("yMethod") == 0;
    let r = ctx.bounds;
    let (o, s) = (b.origin, b.scale);
    // how much of the despill bias's strongest channel is its light
    let l0 = (luma(dbias) / dbias[pi].max(1e-4)).max(1e-4);
    for (i, out) in b.img.px.iter_mut().enumerate() {
        let (c, _) = unpremul(src.px[i]);
        let a = combined[i];
        let sm = screen_matte[i];
        let (lx, ly) = (((o.x + ((i % w) as f64 + 0.5) / s) - r.x0) / r.width().max(1e-9), ((o.y + ((i / w) as f64 + 0.5) / s) - r.y0) / r.height().max(1e-9));
        let cropped = cut_borders && (lx < crops[0] || lx > 1.0 - crops[1] || ly < crops[2] || ly > 1.0 - crops[3]);
        // the screen taken out as much as it shows through (by the unclipped matte), then the spill
        let ar = raw[i].max(1e-4);
        let mut fg = [0, 1, 2].map(|k| ((c[k] - (1.0 - ar) * screen[k]) / ar).max(0.0));
        let spill = screen_difference(neutral(fg, dbias), pi, o1, o2, bal).max(0.0);
        fg[pi] -= spill / l0;
        let intermediate = fg;
        // colour for where the matte was made more solid than the screen said
        let raised = (sm - raw[i]).max(0.0);
        let raised_in = inner.as_ref().map_or(0.0, |p| (p[i] - sm).max(0.0));
        let replaced = |fg: [f32; 3], how: u32, rc: [f32; 3], k: f32| -> [f32; 3] {
            if k <= 0.0 {
                return fg;
            }
            match how {
                1 => [0, 1, 2].map(|j| fg[j] + (c[j] - fg[j]) * k),
                2 => rc,
                3 => {
                    let l = luma(c) / luma(rc).max(1e-4);
                    [0, 1, 2].map(|j| fg[j] + (rc[j] * l - fg[j]) * k)
                }
                _ => fg,
            }
        };
        fg = replaced(fg, replace, rc, raised);
        fg = replaced(fg, in_replace, in_rc, raised_in);
        if fg_cc {
            fg = tinted(suppress(correct(fg, fs, fc, fb), fg_sup.0, fg_sup.1, fg_sup.2), fg_tint);
        }
        let edge = edges.as_ref().map_or(0.0, |e| e[i]);
        if edge > 0.0 {
            let ce = tinted(suppress(correct(fg, es, ec, eb), edge_sup.0, edge_sup.1, edge_sup.2), edge_tint);
            fg = [0, 1, 2].map(|k| fg[k] + (ce[k] - fg[k]) * edge);
        }
        let grey = |v: f32| -> Px { [v, v, v, 1.0] };
        *out = match view {
            1 => grey(src.px[i][3]),
            2 => {
                let mut cs = neutral(c, dbias);
                let s2 = screen_difference(cs, pi, o1, o2, bal).max(0.0);
                cs[pi] -= s2;
                if fg_cc {
                    cs = correct(cs, fs, fc, fb);
                }
                [cs[0], cs[1], cs[2], 1.0]
            }
            3 => grey(edge),
            4 => grey(sm),
            5 => grey(inner.as_ref().map_or(0.0, |p| p[i])),
            6 => grey(outer.as_ref().map_or(0.0, |p| p[i])),
            7 => grey(a),
            // black keyed out, white solid, grey in between
            8 => grey(if a <= 1e-3 {
                0.0
            } else if a >= 0.999 {
                1.0
            } else {
                0.5
            }),
            9 => premul(intermediate, sm),
            _ if cropped => [0.0; 4],
            _ if unpremultiply => premul(fg, a),
            // the result premultiplied once more, read as straight colour
            _ => premul(fg.map(|v| v * a), a),
        };
    }
    b
}

/// Every effect here.
pub const ALL: [Spec; 1] = [SCREEN_KEYER];

#[cfg(test)]
mod tests;
