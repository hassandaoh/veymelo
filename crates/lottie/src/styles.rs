//! Layer styles both ways (Lottie's `sy`): its nine kinds (stroke 0, drop shadow 1, inner shadow 2,
//! outer glow 3, inner glow 4, bevel and emboss 5, satin 6, colour overlay 7, gradient overlay 8)
//! and ours, each setting by its key. Values keep the units the styles have (percent 0 to 100,
//! degrees, pixels); a menu's choice is counted from 1. A style that follows the composition's
//! light is written with that light's angle (and, a bevel, its height); one read is given the
//! angle it carries, unless it says it follows the light.

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::Value;
use veymelo_project::styles::{self, BLENDING, BLENDING_OPTIONS, LIST};
use veymelo_project::{Group, Layer, Node, Property};

use crate::values::{self, Ctx, color_of, flag_of, prop_from, prop_json, same, times, vec2_of};

/// A kind of style: its Lottie number, our style, and its settings (Lottie key, ours).
type Kind = (u8, &'static str, &'static [(&'static str, &'static str)]);

/// Each kind.
const KINDS: [Kind; 9] = [
    (0, "stroke", &[("c", "color"), ("s", "size"), ("bm", "blendMode"), ("o", "opacity"), ("pos", "position")]),
    (1, "dropShadow", &[("c", "color"), ("o", "opacity"), ("a", "angle"), ("s", "size"), ("d", "distance"), ("ch", "spread"), ("bm", "blendMode"), ("no", "noise"), ("lc", "knocksOut")]),
    (2, "innerShadow", &[("c", "color"), ("o", "opacity"), ("a", "angle"), ("s", "size"), ("d", "distance"), ("ch", "choke"), ("bm", "blendMode"), ("no", "noise")]),
    (3, "outerGlow", &[("c", "color"), ("o", "opacity"), ("r", "range"), ("ch", "spread"), ("s", "size"), ("bm", "blendMode"), ("no", "noise"), ("j", "jitter"), ("ct", "colorType"), ("gf", "colors"), ("gs", "gradientSmoothness"), ("te", "technique")]),
    (4, "innerGlow", &[("c", "color"), ("o", "opacity"), ("r", "range"), ("sr", "source"), ("ch", "choke"), ("s", "size"), ("bm", "blendMode"), ("no", "noise"), ("j", "jitter"), ("ct", "colorType"), ("gf", "colors"), ("gs", "gradientSmoothness"), ("te", "technique")]),
    (
        5,
        "bevelEmboss",
        &[
            ("bs", "style"),
            ("bt", "technique"),
            ("sr", "depth"),
            ("s", "size"),
            ("sf", "soften"),
            ("ga", "useGlobalLight"),
            ("a", "angle"),
            ("ll", "altitude"),
            ("hm", "highlightMode"),
            ("hc", "highlightColor"),
            ("ho", "highlightOpacity"),
            ("sm", "shadowMode"),
            ("sc", "shadowColor"),
            ("so", "shadowOpacity"),
            ("dr", "direction"),
        ],
    ),
    (6, "satin", &[("bm", "blendMode"), ("c", "color"), ("o", "opacity"), ("a", "angle"), ("d", "distance"), ("s", "size"), ("in", "invert")]),
    (7, "colorOverlay", &[("bm", "blendMode"), ("c", "color"), ("so", "opacity")]),
    (
        8,
        "gradientOverlay",
        &[("bm", "blendMode"), ("o", "opacity"), ("gf", "colors"), ("gs", "gradientSmoothness"), ("a", "angle"), ("gt", "style"), ("re", "reverse"), ("al", "alignWithLayer"), ("s", "scale"), ("of", "offset")],
    ),
];

/// The Blending Options of a layer's styles list.
fn blending(list: &Group) -> Option<&Group> {
    list.children.iter().filter_map(Node::as_group).find(|g| g.def.id == BLENDING)
}

/// A gradient setting as Lottie's: its number of colours and its stops.
fn gradient_json(cx: &mut Ctx, p: &Property, label: &str) -> Json {
    let count = |v: &Value| v.as_gradient().map_or(2, |g| if g.colors.is_empty() { 2 } else { g.colors.len() });
    let n = count(&p.value);
    if p.keys.iter().any(|k| count(&k.value) != n) {
        cx.warn(format!("{label}: its keys have different numbers of colours, which Lottie cannot animate"));
    }
    Json::object([("p", Json::from(n as f64)), ("k", prop_json(cx, p, &same, label))])
}

/// A layer's styles as Lottie's `sy` (those switched on).
pub fn export(cx: &mut Ctx, l: &Layer) -> Vec<Json> {
    let Ok(list) = l.props.group(LIST) else { return Vec::new() };
    let light = blending(list);
    if let Some(bo) = light
        && let Ok(fill) = bo.prop("advancedBlending/fillOpacity")
        && (fill.is_animated() || fill.value.as_f64() != 100.0)
    {
        cx.warn(format!("{}: Lottie layer styles have no Fill Opacity (the layer is drawn whole)", l.name));
    }
    let mut out = Vec::new();
    for g in list.children.iter().filter_map(Node::as_group) {
        let Some(&(ty, _, keys)) = KINDS.iter().find(|k| k.1 == g.def.id) else { continue };
        let label = format!("{} › {}", l.name, g.name());
        if !g.enabled {
            cx.warn(format!("{label}: switched off, so left out (Lottie styles cannot be off)"));
            continue;
        }
        let global = g.prop("useGlobalLight").is_ok_and(|p| p.value.as_bool());
        let mut o = Map::new();
        o.insert("ty", ty as f64);
        o.insert("nm", g.name());
        o.insert("mn", format!("ADBE {}", g.def.name));
        for &(key, id) in keys {
            // the light a style follows, from Blending Options
            let p = match (id, global, light) {
                ("angle", true, Some(bo)) => bo.prop("globalLightAngle").ok(),
                ("altitude", true, Some(bo)) => bo.prop("globalLightAltitude").ok(),
                _ => g.prop(id).ok(),
            };
            let Some(p) = p else { continue };
            let plabel = format!("{label} › {}", p.def.name);
            let v = if matches!(p.value, Value::Gradient(_)) { gradient_json(cx, p, &plabel) } else { prop_json(cx, p, &times(1.0), &plabel) };
            o.insert(key, v);
        }
        out.push(Json::Object(o));
    }
    out
}

/// Lottie's `sy` as a layer's styles: Blending Options, then each style in its place.
pub fn import(cx: &mut Ctx, list_json: &[Json], l: &mut Layer, ids: &mut dyn FnMut() -> u64) {
    let name = l.name.clone();
    let Ok(list) = l.props.group_mut(LIST) else {
        cx.warn(format!("{name}: it takes no layer styles (they are left out)"));
        return;
    };
    let mut made: Vec<Group> = Vec::new();
    let mut light: Option<(Json, Option<Json>)> = None;
    for s in list_json {
        let ty = s.get("ty").and_then(Json::as_f64).unwrap_or(-1.0) as i64;
        let Some(&(_, id, keys)) = KINDS.iter().find(|k| k.0 as i64 == ty) else {
            cx.warn(format!("{name}: a layer style of Lottie type {ty} is not read"));
            continue;
        };
        let Some(def) = styles::find(id) else { continue };
        let nm = s.get("nm").and_then(Json::as_str).filter(|n| !n.is_empty()).unwrap_or(def.name).to_string();
        let mut g = Group::instance(def, ids(), &nm);
        let label = format!("{name} › {nm}");
        // `ga` as an animated property (its `k`) or a plain number
        let follows = s.get("ga").map(|g| values::numbers(g.get("k").unwrap_or(g))).and_then(|v| v.first().copied()).is_some_and(|v| v != 0.0);
        for &(key, pid) in keys {
            let Some(v) = s.get(key) else { continue };
            if pid == "colors" {
                let count = v.get("p").and_then(Json::as_f64).unwrap_or(2.0).max(1.0) as usize;
                let (Some(k), Ok(p)) = (v.get("k"), g.prop_mut(pid)) else { continue };
                let conv = move |j: &Json| -> Option<Value> {
                    let n = values::numbers(j);
                    (n.len() >= count * 4).then(|| Value::Gradient(values::gradient_of(&n, count)))
                };
                prop_from(cx, k, p, &conv, &label);
                continue;
            }
            let Ok(p) = g.prop_mut(pid) else { continue };
            let conv: &dyn Fn(&Json) -> Option<Value> = match p.value {
                Value::Choice(_) => &values::choice_of,
                Value::Bool(_) => &flag_of,
                Value::Color(_) => &color_of,
                Value::Vec2(_) => &|j: &Json| vec2_of(j).or_else(|| values::numbers(j).first().map(|x| Value::Vec2([*x, 0.0]))),
                _ => &values::number,
            };
            prop_from(cx, v, p, conv, &label);
        }
        // the angle it carries is its own, unless it follows the light (a bevel may say so)
        if let Ok(p) = g.prop_mut("useGlobalLight") {
            p.value = Value::Bool(follows);
        }
        if follows && light.is_none() {
            light = Some((s.get("a").cloned().unwrap_or(Json::Null), s.get("ll").cloned()));
        }
        made.push(g);
    }
    if made.is_empty() {
        return;
    }
    if !list.children.iter().any(|n| n.as_group().is_some_and(|g| g.def.id == BLENDING)) {
        let mut bo = Group::instance(&BLENDING_OPTIONS, ids(), BLENDING_OPTIONS.name);
        if let Some((a, ll)) = &light {
            if let Ok(p) = bo.prop_mut("globalLightAngle") {
                prop_from(cx, a, p, &values::number, &name);
            }
            if let (Some(ll), Ok(p)) = (ll, bo.prop_mut("globalLightAltitude")) {
                prop_from(cx, ll, p, &values::number, &name);
            }
        }
        list.children.insert(0, Node::Group(bo));
    }
    made.sort_by_key(|g| styles::rank(g.def.id));
    for g in made {
        list.children.retain(|n| n.as_group().is_none_or(|o| o.def.id != g.def.id));
        let at = list.children.iter().position(|n| n.as_group().is_some_and(|o| styles::rank(o.def.id) > styles::rank(g.def.id))).unwrap_or(list.children.len());
        list.children.insert(at, Node::Group(g));
    }
}
