//! Mattes: the alpha grown or shrunk (Simple Choker, Matte Choker), and its edge refined to
//! follow the picture, steadied across frames and freed of the background's colour (Refine Soft
//! Matte, Refine Hard Matte).

use veymelo_raster::filter::{self, Edge};
use veymelo_raster::Image;
use veymelo_time::Tick;

use crate::channel::{box_extreme, line_extreme};
use crate::defs::{choice, effect, flag, pct, ranged};
use crate::film::guided;
use crate::util::{luma, map_px, premul, unpremul};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "matte";

/// The alpha of each pixel.
fn alpha_of(img: &Image) -> Vec<f32> {
    img.px.iter().map(|p| p[3]).collect()
}

/// A new alpha, the colours kept.
fn set_alpha(img: &mut Image, a: &[f32]) {
    for (p, na) in img.px.iter_mut().zip(a) {
        let (c, _) = unpremul(*p);
        *p = premul(c, na.clamp(0.0, 1.0));
    }
}

/// A plane's greatest (or, not `most`, least) value within a round brush of whole radius `r`
/// (nothing past the edges).
fn disc_extreme(plane: &[f32], w: usize, h: usize, r: usize, most: bool) -> Vec<f32> {
    if r == 0 {
        return plane.to_vec();
    }
    let outside = Some(0.0);
    let ri = r as i64;
    let half = |dy: i64| (((r * r) as i64 - dy * dy) as f64).sqrt().floor() as usize;
    let mut out = vec![if most { f32::MIN } else { f32::MAX }; w * h];
    // each row's extremes over each half-width the brush has, folded in for the brush's rows
    // that are that wide
    let mut widths: Vec<usize> = (0..=ri).map(half).collect();
    widths.dedup();
    let mut rows = vec![0f32; w * h];
    for hw in widths {
        for y in 0..h {
            rows[y * w..(y + 1) * w].copy_from_slice(&line_extreme(&plane[y * w..(y + 1) * w], hw, most, outside));
        }
        for dy in (-ri..=ri).filter(|dy| half(*dy) == hw) {
            for y in 0..h {
                let sy = y as i64 + dy;
                for x in 0..w {
                    let v = if sy >= 0 && sy < h as i64 { rows[sy as usize * w + x] } else { 0.0 };
                    let o = &mut out[y * w + x];
                    *o = if most { o.max(v) } else { o.min(v) };
                }
            }
        }
    }
    out
}

/// Grown (or, not `most`, shrunk) round by `radius` pixels, part pixels counting by their part.
pub(crate) fn rounded_extreme(plane: &[f32], w: usize, h: usize, radius: f64, most: bool) -> Vec<f32> {
    if radius <= 0.0 {
        return plane.to_vec();
    }
    let r0 = radius.floor() as usize;
    let t = (radius - r0 as f64) as f32;
    let a = disc_extreme(plane, w, h, r0, most);
    if t < 1e-4 {
        return a;
    }
    let b = disc_extreme(plane, w, h, r0 + 1, most);
    a.iter().zip(&b).map(|(x, y)| x + (y - x) * t).collect()
}

effect!(
    SIMPLE_CHOKER_DEF,
    "simpleChoker",
    "Simple Choker",
    "the matte's edge pulled in (above 0) or pushed out (below 0) by so many pixels",
    [choice("view", "View", &["finalOutput", "matte"], 0), ranged("chokeMatte", "Choke Matte", 0.0, -100.0, 100.0),]
);
pub const SIMPLE_CHOKER: Spec = Spec::new(&SIMPLE_CHOKER_DEF, CATEGORY, simple_choker);

/// The alpha shown as grey on black.
fn show_matte(b: &mut Buf) {
    for p in &mut b.img.px {
        let a = p[3].clamp(0.0, 1.0);
        *p = [a, a, a, 1.0];
    }
}

fn simple_choker(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = ctx.num("chokeMatte", 0.0) * b.scale;
    if c.abs() > 1e-6 {
        if c < 0.0 && !ctx.adjustment {
            b.pad((-c).ceil() as usize + 1);
        }
        let (w, h) = (b.img.width, b.img.height);
        let a = rounded_extreme(&alpha_of(&b.img), w, h, c.abs(), c < 0.0);
        set_alpha(&mut b.img, &a);
    }
    if ctx.choice("view") == 1 {
        show_matte(&mut b);
    }
    b
}

effect!(
    MATTE_CHOKER_DEF,
    "matteChoker",
    "Matte Choker",
    "the matte softened and cut again, twice over: holes filled, edges cleaned",
    [
        ranged("geometricSoftness1", "Geometric Softness 1", 4.0, 0.0, 1000.0),
        ranged("choke1", "Choke 1", 75.0, -127.0, 127.0),
        pct("grayLevelSoftness1", "Gray Level Softness 1", 10.0, 0.0, 100.0),
        ranged("geometricSoftness2", "Geometric Softness 2", 0.0, 0.0, 1000.0),
        ranged("choke2", "Choke 2", 0.0, -127.0, 127.0),
        pct("grayLevelSoftness2", "Gray Level Softness 2", 100.0, 0.0, 100.0),
        ranged("iterations", "Iterations", 1.0, 1.0, 100.0),
    ]
);
pub const MATTE_CHOKER: Spec = Spec::new(&MATTE_CHOKER_DEF, CATEGORY, matte_choker);

/// One stage: softened by `soft` pixels, then cut at a level by the choke, over a ramp as wide
/// as the grey-level softness.
fn choke_stage(a: Vec<f32>, w: usize, h: usize, soft: f64, choke: f32, grey: f32) -> Vec<f32> {
    let a = if soft > 0.0 {
        let mut img = Image::new(w, h);
        for (p, v) in img.px.iter_mut().zip(&a) {
            *p = [*v; 4];
        }
        filter::gaussian(&img, soft * 0.5, soft * 0.5, Edge::Clear).px.iter().map(|p| p[0]).collect()
    } else {
        a
    };
    let level = 0.5 + choke / 255.0 * 0.5;
    let ramp = grey.max(1e-3);
    if (level - 0.5).abs() < 1e-6 && (ramp - 1.0).abs() < 1e-6 {
        return a;
    }
    a.iter().map(|v| ((v - level) / ramp + 0.5).clamp(0.0, 1.0)).collect()
}

fn matte_choker(ctx: &Ctx, mut b: Buf) -> Buf {
    let (g1, c1, s1) = (ctx.num("geometricSoftness1", 4.0) * b.scale, ctx.num("choke1", 75.0) as f32, (ctx.num("grayLevelSoftness1", 10.0) / 100.0) as f32);
    let (g2, c2, s2) = (ctx.num("geometricSoftness2", 0.0) * b.scale, ctx.num("choke2", 0.0) as f32, (ctx.num("grayLevelSoftness2", 100.0) / 100.0) as f32);
    let n = ctx.num("iterations", 1.0).round().clamp(1.0, 100.0) as usize;
    if (c1 < 0.0 || c2 < 0.0 || g1 > 0.0 || g2 > 0.0) && !ctx.adjustment {
        b.pad(((g1 + g2) * 1.5 * n as f64).ceil().min(4096.0) as usize + 1);
    }
    let (w, h) = (b.img.width, b.img.height);
    let mut a = alpha_of(&b.img);
    for _ in 0..n {
        a = choke_stage(a, w, h, g1, c1, s1);
        a = choke_stage(a, w, h, g2, c2, s2);
    }
    set_alpha(&mut b.img, &a);
    b
}

/// The settings the two refiners share: steadying across frames, motion blur, taking the
/// background's colour out of the edges, inverting.
macro_rules! refine_effect {
    ($def:ident, $id:literal, $name:literal, $doc:literal, [$($own:expr),*], $radius:expr) => {
        effect!(
            $def,
            $id,
            $name,
            $doc,
            [
                $($own,)*
                ranged("additionalEdgeRadius", "Additional Edge Radius", $radius, 1.0, 200.0).doc("how far either side of the matte's edge is looked at"),
                pct("reduceChatter", "Reduce Chatter", 0.0, 0.0, 100.0).doc("where the frames either side agree and this one jitters, it is steadied"),
                flag("useMotionBlur", "Use Motion Blur", false),
                ranged("motionBlurSamples", "Motion Blur Samples", 16.0, 1.0, 64.0),
                ranged("shutterAngle", "Shutter Angle", 180.0, 0.0, 720.0),
                flag("higherQuality", "Higher Quality", false),
                flag("decontaminateEdgeColors", "Decontaminate Edge Colors", true).doc("the background's colour taken out of the see-through edges"),
                pct("decontaminationAmount", "Decontamination Amount", 100.0, 0.0, 100.0),
                flag("extendWhereSmoothed", "Extend Where Smoothed", true),
                ranged("increaseDecontaminationRadius", "Increase Decontamination Radius", 0.0, 0.0, 100.0),
                flag("viewDecontaminationMap", "View Decontamination Map", false),
                flag("invert", "Invert", false),
            ]
        );
    };
}

refine_effect!(
    REFINE_SOFT_DEF,
    "refineSoftMatte",
    "Refine Soft Matte",
    "a matte's edge made to follow fine detail (hair, fur, smoke) in the picture",
    [
        flag("calculateEdgeDetails", "Calculate Edge Details", true),
        flag("viewEdgeRegion", "View Edge Region", false),
        pct("smooth", "Smooth", 0.0, 0.0, 100.0),
        pct("feather", "Feather", 10.0, 0.0, 100.0),
        pct("contrast", "Contrast", 0.0, 0.0, 100.0),
        ranged("shiftEdge", "Shift Edge", 0.0, -100.0, 100.0).doc("above 0 the matte grows")
    ],
    10.0
);
pub const REFINE_SOFT: Spec = Spec::new(&REFINE_SOFT_DEF, CATEGORY, refine_soft);

refine_effect!(
    REFINE_HARD_DEF,
    "refineHardMatte",
    "Refine Hard Matte",
    "a matte's hard edge made to follow the picture's edges, smooth and clean",
    [
        pct("smooth", "Smooth", 20.0, 0.0, 100.0),
        pct("feather", "Feather", 0.0, 0.0, 100.0),
        ranged("choke", "Choke", 0.0, -100.0, 100.0).doc("above 0 the matte shrinks"),
        ranged("alphaContrast", "Alpha Contrast", 300.0, 100.0, 1000.0)
    ],
    3.0
);
pub const REFINE_HARD: Spec = Spec::new(&REFINE_HARD_DEF, CATEGORY, refine_hard);

fn refine_soft(ctx: &Ctx, b: Buf) -> Buf {
    refine(ctx, b, false)
}

fn refine_hard(ctx: &Ctx, b: Buf) -> Buf {
    refine(ctx, b, true)
}

fn refine(ctx: &Ctx, mut b: Buf, hard: bool) -> Buf {
    let r = (ctx.num("additionalEdgeRadius", if hard { 3.0 } else { 10.0 }) * b.scale).round().max(1.0) as usize;
    let smooth = (ctx.num("smooth", if hard { 20.0 } else { 0.0 }) / 100.0) as f32;
    let feather = ctx.num("feather", if hard { 0.0 } else { 10.0 }) / 100.0 * r as f64 * 0.5;
    // shrinking above 0 (hard's choke), growing above 0 (soft's shift edge)
    let choke = (if hard { ctx.num("choke", 0.0) } else { -ctx.num("shiftEdge", 0.0) }).clamp(-100.0, 100.0) as f32 / 100.0 * 0.5;
    let contrast = if hard { (ctx.num("alphaContrast", 300.0) / 100.0) as f32 } else { 1.0 + (ctx.num("contrast", 0.0).max(0.0) / 100.0) as f32 * 3.0 };
    let details = hard || ctx.flag("calculateEdgeDetails");
    let invert = ctx.flag("invert");
    let (w, h) = (b.img.width, b.img.height);
    // the matte refined: in the band round its edge, made to follow the picture
    let refined = |img: &Image| -> Vec<f32> {
        let a = alpha_of(img);
        let (hi, lo) = (box_extreme(&a, w, h, r, r, true, None), box_extreme(&a, w, h, r, r, false, None));
        let mut na = if details {
            let guide: Vec<f32> = img.px.iter().map(|p| luma([p[0], p[1], p[2]])).collect();
            let filtered = guided(&guide, &a, w, h, r, 1e-4 + smooth * smooth * 0.05);
            (0..w * h).map(|i| if hi[i] - lo[i] > 1e-3 { filtered[i].clamp(0.0, 1.0) } else { a[i] }).collect::<Vec<f32>>()
        } else if smooth > 0.0 {
            let s = smooth as f64 * r as f64 * 0.5;
            soften(&a, w, h, s)
        } else {
            a.clone()
        };
        if feather > 0.0 {
            na = soften(&na, w, h, feather);
        }
        na.iter()
            .map(|v| {
                let mut v = if choke > 0.0 {
                    (v - choke) / (1.0 - choke)
                } else if choke < 0.0 {
                    v / (1.0 + choke)
                } else {
                    *v
                };
                if (contrast - 1.0).abs() > 1e-6 {
                    v = (v - 0.5) * contrast + 0.5;
                }
                let v = v.clamp(0.0, 1.0);
                if invert { 1.0 - v } else { v }
            })
            .collect()
    };
    let mut na = refined(&b.img);
    // the same refinement of this effect's input at another time, if on the same pixels
    let at = |t: Tick| -> Option<Vec<f32>> {
        let o = ctx.host?.this_at(t, ctx.index)?;
        (o.img.width == w && o.img.height == h && (o.origin - b.origin).length() < 1e-6).then(|| refined(&o.img))
    };
    let chatter = (ctx.num("reduceChatter", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    if chatter > 0.0
        && let (Some(p), Some(n)) = (at(ctx.time - ctx.frame), at(ctx.time + ctx.frame))
    {
        for (i, v) in na.iter_mut().enumerate() {
            // steady where the frames either side agree
            let steady = (1.0 - (p[i] - n[i]).abs() / 0.5).clamp(0.0, 1.0);
            *v += ((p[i] + *v + n[i]) / 3.0 - *v) * chatter * steady;
        }
    }
    if ctx.flag("useMotionBlur") {
        let samples = ctx.num("motionBlurSamples", 16.0).clamp(1.0, 64.0) as usize;
        let samples = if ctx.flag("higherQuality") { samples * 2 } else { samples }.min(64);
        let span = ctx.num("shutterAngle", 180.0).clamp(0.0, 720.0) / 360.0 * ctx.frame.seconds();
        if samples > 1 && span > 0.0 {
            let (mut acc, mut n) = (na.clone(), 1.0f32);
            for k in 0..samples {
                let dt = -span * 0.5 + span * k as f64 / (samples - 1) as f64;
                if dt.abs() < 1e-9 {
                    continue;
                }
                if let Some(p) = at(ctx.time + Tick::from_seconds(dt)) {
                    for (a, q) in acc.iter_mut().zip(&p) {
                        *a += q;
                    }
                    n += 1.0;
                }
            }
            if n > 1.0 {
                na = acc.iter().map(|a| a / n).collect();
            }
        }
    }
    if !hard && ctx.flag("viewEdgeRegion") {
        let a = alpha_of(&b.img);
        let (hi, lo) = (box_extreme(&a, w, h, r, r, true, None), box_extreme(&a, w, h, r, r, false, None));
        for (i, p) in b.img.px.iter_mut().enumerate() {
            let v = na[i];
            *p = if hi[i] - lo[i] > 1e-3 { [0.5 + 0.5 * v, 0.5 * v, 0.5 * v, 1.0] } else { [v, v, v, 1.0] };
        }
        return b;
    }
    let decon = ctx.flag("decontaminateEdgeColors");
    let amount = (ctx.num("decontaminationAmount", 100.0) / 100.0) as f32;
    let extend = ctx.flag("extendWhereSmoothed");
    // how much of the background's colour to take out: where the matte is part see-through
    let strength = |i: usize, a0: f32| -> f32 {
        let part = |a: f32| a > 1e-4 && a < 0.999;
        amount
            * if part(a0) {
                1.0 - a0
            } else if extend && part(na[i]) {
                1.0 - na[i]
            } else {
                0.0
            }
    };
    if ctx.flag("viewDecontaminationMap") {
        for (i, p) in b.img.px.iter_mut().enumerate() {
            let v = if decon { strength(i, p[3]) } else { 0.0 };
            *p = [v, v, v, 1.0];
        }
        return b;
    }
    if decon && amount > 0.0 {
        // the colour the solid parts nearby have, spread into the edges
        let reach = r as f64 + ctx.num("increaseDecontaminationRadius", 0.0).max(0.0) * b.scale;
        let near = filter::gaussian(&b.img, reach * 0.5, reach * 0.5, Edge::Clear);
        let before = b.img.clone();
        map_px(&mut b.img, |x, y, p| {
            let i = y * w + x;
            let (c, a0) = unpremul(p);
            let t = strength(i, a0);
            let (ec, ea) = unpremul(near.px[i]);
            if t > 0.0 && a0 > 1e-4 && ea > 1e-4 { premul([0, 1, 2].map(|k| c[k] + (ec[k] - c[k]) * t), a0) } else { before.px[i] }
        });
    }
    set_alpha(&mut b.img, &na);
    b
}

/// A plane softened by a Gaussian of deviation `s`.
pub(crate) fn soften(a: &[f32], w: usize, h: usize, s: f64) -> Vec<f32> {
    let mut img = Image::new(w, h);
    for (p, v) in img.px.iter_mut().zip(a) {
        *p = [*v; 4];
    }
    filter::gaussian(&img, s, s, Edge::Repeat).px.iter().map(|p| p[0]).collect()
}

/// Every effect here.
pub const ALL: [Spec; 4] = [SIMPLE_CHOKER, MATTE_CHOKER, REFINE_SOFT, REFINE_HARD];

#[cfg(test)]
mod tests;
