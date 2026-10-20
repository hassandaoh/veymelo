//! Commands about layer styles: giving a layer a style, taking it away, and the styles there
//! are. A style's settings are properties under layerStyles/<style>/ (prop.set and the rest
//! change them, keys and expressions included); the light the styles share is
//! layerStyles/blendingOptions/globalLightAngle and globalLightAltitude, the same for every
//! layer of the composition.

use veymelo_json::{Map, Value as Json};
use veymelo_project::styles::{self, BLENDING, BLENDING_OPTIONS, LIST};
use veymelo_project::{Group, Layer, Node};

use super::shape::start_with;
use super::{COMP, Command, LAYER, Run, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "style.add",
            label: "Add Layer Style",
            summary: "Give a layer a style: dropShadow, innerShadow, outerGlow, innerGlow, bevelEmboss, satin, colorOverlay, gradientOverlay or stroke (style.types lists their settings). Styles are drawn round and over the layer after its masks and effects, in that order whatever order they were added in; the layer also gets Blending Options (the composition's light, fill opacity, channels, knockout). A style it has already is switched on and given the settings.",
            params: const { &[COMP, LAYER, Param::need("type", Ty::Text, "the style, like \"dropShadow\" or \"Bevel and Emboss\""), Param::opt("props", Ty::Object, "its settings to start with, by path: {\"distance\": 12, \"size\": 20}")] },
            returns: "{path, id}",
            run: Run::Edit(style_add),
        },
        Command {
            id: "style.remove",
            label: "Remove Layer Style",
            summary: "Take a style off a layer; with `all`, every style and the Blending Options.",
            params: const { &[COMP, LAYER, Param::opt("style", Ty::Text, "the style, like \"dropShadow\""), Param::opt("all", Ty::Bool, "take them all away")] },
            returns: "{removed}",
            run: Run::Edit(style_remove),
        },
        Command {
            id: "style.types",
            label: "Layer Style Types",
            summary: "The layer styles there are, in the order they are drawn, with what each does and its settings (and those of Blending Options).",
            params: &[],
            returns: "[{type, name, about, settings}]",
            run: Run::Look(style_types),
        },
    ]
}

/// The layer's styles list.
fn list_of(layer: &mut Layer) -> Result<&mut Group> {
    let name = layer.name.clone();
    if layer.switches.adjustment {
        return Err(Error::refused(format!("\"{name}\" is an adjustment layer; it takes no styles")));
    }
    layer.props.group_mut(LIST).map_err(|_| Error::refused(format!("\"{name}\" has no pixels of its own, so it takes no styles")))
}

/// Put `g` into the list in its place (Blending Options first, then the styles in order).
fn put_in_order(list: &mut Group, g: Group) {
    let rank = styles::rank(g.def.id);
    let at = list.children.iter().position(|n| n.as_group().is_some_and(|o| styles::rank(o.def.id) > rank)).unwrap_or(list.children.len());
    list.children.insert(at, Node::Group(g));
}

fn style_add(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let ty = a.text("type").unwrap_or("");
    let def = styles::find(ty).ok_or_else(|| {
        let ids: Vec<&str> = styles::STYLES.iter().map(|d| d.id).collect();
        Error::params(match crate::params::nearest(ty, ids.iter().copied()) {
            Some(n) => format!("there is no layer style \"{ty}\"; did you mean \"{n}\"?"),
            None => format!("there is no layer style \"{ty}\" (there are: {})", ids.join(", ")),
        })
    })?;
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    // the composition's light, for a layer getting Blending Options now
    let light = styles::comp_light(cx.project.comp(cid).expect("found"));
    let (bo_id, style_id) = (cx.project.alloc(), cx.project.alloc());
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let list = list_of(layer)?;
    let mut new_options = false;
    if !list.children.iter().any(|n| n.as_group().is_some_and(|g| g.def.id == BLENDING)) {
        put_in_order(list, Group::instance(&BLENDING_OPTIONS, bo_id, BLENDING_OPTIONS.name));
        new_options = true;
    }
    let id = match list.children.iter_mut().filter_map(|n| n.as_group_mut()).find(|g| std::ptr::eq(g.def, def)) {
        Some(g) => {
            g.enabled = true;
            start_with(g, a.json("props"))?;
            g.id
        }
        None => {
            let mut g = Group::instance(def, style_id, def.name);
            start_with(&mut g, a.json("props"))?;
            put_in_order(list, g);
            style_id
        }
    };
    if new_options && let Some(light) = light {
        styles::take_light(layer, &light);
    }
    cx.label = Some(format!("Add {}", def.name));
    Ok(Json::object([("path", Json::from(format!("{LIST}/{}", def.id))), ("id", Json::Number(id as f64))]))
}

fn style_remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let layer = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
    let list = list_of(layer)?;
    if a.bool("all").unwrap_or(false) {
        let n = list.children.len();
        list.children.clear();
        cx.label = Some("Remove All Layer Styles".into());
        return Ok(Json::object([("removed", n as f64)]));
    }
    let which = a.text("style").ok_or_else(|| Error::params("say which `style` to take away, or `all`"))?;
    let def = styles::find(which.trim_start_matches("layerStyles/")).ok_or_else(|| Error::params(format!("there is no layer style \"{which}\"")))?;
    let at = list.children.iter().position(|n| n.as_group().is_some_and(|g| std::ptr::eq(g.def, def))).ok_or_else(|| Error::not_found(format!("the layer has no {}", def.name)))?;
    let removed = list.children.remove(at).as_group().map_or(0, |g| g.id);
    cx.label = Some(format!("Remove {}", def.name));
    Ok(Json::object([("removed", removed as f64)]))
}

fn style_types(_: &Look, _: &Args) -> Result<Json> {
    let mut out = Vec::new();
    for def in std::iter::once(&BLENDING_OPTIONS).chain(styles::STYLES.iter().copied()) {
        let mut settings = Vec::new();
        Group::new(def).walk(&mut |path, p| settings.push(Json::from(format!("{path}: {}", p.def.summary()))));
        let mut o = Map::new();
        o.insert("type", def.id);
        o.insert("name", def.name);
        o.insert("about", def.doc);
        o.insert("settings", Json::Array(settings));
        out.push(Json::Object(o));
    }
    Ok(Json::Array(out))
}
