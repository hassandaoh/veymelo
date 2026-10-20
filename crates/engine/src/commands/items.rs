//! Commands for the project's items: folders, names, labels, notes and which folder an item is
//! in; copies of compositions; footage nothing uses taken out; another project's items brought
//! in; the project and its files gathered in one folder.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use veymelo_json::{Map, Value as Json};
use veymelo_project::{FootageKind, ItemKind, Label, Project, Source};

use super::{Command, Run};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

const ITEM: Param = Param::need("item", Ty::Comp, "the item (its id or name)");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "project.folder.new",
            label: "New Folder",
            summary: "Make a folder in the project (inside another, if given) to keep items in.",
            params: const { &[Param::need("name", Ty::Text, "what to call it"), Param::opt("folder", Ty::Comp, "the folder to put it in (default: the top)")] },
            returns: "{item}",
            run: Run::Edit(folder_new),
        },
        Command {
            id: "project.item.set",
            label: "Change Item",
            summary: "Change a project item (a composition, footage or folder): its name, label, note, or the folder it is in (0 or \"\" for the top).",
            params: const { &[ITEM, Param::opt("name", Ty::Text, "a new name"), Param::opt("label", Ty::Choice(Label::IDS), "a colour label"), Param::opt("comment", Ty::Text, "a note"), Param::opt("folder", Ty::Any, "the folder to move it into (its id or name; 0 or \"\" for the top)")] },
            returns: "{item, name, folder?, label, comment?}",
            run: Run::Edit(item_set),
        },
        Command {
            id: "project.item.duplicate",
            label: "Duplicate Item",
            summary: "Copy a composition (its layers given new ids, links between them kept) or a footage item, beside the original.",
            params: const { &[ITEM, Param::opt("name", Ty::Text, "the copy's name (default: the original's with a number)")] },
            returns: "{item, name}",
            run: Run::Edit(item_duplicate),
        },
        Command {
            id: "project.item.remove",
            label: "Remove Item",
            summary: "Take footage or a folder out of the project (a folder's items go to the folder it was in). Footage a layer shows is kept unless `force`, which takes those layers out too. Compositions are taken out with comp.remove.",
            params: const { &[ITEM, Param::opt("force", Ty::Bool, "take out the layers showing it too")] },
            returns: "{removed, layers?}",
            run: Run::Edit(item_remove),
        },
        Command {
            id: "project.removeUnused",
            label: "Remove Unused Footage",
            summary: "Take out every footage item no layer shows.",
            params: &[],
            returns: "{removed: [names]}",
            run: Run::Edit(remove_unused),
        },
        Command {
            id: "project.import",
            label: "Import Project",
            summary: "Bring every item of another .veymelo project into this one, in a folder named after it, every id given anew.",
            params: const { &[Param::need("path", Ty::Text, "the .veymelo file"), Param::opt("folder", Ty::Text, "the folder's name (default: the file's name)")] },
            returns: "{folder, items}",
            run: Run::Edit(project_import),
        },
        Command {
            id: "project.collect",
            label: "Collect Files",
            summary: "Copy the project and the files its footage comes from into one folder (footage/ inside it), the copy's footage pointing at the copies: everything in one place to move or send. The open project is left as it is.",
            params: const { &[Param::need("folder", Ty::Text, "the folder to gather into"), Param::opt("name", Ty::Text, "the project file's name (default project.veymelo)"), Param::opt("unused", Ty::Bool, "footage no layer shows too (default false)")] },
            returns: "{project, files, missing}",
            run: Run::Look(project_collect),
        },
    ]
}

/// The item `j` names: its id or name.
pub(crate) fn find_item(project: &Project, j: &Json) -> Result<u64> {
    let found = match j {
        Json::Number(_) => j.as_u64().filter(|id| project.item(*id).is_some()),
        Json::String(name) => project.items.iter().find(|i| i.name == *name).or_else(|| project.items.iter().find(|i| i.name.eq_ignore_ascii_case(name))).map(|i| i.id),
        _ => None,
    };
    found.ok_or_else(|| {
        let have: Vec<String> = project.items.iter().take(20).map(|i| format!("\"{}\" ({}, {})", i.name, i.type_id(), i.id)).collect();
        Error::not_found(format!("there is no item {} (there is {})", j.to_json(), if have.is_empty() { "nothing".into() } else { have.join(", ") }))
    })
}

fn folder_of(project: &Project, j: &Json) -> Result<Option<u64>> {
    if matches!(j, Json::Null) || j.as_u64() == Some(0) || j.as_str() == Some("") {
        return Ok(None);
    }
    let id = find_item(project, j)?;
    match project.item(id).map(|i| &i.kind) {
        Some(ItemKind::Folder) => Ok(Some(id)),
        _ => Err(Error::params(format!("{} is not a folder", j.to_json()))),
    }
}

fn describe(project: &Project, id: u64) -> Json {
    let it = project.item(id).expect("found");
    let mut o = Map::new();
    o.insert("item", id as f64);
    o.insert("name", it.name.as_str());
    o.insert("type", it.type_id());
    if let Some(f) = it.folder {
        o.insert("folder", f as f64);
    }
    o.insert("label", it.label.id());
    if !it.comment.is_empty() {
        o.insert("comment", it.comment.as_str());
    }
    Json::Object(o)
}

/// A name not yet taken: `base`, else `base 2`, `base 3`…
fn fresh(project: &Project, base: &str) -> String {
    let stem = base.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end();
    let stem = if stem.is_empty() { base } else { stem };
    (2..).map(|n| format!("{stem} {n}")).find(|n| !project.items.iter().any(|i| i.name == *n)).unwrap_or_default()
}

fn folder_new(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let parent = match a.json("folder") {
        Some(j) => folder_of(cx.project, j)?,
        None => None,
    };
    let name = a.text("name").unwrap_or("Folder");
    let id = cx.project.add(name, parent, ItemKind::Folder);
    cx.label = Some(format!("New Folder {name}"));
    Ok(Json::object([("item", Json::Number(id as f64))]))
}

fn item_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = find_item(cx.project, a.need("item"))?;
    let folder = match a.json("folder") {
        Some(j) => {
            let f = folder_of(cx.project, j)?;
            // a folder cannot go inside itself
            let mut at = f;
            while let Some(x) = at {
                if x == id {
                    return Err(Error::refused("a folder cannot go inside itself"));
                }
                at = cx.project.item(x).and_then(|i| i.folder);
            }
            Some(f)
        }
        None => None,
    };
    let it = cx.project.item_mut(id).expect("found");
    if let Some(n) = a.text("name") {
        if n.trim().is_empty() {
            return Err(Error::params("a name cannot be empty"));
        }
        it.name = n.to_string();
    }
    if let Some(l) = a.text("label").and_then(Label::from_id) {
        it.label = l;
    }
    if let Some(c) = a.text("comment") {
        it.comment = c.to_string();
    }
    if let Some(f) = folder {
        it.folder = f;
    }
    let name = it.name.clone();
    cx.label = Some(format!("Change {name}"));
    Ok(describe(cx.project, id))
}

fn item_duplicate(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = find_item(cx.project, a.need("item"))?;
    let it = cx.project.item(id).expect("found").clone();
    let name = a.text("name").map_or_else(|| fresh(cx.project, &it.name), str::to_string);
    let new = match &it.kind {
        ItemKind::Comp(_) => cx.project.duplicate_comp(id, &name).expect("a composition"),
        ItemKind::Footage(f) => cx.project.add(&name, it.folder, ItemKind::Footage(f.clone())),
        ItemKind::Folder => return Err(Error::params("a folder is not copied; copy what is in it")),
    };
    cx.label = Some(format!("Duplicate {}", it.name));
    Ok(Json::object([("item", Json::Number(new as f64)), ("name", Json::from(name))]))
}

/// Ids of footage some layer shows.
fn used(project: &Project) -> HashSet<u64> {
    project.items.iter().filter_map(|i| i.as_comp()).flat_map(|c| c.layers.iter()).filter_map(|l| if let Source::Footage(id) | Source::Model(id) = l.source { Some(id) } else { None }).collect()
}

fn item_remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = find_item(cx.project, a.need("item"))?;
    let it = cx.project.item(id).expect("found").clone();
    let mut layers = 0;
    match it.kind {
        ItemKind::Comp(_) => return Err(Error::params("comp.remove takes compositions out")),
        ItemKind::Folder => {
            for i in &mut cx.project.items {
                if i.folder == Some(id) {
                    i.folder = it.folder;
                }
            }
        }
        ItemKind::Footage(_) => {
            if used(cx.project).contains(&id) {
                if !a.bool("force").unwrap_or(false) {
                    return Err(Error::refused(format!("layers show {}; force: true takes them out too", it.name)));
                }
                let comps: Vec<u64> = cx.project.items.iter().filter(|i| i.as_comp().is_some()).map(|i| i.id).collect();
                for c in comps {
                    let comp = cx.project.comp_mut(c).expect("a composition");
                    let gone: Vec<u64> = comp.layers.iter().filter(|l| matches!(l.source, Source::Footage(x) | Source::Model(x) if x == id)).map(|l| l.id).collect();
                    layers += gone.len();
                    comp.layers.retain(|l| !gone.contains(&l.id));
                    for l in &mut comp.layers {
                        if l.parent.is_some_and(|p| gone.contains(&p)) {
                            l.parent = None;
                        }
                        if l.matte.is_some_and(|m| gone.contains(&m.layer)) {
                            l.matte = None;
                        }
                    }
                }
            }
        }
    }
    cx.project.items.retain(|i| i.id != id);
    cx.label = Some(format!("Remove {}", it.name));
    let mut o = Map::new();
    o.insert("removed", id as f64);
    if layers > 0 {
        o.insert("layers", layers as f64);
    }
    Ok(Json::Object(o))
}

fn remove_unused(cx: &mut Ctx, _: &Args) -> Result<Json> {
    let used = used(cx.project);
    let gone: Vec<String> = cx.project.items.iter().filter(|i| matches!(i.kind, ItemKind::Footage(_)) && !used.contains(&i.id)).map(|i| i.name.clone()).collect();
    cx.project.items.retain(|i| !matches!(i.kind, ItemKind::Footage(_)) || used.contains(&i.id));
    cx.label = Some("Remove Unused Footage".into());
    Ok(Json::object([("removed", Json::array(gone.iter().map(|n| Json::from(n.as_str()))))]))
}

fn project_import(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let path = PathBuf::from(a.text("path").unwrap_or(""));
    let other = Project::open(&path, cx.catalog).map_err(Error::not_found)?;
    let name = a.text("folder").map_or_else(|| path.file_stem().map_or("Imported".into(), |s| s.to_string_lossy().into_owned()), str::to_string);
    let folder = cx.project.add(&name, None, ItemKind::Folder);
    let items = cx.project.merge(other, Some(folder));
    cx.label = Some(format!("Import {name}"));
    Ok(Json::object([("folder", Json::Number(folder as f64)), ("items", Json::array(items.iter().map(|i| Json::Number(*i as f64))))]))
}

fn project_collect(l: &Look, a: &Args) -> Result<Json> {
    let dir = PathBuf::from(a.text("folder").unwrap_or(""));
    let footage_dir = dir.join("footage");
    std::fs::create_dir_all(&footage_dir).map_err(|e| Error::refused(format!("{}: {e}", footage_dir.display())))?;
    let used = used(l.project);
    let mut copy = l.project.clone();
    let (mut files, mut missing) = (Vec::new(), Vec::new());
    let mut taken: HashSet<String> = HashSet::new();
    for it in &mut copy.items {
        let keep = a.bool("unused").unwrap_or(false) || used.contains(&it.id);
        let ItemKind::Footage(f) = &mut it.kind else { continue };
        if !keep {
            continue;
        }
        // a name of its own in footage/ (two files of one name are told apart by their item)
        let name_of = |p: &str| Path::new(p).file_name().map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
        let pattern = name_of(&f.path);
        let target = if taken.insert(pattern.clone()) { pattern } else { format!("{}_{}", it.id, pattern) };
        let frames: Vec<(String, String)> = if f.kind == FootageKind::Sequence {
            let n = (f.duration.seconds() * f.frame_rate.as_f64()).round() as u64;
            let renamed = veymelo_project::Footage { path: footage_dir.join(&target).to_string_lossy().into_owned(), ..f.clone() };
            (0..n).map(|k| (f.frame_path(k), renamed.frame_path(k))).collect()
        } else {
            vec![(f.path.clone(), footage_dir.join(&target).to_string_lossy().into_owned())]
        };
        for (from, to) in &frames {
            match std::fs::copy(from, to) {
                Ok(_) => files.push(to.clone()),
                Err(_) => missing.push(from.clone()),
            }
        }
        // whole here; saved relative to the project file, which is beside footage/
        f.path = footage_dir.join(&target).to_string_lossy().into_owned();
    }
    let file = dir.join(a.text("name").unwrap_or("project.veymelo"));
    copy.save(&file).map_err(Error::refused)?;
    Ok(Json::object([
        ("project", Json::from(file.to_string_lossy().as_ref())),
        ("files", Json::array(files.iter().map(|f| Json::from(f.as_str())))),
        ("missing", Json::array(missing.iter().map(|f| Json::from(f.as_str())))),
    ]))
}
