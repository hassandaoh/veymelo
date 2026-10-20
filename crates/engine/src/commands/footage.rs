//! Commands for footage: files brought into the project (pictures, animated pictures, picture
//! sequences, video, sound), how each is taken (its alpha, the rate it plays at, how many
//! times it loops, its pixels' shape), and putting another file in its place.

use veymelo_json::{Map, Value as Json};
use veymelo_project::{AlphaMode, Footage, FootageKind, ItemKind, fps_json};
use veymelo_time::Tick;

use super::{COMP, Command, LAYER, Run, call, find_footage, layer_ids};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

const ITEM: Param = Param::need("item", Ty::Comp, "the footage (its id or name)");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "layer.sceneEdits",
            label: "Scene Edit Detection",
            summary: "Find where a video layer's footage cuts from one shot to the next, then mark each cut on the layer (markers), cut the layer there into a layer a shot (split), or also put each shot in a composition of its own, trimmed to it (splitPrecompose). Steady motion is not taken for a cut; fades between shots are not found.",
            params: const {
                &[
                    COMP,
                    LAYER,
                    Param::opt("make", Ty::Choice(&["markers", "split", "splitPrecompose", "none"]), "what to do at the cuts (default markers)"),
                    Param::opt("sensitivity", Ty::Number, "0 (only stark cuts) to 100 (faint ones too); default 50"),
                ]
            },
            returns: "{cuts: [{time, frame, strength}], layers?, comps?}",
            run: Run::Edit(scene_edits),
        },
        Command {
            id: "footage.import",
            label: "Import Footage",
            summary: "Bring a file into the project as footage: a picture (PNG, JPEG, HEIC, TIFF, PSD, EXR, camera raw…), a vector picture (SVG, drawn sharp at any size), an animated picture (GIF, WebP, PNG), numbered pictures as a sequence, a video (MOV, MP4…), a sound (WAV, AIFF, MP3, M4A…) or a 3D model. A Photoshop document can come in as a composition of its layers instead (`as`): each layer its own footage, its groups compositions, their blend modes, opacity, visibility and clipping kept. What it is comes back; layer.add type footage puts footage in a composition.",
            params: const {
                &[
                    Param::opt("path", Ty::Text, "the file"),
                    Param::opt("paths", Ty::List, "several files at once, each its own footage"),
                    Param::opt("sequence", Ty::Bool, "a picture whose name ends in a number is read with the pictures beside it numbered the same way, as frames (default off)"),
                    Param::opt("fps", Ty::Number, "a sequence's frames a second (default 30)"),
                    Param::opt("name", Ty::Text, "what to call it (default: its file's name)"),
                    Param::opt("folder", Ty::Whole, "the folder item to put it in"),
                    Param::opt("as", Ty::Choice(super::layered::IMPORT_AS), "a Photoshop document: as footage of the whole picture (default), as a composition of its layers each the document's size, or each just its own size, placed where it is"),
                ]
            },
            returns: "{item, name, kind, width?, height?, fps?, duration?, hasAudio, alpha} (with `paths`: {items: […], failed: [{path, error}]}; as a composition: {item, name, layers, notes})",
            run: Run::Edit(footage_import),
        },
        Command {
            id: "footage.set",
            label: "Interpret Footage",
            summary: "Change how footage is taken: its alpha (straight, premultiplied, or left out), the rate it plays at (conformed: its frames kept, its length changed), how many times it loops, its pixels' shape (pixel aspect), a sequence's first number.",
            params: const {
                &[
                    ITEM,
                    Param::opt("alpha", Ty::Choice(AlphaMode::IDS), "straight, premultiplied (its colours already times its alpha) or ignore (solid)"),
                    Param::opt("fps", Ty::Number, "the rate it plays at; its frames stay, so its length changes (its own rate again: the rate it was made at)"),
                    Param::opt("loops", Ty::Whole, "how many times it plays (1 to 10000)"),
                    Param::opt("pixelAspect", Ty::Number, "how wide a pixel is for how tall (1: square)"),
                    Param::opt("name", Ty::Text, "a new name"),
                ]
            },
            returns: "the footage as footage.info gives it",
            run: Run::Edit(footage_set),
        },
        Command {
            id: "footage.replace",
            label: "Replace Footage",
            summary: "Put another file in a footage item's place (a moved or missing file, or another take): every layer showing it shows the new one. Its alpha, loops and pixel aspect are kept. Or put a placeholder in its place (colour bars of its size and length), or make every layer showing it a solid of its size.",
            params: const {
                &[
                    ITEM,
                    Param::opt("path", Ty::Text, "the new file"),
                    Param::opt("sequence", Ty::Bool, "read as a numbered sequence"),
                    Param::opt("placeholder", Ty::Bool, "a placeholder in its place instead of a file"),
                    Param::opt("solid", Ty::Color, "the layers showing it made solids of this colour instead"),
                ]
            },
            returns: "the footage as footage.info gives it (with `solid`: {layers})",
            run: Run::Edit(footage_replace),
        },
        Command {
            id: "footage.reload",
            label: "Reload Footage",
            summary: "Read footage's files again (they changed on disk): what is kept of them in memory is let go and their size and length read anew, how they are taken kept. Without an item, every footage item; with `changed`, only those whose files changed since they were read.",
            params: const { &[Param::opt("item", Ty::Comp, "the footage (default: all of it)"), Param::opt("changed", Ty::Bool, "only those whose files changed")] },
            returns: "{reloaded: [names], failed: [{name, error}]}",
            run: Run::Edit(footage_reload),
        },
        Command {
            id: "footage.info",
            label: "Footage Info",
            summary: "What a footage item is (its file, kind, size, rate, length, sound, alpha, loops) and whether its file is there; without an item, every footage item and how much is kept in memory.",
            params: const { &[Param::opt("item", Ty::Comp, "the footage (default: all of them)")] },
            returns: "{item, name, path, kind, …, missing?} or {footage: […], kept: {frames, bytes, budget, readers, sounds}}",
            run: Run::Look(footage_info),
        },
    ]
}

fn describe(id: u64, name: &str, f: &Footage) -> Json {
    let mut o = Map::new();
    o.insert("item", id as f64);
    o.insert("name", name);
    o.insert("path", f.path.as_str());
    o.insert("kind", f.kind.id());
    if f.width > 0 {
        o.insert("width", f.width as f64);
        o.insert("height", f.height as f64);
    }
    if f.pixel_aspect != 1.0 {
        o.insert("pixelAspect", f.pixel_aspect);
    }
    if matches!(f.kind, FootageKind::Video | FootageKind::Sequence | FootageKind::Placeholder) {
        o.insert("fps", fps_json(f.frame_rate));
        if let Some(r) = f.native_rate {
            o.insert("madeAt", fps_json(r));
        }
        o.insert("frames", (f.duration.seconds() * f.frame_rate.as_f64()).round());
    }
    if f.duration > Tick::ZERO {
        o.insert("duration", f.duration.seconds());
    }
    if f.kind == FootageKind::Sequence {
        o.insert("start", f.start as f64);
    }
    o.insert("hasAudio", f.has_audio);
    o.insert("alpha", f.alpha.id());
    if f.loops != 1 {
        o.insert("loops", f.loops as f64);
    }
    let there = std::path::Path::new(&f.frame_path(0)).is_file();
    if !there && f.kind != FootageKind::Placeholder {
        o.insert("missing", true);
    }
    if there && veymelo_media::changed(&f.frame_path(0)) {
        o.insert("changed", true);
    }
    Json::Object(o)
}

fn rate_of(a: &Args) -> Result<Option<veymelo_time::FrameRate>> {
    match a.num("fps") {
        None => Ok(None),
        Some(v) if v > 0.0 && v <= 999.0 => Ok(veymelo_time::FrameRate::from_f64(v)),
        Some(v) => Err(Error::params(format!("`fps` is from above 0 to 999, not {v}"))),
    }
}

fn footage_import(cx: &mut Ctx, a: &Args) -> Result<Json> {
    if let Some(list) = a.json("paths") {
        if a.has("path") || a.has("name") {
            return Err(Error::params("give `path` (and `name`) for one file, or `paths` for several"));
        }
        let paths: Vec<String> = list.as_array().map(|v| v.iter().filter_map(|p| p.as_str().map(str::to_string)).collect()).unwrap_or_default();
        if paths.is_empty() {
            return Err(Error::params("`paths` is a list of files"));
        }
        let (mut items, mut failed) = (Vec::new(), Vec::new());
        for p in paths {
            let mut one = Map::new();
            one.insert("path", p.as_str());
            for k in ["sequence", "fps", "folder", "as"] {
                if let Some(v) = a.json(k) {
                    one.insert(k, v.clone());
                }
            }
            match super::call(cx, "footage.import", Json::Object(one)) {
                Ok(made) => items.push(made),
                Err(e) => failed.push(Json::object([("path", Json::from(p)), ("error", Json::from(e.message))])),
            }
        }
        cx.label = Some(format!("Import {} Files", items.len()));
        return Ok(Json::object([("items", Json::Array(items)), ("failed", Json::Array(failed))]));
    }
    let path = a.text("path").ok_or_else(|| Error::params("give `path`, the file (or `paths`, several)"))?;
    if !veymelo_media::importable(path) {
        return Err(Error::params(format!("{path}: not a kind of file footage is made from (pictures: {}; video: {}; sound: {}; 3D models: {})", veymelo_media::PICTURES.join(", "), veymelo_media::VIDEOS.join(", "), veymelo_media::SOUNDS.join(", "), veymelo_media::MODELS.join(", "))));
    }
    let folder = match a.whole("folder") {
        Some(id) if matches!(cx.project.item(id).map(|i| &i.kind), Some(ItemKind::Folder)) => Some(id),
        Some(id) => return Err(Error::not_found(format!("there is no folder item {id}"))),
        None => None,
    };
    let how = a.text("as").unwrap_or("footage");
    if how != "footage" {
        if !veymelo_media::is_layered(path) {
            return Err(Error::params(format!("{path}: only a Photoshop document (.psd, .psb) comes in as a composition of its layers")));
        }
        return super::layered::import(cx, path, how == "compositionLayerSizes", a.text("name"), folder);
    }
    let mut f = veymelo_media::probe(path, a.bool("sequence").unwrap_or(false)).map_err(Error::not_found)?;
    if let Some(r) = rate_of(a)? {
        if f.kind != FootageKind::Sequence {
            return Err(Error::params("`fps` here is for sequences; footage.set conforms other footage"));
        }
        let frames = (f.duration.seconds() * f.frame_rate.as_f64()).round() as i64;
        f.frame_rate = r;
        f.duration = r.tick_of(frames);
    }
    let file = std::path::Path::new(&f.path).file_name().map_or_else(|| f.path.clone(), |n| n.to_string_lossy().replace("###", "").replace('#', ""));
    let name = a.text("name").map_or(file, str::to_string);
    let id = cx.project.add(&name, folder, ItemKind::Footage(f.clone()));
    cx.label = Some(format!("Import {name}"));
    Ok(describe(id, &name, &f))
}

/// The footage `item` names, to change.
fn footage_mut<'p>(cx: &'p mut Ctx, a: &Args) -> Result<(u64, &'p mut veymelo_project::Item)> {
    let id = find_footage(cx.project, a.need("item"))?;
    Ok((id, cx.project.item_mut(id).expect("found")))
}

fn footage_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let rate = rate_of(a)?;
    let (id, item) = footage_mut(cx, a)?;
    let name = a.text("name").map(str::to_string);
    let ItemKind::Footage(f) = &mut item.kind else { unreachable!("found footage") };
    if let Some(alpha) = a.text("alpha").and_then(AlphaMode::from_id) {
        f.alpha = alpha;
    }
    if let Some(r) = rate {
        if !matches!(f.kind, FootageKind::Video | FootageKind::Sequence) {
            return Err(Error::params(format!("{} has no frames to play at another rate", f.kind.id())));
        }
        let own = f.native_rate.unwrap_or(f.frame_rate);
        let frames = (f.duration.seconds() * f.frame_rate.as_f64()).round() as i64;
        f.frame_rate = r;
        f.native_rate = (r != own).then_some(own);
        f.duration = r.tick_of(frames);
    }
    if let Some(n) = a.whole("loops") {
        if !(1..=10_000).contains(&n) {
            return Err(Error::params(format!("`loops` is from 1 to 10000, not {n}")));
        }
        f.loops = n as u32;
    }
    if let Some(par) = a.num("pixelAspect") {
        if !(0.1..=10.0).contains(&par) {
            return Err(Error::params(format!("`pixelAspect` is from 0.1 to 10, not {par}")));
        }
        f.pixel_aspect = par;
    }
    let f = f.clone();
    if let Some(n) = name {
        item.name = n;
    }
    let name = item.name.clone();
    cx.label = Some(format!("Interpret {name}"));
    Ok(describe(id, &name, &f))
}

fn footage_replace(cx: &mut Ctx, a: &Args) -> Result<Json> {
    match (a.has("path"), a.bool("placeholder") == Some(true), a.color("solid")) {
        (false, true, None) => {
            let (id, item) = footage_mut(cx, a)?;
            let ItemKind::Footage(f) = &mut item.kind else { unreachable!("found footage") };
            veymelo_media::forget(Some(&f.path));
            f.kind = FootageKind::Placeholder;
            let (name, f) = (item.name.clone(), f.clone());
            cx.label = Some(format!("Replace {name} with Placeholder"));
            return Ok(describe(id, &name, &f));
        }
        (false, false, Some(color)) => {
            let id = find_footage(cx.project, a.need("item"))?;
            let ItemKind::Footage(f) = cx.project.item(id).expect("found").kind.clone() else { unreachable!("found footage") };
            let (w, h) = (((f.width as f64 * f.pixel_aspect).round() as u32).max(1), f.height.max(1));
            let comps: Vec<u64> = cx.project.items.iter().filter(|i| i.as_comp().is_some()).map(|i| i.id).collect();
            let mut layers = 0;
            for c in comps {
                for l in cx.project.comp_mut(c).expect("a composition").layers.iter_mut().filter(|l| l.source == veymelo_project::Source::Footage(id)) {
                    l.source = veymelo_project::Source::Solid { color, width: w, height: h };
                    layers += 1;
                }
            }
            cx.label = Some("Replace with Solid".into());
            return Ok(Json::object([("layers", Json::Number(layers as f64))]));
        }
        (true, false, None) => {}
        _ => return Err(Error::params("give one of `path`, `placeholder` or `solid`")),
    }
    let path = a.text("path").unwrap_or("");
    let mut new = veymelo_media::probe(path, a.bool("sequence").unwrap_or(false)).map_err(Error::not_found)?;
    let (id, item) = footage_mut(cx, a)?;
    let ItemKind::Footage(old) = &mut item.kind else { unreachable!("found footage") };
    veymelo_media::forget(Some(&old.path));
    (new.loops, new.pixel_aspect) = (old.loops, old.pixel_aspect);
    if new.alpha != AlphaMode::Ignore || old.alpha == AlphaMode::Ignore {
        new.alpha = old.alpha;
    }
    *old = new.clone();
    let name = item.name.clone();
    cx.label = Some(format!("Replace {name}"));
    Ok(describe(id, &name, &new))
}

fn footage_info(l: &Look, a: &Args) -> Result<Json> {
    if let Some(j) = a.json("item") {
        let id = find_footage(l.project, j)?;
        let item = l.project.item(id).expect("found");
        let ItemKind::Footage(f) = &item.kind else { unreachable!("found footage") };
        return Ok(describe(id, &item.name, f));
    }
    let all: Vec<Json> = l.project.items.iter().filter_map(|i| if let ItemKind::Footage(f) = &i.kind { Some(describe(i.id, &i.name, f)) } else { None }).collect();
    let s = veymelo_media::stats();
    let kept = Json::object([("frames", s.frames as f64), ("bytes", s.bytes as f64), ("budget", s.budget as f64), ("readers", s.readers as f64), ("sounds", s.sounds as f64)]);
    Ok(Json::object([("footage", Json::Array(all)), ("kept", kept)]))
}

fn footage_reload(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let ids: Vec<u64> = match a.json("item") {
        Some(j) => vec![find_footage(cx.project, j)?],
        None => cx.project.items.iter().filter(|i| matches!(i.kind, ItemKind::Footage(_))).map(|i| i.id).collect(),
    };
    let only_changed = a.bool("changed") == Some(true);
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for id in ids {
        let item = cx.project.item_mut(id).expect("found");
        let ItemKind::Footage(f) = &mut item.kind else { continue };
        if f.kind == FootageKind::Placeholder || (only_changed && !veymelo_media::changed(&f.frame_path(0))) {
            continue;
        }
        veymelo_media::forget(Some(&f.path));
        let path = if f.kind == FootageKind::Sequence { f.frame_path(0) } else { f.path.clone() };
        match veymelo_media::probe(&path, f.kind == FootageKind::Sequence) {
            Ok(new) => {
                let conformed = f.native_rate.is_some();
                (f.width, f.height, f.has_audio) = (new.width, new.height, new.has_audio);
                if !conformed {
                    (f.frame_rate, f.duration) = (new.frame_rate, new.duration);
                }
                done.push(Json::from(item.name.as_str()));
            }
            Err(e) => failed.push(Json::object([("name", Json::from(item.name.as_str())), ("error", Json::from(e))])),
        }
    }
    cx.label = Some("Reload Footage".into());
    Ok(Json::object([("reloaded", Json::Array(done)), ("failed", Json::Array(failed))]))
}

fn scene_edits(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let (cid, lid) = layer_ids(cx.project, cx.view, a)?;
    let comp = cx.project.comp(cid).expect("found");
    let layer = comp.layer(lid).expect("found").clone();
    let half = Tick(comp.frame_rate.frame_duration().0 / 2);
    let veymelo_project::Source::Footage(item) = layer.source else { return Err(Error::refused(format!("\"{}\" is a {} layer; scene edits are found in footage", layer.name, layer.source.id()))) };
    let Some(ItemKind::Footage(f)) = cx.project.item(item).map(|i| i.kind.clone()) else { return Err(Error::not_found(format!("its footage (item {item}) is not in the project"))) };
    if !matches!(f.kind, FootageKind::Video | FootageKind::Sequence) {
        return Err(Error::refused(format!("\"{}\" shows a {}, with no shots to cut between", layer.name, f.kind.id())));
    }
    if layer.time_remap {
        return Err(Error::refused(format!("\"{}\" has its time remapped; turn that off to find its cuts", layer.name)));
    }
    let sensitivity = a.num("sensitivity").unwrap_or(50.0);
    if !(0.0..=100.0).contains(&sensitivity) {
        return Err(Error::params(format!("`sensitivity` is from 0 to 100, not {sensitivity}")));
    }
    let cuts = veymelo_media::scene_cuts(&f, sensitivity, &mut |_, _| true).map_err(Error::refused)?;
    // where each falls in the composition, every time the footage plays, while the layer is seen
    let (rate, length) = (f.frame_rate.as_f64(), f.duration.seconds());
    let mut found: Vec<(Tick, Tick, &veymelo_media::Cut)> = Vec::new();
    for k in 0..f.loops.max(1) {
        for c in &cuts {
            let own = Tick::from_seconds(c.frame as f64 / rate + k as f64 * length);
            let t = layer.comp_time(own);
            if t > layer.in_point + half && t < layer.out_point - half {
                found.push((t, own, c));
            }
        }
    }
    found.sort_by_key(|x| x.0);
    let mut out = Map::new();
    out.insert("cuts", Json::array(found.iter().map(|(t, _, c)| Json::object([("time", Json::Number(t.seconds())), ("frame", Json::Number(c.frame as f64)), ("strength", Json::Number(c.strength))]))));
    let make = a.text("make").unwrap_or("markers");
    match make {
        "markers" => {
            let l = cx.project.comp_mut(cid).expect("found").layer_mut(lid).expect("found");
            for (_, own, _) in &found {
                if !l.markers.iter().any(|m| (m.time - *own).abs() < half) {
                    l.markers.push(veymelo_project::Marker { time: *own, comment: "Scene cut".into(), ..Default::default() });
                }
            }
            l.markers.sort_by_key(|m| m.time);
        }
        "split" | "splitPrecompose" => {
            // the last cut first, so the layer itself keeps the first shot; later shots above
            let mut pieces = vec![lid];
            for (t, _, _) in found.iter().rev() {
                let made = call(cx, "layer.split", Json::object([("comp", Json::Number(cid as f64)), ("layer", Json::Number(lid as f64)), ("time", Json::Number(t.seconds()))]))?;
                pieces.push(made.get("layer").and_then(Json::as_u64).expect("a new layer"));
            }
            pieces[1..].reverse();
            if make == "splitPrecompose" {
                let mut nests = Vec::new();
                let mut comps = Vec::new();
                for (k, id) in pieces.iter().enumerate() {
                    let (inn, outp) = {
                        let l = cx.project.comp(cid).expect("found").layer(*id).expect("found");
                        (l.in_point, l.out_point)
                    };
                    let name = format!("{} shot {}", layer.name, k + 1);
                    let made = call(cx, "layer.precompose", Json::object([("comp", Json::Number(cid as f64)), ("layers", Json::array([Json::Number(*id as f64)])), ("name", Json::from(name))]))?;
                    let nest = made.get("layer").and_then(Json::as_u64).expect("a layer");
                    let l = cx.project.comp_mut(cid).expect("found").layer_mut(nest).expect("found");
                    (l.in_point, l.out_point) = (inn, outp);
                    nests.push(Json::Number(nest as f64));
                    comps.push(made.get("comp").cloned().unwrap_or(Json::Null));
                }
                out.insert("layers", Json::Array(nests));
                out.insert("comps", Json::Array(comps));
            } else {
                out.insert("layers", Json::array(pieces.iter().map(|id| Json::Number(*id as f64))));
            }
        }
        _ => {}
    }
    cx.label = Some(format!("Scene Edit Detection on {}", layer.name));
    Ok(Json::Object(out))
}
