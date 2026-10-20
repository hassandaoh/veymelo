//! Footage made steady: the Warp Stabilizer (each frame moved so the camera glides or holds
//! still, cropped and scaled so the edges do not show) and Rolling Shutter Repair (each row moved
//! back to where it was mid-frame, for cameras that read their picture a row at a time while it
//! moves). Both need the camera's motion worked out from all the frames first: effect.analyze
//! keeps it in their Analysis setting.

use std::sync::{Arc, Mutex};

use veymelo_geom::vec2;
use veymelo_keyframe::{Kind, Value};
use veymelo_project::PropDef;
use veymelo_raster::{Image, Sampling, par};
use veymelo_track::stabilize::{self, Aim, Analysis, Framing, Method, Plan, Settings};

use crate::defs::{choice, effect, pct};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "distort";

const ANALYSIS: PropDef = PropDef::new("analysis", "Analysis", Kind::Text, Value::Text(String::new())).setting().doc("how the camera moved, frame by frame, as effect.analyze worked it out");

effect!(
    WARP_DEF,
    "warpStabilizer",
    "Warp Stabilizer",
    "shaky footage made steady: each frame moved so the camera glides (or holds still), the edges cropped and scaled away; effect.analyze works out how it moved first",
    [
        choice("result", "Result", &["smoothMotion", "noMotion"], 0).doc("smoothMotion: the camera's moves kept, smoothed; noMotion: held still"),
        pct("smoothness", "Smoothness", 50.0, 0.0, 1000.0).doc("how smooth: 100 smooths over about a second either way"),
        choice("method", "Method", &["position", "positionScaleRotation", "perspective"], 2).doc("how a frame may be moved: shifted; shifted, turned and scaled; or in perspective"),
        choice("framing", "Framing", &["stabilizeOnly", "stabilizeCrop", "stabilizeCropAutoScale"], 2).doc("the moved frames' edges shown, cut to what every frame has, or cut so and scaled up to fill"),
        pct("maximumScale", "Maximum Scale", 150.0, 100.0, 1000.0).doc("the most it scales up to hide the edges"),
        pct("additionalScale", "Additional Scale", 100.0, 1.0, 1000.0).doc("a scale on top"),
        ANALYSIS,
    ]
);
pub const WARP: Spec = Spec::new(&WARP_DEF, CATEGORY, warp_stabilizer);

/// Which way a camera reads its picture.
pub const SCANS: &[&str] = &["topToBottom", "bottomToTop", "leftToRight", "rightToLeft"];

effect!(
    SHUTTER_DEF,
    "rollingShutterRepair",
    "Rolling Shutter Repair",
    "the slant and wobble of a camera that reads its picture a row at a time while it moves, taken out; effect.analyze works out how it moved first",
    [
        pct("rollingShutterRate", "Rolling Shutter Rate", 50.0, 0.0, 100.0).doc("how much of a frame's time the camera takes to read its picture"),
        choice("scanDirection", "Scan Direction", SCANS, 0).doc("which way it reads it"),
        ANALYSIS,
    ]
);
pub const SHUTTER: Spec = Spec::new(&SHUTTER_DEF, CATEGORY, rolling_shutter);

pub const ALL: [Spec; 2] = [WARP, SHUTTER];

/// The ids of the effects that need effect.analyze.
pub const ANALYSED: &[&str] = &["warpStabilizer", "rollingShutterRepair"];

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xCBF2_9CE4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01B3))
}

/// The analysis an effect keeps (and which text it was), read once for as long as it stays the
/// same.
fn analysis(ctx: &Ctx) -> Option<(u64, Arc<Analysis>)> {
    static READ: Mutex<Option<(u64, Arc<Analysis>)>> = Mutex::new(None);
    let text = ctx.value_in(ctx.effect, "analysis")?;
    let text = text.as_text();
    if text.is_empty() {
        return None;
    }
    let key = fnv(text);
    let mut kept = READ.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, a)) = kept.as_ref()
        && *k == key
    {
        return Some((key, a.clone()));
    }
    let a = Arc::new(Analysis::from_text(text)?);
    *kept = Some((key, a.clone()));
    Some((key, a))
}

/// The stabilized clip for an analysis (known by `key`) and settings, worked out once while they
/// stay the same.
fn planned(key: u64, a: &Analysis, s: &Settings) -> Arc<Plan> {
    static PLANS: Mutex<Option<(u64, Settings, Arc<Plan>)>> = Mutex::new(None);
    let mut kept = PLANS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, settings, plan)) = kept.as_ref()
        && *k == key
        && settings == s
    {
        return plan.clone();
    }
    let plan = Arc::new(stabilize::plan(a, s));
    *kept = Some((key, *s, plan.clone()));
    plan
}

/// `buf` drawn again, each output point of the layer taking the layer's colour at `from(point)`;
/// points `keep(point)` refuses left clear.
fn redrawn(buf: &Buf, from: impl Fn([f64; 2]) -> [f64; 2] + Sync, keep: impl Fn([f64; 2]) -> bool + Sync) -> Buf {
    let (w, to_layer) = (buf.img.width, buf.to_layer());
    let mut out = Image::new(w, buf.img.height);
    par::rows(&mut out.px, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let p = to_layer.apply(vec2(x as f64 + 0.5, y as f64 + 0.5));
            if !keep([p.x, p.y]) {
                continue;
            }
            let q = from([p.x, p.y]);
            let at = buf.pixel(vec2(q[0], q[1]));
            *px = buf.img.sample(Sampling::Bilinear, at.x, at.y);
        }
    });
    Buf { img: out, ..buf.clone() }
}

fn warp_stabilizer(ctx: &Ctx, buf: Buf) -> Buf {
    let Some((key, a)) = analysis(ctx).filter(|(_, a)| !a.frames.is_empty()) else {
        ctx.note("Warp Stabilizer has not worked out the camera's motion yet: effect.analyze does".into());
        return buf;
    };
    let settings = Settings {
        aim: if ctx.choice("result") == 1 { Aim::Still } else { Aim::Smooth },
        smoothness: ctx.num("smoothness", 50.0),
        method: [Method::Position, Method::Similar, Method::Perspective][ctx.choice("method").min(2) as usize],
        framing: [Framing::Stabilize, Framing::Crop, Framing::CropScale][ctx.choice("framing").min(2) as usize],
        most_scale: ctx.num("maximumScale", 150.0),
        extra_scale: ctx.num("additionalScale", 100.0),
    };
    let plan = planned(key, &a, &settings);
    let k = a.frame_at(ctx.time.seconds());
    let Some(back) = plan.maps[k].inverse() else { return buf };
    let [x0, y0, x1, y1] = a.rect;
    let (mid, half) = ([(x0 + x1) / 2.0, (y0 + y1) / 2.0], [(x1 - x0) / 2.0 * plan.keep, (y1 - y0) / 2.0 * plan.keep]);
    redrawn(&buf, |p| back.apply(p), |p| (p[0] - mid[0]).abs() <= half[0] + 1e-9 && (p[1] - mid[1]).abs() <= half[1] + 1e-9)
}

fn rolling_shutter(ctx: &Ctx, buf: Buf) -> Buf {
    let Some((_, a)) = analysis(ctx).filter(|(_, a)| !a.frames.is_empty()) else {
        ctx.note("Rolling Shutter Repair has not worked out how the picture moves yet: effect.analyze does".into());
        return buf;
    };
    // how fast the picture moves at this frame, in pixels a frame: the mean of the moves into and
    // out of it
    let k = a.frame_at(ctx.time.seconds());
    let n = a.frames.len();
    let moves: Vec<[f64; 2]> = [k, k + 1].iter().filter(|&&i| i >= 1 && i < n).map(|&i| a.frames[i].shift).collect();
    if moves.is_empty() {
        return buf;
    }
    let v = [moves.iter().map(|m| m[0]).sum::<f64>() / moves.len() as f64, moves.iter().map(|m| m[1]).sum::<f64>() / moves.len() as f64];
    let rate = ctx.num("rollingShutterRate", 50.0) / 100.0;
    let [x0, y0, x1, y1] = a.rect;
    let scan = ctx.choice("scanDirection");
    let along = move |p: [f64; 2]| match scan {
        1 => (y1 - p[1]) / (y1 - y0).max(1e-9),
        2 => (p[0] - x0) / (x1 - x0).max(1e-9),
        3 => (x1 - p[0]) / (x1 - x0).max(1e-9),
        _ => (p[1] - y0) / (y1 - y0).max(1e-9),
    };
    // a row read later shows the picture moved on that much more: taken back from there
    redrawn(
        &buf,
        |p| {
            let late = rate * (along(p) - 0.5);
            [p[0] + v[0] * late, p[1] + v[1] * late]
        },
        |_| true,
    )
}
