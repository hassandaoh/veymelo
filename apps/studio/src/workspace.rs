//! The page after the opening: the person's projects, on the opening's dotted paper.
//!
//! At the top, a bar of how things are: the app's name at the left; at the right, the renders
//! going on elsewhere and the AI's status in hairlines (an empty ring, "AI not connected" and
//! "Connect"; or a ring with an ink dot, "AI connected"), a faint hairline under it. Pressing
//! the status opens the card that connects an AI (`connect`) under it. Under the bar, "Projects"
//! with how many, and at the right two buttons in hairlines: "Open…" with a folder, and
//! "New project" with a plus.
//!
//! With none yet: three blank plates drawn in hairline strokes and "No projects yet" under
//! them; while no AI is connected, the card that connects one lies under them, the first thing
//! to do. When the pointer comes near the plates, the top plate lifts (a new sheet is ready).
//!
//! With some: each one a plate on a grid, the most recently opened first, a frame of it lying
//! on the plate, its name and when it last changed and how long it is under it, and "AI is
//! working" while an agent works in it; after them a plate in dashes with a plus makes a new
//! one. The plate under the pointer lifts; pressing one goes into it.
//!
//! "New project" asks the system where: its save panel takes a name and a folder (the project
//! gets a folder of its own there); "Open…" takes a project folder or file that is not in the
//! list. A plate under the pointer shows a small bin at its corner: pressing it asks whether to
//! take the project off the list only, or also to move it to the Trash.
//!
//! A render shows where it belongs: on its project's plate, a pen going round the edge as far
//! as it has come (its picture faint until it is made), how far and how long is left under the
//! name. One whose project is not on the page (made from the command line, or another
//! project's while inside one) shows in a small pill of paper at the top right. What is going on
//! now (a render, the AI at work) is marked with the pen's dot, as on the icon.
//!
//! Inside a project, a black "Render" button at the top right of the preview makes the
//! composition shown into a video: the system asks where (the project's `renders` folder to
//! start with), then a pen draws a line along the button as far as it has come, with how long
//! is left and a cross to stop it; done, it says at the bottom how long it took. Beside it
//! always, a round button with a folder shows the newest render in Finder (the project's
//! folder before there is one); a green dot on it says a render is made that has not been
//! looked at, and under the pointer it says which file it shows. Going in, the page fades and the project's name joins the app's name at the top left;
//! inside, a round button with a chevron (or Escape) comes back out.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;

use crate::button::{self, Button, Icon, Label, Rect};
use crate::connect::{self, Connect, Place};
use crate::faces;
use crate::iso::{self, Plate};
use crate::motion::{reach, settle, smooth, Spring};
use crate::watch::Watch;
use veymelo_font::Font;
use veymelo_mcp::activity::{self, State};
use veymelo_mcp::library::Entry;
use veymelo_path::Path;
use veymelo_raster::{Canvas, Rgb};

const PAPER: Rgb = Rgb(255, 255, 255);
const INK: Rgb = Rgb(10, 10, 10);
const GREY: Rgb = Rgb(118, 118, 122);
const FAINT: Rgb = Rgb(168, 168, 174);
const RED: Rgb = Rgb(196, 43, 43);
/// The pen's dot (as on the icon): what is going on now, a render or the AI at work.
pub const PEN: Rgb = Rgb(240, 84, 44);
/// What is done (a render made, not looked at yet).
const DONE: Rgb = Rgb(36, 150, 84);
/// The plates' silhouettes, their creases, and a silhouette when lifted.
const EDGE: Rgb = Rgb(161, 161, 170);
const CREASE: Rgb = Rgb(218, 218, 223);
const LIT: Rgb = Rgb(40, 40, 46);

/// The plates of the empty page: their shape, how many, the space between them, how high the
/// top one lifts when pointed at, and how far the pointer is felt from their middle (points).
const PLATE: Plate = Plate { w: 190.0, d: 130.0, round: 14.0, thick: 5.0 };
const PLATES: usize = 3;
const PLATE_GAP: f64 = 14.0;
const LIFT: f64 = 22.0;
const FIGURE_REACH: f64 = 210.0;
/// A project's plate on the grid, the room each takes (points), and how high it lifts.
const CARD_PLATE: Plate = Plate { w: 118.0, d: 82.0, round: 10.0, thick: 4.0 };
const CARD: (f64, f64) = (220.0, 210.0);
const CARD_GAP: f64 = 20.0;
const CARD_LIFT: f64 = 14.0;
/// Where the grid starts (points from the top).
const GRID_TOP: f64 = 140.0;
/// The bar at the top: the middle of its line (the app's name sits on it), and the hairline
/// under it; the row under it with the page's title and its buttons (points from the top).
const BAR_MIDDLE: f64 = 34.0;
const BAR_RULE: f64 = 62.0;
const TITLE_BASE: f64 = 108.0;
const ROW_TOP: f64 = 84.0;
/// The faint hairline under the bar.
const BAR_LINE: Rgb = Rgb(234, 234, 238);
/// How wide the card that connects an AI is (points).
const CONNECT_WIDE: f64 = 400.0;

const WORDS_TEXT: &str = "No projects yet";
/// The Render button's height, the round Show in Finder button's size, and the room between.
const RENDER_HEIGHT: f64 = 38.0;
const FINDER: f64 = 30.0;
const FINDER_GAP: f64 = 10.0;
const BUTTON_TEXT: &str = "New project";
const OPEN_TEXT: &str = "Open…";
/// The way back out: its place and size (points), and how far the app's name moves over for it.
const BACK: Rect = (24.0, 21.0, 30.0, 30.0);
const BACK_ROOM: f64 = 42.0;

/// Words set once.
struct Texts {
    render: Label,
    pill: Font,
    words: Label,
    button: Label,
    open: Label,
    title: Label,
    count: Font,
    status: Font,
    ghost: Label,
    slash: Label,
    crumb: Font,
    name: Font,
    small: Font,
}

fn texts() -> Option<&'static Texts> {
    static TEXTS: OnceLock<Option<Texts>> = OnceLock::new();
    TEXTS
        .get_or_init(|| {
            Some(Texts {
                render: Label::new(faces::sans_at(560.0, 14.0)?, 14.0, "Render"),
                pill: faces::sans_at(560.0, 13.0)?,
                words: Label::new(faces::sans_at(420.0, 15.0)?, 15.0, WORDS_TEXT),
                button: Label::new(faces::sans_at(540.0, 13.0)?, 13.0, BUTTON_TEXT),
                open: Label::new(faces::sans_at(500.0, 13.0)?, 13.0, OPEN_TEXT),
                title: Label::new(faces::sans_at(640.0, 22.0)?, 22.0, "Projects"),
                count: faces::sans_at(430.0, 16.0)?,
                status: faces::sans_at(470.0, 12.0)?,
                ghost: Label::new(faces::sans_at(500.0, 13.0)?, 13.0, BUTTON_TEXT),
                slash: Label::new(faces::sans_at(300.0, 18.0)?, 18.0, "/"),
                crumb: faces::sans_at(520.0, 16.0)?,
                name: faces::sans_at(560.0, 14.0)?,
                small: faces::sans_at(430.0, 12.0)?,
            })
        })
        .as_ref()
}

/// What the page is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// The projects.
    Home,
    /// Gone into one.
    Inside,
}

/// The pointer over the page: the usual arrow, or a hand over something to press.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cursor {
    Arrow,
    Hand,
    /// Over an edge that moves up and down.
    UpDown,
}

/// What the window has to ask the system for (outside the page, which waits for the answer).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ask {
    /// Where the composition shown is rendered to.
    RenderTo,
    /// A name and a folder for a new project.
    NewProject,
    /// A project folder or file to open.
    OpenProject,
    /// What to do with the project being deleted ([`Workspace::deleting`]).
    DeleteProject,
    /// A file to show in Finder ([`Workspace::revealing`]): asked of the system outside the
    /// page, as Finder may make the window draw meanwhile.
    Reveal,
    /// Words to put on the clipboard ([`Workspace::copying`]).
    Copy,
    /// A file to open in its own app ([`Workspace::opening`]).
    OpenFile,
    /// Whether to set an AI app up to reach this one ([`Workspace::setting_up`]): the window
    /// asks, and starts it on a yes.
    SetUp,
}

/// What the menus ask of the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    NewProject,
    Open,
    /// Render the composition shown (inside a project).
    Render,
    /// Back out to the projects.
    Back,
    ZoomIn,
    ZoomOut,
    /// The whole of the timeline.
    ZoomAll,
    /// Fold the AI's steps away, or bring them back (inside a project).
    TogglePanel,
    /// Show the newest render in Finder (inside a project).
    Reveal,
    /// Fold the project's files away, or bring them back (inside a project).
    ToggleFiles,
    /// The composition's settings (inside a project).
    Settings,
}

/// The answer to deleting a project.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Delete {
    /// Off the list only: its folder stays where it is.
    Remove,
    /// Off the list, and its folder (or file) moved to the Trash.
    Trash,
    Cancel,
}

/// A project on the grid, and how far its plate is lifted.
struct Card {
    entry: Entry,
    lift: Spring,
}

/// Where things are on a page `size` points big.
struct Layout {
    /// The empty page: where the room's middle is (the plates stand on it), the plates' middle,
    /// the words' baseline, and where the card that connects an AI lies under them.
    origin: (f64, f64),
    figure: (f64, f64),
    words_baseline: f64,
    lying: (f64, f64),
    /// "New project" and "Open…", at the right of the title's row.
    button: Rect,
    open: Rect,
    /// The grid: how many across, and where it starts.
    columns: usize,
    left: f64,
}

/// The layout of a page `size` points big; `lying` when the card that connects an AI lies
/// under the plates of the empty page (it takes room).
fn layout(size: (f64, f64), t: &Texts, lying: bool) -> Layout {
    let (wp, hp) = size;
    // the empty page's plates, words and card as a block in the room under the title's row
    let block = if lying { 140.0 + 118.0 + 26.0 + 330.0 } else { 140.0 + 130.0 + 40.0 };
    let start = GRID_TOP - 10.0 + ((hp - GRID_TOP - 20.0 - block) / 2.0).max(0.0) * 0.8;
    let origin = (wp / 2.0, start + 140.0);
    let figure = (origin.0, origin.1 - 16.0);
    let words_baseline = origin.1 + if lying { 118.0 } else { 130.0 };
    let lying = ((wp - CONNECT_WIDE) / 2.0, words_baseline + 26.0);
    let w = button::line_wide(&t.button, Icon::Plus);
    let button = (wp - 28.0 - w, ROW_TOP, w, button::LINE_HEIGHT);
    let ow = button::line_wide(&t.open, Icon::Folder);
    let open = (button.0 - 10.0 - ow, ROW_TOP, ow, button::LINE_HEIGHT);
    let columns = (((wp - 56.0 + CARD_GAP) / (CARD.0 + CARD_GAP)).floor() as usize).max(1);
    let left = (wp - (columns as f64 * (CARD.0 + CARD_GAP) - CARD_GAP)) / 2.0;
    Layout { origin, figure, words_baseline, lying, button, open, columns, left }
}

/// The AI's status at the right of the bar on a page `size` points big: its room (to press),
/// where its ring's middle is, and whether an AI is connected.
fn status(size: (f64, f64), t: &Texts) -> (Rect, (f64, f64), bool) {
    let here = veymelo_mcp::bridged() > 0;
    let words = if here { "AI connected" } else { "AI not connected" };
    let mut wide = 10.0 + 7.0 + Label::new(t.status.clone(), 12.0, words).wide;
    if !here {
        wide += 8.0 + Label::new(t.status.clone(), 12.0, "Connect").wide;
    }
    let x = size.0 - 28.0 - wide;
    ((x - 10.0, BAR_MIDDLE - 15.0, wide + 20.0, 30.0), (x + 5.0, BAR_MIDDLE), here)
}

/// Card `i`'s room on the grid (points).
fn card_rect(at: &Layout, i: usize) -> Rect {
    let (col, row) = (i % at.columns, i / at.columns);
    (at.left + col as f64 * (CARD.0 + CARD_GAP), GRID_TOP + row as f64 * (CARD.1 + CARD_GAP), CARD.0, CARD.1)
}

/// The bin at card `i`'s top right corner (points).
fn bin_rect(at: &Layout, i: usize) -> Rect {
    let (x, y, w, _) = card_rect(at, i);
    (x + w - 38.0, y + 6.0, 30.0, 30.0)
}

/// How long ago a time was, in words.
fn ago(when: Option<SystemTime>) -> String {
    let Some(secs) = when.and_then(|w| SystemTime::now().duration_since(w).ok()).map(|d| d.as_secs()) else { return String::new() };
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

/// A length as minutes and seconds (`1:39`), or seconds when under a minute (`12 s`).
fn length(secs: f64) -> String {
    let s = secs.round() as u64;
    if s < 60 { format!("{s} s") } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// Whether an agent works in the project in `file` now (it chose it, and is still there).
fn working(file: &std::path::Path) -> bool {
    veymelo_mcp::library::working().iter().any(|f| f == file)
}

pub struct Workspace {
    pub mode: Mode,
    /// Inside: the composition's settings card, the pencil that opens it (and its line), and
    /// whether the last press was the card's.
    settings: crate::settings::Settings,
    pencil: Option<Rect>,
    about_line: Option<Rect>,
    card_press: bool,
    /// Inside: the Render button, and how wide it is now (it grows and shrinks smoothly).
    render: Button,
    render_w: Spring,
    /// How near the pointer is to the round Show in Finder button (0 to 1), and the render the
    /// person has already been shown (so its dot goes).
    finder_near: Spring,
    finder_seen: Option<PathBuf>,
    /// Whether the mouse went down on the cross that stops a render.
    stop_held: bool,
    /// The project inside: its name.
    pub name: String,
    pub button: Button,
    /// When the page started to come in, and when the mode began.
    shown: Option<f64>,
    since: f64,
    /// How far the top plate of the empty page is lifted (0 to 1).
    lift: Spring,
    /// The projects, as last read, and when.
    cards: Vec<Card>,
    read: f64,
    /// What the window has to ask the system for.
    ask: Option<Ask>,
    /// Something that went wrong, and when (shown a moment at the bottom).
    trouble: Option<(String, f64)>,
    /// What was done, and when (shown a moment at the bottom, in grey).
    notice: Option<(String, f64)>,
    /// The project being deleted, while the system asks what to do with it.
    deleting: Option<Entry>,
    /// A file to show in Finder.
    revealing: Option<PathBuf>,
    /// Words to put on the clipboard.
    copying: Option<String>,
    /// A file to open in its own app.
    opening: Option<PathBuf>,
    /// The card that connects an AI (hanging under the status, or lying in the empty page),
    /// how near the pointer is to the status, and the app the window is to ask to set up.
    connect: Connect,
    was_lying: bool,
    status_near: Spring,
    setting_up: Option<crate::setup::App>,
    /// "Open…" (to press), how near the pointer is to "New project", and how far the plate in
    /// dashes after the projects is lifted.
    open: Button,
    new_near: Spring,
    ghost: Spring,
    /// The renders going on, and their count when the window last showed them.
    renders: Vec<veymelo_engine::progress::Render>,
    renders_seen: u64,
    /// Inside the project: the preview and what the AI does.
    watch: Watch,
    /// How far the app's name has moved over for the way back (0 to 1), how near the pointer is
    /// to that button and to "Open…" (0 to 1), and where the pointer is.
    shift: Spring,
    back_near: Spring,
    open_near: Spring,
    pointer: Option<(f64, f64)>,
    /// The page's size (points), as last laid out.
    size: (f64, f64),
    /// What the page showed last (to know when it has to draw again).
    seen: (u64, Option<State>, usize, u64),
}

impl Workspace {
    pub const fn new() -> Workspace {
        Workspace {
            mode: Mode::Home,
            settings: crate::settings::Settings::new(),
            pencil: None,
            about_line: None,
            card_press: false,
            render: Button::new(),
            render_w: Spring::new(),
            finder_near: Spring::new(),
            finder_seen: None,
            stop_held: false,
            name: String::new(),
            button: Button::new(),
            shown: None,
            since: 0.0,
            lift: Spring::new(),
            cards: Vec::new(),
            read: f64::NEG_INFINITY,
            ask: None,
            trouble: None,
            notice: None,
            deleting: None,
            revealing: None,
            copying: None,
            opening: None,
            connect: Connect::new(),
            was_lying: false,
            status_near: Spring::new(),
            setting_up: None,
            open: Button::new(),
            new_near: Spring::new(),
            ghost: Spring::new(),
            renders: Vec::new(),
            renders_seen: 0,
            watch: Watch::new(),
            shift: Spring::new(),
            back_near: Spring::new(),
            open_near: Spring::new(),
            pointer: None,
            size: (0.0, 0.0),
            seen: (0, None, 0, 0),
        }
    }

    /// The page starts to come in at `t` (once).
    pub fn show(&mut self, t: f64) {
        self.shown.get_or_insert(t);
        self.read = f64::NEG_INFINITY;
    }

    fn age(&self, t: f64) -> f64 {
        self.shown.map_or(-1.0, |s| t - s)
    }

    /// The projects read again (a moment after the last time, or now).
    fn reread(&mut self, t: f64, now: bool) {
        if !now && t - self.read < 1.0 {
            return;
        }
        self.read = t;
        let list = crate::live::library().list();
        let old = std::mem::take(&mut self.cards);
        self.cards = list
            .into_iter()
            .filter(|e| !e.missing)
            .map(|entry| {
                let lift = old.iter().find(|c| c.entry.file == entry.file).map_or(Spring::new(), |c| c.lift);
                Card { entry, lift }
            })
            .collect();
    }

    /// Go into the project in `file` at `t`.
    fn go_in(&mut self, file: &std::path::Path, t: f64) {
        match crate::live::show(file) {
            Ok(name) => {
                self.name = name;
                self.mode = Mode::Inside;
                self.since = t;
                self.watch = Watch::new();
            }
            Err(why) => self.trouble = Some((why, t)),
        }
    }

    /// A project opened from Finder at `t` (double-clicked, opened with Veymelo, or let go on
    /// its icon in the Dock): shown, from the page or in place of the project shown.
    pub fn opened(&mut self, paths: &[PathBuf], t: f64) {
        let Some(file) = paths.first() else { return };
        if self.mode == Mode::Inside {
            if crate::live::shown().is_some_and(|(shown, _)| std::path::absolute(file).is_ok_and(|f| f == shown)) {
                return;
            }
            self.back(t);
        }
        self.go_in(file, t);
        self.reread(t, true);
    }

    /// Back out of the project to the page, at `t`.
    pub fn back(&mut self, t: f64) {
        if self.mode == Mode::Inside {
            self.finder_seen = self.unseen_render().or(self.finder_seen.take());
            self.mode = Mode::Home;
            self.since = t;
            self.watch.leave();
            crate::live::hide();
            self.reread(t, true);
        }
    }

    /// What the window has to ask the system for, taken (it asks, then gives [`Workspace::answer`]).
    pub fn take_ask(&mut self) -> Option<Ask> {
        self.ask.take()
    }

    /// What the render will be called to start with, and the folder it goes in: the composition
    /// shown, in the project's `renders` folder.
    pub fn render_defaults(&self) -> Option<(String, PathBuf)> {
        let (file, session) = crate::live::shown()?;
        let s = session.try_lock().ok()?;
        let comp = s.view.comp?;
        let name = s.project().item(comp).map(|i| i.name.clone()).unwrap_or_else(|| "Render".into());
        Some((format!("{name}.mp4"), file.parent()?.join("renders")))
    }

    /// Starts rendering the composition shown to `out`, at `t`.
    fn start_render(&mut self, mut out: PathBuf, t: f64) {
        if out.extension().is_none() {
            out.set_extension("mp4");
        }
        let Some((file, session)) = crate::live::shown() else { return };
        // the project as it is now (waiting a moment if the AI is in the middle of a step)
        let mut snapshot = None;
        for _ in 0..50 {
            if let Ok(s) = session.try_lock() {
                snapshot = s.view.comp.map(|c| (s.history().project().clone(), c, s.project().item(c).map(|i| i.name.clone()).unwrap_or_default()));
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let Some((project, comp, name)) = snapshot else {
            self.trouble = Some(("The project is busy with the AI's step: press Render again in a moment".into(), t));
            return;
        };
        if let Err(why) = crate::export::start(project, Some(file), comp, &name, out) {
            self.trouble = Some((why, t));
        }
    }

    /// The Render button's place inside a project, as wide as it is now.
    fn render_rect(&self) -> Rect {
        let right = self.size.0 - 28.0;
        let wide = self.render_w.x.max(1.0);
        (right - wide, (crate::watch::BAR - RENDER_HEIGHT) / 2.0 + 1.0, wide, RENDER_HEIGHT)
    }

    /// The round Show in Finder button's place: before the Render button.
    fn finder_rect(&self) -> Rect {
        let (x, y, _, h) = self.render_rect();
        (x - FINDER_GAP - FINDER, y + (h - FINDER) / 2.0, FINDER, FINDER)
    }

    /// What Show in Finder shows: the newest file in the project's `renders` folder (or the one
    /// the app has just made, wherever it went), else the project's folder.
    fn finder_target(&self) -> Option<PathBuf> {
        let (file, _) = crate::live::shown()?;
        let folder = file.parent()?.to_path_buf();
        let newest = |dir: &std::path::Path| -> Option<(SystemTime, PathBuf)> {
            std::fs::read_dir(dir)
                .ok()?
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && !e.file_name().to_string_lossy().starts_with('.'))
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .max_by(|a, b| a.0.cmp(&b.0))
        };
        let mut best = newest(&folder.join("renders"));
        if let Some(crate::export::State::Made(out, ..)) = self.render_state()
            && let Some(when) = std::fs::metadata(&out).ok().and_then(|m| m.modified().ok())
            && best.as_ref().is_none_or(|b| when >= b.0)
        {
            best = Some((when, out));
        }
        Some(best.map_or(folder, |b| b.1))
    }

    /// A render the app has made of this project that the person has not been shown yet.
    fn unseen_render(&self) -> Option<PathBuf> {
        match self.render_state() {
            Some(crate::export::State::Made(out, ..)) if self.finder_seen.as_ref() != Some(&out) => Some(out),
            _ => None,
        }
    }

    /// How the render of the project shown stands.
    fn render_state(&self) -> Option<crate::export::State> {
        crate::live::shown().and_then(|(f, _)| crate::export::state(&f))
    }

    /// How wide the Render button is to be (it grows while it shows how far it has come).
    fn render_wide(&self, tx: &Texts) -> f64 {
        match self.render_state() {
            Some(crate::export::State::Going(..)) => 220.0,
            _ => tx.render.wide + 2.0 * 24.0,
        }
    }

    /// Where the cross that stops a render is (points).
    fn stop_rect(&self) -> Rect {
        let (x, y, w, h) = self.render_rect();
        (x + w - h, y, h, h)
    }

    /// Show in Finder: what is shown is no longer new.
    fn reveal(&mut self) {
        self.finder_seen = self.unseen_render().or(self.finder_seen.take());
        if let Some(target) = self.finder_target() {
            self.revealing = Some(target);
            self.ask = Some(Ask::Reveal);
        }
    }

    /// The file to show in Finder, taken.
    pub fn take_revealing(&mut self) -> Option<PathBuf> {
        self.revealing.take()
    }

    /// The words to put on the clipboard, taken.
    pub fn take_copying(&mut self) -> Option<String> {
        self.copying.take()
    }

    /// The file to open in its own app, taken.
    pub fn take_opening(&mut self) -> Option<PathBuf> {
        self.opening.take()
    }

    /// The app to set up to reach this one (for the window to ask about), taken.
    pub fn take_setting_up(&mut self) -> Option<crate::setup::App> {
        self.setting_up.take()
    }

    /// Whether the card that connects an AI is shown.
    #[cfg(test)]
    pub fn connect_open(&self) -> bool {
        self.connect.open
    }

    /// Whether the card that connects an AI lies in the empty page now (no projects, no AI).
    fn lying(&self) -> bool {
        self.cards.is_empty() && veymelo_mcp::bridged() == 0
    }

    /// Where the card that connects an AI goes on a page `size` points big: under the status,
    /// or lying under the empty page's plates.
    fn connect_place(&self, size: (f64, f64), tx: &Texts) -> Place {
        if self.lying() {
            return Place::Lying { at: layout(size, tx, true).lying, wide: CONNECT_WIDE };
        }
        let ((sx, _, sw, sh), _, _) = status(size, tx);
        Place::Under { point: (sx + sw / 2.0, BAR_MIDDLE - sh / 2.0 + sh + 4.0), right: size.0 - 20.0, wide: CONNECT_WIDE }
    }

    /// What a press on the card that connects an AI asks: a set up (the window asks first),
    /// or words to copy.
    fn connect_press(&mut self, press: connect::Press) {
        match press {
            connect::Press::SetUp(app) => {
                self.connect.started();
                self.setting_up = Some(app);
                self.ask = Some(Ask::SetUp);
            }
            connect::Press::Copy(words) => {
                self.copying = Some(words);
                self.ask = Some(Ask::Copy);
            }
            _ => {}
        }
    }

    /// The name of the project being deleted (for the system to ask about).
    pub fn deleting_name(&self) -> Option<String> {
        self.deleting.as_ref().map(|e| e.name.clone())
    }

    /// What goes to the Trash for the project being deleted (its own folder, or only its file
    /// when it sits among other things), or why it cannot go: the window moves it there itself,
    /// outside the page, then answers [`Workspace::answer_delete`].
    pub fn trash_target(&self) -> Result<PathBuf, String> {
        let entry = self.deleting.as_ref().ok_or("nothing is being deleted")?;
        if working(&entry.file) {
            return Err(format!("The AI is working in \"{}\": it can be deleted when it has finished", entry.name));
        }
        Ok(veymelo_mcp::library::own_folder(&entry.file).unwrap_or_else(|| entry.file.clone()))
    }

    /// The answer to deleting a project, at `t`: off the list, or also to the Trash (`trashed`
    /// says how moving it there went), or nothing. A project an agent works in now stays.
    pub fn answer_delete(&mut self, choice: Delete, trashed: Option<Result<(), String>>, t: f64) {
        let Some(entry) = self.deleting.take() else { return };
        if choice == Delete::Cancel {
            return;
        }
        if working(&entry.file) {
            self.trouble = Some((format!("The AI is working in \"{}\": it can be deleted when it has finished", entry.name), t));
            return;
        }
        if choice == Delete::Trash {
            match trashed {
                Some(Ok(())) => {}
                Some(Err(why)) => {
                    self.trouble = Some((format!("\"{}\" could not go to the Trash: {why}", entry.name), t));
                    return;
                }
                None => return,
            }
        }
        match crate::live::library().forget(&entry.file) {
            Ok(()) => {
                let done = if choice == Delete::Trash { "moved to the Trash" } else { "taken off the list (its folder is where it was)" };
                self.notice = Some((format!("\"{}\" {done}", entry.name), t));
            }
            Err(why) => self.trouble = Some((why, t)),
        }
        self.reread(t, true);
    }

    /// The system's answer to `ask` at `t`: where the person chose, or none (they cancelled).
    pub fn answer(&mut self, ask: Ask, path: Option<PathBuf>, t: f64) {
        let Some(path) = path else { return };
        match ask {
            Ask::NewProject => {
                let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let folder = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                match crate::live::create(&name, &folder) {
                    Ok(name) => {
                        self.name = name;
                        self.mode = Mode::Inside;
                        self.since = t;
                        self.watch = Watch::new();
                    }
                    Err(why) => self.trouble = Some((why, t)),
                }
            }
            Ask::OpenProject => self.go_in(&path, t),
            Ask::DeleteProject => {}
            Ask::RenderTo => self.start_render(path, t),
            Ask::Reveal | Ask::Copy | Ask::SetUp | Ask::OpenFile => {}
        }
        self.reread(t, true);
    }

    /// How much of the dotted paper shows at `t` (0 to 1): all of it on the projects; going
    /// into a project it fades (the preview has its own), and comes again coming back out.
    pub fn paper(&self, t: f64) -> f64 {
        match self.mode {
            Mode::Inside => 1.0 - smooth((t - self.since) / 0.35),
            Mode::Home if self.since == 0.0 => 1.0,
            Mode::Home => smooth((t - self.since) / 0.35),
        }
    }

    /// How far the app's name has moved over (points) for the way back.
    pub fn name_shift(&self) -> f64 {
        self.shift.x.clamp(0.0, 1.2) * BACK_ROOM
    }

    /// Whether the page has something new to show: what the AI did, a frame drawn, a render
    /// gone on.
    pub fn wants_draw(&self) -> bool {
        let renders = crate::renders::count() != self.renders_seen;
        renders
            || match self.mode {
                Mode::Inside => self.watch.wants(),
                Mode::Home => self.seen != Self::now_seen(),
            }
    }

    fn now_seen() -> (u64, Option<State>, usize, u64) {
        (activity::count(), activity::stamp().1, veymelo_mcp::bridged(), crate::thumbs::count())
    }

    /// The pointer moved to `p`.
    pub fn moved(&mut self, p: (f64, f64)) {
        self.pointer = Some(p);
        self.settings.point(Some(p));
        self.connect.point(Some(p));
        if self.mode == Mode::Inside {
            self.watch.drag(p);
            self.watch.point(Some(p));
        }
    }

    /// The card under `p`, if any (none under the card that connects an AI).
    fn card_at(&self, p: (f64, f64)) -> Option<usize> {
        let tx = texts()?;
        let at = layout(self.size, tx, false);
        (!self.connect.over(p) && p.1 > BAR_RULE).then(|| (0..self.cards.len()).find(|&i| button::inside(card_rect(&at, i), p))).flatten()
    }

    /// The card whose bin is under `p`, if any.
    fn bin_at(&self, p: (f64, f64)) -> Option<usize> {
        let i = self.card_at(p)?;
        let at = layout(self.size, texts()?, false);
        button::inside(bin_rect(&at, i), p).then_some(i)
    }

    /// Whether `p` is on the plate in dashes after the projects (it makes a new one).
    fn ghost_at(&self, p: (f64, f64)) -> bool {
        let Some(tx) = texts() else { return false };
        !self.cards.is_empty() && !self.connect.over(p) && button::inside(card_rect(&layout(self.size, tx, false), self.cards.len()), p)
    }

    /// Moves on to `t` (`dt` seconds since the last step), on a page `size` points big, with
    /// the pointer at `pointer`. Gives whether anything is still moving.
    pub fn step(&mut self, t: f64, dt: f64, size: (f64, f64), pointer: Option<(f64, f64)>, scale: f64) -> bool {
        let Some(tx) = texts() else { return false };
        if self.shown.is_none() {
            return false;
        }
        self.size = size;
        // the renders going on
        let count = crate::renders::count();
        let mut moving = count != self.renders_seen;
        if moving {
            self.renders = crate::renders::now();
            self.renders_seen = count;
        }
        let home = self.mode == Mode::Home;
        if home {
            self.reread(t, false);
        }
        let grid = !self.cards.is_empty();
        let at = layout(size, tx, self.lying());
        let age = self.age(t);
        let rest = (home && age > 0.75).then_some(at.button);
        moving |= self.button.step(rest, if home { pointer } else { None }, dt);
        moving |= self.open.step((home && age > 0.75).then_some(at.open), if home { pointer } else { None }, dt);
        let on = |r: Rect| home && pointer.is_some_and(|p| button::inside(r, p));
        moving |= self.new_near.toward(if on(at.button) { 1.0 } else { 0.0 }, dt, 300.0, 30.0);
        moving |= self.status_near.toward(if on(status(size, tx).0) { 1.0 } else { 0.0 }, dt, 300.0, 30.0);
        let ghost = home && pointer.is_some_and(|p| self.ghost_at(p));
        moving |= self.ghost.toward(if ghost { 1.0 } else { 0.0 }, dt, 160.0, 20.0);
        // the card that connects an AI: lying in the empty page while no AI is, else where it
        // was opened, until it is closed
        let lying = home && self.lying();
        if lying {
            self.connect.show(self.connect_place(size, tx), t);
        } else if !home || self.was_lying {
            self.connect.close();
        } else if self.connect.open {
            self.connect.put(self.connect_place(size, tx));
        }
        self.was_lying = lying;
        moving |= self.connect.busy(t);
        // inside: the Render button, there once the page has come in
        let inside_now = self.mode == Mode::Inside && t - self.since > 0.5;
        let wide = self.render_wide(tx);
        if self.render_w.x <= 0.0 {
            self.render_w.x = wide;
        }
        moving |= self.render_w.toward(wide, dt, 240.0, 30.0);
        let render_rest = inside_now.then(|| self.render_rect());
        moving |= self.render.step(render_rest, if inside_now { pointer } else { None }, dt);
        let finder_near = inside_now && pointer.is_some_and(|p| button::inside(self.finder_rect(), p));
        moving |= self.finder_near.toward(if finder_near { 1.0 } else { 0.0 }, dt, 300.0, 30.0);
        if let Some(s) = self.render_state() {
            match s {
                crate::export::State::Going(..) => moving = true,
                crate::export::State::Made(out, ago, took) => {
                    // done: said a moment at the bottom, with how long it took
                    if ago < 0.3 && self.notice.as_ref().is_none_or(|(_, w)| t - w > 1.0) {
                        let name = out.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        self.notice = Some((format!("Rendered \"{name}\" in {}", crate::export::took(took)), t));
                    }
                    moving |= ago < 0.5;
                }
                crate::export::State::Failed(why, ago) => {
                    if ago < 0.3 && why != "stopped" && self.trouble.as_ref().is_none_or(|(w, _)| *w != why) {
                        self.trouble = Some((format!("The render failed: {why}"), t));
                    }
                    moving |= ago < 0.5;
                }
            }
        }
        let lift = match self.mode {
            Mode::Home if !grid => pointer.map_or(0.0, |p| reach((p.0 - at.figure.0).hypot(p.1 - at.figure.1), FIGURE_REACH)).max(self.button.near.x),
            _ => 0.0,
        };
        moving |= self.lift.toward(lift, dt, 120.0, 17.0);
        let over = if home { pointer.and_then(|p| self.card_at(p)) } else { None };
        for (i, c) in self.cards.iter_mut().enumerate() {
            moving |= c.lift.toward(if over == Some(i) { 1.0 } else { 0.0 }, dt, 160.0, 20.0);
        }
        let open_near = on(at.open);
        moving |= self.open_near.toward(if open_near { 1.0 } else { 0.0 }, dt, 300.0, 30.0);
        // inside, the name moves over for the way back, which answers the pointer
        let inside = self.mode == Mode::Inside;
        moving |= self.shift.toward(if inside && t - self.since > 0.3 { 1.0 } else { 0.0 }, dt, 140.0, 22.0);
        let near = inside && pointer.is_some_and(|p| button::inside(BACK, p));
        moving |= self.back_near.toward(if near { 1.0 } else { 0.0 }, dt, 300.0, 30.0);
        if inside {
            moving |= self.watch.step(t, scale);
        } else {
            let now = Self::now_seen();
            moving |= now != self.seen;
            self.seen = now;
        }
        if self.mode != Mode::Inside {
            self.settings.close();
        }
        if let Some(change) = self.settings.take_waiting() {
            self.apply(change, t);
        }
        moving |= self.settings.busy(t);
        moving || age < 1.5 || t - self.since < 1.2 || self.trouble.as_ref().is_some_and(|(_, w)| t - w < 5.0) || self.notice.as_ref().is_some_and(|(_, w)| t - w < 5.0)
    }

    /// The pointer at `p` on a page `size` points big.
    pub fn cursor(&self, p: (f64, f64), size: (f64, f64)) -> Cursor {
        let Some(tx) = texts() else { return Cursor::Arrow };
        let at = layout(size, tx, self.lying());
        match self.mode {
            Mode::Home if self.connect.hand(p) => Cursor::Hand,
            Mode::Home if self.connect.over(p) => Cursor::Arrow,
            Mode::Home if self.button.hit(p) || button::inside(at.open, p) || button::inside(status(size, tx).0, p) || self.card_at(p).is_some() || self.ghost_at(p) => Cursor::Hand,
            Mode::Inside if self.settings.hand(p) => Cursor::Hand,
            Mode::Inside if self.settings.over(p) => Cursor::Arrow,
            Mode::Inside if [self.pencil, self.about_line].iter().flatten().any(|r| button::inside(*r, p)) => Cursor::Hand,
            Mode::Inside if self.watch.on_edge(p) => Cursor::UpDown,
            Mode::Inside if button::inside(BACK, p) || self.watch.hand(p) || self.render.hit(p) || button::inside(self.finder_rect(), p) => Cursor::Hand,
            _ => Cursor::Arrow,
        }
    }

    pub fn down(&mut self, p: (f64, f64), t: f64) {
        // the card takes a press first: on it, or away from it (which closes it)
        self.card_press = self.settings.open;
        if self.settings.open {
            if let Some(change) = self.settings.press(p) {
                self.apply(change, t);
            }
            return;
        }
        if self.mode == Mode::Inside && [self.pencil, self.about_line].iter().flatten().any(|r| button::inside(*r, p)) {
            self.card_press = true;
            self.open_settings(t);
            return;
        }
        if self.mode == Mode::Home && self.connect.open {
            match self.connect.press(p, t) {
                // away from a card hanging under the status, the press closes it (and does
                // nothing else)
                connect::Press::Away if !self.lying() => {
                    self.connect.close();
                    self.card_press = true;
                    return;
                }
                connect::Press::Away => {}
                press => {
                    self.card_press = true;
                    self.connect_press(press);
                    return;
                }
            }
        }
        match self.mode {
            Mode::Home => {
                self.button.down(p);
                self.open.down(p);
            }
            Mode::Inside if button::inside(BACK, p) => {}
            Mode::Inside if self.render.hit(p) => {
                let going = matches!(self.render_state(), Some(crate::export::State::Going(..)));
                self.stop_held = going && button::inside(self.stop_rect(), p);
                self.render.down(p);
            }
            Mode::Inside => {
                self.watch.down(p, t);
                if let Some(words) = self.watch.copy.take() {
                    self.copying = Some(words);
                    self.ask = Some(Ask::Copy);
                }
                if let Some(app) = self.watch.set_up.take() {
                    self.setting_up = Some(app);
                    self.ask = Some(Ask::SetUp);
                }
            }
        }
    }

    /// The mouse came up at `p` at `t`: on the button, a new project; on "Open…", one to open;
    /// on a project, into it; inside, on the way back, out.
    pub fn up(&mut self, p: (f64, f64), t: f64) {
        self.watch.up(p);
        if let Some(path) = self.watch.open_file.take() {
            self.opening = Some(path);
            self.ask = Some(Ask::OpenFile);
        }
        if let Some((path, time)) = self.watch.add.take() {
            self.add_layer(&path, time, t);
        }
        if std::mem::take(&mut self.card_press) {
            return;
        }
        match self.mode {
            Mode::Inside if button::inside(BACK, p) => self.back(t),
            Mode::Inside if button::inside(self.finder_rect(), p) => self.reveal(),
            Mode::Inside => {
                let stop_held = std::mem::take(&mut self.stop_held);
                if self.render.up(p) {
                    match self.render_state() {
                        Some(crate::export::State::Going(..)) => {
                            if stop_held && button::inside(self.stop_rect(), p) {
                                crate::export::stop();
                            }
                        }
                        _ => self.ask = Some(Ask::RenderTo),
                    }
                }
            }
            Mode::Home => {
                let Some(tx) = texts() else { return };
                let opened = self.open.up(p);
                if self.button.up(p) || self.ghost_at(p) {
                    self.ask = Some(Ask::NewProject);
                } else if opened {
                    self.ask = Some(Ask::OpenProject);
                } else if button::inside(status(self.size, tx).0, p) {
                    if !self.lying() {
                        self.connect.show(self.connect_place(self.size, tx), t);
                    }
                } else if let Some(i) = self.bin_at(p) {
                    self.deleting = Some(self.cards[i].entry.clone());
                    self.ask = Some(Ask::DeleteProject);
                } else if let Some(i) = self.card_at(p) {
                    let file = self.cards[i].entry.file.clone();
                    self.go_in(&file, t);
                }
            }
        }
    }

    /// The pointer scrolled by `d` (points) at `p`, at `t`: inside, the timeline moves.
    pub fn scroll(&mut self, d: (f64, f64), p: (f64, f64), t: f64) {
        if self.mode == Mode::Inside && !self.settings.over(p) {
            self.watch.scroll(d, p, t);
        }
    }

    /// A pinch by `k` at `p`: inside, the timeline zooms.
    pub fn magnify(&mut self, k: f64, p: (f64, f64)) {
        if self.mode == Mode::Inside {
            self.watch.magnify(k, p);
        }
    }

    /// What a menu asks, at `t`.
    pub fn command(&mut self, c: Command, t: f64) {
        let inside = self.mode == Mode::Inside;
        match c {
            Command::NewProject => self.ask = Some(Ask::NewProject),
            Command::Open => self.ask = Some(Ask::OpenProject),
            Command::Render if inside && !matches!(self.render_state(), Some(crate::export::State::Going(..))) => self.ask = Some(Ask::RenderTo),
            Command::Back => self.back(t),
            Command::ZoomIn if inside => {
                self.watch.zoom_key(24);
            }
            Command::ZoomOut if inside => {
                self.watch.zoom_key(27);
            }
            Command::ZoomAll if inside => {
                self.watch.zoom_key(29);
            }
            Command::TogglePanel if inside => self.watch.toggle_panel(),
            Command::ToggleFiles if inside => self.watch.toggle_files(),
            Command::Reveal if inside => self.reveal(),
            Command::Settings if inside => self.open_settings(t),
            _ => {}
        }
    }

    /// A key at `t`: its code, what it types, and whether Shift is down. Gives whether the page
    /// used it. Inside a project: Space plays or stops, the arrows step a frame (a second with
    /// Shift), Home and End go to the first frame and the last, Escape goes back out.
    pub fn key(&mut self, code: u16, text: &str, shift: bool, t: f64) -> bool {
        if self.settings.open {
            if let Some(change) = self.settings.key(code, text, shift) {
                self.apply(change, t);
            }
            return true;
        }
        const RETURN: [u16; 2] = [36, 76];
        const ESCAPE: u16 = 53;
        const SPACE: u16 = 49;
        const LEFT: u16 = 123;
        const RIGHT: u16 = 124;
        const HOME: u16 = 115;
        const END: u16 = 119;
        match self.mode {
            Mode::Inside if code == LEFT || code == RIGHT => {
                let by = if shift { self.watch.fps().round().max(1.0) } else { 1.0 };
                self.watch.nudge(if code == LEFT { -by } else { by });
                true
            }
            Mode::Inside if code == HOME || code == END => {
                self.watch.to_edge(code == END);
                true
            }
            Mode::Home if code == ESCAPE && self.connect.open && !self.lying() => {
                self.connect.close();
                true
            }
            Mode::Home if RETURN.contains(&code) && self.button.rest.is_some() => {
                self.ask = Some(Ask::NewProject);
                true
            }
            Mode::Inside if code == SPACE => {
                self.watch.toggle(t);
                true
            }
            Mode::Inside if code == ESCAPE => {
                self.back(t);
                true
            }
            Mode::Inside => self.watch.zoom_key(code),
            _ => false,
        }
    }

    /// Which blink a caret on the page is in (the settings card's, when it is open): the window
    /// draws again when it changes.
    pub fn blink(&self, t: f64) -> i64 {
        self.settings.blink(t)
    }

    /// Words from the clipboard, for the field typed in.
    pub fn paste(&mut self, words: &str) {
        if self.settings.open {
            self.settings.paste(words);
        }
    }

    /// All of the field typed in chosen.
    pub fn select_all(&mut self) {
        if self.settings.open {
            self.settings.select_all();
        }
    }

    /// Makes the file at `path` a layer of the composition shown, starting at `time`, as a step
    /// of the person's (brought in as footage first when the project has none of it), then
    /// saved.
    fn add_layer(&mut self, path: &std::path::Path, time: f64, t: f64) {
        use veymelo_json::Value as Json;
        let Some((file, session)) = crate::live::shown() else { return };
        let Ok(mut s) = session.try_lock() else {
            self.trouble = Some(("The project is busy with the AI's step: let it go again in a moment".into(), t));
            return;
        };
        let Some(comp) = s.view.comp else {
            self.trouble = Some(("There is no composition to put it in yet".into(), t));
            return;
        };
        let project = file.display().to_string();
        let run = |s: &mut veymelo_engine::Session, id: &str, params: Json| -> Result<Json, String> {
            let args = Json::object([("command", Json::from(id)), ("params", params.clone())]);
            let n = activity::begin_person(&project, "command_run", &args);
            match s.run(id, &params) {
                Ok(v) => {
                    activity::end(n, true, "");
                    Ok(v)
                }
                Err(e) => {
                    activity::end(n, false, &e.message);
                    Err(e.message)
                }
            }
        };
        let have = s.project().items.iter().find(|i| matches!(&i.kind, veymelo_engine::project::ItemKind::Footage(f) if std::path::Path::new(&f.path) == path)).map(|i| i.id);
        let made: Result<String, String> = (|| {
            let item = match have {
                Some(id) => id,
                None => run(&mut s, "footage.import", Json::object([("path", Json::from(path.display().to_string().as_str()))]))?.get("item").and_then(Json::as_u64).ok_or("it could not be brought in")?,
            };
            let at = Json::Number(comp as f64);
            let layer = run(&mut s, "layer.add", Json::object([("comp", at.clone()), ("type", Json::from("footage")), ("item", Json::Number(item as f64))]))?.get("layer").and_then(Json::as_u64).ok_or("no layer was made")?;
            let (name, long) = s.project().comp(comp).and_then(|c| c.layer(layer)).map(|l| (l.name.clone(), (l.out_point.seconds() - l.in_point.seconds()).max(0.0))).unwrap_or_default();
            if time > 0.0 {
                let params = Json::object([("comp", at), ("layer", Json::Number(layer as f64)), ("start", Json::Number(time)), ("in", Json::Number(time)), ("out", Json::Number(time + long))]);
                run(&mut s, "layer.set", params)?;
            }
            Ok(name)
        })();
        match made {
            Ok(name) => {
                if s.modified() && s.path.is_some() {
                    let _ = s.run("project.save", &Json::Null);
                }
                self.notice = Some((format!("\u{201c}{name}\u{201d} added at {}", length(time)), t));
            }
            Err(why) => self.trouble = Some((format!("\u{201c}{}\u{201d} could not be added: {why}", path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())), t)),
        }
    }

    /// Files let go on the window from Finder at `p`, at `t`. On the projects, the project
    /// among them opens. Inside a project, they are copied into its `assets` folder (those in
    /// its folder already stay where they are); let go over the preview or the timeline, each
    /// becomes a layer where the time is.
    pub fn dropped(&mut self, paths: &[PathBuf], p: (f64, f64), t: f64) {
        match self.mode {
            Mode::Home => {
                if let Some(first) = paths.first() {
                    let first = first.clone();
                    self.go_in(&first, t);
                }
            }
            Mode::Inside => {
                let Some((file, _)) = crate::live::shown() else { return };
                let Some(root) = file.parent().map(std::path::Path::to_path_buf) else { return };
                let assets = root.join("assets");
                let mut placed = vec![];
                for src in paths {
                    if src.starts_with(&root) {
                        placed.push(src.clone());
                        continue;
                    }
                    if src.is_dir() {
                        self.trouble = Some((format!("\u{201c}{}\u{201d} is a folder: let its files go instead", src.display()), t));
                        continue;
                    }
                    let name = src.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    let mut dest = assets.join(&name);
                    // a name already taken: "name 2.ext", "name 3.ext"…
                    let (stem, ext) = (src.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()), src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default());
                    let mut k = 2;
                    while dest.exists() {
                        dest = assets.join(format!("{stem} {k}{ext}"));
                        k += 1;
                    }
                    match std::fs::create_dir_all(&assets).and_then(|_| std::fs::copy(src, &dest)) {
                        Ok(_) => placed.push(dest),
                        Err(e) => self.trouble = Some((format!("\u{201c}{name}\u{201d} could not be copied: {e}"), t)),
                    }
                }
                let copied = placed.iter().filter(|q| !paths.contains(q)).count();
                if copied > 0 {
                    self.notice = Some((if copied == 1 { "Copied 1 file into assets".to_string() } else { format!("Copied {copied} files into assets") }, t));
                }
                if self.watch.over_middle(p) {
                    let time = self.watch.now();
                    for q in placed {
                        self.add_layer(&q, time, t);
                    }
                }
            }
        }
    }

    /// Opens the settings card on the composition shown, under its pencil.
    fn open_settings(&mut self, t: f64) {
        let Some(about) = self.watch.about() else { return };
        let x = self.pencil.map_or(220.0, |r| r.0 - 60.0).clamp(20.0, (self.size.0 - 510.0 - 20.0).max(20.0));
        self.settings.show(about, (x, crate::watch::BAR + 4.0), t);
    }

    /// Makes the settings card's change in the project shown: its commands one after another,
    /// each a step of the person's (beside the AI's), then saved. While the AI is in the middle
    /// of a step it waits, and is tried again.
    fn apply(&mut self, change: Vec<(String, veymelo_json::Value)>, _t: f64) {
        let Some((file, session)) = crate::live::shown() else { return };
        let Ok(mut s) = session.try_lock() else {
            self.settings.wait(change);
            return;
        };
        let project = file.display().to_string();
        for (id, params) in &change {
            let args = veymelo_json::Value::object([("command", veymelo_json::Value::from(id.as_str())), ("params", params.clone())]);
            let n = activity::begin_person(&project, "command_run", &args);
            match s.run(id, params) {
                Ok(_) => activity::end(n, true, ""),
                Err(e) => {
                    activity::end(n, false, &e.message);
                    self.settings.failed(e.message);
                    return;
                }
            }
        }
        if s.modified() && s.path.is_some() {
            let _ = s.run("project.save", &veymelo_json::Value::Null);
        }
        self.settings.close();
    }

    /// Draws the page at `t` on a canvas `scale` pixels to the point, `size` points big; the
    /// project's name goes after the app's name, which ends at `crumb` (points: x, baseline).
    pub fn paint(&mut self, canvas: &mut Canvas, scale: f64, t: f64, size: (f64, f64), crumb: (f64, f64)) {
        let Some(tx) = texts() else { return };
        let age = self.age(t);
        if age < 0.0 {
            return;
        }
        let grid = !self.cards.is_empty();
        let at = layout(size, tx, self.lying());
        let inside = if self.mode == Mode::Inside { t - self.since } else { -1.0 };
        // going in, the page fades; back out, it comes again
        let page = if inside >= 0.0 { 1.0 - smooth(inside / 0.35) } else { smooth((t - self.since) / 0.35).max(if self.since == 0.0 { 1.0 } else { 0.0 }) };
        let come = smooth(age / 0.6);
        let page = page * come;
        if page > 0.0 {
            self.paint_bar(canvas, scale, t, size, tx, &at, page);
            if grid {
                self.paint_grid(canvas, scale, t, size, tx, &at, page);
            } else {
                self.paint_empty(canvas, scale, t, size, tx, &at, page);
            }
        }
        // inside: the preview and what the AI does, coming in as the page goes
        if inside >= 0.0 {
            let a = smooth((inside - 0.3) / 0.5) as f32;
            if a > 0.0 {
                self.watch.paint(canvas, scale, t, size, a);
            }
        }
        // the way back out: a round button with a chevron, darker under the pointer
        let back = (self.shift.x.clamp(0.0, 1.0) * if inside >= 0.0 { 1.0 } else { 0.0 }) as f32;
        if back > 0.0 {
            let near = self.back_near.x.clamp(0.0, 1.0);
            let (x, y, w, h) = BACK;
            let px = |v: f64| v * scale;
            if near > 0.0 {
                canvas.fill_faded(&Path::rounded(px(x), px(y), px(w), px(h), px(w / 2.0)), CREASE, back * near as f32);
            }
            canvas.stroke(&Path::rounded_points(px(x), px(y), px(w), px(h), px(w / 2.0)), true, scale, EDGE.mix(LIT, near), back);
            let (cx, cy) = (x + w / 2.0 - 1.0, y + h / 2.0);
            canvas.stroke(&[(px(cx + 3.0), px(cy - 6.0)), (px(cx - 3.0), px(cy)), (px(cx + 3.0), px(cy + 6.0))], false, 1.6 * scale, INK, back);
        }
        // inside: the project's name after the app's name
        if inside >= 0.0 {
            let a = smooth((inside - 0.35) / 0.4) as f32;
            if a > 0.0 {
                let rise = (1.0 - settle((inside - 0.35) / 0.6)) * 4.0;
                let x = crumb.0 + 12.0;
                tx.slash.draw(canvas, x, crumb.1 + rise, 1.0, scale, FAINT, a);
                let name = Label::new(tx.crumb.clone(), 16.0, &self.name);
                name.draw(canvas, x + tx.slash.wide + 10.0, crumb.1 + rise, 1.0, scale, INK, a);
                // what the composition shown is, quietly after it
                self.pencil = None;
                self.about_line = None;
                if let Some(line) = self.watch.about_line(&self.name) {
                    let from = x + tx.slash.wide + 10.0 + name.wide + 16.0;
                    let room = self.render.rest.map_or(size.0 - 28.0, |_| self.finder_rect().0) - 24.0 - from - 36.0;
                    if room > 60.0 {
                        let l = fit(&tx.small, 12.0, &line, room);
                        let pencil = (from + l.wide + 8.0, crumb.1 - 18.0, 26.0, 26.0);
                        let near = self.pointer.is_some_and(|p| button::inside(pencil, p) || button::inside((from, crumb.1 - 16.0, l.wide, 22.0), p)) || self.settings.open;
                        l.draw(canvas, from, crumb.1 + rise, 1.0, scale, if near { GREY } else { FAINT }, a);
                        // the pencil: a round button, its edge darker under the pointer
                        let px = |v: f64| v * scale;
                        let (bx, by, bw, bh) = (pencil.0, pencil.1 + rise, pencil.2, pencil.3);
                        if near {
                            canvas.fill_faded(&Path::rounded(px(bx), px(by), px(bw), px(bh), px(bw / 2.0)), CREASE, a);
                        }
                        canvas.stroke(&Path::rounded_points(px(bx), px(by), px(bw), px(bh), px(bw / 2.0)), true, iso::STROKE * scale, EDGE.mix(LIT, if near { 1.0 } else { 0.0 }), a);
                        let (cx, cy) = (bx + bw / 2.0, by + bh / 2.0);
                        let tip = [(cx - 4.5, cy + 4.5), (cx - 3.5, cy + 1.5), (cx + 2.5, cy - 4.5), (cx + 4.5, cy - 2.5), (cx - 1.5, cy + 3.5)];
                        canvas.stroke(&tip.iter().map(|(u, v)| (px(*u), px(*v))).collect::<Vec<_>>(), true, 1.1 * scale, INK, a);
                        self.pencil = Some(pencil);
                        self.about_line = Some((from, crumb.1 - 16.0, l.wide, 22.0));
                    }
                }
            }
        }
        // inside: the Render button, and the round Show in Finder button before it
        if inside >= 0.0 && let Some(rest) = self.render.rest {
            let a = smooth((inside - 0.5) / 0.5);
            self.paint_render_button(canvas, scale, t, tx, rest, a);
            self.paint_finder(canvas, scale, tx, a as f32);
        }
        // the renders going on that are not shown where they belong (on their plate, or in the
        // Render button), in a small pill at the top right
        let own = crate::live::shown().map(|(f, _)| f);
        if inside >= 0.0 {
            let others: Vec<_> = self.renders.iter().filter(|r| r.project.is_none() || r.project != own).cloned().collect();
            let right = self.render.rest.map_or(size.0 - 28.0, |_| self.finder_rect().0) - 12.0;
            let (_, ry, _, rh) = self.render_rect();
            self.paint_renders(canvas, scale, t, (right, ry + rh / 2.0), &others, tx, come as f32);
        } else {
            let shown: Vec<&std::path::Path> = self.cards.iter().map(|c| c.entry.file.as_path()).collect();
            let others: Vec<_> = self.renders.iter().filter(|r| r.project.as_deref().is_none_or(|p| !shown.contains(&p))).cloned().collect();
            let right = status(size, tx).0.0 - 8.0;
            self.paint_renders(canvas, scale, t, (right, BAR_MIDDLE), &others, tx, page as f32);
        }
        // the composition's settings, over the page
        if inside >= 0.0 {
            self.settings.paint(canvas, scale, t, come as f32);
        }
        // the card that connects an AI, hanging under the status
        if inside < 0.0 && !self.lying() {
            self.connect.paint(canvas, scale, t, page as f32);
        }
        // what went wrong (red) or was done (grey), a moment at the bottom, on any page: in a
        // pill of paper, so it reads over what is under it
        for (said, colour) in [(&self.trouble, RED), (&self.notice, GREY)] {
            if let Some((words, when)) = said {
                let k = t - when;
                if k < 5.0 {
                    let a = (smooth(k / 0.2) * (1.0 - smooth((k - 4.4) / 0.6)) * come) as f32;
                    let l = fit(&tx.small, 12.0, words, size.0 - 80.0);
                    let (w, h) = (l.wide + 28.0, 30.0);
                    let (x, y) = ((size.0 - w) / 2.0, size.1 - 24.0 - h);
                    let px = |v: f64| v * scale;
                    canvas.shadow(px(x), px(y + 4.0), px(w), px(h), px(h / 2.0), px(12.0), INK, a * 0.12);
                    canvas.fill_faded(&Path::rounded(px(x), px(y), px(w), px(h), px(h / 2.0)), PAPER, a);
                    l.draw(canvas, x + 14.0, y + h / 2.0 + 4.0, 1.0, scale, colour, a);
                }
            }
        }
    }

    /// The bar at the top of the projects (the AI's status at its right, a faint hairline under
    /// it) and the row under it: "Projects" with how many, "Open…" and "New project".
    #[allow(clippy::too_many_arguments)]
    fn paint_bar(&self, canvas: &mut Canvas, scale: f64, t: f64, size: (f64, f64), tx: &Texts, at: &Layout, alpha: f64) {
        let age = self.age(t);
        let px = |v: f64| v * scale;
        let a = (smooth((age - 0.1) / 0.5) * alpha) as f32;
        canvas.fill_faded(&Path::rounded(px(28.0), px(BAR_RULE), px(size.0 - 56.0), px(iso::STROKE), 0.0), BAR_LINE, a);
        // the AI's status: a ring in hairlines (an ink dot in it when connected), its words
        let (rect, (cx, cy), here) = status(size, tx);
        let near = self.status_near.x.clamp(0.0, 1.0);
        if near > 0.0 || self.connect.open && !self.lying() {
            let k = if self.connect.open && !self.lying() { 1.0 } else { near };
            canvas.fill_faded(&Path::rounded(px(rect.0), px(rect.1), px(rect.2), px(rect.3), px(rect.3 / 2.0)), Rgb(241, 241, 244), a * k as f32);
        }
        canvas.stroke(&Path::rounded_points(px(cx - 5.0), px(cy - 5.0), px(10.0), px(10.0), px(5.0)), true, iso::STROKE * scale, if here { LIT } else { EDGE }, a);
        if here {
            canvas.fill_faded(&Path::rounded(px(cx - 2.2), px(cy - 2.2), px(4.4), px(4.4), px(2.2)), INK, a);
        }
        let words = Label::new(tx.status.clone(), 12.0, if here { "AI connected" } else { "AI not connected" });
        let x = cx + 5.0 + 7.0;
        words.draw(canvas, x, cy + 4.2, 1.0, scale, if here { INK } else { GREY }, a);
        if !here {
            let go = Label::new(tx.status.clone(), 12.0, "Connect");
            let gx = x + words.wide + 8.0;
            go.draw(canvas, gx, cy + 4.2, 1.0, scale, INK, a);
            canvas.fill_faded(&Path::rounded(px(gx), px(cy + 6.6), px(go.wide), px(0.8), 0.0), INK.mix(PAPER, 0.35 * (1.0 - near)), a);
        }
        // the title's row
        let a = (smooth((age - 0.15) / 0.5) * alpha) as f32;
        tx.title.draw(canvas, 28.0, TITLE_BASE, 1.0, scale, INK, a);
        if !self.cards.is_empty() {
            let n = Label::new(tx.count.clone(), 16.0, &self.cards.len().to_string());
            n.draw(canvas, 28.0 + tx.title.wide + 10.0, TITLE_BASE, 1.0, scale, FAINT, a);
        }
        let shown = (age - 0.4) / 0.6;
        if shown > 0.0 {
            let b = (smooth(shown / 0.45) * alpha) as f32;
            let rise = (1.0 - settle(shown)) * 8.0;
            let (ox, oy, ow, oh) = at.open;
            button::line_button(canvas, scale, (ox, oy + rise, ow, oh), &tx.open, Icon::Folder, self.open_near.x, false, b);
            let (nx, ny, nw, nh) = at.button;
            button::line_button(canvas, scale, (nx, ny + rise, nw, nh), &tx.button, Icon::Plus, self.new_near.x, true, b);
        }
    }

    /// The Render button as it stands: "Render"; while rendering, filling with how far it has
    /// come, with a cross to stop it; done, "Show in Finder".
    fn paint_render_button(&self, canvas: &mut Canvas, scale: f64, t: f64, tx: &Texts, rest: Rect, a: f64) {
        let px = |v: f64| v * scale;
        match self.render_state() {
            Some(crate::export::State::Going(k, left)) => {
                let (x, y, w, h) = rest;
                let r = h / 2.0;
                let af = a as f32;
                canvas.shadow(px(x), px(y + 6.0), px(w), px(h), px(r), px(14.0), INK, af * 0.16);
                canvas.fill_faded(&Path::rounded(px(x), px(y), px(w), px(h), px(r)), INK, af);
                // how far: a pen drawing a line along the bottom, under the words
                let (l0, l1, ly) = (x + 18.0, x + w - h - 2.0, y + h - 9.0);
                let front = l0 + (l1 - l0) * k.clamp(0.0, 1.0);
                canvas.fill_faded(&Path::rounded(px(l0), px(ly), px(l1 - l0), px(1.5), px(0.75)), Rgb(64, 64, 70), af);
                canvas.fill_faded(&Path::rounded(px(l0), px(ly), px(front - l0), px(1.5), px(0.75)), PAPER, af);
                pen_dot(canvas, scale, t, (front, ly + 0.75), 6.0, af);
                let said = match left {
                    Some(l) => format!("{}%  ·  {}", (k * 100.0).floor(), crate::export::left(l)),
                    None => format!("Rendering {}%", (k * 100.0).floor()),
                };
                let words = Label::new(tx.pill.clone(), 13.0, &said);
                words.draw(canvas, x + 18.0, y + h / 2.0 + words.drop(1.0) - 3.0, 1.0, scale, PAPER, af);
                // the cross that stops it, lighter under the pointer
                let (sx, sy, sw, sh) = self.stop_rect();
                let near = self.pointer.is_some_and(|p| button::inside((sx, sy, sw, sh), p));
                if near {
                    canvas.fill_faded(&Path::rounded(px(sx + 6.0), px(sy + 6.0), px(sw - 12.0), px(sh - 12.0), px((sw - 12.0) / 2.0)), Rgb(255, 255, 255), af * 0.18);
                }
                let (cx, cy) = (sx + sw / 2.0, sy + sh / 2.0);
                canvas.stroke(&[(px(cx - 4.0), px(cy - 4.0)), (px(cx + 4.0), px(cy + 4.0))], false, 1.6 * scale, PAPER, af);
                canvas.stroke(&[(px(cx + 4.0), px(cy - 4.0)), (px(cx - 4.0), px(cy + 4.0))], false, 1.6 * scale, PAPER, af);
            }
            _ => self.render.draw(canvas, scale, &tx.render, rest, a, 0.0, 0.0, (0.0, 0.0)),
        }
    }

    /// The round Show in Finder button: a folder in hairlines, a green dot while a render the
    /// person has not looked at is there; under the pointer, what it shows.
    fn paint_finder(&self, canvas: &mut Canvas, scale: f64, tx: &Texts, alpha: f32) {
        let px = |v: f64| v * scale;
        let (x, y, w, h) = self.finder_rect();
        let near = self.finder_near.x.clamp(0.0, 1.0);
        if near > 0.0 {
            canvas.fill_faded(&Path::rounded(px(x), px(y), px(w), px(h), px(w / 2.0)), CREASE, alpha * near as f32);
        }
        canvas.fill_faded(&Path::rounded(px(x), px(y), px(w), px(h), px(w / 2.0)), PAPER, alpha * (1.0 - near as f32));
        canvas.stroke(&Path::rounded_points(px(x), px(y), px(w), px(h), px(w / 2.0)), true, iso::STROKE * scale, EDGE.mix(LIT, near), alpha);
        // a folder: its tab, then its body
        let (cx, cy) = (x + w / 2.0, y + h / 2.0 + 0.5);
        let folder: Vec<(f64, f64)> = [(-7.0, 5.0), (-7.0, -5.5), (-2.5, -5.5), (-1.0, -3.5), (7.0, -3.5), (7.0, 5.0)].iter().map(|(dx, dy)| (px(cx + dx), px(cy + dy))).collect();
        canvas.stroke(&folder, true, 1.2 * scale, INK, alpha);
        if self.unseen_render().is_some() {
            canvas.fill_faded(&Path::rounded(px(x + w - 8.0), px(y + 1.0), px(8.0), px(8.0), px(4.0)), DONE, alpha);
        }
        // what it shows, under it
        if near > 0.05
            && let Some(target) = self.finder_target()
        {
            let name = target.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let l = fit(&tx.small, 12.0, &format!("Show in Finder  ·  {name}"), 300.0);
            let (tw, th) = (l.wide + 20.0, 24.0);
            let tx0 = (x + w / 2.0 - tw / 2.0).min(self.size.0 - tw - 8.0);
            let ty = y + h + 8.0;
            let a = alpha * near as f32;
            canvas.shadow(px(tx0), px(ty + 2.0), px(tw), px(th), px(8.0), px(10.0), INK, a * 0.14);
            canvas.fill_faded(&Path::rounded(px(tx0), px(ty), px(tw), px(th), px(8.0)), PAPER, a);
            canvas.stroke(&Path::rounded_points(px(tx0), px(ty), px(tw), px(th), px(8.0)), true, iso::STROKE * scale, CREASE, a);
            l.draw(canvas, tx0 + 10.0, ty + 16.0, 1.0, scale, INK, a);
        }
    }

    /// The renders in `renders` (those not shown where they belong) as one small pill of paper
    /// ending at `right` with its middle at `cy` (points): the pen's dot, whose the first is
    /// and how far it has come, how many more. Gives its left edge.
    #[allow(clippy::too_many_arguments)]
    fn paint_renders(&self, canvas: &mut Canvas, scale: f64, t: f64, (right, cy): (f64, f64), renders: &[veymelo_engine::progress::Render], tx: &Texts, alpha: f32) -> f64 {
        let Some(r) = renders.first() else { return right };
        let px = |v: f64| v * scale;
        let more = if renders.len() > 1 { format!("  +{}", renders.len() - 1) } else { String::new() };
        let share = if r.total == 0 { "…".to_string() } else { format!("{}%", (r.fraction() * 100.0).floor()) };
        let words = fit(&tx.small, 12.0, &format!("{}  {share}{more}", r.name()), 260.0);
        let (w, h) = (words.wide + 40.0, 28.0);
        let (x, y) = (right - w, cy - h / 2.0);
        canvas.fill_faded(&Path::rounded(px(x), px(y), px(w), px(h), px(h / 2.0)), PAPER, alpha);
        canvas.stroke(&Path::rounded_points(px(x), px(y), px(w), px(h), px(h / 2.0)), true, iso::STROKE * scale, EDGE, alpha);
        pen_dot(canvas, scale, t, (x + 16.0, cy), 7.0, alpha);
        words.draw(canvas, x + 26.0, cy + 4.0, 1.0, scale, INK, alpha);
        x
    }

    /// No projects yet: the plates, the words, the link under the button.
    #[allow(clippy::too_many_arguments)]
    fn paint_empty(&mut self, canvas: &mut Canvas, scale: f64, t: f64, size: (f64, f64), tx: &Texts, at: &Layout, alpha: f64) {
        let age = self.age(t);
        let lift = self.lift.x.clamp(0.0, 1.2);
        // the plates drop in one after another; the top one lifts toward the pointer
        for i in 0..PLATES {
            let p = (age - 0.08 * i as f64) / 0.7;
            if p <= 0.0 {
                continue;
            }
            let top = i == PLATES - 1;
            let a = smooth(p / 0.5) * alpha;
            let mut z = i as f64 * PLATE_GAP + (1.0 - settle(p)) * 40.0;
            let (mut edge, mut mark) = (EDGE, None);
            if top {
                z += lift * LIFT;
                edge = EDGE.mix(LIT, lift);
                mark = Some((16.0, CREASE.mix(LIT, lift)));
            }
            iso::plate(canvas, scale, at.origin, z, &PLATE, PAPER, edge, CREASE, a as f32, mark);
        }
        let words = smooth((age - 0.35) / 0.4) * alpha;
        if words > 0.0 {
            let rise = (1.0 - settle((age - 0.35) / 0.6)) * 10.0;
            tx.words.draw(canvas, (size.0 - tx.words.wide) / 2.0, at.words_baseline + rise, 1.0, scale, GREY, words as f32);
            if !self.lying() {
                let hint = Label::new(tx.small.clone(), 12.0, "Make one with New project, or ask the AI to.");
                hint.draw(canvas, (size.0 - hint.wide) / 2.0, at.words_baseline + 24.0 + rise, 1.0, scale, FAINT, words as f32);
            }
        }
        // no AI yet: connecting one is the first thing to do
        if self.lying() {
            let a = smooth((age - 0.5) / 0.5) * alpha;
            self.connect.paint(canvas, scale, t, a as f32);
        }
    }

    /// The projects on a grid: each a plate, its name, when it last changed, whether an AI
    /// works in it.
    #[allow(clippy::too_many_arguments)]
    fn paint_grid(&self, canvas: &mut Canvas, scale: f64, t: f64, size: (f64, f64), tx: &Texts, at: &Layout, alpha: f64) {
        let age = self.age(t);
        // after the projects, a plate in dashes with a plus: a new one
        let n = self.cards.len();
        let (x, y, w, h) = card_rect(at, n);
        let p = (age - 0.2 - 0.05 * n as f64) / 0.6;
        let a = (smooth(p / 0.5) * alpha) as f32;
        if y < size.1 && a > 0.0 {
            let lift = self.ghost.x.clamp(0.0, 1.2);
            let z = lift * CARD_LIFT + (1.0 - settle(p)) * 20.0;
            iso::ghost(canvas, scale, (x + w / 2.0, y + 78.0), z, &CARD_PLATE, PAPER, EDGE.mix(LIT, lift.min(1.0)), 4.0, 14.0, a);
            let ink = FAINT.mix(INK, lift.min(1.0));
            tx.ghost.draw(canvas, x + (w - tx.ghost.wide) / 2.0, y + h - 46.0, 1.0, scale, ink, a);
        }
        for (i, c) in self.cards.iter().enumerate() {
            let (x, y, w, h) = card_rect(at, i);
            if y > size.1 {
                break;
            }
            // one after another
            let p = (age - 0.2 - 0.05 * i as f64) / 0.6;
            let a = (smooth(p / 0.5) * alpha) as f32;
            if a <= 0.0 {
                continue;
            }
            let lift = c.lift.x.clamp(0.0, 1.2);
            let origin = (x + w / 2.0, y + 78.0);
            let z = lift * CARD_LIFT + (1.0 - settle(p)) * 20.0;
            iso::plate(canvas, scale, origin, z, &CARD_PLATE, PAPER, EDGE.mix(LIT, lift), CREASE, a, None);
            // its picture lying on it, once drawn (faint while it is being rendered)
            let rendering = self.renders.iter().find(|r| r.project.as_deref() == Some(c.entry.file.as_path()));
            let thumb = crate::thumbs::get(&c.entry.file, c.entry.modified);
            if let Some(th) = &thumb {
                let k = if rendering.is_some() { 0.45 } else { 1.0 };
                iso::picture(canvas, scale, origin, z, &CARD_PLATE, 7.0, th.w as f64 / th.h.max(1) as f64, &|x, y| th.at(x, y), EDGE, a * k);
            }
            let name = fit(&tx.name, 14.0, &c.entry.name, w - 16.0);
            name.draw(canvas, x + (w - name.wide) / 2.0, y + h - 46.0, 1.0, scale, INK, a);
            if let Some(r) = rendering {
                // a render: a pen going round the plate, as far as it has come
                iso::trace(canvas, scale, origin, z, &CARD_PLATE, r.fraction(), INK, 1.4, PEN, 6.0, a);
                let words = Label::new(tx.small.clone(), 12.0, &render_words(r));
                words.draw(canvas, x + (w - words.wide) / 2.0, y + h - 25.0, 1.0, scale, INK, a);
            } else if working(&c.entry.file) {
                let words = Label::new(tx.small.clone(), 12.0, "AI is working");
                let x0 = x + (w - words.wide - 12.0) / 2.0;
                pen_dot(canvas, scale, t, (x0 + 3.5, y + h - 29.0), 7.0, a);
                words.draw(canvas, x0 + 12.0, y + h - 25.0, 1.0, scale, GREY, a);
            } else {
                // when it last changed, and how long it is
                let long = thumb.as_ref().filter(|th| th.duration > 0.0).map(|th| format!("  ·  {}", length(th.duration))).unwrap_or_default();
                let when = Label::new(tx.small.clone(), 12.0, &format!("{}{long}", ago(c.entry.modified)));
                when.draw(canvas, x + (w - when.wide) / 2.0, y + h - 25.0, 1.0, scale, FAINT, a);
            }
            // the bin, at the corner of the plate under the pointer
            if lift > 0.05 {
                let (bx, by, bw, bh) = bin_rect(at, i);
                let near = self.pointer.is_some_and(|p| button::inside((bx, by, bw, bh), p));
                let ink = if near { RED } else { GREY };
                let b = a * lift.min(1.0) as f32;
                if near {
                    canvas.fill_faded(&Path::rounded(bx * scale, by * scale, bw * scale, bh * scale, bw / 2.0 * scale), Rgb(250, 232, 232), b);
                }
                let (cx, cy) = (bx + bw / 2.0, by + bh / 2.0);
                let p = |dx: f64, dy: f64| ((cx + dx) * scale, (cy + dy) * scale);
                // a lid with its handle, and a can with two ribs
                canvas.stroke(&[p(-6.5, -4.5), p(6.5, -4.5)], false, 1.4 * scale, ink, b);
                canvas.stroke(&[p(-2.0, -4.5), p(-2.0, -6.5), p(2.0, -6.5), p(2.0, -4.5)], false, 1.4 * scale, ink, b);
                canvas.stroke(&[p(-5.0, -4.5), p(-4.2, 6.5), p(4.2, 6.5), p(5.0, -4.5)], false, 1.4 * scale, ink, b);
                canvas.stroke(&[p(-1.6, -2.0), p(-1.4, 4.0)], false, 1.2 * scale, ink, b);
                canvas.stroke(&[p(1.6, -2.0), p(1.4, 4.0)], false, 1.2 * scale, ink, b);
            }
        }
    }
}

/// How far a render has come, in words: `Rendering 56%  ·  12 s left`.
fn render_words(r: &veymelo_engine::progress::Render) -> String {
    let k = r.fraction();
    let spent = r.updated - r.started;
    match r.total {
        0 => "Rendering…".to_string(),
        _ if k > 0.03 && spent > 1.0 => format!("Rendering {}%  ·  {}", (k * 100.0).floor(), crate::export::left(spent * (1.0 - k) / k)),
        _ => format!("Rendering {}%", (k * 100.0).floor()),
    }
}

/// The pen's dot at `at` (points), `size` across, breathing as the work goes on.
pub fn pen_dot(canvas: &mut Canvas, scale: f64, t: f64, at: (f64, f64), size: f64, alpha: f32) {
    let breath = 0.55 + 0.45 * (0.5 + 0.5 * (t * 3.2).sin());
    let r = size / 2.0;
    canvas.fill_faded(&Path::rounded((at.0 - r) * scale, (at.1 - r) * scale, size * scale, size * scale, r * scale), PEN, alpha * breath as f32);
}

/// Words in a face at a size, cut to fit `wide` points (with an ellipsis).
pub fn fit(font: &Font, size: f64, text: &str, wide: f64) -> Label {
    let full = Label::new(font.clone(), size, text);
    if full.wide <= wide {
        return full;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len();
    while n > 0 {
        n -= 1;
        let l = Label::new(font.clone(), size, &(chars[..n].iter().collect::<String>() + "…"));
        if l.wide <= wide {
            return l;
        }
    }
    Label::new(font.clone(), size, "…")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Projects opened from Finder: the first shows from the page, another takes its place, and
    /// opening the one shown leaves it as it is.
    #[test]
    fn projects_opened_from_finder_show() {
        let dir = std::env::temp_dir().join(format!("veymelo-opened-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::live::use_library(dir.join("projects.json"));
        let one = crate::live::library().create("One", &dir).unwrap().0;
        let two = crate::live::library().create("Two", &dir).unwrap().0;
        let mut w = Workspace::new();
        w.opened(std::slice::from_ref(&one), 1.0);
        assert_eq!((w.mode, w.name.as_str()), (Mode::Inside, "One"));
        w.opened(std::slice::from_ref(&two), 2.0);
        assert_eq!((w.mode, w.name.as_str(), w.since), (Mode::Inside, "Two", 2.0));
        w.opened(&[two.clone()], 3.0);
        assert_eq!(w.since, 2.0);
        assert_eq!(crate::live::shown().map(|(f, _)| f), Some(two));
        crate::live::hide();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn times_ago_in_words() {
        let now = SystemTime::now();
        assert_eq!(ago(Some(now)), "just now");
        assert_eq!(ago(Some(now - std::time::Duration::from_secs(300))), "5 min ago");
        assert_eq!(ago(Some(now - std::time::Duration::from_secs(3 * 86_400))), "3 days ago");
        assert_eq!(ago(None), "");
        assert_eq!((length(12.2), length(99.0), length(600.0)), ("12 s".to_string(), "1:39".to_string(), "10:00".to_string()));
    }

    #[test]
    fn the_grid_fits_the_page() {
        let Some(tx) = texts() else { return };
        let at = layout((1280.0, 800.0), tx, false);
        assert_eq!(at.columns, 5);
        let (a, b) = (card_rect(&at, 0), card_rect(&at, 5));
        assert!(a.0 >= 28.0 && b.1 > a.1 && b.0 == a.0);
    }
}
