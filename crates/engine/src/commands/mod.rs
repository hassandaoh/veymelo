//! Every command, and what commands share: finding the composition and layer a parameter names.

mod arrange;
mod assist;
pub(crate) mod audio;
mod camera;
mod comp;
mod cut;
pub(crate) mod effect;
mod essential;
mod files;
mod footage;
mod group;
mod items;
mod key;
mod layer;
mod layered;
mod link;
mod lottie;
mod mask;
mod model;
mod paint;
mod paths;
mod preset;
mod prerender;
mod prop;
mod puppet;
mod queue;
mod render;
mod session;
mod shape;
mod style;
mod templates;
mod text;
mod time;
mod timeline;
mod track;
mod vector;

use std::sync::OnceLock;

use veymelo_json::Value as Json;
use veymelo_project::{Comp, Project};

use crate::{Args, Ctx, Error, Look, Param, Result, Session};

/// How a command runs.
#[derive(Clone, Copy)]
pub enum Run {
    /// It only looks.
    Look(fn(&Look, &Args) -> Result<Json>),
    /// It changes the project (or the view) on a copy, kept when it succeeds: one undo step.
    Edit(fn(&mut Ctx, &Args) -> Result<Json>),
    /// It works on the whole session (undo, open, save, batch); not inside a batch.
    Session(fn(&mut Session, &Args) -> Result<Json>),
}

/// A command: its id, what it is called as an undo step, what it does and what it takes.
pub struct Command {
    pub id: &'static str,
    pub label: &'static str,
    pub summary: &'static str,
    pub params: &'static [Param],
    /// What it gives back.
    pub returns: &'static str,
    pub run: Run,
}

impl Command {
    /// Whether it changes the project.
    pub fn changes(&self) -> bool {
        !matches!(self.run, Run::Look(_))
    }
}

/// Every command, ordered by id.
pub fn all() -> &'static [Command] {
    static ALL: OnceLock<Vec<Command>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut v = Vec::new();
        v.extend(session::commands());
        v.extend(comp::commands());
        v.extend(layer::commands());
        v.extend(prop::commands());
        v.extend(render::commands());
        v.extend(key::commands());
        v.extend(time::commands());
        v.extend(shape::commands());
        v.extend(text::commands());
        v.extend(effect::commands());
        v.extend(style::commands());
        v.extend(queue::commands());
        v.extend(lottie::commands());
        v.extend(footage::commands());
        v.extend(audio::commands());
        v.extend(items::commands());
        v.extend(assist::commands());
        v.extend(cut::commands());
        v.extend(paths::commands());
        v.extend(arrange::commands());
        v.extend(preset::commands());
        v.extend(prerender::commands());
        v.extend(essential::commands());
        v.extend(link::commands());
        v.extend(group::commands());
        v.extend(mask::commands());
        v.extend(files::commands());
        v.extend(templates::commands());
        v.extend(timeline::commands());
        v.extend(camera::commands());
        v.extend(paint::commands());
        v.extend(puppet::commands());
        v.extend(track::commands());
        v.extend(model::commands());
        v.extend(vector::commands());
        v.sort_by_key(|c| c.id);
        v
    })
}

pub fn find(id: &str) -> Option<&'static Command> {
    let all = all();
    all.binary_search_by_key(&id, |c| c.id).ok().map(|i| &all[i])
}

/// Run another command that changes the project inside this one (part of the same undo step).
pub(crate) fn call(cx: &mut Ctx, id: &str, params: Json) -> Result<Json> {
    let c = find(id).unwrap_or_else(|| panic!("there is a command {id}"));
    let args = Args::check(c.id, c.params, &params)?;
    match c.run {
        Run::Edit(f) => f(cx, &args),
        _ => panic!("{id} changes the project"),
    }
}

/// The composition `comp` names (an id or a name), or the one the view is on.
pub(crate) fn comp_id(project: &Project, view: &crate::View, args: &Args) -> Result<u64> {
    match args.json("comp") {
        Some(j) => find_comp(project, j),
        None => view.comp.filter(|c| project.comp(*c).is_some()).ok_or_else(|| Error::not_found("there is no composition yet (comp.new makes one)")),
    }
}

pub(crate) fn find_comp(project: &Project, j: &Json) -> Result<u64> {
    let comps: Vec<(u64, &str)> = project.items.iter().filter(|i| i.as_comp().is_some()).map(|i| (i.id, i.name.as_str())).collect();
    let found = match j {
        Json::Number(_) => j.as_u64().filter(|id| comps.iter().any(|c| c.0 == *id)),
        Json::String(name) => comps.iter().find(|c| c.1 == name).or_else(|| comps.iter().find(|c| c.1.eq_ignore_ascii_case(name))).map(|c| c.0),
        _ => None,
    };
    found.ok_or_else(|| {
        let have = if comps.is_empty() { "there are none".to_string() } else { format!("there is {}", comps.iter().map(|(id, n)| format!("\"{n}\" ({id})")).collect::<Vec<_>>().join(", ")) };
        Error::not_found(format!("there is no composition {} ({have})", j.to_json()))
    })
}

/// The footage item `j` names: its id or its name.
pub(crate) fn find_footage(project: &Project, j: &Json) -> Result<u64> {
    let all: Vec<(u64, &str)> = project.items.iter().filter(|i| matches!(i.kind, veymelo_project::ItemKind::Footage(_))).map(|i| (i.id, i.name.as_str())).collect();
    let found = match j {
        Json::Number(_) => j.as_u64().filter(|id| all.iter().any(|c| c.0 == *id)),
        Json::String(name) => all.iter().find(|c| c.1 == name).or_else(|| all.iter().find(|c| c.1.eq_ignore_ascii_case(name))).map(|c| c.0),
        _ => None,
    };
    found.ok_or_else(|| {
        let have = if all.is_empty() { "there is none (footage.import brings files in)".to_string() } else { format!("there is {}", all.iter().map(|(id, n)| format!("\"{n}\" ({id})")).collect::<Vec<_>>().join(", ")) };
        Error::not_found(format!("there is no footage {} ({have})", j.to_json()))
    })
}

/// The layer of `comp` that `j` names: its id, `"#n"` (its place from the top) or its name.
pub(crate) fn find_layer(comp: &Comp, j: &Json) -> Result<u64> {
    let found = match j {
        Json::Number(_) => j.as_u64().filter(|id| comp.layer(*id).is_some()),
        Json::String(s) => match s.strip_prefix('#').map(str::parse::<usize>) {
            Some(Ok(n)) => n.checked_sub(1).and_then(|i| comp.layers.get(i)).map(|l| l.id),
            _ => comp.layers.iter().find(|l| l.name == *s).or_else(|| comp.layers.iter().find(|l| l.name.eq_ignore_ascii_case(s))).map(|l| l.id),
        },
        _ => None,
    };
    found.ok_or_else(|| {
        let have = if comp.layers.is_empty() {
            "it has no layers".to_string()
        } else {
            format!("it has {}", comp.layers.iter().enumerate().map(|(i, l)| format!("#{} \"{}\" ({})", i + 1, l.name, l.id)).collect::<Vec<_>>().join(", "))
        };
        Error::not_found(format!("there is no layer {} ({have})", j.to_json()))
    })
}

/// The composition and layer the `comp` and `layer` parameters name.
pub(crate) fn layer_ids(project: &Project, view: &crate::View, args: &Args) -> Result<(u64, u64)> {
    let cid = comp_id(project, view, args)?;
    let comp = project.comp(cid).expect("comp_id found it");
    Ok((cid, find_layer(comp, args.need("layer"))?))
}

/// A name like `base` not used by any of `taken`: `Solid 1`, `Solid 2`…
pub(crate) fn fresh_name<'a>(base: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    (1..).map(|n| format!("{base} {n}")).find(|name| !taken.clone().any(|t| t == name)).expect("some number is free")
}

/// The parameters that pick a composition and a layer, for commands about one layer.
pub(crate) const COMP: Param = Param::opt("comp", crate::Ty::Comp, "the composition (default: the one being worked on)");
pub(crate) const LAYER: Param = Param::need("layer", crate::Ty::Layer, "the layer");
