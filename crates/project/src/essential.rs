//! Essential properties: what a composition opens to the layers that show it, and what each of
//! those layers sets for itself.
//!
//! A composition's [`Essentials`] list controls: a property of one of its layers (other
//! properties linked to follow it; a Source Text shown only as its font, a scale as one
//! number), a mirror of another control, a footage layer whose footage can be swapped (media
//! replacement), a comment, a group of controls. Every layer showing the composition gets an
//! `essentials` group with a property a control, going by the control's id and name: what is
//! set there is that layer's own (overridden), the rest follows the composition. [`sync`] keeps
//! the groups in step after every change; [`instance`] is the composition as one layer shows it.

use std::sync::Arc;

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Kind, Value};
use veymelo_time::Tick;

use crate::defs::{Body, GroupDef, PropDef};
use crate::props::{ESSENTIALS_ID, Expression, Group, Node, Own, Property};
use crate::read::{LoadError, Obj, fail};
use crate::{Comp, ItemKind, Layer, Project, Source};

/// A composition layer's essential properties.
pub static ESSENTIALS: GroupDef = GroupDef { id: ESSENTIALS_ID, name: "Essential Properties", body: Body::List("essentials"), doc: "what the composition it shows lets this layer set for itself" };
/// A group of them, as the composition groups its controls.
pub static ESSENTIAL_GROUP: GroupDef = GroupDef { id: "essentialGroup", name: "Group", body: Body::List("essentials"), doc: "controls kept together" };
/// The footage a media replacement shows (its item's id).
pub static MEDIA: PropDef = PropDef::new("media", "Media", Kind::Number, Value::Number(0.0)).setting().doc("the footage item it shows, by id");
/// What a file wrote for one, until what it stands for is known.
static PENDING: PropDef = PropDef::new("pending", "Pending", Kind::Number, Value::Number(0.0));

/// What a composition opens to the layers that show it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Essentials {
    /// What it is called as a template.
    pub name: String,
    pub controls: Vec<Control>,
}

/// One of a composition's controls.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub id: u64,
    pub name: String,
    pub kind: ControlKind,
}

/// A property of a layer: the layer's id and the property's path (instances by id).
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub layer: u64,
    pub path: String,
}

/// How a property is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Show {
    #[default]
    Whole,
    /// A Source Text's font, style and size only (its words stay the composition's).
    Font,
    /// A scale as one number, every axis the same.
    Scale,
}

impl Show {
    pub const IDS: &[&str] = &["whole", "font", "scale"];
    pub fn id(self) -> &'static str {
        Self::IDS[self as usize]
    }
    pub fn from_id(s: &str) -> Option<Show> {
        Some(match s {
            "whole" => Show::Whole,
            "font" => Show::Font,
            "scale" => Show::Scale,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlKind {
    /// A property of one of its layers, and others that follow it.
    Property { target: Target, links: Vec<Target>, show: Show },
    /// Control `of` again, under its own name: one value.
    Mirror { of: u64 },
    /// A footage layer whose footage each showing layer can swap.
    Media { layer: u64 },
    Comment { text: String },
    Group { children: Vec<Control> },
}

impl ControlKind {
    pub fn id(&self) -> &'static str {
        match self {
            ControlKind::Property { .. } => "property",
            ControlKind::Mirror { .. } => "mirror",
            ControlKind::Media { .. } => "media",
            ControlKind::Comment { .. } => "comment",
            ControlKind::Group { .. } => "group",
        }
    }
}

impl Essentials {
    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && self.controls.is_empty()
    }

    /// Every control, groups before what is in them.
    pub fn flat(&self) -> Vec<&Control> {
        fn go<'a>(v: &'a [Control], out: &mut Vec<&'a Control>) {
            for c in v {
                out.push(c);
                if let ControlKind::Group { children } = &c.kind {
                    go(children, out);
                }
            }
        }
        let mut out = Vec::new();
        go(&self.controls, &mut out);
        out
    }

    pub fn find(&self, id: u64) -> Option<&Control> {
        self.flat().into_iter().find(|c| c.id == id)
    }

    pub fn find_mut(&mut self, id: u64) -> Option<&mut Control> {
        fn go(v: &mut [Control], id: u64) -> Option<&mut Control> {
            for c in v {
                if c.id == id {
                    return Some(c);
                }
                if let ControlKind::Group { children } = &mut c.kind
                    && let Some(x) = go(children, id)
                {
                    return Some(x);
                }
            }
            None
        }
        go(&mut self.controls, id)
    }

    /// Take a control out (from wherever it is).
    pub fn remove(&mut self, id: u64) -> Option<Control> {
        fn go(v: &mut Vec<Control>, id: u64) -> Option<Control> {
            if let Some(i) = v.iter().position(|c| c.id == id) {
                return Some(v.remove(i));
            }
            v.iter_mut().find_map(|c| if let ControlKind::Group { children } = &mut c.kind { go(children, id) } else { None })
        }
        go(&mut self.controls, id)
    }

    /// The list a control goes in: the top, or a group's.
    pub fn list_mut(&mut self, group: Option<u64>) -> Option<&mut Vec<Control>> {
        match group {
            None => Some(&mut self.controls),
            Some(g) => match &mut self.find_mut(g)?.kind {
                ControlKind::Group { children } => Some(children),
                _ => None,
            },
        }
    }

    /// The control a mirror stands for (the control itself when it is not one).
    pub fn resolve(&self, id: u64) -> Option<&Control> {
        let mut c = self.find(id)?;
        for _ in 0..16 {
            match c.kind {
                ControlKind::Mirror { of } => c = self.find(of)?,
                _ => return Some(c),
            }
        }
        None
    }

    /// Whether a layer showing it has anything to set.
    pub fn has_instance_controls(&self) -> bool {
        self.flat().iter().any(|c| matches!(c.kind, ControlKind::Property { .. } | ControlKind::Mirror { .. } | ControlKind::Media { .. }))
    }

    /// Every property a control drives (a mirror: its control's).
    pub fn targets(&self, id: u64) -> Vec<&Target> {
        match self.resolve(id).map(|c| &c.kind) {
            Some(ControlKind::Property { target, links, .. }) => std::iter::once(target).chain(links).collect(),
            _ => Vec::new(),
        }
    }

    /// The control of property `path` of layer `layer`, as it is shown.
    pub fn control_for(&self, layer: u64, path: &str, show: Show) -> Option<&Control> {
        self.flat().into_iter().find(|c| matches!(&c.kind, ControlKind::Property { target, show: s, .. } if target.layer == layer && target.path == path && *s == show))
    }

    /// Change every id in it (when it comes into another project).
    pub fn remap(&mut self, ids: &mut dyn FnMut(u64) -> u64) {
        fn go(v: &mut [Control], ids: &mut dyn FnMut(u64) -> u64) {
            for c in v {
                c.id = ids(c.id);
                match &mut c.kind {
                    ControlKind::Property { target, links, .. } => {
                        target.layer = ids(target.layer);
                        target.path = remap_path(&target.path, ids);
                        for k in links {
                            k.layer = ids(k.layer);
                            k.path = remap_path(&k.path, ids);
                        }
                    }
                    ControlKind::Mirror { of } => *of = ids(*of),
                    ControlKind::Media { layer } => *layer = ids(*layer),
                    ControlKind::Group { children } => go(children, ids),
                    ControlKind::Comment { .. } => {}
                }
            }
        }
        go(&mut self.controls, ids);
    }

    pub(crate) fn to_json(&self) -> Json {
        fn control(c: &Control) -> Json {
            let mut o = Map::new();
            o.insert("id", c.id as f64);
            o.insert("type", c.kind.id());
            if !c.name.is_empty() {
                o.insert("name", c.name.as_str());
            }
            let target = |t: &Target| Json::object([("layer", Json::Number(t.layer as f64)), ("path", Json::from(t.path.as_str()))]);
            match &c.kind {
                ControlKind::Property { target: t, links, show } => {
                    o.insert("layer", t.layer as f64);
                    o.insert("path", t.path.as_str());
                    if *show != Show::Whole {
                        o.insert("as", show.id());
                    }
                    if !links.is_empty() {
                        o.insert("links", Json::array(links.iter().map(target)));
                    }
                }
                ControlKind::Mirror { of } => {
                    o.insert("of", *of as f64);
                }
                ControlKind::Media { layer } => {
                    o.insert("layer", *layer as f64);
                }
                ControlKind::Comment { text } => {
                    o.insert("text", text.as_str());
                }
                ControlKind::Group { children } => {
                    o.insert("children", Json::array(children.iter().map(control)));
                }
            }
            Json::Object(o)
        }
        let mut o = Map::new();
        if !self.name.is_empty() {
            o.insert("name", self.name.as_str());
        }
        o.insert("controls", Json::array(self.controls.iter().map(control)));
        Json::Object(o)
    }

    pub(crate) fn from_json(j: &Json, at: &str) -> Result<Essentials, LoadError> {
        fn controls(list: &[Json], at: &str) -> Result<Vec<Control>, LoadError> {
            let mut out = Vec::new();
            for (i, j) in list.iter().enumerate() {
                let here = format!("{at}[{i}]");
                let o = Obj::new(j, &here, "a control")?;
                o.only(&["id", "type", "name", "layer", "path", "as", "links", "of", "text", "children"])?;
                let ty = o.need_text("type")?;
                let target = |o: &Obj| -> Result<Target, LoadError> {
                    let Some(layer) = o.id("layer")? else { return fail(o.at, "it needs \"layer\", the layer's id") };
                    Ok(Target { layer, path: o.need_text("path")?.to_string() })
                };
                let kind = match ty {
                    "property" => {
                        let mut links = Vec::new();
                        for (k, l) in o.list("links")?.iter().enumerate() {
                            let la = format!("{}[{k}]", o.at("links"));
                            let lo = Obj::new(l, &la, "a link")?;
                            lo.only(&["layer", "path"])?;
                            links.push(target(&lo)?);
                        }
                        ControlKind::Property { target: target(&o)?, links, show: o.choice("as", Show::IDS, Show::from_id)?.unwrap_or_default() }
                    }
                    "mirror" => {
                        let Some(of) = o.id("of")? else { return fail(&here, "a mirror needs \"of\", the control it mirrors") };
                        ControlKind::Mirror { of }
                    }
                    "media" => {
                        let Some(layer) = o.id("layer")? else { return fail(&here, "a media replacement needs \"layer\"") };
                        ControlKind::Media { layer }
                    }
                    "comment" => ControlKind::Comment { text: o.text("text")?.unwrap_or("").to_string() },
                    "group" => ControlKind::Group { children: controls(o.list("children")?, &o.at("children"))? },
                    _ => return fail(&o.at("type"), format!("there is no control type \"{ty}\" (there is property, mirror, media, comment, group)")),
                };
                out.push(Control { id: o.id("id")?.unwrap_or(0), name: o.text("name")?.unwrap_or("").to_string(), kind });
            }
            Ok(out)
        }
        let o = Obj::new(j, at, "essential properties")?;
        o.only(&["name", "controls"])?;
        Ok(Essentials { name: o.text("name")?.unwrap_or("").to_string(), controls: controls(o.list("controls")?, &o.at("controls"))? })
    }

    /// Give every control without an id one.
    pub(crate) fn give_ids(&mut self, give: &mut dyn FnMut(&mut u64)) {
        fn go(v: &mut [Control], give: &mut dyn FnMut(&mut u64)) {
            for c in v {
                give(&mut c.id);
                if let ControlKind::Group { children } = &mut c.kind {
                    go(children, give);
                }
            }
        }
        go(&mut self.controls, give);
    }
}

/// A path with its instance ids (`effects/@12/color`) changed.
fn remap_path(path: &str, ids: &mut dyn FnMut(u64) -> u64) -> String {
    path.split('/').map(|seg| match seg.strip_prefix('@').and_then(|n| n.parse::<u64>().ok()) {
        Some(n) => format!("@{}", ids(n)),
        None => seg.to_string(),
    }).collect::<Vec<_>>().join("/")
}

/// The property a control stands for, in its composition (a mirror: its control's).
pub fn source(comp: &Comp, id: u64) -> Option<(&Layer, &Property)> {
    let ControlKind::Property { target, .. } = &comp.essentials.resolve(id)?.kind else { return None };
    let l = comp.layer(target.layer)?;
    Some((l, l.props.prop(&target.path).ok()?))
}

/// Whether a property can be a control: something to set, not a path, gradient or layer.
pub fn can_open(p: &Property) -> bool {
    p.def.animatable && !matches!(p.def.kind, Kind::Path | Kind::Gradient | Kind::Layer)
}

/// What a layer's essential properties write: only what it set for itself, by control id.
pub(crate) fn overrides_json(g: &Group) -> Option<Json> {
    fn go(g: &Group, o: &mut Map) {
        for c in &g.children {
            match c {
                Node::Group(sub) => go(sub, o),
                Node::Prop(p) => {
                    let Some(own) = p.own.as_ref().filter(|w| w.overridden) else { continue };
                    if let Some(j) = &own.pending {
                        o.insert(own.id.to_string(), j.clone());
                        continue;
                    }
                    let mut v = Map::new();
                    if p.keys.is_empty() {
                        v.insert("value", p.value.to_json(p.def.kind));
                    } else {
                        v.insert("keys", veymelo_keyframe::keys_to_json(&p.keys, p.def.kind));
                    }
                    if let Some(e) = &p.expression {
                        v.insert("expression", e.text.as_str());
                        if !e.enabled {
                            v.insert("expressionEnabled", false);
                        }
                    }
                    o.insert(own.id.to_string(), Json::Object(v));
                }
            }
        }
    }
    let mut o = Map::new();
    go(g, &mut o);
    (!o.is_empty()).then_some(Json::Object(o))
}

/// A layer's essential properties as a file writes them, kept until the project is read whole.
pub(crate) fn pending(j: &Json, at: &str) -> Result<Group, LoadError> {
    let o = Obj::new(j, at, "essential properties")?;
    let mut g = Group::new(&ESSENTIALS);
    for (key, v) in o.map.iter() {
        let Ok(id) = key.parse::<u64>() else { return fail(at, format!("essential properties go by their control's id, not \"{key}\"")) };
        let mut p = Property::new(&PENDING);
        p.own = Some(Box::new(Own { id, name: String::new(), overridden: true, pending: Some(v.clone()) }));
        g.children.push(Node::Prop(p));
    }
    Ok(g)
}

/// Every property below a group, wherever it is.
fn props_of(g: &Group) -> Vec<&Property> {
    let mut out = Vec::new();
    fn go<'a>(g: &'a Group, out: &mut Vec<&'a Property>) {
        for c in &g.children {
            match c {
                Node::Prop(p) => out.push(p),
                Node::Group(s) => go(s, out),
            }
        }
    }
    go(g, &mut out);
    out
}

fn own_prop(g: &Group, id: u64) -> Option<&Property> {
    props_of(g).into_iter().find(|p| p.own.as_ref().is_some_and(|o| o.id == id))
}

fn own_prop_mut(g: &mut Group, id: u64) -> Option<&mut Property> {
    for c in &mut g.children {
        match c {
            Node::Prop(p) if p.own.as_ref().is_some_and(|o| o.id == id) => return Some(p),
            Node::Group(s) => {
                if let Some(p) = own_prop_mut(s, id) {
                    return Some(p);
                }
            }
            _ => {}
        }
    }
    None
}

fn essentials_of(l: &Layer) -> Option<&Group> {
    l.props.children.iter().find_map(|c| c.as_group().filter(|g| g.def.id == ESSENTIALS_ID))
}

fn essentials_mut(l: &mut Layer) -> Option<&mut Group> {
    l.props.children.iter_mut().find_map(|c| c.as_group_mut().filter(|g| g.def.id == ESSENTIALS_ID))
}

/// A layer's essential properties (none when what it shows opens nothing).
pub fn group(l: &Layer) -> Option<&Group> {
    essentials_of(l)
}

fn same(a: &Property, b: &Property) -> bool {
    a.value == b.value && a.keys == b.keys && a.expression == b.expression
}

/// After a change: linked properties follow, a mirror set on a layer sets its control, what a
/// layer set becomes its own, and every composition layer's essential properties are made to
/// match what its composition opens.
pub fn sync(before: &Project, after: &mut Project) {
    follow_links(before, after);
    mirror_edits(before, after);
    mark_overrides(before, after);
    rebuild(after);
}

/// The ids of the compositions in a project.
fn comp_ids(p: &Project) -> Vec<u64> {
    p.items.iter().filter(|i| i.as_comp().is_some()).map(|i| i.id).collect()
}

/// A control's properties, in its composition: the one that changed leads, the rest take its value.
fn follow_links(before: &Project, after: &mut Project) {
    for cid in comp_ids(after) {
        let comp = after.comp(cid).expect("a comp");
        let old = before.comp(cid);
        let mut writes: Vec<(Target, Property)> = Vec::new();
        for c in comp.essentials.flat() {
            let ControlKind::Property { target, links, show } = &c.kind else { continue };
            if links.is_empty() || *show == Show::Font {
                continue;
            }
            let all: Vec<&Target> = std::iter::once(target).chain(links).collect();
            let get = |cm: &Comp, t: &Target| cm.layer(t.layer).and_then(|l| l.props.prop(&t.path).ok()).cloned();
            let changed = |t: &Target| match (get(comp, t), old.and_then(|o| get(o, t))) {
                (Some(a), Some(b)) => !same(&a, &b),
                _ => false,
            };
            let lead = if changed(all[0]) { all[0] } else { all.iter().copied().find(|t| changed(t)).unwrap_or(all[0]) };
            let (Some(from_l), Some(src)) = (comp.layer(lead.layer), get(comp, lead)) else { continue };
            for t in all.into_iter().filter(|t| *t != lead) {
                let (Some(l), Some(dst)) = (comp.layer(t.layer), get(comp, t)) else { continue };
                if dst.def.kind != src.def.kind {
                    continue;
                }
                let mut p = dst.clone();
                p.value = src.value.clone();
                p.keys = src.keys.iter().map(|k| {
                    let mut k = k.clone();
                    k.time = l.layer_time(from_l.comp_time(k.time));
                    k
                }).collect();
                if !same(&p, &dst) {
                    writes.push((t.clone(), p));
                }
            }
        }
        if writes.is_empty() {
            continue;
        }
        let comp = after.comp_mut(cid).expect("a comp");
        for (t, p) in writes {
            if let Some(dst) = comp.layer_mut(t.layer).and_then(|l| l.props.prop_mut(&t.path).ok()) {
                (dst.value, dst.keys) = (p.value, p.keys);
            }
        }
    }
}

/// A mirror set on a layer sets the control it mirrors there too.
fn mirror_edits(before: &Project, after: &mut Project) {
    for cid in comp_ids(after) {
        let (Some(comp), Some(old_comp)) = (after.comp(cid), before.comp(cid)) else { continue };
        let mut writes: Vec<(usize, u64, Property)> = Vec::new();
        for (i, l) in comp.layers.iter().enumerate() {
            let Source::Comp(inner) = l.source else { continue };
            let Some(ess) = after.comp(inner).map(|c| &c.essentials) else { continue };
            let (Some(g), Some(og)) = (essentials_of(l), old_comp.layer(l.id).and_then(essentials_of)) else { continue };
            for c in ess.flat() {
                if !matches!(c.kind, ControlKind::Mirror { .. }) {
                    continue;
                }
                let Some(master) = ess.resolve(c.id) else { continue };
                let (Some(mp), Some(cp), Some(omp), Some(ocp)) = (own_prop(g, master.id), own_prop(g, c.id), own_prop(og, master.id), own_prop(og, c.id)) else { continue };
                if !same(cp, ocp) && same(mp, omp) {
                    writes.push((i, master.id, cp.clone()));
                }
            }
        }
        if writes.is_empty() {
            continue;
        }
        let comp = after.comp_mut(cid).expect("a comp");
        for (i, id, src) in writes {
            if let Some(dst) = essentials_mut(&mut comp.layers[i]).and_then(|g| own_prop_mut(g, id)) {
                (dst.value, dst.keys, dst.expression) = (src.value, src.keys, src.expression);
                if let Some(o) = dst.own.as_mut() {
                    o.overridden = true;
                }
            }
        }
    }
}

/// What changed on a layer's essential properties becomes that layer's own.
fn mark_overrides(before: &Project, after: &mut Project) {
    for cid in comp_ids(after) {
        let (Some(comp), Some(old_comp)) = (after.comp(cid), before.comp(cid)) else { continue };
        let mut marks: Vec<(usize, u64)> = Vec::new();
        for (i, l) in comp.layers.iter().enumerate() {
            let (Some(g), Some(og)) = (essentials_of(l), old_comp.layer(l.id).and_then(essentials_of)) else { continue };
            for p in props_of(g) {
                let Some(own) = p.own.as_ref().filter(|o| !o.overridden) else { continue };
                if let Some(op) = own_prop(og, own.id)
                    && !same(p, op)
                {
                    marks.push((i, own.id));
                }
            }
        }
        if marks.is_empty() {
            continue;
        }
        let comp = after.comp_mut(cid).expect("a comp");
        for (i, id) in marks {
            if let Some(o) = essentials_mut(&mut comp.layers[i]).and_then(|g| own_prop_mut(g, id)).and_then(|p| p.own.as_mut()) {
                o.overridden = true;
            }
        }
    }
}

/// What a control shows when a layer has not set it: its property's value at the start.
fn shown(src: &Property, show: Show) -> Value {
    let v = src.value_at(Tick::ZERO);
    match (show, &v) {
        (Show::Scale, Value::Vec3(s)) => Value::Vec3([s[0], s[0], s[2]]),
        _ => v,
    }
}

/// The essential properties a layer showing `comp` has, keeping what it set (`old`).
fn build(comp: &Comp, old: Option<&Group>) -> Group {
    fn go(comp: &Comp, list: &[Control], old: Option<&Group>, out: &mut Vec<Node>) {
        for c in list {
            let prev = old.and_then(|g| own_prop(g, c.id));
            let mut made = match &c.kind {
                ControlKind::Comment { .. } => continue,
                ControlKind::Group { children } => {
                    let mut g = Group::instance(&ESSENTIAL_GROUP, c.id, &c.name);
                    go(comp, children, old, &mut g.children);
                    out.push(Node::Group(g));
                    continue;
                }
                ControlKind::Property { .. } | ControlKind::Mirror { .. } => {
                    let Some((_, src)) = source(comp, c.id) else { continue };
                    let show = match comp.essentials.resolve(c.id).map(|r| &r.kind) {
                        Some(ControlKind::Property { show, .. }) => *show,
                        _ => Show::Whole,
                    };
                    let mut p = Property::new(src.def);
                    p.value = shown(src, show);
                    p
                }
                ControlKind::Media { layer } => {
                    let Some(&Source::Footage(item)) = comp.layer(*layer).map(|l| &l.source) else { continue };
                    let mut p = Property::new(&MEDIA);
                    p.value = Value::Number(item as f64);
                    p
                }
            };
            let mut own = Own { id: c.id, name: c.name.clone(), overridden: false, pending: None };
            if let Some(prev) = prev.filter(|p| p.own.as_ref().is_some_and(|o| o.overridden)) {
                match prev.own.as_ref().and_then(|o| o.pending.as_ref()) {
                    Some(j) => {
                        if let Ok(read) = Property::from_json(made.def, j, "") {
                            (made.value, made.keys, made.expression) = (read.value, read.keys, read.expression);
                            own.overridden = true;
                        }
                    }
                    None if std::ptr::eq(prev.def, made.def) => {
                        (made.value, made.keys, made.expression) = (prev.value.clone(), prev.keys.clone(), prev.expression.clone());
                        own.overridden = true;
                    }
                    None => {}
                }
            }
            made.own = Some(Box::new(own));
            out.push(Node::Prop(made));
        }
    }
    let mut g = Group::new(&ESSENTIALS);
    go(comp, &comp.essentials.controls, old, &mut g.children);
    // a mirror shows what its control has
    let mirrors: Vec<(u64, u64)> = comp.essentials.flat().iter().filter(|c| matches!(c.kind, ControlKind::Mirror { .. })).filter_map(|c| comp.essentials.resolve(c.id).map(|m| (c.id, m.id))).collect();
    for (mirror, master) in mirrors {
        let Some(src) = own_prop(&g, master).cloned() else { continue };
        if let Some(dst) = own_prop_mut(&mut g, mirror) {
            (dst.value, dst.keys, dst.expression) = (src.value, src.keys, src.expression);
            if let (Some(d), Some(s)) = (dst.own.as_mut(), src.own.as_ref()) {
                d.overridden = s.overridden;
            }
        }
    }
    g
}

/// Every composition layer's essential properties made to match what its composition opens.
fn rebuild(p: &mut Project) {
    for cid in comp_ids(p) {
        let comp = p.comp(cid).expect("a comp");
        let mut updates: Vec<(usize, Option<Group>)> = Vec::new();
        for (i, l) in comp.layers.iter().enumerate() {
            let want = match l.source {
                Source::Comp(inner) => p.comp(inner).filter(|c| c.essentials.has_instance_controls()),
                _ => None,
            };
            let old = essentials_of(l);
            match want {
                Some(inner) => {
                    let g = build(inner, old);
                    if old != Some(&g) {
                        updates.push((i, Some(g)));
                    }
                }
                None if old.is_some() => updates.push((i, None)),
                None => {}
            }
        }
        if updates.is_empty() {
            continue;
        }
        let comp = p.comp_mut(cid).expect("a comp");
        for (i, g) in updates {
            let l = &mut comp.layers[i];
            let at = l.props.children.iter().position(|c| c.as_group().is_some_and(|g| g.def.id == ESSENTIALS_ID));
            match (at, g) {
                (Some(k), Some(g)) => l.props.children[k] = Node::Group(g),
                (None, Some(g)) => l.props.children.insert(0, Node::Group(g)),
                (Some(k), None) => {
                    l.props.children.remove(k);
                }
                (None, None) => {}
            }
        }
    }
}

/// Put a font's family, weight, slant and size from `from` onto the document `to` (its words
/// and the rest stay).
fn merge_font(to: &mut Value, from: &Value) {
    if let (Value::Doc(t), Value::Doc(f)) = (to, from) {
        t.font = f.font.clone();
        t.weight = f.weight;
        t.italic = f.italic;
        t.size = f.size;
    }
}

/// The composition `comp_id` as layer `outer` shows it: what the layer set for itself put in
/// place, `read` giving each of its essential properties' values (at the moment drawn). None
/// when it sets nothing.
pub fn instance(project: &Project, comp_id: u64, outer: &Layer, read: &dyn Fn(&Property) -> Value) -> Option<Arc<Comp>> {
    let comp = project.comp(comp_id)?;
    let g = essentials_of(outer)?;
    let set: Vec<&Property> = props_of(g).into_iter().filter(|p| p.own.as_ref().is_some_and(|o| o.overridden && o.pending.is_none())).collect();
    if set.is_empty() {
        return None;
    }
    let mut copy = comp.clone();
    for p in set {
        let id = p.own.as_ref().expect("own").id;
        let Some(c) = comp.essentials.resolve(id) else { continue };
        let value = read(p);
        match &c.kind {
            ControlKind::Property { show, .. } => {
                for t in comp.essentials.targets(c.id) {
                    let Some(dst) = copy.layer_mut(t.layer).and_then(|l| l.props.prop_mut(&t.path).ok()) else { continue };
                    match show {
                        Show::Font => {
                            merge_font(&mut dst.value, &value);
                            dst.keys.iter_mut().for_each(|k| merge_font(&mut k.value, &value));
                        }
                        _ => {
                            let v = match (show, &value) {
                                (Show::Scale, Value::Vec3(s)) => Value::Vec3([s[0], s[0], s[2]]),
                                _ => value.clone(),
                            };
                            if dst.def.check(&v).is_ok() {
                                dst.value = v;
                                dst.keys.clear();
                                dst.expression = None::<Expression>;
                            }
                        }
                    }
                }
            }
            ControlKind::Media { layer } => {
                let item = value.as_f64().max(0.0) as u64;
                if matches!(project.item(item).map(|i| &i.kind), Some(ItemKind::Footage(_)))
                    && let Some(l) = copy.layer_mut(*layer)
                {
                    l.source = Source::Footage(item);
                }
            }
            _ => {}
        }
    }
    Some(Arc::new(copy))
}

/// A layer's essential properties given the new ids of the controls they stand for.
pub(crate) fn remap_instance(l: &mut Layer, ids: &mut dyn FnMut(u64) -> u64) {
    fn go(g: &mut Group, ids: &mut dyn FnMut(u64) -> u64) {
        for c in &mut g.children {
            match c {
                Node::Group(s) => {
                    s.id = ids(s.id);
                    go(s, ids);
                }
                Node::Prop(p) => {
                    if let Some(o) = p.own.as_mut() {
                        o.id = ids(o.id);
                    }
                }
            }
        }
    }
    if let Some(g) = essentials_mut(l) {
        go(g, ids);
    }
}

/// Settle what a file wrote for layers' essential properties, once the whole project is read.
pub(crate) fn settle(p: &mut Project) {
    rebuild(p);
}
