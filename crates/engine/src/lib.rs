//! The session: one project, worked on through named commands.
//!
//! Every action is a command with an id (`comp.new`, `layer.add`, `prop.set`) and checked
//! parameters, the same for an agent over MCP, a script, the command line or the app. A command
//! that changes the project works on a copy: when it fails nothing has changed, when it succeeds
//! the copy becomes the current state, one step of the undo history. A batch runs several
//! commands as one step, later ones using what earlier ones gave back (`"$1.layer"`).

mod commands;
mod history;
mod params;
pub mod progress;

use std::path::PathBuf;
use std::sync::Arc;

use veymelo_json::Value as Json;
use veymelo_project::{Catalog, Project};
use veymelo_time::Tick;

pub use commands::{Command, Run, all as commands, find as command};
pub use history::{Entry, FIRST, History};
pub use params::{Args, Param, Ty, nearest, schema};
pub use veymelo_render as render;
pub use veymelo_project as project;

/// What kind of thing went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// No command has that id.
    UnknownCommand,
    /// A parameter is missing, unknown or of the wrong type.
    Params,
    /// What a parameter names is not there (a layer, a property, a file).
    NotFound,
    /// It cannot be done (removing a layer another one needs, undoing past the start).
    Refused,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

impl Error {
    pub fn params(message: impl Into<String>) -> Error {
        Error { kind: ErrorKind::Params, message: message.into() }
    }

    pub fn not_found(message: impl Into<String>) -> Error {
        Error { kind: ErrorKind::NotFound, message: message.into() }
    }

    pub fn refused(message: impl Into<String>) -> Error {
        Error { kind: ErrorKind::Refused, message: message.into() }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Where the work is: the composition commands act on when they are not told one, and the time
/// they use when not given one. Not part of the project, and not undone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub comp: Option<u64>,
    pub time: Tick,
}

/// What a command that only looks is given.
pub struct Look<'a> {
    pub project: &'a Project,
    pub view: &'a View,
    pub catalog: &'a Catalog,
}

/// What a command that changes the project is given: a copy to change, kept only if it succeeds.
pub struct Ctx<'a> {
    pub project: &'a mut Project,
    pub view: &'a mut View,
    pub catalog: &'a Catalog,
    /// The name of the undo step, when the command's own label is not enough.
    pub label: Option<String>,
    /// Steps with the same key, one after another, become one (a value dragged in the app).
    pub merge: Option<String>,
}

impl Ctx<'_> {
    pub fn look(&self) -> Look<'_> {
        Look { project: self.project, view: self.view, catalog: self.catalog }
    }
}

/// One project, its history, and where the work is.
pub struct Session {
    pub catalog: Catalog,
    history: History,
    pub view: View,
    /// The file the project was opened from or last saved to.
    pub path: Option<PathBuf>,
    /// The history state that is in that file.
    saved: Option<u64>,
    revision: u64,
}

impl Default for Session {
    fn default() -> Session {
        Session::new()
    }
}

impl Session {
    /// A session with an empty project.
    pub fn new() -> Session {
        Session::with(Project::new(), None)
    }

    /// A session with `project` (from the file at `path`).
    pub fn with(mut project: Project, path: Option<PathBuf>) -> Session {
        // expressions worked out here read pictures as drawing does
        veymelo_render::show_pictures();
        // the composition it was saved showing is where the work is, not part of the project
        let shown = project.shown.take();
        let history = History::new(Arc::new(project));
        let saved = Some(history.current());
        let mut s = Session { catalog: catalog(), history, view: View { comp: shown, ..View::default() }, path, saved, revision: 0 };
        s.settle_view();
        s
    }

    /// A session with the project in the file at `path`.
    pub fn open(path: &std::path::Path) -> std::result::Result<Session, String> {
        let project = Project::open(path, &catalog())?;
        Ok(Session::with(project, Some(path.to_path_buf())))
    }

    pub fn project(&self) -> &Project {
        self.history.project()
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub(crate) fn history_mut(&mut self) -> &mut History {
        &mut self.history
    }

    /// Goes up by one with every change to the project or the view (for whoever shows them).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether the project differs from its file.
    pub fn modified(&self) -> bool {
        self.saved != Some(self.history.current())
    }

    pub(crate) fn mark_saved(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.saved = Some(self.history.current());
    }

    /// Start over with `project`, its history new.
    pub(crate) fn replace(&mut self, project: Project, path: Option<PathBuf>) {
        let revision = self.revision;
        *self = Session::with(project, path);
        self.revision = revision + 1;
    }

    pub(crate) fn changed(&mut self) {
        self.settle_view();
        self.revision += 1;
    }

    /// The view's composition is one that is there (after an undo took it away, the first one).
    fn settle_view(&mut self) {
        let p = self.history.project();
        if self.view.comp.is_none_or(|c| p.comp(c).is_none()) {
            self.view.comp = p.items.iter().find(|i| i.as_comp().is_some()).map(|i| i.id);
        }
    }

    /// The project as it is saved: as it is, showing the composition shown now.
    pub fn to_save(&self) -> Project {
        let mut p = self.project().clone();
        p.shown = self.view.comp;
        p
    }

    /// Run command `id` with `params` (a JSON object, or null for none).
    pub fn run(&mut self, id: &str, params: &Json) -> Result<Json> {
        let cmd = commands::find(id).ok_or_else(|| unknown(id))?;
        // what it renders says which project it belongs to
        let _working = progress::Working::on(self.path.as_deref());
        let args = Args::check(cmd.id, cmd.params, params)?;
        match cmd.run {
            Run::Look(f) => f(&Look { project: self.history.project(), view: &self.view, catalog: &self.catalog }, &args),
            Run::Session(f) => f(self, &args),
            Run::Edit(f) => {
                let mut project = (**self.history.project()).clone();
                let mut view = self.view.clone();
                let mut cx = Ctx { project: &mut project, view: &mut view, catalog: &self.catalog, label: None, merge: None };
                let out = f(&mut cx, &args)?;
                let (label, merge) = (cx.label.take().unwrap_or_else(|| cmd.label.to_string()), cx.merge.take());
                self.commit(&label, project, view, merge.as_deref())?;
                Ok(out)
            }
        }
    }

    /// Keep a changed copy of the project as the next state (if it did change), and the view.
    pub(crate) fn commit(&mut self, label: &str, mut project: Project, view: View, merge: Option<&str>) -> Result<()> {
        // a composition's layers share one light for their styles
        veymelo_project::styles::share_global_light(self.history.project(), &mut project);
        // layers showing a composition keep its essential properties, and what they set
        veymelo_project::essential::sync(self.history.project(), &mut project);
        if let Some(p) = project.problems().into_iter().next() {
            // a command that leaves the project broken is a mistake in the command: refuse it
            return Err(Error::refused(format!("that would break the project ({p}); nothing was changed")));
        }
        let project_changed = project != **self.history.project();
        if project_changed {
            self.history.record(label, Arc::new(project), merge);
        }
        if project_changed || view != self.view {
            self.view = view;
            self.changed();
        }
        Ok(())
    }
}

impl Session {
    /// Draw a frame: of composition `comp` (default: the view's) at `time` (default: the view's).
    /// A picture of a composition's sound (PNG) and how loud it is, for an agent that cannot
    /// hear: `params` as `audio.waveform` takes them, without a path.
    pub fn hear(&self, params: &Json) -> Result<(Vec<u8>, Json)> {
        let a = Args::check("audio_preview", &commands::audio::PICTURE, params)?;
        let look = Look { project: self.project(), view: &self.view, catalog: &self.catalog };
        commands::audio::picture(&look, &a)
    }

    pub fn render(&self, comp: Option<&Json>, time: Option<Tick>, options: &veymelo_render::Options) -> Result<veymelo_render::Rendered> {
        let look = Look { project: self.project(), view: &self.view, catalog: &self.catalog };
        let cid = match comp {
            Some(c) => commands::find_comp(look.project, c)?,
            None => self.view.comp.ok_or_else(|| Error::not_found("there is no composition yet (comp.new makes one)"))?,
        };
        veymelo_render::render(look.project, cid, time.unwrap_or(self.view.time), options).map_err(Error::params)
    }
}

/// Every type of instance there is (masks now; effects and the rest as they come).
pub fn catalog() -> Catalog {
    let mut c = Catalog::base();
    veymelo_effects::register(&mut c);
    c
}

/// The error for a command id that is not one, with the nearest one.
fn unknown(id: &str) -> Error {
    let hint = nearest(id, commands::all().iter().map(|c| c.id)).map(|n| format!(" Did you mean `{n}`?")).unwrap_or_default();
    Error { kind: ErrorKind::UnknownCommand, message: format!("there is no command `{id}`.{hint} (`command.list` lists them all.)") }
}

#[cfg(test)]
mod tests;
