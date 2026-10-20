//! Keyframe assistants: the Wiggler (keys made to wander, smooth or jagged, the same each
//! time for a seed), the Smoother (a run of keys made the fewest that keep to the curve), Motion
//! Sketch (a path drawn as position keys), keys moved and scaled together in time and value,
//! Exponential Scale (a zoom that looks even), and layers put one after another or staggered.

use veymelo_json::Value as Json;
use veymelo_keyframe::{Interp, Keyframe, Value, evaluate};
use veymelo_project::Layer;
use veymelo_time::{FrameRate, Tick};

use super::key::{chosen, keys_json, store, target};
use super::{COMP, Command, LAYER, Run, comp_id, find_layer};
use crate::{Args, Ctx, Error, Param, Result, Ty};

const PATH: Param = Param::need("path", Ty::Text, "the property, like transform/position");
const TIMES: Param = Param::opt("times", Ty::List, "the keys, by their times (default: all; the first and last given bound the run)");
const LAYERS: Param = Param::need("layers", Ty::List, "the layers, in the order they go (ids, names or \"#n\")");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "key.wiggle",
            label: "Wiggler",
            summary: "Make a property wander between two of its keys: new keys every 1 / frequency seconds, each off by up to `magnitude` (in the property's units) — smooth (eased, gradual) or jagged (each its own); every number the same way, each on its own, or one. On a position, `temporal` makes its speed along its path wander instead. The same seed gives the same keys.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    TIMES,
                    Param::opt("frequency", Ty::Number, "keys a second (default 5)"),
                    Param::opt("magnitude", Ty::Number, "how far off, in the property's units (temporal: % of the time between keys) (default 1)"),
                    Param::opt("noise", Ty::Choice(&["smooth", "jagged"]), "default smooth"),
                    Param::opt("dimensions", Ty::Choice(&["independent", "same", "one"]), "default independent"),
                    Param::opt("dimension", Ty::Whole, "which number, with dimensions one (0 is x)"),
                    Param::opt("apply", Ty::Choice(&["spatial", "temporal"]), "default spatial"),
                    Param::opt("seed", Ty::Whole, "default 0"),
                ]
            },
            returns: "{before, after, keys}",
            run: Run::Edit(wiggle),
        },
        Command {
            id: "key.smooth",
            label: "Smoother",
            summary: "Make a run of keys (three or more) the fewest smooth keys that keep within `tolerance` of the curve they made, sampled every frame.",
            params: const { &[COMP, LAYER, PATH, TIMES, Param::opt("tolerance", Ty::Number, "how far it may stray, in the property's units (default 1)")] },
            returns: "{before, after, keys}",
            run: Run::Edit(smooth),
        },
        Command {
            id: "motion.sketch",
            label: "Motion Sketch",
            summary: "Turn a path drawn over time into a layer's position keys, a key a frame, then smoothed: points [[t, x, y]…] (t: seconds from the drawing's start) or [[x, y]…] (one a frame), in composition pixels.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("points", Ty::List, "[[t, x, y]…] or [[x, y]…]"),
                    Param::opt("start", Ty::Time, "where the keys start (default: the view's time)"),
                    Param::opt("captureSpeed", Ty::Number, "% (100: plays back as fast as drawn; 200: half as fast)"),
                    Param::opt("smoothing", Ty::Number, "pixels the smoothed path may stray (0: a key every frame, as drawn) (default 1)"),
                ]
            },
            returns: "{keys}",
            run: Run::Edit(sketch),
        },
        Command {
            id: "key.transform",
            label: "Transform Keyframes",
            summary: "Move and scale keys together: their times scaled about `timeAnchor` and moved by `timeOffset` (landing on frames), their values scaled about `valueAnchor` and moved by `valueOffset` (every number, or only `dim`). Moved keys win over others at the same time.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    TIMES,
                    Param::opt("timeScale", Ty::Number, "1 as they are, 2 twice as far apart"),
                    Param::opt("timeAnchor", Ty::Time, "the time that stays (default: the first key's)"),
                    Param::opt("timeOffset", Ty::Number, "seconds to move by"),
                    Param::opt("valueScale", Ty::Number, "1 as they are"),
                    Param::opt("valueAnchor", Ty::Number, "the value that stays (default 0)"),
                    Param::opt("valueOffset", Ty::Number, "to add"),
                    Param::opt("dim", Ty::Whole, "only this number of the value (0 is x)"),
                ]
            },
            returns: "{keys}",
            run: Run::Edit(transform),
        },
        Command {
            id: "key.exponentialScale",
            label: "Exponential Scale",
            summary: "Make a scale between two keys grow by the same factor every frame (a key each frame), so a zoom looks even instead of slowing down as it gets bigger.",
            params: const { &[COMP, LAYER, Param::opt("path", Ty::Text, "default transform/scale"), TIMES] },
            returns: "{keys}",
            run: Run::Edit(exponential),
        },
        Command {
            id: "layer.sequence",
            label: "Sequence Layers",
            summary: "Put layers one after another in the order given: each starts where the one before ends, or overlapping it by `overlap` seconds, with a fade in (dissolveFront) or a fade in and out (crossDissolve) across each overlap.",
            params: const { &[COMP, LAYERS, Param::opt("overlap", Ty::Number, "seconds each overlaps the one before (default 0)"), Param::opt("transition", Ty::Choice(&["off", "dissolveFront", "crossDissolve"]), "default off")] },
            returns: "{layers: [{layer, in, out}]}",
            run: Run::Edit(sequence),
        },
        Command {
            id: "layer.stagger",
            label: "Stagger Layers",
            summary: "Move layers in time by steps in the order given: the first stays, each next one `step` seconds later than the one before (or spread so the last moves by `total`). With `path`, only that property's keys move, the layers staying.",
            params: const { &[COMP, LAYERS, Param::opt("step", Ty::Number, "seconds between each and the next"), Param::opt("total", Ty::Number, "seconds the last moves, the others spread evenly"), Param::opt("path", Ty::Text, "move only this property's keys")] },
            returns: "{layers: [{layer, offset}]}",
            run: Run::Edit(stagger),
        },
    ]
}

// ---------------------------------------------------------------- noise

fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A number from −1 to 1, the same for the same seed, stream and index.
fn rnd(seed: u64, stream: u64, i: i64) -> f64 {
    let h = mix64(seed ^ mix64(stream.wrapping_mul(0x1000_0000_01B3) ^ i as u64));
    (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// The `k`th wander: smooth (half as many points, eased between) or jagged.
fn wander(smooth: bool, seed: u64, stream: u64, k: i64) -> f64 {
    if !smooth {
        return rnd(seed, stream, k);
    }
    let (i, f) = (k.div_euclid(2), k.rem_euclid(2) as f64 / 2.0);
    let w = (1.0 - (f * std::f64::consts::PI).cos()) / 2.0;
    rnd(seed, stream, i) * (1.0 - w) + rnd(seed, stream, i + 1) * w
}

/// The first and last of the chosen keys.
fn run_of(layer: &Layer, keys: &[Keyframe], a: &Args, near: Tick, least: usize, what: &str) -> Result<(usize, usize)> {
    let idx = chosen(layer, keys, a, near)?;
    match (idx.first(), idx.last()) {
        (Some(&f), Some(&l)) if l + 1 >= f + least => Ok((f, l)),
        _ => Err(Error::params(format!("{what} needs at least {least} keys (it has {})", keys.len()))),
    }
}

fn components(v: &Value) -> Vec<f64> {
    (0..v.dims()).map(|d| v.component(d)).collect()
}

/// The keys from `a` to `b` wandered.
#[allow(clippy::too_many_arguments)]
fn wiggled(keys: &[Keyframe], a: usize, b: usize, spatial: bool, rate: FrameRate, layer: &Layer, a_: &Args) -> Vec<Keyframe> {
    let frequency = a_.num("frequency").unwrap_or(5.0).max(0.01);
    let magnitude = a_.num("magnitude").unwrap_or(1.0).abs();
    let smooth = a_.text("noise") != Some("jagged");
    let along = a_.text("apply") == Some("temporal") && spatial;
    let seed = a_.whole("seed").unwrap_or(0);
    let dims = a_.text("dimensions").unwrap_or("independent");
    let only = a_.whole("dimension").unwrap_or(0) as usize;
    let (t0, t1) = (keys[a].time, keys[b].time);
    let (f0, f1) = (rate.frame_at(layer.comp_time(t0)), rate.frame_at(layer.comp_time(t1)));
    let fps = rate.as_f64();
    let step = ((fps / frequency).round() as i64).max(1);
    let mut out: Vec<Keyframe> = keys.iter().filter(|k| k.time <= t0 || k.time >= t1).cloned().collect();
    let mut k = 1;
    loop {
        let f = f0 + k * step;
        if f >= f1 {
            break;
        }
        let lt = layer.layer_time(rate.tick_of(f));
        // temporal: the path read at a time that wanders, so the speed along it does
        let at = if along {
            let off = wander(smooth, seed, 1, k) * magnitude / 100.0 * step as f64 / fps;
            Tick::from_seconds((lt.seconds() + off).clamp(t0.seconds(), t1.seconds()))
        } else {
            lt
        };
        let Some(v) = evaluate(keys, at, spatial) else { break };
        let v = if along {
            v
        } else {
            v.with_components(|d| {
                let n = match dims {
                    "one" if d != only => 0.0,
                    "one" | "same" => wander(smooth, seed, 0, k),
                    _ => wander(smooth, seed, 1 + d as u64 * 7919, k),
                };
                v.component(d) + n * magnitude
            })
        };
        let mut key = Keyframe::new(lt, v);
        if smooth {
            (key.interp_in, key.interp_out, key.auto_bezier) = (Interp::Bezier, Interp::Bezier, true);
        }
        out.push(key);
        k += 1;
    }
    out.sort_by_key(|k| k.time);
    out
}

/// The keys from `a` to `b` replaced by the fewest smooth keys, at sample times, whose curve
/// keeps within `tolerance` of `samples`.
fn smoothed(keys: &[Keyframe], a: usize, b: usize, samples: &[(Tick, Vec<f64>)], tolerance: f64, spatial: bool) -> Vec<Keyframe> {
    let template = keys[a].value.clone();
    let make = |t: Tick, c: &[f64]| {
        let mut k = Keyframe::new(t, template.with_components(|d| c.get(d).copied().unwrap_or(0.0)));
        if !spatial {
            (k.interp_in, k.interp_out, k.auto_bezier) = (Interp::Bezier, Interp::Bezier, true);
        }
        k
    };
    let build = |picked: &[usize]| {
        let mut v: Vec<Keyframe> = keys[..=a].to_vec();
        v.extend(picked.iter().map(|&i| make(samples[i].0, &samples[i].1)));
        v.extend(keys[b..].iter().cloned());
        v.sort_by_key(|k| k.time);
        v
    };
    let mut picked: Vec<usize> = Vec::new();
    loop {
        let now = build(&picked);
        // where the curve strays furthest
        let (mut worst, mut at) = (0.0f64, 0usize);
        for (i, (t, v)) in samples.iter().enumerate() {
            if let Some(e) = evaluate(&now, *t, spatial) {
                let d = components(&e).iter().zip(v).map(|(p, q)| (p - q).powi(2)).sum::<f64>().sqrt();
                if d > worst {
                    (worst, at) = (d, i);
                }
            }
        }
        let t = samples.get(at).map_or(keys[a].time, |s| s.0);
        if worst <= tolerance || picked.contains(&at) || t <= keys[a].time || t >= keys[b].time {
            return now;
        }
        picked.push(at);
        picked.sort_unstable();
    }
}

/// The curve between layer times `t0` and `t1` sampled every composition frame.
fn samples(keys: &[Keyframe], t0: Tick, t1: Tick, spatial: bool, rate: FrameRate, layer: &Layer) -> Vec<(Tick, Vec<f64>)> {
    let (f0, f1) = (rate.frame_at(layer.comp_time(t0)), rate.frame_at(layer.comp_time(t1)));
    (f0..=f1)
        .filter_map(|f| {
            let t = layer.layer_time(rate.tick_of(f));
            (t >= t0 && t <= t1).then(|| evaluate(keys, t, spatial).map(|v| (t, components(&v))))?
        })
        .collect()
}

fn rate_of(cx: &Ctx, a: &Args) -> Result<FrameRate> {
    let cid = comp_id(cx.project, cx.view, a)?;
    Ok(cx.project.comp(cid).expect("found").frame_rate)
}

fn wiggle(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let rate = rate_of(cx, a)?;
    let (layer, path, near) = target(cx, a)?;
    let p = layer.props.prop(&path).map_err(Error::not_found)?;
    if !p.value.interpolates() || p.value.dims() == 0 {
        return Err(Error::params(format!("{path} is not numbers to wiggle")));
    }
    let (keys, spatial) = (p.keys.clone(), p.def.spatial);
    let (f, l) = run_of(layer, &keys, a, near, 2, "the Wiggler")?;
    let before = l - f + 1;
    let new = wiggled(&keys, f, l, spatial, rate, layer, a);
    let after = new.iter().filter(|k| k.time >= keys[f].time && k.time <= keys[l].time).count();
    store(layer, &path, new)?;
    let mut o = keys_json(layer, &path);
    if let Some(m) = o.as_object_mut() {
        m.insert("before", before as f64);
        m.insert("after", after as f64);
    }
    cx.label = Some("Wiggler".into());
    Ok(o)
}

fn smooth(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let rate = rate_of(cx, a)?;
    let (layer, path, near) = target(cx, a)?;
    let p = layer.props.prop(&path).map_err(Error::not_found)?;
    let (keys, spatial) = (p.keys.clone(), p.def.spatial);
    let (f, l) = run_of(layer, &keys, a, near, 3, "the Smoother")?;
    let s = samples(&keys, keys[f].time, keys[l].time, spatial, rate, layer);
    let new = smoothed(&keys, f, l, &s, a.num("tolerance").unwrap_or(1.0).max(0.0), spatial);
    let (before, after) = (l - f + 1, new.iter().filter(|k| k.time >= keys[f].time && k.time <= keys[l].time).count());
    store(layer, &path, new)?;
    let mut o = keys_json(layer, &path);
    if let Some(m) = o.as_object_mut() {
        m.insert("before", before as f64);
        m.insert("after", after as f64);
    }
    cx.label = Some("Smoother".into());
    Ok(o)
}

/// How a drawing done over time becomes keys: from `start` (a composition time), played back at
/// `speed` (1: as fast as it was drawn), smoothed to stray at most `smoothing` (0: a key every
/// frame), ending before the composition's `end`.
pub(super) struct Take {
    pub start: Tick,
    pub speed: f64,
    pub smoothing: f64,
    pub rate: FrameRate,
    pub end: Tick,
}

/// A property's keys `old` with a drawing `drawn` ([(seconds from its start, point)], in time
/// order) put in as a key a frame (each point made a value by `value`), smoothed, in place of
/// the keys it covers; and the layer times it covers. None when it falls after the end.
pub(super) fn recorded(old: &[Keyframe], drawn: &[(f64, [f64; 2])], take: &Take, layer: &Layer, value: impl Fn([f64; 2]) -> Value) -> Option<(Vec<Keyframe>, Tick, Tick)> {
    let (rate, fd) = (take.rate, take.rate.frame_duration().seconds());
    let first = drawn.first()?.0;
    let frames = (((drawn.last().map_or(0.0, |d| d.0) - first) * take.speed) / fd).round() as i64;
    let last_frame = rate.frame_at(take.end - rate.frame_duration());
    let at = |s: f64| {
        if drawn.len() == 1 {
            return drawn[0].1;
        }
        let i = drawn.partition_point(|d| d.0 <= s).clamp(1, drawn.len() - 1);
        let (p, q) = (drawn[i - 1], drawn[i]);
        let f = if q.0 > p.0 { ((s - p.0) / (q.0 - p.0)).clamp(0.0, 1.0) } else { 1.0 };
        [p.1[0] + (q.1[0] - p.1[0]) * f, p.1[1] + (q.1[1] - p.1[1]) * f]
    };
    let f0 = rate.frame_at(take.start);
    let new: Vec<Keyframe> = (0..=frames).take_while(|k| f0 + k <= last_frame).map(|k| Keyframe::new(layer.layer_time(rate.tick_of(f0 + k)), value(at(first + k as f64 * fd / take.speed)))).collect();
    let (t0, t1) = (new.first()?.time, new.last()?.time);
    let mut keys: Vec<Keyframe> = old.iter().filter(|k| k.time < t0 || k.time > t1).cloned().collect();
    keys.extend(new);
    keys.sort_by_key(|k| k.time);
    let (i0, i1) = (keys.iter().position(|k| k.time == t0).unwrap_or(0), keys.iter().position(|k| k.time == t1).unwrap_or(0));
    if take.smoothing > 0.0 && i1 > i0 + 1 {
        let s: Vec<(Tick, Vec<f64>)> = keys[i0..=i1].iter().map(|k| (k.time, components(&k.value))).collect();
        keys = smoothed(&keys, i0, i1, &s, take.smoothing, true);
    }
    Some((keys, t0, t1))
}

fn sketch(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let (rate, end) = {
        let c = cx.project.comp(cid).expect("found");
        (c.frame_rate, c.duration)
    };
    let fd = rate.frame_duration().seconds();
    let mut drawn: Vec<(f64, [f64; 2])> = Vec::new();
    for (i, v) in a.json("points").and_then(Json::as_array).cloned().unwrap_or_default().iter().enumerate() {
        let n: Vec<f64> = v.as_array().map(|a| a.iter().filter_map(Json::as_f64).collect()).unwrap_or_default();
        match n.len() {
            2 => drawn.push((i as f64 * fd, [n[0], n[1]])),
            3 => drawn.push((n[0], [n[1], n[2]])),
            _ => return Err(Error::params(format!("point {i} is [t, x, y] or [x, y]"))),
        }
    }
    if drawn.len() < 2 {
        return Err(Error::params("a sketch needs two points or more"));
    }
    drawn.sort_by(|p, q| p.0.total_cmp(&q.0));
    let speed = a.num("captureSpeed").unwrap_or(100.0).max(1.0) / 100.0;
    let smoothing = a.num("smoothing").unwrap_or(1.0).max(0.0);
    let start = rate.snap_nearest(a.time("start").unwrap_or(cx.view.time));
    let lid = find_layer(cx.project.comp(cid).expect("found"), a.need("layer"))?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    if layer.separate_position {
        return Err(Error::refused("its position is in separate dimensions; sketch into positionX and positionY with prop.keys"));
    }
    let pos = layer.props.prop("transform/position").map_err(Error::not_found)?;
    let z = pos.value_at(layer.layer_time(start)).as_vec3()[2];
    let take = Take { start, speed, smoothing, rate, end };
    let (keys, t0, t1) = recorded(&pos.keys, &drawn, &take, layer, |q| Value::Vec3([q[0], q[1], z])).ok_or_else(|| Error::params("the sketch falls after the composition's end"))?;
    let n = keys.iter().filter(|k| k.time >= t0 && k.time <= t1).count();
    store(layer, "transform/position", keys)?;
    cx.label = Some("Motion Sketch".into());
    Ok(Json::object([("keys", Json::Number(n as f64))]))
}

fn transform(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let rate = rate_of(cx, a)?;
    let (layer, path, near) = target(cx, a)?;
    let keys = layer.props.prop(&path).map_err(Error::not_found)?.keys.clone();
    let idx = chosen(layer, &keys, a, near)?;
    if idx.is_empty() {
        return Err(Error::params(format!("{path} has no keys")));
    }
    let ts = a.num("timeScale").unwrap_or(1.0);
    if !(ts > 0.0 && ts.is_finite()) {
        return Err(Error::params("`timeScale` is above 0"));
    }
    let (toff, vs, va, voff) = (a.num("timeOffset").unwrap_or(0.0), a.num("valueScale").unwrap_or(1.0), a.num("valueAnchor").unwrap_or(0.0), a.num("valueOffset").unwrap_or(0.0));
    let dim = a.whole("dim").map(|d| d as usize);
    let anchor = a.time("timeAnchor").unwrap_or_else(|| layer.comp_time(keys[idx[0]].time)).seconds();
    let (mut moved, mut kept): (Vec<Keyframe>, Vec<Keyframe>) = (Vec::new(), Vec::new());
    for (i, k) in keys.into_iter().enumerate() {
        if idx.contains(&i) { moved.push(k) } else { kept.push(k) }
    }
    for k in &mut moved {
        let ct = anchor + (layer.comp_time(k.time).seconds() - anchor) * ts + toff;
        k.time = layer.layer_time(rate.snap_nearest(Tick::from_seconds(ct.max(0.0))));
        if vs != 1.0 || voff != 0.0 {
            let v = k.value.clone();
            k.value = v.with_components(|d| if dim.is_none_or(|x| x == d) { va + (v.component(d) - va) * vs + voff } else { v.component(d) });
        }
    }
    moved.sort_by_key(|k| k.time);
    moved.dedup_by_key(|k| k.time);
    kept.retain(|k| !moved.iter().any(|m| m.time == k.time));
    let n = moved.len();
    kept.extend(moved);
    store(layer, &path, kept)?;
    cx.label = Some("Transform Keyframes".into());
    Ok(Json::object([("keys", Json::Number(n as f64))]))
}

fn exponential(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let rate = rate_of(cx, a)?;
    let path_given = a.text("path").unwrap_or("transform/scale").to_string();
    let (cid, lid) = super::layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let half = Tick(rate.frame_duration().0 / 2);
    let keys = layer.props.prop(&path_given).map_err(Error::not_found)?.keys.clone();
    let (f, l) = run_of(layer, &keys, a, half, 2, "Exponential Scale")?;
    let (ta, tb) = (keys[f].time, keys[l].time);
    let (va, vb) = (keys[f].value.clone(), keys[l].value.clone());
    let (fa, fb) = (rate.frame_at(layer.comp_time(ta)), rate.frame_at(layer.comp_time(tb)));
    let mut out: Vec<Keyframe> = keys.iter().filter(|k| k.time < ta || k.time > tb).cloned().collect();
    let n = (fb - fa).max(1);
    for i in 0..=n {
        let u = i as f64 / n as f64;
        let t = if i == n { tb } else { layer.layer_time(rate.tick_of(fa + i)) };
        // the same factor each frame (where the two ends share a sign and neither is 0)
        let v = va.with_components(|d| {
            let (x, y) = (va.component(d), vb.component(d));
            if x.abs() < 1e-9 || y.abs() < 1e-9 || x.signum() != y.signum() { x + (y - x) * u } else { x * (y / x).powf(u) }
        });
        out.push(Keyframe::new(t, v));
    }
    out.sort_by_key(|k| k.time);
    out.dedup_by_key(|k| k.time);
    store(layer, &path_given, out)?;
    cx.label = Some("Exponential Scale".into());
    Ok(Json::object([("keys", Json::Number((n + 1) as f64))]))
}

/// The layers `layers` names, in order, each once.
fn ordered(cx: &Ctx, a: &Args) -> Result<(u64, Vec<u64>)> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let mut out = Vec::new();
    for j in a.json("layers").and_then(Json::as_array).cloned().unwrap_or_default() {
        let id = find_layer(comp, &j)?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    if out.len() < 2 {
        return Err(Error::params("give two layers or more, in the order they go"));
    }
    Ok((cid, out))
}

fn sequence(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, ids) = ordered(cx, a)?;
    let overlap = Tick::from_seconds(a.num("overlap").unwrap_or(0.0).max(0.0));
    let transition = a.text("transition").unwrap_or("off").to_string();
    if transition != "off" && overlap == Tick::ZERO {
        return Err(Error::params("a transition needs `overlap`, the seconds it lasts"));
    }
    let comp = cx.project.comp_mut(cid).expect("found");
    let mut next: Option<Tick> = None;
    let mut out = Vec::new();
    for (k, id) in ids.iter().enumerate() {
        let l = comp.layer_mut(*id).expect("found");
        if let Some(at) = next {
            let d = at - l.in_point;
            (l.start, l.in_point, l.out_point) = (l.start + d, l.in_point + d, l.out_point + d);
        }
        if transition != "off" {
            let last = k + 1 == ids.len();
            let (a0, a1) = (l.layer_time(l.in_point), l.layer_time(l.in_point + overlap));
            let (b0, b1) = (l.layer_time(l.out_point - overlap), l.layer_time(l.out_point));
            let mut keys = Vec::new();
            if k > 0 {
                keys.extend([Keyframe::new(a0, Value::Number(0.0)), Keyframe::new(a1, Value::Number(100.0))]);
            }
            if transition == "crossDissolve" && !last {
                keys.extend([Keyframe::new(b0, Value::Number(100.0)), Keyframe::new(b1, Value::Number(0.0))]);
            }
            keys.sort_by_key(|k| k.time);
            keys.dedup_by_key(|k| k.time);
            let p = l.props.prop_mut("transform/opacity").map_err(Error::not_found)?;
            if keys.is_empty() { p.set_value(Value::Number(100.0)) } else { p.set_keys(keys) }.map_err(Error::params)?;
        }
        out.push(Json::object([("layer", Json::Number(*id as f64)), ("in", Json::Number(l.in_point.seconds())), ("out", Json::Number(l.out_point.seconds()))]));
        next = Some(l.out_point - overlap);
    }
    cx.label = Some("Sequence Layers".into());
    Ok(Json::object([("layers", Json::Array(out))]))
}

fn stagger(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, ids) = ordered(cx, a)?;
    let n = ids.len();
    let offsets: Vec<f64> = match (a.num("step"), a.num("total")) {
        (Some(s), None) => (0..n).map(|i| s * i as f64).collect(),
        (None, Some(t)) => (0..n).map(|i| t * i as f64 / (n - 1) as f64).collect(),
        _ => return Err(Error::params("give `step` or `total`")),
    };
    let path = a.text("path").map(str::to_string);
    let comp = cx.project.comp_mut(cid).expect("found");
    let mut out = Vec::new();
    for (id, off) in ids.iter().zip(&offsets) {
        let d = Tick::from_seconds(*off);
        let l = comp.layer_mut(*id).expect("found");
        match &path {
            Some(p) => {
                let stretch = l.stretch / 100.0;
                let prop = l.props.prop_mut(p).map_err(Error::not_found)?;
                let mut keys = prop.keys.clone();
                // a composition-time move in the layer's own time
                let dl = Tick::from_seconds(d.seconds() / stretch);
                for k in &mut keys {
                    k.time += dl;
                }
                prop.set_keys(keys).map_err(Error::params)?;
            }
            None => (l.start, l.in_point, l.out_point) = (l.start + d, l.in_point + d, l.out_point + d),
        }
        out.push(Json::object([("layer", Json::Number(*id as f64)), ("offset", Json::Number(*off))]));
    }
    cx.label = Some("Stagger Layers".into());
    Ok(Json::object([("layers", Json::Array(out))]))
}
