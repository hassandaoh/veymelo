//! The tools an agent is given. They are few: everything Veymelo can do is a command, and
//! these find, describe and run commands, read the project, and show a frame and the sound.

use veymelo_engine::render::Options;
use veymelo_engine::{Args, Param, Session, Ty};
use veymelo_json::{Map, Value as Json};

use crate::base64;

/// A tool: its name, what it is for, and what it takes.
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub params: &'static [Param],
}

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "project_list",
        description: "List the person's Veymelo projects (most recently opened first), and which one you work in. Work happens in one project at a time: open one with project_open, or make one with project_new.",
        params: &[],
    },
    Tool {
        name: "project_open",
        description: "Work in a project from now on: every other tool acts on it, and its changes are saved to its file as they are made. The person sees it in the app when they open it there.",
        params: &[Param::need("project", Ty::Text, "its name (as project_list gives it), or its folder or .veymelo file")],
    },
    Tool {
        name: "project_new",
        description: "Make a project in a folder of its own (folder/name/name.veymelo) and work in it from now on. Ask the person where they keep their projects if you do not know.",
        params: &[Param::need("name", Ty::Text, "its name"), Param::need("folder", Ty::Text, "the folder to make it in (its own folder goes inside)")],
    },
    Tool {
        name: "command_list",
        description: "List Veymelo's commands (every action in the program is one), or those matching a word. Use command_describe to see what one takes.",
        params: &[Param::opt("search", Ty::Text, "a word to look for in ids and summaries")],
    },
    Tool {
        name: "command_describe",
        description: "What a command does, the parameters it takes (as JSON Schema) and what it gives back.",
        params: &[Param::need("command", Ty::Text, "the command's id, like layer.add")],
    },
    Tool {
        name: "command_run",
        description: "Run one command. A command that changes the project is one undo step; if it fails, nothing changes and the error says why and what is there instead.",
        params: &[Param::need("command", Ty::Text, "the command's id"), Param::opt("params", Ty::Object, "its parameters")],
    },
    Tool {
        name: "command_batch",
        description: "Run several commands as one undo step, all or nothing. In a step's params, the string \"$2\" is what step 2 gave back and \"$2.layer\" its layer field, so later steps can use what earlier ones made.",
        params: &[Param::need("steps", Ty::List, "[{\"command\": …, \"params\": {…}}, …]"), Param::opt("label", Ty::Text, "what to call the undo step")],
    },
    Tool {
        name: "doc_inspect",
        description: "Read the project, one composition or one layer as JSON, written as in a .veymelo file: only what differs from the defaults (use command_run prop.list to see every property).",
        params: &[Param::opt("comp", Ty::Comp, "just this composition"), Param::opt("layer", Ty::Layer, "just this layer")],
    },
    Tool {
        name: "render_preview",
        description: "See a frame of a composition as a picture, to check your work. Notes say what could not be drawn yet.",
        params: &[
            Param::opt("comp", Ty::Comp, "the composition (default: the one being worked on)"),
            Param::opt("time", Ty::Time, "when, in seconds (default: the view's time)"),
            Param::opt("maxSize", Ty::Whole, "the longest side of the picture in pixels (default 1024; never more than the composition's own size)"),
            Param::opt("background", Ty::Bool, "draw the background colour (default true)"),
            Param::opt("motionBlur", Ty::Bool, "blur moving layers that have motion blur on (default true)"),
            Param::opt("guides", Ty::Bool, "draw guide layers too (default false)"),
        ],
    },
    Tool {
        name: "audio_preview",
        description: "See a composition's sound, as you cannot hear it: a picture of it (the left channel above the right, lines at 0 and ±6 dB, clipping in red) and how loud it is (LUFS as broadcasters and clip apps measure, peaks in dBFS, whether it clips).",
        params: &[
            Param::opt("comp", Ty::Comp, "the composition (default: the one being worked on)"),
            Param::opt("start", Ty::Time, "from when, in seconds (default 0)"),
            Param::opt("end", Ty::Time, "to when (default: the composition's end)"),
            Param::opt("layer", Ty::Layer, "only this layer's sound"),
            Param::opt("width", Ty::Whole, "pixels across (default 1200)"),
            Param::opt("height", Ty::Whole, "pixels down (default 300)"),
        ],
    },
];

/// What the agent is told when it connects.
pub const INSTRUCTIONS: &str = "Veymelo makes videos. With the app open, first choose the project to work in: project_list shows the person's projects, \
project_open opens one, project_new makes one in a folder (ask the person where if you do not know); your changes are saved as you make them, and the person watches in the app. \
A project holds compositions; a composition has a size, a frame rate, a length and layers stacked top first; \
each layer has properties (transform/position, transform/opacity…) that can be animated with keyframes, or worked out by an expression \
(prop.expression: wiggle(2, 30), loopOut(\"cycle\"), thisComp.layer(\"Null 1\").transform.position…; try one first with prop.evaluate). Layers take effects (effect.types; a puppet: puppet.pin, then animate the pins' position). Everything is done with commands: find them with \
command_list, read what one takes with command_describe, run it with command_run (or several at once with command_batch). Read the project with doc_inspect \
and look at your work with render_preview (and at its sound with audio_preview). Times are in seconds; positions are in pixels with y going down; rotations are in degrees, clockwise. \
Layers, compositions and properties can be named by id or by name, and mistakes come back saying what there is.";

/// A piece of a reply.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Text(String),
    Png(Vec<u8>),
}

/// What a tool gives back.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub content: Vec<Content>,
    pub is_error: bool,
}

impl Reply {
    pub fn text(text: impl Into<String>) -> Reply {
        Reply { content: vec![Content::Text(text.into())], is_error: false }
    }

    pub fn error(text: impl Into<String>) -> Reply {
        Reply { content: vec![Content::Text(text.into())], is_error: true }
    }

    /// As an MCP tool result.
    pub fn to_json(&self) -> Json {
        let content = self.content.iter().map(|c| match c {
            Content::Text(t) => Json::object([("type", "text"), ("text", t.as_str())]),
            Content::Png(png) => Json::object([("type", "image"), ("data", base64::encode(png).as_str()), ("mimeType", "image/png")]),
        });
        let mut o = Map::new();
        o.insert("content", Json::array(content));
        if self.is_error {
            o.insert("isError", true);
        }
        Json::Object(o)
    }
}

/// A mistake in calling a tool itself (as opposed to the tool failing): an unknown tool, or
/// arguments it does not take.
#[derive(Clone, Debug, PartialEq)]
pub struct BadCall(pub String);

/// The tools as MCP lists them.
pub fn list() -> Json {
    Json::array(TOOLS.iter().map(|t| {
        Json::object([("name", Json::from(t.name)), ("description", Json::from(t.description)), ("inputSchema", veymelo_engine::schema(t.params))])
    }))
}

fn result(r: veymelo_engine::Result<Json>) -> Reply {
    match r {
        Ok(j) => Reply::text(j.to_json()),
        Err(e) => Reply::error(e.message),
    }
}

/// Call tool `name` with `args` on `session`.
pub fn call(session: &mut Session, name: &str, args: &Json) -> Result<Reply, BadCall> {
    let tool = TOOLS.iter().find(|t| t.name == name).ok_or_else(|| BadCall(format!("there is no tool `{name}` (there is {})", TOOLS.iter().map(|t| t.name).collect::<Vec<_>>().join(", "))))?;
    let a = Args::check(tool.name, tool.params, args).map_err(|e| BadCall(e.message))?;
    let pass = |keys: &[&str]| {
        let mut m = Map::new();
        for k in keys {
            if let Some(v) = a.json(k) {
                m.insert(*k, v.clone());
            }
        }
        Json::Object(m)
    };
    Ok(match name {
        "project_list" | "project_open" | "project_new" => Reply::error("this server works on a project of its own (`veymelo mcp`); the person's projects are reached through the app: `veymelo mcp --bridge` with Veymelo open"),
        "command_list" => result(session.run("command.list", &pass(&["search"]))),
        "command_describe" => result(session.run("command.describe", &pass(&["command"]))),
        "command_run" => result(session.run(a.text("command").unwrap_or(""), a.json("params").unwrap_or(&Json::Null))),
        "command_batch" => result(session.run("batch", &pass(&["steps", "label"]))),
        "doc_inspect" => result(session.run("doc.inspect", &pass(&["comp", "layer"]))),
        "audio_preview" => match session.hear(&pass(&["comp", "start", "end", "layer", "width", "height"])) {
            Ok((png, about)) => Reply { content: vec![Content::Png(png), Content::Text(about.to_json())], is_error: false },
            Err(e) => Reply::error(e.message),
        },
        _ => preview(session, &a),
    })
}

fn preview(session: &mut Session, a: &Args) -> Reply {
    let mut which = Map::new();
    if let Some(c) = a.json("comp") {
        which.insert("comp", c.clone());
    }
    let info = match session.run("comp.info", &Json::Object(which)) {
        Ok(info) => info,
        Err(e) => return Reply::error(e.message),
    };
    let (w, h) = (info.get("width").and_then(Json::as_f64).unwrap_or(1.0), info.get("height").and_then(Json::as_f64).unwrap_or(1.0));
    let longest = a.whole("maxSize").unwrap_or(1024).max(16) as f64;
    let options = Options {
        scale: (longest / w.max(h)).min(1.0),
        background: a.bool("background").unwrap_or(true),
        motion_blur: a.bool("motionBlur").unwrap_or(true),
        guides: a.bool("guides").unwrap_or(false),
        ..Options::default()
    };
    let time = a.time("time");
    match session.render(info.get("comp"), time, &options) {
        Err(e) => Reply::error(e.message),
        Ok(r) => {
            let about = Json::object([
                ("comp", info.get("comp").cloned().unwrap_or(Json::Null)),
                ("time", Json::Number(time.unwrap_or(session.view.time).seconds())),
                ("width", Json::Number(r.image.width as f64)),
                ("height", Json::Number(r.image.height as f64)),
                ("notes", Json::array(r.notes)),
            ]);
            Reply { content: vec![Content::Png(veymelo_engine::render::to_png(&r.image)), Content::Text(about.to_json())], is_error: false }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_text(s: &mut Session, name: &str, args: &str) -> Reply {
        call(s, name, &Json::parse(args).unwrap()).unwrap()
    }

    fn text(r: &Reply) -> &str {
        match &r.content[0] {
            Content::Text(t) => t,
            Content::Png(_) => panic!("a picture"),
        }
    }

    #[test]
    fn an_agent_makes_and_sees_a_frame() {
        let mut s = Session::new();
        let r = call_text(&mut s, "command_run", r#"{"command": "comp.new", "params": {"width": 640, "height": 360}}"#);
        assert!(!r.is_error, "{}", text(&r));
        let r = call_text(&mut s, "command_batch", r##"{"steps": [{"command": "layer.add", "params": {"type": "solid", "color": "#3366ff", "width": 100, "height": 100}}, {"command": "prop.set", "params": {"layer": "$1.layer", "path": "transform/rotation", "value": 45}}]}"##);
        assert!(!r.is_error, "{}", text(&r));
        let r = call_text(&mut s, "render_preview", r#"{"maxSize": 320}"#);
        let Content::Png(png) = &r.content[0] else { panic!("no picture") };
        assert_eq!(&png[1..4], b"PNG");
        assert!(text(&Reply { content: r.content[1..].to_vec(), is_error: false }).contains(r#""width":320,"height":180"#));
        let j = r.to_json();
        assert_eq!(j.get("content").unwrap().at(0).unwrap().get("mimeType").unwrap().as_str(), Some("image/png"));
        assert_eq!(base64::decode(j.get("content").unwrap().at(0).unwrap().get("data").unwrap().as_str().unwrap()), *png);
        let doc = call_text(&mut s, "doc_inspect", r##"{"layer": "#1"}"##);
        assert!(text(&doc).contains(r#""rotation":45"#), "{}", text(&doc));
    }

    #[test]
    fn failures_come_back_as_errors_the_agent_reads() {
        let mut s = Session::new();
        let r = call_text(&mut s, "command_run", r#"{"command": "layer.add", "params": {"type": "null"}}"#);
        assert!(r.is_error);
        assert_eq!(text(&r), "there is no composition yet (comp.new makes one)");
        assert_eq!(r.to_json().get("isError"), Some(&Json::Bool(true)));
        // mistakes in calling the tool itself are not the tool failing
        assert_eq!(call(&mut s, "command_runn", &Json::Null).unwrap_err().0, "there is no tool `command_runn` (there is project_list, project_open, project_new, command_list, command_describe, command_run, command_batch, doc_inspect, render_preview, audio_preview)");
        assert!(call(&mut s, "command_run", &Json::parse(r#"{"cmd": "x"}"#).unwrap()).unwrap_err().0.starts_with("command_run has no parameter `cmd`"));
    }

    #[test]
    fn every_tool_is_listed_with_its_schema() {
        let l = list();
        assert_eq!(l.as_array().unwrap().len(), TOOLS.len());
        for t in l.as_array().unwrap() {
            assert_eq!(t.get("inputSchema").unwrap().get("type").unwrap().as_str(), Some("object"));
        }
    }

    #[test]
    fn an_agent_sees_the_sound() {
        let mut s = Session::new();
        call_text(&mut s, "command_run", r#"{"command": "comp.new", "params": {"width": 64, "height": 64, "duration": 2}}"#);
        call_text(&mut s, "command_run", r##"{"command": "layer.add", "params": {"type": "solid", "color": "#000000"}}"##);
        call_text(&mut s, "command_run", r##"{"command": "effect.add", "params": {"layer": "#1", "type": "tone"}}"##);
        let r = call_text(&mut s, "audio_preview", r#"{"width": 400, "height": 120}"#);
        assert!(!r.is_error);
        assert!(matches!(&r.content[0], Content::Png(p) if p.starts_with(b"\x89PNG")));
        let Content::Text(about) = &r.content[1] else { panic!("the numbers") };
        assert!(about.contains("\"integrated\"") && about.contains("\"heard\":true"), "{about}");
    }
}
