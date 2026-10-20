//! Named ways to make things, kept for every project: render settings and output module
//! templates (some come with the program), composition presets; and more of the render queue:
//! an item's further output modules, copies of items, items moved in the queue.
//!
//! Saved templates and presets are kept in Templates.json in the program's Application Support
//! folder (or `$VEYMELO_TEMPLATES`, or the `folder` given).

use std::path::PathBuf;

use veymelo_json::{Map, Value as Json};
use veymelo_project::queue::{Format, OutputModule, RenderItem, Status};
use veymelo_time::{FrameRate, Tick};

use super::{COMP, Command, Run, comp_id};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

const FOLDER: Param = Param::opt("folder", Ty::Text, "where saved templates are kept (default: $VEYMELO_TEMPLATES, or the program's Application Support folder)");
const KIND: Param = Param::need("kind", Ty::Choice(&["settings", "output", "comp"]), "render settings, an output module, or a composition preset");

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "queue.templates",
            label: "Templates",
            summary: "The render settings templates, output module templates and composition presets there are: those that come with the program and those saved.",
            params: const { &[FOLDER] },
            returns: "{settings: [{name, saved?, …}], output: […], comp: […]}",
            run: Run::Look(list),
        },
        Command {
            id: "queue.saveTemplate",
            label: "Save Template",
            summary: "Keep a render queue item's render settings or (first) output module as a template under a name, for queue.add and queue.set to use (`settingsTemplate`, `outputTemplate`); or a composition's size, rate, length and pixel shape as a preset for comp.new (`preset`).",
            params: const {
                &[
                    KIND,
                    Param::need("name", Ty::Text, "what to call it"),
                    Param::opt("item", Ty::Whole, "the render queue item (settings, output)"),
                    COMP,
                    FOLDER,
                ]
            },
            returns: "{kind, name}",
            run: Run::Look(save),
        },
        Command {
            id: "queue.deleteTemplate",
            label: "Delete Template",
            summary: "Forget a saved template or composition preset (those that come with the program stay).",
            params: const { &[KIND, Param::need("name", Ty::Text, "its name"), FOLDER] },
            returns: "{deleted}",
            run: Run::Look(delete),
        },
        Command {
            id: "queue.output",
            label: "Output Modules",
            summary: "A render queue item's output modules, each a file made of the same frames: add one (`format`, `path`, `output` or `outputTemplate`), change one, or remove one (not the first); `index` 0 is the first.",
            params: const {
                &[
                    Param::need("item", Ty::Whole, "the item's id"),
                    Param::need("op", Ty::Choice(&["add", "set", "remove"]), "what to do"),
                    Param::opt("index", Ty::Whole, "set, remove: which (from 0)"),
                    Param::opt("format", Ty::Choice(Format::IDS), "the kind of file"),
                    Param::opt("path", Ty::Text, "where it goes"),
                    Param::opt("output", Ty::Object, "what else it is, as queue.add takes it"),
                    Param::opt("outputTemplate", Ty::Text, "start from this output module template"),
                    FOLDER,
                ]
            },
            returns: "{outputs: [{format, path, …}]}",
            run: Run::Edit(outputs),
        },
        Command {
            id: "queue.duplicate",
            label: "Duplicate Render Queue Item",
            summary: "Copy a render queue item just below it, to be made again (its files named anew, with a number, unless `path` says).",
            params: const { &[Param::need("item", Ty::Whole, "the item's id"), Param::opt("path", Ty::Text, "where the copy's (first) file goes")] },
            returns: "{item, index}",
            run: Run::Edit(duplicate),
        },
        Command {
            id: "queue.move",
            label: "Move Render Queue Item",
            summary: "Move a render queue item to another place in the queue (they are made top to bottom).",
            params: const { &[Param::need("item", Ty::Whole, "the item's id"), Param::need("to", Ty::Whole, "its place, from 1")] },
            returns: "{order: [item]}",
            run: Run::Edit(move_item),
        },
    ]
}

/// Where saved templates are.
fn file(a: &Args) -> PathBuf {
    let dir = match (a.text("folder"), std::env::var_os("VEYMELO_TEMPLATES")) {
        (Some(f), _) => PathBuf::from(f),
        (None, Some(p)) => PathBuf::from(p),
        (None, None) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Library/Application Support/Veymelo"),
    };
    dir.join("Templates.json")
}

/// What is saved: {settings: {name: …}, output: {…}, comp: {…}}.
fn saved(a: &Args) -> Result<Map> {
    let path = file(a);
    match std::fs::read_to_string(&path) {
        Ok(s) => match Json::parse(&s) {
            Ok(Json::Object(m)) => Ok(m),
            _ => Err(Error::refused(format!("{} is not a templates file", path.display()))),
        },
        Err(_) => Ok(Map::new()),
    }
}

fn write(a: &Args, all: Map) -> Result<()> {
    let path = file(a);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::refused(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(&path, Json::Object(all).to_pretty()).map_err(|e| Error::refused(format!("{}: {e}", path.display())))
}

/// The templates that come with the program, by kind.
fn built_in(kind: &str) -> Vec<(&'static str, Json)> {
    let p = |s: &str| Json::parse(s).expect("a template");
    match kind {
        "settings" => vec![
            ("Best Settings", p(r#"{"quality": "best", "resolution": 1}"#)),
            ("Draft Settings", p(r#"{"quality": "draft", "resolution": 0.5, "motionBlur": "off", "frameBlending": "off"}"#)),
            ("Current Settings", p("{}")),
            ("Work Area Only", p(r#"{"span": "workArea"}"#)),
            ("Whole Composition", p(r#"{"span": "comp"}"#)),
        ],
        "output" => vec![
            ("H.264", p(r#"{"format": "mp4"}"#)),
            ("H.264 High", p(r#"{"format": "mp4", "quality": 95}"#)),
            ("HEVC", p(r#"{"format": "hevc"}"#)),
            ("ProRes 422 HQ", p(r#"{"format": "prores", "prores": "hq"}"#)),
            ("ProRes 4444 with Alpha", p(r#"{"format": "prores", "prores": "4444", "channels": "rgba"}"#)),
            ("PNG Sequence with Alpha", p(r#"{"format": "pngSequence", "channels": "rgba"}"#)),
            ("EXR Sequence", p(r#"{"format": "exrSequence", "channels": "rgba"}"#)),
            ("Photoshop Sequence", p(r#"{"format": "psdSequence"}"#)),
            ("Alpha Only", p(r#"{"format": "pngSequence", "channels": "alpha"}"#)),
            ("Animated GIF", p(r#"{"format": "gif"}"#)),
            ("WAV", p(r#"{"format": "wav"}"#)),
        ],
        _ => vec![
            ("HD 1080 30", p(r#"{"width": 1920, "height": 1080, "fps": 30, "duration": 10}"#)),
            ("HD 1080 29.97", p(r#"{"width": 1920, "height": 1080, "fps": "29.97", "duration": 10}"#)),
            ("HD 1080 25", p(r#"{"width": 1920, "height": 1080, "fps": 25, "duration": 10}"#)),
            ("HD 1080 24", p(r#"{"width": 1920, "height": 1080, "fps": 24, "duration": 10}"#)),
            ("HD 1080 60", p(r#"{"width": 1920, "height": 1080, "fps": 60, "duration": 10}"#)),
            ("HD 720 30", p(r#"{"width": 1280, "height": 720, "fps": 30, "duration": 10}"#)),
            ("UHD 4K 30", p(r#"{"width": 3840, "height": 2160, "fps": 30, "duration": 10}"#)),
            ("Vertical 1080 × 1920 30", p(r#"{"width": 1080, "height": 1920, "fps": 30, "duration": 15}"#)),
            ("Square 1080 30", p(r#"{"width": 1080, "height": 1080, "fps": 30, "duration": 15}"#)),
            ("Portrait 1080 × 1350 30", p(r#"{"width": 1080, "height": 1350, "fps": 30, "duration": 15}"#)),
        ],
    }
}

/// A template by kind and name: a saved one first, then one that comes with the program.
pub(super) fn template(a: &Args, kind: &str, name: &str) -> Result<Json> {
    let all = saved(a)?;
    if let Some(t) = all.get(kind).and_then(Json::as_object).and_then(|m| m.get(name)) {
        return Ok(t.clone());
    }
    built_in(kind).into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, t)| t).ok_or_else(|| {
        let mut names: Vec<String> = built_in(kind).iter().map(|(n, _)| n.to_string()).collect();
        names.extend(all.get(kind).and_then(Json::as_object).map(|m| m.keys().map(|k| k.to_string()).collect::<Vec<_>>()).unwrap_or_default());
        Error::not_found(format!("there is no {kind} template \"{name}\" (there are: {})", names.join(", ")))
    })
}

/// A composition preset: width, height, rate, length, pixel shape.
pub(super) struct CompPreset {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub duration: Tick,
    pub pixel_aspect: f64,
}

pub(super) fn comp_preset(a: &Args, name: &str) -> Result<CompPreset> {
    let t = template(a, "comp", name)?;
    let num = |k: &str, d: f64| t.get(k).and_then(Json::as_f64).unwrap_or(d);
    let fps = match t.get("fps") {
        Some(f) => veymelo_project::fps_from_json(f).ok_or_else(|| Error::refused(format!("the preset \"{name}\" has a frame rate that is not one")))?,
        None => FrameRate::FPS_30,
    };
    Ok(CompPreset { width: num("width", 1920.0) as u32, height: num("height", 1080.0) as u32, fps, duration: Tick::from_seconds(num("duration", 10.0)), pixel_aspect: num("pixelAspect", 1.0) })
}

fn list(_: &Look, a: &Args) -> Result<Json> {
    let all = saved(a)?;
    let mut out = Map::new();
    for kind in ["settings", "output", "comp"] {
        let mut rows: Vec<Json> = Vec::new();
        for (name, t) in built_in(kind) {
            let mut o = t.as_object().cloned().unwrap_or_default();
            o.insert("name", name);
            rows.push(Json::Object(o));
        }
        if let Some(m) = all.get(kind).and_then(Json::as_object) {
            for (name, t) in m.iter() {
                let mut o = t.as_object().cloned().unwrap_or_default();
                o.insert("name", Json::from(name.to_string()));
                o.insert("saved", true);
                rows.push(Json::Object(o));
            }
        }
        out.insert(kind, Json::Array(rows));
    }
    Ok(Json::Object(out))
}

fn find_item(queue: &[RenderItem], a: &Args) -> Result<usize> {
    let id = a.whole("item").unwrap_or(0);
    queue.iter().position(|q| q.id == id).ok_or_else(|| Error::not_found(format!("there is no render queue item {id}")))
}

fn save(l: &Look, a: &Args) -> Result<Json> {
    let kind = a.text("kind").unwrap_or("settings");
    let name = a.text("name").unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err(Error::params("a template needs a name"));
    }
    let what = match kind {
        "comp" => {
            let c = l.project.comp(comp_id(l.project, l.view, a)?).expect("found");
            let mut o = Map::new();
            o.insert("width", c.width as f64);
            o.insert("height", c.height as f64);
            o.insert("fps", veymelo_project::fps_json(c.frame_rate));
            o.insert("duration", c.duration.seconds());
            if c.pixel_aspect != 1.0 {
                o.insert("pixelAspect", c.pixel_aspect);
            }
            Json::Object(o)
        }
        _ => {
            let item = &l.project.queue[find_item(&l.project.queue, a)?];
            let j = item.to_json();
            match kind {
                "settings" => j.get("settings").cloned().unwrap_or_else(|| Json::Object(Map::new())),
                _ => {
                    // the output module without where its file goes
                    let mut o = j.get("output").and_then(Json::as_object).cloned().unwrap_or_default();
                    o.remove("path");
                    Json::Object(o)
                }
            }
        }
    };
    let mut all = saved(a)?;
    let mut list = all.get(kind).and_then(Json::as_object).cloned().unwrap_or_default();
    list.insert(&name, what);
    all.insert(kind, Json::Object(list));
    write(a, all)?;
    Ok(Json::object([("kind", Json::from(kind)), ("name", Json::from(name))]))
}

fn delete(_: &Look, a: &Args) -> Result<Json> {
    let kind = a.text("kind").unwrap_or("settings");
    let name = a.text("name").unwrap_or("");
    let mut all = saved(a)?;
    let mut list = all.get(kind).and_then(Json::as_object).cloned().unwrap_or_default();
    if list.remove(name).is_none() {
        let built = built_in(kind).iter().any(|(n, _)| n.eq_ignore_ascii_case(name));
        return Err(if built { Error::refused(format!("\"{name}\" comes with the program and stays")) } else { Error::not_found(format!("there is no saved {kind} template \"{name}\"")) });
    }
    all.insert(kind, Json::Object(list));
    write(a, all)?;
    Ok(Json::object([("deleted", Json::from(name))]))
}

/// An output module from the parameters: a template's, `base`'s, then what is given.
fn module_from(a: &Args, base: &OutputModule) -> Result<OutputModule> {
    let mut o = base.to_json().as_object().cloned().unwrap_or_default();
    if let Some(t) = a.text("outputTemplate") {
        for (k, v) in template(a, "output", t)?.as_object().cloned().unwrap_or_default().iter() {
            o.insert(k, v.clone());
        }
    }
    if let Some(Json::Object(m)) = a.json("output") {
        for (k, v) in m.iter() {
            o.insert(k, v.clone());
        }
    }
    if let Some(f) = a.text("format") {
        o.insert("format", f);
    }
    if let Some(p) = a.text("path") {
        o.insert("path", p);
    }
    OutputModule::from_json(&Json::Object(o), "output").map_err(|e| Error::params(e.to_string()))
}

fn outputs_json(item: &RenderItem) -> Json {
    Json::object([("outputs", Json::array(std::iter::once(&item.output).chain(&item.extra).map(OutputModule::to_json)))])
}

fn outputs(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let i = find_item(&cx.project.queue, a)?;
    let mut item = cx.project.queue[i].clone();
    let n = 1 + item.extra.len();
    let index = a.whole("index").map(|k| k as usize);
    match a.text("op").unwrap_or("add") {
        "add" => {
            if !a.has("path") {
                return Err(Error::params("a new output module needs `path`, where its file goes"));
            }
            let m = module_from(a, &OutputModule::default())?;
            item.extra.push(m);
        }
        "set" => {
            let k = index.filter(|k| *k < n).ok_or_else(|| Error::params(format!("give `index`, from 0 to {}", n - 1)))?;
            let base = if k == 0 { item.output.clone() } else { item.extra[k - 1].clone() };
            let m = module_from(a, &base)?;
            if k == 0 {
                item.output = m;
            } else {
                item.extra[k - 1] = m;
            }
        }
        _ => {
            let k = index.filter(|k| *k >= 1 && *k < n).ok_or_else(|| Error::params(if n == 1 { "it has only its first output module, which stays".to_string() } else { format!("give `index`, from 1 to {} (the first stays)", n - 1) }))?;
            item.extra.remove(k - 1);
        }
    }
    item.status = Status::Queued;
    item.message.clear();
    let out = outputs_json(&item);
    cx.project.queue[i] = item;
    cx.label = Some("Output Modules".into());
    Ok(out)
}

/// A path with a number before its ending, so a copy's file is its own: "shot.mp4" → "shot 2.mp4".
fn numbered(path: &str, taken: &[String]) -> String {
    let (stem, ext) = match path.rsplit_once('.') {
        Some((s, e)) if !e.contains('/') => (s.to_string(), format!(".{e}")),
        _ => (path.to_string(), String::new()),
    };
    (2..).map(|k| format!("{stem} {k}{ext}")).find(|p| !taken.contains(p)).expect("a free name")
}

fn duplicate(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let i = find_item(&cx.project.queue, a)?;
    let mut copy = cx.project.queue[i].clone();
    copy.id = cx.project.alloc();
    copy.status = Status::Queued;
    copy.message.clear();
    let taken: Vec<String> = cx.project.queue.iter().flat_map(|q| std::iter::once(&q.output).chain(&q.extra).map(|m| m.path.clone())).collect();
    copy.output.path = match a.text("path") {
        Some(p) => p.to_string(),
        None => numbered(&copy.output.path, &taken),
    };
    for m in &mut copy.extra {
        m.path = numbered(&m.path, &taken);
    }
    let id = copy.id;
    cx.project.queue.insert(i + 1, copy);
    cx.label = Some("Duplicate Render Queue Item".into());
    Ok(Json::object([("item", Json::Number(id as f64)), ("index", Json::Number((i + 2) as f64))]))
}

fn move_item(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let i = find_item(&cx.project.queue, a)?;
    let item = cx.project.queue.remove(i);
    let to = (a.whole("to").unwrap_or(1).max(1) as usize - 1).min(cx.project.queue.len());
    cx.project.queue.insert(to, item);
    cx.label = Some("Move Render Queue Item".into());
    Ok(Json::object([("order", Json::array(cx.project.queue.iter().map(|q| Json::Number(q.id as f64))))]))
}
