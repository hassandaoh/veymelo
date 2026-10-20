//! Layers arranged in the picture — aligned to an edge or centre of the composition or of each
//! other, spread evenly, nudged — by what they draw; and effects copied (to the same layer or
//! another), reset to how they start, switched on or off, renamed, all taken off.

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Keyframe, Value};
use veymelo_project::{Group, Layer, Node, Source};
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, comp_id, find_layer, layer_ids};
use crate::{Args, Ctx, Error, Param, Result, Ty};

const LAYERS: Param = Param::need("layers", Ty::List, "the layers (ids, names or \"#n\")");
const EFFECT: Param = Param::need("effect", Ty::Text, "the effect, like \"Gaussian Blur\" or \"#1\"");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "layer.align",
            label: "Align Layers",
            summary: "Line layers up by what they draw: their left, horizontal centre, right, top, vertical centre or bottom on the composition's, or (to: layers) on the outermost of the layers given. Each moves as a whole (a key where its position is animated).",
            params: const { &[COMP, LAYERS, Param::need("edge", Ty::Choice(&["left", "hcenter", "right", "top", "vcenter", "bottom"]), "which edge or centre"), Param::opt("to", Ty::Choice(&["composition", "layers"]), "default composition"), Param::opt("time", Ty::Time, "when (default: the view's time)")] },
            returns: "{moved: [{layer, by}]}",
            run: Run::Edit(align),
        },
        Command {
            id: "layer.distribute",
            label: "Distribute Layers",
            summary: "Spread three or more layers evenly between the outermost two, by their left edges, centres, right edges, tops or bottoms, or so the gaps between them are equal (gaps).",
            params: const { &[COMP, LAYERS, Param::need("by", Ty::Choice(&["left", "hcenter", "right", "top", "vcenter", "bottom", "hgaps", "vgaps"]), "what is spread evenly"), Param::opt("time", Ty::Time, "when (default: the view's time)")] },
            returns: "{moved: [{layer, by}]}",
            run: Run::Edit(distribute),
        },
        Command {
            id: "layer.nudge",
            label: "Nudge Layers",
            summary: "Move layers by so many composition pixels across and down (a key where position is animated).",
            params: const { &[COMP, LAYERS, Param::opt("x", Ty::Number, "pixels right"), Param::opt("y", Ty::Number, "pixels down"), Param::opt("time", Ty::Time, "when (default: the view's time)")] },
            returns: "{moved: [{layer, by}]}",
            run: Run::Edit(nudge),
        },
        Command {
            id: "layer.transform",
            label: "Transform",
            summary: "Layers' transform set the usual ways at the view's time (a key there where it is animated): reset (as a new layer starts), center (in the middle of the composition), fit (what it shows filling the composition), fitWidth or fitHeight (filling it across or down, keeping its shape), flipH or flipV (turned over across or down).",
            params: const { &[COMP, LAYERS, Param::need("op", Ty::Choice(&["reset", "center", "fit", "fitWidth", "fitHeight", "flipH", "flipV"]), "what to do")] },
            returns: "{layers}",
            run: Run::Edit(transform_op),
        },
        Command {
            id: "layer.centerAnchor",
            label: "Center Anchor Point in Layer Content",
            summary: "Put layers' anchor points in the middle of what they show (text and shapes too), each layer staying where it is (its position, every key of it, moved to make up).",
            params: const { &[COMP, LAYERS] },
            returns: "{layers: [{layer, anchor}]}",
            run: Run::Edit(center_anchor),
        },
        Command {
            id: "effect.duplicate",
            label: "Duplicate Effect",
            summary: "Copy an effect (its settings, keys and expressions) just below itself, or to the end of another layer's effects.",
            params: const { &[COMP, LAYER, EFFECT, Param::opt("toLayer", Ty::Layer, "the layer to copy it to (default: the same one)"), Param::opt("name", Ty::Text, "the copy's name")] },
            returns: "{path, name}",
            run: Run::Edit(duplicate),
        },
        Command {
            id: "effect.reset",
            label: "Reset Effect",
            summary: "Put an effect's settings back as they start (keys and expressions gone; points back where they start on the layer).",
            params: const { &[COMP, LAYER, EFFECT] },
            returns: "{path}",
            run: Run::Edit(reset),
        },
        Command {
            id: "effect.pickColor",
            label: "Pick Effect Colour",
            summary: "Set a colour setting of an effect to the colour at a point of the layer as that effect sees it (before it and the effects under it: a keyer's screen can still be picked once keyed). `average` takes the 5 × 5 pixels around the point.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    EFFECT,
                    Param::need("param", Ty::Text, "the colour setting, like keyColor (effect.types lists them)"),
                    Param::need("point", Ty::List, "where on the layer, [x, y] in its own pixels"),
                    Param::opt("average", Ty::Bool, "the 5 × 5 pixels around it"),
                    Param::opt("time", Ty::Time, "when (default: the view's time)"),
                ]
            },
            returns: "{path, color}",
            run: Run::Edit(pick_color),
        },
        Command {
            id: "effect.set",
            label: "Change Effect",
            summary: "Switch an effect on or off, or rename it.",
            params: const { &[COMP, LAYER, EFFECT, Param::opt("enabled", Ty::Bool, "on or off"), Param::opt("name", Ty::Text, "a new name")] },
            returns: "{path, enabled}",
            run: Run::Edit(set),
        },
        Command {
            id: "effect.removeAll",
            label: "Remove All Effects",
            summary: "Take every effect off a layer.",
            params: const { &[COMP, LAYER] },
            returns: "{removed}",
            run: Run::Edit(remove_all),
        },
    ]
}

/// The comp-space box a layer draws in at `t` (as if solid: its opacity not counted), or none
/// when it draws nothing.
pub(super) fn bounds(project: &veymelo_project::Project, cid: u64, lid: u64, t: Tick) -> Result<Option<[f64; 4]>> {
    let mut solo = project.clone();
    if let Some(veymelo_project::ItemKind::Comp(c)) = solo.item_mut(cid).map(|i| &mut i.kind) {
        for l in &mut std::sync::Arc::make_mut(c).layers {
            l.switches.video = l.id == lid;
            l.switches.solo = false;
            if l.id == lid
                && let Ok(p) = l.props.prop_mut("transform/opacity")
            {
                p.keys.clear();
                p.expression = None;
                p.value = Value::Number(100.0);
            }
        }
    }
    let r = veymelo_render::render(&solo, cid, t, &veymelo_render::Options { background: false, ..Default::default() }).map_err(Error::refused)?;
    let img = &r.image;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..img.height {
        for x in 0..img.width {
            if img.px[y * img.width + x][3] > 0.5 / 255.0 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    Ok((x0 != usize::MAX).then(|| [x0 as f64, y0 as f64, (x1 + 1) as f64, (y1 + 1) as f64]))
}

fn layers_of(cx: &Ctx, a: &Args, least: usize) -> Result<(u64, Vec<u64>)> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let mut ids = Vec::new();
    for j in a.json("layers").and_then(Json::as_array).cloned().unwrap_or_default() {
        let id = find_layer(comp, &j)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.len() < least {
        return Err(Error::params(format!("give {least} layer{} or more", if least == 1 { "" } else { "s" })));
    }
    Ok((cid, ids))
}

/// A move of `d` composition pixels at comp time `t` as a move of the layer's position: in its
/// parent's space.
pub(super) fn to_parent(project: &veymelo_project::Project, cid: u64, lid: u64, d: [f64; 2], t: Tick) -> [f64; 2] {
    let comp = project.comp(cid).expect("found");
    let l = comp.layer(lid).expect("found");
    let Some(parent) = l.parent.and_then(|p| comp.layer(p)) else { return d };
    let eval = veymelo_expr::Eval::new(project);
    let m = eval.world(veymelo_expr::Place { layer: parent, comp, comp_id: cid }, t, None);
    match m.inverse() {
        Some(inv) => {
            let q = inv.apply_vec(veymelo_geom::vec3(d[0], d[1], 0.0));
            [q.x, q.y]
        }
        None => d,
    }
}

/// Move a layer by `d` composition pixels at layer-comp time `t`: through its parent's space,
/// into its position (a key there when it is animated).
fn shift(cx: &mut Ctx, cid: u64, lid: u64, d: [f64; 2], t: Tick) -> Result<()> {
    if d[0].abs() < 1e-9 && d[1].abs() < 1e-9 {
        return Ok(());
    }
    let v = to_parent(cx.project, cid, lid, d, t);
    let l = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let lt = l.layer_time(t);
    let paths: Vec<(&str, usize)> = if l.separate_position { vec![("transform/positionX", 0), ("transform/positionY", 1)] } else { vec![("transform/position", 9)] };
    for (path, axis) in paths {
        let p = l.props.prop_mut(path).map_err(Error::not_found)?;
        let moved = |v0: &Value| match (v0, axis) {
            (Value::Vec3(q), _) => Value::Vec3([q[0] + v[0], q[1] + v[1], q[2]]),
            (Value::Number(n), a) => Value::Number(n + v[a.min(1)]),
            (other, _) => other.clone(),
        };
        if p.keys.is_empty() {
            let nv = moved(&p.value);
            p.set_value(nv).map_err(Error::params)?;
        } else {
            let now = p.value_at(lt);
            let mut keys = p.keys.clone();
            veymelo_keyframe::set_key(&mut keys, Keyframe::new(lt, moved(&now)));
            p.set_keys(keys).map_err(Error::params)?;
        }
    }
    Ok(())
}

fn report(moves: &[(u64, [f64; 2])]) -> Json {
    Json::object([("moved", Json::array(moves.iter().map(|(id, d)| Json::object([("layer", Json::Number(*id as f64)), ("by", Json::array(d.iter().map(|v| Json::Number((v * 1000.0).round() / 1000.0))))]))))])
}

fn align(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let t = a.time("time").unwrap_or(cx.view.time);
    let (cid, ids) = layers_of(cx, a, 1)?;
    let mut boxes = Vec::new();
    for &id in &ids {
        if let Some(b) = bounds(cx.project, cid, id, t)? {
            boxes.push((id, b));
        }
    }
    if boxes.is_empty() {
        return Err(Error::refused("the layers draw nothing then"));
    }
    let (w, h) = {
        let c = cx.project.comp(cid).expect("found");
        (c.width as f64, c.height as f64)
    };
    let to_layers = a.text("to") == Some("layers");
    let all = boxes.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |m, (_, b)| [m[0].min(b[0]), m[1].min(b[1]), m[2].max(b[2]), m[3].max(b[3])]);
    let frame = if to_layers { all } else { [0.0, 0.0, w, h] };
    let edge = a.text("edge").unwrap_or("left");
    let mut moves = Vec::new();
    for (id, b) in boxes {
        let d = match edge {
            "left" => [frame[0] - b[0], 0.0],
            "right" => [frame[2] - b[2], 0.0],
            "hcenter" => [(frame[0] + frame[2]) / 2.0 - (b[0] + b[2]) / 2.0, 0.0],
            "top" => [0.0, frame[1] - b[1]],
            "bottom" => [0.0, frame[3] - b[3]],
            _ => [0.0, (frame[1] + frame[3]) / 2.0 - (b[1] + b[3]) / 2.0],
        };
        shift(cx, cid, id, d, t)?;
        moves.push((id, d));
    }
    cx.label = Some("Align Layers".into());
    Ok(report(&moves))
}

fn distribute(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let t = a.time("time").unwrap_or(cx.view.time);
    let (cid, ids) = layers_of(cx, a, 3)?;
    let mut boxes = Vec::new();
    for &id in &ids {
        if let Some(b) = bounds(cx.project, cid, id, t)? {
            boxes.push((id, b));
        }
    }
    if boxes.len() < 3 {
        return Err(Error::refused("three layers that draw something are needed"));
    }
    let by = a.text("by").unwrap_or("hcenter");
    let vertical = matches!(by, "top" | "vcenter" | "bottom" | "vgaps");
    let (lo, hi) = if vertical { (1, 3) } else { (0, 2) };
    let key = |b: &[f64; 4]| match by {
        "left" | "top" => b[lo],
        "right" | "bottom" => b[hi],
        _ => (b[lo] + b[hi]) / 2.0,
    };
    boxes.sort_by(|x, y| key(&x.1).total_cmp(&key(&y.1)));
    let n = boxes.len();
    let mut moves = Vec::new();
    if by.ends_with("gaps") {
        // the space between the outermost two shared out equally between the gaps
        let total: f64 = boxes.iter().map(|(_, b)| b[hi] - b[lo]).sum();
        let span = boxes[n - 1].1[hi] - boxes[0].1[lo];
        let gap = (span - total) / (n - 1) as f64;
        let mut at = boxes[0].1[hi] + gap;
        for (id, b) in boxes.iter().skip(1).take(n - 2) {
            let d = at - b[lo];
            let v = if vertical { [0.0, d] } else { [d, 0.0] };
            shift(cx, cid, *id, v, t)?;
            moves.push((*id, v));
            at += b[hi] - b[lo] + gap;
        }
    } else {
        let (first, last) = (key(&boxes[0].1), key(&boxes[n - 1].1));
        for (k, (id, b)) in boxes.iter().enumerate().skip(1).take(n - 2) {
            let want = first + (last - first) * k as f64 / (n - 1) as f64;
            let d = want - key(b);
            let v = if vertical { [0.0, d] } else { [d, 0.0] };
            shift(cx, cid, *id, v, t)?;
            moves.push((*id, v));
        }
    }
    cx.label = Some("Distribute Layers".into());
    Ok(report(&moves))
}

fn nudge(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let t = a.time("time").unwrap_or(cx.view.time);
    let (cid, ids) = layers_of(cx, a, 1)?;
    let d = [a.num("x").unwrap_or(0.0), a.num("y").unwrap_or(0.0)];
    let mut moves = Vec::new();
    for id in ids {
        shift(cx, cid, id, d, t)?;
        moves.push((id, d));
    }
    cx.label = Some("Nudge Layers".into());
    Ok(report(&moves))
}

fn effect_group<'a>(layer: &'a mut Layer, which: &str) -> Result<(&'a mut Group, usize)> {
    let name = layer.name.clone();
    let list = layer.props.group_mut("effects").map_err(|_| Error::refused(format!("\"{name}\" takes no effects")))?;
    let which = which.trim_matches('/').trim_start_matches("effects/");
    let id = list.group(which).map_err(Error::not_found)?.id;
    let at = list.children.iter().position(|n| matches!(n, Node::Group(g) if g.id == id)).expect("found");
    Ok((list, at))
}

fn duplicate(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let to = match a.json("toLayer") {
        Some(j) => find_layer(cx.project.comp(cid).expect("found"), j)?,
        None => lid,
    };
    let src = {
        let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
        let (list, at) = effect_group(layer, a.text("effect").unwrap_or(""))?;
        list.children[at].as_group().expect("a group").clone()
    };
    let mut copy = src.clone();
    copy.id = cx.project.alloc();
    copy.instances_mut(&mut |g| g.id = cx.project.alloc());
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(to).expect("found");
    let lname = layer.name.clone();
    let list = layer.props.group_mut("effects").map_err(|_| Error::refused(format!("\"{lname}\" takes no effects")))?;
    let taken: Vec<String> = list.children.iter().map(|c| c.name().to_string()).collect();
    let base = src.name().trim_end_matches(|c: char| c.is_ascii_digit()).trim_end().to_string();
    copy.name = a.text("name").map_or_else(|| (if taken.contains(&src.name().to_string()) { 2.. } else { 1.. }).map(|n| if n == 1 { src.name().to_string() } else { format!("{base} {n}") }).find(|n| !taken.contains(n)).unwrap_or_default(), str::to_string);
    let name = copy.name.clone();
    let at = if to == lid { list.children.iter().position(|n| matches!(n, Node::Group(g) if g.id == src.id)).map_or(list.children.len(), |i| i + 1) } else { list.children.len() };
    list.children.insert(at, Node::Group(copy));
    cx.label = Some(format!("Duplicate {}", src.name()));
    Ok(Json::object([("path", Json::from(format!("effects/{name}"))), ("name", Json::from(name))]))
}

fn reset(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let rect = {
        let project = &*cx.project;
        let l = project.comp(cid).expect("found").layer(lid).expect("found");
        super::effect::bounds(project, cid, l)
    };
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let (list, at) = effect_group(layer, a.text("effect").unwrap_or(""))?;
    let old = list.children[at].as_group().expect("a group").clone();
    let mut fresh = Group::instance(old.def, old.id, &old.name);
    fresh.enabled = old.enabled;
    if let Some(spec) = veymelo_effects::find(old.def.id) {
        spec.start(&mut fresh, rect);
    }
    // what is in its lists (a puppet's meshes) stays
    for (n, o) in fresh.children.iter_mut().zip(&old.children) {
        if let (Node::Group(f), Node::Group(g)) = (n, o)
            && f.is_list()
        {
            *f = g.clone();
        }
    }
    list.children[at] = Node::Group(fresh);
    cx.label = Some(format!("Reset {}", old.name()));
    Ok(Json::object([("path", Json::from(format!("effects/{}", old.name())))]))
}

fn pick_color(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let t = a.time("time").unwrap_or(cx.view.time);
    let which = a.text("effect").unwrap_or("").to_string();
    let param = a.text("param").unwrap_or("").trim_matches('/').to_string();
    let point = a.json("point").and_then(Json::as_array).filter(|p| p.len() == 2).and_then(|p| Some((p[0].as_f64()?, p[1].as_f64()?))).ok_or_else(|| Error::params("`point` is [x, y] in the layer's own pixels"))?;
    let (index, name) = {
        let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
        let (list, at) = effect_group(layer, &which)?;
        let g = list.children[at].as_group().expect("a group");
        let p = g.prop(&param).map_err(Error::not_found)?;
        if p.def.kind != veymelo_keyframe::Kind::Color {
            return Err(Error::params(format!("{param} of {} is not a colour", g.name())));
        }
        (at, g.name().to_string())
    };
    let (buf, linear) = veymelo_render::effect_input(cx.project, cid, lid, index, t).ok_or_else(|| Error::refused("this layer has no pixels to pick from"))?;
    let px = buf.pixel(veymelo_geom::vec2(point.0, point.1));
    let (x, y) = (px.x.floor() as i64, px.y.floor() as i64);
    let reach = if a.bool("average") == Some(true) { 2 } else { 0 };
    let mut sum = [0.0f64; 4];
    for yy in y - reach..=y + reach {
        for xx in x - reach..=x + reach {
            let c = buf.img.get(xx, yy);
            (0..4).for_each(|k| sum[k] += c[k] as f64);
        }
    }
    if sum[3] <= 1e-6 {
        return Err(Error::refused(format!("there is nothing to pick at [{}, {}] (it is clear, or off the layer)", point.0, point.1)));
    }
    // the colour without its alpha, as settings are written (not in linear light)
    let straight = |k: usize| {
        let v = (sum[k] / sum[3]).clamp(0.0, 1.0);
        if linear { veymelo_color::linear_to_srgb(v) } else { v }
    };
    let color = veymelo_color::Rgba { r: straight(0), g: straight(1), b: straight(2), a: 1.0 };
    let path = format!("effects/{name}/{param}");
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let now = layer.layer_time(t);
    let p = layer.props.prop_mut(&path).expect("found");
    let v = veymelo_keyframe::Value::Color(color);
    if p.is_animated() {
        let mut keys = p.keys.clone();
        veymelo_keyframe::set_key(&mut keys, veymelo_keyframe::Keyframe::new(now, v));
        p.set_keys(keys).map_err(Error::params)?;
    } else {
        p.set_value(v).map_err(Error::params)?;
    }
    let shown = p.value_at(now).to_json(p.def.kind);
    cx.label = Some(format!("Pick {param} of {name}"));
    Ok(Json::object([("path", Json::from(path)), ("color", shown)]))
}

fn set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let (list, at) = effect_group(layer, a.text("effect").unwrap_or(""))?;
    let Node::Group(g) = &mut list.children[at] else { unreachable!("a group") };
    if let Some(on) = a.bool("enabled") {
        g.enabled = on;
    }
    if let Some(n) = a.text("name") {
        if n.trim().is_empty() {
            return Err(Error::params("a name cannot be empty"));
        }
        g.name = n.to_string();
    }
    let (name, on) = (g.name().to_string(), g.enabled);
    cx.label = Some(format!("Change {name}"));
    let mut o = Map::new();
    o.insert("path", format!("effects/{name}"));
    o.insert("enabled", on);
    Ok(Json::Object(o))
}

fn remove_all(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let name = layer.name.clone();
    let list = layer.props.group_mut("effects").map_err(|_| Error::refused(format!("\"{name}\" takes no effects")))?;
    let n = list.children.len();
    list.children.clear();
    cx.label = Some("Remove All Effects".into());
    Ok(Json::object([("removed", Json::Number(n as f64))]))
}

/// A layer's own picture's box, in its own units, at composition time `t`: [left, top, right,
/// bottom] of what it shows (none if it shows nothing).
pub(super) fn content_box(project: &veymelo_project::Project, cid: u64, lid: u64, t: Tick) -> Option<[f64; 4]> {
    let buf = veymelo_render::layer_pixels(project, cid, lid, t)?;
    let img = &buf.img;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..img.height {
        for x in 0..img.width {
            if img.px[y * img.width + x][3] > 0.5 / 255.0 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    (x0 != usize::MAX).then(|| {
        let at = |x: usize, y: usize| buf.origin + veymelo_geom::vec2(x as f64, y as f64) * (1.0 / buf.scale);
        let (a, b) = (at(x0, y0), at(x1 + 1, y1 + 1));
        [a.x, a.y, b.x, b.y]
    })
}

/// Give a layer's property `path` value `v` at its own time `lt`: a key there if it is animated.
pub(super) fn put(layer: &mut Layer, path: &str, v: Value, lt: Tick) -> Result<()> {
    let p = layer.props.prop_mut(path).map_err(Error::not_found)?;
    if p.is_animated() {
        let mut keys = p.keys.clone();
        veymelo_keyframe::set_key(&mut keys, Keyframe::new(lt, v));
        p.set_keys(keys).map_err(Error::params)
    } else {
        p.set_value(v).map_err(Error::params)
    }
}

/// A layer's position at its own time `lt` set by `f` (from what it is): one property, or x, y
/// and z separated.
pub(super) fn put_position(layer: &mut Layer, lt: Tick, f: impl Fn([f64; 3]) -> [f64; 3]) -> Result<()> {
    let now = layer.position_at(lt);
    let to = f(now);
    if layer.separate_position {
        for (k, axis) in ["positionX", "positionY", "positionZ"].iter().enumerate() {
            put(layer, &format!("transform/{axis}"), Value::Number(to[k]), lt)?;
        }
        Ok(())
    } else {
        put(layer, "transform/position", Value::Vec3(to), lt)
    }
}

fn transform_op(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, ids) = layers_of(cx, a, 1)?;
    let op = a.text("op").unwrap_or("reset").to_string();
    let t = cx.view.time;
    let (cw, ch) = cx.project.comp(cid).map(|c| (c.width as f64, c.height as f64)).expect("found");
    if op == "reset" {
        for id in &ids {
            super::call(cx, "prop.reset", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(*id as f64)), ("path", Json::from("transform"))]))?;
        }
        cx.label = Some("Reset Transform".into());
        return Ok(Json::object([("layers", Json::Number(ids.len() as f64))]));
    }
    // what each shows, in its own units: its size, or the box of its picture (text, shapes)
    let boxes: Vec<Option<[f64; 4]>> = ids
        .iter()
        .map(|id| {
            let layer = cx.project.comp(cid).expect("found").layer(*id).expect("found");
            let (w, h) = super::layer::natural_size(cx.project, layer);
            if w > 0.0 && h > 0.0 && !matches!(layer.source, Source::Null) { Some([0.0, 0.0, w, h]) } else { content_box(cx.project, cid, *id, t) }
        })
        .collect();
    let mut notes = Vec::new();
    for (id, bx) in ids.iter().zip(boxes) {
        let layer = cx.project.comp_mut(cid).expect("found").layer_mut(*id).expect("found");
        let lt = layer.layer_time(t);
        let scale = layer.props.prop("transform/scale").map(|p| p.value_at(lt).as_vec3()).unwrap_or([100.0; 3]);
        match op.as_str() {
            "center" => put_position(layer, lt, |p| [cw / 2.0, ch / 2.0, p[2]])?,
            "fit" | "fitWidth" | "fitHeight" => {
                let Some([x0, y0, x1, y1]) = bx.filter(|b| b[2] > b[0] && b[3] > b[1]) else {
                    notes.push(format!("\"{}\" shows nothing to fit", layer.name));
                    continue;
                };
                let (w, h) = (x1 - x0, y1 - y0);
                let (sx, sy) = match op.as_str() {
                    "fitWidth" => (cw / w, cw / w),
                    "fitHeight" => (ch / h, ch / h),
                    _ => (cw / w, ch / h),
                };
                let anchor_z = layer.props.prop("transform/anchor").map(|p| p.value_at(lt).as_vec3()[2]).unwrap_or(0.0);
                put(layer, "transform/scale", Value::Vec3([sx * 100.0, sy * 100.0, scale[2]]), lt)?;
                put(layer, "transform/anchor", Value::Vec3([(x0 + x1) / 2.0, (y0 + y1) / 2.0, anchor_z]), lt)?;
                put_position(layer, lt, |p| [cw / 2.0, ch / 2.0, p[2]])?;
            }
            "flipH" => put(layer, "transform/scale", Value::Vec3([-scale[0], scale[1], scale[2]]), lt)?,
            _ => put(layer, "transform/scale", Value::Vec3([scale[0], -scale[1], scale[2]]), lt)?,
        }
    }
    cx.label = Some("Transform".into());
    let mut o = Map::new();
    o.insert("layers", ids.len() as f64);
    if !notes.is_empty() {
        o.insert("notes", Json::array(notes.into_iter().map(Json::from)));
    }
    Ok(Json::Object(o))
}

fn center_anchor(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, ids) = layers_of(cx, a, 1)?;
    let t = cx.view.time;
    let middles: Vec<(u64, Option<[f64; 2]>)> = ids.iter().map(|id| (*id, content_box(cx.project, cid, *id, t).map(|b| [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]))).collect();
    let mut out = Vec::new();
    for (id, middle) in middles {
        let Some(m) = middle else { continue };
        let layer = cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found");
        let lt = layer.layer_time(t);
        let get = |l: &Layer, p: &str, d: [f64; 3]| l.props.prop(p).map(|q| q.value_at(lt).as_vec3()).unwrap_or(d);
        let (anchor, scale) = (get(layer, "transform/anchor", [0.0; 3]), get(layer, "transform/scale", [100.0; 3]));
        let turn = layer.props.prop("transform/rotation").map(|p| p.value_at(lt).as_f64()).unwrap_or(0.0).to_radians();
        // where the anchor moves to, as the layer is scaled and turned: the position makes up
        let (dx, dy) = ((m[0] - anchor[0]) * scale[0] / 100.0, (m[1] - anchor[1]) * scale[1] / 100.0);
        let (s, c) = turn.sin_cos();
        let shift = [dx * c - dy * s, dx * s + dy * c];
        put(layer, "transform/anchor", Value::Vec3([m[0], m[1], anchor[2]]), lt)?;
        // every key of the position moves alike, so the layer stays where it is all along
        let moved = |p: &mut veymelo_project::Property, f: &dyn Fn(&Value) -> Value| {
            p.value = f(&p.value);
            for k in &mut p.keys {
                k.value = f(&k.value);
            }
        };
        if layer.separate_position {
            for (axis, d) in [("positionX", shift[0]), ("positionY", shift[1])] {
                if let Ok(p) = layer.props.prop_mut(&format!("transform/{axis}")) {
                    moved(p, &|v| Value::Number(v.as_f64() + d));
                }
            }
        } else if let Ok(p) = layer.props.prop_mut("transform/position") {
            moved(p, &|v| {
                let q = v.as_vec3();
                Value::Vec3([q[0] + shift[0], q[1] + shift[1], q[2]])
            });
        }
        out.push(Json::object([("layer", Json::Number(id as f64)), ("anchor", Json::from([m[0], m[1]]))]));
    }
    cx.label = Some("Center Anchor Point".into());
    Ok(Json::object([("layers", Json::Array(out))]))
}
