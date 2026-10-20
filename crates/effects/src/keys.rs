//! Keying: see-through where the picture is a colour (Linear Color Key, Color Difference Key,
//! Color Range), bright or dark (Extract), unlike another layer (Difference Matte), outside masks
//! (Inner/Outer Key) or black (Unmult); the screen's colour taken off what is left (Advanced
//! Spill Suppressor), the matte's edge cleaned (Key Cleaner), wires painted out (Wire Removal);
//! and the old keyers (Color Key, Luma Key, Spill Suppressor).

use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px, Sampling};

use crate::color::rgb3;
use crate::defs::{choice, color, effect, flag, layer, num, pct, point, ranged};
use crate::draw::{distance, inside};
use crate::matte::{rounded_extreme, soften};
use crate::util::{hsl, luma, map_px, premul, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "keying";

fn mix4(a: Px, b: Px, t: f32) -> Px {
    [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t)
}

const VIEWS: &[&str] = &["finalOutput", "sourceOnly", "matteOnly"];

/// A matte (1: kept) put on the picture as the view says.
fn keep_by(img: &mut Image, m: &[f32], view: u32) {
    if view == 1 {
        return;
    }
    for (p, k) in img.px.iter_mut().zip(m) {
        let k = k.clamp(0.0, 1.0);
        *p = if view == 2 {
            let a = p[3] * k;
            [a, a, a, 1.0]
        } else {
            p.map(|v| v * k)
        };
    }
}

/// A matte from each pixel's straight colour and alpha.
fn matte_by(img: &Image, f: impl Fn([f32; 3], f32) -> f32) -> Vec<f32> {
    img.px
        .iter()
        .map(|p| {
            let (c, a) = unpremul(*p);
            f(c, a)
        })
        .collect()
}

/// A matte thinned (above 0) or grown (below 0) and feathered, in buffer pixels.
fn thin_feather(m: Vec<f32>, w: usize, h: usize, thin: f64, feather: f64) -> Vec<f32> {
    let m = if thin.abs() > 1e-6 { rounded_extreme(&m, w, h, thin.abs(), thin < 0.0) } else { m };
    if feather > 0.0 { soften(&m, w, h, feather * 0.5) } else { m }
}

/// Which channel of a colour is strongest, and the other two.
fn primary(c: [f32; 3]) -> (usize, usize, usize) {
    if c[1] >= c[0] && c[1] >= c[2] {
        (1, 0, 2)
    } else if c[2] >= c[0] && c[2] >= c[1] {
        (2, 0, 1)
    } else {
        (0, 1, 2)
    }
}

effect!(
    LINEAR_DEF,
    "linearColorKey",
    "Linear Color Key",
    "see-through where the picture is near a colour (by its red-green-blue, hue or chroma), or kept only there",
    [
        choice("view", "View", VIEWS, 0),
        color("keyColor", "Key Color", 0.0, 0.0, 1.0),
        choice("matchColors", "Match colors", &["usingRgb", "usingHue", "usingChroma"], 0),
        pct("matchingTolerance", "Matching Tolerance", 10.0, 0.0, 100.0),
        pct("matchingSoftness", "Matching Softness", 10.0, 0.0, 100.0),
        choice("keyOperation", "Key Operation", &["keyColors", "keepColors"], 0),
    ]
);
pub const LINEAR: Spec = Spec::new(&LINEAR_DEF, CATEGORY, linear_key);

fn chroma(c: [f32; 3]) -> [f32; 2] {
    let s = c[0] + c[1] + c[2];
    if s <= 1e-6 { [1.0 / 3.0, 1.0 / 3.0] } else { [c[0] / s, c[1] / s] }
}

fn linear_key(ctx: &Ctx, mut b: Buf) -> Buf {
    let key = rgb3(ctx.color("keyColor"));
    let how = ctx.choice("matchColors");
    let tol = (ctx.num("matchingTolerance", 10.0) / 100.0) as f32;
    let soft = (ctx.num("matchingSoftness", 10.0) / 100.0) as f32;
    let keep = ctx.choice("keyOperation") == 1;
    let (kh, kc) = (hsl(key).0, chroma(key));
    let m = matte_by(&b.img, |c, _| {
        let d = match how {
            1 => {
                let (h, s, _) = hsl(c);
                if s < 0.02 {
                    1.0
                } else {
                    let d = (h - kh).abs();
                    d.min(1.0 - d) * 2.0
                }
            }
            2 => {
                let pc = chroma(c);
                (pc[0] - kc[0]).hypot(pc[1] - kc[1]) / std::f32::consts::SQRT_2
            }
            _ => ((c[0] - key[0]).powi(2) + (c[1] - key[1]).powi(2) + (c[2] - key[2]).powi(2)).sqrt() / 3f32.sqrt(),
        };
        let v = if soft > 1e-6 {
            ((d - tol) / soft).clamp(0.0, 1.0)
        } else if d > tol {
            1.0
        } else {
            0.0
        };
        if keep { 1.0 - v } else { v }
    });
    keep_by(&mut b.img, &m, ctx.choice("view"));
    b
}

const DIFF_VIEWS: &[&str] = &[
    "source",
    "uncorrectedMattePartialA",
    "correctedMattePartialA",
    "uncorrectedMattePartialB",
    "correctedMattePartialB",
    "uncorrectedMatte",
    "correctedMatte",
    "finalOutput",
    "abMatteCorrectedFinal",
];

effect!(
    DIFFERENCE_KEY_DEF,
    "colorDifferenceKey",
    "Color Difference Key",
    "a matte from how much the key colour's strongest channel stands above each of the other two, the two partial mattes joined",
    [
        choice("view", "View", DIFF_VIEWS, 7),
        color("keyColor", "Key Color", 0.0, 0.0, 1.0),
        choice("colorMatchingAccuracy", "Color Matching Accuracy", &["faster", "moreAccurate"], 0).doc("more accurate: worked out in linear light"),
        ranged("partialAInBlack", "Partial A In Black", 0.0, 0.0, 255.0),
        ranged("partialAInWhite", "Partial A In White", 255.0, 0.0, 255.0),
        ranged("partialAGamma", "Partial A Gamma", 1.0, 0.01, 10.0),
        ranged("partialAOutBlack", "Partial A Out Black", 0.0, 0.0, 255.0),
        ranged("partialAOutWhite", "Partial A Out White", 255.0, 0.0, 255.0),
        ranged("partialBInBlack", "Partial B In Black", 0.0, 0.0, 255.0),
        ranged("partialBInWhite", "Partial B In White", 255.0, 0.0, 255.0),
        ranged("partialBGamma", "Partial B Gamma", 1.0, 0.01, 10.0),
        ranged("partialBOutBlack", "Partial B Out Black", 0.0, 0.0, 255.0),
        ranged("partialBOutWhite", "Partial B Out White", 255.0, 0.0, 255.0),
        ranged("matteInBlack", "Matte In Black", 0.0, 0.0, 255.0),
        ranged("matteInWhite", "Matte In White", 255.0, 0.0, 255.0),
        ranged("matteGamma", "Matte Gamma", 1.0, 0.01, 10.0),
    ]
);
pub const DIFFERENCE_KEY: Spec = Spec::new(&DIFFERENCE_KEY_DEF, CATEGORY, difference_key);

/// Levels on a matte value: in black and white (0 to 255), gamma, out black and white.
fn levels(v: f32, in_b: f32, in_w: f32, gamma: f32, out_b: f32, out_w: f32) -> f32 {
    let t = ((v - in_b / 255.0) / ((in_w - in_b) / 255.0).max(1e-4)).clamp(0.0, 1.0).powf(1.0 / gamma.max(0.01));
    (out_b / 255.0 + t * (out_w - out_b) / 255.0).clamp(0.0, 1.0)
}

fn difference_key(ctx: &Ctx, mut b: Buf) -> Buf {
    let view = ctx.choice("view");
    if view == 0 {
        return b;
    }
    let linear = ctx.choice("colorMatchingAccuracy") == 1;
    let lin = |c: [f32; 3]| if linear { c.map(|v| veymelo_color::srgb_to_linear(v.clamp(0.0, 1.0) as f64) as f32) } else { c };
    let key = lin(rgb3(ctx.color("keyColor")));
    let (pi, o1, o2) = primary(key);
    let (kd1, kd2) = ((key[pi] - key[o1]).max(1e-3), (key[pi] - key[o2]).max(1e-3));
    let lv = |pre: &str| {
        let g = |s: &str, d: f64| ctx.num(&format!("{pre}{s}"), d) as f32;
        (g("InBlack", 0.0), g("InWhite", 255.0), g("Gamma", 1.0), g("OutBlack", 0.0), g("OutWhite", 255.0))
    };
    let (pa, pb) = (lv("partialA"), lv("partialB"));
    let (mb, mw, mg) = (ctx.num("matteInBlack", 0.0) as f32, ctx.num("matteInWhite", 255.0) as f32, ctx.num("matteGamma", 1.0) as f32);
    let (w, h) = (b.img.width, b.img.height);
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let i = y * w + x;
        let (c, a0) = unpremul(src.px[i]);
        let c = lin(c);
        let ua = 1.0 - ((c[pi] - c[o1]) / kd1).clamp(0.0, 1.0);
        let ub = 1.0 - ((c[pi] - c[o2]) / kd2).clamp(0.0, 1.0);
        let ca = levels(ua, pa.0, pa.1, pa.2, pa.3, pa.4);
        let cb = levels(ub, pb.0, pb.1, pb.2, pb.3, pb.4);
        let cm = levels(ca.max(cb), mb, mw, mg, 0.0, 255.0);
        // four at once: partial A, partial B and the matte (corrected), and the result
        let v = if view == 8 {
            match (x * 2 >= w, y * 2 >= h) {
                (false, false) => 2,
                (true, false) => 4,
                (false, true) => 6,
                (true, true) => 7,
            }
        } else {
            view
        };
        let grey = |m: f32| [m * a0, m * a0, m * a0, 1.0];
        match v {
            1 => grey(ua),
            2 => grey(ca),
            3 => grey(ub),
            4 => grey(cb),
            5 => grey(ua.max(ub)),
            6 => grey(cm),
            _ => src.px[i].map(|v| v * cm),
        }
    });
    b
}

effect!(
    COLOR_RANGE_DEF,
    "colorRange",
    "Color Range",
    "see-through where the picture's colour falls inside a range (in Lab, YUV or RGB)",
    [
        ranged("fuzziness", "Fuzziness", 0.0, 0.0, 255.0),
        choice("colorSpace", "Color Space", &["lab", "yuv", "rgb"], 0),
        ranged("minL", "Min (L, Y, R)", 0.0, 0.0, 255.0),
        ranged("maxL", "Max (L, Y, R)", 255.0, 0.0, 255.0),
        ranged("minA", "Min (a, U, G)", 0.0, 0.0, 255.0),
        ranged("maxA", "Max (a, U, G)", 100.0, 0.0, 255.0),
        ranged("minB", "Min (b, V, B)", 128.0, 0.0, 255.0),
        ranged("maxB", "Max (b, V, B)", 255.0, 0.0, 255.0),
    ]
);
pub const COLOR_RANGE: Spec = Spec::new(&COLOR_RANGE_DEF, CATEGORY, color_range);

fn to_lab(c: [f32; 3]) -> [f32; 3] {
    let l = c.map(|v| veymelo_color::srgb_to_linear(v.clamp(0.0, 1.0) as f64) as f32);
    let x = (0.4124 * l[0] + 0.3576 * l[1] + 0.1805 * l[2]) / 0.95047;
    let y = 0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2];
    let z = (0.0193 * l[0] + 0.1192 * l[1] + 0.9505 * l[2]) / 1.08883;
    let f = |t: f32| if t > 0.008_856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// A colour as three numbers from 0 to 255 in the space.
fn in_space(space: u32, c: [f32; 3]) -> [f32; 3] {
    match space {
        1 => {
            let y = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
            let u = -0.147_13 * c[0] - 0.288_86 * c[1] + 0.436 * c[2];
            let v = 0.615 * c[0] - 0.514_99 * c[1] - 0.100_01 * c[2];
            [y * 255.0, u * 255.0 + 128.0, v * 255.0 + 128.0]
        }
        2 => c.map(|v| v * 255.0),
        _ => {
            let lab = to_lab(c);
            [lab[0] * 2.55, lab[1] + 128.0, lab[2] + 128.0]
        }
    }
}

fn color_range(ctx: &Ctx, mut b: Buf) -> Buf {
    let space = ctx.choice("colorSpace");
    let lo = [ctx.num("minL", 0.0), ctx.num("minA", 0.0), ctx.num("minB", 128.0)].map(|v| v as f32);
    let hi = [ctx.num("maxL", 255.0), ctx.num("maxA", 100.0), ctx.num("maxB", 255.0)].map(|v| v as f32);
    let fuzz = ctx.num("fuzziness", 0.0) as f32;
    let m = matte_by(&b.img, |c, _| {
        let v = in_space(space, c);
        let out = (0..3).fold(0f32, |o, i| o.max(lo[i] - v[i]).max(v[i] - hi[i]));
        if out <= 0.0 {
            0.0
        } else if fuzz > 0.0 {
            (out / fuzz).clamp(0.0, 1.0)
        } else {
            1.0
        }
    });
    keep_by(&mut b.img, &m, 0);
    b
}

effect!(
    EXTRACT_DEF,
    "extract",
    "Extract",
    "see-through where a channel is darker than the black point or lighter than the white point",
    [
        choice("channel", "Channel", &["luminance", "red", "green", "blue", "alpha"], 0),
        ranged("blackPoint", "Black Point", 0.0, 0.0, 255.0),
        ranged("whitePoint", "White Point", 255.0, 0.0, 255.0),
        ranged("blackSoftness", "Black Softness", 0.0, 0.0, 255.0),
        ranged("whiteSoftness", "White Softness", 0.0, 0.0, 255.0),
        flag("invert", "Invert", false),
    ]
);
pub const EXTRACT: Spec = Spec::new(&EXTRACT_DEF, CATEGORY, extract);

fn extract(ctx: &Ctx, mut b: Buf) -> Buf {
    let ch = ctx.choice("channel");
    let (bp, wp) = (ctx.num("blackPoint", 0.0) as f32, ctx.num("whitePoint", 255.0) as f32);
    let (bs, ws) = (ctx.num("blackSoftness", 0.0) as f32, ctx.num("whiteSoftness", 0.0) as f32);
    let flip = ctx.flag("invert");
    let m = matte_by(&b.img, |c, a| {
        let v = 255.0
            * match ch {
                1 => c[0],
                2 => c[1],
                3 => c[2],
                4 => a,
                _ => luma(c),
            };
        let k = if v < bp {
            if bs > 0.0 { (1.0 - (bp - v) / bs).clamp(0.0, 1.0) } else { 0.0 }
        } else if v > wp {
            if ws > 0.0 { (1.0 - (v - wp) / ws).clamp(0.0, 1.0) } else { 0.0 }
        } else {
            1.0
        };
        if flip { 1.0 - k } else { k }
    });
    keep_by(&mut b.img, &m, 0);
    b
}

effect!(
    DIFFERENCE_MATTE_DEF,
    "differenceMatte",
    "Difference Matte",
    "see-through where the layer matches another (an empty plate of the same shot)",
    [
        choice("view", "View", VIEWS, 0),
        layer("differenceLayer", "Difference Layer").doc("none: this layer"),
        choice("ifLayerSizesDiffer", "If Layer Sizes Differ", &["center", "stretchToFit"], 0),
        pct("matchingTolerance", "Matching Tolerance", 15.0, 0.0, 100.0),
        pct("matchingSoftness", "Matching Softness", 0.0, 0.0, 100.0),
        ranged("blurBeforeDifference", "Blur Before Difference", 0.0, 0.0, 1000.0),
    ]
);
pub const DIFFERENCE_MATTE: Spec = Spec::new(&DIFFERENCE_MATTE_DEF, CATEGORY, difference_matte);

fn difference_matte(ctx: &Ctx, mut b: Buf) -> Buf {
    let view = ctx.choice("view");
    if view == 1 {
        return b;
    }
    let tol = (ctx.num("matchingTolerance", 15.0) / 100.0) as f32;
    let soft = (ctx.num("matchingSoftness", 0.0) / 100.0) as f32;
    let blur = ctx.num("blurBeforeDifference", 0.0) * b.scale;
    let other = ctx.layer_over("differenceLayer", &b, Take::All, ctx.choice("ifLayerSizesDiffer") == 1).unwrap_or_else(|| b.img.clone());
    let (src, other) = if blur > 0.05 { (filter::gaussian(&b.img, blur * 0.5, blur * 0.5, Edge::Repeat), filter::gaussian(&other, blur * 0.5, blur * 0.5, Edge::Repeat)) } else { (b.img.clone(), other) };
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let i = y * w + x;
        let (c, a) = unpremul(src.px[i]);
        let (oc, oa) = unpremul(other.px[i]);
        let d = (c[0] - oc[0]).abs().max((c[1] - oc[1]).abs()).max((c[2] - oc[2]).abs()).max((a - oa).abs());
        let m = if d <= tol {
            0.0
        } else if soft <= 1e-6 {
            1.0
        } else {
            ((d - tol) / soft).clamp(0.0, 1.0)
        };
        if view == 2 { [m * p[3], m * p[3], m * p[3], p[3]] } else { p.map(|v| v * m) }
    });
    b
}

/// Inner/Outer Key's settings: the masks inside and outside the subject (ten more of each), ten
/// brush strokes along masks each side to paint the matte solid or clear, and the edge.
macro_rules! inner_outer {
    ($($n:literal),*) => {
        pub static INNER_OUTER_DEF: veymelo_project::GroupDef = veymelo_project::GroupDef {
            id: "innerOuterKey",
            name: "Inner/Outer Key",
            doc: "the subject cut out between a mask round its inside and one round its outside, the edge between found from the colours either side and freed of the background's",
            body: veymelo_project::Body::Fixed(&[
                veymelo_project::DefNode::Prop(num("foregroundInside", "Foreground (Inside)", 0.0).range(0.0, 1000.0).doc("a mask (from 1) round the inside of the subject; 0: none")),
                $(veymelo_project::DefNode::Prop(num(concat!("foreground", $n), concat!("Foreground ", $n), 0.0).range(0.0, 1000.0)),)*
                veymelo_project::DefNode::Prop(num("backgroundOutside", "Background (Outside)", 0.0).range(0.0, 1000.0).doc("a mask round the outside; 0: none (then the inside mask's edge, the highlight radius each way)")),
                $(veymelo_project::DefNode::Prop(num(concat!("background", $n), concat!("Background ", $n), 0.0).range(0.0, 1000.0)),)*
                $(
                    veymelo_project::DefNode::Prop(num(concat!("cleanupForeground", $n), concat!("Cleanup Foreground ", $n), 0.0).range(0.0, 1000.0).doc("a mask path brushed along to make the matte solid")),
                    veymelo_project::DefNode::Prop(ranged(concat!("cleanupForeground", $n, "BrushRadius"), "Brush Radius", 10.0, 0.0, 1000.0)),
                    veymelo_project::DefNode::Prop(pct(concat!("cleanupForeground", $n, "BrushPressure"), "Brush Pressure", 80.0, 0.0, 100.0)),
                )*
                $(
                    veymelo_project::DefNode::Prop(num(concat!("cleanupBackground", $n), concat!("Cleanup Background ", $n), 0.0).range(0.0, 1000.0).doc("a mask path brushed along to make the matte clear")),
                    veymelo_project::DefNode::Prop(ranged(concat!("cleanupBackground", $n, "BrushRadius"), "Brush Radius", 10.0, 0.0, 1000.0)),
                    veymelo_project::DefNode::Prop(pct(concat!("cleanupBackground", $n, "BrushPressure"), "Brush Pressure", 80.0, 0.0, 100.0)),
                )*
                veymelo_project::DefNode::Prop(ranged("singleMaskHighlightRadius", "Single Mask Highlight Radius", 0.0, 0.0, 100.0)),
                veymelo_project::DefNode::Prop(ranged("edgeThin", "Edge Thin", 0.0, -10.0, 10.0)),
                veymelo_project::DefNode::Prop(ranged("edgeFeather", "Edge Feather", 0.0, 0.0, 100.0)),
                veymelo_project::DefNode::Prop(pct("edgeThreshold", "Edge Threshold", 0.0, 0.0, 100.0)),
                veymelo_project::DefNode::Prop(flag("invertExtraction", "Invert Extraction", false)),
                veymelo_project::DefNode::Prop(pct("blendWithOriginal", "Blend with Original", 0.0, 0.0, 100.0)),
            ]),
        };
    };
}
inner_outer!("1", "2", "3", "4", "5", "6", "7", "8", "9", "10");
pub const INNER_OUTER: Spec = Spec::new(&INNER_OUTER_DEF, CATEGORY, inner_outer_key);

/// Where the masks numbered by these settings cover (1) or not (0), and how far each pixel is
/// from their outlines (buffer pixels, above 0 inside).
fn masks_cover(ctx: &Ctx, b: &Buf, ids: &[String]) -> Option<(Vec<f32>, Vec<f32>)> {
    let shapes: Vec<Vec<[f64; 2]>> = ids
        .iter()
        .filter_map(|id| ctx.mask(ctx.num(id, 0.0).round().max(0.0) as usize))
        .filter(|m| m.points.len() >= 3)
        .map(|m| {
            m.points
                .iter()
                .map(|q| {
                    let p = b.pixel(veymelo_geom::vec2(q[0], q[1]));
                    [p.x, p.y]
                })
                .collect()
        })
        .collect();
    if shapes.is_empty() {
        return None;
    }
    let (w, h) = (b.img.width, b.img.height);
    let (mut cover, mut sd) = (vec![0f32; w * h], vec![0f32; w * h]);
    for i in 0..w * h {
        let q = [(i % w) as f64 + 0.5, (i / w) as f64 + 0.5];
        let on = shapes.iter().any(|s| inside(s, q));
        let d = shapes.iter().map(|s| distance(s, true, q)).fold(f64::INFINITY, f64::min) as f32;
        cover[i] = if on { 1.0 } else { 0.0 };
        sd[i] = if on { d } else { -d };
    }
    Some((cover, sd))
}

fn inner_outer_key(ctx: &Ctx, mut b: Buf) -> Buf {
    let numbered = |first: &str, more: &str| -> Vec<String> { std::iter::once(first.to_string()).chain((1..=10).map(|k| format!("{more}{k}"))).collect() };
    let (fg_ids, bg_ids) = (numbered("foregroundInside", "foreground"), numbered("backgroundOutside", "background"));
    let Some((fg, fg_sd)) = masks_cover(ctx, &b, &fg_ids) else { return b };
    let bg = masks_cover(ctx, &b, &bg_ids);
    let radius = (ctx.num("singleMaskHighlightRadius", 0.0) * b.scale) as f32;
    let (w, h) = (b.img.width, b.img.height);
    // known inside (1), known outside (0), between (½)
    let tri: Vec<f32> = match &bg {
        Some((bk, _)) => (0..w * h).map(|i| if fg[i] > 0.5 { 1.0 } else if bk[i] > 0.5 { 0.5 } else { 0.0 }).collect(),
        None if radius > 0.0 => fg_sd.iter().map(|d| if *d > radius { 1.0 } else if *d < -radius { 0.0 } else { 0.5 }).collect(),
        None => fg.clone(),
    };
    let between = |t: f32| t > 0.25 && t < 0.75;
    let mut alpha = tri.clone();
    let straight: Vec<[f32; 3]> = b.img.px.iter().map(|p| unpremul(*p).0).collect();
    let mut colours: Option<Vec<[f32; 3]>> = None;
    if tri.iter().any(|t| between(*t)) {
        // the colours of the inside and the outside near each pixel: each softened over the part
        // that is known
        let sigma = (radius.max(4.0) as f64).max(w.min(h) as f64 * 0.02);
        let near = |known: &dyn Fn(f32) -> bool| -> Vec<[f32; 3]> {
            let mut img = Image::new(w, h);
            for (i, p) in img.px.iter_mut().enumerate() {
                let k = if known(tri[i]) { 1.0 } else { 0.0 };
                *p = [straight[i][0] * k, straight[i][1] * k, straight[i][2] * k, k];
            }
            filter::gaussian(&img, sigma, sigma, Edge::Clear).px.iter().map(|p| [0, 1, 2].map(|k| p[k] / p[3].max(1e-6))).collect()
        };
        let (fc, bc) = (near(&|t| t > 0.75), near(&|t| t < 0.25));
        for (i, a) in alpha.iter_mut().enumerate() {
            if between(*a) {
                // how far the colour is from the outside's toward the inside's
                let c = straight[i];
                let d = [fc[i][0] - bc[i][0], fc[i][1] - bc[i][1], fc[i][2] - bc[i][2]];
                let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                *a = if dd < 1e-6 { 0.5 } else { (((c[0] - bc[i][0]) * d[0] + (c[1] - bc[i][1]) * d[1] + (c[2] - bc[i][2]) * d[2]) / dd).clamp(0.0, 1.0) };
            }
        }
        // the outside's colour taken back out of the edge
        colours = Some(
            (0..w * h)
                .map(|i| {
                    let (a, c) = (alpha[i], straight[i]);
                    if between(tri[i]) && a > 0.02 { [0, 1, 2].map(|k| ((c[k] - (1.0 - a) * bc[i][k]) / a).clamp(0.0, 1.0)) } else { c }
                })
                .collect(),
        );
    }
    // the clean-up brushes along their mask paths
    for (side, target) in [("cleanupForeground", 1.0f32), ("cleanupBackground", 0.0)] {
        for k in 1..=10 {
            let Some(m) = ctx.mask(ctx.num(&format!("{side}{k}"), 0.0).round().max(0.0) as usize) else { continue };
            let r = (ctx.num(&format!("{side}{k}BrushRadius"), 10.0) * b.scale).max(0.5);
            let pressure = (ctx.num(&format!("{side}{k}BrushPressure"), 80.0) / 100.0).clamp(0.0, 1.0) as f32;
            let pts: Vec<[f64; 2]> = m
                .points
                .iter()
                .map(|q| {
                    let p = b.pixel(veymelo_geom::vec2(q[0], q[1]));
                    [p.x, p.y]
                })
                .collect();
            for (i, a) in alpha.iter_mut().enumerate() {
                let d = distance(&pts, m.closed, [(i % w) as f64 + 0.5, (i / w) as f64 + 0.5]);
                if d < r + 1.0 {
                    let c = ((r - d) / r.max(1.0) * 2.0).clamp(0.0, 1.0) as f32 * pressure;
                    *a += (target - *a) * c;
                }
            }
        }
    }
    let alpha = thin_feather(alpha, w, h, ctx.num("edgeThin", 0.0) * b.scale, ctx.num("edgeFeather", 0.0) * b.scale);
    let threshold = (ctx.num("edgeThreshold", 0.0) / 100.0) as f32;
    let flip = ctx.flag("invertExtraction");
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0) as f32;
    let src = b.img.clone();
    map_px(&mut b.img, |x, y, _| {
        let i = y * w + x;
        let mut m = alpha[i];
        if threshold > 0.0 && m < threshold {
            m = 0.0;
        }
        if flip {
            m = 1.0 - m;
        }
        let (_, a) = unpremul(src.px[i]);
        let c = colours.as_ref().map_or(straight[i], |v| v[i]);
        mix4(premul(c, a * m), src.px[i], keep)
    });
    b
}

effect!(
    UNMULT_DEF,
    "unmult",
    "Unmult",
    "a black (or white) background made see-through, as light on black would be",
    [
        choice("backgroundColor", "Background Color", &["black", "white"], 0),
        pct("blackLevel", "Black Level", 0.0, 0.0, 100.0),
        pct("whiteLevel", "White Level", 100.0, 0.0, 100.0),
        pct("softness", "Softness", 100.0, 0.0, 100.0),
        flag("removeColorMatting", "Remove Color Matting", true),
        flag("clipHdrResults", "Clip HDR Results", true),
    ]
);
pub const UNMULT: Spec = Spec::new(&UNMULT_DEF, CATEGORY, unmult);

fn unmult(ctx: &Ctx, mut b: Buf) -> Buf {
    let white = ctx.choice("backgroundColor") == 1;
    let level = (if white { ctx.num("whiteLevel", 100.0) } else { ctx.num("blackLevel", 0.0) }.clamp(0.0, 100.0) / 100.0) as f32;
    let soft = (ctx.num("softness", 100.0).clamp(0.0, 100.0) / 100.0) as f32;
    let unmatte = ctx.flag("removeColorMatting");
    let clip = ctx.flag("clipHdrResults");
    map_px(&mut b.img, |_, _, p| {
        let (c, a0) = unpremul(p);
        // how far from the background, and where on that the ramp starts
        let (d, start) = if white { (1.0 - c[0].min(c[1]).min(c[2]), 1.0 - level) } else { (c[0].max(c[1]).max(c[2]), level) };
        let d = d.clamp(0.0, 1.0);
        let width = soft * (1.0 - start);
        let m = if width > 1e-6 {
            ((d - start) / width).clamp(0.0, 1.0)
        } else if d > start {
            1.0
        } else {
            0.0
        };
        if m <= 1e-6 {
            return [0.0; 4];
        }
        let bg = if white { 1.0 } else { 0.0 };
        let mut out = if unmatte { c.map(|v| (v - bg * (1.0 - m)) / m) } else { c };
        if clip {
            out = out.map(|v| v.clamp(0.0, 1.0));
        }
        premul(out, a0 * m)
    });
    b
}

/// The light's share of each channel.
const LUMA_W: [f32; 3] = [0.2126, 0.7152, 0.0722];

effect!(
    ADVANCED_SPILL_DEF,
    "advancedSpillSuppressor",
    "Advanced Spill Suppressor",
    "the screen's colour that spilled onto the subject taken off it",
    [
        choice("method", "Method", &["standard", "ultra"], 0).doc("standard finds the screen's colour by itself"),
        ranged("suppression", "Suppression", 100.0, 0.0, 200.0),
        color("keyColor", "Key Color", 0.0, 1.0, 0.0),
        pct("tolerance", "Tolerance", 50.0, 0.0, 100.0),
        pct("desaturate", "Desaturate", 0.0, 0.0, 100.0),
        pct("spillRange", "Spill Range", 50.0, 0.0, 100.0),
        pct("spillColorCorrection", "Spill Color Correction", 0.0, 0.0, 100.0),
        pct("lumaCorrection", "Luma Correction", 0.0, 0.0, 100.0),
    ]
);
pub const ADVANCED_SPILL: Spec = Spec::new(&ADVANCED_SPILL_DEF, CATEGORY, advanced_spill);

fn advanced_spill(ctx: &Ctx, mut b: Buf) -> Buf {
    let ultra = ctx.choice("method") == 1;
    let key = rgb3(ctx.color("keyColor"));
    let pi = if ultra {
        primary(key).0
    } else {
        // the channel standing most above the others over the whole picture
        let ex = b.img.px.iter().fold([0f32; 3], |acc, p| {
            let (c, a) = unpremul(*p);
            [acc[0] + (c[0] - c[1].max(c[2])).max(0.0) * a, acc[1] + (c[1] - c[0].max(c[2])).max(0.0) * a, acc[2] + (c[2] - c[0].max(c[1])).max(0.0) * a]
        });
        if ex[1] >= ex[0] && ex[1] >= ex[2] {
            1
        } else if ex[2] >= ex[0] {
            2
        } else {
            0
        }
    };
    let (o1, o2) = match pi {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let amount = (ctx.num("suppression", 100.0) / 100.0) as f32;
    let range = (ctx.num("spillRange", 50.0) / 100.0) as f32;
    let luma_back = (ctx.num("lumaCorrection", 0.0) / 100.0) as f32;
    let key_hue = hsl(key).0;
    let tol = if ultra { (ctx.num("tolerance", 50.0) / 100.0) as f32 } else { 1.0 };
    let desat = if ultra { (ctx.num("desaturate", 0.0) / 100.0) as f32 } else { 0.0 };
    let neutral = if ultra { (ctx.num("spillColorCorrection", 0.0) / 100.0) as f32 } else { 0.0 };
    crate::util::map_straight(&mut b.img, |mut c| {
        // only near the key's hue (ultra)
        let near = if tol < 1.0 {
            let d = (hsl(c).0 - key_hue).abs();
            (1.0 - d.min(1.0 - d) / (tol * 0.5).max(1e-4)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let (most, mean) = (c[o1].max(c[o2]), 0.5 * (c[o1] + c[o2]));
        let limit = most + (mean - most) * range;
        let spill = (c[pi] - limit).max(0.0) * amount * near;
        c[pi] -= spill;
        // the light taken away given back, as brightness or as plain light
        let back = spill * LUMA_W[pi] * luma_back + spill * neutral / 3.0;
        let mut c = c.map(|v| v + back);
        if desat > 0.0 && spill > 0.0 {
            let l = luma(c);
            let k = (desat * (spill * 4.0).min(1.0)).min(1.0);
            c = c.map(|v| v + (l - v) * k);
        }
        c
    });
    b
}

effect!(
    KEY_CLEANER_DEF,
    "keyCleaner",
    "Key Cleaner",
    "a keyed matte's edge cleaned: its colours taken from the solid parts nearby, its alpha firmed",
    [
        ranged("additionalEdgeRadius", "Additional Edge Radius", 3.0, 0.0, 100.0),
        flag("reduceChatter", "Reduce Chatter", false),
        ranged("alphaContrast", "Alpha Contrast", 100.0, 100.0, 500.0),
        pct("strength", "Strength", 100.0, 0.0, 100.0),
    ]
);
pub const KEY_CLEANER: Spec = Spec::new(&KEY_CLEANER_DEF, CATEGORY, key_cleaner);

fn key_cleaner(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = (ctx.num("additionalEdgeRadius", 3.0) * b.scale).max(0.0);
    let contrast = (ctx.num("alphaContrast", 100.0) / 100.0) as f32;
    let strength = (ctx.num("strength", 100.0) / 100.0) as f32;
    let (w, h) = (b.img.width, b.img.height);
    let mut a: Vec<f32> = b.img.px.iter().map(|p| p[3]).collect();
    if ctx.flag("reduceChatter") {
        a = soften(&a, w, h, 1.0).iter().map(|v| ((v - 0.5) * 1.5 + 0.5).clamp(0.0, 1.0)).collect();
    }
    let near = (r > 0.0 && strength > 0.0).then(|| filter::gaussian(&b.img, r * 0.5, r * 0.5, Edge::Clear));
    map_px(&mut b.img, |x, y, p| {
        let i = y * w + x;
        let (mut c, a0) = unpremul(p);
        if let Some(n) = &near
            && a0 > 1e-4
            && a0 < 0.999
        {
            let (nc, na) = unpremul(n.px[i]);
            if na > 1e-4 {
                let t = strength * (1.0 - a0);
                c = [0, 1, 2].map(|k| c[k] + (nc[k] - c[k]) * t);
            }
        }
        let na = if a0 <= 1e-6 { 0.0 } else { ((a[i] - 0.5) * contrast + 0.5).clamp(0.0, 1.0) };
        premul(c, na)
    });
    b
}

effect!(
    WIRE_DEF,
    "wireRemoval",
    "Wire Removal",
    "a thin wire between two points painted out from what is either side of it",
    [
        point("pointA", "Point A"),
        point("pointB", "Point B"),
        choice("removalStyle", "Removal Style", &["fade", "frameOffset", "displace", "displaceHorizontal"], 2),
        ranged("thickness", "Thickness", 5.0, 0.0, 100.0),
        pct("slope", "Slope", 0.0, 0.0, 100.0).doc("a soft edge past the wire"),
        pct("mirrorBlend", "Mirror Blend", 0.0, 0.0, 100.0),
        ranged("frameOffset", "Frame Offset", 0.0, -100.0, 100.0).doc("for frame offset: the same place this many frames away"),
    ]
);
pub const WIRE: Spec = Spec::new(&WIRE_DEF, CATEGORY, wire_removal).points(&[("pointA", [0.25, 0.25]), ("pointB", [0.75, 0.75])]);

fn wire_removal(ctx: &Ctx, mut b: Buf) -> Buf {
    let half = (ctx.num("thickness", 5.0) * b.scale * 0.5).max(0.0);
    if half <= 0.0 {
        return b;
    }
    let style = ctx.choice("removalStyle");
    let slope = (ctx.num("slope", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    let mirror = (ctx.num("mirrorBlend", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    let (a, z) = (b.pixel(ctx.point("pointA")), b.pixel(ctx.point("pointB")));
    let len = (z - a).length();
    if len < 1e-6 {
        return b;
    }
    let (tx, ty) = ((z.x - a.x) / len, (z.y - a.y) / len);
    let (nx, ny) = (-ty, tx);
    let soft_w = half * slope as f64;
    let src = b.img.clone();
    let (w, h) = (src.width as f64, src.height as f64);
    let held = |x: f64, y: f64| src.sample(Sampling::Bilinear, x.clamp(0.5, w - 0.5), y.clamp(0.5, h - 0.5));
    // the same pixels some frames away, on the same grid
    let offset = ctx.num("frameOffset", 0.0).round();
    let other = (style == 1 && offset != 0.0)
        .then(|| ctx.host.and_then(|host| host.this_at(ctx.time + veymelo_time::Tick::from_seconds(offset * ctx.frame.seconds()), ctx.index)))
        .flatten()
        .filter(|o| o.img.width == src.width && o.img.height == src.height && (o.origin - b.origin).length() < 1e-6);
    map_px(&mut b.img, |x, y, p| {
        let (qx, qy) = (x as f64 + 0.5 - a.x, y as f64 + 0.5 - a.y);
        let d = qx * nx + qy * ny;
        let ad = d.abs();
        // the wire is the piece from A to B
        let along = qx * tx + qy * ty;
        if !(0.0..=len).contains(&along) || ad > half + soft_w {
            return p;
        }
        let base = (x as f64 + 0.5 - d * nx, y as f64 + 0.5 - d * ny);
        let edge = half + 1.0;
        let side = |s: f64| held(base.0 + nx * s, base.1 + ny * s);
        let fill = match style {
            0 => mix4(side(-edge), side(edge), (((d + edge) / (2.0 * edge)) as f32).clamp(0.0, 1.0)),
            1 if other.is_some() => other.as_ref().map_or(p, |o| o.img.px[y * src.width + x]),
            // shifted across the wire from the nearer side
            1 | 2 => {
                let s = if d >= 0.0 { d + 2.0 * (half - ad) + 1.0 } else { d - 2.0 * (half - ad) - 1.0 };
                let sh = side(s);
                if mirror > 0.0 { mix4(sh, side(-s), mirror * 0.5) } else { sh }
            }
            _ => {
                let sx = if nx.abs() > 1e-6 { (half - ad + 1.0) / nx.abs() } else { 0.0 };
                let dir = if d * nx >= 0.0 { 1.0 } else { -1.0 };
                let (sh, mi) = (held(x as f64 + 0.5 + dir * sx, y as f64 + 0.5), held(x as f64 + 0.5 - dir * sx, y as f64 + 0.5));
                if mirror > 0.0 { mix4(sh, mi, mirror * 0.5) } else { sh }
            }
        };
        let k = if ad <= half {
            1.0
        } else {
            let t = (((ad - half) / soft_w.max(1e-9)) as f32).clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t)
        };
        mix4(p, fill, k)
    });
    b
}

const OBSOLETE: &str = "obsolete";

effect!(
    COLOR_KEY_DEF,
    "colorKey",
    "Color Key",
    "the old keyer: see-through where the picture is within a tolerance of a colour",
    [
        color("keyColor", "Key Color", 0.0, 0.0, 1.0),
        ranged("colorTolerance", "Color Tolerance", 0.0, 0.0, 255.0),
        ranged("edgeThin", "Edge Thin", 0.0, -5.0, 5.0),
        ranged("edgeFeather", "Edge Feather", 0.0, 0.0, 100.0),
    ]
);
pub const COLOR_KEY: Spec = Spec::new(&COLOR_KEY_DEF, OBSOLETE, color_key);

fn color_key(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = rgb3(ctx.color("keyColor"));
    let tol = (ctx.num("colorTolerance", 0.0) / 255.0) as f32;
    let (w, h) = (b.img.width, b.img.height);
    let m = matte_by(&b.img, |c, _| if (c[0] - k[0]).abs().max((c[1] - k[1]).abs()).max((c[2] - k[2]).abs()) <= tol { 0.0 } else { 1.0 });
    let m = thin_feather(m, w, h, ctx.num("edgeThin", 0.0) * b.scale, ctx.num("edgeFeather", 0.0) * b.scale);
    keep_by(&mut b.img, &m, 0);
    b
}

effect!(
    LUMA_KEY_DEF,
    "lumaKey",
    "Luma Key",
    "the old keyer: see-through where the picture is brighter, darker, near or far from a level",
    [
        choice("keyType", "Key Type", &["keyOutBrighter", "keyOutDarker", "keyOutSimilar", "keyOutDissimilar"], 1),
        ranged("threshold", "Threshold", 0.0, 0.0, 255.0),
        ranged("tolerance", "Tolerance", 0.0, 0.0, 255.0),
        ranged("edgeThin", "Edge Thin", 0.0, -5.0, 5.0),
        ranged("edgeFeather", "Edge Feather", 0.0, 0.0, 100.0),
    ]
);
pub const LUMA_KEY: Spec = Spec::new(&LUMA_KEY_DEF, OBSOLETE, luma_key);

fn luma_key(ctx: &Ctx, mut b: Buf) -> Buf {
    let kind = ctx.choice("keyType");
    let (t, tol) = ((ctx.num("threshold", 0.0) / 255.0) as f32, (ctx.num("tolerance", 0.0) / 255.0) as f32);
    let (w, h) = (b.img.width, b.img.height);
    let m = matte_by(&b.img, |c, _| {
        let l = luma(c);
        match kind {
            0 => {
                if tol > 0.0 {
                    ((t + tol - l) / tol).clamp(0.0, 1.0)
                } else if l > t {
                    0.0
                } else {
                    1.0
                }
            }
            2 => {
                if (l - t).abs() <= tol {
                    0.0
                } else {
                    1.0
                }
            }
            3 => {
                if (l - t).abs() <= tol {
                    1.0
                } else {
                    0.0
                }
            }
            _ => {
                if tol > 0.0 {
                    ((l - (t - tol)) / tol).clamp(0.0, 1.0)
                } else if l < t {
                    0.0
                } else {
                    1.0
                }
            }
        }
    });
    let m = thin_feather(m, w, h, ctx.num("edgeThin", 0.0) * b.scale, ctx.num("edgeFeather", 0.0) * b.scale);
    keep_by(&mut b.img, &m, 0);
    b
}

effect!(
    SPILL_DEF,
    "spillSuppressor",
    "Spill Suppressor",
    "the old spill remover: a colour taken off where it spilled",
    [
        color("colorToSuppress", "Color To Suppress", 0.0, 0.0, 1.0),
        choice("colorAccuracy", "Color Accuracy", &["faster", "better"], 0).doc("better works for any colour, not only red, green or blue"),
        ranged("suppression", "Suppression", 100.0, 0.0, 200.0),
    ]
);
pub const SPILL: Spec = Spec::new(&SPILL_DEF, OBSOLETE, spill);

fn spill(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = rgb3(ctx.color("colorToSuppress"));
    let amount = (ctx.num("suppression", 100.0) / 100.0) as f32;
    if ctx.choice("colorAccuracy") == 1 {
        // the part of each colour pointing the way the key colour does, taken off
        let g = (k[0] + k[1] + k[2]) / 3.0;
        let kc = [k[0] - g, k[1] - g, k[2] - g];
        let n = (kc[0] * kc[0] + kc[1] * kc[1] + kc[2] * kc[2]).sqrt();
        if n < 1e-5 {
            return b;
        }
        let dir = kc.map(|v| v / n);
        crate::util::map_straight(&mut b.img, |c| {
            let g = (c[0] + c[1] + c[2]) / 3.0;
            let along = (c[0] - g) * dir[0] + (c[1] - g) * dir[1] + (c[2] - g) * dir[2];
            if along <= 0.0 { c } else { [0, 1, 2].map(|i| c[i] - along * dir[i] * amount) }
        });
        return b;
    }
    let (pi, o1, o2) = primary(k);
    crate::util::map_straight(&mut b.img, |mut c| {
        c[pi] -= (c[pi] - c[o1].max(c[o2])).max(0.0) * amount;
        c
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 13] = [LINEAR, DIFFERENCE_KEY, COLOR_RANGE, EXTRACT, DIFFERENCE_MATTE, INNER_OUTER, UNMULT, ADVANCED_SPILL, KEY_CLEANER, WIRE, COLOR_KEY, LUMA_KEY, SPILL];

#[cfg(test)]
mod tests;
