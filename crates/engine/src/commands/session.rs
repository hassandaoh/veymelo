//! Commands about the whole session: the project file, undo and the history, batches, the
//! view, and the list of commands itself.

use std::path::PathBuf;

use veymelo_json::{Map, Value as Json};
use veymelo_project::Project;

use super::{Command, Run, all, comp_id, find, find_comp, find_layer};
use crate::{Args, Ctx, Error, Look, Param, Result, Session, Ty, params, unknown};

pub fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "project.new",
            label: "New Project",
            summary: "Start an empty project (its history starts over).",
            params: const { &[Param::opt("discard", Ty::Bool, "throw away changes not yet saved (without it, unsaved changes are kept and this is refused)")] },
            returns: "{}",
            run: Run::Session(project_new),
        },
        Command {
            id: "project.open",
            label: "Open Project",
            summary: "Open a .veymelo file (its history starts over).",
            params: const { &[Param::need("path", Ty::Text, "the file"), Param::opt("discard", Ty::Bool, "throw away changes not yet saved")] },
            returns: "{path, items}",
            run: Run::Session(project_open),
        },
        Command {
            id: "project.save",
            label: "Save Project",
            summary: "Write the project to its file, or to a new one; or `increment`: to the next numbered name beside it (Show 2.veymelo, Show 3…), worked on from there; or `copy`: a copy written elsewhere, the project still its own file.",
            params: const {
                &[
                    Param::opt("path", Ty::Text, "where to write it (default: the file it came from)"),
                    Param::opt("increment", Ty::Bool, "to the next numbered name beside its file"),
                    Param::opt("copy", Ty::Text, "write a copy here, staying with its own file"),
                ]
            },
            returns: "{path}",
            run: Run::Session(project_save),
        },
        Command {
            id: "project.info",
            label: "Project Info",
            summary: "The project's file, whether it is saved, its items and where the work is.",
            params: &[],
            returns: "{path, modified, comp, time, items: [{id, type, name, …}]}",
            run: Run::Session(project_info),
        },
        Command {
            id: "project.set",
            label: "Project Settings",
            summary: "Change how the whole project works: its bits a channel and whether colours mix in linear light.",
            params: const {
                &[
                    Param::opt("bitDepth", Ty::Choice(&["8", "16", "32"]), "bits for each colour channel: 8 and 16 round every step as such a project does; 32 is floating point, colours may go past white"),
                    Param::opt("linearBlending", Ty::Bool, "mix colours in linear light (brighter, more like light adds up) instead of as stored"),
                    Param::opt("audioRate", Ty::Whole, "samples a second of sound, 8000 to 192000"),
                    Param::opt("timeDisplay", Ty::Choice(&["timecode", "frames"]), "how times are shown"),
                    Param::opt("frameStart", Ty::Whole, "the first frame's number when counting in frames, 0 or 1"),
                ]
            },
            returns: "{bitDepth, linearBlending, audioRate, timeDisplay, frameStart}",
            run: Run::Edit(project_set),
        },
        Command {
            id: "doc.inspect",
            label: "Inspect",
            summary: "The project, one composition or one layer, written as in a .veymelo file (only what differs from the defaults).",
            params: const { &[Param::opt("comp", Ty::Comp, "just this composition"), Param::opt("layer", Ty::Layer, "just this layer (of `comp`, or the one being worked on)")] },
            returns: "the JSON",
            run: Run::Look(inspect),
        },
        Command { id: "edit.undo", label: "Undo", summary: "Go back one step.", params: &[], returns: "{undid: step}", run: Run::Session(undo) },
        Command { id: "edit.redo", label: "Redo", summary: "Go forward one step again.", params: &[], returns: "{redid: step}", run: Run::Session(redo) },
        Command {
            id: "history.list",
            label: "History",
            summary: "Every state the project has been in this session, as a tree: undoing and doing something else starts a branch, and nothing is lost.",
            params: &[],
            returns: "[{id, step, parent, current?, past?, future?}]",
            run: Run::Session(history_list),
        },
        Command {
            id: "history.goto",
            label: "Go to State",
            summary: "Go to any state of the history, on any branch.",
            params: const { &[Param::need("id", Ty::Whole, "the state, from history.list")] },
            returns: "{step}",
            run: Run::Session(history_goto),
        },
        Command {
            id: "batch",
            label: "Batch",
            summary: "Run several commands as one undo step; if one fails, none of them change anything. A string \"$2\" in a step's params is what step 2 gave back, \"$2.layer\" its `layer`.",
            params: const { &[Param::need("steps", Ty::List, "[{command, params?}, …]"), Param::opt("label", Ty::Text, "what the undo step is called")] },
            returns: "{results: [what each step gave back]}",
            run: Run::Session(batch),
        },
        Command {
            id: "view.set",
            label: "View",
            summary: "Which composition and time commands use when they are not given one.",
            params: const { &[Param::opt("comp", Ty::Comp, "the composition"), Param::opt("time", Ty::Time, "the time")] },
            returns: "{comp, time}",
            run: Run::Edit(view_set),
        },
        Command {
            id: "command.list",
            label: "Commands",
            summary: "Every command, or those whose id or summary has `search` in it.",
            params: const { &[Param::opt("search", Ty::Text, "a word to look for")] },
            returns: "[{id, summary, changes}]",
            run: Run::Look(command_list),
        },
        Command {
            id: "command.describe",
            label: "Describe Command",
            summary: "What a command does, takes (as JSON Schema) and gives back.",
            params: const { &[Param::need("command", Ty::Text, "its id")] },
            returns: "{id, summary, params, returns, changes}",
            run: Run::Look(command_describe),
        },
    ]
}

fn unsaved(s: &Session, a: &Args) -> Result<()> {
    if s.modified() && !s.project().items.is_empty() && a.bool("discard") != Some(true) {
        return Err(Error::refused(format!("the project has changes that are not saved; save them first (project.save) or give `discard: true` to {}", a.command())));
    }
    Ok(())
}

fn project_new(s: &mut Session, a: &Args) -> Result<Json> {
    unsaved(s, a)?;
    s.replace(Project::new(), None);
    Ok(Json::object::<&str, Json>([]))
}

fn project_open(s: &mut Session, a: &Args) -> Result<Json> {
    unsaved(s, a)?;
    let path = PathBuf::from(a.text("path").unwrap_or(""));
    let project = Project::open(&path, &s.catalog).map_err(Error::not_found)?;
    let items = project.items.len();
    s.replace(project, Some(path.clone()));
    Ok(Json::object([("path", Json::from(path.display().to_string())), ("items", Json::Number(items as f64))]))
}

fn project_save(s: &mut Session, a: &Args) -> Result<Json> {
    if let Some(copy) = a.text("copy") {
        let path = PathBuf::from(copy);
        s.to_save().save(&path).map_err(Error::refused)?;
        return Ok(Json::object([("copy", path.display().to_string())]));
    }
    let path = match a.text("path") {
        Some(p) => PathBuf::from(p),
        None => s.path.clone().ok_or_else(|| Error::params("the project has no file yet; give `path`"))?,
    };
    let path = if a.bool("increment") == Some(true) { next_numbered(&path) } else { path };
    s.to_save().save(&path).map_err(Error::refused)?;
    s.mark_saved(path.clone());
    s.changed();
    Ok(Json::object([("path", path.display().to_string())]))
}

/// The next numbered name beside a file not taken yet: "Show.veymelo" → "Show 2.veymelo",
/// "Show 2.veymelo" → "Show 3.veymelo", "Show_09.veymelo" → "Show_10.veymelo".
fn next_numbered(path: &std::path::Path) -> PathBuf {
    let stem = path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
    let ext = path.extension().map_or(String::new(), |e| format!(".{}", e.to_string_lossy()));
    let digits = stem.chars().rev().take_while(char::is_ascii_digit).count();
    let (base, mut n, width) = match stem.len() - digits {
        cut if digits > 0 && cut > 0 && matches!(stem.as_bytes()[cut - 1], b' ' | b'_' | b'-') => (stem[..cut].to_string(), stem[cut..].parse::<u64>().unwrap_or(1) + 1, digits),
        _ => (format!("{stem} "), 2, 1),
    };
    loop {
        let candidate = path.with_file_name(format!("{base}{n:0width$}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

fn project_info(s: &mut Session, _: &Args) -> Result<Json> {
    let items = s.project().items.iter().map(|i| {
        let mut o = Map::new();
        o.insert("id", i.id as f64);
        o.insert("type", i.type_id());
        o.insert("name", i.name.as_str());
        if let Some(f) = i.folder {
            o.insert("folder", f as f64);
        }
        if let Some(c) = i.as_comp() {
            o.insert("size", [c.width as f64, c.height as f64]);
            o.insert("fps", veymelo_project::fps_json(c.frame_rate));
            o.insert("duration", c.duration.seconds());
            o.insert("layers", c.layers.len() as f64);
        }
        Json::Object(o)
    });
    Ok(Json::object([
        ("path", s.path.as_ref().map_or(Json::Null, |p| Json::from(p.display().to_string()))),
        ("modified", Json::Bool(s.modified())),
        ("comp", s.view.comp.map_or(Json::Null, |c| Json::Number(c as f64))),
        ("time", Json::Number(s.view.time.seconds())),
        ("items", Json::array(items)),
    ]))
}

fn project_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    let st = &mut cx.project.settings;
    if let Some(b) = a.text("bitDepth") {
        st.bit_depth = b.parse().expect("checked");
    }
    if let Some(on) = a.bool("linearBlending") {
        st.linear_blending = on;
    }
    if let Some(r) = a.whole("audioRate") {
        if !(8_000..=192_000).contains(&r) {
            return Err(Error::params(format!("`audioRate` is 8000 to 192000, not {r}")));
        }
        st.audio_rate = r as u32;
    }
    if let Some(d) = a.text("timeDisplay") {
        st.time_display = veymelo_project::TimeDisplay::from_id(d).expect("checked");
    }
    if let Some(f) = a.whole("frameStart") {
        if f > 1 {
            return Err(Error::params("`frameStart` is 0 or 1"));
        }
        st.frame_start = f as u8;
    }
    Ok(Json::object([
        ("bitDepth", Json::Number(st.bit_depth as f64)),
        ("linearBlending", Json::Bool(st.linear_blending)),
        ("audioRate", Json::Number(st.audio_rate as f64)),
        ("timeDisplay", Json::from(st.time_display.id())),
        ("frameStart", Json::Number(st.frame_start as f64)),
    ]))
}

fn inspect(l: &Look, a: &Args) -> Result<Json> {
    if let Some(layer) = a.json("layer") {
        let cid = comp_id(l.project, l.view, a)?;
        let comp = l.project.comp(cid).expect("found");
        let id = find_layer(comp, layer)?;
        return Ok(comp.layer(id).expect("found").to_json());
    }
    if let Some(c) = a.json("comp") {
        let id = find_comp(l.project, c)?;
        return Ok(l.project.item(id).expect("found").to_json());
    }
    Ok(l.project.to_json())
}

fn undo(s: &mut Session, _: &Args) -> Result<Json> {
    let label = s.history_mut().undo().ok_or_else(|| Error::refused("there is nothing to undo"))?;
    s.changed();
    Ok(Json::object([("undid", label)]))
}

fn redo(s: &mut Session, _: &Args) -> Result<Json> {
    let label = s.history_mut().redo().ok_or_else(|| Error::refused("there is nothing to redo"))?;
    s.changed();
    Ok(Json::object([("redid", label)]))
}

fn history_list(s: &mut Session, _: &Args) -> Result<Json> {
    Ok(Json::array(s.history().list().into_iter().map(|e| {
        let mut o = Map::new();
        o.insert("id", e.id as f64);
        o.insert("step", e.label);
        o.insert("parent", e.parent.map_or(Json::Null, |p| Json::Number(p as f64)));
        for (key, on) in [("current", e.current), ("past", e.past), ("future", e.future)] {
            if on {
                o.insert(key, true);
            }
        }
        Json::Object(o)
    })))
}

fn history_goto(s: &mut Session, a: &Args) -> Result<Json> {
    let id = a.whole("id").unwrap_or(0);
    if !s.history_mut().goto(id) {
        return Err(Error::not_found(format!("there is no state {id} in the history (history.list lists them)")));
    }
    s.changed();
    Ok(Json::object([("step", s.history().label())]))
}

fn view_set(cx: &mut Ctx, a: &Args) -> Result<Json> {
    if a.has("comp") {
        cx.view.comp = Some(comp_id(cx.project, cx.view, a)?);
    }
    if let Some(t) = a.time("time") {
        cx.view.time = t;
    }
    Ok(Json::object([("comp", cx.view.comp.map_or(Json::Null, |c| Json::Number(c as f64))), ("time", Json::Number(cx.view.time.seconds()))]))
}

fn command_list(_: &Look, a: &Args) -> Result<Json> {
    let search = a.text("search").map(str::to_lowercase);
    let hits = all().iter().filter(|c| search.as_ref().is_none_or(|s| c.id.to_lowercase().contains(s) || c.summary.to_lowercase().contains(s)));
    Ok(Json::array(hits.map(|c| Json::object([("id", Json::from(c.id)), ("summary", Json::from(c.summary)), ("changes", Json::Bool(c.changes()))]))))
}

fn command_describe(_: &Look, a: &Args) -> Result<Json> {
    let id = a.text("command").unwrap_or("");
    let c = find(id).ok_or_else(|| unknown(id))?;
    Ok(Json::object([
        ("id", Json::from(c.id)),
        ("summary", Json::from(c.summary)),
        ("params", params::schema(c.params)),
        ("returns", Json::from(c.returns)),
        ("changes", Json::Bool(c.changes())),
    ]))
}

/// A string `"$N"` or `"$N.key.0"` is what step N gave back (or a part of it); `"$$…"` is a
/// string starting with `$`. Not a reference: None.
fn reference(text: &str, results: &[Json]) -> Option<Result<Json>> {
    if let Some(rest) = text.strip_prefix("$$") {
        return Some(Ok(Json::from(format!("${rest}"))));
    }
    let mut parts = text.strip_prefix('$')?.split('.');
    let n: usize = parts.next()?.parse().ok()?;
    let Some(mut v) = n.checked_sub(1).and_then(|i| results.get(i)) else {
        return Some(Err(Error::params(format!("\"{text}\": there is no step {n} before this one"))));
    };
    for key in parts {
        let next = match v {
            Json::Object(o) => o.get(key),
            Json::Array(a) => key.parse::<usize>().ok().and_then(|i| a.get(i)),
            _ => None,
        };
        match next {
            Some(x) => v = x,
            None => return Some(Err(Error::params(format!("\"{text}\": step {n} gave back {}, which has no `{key}`", v.to_json())))),
        }
    }
    Some(Ok(v.clone()))
}

fn substitute(j: &Json, results: &[Json]) -> Result<Json> {
    Ok(match j {
        Json::String(s) => match reference(s, results) {
            Some(r) => r?,
            None => j.clone(),
        },
        Json::Array(a) => Json::Array(a.iter().map(|x| substitute(x, results)).collect::<Result<_>>()?),
        Json::Object(o) => {
            let mut m = Map::new();
            for (k, v) in o.iter() {
                m.insert(k, substitute(v, results)?);
            }
            Json::Object(m)
        }
        _ => j.clone(),
    })
}

fn batch(s: &mut Session, a: &Args) -> Result<Json> {
    let steps = a.need("steps").as_array().cloned().unwrap_or_default();
    if steps.is_empty() {
        return Err(Error::params("batch needs at least one step in `steps`"));
    }
    let mut project = s.project().clone();
    let mut view = s.view.clone();
    let mut results = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        let id = step.get("command").and_then(Json::as_str).unwrap_or("");
        let mut run = || -> Result<Json> {
            let o = step.as_object().ok_or_else(|| Error::params("a step is {\"command\": …, \"params\"?: {…}}"))?;
            if let Some(k) = o.keys().find(|k| *k != "command" && *k != "params") {
                return Err(Error::params(format!("a step has no `{k}` (it has command, params)")));
            }
            let cmd = find(id).ok_or_else(|| unknown(id))?;
            let params = substitute(step.get("params").unwrap_or(&Json::Null), &results)?;
            let args = Args::check(cmd.id, cmd.params, &params)?;
            match cmd.run {
                Run::Look(f) => f(&Look { project: &project, view: &view, catalog: &s.catalog }, &args),
                Run::Edit(f) => {
                    let before = project.clone();
                    let out = f(&mut Ctx { project: &mut project, view: &mut view, catalog: &s.catalog, label: None, merge: None }, &args)?;
                    // each step settles as a command on its own would: later steps see it whole
                    veymelo_project::essential::sync(&before, &mut project);
                    Ok(out)
                }
                Run::Session(_) => Err(Error::refused(format!("{id} cannot run inside a batch"))),
            }
        };
        match run() {
            Ok(out) => results.push(out),
            Err(e) => return Err(Error { kind: e.kind, message: format!("step {} ({}): {}; nothing was changed", i + 1, if id.is_empty() { "?" } else { id }, e.message) }),
        }
    }
    s.commit(a.text("label").unwrap_or("Batch"), project, view, None)?;
    Ok(Json::object([("results", Json::Array(results))]))
}
