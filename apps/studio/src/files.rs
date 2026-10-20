//! The project's folder, in the column at the left of a project: what is in it as folders and
//! files (folders first, by name), the folders open as the person left them. A file the video
//! uses is in ink, with how many layers use it, "silent" when none of them is heard in the
//! composition shown, "missing" when it is not there; what the video does not use is faint
//! (but for the project's renders). A file the video uses from outside the folder is listed
//! under it.
//!
//! Pressing a folder opens or closes it; pressing a sound plays it (again: stops); anything
//! else opens in its own app. Dragged onto the preview or the timeline, a file becomes a layer
//! where the time is. Under the pointer, the layers that use a file light up on the timeline.
//! The folder is read again every second or so, so what the AI adds shows by itself.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path as FsPath, PathBuf};
use std::time::Instant;

use veymelo_engine::project::{FootageKind, ItemKind, Source};
use veymelo_font::Font;
use veymelo_path::Path;
use veymelo_raster::{Canvas, Rgb};

use crate::button::{self, Label, Rect};

const INK: Rgb = Rgb(10, 10, 10);
const GREY: Rgb = Rgb(118, 118, 122);
const FAINT: Rgb = Rgb(168, 168, 174);
const RED: Rgb = Rgb(196, 43, 43);
const TINT: Rgb = Rgb(243, 243, 246);
const RULE: Rgb = Rgb(228, 228, 232);
const EDGE: Rgb = Rgb(161, 161, 170);
/// The warm grey of a file the video uses but nobody hears.
const QUIET: Rgb = Rgb(176, 112, 40);

/// A row's height, how far a level is set in, where the rows start under the column's top, and
/// how often the folder is read again (points, seconds).
pub const ROW: f64 = 26.0;
const INDENT: f64 = 16.0;
const ROWS_TOP: f64 = 56.0;
const READ_EVERY: f64 = 1.0;

/// What a file is, by its name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Folder,
    Sound,
    Picture,
    Video,
    Other,
}

impl Kind {
    pub fn of(path: &FsPath) -> Kind {
        if path.is_dir() {
            return Kind::Folder;
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "wav" | "m4a" | "mp3" | "aif" | "aiff" | "aac" | "flac" | "caf" | "ogg" => Kind::Sound,
            "png" | "jpg" | "jpeg" | "heic" | "tif" | "tiff" | "psd" | "exr" | "svg" | "gif" | "webp" | "bmp" => Kind::Picture,
            "mp4" | "mov" | "m4v" | "webm" | "mkv" | "avi" => Kind::Video,
            _ => Kind::Other,
        }
    }
}

/// How the video uses a file: the layers in the composition shown, how many layers in all,
/// whether none of them is heard, whether the file is not there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Use {
    pub shown: Vec<u64>,
    pub layers: usize,
    pub silent: bool,
    pub missing: bool,
}

/// A row: its file, how deep, its name, what it is, and (a folder) how many things are in it
/// and whether the video uses anything in it.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    path: PathBuf,
    depth: usize,
    name: String,
    kind: Kind,
    inside: usize,
    used: bool,
}

/// What a press on the column asks of the page.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// Open the file in its own app.
    Open(PathBuf),
    /// Make the file a layer, let go at this point (the page knows whether that is on the
    /// preview or the timeline).
    Drop(PathBuf, (f64, f64)),
}

/// The faces the column writes in.
struct Faces {
    title: Font,
    body: Font,
    small: Font,
}

fn faces() -> Option<&'static Faces> {
    static F: std::sync::OnceLock<Option<Faces>> = std::sync::OnceLock::new();
    F.get_or_init(|| Some(Faces { title: crate::faces::sans_at(620.0, 15.0)?, body: crate::faces::sans_at(450.0, 12.5)?, small: crate::faces::sans_at(430.0, 11.0)? })).as_ref()
}

/// Everything under `dir` (names not starting with a dot), folders first, each by name.
fn listing(dir: &FsPath) -> Vec<(PathBuf, bool)> {
    let mut v: Vec<(PathBuf, bool)> = std::fs::read_dir(dir)
        .map(|r| r.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).map(|e| (e.path(), e.file_type().is_ok_and(|t| t.is_dir()))).collect())
        .unwrap_or_default();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.file_name().map(|n| n.to_string_lossy().to_lowercase()).cmp(&b.0.file_name().map(|n| n.to_string_lossy().to_lowercase()))));
    v
}

pub struct Files {
    /// The folder shown and the project's own file in it (not listed).
    root: Option<PathBuf>,
    project: Option<PathBuf>,
    /// The folders open (under the root).
    open: BTreeSet<PathBuf>,
    rows: Vec<Row>,
    /// Files the video uses from outside the folder.
    outside: Vec<PathBuf>,
    uses: BTreeMap<PathBuf, Use>,
    read: Option<Instant>,
    /// What the folder was when last read (its files' names, sizes and times), and when it was
    /// last looked at to tell.
    print: u64,
    looked: std::cell::Cell<Option<Instant>>,
    /// How far the rows are scrolled (points), and where the pointer is.
    down: f64,
    pointer: Option<(f64, f64)>,
    /// A press on a row: its file and where it went down; whether it is being dragged.
    held: Option<(PathBuf, (f64, f64))>,
    pub dragging: bool,
    /// The sound playing, if one is.
    playing: Option<PathBuf>,
}

impl Files {
    pub const fn new() -> Files {
        Files { root: None, project: None, open: BTreeSet::new(), rows: Vec::new(), outside: Vec::new(), uses: BTreeMap::new(), read: None, print: 0, looked: std::cell::Cell::new(None), down: 0.0, pointer: None, held: None, dragging: false, playing: None }
    }

    /// The folders open kept for the project in `file`, else its `assets` and `renders`.
    fn opened(file: &FsPath, root: &FsPath) -> BTreeSet<PathBuf> {
        match crate::prefs::list(&format!("open:{}", file.display())) {
            Some(list) => list.iter().map(|r| root.join(r)).collect(),
            None => [root.join("assets"), root.join("renders")].into(),
        }
    }

    fn keep_opened(&self) {
        let (Some(file), Some(root)) = (&self.project, &self.root) else { return };
        let list: Vec<String> = self.open.iter().filter_map(|p| p.strip_prefix(root).ok()).map(|p| p.display().to_string()).collect();
        crate::prefs::set_list(&format!("open:{}", file.display()), &list);
    }

    /// Reads the folder (and what the video uses) again when it is time, or `now`. Gives
    /// whether what shows changed.
    pub fn refresh(&mut self, now: bool) -> bool {
        if !now && self.read.is_some_and(|r| r.elapsed().as_secs_f64() < READ_EVERY) {
            return false;
        }
        self.read = Some(Instant::now());
        let shown = crate::live::shown();
        let file = shown.as_ref().map(|(f, _)| f.clone());
        if file != self.project {
            self.project = file.clone();
            self.root = file.as_ref().and_then(|f| f.parent().map(FsPath::to_path_buf));
            self.open = match (&file, &self.root) {
                (Some(f), Some(r)) => Files::opened(f, r),
                _ => BTreeSet::new(),
            };
            self.down = 0.0;
        }
        // what the video uses (as it was when the project was last free)
        let mut uses = self.uses.clone();
        if let Some((_, session)) = &shown
            && let Ok(s) = session.try_lock()
        {
            uses.clear();
            let project = s.project();
            let comp = s.view.comp;
            for item in &project.items {
                let ItemKind::Footage(f) = &item.kind else { continue };
                if f.path.is_empty() {
                    continue;
                }
                let path = PathBuf::from(&f.path);
                let u = uses.entry(path.clone()).or_default();
                u.missing = !path.exists();
                for c in project.items.iter().filter_map(|i| if let ItemKind::Comp(c) = &i.kind { Some((i.id, c)) } else { None }) {
                    for l in c.1.layers.iter().filter(|l| l.source == Source::Footage(item.id)) {
                        u.layers += 1;
                        if Some(c.0) == comp {
                            u.shown.push(l.id);
                        }
                    }
                }
                // a sound none of whose layers in the composition shown is heard
                u.silent = f.kind == FootageKind::Audio && !u.shown.is_empty() && u.shown.iter().all(|id| crate::timeline::art(*id).is_some_and(|a| a.peaks.iter().all(|v| *v < 1e-4)));
            }
        }
        let mut rows = vec![];
        if let Some(root) = self.root.clone() {
            self.walk(&root, 0, &uses, &mut rows);
        }
        self.print = self.fingerprint();
        let outside: Vec<PathBuf> = match &self.root {
            Some(root) => uses.keys().filter(|p| !p.starts_with(root)).cloned().collect::<BTreeSet<_>>().into_iter().collect(),
            None => vec![],
        };
        // a sound that has finished
        if self.playing.is_some() && !crate::mac::sound_playing() {
            self.playing = None;
        }
        let changed = rows != self.rows || outside != self.outside || uses != self.uses;
        (self.rows, self.outside, self.uses) = (rows, outside, uses);
        changed
    }

    /// What is in the folder and its open folders now: names, sizes and times.
    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for dir in self.root.iter().chain(self.open.iter()) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                e.file_name().hash(&mut h);
                if let Ok(m) = e.metadata() {
                    m.len().hash(&mut h);
                    m.modified().ok().hash(&mut h);
                }
            }
        }
        h.finish()
    }

    /// Whether the folder changed since it was read (looked at every second or two).
    pub fn stale(&self) -> bool {
        if self.root.is_none() || self.looked.get().is_some_and(|l| l.elapsed().as_secs_f64() < 1.5) {
            return false;
        }
        self.looked.set(Some(Instant::now()));
        self.fingerprint() != self.print
    }

    fn walk(&self, dir: &FsPath, depth: usize, uses: &BTreeMap<PathBuf, Use>, rows: &mut Vec<Row>) {
        for (path, is_dir) in listing(dir) {
            if Some(&path) == self.project.as_ref() {
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // what the video uses, and the project's own renders
            let renders = self.root.as_ref().is_some_and(|r| path.starts_with(r.join("renders")));
            let used = renders || if is_dir { uses.keys().any(|u| u.starts_with(&path)) } else { uses.contains_key(&path) };
            let inside = if is_dir { listing(&path).len() } else { 0 };
            let kind = if is_dir { Kind::Folder } else { Kind::of(&path) };
            rows.push(Row { path: path.clone(), depth, name, kind, inside, used });
            if is_dir && self.open.contains(&path) {
                self.walk(&path, depth + 1, uses, rows);
            }
        }
    }

    /// The layers in the composition shown that use the file under the pointer (to light up).
    pub fn lit(&self, column: Rect) -> Vec<u64> {
        let Some(p) = self.pointer else { return vec![] };
        let path = if self.dragging { self.held.as_ref().map(|h| h.0.clone()) } else { self.row_at(column, p).map(|r| r.path.clone()) };
        path.and_then(|p| self.uses.get(&p)).map(|u| u.shown.clone()).unwrap_or_default()
    }

    /// The rows shown, each with its top (points): the folder's, then those from outside it.
    fn placed(&self, column: Rect) -> Vec<(&Row, f64)> {
        let mut out = vec![];
        let mut y = column.1 + ROWS_TOP - self.down;
        for r in &self.rows {
            out.push((r, y));
            y += ROW;
        }
        out
    }

    fn row_at(&self, column: Rect, p: (f64, f64)) -> Option<&Row> {
        if !button::inside(column, p) || p.1 < column.1 + ROWS_TOP - 4.0 {
            return None;
        }
        self.placed(column).into_iter().find(|(_, y)| p.1 >= *y && p.1 < y + ROW).map(|(r, _)| r)
    }

    pub fn point(&mut self, p: Option<(f64, f64)>) {
        self.pointer = p;
    }

    /// Whether `p` is on a row of the column at `column`.
    pub fn hand(&self, column: Rect, p: (f64, f64)) -> bool {
        self.row_at(column, p).is_some()
    }

    /// The mouse went down at `p` on the column at `column`.
    pub fn down(&mut self, column: Rect, p: (f64, f64)) {
        self.held = self.row_at(column, p).map(|r| (r.path.clone(), p));
        self.dragging = false;
    }

    /// The pointer moved to `p` while held: far enough, the file is being dragged.
    pub fn drag(&mut self, p: (f64, f64)) {
        self.pointer = Some(p);
        if let Some((path, from)) = &self.held
            && !self.dragging
            && (p.0 - from.0).hypot(p.1 - from.1) > 5.0
            && !path.is_dir()
        {
            self.dragging = true;
        }
    }

    /// The mouse came up at `p`: a press on a row (a folder opens or closes, a sound plays or
    /// stops, anything else opens), or a file let go.
    pub fn up(&mut self, column: Rect, p: (f64, f64)) -> Option<Act> {
        let (path, _) = self.held.take()?;
        if std::mem::take(&mut self.dragging) {
            return Some(Act::Drop(path, p));
        }
        if self.row_at(column, p).map(|r| &r.path) != Some(&path) {
            return None;
        }
        match Kind::of(&path) {
            Kind::Folder => {
                if !self.open.remove(&path) {
                    self.open.insert(path);
                }
                self.keep_opened();
                self.refresh(true);
                None
            }
            Kind::Sound => {
                crate::mac::stop_sound();
                if self.playing.take() != Some(path.clone()) && crate::mac::play_sound(&path) {
                    self.playing = Some(path);
                }
                None
            }
            _ => Some(Act::Open(path)),
        }
    }

    /// Scrolled by `dy` points over the column at `column`.
    pub fn scroll(&mut self, column: Rect, dy: f64) {
        let all = self.rows.len() as f64 * ROW + if self.outside.is_empty() { 0.0 } else { 40.0 + self.outside.len() as f64 * ROW };
        let room = column.3 - ROWS_TOP - 12.0;
        self.down = (self.down + dy).clamp(0.0, (all - room).max(0.0));
    }

    /// Whether a sound plays (the page draws its dot).
    pub fn busy(&self) -> bool {
        self.playing.is_some() || self.dragging
    }

    /// Stops the sound playing, if one is (going out of the project).
    pub fn stop(&mut self) {
        if self.playing.take().is_some() {
            crate::mac::stop_sound();
        }
    }

    /// Draws the column at `column` (points; its right edge is where it ends), `alpha` opaque.
    pub fn paint(&self, canvas: &mut Canvas, scale: f64, t: f64, column: Rect, alpha: f32) {
        let Some(f) = faces() else { return };
        let (x0, y0, w, h) = column;
        let px = |v: f64| v * scale;
        canvas.fill_faded(&Label::new(f.title.clone(), 15.0, "Files").path(x0 + 20.0, y0 + 32.0, 1.0, scale), INK, alpha);
        let bottom = y0 + h - 8.0;
        let mut last_y = y0 + ROWS_TOP;
        if self.rows.is_empty() {
            canvas.fill_faded(&Label::new(f.body.clone(), 12.5, "Nothing in the folder yet").path(x0 + 20.0, y0 + ROWS_TOP + 17.0, 1.0, scale), FAINT, alpha);
        }
        for (r, y) in self.placed(column) {
            if y < y0 + ROWS_TOP - 2.0 || y + ROW > bottom {
                last_y = y + ROW;
                continue;
            }
            last_y = y + ROW;
            let held = self.held.as_ref().is_some_and(|h| h.0 == r.path);
            let over = self.pointer.is_some_and(|p| button::inside((x0, y, w, ROW), p));
            if over || held {
                canvas.fill_faded(&Path::rounded(px(x0 + 8.0), px(y + 1.0), px(w - 16.0), px(ROW - 2.0), px(6.0)), TINT, alpha);
            }
            let x = x0 + 14.0 + r.depth as f64 * INDENT;
            let cy = y + ROW / 2.0;
            let u = self.uses.get(&r.path);
            let ink = if r.used { INK } else { FAINT };
            if r.kind == Kind::Folder {
                let open = self.open.contains(&r.path);
                let pts: Vec<(f64, f64)> = if open { vec![(x + 1.5, cy - 2.0), (x + 4.5, cy + 1.5), (x + 7.5, cy - 2.0)] } else { vec![(x + 3.0, cy - 3.5), (x + 6.5, cy), (x + 3.0, cy + 3.5)] };
                canvas.stroke(&pts.iter().map(|(a, b)| (px(*a), px(*b))).collect::<Vec<_>>(), false, 1.1 * scale, if r.used { GREY } else { FAINT }, alpha);
            }
            let ix = x + 12.0;
            if self.playing.as_ref() == Some(&r.path) {
                crate::workspace::pen_dot(canvas, scale, t, (ix + 6.0, cy), 7.0, alpha);
            } else {
                icon(canvas, scale, r.kind, (ix + 6.0, cy), ink, alpha);
            }
            // at the right: how it is used, or how much a closed folder holds
            let (said, colour) = match u {
                Some(u) if u.missing => ("missing".to_string(), RED),
                Some(u) if u.silent => ("silent".to_string(), QUIET),
                Some(u) if u.layers > 0 => (format!("×{}", u.layers), GREY),
                _ if r.kind == Kind::Folder && !self.open.contains(&r.path) && r.inside > 0 => (r.inside.to_string(), FAINT),
                _ => (String::new(), FAINT),
            };
            let mut room = x0 + w - 16.0 - (ix + 18.0);
            if !said.is_empty() {
                let l = Label::new(f.small.clone(), 11.0, &said);
                canvas.fill_faded(&l.path(x0 + w - 16.0 - l.wide, cy + 4.0, 1.0, scale), colour, alpha);
                room -= l.wide + 8.0;
            }
            canvas.fill_faded(&crate::workspace::fit(&f.body, 12.5, &r.name, room.max(20.0)).path(ix + 18.0, cy + 4.5, 1.0, scale), ink, alpha);
        }
        // used from outside the folder
        if !self.outside.is_empty() {
            let mut y = last_y + 14.0;
            if y + 20.0 < bottom {
                canvas.fill_faded(&Path::rounded(px(x0 + 20.0), px(y), px(w - 40.0), px(crate::iso::STROKE), 0.0), RULE, alpha);
                canvas.fill_faded(&Label::new(f.small.clone(), 11.0, "Used from outside this folder").path(x0 + 20.0, y + 22.0, 1.0, scale), GREY, alpha);
            }
            y += 30.0;
            for p in &self.outside {
                if y + ROW > bottom {
                    break;
                }
                let cy = y + ROW / 2.0;
                let u = self.uses.get(p);
                icon(canvas, scale, Kind::of(p), (x0 + 32.0, cy), INK, alpha);
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let said = match u {
                    Some(u) if u.missing => "missing".to_string(),
                    Some(u) => format!("×{}", u.layers),
                    None => String::new(),
                };
                let l = Label::new(f.small.clone(), 11.0, &said);
                canvas.fill_faded(&l.path(x0 + w - 16.0 - l.wide, cy + 4.0, 1.0, scale), if u.is_some_and(|u| u.missing) { RED } else { GREY }, alpha);
                canvas.fill_faded(&crate::workspace::fit(&f.body, 12.5, &name, w - 80.0 - l.wide).path(x0 + 44.0, cy + 4.5, 1.0, scale), INK, alpha);
                y += ROW;
            }
        }
    }

    /// The file being dragged, under the pointer: a small pill with its name, its edge in ink
    /// over where it can go (`can`).
    pub fn paint_drag(&self, canvas: &mut Canvas, scale: f64, can: bool, alpha: f32) {
        let (Some(f), Some((path, _)), Some(p), true) = (faces(), &self.held, self.pointer, self.dragging) else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let l = crate::workspace::fit(&f.body, 12.5, &name, 220.0);
        let (w, h) = (l.wide + 44.0, 28.0);
        let (x, y) = (p.0 + 12.0, p.1 + 8.0);
        let px = |v: f64| v * scale;
        canvas.shadow(px(x), px(y + 4.0), px(w), px(h), px(h / 2.0), px(12.0), INK, alpha * 0.14);
        canvas.fill_faded(&Path::rounded(px(x), px(y), px(w), px(h), px(h / 2.0)), Rgb(255, 255, 255), alpha);
        canvas.stroke(&Path::rounded_points(px(x), px(y), px(w), px(h), px(h / 2.0)), true, crate::iso::STROKE * scale, if can { INK } else { EDGE }, alpha);
        icon(canvas, scale, Kind::of(path), (x + 18.0, y + h / 2.0), INK, alpha);
        canvas.fill_faded(&l.path(x + 30.0, y + h / 2.0 + 4.5, 1.0, scale), INK, alpha);
    }
}

/// A file's kind as a small drawing in hairlines, its middle at `at` (points).
fn icon(canvas: &mut Canvas, scale: f64, kind: Kind, at: (f64, f64), ink: Rgb, alpha: f32) {
    let (cx, cy) = at;
    let p = |dx: f64, dy: f64| ((cx + dx) * scale, (cy + dy) * scale);
    let w = 1.05 * scale;
    match kind {
        Kind::Folder => canvas.stroke(&[p(-6.5, 4.5), p(-6.5, -4.5), p(-2.5, -4.5), p(-1.0, -2.8), p(6.5, -2.8), p(6.5, 4.5)], true, w, ink, alpha),
        Kind::Sound => {
            for (dx, hh) in [(-4.5, 1.5), (-1.5, 4.5), (1.5, 2.5), (4.5, 3.5)] {
                canvas.stroke(&[p(dx, -hh), p(dx, hh)], false, w * 1.1, ink, alpha);
            }
        }
        Kind::Picture => {
            canvas.stroke(&Path::rounded_points((cx - 6.5) * scale, (cy - 5.0) * scale, 13.0 * scale, 10.0 * scale, 1.5 * scale), true, w, ink, alpha);
            canvas.stroke(&[p(-5.0, 3.5), p(-1.5, -0.5), p(1.0, 2.0), p(3.0, 0.0), p(5.0, 3.5)], false, w, ink, alpha);
        }
        Kind::Video => {
            canvas.stroke(&Path::rounded_points((cx - 6.5) * scale, (cy - 5.0) * scale, 13.0 * scale, 10.0 * scale, 1.5 * scale), true, w, ink, alpha);
            canvas.stroke(&[p(-1.5, -2.5), p(2.5, 0.0), p(-1.5, 2.5)], true, w, ink, alpha);
        }
        Kind::Other => canvas.stroke(&[p(-4.5, -6.0), p(1.5, -6.0), p(4.5, -3.0), p(4.5, 6.0), p(-4.5, 6.0)], true, w, ink, alpha),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_known_by_their_names_and_folders_come_first() {
        let d = std::env::temp_dir().join(format!("veymelo-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("b-folder")).unwrap();
        for f in ["a.wav", "C.png", ".hidden", "clip.MOV", "notes.txt"] {
            std::fs::write(d.join(f), "").unwrap();
        }
        let names: Vec<String> = listing(&d).iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["b-folder", "a.wav", "C.png", "clip.MOV", "notes.txt"]);
        assert_eq!([Kind::of(&d.join("a.wav")), Kind::of(&d.join("C.png")), Kind::of(&d.join("clip.MOV")), Kind::of(&d.join("notes.txt")), Kind::of(&d.join("b-folder"))], [Kind::Sound, Kind::Picture, Kind::Video, Kind::Other, Kind::Folder]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
