//! The open app's control channel, and the bridge that reaches it.
//!
//! The app listens on a port of this computer only, and writes the port with a secret to a file
//! only its user can read. `veymelo mcp --bridge` reads that file and passes each tool call
//! through, so the agent works on the very project the person is looking at, live.
//!
//! On the line, each message is one line of JSON: first `{"token": …}` (answered with
//! `{"ok": true}`), then `{"name": …, "arguments": {…}}`, answered with an MCP tool result or
//! `{"error": "…"}` for a call the tool could not take.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use veymelo_json::Value as Json;

use crate::library::{Agent, Library};
use crate::server::Backend;
use crate::tools::{BadCall, Content, Reply};

/// Where the app says where its channel is: `$VEYMELO_CHANNEL`, or the app's folder in
/// Application Support.
pub fn default_file() -> PathBuf {
    if let Some(p) = std::env::var_os("VEYMELO_CHANNEL") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join("Library/Application Support/Veymelo/channel.json")
}

/// A secret no one else can guess, from the system's random source.
fn secret() -> io::Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 24];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The app's end: open while it lives.
pub struct Channel {
    pub port: u16,
    file: PathBuf,
}

impl Channel {
    /// Start listening for bridges to `session`, and say where in `file`.
    pub fn open(library: Arc<Library>, file: &Path) -> io::Result<Channel> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let token = secret()?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(file)?;
            let about = Json::object([("port", Json::Number(port as f64)), ("token", Json::from(token.as_str())), ("pid", Json::Number(std::process::id() as f64))]);
            f.write_all(about.to_pretty().as_bytes())?;
        }
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (library, token) = (library.clone(), token.clone());
                std::thread::spawn(move || {
                    let _ = serve_bridge(stream, library, &token);
                });
            }
        });
        Ok(Channel { port, file: file.to_path_buf() })
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

/// How many agents are bridged to the app now.
static BRIDGED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many agents are bridged to the app now (for whoever shows whether one is there).
pub fn bridged() -> usize {
    BRIDGED.load(std::sync::atomic::Ordering::Relaxed)
}

/// One bridged agent counted while it stays.
struct Counted;

impl Counted {
    fn new() -> Counted {
        BRIDGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Counted
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        BRIDGED.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

fn serve_bridge(stream: TcpStream, library: Arc<Library>, token: &str) -> io::Result<()> {
    let mut out = stream.try_clone()?;
    let mut lines = BufReader::new(stream).lines();
    let first = lines.next().transpose()?.unwrap_or_default();
    let given = Json::parse(&first).ok().and_then(|j| j.get("token").and_then(Json::as_str).map(str::to_string));
    if given.as_deref() != Some(token) {
        writeln!(out, "{}", Json::object([("error", "wrong token")]).to_json())?;
        return Ok(());
    }
    writeln!(out, "{}", Json::object([("ok", true)]).to_json())?;
    let _here = Counted::new();
    // this agent's own choice of project
    let mut agent = Agent::new(library);
    for line in lines {
        let line = line?;
        let reply = match Json::parse(&line) {
            Err(e) => Json::object([("error", format!("not JSON: {e}"))]),
            Ok(m) => {
                let name = m.get("name").and_then(Json::as_str).unwrap_or("");
                let args = m.get("arguments").cloned().unwrap_or(Json::Null);
                match agent.call(name, &args) {
                    Ok(r) => r.to_json(),
                    Err(BadCall(why)) => Json::object([("error", why)]),
                }
            }
        };
        writeln!(out, "{}", reply.to_json())?;
    }
    Ok(())
}

/// The bridge's end: tool calls passed to the open app. It connects when first needed and again
/// after the app was closed and opened, so it can start before the app does.
pub struct Bridge {
    file: PathBuf,
    line: Option<(BufReader<TcpStream>, TcpStream)>,
    /// The project the agent chose (its file), chosen again when the line is new.
    project: Option<String>,
}

impl Bridge {
    pub fn new(file: PathBuf) -> Bridge {
        Bridge { file, line: None, project: None }
    }

    fn connect(&mut self) -> Result<(), String> {
        let text = std::fs::read_to_string(&self.file).map_err(|_| "Veymelo is not open (open the app, then try again)".to_string())?;
        let about = Json::parse(&text).map_err(|e| format!("{}: {e}", self.file.display()))?;
        let port = about.get("port").and_then(Json::as_u64).ok_or("the channel file has no port")?;
        let token = about.get("token").and_then(Json::as_str).ok_or("the channel file has no token")?;
        let stream = TcpStream::connect(("127.0.0.1", port as u16)).map_err(|_| "Veymelo is not open (open the app, then try again)".to_string())?;
        let mut out = stream.try_clone().map_err(|e| e.to_string())?;
        let mut input = BufReader::new(stream);
        writeln!(out, "{}", Json::object([("token", token)]).to_json()).map_err(|e| e.to_string())?;
        let mut first = String::new();
        input.read_line(&mut first).map_err(|e| e.to_string())?;
        if Json::parse(&first).ok().and_then(|j| j.get("ok").and_then(Json::as_bool)) != Some(true) {
            return Err("the app did not take the bridge's token (it may have been opened again: try again)".into());
        }
        self.line = Some((input, out));
        // a new line: the app has forgotten which project this agent works in
        if let Some(file) = self.project.clone() {
            let again = Json::object([("name", Json::from("project_open")), ("arguments", Json::object([("project", Json::from(file.as_str()))]))]).to_json();
            self.ask(&again)?;
        }
        Ok(())
    }

    fn ask(&mut self, message: &str) -> Result<Json, String> {
        if self.line.is_none() {
            self.connect()?;
        }
        let (input, out) = self.line.as_mut().expect("connected");
        let mut reply = String::new();
        let sent = writeln!(out, "{message}").and_then(|_| input.read_line(&mut reply));
        match sent {
            Ok(n) if n > 0 => Json::parse(&reply).map_err(|e| format!("the app answered something that is not JSON: {e}")),
            _ => {
                self.line = None;
                Err("the connection to Veymelo was lost".into())
            }
        }
    }
}

impl Backend for Bridge {
    fn call(&mut self, name: &str, args: &Json) -> Result<Reply, BadCall> {
        let message = Json::object([("name", Json::from(name)), ("arguments", args.clone())]).to_json();
        // once more on a lost line: the app may have been closed and opened again
        let reply = self.ask(&message).or_else(|_| self.ask(&message));
        match reply {
            Err(why) => Ok(Reply::error(why)),
            Ok(j) => {
                if let Some(why) = j.get("error").and_then(Json::as_str) {
                    return Err(BadCall(why.to_string()));
                }
                let reply = reply_from_json(&j);
                if matches!(name, "project_open" | "project_new") && !reply.is_error {
                    let text = reply.content.iter().find_map(|c| if let Content::Text(t) = c { Some(t.as_str()) } else { None }).unwrap_or("");
                    self.project = Json::parse(text).ok().and_then(|j| j.get("file").and_then(Json::as_str).map(str::to_string));
                }
                Ok(reply)
            }
        }
    }
}

/// A tool result back from its JSON.
fn reply_from_json(j: &Json) -> Reply {
    let content = j
        .get("content")
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|c| match c.get("type").and_then(Json::as_str) {
                    Some("text") => Some(Content::Text(c.get("text").and_then(Json::as_str).unwrap_or("").to_string())),
                    Some("image") => Some(Content::Png(crate::base64::decode(c.get("data").and_then(Json::as_str).unwrap_or("")))),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Reply { content, is_error: j.get("isError").and_then(Json::as_bool) == Some(true) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bridge_works_on_the_apps_projects() {
        let dir = std::env::temp_dir().join(format!("veymelo-channel-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("channel.json");
        let library = Arc::new(Library::new(dir.join("projects.json")));
        let channel = Channel::open(library.clone(), &file).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600, "only its user can read the secret");
        }
        let mut bridge = Bridge::new(file.clone());
        let folder = dir.join("work").display().to_string();
        let r = bridge.call("project_new", &Json::parse(&format!(r#"{{"name": "Show", "folder": "{folder}"}}"#)).unwrap()).unwrap();
        assert!(!r.is_error, "{r:?}");
        let r = bridge.call("command_run", &Json::parse(r#"{"command": "comp.new", "params": {"name": "Live"}}"#).unwrap()).unwrap();
        assert!(!r.is_error, "{r:?}");
        let project = library.list()[0].file.clone();
        let session = library.session(&project).unwrap();
        assert_eq!(session.lock().unwrap().project().items[0].name, "Live", "the app's own session of the project changed");
        let r = bridge.call("render_preview", &Json::parse(r#"{"maxSize": 64}"#).unwrap()).unwrap();
        assert!(matches!(&r.content[0], Content::Png(p) if &p[1..4] == b"PNG"));
        assert_eq!(bridge.call("nope", &Json::Null).unwrap_err().0.split(" (").next().unwrap(), "there is no tool `nope`");
        // a wrong secret gets nothing
        let mut other = TcpStream::connect(("127.0.0.1", channel.port)).unwrap();
        writeln!(other, r#"{{"token": "guess"}}"#).unwrap();
        let mut line = String::new();
        BufReader::new(other).read_line(&mut line).unwrap();
        assert!(line.contains("wrong token"));
        drop(channel);
        assert!(!file.exists(), "the channel file goes when the app closes");
        // the app opened again: the agent goes on in the project it chose
        let again = Channel::open(library.clone(), &file).unwrap();
        let r = bridge.call("command_run", &Json::parse(r#"{"command": "comp.new", "params": {"name": "Later"}}"#).unwrap()).unwrap();
        assert!(!r.is_error, "{r:?}");
        assert!(session.lock().unwrap().project().items.iter().any(|i| i.name == "Later"));
        drop(again);
        let mut lost = Bridge::new(file);
        let r = lost.call("command_list", &Json::Null).unwrap();
        assert!(r.is_error);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
