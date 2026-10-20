//! What agents have done, step by step, for whoever shows it (the app beside its preview): each
//! tool call as it starts, what it is in a few words, and how it ended. Kept apart from the
//! session, so it can be read while a step still holds the session.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use veymelo_json::Value as Json;

/// How a step stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    Done,
    Failed,
}

/// One tool call.
#[derive(Clone, Debug)]
pub struct Step {
    /// Counted from 1, across the life of the program.
    pub n: u64,
    /// The project's file it was in ("" for none).
    pub project: String,
    /// What it does: a command's id (`layer.add`), or what a look is (`render_preview`).
    pub title: String,
    /// The command's own name for it (`New Layer`), if it is one.
    pub label: String,
    /// What it was given, in short.
    pub detail: String,
    pub state: State,
    /// What came of it, in short: the answer, or why it failed.
    pub note: String,
    /// Whether it changes the project (else it only looks).
    pub edits: bool,
    /// The layers it names (by name), for whoever shows where it worked.
    pub layers: Vec<String>,
    pub started: Instant,
    pub took: Option<Duration>,
    /// Made by the person in the app (else by an agent).
    pub person: bool,
}

/// The steps kept (the latest).
const KEEP: usize = 500;
/// The most characters a step's detail or note keeps.
const SHORT: usize = 160;

struct Log {
    steps: VecDeque<Step>,
    next: u64,
}

static LOG: Mutex<Log> = Mutex::new(Log { steps: VecDeque::new(), next: 1 });

fn log() -> std::sync::MutexGuard<'static, Log> {
    LOG.lock().unwrap_or_else(|e| e.into_inner())
}

/// `s` cut to `n` characters at most, on one line.
fn short(s: &str, n: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= n {
        return one;
    }
    let mut cut: String = one.chars().take(n - 1).collect();
    cut.push('…');
    cut
}

/// A number as people write it: whole ones without a point, others to three places at most.
fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A value as people read it: words as they are, numbers plainly, lists of numbers with commas,
/// on and off for yes and no, a list of anything else by how many, a group by what is in it.
fn plain(v: &Json) -> String {
    match v {
        Json::Null => String::new(),
        Json::Bool(b) => (if *b { "on" } else { "off" }).to_string(),
        Json::Number(n) => number(*n),
        Json::String(s) => s.clone(),
        Json::Array(a) if a.iter().all(|x| matches!(x, Json::Number(_))) => a.iter().map(plain).collect::<Vec<_>>().join(", "),
        Json::Array(a) if a.iter().all(|x| matches!(x, Json::String(_))) => a.iter().map(plain).collect::<Vec<_>>().join(", "),
        Json::Array(a) => format!("×{}", a.len()),
        Json::Object(_) => readable(v),
    }
}

/// A command's parameters as people read them, on one line: what it works on first (in
/// quotes), then each setting by its name (a width with its height as one size, a value after
/// the property it goes to without its name), e.g.
/// `“Sun” · solid · color #f2b134 · 600 × 600`.
pub fn readable(params: &Json) -> String {
    let Some(map) = params.as_object() else { return plain(params) };
    // what it works on: by name first, then the rest of what is quoted
    let (mut first, mut quoted, mut rest) = (Vec::new(), Vec::new(), Vec::new());
    let size = match (map.get("width").and_then(Json::as_f64), map.get("height").and_then(Json::as_f64)) {
        (Some(w), Some(h)) => Some(format!("{} × {}", number(w), number(h))),
        _ => None,
    };
    for (k, v) in map.iter() {
        let text = plain(v);
        if text.is_empty() {
            continue;
        }
        match k {
            "name" | "layer" => first.push(format!("“{text}”")),
            // (a composition given by its id says nothing to whoever reads it)
            "comp" if v.as_f64().is_some() => {}
            "comp" | "text" | "project" => quoted.push(format!("“{text}”")),
            "layers" => first.push(text),
            "type" | "command" | "path" | "preset" | "effect" | "style" | "light" | "value" => rest.push(text),
            "width" if size.is_some() => rest.push(size.clone().unwrap_or_default()),
            "height" if size.is_some() => {}
            _ => rest.push(format!("{k} {text}")),
        }
    }
    first.extend(quoted);
    first.extend(rest);
    first.join(" · ")
}

/// The layers a command's parameters name: `layer`, or `layers` (names only); a new layer by
/// its `name`.
fn layers_of(command: &str, params: Option<&Json>, into: &mut Vec<String>) {
    let Some(p) = params else { return };
    let mut add = |v: &Json| {
        if let Some(n) = v.as_str().filter(|n| !n.starts_with('#'))
            && !into.iter().any(|x| x == n)
        {
            into.push(n.to_string());
        }
    };
    if let Some(v) = p.get("layer") {
        add(v);
    }
    if let Some(a) = p.get("layers").and_then(Json::as_array) {
        a.iter().for_each(&mut add);
    }
    if command == "layer.add"
        && let Some(v) = p.get("name")
    {
        add(v);
    }
}

/// The layers a tool call's commands name, when they change the project.
fn layers(tool: &str, args: &Json) -> Vec<String> {
    let mut out = Vec::new();
    match tool {
        "command_run" => layers_of(args.get("command").and_then(Json::as_str).unwrap_or(""), args.get("params"), &mut out),
        "command_batch" => {
            for s in args.get("steps").and_then(Json::as_array).into_iter().flatten() {
                layers_of(s.get("command").and_then(Json::as_str).unwrap_or(""), s.get("params"), &mut out);
            }
        }
        _ => {}
    }
    out
}

/// A step's title, label, detail, and whether it edits, from the tool and its arguments.
fn describe(tool: &str, args: &Json) -> (String, String, String, bool) {
    let text = |k: &str| args.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    let edits = |id: &str| veymelo_engine::command(id).is_some_and(|c| matches!(c.run, veymelo_engine::Run::Edit(_) | veymelo_engine::Run::Session(_)));
    match tool {
        "command_run" => {
            let id = text("command");
            let label = veymelo_engine::command(&id).map(|c| c.label.to_string()).unwrap_or_default();
            let detail = args.get("params").filter(|p| !p.is_null()).map(readable).unwrap_or_default();
            let e = edits(&id);
            (id, label, short(&detail, SHORT), e)
        }
        "command_batch" => {
            let steps = args.get("steps").and_then(Json::as_array).cloned().unwrap_or_default();
            let ids: Vec<String> = steps.iter().filter_map(|s| s.get("command").or_else(|| s.get("id")).and_then(Json::as_str).map(str::to_string)).collect();
            let label = match text("label") {
                l if l.is_empty() => format!("{} steps at once", steps.len()),
                l => l,
            };
            let e = ids.iter().any(|id| edits(id));
            ("batch".into(), label, short(&ids.join(", "), SHORT), e)
        }
        "command_list" => ("command.list".into(), "Find commands".into(), text("search"), false),
        "project_list" => ("project_list".into(), "List the projects".into(), String::new(), false),
        "project_open" => ("project_open".into(), "Work in a project".into(), text("project"), false),
        "project_new" => ("project_new".into(), "Make a project".into(), short(&format!("{} in {}", text("name"), text("folder")), SHORT), true),
        "command_describe" => ("command.describe".into(), "Read about a command".into(), text("command"), false),
        "doc_inspect" => ("doc.inspect".into(), "Read the project".into(), short(&readable(args), SHORT), false),
        "render_preview" => ("render_preview".into(), "Look at a frame".into(), short(&readable(args), SHORT), false),
        "audio_preview" => ("audio_preview".into(), "Look at the sound".into(), short(&readable(args), SHORT), false),
        other => (other.to_string(), String::new(), short(&readable(args), SHORT), false),
    }
}

/// A tool call starts: its step's number.
pub fn begin(tool: &str, args: &Json) -> u64 {
    begin_in("", tool, args)
}

/// A tool call starts in the project in file `project`: its step's number.
pub fn begin_in(project: &str, tool: &str, args: &Json) -> u64 {
    begin_as(project, tool, args, false)
}

/// A change the person makes in the app starts, in the project in file `project` (told as an
/// agent's steps are, so agents and the person see each other's): its step's number.
pub fn begin_person(project: &str, tool: &str, args: &Json) -> u64 {
    begin_as(project, tool, args, true)
}

fn begin_as(project: &str, tool: &str, args: &Json, person: bool) -> u64 {
    let (title, label, detail, edits) = describe(tool, args);
    let layers = if edits { layers(tool, args) } else { Vec::new() };
    let mut l = log();
    let n = l.next;
    l.next += 1;
    l.steps.push_back(Step { n, project: project.to_string(), title, label, detail, state: State::Running, note: String::new(), edits, layers, started: Instant::now(), took: None, person });
    while l.steps.len() > KEEP {
        l.steps.pop_front();
    }
    n
}

/// Step `n` ended, well or not, with what came of it.
pub fn end(n: u64, ok: bool, note: &str) {
    let mut l = log();
    if let Some(s) = l.steps.iter_mut().rev().find(|s| s.n == n) {
        s.state = if ok { State::Done } else { State::Failed };
        s.note = short(note, SHORT);
        s.took = Some(s.started.elapsed());
    }
}

/// The steps kept, oldest first.
pub fn steps() -> Vec<Step> {
    log().steps.iter().cloned().collect()
}

/// How many steps there have been (it goes up with every one, so a change can be seen).
pub fn count() -> u64 {
    log().next - 1
}

/// The latest step's state and when it last changed: what shows has to be drawn again when
/// this differs.
pub fn stamp() -> (u64, Option<State>) {
    let l = log();
    (l.next, l.steps.back().map(|s| s.state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_described_and_ended() {
        let args = Json::parse(r##"{"command": "layer.add", "params": {"type": "solid", "color": "#ff0000"}}"##).unwrap();
        let n = begin("command_run", &args);
        let s = steps().into_iter().find(|s| s.n == n).unwrap();
        assert_eq!((s.title.as_str(), s.label.as_str(), s.state, s.edits), ("layer.add", "New Layer", State::Running, true));
        assert!(s.detail.contains("solid"), "{}", s.detail);
        let batch = Json::parse(r##"{"steps": [{"command": "prop.set", "params": {"layer": "Hero", "path": "transform/opacity", "value": 50}}, {"command": "anim.applyPreset", "params": {"layers": ["Hero", "Dot", "#3"], "preset": "fadeIn"}}]}"##).unwrap();
        let b = begin("command_batch", &batch);
        let s = steps().into_iter().find(|s| s.n == b).unwrap();
        assert_eq!(s.layers, ["Hero", "Dot"], "the layers named, once each, not by place");
        let added = begin("command_run", &Json::parse(r#"{"command": "layer.add", "params": {"type": "text", "name": "Title"}}"#).unwrap());
        assert_eq!(steps().into_iter().find(|s| s.n == added).unwrap().layers, ["Title"]);
        end(n, false, "there is no composition yet\n(comp.new makes one)");
        let s = steps().into_iter().find(|s| s.n == n).unwrap();
        assert_eq!((s.state, s.note.as_str()), (State::Failed, "there is no composition yet (comp.new makes one)"));
        let look = begin("render_preview", &Json::parse(r#"{"time": 1}"#).unwrap());
        let s = steps().into_iter().find(|s| s.n == look).unwrap();
        assert!(!s.edits && s.label == "Look at a frame");
        assert_eq!(short(&"x".repeat(200), 10).chars().count(), 10);
    }

    #[test]
    fn parameters_read_as_words() {
        let p = |s: &str| readable(&Json::parse(s).unwrap());
        assert_eq!(p(r##"{"type": "solid", "color": "#f2b134", "width": 600, "height": 600, "name": "Sun"}"##), "“Sun” · solid · color #f2b134 · 600 × 600");
        assert_eq!(p(r#"{"layer": "Card", "path": "transform/position", "value": [60, 60.5, 0]}"#), "“Card” · transform/position · 60, 60.5, 0");
        assert_eq!(p(r#"{"layer": "Card", "switches": {"threeD": true}, "in": 1.25}"#), "“Card” · switches threeD on · in 1.25");
        assert_eq!(p(r#"{"layer": "Dot", "path": "transform/scale", "keys": [{"t": 0, "v": 1}, {"t": 1, "v": 2}]}"#), "“Dot” · transform/scale · keys ×2");
        assert_eq!(p(r#"{"time": 1}"#), "time 1");
        assert_eq!(p(r#"{"type": "text", "text": "hello", "name": "Title"}"#), "“Title” · “hello” · text", "by its name first");
    }
}
