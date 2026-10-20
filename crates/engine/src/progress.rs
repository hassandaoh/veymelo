//! Renders as they go, told to whoever watches: each running render keeps a small file in one
//! folder (its project, composition, file, and how many frames of how many are made), so the
//! app shows how far every render has come, whichever process makes it (the app's own agent,
//! `veymelo exec`, another app). A render's file goes when it ends; one left by a process that
//! stopped is no longer read once it has not changed for a while.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use veymelo_json::Value as Json;

/// How often a render tells how far it has come, at most.
const EVERY: Duration = Duration::from_millis(250);
/// A file not changed for this long belongs to a render that stopped.
const STALE: f64 = 20.0;

thread_local! {
    /// The file of the project whose command runs on this thread.
    static FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// While it lives, `file` is the project being worked on on this thread (what renders started
/// meanwhile say they belong to); the one before comes back when it goes.
pub(crate) struct Working(Option<PathBuf>);

impl Working {
    pub(crate) fn on(file: Option<&Path>) -> Working {
        Working(FILE.with(|c| c.replace(file.map(Path::to_path_buf))))
    }
}

impl Drop for Working {
    fn drop(&mut self) {
        FILE.with(|c| *c.borrow_mut() = self.0.take());
    }
}

/// The folder renders tell how far they have come in: `$VEYMELO_RENDERS`, or `Renders` in the
/// program's Application Support folder.
pub fn folder() -> PathBuf {
    if let Some(d) = std::env::var_os("VEYMELO_RENDERS") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join("Library/Application Support/Veymelo/Renders")
}

/// A render as it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Render {
    /// The project's file, if it has one.
    pub project: Option<PathBuf>,
    /// The composition's name, and the file it becomes.
    pub comp: String,
    pub out: String,
    pub done: u64,
    pub total: u64,
    /// When it started, and when it last told (seconds since 1970).
    pub started: f64,
    pub updated: f64,
}

impl Render {
    /// How far it has come, 0 to 1.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 { 0.0 } else { (self.done as f64 / self.total as f64).clamp(0.0, 1.0) }
    }

    /// The project's name (its file's, without `.veymelo`), or the file it becomes.
    pub fn name(&self) -> String {
        match &self.project {
            Some(p) => p.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
            None => Path::new(&self.out).file_name().map_or_else(|| self.out.clone(), |s| s.to_string_lossy().into_owned()),
        }
    }

    fn to_json(&self) -> Json {
        Json::object([
            ("project", self.project.as_ref().map_or(Json::Null, |p| Json::from(p.display().to_string()))),
            ("comp", Json::from(self.comp.as_str())),
            ("out", Json::from(self.out.as_str())),
            ("done", Json::Number(self.done as f64)),
            ("total", Json::Number(self.total as f64)),
            ("started", Json::Number(self.started)),
            ("updated", Json::Number(self.updated)),
        ])
    }

    fn from_json(j: &Json) -> Option<Render> {
        Some(Render {
            project: j.get("project").and_then(Json::as_str).map(PathBuf::from),
            comp: j.get("comp")?.as_str()?.to_string(),
            out: j.get("out")?.as_str()?.to_string(),
            done: j.get("done")?.as_u64()?,
            total: j.get("total")?.as_u64()?,
            started: j.get("started")?.as_f64()?,
            updated: j.get("updated")?.as_f64()?,
        })
    }
}

fn now() -> f64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// The renders going now, the oldest first.
pub fn renders() -> Vec<Render> {
    renders_in(&folder())
}

fn renders_in(dir: &Path) -> Vec<Render> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let t = now();
    let mut out: Vec<Render> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| Render::from_json(&Json::parse(&std::fs::read_to_string(e.path()).ok()?).ok()?))
        .filter(|r| t - r.updated < STALE)
        .collect();
    out.sort_by(|a, b| a.started.total_cmp(&b.started));
    out
}

/// One render telling how far it has come, in its own file, gone when it is dropped.
pub struct Reporter {
    path: PathBuf,
    render: Render,
    told: Option<Instant>,
}

impl Reporter {
    /// A render of composition `comp` to `out` starting, in the project being worked on.
    pub(crate) fn start(comp: &str, out: &str) -> Reporter {
        let project = FILE.with(|c| c.borrow().clone());
        Reporter::start_in(&folder(), project, comp, out)
    }

    /// A render of composition `comp` of the project in `project` to `out` starting (one made
    /// outside a command: the app's own).
    pub fn begin(project: Option<&Path>, comp: &str, out: &str) -> Reporter {
        Reporter::start_in(&folder(), project.map(Path::to_path_buf), comp, out)
    }

    fn start_in(dir: &Path, project: Option<PathBuf>, comp: &str, out: &str) -> Reporter {
        let t = now();
        let id = format!("{}-{}", std::process::id(), (t * 1000.0) as u64);
        let render = Render { project, comp: comp.to_string(), out: out.to_string(), done: 0, total: 0, started: t, updated: t };
        let mut r = Reporter { path: dir.join(format!("{id}.json")), render, told: None };
        r.write();
        r
    }

    /// `done` frames of `total` are made (written at most every quarter second, and at once when
    /// the count of frames becomes known and at the end).
    pub fn tick(&mut self, done: u64, total: u64) {
        let known = self.render.total != total;
        self.render.done = done;
        self.render.total = total;
        if known || self.told.is_none_or(|w| w.elapsed() >= EVERY) || done >= total {
            self.write();
        }
    }

    fn write(&mut self) {
        self.render.updated = now();
        self.told = Some(Instant::now());
        // written beside it, then put in its place: a reader never sees half a file
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let part = self.path.with_extension("part");
        if std::fs::write(&part, self.render.to_json().to_json()).is_ok() {
            let _ = std::fs::rename(&part, &self.path);
        }
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_render_tells_how_far_it_has_come_and_goes_when_done() {
        let dir = std::env::temp_dir().join(format!("veymelo-renders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let project = Path::new("/somewhere/Shot/Shot.veymelo");
        {
            let _working = Working::on(Some(project));
            let mut r = Reporter::start_in(&dir, FILE.with(|c| c.borrow().clone()), "Main", "renders/shot.mp4");
            r.tick(30, 120);
            let seen = renders_in(&dir);
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].project.as_deref(), Some(project));
            assert_eq!(seen[0].name(), "Shot");
            assert_eq!((seen[0].done, seen[0].total), (30, 120));
            assert!((seen[0].fraction() - 0.25).abs() < 1e-9);
            // told again only after a while, except at the end
            r.tick(31, 120);
            assert_eq!(renders_in(&dir)[0].done, 30);
            r.tick(120, 120);
            assert_eq!(renders_in(&dir)[0].done, 120);
        }
        assert!(renders_in(&dir).is_empty(), "a render's file goes when it ends");
        assert_eq!(FILE.with(|c| c.borrow().clone()), None, "the project worked on before comes back");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
