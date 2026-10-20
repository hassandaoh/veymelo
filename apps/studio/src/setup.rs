//! Connecting an AI to the app: the command an AI runs to reach it (the command line tool in
//! the app's bundle, by its full path, so another `veymelo` on the person's PATH is never the
//! one run), and setting it up in Claude Code or Claude Desktop for the person, when they ask.
//!
//! Claude Code is set up with its own command line tool (`claude mcp add --scope user …`),
//! found on the PATH or where Claude Desktop keeps one of its own. Claude Desktop is set up by
//! adding the server to its settings file, a copy of the file kept beside it first; it reads
//! the file when it starts. Either is done on a thread of its own, and only when the person
//! has said yes.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use veymelo_json::{Map, Value};

/// The name the server goes by in the AI's settings.
pub const NAME: &str = "veymelo";

/// An app to set up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum App {
    Code,
    Desktop,
}

/// The command line tool beside the app (in its bundle, or where it was built).
pub fn program() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("veymelo"))).filter(|p| p.is_file())
}

/// A path as a shell reads it: in quotes when it has a space or a quote in it.
fn quoted(p: &Path) -> String {
    let s = p.display().to_string();
    if s.contains([' ', '\'', '"', '(', ')', '&']) { format!("\"{}\"", s.replace('"', "\\\"")) } else { s }
}

/// What an AI runs to reach the app, by its full path.
pub fn command() -> String {
    match program() {
        Some(p) => format!("{} mcp --bridge", quoted(&p)),
        None => "veymelo mcp --bridge".into(),
    }
}

/// The line that adds the app to Claude Code, to run in a terminal.
pub fn code_line() -> String {
    format!("claude mcp add --scope user {NAME} -- {}", command())
}

/// The server as Claude Desktop's settings hold it.
fn server(program: &Path) -> Value {
    Value::object([("command", Value::from(program.display().to_string().as_str())), ("args", Value::Array(vec![Value::from("mcp"), Value::from("--bridge")]))])
}

/// What goes in Claude Desktop's settings, under `mcpServers`, to add by hand.
pub fn desktop_snippet() -> String {
    let program = program().unwrap_or_else(|| PathBuf::from("veymelo"));
    format!("\"{NAME}\": {}", server(&program).to_json().replace(",\"", ", \"").replace("\":", "\": "))
}

/// The person's home folder.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_dir())
}

/// Where Claude Desktop keeps its settings, under `home`.
pub fn desktop_config(home: &Path) -> PathBuf {
    home.join("Library/Application Support/Claude/claude_desktop_config.json")
}

/// Claude Code's own settings (where a server added for all projects goes), under `home`.
fn code_config(home: &Path) -> PathBuf {
    home.join(".claude.json")
}

/// A version as numbers (`2.1.299`), to put versions in order.
fn version(name: &str) -> Vec<u64> {
    name.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Claude Code's command line tool: on the PATH (`path`, as the variable holds it), in the
/// usual places under `home`, else the one Claude Desktop keeps (its newest).
pub fn find_claude(home: &Path, path: Option<&str>) -> Option<PathBuf> {
    let mut places: Vec<PathBuf> = path.unwrap_or_default().split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("claude")).collect();
    for p in [".local/bin/claude", ".claude/local/claude", ".npm-global/bin/claude", ".bun/bin/claude"] {
        places.push(home.join(p));
    }
    places.extend(["/opt/homebrew/bin/claude", "/usr/local/bin/claude"].map(PathBuf::from));
    if let Some(p) = places.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    // Claude Desktop's own: claude-code/<version>/<build>/claude.app/Contents/MacOS/claude
    let root = home.join("Library/Application Support/Claude/claude-code");
    let mut versions: Vec<(Vec<u64>, PathBuf)> = std::fs::read_dir(&root).ok()?.flatten().map(|e| (version(&e.file_name().to_string_lossy()), e.path())).collect();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    versions.into_iter().find_map(|(_, dir)| std::fs::read_dir(dir).ok()?.flatten().map(|b| b.path().join("claude.app/Contents/MacOS/claude")).find(|p| p.is_file()))
}

/// How an app stands with the server: not set up, set up to run this app's tool, or set up to
/// run another (an older copy, say).
#[derive(Clone, Debug, PartialEq)]
pub enum Standing {
    No,
    Here,
    Elsewhere(String),
}

/// How the settings in `file` (a `mcpServers` object at the top) stand with the server for
/// `program`. A file that cannot be read says not set up.
pub fn standing_in(file: &Path, program: &Path) -> Standing {
    let Some(v) = std::fs::read_to_string(file).ok().and_then(|t| Value::parse(&t).ok()) else { return Standing::No };
    let Some(command) = v.get("mcpServers").and_then(|s| s.get(NAME)).map(|s| s.get("command").and_then(Value::as_str).unwrap_or_default().to_string()) else {
        return Standing::No;
    };
    if Path::new(&command) == program { Standing::Here } else { Standing::Elsewhere(command) }
}

/// How each app stands now (Claude Code, then Claude Desktop).
pub fn standing() -> [Standing; 2] {
    let (Some(home), Some(program)) = (home(), program()) else { return [Standing::No, Standing::No] };
    [standing_in(&code_config(&home), &program), standing_in(&desktop_config(&home), &program)]
}

/// Runs `claude` with `args`: what it said, or why it could not.
fn run(claude: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(claude).args(args).output().map_err(|e| format!("could not run {}: {e}", claude.display()))?;
    let said = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if out.status.success() {
        Ok(said(&out.stdout))
    } else {
        let why = [said(&out.stderr), said(&out.stdout)].into_iter().find(|s| !s.is_empty()).unwrap_or_else(|| format!("it stopped with {}", out.status));
        Err(why)
    }
}

/// Adds the server for `program` to Claude Code for all the person's projects, with its tool
/// `claude` (taking off one of the same name first, so it points here).
pub fn set_up_code(claude: &Path, program: &Path) -> Result<String, String> {
    let _ = run(claude, &["mcp", "remove", NAME, "--scope", "user"]);
    let program = program.display().to_string();
    run(claude, &["mcp", "add", "--scope", "user", NAME, "--", &program, "mcp", "--bridge"])?;
    Ok("Added to Claude Code: new sessions have Veymelo".into())
}

/// Adds the server for `program` to Claude Desktop's settings in `file`, keeping what else is
/// there, and a copy of the file as it was beside it. Gives where the copy went (none when
/// there was no file yet).
pub fn set_up_desktop(file: &Path, program: &Path) -> Result<Option<PathBuf>, String> {
    let folder = file.parent().ok_or("no folder for the settings")?;
    if !folder.is_dir() {
        return Err("Claude Desktop is not on this Mac (it has no settings folder)".into());
    }
    let old = match std::fs::read_to_string(file) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("its settings could not be read: {e}")),
    };
    let mut v = match &old {
        Some(t) if !t.trim().is_empty() => Value::parse(t).map_err(|e| format!("its settings are not JSON we can read ({e}): add it by hand"))?,
        _ => Value::Object(Map::new()),
    };
    // a number too long to read back exactly would change: left as it is, by hand
    if old.as_deref().is_some_and(long_number) {
        return Err("its settings hold a number too long to keep exactly: add it by hand".into());
    }
    let Value::Object(top) = &mut v else { return Err("its settings are not an object: add it by hand".into()) };
    if !top.get("mcpServers").is_some_and(|s| matches!(s, Value::Object(_))) {
        top.insert("mcpServers", Value::Object(Map::new()));
    }
    if let Some(Value::Object(servers)) = top.get_mut("mcpServers") {
        servers.insert(NAME, server(program));
    }
    let copy = match &old {
        Some(t) => {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let copy = folder.join(format!("claude_desktop_config.json.before-veymelo-{secs}"));
            std::fs::write(&copy, t).map_err(|e| format!("a copy of its settings could not be kept: {e}"))?;
            Some(copy)
        }
        None => None,
    };
    // written whole beside it, then put in its place
    let next = folder.join(".claude_desktop_config.json.veymelo");
    std::fs::write(&next, v.to_pretty()).map_err(|e| format!("its settings could not be written: {e}"))?;
    std::fs::rename(&next, file).map_err(|e| format!("its settings could not be written: {e}"))?;
    Ok(copy)
}

/// Whether JSON text has a number with more digits than a float keeps.
fn long_number(text: &str) -> bool {
    let (mut run, mut in_string, mut escaped) = (0, false, false);
    for c in text.chars() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                run = 0;
            }
            '0'..='9' => {
                run += 1;
                if run > 15 {
                    return true;
                }
            }
            '.' | 'e' | 'E' | '-' | '+' => {}
            _ => run = 0,
        }
    }
    false
}

/// A setting up going on or done: which app, and how it went (once it has), and when.
struct Job {
    app: App,
    done: Option<(Result<String, String>, Instant)>,
}

static JOB: Mutex<Option<Job>> = Mutex::new(None);

fn job() -> std::sync::MutexGuard<'static, Option<Job>> {
    JOB.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sets `app` up on a thread of its own (the person has said yes).
pub fn start(app: App) {
    if busy().is_some() {
        return;
    }
    *job() = Some(Job { app, done: None });
    std::thread::spawn(move || {
        let said = (|| {
            let program = program().ok_or("the app's command line tool is not beside it")?;
            let home = home().ok_or("no home folder")?;
            match app {
                App::Code => {
                    let claude = find_claude(&home, std::env::var("PATH").ok().as_deref()).ok_or("Claude Code's command line tool was not found: run the command below in a terminal")?;
                    set_up_code(&claude, &program)
                }
                App::Desktop => set_up_desktop(&desktop_config(&home), &program).map(|_| "Added to Claude Desktop: quit it and open it again to use Veymelo".to_string()),
            }
        })();
        if let Some(j) = job().as_mut() {
            j.done = Some((said, Instant::now()));
        }
    });
}

/// The app being set up now, if one is.
pub fn busy() -> Option<App> {
    job().as_ref().filter(|j| j.done.is_none()).map(|j| j.app)
}

/// How the last setting up went: the app, what it said (or why it failed), how long ago
/// (seconds).
pub fn last() -> Option<(App, Result<String, String>, f64)> {
    job().as_ref().and_then(|j| j.done.as_ref().map(|(r, at)| (j.app, r.clone(), at.elapsed().as_secs_f64())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("veymelo-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn desktop_settings_keep_what_is_there_and_a_copy() {
        let d = scratch("desktop");
        let file = d.join("claude_desktop_config.json");
        std::fs::write(&file, "{\"preferences\": {\"a\": 1}, \"mcpServers\": {\"other\": {\"command\": \"x\"}}}").unwrap();
        let program = Path::new("/Applications/Veymelo.app/Contents/MacOS/veymelo");
        let copy = set_up_desktop(&file, program).unwrap().unwrap();
        assert!(std::fs::read_to_string(&copy).unwrap().contains("preferences"));
        let v = Value::parse(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v.get("preferences").and_then(|p| p.get("a")).and_then(Value::as_f64), Some(1.0));
        assert!(v.get("mcpServers").and_then(|s| s.get("other")).is_some());
        assert_eq!(standing_in(&file, program), Standing::Here);
        assert_eq!(standing_in(&file, Path::new("/elsewhere/veymelo")), Standing::Elsewhere(program.display().to_string()));
        // again: still one, still here
        set_up_desktop(&file, program).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.matches("\"veymelo\"").count(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn desktop_settings_are_made_when_there_are_none_and_left_when_unreadable() {
        let d = scratch("fresh");
        let file = d.join("claude_desktop_config.json");
        let program = Path::new("/a/veymelo");
        assert_eq!(set_up_desktop(&file, program).unwrap(), None);
        assert_eq!(standing_in(&file, program), Standing::Here);
        std::fs::write(&file, "{ not json").unwrap();
        assert!(set_up_desktop(&file, program).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json");
        std::fs::write(&file, "{\"id\": 12345678901234567890}").unwrap();
        assert!(set_up_desktop(&file, program).is_err(), "a long number would change");
        assert!(set_up_desktop(&d.join("missing/claude_desktop_config.json"), program).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn claude_is_found_on_the_path_else_the_newest_kept_by_claude_desktop() {
        let d = scratch("find");
        let kept = |v: &str| d.join(format!("Library/Application Support/Claude/claude-code/{v}/b/claude.app/Contents/MacOS"));
        for v in ["2.1.9", "2.1.10"] {
            std::fs::create_dir_all(kept(v)).unwrap();
            std::fs::write(kept(v).join("claude"), "").unwrap();
        }
        assert_eq!(find_claude(&d, Some("/nowhere")), Some(kept("2.1.10").join("claude")));
        let bin = d.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("claude"), "").unwrap();
        assert_eq!(find_claude(&d, Some(&format!("/nowhere:{}", bin.display()))), Some(bin.join("claude")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_lines_to_copy_name_the_tool_by_its_full_path() {
        assert!(code_line().starts_with("claude mcp add --scope user veymelo -- "));
        assert!(code_line().ends_with(" mcp --bridge"));
        assert!(desktop_snippet().starts_with("\"veymelo\": {\"command\": "));
        assert_eq!(quoted(Path::new("/a b/veymelo")), "\"/a b/veymelo\"");
        assert!(!long_number("{\"a\": \"12345678901234567890\", \"b\": 1.5}"));
    }
}
