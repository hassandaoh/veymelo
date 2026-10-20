//! Tools for properties beyond giving them values: one made to follow another by an expression
//! (what a pick whip writes), an expression turned into keys, a property put back as it starts,
//! the stopwatch; and for expressions, the ones that fail and what they can say.

use veymelo_expr::Eval;
use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Interp, Keyframe, Kind, Value, keys_to_json};
use veymelo_project::{Expression, Group, Layer, Property, Source};
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, find_layer, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

const PATH: Param = Param::need("path", Ty::Text, "the property, like transform/position");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "prop.link",
            label: "Link Property",
            summary: "Make a property follow another, on this layer or another, with an expression, as a pick whip writes it: thisComp.layer(\"Null 1\").transform.position, effect(\"Blur\")(\"Blurriness\"). One number followed by a list goes to each of its numbers; a list followed by one number gives its first (or the one `dimension` picks).",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    Param::need("toPath", Ty::Text, "the property to follow"),
                    Param::opt("toLayer", Ty::Layer, "its layer (default: the same one)"),
                    Param::opt("dimension", Ty::Number, "follow only this one of its numbers, from 0"),
                ]
            },
            returns: "{path, expression, value}",
            run: Run::Edit(prop_link),
        },
        Command {
            id: "prop.bake",
            label: "Convert Expression to Keyframes",
            summary: "Turn a property's expression into a key at every frame (or every `step` frames) from when the layer starts to when it ends, or between `from` and `to`; the expression is switched off, kept.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    Param::opt("from", Ty::Time, "the first key (default: where the layer starts)"),
                    Param::opt("to", Ty::Time, "the last (default: where it ends)"),
                    Param::opt("step", Ty::Number, "frames between keys (default 1)"),
                ]
            },
            returns: "{path, keys}",
            run: Run::Edit(prop_bake),
        },
        Command {
            id: "prop.reset",
            label: "Reset Property",
            summary: "Put a property back as it starts on this layer (an effect's point in the layer's middle, the anchor in the middle of what the layer shows, the position in the middle of the composition), its expression taken away; one with keys gets a key at the view's time instead. A group (transform, an effect) puts back everything in it.",
            params: const { &[COMP, LAYER, Param::need("path", Ty::Text, "the property or group, like transform/scale or transform")] },
            returns: "{reset: [path]}",
            run: Run::Edit(prop_reset),
        },
        Command {
            id: "prop.animate",
            label: "Toggle Animation",
            summary: "The stopwatch: on gives a property a key at the view's time with the value it has (so it can be keyed from then on); off takes its keys away, keeping the value it has at the view's time.",
            params: const { &[COMP, LAYER, PATH, Param::need("on", Ty::Bool, "keys (true) or none (false)")] },
            returns: "{path, value, keys?}",
            run: Run::Edit(prop_animate),
        },
        Command {
            id: "expr.errors",
            label: "Expression Errors",
            summary: "Every expression of a composition that fails at a time, on every layer, with what is wrong; and those switched off that are not written right.",
            params: const { &[COMP, Param::opt("time", Ty::Time, "when (default: the view's time)")] },
            returns: "{time, count, errors: [{layer, index, name, path, message, off?}]}",
            run: Run::Look(expr_errors),
        },
        Command {
            id: "expr.reference",
            label: "Expression Reference",
            summary: "What expressions can say, by category: how each thing is written, what it gives, and an example that works.",
            params: const { &[Param::opt("category", Ty::Text, "only this category (Global, Time, Vector math, Random, Interpolation, Colour, Angles, Property, Layer, Composition, Property group, Markers, Path, JavaScript)")] },
            returns: "[{category, entries: [{write, about, example}]}]",
            run: Run::Look(expr_reference),
        },
    ]
}

/// `X Position` → `xPosition`, `Mask Opacity` → `maskOpacity`.
fn camel(name: &str) -> String {
    let mut out = String::new();
    for (i, w) in name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).enumerate() {
        let mut cs = w.chars();
        if let Some(f) = cs.next() {
            if i == 0 {
                out.extend(f.to_lowercase());
            } else {
                out.extend(f.to_uppercase());
            }
            out.push_str(cs.as_str());
        }
    }
    out
}

/// Text as an expression writes it, in quotes.
fn quote(s: &str) -> String {
    Json::from(s).to_json()
}

/// The groups along `path` of `layer`, outermost first, and the property at its end.
fn along<'a>(layer: &'a Layer, path: &str) -> Result<(Vec<&'a Group>, &'a Property)> {
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    let mut groups = Vec::new();
    for k in 1..segs.len() {
        groups.push(layer.props.group(&segs[..k].join("/")).map_err(Error::not_found)?);
    }
    Ok((groups, layer.props.prop(path).map_err(Error::not_found)?))
}

/// How a property of `from` reads property `path` of `target`, as a pick whip writes it:
/// transform.position, mask("Mask 1").maskOpacity, effect("Blur")("Blurriness"),
/// content("Group 1").content("Rectangle 1").size; another layer by its name.
pub(super) fn reference(from: &Layer, target: &Layer, path: &str) -> Result<String> {
    let (groups, prop) = along(target, path)?;
    let mut out = if from.id == target.id { String::new() } else { format!("thisComp.layer({})", quote(&target.name)) };
    let push = |out: &mut String, part: String| {
        if !out.is_empty() && !part.starts_with('(') {
            out.push('.');
        }
        out.push_str(&part);
    };
    let (mut i, mut in_effect) = (0, false);
    while i < groups.len() {
        let (g, next) = (groups[i], groups.get(i + 1));
        match (g.def.id, next) {
            ("masks", Some(m)) => push(&mut out, format!("mask({})", quote(m.name()))),
            ("effects", Some(fx)) => {
                push(&mut out, format!("effect({})", quote(fx.name())));
                in_effect = true;
            }
            ("contents", Some(c)) => push(&mut out, format!("content({})", quote(c.name()))),
            _ if in_effect || g.id != 0 => push(&mut out, format!("({})", quote(g.name()))),
            _ => push(&mut out, camel(g.name())),
        }
        i += if matches!((g.def.id, next), ("masks" | "effects" | "contents", Some(_))) { 2 } else { 1 };
    }
    let name = prop.own.as_ref().map_or(prop.def.name, |o| o.name.as_str());
    push(&mut out, if in_effect || prop.own.is_some() { format!("({})", quote(name)) } else { camel(name) });
    Ok(out)
}

/// The same read the plainest way, each step by its id (or an instance's name), which any
/// property can be: thisLayer("transform")("position").
pub(super) fn plain_reference(from: &Layer, target: &Layer, path: &str) -> Result<String> {
    let (groups, prop) = along(target, path)?;
    let mut out = if from.id == target.id { "thisLayer".to_string() } else { format!("thisComp.layer({})", quote(&target.name)) };
    for g in groups {
        out.push_str(&format!("({})", quote(if g.id != 0 { g.name() } else { g.def.id })));
    }
    out.push_str(&format!("({})", quote(prop.own.as_ref().map_or(prop.def.id, |o| o.name.as_str()))));
    Ok(out)
}

/// How many numbers a property's value has as an expression sees it (a 2D layer's position,
/// anchor and scale have two).
fn dims(layer: &Layer, p: &Property) -> usize {
    match p.def.kind {
        Kind::Vec3 if !layer.is_3d() => 2,
        Kind::Vec3 => 3,
        Kind::Vec2 => 2,
        Kind::Color => 4,
        _ => 1,
    }
}

/// What an expression on property `path` of layer `lid` gives now, or what goes wrong.
fn try_expression(cx: &mut Ctx, cid: u64, lid: u64, path: &str, text: &str) -> std::result::Result<Json, String> {
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    layer.props.prop_mut(path).expect("found").expression = Some(Expression { text: text.to_string(), enabled: true });
    let project: &veymelo_project::Project = cx.project;
    let comp = project.comp(cid).expect("found");
    let layer = comp.layer(lid).expect("found");
    let p = layer.props.prop(path).expect("found");
    let e = Eval::new(project);
    e.work(p, e.place(comp, layer), cx.view.time).map(|v| v.to_json(p.def.kind)).map_err(|(_, why)| why)
}

fn prop_link(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let to_lid = match a.json("toLayer") {
        Some(j) => find_layer(comp, j)?,
        None => lid,
    };
    let (path, to_path) = (a.text("path").unwrap_or("").to_string(), a.text("toPath").unwrap_or("").to_string());
    let (from, target) = (comp.layer(lid).expect("found"), comp.layer(to_lid).expect("found"));
    let me = from.props.prop(&path).map_err(Error::not_found)?;
    let them = target.props.prop(&to_path).map_err(Error::not_found)?;
    if std::ptr::eq(me, them) {
        return Err(Error::refused("a property cannot follow itself"));
    }
    if !me.def.animatable {
        return Err(Error::refused(format!("{path} is a setting, not animated, so it cannot follow another")));
    }
    super::prop::position_in_use(from, me.def.id)?;
    super::prop::position_in_use(target, them.def.id)?;
    let (mine, theirs) = (dims(from, me), dims(target, them));
    let pick = match a.num("dimension") {
        Some(d) if d >= 0.0 && d.fract() == 0.0 && (d as usize) < theirs && theirs > 1 => Some(d as usize),
        Some(d) => return Err(Error::params(format!("{to_path} has {theirs} number{}: `dimension` is from 0 to {}, not {d}", if theirs == 1 { "" } else { "s" }, theirs.saturating_sub(1)))),
        None => None,
    };
    let shape = |r: String| -> String {
        let (r, theirs) = match pick {
            Some(d) => (format!("{r}[{d}]"), 1),
            None => (r, theirs),
        };
        let numbers = |k: Kind| matches!(k, Kind::Number | Kind::Vec2 | Kind::Vec3 | Kind::Color);
        if !numbers(me.def.kind) || !numbers(them.def.kind) {
            r
        } else if theirs == 1 && mine > 1 {
            format!("temp = {r};\n[{}]", vec!["temp"; mine].join(", "))
        } else if theirs > 1 && mine == 1 {
            format!("{r}[0]")
        } else {
            r
        }
    };
    let (short, long) = (shape(reference(from, target, &to_path)?), shape(plain_reference(from, target, &to_path)?));
    let layer_name = from.name.clone();
    let (text, value) = match try_expression(cx, cid, lid, &path, &short) {
        Ok(v) => (short, v),
        Err(first) => match try_expression(cx, cid, lid, &path, &long) {
            Ok(v) => (long, v),
            Err(_) => return Err(Error::refused(format!("{path} cannot follow {to_path}: {first}"))),
        },
    };
    cx.label = Some(format!("Link {path} of {layer_name}"));
    Ok(Json::object([("path", Json::from(path.as_str())), ("expression", Json::from(text)), ("value", value)]))
}

fn prop_bake(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let path = a.text("path").unwrap_or("").to_string();
    let project: &veymelo_project::Project = cx.project;
    let comp = project.comp(cid).expect("found");
    let layer = comp.layer(lid).expect("found");
    let p = layer.props.prop(&path).map_err(Error::not_found)?;
    if !p.expression.as_ref().is_some_and(|x| x.enabled && !x.text.trim().is_empty()) {
        return Err(Error::refused(format!("{path} has no expression switched on to turn into keys")));
    }
    let frame = comp.frame_rate.frame_duration();
    let step = a.num("step").unwrap_or(1.0);
    if !(step >= 1.0 && step.fract() == 0.0 && step <= 10_000.0) {
        return Err(Error::params(format!("`step` is a whole number of frames from 1, not {step}")));
    }
    let (from, to) = (a.time("from").unwrap_or(layer.in_point), a.time("to").unwrap_or(layer.out_point - frame));
    if to < from {
        return Err(Error::params(format!("`to` ({} s) comes before `from` ({} s)", to.seconds(), from.seconds())));
    }
    let count = (to - from).0 / (frame.0 * step as i64) + 1;
    if count > 100_000 {
        return Err(Error::params(format!("that is {count} keys; give a bigger `step` or a shorter stretch")));
    }
    let e = Eval::new(project);
    let at = e.place(comp, layer);
    // values that cannot be in between hold until the next key
    let hold = matches!(p.def.kind, Kind::Bool | Kind::Choice(_) | Kind::Text | Kind::Layer | Kind::Doc);
    let mut keys: Vec<Keyframe> = (0..count)
        .map(|k| {
            let t = from + Tick(k * frame.0 * step as i64);
            let mut key = Keyframe::new(layer.layer_time(t), e.value(p, at, t));
            if hold {
                key.interp_in = Interp::Hold;
                key.interp_out = Interp::Hold;
            }
            key
        })
        .collect();
    // a layer turned around in time meets its own times backwards
    keys.sort_by_key(|k| k.time);
    keys.dedup_by_key(|k| k.time);
    let name = layer.name.clone();
    let p = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found").props.prop_mut(&path).expect("found");
    p.set_keys(keys).map_err(Error::params)?;
    if let Some(x) = &mut p.expression {
        x.enabled = false;
    }
    let out = Json::object([("path", Json::from(path.as_str())), ("keys", Json::Number(p.keys.len() as f64))]);
    cx.label = Some(format!("Convert Expression to Keyframes of {path} of {name}"));
    Ok(out)
}

/// What a property starts as on its layer: an effect's setting as the effect starts there,
/// the transform as a new layer's, anything else its default. None for what has nothing to go
/// back to (words, paths).
fn start_value(project: &veymelo_project::Project, cid: u64, layer: &Layer, path: &str) -> Option<Value> {
    let p = layer.props.prop(path).ok()?;
    if matches!(p.def.kind, Kind::Doc | Kind::Path) {
        return None;
    }
    let comp = project.comp(cid)?;
    let (w, h) = (comp.width as f64, comp.height as f64);
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    if segs.first() == Some(&"effects") && segs.len() >= 3 {
        let fx = layer.props.group(&segs[..2].join("/")).ok()?;
        let mut fresh = Group::instance(fx.def, fx.id, fx.name());
        if let Some(spec) = veymelo_effects::find(fx.def.id) {
            spec.start(&mut fresh, super::effect::bounds(project, cid, layer));
        }
        return fresh.prop(&segs[2..].join("/")).ok().map(|q| q.value.clone());
    }
    if segs.first() == Some(&"transform") {
        let zoom = layer.props.prop("cameraOptions/zoom").map_or(w, |z| z.value_at(Tick::ZERO).as_f64());
        let position = match layer.source {
            Source::Camera => [w / 2.0, h / 2.0, -zoom],
            Source::Light(_) => [w / 2.0 - w / 6.0, h / 2.0 - h / 4.0, -w / 3.0],
            _ => [w / 2.0, h / 2.0, 0.0],
        };
        let (sw, sh) = super::layer::natural_size(project, layer);
        match p.def.id {
            "anchor" => return Some(Value::Vec3([sw / 2.0, sh / 2.0, 0.0])),
            "position" => return Some(Value::Vec3(position)),
            "positionX" => return Some(Value::Number(position[0])),
            "positionY" => return Some(Value::Number(position[1])),
            "positionZ" => return Some(Value::Number(position[2])),
            "pointOfInterest" => return Some(Value::Vec3([w / 2.0, h / 2.0, 0.0])),
            _ => {}
        }
    }
    Some(p.def.default.clone())
}

fn prop_reset(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let path = a.text("path").unwrap_or("").trim_matches('/').to_string();
    let project: &veymelo_project::Project = cx.project;
    let layer = project.comp(cid).expect("found").layer(lid).expect("found");
    // the properties to put back: this one, or every one in this group
    let mut paths: Vec<String> = Vec::new();
    match (layer.props.prop(&path), layer.props.group(&path)) {
        (Ok(_), _) => paths.push(path.clone()),
        (Err(_), Ok(g)) => g.walk(&mut |p, _| paths.push(format!("{path}/{p}"))),
        (Err(why), Err(_)) => return Err(Error::not_found(why)),
    }
    let mut starts: Vec<(String, Value)> = Vec::new();
    for p in &paths {
        let used = layer.props.prop(p).map(|q| super::prop::position_in_use(layer, q.def.id).is_ok()).unwrap_or(false);
        match start_value(project, cid, layer, p) {
            Some(v) if used => starts.push((p.clone(), v)),
            _ if paths.len() == 1 => return Err(Error::refused(format!("{p} has nothing to go back to (words and paths are made, not reset)"))),
            _ => {}
        }
    }
    let now = layer.layer_time(cx.view.time);
    let name = layer.name.clone();
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let mut done = Vec::new();
    for (p, v) in starts {
        let q = layer.props.prop_mut(&p).expect("found");
        q.expression = None;
        if q.is_animated() {
            let mut keys = q.keys.clone();
            veymelo_keyframe::set_key(&mut keys, Keyframe::new(now, v));
            q.set_keys(keys).map_err(Error::params)?;
        } else {
            q.set_value(v).map_err(Error::params)?;
        }
        done.push(Json::from(p));
    }
    cx.label = Some(format!("Reset {path} of {name}"));
    Ok(Json::object([("reset", Json::Array(done))]))
}

fn prop_animate(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let path = a.text("path").unwrap_or("").to_string();
    let on = a.bool("on").unwrap_or(true);
    let view = cx.view.time;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let p = layer.props.prop(&path).map_err(Error::not_found)?;
    super::prop::position_in_use(layer, p.def.id)?;
    let now = layer.layer_time(view);
    let name = layer.name.clone();
    let p = layer.props.prop_mut(&path).expect("found");
    if !p.def.animatable {
        return Err(Error::refused(format!("{path} is a setting and cannot be animated")));
    }
    let value = p.value_at(now);
    match (on, p.is_animated()) {
        (true, false) => p.set_keys(vec![Keyframe::new(now, value)]).map_err(Error::params)?,
        (false, true) => p.set_value(value).map_err(Error::params)?,
        _ => {}
    }
    let mut o = Map::new();
    o.insert("path", path.as_str());
    o.insert("value", p.value_at(now).to_json(p.def.kind));
    if p.is_animated() {
        o.insert("keys", keys_to_json(&p.keys, p.def.kind));
    }
    cx.label = Some(format!("{} {path} of {name}", if on { "Animate" } else { "Stop animating" }));
    Ok(Json::Object(o))
}

fn expr_errors(l: &Look, a: &Args) -> Result<Json> {
    let cid = super::comp_id(l.project, l.view, a)?;
    let comp = l.project.comp(cid).expect("found");
    let t = a.time("time").unwrap_or(l.view.time);
    let e = Eval::new(l.project);
    let mut errors = Vec::new();
    for (i, layer) in comp.layers.iter().enumerate() {
        let at = e.place(comp, layer);
        let mut found: Vec<(String, &Property)> = Vec::new();
        layer.props.walk(&mut |path, p| {
            if p.expression.as_ref().is_some_and(|x| !x.text.trim().is_empty()) {
                found.push((path.to_string(), p));
            }
        });
        for (path, p) in found {
            let x = p.expression.as_ref().expect("has one");
            let (message, off) = if x.enabled {
                match e.work(p, at, t) {
                    Ok(_) => continue,
                    Err((_, why)) => (why, false),
                }
            } else {
                match veymelo_expr::syntax::parse(&x.text) {
                    Ok(_) => continue,
                    Err(err) => (format!("not written right at character {}: {}", err.at, err.message), true),
                }
            };
            let mut o = Map::new();
            o.insert("layer", layer.id as f64);
            o.insert("index", (i + 1) as f64);
            o.insert("name", layer.name.as_str());
            o.insert("path", path.as_str());
            o.insert("message", message);
            if off {
                o.insert("off", true);
            }
            errors.push(Json::Object(o));
        }
    }
    Ok(Json::object([("time", Json::Number(t.seconds())), ("count", Json::Number(errors.len() as f64)), ("errors", Json::Array(errors))]))
}

fn expr_reference(_: &Look, a: &Args) -> Result<Json> {
    let only = a.text("category");
    let all = veymelo_expr::reference::reference();
    if let Some(c) = only
        && !all.iter().any(|(name, _)| name.eq_ignore_ascii_case(c))
    {
        let names: Vec<&str> = all.iter().map(|(n, _)| *n).collect();
        return Err(Error::params(format!("there is no category \"{c}\" (there are: {})", names.join(", "))));
    }
    let out = all.into_iter().filter(|(name, _)| only.is_none_or(|c| name.eq_ignore_ascii_case(c))).map(|(name, entries)| {
        let entries = entries.into_iter().map(|e| Json::object([("write", Json::from(e.write)), ("about", Json::from(e.about)), ("example", Json::from(e.example))]));
        Json::object([("category", Json::from(name)), ("entries", Json::array(entries))])
    });
    Ok(Json::array(out))
}
