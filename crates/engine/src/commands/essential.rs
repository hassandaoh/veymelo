//! Commands for essential properties: the controls a composition opens to the layers that show
//! it (a property of one of its layers and others linked to it, a mirror, a footage layer to
//! swap, a comment, a group), what each such layer sets for itself, and templates: a
//! composition and what it uses in a file of its own, to bring into other projects.

use std::collections::HashSet;
use std::path::PathBuf;

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Kind, Value};
use veymelo_project::essential::{self, Control, ControlKind, Show, Target};
use veymelo_project::{Comp, Expression, ItemKind, Layer, Node, Project, Property, Source};

use super::{COMP, Command, LAYER, Run, comp_id, find_footage, find_layer, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

const CONTROL: Param = Param::need("control", Ty::Any, "the control: its id or name");
const GROUP: Param = Param::opt("group", Ty::Any, "the group to put it in: its id or name (default: the top)");
const AT: Param = Param::opt("at", Ty::Whole, "its place in its list, 1 being the first (default: the end)");
const TEMPLATE: &str = "Veymelo template";

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "essential.add",
            label: "Add Property to Essential Graphics",
            summary: "Open a property of one of a composition's layers to the layers that show the composition: each of them gets it under essentials/<name>, to set for itself (prop.set, prop.keys, an expression) while the rest follow the composition. A Source Text can be opened as its font only (as: font), a scale as one number (as: scale).",
            params: const {
                &[
                    COMP,
                    Param::need("layer", Ty::Layer, "the composition's layer the property is on"),
                    Param::need("path", Ty::Text, "the property, like \"text/sourceText\" or \"effects/Fill/color\""),
                    Param::opt("name", Ty::Text, "what it is called (default: the property's name)"),
                    Param::opt("as", Ty::Choice(Show::IDS), "whole (the default), font or scale"),
                    GROUP,
                    AT,
                ]
            },
            returns: "{control, name}",
            run: Run::Edit(add),
        },
        Command {
            id: "essential.addMirror",
            label: "Add Mirror",
            summary: "Show a control again under a name of its own: one value, setting either sets both.",
            params: const { &[COMP, CONTROL, Param::opt("name", Ty::Text, "what it is called (default: the control's name)"), GROUP, AT] },
            returns: "{control, name}",
            run: Run::Edit(add_mirror),
        },
        Command {
            id: "essential.addMedia",
            label: "Add Media Replacement",
            summary: "Let the layers that show a composition each swap the footage one of its footage layers shows.",
            params: const { &[COMP, Param::need("layer", Ty::Layer, "the footage layer"), Param::opt("name", Ty::Text, "what it is called (default: the layer's name)"), GROUP, AT] },
            returns: "{control, name}",
            run: Run::Edit(add_media),
        },
        Command {
            id: "essential.addComment",
            label: "Add Comment",
            summary: "Put a note among a composition's controls.",
            params: const { &[COMP, Param::need("text", Ty::Text, "the note"), GROUP, AT] },
            returns: "{control}",
            run: Run::Edit(add_comment),
        },
        Command {
            id: "essential.addGroup",
            label: "Add Group",
            summary: "Make a named group to keep controls together.",
            params: const { &[COMP, Param::opt("name", Ty::Text, "what it is called (default: Group 1…)"), GROUP, AT] },
            returns: "{control, name}",
            run: Run::Edit(add_group),
        },
        Command {
            id: "essential.link",
            label: "Link Property to Control",
            summary: "Make another property of the composition follow a property control (the same kind of value): set in the composition or by a layer showing it, both take the value.",
            params: const { &[COMP, CONTROL, Param::need("layer", Ty::Layer, "the layer of the property"), Param::need("path", Ty::Text, "the property")] },
            returns: "{control, links}",
            run: Run::Edit(link),
        },
        Command {
            id: "essential.unlink",
            label: "Unlink Property",
            summary: "Let a property no longer follow a control.",
            params: const { &[COMP, CONTROL, Param::need("layer", Ty::Layer, "the layer of the property"), Param::need("path", Ty::Text, "the property")] },
            returns: "{control, links}",
            run: Run::Edit(unlink),
        },
        Command {
            id: "essential.list",
            label: "Essential Graphics Controls",
            summary: "A composition's controls in order (groups with what is in them): each one's id, name, kind, what it stands for and its value in the composition.",
            params: const { &[COMP] },
            returns: "{name, controls: [{id, name, type, layer?, path?, as?, links?, of?, text?, value?, children?}]}",
            run: Run::Look(list),
        },
        Command {
            id: "essential.move",
            label: "Move Control",
            summary: "Put a control at another place, in a group or at the top.",
            params: const { &[COMP, CONTROL, Param::opt("group", Ty::Any, "the group (its id or name), or null for the top"), AT] },
            returns: "{control}",
            run: Run::Edit(move_control),
        },
        Command {
            id: "essential.rename",
            label: "Rename Control",
            summary: "Give a control (or a group) another name; layers showing the composition see it under that name.",
            params: const { &[COMP, CONTROL, Param::need("name", Ty::Text, "the new name")] },
            returns: "{control, name}",
            run: Run::Edit(rename),
        },
        Command {
            id: "essential.remove",
            label: "Remove Control",
            summary: "Take a control away (a group with what is in it; mirrors of it too). Layers showing the composition lose what they set for it.",
            params: const { &[COMP, CONTROL] },
            returns: "{removed}",
            run: Run::Edit(remove),
        },
        Command {
            id: "essential.setName",
            label: "Essential Graphics Name",
            summary: "What the composition is called as a template.",
            params: const { &[COMP, Param::need("name", Ty::Text, "the name")] },
            returns: "{name}",
            run: Run::Edit(set_name),
        },
        Command {
            id: "essential.set",
            label: "Set Essential Property",
            summary: "Set what a layer showing a composition takes for one of its controls: a value, keys or an expression (as prop.set, prop.keys and prop.expression on essentials/<name> do), or for a media replacement the footage.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    CONTROL,
                    Param::opt("value", Ty::Any, "the value"),
                    Param::opt("keys", Ty::List, "keyframes, as prop.keys takes them"),
                    Param::opt("expression", Ty::Text, "an expression (\"\" for none)"),
                    Param::opt("item", Ty::Any, "for a media replacement: the footage (its id or name)"),
                ]
            },
            returns: "{control, value, overridden}",
            run: Run::Edit(set),
        },
        Command {
            id: "essential.revert",
            label: "Revert Essential Property",
            summary: "Let a layer showing a composition follow the composition again for a control, or for every one.",
            params: const { &[COMP, LAYER, Param::opt("control", Ty::Any, "the control (default: every one)")] },
            returns: "{reverted}",
            run: Run::Edit(revert),
        },
        Command {
            id: "essential.push",
            label: "Push Override Values to Source",
            summary: "Make what a layer set for a control (or every control) the composition's own value, so every layer that does not set it takes it; the layer then follows the composition again.",
            params: const { &[COMP, LAYER, Param::opt("control", Ty::Any, "the control (default: every one the layer set)")] },
            returns: "{pushed}",
            run: Run::Edit(push),
        },
        Command {
            id: "essential.instance",
            label: "Essential Properties of a Layer",
            summary: "What a layer showing a composition has of its controls: each one's path, value and whether the layer set it.",
            params: const { &[COMP, LAYER] },
            returns: "[{control, name, path, value, overridden}]",
            run: Run::Look(instance),
        },
        Command {
            id: "essential.exportTemplate",
            label: "Export Essential Graphics Template",
            summary: "Write a composition with what it uses (compositions inside it, footage) as a template file, to bring into other projects with essential.importTemplate.",
            params: const { &[COMP, Param::need("path", Ty::Text, "the file (.json)"), Param::opt("name", Ty::Text, "its name (default: the composition's essential graphics name, or its own)")] },
            returns: "{path, name, items}",
            run: Run::Look(export_template),
        },
        Command {
            id: "essential.importTemplate",
            label: "Import Essential Graphics Template",
            summary: "Bring a template's composition and what it uses into the project, in a folder of their own; with addToComp, a layer showing it is put in a composition too.",
            params: const { &[Param::need("path", Ty::Text, "the template file"), Param::opt("addToComp", Ty::Comp, "a composition to put a layer showing it in")] },
            returns: "{comp, folder, items, layer?}",
            run: Run::Edit(import_template),
        },
        Command {
            id: "essential.templateInfo",
            label: "Template Info",
            summary: "What a template file holds: its name, its composition and its controls.",
            params: const { &[Param::need("path", Ty::Text, "the template file")] },
            returns: "{name, comp, width, height, duration, controls}",
            run: Run::Look(template_info),
        },
    ]
}

/// The control `j` names in `ess`: its id or name.
fn find_control(ess: &essential::Essentials, j: &Json) -> Result<u64> {
    let all = ess.flat();
    let found = match j {
        Json::Number(_) => j.as_u64().filter(|id| all.iter().any(|c| c.id == *id)),
        Json::String(n) => all.iter().find(|c| c.name == *n).or_else(|| all.iter().find(|c| c.name.eq_ignore_ascii_case(n))).map(|c| c.id),
        _ => None,
    };
    found.ok_or_else(|| {
        let have: Vec<String> = all.iter().map(|c| format!("\"{}\" ({}, {})", c.name, c.kind.id(), c.id)).collect();
        Error::not_found(format!("there is no control {} ({})", j.to_json(), if have.is_empty() { "it has none".into() } else { format!("there is {}", have.join(", ")) }))
    })
}

/// The group `group` names (none: the top).
fn group_of(ess: &essential::Essentials, a: &Args) -> Result<Option<u64>> {
    match a.json("group") {
        None | Some(Json::Null) => Ok(None),
        Some(j) => {
            let id = find_control(ess, j)?;
            if !matches!(ess.find(id).map(|c| &c.kind), Some(ControlKind::Group { .. })) {
                return Err(Error::params(format!("{} is not a group", j.to_json())));
            }
            Ok(Some(id))
        }
    }
}

/// A property's path as its layer's walk gives it (instances by id), whatever it was written as.
fn canonical(layer: &Layer, path: &str) -> Result<(String, Property)> {
    let p = layer.props.prop(path).map_err(Error::not_found)?;
    let ptr: *const Property = p;
    let mut found = None;
    layer.props.walk(&mut |q, prop| {
        if std::ptr::eq(prop, ptr) && found.is_none() {
            found = Some(q.to_string());
        }
    });
    let path = found.ok_or_else(|| Error::not_found(format!("{path} is not a property of \"{}\"", layer.name)))?;
    if path.starts_with("essentials/") {
        return Err(Error::refused("a layer's own essential properties cannot be opened again; open the property inside its composition"));
    }
    Ok((path, p.clone()))
}

/// A name not yet taken among the controls.
fn fresh(ess: &essential::Essentials, base: &str) -> String {
    let taken = |n: &str| ess.flat().iter().any(|c| c.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base} {n}")).find(|n| !taken(n)).expect("some number is free")
}

/// Put a new control in its place. Its id.
fn put(cx: &mut Ctx, cid: u64, a: &Args, make: impl FnOnce(u64, &essential::Essentials) -> Result<Control>) -> Result<(u64, String)> {
    let id = cx.project.alloc();
    let before = cx.project.clone();
    let comp = cx.project.comp_mut(cid).expect("found");
    let group = group_of(&comp.essentials, a)?;
    let control = make(id, &comp.essentials)?;
    let name = control.name.clone();
    let list = comp.essentials.list_mut(group).expect("a group");
    let at = a.whole("at").map_or(list.len(), |k| (k.max(1) as usize - 1).min(list.len()));
    list.insert(at, control);
    essential::sync(&before, cx.project);
    Ok((id, name))
}

fn add(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let lid = find_layer(comp, a.need("layer"))?;
    let layer = comp.layer(lid).expect("found");
    let (path, p) = canonical(layer, a.text("path").unwrap_or(""))?;
    if !essential::can_open(&p) {
        return Err(Error::refused(format!("{path} cannot be a control (only values to set: not paths, gradients or layer pickers)")));
    }
    let show = a.text("as").and_then(Show::from_id).unwrap_or_default();
    match show {
        Show::Font if p.def.kind != Kind::Doc => return Err(Error::params("only a Source Text can be opened as its font")),
        Show::Scale if !matches!(p.def.kind, Kind::Vec2 | Kind::Vec3) => return Err(Error::params("only a property of two or three numbers can be opened as one scale")),
        _ => {}
    }
    if let Some(c) = comp.essentials.control_for(lid, &path, show) {
        return Err(Error::refused(format!("it is a control already: \"{}\" ({})", c.name, c.id)));
    }
    let base = a.text("name").map(str::to_string).unwrap_or_else(|| p.def.name.to_string());
    let target = Target { layer: lid, path };
    let (id, name) = put(cx, cid, a, |id, ess| Ok(Control { id, name: if a.has("name") { base.clone() } else { fresh(ess, &base) }, kind: ControlKind::Property { target, links: Vec::new(), show } }))?;
    cx.label = Some(format!("Add {name} to Essential Graphics"));
    Ok(Json::object([("control", Json::Number(id as f64)), ("name", Json::from(name))]))
}

fn add_mirror(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let ess = &cx.project.comp(cid).expect("found").essentials;
    let of = find_control(ess, a.need("control"))?;
    let master = ess.resolve(of).filter(|c| matches!(c.kind, ControlKind::Property { .. })).ok_or_else(|| Error::refused("only a property control can be mirrored"))?;
    let base = a.text("name").map(str::to_string).unwrap_or_else(|| master.name.clone());
    let master = master.id;
    let (id, name) = put(cx, cid, a, |id, ess| Ok(Control { id, name: fresh(ess, &base), kind: ControlKind::Mirror { of: master } }))?;
    cx.label = Some(format!("Add Mirror {name}"));
    Ok(Json::object([("control", Json::Number(id as f64)), ("name", Json::from(name))]))
}

fn add_media(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let lid = find_layer(comp, a.need("layer"))?;
    let layer = comp.layer(lid).expect("found");
    if !matches!(layer.source, Source::Footage(_)) {
        return Err(Error::refused(format!("\"{}\" is a {} layer; media replacement is for footage layers", layer.name, layer.source.id())));
    }
    if comp.essentials.flat().iter().any(|c| matches!(c.kind, ControlKind::Media { layer } if layer == lid)) {
        return Err(Error::refused(format!("\"{}\" is a media replacement already", layer.name)));
    }
    let base = a.text("name").map(str::to_string).unwrap_or_else(|| layer.name.clone());
    let (id, name) = put(cx, cid, a, |id, ess| Ok(Control { id, name: fresh(ess, &base), kind: ControlKind::Media { layer: lid } }))?;
    cx.label = Some(format!("Add Media Replacement {name}"));
    Ok(Json::object([("control", Json::Number(id as f64)), ("name", Json::from(name))]))
}

fn add_comment(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let text = a.text("text").unwrap_or("").to_string();
    let (id, _) = put(cx, cid, a, |id, _| Ok(Control { id, name: String::new(), kind: ControlKind::Comment { text } }))?;
    cx.label = Some("Add Comment".into());
    Ok(Json::object([("control", Json::Number(id as f64))]))
}

fn add_group(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let given = a.text("name").map(str::to_string);
    let (id, name) = put(cx, cid, a, |id, ess| {
        let name = given.clone().unwrap_or_else(|| super::fresh_name("Group", ess.flat().iter().map(|c| c.name.as_str()).collect::<Vec<_>>().into_iter()));
        Ok(Control { id, name, kind: ControlKind::Group { children: Vec::new() } })
    })?;
    cx.label = Some(format!("Add Group {name}"));
    Ok(Json::object([("control", Json::Number(id as f64)), ("name", Json::from(name))]))
}

fn links_json(c: &Control) -> Json {
    match &c.kind {
        ControlKind::Property { links, .. } => Json::array(links.iter().map(|t| Json::object([("layer", Json::Number(t.layer as f64)), ("path", Json::from(t.path.as_str()))]))),
        _ => Json::Array(Vec::new()),
    }
}

fn link_or_unlink(cx: &mut Ctx, a: &Args, on: bool) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let before = cx.project.clone();
    let comp = cx.project.comp(cid).expect("found");
    let id = find_control(&comp.essentials, a.need("control"))?;
    let lid = find_layer(comp, a.need("layer"))?;
    let (path, p) = canonical(comp.layer(lid).expect("found"), a.text("path").unwrap_or(""))?;
    let main = essential::source(comp, id).map(|(_, q)| q.def.kind).ok_or_else(|| Error::refused("only a property control takes links"))?;
    if on && p.def.kind != main {
        return Err(Error::refused(format!("{path} holds another kind of value than the control")));
    }
    let comp = cx.project.comp_mut(cid).expect("found");
    let Some(Control { kind: ControlKind::Property { target, links, .. }, .. }) = comp.essentials.find_mut(id) else { return Err(Error::refused("only a property control takes links (a mirror: link its control)")) };
    let t = Target { layer: lid, path };
    if on {
        if *target == t || links.contains(&t) {
            return Err(Error::refused("it follows the control already"));
        }
        links.push(t);
    } else {
        let n = links.len();
        links.retain(|k| *k != t);
        if links.len() == n {
            return Err(Error::not_found("it does not follow the control"));
        }
    }
    let out = links_json(comp.essentials.find(id).expect("found"));
    // (what now follows takes the control's value as the change is settled)
    essential::sync(&before, cx.project);
    cx.label = Some(if on { "Link Property to Control" } else { "Unlink Property" }.into());
    Ok(Json::object([("control", Json::Number(id as f64)), ("links", out)]))
}

fn link(cx: &mut Ctx, a: &Args) -> Result<Json> {
    link_or_unlink(cx, a, true)
}

fn unlink(cx: &mut Ctx, a: &Args) -> Result<Json> {
    link_or_unlink(cx, a, false)
}

fn control_json(comp: &Comp, c: &Control) -> Json {
    let mut o = Map::new();
    o.insert("id", c.id as f64);
    o.insert("name", c.name.as_str());
    o.insert("type", c.kind.id());
    match &c.kind {
        ControlKind::Property { target, show, .. } => {
            o.insert("layer", target.layer as f64);
            o.insert("path", target.path.as_str());
            if *show != Show::Whole {
                o.insert("as", show.id());
            }
            o.insert("links", links_json(c));
        }
        ControlKind::Mirror { of } => {
            o.insert("of", *of as f64);
        }
        ControlKind::Media { layer } => {
            o.insert("layer", *layer as f64);
            if let Some(&Source::Footage(item)) = comp.layer(*layer).map(|l| &l.source) {
                o.insert("item", item as f64);
            }
        }
        ControlKind::Comment { text } => {
            o.insert("text", text.as_str());
        }
        ControlKind::Group { children } => {
            o.insert("children", Json::array(children.iter().map(|k| control_json(comp, k))));
        }
    }
    if let Some((_, p)) = essential::source(comp, c.id) {
        o.insert("value", p.value_at(veymelo_time::Tick::ZERO).to_json(p.def.kind));
        if p.is_animated() {
            o.insert("animated", true);
        }
    } else if matches!(c.kind, ControlKind::Property { .. } | ControlKind::Mirror { .. }) {
        o.insert("broken", true);
    }
    Json::Object(o)
}

fn list(l: &Look, a: &Args) -> Result<Json> {
    let cid = comp_id(l.project, l.view, a)?;
    let comp = l.project.comp(cid).expect("found");
    Ok(Json::object([("name", Json::from(comp.essentials.name.as_str())), ("controls", Json::array(comp.essentials.controls.iter().map(|c| control_json(comp, c))))]))
}

fn move_control(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let before = cx.project.clone();
    let comp = cx.project.comp_mut(cid).expect("found");
    let id = find_control(&comp.essentials, a.need("control"))?;
    let group = group_of(&comp.essentials, a)?;
    // not into itself
    if let Some(g) = group {
        let inside = |c: &Control| -> bool {
            fn has(c: &Control, id: u64) -> bool {
                c.id == id || matches!(&c.kind, ControlKind::Group { children } if children.iter().any(|k| has(k, id)))
            }
            has(c, g)
        };
        if comp.essentials.find(id).is_some_and(inside) {
            return Err(Error::refused("a group cannot go inside itself"));
        }
    }
    let c = comp.essentials.remove(id).expect("found");
    let list = comp.essentials.list_mut(group).expect("a group");
    let at = a.whole("at").map_or(list.len(), |k| (k.max(1) as usize - 1).min(list.len()));
    list.insert(at, c);
    essential::sync(&before, cx.project);
    cx.label = Some("Move Control".into());
    Ok(Json::object([("control", Json::Number(id as f64))]))
}

fn rename(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let name = a.text("name").unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err(Error::params("a name cannot be empty"));
    }
    let before = cx.project.clone();
    let comp = cx.project.comp_mut(cid).expect("found");
    let id = find_control(&comp.essentials, a.need("control"))?;
    comp.essentials.find_mut(id).expect("found").name = name.clone();
    essential::sync(&before, cx.project);
    cx.label = Some(format!("Rename Control {name}"));
    Ok(Json::object([("control", Json::Number(id as f64)), ("name", Json::from(name))]))
}

fn remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let before = cx.project.clone();
    let comp = cx.project.comp_mut(cid).expect("found");
    let id = find_control(&comp.essentials, a.need("control"))?;
    let mut gone = vec![id];
    comp.essentials.remove(id);
    // mirrors of what went
    loop {
        let orphans: Vec<u64> = comp.essentials.flat().iter().filter(|c| matches!(c.kind, ControlKind::Mirror { of } if comp.essentials.find(of).is_none())).map(|c| c.id).collect();
        if orphans.is_empty() {
            break;
        }
        for o in orphans {
            comp.essentials.remove(o);
            gone.push(o);
        }
    }
    essential::sync(&before, cx.project);
    cx.label = Some("Remove Control".into());
    Ok(Json::object([("removed", Json::array(gone.iter().map(|i| Json::Number(*i as f64))))]))
}

fn set_name(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let name = a.text("name").unwrap_or("").to_string();
    cx.project.comp_mut(cid).expect("found").essentials.name = name.clone();
    cx.label = Some("Essential Graphics Name".into());
    Ok(Json::object([("name", Json::from(name))]))
}

/// The layer showing a composition with controls, and that composition.
fn showing(project: &Project, cid: u64, lid: u64) -> Result<u64> {
    let l = project.comp(cid).expect("found").layer(lid).expect("found");
    match l.source {
        Source::Comp(inner) if project.comp(inner).is_some_and(|c| c.essentials.has_instance_controls()) => Ok(inner),
        Source::Comp(_) => Err(Error::refused(format!("the composition \"{}\" shows opens nothing to set (essential.add opens a property)", l.name))),
        _ => Err(Error::refused(format!("\"{}\" is a {} layer; essential properties are on layers showing a composition", l.name, l.source.id()))),
    }
}

fn own_mut(l: &mut Layer, id: u64) -> Option<&mut Property> {
    fn go(g: &mut veymelo_project::Group, id: u64) -> Option<&mut Property> {
        for c in &mut g.children {
            match c {
                Node::Prop(p) if p.own.as_ref().is_some_and(|o| o.id == id) => return Some(p),
                Node::Group(s) => {
                    if let Some(p) = go(s, id) {
                        return Some(p);
                    }
                }
                _ => {}
            }
        }
        None
    }
    l.props.group_mut("essentials").ok().and_then(|g| go(g, id))
}

fn set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let inner = showing(cx.project, cid, lid)?;
    let ess = cx.project.comp(inner).expect("found").essentials.clone();
    let id = find_control(&ess, a.need("control"))?;
    let item = match a.json("item") {
        Some(j) => Some(find_footage(cx.project, j)?),
        None => None,
    };
    let l = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let p = own_mut(l, id).ok_or_else(|| Error::refused("that control is not one a layer sets (a comment or a group, or broken)"))?;
    if let Some(item) = item {
        if !std::ptr::eq(p.def, &essential::MEDIA) {
            return Err(Error::params("`item` is for a media replacement"));
        }
        p.value = Value::Number(item as f64);
    }
    if let Some(v) = a.json("value") {
        let v = Value::from_json(p.def.kind, v).map_err(Error::params)?;
        p.set_value(v).map_err(Error::params)?;
    }
    if let Some(k) = a.json("keys") {
        let keys = veymelo_keyframe::keys_from_json(p.def.kind, k).map_err(Error::params)?;
        p.set_keys(keys).map_err(Error::params)?;
    }
    if let Some(e) = a.text("expression") {
        p.expression = (!e.trim().is_empty()).then(|| Expression { text: e.to_string(), enabled: true });
    }
    if let Some(o) = p.own.as_mut() {
        o.overridden = true;
    }
    let out = Json::object([("control", Json::Number(id as f64)), ("value", p.value.to_json(p.def.kind)), ("overridden", Json::from(true))]);
    cx.label = Some("Set Essential Property".into());
    Ok(out)
}

/// The controls named by `control`, else every one the layer set.
fn chosen(project: &Project, cid: u64, lid: u64, a: &Args) -> Result<(u64, Vec<u64>)> {
    let inner = showing(project, cid, lid)?;
    let ess = &project.comp(inner).expect("found").essentials;
    if let Some(j) = a.json("control") {
        return Ok((inner, vec![find_control(ess, j)?]));
    }
    let l = project.comp(cid).expect("found").layer(lid).expect("found");
    let mut out = Vec::new();
    if let Some(g) = essential::group(l) {
        g.walk(&mut |_, p| {
            if let Some(o) = p.own.as_ref().filter(|o| o.overridden) {
                out.push(o.id);
            }
        });
    }
    Ok((inner, out))
}

/// Take the layer's own value for these controls away (its essential properties are made anew,
/// following the composition).
fn forget(l: &mut Layer, ids: &[u64]) {
    fn go(g: &mut veymelo_project::Group, ids: &[u64]) {
        g.children.retain(|c| !matches!(c, Node::Prop(p) if p.own.as_ref().is_some_and(|o| ids.contains(&o.id))));
        for c in &mut g.children {
            if let Node::Group(s) = c {
                go(s, ids);
            }
        }
    }
    if let Ok(g) = l.props.group_mut("essentials") {
        go(g, ids);
    }
}

fn revert(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let (_, ids) = chosen(cx.project, cid, lid, a)?;
    let before = cx.project.clone();
    forget(cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found"), &ids);
    essential::sync(&before, cx.project);
    cx.label = Some("Revert Essential Properties".into());
    Ok(Json::object([("reverted", Json::array(ids.iter().map(|i| Json::Number(*i as f64))))]))
}

fn push(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let (inner, ids) = chosen(cx.project, cid, lid, a)?;
    let before = cx.project.clone();
    let outer = cx.project.comp(cid).expect("found").layer(lid).expect("found").clone();
    let ess = cx.project.comp(inner).expect("found").essentials.clone();
    let mut pushed = Vec::new();
    for id in &ids {
        let mut src = None;
        if let Some(g) = essential::group(&outer) {
            g.walk(&mut |_, p| {
                if p.own.as_ref().is_some_and(|o| o.id == *id && o.overridden) {
                    src = Some(p.clone());
                }
            });
        }
        let Some(src) = src else { continue };
        let Some(c) = ess.resolve(*id) else { continue };
        let comp = cx.project.comp_mut(inner).expect("found");
        match &c.kind {
            ControlKind::Property { show, .. } => {
                for t in ess.targets(c.id) {
                    let Some(l) = comp.layer_mut(t.layer) else { continue };
                    // the outer layer's own time is the composition's time inside it
                    let keys: Vec<_> = src.keys.iter().map(|k| {
                        let mut k = k.clone();
                        k.time = l.layer_time(k.time);
                        k
                    }).collect();
                    let Ok(dst) = l.props.prop_mut(&t.path) else { continue };
                    if *show == Show::Font {
                        if let (Value::Doc(d), Value::Doc(f)) = (&mut dst.value, &src.value) {
                            (d.font, d.weight, d.italic, d.size) = (f.font.clone(), f.weight, f.italic, f.size);
                        }
                        continue;
                    }
                    if dst.def.kind == src.def.kind {
                        (dst.value, dst.keys, dst.expression) = (src.value.clone(), keys, src.expression.clone());
                    }
                }
            }
            ControlKind::Media { layer } => {
                let item = src.value.as_f64().max(0.0) as u64;
                if let Some(l) = comp.layer_mut(*layer) {
                    l.source = Source::Footage(item);
                }
            }
            _ => continue,
        }
        pushed.push(*id);
    }
    forget(cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found"), &pushed);
    essential::sync(&before, cx.project);
    cx.label = Some("Push Override Values to Source".into());
    Ok(Json::object([("pushed", Json::array(pushed.iter().map(|i| Json::Number(*i as f64))))]))
}

fn instance(l: &Look, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(l.project, l.view, a)?;
    showing(l.project, cid, lid)?;
    let layer = l.project.comp(cid).expect("found").layer(lid).expect("found");
    let mut out = Vec::new();
    if let Some(g) = essential::group(layer) {
        g.walk(&mut |path, p| {
            let Some(o) = &p.own else { return };
            let mut j = Map::new();
            j.insert("control", o.id as f64);
            j.insert("name", o.name.as_str());
            j.insert("path", format!("essentials/{}", path.trim_start_matches("essentials/")));
            j.insert("value", p.value.to_json(p.def.kind));
            if p.is_animated() {
                j.insert("animated", true);
            }
            if p.expression.is_some() {
                j.insert("expression", p.expression.as_ref().map_or("", |e| e.text.as_str()));
            }
            j.insert("overridden", o.overridden);
            out.push(Json::Object(j));
        });
    }
    Ok(Json::Array(out))
}

/// A composition and every item it uses (compositions inside it, footage), as a project of
/// their own.
fn subset(project: &Project, cid: u64) -> Project {
    let mut keep: HashSet<u64> = HashSet::new();
    let mut todo = vec![cid];
    while let Some(id) = todo.pop() {
        if !keep.insert(id) {
            continue;
        }
        if let Some(c) = project.comp(id) {
            for l in &c.layers {
                if let Source::Comp(i) | Source::Footage(i) = l.source {
                    todo.push(i);
                }
            }
        }
    }
    let mut out = Project::new();
    out.settings = project.settings.clone();
    for it in &project.items {
        if keep.contains(&it.id) {
            let mut it = it.clone();
            it.folder = None;
            out.items.push(it);
        }
    }
    out.next_id = project.next_id;
    out
}

fn export_template(l: &Look, a: &Args) -> Result<Json> {
    let cid = comp_id(l.project, l.view, a)?;
    let comp = l.project.comp(cid).expect("found");
    let item = l.project.item(cid).expect("found");
    let name = a.text("name").map(str::to_string).unwrap_or_else(|| if comp.essentials.name.is_empty() { item.name.clone() } else { comp.essentials.name.clone() });
    let sub = subset(l.project, cid);
    let project = Json::parse(&sub.to_text()).map_err(|e| Error::refused(format!("the project could not be written: {e}")))?;
    let doc = Json::object([("format", Json::from(TEMPLATE)), ("version", Json::Number(1.0)), ("name", Json::from(name.as_str())), ("comp", Json::Number(cid as f64)), ("project", project)]);
    let path = PathBuf::from(a.text("path").unwrap_or(""));
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| Error::refused(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(&path, doc.to_pretty() + "\n").map_err(|e| Error::refused(format!("{}: {e}", path.display())))?;
    Ok(Json::object([("path", Json::from(path.display().to_string())), ("name", Json::from(name)), ("items", Json::Number(sub.items.len() as f64))]))
}

/// A template file: its document, name, composition id and project.
fn read_template(path: &str, catalog: &veymelo_project::Catalog) -> Result<(String, u64, Project)> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::not_found(format!("{path}: {e}")))?;
    let j = Json::parse(&text).map_err(|e| Error::params(format!("{path}: not JSON: {e}")))?;
    if j.get("format").and_then(Json::as_str) != Some(TEMPLATE) {
        return Err(Error::params(format!("{path} is not a Veymelo template")));
    }
    let comp = j.get("comp").and_then(Json::as_u64).ok_or_else(|| Error::params(format!("{path}: the template names no composition")))?;
    let project = Project::from_json(j.get("project").unwrap_or(&Json::Null), catalog).map_err(|e| Error::params(format!("{path}: {e}")))?;
    if project.comp(comp).is_none() {
        return Err(Error::params(format!("{path}: its composition is not in it")));
    }
    Ok((j.get("name").and_then(Json::as_str).unwrap_or("").to_string(), comp, project))
}

fn import_template(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let path = a.text("path").unwrap_or("").to_string();
    let (name, comp, other) = read_template(&path, cx.catalog)?;
    let place = other.items.iter().position(|i| i.id == comp).expect("in it");
    let folder_name = if name.is_empty() { "Template".to_string() } else { name.clone() };
    let folder = cx.project.add(&folder_name, None, ItemKind::Folder);
    let before = cx.project.clone();
    let items = cx.project.merge(other, Some(folder));
    let new_comp = items[place];
    essential::sync(&before, cx.project);
    let mut out = Map::new();
    out.insert("comp", new_comp as f64);
    out.insert("folder", folder as f64);
    out.insert("items", Json::array(items.iter().map(|i| Json::Number(*i as f64))));
    if let Some(target) = a.json("addToComp") {
        let into = super::find_comp(cx.project, target)?;
        let made = super::call(cx, "layer.add", Json::object([("comp", Json::Number(into as f64)), ("type", Json::from("comp")), ("item", Json::Number(new_comp as f64))]))?;
        if let Some(l) = made.get("layer") {
            out.insert("layer", l.clone());
        }
    }
    cx.label = Some(format!("Import Template {folder_name}"));
    Ok(Json::Object(out))
}

fn template_info(l: &Look, a: &Args) -> Result<Json> {
    let path = a.text("path").unwrap_or("").to_string();
    let (name, comp, project) = read_template(&path, l.catalog)?;
    let c = project.comp(comp).expect("checked");
    Ok(Json::object([
        ("name", Json::from(name)),
        ("comp", Json::from(project.item(comp).map_or("", |i| i.name.as_str()))),
        ("width", Json::Number(c.width as f64)),
        ("height", Json::Number(c.height as f64)),
        ("duration", Json::Number(c.duration.seconds())),
        ("controls", Json::array(c.essentials.controls.iter().map(|k| control_json(c, k)))),
    ]))
}
