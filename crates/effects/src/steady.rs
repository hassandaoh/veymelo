//! Footage made steady: the Warp Stabilizer (each frame moved so the camera glides or holds
//! still, cropped and scaled so the edges do not show) and Rolling Shutter Repair (each row moved
//! back to where it was mid-frame, for cameras that read their picture a row at a time while it
//! moves). Both need the camera's motion worked out from all the frames first: effect.analyze
//! keeps it in their Analysis setting.

use std::sync::{Arc, Mutex};

use veymelo_geom::vec2;
use veymelo_keyframe::{Kind, Value};
use veymelo_project::{PropDef, Unit};
use veymelo_raster::{Image, Sampling, par};
use veymelo_track::stabilize::{self, Aim, Analysis, Framing, Method, Plan, Settings};

use crate::defs::{choice, effect, num, pct};
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
        choice("method", "Method", &["position", "positionScaleRotation", "perspective", "subspaceWarp"], 2).doc("how a frame may be moved: shifted; shifted, turned and scaled; in perspective; or in perspective and bent on a grid so each part is steadied on its own (things near and far, a rolling shutter's wobble)"),
        choice("framing", "Framing", &["stabilizeOnly", "stabilizeCrop", "stabilizeCropAutoScale", "synthesizeEdges"], 2).doc("the moved frames' edges shown, cut to what every frame has, cut so and scaled up to fill, or what they leave empty filled from the frames round them"),
        pct("maximumScale", "Maximum Scale", 150.0, 100.0, 1000.0).doc("the most it scales up to hide the edges"),
        pct("additionalScale", "Additional Scale", 100.0, 1.0, 1000.0).doc("a scale on top"),
        num("synthesisInputRange", "Synthesis Input Range", 1.0).range(0.0, 10.0).unit(Unit::Seconds).doc("synthesize edges: how far before and after a frame it looks for what its edges lack"),
        num("synthesisEdgeFeather", "Synthesis Edge Feather", 0.0).range(0.0, 200.0).unit(Unit::Pixels).doc("synthesize edges: how far into the frame its own picture fades into what fills round it"),
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
        choice("method", "Method", &["warp", "pixelMotion"], 0).doc("warp: the whole picture's motion (from effect.analyze); pixelMotion: each part's own, from the frames before and after (things moving across a still shot, or parts at different depths)"),
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
        method: [Method::Position, Method::Similar, Method::Perspective, Method::Subspace][ctx.choice("method").min(3) as usize],
        framing: [Framing::Stabilize, Framing::Crop, Framing::CropScale, Framing::Synthesize][ctx.choice("framing").min(3) as usize],
        most_scale: ctx.num("maximumScale", 150.0),
        extra_scale: ctx.num("additionalScale", 100.0),
    };
    let plan = planned(key, &a, &settings);
    if settings.method == Method::Subspace && plan.bends.is_empty() && a.frames.len() > 1 {
        ctx.note("Warp Stabilizer's Subspace Warp needs the parts' own moves, which this analysis lacks: effect.analyze again".into());
    }
    let k = a.frame_at(ctx.time.seconds());
    if plan.maps[k].inverse().is_none() {
        return buf;
    }
    if settings.framing == Framing::Synthesize {
        return synthesized(ctx, buf, &a, &plan, k);
    }
    let [x0, y0, x1, y1] = a.rect;
    let (mid, half) = ([(x0 + x1) / 2.0, (y0 + y1) / 2.0], [(x1 - x0) / 2.0 * plan.keep, (y1 - y0) / 2.0 * plan.keep]);
    redrawn(&buf, |p| plan.source(k, p, a.rect).unwrap_or(p), |p| (p[0] - mid[0]).abs() <= half[0] + 1e-9 && (p[1] - mid[1]).abs() <= half[1] + 1e-9)
}

/// Frame `k` stabilized, shown whole, what it leaves empty filled from the frames round it (the
/// nearest first, as far as the input range reaches), each brought to the same steadied view: a
/// point of the frame takes the colour of the nearest frame whose picture reaches there once
/// moved. Its own picture fades into that over the edge feather.
fn synthesized(ctx: &Ctx, buf: Buf, a: &Analysis, plan: &Plan, k: usize) -> Buf {
    let [x0, y0, x1, y1] = a.rect;
    let source = |j: usize, p: [f64; 2]| plan.source(j, p, a.rect);
    let within = |q: [f64; 2]| q[0] >= x0 && q[0] <= x1 && q[1] >= y0 && q[1] <= y1;
    // how far a point is inside the frame's picture (0 at its edge)
    let depth = |q: [f64; 2]| (q[0] - x0).min(x1 - q[0]).min(q[1] - y0).min(y1 - q[1]);
    let feather = ctx.num("synthesisEdgeFeather", 0.0).max(0.0);
    let reach = (ctx.num("synthesisInputRange", 1.0).max(0.0) / a.step.max(1e-9)).round() as usize;
    let (w, h, to_layer) = (buf.img.width, buf.img.height, buf.to_layer());
    let point = |i: usize| {
        let p = to_layer.apply(vec2((i % w) as f64 + 0.5, (i / w) as f64 + 0.5));
        [p.x, p.y]
    };
    if source(k, [x0, y0]).is_none() {
        return buf;
    }
    // its own picture where it reaches, as much as the feather lets it
    let mut out = Image::new(w, h);
    let mut wanted = vec![0f32; w * h];
    par::chunks(&mut out.px, w, |row, line| {
        for (x, px) in line.iter_mut().enumerate() {
            let p = point(row * w + x);
            if !within(p) {
                continue;
            }
            let Some(q) = source(k, p) else { continue };
            if within(q) {
                let at = buf.pixel(vec2(q[0], q[1]));
                let c = buf.img.sample(Sampling::Bilinear, at.x, at.y);
                let k = if feather > 0.0 { (depth(q) / feather).clamp(0.0, 1.0) as f32 } else { 1.0 };
                *px = c.map(|v| v * k);
            }
        }
    });
    for (i, want) in wanted.iter_mut().enumerate() {
        let p = point(i);
        if within(p) {
            *want = match source(k, p).filter(|q| within(*q)) {
                Some(q) if feather > 0.0 => 1.0 - (depth(q) / feather).clamp(0.0, 1.0) as f32,
                Some(_) => 0.0,
                None => 1.0,
            };
        }
    }
    // the frames round it, nearest first, each read only where it is still wanted
    let host = ctx.host;
    for d in 1..=reach {
        for j in [k as isize - d as isize, k as isize + d as isize] {
            if j < 0 || j as usize >= plan.maps.len() || wanted.iter().all(|v| *v <= 0.0) {
                continue;
            }
            let j = j as usize;
            let takes: Vec<usize> = (0..w * h).filter(|&i| wanted[i] > 0.0 && source(j, point(i)).is_some_and(within)).collect();
            if takes.is_empty() {
                continue;
            }
            let Some(other) = host.and_then(|h| h.this_at(veymelo_time::Tick::from_seconds(a.start + j as f64 * a.step), ctx.index)) else { continue };
            for i in takes {
                let Some(q) = source(j, point(i)) else { continue };
                let at = other.pixel(vec2(q[0], q[1]));
                let c = other.img.sample(Sampling::Bilinear, at.x, at.y);
                let k = wanted[i];
                for (o, v) in out.px[i].iter_mut().zip(c) {
                    *o += v * k;
                }
                wanted[i] = 0.0;
            }
        }
    }
    Buf { img: out, ..buf }
}

/// How each part of `buf` (the effect's layer now) moves, in its pixels a frame: the mean of its
/// flow into the next frame and back from the one before (`step` seconds apart), as many of them
/// as there are; none when neither can be had.
fn pixel_motion(ctx: &Ctx, buf: &Buf, step: f64) -> Option<impl Fn([f64; 2]) -> [f64; 2] + Sync + use<>> {
    let host = ctx.host?;
    let gray = |b: &Buf| veymelo_track::klt::Gray::new(&b.img, [-b.origin.x * b.scale, -b.origin.y * b.scale], 3);
    let now = gray(buf);
    let other = |dt: f64| host.this_at(ctx.time + veymelo_time::Tick::from_seconds(dt), ctx.index).filter(|b| b.img.width > 0 && b.img.height > 0).map(|b| veymelo_track::flow::Flow::between(&now, &gray(&b), 6));
    let (next, prev) = (other(step), other(-step));
    if next.is_none() && prev.is_none() {
        return None;
    }
    Some(move |p: [f64; 2]| {
        let mut sum = [0.0; 2];
        let mut n = 0.0;
        if let Some(f) = &next {
            let m = f.at(p);
            sum = [sum[0] + m[0], sum[1] + m[1]];
            n += 1.0;
        }
        if let Some(f) = &prev {
            let m = f.at(p);
            sum = [sum[0] - m[0], sum[1] - m[1]];
            n += 1.0;
        }
        [sum[0] / n, sum[1] / n]
    })
}

fn rolling_shutter(ctx: &Ctx, buf: Buf) -> Buf {
    let rate = ctx.num("rollingShutterRate", 50.0) / 100.0;
    let scan = ctx.choice("scanDirection");
    // each part's own motion, from the frames round it
    if ctx.choice("method") == 1 {
        let step = analysis(ctx).map_or(ctx.frame.seconds(), |(_, a)| a.step);
        let area = buf.area();
        let [x0, y0, x1, y1] = analysis(ctx).map_or([area.x0, area.y0, area.x1, area.y1], |(_, a)| a.rect);
        let along = move |p: [f64; 2]| match scan {
            1 => (y1 - p[1]) / (y1 - y0).max(1e-9),
            2 => (p[0] - x0) / (x1 - x0).max(1e-9),
            3 => (x1 - p[0]) / (x1 - x0).max(1e-9),
            _ => (p[1] - y0) / (y1 - y0).max(1e-9),
        };
        match pixel_motion(ctx, &buf, step) {
            Some(v) => {
                return redrawn(
                    &buf,
                    |p| {
                        let (m, late) = (v(p), rate * (along(p) - 0.5));
                        [p[0] + m[0] * late, p[1] + m[1] * late]
                    },
                    |_| true,
                );
            }
            None => {
                ctx.note("Rolling Shutter Repair's pixel motion needs the frames before or after this one".into());
                return buf;
            }
        }
    }
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
    let [x0, y0, x1, y1] = a.rect;
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
