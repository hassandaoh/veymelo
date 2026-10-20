//! Many projects, each in a folder of its own wherever the person keeps it (`Name/Name.veymelo`),
//! known by a list the app keeps (most recently opened first). Each project opened is one
//! session, shared by the app and every agent working in it; each agent chooses the project it
//! works in (`project_list`, `project_open`, `project_new`), and its changes are saved to the
//! project's file as they are made.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use veymelo_engine::Session;
use veymelo_json::Value as Json;

use crate::server::call_in;
use crate::tools::{BadCall, Reply};

/// A project the list knows.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Its name (its file's, without `.veymelo`).
    pub name: String,
    /// Its file.
    pub file: PathBuf,
    /// When its file last changed.
    pub modified: Option<SystemTime>,
    /// Its file is not there (moved or deleted outside the app).
    pub missing: bool,
}

/// The projects: the list's file, and those open.
pub struct Library {
    index: PathBuf,
    open: Mutex<Vec<(PathBuf, Arc<Mutex<Session>>)>>,
}

/// Where the list of projects is kept: `$VEYMELO_LIBRARY`, or the app's folder in Application
/// Support.
pub fn default_index() -> PathBuf {
    if let Some(p) = std::env::var_os("VEYMELO_LIBRARY") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join("Library/Application Support/Veymelo/projects.json")
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A project's file from what names it: its file, or its folder (holding `Name.veymelo`).
fn file_of(path: &Path) -> PathBuf {
    if path.extension().is_some_and(|e| e == "veymelo") {
        return path.to_path_buf();
    }
    let name = path.file_name().map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned());
    path.join(format!("{name}.veymelo"))
}

/// The folder that is the project in `file`'s own (`Name/Name.veymelo`), or none when the file
/// sits in a folder of other things.
pub fn own_folder(file: &Path) -> Option<PathBuf> {
    let dir = file.parent()?;
    (dir.file_name()? == file.file_stem()?).then(|| dir.to_path_buf())
}

/// A name that can be a folder's and a file's.
fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a project needs a name".into());
    }
    if name.starts_with('.') || name.chars().any(|c| matches!(c, '/' | ':' | '\\') || c.is_control()) {
        return Err(format!("\"{name}\" cannot be a folder's name (no / : \\, and not starting with a dot)"));
    }
    Ok(name.to_string())
}

impl Library {
    pub fn new(index: PathBuf) -> Library {
        Library { index, open: Mutex::new(Vec::new()) }
    }

    /// The list's files, most recent first.
    fn files(&self) -> Vec<PathBuf> {
        let Ok(text) = std::fs::read_to_string(&self.index) else { return Vec::new() };
        let Ok(j) = Json::parse(&text) else { return Vec::new() };
        j.get("projects").and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_str).map(PathBuf::from).collect()).unwrap_or_default()
    }

    fn write(&self, files: &[PathBuf]) -> Result<(), String> {
        if let Some(dir) = self.index.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot keep the list of projects in {}: {e}", dir.display()))?;
        }
        let j = Json::object([("projects", Json::array(files.iter().map(|f| Json::from(f.display().to_string()))))]);
        std::fs::write(&self.index, j.to_pretty()).map_err(|e| format!("cannot write the list of projects: {e}"))
    }

    /// `file` first in the list.
    fn remember(&self, file: &Path) -> Result<(), String> {
        let mut files = self.files();
        files.retain(|f| f != file);
        files.insert(0, file.to_path_buf());
        self.write(&files)
    }

    /// The projects, most recently opened first.
    pub fn list(&self) -> Vec<Entry> {
        self.files()
            .into_iter()
            .map(|file| {
                let meta = std::fs::metadata(&file).ok();
                let name = file.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                Entry { name, modified: meta.as_ref().and_then(|m| m.modified().ok()), missing: meta.is_none(), file }
            })
            .collect()
    }

    /// The project a name or path means: one of the list by its name (any case), or a file or
    /// folder.
    pub fn find(&self, what: &str) -> Result<PathBuf, String> {
        let list = self.list();
        if let Some(e) = list.iter().find(|e| e.name == what).or_else(|| list.iter().find(|e| e.name.eq_ignore_ascii_case(what))) {
            return Ok(e.file.clone());
        }
        let file = file_of(Path::new(what));
        if file.exists() {
            return Ok(file);
        }
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        Err(format!("there is no project \"{what}\" (there is: {})", if names.is_empty() { "none yet".to_string() } else { names.join(", ") }))
    }

    /// The project in `file` (or its folder): its file and session, opened if it is not yet, and
    /// first in the list.
    pub fn open(&self, file: &Path) -> Result<(PathBuf, Arc<Mutex<Session>>), String> {
        let file = std::path::absolute(file_of(file)).map_err(|e| e.to_string())?;
        let found = lock(&self.open).iter().find(|(f, _)| *f == file).map(|(_, s)| s.clone());
        let session = match found {
            Some(s) => s,
            None => {
                let s = Arc::new(Mutex::new(Session::open(&file)?));
                lock(&self.open).push((file.clone(), s.clone()));
                s
            }
        };
        self.remember(&file)?;
        Ok((file, session))
    }

    /// A new project called `name` in a folder of its own inside `folder`, saved, opened and
    /// first in the list: its file and session.
    pub fn create(&self, name: &str, folder: &Path) -> Result<(PathBuf, Arc<Mutex<Session>>), String> {
        let name = check_name(name)?;
        let dir = std::path::absolute(folder.join(&name)).map_err(|e| e.to_string())?;
        let file = dir.join(format!("{name}.veymelo"));
        if file.exists() {
            return Err(format!("there is a project in {} already (open it with project_open)", dir.display()));
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
        let mut s = Session::new();
        s.run("project.save", &Json::object([("path", Json::from(file.display().to_string()))])).map_err(|e| e.message)?;
        let session = Arc::new(Mutex::new(s));
        lock(&self.open).push((file.clone(), session.clone()));
        self.remember(&file)?;
        Ok((file, session))
    }

    /// The project in `file` taken off the list, and closed (its folder stays as it is).
    pub fn forget(&self, file: &Path) -> Result<(), String> {
        let mut files = self.files();
        files.retain(|f| f != file);
        self.write(&files)?;
        lock(&self.open).retain(|(f, _)| f != file);
        Ok(())
    }

    /// The session of `file`, if it is open.
    pub fn session(&self, file: &Path) -> Option<Arc<Mutex<Session>>> {
        lock(&self.open).iter().find(|(f, _)| f == file).map(|(_, s)| s.clone())
    }
}

/// Saves a session to its file if it has changed since.
pub fn save(session: &Mutex<Session>) {
    let mut s = lock(session);
    if s.modified() && s.path.is_some() {
        let _ = s.run("project.save", &Json::Null);
    }
}

/// The projects agents work in now: each agent's number and its project's file.
static WORKING: Mutex<Vec<(u64, PathBuf)>> = Mutex::new(Vec::new());
static AGENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The files of the projects agents work in now (for whoever shows which).
pub fn working() -> Vec<PathBuf> {
    lock(&WORKING).iter().map(|(_, f)| f.clone()).collect()
}

/// An agent at work in the library: the project it has chosen.
pub struct Agent {
    pub library: Arc<Library>,
    pub current: Option<(PathBuf, Arc<Mutex<Session>>)>,
    /// Its number among the agents (for [`working`]).
    id: u64,
}

impl Drop for Agent {
    fn drop(&mut self) {
        lock(&WORKING).retain(|(a, _)| *a != self.id);
    }
}

impl Agent {
    pub fn new(library: Arc<Library>) -> Agent {
        Agent { library, current: None, id: AGENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) }
    }

    /// Work in the project in `file` (its session) from now on.
    fn choose(&mut self, file: PathBuf, session: Arc<Mutex<Session>>) {
        let mut w = lock(&WORKING);
        w.retain(|(a, _)| *a != self.id);
        w.push((self.id, file.clone()));
        self.current = Some((file, session));
    }

    /// The project tools; the others in the chosen project (saved after each).
    pub fn call(&mut self, name: &str, args: &Json) -> Result<Reply, BadCall> {
        if !crate::tools::TOOLS.iter().any(|t| t.name == name) {
            return Err(BadCall(format!("there is no tool `{name}` (there is {})", crate::tools::TOOLS.iter().map(|t| t.name).collect::<Vec<_>>().join(", "))));
        }
        let text = |k: &str| args.get(k).and_then(Json::as_str).unwrap_or("").to_string();
        match name {
            "project_list" => {
                let step = crate::activity::begin_in("", name, args);
                let current = self.current.as_ref().map(|c| c.0.clone());
                let list = Json::array(self.library.list().into_iter().map(|e| {
                    Json::object([
                        ("name", Json::from(e.name.as_str())),
                        ("file", Json::from(e.file.display().to_string())),
                        ("working", Json::Bool(current.as_deref() == Some(e.file.as_path()))),
                        ("missing", Json::Bool(e.missing)),
                    ])
                }));
                let reply = Reply::text(Json::object([("projects", list)]).to_json());
                crate::activity::end(step, true, "");
                Ok(reply)
            }
            "project_open" | "project_new" => {
                let made = if name == "project_open" {
                    self.library.find(&text("project")).and_then(|file| self.library.open(&file))
                } else {
                    self.library.create(&text("name"), Path::new(&text("folder")))
                };
                let file = made.as_ref().map(|(f, _)| f.display().to_string()).unwrap_or_default();
                let step = crate::activity::begin_in(&file, name, args);
                match made {
                    Ok((file, session)) => {
                        let comps = lock(&session).project().items.iter().filter_map(|i| i.as_comp().map(|_| Json::from(i.name.as_str()))).collect::<Vec<_>>();
                        let name = file.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                        let file = std::path::absolute(&file).unwrap_or(file);
                        self.choose(file.clone(), session);
                        crate::activity::end(step, true, "");
                        Ok(Reply::text(Json::object([("project", Json::from(name.as_str())), ("file", Json::from(file.display().to_string())), ("compositions", Json::array(comps))]).to_json()))
                    }
                    Err(why) => {
                        crate::activity::end(step, false, &why);
                        Ok(Reply::error(why))
                    }
                }
            }
            _ => {
                let Some((file, session)) = &self.current else {
                    return Ok(Reply::error("choose a project to work in first: project_list shows them, project_open opens one, project_new makes one"));
                };
                // a session here stays its project's: opening or starting another is project_open / project_new
                let command = text("command");
                if name == "command_run" && matches!(command.as_str(), "project.open" | "project.new") {
                    return Ok(Reply::error(format!("{command} would turn this project into another: use the project_open or project_new tool")));
                }
                let reply = call_in(session, &file.display().to_string(), name, args);
                save(session);
                reply
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_made_listed_opened_and_kept() {
        // (other tests' agents work elsewhere at the same time)
        let mine = |dir: &Path| working().into_iter().filter(|f| f.starts_with(dir)).collect::<Vec<_>>();
        let dir = std::env::temp_dir().join(format!("veymelo-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = Arc::new(Library::new(dir.join("projects.json")));
        let mut agent = Agent::new(lib.clone());
        let call = |a: &mut Agent, name: &str, args: &str| a.call(name, &Json::parse(args).unwrap()).unwrap();
        // nothing to work in until one is chosen
        assert!(call(&mut agent, "command_run", r#"{"command": "comp.new"}"#).is_error);
        let folder = dir.join("work").display().to_string();
        let made = call(&mut agent, "project_new", &format!(r#"{{"name": "Teaser", "folder": "{folder}"}}"#));
        assert!(!made.is_error, "{:?}", made.content);
        assert!(dir.join("work/Teaser/Teaser.veymelo").exists());
        call(&mut agent, "command_run", r#"{"command": "comp.new", "params": {"name": "Main"}}"#);
        // saved as it is made: read back from the file
        let back = Session::open(&dir.join("work/Teaser/Teaser.veymelo")).unwrap();
        assert!(back.project().items.iter().any(|i| i.name == "Main"));
        assert_eq!(mine(&dir), [dir.join("work/Teaser/Teaser.veymelo")]);
        // a second project, then back to the first by its name
        call(&mut agent, "project_new", &format!(r#"{{"name": "Second", "folder": "{folder}"}}"#));
        assert_eq!(mine(&dir), [dir.join("work/Second/Second.veymelo")], "one project an agent at a time");
        assert_eq!(lib.list().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Second", "Teaser"]);
        let opened = call(&mut agent, "project_open", r#"{"project": "teaser"}"#);
        assert!(!opened.is_error);
        assert_eq!(lib.list()[0].name, "Teaser");
        // the same session as before: one per project
        assert!(Arc::ptr_eq(&lib.session(&lib.list()[0].file).unwrap(), &agent.current.as_ref().unwrap().1));
        assert!(call(&mut agent, "project_new", &format!(r#"{{"name": "Teaser", "folder": "{folder}"}}"#)).is_error, "there already");
        assert!(call(&mut agent, "project_new", &format!(r#"{{"name": "a/b", "folder": "{folder}"}}"#)).is_error);
        assert!(call(&mut agent, "command_run", r#"{"command": "project.new"}"#).is_error);
        assert!(call(&mut agent, "project_open", r#"{"project": "Nope"}"#).is_error);
        drop(agent);
        assert!(mine(&dir).is_empty(), "an agent gone works nowhere");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
