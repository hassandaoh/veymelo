//! Commands for the groups a layer has in its lists, whatever they are: masks, effects, shape
//! items, text animators and their selectors, layer styles. Copy one, take it away, rename it,
//! switch it off, move it in its list or into another list of its kind.

use veymelo_json::Value as Json;
use veymelo_project::{Body, Group, Layer, Node};

use super::{COMP, Command, LAYER, Run, find_layer, layer_ids};
use crate::{Args, Ctx, Error, Param, Result, Ty};

const PATH: Param = Param::need("path", Ty::Text, "the group, like masks/Mask 1, effects/Blur, contents/Group 1/contents/Fill 1, text/animators/Animator 1");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "group.duplicate",
            label: "Duplicate",
            summary: "Copy a group of a layer's lists (a mask, an effect, a shape item, an animator, a selector, a layer style) with all it holds, its keys and expressions, just below itself; or to the end of the same list on another layer.",
            params: const { &[COMP, LAYER, PATH, Param::opt("toLayer", Ty::Layer, "the layer to copy it to (default: the same one)"), Param::opt("name", Ty::Text, "the copy's name")] },
            returns: "{path, name}",
            run: Run::Edit(duplicate),
        },
        Command {
            id: "group.remove",
            label: "Delete",
            summary: "Take a group out of a layer's list (a mask, an effect, a shape item, an animator, a selector, a layer style).",
            params: const { &[COMP, LAYER, PATH] },
            returns: "{removed}",
            run: Run::Edit(remove),
        },
        Command {
            id: "group.rename",
            label: "Rename",
            summary: "Give a group of a layer's lists another name (what paths and expressions call it by).",
            params: const { &[COMP, LAYER, PATH, Param::need("name", Ty::Text, "the new name")] },
            returns: "{path, name}",
            run: Run::Edit(rename),
        },
        Command {
            id: "group.enable",
            label: "Switch Group",
            summary: "Switch a group of a layer's lists off or on: it stays, doing nothing while off.",
            params: const { &[COMP, LAYER, PATH, Param::need("on", Ty::Bool, "on (true) or off (false)")] },
            returns: "{path, on}",
            run: Run::Edit(enable),
        },
        Command {
            id: "group.move",
            label: "Move Group",
            summary: "Move a group to another place in its list (`to`, from 1 at the top), or into another list of the same kind on the layer (`into`: a shape item into a group's contents, contents/Group 2/contents).",
            params: const {
                &[
                    COMP,
                    LAYER,
                    PATH,
                    Param::opt("to", Ty::Number, "its place in the list, from 1 (default: the end of `into`, or where it is)"),
                    Param::opt("into", Ty::Text, "another list of the same kind, like contents or contents/Group 2/contents"),
                ]
            },
            returns: "{path, list: [names]}",
            run: Run::Edit(move_group),
        },
    ]
}

/// The list a group is in, and its path.
fn split(path: &str) -> Result<(&str, &str)> {
    let path = path.trim_matches('/');
    path.rsplit_once('/').ok_or_else(|| Error::params(format!("\"{path}\" is not in a list; give the group's whole path, like masks/Mask 1")))
}

/// The list holding the group `path` names, and where in it the group is.
fn place<'a>(layer: &'a mut Layer, path: &str) -> Result<(&'a mut Group, usize)> {
    let (list_path, _) = split(path)?;
    let id = layer.props.group(path.trim_matches('/')).map_err(Error::not_found)?.id;
    let list = layer.props.group_mut(list_path).map_err(Error::not_found)?;
    if !list.is_list() || id == 0 {
        return Err(Error::refused(format!("{} is part of what {} always has, not one of a list; only groups in lists (masks, effects, shape items, animators, selectors, layer styles) can be copied, taken away, renamed or moved", path.trim_matches('/'), list.name())));
    }
    let at = list.children.iter().position(|n| matches!(n, Node::Group(g) if g.id == id)).expect("found");
    Ok((list, at))
}

/// What kind of groups a list holds.
fn holds(list: &Group) -> Option<&'static str> {
    match list.def.body {
        Body::List(kind) => Some(kind),
        _ => None,
    }
}

/// A name for a copy of `name` that `list` does not have yet: "Mask 1" makes "Mask 2".
fn copy_name(list: &Group, name: &str) -> String {
    let taken: Vec<&str> = list.children.iter().map(Node::name).collect();
    if !taken.contains(&name) {
        return name.to_string();
    }
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end();
    (2..).map(|n| format!("{base} {n}")).find(|n| !taken.contains(&n.as_str())).expect("a free name")
}

fn duplicate(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let to = match a.json("toLayer") {
        Some(j) => find_layer(cx.project.comp(cid).expect("found"), j)?,
        None => lid,
    };
    let path = a.text("path").unwrap_or("").trim_matches('/').to_string();
    let (list_path, _) = split(&path)?;
    let source = {
        let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
        let (list, at) = place(layer, &path)?;
        list.children[at].as_group().expect("a group").clone()
    };
    let mut copy = source.clone();
    copy.id = cx.project.alloc();
    copy.instances_mut(&mut |g| g.id = cx.project.alloc());
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(to).expect("found");
    let lname = layer.name.clone();
    let list = layer.props.group_mut(list_path).map_err(|_| Error::refused(format!("\"{lname}\" has no {list_path} to copy it into")))?;
    copy.name = match a.text("name") {
        Some(n) if n.trim().is_empty() => return Err(Error::params("a name has to have something in it")),
        Some(n) => n.to_string(),
        None => copy_name(list, source.name()),
    };
    if list.children.iter().any(|n| n.name() == copy.name) {
        return Err(Error::refused(format!("{list_path} already has \"{}\"", copy.name)));
    }
    let name = copy.name.clone();
    let at = if to == lid { list.children.iter().position(|n| matches!(n, Node::Group(g) if g.id == source.id)).map_or(list.children.len(), |i| i + 1) } else { list.children.len() };
    list.children.insert(at, Node::Group(copy));
    cx.label = Some(format!("Duplicate {}", source.name()));
    Ok(Json::object([("path", Json::from(format!("{list_path}/{name}"))), ("name", Json::from(name))]))
}

fn remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let path = a.text("path").unwrap_or("").trim_matches('/').to_string();
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let (list, at) = place(layer, &path)?;
    let gone = list.children.remove(at);
    cx.label = Some(format!("Delete {}", gone.name()));
    Ok(Json::object([("removed", Json::from(path))]))
}

fn rename(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let path = a.text("path").unwrap_or("").trim_matches('/').to_string();
    let name = a.text("name").unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err(Error::params("a name has to have something in it"));
    }
    if name.contains('/') || name.starts_with('@') || name.starts_with('#') {
        return Err(Error::params("a name cannot have \"/\" in it or start with \"@\" or \"#\" (paths use them)"));
    }
    let (list_path, _) = split(&path)?;
    let list_path = list_path.to_string();
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let (list, at) = place(layer, &path)?;
    if list.children.iter().enumerate().any(|(i, n)| i != at && n.name() == name) {
        return Err(Error::refused(format!("{list_path} already has \"{name}\"")));
    }
    let Node::Group(g) = &mut list.children[at] else { unreachable!("a group") };
    let old = g.name().to_string();
    g.name = name.clone();
    cx.label = Some(format!("Rename {old}"));
    Ok(Json::object([("path", Json::from(format!("{list_path}/{name}"))), ("name", Json::from(name))]))
}

fn enable(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let path = a.text("path").unwrap_or("").trim_matches('/').to_string();
    let on = a.bool("on").unwrap_or(true);
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let (list, at) = place(layer, &path)?;
    let Node::Group(g) = &mut list.children[at] else { unreachable!("a group") };
    g.enabled = on;
    cx.label = Some(format!("{} {}", if on { "Switch on" } else { "Switch off" }, g.name()));
    Ok(Json::object([("path", Json::from(path)), ("on", Json::Bool(on))]))
}

fn move_group(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let path = a.text("path").unwrap_or("").trim_matches('/').to_string();
    let (from_list, _) = split(&path)?;
    let from_list = from_list.to_string();
    let into = a.text("into").map(|p| p.trim_matches('/').to_string());
    if into.is_none() && !a.has("to") {
        return Err(Error::params("give `to` (its place in the list) or `into` (another list)"));
    }
    if let Some(into) = &into
        && into.starts_with(&format!("{path}/"))
    {
        return Err(Error::refused(format!("{path} cannot go inside itself")));
    }
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let (list, at) = place(layer, &path)?;
    let (kind, id) = (holds(list), list.children[at].as_group().expect("a group").id);
    let node = list.children.remove(at);
    let target = into.clone().unwrap_or_else(|| from_list.clone());
    let name = node.name().to_string();
    let undo = |layer: &mut Layer, node: Node| {
        if let Ok(l) = layer.props.group_mut(&from_list) {
            l.children.insert(at, node);
        }
    };
    // into another list: one of the same kind, not inside what moves
    let ok = match layer.props.group(&target) {
        Ok(l) if !l.is_list() => Err(format!("{target} is not a list")),
        Ok(l) if holds(l) != kind => Err(format!("{target} holds {}, not what {from_list} holds", l.def.name)),
        Ok(l) if l.children.iter().any(|n| n.name() == name) => Err(format!("{target} already has \"{name}\"")),
        Ok(_) => Ok(()),
        Err(why) => Err(why),
    };
    if let Err(why) = ok {
        undo(layer, node);
        return Err(Error::refused(why));
    }
    let list = layer.props.group_mut(&target).expect("checked");
    let n = list.children.len();
    let to = match a.num("to") {
        Some(k) if k >= 1.0 && k.fract() == 0.0 => (k as usize - 1).min(n),
        Some(k) => {
            undo(layer, node);
            return Err(Error::params(format!("`to` is a place from 1, not {k}")));
        }
        None => n,
    };
    list.children.insert(to, node);
    let names: Vec<Json> = list.children.iter().map(|c| Json::from(c.name())).collect();
    debug_assert!(list.children.iter().any(|c| matches!(c, Node::Group(g) if g.id == id)));
    cx.label = Some(format!("Move {name}"));
    Ok(Json::object([("path", Json::from(format!("{target}/{name}"))), ("list", Json::Array(names))]))
}
