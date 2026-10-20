//! Footage files and the project's items: the files not there and finding them again (a moved
//! folder found from one file, or a folder searched), placeholders, footage taken the way other
//! footage is, items that are the same made one, a project cut down to some compositions, where
//! an item is used, and footage put into a composition as an edit (over what is there, or
//! pushing it later).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use veymelo_json::Value as Json;
use veymelo_project::{Footage, FootageKind, ItemKind, Project, Source};
use veymelo_time::{FrameRate, Tick};

use super::{COMP, Command, Run, comp_id, find_comp, find_footage};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "footage.missing",
            label: "Missing Footage",
            summary: "The footage items whose files are not there (moved, renamed, on a disk not in).",
            params: &[],
            returns: "[{item, name, path}]",
            run: Run::Look(missing_list),
        },
        Command {
            id: "footage.relink",
            label: "Relink Footage",
            summary: "Point a footage item whose file moved at where it is now, how it is taken kept; and every other missing file that was in the same place beside it is found again too (a folder moved, a disk renamed).",
            params: const { &[Param::need("item", Ty::Any, "the footage (id or name)"), Param::need("path", Ty::Text, "where its file is now")] },
            returns: "{relinked: [name], stillMissing}",
            run: Run::Edit(relink),
        },
        Command {
            id: "footage.find",
            label: "Find Missing Footage",
            summary: "Look through a folder (and the folders in it) for the files of missing footage, by name, and relink each found whose size, kind and length match; `dryRun` only says what it would do.",
            params: const { &[Param::need("folder", Ty::Text, "where to look"), Param::opt("dryRun", Ty::Bool, "only say what would be relinked")] },
            returns: "{relinked: [{name, path}], differs: [{name, path, why}], notFound: [name]}",
            run: Run::Edit(find_missing),
        },
        Command {
            id: "footage.placeholder",
            label: "Import Placeholder",
            summary: "Make footage to stand in for a file there is not yet: drawn as colour bars of its size, as long as it is given; replace it (footage.replace or footage.relink) when the file comes.",
            params: const {
                &[
                    Param::need("name", Ty::Text, "what to call it"),
                    Param::opt("width", Ty::Whole, "pixels across (default the composition being worked on's, or 1920)"),
                    Param::opt("height", Ty::Whole, "pixels down (default its, or 1080)"),
                    Param::opt("duration", Ty::Number, "seconds long (default 10; 0: a still)"),
                    Param::opt("fps", Ty::Number, "frames a second (default 30)"),
                    Param::opt("path", Ty::Text, "the file it stands for, if known"),
                    Param::opt("folder", Ty::Whole, "the folder item to put it in"),
                ]
            },
            returns: "{item, name}",
            run: Run::Edit(placeholder),
        },
        Command {
            id: "footage.interpretLike",
            label: "Apply Interpretation",
            summary: "Take footage the way other footage is taken: its alpha, the rate it plays at, its loops and its pixels' shape.",
            params: const { &[Param::need("item", Ty::Any, "the footage to change"), Param::need("from", Ty::Any, "the footage to take it like")] },
            returns: "{item, alpha, fps, loops, pixelAspect}",
            run: Run::Edit(interpret_like),
        },
        Command {
            id: "project.consolidate",
            label: "Consolidate All Footage",
            summary: "Make footage items that are the same (one file taken one way) into one: the layers showing the others show it, and the others go.",
            params: &[],
            returns: "{merged: [name]}",
            run: Run::Edit(consolidate),
        },
        Command {
            id: "project.reduce",
            label: "Reduce Project",
            summary: "Keep only these compositions and what they show (the compositions and footage in them, and so on) and take every other item out, with renders of what goes; folders left empty go too.",
            params: const { &[Param::need("comps", Ty::List, "the compositions to keep (ids or names)")] },
            returns: "{removed: [name]}",
            run: Run::Edit(reduce),
        },
        Command {
            id: "project.item.usage",
            label: "Item Usage",
            summary: "Where an item is used: the layers that show it (in which compositions), and the renders of it.",
            params: const { &[Param::need("item", Ty::Any, "the item (id or name)")] },
            returns: "{layers: [{comp, layer, name}], renders: [id]}",
            run: Run::Look(usage),
        },
        Command {
            id: "layer.insertFootage",
            label: "Insert Footage",
            summary: "Put footage (or a composition) into a composition as an edit at a time: from `from` to `to` of it (default all), on top of what is there (overlay); or with `ripple`, pushing the layers from then on later by its length (one across that moment cut there, its later part pushed).",
            params: const {
                &[
                    COMP,
                    Param::need("item", Ty::Any, "the footage or composition (id or name)"),
                    Param::opt("at", Ty::Time, "when in the composition it goes (default: the view's time)"),
                    Param::opt("from", Ty::Number, "where in it to start, seconds (default 0)"),
                    Param::opt("to", Ty::Number, "where in it to end, seconds (default its end)"),
                    Param::opt("ripple", Ty::Bool, "push what comes after it later (default: over it)"),
                ]
            },
            returns: "{layer, in, out, pushed: [name]}",
            run: Run::Edit(insert_footage),
        },
    ]
}

/// The file whose being there says footage is there (a sequence's first picture).
fn first_file(f: &Footage) -> String {
    f.frame_path(0)
}

/// Whether footage's file is not there (a placeholder stands for none).
fn is_missing(f: &Footage) -> bool {
    f.kind != FootageKind::Placeholder && !f.path.is_empty() && !Path::new(&first_file(f)).exists()
}

fn missing_list(l: &Look, _: &Args) -> Result<Json> {
    Ok(Json::array(l.project.items.iter().filter_map(|i| match &i.kind {
        ItemKind::Footage(f) if is_missing(f) => Some(Json::object([("item", Json::Number(i.id as f64)), ("name", Json::from(i.name.as_str())), ("path", Json::from(f.path.as_str()))])),
        _ => None,
    })))
}

/// A path's parts (split on `/` and `\`, so a project made on another system relinks too).
fn parts(path: &str) -> Vec<&str> {
    path.split(['/', '\\']).filter(|p| !p.is_empty()).collect()
}

/// What moved when `old` became `new`: the parts both end with are the file's place inside the
/// moved folder; what comes before is where the folder was (parts) and is now (a path).
fn moved_root<'a>(old: &'a str, new: &str) -> Option<(Vec<&'a str>, String)> {
    let (o, n) = (parts(old), parts(new));
    let same = o.iter().rev().zip(n.iter().rev()).take_while(|(a, b)| a == b).count().max(1).min(o.len()).min(n.len());
    let was = o.get(..o.len() - same)?.to_vec();
    let lead = if new.starts_with('/') { "/" } else { "" };
    Some((was, format!("{lead}{}", n[..n.len() - same].join("/"))))
}

/// `path` under where the folder is now, if it was under where it was.
fn remap(path: &str, was: &[&str], now: &str) -> Option<String> {
    let p = parts(path);
    (p.len() > was.len() && p[..was.len()] == *was).then(|| format!("{}/{}", now.trim_end_matches('/'), p[was.len()..].join("/")))
}

/// Footage pointed at another file, how it is taken kept (a sequence its pattern kept beside
/// its first picture there).
fn moved_to(f: &Footage, path: &str) -> Footage {
    let mut g = f.clone();
    if f.kind == FootageKind::Sequence {
        // the pattern's own file name, in the new folder
        let name = Path::new(&f.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        g.path = Path::new(path).parent().map_or(name.clone(), |d| d.join(&name).to_string_lossy().into_owned());
    } else {
        g.path = path.to_string();
    }
    g
}

fn relink(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = find_footage(cx.project, a.need("item"))?;
    let path = a.text("path").unwrap_or("").to_string();
    if !Path::new(&path).exists() {
        return Err(Error::not_found(format!("{path} is not there either")));
    }
    let ItemKind::Footage(old) = cx.project.item(id).expect("found").kind.clone() else { unreachable!("found footage") };
    let moved = moved_to(&old, &path);
    let old_file = first_file(&old);
    veymelo_media::forget(Some(&old.path));
    let mut relinked = vec![cx.project.item(id).expect("found").name.clone()];
    if let Some(ItemKind::Footage(f)) = cx.project.item_mut(id).map(|i| &mut i.kind) {
        *f = moved;
    }
    // the others that were beside it, found where it is now
    if let Some((was, now)) = moved_root(&old_file, &path) {
        let others: Vec<(u64, Footage)> = cx.project.items.iter().filter(|i| i.id != id).filter_map(|i| match &i.kind {
            ItemKind::Footage(f) if is_missing(f) => Some((i.id, f.clone())),
            _ => None,
        }).collect();
        for (oid, f) in others {
            let Some(there) = remap(&first_file(&f), &was, &now).filter(|p| Path::new(p).exists()) else { continue };
            veymelo_media::forget(Some(&f.path));
            let item = cx.project.item_mut(oid).expect("listed");
            relinked.push(item.name.clone());
            item.kind = ItemKind::Footage(moved_to(&f, &there));
        }
    }
    let still = cx.project.items.iter().filter(|i| matches!(&i.kind, ItemKind::Footage(f) if is_missing(f))).count();
    cx.label = Some("Relink Footage".into());
    Ok(Json::object([("relinked", Json::array(relinked.into_iter().map(Json::from))), ("stillMissing", Json::Number(still as f64))]))
}

/// Every file under `folder` by name (not following links), at most so many.
fn files_under(folder: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    const MOST: usize = 100_000;
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let (mut pending, mut seen) = (vec![folder.to_path_buf()], 0usize);
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            seen += 1;
            if seen > MOST {
                return Err(Error::refused(format!("there are over {MOST} files there; look in a smaller folder")));
            }
            let Ok(ty) = e.file_type() else { continue };
            if ty.is_dir() {
                pending.push(e.path());
            } else if ty.is_file()
                && let (Some(name), Some(p)) = (e.file_name().to_str(), e.path().to_str())
            {
                found.entry(name.to_string()).or_default().push(p.to_string());
            }
        }
    }
    for v in found.values_mut() {
        v.sort();
    }
    Ok(found)
}

/// Why a file found is not the footage it might be (none: it is).
fn differs(f: &Footage, path: &str) -> Option<String> {
    let g = match veymelo_media::probe(path, false) {
        Ok(g) => g,
        Err(e) => return Some(format!("cannot be read: {e}")),
    };
    let kind = if f.kind == FootageKind::Sequence { FootageKind::Image } else { f.kind };
    if g.kind != kind {
        return Some(format!("a {}, not a {}", g.kind.id(), kind.id()));
    }
    if f.kind != FootageKind::Audio && (g.width, g.height) != (f.width, f.height) {
        return Some(format!("{} × {}, not {} × {}", g.width, g.height, f.width, f.height));
    }
    if matches!(f.kind, FootageKind::Video | FootageKind::Audio) {
        let rate = f.native_rate.unwrap_or(f.frame_rate);
        let frames = |t: Tick, r: FrameRate| (t.seconds() * r.as_f64()).round();
        let own = frames(f.duration, f.frame_rate);
        if (frames(g.duration, g.frame_rate) - own).abs() > 1.0 || (g.frame_rate.as_f64() - rate.as_f64()).abs() > 0.01 {
            return Some(format!("{:.2} s at {} a second, not {:.2} s at {}", g.duration.seconds(), g.frame_rate.as_f64(), f.duration.seconds(), rate.as_f64()));
        }
    }
    None
}

fn find_missing(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let folder = Path::new(a.text("folder").unwrap_or(""));
    if !folder.is_dir() {
        return Err(Error::not_found(format!("{} is not a folder", folder.display())));
    }
    let index = files_under(folder)?;
    let dry = a.bool("dryRun").unwrap_or(false);
    let missing: Vec<(u64, String, Footage)> = cx.project.items.iter().filter_map(|i| match &i.kind {
        ItemKind::Footage(f) if is_missing(f) => Some((i.id, i.name.clone(), f.clone())),
        _ => None,
    }).collect();
    let (mut relinked, mut odd, mut none) = (Vec::new(), Vec::new(), Vec::new());
    for (id, name, f) in missing {
        let file = first_file(&f);
        let wanted = Path::new(&file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let candidates = index.get(&wanted).cloned().unwrap_or_default();
        let mut hit = None;
        for c in &candidates {
            match differs(&f, c) {
                None => {
                    hit = Some(c.clone());
                    break;
                }
                Some(why) => odd.push(Json::object([("name", Json::from(name.as_str())), ("path", Json::from(c.as_str())), ("why", Json::from(why))])),
            }
        }
        match hit {
            Some(path) => {
                if !dry {
                    veymelo_media::forget(Some(&f.path));
                    cx.project.item_mut(id).expect("listed").kind = ItemKind::Footage(moved_to(&f, &path));
                }
                relinked.push(Json::object([("name", Json::from(name.as_str())), ("path", Json::from(path))]));
            }
            None if candidates.is_empty() => none.push(Json::from(name.as_str())),
            None => {}
        }
    }
    cx.label = Some("Find Missing Footage".into());
    Ok(Json::object([("relinked", Json::Array(relinked)), ("differs", Json::Array(odd)), ("notFound", Json::Array(none))]))
}

fn placeholder(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (w0, h0) = cx.view.comp.and_then(|c| cx.project.comp(c)).map_or((1920, 1080), |c| (c.width, c.height));
    let side = |k: &str, d: u32| -> Result<u32> {
        match a.whole(k) {
            Some(v) if (1..=veymelo_project::MAX_SIZE as u64).contains(&v) => Ok(v as u32),
            Some(v) => Err(Error::params(format!("`{k}` is from 1 to {}, not {v}", veymelo_project::MAX_SIZE))),
            None => Ok(d),
        }
    };
    let (width, height) = (side("width", w0)?, side("height", h0)?);
    let fps = a.num("fps").unwrap_or(30.0);
    let rate = FrameRate::from_f64(fps).ok_or_else(|| Error::params(format!("`fps` is above 0 and at most 1000, not {fps}")))?;
    let seconds = a.num("duration").unwrap_or(10.0);
    if !(0.0..=veymelo_project::MAX_SECONDS).contains(&seconds) {
        return Err(Error::params(format!("`duration` is from 0 to {} seconds", veymelo_project::MAX_SECONDS)));
    }
    let folder = match a.whole("folder") {
        Some(id) if matches!(cx.project.item(id).map(|i| &i.kind), Some(ItemKind::Folder)) => Some(id),
        Some(id) => return Err(Error::not_found(format!("there is no folder item {id}"))),
        None => None,
    };
    let mut f = Footage::new(a.text("path").unwrap_or(""), FootageKind::Placeholder);
    (f.width, f.height, f.frame_rate, f.duration) = (width, height, rate, Tick::from_seconds(seconds));
    let name = a.text("name").unwrap_or("Placeholder").to_string();
    let id = cx.project.add(&name, folder, ItemKind::Footage(f));
    cx.label = Some(format!("Import Placeholder {name}"));
    Ok(Json::object([("item", Json::Number(id as f64)), ("name", Json::from(name))]))
}

fn interpret_like(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let id = find_footage(cx.project, a.need("item"))?;
    let from = find_footage(cx.project, a.need("from"))?;
    let ItemKind::Footage(model) = cx.project.item(from).expect("found").kind.clone() else { unreachable!("found footage") };
    let item = cx.project.item_mut(id).expect("found");
    let ItemKind::Footage(f) = &mut item.kind else { unreachable!("found footage") };
    f.alpha = model.alpha;
    f.loops = model.loops;
    f.pixel_aspect = model.pixel_aspect;
    // its rate, conformed to the other's: its frames kept, its length changed
    if model.native_rate.is_some() && f.kind != FootageKind::Image {
        let native = f.native_rate.unwrap_or(f.frame_rate);
        let frames = (f.duration.seconds() * f.frame_rate.as_f64()).round() as i64;
        f.native_rate = Some(native);
        f.frame_rate = model.frame_rate;
        f.duration = model.frame_rate.tick_of(frames);
    }
    veymelo_media::forget(Some(&f.path));
    let out = Json::object([
        ("item", Json::Number(id as f64)),
        ("alpha", Json::from(f.alpha.id())),
        ("fps", Json::Number(f.frame_rate.as_f64())),
        ("loops", Json::Number(f.loops as f64)),
        ("pixelAspect", Json::Number(f.pixel_aspect)),
    ]);
    cx.label = Some(format!("Apply Interpretation to {}", item.name));
    Ok(out)
}

/// Every layer showing item `id`, in every composition: (composition, layer).
fn layers_showing(project: &Project, id: u64) -> Vec<(u64, u64)> {
    project
        .items
        .iter()
        .filter_map(|i| i.as_comp().map(|c| (i.id, c)))
        .flat_map(|(cid, c)| c.layers.iter().filter(move |l| matches!(l.source, Source::Footage(x) | Source::Comp(x) | Source::Model(x) if x == id)).map(move |l| (cid, l.id)))
        .collect()
}

fn consolidate(cx: &mut Ctx, _: &Args) -> Result<Json> {
    let footage: Vec<(u64, String, Footage)> = cx.project.items.iter().filter_map(|i| if let ItemKind::Footage(f) = &i.kind { Some((i.id, i.name.clone(), f.clone())) } else { None }).collect();
    let mut keep: Vec<(u64, Footage)> = Vec::new();
    let mut to: Vec<(u64, u64, String)> = Vec::new();
    for (id, name, f) in footage {
        match keep.iter().find(|(_, k)| *k == f && f.kind != FootageKind::Placeholder) {
            Some((first, _)) => to.push((id, *first, name)),
            None => keep.push((id, f)),
        }
    }
    let comps: Vec<u64> = cx.project.items.iter().filter(|i| i.as_comp().is_some()).map(|i| i.id).collect();
    for (gone, first, _) in &to {
        for c in &comps {
            let comp = cx.project.comp_mut(*c).expect("a composition");
            for l in comp.layers.iter_mut() {
                match &mut l.source {
                    Source::Footage(x) | Source::Model(x) if *x == *gone => *x = *first,
                    _ => {}
                }
            }
        }
    }
    let gone: HashSet<u64> = to.iter().map(|t| t.0).collect();
    cx.project.items.retain(|i| !gone.contains(&i.id));
    cx.label = Some("Consolidate All Footage".into());
    Ok(Json::object([("merged", Json::array(to.into_iter().map(|t| Json::from(t.2))))]))
}

fn reduce(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let mut keep: HashSet<u64> = HashSet::new();
    let mut pending: Vec<u64> = Vec::new();
    for j in a.json("comps").and_then(Json::as_array).cloned().unwrap_or_default() {
        pending.push(find_comp(cx.project, &j)?);
    }
    if pending.is_empty() {
        return Err(Error::params("give `comps`, the compositions to keep"));
    }
    // what they show, and what that shows
    while let Some(id) = pending.pop() {
        if !keep.insert(id) {
            continue;
        }
        if let Some(c) = cx.project.comp(id) {
            for l in &c.layers {
                if let Source::Footage(x) | Source::Comp(x) | Source::Model(x) = l.source {
                    pending.push(x);
                }
            }
        }
    }
    // folders holding what is kept stay
    let folders: Vec<u64> = cx.project.items.iter().filter(|i| keep.contains(&i.id)).filter_map(|i| i.folder).collect();
    let mut up = folders;
    while let Some(f) = up.pop() {
        if keep.insert(f)
            && let Some(parent) = cx.project.item(f).and_then(|i| i.folder)
        {
            up.push(parent);
        }
    }
    let removed: Vec<String> = cx.project.items.iter().filter(|i| !keep.contains(&i.id)).map(|i| i.name.clone()).collect();
    cx.project.items.retain(|i| keep.contains(&i.id));
    cx.project.queue.retain(|r| keep.contains(&r.comp));
    cx.label = Some("Reduce Project".into());
    Ok(Json::object([("removed", Json::array(removed.into_iter().map(Json::from)))]))
}

fn usage(l: &Look, a: &Args) -> Result<Json> {
    let id = super::items::find_item(l.project, a.need("item"))?;
    let layers = layers_showing(l.project, id).into_iter().map(|(c, lid)| {
        let comp = l.project.comp(c).expect("a composition");
        let name = comp.layer(lid).map_or(String::new(), |x| x.name.clone());
        Json::object([("comp", Json::from(l.project.item(c).map_or("", |i| i.name.as_str()))), ("layer", Json::Number(lid as f64)), ("name", Json::from(name))])
    });
    let renders = l.project.queue.iter().filter(|r| r.comp == id).map(|r| Json::Number(r.id as f64));
    Ok(Json::object([("layers", Json::array(layers)), ("renders", Json::array(renders))]))
}

fn insert_footage(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let cid = comp_id(cx.project, cx.view, a)?;
    let item = super::items::find_item(cx.project, a.need("item"))?;
    let (kind, length) = match cx.project.item(item).map(|i| &i.kind) {
        Some(ItemKind::Footage(f)) => ("footage", if f.duration > Tick::ZERO && f.kind != FootageKind::Image { f.duration } else { cx.project.comp(cid).expect("found").duration }),
        Some(ItemKind::Comp(c)) => ("comp", c.duration),
        _ => return Err(Error::params("`item` is footage or a composition")),
    };
    if item == cid {
        return Err(Error::refused("a composition cannot show itself"));
    }
    let at = a.time("at").unwrap_or(cx.view.time);
    let from = Tick::from_seconds(a.num("from").unwrap_or(0.0).max(0.0));
    let to = a.num("to").map_or(length, Tick::from_seconds).min(length);
    if to <= from {
        return Err(Error::params(format!("`to` ({} s) comes after `from` ({} s)", to.seconds(), from.seconds())));
    }
    let span = to - from;
    let mut pushed = Vec::new();
    if a.bool("ripple") == Some(true) {
        // one across the moment is cut there; it and all after go later
        let across: Vec<u64> = cx.project.comp(cid).expect("found").layers.iter().filter(|l| l.in_point < at && l.out_point > at).map(|l| l.id).collect();
        for id in across {
            super::call(cx, "layer.split", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(id as f64)), ("time", Json::Number(at.seconds()))]))?;
        }
        let comp = cx.project.comp_mut(cid).expect("found");
        for l in comp.layers.iter_mut().filter(|l| l.in_point >= at) {
            (l.start, l.in_point, l.out_point) = (l.start + span, l.in_point + span, l.out_point + span);
            pushed.push(Json::from(l.name.as_str()));
        }
        let end = comp.layers.iter().map(|l| l.out_point).max().unwrap_or(Tick::ZERO);
        if end > comp.duration {
            comp.duration = end.min(Tick::from_seconds(veymelo_project::MAX_SECONDS));
        }
    }
    // the new layer on top: its own time `from` at `at`
    let made = super::call(cx, "layer.add", Json::object([("comp", Json::Number(cid as f64)), ("type", Json::from(kind)), ("item", Json::Number(item as f64))]))?;
    let lid = made.get("layer").and_then(Json::as_u64).expect("made");
    let comp = cx.project.comp_mut(cid).expect("found");
    let layer = comp.layer_mut(lid).expect("made");
    (layer.start, layer.in_point, layer.out_point) = (at - from, at, at + span);
    let out = Json::object([("layer", Json::Number(lid as f64)), ("in", Json::Number(at.seconds())), ("out", Json::Number((at + span).seconds())), ("pushed", Json::Array(pushed))]);
    cx.label = Some(if a.bool("ripple") == Some(true) { "Ripple Insert Edit" } else { "Overlay Edit" }.into());
    Ok(out)
}
