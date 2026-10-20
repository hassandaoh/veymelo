//! MCP over standard input and output: JSON-RPC 2.0 messages, one a line.

use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};

use veymelo_engine::Session;
use veymelo_json::{Map, Value as Json};

use crate::tools::{self, BadCall, INSTRUCTIONS, Reply};

/// The protocol version answered when the client asks for one this server does not know.
pub const PROTOCOL: &str = "2025-06-18";
const KNOWN: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Where tool calls go: a session here, or the open app.
pub trait Backend {
    fn call(&mut self, name: &str, args: &Json) -> Result<Reply, BadCall>;
}

/// A session in this process (shared, so the app's channel can serve the same one).
pub struct Local(pub Arc<Mutex<Session>>);

impl Local {
    pub fn new(session: Session) -> Local {
        Local(Arc::new(Mutex::new(session)))
    }
}

/// Run a tool on a shared session. A command that panics is a mistake in Veymelo: it is
/// reported, and the session goes on from the state before it.
pub fn call_shared(session: &Mutex<Session>, name: &str, args: &Json) -> Result<Reply, BadCall> {
    call_in(session, "", name, args)
}

/// [`call_shared`] in the project in file `project` (its steps say so).
pub fn call_in(session: &Mutex<Session>, project: &str, name: &str, args: &Json) -> Result<Reply, BadCall> {
    // kept as a step for whoever shows what agents do, from before it waits for the session
    let step = crate::activity::begin_in(project, name, args);
    // long work that only reads the project (a render to a file) is done on a copy of it, so
    // the project is not held meanwhile: the app and other agents go on with it
    let mut copy = None;
    let mut held = None;
    if long_and_reading(name, args) {
        let s = session.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut c = Session::with(s.project().clone(), s.path.clone());
        c.view = s.view.clone();
        copy = Some(c);
    } else {
        held = Some(session.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
    }
    let s: &mut Session = match (&mut copy, &mut held) {
        (Some(c), _) => c,
        (None, Some(h)) => h,
        (None, None) => unreachable!("one or the other"),
    };
    let out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tools::call(s, name, args))) {
        Ok(r) => r,
        Err(why) => {
            let what = why.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| why.downcast_ref::<String>().cloned()).unwrap_or_default();
            Ok(Reply::error(format!("Veymelo failed inside ({what}); nothing was changed. This is a mistake in Veymelo, not in the call.")))
        }
    };
    match &out {
        Ok(r) => {
            let text = r.content.iter().find_map(|c| if let crate::tools::Content::Text(t) = c { Some(t.as_str()) } else { None }).unwrap_or("");
            crate::activity::end(step, !r.is_error, text);
        }
        Err(BadCall(why)) => crate::activity::end(step, false, why),
    }
    out
}

/// Whether a call is long work that changes nothing in the project: a render made into a file
/// or a frame written out.
fn long_and_reading(name: &str, args: &Json) -> bool {
    name == "command_run" && matches!(args.get("command").and_then(Json::as_str), Some("render.export" | "render.frame"))
}

impl Backend for Local {
    fn call(&mut self, name: &str, args: &Json) -> Result<Reply, BadCall> {
        call_shared(&self.0, name, args)
    }
}

fn error(id: &Json, code: i32, message: &str) -> Json {
    Json::object([("jsonrpc", Json::from("2.0")), ("id", id.clone()), ("error", Json::object([("code", Json::Number(code as f64)), ("message", Json::from(message))]))])
}

fn answer(id: &Json, result: Json) -> Json {
    Json::object([("jsonrpc", Json::from("2.0")), ("id", id.clone()), ("result", result)])
}

/// The answer to one message (none for a notification).
pub fn handle(backend: &mut dyn Backend, message: &Json) -> Option<Json> {
    let Some(m) = message.as_object() else { return Some(error(&Json::Null, -32600, "a message is a JSON-RPC 2.0 object")) };
    let id = m.get("id")?.clone();
    let Some(method) = m.get("method").and_then(Json::as_str) else { return Some(error(&id, -32600, "a request needs a method")) };
    let params = m.get("params").cloned().unwrap_or(Json::Object(Map::new()));
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Json::as_str).unwrap_or(PROTOCOL);
            let version = if KNOWN.contains(&asked) { asked } else { PROTOCOL };
            answer(
                &id,
                Json::object([
                    ("protocolVersion", Json::from(version)),
                    ("capabilities", Json::object([("tools", Json::object([("listChanged", false)]))])),
                    ("serverInfo", Json::object([("name", "veymelo"), ("version", env!("CARGO_PKG_VERSION"))])),
                    ("instructions", Json::from(INSTRUCTIONS)),
                ]),
            )
        }
        "ping" => answer(&id, Json::Object(Map::new())),
        "tools/list" => answer(&id, Json::object([("tools", tools::list())])),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Json::as_str) else { return Some(error(&id, -32602, "tools/call needs the tool's name")) };
            let args = params.get("arguments").cloned().unwrap_or(Json::Object(Map::new()));
            match backend.call(name, &args) {
                Ok(reply) => answer(&id, reply.to_json()),
                Err(BadCall(why)) => error(&id, -32602, &why),
            }
        }
        _ => error(&id, -32601, &format!("this server has no method {method}")),
    })
}

/// Answer messages from `input` on `output` until the input ends.
pub fn serve(backend: &mut dyn Backend, input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match Json::parse(&line) {
            Ok(message) => handle(backend, &message),
            Err(e) => Some(error(&Json::Null, -32700, &format!("not JSON: {e}"))),
        };
        if let Some(r) = reply {
            writeln!(output, "{}", r.to_json())?;
            output.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn talk(lines: &[&str]) -> Vec<Json> {
        let mut backend = Local::new(Session::new());
        let input = lines.join("\n");
        let mut out = Vec::new();
        serve(&mut backend, input.as_bytes(), &mut out).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| Json::parse(l).unwrap()).collect()
    }

    #[test]
    fn a_whole_conversation() {
        let replies = talk(&[
            r#"{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}"#,
            r#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#,
            r#"{"jsonrpc": "2.0", "id": 2, "method": "tools/list"}"#,
            r#"{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "command_run", "arguments": {"command": "comp.new", "params": {"name": "A"}}}}"#,
            r#"{"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "command_run", "arguments": {"command": "comp.info"}}}"#,
            r#"{"jsonrpc": "2.0", "id": "five", "method": "ping"}"#,
        ]);
        assert_eq!(replies.len(), 5, "no answer to a notification");
        let init = replies[0].get("result").unwrap();
        assert_eq!(init.get("protocolVersion").unwrap().as_str(), Some("2025-06-18"));
        assert!(init.get("instructions").unwrap().as_str().unwrap().contains("command_list"));
        assert_eq!(replies[1].get("result").unwrap().get("tools").unwrap().as_array().unwrap().len(), 10);
        let made = replies[2].get("result").unwrap().get("content").unwrap().at(0).unwrap().get("text").unwrap().as_str().unwrap();
        assert_eq!(made, r#"{"comp":1,"name":"A"}"#);
        let info = replies[3].get("result").unwrap().get("content").unwrap().at(0).unwrap().get("text").unwrap().as_str().unwrap();
        assert!(info.contains(r#""name":"A""#), "the session lasts the whole conversation");
        assert_eq!(replies[4].get("id").unwrap().as_str(), Some("five"));
    }

    #[test]
    fn protocol_mistakes() {
        let replies = talk(&[
            "{not json",
            r#"{"jsonrpc": "2.0", "id": 1, "method": "resources/list"}"#,
            r#"{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "nope"}}"#,
            r#"{"jsonrpc": "2.0", "id": 3, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}"#,
            "[1, 2]",
        ]);
        let code = |r: &Json| r.get("error").and_then(|e| e.get("code")).and_then(Json::as_f64);
        assert_eq!(code(&replies[0]), Some(-32700.0));
        assert_eq!(code(&replies[1]), Some(-32601.0));
        assert_eq!(code(&replies[2]), Some(-32602.0));
        assert_eq!(replies[3].get("result").unwrap().get("protocolVersion").unwrap().as_str(), Some(PROTOCOL));
        assert_eq!(code(&replies[4]), Some(-32600.0));
    }
}
