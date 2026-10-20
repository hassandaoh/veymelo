//! Commands that add to a layer's lists: shape items in a shape layer's contents, and masks.

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{ShapePath, Value};
use veymelo_project::{Group, GroupDef, Layer, Node, Source};

use super::{COMP, Command, LAYER, Run, fresh_name, layer_ids};
use crate::{Args, Ctx, Error, Param, Result, Ty};

/// The types of shape item, by id.
const SHAPE_TYPES: &[&str] = &["group", "rect", "ellipse", "star", "path", "fill", "stroke", "gradientFill", "gradientStroke", "trim", "round", "offset", "pucker", "twist", "zigzag", "wiggle", "merge", "repeater"];

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "shape.add",
            label: "Add Shape Item",
            summary: "Add an item to a shape layer: a path (rect, ellipse, star, path), a paint (fill, stroke, gradientFill, gradientStroke), an operator (trim, round, offset, pucker, twist, zigzag, wiggle, merge, repeater) or a group. Paints paint the paths above them in their group; operators change the paths above them.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::need("type", Ty::Choice(SHAPE_TYPES), "what to add"),
                    Param::opt("into", Ty::Text, "a group to add it to, like \"Group 1\" (default: the layer's contents)"),
                    Param::opt("at", Ty::Whole, "its place in the list, 1 being the top (default: the bottom)"),
                    Param::opt("name", Ty::Text, "what to call it (default: Rectangle 1, Fill 1…)"),
                    Param::opt("props", Ty::Object, "its values to start with, by path: {\"size\": [200, 100], \"color\": \"#ff0000\", \"transform/position\": [0, 50]}"),
                ]
            },
            returns: "{path, id, name}",
            run: Run::Edit(shape_add),
        },
        Command {
            id: "shape.remove",
            label: "Remove Shape Item",
            summary: "Take an item (and everything in it, for a group) out of a shape layer.",
            params: const { &[COMP, LAYER, Param::need("item", Ty::Text, "the item, like \"Rectangle 1\" or \"Group 1/contents/Fill 1\"")] },
            returns: "{removed}",
            run: Run::Edit(shape_remove),
        },
        Command {
            id: "item.remove",
            label: "Remove Item",
            summary: "Take any item out of the list it is in, by its path: a mask, a shape item, an effect, a puppet mesh or pin, a text animator or selector.",
            params: const { &[COMP, LAYER, Param::need("path", Ty::Text, "the item, like \"effects/Puppet/meshes/Mesh 2\" or \"text/animators/Animator 1\"")] },
            returns: "{removed}",
            run: Run::Edit(item_remove),
        },
        Command {
            id: "mask.add",
            label: "Add Mask",
            summary: "Add a mask to a layer: a rectangle, an ellipse or a path in the layer's own pixels (default: a rectangle as big as the layer).",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::opt("rect", Ty::List, "[x, y, width, height] in the layer's pixels"),
                    Param::opt("ellipse", Ty::List, "[x, y, width, height] of the box round it"),
                    Param::opt("path", Ty::Object, "{\"points\": [[x, y]…], \"inTangents\"?: …, \"outTangents\"?: …}"),
                    Param::opt("mode", Ty::Choice(veymelo_project::MASK_MODES), "how it joins the masks above it (default add)"),
                    Param::opt("inverted", Ty::Bool, "keep the outside instead"),
                    Param::opt("feather", Ty::Any, "how soft its edge is: a number, or [across, down]"),
                    Param::opt("opacity", Ty::Number, "0 to 100"),
                    Param::opt("expansion", Ty::Number, "grow (or below zero, shrink) it, in pixels"),
                    Param::opt("name", Ty::Text, "what to call it (default: Mask 1…)"),
                ]
            },
            returns: "{path, id, name}",
            run: Run::Edit(mask_add),
        },
        Command {
            id: "mask.remove",
            label: "Remove Mask",
            summary: "Take a mask off a layer.",
            params: const { &[COMP, LAYER, Param::need("mask", Ty::Text, "the mask, like \"Mask 1\" or \"#1\"")] },
            returns: "{removed}",
            run: Run::Edit(mask_remove),
        },
    ]
}

/// The layer the parameters name, to change.
fn the_layer<'a>(cx: &'a mut Ctx, a: &Args) -> Result<&'a mut Layer> {
    let (cid, id) = layer_ids(cx.project, cx.view, a)?;
    Ok(cx.project.comp_mut(cid).expect("found").layer_mut(id).expect("found"))
}

/// Give a new instance the values in `props` (paths inside it to JSON values).
pub(super) fn start_with(g: &mut Group, props: Option<&Json>) -> Result<()> {
    let Some(Json::Object(m)) = props else { return Ok(()) };
    for (path, v) in m.iter() {
        let p = g.prop_mut(path).map_err(|e| Error::params(format!("`props`: {e}")))?;
        let value = Value::from_json(p.def.kind, v).map_err(|e| Error::params(format!("`props.{path}`: {e}")))?;
        p.set_value(value).map_err(|e| Error::params(format!("`props.{path}`: {e}")))?;
    }
    Ok(())
}

/// Put a new instance of `def` into `list`, named and numbered, at `at` (1-based, default the
/// end). Where it went, by name.
pub(super) fn insert(list: &mut Group, def: &'static GroupDef, id: u64, name: Option<&str>, at: Option<u64>, props: Option<&Json>) -> Result<(String, usize)> {
    let name = match name {
        Some(n) => n.to_string(),
        None => fresh_name(def.name, list.children.iter().map(Node::name)),
    };
    let mut g = Group::instance(def, id, &name);
    start_with(&mut g, props)?;
    let index = at.map_or(list.children.len(), |a| (a.max(1) as usize - 1).min(list.children.len()));
    list.children.insert(index, Node::Group(g));
    Ok((name, index))
}

fn shape_add(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let ty = a.text("type").unwrap_or("");
    let def = cx.catalog.find("contents", ty).ok_or_else(|| Error::params(format!("there is no shape item type \"{ty}\"")))?;
    let id = cx.project.alloc();
    let layer = the_layer(cx, a)?;
    if layer.source != Source::Shape {
        return Err(Error::refused(format!("\"{}\" is a {} layer; shape items go in shape layers (layer.add type shape)", layer.name, layer.source.id())));
    }
    let base = match a.text("into") {
        None => "contents".to_string(),
        Some(g) => format!("contents/{}/contents", g.trim_matches('/').trim_start_matches("contents/")),
    };
    let list = layer.props.group_mut(&base).map_err(Error::not_found)?;
    if !list.is_list() {
        return Err(Error::params(format!("`into` names {base}, which is not a group's contents")));
    }
    let (name, _) = insert(list, def, id, a.text("name"), a.whole("at"), a.json("props"))?;
    let path = format!("{base}/{name}");
    cx.label = Some(format!("Add {name}"));
    Ok(Json::object([("path", Json::from(path)), ("id", Json::Number(id as f64)), ("name", Json::from(name))]))
}

/// Take the instance at `path` out of the list it is in.
pub(super) fn remove_at(layer: &mut Layer, path: &str) -> Result<u64> {
    let (list_path, item) = path.rsplit_once('/').ok_or_else(|| Error::params(format!("{path} is not inside a list")))?;
    let list = layer.props.group_mut(list_path).map_err(Error::not_found)?;
    let target = list.group(item).map_err(Error::not_found)?;
    let id = target.id;
    list.children.retain(|n| !matches!(n, Node::Group(g) if g.id == id));
    Ok(id)
}

fn shape_remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let layer = the_layer(cx, a)?;
    let item = a.text("item").unwrap_or("").trim_matches('/');
    let path = if item.starts_with("contents/") { item.to_string() } else { format!("contents/{item}") };
    let id = remove_at(layer, &path)?;
    cx.label = Some(format!("Remove {item}"));
    Ok(Json::object([("removed", id as f64)]))
}

fn item_remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let layer = the_layer(cx, a)?;
    let path = a.text("path").unwrap_or("").trim_matches('/').to_string();
    let (list, _) = path.rsplit_once('/').ok_or_else(|| Error::params(format!("{path} is not inside a list")))?;
    if !layer.props.group(list).map_err(Error::not_found)?.is_list() {
        return Err(Error::params(format!("{list} is not a list; only what is in a list can be taken out")));
    }
    let id = remove_at(layer, &path)?;
    cx.label = Some(format!("Remove {}", path.rsplit('/').next().unwrap_or(&path)));
    Ok(Json::object([("removed", id as f64)]))
}

/// Four numbers, [x, y, width, height].
pub(super) fn bounds(j: &Json, what: &str) -> Result<[f64; 4]> {
    let v: Option<Vec<f64>> = j.as_array().map(|a| a.iter().filter_map(Json::as_f64).collect());
    match v {
        Some(v) if v.len() == 4 => Ok([v[0], v[1], v[2], v[3]]),
        _ => Err(Error::params(format!("`{what}` is [x, y, width, height]"))),
    }
}

fn mask_add(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = cx.project.alloc();
    let size = {
        let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
        let comp = cx.project.comp(cid).expect("found");
        let layer = comp.layer(lid).expect("found");
        match &layer.source {
            Source::Solid { width, height, .. } => Some((*width as f64, *height as f64)),
            Source::Comp(c) => cx.project.comp(*c).map(|c| (c.width as f64, c.height as f64)),
            _ => None,
        }
    };
    let given = ["rect", "ellipse", "path"].iter().filter(|k| a.has(k)).count();
    if given > 1 {
        return Err(Error::params("give one of `rect`, `ellipse` or `path`"));
    }
    let path = if let Some(r) = a.json("rect") {
        let [x, y, w, h] = bounds(r, "rect")?;
        ShapePath::polygon(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]], true)
    } else if let Some(e) = a.json("ellipse") {
        let [x, y, w, h] = bounds(e, "ellipse")?;
        let s = veymelo_path::shape::ellipse([w, h], [x + w / 2.0, y + h / 2.0]);
        ShapePath { points: s.v.iter().map(|v| v.p).collect(), in_tangents: s.v.iter().map(|v| v.i).collect(), out_tangents: s.v.iter().map(|v| v.o).collect(), closed: true, feather: veymelo_keyframe::Feathers::NONE }
    } else if let Some(p) = a.json("path") {
        let Value::Path(mut p) = Value::from_json(veymelo_keyframe::Kind::Path, p).map_err(|e| Error::params(format!("`path`: {e}")))? else {
            return Err(Error::params("`path` is a path"));
        };
        p.closed = true;
        p
    } else {
        let (w, h) = size.ok_or_else(|| Error::params("this layer has no size of its own: give `rect`, `ellipse` or `path`"))?;
        ShapePath::polygon(&[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]], true)
    };
    let mut props = Map::new();
    props.insert("path", Value::Path(path).to_json(veymelo_keyframe::Kind::Path));
    for key in ["mode", "inverted", "opacity", "expansion"] {
        if let Some(v) = a.json(key) {
            props.insert(key, v.clone());
        }
    }
    if let Some(f) = a.json("feather") {
        let pair = match f {
            Json::Number(n) => Json::from([*n, *n]),
            other => other.clone(),
        };
        props.insert("feather", pair);
    }
    let def = cx.catalog.find("masks", "mask").expect("masks are in the catalog");
    let layer = the_layer(cx, a)?;
    let list = layer.props.group_mut("masks").map_err(|_| Error::refused(format!("\"{}\" is a {} layer, which has no masks", layer.name, layer.source.id())))?;
    let (name, _) = insert(list, def, id, a.text("name"), None, Some(&Json::Object(props)))?;
    cx.label = Some(format!("Add {name}"));
    Ok(Json::object([("path", Json::from(format!("masks/{name}"))), ("id", Json::Number(id as f64)), ("name", Json::from(name))]))
}

fn mask_remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let layer = the_layer(cx, a)?;
    let mask = a.text("mask").unwrap_or("");
    let id = remove_at(layer, &format!("masks/{}", mask.trim_start_matches("masks/")))?;
    cx.label = Some(format!("Remove {mask}"));
    Ok(Json::object([("removed", id as f64)]))
}
