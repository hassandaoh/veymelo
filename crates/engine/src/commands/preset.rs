//! Animation presets: what makes a layer move (its keys, expressions, effects and text
//! animators) kept in a file and put on other layers, timed from where they go; and the presets
//! that come with the program: fades, slides, pops, blurs, loops, text that types itself on.
//!
//! A preset is JSON: `{"preset": 1, "name", "about", "category", "length", "at", "layer",
//! "effects", "textAnimators"}`. Each part (the layer, an effect, an animator, a selector) has
//! `props` (values), `keys` and `expressions` by path, and the `ease` its short keys take. A key
//! is a keyframe as prop.keys takes it, its time in seconds from the preset's start, or
//! `[f, value, ease?]` with `f` a fraction of the preset's length. A value may be worked out on
//! the layer it goes on: `{"current": true}` (its own value then), `{"by": d}`, `{"times": k}`,
//! `{"ofLayer": [fx, fy]}` (a point that far across and down the layer, as effects' points are)
//! or `{"ofLayer": k}` (a length, that much of the layer's diagonal), or for position
//! `{"off": "left"}` (just past that edge of the composition).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Interp, Keyframe, Kind, Value};
use veymelo_project::{Expression, Group, Layer, Node, Project, Source};
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, comp_id, find_layer, fresh_name, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

const BUILT_IN: &str = include_str!("../presets.json");
/// The format this program writes and reads.
const FORMAT: f64 = 1.0;
const EASES: &[&str] = &["easy", "easyIn", "easyOut", "linear", "hold"];
const EDGES: &[&str] = &["left", "right", "top", "bottom"];
const FOLDER: Param = Param::opt("folder", Ty::Text, "where saved presets are kept (default: $VEYMELO_PRESETS, or Presets in the program's Application Support folder)");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "anim.presets",
            label: "Animation Presets",
            summary: "The animation presets there are: those that come with the program (fades, slides, pops, blurs, loops, text) and those saved in the presets folder, each with what it changes and how long it takes.",
            params: const { &[Param::opt("category", Ty::Text, "only these: in, out, loop, text, or a saved preset's own"), FOLDER] },
            returns: "[{id, name, category, about, length, at, saved, path?, changes}]",
            run: Run::Look(list),
        },
        Command {
            id: "anim.applyPreset",
            label: "Apply Animation Preset",
            summary: "Put an animation preset on layers: its keys timed from `time` (by default an in preset starts at the layer's in point and an out preset ends at its out point) and stretched to `duration`, its effects and text animators added, its expressions set. Keys join the ones there (one at the same time is replaced). Values it works out from the layer (its own value, just past an edge) are worked out on each one.",
            params: const {
                &[
                    COMP,
                    Param::need("layers", Ty::List, "the layers (ids, names or \"#n\")"),
                    Param::need("preset", Ty::Any, "a preset's id or name (anim.presets lists them), a preset file's path, or a preset itself (an object as a preset file holds)"),
                    Param::opt("time", Ty::Time, "where it starts (an out preset: where it ends)"),
                    Param::opt("duration", Ty::Time, "how long its keys take (default: the preset's own length)"),
                    FOLDER,
                ]
            },
            returns: "{preset, applied: [{layer, from, to, changed}]}",
            run: Run::Edit(apply),
        },
        Command {
            id: "anim.savePreset",
            label: "Save Animation Preset",
            summary: "Keep what animates a layer as a preset file: the keys (their times from the first), expressions and values of the properties named, and its effects and text animators whole. Without `props`: everything with keys or an expression, every effect and every animator.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("name", Ty::Text, "what the preset is called"),
                    Param::opt("props", Ty::List, "what goes in: properties (\"transform/opacity\"), effects (\"effects/Gaussian Blur\"), animators (\"text/animators/Animator 1\")"),
                    Param::opt("path", Ty::Text, "the file (default: <name>.json in the presets folder)"),
                    FOLDER,
                    Param::opt("about", Ty::Text, "what it does, in a few words"),
                    Param::opt("category", Ty::Text, "its family, like in, out, loop or text"),
                    Param::opt("at", Ty::Choice(&["in", "out"]), "whether it is timed from its start (in, the default) or to its end (out) when it is put on a layer"),
                    Param::opt("from", Ty::Time, "the time its keys are measured from (default: the first key's)"),
                ]
            },
            returns: "{path, name, length, changes, warnings?}",
            run: Run::Look(save),
        },
    ]
}

fn built_ins() -> &'static [Json] {
    static ALL: OnceLock<Vec<Json>> = OnceLock::new();
    ALL.get_or_init(|| {
        let j = Json::parse(BUILT_IN).expect("the presets that come with the program are JSON");
        j.get("presets").and_then(Json::as_array).cloned().unwrap_or_default()
    })
}

fn folder(a: &Args) -> PathBuf {
    if let Some(f) = a.text("folder") {
        return PathBuf::from(f);
    }
    if let Some(p) = std::env::var_os("VEYMELO_PRESETS") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Application Support/Veymelo/Presets")
}

fn text<'a>(j: &'a Json, key: &str) -> Option<&'a str> {
    j.get(key).and_then(Json::as_str)
}

/// A preset file read, or what is wrong with it.
fn read_file(path: &Path) -> std::result::Result<Json, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let j = Json::parse(&s).map_err(|e| format!("{}: {e}", path.display()))?;
    match j.get("preset").and_then(Json::as_f64) {
        None => Err(format!("{} is not an animation preset (it has no \"preset\")", path.display())),
        Some(v) if v > FORMAT => Err(format!("{} was written by a newer program (format {v})", path.display())),
        Some(_) => Ok(j),
    }
}

/// The preset files in `dir` and below, in order.
fn saved(dir: &Path) -> Vec<(PathBuf, std::result::Result<Json, String>)> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "json") {
                files.push(p);
            }
        }
    }
    files.sort();
    files.into_iter().map(|p| {
        let j = read_file(&p);
        (p, j)
    }).collect()
}

fn stem(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
}

/// The preset `preset` names: a built-in's id or name, a file, or a saved one's name.
fn find_preset(a: &Args) -> Result<Json> {
    if let Some(Json::Object(m)) = a.json("preset") {
        if m.get("preset").and_then(Json::as_f64).is_some_and(|v| v > FORMAT) {
            return Err(Error::params("the preset was written for a newer program"));
        }
        return Ok(Json::Object(m.clone()));
    }
    let want = a.text("preset").ok_or_else(|| Error::params("`preset` is a preset's id, name or file, or a preset object"))?.trim();
    let all = built_ins();
    let hit = all.iter().find(|p| text(p, "id") == Some(want)).or_else(|| all.iter().find(|p| [text(p, "id"), text(p, "name")].iter().flatten().any(|n| n.eq_ignore_ascii_case(want))));
    if let Some(p) = hit {
        return Ok(p.clone());
    }
    if want.ends_with(".json") || want.contains('/') {
        return read_file(Path::new(want)).map_err(Error::not_found);
    }
    let dir = folder(a);
    let saved = saved(&dir);
    for (path, j) in &saved {
        let named = j.as_ref().ok().and_then(|j| text(j, "name")).is_some_and(|n| n.eq_ignore_ascii_case(want));
        if stem(path).eq_ignore_ascii_case(want) || named {
            return j.clone().map_err(Error::refused);
        }
    }
    let mut names: Vec<String> = all.iter().filter_map(|p| text(p, "id").map(str::to_string)).collect();
    names.extend(saved.iter().map(|(p, _)| stem(p)));
    Err(Error::not_found(match crate::params::nearest(want, names.iter().map(String::as_str)) {
        Some(n) => format!("there is no preset \"{want}\"; did you mean \"{n}\"? (anim.presets lists them)"),
        None => format!("there is no preset \"{want}\" (anim.presets lists them)"),
    }))
}

/// What a preset changes, in words: the layer's paths, then effects and animators.
fn changes(p: &Json) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(part) = p.get("layer") {
        for what in ["props", "keys", "expressions"] {
            if let Some(m) = part.get(what).and_then(Json::as_object) {
                for (path, _) in m.iter() {
                    if !out.iter().any(|o| o == path) {
                        out.push(path.to_string());
                    }
                }
            }
        }
    }
    for e in p.get("effects").and_then(Json::as_array).into_iter().flatten() {
        out.push(format!("effect {}", text(e, "type").unwrap_or("?")));
    }
    for an in p.get("textAnimators").and_then(Json::as_array).into_iter().flatten() {
        out.push(format!("text animator {}", text(an, "name").unwrap_or("Animator")));
    }
    out
}

fn entry(p: &Json, id: String, path: Option<&Path>) -> Json {
    let mut o = Map::new();
    o.insert("id", id);
    o.insert("name", text(p, "name").unwrap_or(""));
    o.insert("category", text(p, "category").unwrap_or(""));
    o.insert("about", text(p, "about").unwrap_or(""));
    o.insert("length", p.get("length").and_then(Json::as_f64).unwrap_or(0.0));
    o.insert("at", text(p, "at").unwrap_or("in"));
    o.insert("saved", path.is_some());
    if let Some(path) = path {
        o.insert("path", path.display().to_string());
    }
    o.insert("changes", Json::array(changes(p)));
    Json::Object(o)
}

fn list(_: &Look, a: &Args) -> Result<Json> {
    let only = a.text("category");
    let keep = |p: &Json| only.is_none_or(|c| text(p, "category").is_some_and(|k| k.eq_ignore_ascii_case(c)));
    let mut out: Vec<Json> = built_ins().iter().filter(|p| keep(p)).map(|p| entry(p, text(p, "id").unwrap_or("").to_string(), None)).collect();
    for (path, j) in saved(&folder(a)) {
        match j {
            Ok(p) if keep(&p) => out.push(entry(&p, stem(&path), Some(&path))),
            Ok(_) => {}
            Err(e) if only.is_none() => out.push(Json::object([("id", Json::from(stem(&path))), ("path", Json::from(path.display().to_string())), ("error", Json::from(e))])),
            Err(_) => {}
        }
    }
    Ok(Json::Array(out))
}

// ---- putting a preset on a layer

/// Where a preset goes: from `start` (composition seconds) for `duration`; the seconds its own
/// keys give, times `scale`.
#[derive(Clone, Copy)]
struct Timing {
    start: f64,
    duration: f64,
    scale: f64,
}

/// The layer a preset goes on, as it was before.
struct Env<'a> {
    before: &'a Project,
    cid: u64,
    layer: &'a Layer,
    timing: Timing,
    /// The layer's own box, in its pixels (where effects' points are measured).
    rect: veymelo_geom::Rect,
    /// The boxes it draws in, by time, once worked out.
    boxes: RefCell<Vec<(Tick, Option<[f64; 4]>)>>,
}

impl Env<'_> {
    /// The composition time `f` of the way through, or `secs` of the preset's own seconds in.
    fn at_fraction(&self, f: f64) -> f64 {
        self.timing.start + f * self.timing.duration
    }

    fn at_seconds(&self, secs: f64) -> f64 {
        self.timing.start + secs * self.timing.scale
    }

    /// How far to move the layer at composition time `t`, in its parent's space, for it to be
    /// just past `edge` of the composition.
    fn off(&self, edge: &str, t: Tick) -> Result<[f64; 2]> {
        let comp = self.before.comp(self.cid).expect("found");
        let frame = comp.frame_rate.frame_duration();
        let t = t.max(self.layer.in_point).min(self.layer.out_point - frame).max(self.layer.in_point);
        let cached = self.boxes.borrow().iter().find(|(at, _)| *at == t).map(|(_, b)| *b);
        let b = match cached {
            Some(b) => b,
            None => {
                let b = super::arrange::bounds(self.before, self.cid, self.layer.id, t)?;
                self.boxes.borrow_mut().push((t, b));
                b
            }
        };
        let b = b.ok_or_else(|| Error::refused(format!("\"{}\" draws nothing at {} s, so where its edge is cannot be known", self.layer.name, t.seconds())))?;
        let (w, h) = (comp.width as f64, comp.height as f64);
        let d = match edge {
            "left" => [-b[2], 0.0],
            "right" => [w - b[0], 0.0],
            "top" => [0.0, -b[3]],
            "bottom" => [0.0, h - b[1]],
            _ => return Err(Error::params(format!("\"off\" is one of {}, not \"{edge}\"", EDGES.join(", ")))),
        };
        Ok(super::arrange::to_parent(self.before, self.cid, self.layer.id, d, t))
    }
}

/// Where a part's current values come from: the layer as it was, or the group being made.
#[derive(Clone, Copy, PartialEq)]
enum Base {
    Layer,
    Fresh,
}

fn add(v: Value, d: &Json, at: &str) -> Result<Value> {
    let nums = |n: usize| -> Result<Vec<f64>> {
        let list = match d {
            Json::Number(x) => vec![*x],
            Json::Array(items) => items.iter().map(|x| x.as_f64().ok_or_else(|| Error::params(format!("{at}: \"by\" takes numbers")))).collect::<Result<_>>()?,
            _ => return Err(Error::params(format!("{at}: \"by\" is a number or a list of them"))),
        };
        if list.len() > n {
            return Err(Error::params(format!("{at}: \"by\" has {} numbers, the value {n}", list.len())));
        }
        Ok(list)
    };
    Ok(match v {
        Value::Number(x) => Value::Number(x + nums(1)?[0]),
        Value::Vec2(mut p) => {
            nums(2)?.iter().enumerate().for_each(|(i, d)| p[i] += d);
            Value::Vec2(p)
        }
        Value::Vec3(mut p) => {
            nums(3)?.iter().enumerate().for_each(|(i, d)| p[i] += d);
            Value::Vec3(p)
        }
        _ => return Err(Error::params(format!("{at}: \"by\" is for numbers and points"))),
    })
}

fn times(v: Value, k: &Json, at: &str) -> Result<Value> {
    let k = k.as_f64().ok_or_else(|| Error::params(format!("{at}: \"times\" is a number")))?;
    Ok(match v {
        Value::Number(x) => Value::Number(x * k),
        Value::Vec2(p) => Value::Vec2(p.map(|x| x * k)),
        Value::Vec3(p) => Value::Vec3(p.map(|x| x * k)),
        _ => return Err(Error::params(format!("{at}: \"times\" is for numbers and points"))),
    })
}

/// The value `v` names for a property of kind `kind`: as it is, or worked out from `now`.
fn resolve(v: &Json, kind: Kind, at: &str, rect: &veymelo_geom::Rect, now: &dyn Fn() -> Result<Value>, off: &dyn Fn(&str) -> Result<[f64; 2]>) -> Result<Value> {
    if let Json::Object(m) = v
        && m.len() == 1
        && let Some((how, x)) = m.iter().next()
        && ["current", "by", "times", "off", "ofLayer"].contains(&how)
    {
        return match how {
            "current" => now(),
            "by" => add(now()?, x, at),
            "times" => times(now()?, x, at),
            "ofLayer" => {
                let (w, h) = (rect.width(), rect.height());
                let f = |j: Option<&Json>| j.and_then(Json::as_f64).ok_or_else(|| Error::params(format!("{at}: \"ofLayer\" is [across, down] or a share of the diagonal")));
                match (x, kind) {
                    (Json::Array(p), Kind::Vec2) => Ok(Value::Vec2([rect.x0 + f(p.first())? * w, rect.y0 + f(p.get(1))? * h])),
                    (Json::Array(p), Kind::Vec3) => Ok(Value::Vec3([rect.x0 + f(p.first())? * w, rect.y0 + f(p.get(1))? * h, 0.0])),
                    (Json::Number(k), Kind::Number) => Ok(Value::Number(k * w.hypot(h))),
                    _ => Err(Error::params(format!("{at}: \"ofLayer\" is [across, down] for a point or a number for a length"))),
                }
            }
            _ => {
                let edge = x.as_str().ok_or_else(|| Error::params(format!("{at}: \"off\" names an edge: {}", EDGES.join(", "))))?;
                let d = off(edge)?;
                add(now()?, &Json::array(d), at)
            }
        };
    }
    Value::from_json(kind, v).map_err(|e| Error::params(format!("{at}: {e}")))
}

fn ease(k: &mut Keyframe, how: &str, at: &str) -> Result<()> {
    let (i, o) = match how {
        "easy" => (Interp::Bezier, Interp::Bezier),
        "easyIn" => (Interp::Bezier, Interp::Linear),
        "easyOut" => (Interp::Linear, Interp::Bezier),
        "linear" => (Interp::Linear, Interp::Linear),
        "hold" => (Interp::Hold, Interp::Hold),
        _ => return Err(Error::params(format!("{at}: the ease is one of {}, not \"{how}\"", EASES.join(", ")))),
    };
    (k.interp_in, k.interp_out) = (i, o);
    k.ease_in.clear();
    k.ease_out.clear();
    Ok(())
}

/// Speeds in a key's eases times `f` (its time between keys divided by `f`).
fn speeds(k: &mut Keyframe, f: f64) {
    if (f - 1.0).abs() > 1e-12 {
        k.ease_in.iter_mut().chain(k.ease_out.iter_mut()).for_each(|e| e.speed *= f);
    }
}

/// The properties a path names on this group: one, or a layer's position in separate
/// dimensions (each with which number of the position it takes).
fn targets(g: &Group, path: &str, separate: bool) -> Vec<(String, Option<usize>)> {
    if separate && path.trim_matches('/') == "transform/position" {
        let n = if g.prop("transform/positionZ").is_ok() { 3 } else { 2 };
        return ["transform/positionX", "transform/positionY", "transform/positionZ"][..n].iter().enumerate().map(|(i, p)| (p.to_string(), Some(i))).collect();
    }
    vec![(path.to_string(), None)]
}

fn component(v: &Value, i: usize) -> Value {
    match v {
        Value::Vec3(p) => Value::Number(p[i]),
        Value::Vec2(p) => Value::Number(p[i.min(1)]),
        other => other.clone(),
    }
}

fn only(part: &Json, at: &str, extra: &[&str]) -> Result<()> {
    let m = part.as_object().ok_or_else(|| Error::params(format!("{at} is an object")))?;
    for k in m.keys() {
        if !["props", "keys", "expressions", "ease"].contains(&k) && !extra.contains(&k) {
            let mut have = vec!["props", "keys", "expressions", "ease"];
            have.extend(extra);
            return Err(Error::params(format!("{at} has no \"{k}\" (it has {})", have.join(", "))));
        }
    }
    Ok(())
}

/// Put one part of a preset on a group: its values, keys and expressions. The paths changed.
fn put_part(g: &mut Group, part: &Json, at: &str, base: Base, env: &Env, separate: bool, extra: &[&str]) -> Result<Vec<String>> {
    only(part, at, extra)?;
    let short_ease = text(part, "ease").unwrap_or("linear");
    let mut changed = Vec::new();
    let layer = env.layer;
    let lt = |secs: f64| layer.layer_time(Tick::from_seconds(secs));
    // the value of `path` at composition time `t`, before the preset
    let current = |g: &Group, path: &str, t: Tick| -> Result<Value> {
        let read = |p: &str| -> Result<Value> {
            let src = if base == Base::Layer { &layer.props } else { g };
            Ok(src.prop(p).map_err(|e| Error::params(format!("{at}: {e}")))?.value_at(layer.layer_time(t)))
        };
        match targets(g, path, separate).as_slice() {
            [(p, None)] => read(p),
            many => {
                let mut v = [0.0; 3];
                for (p, i) in many {
                    if let (Value::Number(x), Some(i)) = (read(p)?, i) {
                        v[*i] = x;
                    }
                }
                Ok(Value::Vec3(v))
            }
        }
    };
    let kind_of = |g: &Group, path: &str| -> Result<(Kind, bool)> {
        match targets(g, path, separate).as_slice() {
            [(p, None)] => {
                let p = g.prop(p).map_err(|e| Error::params(format!("{at}: {e}")))?;
                Ok((p.def.kind, p.def.spatial))
            }
            _ => Ok((Kind::Vec3, true)),
        }
    };
    let is_position = |path: &str| matches!(path.trim_matches('/'), "transform/position" | "transform/positionX" | "transform/positionY");
    let mut place = |g: &mut Group, path: &str, value: Option<Value>, keys: Vec<Keyframe>| -> Result<()> {
        for (p, axis) in targets(g, path, separate) {
            if base == Base::Layer {
                super::prop::position_in_use(layer, p.rsplit('/').next().unwrap_or(""))?;
            }
            let prop = g.prop_mut(&p).map_err(|e| Error::params(format!("{at}: {e}")))?;
            let pick = |v: &Value| match axis {
                Some(i) => component(v, i),
                None => v.clone(),
            };
            if let Some(v) = &value {
                prop.set_value(pick(v)).map_err(|e| Error::params(format!("{at}/{path}: {e}")))?;
            }
            if !keys.is_empty() {
                let mut all = prop.keys.clone();
                for k in &keys {
                    let mut k = k.clone();
                    k.value = pick(&k.value);
                    if let Some(i) = axis {
                        k.ease_in = k.ease_in.get(i).or(k.ease_in.first()).copied().into_iter().collect();
                        k.ease_out = k.ease_out.get(i).or(k.ease_out.first()).copied().into_iter().collect();
                    }
                    veymelo_keyframe::set_key(&mut all, k);
                }
                all.sort_by_key(|k| k.time);
                if all.iter().any(|k| k.roving) {
                    veymelo_keyframe::retime_roving(&mut all, prop.def.spatial);
                }
                prop.set_keys(all).map_err(|e| Error::params(format!("{at}/{path}: {e}")))?;
            }
            changed.push(if at.is_empty() { p.clone() } else { format!("{at}/{p}") });
        }
        Ok(())
    };
    if let Some(m) = part.get("props").map(|j| j.as_object().ok_or_else(|| Error::params(format!("{at}: \"props\" is an object of values by path")))).transpose()? {
        for (path, v) in m.iter() {
            let t = Tick::from_seconds(env.timing.start);
            let (kind, _) = kind_of(g, path)?;
            let off = |edge: &str| if base == Base::Layer && is_position(path) { env.off(edge, t) } else { Err(Error::params(format!("{at}/{path}: \"off\" is for a layer's position"))) };
            let value = resolve(v, kind, &format!("{at}/{path}"), &env.rect, &|| current(g, path, t), &off)?;
            place(g, path, Some(value), Vec::new())?;
        }
    }
    if let Some(m) = part.get("keys").map(|j| j.as_object().ok_or_else(|| Error::params(format!("{at}: \"keys\" is an object of key lists by path")))).transpose()? {
        for (path, list) in m.iter() {
            let here = format!("{at}/{path}");
            let list = list.as_array().ok_or_else(|| Error::params(format!("{here}: the keys are a list")))?;
            let (kind, spatial) = kind_of(g, path)?;
            let mut keys = Vec::new();
            for k in list {
                let (secs, v, short) = match k {
                    Json::Array(f) => {
                        let at_f = f.first().and_then(Json::as_f64).filter(|x| x.is_finite()).ok_or_else(|| Error::params(format!("{here}: a short key is [fraction, value, ease?]")))?;
                        let v = f.get(1).ok_or_else(|| Error::params(format!("{here}: a short key is [fraction, value, ease?]")))?;
                        let e = match f.get(2) {
                            Some(e) => e.as_str().ok_or_else(|| Error::params(format!("{here}: a key's ease is a word: {}", EASES.join(", "))))?,
                            None => short_ease,
                        };
                        (env.at_fraction(at_f), v, Some(e))
                    }
                    Json::Object(o) => {
                        let t = o.get("t").and_then(Json::as_f64).filter(|x| x.is_finite()).ok_or_else(|| Error::params(format!("{here}: a key needs \"t\", seconds from the preset's start")))?;
                        (env.at_seconds(t), o.get("v").ok_or_else(|| Error::params(format!("{here}: a key needs \"v\"")))?, None)
                    }
                    _ => return Err(Error::params(format!("{here}: a key is a keyframe object or [fraction, value, ease?]"))),
                };
                let t = Tick::from_seconds(secs);
                let off = |edge: &str| if base == Base::Layer && is_position(path) { env.off(edge, t) } else { Err(Error::params(format!("{here}: \"off\" is for a layer's position"))) };
                let value = resolve(v, kind, &here, &env.rect, &|| current(g, path, t), &off)?;
                let key = match short {
                    Some(e) => {
                        let mut key = Keyframe::new(lt(secs), value);
                        ease(&mut key, e, &here)?;
                        if spatial {
                            // straight between keys, no swing past them
                            key.spatial_auto = false;
                        }
                        key
                    }
                    None => {
                        let mut o = k.as_object().expect("an object").clone();
                        o.insert("v", value.to_json(kind));
                        o.insert("t", 0.0);
                        let mut key = Keyframe::from_json(kind, &Json::Object(o)).map_err(|e| Error::params(format!("{here}: {e}")))?;
                        key.time = lt(secs);
                        speeds(&mut key, layer.stretch / 100.0 / env.timing.scale);
                        key
                    }
                };
                keys.push(key);
            }
            place(g, path, None, keys)?;
        }
    }
    if let Some(m) = part.get("expressions").map(|j| j.as_object().ok_or_else(|| Error::params(format!("{at}: \"expressions\" is an object of expressions by path")))).transpose()? {
        for (path, e) in m.iter() {
            let here = format!("{at}/{path}");
            let e = e.as_str().ok_or_else(|| Error::params(format!("{here}: an expression is text")))?;
            if targets(g, path, separate).len() > 1 {
                return Err(Error::refused(format!("\"{}\" has its position in separate dimensions; this preset's expression is for position as one (join them with layer.set separatePosition false)", layer.name)));
            }
            let prop = g.prop_mut(path).map_err(|e| Error::params(format!("{here}: {e}")))?;
            if !prop.def.animatable {
                return Err(Error::params(format!("{here} cannot have an expression")));
            }
            prop.expression = Some(Expression { text: e.to_string(), enabled: true });
            changed.push(if at.is_empty() { path.to_string() } else { format!("{at}/{path}") });
        }
    }
    changed.dedup();
    Ok(changed)
}

/// A name like `base` that none of `taken` has: itself, else numbered from 2.
fn unused(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base} {n}")).find(|n| !taken.contains(n)).expect("some number is free")
}

/// Put `p` on one layer. What changed.
fn put(cx: &mut Ctx, env: &Env, p: &Json) -> Result<Vec<String>> {
    let (cid, lid) = (env.cid, env.layer.id);
    let layer_b = env.layer;
    let effects = p.get("effects").map(|e| e.as_array().cloned().ok_or_else(|| Error::params("\"effects\" is a list"))).transpose()?.unwrap_or_default();
    let animators = p.get("textAnimators").map(|e| e.as_array().cloned().ok_or_else(|| Error::params("\"textAnimators\" is a list"))).transpose()?.unwrap_or_default();
    if !animators.is_empty() && layer_b.source != Source::Text {
        return Err(Error::refused(format!("{} is for text layers; \"{}\" is a {} layer", text(p, "name").unwrap_or("the preset"), layer_b.name, layer_b.source.id())));
    }
    let mut changed = Vec::new();
    // the layer's own properties
    if let Some(part) = p.get("layer") {
        let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
        let separate = layer.separate_position;
        changed.extend(put_part(&mut layer.props, part, "", Base::Layer, env, separate, &[])?);
    }
    // effects, made whole then added
    if !effects.is_empty() {
        let mut taken: Vec<String> = match layer_b.props.group("effects") {
            Ok(list) => list.children.iter().map(|c| c.name().to_string()).collect(),
            Err(_) => return Err(Error::refused(format!("\"{}\" has no pixels of its own, so it takes no effects", layer_b.name))),
        };
        let rect = super::effect::bounds(env.before, cid, layer_b);
        let mut made = Vec::new();
        for (i, e) in effects.iter().enumerate() {
            let at = format!("effects #{}", i + 1);
            let ty = text(e, "type").ok_or_else(|| Error::params(format!("{at}: an effect needs its \"type\"")))?;
            let def = cx.catalog.find("effects", ty).ok_or_else(|| Error::params(format!("{at}: there is no effect \"{ty}\" (effect.types lists them)")))?;
            let name = unused(text(e, "name").unwrap_or(def.name), &taken);
            taken.push(name.clone());
            let mut g = Group::instance(def, cx.project.alloc(), &name);
            if let Some(spec) = veymelo_effects::find(def.id) {
                spec.start(&mut g, rect);
            }
            if let Some(on) = e.get("enabled") {
                g.enabled = on.as_bool().ok_or_else(|| Error::params(format!("{at}: \"enabled\" is true or false")))?;
            }
            put_part(&mut g, e, &format!("effects/{name}"), Base::Fresh, env, false, &["type", "name", "enabled"])?;
            changed.push(format!("effects/{name}"));
            made.push(g);
        }
        let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
        layer.props.group_mut("effects").expect("it takes effects").children.extend(made.into_iter().map(Node::Group));
    }
    // text animators with their selectors
    if !animators.is_empty() {
        let mut taken: Vec<String> = layer_b.props.group("text/animators").map(|l| l.children.iter().map(|c| c.name().to_string()).collect()).unwrap_or_default();
        let def = cx.catalog.find("animators", "animator").expect("in the catalog");
        let mut made = Vec::new();
        for (i, an) in animators.iter().enumerate() {
            let at = format!("textAnimators #{}", i + 1);
            let name = unused(text(an, "name").unwrap_or("Animator 1"), &taken);
            taken.push(name.clone());
            let here = format!("text/animators/{name}");
            let mut g = Group::instance(def, cx.project.alloc(), &name);
            put_part(&mut g, an, &here, Base::Fresh, env, false, &["name", "selectors"])?;
            let selectors = an.get("selectors").map(|s| s.as_array().cloned().ok_or_else(|| Error::params(format!("{at}: \"selectors\" is a list")))).transpose()?.unwrap_or_default();
            for s in &selectors {
                let ty = text(s, "type").unwrap_or("range");
                let sdef = cx.catalog.find("selectors", ty).filter(|_| ["range", "wiggly"].contains(&ty)).ok_or_else(|| Error::params(format!("{at}: a selector is range or wiggly, not \"{ty}\"")))?;
                let id = cx.project.alloc();
                let list = g.group_mut("selectors").expect("defined");
                let sname = match text(s, "name") {
                    Some(n) => n.to_string(),
                    None => fresh_name(sdef.name, list.children.iter().map(Node::name)),
                };
                let mut sg = Group::instance(sdef, id, &sname);
                put_part(&mut sg, s, &format!("{here}/selectors/{sname}"), Base::Fresh, env, false, &["type", "name"])?;
                g.group_mut("selectors").expect("defined").children.push(Node::Group(sg));
            }
            changed.push(here);
            made.push(g);
        }
        let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
        layer.props.group_mut("text/animators").expect("a text layer").children.extend(made.into_iter().map(Node::Group));
    }
    if changed.is_empty() {
        return Err(Error::params(format!("{} changes nothing", text(p, "name").unwrap_or("the preset"))));
    }
    Ok(changed)
}

fn apply(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let p = find_preset(a)?;
    let cid = comp_id(cx.project, cx.view, a)?;
    let mut ids = Vec::new();
    {
        let comp = cx.project.comp(cid).expect("found");
        for j in a.json("layers").and_then(Json::as_array).cloned().unwrap_or_default() {
            let id = find_layer(comp, &j)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    if ids.is_empty() {
        return Err(Error::params("give a layer or more"));
    }
    let length = p.get("length").and_then(Json::as_f64).filter(|l| l.is_finite() && *l >= 0.0).unwrap_or(0.0);
    let duration = a.time("duration").map_or(length, Tick::seconds);
    if duration < 0.0 || (duration == 0.0 && length > 0.0) {
        return Err(Error::params("`duration` is a time above 0"));
    }
    let scale = if length > 0.0 { duration / length } else { 1.0 };
    let at_out = text(&p, "at") == Some("out");
    let before = cx.project.clone();
    let comp = before.comp(cid).expect("found");
    let mut applied = Vec::new();
    for lid in ids {
        let layer = comp.layer(lid).expect("found");
        let start = match a.time("time") {
            Some(t) => t.seconds() - if at_out { duration } else { 0.0 },
            None if at_out => layer.out_point.seconds() - duration,
            None => layer.in_point.seconds(),
        };
        let rect = super::effect::bounds(&before, cid, layer);
        let env = Env { before: &before, cid, layer, timing: Timing { start, duration, scale }, rect, boxes: RefCell::new(Vec::new()) };
        let changed = put(cx, &env, &p).map_err(|e| Error { message: format!("on \"{}\": {}", layer.name, e.message), ..e })?;
        let mut o = Map::new();
        o.insert("layer", lid as f64);
        o.insert("from", start);
        o.insert("to", start + duration);
        o.insert("changed", Json::array(changed));
        applied.push(Json::Object(o));
    }
    let name = text(&p, "name").unwrap_or("Animation Preset").to_string();
    cx.label = Some(format!("Apply {name}"));
    Ok(Json::object([("preset", Json::from(name)), ("applied", Json::Array(applied))]))
}

// ---- keeping one

/// What of a group goes in a preset: values not as they start (or every one `all`), keys timed
/// from `from`, expressions. Lists inside are left out (named in `warnings`).
fn part_of(g: &Group, layer: &Layer, from: Tick, pick: &dyn Fn(&str) -> bool, statics: &dyn Fn(&str) -> bool, warnings: &mut Vec<String>, at: &str) -> Map {
    let (mut props, mut keys, mut exprs) = (Map::new(), Map::new(), Map::new());
    let mut walk = vec![(String::new(), g)];
    while let Some((prefix, g)) = walk.pop() {
        for c in g.children.iter().rev() {
            match c {
                Node::Group(sub) if sub.is_list() => {
                    if !sub.children.is_empty() && !(at.starts_with("text/animators") && sub.def.id == "selectors") {
                        warnings.push(format!("{at}{prefix}{} is a list; it is not kept", sub.def.id));
                    }
                }
                Node::Group(sub) => walk.push((format!("{prefix}{}/", sub.def.id), sub)),
                Node::Prop(p) => {
                    let path = format!("{prefix}{}", p.def.id);
                    if !pick(&path) {
                        continue;
                    }
                    if matches!(p.value, Value::Layer(Some(_))) {
                        warnings.push(format!("{at}{path} names a layer of this project; it is not kept"));
                        continue;
                    }
                    if !p.keys.is_empty() {
                        let list = p.keys.iter().map(|k| {
                            let mut k = k.clone();
                            k.time = layer.comp_time(k.time) - from;
                            speeds(&mut k, 100.0 / layer.stretch);
                            k.to_json(p.def.kind)
                        });
                        keys.insert(path.as_str(), Json::array(list));
                    } else if p.expression.is_none() && statics(&path) && (at.is_empty() || p.value != p.def.default) {
                        // a layer's value as named; an effect's where it is not as it starts
                        props.insert(path.as_str(), p.value.to_json(p.def.kind));
                    }
                    if let Some(e) = &p.expression {
                        exprs.insert(path.as_str(), e.text.as_str());
                    }
                }
            }
        }
    }
    let mut o = Map::new();
    for (k, m) in [("props", props), ("keys", keys), ("expressions", exprs)] {
        if !m.is_empty() {
            o.insert(k, Json::Object(m));
        }
    }
    o
}

/// The comp times of every key below `g`.
fn key_times(g: &Group, layer: &Layer, out: &mut Vec<Tick>) {
    g.walk(&mut |_, p| out.extend(p.keys.iter().map(|k| layer.comp_time(k.time))));
}

fn file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { '-' }).collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "preset".into() } else { s }
}

fn save(l: &Look, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(l.project, l.view, a)?;
    let layer = l.project.comp(cid).expect("found").layer(lid).expect("found");
    let name = a.text("name").unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err(Error::params("a preset needs a name"));
    }
    // what goes in: paths of the layer's properties, and effects and animators whole
    let (mut paths, mut effects, mut animators): (Vec<String>, Vec<&Group>, Vec<&Group>) = (Vec::new(), Vec::new(), Vec::new());
    let named = a.json("props").and_then(Json::as_array).cloned();
    let whole = |list: &str, rest: &str| -> Result<&Group> {
        let seg = rest.split('/').next().unwrap_or("");
        layer.props.group(&format!("{list}/{seg}")).map_err(Error::not_found)
    };
    match &named {
        Some(list) => {
            for j in list {
                let path = j.as_str().ok_or_else(|| Error::params(format!("`props` are paths, not {}", j.to_json())))?.trim_matches('/');
                if let Some(rest) = path.strip_prefix("effects/") {
                    let g = whole("effects", rest)?;
                    if !effects.iter().any(|e| e.id == g.id) {
                        effects.push(g);
                    }
                } else if let Some(rest) = path.strip_prefix("text/animators/") {
                    let g = whole("text/animators", rest)?;
                    if !animators.iter().any(|e| e.id == g.id) {
                        animators.push(g);
                    }
                } else {
                    // by the ids the walk gives, whatever case or names it was written in
                    let found: Vec<*const veymelo_project::Property> = match layer.props.node(path).map_err(Error::not_found)? {
                        Node::Prop(p) => vec![p as *const _],
                        Node::Group(g) if !g.is_list() => {
                            let mut v = Vec::new();
                            g.walk(&mut |_, p| v.push(p as *const _));
                            v
                        }
                        Node::Group(_) => return Err(Error::params(format!("{path} is a list; name what in it to keep"))),
                    };
                    layer.props.walk(&mut |q, p| {
                        let listed = q.split('/').any(|s| s.starts_with('@') || s.starts_with('#'));
                        if !listed && found.contains(&(p as *const _)) && !paths.iter().any(|x| x == q) {
                            paths.push(q.to_string());
                        }
                    });
                }
            }
        }
        None => {
            layer.props.walk(&mut |path, p| {
                let listed = path.split('/').any(|s| s.starts_with('@') || s.starts_with('#'));
                if !listed && (p.is_animated() || p.expression.is_some()) {
                    paths.push(path.to_string());
                }
            });
            if let Ok(list) = layer.props.group("effects") {
                effects.extend(list.children.iter().filter_map(Node::as_group));
            }
            if let Ok(list) = layer.props.group("text/animators") {
                animators.extend(list.children.iter().filter_map(Node::as_group));
            }
        }
    }
    if paths.is_empty() && effects.is_empty() && animators.is_empty() {
        return Err(Error::refused(format!("nothing on \"{}\" is animated; name what to keep with `props`", layer.name)));
    }
    // measured from the first key
    let mut ts = Vec::new();
    layer.props.walk(&mut |path, p| {
        if paths.iter().any(|q| q == path) {
            ts.extend(p.keys.iter().map(|k| layer.comp_time(k.time)));
        }
    });
    for g in effects.iter().chain(&animators) {
        key_times(g, layer, &mut ts);
    }
    let from = a.time("from").or_else(|| ts.iter().min().copied()).unwrap_or(layer.in_point);
    let length = ts.iter().max().map_or(0.0, |t| (*t - from).seconds().max(0.0));
    let mut warnings = Vec::new();
    let explicit = named.is_some();
    let mut o = Map::new();
    o.insert("preset", FORMAT);
    o.insert("name", name.as_str());
    if let Some(s) = a.text("about") {
        o.insert("about", s);
    }
    if let Some(s) = a.text("category") {
        o.insert("category", s);
    }
    o.insert("length", (length * 1e6).round() / 1e6);
    if a.text("at") == Some("out") {
        o.insert("at", "out");
    }
    if !paths.is_empty() {
        let part = part_of(&layer.props, layer, from, &|p| paths.iter().any(|q| q == p), &|_| explicit, &mut warnings, "");
        if !part.is_empty() {
            o.insert("layer", Json::Object(part));
        }
    }
    if !effects.is_empty() {
        let list = effects.iter().map(|g| {
            let mut e = Map::new();
            e.insert("type", g.def.id);
            e.insert("name", g.name());
            if !g.enabled {
                e.insert("enabled", false);
            }
            let at = format!("effects/{}/", g.name());
            for (k, v) in part_of(g, layer, from, &|_| true, &|_| true, &mut warnings, &at).iter() {
                e.insert(k, v.clone());
            }
            Json::Object(e)
        });
        o.insert("effects", Json::array(list));
    }
    if !animators.is_empty() {
        let list = animators.iter().map(|g| {
            let mut an = Map::new();
            an.insert("name", g.name());
            let at = format!("text/animators/{}/", g.name());
            for (k, v) in part_of(g, layer, from, &|_| true, &|_| true, &mut warnings, &at).iter() {
                an.insert(k, v.clone());
            }
            let sels = g.group("selectors").map(|s| s.children.iter().filter_map(Node::as_group).collect::<Vec<_>>()).unwrap_or_default();
            let sels = sels.iter().map(|s| {
                let mut sj = Map::new();
                sj.insert("type", s.def.id);
                sj.insert("name", s.name());
                let sat = format!("{at}selectors/{}/", s.name());
                for (k, v) in part_of(s, layer, from, &|_| true, &|_| true, &mut warnings, &sat).iter() {
                    sj.insert(k, v.clone());
                }
                Json::Object(sj)
            });
            an.insert("selectors", Json::array(sels.collect::<Vec<_>>()));
            Json::Object(an)
        });
        o.insert("textAnimators", Json::array(list));
    }
    let preset = Json::Object(o);
    let path = match a.text("path") {
        Some(p) => PathBuf::from(p),
        None => folder(a).join(format!("{}.json", file_name(&name))),
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| Error::refused(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(&path, preset.to_pretty() + "\n").map_err(|e| Error::refused(format!("{}: {e}", path.display())))?;
    let mut out = Map::new();
    out.insert("path", path.display().to_string());
    out.insert("name", name);
    out.insert("length", (length * 1e6).round() / 1e6);
    out.insert("changes", Json::array(changes(&preset)));
    if !warnings.is_empty() {
        out.insert("warnings", Json::array(warnings));
    }
    Ok(Json::Object(out))
}
