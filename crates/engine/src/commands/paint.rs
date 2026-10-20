//! Paint strokes laid on a layer by hand: painted with a brush, copied with a clone stamp,
//! rubbed out with an eraser. Each is kept in the layer's Paint effect (one is added when the
//! layer's last effect is not Paint) as settings to change and animate afterwards; group.remove
//! takes one away.

use veymelo_effects::strokes::{CHANNELS, ERASE_MODES, PAINT_MODES, StrokeKind, TIPS};
use veymelo_json::Value as Json;
use veymelo_keyframe::{Keyframe, ShapePath, Value};
use veymelo_project::{Group, Node, Source};
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, find_layer, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

/// How long a stroke shows: to the layer's end, the same while it writes itself on, a single
/// frame, or some frames.
const DURATIONS: &[&str] = &["constant", "writeOn", "singleFrame", "custom"];

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "paint.stroke",
            label: "Paint Stroke",
            summary: "Paint a stroke on a layer along points in its pixels: a brush paints colour, a clone stamp copies pixels from elsewhere (on this layer or another, now or at another time), an eraser rubs out the layer and paint (or paint alone, or only the stroke before it). It goes into the layer's Paint effect, from the time on: to the layer's end, writing itself on over `drawTime`, for a single frame or for `frames`. Its settings (effects/Paint/strokes/Brush 1/…) stay to change and animate.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("points", Ty::List, "where it goes, in the layer's pixels: [[x, y], …], or [x, y, pressure] with the pen's pressure 0 to 1"),
                    Param::opt("kind", Ty::Choice(&StrokeKind::IDS), "brush (default), clone or eraser"),
                    Param::opt("smooth", Ty::Bool, "a smooth curve through the points rather than straight lines between them (default false)"),
                    Param::opt("tip", Ty::Text, "a brush tip to start from (paint.tips lists them); what else is given changes it"),
                    Param::opt("color", Ty::Color, "a brush's colour (default white)"),
                    Param::opt("diameter", Ty::Number, "the tip's width in pixels (default 10)"),
                    Param::opt("angle", Ty::Number, "the tip turned, in degrees"),
                    Param::opt("hardness", Ty::Number, "how much of the tip is solid before it fades, in % (default 100)"),
                    Param::opt("roundness", Ty::Number, "the tip's width the other way, in % of its diameter (default 100)"),
                    Param::opt("spacing", Ty::Number, "how far apart its dabs are, in % of the diameter (default 25)"),
                    Param::opt("opacity", Ty::Number, "the most it covers, in % (default 100)"),
                    Param::opt("flow", Ty::Number, "how much each dab adds, in % (default 100)"),
                    Param::opt("mode", Ty::Choice(PAINT_MODES), "how its colour mixes with what is under it (default normal)"),
                    Param::opt("channels", Ty::Choice(CHANNELS), "what it paints into: rgba (default), rgb or alpha"),
                    Param::opt("eraseMode", Ty::Choice(ERASE_MODES), "an eraser's: layerAndPaint (default), paintOnly or lastStroke"),
                    Param::opt("cloneSource", Ty::Layer, "the layer a clone copies from (default: this one)"),
                    Param::opt("clonePosition", Ty::List, "[x, y]: where in the source a clone's first point copies from, in its pixels (a clone needs it)"),
                    Param::opt("cloneTimeShift", Ty::Number, "seconds later (below 0: earlier) a clone copies its source"),
                    Param::opt("cloneTime", Ty::Number, "a clone always copies its source at this time of its own, in seconds"),
                    Param::opt("sizeByPressure", Ty::Bool, "lighter pressure, a smaller tip (default true)"),
                    Param::opt("minSize", Ty::Number, "the tip's size at no pressure, in % (default 1)"),
                    Param::opt("opacityByPressure", Ty::Bool, "lighter pressure, less opaque (default false)"),
                    Param::opt("flowByPressure", Ty::Bool, "lighter pressure, less flow (default false)"),
                    Param::opt("time", Ty::Time, "when it starts to show (default: the view's time)"),
                    Param::opt("duration", Ty::Choice(DURATIONS), "constant (default): to the layer's end; writeOn: the same, drawn on over drawTime; singleFrame; custom: for `frames`"),
                    Param::opt("drawTime", Ty::Number, "writeOn: how long it takes to draw, in seconds (default 1)"),
                    Param::opt("frames", Ty::Whole, "custom: how many frames it shows (default 1)"),
                ]
            },
            returns: "{effect, stroke}",
            run: Run::Edit(stroke),
        },
        Command {
            id: "paint.tips",
            label: "Brush Tips",
            summary: "The brush tips there are to start strokes from: each one's name, diameter, angle, roundness, hardness and spacing.",
            params: &[],
            returns: "[{name, diameter, angle, roundness, hardness, spacing}]",
            run: Run::Look(tips),
        },
    ]
}

fn tips(_: &Look, _: &Args) -> Result<Json> {
    Ok(Json::array(TIPS.iter().map(|t| {
        Json::object([
            ("name", Json::from(t.name)),
            ("diameter", Json::Number(t.diameter)),
            ("angle", Json::Number(t.angle)),
            ("roundness", Json::Number(t.roundness)),
            ("hardness", Json::Number(t.hardness)),
            ("spacing", Json::Number(t.spacing)),
        ])
    })))
}

/// A curve through `points`: straight between them, or smooth (each point's tangents along the
/// line from the point before it to the one after, a sixth of that long).
fn path_through(points: &[[f64; 2]], smooth: bool) -> ShapePath {
    let n = points.len();
    let mut ins = vec![[0.0; 2]; n];
    let mut outs = vec![[0.0; 2]; n];
    if smooth && n > 2 {
        for i in 0..n {
            let (a, b) = (points[i.saturating_sub(1)], points[(i + 1).min(n - 1)]);
            let t = [(b[0] - a[0]) / 6.0, (b[1] - a[1]) / 6.0];
            outs[i] = t;
            ins[i] = [-t[0], -t[1]];
        }
        (ins[0], outs[n - 1]) = ([0.0; 2], [0.0; 2]);
    }
    ShapePath { points: points.to_vec(), in_tangents: ins, out_tangents: outs, closed: false, feather: Default::default() }
}

fn stroke(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let layer = comp.layer(lid).expect("found");
    if matches!(layer.source, Source::Camera | Source::Light(_) | Source::Null) {
        return Err(Error::refused(format!("\"{}\" shows nothing to paint on", layer.name)));
    }
    let kind = match a.text("kind").unwrap_or("brush") {
        "clone" => StrokeKind::Clone,
        "eraser" => StrokeKind::Eraser,
        _ => StrokeKind::Brush,
    };
    // where it goes, and the pen's pressure there
    let mut points = Vec::new();
    let mut pressure = Vec::new();
    for (i, q) in a.json("points").and_then(Json::as_array).cloned().unwrap_or_default().iter().enumerate() {
        let n: Vec<f64> = q.as_array().map(|v| v.iter().filter_map(Json::as_f64).collect()).unwrap_or_default();
        if !(2..=3).contains(&n.len()) || n.iter().any(|v| !v.is_finite()) {
            return Err(Error::params(format!("point {} is [x, y] or [x, y, pressure], not {}", i + 1, q.to_json())));
        }
        points.push([n[0], n[1]]);
        pressure.push(n.get(2).copied().unwrap_or(1.0).clamp(0.0, 1.0));
    }
    if points.is_empty() {
        return Err(Error::params("`points` needs a point at least"));
    }
    for (key, only) in [("color", StrokeKind::Brush), ("eraseMode", StrokeKind::Eraser), ("cloneSource", StrokeKind::Clone), ("clonePosition", StrokeKind::Clone), ("cloneTimeShift", StrokeKind::Clone), ("cloneTime", StrokeKind::Clone)] {
        if a.has(key) && kind != only {
            return Err(Error::params(format!("`{key}` is for {} strokes", StrokeKind::IDS[only as usize])));
        }
    }
    if kind == StrokeKind::Clone && !a.has("clonePosition") {
        return Err(Error::params("a clone stroke needs `clonePosition`: where in its source its first point copies from"));
    }
    let clone_source = match a.json("cloneSource") {
        Some(j) => Some(find_layer(comp, j)?),
        None => None,
    };
    let rate = comp.frame_rate;
    let t = rate.snap_nearest(a.time("time").unwrap_or(cx.view.time));
    let lt = layer.layer_time(t);
    let duration = a.text("duration").unwrap_or("constant");
    let frame = rate.frame_duration();
    let length = match duration {
        "singleFrame" => frame.seconds(),
        "custom" => frame.seconds() * a.whole("frames").unwrap_or(1).max(1) as f64,
        _ => 0.0,
    };
    // the settings it starts with: a tip, then what is given
    let mut set: Vec<(&str, Value)> = Vec::new();
    if let Some(name) = a.text("tip") {
        let tip = TIPS.iter().find(|t| t.name.eq_ignore_ascii_case(name)).ok_or_else(|| Error::params(format!("there is no tip \"{name}\" (paint.tips lists them)")))?;
        set.extend([("options/diameter", tip.diameter), ("options/angle", tip.angle), ("options/roundness", tip.roundness), ("options/hardness", tip.hardness), ("options/spacing", tip.spacing)].map(|(k, v)| (k, Value::Number(v))));
    }
    for (key, path) in [
        ("diameter", "options/diameter"),
        ("angle", "options/angle"),
        ("hardness", "options/hardness"),
        ("roundness", "options/roundness"),
        ("spacing", "options/spacing"),
        ("opacity", "options/opacity"),
        ("flow", "options/flow"),
        ("cloneTimeShift", "options/cloneTimeShift"),
        ("minSize", "minSize"),
    ] {
        if let Some(v) = a.num(key) {
            set.push((path, Value::Number(v)));
        }
    }
    for (key, path) in [("sizeByPressure", "sizeByPressure"), ("opacityByPressure", "opacityByPressure"), ("flowByPressure", "flowByPressure")] {
        if let Some(v) = a.bool(key) {
            set.push((path, Value::Bool(v)));
        }
    }
    let pick = |key: &str, ids: &[&str]| a.text(key).and_then(|v| ids.iter().position(|i| *i == v)).map(|i| Value::Choice(i as u32));
    if let Some(v) = pick("mode", PAINT_MODES).filter(|_| kind != StrokeKind::Eraser) {
        set.push(("mode", v));
    }
    if let Some(v) = pick("channels", CHANNELS).filter(|_| kind != StrokeKind::Eraser) {
        set.push(("options/channels", v));
    }
    if let Some(v) = pick("eraseMode", ERASE_MODES) {
        set.push(("options/eraseMode", v));
    }
    if let Some(c) = a.color("color") {
        set.push(("options/color", Value::Color(c)));
    }
    if let Some(j) = a.json("clonePosition") {
        let p: Vec<f64> = j.as_array().map(|v| v.iter().filter_map(Json::as_f64).collect()).unwrap_or_default();
        if p.len() != 2 {
            return Err(Error::params(format!("`clonePosition` is [x, y], not {}", j.to_json())));
        }
        set.push(("options/clonePosition", Value::Vec2([p[0], p[1]])));
    }
    if let Some(at) = a.num("cloneTime") {
        set.extend([("options/lockTime", Value::Bool(true)), ("options/cloneTime", Value::Number(at))]);
    }
    if let Some(id) = clone_source {
        set.push(("options/cloneSource", Value::Layer(Some(id))));
    }
    let first = points[0];
    set.extend([
        ("path", Value::Path(path_through(&points, a.bool("smooth").unwrap_or(false)))),
        ("transform/anchor", Value::Vec2(first)),
        ("transform/position", Value::Vec2(first)),
        ("from", Value::Number(lt.seconds())),
        ("length", Value::Number(length)),
    ]);
    if pressure.iter().any(|p| *p < 1.0) {
        set.push(("pressure", Value::Text(pressure.iter().map(|p| format!("{}", (p * 1000.0).round() / 1000.0)).collect::<Vec<_>>().join(","))));
    }
    // the layer's Paint effect: its last effect, or a new one
    let last_paint = layer.props.group("effects").ok().and_then(|fx| fx.children.last()).and_then(Node::as_group).filter(|g| g.def.id == "paint").map(|g| g.name().to_string());
    let effect = match last_paint {
        Some(name) => format!("effects/{name}"),
        None => {
            let made = super::call(cx, "effect.add", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(lid as f64)), ("type", Json::from("paint"))]))?;
            made.get("path").and_then(Json::as_str).expect("made").to_string()
        }
    };
    let id = cx.project.alloc();
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let list = layer.props.group_mut(&format!("{effect}/strokes")).map_err(Error::refused)?;
    let label = kind.def().name;
    let name = (1..).map(|n| format!("{label} {n}")).find(|n| !list.children.iter().any(|c| c.name() == n)).expect("some number is free");
    let mut g = Group::instance(kind.def(), id, &name);
    for (path, v) in set {
        g.prop_mut(path).map_err(Error::refused)?.set_value(v).map_err(|e| Error::params(format!("{path}: {e}")))?;
    }
    if duration == "writeOn" {
        let draw = a.num("drawTime").unwrap_or(1.0);
        if !(draw > 0.0 && draw.is_finite()) {
            return Err(Error::params(format!("`drawTime` is above 0 seconds, not {draw}")));
        }
        let done = lt + rate.snap_nearest(Tick::from_seconds(draw)).max(frame);
        g.prop_mut("options/end").expect("defined").set_keys(vec![Keyframe::new(lt, Value::Number(0.0)), Keyframe::new(done, Value::Number(100.0))]).map_err(Error::refused)?;
    }
    list.children.push(Node::Group(g));
    cx.label = Some(format!("{label} Stroke"));
    Ok(Json::object([("effect", Json::from(effect.as_str())), ("stroke", Json::from(format!("{effect}/strokes/{name}")))]))
}
