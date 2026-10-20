//! Commands about making compositions into files: the render queue (items kept in the project,
//! each a composition, how it is drawn and what file it becomes) and making a file at once.

use veymelo_json::{Map, Value as Json};
use veymelo_project::queue::{After, Format, RenderItem, Status};

use super::{COMP, Command, Run, comp_id};
use crate::{Args, Ctx, Error, Look, Param, Result, Ty};

const SETTINGS: Param = Param::opt(
    "settings",
    Ty::Object,
    "how it is drawn: {quality: current|best|draft|wireframe, resolution (1, 0.5…), span: workArea|comp|custom, start, end, frameRate, motionBlur / frameBlending / effects / solo / guides: current|on|off, depth: 8|16|32, proxies: none|current|all|comps}",
);
const SETTINGS_TEMPLATE: Param = Param::opt("settingsTemplate", Ty::Text, "start the settings from this template (queue.templates lists them)");
const OUTPUT_TEMPLATE: Param = Param::opt("outputTemplate", Ty::Text, "start the output from this template");
const TEMPLATES: Param = Param::opt("folder", Ty::Text, "where saved templates are kept (default: $VEYMELO_TEMPLATES, or the program's Application Support folder)");
const AFTER: Param = Param::opt("after", Ty::Choice(After::IDS), "what the file does once made: none, import (comes in as footage), importReplace (…and takes the composition's place where it is used), setProxy (…as its proxy)");
const OUTPUT: Param = Param::opt(
    "output",
    Ty::Object,
    "what file it becomes besides format and path: {channels: rgb|rgba|alpha, premultiplied, quality 0–100, bitrate (kilobits a second), keyframes, prores: proxy|lt|422|hq|4444|4444xq, crop: [top, left, bottom, right], resize: [w, h], gifLoop, audio}",
);

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "queue.add",
            label: "Add to Render Queue",
            summary: "Put a composition in the render queue: what file it becomes (format and path; a sequence's frame number goes where # are) and how it is drawn. queue.render makes it.",
            params: const { &[COMP, Param::need("path", Ty::Text, "where the file goes, like renders/shot.mp4 or frames/shot_####.png"), Param::opt("format", Ty::Choice(Format::IDS), "the kind of file (default mp4: H.264)"), SETTINGS, OUTPUT, SETTINGS_TEMPLATE, OUTPUT_TEMPLATE, TEMPLATES, AFTER] },
            returns: "{item, index}",
            run: Run::Edit(queue_add),
        },
        Command {
            id: "queue.set",
            label: "Change Render Queue Item",
            summary: "Change a render queue item: its path, format, settings or output (only what is given), or switch it off (enabled).",
            params: const { &[Param::need("item", Ty::Whole, "the item's id"), Param::opt("path", Ty::Text, "where the file goes"), Param::opt("format", Ty::Choice(Format::IDS), "the kind of file"), Param::opt("enabled", Ty::Bool, "made by queue.render"), SETTINGS, OUTPUT, SETTINGS_TEMPLATE, OUTPUT_TEMPLATE, TEMPLATES, AFTER] },
            returns: "{item}",
            run: Run::Edit(queue_set),
        },
        Command {
            id: "queue.remove",
            label: "Remove from Render Queue",
            summary: "Take an item out of the render queue.",
            params: const { &[Param::need("item", Ty::Whole, "the item's id")] },
            returns: "{removed}",
            run: Run::Edit(queue_remove),
        },
        Command {
            id: "queue.list",
            label: "Render Queue",
            summary: "The render queue: each item's composition, file, settings and how far it has come.",
            params: &[],
            returns: "[{item, comp, enabled, status, message?, settings?, output}]",
            run: Run::Look(queue_list),
        },
        Command {
            id: "queue.render",
            label: "Render",
            summary: "Make the render queue's items into their files: one, or every one switched on and not yet done. Each item is marked done or failed with what happened, then does what it says after (comes in as footage, takes its composition's place, becomes its proxy).",
            params: const { &[Param::opt("item", Ty::Whole, "only this item"), Param::opt("again", Ty::Bool, "make items already done too")] },
            returns: "{made: [{item, files, frames, size, notes?, footage?, replaced?, proxy?}], failed: [{item, error}]}",
            run: Run::Edit(queue_render),
        },
        Command {
            id: "render.export",
            label: "Export",
            summary: "Make a composition into a file at once (without keeping it in the render queue): mp4 (H.264), hevc, prores, pngSequence, jpegSequence, tiffSequence or gif, drawn and written as the settings and output say.",
            params: const { &[COMP, Param::need("path", Ty::Text, "where the file goes"), Param::opt("format", Ty::Choice(Format::IDS), "the kind of file (default mp4)"), SETTINGS, OUTPUT] },
            returns: "{files, frames, size, notes?}",
            run: Run::Look(render_export),
        },
    ]
}

/// An item from the parameters (`base` the one being changed), checked as the project file
/// checks it.
pub(super) fn item_from(a: &Args, base: &RenderItem) -> Result<RenderItem> {
    let mut j = base.to_json();
    let o = j.as_object_mut().expect("an item is an object");
    // a template's first, then what is given
    let settings_template = match a.text("settingsTemplate") {
        Some(t) => super::templates::template(a, "settings", t)?.as_object().cloned().unwrap_or_default(),
        None => Map::new(),
    };
    let given = match a.json("settings") {
        Some(Json::Object(s)) => s.clone(),
        _ => Map::new(),
    };
    if !settings_template.is_empty() || !given.is_empty() {
        let mut merged = o.get("settings").and_then(Json::as_object).cloned().unwrap_or_default();
        for (k, v) in settings_template.iter().chain(given.iter()) {
            merged.insert(k, v.clone());
        }
        o.insert("settings", Json::Object(merged));
    }
    let mut out = o.get("output").and_then(Json::as_object).cloned().unwrap_or_default();
    if let Some(t) = a.text("outputTemplate") {
        for (k, v) in super::templates::template(a, "output", t)?.as_object().cloned().unwrap_or_default().iter() {
            out.insert(k, v.clone());
        }
    }
    if let Some(Json::Object(m)) = a.json("output") {
        for (k, v) in m.iter() {
            if k == "format" || k == "path" {
                return Err(Error::params(format!("give `{k}` on its own, not inside `output`")));
            }
            out.insert(k, v.clone());
        }
    }
    if let Some(f) = a.text("format") {
        out.insert("format", f);
    }
    if let Some(p) = a.text("path") {
        out.insert("path", p);
    }
    o.insert("output", Json::Object(out));
    if let Some(e) = a.bool("enabled") {
        o.insert("enabled", e);
    }
    if let Some(e) = a.text("after") {
        o.insert("after", e);
    }
    RenderItem::from_json(&j, "").map_err(|e| Error::params(e.to_string().trim_start_matches(": ").to_string()))
}

fn queue_add(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let comp = comp_id(cx.project, cx.view, a)?;
    let id = cx.project.alloc();
    let item = item_from(a, &RenderItem::new(id, comp))?;
    cx.project.queue.push(item);
    cx.label = Some("Add to Render Queue".into());
    Ok(Json::object([("item", Json::Number(id as f64)), ("index", Json::Number(cx.project.queue.len() as f64))]))
}

fn find(queue: &[RenderItem], a: &Args) -> Result<usize> {
    let id = a.whole("item").unwrap_or(0);
    queue.iter().position(|q| q.id == id).ok_or_else(|| {
        let have: Vec<String> = queue.iter().map(|q| q.id.to_string()).collect();
        Error::not_found(if have.is_empty() { format!("there is no render queue item {id} (the queue is empty)") } else { format!("there is no render queue item {id} (there is {})", have.join(", ")) })
    })
}

fn queue_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let i = find(&cx.project.queue, a)?;
    let mut item = item_from(a, &cx.project.queue[i])?;
    // changed: to be made again
    item.status = Status::Queued;
    item.message.clear();
    cx.project.queue[i] = item;
    cx.label = Some("Change Render Queue Item".into());
    Ok(Json::object([("item", cx.project.queue[i].to_json())]))
}

fn queue_remove(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let i = find(&cx.project.queue, a)?;
    let id = cx.project.queue.remove(i).id;
    cx.label = Some("Remove from Render Queue".into());
    Ok(Json::object([("removed", id as f64)]))
}

fn queue_list(l: &Look, _: &Args) -> Result<Json> {
    Ok(Json::array(l.project.queue.iter().map(|q| {
        let mut j = q.to_json();
        if let Some(o) = j.as_object_mut() {
            o.insert("item", q.id as f64);
            o.remove("id");
            o.insert("status", q.status.id());
            o.insert("enabled", q.enabled);
        }
        j
    })))
}

/// What a made item says.
pub(super) fn made_json(m: &veymelo_export::Made) -> Map {
    let mut o = Map::new();
    o.insert("files", Json::array(m.files.iter().map(|f| Json::from(f.as_str()))));
    o.insert("frames", m.frames as f64);
    o.insert("size", Json::array([Json::Number(m.size[0] as f64), Json::Number(m.size[1] as f64)]));
    if !m.notes.is_empty() {
        o.insert("notes", Json::array(m.notes.iter().map(|n| Json::from(n.as_str()))));
    }
    o
}

fn queue_render(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let again = a.bool("again").unwrap_or(false);
    let only = if a.has("item") { Some(cx.project.queue[find(&cx.project.queue, a)?].id) } else { None };
    let todo: Vec<usize> = (0..cx.project.queue.len())
        .filter(|&i| {
            let q = &cx.project.queue[i];
            match only {
                Some(id) => q.id == id,
                None => q.enabled && (again || q.status != Status::Done),
            }
        })
        .collect();
    if todo.is_empty() {
        return Err(Error::refused("there is nothing in the render queue to make (queue.add puts compositions in; again: true makes done ones again)"));
    }
    let (mut made, mut failed) = (Vec::new(), Vec::new());
    for i in todo {
        let item = cx.project.queue[i].clone();
        let started = std::time::Instant::now();
        let mut told = crate::progress::Reporter::start(&comp_name(cx.project, item.comp), &item.output.path);
        let result = veymelo_export::make(cx.project, &item, &mut |done, total| {
            told.tick(done, total);
            true
        });
        drop(told);
        match result {
            Ok(m) => {
                let q = &mut cx.project.queue[i];
                q.status = Status::Done;
                q.message = format!("{} frames in {:.1} s", m.frames, started.elapsed().as_secs_f64());
                let mut o = made_json(&m);
                o.insert("item", item.id as f64);
                match super::prerender::after_made(cx.project, &item, &m.files) {
                    Ok(Some(Json::Object(next))) => next.iter().for_each(|(k, v)| {
                        o.insert(k, v.clone());
                    }),
                    Ok(_) => {}
                    Err(e) => {
                        o.insert("afterError", e.message);
                    }
                }
                made.push(Json::Object(o));
            }
            Err(e) => {
                let q = &mut cx.project.queue[i];
                q.status = Status::Failed;
                q.message = e.clone();
                failed.push(Json::object([("item", Json::Number(item.id as f64)), ("error", Json::from(e))]));
            }
        }
    }
    cx.label = Some("Render".into());
    Ok(Json::object([("made", Json::Array(made)), ("failed", Json::Array(failed))]))
}

fn render_export(l: &Look, a: &Args) -> Result<Json> {
    let comp = comp_id(l.project, l.view, a)?;
    let item = item_from(a, &RenderItem::new(0, comp))?;
    let mut told = crate::progress::Reporter::start(&comp_name(l.project, comp), &item.output.path);
    let m = veymelo_export::make(l.project, &item, &mut |done, total| {
        told.tick(done, total);
        true
    })
    .map_err(Error::refused)?;
    Ok(Json::Object(made_json(&m)))
}

/// A composition's name, for whoever watches its render.
fn comp_name(project: &veymelo_project::Project, comp: u64) -> String {
    project.item(comp).map_or_else(|| format!("composition {comp}"), |i| i.name.clone())
}
