//! Commands about a layer's properties: listing them, reading them, giving them a value or keys.
//!
//! Times given to these commands are composition times. A layer's keys are kept in its own time,
//! which is the same unless the layer was moved in time (`start`) or stretched; the keys a
//! command gives back are in the layer's own time.

use veymelo_expr::{Eval, Place};
use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Keyframe, Value, keys_from_json, keys_to_json};
use veymelo_project::{Applies, Expression, Group, Layer, Property};
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

const PATH: Param = Param::need("path", Ty::Text, "the property, like transform/position or masks/Mask 1/feather");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "prop.list",
            label: "Properties",
            summary: "A layer's properties with what they hold now (at the view's time), what kind of value each takes, its limits and default.",
            params: const { &[COMP, LAYER, Param::opt("group", Ty::Text, "only those inside this group, like transform"), Param::opt("changed", Ty::Bool, "only those not at their default")] },
            returns: "[{path, name, value, about, animated?, keys?, expression?}]",
            run: Run::Look(prop_list),
        },
        Command {
            id: "prop.get",
            label: "Get Property",
            summary: "A property's value at a time, its keys and expression if it has them, and what it takes.",
            params: const { &[COMP, LAYER, PATH, Param::opt("time", Ty::Time, "when (default: the view's time)")] },
            returns: "{path, value, about, keys?, expression?}",
            run: Run::Look(prop_get),
        },
        Command {
            id: "prop.set",
            label: "Set Property",
            summary: "Give a property a value: for all time, or with `time` as a keyframe there (a property with keys needs `time`).",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    Param::need("value", Ty::Any, "the value, as prop.list shows it takes it ([x, y] for a point, \"#ff0000\" for a colour…)"),
                    Param::opt("time", Ty::Time, "set a keyframe at this time instead"),
                ]
            },
            returns: "{path, value, keys?}",
            run: Run::Edit(prop_set),
        },
        Command {
            id: "prop.keys",
            label: "Set Keyframes",
            summary: "Give a property all its keys at once: [{t, v, in?, out?, easeIn?, easeOut?…}], times in the layer's own seconds. An empty list takes its keys away, keeping its value at the view's time.",
            params: const { &[COMP, LAYER, PATH, Param::need("keys", Ty::List, "the keys: [{\"t\": 0, \"v\": 0}, {\"t\": 1, \"v\": 100, \"in\": \"bezier\"}]")] },
            returns: "{path, keys}",
            run: Run::Edit(prop_keys),
        },
        Command {
            id: "prop.expression",
            label: "Set Expression",
            summary: "Give a property an expression: a little program, written as compositing tools' expressions are, that works out its value at each moment (wiggle(2, 30), loopOut(\"cycle\"), value + [0, Math.sin(time * 4) * 20], thisComp.layer(\"Null 1\").transform.position). An empty text takes it away; `enabled` switches it off and on.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    Param::opt("text", Ty::Text, "the expression (\"\" takes it away)"),
                    Param::opt("enabled", Ty::Bool, "switch it on or off, keeping its text"),
                ]
            },
            returns: "{path, expression?, enabled?, value, error?}",
            run: Run::Edit(prop_expression),
        },
        Command {
            id: "prop.evaluate",
            label: "Evaluate Property",
            summary: "A property's value, with its expression, at several times; with `text`, what that expression would give, without keeping it. Says what is wrong when an expression fails.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    Param::opt("text", Ty::Text, "try this expression instead of the property's own"),
                    Param::opt("times", Ty::List, "the times, in composition seconds (default: the view's time)"),
                ]
            },
            returns: "{path, values: [{t, value}], error?}",
            run: Run::Look(prop_evaluate),
        },
    ]
}

/// The JSON of a property's value at layer time `t` (without its expression).
fn value_json(p: &Property, t: Tick) -> Json {
    p.value_at(t).to_json(p.def.kind)
}

/// What an expression works out to at composition time `t`: the value (its keys' value when
/// it fails) and what went wrong.
fn worked(e: &Eval, p: &Property, at: Place, t: Tick) -> (Json, Option<String>) {
    // a text expression selector's amount reads its character: the first of one, wholly picked
    let mut selector = false;
    if let Ok(list) = at.layer.props.group("text/animators") {
        list.instances(&mut |g| {
            if g.def.id == "expression" && g.prop("amount").is_ok_and(|q| std::ptr::eq(q, p)) {
                selector = true;
            }
        });
    }
    let vars: Vec<(&str, Vec<f64>)> = if selector { vec![("textIndex", vec![1.0]), ("textTotal", vec![1.0]), ("selectorValue", vec![100.0; 3])] } else { Vec::new() };
    match e.work_with(p, at, t, &vars) {
        Ok(v) => (v.to_json(p.def.kind), None),
        Err((v, why)) => (v.to_json(p.def.kind), Some(why)),
    }
}

/// A note when a property does nothing on this layer.
fn note(layer: &Layer, p: &Property) -> Option<String> {
    match p.def.applies {
        Applies::ThreeD if !layer.is_3d() => Some(format!("{} only acts on 3D layers; this one is 2D (layer.set switches {{\"threeD\": true}} makes it 3D)", p.def.id)),
        Applies::TwoD if layer.is_3d() => Some(format!("{} only acts on 2D layers", p.def.id)),
        _ => None,
    }
}

/// A property as commands show it, at composition time `t`.
fn describe_prop(path: &str, e: &Eval, at: Place, p: &Property, t: Tick, keys: bool) -> Map {
    let layer = at.layer;
    let mut o = Map::new();
    o.insert("path", path);
    o.insert("name", p.def.name);
    let (value, error) = worked(e, p, at, t);
    let own = value_json(p, layer.layer_time(t));
    if value != own {
        // what its keys give before the expression changes it
        o.insert("keyed", own);
    }
    o.insert("value", value);
    o.insert("about", p.def.summary());
    if p.is_animated() {
        o.insert("animated", true);
        if keys {
            o.insert("keys", keys_to_json(&p.keys, p.def.kind));
        } else {
            o.insert("keys", p.keys.len() as f64);
        }
    }
    if let Some(x) = &p.expression {
        o.insert("expression", x.text.as_str());
        if !x.enabled {
            o.insert("expressionEnabled", false);
        }
    }
    if let Some(why) = error {
        o.insert("error", why);
    }
    if let Some(n) = note(layer, p) {
        o.insert("note", n);
    }
    o
}

fn prop_list(l: &Look, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(l.project, l.view, a)?;
    let comp = l.project.comp(cid).expect("found");
    let layer = comp.layer(id).expect("found");
    let e = Eval::new(l.project);
    let at = e.place(comp, layer);
    let (root, prefix): (&Group, String) = match a.text("group") {
        Some(g) => (layer.props.group(g).map_err(Error::not_found)?, format!("{}/", g.trim_matches('/'))),
        None => (&layer.props, String::new()),
    };
    let changed = a.bool("changed") == Some(true);
    let mut out = Vec::new();
    root.walk(&mut |path, p| {
        if !changed || !p.is_default() {
            out.push(Json::Object(describe_prop(&format!("{prefix}{path}"), &e, at, p, l.view.time, false)));
        }
    });
    Ok(Json::Array(out))
}

fn prop_get(l: &Look, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(l.project, l.view, a)?;
    let comp = l.project.comp(cid).expect("found");
    let layer = comp.layer(id).expect("found");
    let path = a.text("path").unwrap_or("");
    let p = layer.props.prop(path).map_err(Error::not_found)?;
    let e = Eval::new(l.project);
    Ok(Json::Object(describe_prop(path, &e, e.place(comp, layer), p, a.time("time").unwrap_or(l.view.time), true)))
}

/// The property `path` of the layer the parameters name, to change.
fn target<'a>(cx: &'a mut Ctx, a: &Args) -> Result<(&'a mut Layer, String)> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
    let path = a.text("path").unwrap_or("").to_string();
    let p = layer.props.prop(&path).map_err(Error::not_found)?;
    position_in_use(layer, p.def.id)?;
    Ok((layer, path))
}

/// A layer's position is either one property or three: the other is not used.
pub(crate) fn position_in_use(layer: &Layer, id: &str) -> Result<()> {
    match (id, layer.separate_position) {
        ("position", true) => Err(Error::refused(format!("\"{}\" has its position in separate dimensions: use transform/positionX and transform/positionY (or join them with layer.set separatePosition false)", layer.name))),
        ("positionX" | "positionY" | "positionZ", false) => Err(Error::refused(format!("\"{}\" has its position as one property: use transform/position (or separate it with layer.set separatePosition true)", layer.name))),
        _ => Ok(()),
    }
}

fn prop_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view_time = cx.view.time;
    let (layer, path) = target(cx, a)?;
    let time = a.time("time").map(|t| layer.layer_time(t));
    let now = layer.layer_time(view_time);
    let layer_name = layer.name.clone();
    let note = note(layer, layer.props.prop(&path).expect("found"));
    let p = layer.props.prop_mut(&path).expect("found");
    let value = Value::from_json(p.def.kind, a.need("value")).map_err(|e| Error::params(format!("`value`: {e}")))?;
    p.def.check(&value).map_err(Error::params)?;
    match time {
        None if p.is_animated() => {
            return Err(Error::refused(format!(
                "{path} has keys; give `time` to set a key there, prop.keys to change them all, or prop.keys with [] to take them away"
            )));
        }
        None => p.set_value(value).map_err(Error::params)?,
        Some(t) => {
            if !p.def.animatable {
                return Err(Error::refused(format!("{path} cannot be animated; leave out `time`")));
            }
            let mut keys = p.keys.clone();
            veymelo_keyframe::set_key(&mut keys, Keyframe::new(t, value));
            p.set_keys(keys).map_err(Error::params)?;
        }
    }
    let mut o = Map::new();
    o.insert("path", path.as_str());
    o.insert("value", value_json(p, time.unwrap_or(now)));
    if p.is_animated() {
        o.insert("keys", keys_to_json(&p.keys, p.def.kind));
    }
    if let Some(n) = note {
        o.insert("note", n);
    }
    cx.label = Some(format!("Set {path} of {layer_name}"));
    Ok(Json::Object(o))
}

fn prop_keys(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let view_time = cx.view.time;
    let (layer, path) = target(cx, a)?;
    let t = layer.layer_time(view_time);
    let layer_name = layer.name.clone();
    let p = layer.props.prop_mut(&path).expect("found");
    let keys = keys_from_json(p.def.kind, a.need("keys")).map_err(|e| Error::params(format!("`keys`: {e}")))?;
    if keys.is_empty() {
        let now = p.value_at(t);
        p.set_value(now).map_err(Error::params)?;
    } else {
        p.set_keys(keys).map_err(Error::params)?;
    }
    let out = Json::object([("path", Json::from(path.as_str())), ("keys", keys_to_json(&p.keys, p.def.kind))]);
    cx.label = Some(format!("Set keys of {path} of {layer_name}"));
    Ok(out)
}

fn prop_expression(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    let (text, enabled) = (a.text("text"), a.bool("enabled"));
    if text.is_none() && enabled.is_none() {
        return Err(Error::params("give `text` (the expression, or \"\" to take it away) or `enabled`"));
    }
    if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
        veymelo_expr::syntax::parse(t).map_err(|e| Error::params(format!("`text` is not written right at character {}: {}", e.at, e.message)))?;
    }
    let (layer, path) = target(cx, a)?;
    let layer_name = layer.name.clone();
    let p = layer.props.prop_mut(&path).expect("found");
    if !p.def.animatable {
        return Err(Error::refused(format!("{path} is a setting, not animated, so it cannot have an expression")));
    }
    let label = match (text, &mut p.expression) {
        (Some(t), _) if t.trim().is_empty() => {
            p.expression = None;
            "Remove expression"
        }
        (Some(t), x) => {
            *x = Some(Expression { text: t.to_string(), enabled: enabled.unwrap_or(true) });
            "Set expression"
        }
        (None, Some(x)) => {
            x.enabled = enabled.unwrap_or(x.enabled);
            if x.enabled { "Switch on expression" } else { "Switch off expression" }
        }
        (None, None) => return Err(Error::refused(format!("{path} has no expression to switch; give `text`"))),
    };
    let project: &veymelo_project::Project = cx.project;
    let comp = project.comp(cid).expect("found");
    let layer = comp.layer(id).expect("found");
    let p = layer.props.prop(&path).expect("found");
    let e = Eval::new(project);
    let (value, error) = worked(&e, p, e.place(comp, layer), cx.view.time);
    let mut o = Map::new();
    o.insert("path", path.as_str());
    if let Some(x) = &p.expression {
        o.insert("expression", x.text.as_str());
        if !x.enabled {
            o.insert("enabled", false);
        }
    }
    o.insert("value", value);
    if let Some(why) = error {
        o.insert("error", why);
    }
    cx.label = Some(format!("{label} of {path} of {layer_name}"));
    Ok(Json::Object(o))
}

fn prop_evaluate(l: &Look, a: &Args) -> Result<Json> {
    let (cid, id) = layer_ids(l.project, l.view, a)?;
    let comp = l.project.comp(cid).expect("found");
    let original = comp.layer(id).expect("found");
    let path = a.text("path").unwrap_or("");
    original.props.prop(path).map_err(Error::not_found)?;
    // an expression to try goes on a copy of the layer, so nothing changes
    let tried;
    let layer = match a.text("text") {
        Some(t) => {
            veymelo_expr::syntax::parse(t).map_err(|e| Error::params(format!("`text` is not written right at character {}: {}", e.at, e.message)))?;
            let mut copy = original.clone();
            let p = copy.props.prop_mut(path).expect("found");
            if !p.def.animatable {
                return Err(Error::refused(format!("{path} is a setting, not animated, so it cannot have an expression")));
            }
            p.expression = Some(Expression { text: t.to_string(), enabled: true });
            tried = copy;
            &tried
        }
        None => original,
    };
    let times: Vec<Tick> = match a.json("times") {
        Some(Json::Array(ts)) => ts
            .iter()
            .map(|t| t.as_f64().filter(|t| t.is_finite() && t.abs() <= 1e9).map(Tick::from_seconds).ok_or_else(|| Error::params("`times` is a list of times in seconds, like [0, 0.5, 1]")))
            .collect::<Result<_>>()?,
        _ => vec![l.view.time],
    };
    if times.len() > 1000 {
        return Err(Error::params("`times` takes at most 1000 times"));
    }
    let p = layer.props.prop(path).expect("found");
    let e = Eval::new(l.project);
    let at = e.place(comp, layer);
    let mut values = Vec::new();
    let mut first_error = None;
    for t in times {
        let (value, error) = worked(&e, p, at, t);
        values.push(Json::object([("t", Json::from(t.seconds())), ("value", value)]));
        if first_error.is_none() {
            first_error = error;
        }
    }
    let mut o = Map::new();
    o.insert("path", path);
    o.insert("values", Json::Array(values));
    if let Some(why) = first_error {
        o.insert("error", why);
    }
    Ok(Json::Object(o))
}
