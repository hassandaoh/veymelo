//! Commands about keyframes one at a time: list them, set one with all it can have, ease them
//! (easy ease, linear, hold, auto, continuous), move, remove, reverse and rove them.
//!
//! Times given are composition times; a key is found within half a frame of the time given.
//! Keys are kept in the layer's own time, which is the same unless the layer was moved in time
//! or stretched; what is given back has both (`time` the composition's, `t` the layer's).

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Interp, Keyframe, retime_roving};
use veymelo_project::Layer;
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

const PATH: Param = Param::need("path", Ty::Text, "the property, like transform/position");
const TIMES: Param = Param::opt("times", Ty::List, "the keys, by their times in seconds (default: all of them)");

/// The ways keys can be eased at once.
const EASES: &[&str] = &["easy", "easyIn", "easyOut", "linear", "hold", "auto", "continuous"];

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "key.list",
            label: "Keyframes",
            summary: "A property's keys with everything about each.",
            params: const { &[COMP, LAYER, PATH] },
            returns: "[{index, time, t, v, in?, out?, easeIn?, easeOut?, …}]",
            run: Run::Look(key_list),
        },
        Command {
            id: "key.set",
            label: "Set Keyframe",
            summary: "Make a key at a time, or change the one there: its value, how it moves in and out (linear, bezier, hold), its eases (speed and influence), auto or continuous Bezier, its path tangents, roving, label.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    Param::need("time", Ty::Time, "the key's time"),
                    Param::opt("value", Ty::Any, "its value (needed to make a key)"),
                    Param::opt("in", Ty::Choice(&["linear", "bezier", "hold"]), "how the value arrives"),
                    Param::opt("out", Ty::Choice(&["linear", "bezier", "hold"]), "how the value leaves"),
                    Param::opt("easeIn", Ty::Any, "{speed, influence} the value arrives with (influence 0.001 to 1), or one for each of its numbers"),
                    Param::opt("easeOut", Ty::Any, "{speed, influence} the value leaves with"),
                    Param::opt("autoBezier", Ty::Bool, "Bezier sides whose speed comes from the keys around it"),
                    Param::opt("continuous", Ty::Bool, "one speed through the key: the side given sets the other"),
                    Param::opt("spatialIn", Ty::List, "a point's path tangent into the key, [x, y] from its value (turns spatialAuto off)"),
                    Param::opt("spatialOut", Ty::List, "the path tangent out of the key (turns spatialAuto off)"),
                    Param::opt("spatialAuto", Ty::Bool, "path tangents from the keys around it (on for new keys)"),
                    Param::opt("roving", Ty::Bool, "its time follows from its neighbours, for one speed through it"),
                    Param::opt("label", Ty::Choice(veymelo_project::Label::IDS), "its colour tag"),
                ]
            },
            returns: "{index, time, t, v, …}",
            run: Run::Edit(key_set),
        },
        Command {
            id: "key.ease",
            label: "Ease Keyframes",
            summary: "Ease keys the usual ways: easy (slow in and out), easyIn, easyOut, linear, hold (the value stays until the next key), auto (smooth from the keys around), continuous (one speed through each key).",
            params: const { &[COMP, LAYER, PATH, Param::need("ease", Ty::Choice(EASES), "how"), TIMES] },
            returns: "{keys}",
            run: Run::Edit(key_ease),
        },
        Command {
            id: "key.move",
            label: "Move Keyframes",
            summary: "Move one key to another time (`time` and `to`), or keys by an amount (`times` and `by`).",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    Param::opt("time", Ty::Time, "the key to move"),
                    Param::opt("to", Ty::Time, "where it goes"),
                    TIMES,
                    Param::opt("by", Ty::Number, "seconds to move them by (below zero: earlier)"),
                ]
            },
            returns: "{keys}",
            run: Run::Edit(key_move),
        },
        Command {
            id: "key.remove",
            label: "Remove Keyframes",
            summary: "Take keys away. With none left, the property keeps the value it had at the view's time.",
            params: const { &[COMP, LAYER, PATH, TIMES] },
            returns: "{keys}",
            run: Run::Edit(key_remove),
        },
        Command {
            id: "key.copy",
            label: "Copy Keyframes",
            summary: "Copy keys of a property (all, or those at `times`) to another property, on this layer or another, keeping how far apart they are: the first lands at `at` (default: where it is). Keys already at those moments are replaced. Both properties take the same kind of value.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    TIMES,
                    Param::opt("toLayer", Ty::Layer, "the layer they go to (default: the same one)"),
                    Param::opt("toPath", Ty::Text, "the property they go to (default: the same one)"),
                    Param::opt("at", Ty::Time, "where the first one lands (default: where it is)"),
                ]
            },
            returns: "{path, keys}",
            run: Run::Edit(key_copy),
        },
        Command {
            id: "key.reverse",
            label: "Time-Reverse Keyframes",
            summary: "Turn keys around within the time they span: what happened first happens last.",
            params: const { &[COMP, LAYER, PATH, TIMES] },
            returns: "{keys}",
            run: Run::Edit(key_reverse),
        },
    ]
}

/// The layer and the property's path the parameters name.
pub(super) fn target<'a>(cx: &'a mut Ctx, a: &Args) -> Result<(&'a mut Layer, String, Tick)> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp_mut(cid).expect("found");
    let half_frame = Tick(comp.frame_rate.frame_duration().0 / 2);
    let layer = comp.layer_mut(id).expect("found");
    let path = a.text("path").unwrap_or("").to_string();
    let id = layer.props.prop(&path).map_err(Error::not_found)?.def.id;
    super::prop::position_in_use(layer, id)?;
    Ok((layer, path, half_frame))
}

/// The key nearest `t` (layer time), within `near`.
fn key_near(keys: &[Keyframe], t: Tick, near: Tick) -> Option<usize> {
    keys.iter().enumerate().map(|(i, k)| (i, (k.time - t).abs())).filter(|(_, d)| *d <= near).min_by_key(|(_, d)| *d).map(|(i, _)| i)
}

/// The keys `times` names (all without it), or what is wrong.
pub(super) fn chosen(layer: &Layer, keys: &[Keyframe], a: &Args, near: Tick) -> Result<Vec<usize>> {
    let Some(times) = a.json("times") else { return Ok((0..keys.len()).collect()) };
    let mut out = Vec::new();
    for t in times.as_array().cloned().unwrap_or_default() {
        let secs = t.as_f64().ok_or_else(|| Error::params(format!("`times` are seconds, not {}", t.to_json())))?;
        let at = layer.layer_time(Tick::from_seconds(secs));
        let i = key_near(keys, at, near).ok_or_else(|| Error::not_found(format!("there is no key at {secs} s{}", list(layer, keys))))?;
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out.sort();
    Ok(out)
}

/// The times there are keys at, for mistakes.
fn list(layer: &Layer, keys: &[Keyframe]) -> String {
    if keys.is_empty() {
        return " (it has no keys)".into();
    }
    let times: Vec<String> = keys.iter().map(|k| format!("{}", layer.comp_time(k.time).seconds())).collect();
    format!(" (there are keys at {} s)", times.join(", "))
}

fn key_json(layer: &Layer, i: usize, k: &Keyframe, kind: veymelo_keyframe::Kind) -> Json {
    let mut o = Map::new();
    o.insert("index", i as f64);
    o.insert("time", layer.comp_time(k.time).seconds());
    if let Json::Object(fields) = k.to_json(kind) {
        for (key, v) in fields.iter() {
            o.insert(key, v.clone());
        }
    }
    Json::Object(o)
}

pub(super) fn keys_json(layer: &Layer, path: &str) -> Json {
    let p = layer.props.prop(path).expect("found");
    Json::object([("keys", Json::array(p.keys.iter().enumerate().map(|(i, k)| key_json(layer, i, k, p.def.kind))))])
}

/// Give a property its changed keys: roving keys retimed, then checked.
pub(super) fn store(layer: &mut Layer, path: &str, mut keys: Vec<Keyframe>) -> Result<()> {
    let p = layer.props.prop_mut(path).expect("found");
    keys.sort_by_key(|k| k.time);
    if keys.iter().any(|k| k.roving) {
        retime_roving(&mut keys, p.def.spatial);
    }
    p.set_keys(keys).map_err(Error::params)
}

fn key_list(l: &Look, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(l.project, l.view, a)?;
    let layer = l.project.comp(cid).expect("found").layer(id).expect("found");
    let path = a.text("path").unwrap_or("");
    layer.props.prop(path).map_err(Error::not_found)?;
    Ok(keys_json(layer, path).get("keys").cloned().unwrap_or(Json::Null))
}

fn key_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (layer, path, near) = target(cx, a)?;
    let time = a.num("time").unwrap_or(0.0);
    let at = layer.layer_time(Tick::from_seconds(time));
    let p = layer.props.prop(&path).expect("found");
    if !p.def.animatable {
        return Err(Error::refused(format!("{path} cannot be animated")));
    }
    let (kind, mut keys) = (p.def.kind, p.keys.clone());
    let found = key_near(&keys, at, near);
    // the key as JSON, the given fields put over it, read back the way a file is read
    let mut j = match found {
        Some(i) => match keys[i].to_json(kind) {
            Json::Object(m) => m,
            _ => Map::new(),
        },
        None => {
            let v = a.json("value").ok_or_else(|| Error::params(format!("there is no key at {time} s{}; give `value` to make one", list(layer, &keys))))?;
            Map::from_iter([("t", Json::Number(at.seconds())), ("v", v.clone())])
        }
    };
    for (param, field) in [
        ("value", "v"),
        ("in", "in"),
        ("out", "out"),
        ("easeIn", "easeIn"),
        ("easeOut", "easeOut"),
        ("autoBezier", "autoBezier"),
        ("continuous", "continuous"),
        ("spatialIn", "spatialIn"),
        ("spatialOut", "spatialOut"),
        ("spatialAuto", "spatialAuto"),
        ("roving", "roving"),
        ("label", "label"),
    ] {
        if let Some(v) = a.json(param) {
            let v = if param == "label" { Json::Number(veymelo_project::Label::from_id(v.as_str().unwrap_or("")).map_or(0, |l| l as u8) as f64) } else { v.clone() };
            j.insert(field, v);
        }
    }
    // tangents given by hand: the path is no longer automatic there
    if (a.has("spatialIn") || a.has("spatialOut")) && !a.has("spatialAuto") {
        j.insert("spatialAuto", false);
        let zero = Json::array([0.0, 0.0]);
        for side in ["spatialIn", "spatialOut"] {
            if !j.contains(side) {
                j.insert(side, zero.clone());
            }
        }
    }
    let mut key = Keyframe::from_json(kind, &Json::Object(j)).map_err(Error::params)?;
    if key.continuous {
        if a.has("easeOut") && !a.has("easeIn") {
            key.ease_in = key.ease_out.clone();
        } else if a.has("easeIn") {
            key.ease_out = key.ease_in.clone();
        }
    }
    match found {
        Some(i) => keys[i] = key,
        None => keys.push(key),
    }
    store(layer, &path, keys)?;
    let p = layer.props.prop(&path).expect("found");
    let i = key_near(&p.keys, at, near).unwrap_or(0);
    let out = key_json(layer, i, &p.keys[i], p.def.kind);
    cx.label = Some(format!("Set key of {path}"));
    Ok(out)
}

fn key_ease(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (layer, path, near) = target(cx, a)?;
    let mut keys = layer.props.prop(&path).expect("found").keys.clone();
    let which = chosen(layer, &keys, a, near)?;
    let how = a.text("ease").unwrap_or("easy");
    for &i in &which {
        let k = &mut keys[i];
        match how {
            "easy" | "easyIn" | "easyOut" => {
                if how != "easyOut" {
                    k.interp_in = Interp::Bezier;
                    k.ease_in.clear();
                }
                if how != "easyIn" {
                    k.interp_out = Interp::Bezier;
                    k.ease_out.clear();
                }
                k.auto_bezier = false;
                k.continuous = false;
            }
            "linear" => {
                k.interp_in = Interp::Linear;
                k.interp_out = Interp::Linear;
                k.ease_in.clear();
                k.ease_out.clear();
                k.auto_bezier = false;
                k.continuous = false;
            }
            "hold" => {
                k.interp_in = Interp::Hold;
                k.interp_out = Interp::Hold;
            }
            "auto" => {
                k.interp_in = Interp::Bezier;
                k.interp_out = Interp::Bezier;
                k.auto_bezier = true;
                k.continuous = true;
            }
            _ => {
                k.interp_in = Interp::Bezier;
                k.interp_out = Interp::Bezier;
                k.auto_bezier = false;
                k.continuous = true;
                k.ease_in = k.ease_out.clone();
            }
        }
    }
    store(layer, &path, keys)?;
    let out = keys_json(layer, &path);
    cx.label = Some(format!("Ease keys of {path}"));
    Ok(out)
}

fn key_move(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (layer, path, near) = target(cx, a)?;
    let mut keys = layer.props.prop(&path).expect("found").keys.clone();
    let (which, shift) = match (a.num("time"), a.num("to"), a.num("by")) {
        (Some(from), Some(to), None) if !a.has("times") => {
            let at = layer.layer_time(Tick::from_seconds(from));
            let i = key_near(&keys, at, near).ok_or_else(|| Error::not_found(format!("there is no key at {from} s{}", list(layer, &keys))))?;
            (vec![i], layer.layer_time(Tick::from_seconds(to)) - keys[i].time)
        }
        (None, None, Some(by)) => {
            let which = chosen(layer, &keys, a, near)?;
            // in the layer's own time, an amount of composition time is stretched with it
            let shift = layer.layer_time(Tick::from_seconds(by)) - layer.layer_time(Tick::ZERO);
            (which, shift)
        }
        _ => return Err(Error::params("key.move takes `time` and `to` (one key), or `by` with `times` (default: all)")),
    };
    for &i in &which {
        keys[i].time += shift;
    }
    let mut times: Vec<Tick> = keys.iter().map(|k| k.time).collect();
    times.sort();
    if times.windows(2).any(|w| w[0] == w[1]) {
        return Err(Error::refused("that would put two keys at one moment"));
    }
    store(layer, &path, keys)?;
    let out = keys_json(layer, &path);
    cx.label = Some(format!("Move keys of {path}"));
    Ok(out)
}

fn key_remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view = cx.view.time;
    let (layer, path, near) = target(cx, a)?;
    let keys = layer.props.prop(&path).expect("found").keys.clone();
    let which = chosen(layer, &keys, a, near)?;
    let left: Vec<Keyframe> = keys.iter().enumerate().filter(|(i, _)| !which.contains(i)).map(|(_, k)| k.clone()).collect();
    if left.is_empty() {
        let now = layer.layer_time(view);
        let p = layer.props.prop_mut(&path).expect("found");
        let v = p.value_at(now);
        p.set_value(v).map_err(Error::params)?;
    } else {
        store(layer, &path, left)?;
    }
    let out = keys_json(layer, &path);
    cx.label = Some(format!("Remove keys of {path}"));
    Ok(out)
}

fn key_reverse(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (layer, path, near) = target(cx, a)?;
    let mut keys = layer.props.prop(&path).expect("found").keys.clone();
    let which = chosen(layer, &keys, a, near)?;
    if which.len() < 2 {
        return Err(Error::refused("reversing needs at least two keys"));
    }
    let mut part: Vec<Keyframe> = which.iter().map(|&i| keys[i].clone()).collect();
    veymelo_keyframe::reverse(&mut part);
    for (&i, k) in which.iter().zip(part) {
        keys[i] = k;
    }
    store(layer, &path, keys)?;
    let out = keys_json(layer, &path);
    cx.label = Some(format!("Reverse keys of {path}"));
    Ok(out)
}

fn key_copy(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let half = Tick(comp.frame_rate.frame_duration().0 / 2);
    let to_lid = match a.json("toLayer") {
        Some(j) => super::find_layer(comp, j)?,
        None => lid,
    };
    let path = a.text("path").unwrap_or("").to_string();
    let to_path = a.text("toPath").map_or_else(|| path.clone(), str::to_string);
    let (from, target) = (comp.layer(lid).expect("found"), comp.layer(to_lid).expect("found"));
    let p = from.props.prop(&path).map_err(Error::not_found)?;
    let q = target.props.prop(&to_path).map_err(Error::not_found)?;
    super::prop::position_in_use(from, p.def.id)?;
    super::prop::position_in_use(target, q.def.id)?;
    if p.keys.is_empty() {
        return Err(Error::refused(format!("{path} has no keys to copy")));
    }
    if !q.def.animatable {
        return Err(Error::refused(format!("{to_path} cannot be animated")));
    }
    if p.def.kind != q.def.kind {
        return Err(Error::refused(format!("{path} takes a {} and {to_path} a {}: keys go only between properties of one kind", p.def.kind.name(), q.def.kind.name())));
    }
    let which = chosen(from, &p.keys, a, half)?;
    // the keys at their composition times, moved so the first lands at `at`
    let first = from.comp_time(p.keys[which[0]].time);
    let shift = a.time("at").map_or(Tick::ZERO, |at| at - first);
    if lid == to_lid && path == to_path && shift == Tick::ZERO {
        return Err(Error::refused("they would land where they are; give `at`, `toPath` or `toLayer`"));
    }
    let copies: Vec<Keyframe> = which
        .iter()
        .map(|&i| {
            let k = &p.keys[i];
            Keyframe { time: target.layer_time(from.comp_time(k.time) + shift), ..k.clone() }
        })
        .collect();
    let mut keys: Vec<Keyframe> = q.keys.iter().filter(|k| !copies.iter().any(|c| (c.time - k.time).abs() <= half)).cloned().collect();
    keys.extend(copies);
    let name = target.name.clone();
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(to_lid).expect("found");
    store(layer, &to_path, keys)?;
    let out = keys_json(layer, &to_path);
    cx.label = Some(format!("Copy keys to {to_path} of {name}"));
    Ok(out)
}
