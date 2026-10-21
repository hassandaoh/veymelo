//! Inside a project: what the AI does, as it does it.
//!
//! On the left, the composition it works on, drawn by our own renderer at the size it shows
//! (on a thread of its own, again whenever the project changes, so the window never waits),
//! lying as a sheet with a soft shadow on dotted paper, as on the opening; what it is (its size,
//! rate and length) goes after the project's name at the top. One with 3D layers can be looked
//! round (the project unchanged): drag over it to turn round what its camera sees, scroll to go
//! nearer or farther; it is drawn at half the detail while it moves, whole once it stops, and
//! "Reset view" goes back to its camera. Under it the timeline
//! (`timeline`: the ruler, the sound, the layers; drag along it to look, Space or the round
//! button to play, the arrows to step a frame). Playing, its sound plays too, and the time the
//! sound has reached is the picture's.
//!
//! On the right, its steps, the newest at the bottom: what each did in words (the command's own
//! name for it and what it worked on, the command itself small at the side), what it was given
//! (with a swatch of each colour) and how long it took when it took a while; one going on has
//! the pen's dot before it; one that failed says why in red; one that only looked is fainter.
//! While no AI is connected, the card that connects one (`connect`) lies at its top.
//!
//! On the left, the project's folder (`files`). Each column folds away with the round button at
//! its top (toward the preview; ⇧⌘\ the files, ⌘\ the steps) to a strip at the window's edge,
//! and the preview takes its room; the window keeps them as they were left. Over all three, a
//! bar across the window: the way back, the project's name and the Render button.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use veymelo_font::Font;
use veymelo_mcp::activity::{self, State, Step};
use veymelo_path::Path;
use veymelo_raster::{Canvas, Rgb};

use crate::button::Label;
use crate::faces;
use crate::motion::{settle, smooth};
use crate::timeline::{self, Hit, Lane, Timeline};

const INK: Rgb = Rgb(10, 10, 10);
const GREY: Rgb = Rgb(118, 118, 122);
const FAINT: Rgb = Rgb(168, 168, 174);
const RULE: Rgb = Rgb(228, 228, 232);
const RED: Rgb = Rgb(196, 43, 43);
/// The dots of the paper the preview lies on, the edges of round buttons.
const DOTS: Rgb = Rgb(200, 200, 206);
const EDGE: Rgb = Rgb(161, 161, 170);
/// The checkerboard behind what is clear.
const CHECK: [u8; 2] = [255, 226];

/// The bar across the top (where its hairline is), the room above the preview, the columns'
/// widths (the steps', the files'), a folded column's strip, the timeline's share of the height
/// (and the least and most it takes), and the room round the page (points).
pub const BAR: f64 = 70.0;
const TOP: f64 = BAR + 14.0;
const PANEL: f64 = 360.0;
const FILES: f64 = 250.0;
const RAIL: f64 = 40.0;
/// The round buttons that fold and open the columns.
const FOLD: f64 = 30.0;
const TIMELINE: (f64, f64, f64) = (0.44, 220.0, 420.0);
/// The least room the preview keeps when the timeline is dragged taller (points).
const PREVIEW_LEAST: f64 = 150.0;
const MARGIN: f64 = 28.0;
/// A step's row height, and the room above the first (points).
const ROW: f64 = 50.0;
const LIST_TOP: f64 = BAR + 92.0;

/// The composition being shown: its name, size, rate and length.
#[derive(Clone, Debug, PartialEq)]
pub struct About {
    pub comp: u64,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    /// Its background colour (0 to 1), and when its last layer ends (seconds).
    pub background: [f64; 3],
    pub last_end: f64,
    /// Whether it has 3D layers (and can be looked round).
    pub three_d: bool,
}

/// How the preview is looked round: as its camera sees it turned `turn` degrees about the
/// vertical and `tilt` up and down, `dolly` times as far from what it looks at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub turn: f64,
    pub tilt: f64,
    pub dolly: f64,
}

impl Look {
    /// As the composition's camera sees it.
    pub const AS_IS: Look = Look { turn: 0.0, tilt: 0.0, dolly: 1.0 };
}

/// What the page asks the drawing thread for: the time, the room (pixels), how it is looked
/// round, and whether a quick draft does (while the look moves).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ask {
    time: f64,
    room: (usize, usize),
    look: Look,
    draft: bool,
}

/// A frame drawn: its pixels (RGBA, over the checkerboard) and size.
struct Frame {
    px: Vec<u8>,
    w: usize,
    h: usize,
}

/// What a frame is drawn for: the project's state, the composition, the time and the room it
/// has (pixels).
#[derive(Clone, Debug, PartialEq)]
struct Key {
    revision: u64,
    comp: u64,
    time: f64,
    room: (usize, usize),
    look: Look,
    draft: bool,
}

/// What the page asks of the drawing thread, and what it gives back.
struct Worker {
    /// What the page asks for.
    ask: Mutex<Ask>,
    frame: Mutex<Option<Arc<Frame>>>,
    about: Mutex<Option<About>>,
    /// The time the AI last set its view to.
    view_time: Mutex<Option<f64>>,
    /// The composition's layers and markers, as of which state of the project.
    lanes: Mutex<(u64, u64, Arc<Vec<Lane>>, Arc<Vec<f64>>)>,
    /// Frames drawn so far (the window draws again when it goes up).
    drawn: AtomicU64,
}

fn worker() -> &'static Worker {
    static WORKER: OnceLock<Worker> = OnceLock::new();
    WORKER.get_or_init(|| {
        let w = Worker { ask: Mutex::new(Ask { time: 0.0, room: (0, 0), look: Look::AS_IS, draft: false }), frame: Mutex::new(None), about: Mutex::new(None), view_time: Mutex::new(None), lanes: Mutex::new((u64::MAX, 0, Arc::new(Vec::new()), Arc::new(Vec::new()))), drawn: AtomicU64::new(0) };
        std::thread::spawn(draw_loop);
        w
    })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The drawing thread: looks at the shown project's session when it is free, and draws the
/// frame the page asks for whenever what it would show has changed.
fn draw_loop() {
    let w = worker();
    let mut last: Option<Key> = None;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(15));
        let Some((_, session)) = crate::live::shown() else {
            *lock(&w.about) = None;
            *lock(&w.frame) = None;
            last = None;
            continue;
        };
        // the project as it is (a step may hold the session: then next time)
        let Ok(s) = session.try_lock() else { continue };
        let (revision, project, comp, view_time) = (s.revision(), s.history().project().clone(), s.view.comp, s.view.time.seconds());
        drop(s);
        *lock(&w.view_time) = Some(view_time);
        let Some(comp) = comp.filter(|c| project.comp(*c).is_some()) else {
            *lock(&w.about) = None;
            *lock(&w.frame) = None;
            last = None;
            continue;
        };
        let c = project.comp(comp).expect("a composition");
        // its layers for the timeline, and its sound for the waveform, as the project is now
        {
            let mut l = lock(&w.lanes);
            if (l.0, l.1) != (revision, comp) {
                let (lanes, markers) = timeline::lanes(&project, comp);
                *l = (revision, comp, Arc::new(lanes), Arc::new(markers));
                w.drawn.fetch_add(1, Ordering::Relaxed);
            }
        }
        timeline::want_art(revision, comp, project.clone());
        let about = About {
            comp,
            name: project.item(comp).map(|i| i.name.clone()).unwrap_or_default(),
            width: c.width,
            height: c.height,
            fps: c.frame_rate.as_f64(),
            duration: c.duration.seconds(),
            background: [c.background.r, c.background.g, c.background.b],
            last_end: c.layers.iter().map(|l| l.out_point.seconds()).fold(0.0, f64::max),
            three_d: c.layers.iter().any(|l| l.is_3d()),
        };
        *lock(&w.about) = Some(about.clone());
        let ask = *lock(&w.ask);
        let room = ask.room;
        // held to a frame of the composition, inside its length
        let fps = about.fps.max(1.0);
        let time = ((ask.time * fps + 1e-6).floor() / fps).clamp(0.0, (about.duration - 1.0 / fps).max(0.0));
        // looked round only when it is 3D
        let look = if about.three_d { ask.look } else { Look::AS_IS };
        let key = Key { revision, comp, time, room, look, draft: ask.draft };
        if last.as_ref() == Some(&key) || room.0 < 8 || room.1 < 8 {
            continue;
        }
        let scale = (room.0 as f64 / about.width.max(1) as f64).min(room.1 as f64 / about.height.max(1) as f64).min(4.0);
        let orbit = (look != Look::AS_IS).then_some(veymelo_engine::render::Orbit { comp, turn: look.turn, tilt: look.tilt, dolly: look.dolly });
        // a draft: half the detail, no motion blur, made as big again after
        let options = if ask.draft { veymelo_engine::render::Options { scale: scale / 2.0, motion_blur: false, orbit, ..Default::default() } } else { veymelo_engine::render::Options { scale, orbit, ..Default::default() } };
        let Ok(r) = veymelo_engine::render::render(&project, comp, veymelo_time::Tick::from_seconds(time), &options) else { continue };
        let img = if ask.draft { grown(&r.image, ((about.width as f64 * scale).round() as usize).max(1), ((about.height as f64 * scale).round() as usize).max(1)) } else { r.image };
        let mut px = Vec::with_capacity(img.width * img.height * 4);
        for (i, p) in img.px.iter().enumerate() {
            let (x, y) = (i % img.width, i / img.width);
            let back = CHECK[(x / 8 + y / 8) % 2] as f32 / 255.0;
            let a = p[3].clamp(0.0, 1.0);
            for c in &p[..3] {
                px.push(((c + back * (1.0 - a)).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            px.push(255);
        }
        *lock(&w.frame) = Some(Arc::new(Frame { px, w: img.width, h: img.height }));
        w.drawn.fetch_add(1, Ordering::Relaxed);
        last = Some(key);
    }
}

/// Picture `img` made `w` × `h` pixels big, between its pixels.
fn grown(img: &veymelo_raster::Image, w: usize, h: usize) -> veymelo_raster::Image {
    let mut out = veymelo_raster::Image::new(w, h);
    let (kx, ky) = (img.width as f64 / w as f64, img.height as f64 / h as f64);
    let (iw, ih) = (img.width as f64, img.height as f64);
    veymelo_raster::par::rows(&mut out.px, w, |y, row| {
        for (x, p) in row.iter_mut().enumerate() {
            let (sx, sy) = (((x as f64 + 0.5) * kx).clamp(0.5, iw - 0.5), ((y as f64 + 0.5) * ky).clamp(0.5, ih - 0.5));
            *p = img.sample(veymelo_raster::Sampling::Bilinear, sx, sy);
        }
    });
    out
}

/// Words in a face at a size, cut to fit `wide` points (with an ellipsis).
fn fitted(font: &Font, size: f64, text: &str, wide: f64) -> String {
    if Label::new(font.clone(), size, text).wide <= wide {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().collect::<String>() + "…";
        if Label::new(font.clone(), size, &s).wide <= wide { lo = mid } else { hi = mid - 1 }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

fn fit(font: &Font, size: f64, text: &str, wide: f64) -> Label {
    Label::new(font.clone(), size, &fitted(font, size, text, wide))
}

/// The colour a `#rrggbb` code at the start of `s` is, if one is.
fn colour_code(s: &str) -> Option<Rgb> {
    let hex = s.strip_prefix('#')?.get(..6)?;
    let v = u32::from_str_radix(hex, 16).ok()?;
    hex.chars().all(|c| c.is_ascii_hexdigit()).then(|| Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// How wide a swatch is with the room after it (points).
const SWATCH: f64 = 13.0;

/// A line of words from (`x`, `baseline`), cut to `wide` points, with a small swatch of each
/// colour written in it (`#rrggbb`) before its code.
#[allow(clippy::too_many_arguments)]
fn line_with_colours(canvas: &mut Canvas, font: &Font, size: f64, text: &str, (x, baseline, wide): (f64, f64, f64), ink: Rgb, scale: f64, alpha: f32) {
    let codes = text.char_indices().filter(|&(i, c)| c == '#' && colour_code(&text[i..]).is_some()).count();
    let text = fitted(font, size, text, wide - codes as f64 * SWATCH);
    let px = |v: f64| v * scale;
    let mut pen = x;
    let mut piece = String::new();
    let mut rest = text.as_str();
    while !rest.is_empty() {
        if let Some(colour) = colour_code(rest) {
            if !piece.is_empty() {
                let l = Label::new(font.clone(), size, &piece);
                l.draw(canvas, pen, baseline, 1.0, scale, ink, alpha);
                pen += l.wide;
                piece.clear();
            }
            // a round swatch, its edge drawn so a light one shows on white
            let (d, cy) = (9.0, baseline - size * 0.36);
            canvas.fill_faded(&Path::rounded(px(pen + 1.0), px(cy - d / 2.0), px(d), px(d), px(d / 2.0)), colour, alpha);
            canvas.stroke(&Path::rounded_points(px(pen + 1.0), px(cy - d / 2.0), px(d), px(d), px(d / 2.0)), true, 0.6 * scale, Rgb(0, 0, 0), alpha * 0.18);
            pen += SWATCH;
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            piece.push(c);
        }
        rest = chars.as_str();
    }
    if !piece.is_empty() {
        Label::new(font.clone(), size, &piece).draw(canvas, pen, baseline, 1.0, scale, ink, alpha);
    }
}

/// The faces the page writes in.
struct Faces {
    title: Font,
    body: Font,
    small: Font,
    head: Font,
    strong: Font,
}

fn faces() -> Option<&'static Faces> {
    static FACES: OnceLock<Option<Faces>> = OnceLock::new();
    FACES
        .get_or_init(|| Some(Faces { title: faces::sans_at(600.0, 13.0)?, body: faces::sans_at(430.0, 12.0)?, small: faces::sans_at(430.0, 11.0)?, head: faces::sans_at(620.0, 15.0)?, strong: faces::sans_at(600.0, 12.0)? }))
        .as_ref()
}

/// Where things are on a page `size` points big, with the files' and the steps' columns
/// `open` of the way open (0 folded, 1 open): the preview's room, the timeline's parts, where
/// the steps' column starts and the files' ends (x).
struct Layout {
    room: (f64, f64, f64, f64),
    tl: timeline::Parts,
    panel: f64,
    files: f64,
}

/// How tall the timeline is on a page `size` points big: as the person dragged it (`asked`),
/// else its share of the page; the preview keeps some room either way.
fn timeline_tall(size: (f64, f64), asked: Option<f64>) -> f64 {
    let most = (size.1 - TOP - PREVIEW_LEAST).max(TIMELINE.1);
    asked.unwrap_or((size.1 * TIMELINE.0).clamp(TIMELINE.1, TIMELINE.2)).clamp(TIMELINE.1, most)
}

fn layout(size: (f64, f64), open: (f64, f64), asked: Option<f64>) -> Layout {
    let (files_open, steps_open) = (open.0.clamp(0.0, 1.0), open.1.clamp(0.0, 1.0));
    let files = RAIL + (FILES - RAIL) * files_open;
    let panel = size.0 - RAIL - (PANEL - RAIL) * steps_open;
    let (left, right) = (files + MARGIN, panel - MARGIN);
    let tall = timeline_tall(size, asked);
    let room = (left, TOP, (right - left).max(10.0), (size.1 - TOP - tall - 14.0).max(10.0));
    let tl = timeline::parts((left - 10.0, size.1 - tall - 4.0, (right - left + 20.0).max(10.0), tall));
    Layout { room, tl, panel, files }
}

/// Where the card that connects an AI lies (while none is): at the top of the steps' column.
fn connect_place(at: &Layout) -> crate::connect::Place {
    crate::connect::Place::Lying { at: (at.panel + 24.0, LIST_TOP - 10.0), wide: PANEL - 48.0 }
}

/// The round buttons that fold and open the columns, at their tops toward the preview (in the
/// strip at the window's edge when folded), the columns `open` of the way open.
fn fold_files(at: &Layout, open: f64) -> (f64, f64, f64, f64) {
    (at.files - FOLD - 5.0 - 11.0 * open, BAR + 13.0, FOLD, FOLD)
}

fn fold_steps(at: &Layout, open: f64) -> (f64, f64, f64, f64) {
    (at.panel + 5.0 + 11.0 * open, BAR + 13.0, FOLD, FOLD)
}

/// A command's own name for itself in sentence case: `New Layer` → `New layer` (words in
/// capitals, like AI or 3D, kept).
fn sentence(label: &str) -> String {
    label
        .split(' ')
        .enumerate()
        .map(|(i, w)| {
            let rest_lower = w.chars().skip(1).any(|c| c.is_lowercase());
            if i > 0 && rest_lower { w.to_lowercase() } else { w.to_string() }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A step's detail parted into what it worked on (its first quoted name) and the rest.
fn subject(detail: &str) -> (&str, &str) {
    if let Some(rest) = detail.strip_prefix('“')
        && let Some(end) = rest.find('”')
    {
        let cut = '“'.len_utf8() + end + '”'.len_utf8();
        let after = detail[cut..].trim_start_matches(" · ");
        return (&detail[..cut], after);
    }
    ("", detail)
}

/// The paper the preview lies on: a grid of dots in `room` (points), fainter toward its edges,
/// as on the opening.
fn paper(canvas: &mut Canvas, scale: f64, room: (f64, f64, f64, f64), alpha: f32) {
    static STAMP: Mutex<Option<(f64, veymelo_raster::Dot)>> = Mutex::new(None);
    let mut kept = lock(&STAMP);
    if kept.as_ref().is_none_or(|k| k.0 != scale) {
        *kept = Some((scale, veymelo_raster::Dot::new(1.05 * scale, 0.0, 0.0)));
    }
    let Some((_, dot)) = kept.as_ref() else { return };
    let (x, y, w, h) = room;
    let step = 22.0;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (nx, ny) = ((w / 2.0 / step).floor() as i64, (h / 2.0 / step).floor() as i64);
    for j in -ny..=ny {
        for i in -nx..=nx {
            let (gx, gy) = (cx + i as f64 * step, cy + j as f64 * step);
            let d = ((gx - cx) / (0.62 * w)).hypot((gy - cy) / (0.66 * h));
            let a = (1.0 - d).max(0.0).powf(1.4) as f32 * alpha;
            if a > 0.01 {
                canvas.stamp(dot, (gx * scale).round() as i64, (gy * scale).round() as i64, DOTS, a);
            }
        }
    }
}

/// A time as minutes, seconds and frames.
pub fn timecode(t: f64, fps: f64) -> String {
    let fps = fps.max(1.0);
    let frames = (t * fps + 1e-6).floor() as i64;
    let per = fps.round() as i64;
    let secs = frames / per;
    format!("{:02}:{:02}:{:02}", secs / 60, secs % 60, frames % per)
}

/// The page's own state: the time looked at, whether it plays (from when, from where), the
/// time bar held, and what the window last showed (to know when to draw again).
pub struct Watch {
    time: f64,
    playing: Option<(f64, f64)>,
    dragging: bool,
    /// How tall the person made the timeline (none: as it comes), and whether they are
    /// dragging its top edge now.
    tall: Option<f64>,
    sizing: bool,
    timeline: Timeline,
    /// Where the pointer is, when the page came in, and the last step's time.
    pointer: Option<(f64, f64)>,
    born: Option<f64>,
    last: Option<f64>,
    /// The AI's view time last followed.
    followed: Option<f64>,
    size: (f64, f64),
    seen: (u64, u64, Option<State>, usize),
    /// Words to put on the clipboard and an app to set up (taken by the page, which asks the
    /// system), and the card that connects an AI.
    pub copy: Option<String>,
    pub set_up: Option<crate::setup::App>,
    connect: crate::connect::Connect,
    /// Whether the steps' column is open, and how far it is (0 to 1); the same for the files'
    /// column; whether they have been set as the window keeps them.
    open: bool,
    panel: crate::motion::Spring,
    files_open: bool,
    files_panel: crate::motion::Spring,
    kept: bool,
    /// The project's folder; a file of it to open in its own app, or to make a layer at a time
    /// (taken by the page).
    files: crate::files::Files,
    pub open_file: Option<std::path::PathBuf>,
    pub add: Option<(std::path::PathBuf, f64)>,
    /// How the preview is looked round, of which composition; where the drag turning it last
    /// was (none: not turning); when it last moved; whether a draft shows while it moves.
    look: Look,
    look_comp: Option<u64>,
    turning: Option<(f64, f64)>,
    moved: f64,
    draft: bool,
}

impl Watch {
    pub const fn new() -> Watch {
        Watch { time: 0.0, playing: None, dragging: false, tall: None, sizing: false, timeline: Timeline::new(), pointer: None, born: None, last: None, followed: None, size: (0.0, 0.0), seen: (0, 0, None, 0), copy: None, set_up: None, connect: crate::connect::Connect::new(), open: true, panel: crate::motion::Spring { x: 1.0, v: 0.0 }, files_open: true, files_panel: crate::motion::Spring { x: 1.0, v: 0.0 }, kept: false, files: crate::files::Files::new(), open_file: None, add: None, look: Look::AS_IS, look_comp: None, turning: None, moved: f64::NEG_INFINITY, draft: false }
    }

    fn layout(&self) -> Layout {
        layout(self.size, (self.files_panel.x, self.panel.x), self.tall)
    }

    /// The files' column (points), sliding out to the left as it folds.
    fn files_rect(&self) -> (f64, f64, f64, f64) {
        let at = self.layout();
        (at.files - FILES, BAR, FILES, self.size.1 - BAR)
    }

    /// The time looked at (seconds).
    pub fn now(&self) -> f64 {
        self.time
    }

    /// Whether `p` is over the preview of a composition that can be looked round.
    fn over_3d(&self, p: (f64, f64)) -> bool {
        self.about().is_some_and(|a| a.three_d) && crate::button::inside(self.layout().room, p)
    }

    /// The button that goes back to the composition's camera (while it is looked round).
    fn reset_rect(&self) -> Option<(f64, f64, f64, f64)> {
        if self.look == Look::AS_IS || !self.about().is_some_and(|a| a.three_d) {
            return None;
        }
        let (x, y, _, _) = self.layout().room;
        let wide = faces().map_or(90.0, |f| crate::button::line_wide(&Label::new(f.small.clone(), 11.0, "Reset view"), crate::button::Icon::None));
        Some((x + 10.0, y + 8.0, wide, 26.0))
    }

    /// Whether `p` is over the preview or the timeline (where a file can be let go).
    pub fn over_middle(&self, p: (f64, f64)) -> bool {
        let at = self.layout();
        p.0 > at.files && p.0 < at.panel && p.1 > BAR
    }

    /// The edge between the preview and the timeline, to drag (points).
    fn edge(&self) -> (f64, f64, f64, f64) {
        let at = self.layout();
        let (x, y, w, _) = at.tl.room;
        (x, y - 12.0, w, 12.0)
    }

    /// Whether the pointer at `p` is on the edge that makes the timeline taller or shorter.
    pub fn on_edge(&self, p: (f64, f64)) -> bool {
        crate::button::inside(self.edge(), p)
    }

    /// Folds the steps' column away, or opens it.
    pub fn toggle_panel(&mut self) {
        self.open = !self.open;
        crate::prefs::set_flag("steps", self.open);
    }

    /// Folds the files' column away, or opens it.
    pub fn toggle_files(&mut self) {
        self.files_open = !self.files_open;
        crate::prefs::set_flag("files", self.files_open);
    }

    /// Leaving the project: its sound stops.
    pub fn leave(&mut self) {
        self.files.stop();
    }

    /// The composition shown, in a line: its size, rate and length (its name too when it is
    /// not the project's).
    pub fn about_line(&self, project: &str) -> Option<String> {
        let a = self.about()?;
        let secs = if a.duration.fract().abs() < 1e-6 { format!("{} s", a.duration) } else { format!("{:.1} s", a.duration) };
        let what = format!("{} × {}  ·  {} fps  ·  {secs}", a.width, a.height, (a.fps * 100.0).round() / 100.0);
        Some(if a.name == project { what } else { format!("{}  ·  {what}", a.name) })
    }

    /// What would show now, to tell whether it changed.
    fn now_seen() -> (u64, u64, Option<State>, usize) {
        let stamp = activity::stamp();
        let drawn = worker().drawn.load(Ordering::Relaxed) + timeline::art_count();
        (drawn, activity::count(), stamp.1, veymelo_mcp::bridged())
    }

    /// The composition's layers and markers, as last read.
    fn lanes() -> (Arc<Vec<Lane>>, Arc<Vec<f64>>) {
        let l = lock(&worker().lanes);
        (l.2.clone(), l.3.clone())
    }

    /// The steps that worked on layers in the project shown: their numbers and the layers.
    fn worked() -> Vec<(u64, Vec<String>)> {
        let file = crate::live::shown().map(|(f, _)| f.display().to_string()).unwrap_or_default();
        activity::steps().into_iter().filter(|s| s.project == file && s.edits && !s.layers.is_empty()).map(|s| (s.n, s.layers)).collect()
    }

    /// Whether the window has to draw again: something new to show, a step running, playing.
    pub fn wants(&self) -> bool {
        let now = Self::now_seen();
        now != self.seen || now.2 == Some(State::Running) || self.playing.is_some() || self.dragging || self.sizing || self.files.busy() || self.files.stale() || self.draft || self.turning.is_some()
    }

    pub fn about(&self) -> Option<About> {
        lock(&worker().about).clone()
    }

    /// Moves on to `t`: plays, follows where the AI looks, asks for the frame. Gives whether
    /// the window has to draw again.
    pub fn step(&mut self, t: f64, scale: f64) -> bool {
        let w = worker();
        self.born.get_or_insert(t);
        let dt = self.last.map_or(0.0, |l| (t - l).clamp(0.0, 0.05));
        self.last = Some(t);
        // the columns as the window keeps them, the first time
        if !self.kept {
            self.kept = true;
            self.open = crate::prefs::flag("steps", true);
            self.files_open = crate::prefs::flag("files", true);
            self.panel.x = if self.open { 1.0 } else { 0.0 };
            self.files_panel.x = if self.files_open { 1.0 } else { 0.0 };
        }
        let springs = self.timeline.step(dt) | self.panel.toward(if self.open { 1.0 } else { 0.0 }, dt, 170.0, 26.0) | self.files_panel.toward(if self.files_open { 1.0 } else { 0.0 }, dt, 170.0, 26.0);
        let springs = springs | self.files.refresh(false);
        let about = self.about();
        if let Some((from, start)) = self.playing {
            let length = about.as_ref().map_or(0.0, |a| a.duration).max(1e-3);
            let time = crate::sound::clock().unwrap_or(start + t - from);
            if time >= length {
                // round again from the start, the sound with it
                self.playing = Some((t, 0.0));
                start_sound(0.0);
                self.time = 0.0;
            } else {
                self.time = time;
            }
        } else if !self.dragging {
            // the AI looked somewhere else: go there
            let vt = *lock(&w.view_time);
            if vt.is_some() && vt != self.followed {
                self.followed = vt;
                self.time = vt.unwrap_or(0.0);
            }
        }
        let at = self.layout();
        let room = ((((at.room.2 - 24.0).max(1.0)) * scale) as usize, ((at.room.3 - 24.0).max(1.0) * scale) as usize);
        // another composition: seen through its own camera again
        let comp = about.as_ref().map(|a| a.comp);
        if comp != self.look_comp {
            (self.look, self.look_comp) = (Look::AS_IS, comp);
        }
        // a draft while the look moves, and a moment after
        self.draft = self.turning.is_some() || t - self.moved < 0.3;
        *lock(&w.ask) = Ask { time: self.time, room, look: self.look, draft: self.draft };
        // the timeline: inside the composition's length, following where the AI works
        if let Some(a) = &about {
            self.timeline.fit(a.duration);
        }
        let (lanes, _) = Self::lanes();
        let room = at.tl.bands.1 - at.tl.bands.0;
        self.timeline.follow(&Self::worked(), &lanes, room, t);
        // (taller now, or fewer rows: no empty room under the last band)
        self.timeline.down = self.timeline.down.min((self.timeline.height(&lanes) - room).max(0.0));
        let changed = self.wants() || springs || self.born.is_some_and(|b| t - b < 1.2) || self.connect.busy(t);
        self.seen = Self::now_seen();
        changed
    }

    /// The mouse went down at `p`: on the round button it plays or stops; on the switch it
    /// lists the layers grouped or in order; on the ruler or the sound it takes the time there
    /// and holds it; on a heading it folds or opens the group; on a layer, it goes to where the
    /// layer comes in.
    pub fn down(&mut self, p: (f64, f64), t: f64) {
        let at = self.layout();
        let (play, tl) = (at.tl.play, &at.tl);
        let lanes = Self::lanes().0;
        let rail = |x0: f64| p.0 >= x0 && p.0 <= x0 + RAIL && p.1 > BAR;
        if self.on_edge(p) {
            self.sizing = true;
        } else if self.reset_rect().is_some_and(|r| crate::button::inside(r, p)) {
            self.look = Look::AS_IS;
            self.moved = t;
        } else if self.over_3d(p) {
            self.turning = Some(p);
            self.moved = t;
        } else if crate::button::inside(fold_files(&at, self.files_panel.x), p) || (!self.files_open && rail(0.0)) {
            self.toggle_files();
        } else if crate::button::inside(fold_steps(&at, self.panel.x), p) || (!self.open && rail(self.size.0 - RAIL)) {
            self.toggle_panel();
        } else if self.files_open && crate::button::inside(self.files_rect(), p) {
            self.files.down(self.files_rect(), p);
        } else if veymelo_mcp::bridged() == 0 && self.open && self.connect.over(p) {
            match self.connect.press(p, t) {
                crate::connect::Press::Copy(words) => self.copy = Some(words),
                crate::connect::Press::SetUp(app) => {
                    self.connect.started();
                    self.set_up = Some(app);
                }
                _ => {}
            }
        } else if (p.0 - play.0).abs() < 16.0 && (p.1 - play.1).abs() < 16.0 {
            self.toggle(t);
        } else if self.timeline.on_time(tl, p) {
            self.halt();
            self.dragging = true;
            self.drag(p);
        } else {
            match self.timeline.hit(tl, p, &lanes) {
                Some(Hit::Head(kind)) => self.timeline.toggle(kind),
                Some(Hit::Block(i)) => {
                    self.halt();
                    self.time = lanes[i].from;
                }
                None => {}
            }
        }
    }

    /// The pointer is at `p` (for what it is over).
    pub fn point(&mut self, p: Option<(f64, f64)>) {
        self.connect.point(p);
        self.files.point(p);
        self.pointer = p;
    }

    /// Stops playing (and its sound).
    fn halt(&mut self) {
        if self.playing.take().is_some() {
            crate::sound::stop();
        }
    }

    /// The pointer scrolled by `d` (points) at `p` at `t`: over the files, they go up and down;
    /// else the layers up and down, the time along.
    pub fn scroll(&mut self, d: (f64, f64), p: (f64, f64), t: f64) {
        if self.files_open && crate::button::inside(self.files_rect(), p) {
            self.files.scroll(self.files_rect(), d.1);
            return;
        }
        // over a 3D preview: nearer (scrolled up) or farther
        if self.over_3d(p) {
            self.look.dolly = (self.look.dolly * (d.1 * 0.004).exp()).clamp(0.15, 8.0);
            self.moved = t;
            return;
        }
        let at = self.layout();
        let length = self.about().map_or(0.0, |a| a.duration);
        let all = self.timeline.height(&Self::lanes().0);
        self.timeline.scroll(d, all, at.tl.bands.1 - at.tl.bands.0, length, at.tl.track.1 - at.tl.track.0, t);
    }

    /// Pinched by `k` (above 0: apart) with the pointer at `p`: the time zooms round it.
    pub fn magnify(&mut self, k: f64, p: (f64, f64)) {
        let at = self.layout();
        let length = self.about().map_or(0.0, |a| a.duration);
        let pivot = if p.0 >= at.tl.track.0 && p.0 <= at.tl.track.1 { self.timeline.time_at(&at.tl, p.0) } else { self.time };
        self.timeline.zoom((1.0 + k).max(0.2), pivot, length);
    }

    /// A key for the timeline: `=` zooms in, `-` out, `0` shows all of it. Gives whether it was one.
    pub fn zoom_key(&mut self, code: u16) -> bool {
        let length = self.about().map_or(0.0, |a| a.duration);
        match code {
            24 => self.timeline.zoom(1.5, self.time, length),
            27 => self.timeline.zoom(1.0 / 1.5, self.time, length),
            29 => self.timeline.whole(length),
            _ => return false,
        }
        true
    }

    /// The mouse came up at `p`: a file of the folder pressed (opened), or let go over the
    /// preview or the timeline (a layer of it where the time is).
    pub fn up(&mut self, p: (f64, f64)) {
        self.dragging = false;
        self.sizing = false;
        self.turning = None;
        match self.files.up(self.files_rect(), p) {
            Some(crate::files::Act::Open(path)) => self.open_file = Some(path),
            Some(crate::files::Act::Drop(path, q)) if self.over_middle(q) => self.add = Some((path, self.time)),
            _ => {}
        }
    }

    /// The time `frames` frames on (back, below 0), held to the composition; playing stops.
    pub fn nudge(&mut self, frames: f64) {
        let Some(a) = self.about() else { return };
        self.halt();
        let fps = a.fps.max(1.0);
        let last = ((a.duration * fps).ceil() - 1.0).max(0.0);
        let now = (self.time * fps + 1e-6).floor();
        self.time = (now + frames).clamp(0.0, last) / fps;
    }

    /// The time at the composition's first frame, or its last; playing stops.
    pub fn to_edge(&mut self, end: bool) {
        let Some(a) = self.about() else { return };
        self.halt();
        let fps = a.fps.max(1.0);
        self.time = if end { ((a.duration * fps).ceil() - 1.0).max(0.0) / fps } else { 0.0 };
    }

    /// The composition's frame rate (30 before there is one).
    pub fn fps(&self) -> f64 {
        self.about().map_or(30.0, |a| a.fps)
    }

    /// The pointer moved to `p` (held on the time bar: the time follows it).
    pub fn drag(&mut self, p: (f64, f64)) {
        self.files.drag(p);
        // turning round what the camera sees, as if turning it by hand: across turns it about the
        // vertical, down tips its top toward the viewer
        if let Some(q) = self.turning {
            self.look.turn += (p.0 - q.0) * 0.35;
            self.look.tilt = (self.look.tilt + (p.1 - q.1) * 0.35).clamp(-85.0, 85.0);
            self.turning = Some(p);
            self.moved = self.last.unwrap_or(0.0);
            return;
        }
        if self.sizing {
            // the timeline's top follows the pointer (its bottom stays at the page's)
            self.tall = Some(timeline_tall(self.size, Some(self.size.1 - 4.0 - p.1 - 6.0)));
            return;
        }
        if !self.dragging {
            return;
        }
        let at = self.layout();
        self.time = self.timeline.time_at(&at.tl, p.0);
    }

    /// Play from here (with its sound), or stop.
    pub fn toggle(&mut self, t: f64) {
        self.playing = match self.playing {
            Some(_) => {
                crate::sound::stop();
                None
            }
            None => {
                start_sound(self.time);
                Some((t, self.time))
            }
        };
    }

    /// Whether the pointer at `p` is over something to press.
    pub fn hand(&self, p: (f64, f64)) -> bool {
        let at = self.layout();
        let play = at.tl.play;
        let lanes = Self::lanes().0;
        let rail = |x0: f64| p.0 >= x0 && p.0 <= x0 + RAIL && p.1 > BAR;
        ((p.0 - play.0).abs() < 16.0 && (p.1 - play.1).abs() < 16.0)
            || self.reset_rect().is_some_and(|r| crate::button::inside(r, p))
            || crate::button::inside(fold_files(&at, self.files_panel.x), p)
            || crate::button::inside(fold_steps(&at, self.panel.x), p)
            || (!self.files_open && rail(0.0))
            || (!self.open && rail(self.size.0 - RAIL))
            || (self.files_open && self.files.hand(self.files_rect(), p))
            || (veymelo_mcp::bridged() == 0 && self.open && self.connect.hand(p))
            || self.timeline.on_time(&at.tl, p)
            || self.timeline.hit(&at.tl, p, &lanes).is_some()
    }

    /// Draws the page on `canvas` (`scale` pixels to the point, `size` points big), `alpha`
    /// faded in, at `t`.
    pub fn paint(&mut self, canvas: &mut Canvas, scale: f64, t: f64, size: (f64, f64), alpha: f32) {
        self.size = size;
        let Some(f) = faces() else { return };
        let at = self.layout();
        let px = |v: f64| v * scale;
        let rect = |x: f64, y: f64, w: f64, h: f64| Path::rounded(px(x), px(y), px(w), px(h), 0.0);
        // the parts come in one after another, each rising a little into place
        let since = t - self.born.unwrap_or(t);
        let part = |delay: f64| ((smooth((since - delay) / 0.45) as f32) * alpha, (1.0 - settle((since - delay) / 0.7)) * 14.0);
        // the preview: the frame on dotted paper, as a sheet with a soft shadow
        let (stage_a, stage_rise) = part(0.0);
        let (rx, ry, rw, rh) = at.room;
        let ry = ry + stage_rise;
        let about = self.about();
        paper(canvas, scale, (rx, ry, rw, rh), stage_a);
        let frame = lock(&worker().frame).clone();
        if let (Some(fr), Some(_)) = (&frame, &about) {
            let (cx, cy) = (px(rx + rw / 2.0) as i64 - fr.w as i64 / 2, px(ry + rh / 2.0) as i64 - fr.h as i64 / 2);
            canvas.shadow(cx as f64, cy as f64 + px(8.0), fr.w as f64, fr.h as f64, px(2.0), px(22.0), INK, stage_a * 0.16);
            blit(canvas, fr, cx, cy, stage_a);
        }
        if about.is_none() {
            let words = Label::new(f.body.clone(), 12.0, "No composition yet: it shows here when the AI makes one");
            words.draw(canvas, rx + (rw - words.wide) / 2.0, ry + rh / 2.0, 1.0, scale, FAINT, stage_a);
        }
        // looked round: the way back to its camera; else, over a 3D one, how to look round
        if let Some(r) = self.reset_rect() {
            let near = if self.pointer.is_some_and(|q| crate::button::inside(r, q)) { 1.0 } else { 0.0 };
            crate::button::line_button(canvas, scale, (r.0, r.1 + stage_rise, r.2, r.3), &Label::new(f.small.clone(), 11.0, "Reset view"), crate::button::Icon::None, near, false, stage_a);
        } else if self.turning.is_none() && self.pointer.is_some_and(|q| self.over_3d(q)) {
            let words = Label::new(f.small.clone(), 11.0, "Drag to look around · scroll to move in");
            words.draw(canvas, rx + (rw - words.wide) / 2.0, ry + rh - 4.0, 1.0, scale, FAINT, stage_a);
        }
        // the edge to drag the timeline taller: a small grip, darker under the pointer
        {
            let (ex, ey, ew, eh) = self.edge();
            let near = self.sizing || self.pointer.is_some_and(|q| crate::button::inside((ex, ey, ew, eh), q));
            let (gx, gy) = (ex + ew / 2.0 - 18.0, ey + eh / 2.0 - 1.5);
            canvas.fill_faded(&Path::rounded(px(gx), px(gy), px(36.0), px(3.0), px(1.5)), if near { GREY } else { RULE }, stage_a);
        }
        // the timeline
        if let Some(a) = &about {
            let (tl_a, tl_rise) = part(0.12);
            let (x, y, w, h) = at.tl.room;
            let tl = timeline::parts((x, y + tl_rise, w, h));
            let (lanes, markers) = Self::lanes();
            let fresh = timeline::freshness(&Self::worked());
            let lit = if self.files_open { self.files.lit(self.files_rect()) } else { vec![] };
            let show = timeline::Show { lanes: &lanes, markers: &markers, fresh: &fresh, lit: &lit, time: self.time, length: a.duration, fps: a.fps, playing: self.playing.is_some(), pointer: self.pointer, body: &f.body, small: &f.small, strong: &f.strong };
            self.timeline.paint(canvas, scale, &tl, &show, tl_a);
        }
        let (alpha, _) = part(0.22);
        let here = veymelo_mcp::bridged() > 0;
        let working = here && activity::stamp().1 == Some(State::Running);
        // the bar's hairline across the window, and the columns' edges under it
        let line = crate::iso::STROKE;
        canvas.fill_faded(&rect(0.0, BAR, size.0, line), RULE, alpha);
        canvas.fill_faded(&rect(at.files, BAR, line, size.1 - BAR), RULE, alpha);
        canvas.fill_faded(&rect(at.panel, BAR, line, size.1 - BAR), RULE, alpha);
        // the buttons that fold the columns away or bring them back: a page with its column
        // at the left (the files), at the right (the steps, with how the AI is)
        for (b, left) in [(fold_files(&at, self.files_panel.x), true), (fold_steps(&at, self.panel.x), false)] {
            let (fx, fy, fw, fh) = b;
            let near = self.pointer.is_some_and(|p| crate::button::inside(b, p));
            if near {
                canvas.fill_faded(&Path::rounded(px(fx), px(fy), px(fw), px(fh), px(fw / 2.0)), RULE, alpha);
            }
            canvas.stroke(&Path::rounded_points(px(fx), px(fy), px(fw), px(fh), px(fw / 2.0)), true, line * scale, if near { INK } else { EDGE }, alpha);
            let (gx, gy) = (fx + 8.0, fy + 9.0);
            canvas.stroke(&Path::rounded_points(px(gx), px(gy), px(14.0), px(12.0), px(2.5)), true, 1.2 * scale, INK, alpha);
            canvas.fill_faded(&rect(if left { gx + 4.3 } else { gx + 8.5 }, gy, 1.2, 12.0), INK, alpha);
            if !left && working {
                crate::workspace::pen_dot(canvas, scale, t, (fx + fw - 4.0, fy + 4.0), 7.0, alpha);
            }
        }
        // the files' column, sliding in from the left as it opens
        let files_a = alpha * self.files_panel.x.clamp(0.0, 1.0) as f32;
        if files_a > 0.01 {
            self.files.paint(canvas, scale, t, self.files_rect(), files_a);
        }
        // a file being dragged, over all
        let over = self.pointer.is_some_and(|p| self.over_middle(p));
        // the steps' column, sliding in from the right as it opens
        let open = self.panel.x.clamp(0.0, 1.0) as f32;
        let steps_a = alpha * open;
        if steps_a <= 0.01 {
            self.files.paint_drag(canvas, scale, over, alpha);
            return;
        }
        let alpha = steps_a;
        let x = at.panel;
        let (lx, lw) = (x + 24.0, PANEL - 48.0);
        let tx = x + 16.0 + FOLD + 12.0;
        Label::new(f.head.clone(), 15.0, "What the AI does").draw(canvas, tx, BAR + 26.0, 1.0, scale, INK, alpha);
        // how the AI is: working (the pen's dot), there, or not connected (an empty ring)
        let sy = BAR + 40.0;
        let (status, ink) = if working {
            crate::workspace::pen_dot(canvas, scale, t, (tx + 3.5, sy + 3.5), 7.0, alpha);
            ("Working", INK)
        } else if here {
            canvas.fill_faded(&Path::rounded(px(tx), px(sy), px(7.0), px(7.0), px(3.5)), INK, alpha);
            ("AI connected", GREY)
        } else {
            canvas.stroke(&Path::rounded_points(px(tx + 0.5), px(sy + 0.5), px(6.0), px(6.0), px(3.0)), true, 1.1 * scale, FAINT, alpha);
            ("AI not connected", GREY)
        };
        fit(&f.small, 11.0, status, lw - 14.0).draw(canvas, tx + 13.0, sy + 7.0, 1.0, scale, ink, alpha);
        canvas.fill_faded(&rect(lx, BAR + 64.0, lw, 1.0), RULE, alpha);
        // with none connected: the card that connects one
        let mut list_top = LIST_TOP;
        if here {
            self.connect.close();
        } else {
            self.connect.show(connect_place(&at), t);
            self.connect.paint(canvas, scale, t, alpha);
            if let Some((_, cy, _, ch)) = self.connect.rect() {
                list_top = cy + ch + 22.0;
            }
        }
        // this project's steps
        let file = crate::live::shown().map(|(f, _)| f.display().to_string()).unwrap_or_default();
        let steps: Vec<Step> = activity::steps().into_iter().filter(|s| s.project == file).collect();
        if steps.is_empty() {
            let lines = if here { ["Nothing yet.", "Ask the AI to work in this project: what it does", "shows here, step by step."] } else { ["Nothing yet.", "Once an AI is connected, what it does in this", "project shows here, step by step."] };
            for (k, line) in lines.iter().enumerate() {
                Label::new(f.body.clone(), 12.0, line).draw(canvas, lx, list_top + 10.0 + 18.0 * k as f64, 1.0, scale, if k == 0 { GREY } else { FAINT }, alpha);
            }
            self.files.paint_drag(canvas, scale, over, alpha);
            return;
        }
        // the newest at the bottom: as many as fit
        let fits = (((size.1 - list_top - 16.0) / ROW).floor() as usize).max(1);
        let shown = &steps[steps.len().saturating_sub(fits)..];
        for (k, s) in shown.iter().enumerate() {
            let top = list_top + ROW * k as f64;
            row(canvas, f, s, (lx, top, lw), scale, t, alpha);
        }
        self.files.paint_drag(canvas, scale, over, steps_a.max(alpha));
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if self.playing.is_some() {
            crate::sound::stop();
        }
    }
}

/// The shown composition's sound plays from `from` (none when the project is busy or silent).
fn start_sound(from: f64) {
    let Some((_, session)) = crate::live::shown() else { return };
    let Ok(s) = session.try_lock() else { return };
    let (project, comp) = (s.history().project().clone(), s.view.comp);
    drop(s);
    if let Some(comp) = comp {
        crate::sound::play(project, comp, from);
    }
}

/// One step's row: what it did in words (its own name for it, and what it worked on) with the
/// command it was at the right; under it, what it was given (or why it failed) and how long it
/// took when it took a while. One still going has the pen's dot before it; one that only looked
/// is fainter.
fn row(canvas: &mut Canvas, f: &Faces, s: &Step, (x, top, w): (f64, f64, f64), scale: f64, t: f64, alpha: f32) {
    let looks = !s.edits;
    if s.state == State::Running {
        crate::workspace::pen_dot(canvas, scale, t, (x + 3.0, top + 11.5), 6.0, alpha);
    }
    let x0 = x + 14.0;
    let room = w - 14.0;
    // the command, or who made it when it was the person
    let id = Label::new(if s.person { f.strong.clone() } else { f.small.clone() }, 10.5, if s.person { "You" } else { &s.title });
    id.draw(canvas, x + w - id.wide, top + 16.0, 1.0, scale, if s.person { INK } else { FAINT }, alpha);
    let (what, rest) = subject(&s.detail);
    let said = sentence(if s.label.is_empty() { &s.title } else { &s.label });
    let first = room - id.wide - 12.0;
    let l = fit(&f.title, 13.0, &said, first);
    l.draw(canvas, x0, top + 16.0, 1.0, scale, if looks { GREY } else { INK }, alpha);
    if !what.is_empty() && first - l.wide - 6.0 > 24.0 {
        fit(&f.body, 13.0, what, first - l.wide - 6.0).draw(canvas, x0 + l.wide + 5.0, top + 16.0, 1.0, scale, if looks { GREY } else { INK }, alpha);
    }
    // under it: why it failed, or what it was given and how long it took
    let took = s.took.filter(|d| d.as_secs_f64() >= 0.05).map(|d| {
        let ms = d.as_secs_f64() * 1000.0;
        if ms < 1000.0 { format!("{} ms", ms.round()) } else { format!("{:.1} s", ms / 1000.0) }
    });
    let second = match s.state {
        State::Failed => s.note.clone(),
        _ => match (rest.is_empty(), &took) {
            (false, Some(d)) => format!("{rest}  ·  {d}"),
            (false, None) => rest.to_string(),
            (true, Some(d)) => d.clone(),
            (true, None) => String::new(),
        },
    };
    if !second.is_empty() {
        line_with_colours(canvas, &f.small, 11.0, &second, (x0, top + 33.0, room), if s.state == State::Failed { RED } else { GREY }, scale, alpha);
    }
}

/// A frame's pixels put on the canvas with their top left at (`x`, `y`) pixels.
fn blit(canvas: &mut Canvas, fr: &Frame, x: i64, y: i64, alpha: f32) {
    let (cw, ch) = (canvas.width as i64, canvas.height as i64);
    for row in 0..fr.h as i64 {
        let ty = y + row;
        if ty < 0 || ty >= ch {
            continue;
        }
        for col in 0..fr.w as i64 {
            let tx = x + col;
            if tx < 0 || tx >= cw {
                continue;
            }
            let s = ((row as usize) * fr.w + col as usize) * 4;
            let d = ((ty * cw + tx) as usize) * 4;
            for c in 0..3 {
                let (a, b) = (canvas.pixels[d + c] as f32, fr.px[s + c] as f32);
                canvas.pixels[d + c] = (a + (b - a) * alpha).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_words_fit() {
        assert_eq!(timecode(61.5, 30.0), "01:01:15");
        assert_eq!(timecode(0.0, 23.976), "00:00:00");
        if let Some(f) = faces() {
            let l = fit(&f.body, 12.0, "a very long line of words that will not fit in the room", 80.0);
            assert!(l.wide <= 80.0 + 1e-9);
        }
    }
}
