//! Time: the layer mixed with itself at other times — echoes trailing behind (Echo), held to a
//! lower frame rate (Posterize Time), compared with a moment before (Time Difference), each pixel
//! from its own moment (Time Displacement), slowed, sped or frozen with frames built in between
//! (Timewarp), and blurred by its own motion (Force Motion Blur, Wide Time, Pixel Motion Blur).
//!
//! The layer at another time is as the renderer draws it there: its masks and the effects above
//! this one. With no renderer to ask (thumbnails, tests) the layer passes through.

use std::rc::Rc;

use veymelo_raster::flow::{self, Field, Flow, Plane};
use veymelo_raster::{Image, Px, Sampling};
use veymelo_time::Tick;

use crate::defs::{choice, effect, flag, layer, pct, ranged};
use crate::util::{luma, map_px, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "time";

/// The layer at its own times `times` (seconds), all laid on one grid of pixels: the grid and a
/// picture for each time.
fn frames(ctx: &Ctx, times: &[f64]) -> Option<(Buf, Vec<Image>)> {
    let host = ctx.host?;
    let ticks: Vec<Tick> = times.iter().map(|t| Tick::from_seconds(*t)).collect();
    let mut keys = ticks.clone();
    keys.sort_unstable();
    keys.dedup();
    let got: Vec<Option<Buf>> = keys.iter().map(|k| host.this_at(*k, ctx.index)).collect();
    let first = got.iter().flatten().next()?;
    let s = first.scale;
    // all of them together, in layer units
    let mut area = first.area();
    for b in got.iter().flatten() {
        area = area.union(&b.area());
    }
    let origin = veymelo_geom::vec2((area.x0 * s).floor() / s, (area.y0 * s).floor() / s);
    let (w, h) = ((((area.x1 - origin.x) * s).ceil() as usize).clamp(1, 16_384), (((area.y1 - origin.y) * s).ceil() as usize).clamp(1, 16_384));
    let lay = |b: &Buf| -> Image {
        if (b.scale - s).abs() < 1e-9 && b.img.width == w && b.img.height == h && (b.origin - origin).length() < 1e-9 {
            return b.img.clone();
        }
        let mut out = Image::new(w, h);
        map_px(&mut out, |x, y, _| {
            let q = b.pixel(origin + veymelo_geom::vec2(x as f64 + 0.5, y as f64 + 0.5) / s);
            b.img.sample(Sampling::Bilinear, q.x, q.y)
        });
        out
    };
    let laid: Vec<Image> = got.iter().map(|b| b.as_ref().map_or_else(|| Image::new(w, h), lay)).collect();
    let pics = ticks.iter().map(|t| keys.binary_search(t).map_or_else(|_| Image::new(w, h), |i| laid[i].clone())).collect();
    Some((Buf { img: Image::new(w, h), origin, scale: s }, pics))
}

/// Pictures mixed by weights.
fn mean(pics: &[Image], weights: &[f32]) -> Image {
    let mut out = Image::new(pics[0].width, pics[0].height);
    map_px(&mut out, |x, y, _| {
        let i = y * pics[0].width + x;
        let mut acc = [0f32; 4];
        for (p, k) in pics.iter().zip(weights) {
            for (a, v) in acc.iter_mut().zip(p.px[i]) {
                *a += v * k;
            }
        }
        acc
    });
    out
}

/// The layer averaged over these times.
fn averaged(ctx: &Ctx, b: Buf, times: &[f64]) -> Buf {
    if times.is_empty() {
        return b;
    }
    let Some((grid, pics)) = frames(ctx, times) else { return b };
    let k = 1.0 / times.len() as f32;
    Buf { img: mean(&pics, &vec![k; times.len()]), ..grid }
}

const ECHO_OPERATORS: &[&str] = &["add", "maximum", "minimum", "screen", "compositeInBack", "compositeInFront", "blend"];

effect!(
    ECHO_DEF,
    "echo",
    "Echo",
    "the layer with copies of itself from moments before (or after) trailing it, each fainter",
    [
        ranged("echoTime", "Echo Time (seconds)", -1.0 / 30.0, -30.0, 30.0).doc("how far apart the echoes are; below 0 they come from before"),
        ranged("numberOfEchoes", "Number Of Echoes", 1.0, 0.0, 255.0),
        ranged("startingIntensity", "Starting Intensity", 1.0, 0.0, 1.0),
        ranged("decay", "Decay", 1.0, 0.0, 1.0).doc("each echo this much of the one before"),
        choice("echoOperator", "Echo Operator", ECHO_OPERATORS, 0),
    ]
);
pub const ECHO: Spec = Spec::new(&ECHO_DEF, CATEGORY, echo);

fn echo(ctx: &Ctx, b: Buf) -> Buf {
    let n = ctx.num("numberOfEchoes", 1.0).round().clamp(0.0, 255.0) as usize;
    let dt = ctx.num("echoTime", -1.0 / 30.0);
    let start = ctx.num("startingIntensity", 1.0).clamp(0.0, 1.0) as f32;
    let decay = ctx.num("decay", 1.0).clamp(0.0, 1.0) as f32;
    let op = ctx.choice("echoOperator");
    let t = ctx.time.seconds();
    let times: Vec<f64> = (0..=n).map(|i| t + i as f64 * dt).collect();
    let Some((grid, pics)) = frames(ctx, &times) else { return b };
    let weights: Vec<f32> = (0..=n).map(|i| start * decay.powi(i as i32)).collect();
    let count = (n + 1) as f32;
    let w = grid.img.width;
    let mut out = Image::new(w, grid.img.height);
    map_px(&mut out, |x, y, _| {
        let i = y * w + x;
        let mut acc: Px = if op == 2 { [f32::MAX; 4] } else { [0.0; 4] };
        for (p, k) in pics.iter().zip(&weights) {
            let q = p.px[i].map(|v| v * k);
            match op {
                1 => (0..4).for_each(|c| acc[c] = acc[c].max(q[c])),
                2 => (0..4).for_each(|c| acc[c] = acc[c].min(q[c])),
                3 => (0..4).for_each(|c| acc[c] = acc[c] + q[c] - acc[c] * q[c]),
                // each later echo behind what is there
                4 => {
                    let under = 1.0 - acc[3];
                    (0..4).for_each(|c| acc[c] += q[c] * under);
                }
                // each later echo in front
                5 => {
                    let under = 1.0 - q[3];
                    (0..4).for_each(|c| acc[c] = q[c] + acc[c] * under);
                }
                _ => (0..4).for_each(|c| acc[c] += q[c]),
            }
        }
        if op == 6 {
            acc = acc.map(|v| v / count);
        }
        if op == 2 && acc[0] == f32::MAX {
            acc = [0.0; 4];
        }
        [acc[0].max(0.0), acc[1].max(0.0), acc[2].max(0.0), acc[3].clamp(0.0, 1.0)]
    });
    Buf { img: out, ..grid }
}

effect!(POSTERIZE_TIME_DEF, "posterizeTime", "Posterize Time", "the layer moving at a lower frame rate, each frame held", [ranged("frameRate", "Frame Rate", 12.0, 0.1, 99.0),]);
pub const POSTERIZE_TIME: Spec = Spec::new(&POSTERIZE_TIME_DEF, CATEGORY, posterize_time);

/// The time a frame rate of `rate` holds at `t`.
pub fn held(t: f64, rate: f64) -> f64 {
    if rate <= 0.0 { t } else { ((t * rate) + 1e-6).floor() / rate }
}

fn posterize_time(ctx: &Ctx, b: Buf) -> Buf {
    let t = held(ctx.time.seconds(), ctx.num("frameRate", 12.0));
    match frames(ctx, &[t]) {
        Some((grid, mut pics)) => Buf { img: pics.pop().unwrap_or_else(|| Image::new(1, 1)), ..grid },
        None => b,
    }
}

const DIFF_ALPHA: &[&str] = &["original", "target", "blend", "max", "fullOn", "lightnessOfResult", "maxOfResult", "alphaDifference", "alphaDifferenceOnly"];

effect!(
    TIME_DIFF_DEF,
    "timeDifference",
    "Time Difference",
    "how the layer differs from another (or itself) a moment away: still parts dark, changes bright",
    [
        layer("target", "Target").doc("none: this layer"),
        ranged("timeOffsetSec", "Time Offset (sec)", 0.0, -30.0, 30.0),
        pct("contrast", "Contrast", 0.0, 0.0, 100.0),
        flag("absoluteDifference", "Absolute Difference", true).doc("off: no difference is mid grey"),
        choice("alphaChannel", "Alpha Channel", DIFF_ALPHA, 0),
    ]
);
pub const TIME_DIFF: Spec = Spec::new(&TIME_DIFF_DEF, CATEGORY, time_difference);

fn time_difference(ctx: &Ctx, b: Buf) -> Buf {
    let Some(host) = ctx.host else { return b };
    let off = ctx.num("timeOffsetSec", 0.0);
    let gain = 1.0 + (ctx.num("contrast", 0.0) / 25.0) as f32;
    let abs = ctx.flag("absoluteDifference");
    let how = ctx.choice("alphaChannel");
    let t = ctx.time.seconds();
    let target_id = ctx.value_in(ctx.effect, "target").and_then(|v| v.as_layer());
    let (cur, target) = match target_id.and_then(|id| host.layer_at(id, Tick::from_seconds(off), Take::Source)) {
        Some((other, rect)) => {
            let Some((grid, mut pics)) = frames(ctx, &[t]) else { return b };
            let now = pics.pop().unwrap_or_else(|| Image::new(1, 1));
            let cur = Buf { img: now, ..grid };
            // the target stretched over this layer
            let r = ctx.bounds;
            let mut tgt = Image::new(cur.img.width, cur.img.height);
            map_px(&mut tgt, |x, y, _| {
                let p = cur.origin + veymelo_geom::vec2(x as f64 + 0.5, y as f64 + 0.5) / cur.scale;
                let q = veymelo_geom::vec2(rect.x0 + (p.x - r.x0) / r.width().max(1e-9) * rect.width(), rect.y0 + (p.y - r.y0) / r.height().max(1e-9) * rect.height());
                let s = other.pixel(q);
                other.img.sample(Sampling::Bilinear, s.x, s.y)
            });
            (cur, tgt)
        }
        None => {
            let Some((grid, mut pics)) = frames(ctx, &[t, t + off]) else { return b };
            let then = pics.pop().unwrap_or_else(|| Image::new(1, 1));
            let now = pics.pop().unwrap_or_else(|| Image::new(1, 1));
            (Buf { img: now, ..grid }, then)
        }
    };
    let mut out = cur.img.clone();
    for (o, q) in out.px.iter_mut().zip(&target.px) {
        let (c, ca) = unpremul(*o);
        let (d, da) = unpremul(*q);
        let mut rgb = [0, 1, 2].map(|i| {
            let diff = c[i] - d[i];
            if abs { (diff.abs() * gain).clamp(0.0, 1.0) } else { (0.5 + diff * 0.5 * gain).clamp(0.0, 1.0) }
        });
        let light = (rgb[0].max(rgb[1]).max(rgb[2]) + rgb[0].min(rgb[1]).min(rgb[2])) * 0.5;
        let a = match how {
            1 => da,
            2 => (ca + da) * 0.5,
            3 => ca.max(da),
            4 => 1.0,
            5 => light,
            6 => rgb[0].max(rgb[1]).max(rgb[2]),
            7 => (ca - da).abs() * gain,
            8 => {
                rgb = [1.0; 3];
                (ca - da).abs() * gain
            }
            _ => ca,
        }
        .clamp(0.0, 1.0);
        *o = [rgb[0] * a, rgb[1] * a, rgb[2] * a, a];
    }
    Buf { img: out, ..cur }
}

effect!(
    TIME_DISPLACEMENT_DEF,
    "timeDisplacement",
    "Time Displacement",
    "each pixel from its own moment, earlier or later by how bright a map layer is there",
    [
        layer("timeDisplacementLayer", "Time Displacement Layer").doc("none: this layer"),
        ranged("maxDisplacementTimeSec", "Max Displacement Time (sec)", 1.0, -30.0, 30.0),
        ranged("timeResolutionFps", "Time Resolution (fps)", 60.0, 1.0, 999.0),
        flag("stretchMapToFit", "Stretch Map To Fit", true),
    ]
);
pub const TIME_DISPLACEMENT: Spec = Spec::new(&TIME_DISPLACEMENT_DEF, CATEGORY, time_displacement);

fn time_displacement(ctx: &Ctx, b: Buf) -> Buf {
    let most = ctx.num("maxDisplacementTimeSec", 1.0);
    let res = ctx.num("timeResolutionFps", 60.0).clamp(1.0, 999.0);
    let t = ctx.time.seconds();
    let Some((grid, mut now)) = frames(ctx, &[t]) else { return b };
    let cur = Buf { img: now.pop().unwrap_or_else(|| Image::new(1, 1)), ..grid };
    let map = ctx.layer_over("timeDisplacementLayer", &cur, Take::All, ctx.flag("stretchMapToFit")).unwrap_or_else(|| cur.img.clone());
    // each pixel's offset, in steps of the time resolution (at most 64 different moments)
    let span = 2.0 * most.abs() * res;
    let step = if span > 63.0 { 2.0 * most.abs() / 63.0 } else { 1.0 / res };
    let index: Vec<i32> = map
        .px
        .iter()
        .map(|p| {
            let (c, a) = unpremul(*p);
            let l = if a > 0.0 { luma(c) } else { 0.5 };
            ((l as f64 - 0.5) * 2.0 * most / step).round() as i32
        })
        .collect();
    let mut keys = index.clone();
    keys.sort_unstable();
    keys.dedup();
    if keys == [0] {
        return cur;
    }
    let times: Vec<f64> = keys.iter().map(|k| t + *k as f64 * step).collect();
    let Some((g2, pics)) = frames(ctx, &times) else { return cur };
    let w = cur.img.width;
    let mut out = cur.img.clone();
    map_px(&mut out, |x, y, _| {
        let k = keys.binary_search(&index[y * w + x]).unwrap_or(0);
        let q = g2.pixel(cur.origin + veymelo_geom::vec2(x as f64 + 0.5, y as f64 + 0.5) / cur.scale);
        pics[k].sample(Sampling::Nearest, q.x, q.y)
    });
    Buf { img: out, ..cur }
}

/// How Pixel Motion finds and uses motion: from the Tuning settings.
#[derive(Clone, Copy, Debug)]
struct Motion {
    step: usize,
    window: usize,
    global: f32,
    local: f32,
    passes: u32,
    one_image: bool,
    even_light: bool,
    extreme: bool,
    error: f32,
    weights: [f32; 3],
    crops: [usize; 4],
}

impl Motion {
    fn of(ctx: &Ctx, scale: f64, detail: f64) -> Motion {
        let px = |id: &str| (ctx.num(id, 0.0).max(0.0) * scale).round() as usize;
        Motion {
            // detail 0: a vector every 32 pixels; 100: every 2
            step: (32.0 * (1.0f64 / 16.0).powf(detail.clamp(0.0, 100.0) / 100.0) * scale).round().max(2.0) as usize,
            window: (ctx.num("blockSize", 8.0).max(4.0) * scale).round().max(4.0) as usize,
            global: ctx.num("globalSmoothness", 0.5).clamp(0.0, 1.0) as f32,
            local: ctx.num("localSmoothness", 0.5).clamp(0.0, 1.0) as f32,
            passes: ctx.num("smoothingIterations", 20.0).clamp(0.0, 200.0) as u32,
            one_image: ctx.flag("buildFromOneImage"),
            even_light: ctx.flag("correctLuminanceChanges"),
            extreme: ctx.choice("filtering") == 1,
            error: (ctx.num("errorThreshold", 10.0) / 100.0).clamp(0.0, 1.0) as f32,
            weights: [ctx.num("redWeighting", 0.3), ctx.num("greenWeighting", 0.59), ctx.num("blueWeighting", 0.11)].map(|v| v.max(0.0) as f32),
            crops: [px("leftCrop"), px("rightCrop"), px("topCrop"), px("bottomCrop")],
        }
    }

    /// A frame with its cropped edges filled from the nearest kept pixels (edge garbage must
    /// not move the motion).
    fn cropped(&self, img: &Image) -> Image {
        let [l, r, t, bm] = self.crops;
        if l + r + t + bm == 0 {
            return img.clone();
        }
        let (w, h) = (img.width as i64, img.height as i64);
        let (x0, x1, y0, y1) = ((l as i64).min(w - 1), (w - 1 - r as i64).max(0), (t as i64).min(h - 1), (h - 1 - bm as i64).max(0));
        let mut out = img.clone();
        map_px(&mut out, |x, y, _| img.get((x as i64).clamp(x0, x1.max(x0)), (y as i64).clamp(y0, y1.max(y0))));
        out
    }

    /// The weighted grey the motion is found on.
    fn grey(&self, img: &Image, gain: f32) -> Plane {
        let sum = (self.weights[0] + self.weights[1] + self.weights[2]).max(1e-6);
        Plane { w: img.width, h: img.height, v: img.px.iter().map(|p| (p[0] * self.weights[0] + p[1] * self.weights[1] + p[2] * self.weights[2]) / sum * gain).collect() }
    }

    /// The moves between `a` and `b` both ways, one for every pixel: found on blocks, smoothed,
    /// then each pixel given the nearby move that fits it. A pixel of one frame hidden in the
    /// other (nothing from the other lands on it: it is about to be covered, or was just
    /// uncovered) has no true move between them, so it is given the move it makes on its other
    /// side instead (from `before` to `a`, from `b` to `after`), as if it went on moving so.
    fn between(&self, before: &Image, a: &Image, b: &Image, after: &Image, reach: usize) -> Moves {
        let a = self.cropped(a);
        let ga = self.grey(&a, 1.0);
        // a frame as grey (to find blocks' moves on) and as colours (to give each pixel its own),
        // brightened to match `a` if asked
        let seen = |img: &Image| {
            let img = self.cropped(img);
            let gain = if self.even_light {
                let mean = |p: &Plane| p.v.iter().sum::<f32>() / p.v.len().max(1) as f32;
                let mb = mean(&self.grey(&img, 1.0));
                if mb > 1e-4 { mean(&ga) / mb } else { 1.0 }
            } else {
                1.0
            };
            (self.grey(&img, gain), channels(&img, gain))
        };
        let (pa, pb) = ((ga.clone(), channels(&a, 1.0)), seen(b));
        let one = |p: &(Plane, [Plane; 3]), q: &(Plane, [Plane; 3]), only: Option<&[bool]>| {
            let raw = flow::measure(&p.0, &q.0, self.step, self.window, reach);
            let (pc, qc) = (p.1.each_ref(), q.1.each_ref());
            flow::per_pixel(&pc, &qc, &raw, &self.smoothed(&raw), self.window.div_ceil(8), only)
        };
        let (mut fwd, mut back) = (one(&pa, &pb, None), one(&pb, &pa, None));
        let (hidden_in_b, hidden_in_a) = (unseen(&back), unseen(&fwd));
        if hidden_in_b.contains(&true) {
            let earlier = one(&pa, &seen(before), Some(&hidden_in_b));
            for (k, h) in hidden_in_b.iter().enumerate() {
                if *h {
                    let e = earlier.v[k];
                    (fwd.v[k], fwd.sure[k]) = ([-e[0], -e[1]], earlier.sure[k]);
                }
            }
        }
        if hidden_in_a.contains(&true) {
            let later = one(&pb, &seen(after), Some(&hidden_in_a));
            for (k, h) in hidden_in_a.iter().enumerate() {
                if *h {
                    let l = later.v[k];
                    (back.v[k], back.sure[k]) = ([-l[0], -l[1]], later.sure[k]);
                }
            }
        }
        Moves { fwd, back }
    }

    /// Block moves made smooth: each pulled toward its neighbours' (local, as many times as the
    /// iterations; then toward a wider average once, global), across and down apart. The
    /// neighbours count by how sure they are, and a block holds to its own move by how sure it
    /// is, so what a flat block cannot say comes from the edges and texture round it.
    fn smoothed(&self, raw: &Flow) -> Flow {
        let mut f = raw.clone();
        let (c, r) = (f.cols, f.rows);
        let pass = |v: &[[f32; 2]], rad: i64, k: f32| -> Vec<[f32; 2]> {
            (0..c * r)
                .map(|i| {
                    let (x, y) = ((i % c) as i64, (i / c) as i64);
                    let mut out = v[i];
                    for (a, o) in out.iter_mut().enumerate() {
                        let (mut s, mut n) = (0f32, 0f32);
                        for dy in -rad..=rad {
                            for dx in -rad..=rad {
                                let (nx, ny) = (x + dx, y + dy);
                                if nx >= 0 && ny >= 0 && (nx as usize) < c && (ny as usize) < r {
                                    let j = ny as usize * c + nx as usize;
                                    let w = raw.sure[j][a] + 0.02;
                                    s += v[j][a] * w;
                                    n += w;
                                }
                            }
                        }
                        *o += (s / n - *o) * k * (1.0 - 0.9 * raw.sure[i][a]);
                    }
                    out
                })
                .collect()
        };
        if self.local > 0.0 {
            for _ in 0..self.passes {
                f.v = pass(&f.v, 1, self.local);
            }
        }
        if self.global > 0.0 {
            f.v = pass(&f.v, 3, self.global);
        }
        f
    }

    /// The frame `w` of the way from `a` to `b`. Each pixel takes, of the moves landing on it
    /// from either frame and the frames' own moves there, the one under which it is seen in both
    /// frames (each frame's move where it comes from agrees) and looks most alike in each, and is
    /// mixed from the two along it. A pixel seen in only one frame (uncovered or covered by
    /// something moving) is taken from that one. Where the two frames along the best move still
    /// disagree, they are mixed as they are instead: the higher the error threshold, the less
    /// they need disagree (at 0 never, at 100 everywhere).
    fn in_between(&self, a: &Image, b: &Image, w: f32, m: &Moves) -> Image {
        const SLOTS: usize = 4;
        let (a, b) = (self.cropped(a), self.cropped(b));
        let (wi, hi) = (a.width, a.height);
        let how = if self.extreme { Sampling::Bicubic } else { Sampling::Bilinear };
        let (wd, ww, hh) = (w as f64, wi as f64, hi as f64);
        let unlike = |p: [f32; 2], q: [f32; 2]| (p[0] - q[0]).abs() + (p[1] - q[1]).abs();
        // the moves landing on each pixel of the frame between, from both frames' pixels
        let mut landed = vec![([[0f32; 2]; SLOTS], 0u8); wi * hi];
        let mut land = |x: f64, y: f64, v: [f32; 2]| {
            let (fx, fy) = ((x - 0.5).floor() as i64, (y - 0.5).floor() as i64);
            for (i, j) in [(fx, fy), (fx + 1, fy), (fx, fy + 1), (fx + 1, fy + 1)] {
                if i >= 0 && j >= 0 && (i as usize) < wi && (j as usize) < hi {
                    let (slots, n) = &mut landed[j as usize * wi + i as usize];
                    if (*n as usize) < SLOTS && slots[..*n as usize].iter().all(|q| unlike(*q, v) > 0.25) {
                        slots[*n as usize] = v;
                        *n += 1;
                    }
                }
            }
        };
        for y in 0..hi {
            for x in 0..wi {
                let (cx, cy, k) = (x as f64 + 0.5, y as f64 + 0.5, y * wi + x);
                let v = m.fwd.v[k];
                land(cx + wd * v[0] as f64, cy + wd * v[1] as f64, v);
                let u = m.back.v[k];
                land(cx + (1.0 - wd) * u[0] as f64, cy + (1.0 - wd) * u[1] as f64, [-u[0], -u[1]]);
            }
        }
        let held = |img: &Image, x: f64, y: f64| img.sample(how, x.clamp(0.5, ww - 0.5), y.clamp(0.5, hh - 0.5));
        let within = |x: f64, y: f64| x >= 0.0 && y >= 0.0 && x <= ww && y <= hh;
        // a move fits a pixel's when near it: nearer where the picture says the pixel's move, less
        // so where that is a guess (flat)
        let fits = |f: [f32; 2], v: [f32; 2], sure: bool| (f[0] - v[0]).hypot(f[1] - v[1]) <= 0.75 + if sure { 0.15 } else { 0.4 } * v[0].hypot(v[1]);
        let mix = |p: Px, q: Px| [0, 1, 2, 3].map(|c| p[c] + (q[c] - p[c]) * w);
        let mut out = Image::new(wi, hi);
        map_px(&mut out, |x, y, _| {
            let (cx, cy, k) = (x as f64 + 0.5, y as f64 + 0.5, y * wi + x);
            let u = m.back.v[k];
            let own = [m.fwd.v[k], [-u[0], -u[1]]];
            let (slots, n) = &landed[k];
            // (cost, pixel, the two frames' disagreement when seen in both)
            let mut best = (f32::MAX, [0f32; 4], None);
            for v in slots[..*n as usize].iter().chain(&own) {
                let (vx, vy) = (v[0] as f64, v[1] as f64);
                let (ax, ay, bx, by) = (cx - wd * vx, cy - wd * vy, cx + (1.0 - wd) * vx, cy + (1.0 - wd) * vy);
                // seen there in each frame: inside it, and moving so (where the frame says)
                let in_a = within(ax, ay) && fits(m.fwd.at(ax, ay), *v, m.fwd.sure_at(ax, ay));
                let back = m.back.at(bx, by);
                let in_b = within(bx, by) && fits([-back[0], -back[1]], *v, m.back.sure_at(bx, by));
                let (pa, pb) = (held(&a, ax, ay), held(&b, bx, by));
                let apart = (0..4).map(|c| (pa[c] - pb[c]).abs()).fold(0f32, f32::max);
                let near = if w < 0.5 { pa } else { pb };
                let (cost, px, both) = match (in_a, in_b) {
                    (true, true) => (apart, if self.one_image { near } else { mix(pa, pb) }, Some(apart)),
                    (true, false) => (0.2, pa, None),
                    (false, true) => (0.2, pb, None),
                    (false, false) => (1.0 + apart, if self.one_image { near } else { mix(pa, pb) }, Some(apart)),
                };
                if cost < best.0 {
                    best = (cost, px, both);
                }
            }
            match best.2 {
                Some(apart) if !self.one_image && self.error > 0.0 && apart >= 1.0 - self.error => mix(a.px[k], b.px[k]),
                _ => best.1,
            }
        });
        out
    }
}

/// A picture's red, green and blue, each times `gain`.
fn channels(img: &Image, gain: f32) -> [Plane; 3] {
    [0, 1, 2].map(|c| Plane { w: img.width, h: img.height, v: img.px.iter().map(|p| p[c] * gain).collect() })
}

/// The pixels of a frame nothing lands on when each pixel of another moves by `f` into it (each
/// lands on the pixel it falls in, and on the next one too within a quarter of a pixel of it).
fn unseen(f: &Field) -> Vec<bool> {
    let mut hit = vec![false; f.w * f.h];
    for (k, v) in f.v.iter().enumerate() {
        let (x, y) = ((k % f.w) as f64 + 0.5 + v[0] as f64, (k / f.w) as f64 + 0.5 + v[1] as f64);
        for (i, j) in [(x - 0.25, y - 0.25), (x + 0.25, y - 0.25), (x - 0.25, y + 0.25), (x + 0.25, y + 0.25)] {
            let (i, j) = (i.floor() as i64, j.floor() as i64);
            if i >= 0 && j >= 0 && (i as usize) < f.w && (j as usize) < f.h {
                hit[j as usize * f.w + i as usize] = true;
            }
        }
    }
    hit.into_iter().map(|h| !h).collect()
}

/// The moves between two frames, both ways.
struct Moves {
    fwd: Field,
    back: Field,
}

/// Moves already found, by the source frame they start from and what they were found on (0:
/// the whole picture or the warp layer, 1: the subject, 2: what is behind it).
#[derive(Default)]
struct Kept(Vec<((i64, u8), Rc<Moves>)>);

impl Kept {
    fn get(&mut self, key: (i64, u8), find: impl FnOnce() -> Moves) -> Rc<Moves> {
        if let Some((_, m)) = self.0.iter().find(|(k, _)| *k == key) {
            return m.clone();
        }
        let m = Rc::new(find());
        self.0.push((key, m.clone()));
        m
    }
}

/// How far a move between frames is looked for: a sixth of the picture, 16 to 96 pixels.
fn reach_for(grid: &Buf) -> usize {
    (grid.img.width.max(grid.img.height) / 6).clamp(16, 96)
}

/// The frame at or before source time `s` (seconds) for frames `fd` long, and how far past it.
fn split(s: f64, fd: f64) -> (f64, f32) {
    let f = s / fd;
    let f0 = (f + 1e-6).floor();
    (f0, ((f - f0) as f32).max(0.0))
}

const MATTE_CHANNELS: &[&str] = &["alpha", "invertedAlpha", "luminance", "invertedLuminance"];
const SHOW: &[&str] = &["normal", "foregroundOnly", "backgroundOnly", "matte"];

effect!(
    TIMEWARP_DEF,
    "timewarp",
    "Timewarp",
    "the layer slowed, sped up or frozen, frames in between built by blending or by following the motion",
    [
        choice("method", "Method", &["wholeFrames", "frameMix", "pixelMotion"], 2),
        choice("adjustTimeBy", "Adjust Time By", &["speed", "sourceFrame"], 0),
        ranged("speed", "Speed", 50.0, -10_000.0, 10_000.0).doc("percent of normal"),
        ranged("sourceFrame", "Source Frame", 0.0, -100_000.0, 100_000.0).doc("animate it to choose the frame shown"),
        pct("vectorDetail", "Vector Detail", 20.0, 0.0, 100.0),
        ranged("globalSmoothness", "Global Smoothness", 0.5, 0.0, 1.0),
        ranged("localSmoothness", "Local Smoothness", 0.5, 0.0, 1.0),
        ranged("smoothingIterations", "Smoothing Iterations", 20.0, 0.0, 200.0),
        flag("buildFromOneImage", "Build From One Image", false),
        flag("correctLuminanceChanges", "Correct Luminance Changes", false),
        choice("filtering", "Filtering", &["normal", "extreme"], 0),
        pct("errorThreshold", "Error Threshold", 10.0, 0.0, 100.0).doc("higher: fewer pixels moved along the motion, more mixed as they are"),
        ranged("blockSize", "Block Size", 8.0, 4.0, 64.0),
        ranged("redWeighting", "Red Weighting", 0.3, 0.0, 1.0),
        ranged("greenWeighting", "Green Weighting", 0.59, 0.0, 1.0),
        ranged("blueWeighting", "Blue Weighting", 0.11, 0.0, 1.0),
        flag("enableMotionBlur", "Enable Motion Blur", false),
        choice("shutterControl", "Shutter Control", &["automatic", "manual"], 0),
        ranged("shutterAngle", "Shutter Angle", 180.0, 0.0, 720.0),
        ranged("shutterSamples", "Shutter Samples", 5.0, 1.0, 64.0),
        layer("matteLayer", "Matte Layer").doc("splits the subject from what is behind, each moving on its own"),
        choice("matteChannel", "Matte Channel", MATTE_CHANNELS, 0),
        layer("warpLayer", "Warp Layer").doc("the motion found on this layer instead"),
        choice("show", "Show", SHOW, 0),
        ranged("leftCrop", "Left Crop", 0.0, 0.0, 4000.0),
        ranged("rightCrop", "Right Crop", 0.0, 0.0, 4000.0),
        ranged("bottomCrop", "Bottom Crop", 0.0, 0.0, 4000.0),
        ranged("topCrop", "Top Crop", 0.0, 0.0, 4000.0),
    ]
);
pub const TIMEWARP: Spec = Spec::new(&TIMEWARP_DEF, CATEGORY, timewarp);

/// A matte (1: the subject) from a picture.
fn matte_of(img: &Image, ch: u32) -> Vec<f32> {
    img.px
        .iter()
        .map(|p| {
            let (c, a) = unpremul(*p);
            let l = luma(c) * a;
            match ch {
                1 => 1.0 - a,
                2 => l,
                3 => 1.0 - l,
                _ => a,
            }
            .clamp(0.0, 1.0)
        })
        .collect()
}

fn masked(img: &Image, m: &[f32], outside: bool) -> Image {
    let mut out = img.clone();
    for (p, k) in out.px.iter_mut().zip(m) {
        let k = if outside { 1.0 - k } else { *k };
        *p = p.map(|v| v * k);
    }
    out
}

fn grey_of(m: &[f32], w: usize, h: usize) -> Image {
    let mut out = Image::new(w, h);
    for (p, k) in out.px.iter_mut().zip(m) {
        *p = [*k, *k, *k, 1.0];
    }
    out
}

/// Another layer at this layer's times `times`, laid on `grid` (centred), if named.
fn other_at(ctx: &Ctx, id: &str, times: &[f64], grid: &Buf) -> Option<Vec<Image>> {
    let lid = ctx.value_in(ctx.effect, id).and_then(|v| v.as_layer())?;
    let host = ctx.host?;
    let now = ctx.time.seconds();
    let r = ctx.bounds;
    Some(
        times
            .iter()
            .map(|t| match host.layer_at(lid, Tick::from_seconds(t - now), Take::All) {
                Some((other, rect)) => {
                    let mut out = Image::new(grid.img.width, grid.img.height);
                    map_px(&mut out, |x, y, _| {
                        let p = grid.origin + veymelo_geom::vec2(x as f64 + 0.5, y as f64 + 0.5) / grid.scale;
                        let q = other.pixel(p - r.center() + rect.center());
                        other.img.sample(Sampling::Bilinear, q.x, q.y)
                    });
                    out
                }
                None => Image::new(grid.img.width, grid.img.height),
            })
            .collect(),
    )
}

fn timewarp(ctx: &Ctx, b: Buf) -> Buf {
    let fd = ctx.frame.seconds().max(1e-6);
    let by_frame = ctx.choice("adjustTimeBy") == 1;
    let speed = ctx.num("speed", 50.0);
    let t = ctx.time.seconds();
    let source = if by_frame { ctx.num("sourceFrame", 0.0) * fd } else { t * speed / 100.0 };
    // the moments shown, across the shutter if blurred, with their weights
    let mut moments: Vec<(f64, f32)> = Vec::new();
    if ctx.flag("enableMotionBlur") {
        let angle = if ctx.choice("shutterControl") == 1 { ctx.num("shutterAngle", 180.0) } else { 180.0 };
        let n = ctx.num("shutterSamples", 5.0).round().clamp(1.0, 64.0) as usize;
        let rate = if by_frame { 1.0 } else { speed / 100.0 };
        let span = angle / 360.0 * rate * fd;
        for i in 0..n {
            moments.push((source + if n > 1 { span * i as f64 / (n - 1) as f64 } else { 0.0 }, 1.0 / n as f32));
        }
    } else {
        moments.push((source, 1.0));
    }
    let method = ctx.choice("method");
    // each moment's frames either side (and for pixel motion, the next out each way)
    let mut times: Vec<f64> = Vec::new();
    for (s, _) in &moments {
        let (f0, _) = split(*s, fd);
        let reach = if method == 2 { -1..=2 } else { 0..=1 };
        times.extend(reach.map(|k| (f0 + k as f64) * fd));
    }
    let Some((grid, pics)) = frames(ctx, &times) else { return b };
    let m = Motion::of(ctx, grid.scale, ctx.num("vectorDetail", 20.0));
    let mattes: Option<Vec<Vec<f32>>> = other_at(ctx, "matteLayer", &times, &grid).map(|v| v.iter().map(|i| matte_of(i, ctx.choice("matteChannel"))).collect());
    let warps = other_at(ctx, "warpLayer", &times, &grid);
    let show = ctx.choice("show");
    let (w, h) = (grid.img.width, grid.img.height);
    let index = |s: f64| times.iter().position(|q| (q - s).abs() < 1e-9).unwrap_or(0);
    let reach = reach_for(&grid);
    let mut kept = Kept::default();
    let shown: Vec<Image> = moments
        .iter()
        .map(|(s, _)| {
            let (f0, part) = split(*s, fd);
            let at = [-1.0, 0.0, 1.0, 2.0].map(|k| index((f0 + k) * fd));
            let (i0, i1) = (at[1], at[2]);
            // a frame as shown: all of it, the subject, what is behind it, or the matte
            let pick = |i: usize| -> Image {
                match (show, &mattes) {
                    (1, Some(mt)) => masked(&pics[i], &mt[i], false),
                    (2, Some(mt)) => masked(&pics[i], &mt[i], true),
                    (3, Some(mt)) => grey_of(&mt[i], w, h),
                    _ => pics[i].clone(),
                }
            };
            if method == 0 || part < 1e-4 || (method == 1 && m.one_image) {
                return if method != 0 && part >= 0.5 { pick(i1) } else { pick(i0) };
            }
            if method == 1 {
                return mean(&[pick(i0), pick(i1)], &[1.0 - part, part]);
            }
            // pixel motion: on the warp layer's motion if any, else the layer's own
            let key = f0 as i64;
            let found = |imgs: [&Image; 4]| m.between(imgs[0], imgs[1], imgs[2], imgs[3], reach);
            let whole = |kept: &mut Kept| match &warps {
                Some(v) => kept.get((key, 0), || found(at.map(|i| &v[i]))),
                None => kept.get((key, 0), || found(at.map(|i| &pics[i]))),
            };
            let Some(mt) = &mattes else { return m.in_between(&pics[i0], &pics[i1], part, &whole(&mut kept)) };
            // the subject and what is behind it, each on its own motion
            let fronts = at.map(|i| masked(&pics[i], &mt[i], false));
            let backs = at.map(|i| masked(&pics[i], &mt[i], true));
            let (mf, mg) = if warps.is_some() {
                let w = whole(&mut kept);
                (w.clone(), w)
            } else {
                (kept.get((key, 1), || found([&fronts[0], &fronts[1], &fronts[2], &fronts[3]])), kept.get((key, 2), || found([&backs[0], &backs[1], &backs[2], &backs[3]])))
            };
            let (front, back) = (m.in_between(&fronts[1], &fronts[2], part, &mf), m.in_between(&backs[1], &backs[2], part, &mg));
            match show {
                1 => front,
                2 => back,
                3 => grey_of(&front.px.iter().map(|p| p[3]).collect::<Vec<_>>(), w, h),
                _ => {
                    let mut o = back;
                    for (p, q) in o.px.iter_mut().zip(&front.px) {
                        let k = 1.0 - q[3];
                        *p = [0, 1, 2, 3].map(|c| q[c] + p[c] * k);
                    }
                    o
                }
            }
        })
        .collect();
    let weights: Vec<f32> = moments.iter().map(|(_, k)| *k).collect();
    let img = if shown.len() == 1 { shown.into_iter().next().expect("one") } else { mean(&shown, &weights) };
    Buf { img, ..grid }
}

effect!(
    FORCE_BLUR_DEF,
    "forceMotionBlur",
    "Force Motion Blur",
    "motion blur made by mixing the layer over the shutter, whatever moves it (effects, nested compositions)",
    [
        ranged("motionBlurLevels", "Motion Blur Levels", 8.0, 2.0, 64.0),
        flag("overrideShutterAngle", "Override Shutter Angle", true),
        ranged("shutterAngle", "Shutter Angle", 180.0, 0.0, 3600.0),
        choice("nativeMotionBlur", "Native Motion Blur", &["off", "on"], 0),
    ]
);
pub const FORCE_BLUR: Spec = Spec::new(&FORCE_BLUR_DEF, CATEGORY, force_motion_blur);

fn force_motion_blur(ctx: &Ctx, b: Buf) -> Buf {
    let n = ctx.num("motionBlurLevels", 8.0).round().clamp(2.0, 64.0) as usize;
    let angle = if ctx.flag("overrideShutterAngle") { ctx.num("shutterAngle", 180.0) } else { ctx.shutter.map_or(180.0, |s| s.0) };
    let span = angle.clamp(0.0, 3600.0) / 360.0 * ctx.frame.seconds();
    let t = ctx.time.seconds();
    let times: Vec<f64> = (0..n).map(|i| t + span * i as f64 / n as f64).collect();
    averaged(ctx, b, &times)
}

effect!(
    WIDE_TIME_DEF,
    "wideTime",
    "Wide Time",
    "the layer mixed with its frames just before and after: a long, smeared exposure",
    [ranged("forwardSteps", "Forward Steps", 3.0, 0.0, 45.0), ranged("backwardSteps", "Backward Steps", 3.0, 0.0, 45.0), choice("nativeMotionBlur", "Native Motion Blur", &["off", "on"], 0),]
);
pub const WIDE_TIME: Spec = Spec::new(&WIDE_TIME_DEF, CATEGORY, wide_time);

fn wide_time(ctx: &Ctx, b: Buf) -> Buf {
    let (fwd, back) = (ctx.num("forwardSteps", 3.0).round().clamp(0.0, 45.0) as i64, ctx.num("backwardSteps", 3.0).round().clamp(0.0, 45.0) as i64);
    let (t, fd) = (ctx.time.seconds(), ctx.frame.seconds());
    let times: Vec<f64> = (-back..=fwd).map(|k| t + k as f64 * fd).collect();
    averaged(ctx, b, &times)
}

effect!(
    PIXEL_BLUR_DEF,
    "pixelMotionBlur",
    "Pixel Motion Blur",
    "motion blur from how each part of the picture moves between frames (footage, not keyframes)",
    [
        choice("shutterControl", "Shutter Control", &["automatic", "manual"], 0),
        ranged("shutterAngle", "Shutter Angle", 180.0, 0.0, 720.0),
        ranged("shutterSamples", "Shutter Samples", 16.0, 1.0, 64.0),
        pct("vectorDetail", "Vector Detail", 20.0, 0.0, 100.0),
    ]
);
pub const PIXEL_BLUR: Spec = Spec::new(&PIXEL_BLUR_DEF, CATEGORY, pixel_motion_blur);

fn pixel_motion_blur(ctx: &Ctx, b: Buf) -> Buf {
    let angle = if ctx.choice("shutterControl") == 1 { ctx.num("shutterAngle", 180.0) } else { 180.0 };
    let n = ctx.num("shutterSamples", 16.0).round().clamp(1.0, 64.0) as usize;
    let fd = ctx.frame.seconds().max(1e-6);
    let span = angle.clamp(0.0, 720.0) / 360.0 * fd;
    let t = ctx.time.seconds();
    let samples: Vec<f64> = (0..n).map(|i| t - span * 0.5 + if n > 1 { span * i as f64 / (n - 1) as f64 } else { span * 0.5 }).collect();
    let mut times = Vec::new();
    for s in &samples {
        let (f0, _) = split(*s, fd);
        times.extend((-1..=2).map(|k| (f0 + k as f64) * fd));
    }
    let Some((grid, pics)) = frames(ctx, &times) else { return b };
    let m = Motion::of(ctx, grid.scale, ctx.num("vectorDetail", 20.0));
    let index = |s: f64| times.iter().position(|q| (q - s).abs() < 1e-9).unwrap_or(0);
    let reach = reach_for(&grid);
    let mut kept = Kept::default();
    let shown: Vec<Image> = samples
        .iter()
        .map(|s| {
            let (f0, part) = split(*s, fd);
            let at = [-1.0, 0.0, 1.0, 2.0].map(|k| index((f0 + k) * fd));
            let (i0, i1) = (at[1], at[2]);
            if part < 1e-4 {
                return pics[i0].clone();
            }
            let moves = kept.get((f0 as i64, 0), || m.between(&pics[at[0]], &pics[i0], &pics[i1], &pics[at[3]], reach));
            m.in_between(&pics[i0], &pics[i1], part, &moves)
        })
        .collect();
    Buf { img: mean(&shown, &vec![1.0 / n as f32; n]), ..grid }
}

/// Every effect here.
pub const ALL: [Spec; 8] = [ECHO, POSTERIZE_TIME, TIME_DIFF, TIME_DISPLACEMENT, TIMEWARP, FORCE_BLUR, WIDE_TIME, PIXEL_BLUR];

#[cfg(test)]
mod tests;

/// The frame `w` (0 to 1) of the way from `a` to `b` by how things move between them,
/// `before` and `after` the frames either side — how a footage layer's frames are blended by
/// pixel motion (Timewarp's motion at its usual settings).
pub fn tween(before: &Image, a: &Image, b: &Image, after: &Image, w: f32) -> Image {
    let m = Motion {
        step: (32.0 * (1.0f64 / 16.0).powf(0.2)).round() as usize,
        window: 8,
        global: 0.5,
        local: 0.5,
        passes: 20,
        one_image: false,
        even_light: false,
        extreme: false,
        error: 0.1,
        weights: [0.3, 0.59, 0.11],
        crops: [0; 4],
    };
    let reach = (a.width.max(a.height) / 6).clamp(16, 96);
    let moves = m.between(before, a, b, after, reach);
    m.in_between(a, b, w, &moves)
}
