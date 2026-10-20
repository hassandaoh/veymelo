//! What the window shows when it opens: a faint grid of dots comes out from the middle, a caret
//! appears, and two lines are typed one letter at a time, at a person's uneven pace: "hello",
//! then under it "welcome to *veymelo*". Each letter rises a little into place as it
//! fades in; the caret glides after the typing and down to the second line, and blinks once
//! all is typed. Then a black "Get started" button rises in under the words.
//!
//! It answers the pointer: the dots near it darken and grow, and leave a trail that fades; the
//! letters near it grow heavier and lift, the farther ones a moment later; the words and the
//! dots slide a little against the pointer, the words more (they are nearer); and the button
//! leans toward it. Each answer is a spring, so it moves smoothly and comes back to rest when
//! the pointer leaves.
//!
//! Pressing the button (or Return) goes in: the button sinks away, the other words and the
//! dots fade, "veymelo" glides to the top left as the app's name, and the workspace
//! comes in (see `workspace`).

use crate::button::{Button, Label, Rect};
use veymelo_path::Path;
use veymelo_raster::{Canvas, Dot, Rgb};
use crate::faces;
use veymelo_font::Font;
use crate::motion::{jitter, reach, settle, smooth, Spring};
use crate::workspace::{Cursor, Workspace};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const PAPER: Rgb = Rgb(255, 255, 255);
const INK: Rgb = Rgb(10, 10, 10);
const GREY: Rgb = Rgb(118, 118, 122);
const DOTS: Rgb = Rgb(196, 196, 202);
/// A dot right under the pointer.
const DOT_LIT: Rgb = Rgb(112, 112, 120);

#[derive(Clone, Copy, PartialEq)]
enum Face {
    /// The system's sans serif (SF).
    Sans,
    /// The system's serif italic (New York Italic).
    Serif,
}

/// Words in one face, weight (100 to 900), size (points), colour and letter spacing (ems).
struct Run {
    text: &'static str,
    face: Face,
    weight: f64,
    size: f64,
    ink: Rgb,
    track: f64,
}

/// A line to type: its runs, the pause before its first letter and the usual time between
/// its letters (seconds).
struct Line {
    runs: &'static [Run],
    pause: f64,
    pace: f64,
}

const LINES: &[Line] = &[
    Line { runs: &[Run { text: "hello", face: Face::Sans, weight: 620.0, size: 76.0, ink: INK, track: -0.03 }], pause: 1.05, pace: 0.13 },
    Line {
        runs: &[
            Run { text: "welcome to ", face: Face::Sans, weight: 420.0, size: 26.0, ink: GREY, track: 0.0 },
            Run { text: "veymelo", face: Face::Serif, weight: 400.0, size: 29.0, ink: INK, track: 0.0 },
        ],
        pause: 0.5,
        pace: 0.06,
    },
];
/// The space between the lines, in points.
const LEADING: f64 = 16.0;
/// The run that becomes the app's name: (line, run).
const NAME: (usize, usize) = (1, 1);

/// The button: its words, their size and weight; the space above it (points); and when it
/// comes in after the typing (seconds).
const BUTTON_TEXT: &str = "Get started";
const BUTTON_SIZE: f64 = 17.0;
const BUTTON_WEIGHT: f64 = 560.0;
const BUTTON_GAP: f64 = 44.0;
const BUTTON_AFTER: f64 = 0.45;

/// The dots: their spacing and radius (points); when they start to come out from the middle,
/// and how long the front takes to reach the edges (seconds).
const DOT_SPACING: f64 = 22.0;
const DOT_RADIUS: f64 = 1.05;
const DOTS_FROM: f64 = 0.1;
const DOTS_SPREAD: f64 = 1.5;
/// How soft the front is, as a share of the way out.
const DOTS_EDGE: f64 = 0.25;

/// When the caret fades in, how long a letter takes to come into place, and how long the
/// caret stays on, then off, once all is typed (seconds).
const CARET_IN: f64 = 0.55;
const ENTER: f64 = 0.6;
const BLINK: f64 = 0.53;
/// How quickly the caret catches up: along a line, and down to the next (seconds). Going down
/// it drops first and slides across after `CARET_WAIT`, so it never crosses the letters.
const CARET_ALONG: f64 = 0.045;
const CARET_DOWN: f64 = 0.06;
const CARET_WAIT: f64 = 0.1;

/// How far a letter feels the pointer (points, plus a share of its size), how much heavier
/// and how much higher (a share of its size) it gets right under it, and how much later a
/// letter answers for each reach of distance (seconds).
const HOVER_REACH: f64 = 40.0;
const HOVER_REACH_SIZE: f64 = 0.6;
const HOVER_WEIGHT: f64 = 200.0;
const HOVER_LIFT: f64 = 0.07;
const STAGGER: f64 = 0.04;
/// How far the dots feel the pointer (points), how long its trail lasts (seconds), and how
/// much bigger a dot right under it gets.
const DOT_REACH: f64 = 90.0;
const TRAIL: f64 = 0.6;
const DOT_GROW: f64 = 0.7;
/// On the projects, how strong the dots are, the same all over the page (the opening's fade
/// toward its edges gone).
const HOME_DOTS: f64 = 0.62;
/// How far the words and the dots slide (points) with the pointer at the window's edge.
const SHIFT_WORDS: f64 = 12.0;
const SHIFT_DOTS: f64 = 5.0;

/// Going in: how long the button takes to sink away, the other words to fade (and how much
/// later each next letter starts) and the dots to fade; when the name starts to glide (once
/// the words it crosses have faded) and how long it takes (seconds); where it ends, its size
/// (points from the top left); and how far into its glide the workspace starts to come in.
const GO_BUTTON: f64 = 0.3;
const GO_FADE: f64 = 0.3;
const GO_STAGGER: f64 = 0.015;
const GO_DOTS: f64 = 0.7;
const GO_NAME_AFTER: f64 = 0.32;
const GO_NAME: f64 = 0.9;
const NAME_AT: (f64, f64) = (28.0, 40.0);
const NAME_SIZE: f64 = 20.0;
const WORK_AFTER: f64 = 0.55;

/// One letter: its line and run, glyph, where it starts and how wide it is (points, from the
/// line's start), and when it is typed.
struct Letter {
    line: usize,
    run: usize,
    glyph: u16,
    x: f64,
    adv: f64,
    at: f64,
}

/// Everything laid out once: the fonts of each run, the letters, each line's width and size,
/// where the caret goes when (time, line, points from the line's start, how quickly), and the
/// button's words.
struct Scene {
    sans: &'static Font,
    serif: &'static Font,
    fonts: Vec<Vec<Font>>,
    letters: Vec<Letter>,
    lines: Vec<(f64, f64)>,
    caret: Vec<(f64, usize, f64, f64)>,
    /// When the last letter is typed.
    done: f64,
    button: Label,
    /// How wide the app's name is at its size at the top left (points).
    name_wide: f64,
}

impl Scene {
    /// When the button comes in.
    fn button_at(&self) -> f64 {
        self.done + BUTTON_AFTER
    }
}

fn scene() -> Option<&'static Scene> {
    static SCENE: OnceLock<Option<Scene>> = OnceLock::new();
    SCENE
        .get_or_init(|| {
            let (sans, serif) = (faces::sans()?, faces::serif()?);
            let mut fonts = vec![];
            let mut letters = vec![];
            let mut lines = vec![];
            let mut caret = vec![(CARET_IN, 0, 0.0, 0.0)];
            let mut t = 0.0;
            let mut count = 0;
            let mut name_wide = 0.0;
            for (li, line) in LINES.iter().enumerate() {
                let mut faces = vec![];
                let mut pen = 0.0;
                let mut first = true;
                let mut prev = ' ';
                if li > 0 {
                    // the caret goes down a little before the line is typed
                    caret.push((t + line.pause * 0.45, li, 0.0, CARET_DOWN));
                }
                for (ri, run) in line.runs.iter().enumerate() {
                    let base = if run.face == Face::Serif { serif } else { sans };
                    // the weight asked for, and the shapes drawn for this size
                    let f = base.at(&[("wght", run.weight), ("opsz", run.size)]);
                    let k = run.size / f.units;
                    let start = pen;
                    for c in run.text.chars() {
                        let g = f.glyph(c);
                        // a person's pace: uneven, a little slower at the start of a word
                        t += if first {
                            line.pause
                        } else {
                            let mut wait = line.pace * (0.65 + 0.8 * jitter(count));
                            if prev == ' ' {
                                wait *= 1.5;
                            }
                            if c == ' ' {
                                wait *= 0.8;
                            }
                            wait
                        };
                        first = false;
                        let adv = f.advance(g) * k;
                        letters.push(Letter { line: li, run: ri, glyph: g, x: pen, adv, at: t });
                        pen += adv + run.track * run.size;
                        caret.push((t, li, pen, CARET_ALONG));
                        prev = c;
                        count += 1;
                    }
                    if (li, ri) == NAME {
                        name_wide = (pen - start) * NAME_SIZE / run.size;
                    }
                    faces.push(f);
                }
                let size = line.runs.iter().map(|r| r.size).fold(0.0, f64::max);
                lines.push((pen, size));
                fonts.push(faces);
            }
            let button = Label::new(sans.at(&[("wght", BUTTON_WEIGHT), ("opsz", BUTTON_SIZE)]), BUTTON_SIZE, BUTTON_TEXT);
            Some(Scene { sans, serif, fonts, letters, lines, caret, done: t, button, name_wide })
        })
        .as_ref()
}

/// What the pointer has done, and the springs it moves.
struct Live {
    /// When the springs last moved on.
    last: Option<f64>,
    /// Where the pointer is (points from the window's top left), if it is over the window.
    pointer: Option<(f64, f64)>,
    /// Where it has been lately (time, x, y), oldest first.
    trail: VecDeque<(f64, f64, f64)>,
    /// Each letter's answer (0 to 1).
    hover: Vec<Spring>,
    /// How far the layers have slid (-1 to 1): across, down.
    tilt: [Spring; 2],
    /// The "Get started" button, and when it was pressed to go in.
    button: Button,
    gone: Option<f64>,
    /// The page after the opening.
    work: Workspace,
    /// The window's size in points, as last drawn.
    size: (f64, f64),
    /// Something is still moving.
    moving: bool,
    /// The dots ready to put down, at one scale: eight sizes, each at sixteen places within a
    /// pixel.
    stamps: Option<(f64, Vec<Dot>)>,
    /// The paper with its dots at rest, kept once they have come out: scale, whether it is the
    /// projects' paper, picture.
    kept: Option<(f64, bool, Canvas)>,
    /// Pixels to the point, as last drawn.
    scale: f64,
}

static LIVE: Mutex<Live> = Mutex::new(Live {
    last: None,
    pointer: None,
    trail: VecDeque::new(),
    hover: Vec::new(),
    tilt: [Spring::new(); 2],
    button: Button::new(),
    gone: None,
    work: Workspace::new(),
    size: (0.0, 0.0),
    moving: false,
    stamps: None,
    kept: None,
    scale: 1.0,
});

fn live() -> std::sync::MutexGuard<'static, Live> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The pointer is at `at` (points from the window's top left), or has left the window, `t`
/// seconds after it opened. Gives what the pointer should look like there.
pub fn pointer(at: Option<(f64, f64)>, t: f64) -> Cursor {
    let mut l = live();
    if let Some(p) = at
        && l.pointer != Some(p)
    {
        l.trail.push_back((t, p.0, p.1));
    }
    l.pointer = at;
    l.moving = true;
    if let (Some(p), Some(_)) = (at, l.gone) {
        l.work.moved(p);
    }
    match (at, l.gone) {
        (Some(p), None) if l.button.hit(p) => Cursor::Hand,
        (Some(p), Some(_)) => l.work.cursor(p, l.size),
        _ => Cursor::Arrow,
    }
}

/// What the window has to ask the system for now (a folder, a file), taken: the window asks,
/// outside the page, then gives the answer to [`answer`].
pub fn take_ask() -> Option<crate::workspace::Ask> {
    live().work.take_ask()
}

/// What a render will be called to start with, and the folder it goes in.
pub fn render_defaults() -> Option<(String, std::path::PathBuf)> {
    live().work.render_defaults()
}

/// The file to show in Finder, taken.
pub fn take_copying() -> Option<String> {
    live().work.take_copying()
}

pub fn take_revealing() -> Option<std::path::PathBuf> {
    live().work.take_revealing()
}

/// The file to open in its own app, taken.
pub fn take_opening() -> Option<std::path::PathBuf> {
    live().work.take_opening()
}

/// Files let go on the window from Finder at `at`, at `t`.
pub fn dropped(paths: &[std::path::PathBuf], at: (f64, f64), t: f64) {
    let mut l = live();
    l.moving = true;
    if l.gone.is_none() {
        l.gone = Some(t);
    }
    l.work.dropped(paths, at, t);
}

/// Projects opened from Finder at `t`: the first is shown, past the opening.
pub fn opened(paths: &[std::path::PathBuf], t: f64) {
    let mut l = live();
    l.moving = true;
    if l.gone.is_none() {
        l.gone = Some(t);
    }
    l.work.opened(paths, t);
}

/// The app to set up to reach this one, taken.
pub fn take_setting_up() -> Option<crate::setup::App> {
    live().work.take_setting_up()
}

/// The name of the project being deleted (for the system to ask about).
pub fn deleting_name() -> Option<String> {
    live().work.deleting_name()
}

/// What goes to the Trash for the project being deleted, or why it cannot.
pub fn trash_target() -> Result<std::path::PathBuf, String> {
    live().work.trash_target()
}

/// The answer to deleting a project (and how moving it to the Trash went), at `t`.
pub fn answer_delete(choice: crate::workspace::Delete, trashed: Option<Result<(), String>>, t: f64) {
    let mut l = live();
    l.moving = true;
    l.work.answer_delete(choice, trashed, t);
}

/// The system's answer to `ask` at `t`: where the person chose, or none.
pub fn answer(ask: crate::workspace::Ask, path: Option<std::path::PathBuf>, t: f64) {
    let mut l = live();
    l.moving = true;
    l.work.answer(ask, path, t);
}

/// What the pointer should look like where it is now (after a click or a key has changed
/// what is under it).
pub fn cursor_now() -> Cursor {
    let l = live();
    match (l.pointer, l.gone) {
        (Some(p), None) if l.button.hit(p) => Cursor::Hand,
        (Some(p), Some(_)) => l.work.cursor(p, l.size),
        _ => Cursor::Arrow,
    }
}

/// The mouse button went down (`down`) or up at `at`, `t` seconds after the window opened: a
/// press that starts and ends on the button goes in; on the workspace, it goes to the page.
pub fn press(down: bool, at: (f64, f64), t: f64) {
    let mut l = live();
    l.moving = true;
    if l.gone.is_some() {
        if down { l.work.down(at, t) } else { l.work.up(at, t) }
    } else if down {
        l.button.down(at);
    } else if l.button.up(at) {
        l.gone = Some(t);
    }
}

/// A key, `t` seconds after the window opened: its code and what it types. Return presses
/// the button once it is there; on the workspace, keys go to the page.
#[cfg(test)]
pub fn key(code: u16, text: &str, t: f64) {
    key_with(code, text, false, t);
}

/// A key, with whether Shift is down.
pub fn key_with(code: u16, text: &str, shift: bool, t: f64) {
    let mut l = live();
    l.moving = true;
    match l.gone {
        None if (code == 36 || code == 76) && l.button.rest.is_some() => l.gone = Some(t),
        Some(_) => {
            l.work.key(code, text, shift, t);
        }
        None => {}
    }
}

/// Words from the clipboard, for the field typed in.
pub fn paste(words: &str) {
    let mut l = live();
    l.moving = true;
    l.work.paste(words);
}

/// All of the field typed in chosen.
pub fn select_all() {
    let mut l = live();
    l.moving = true;
    l.work.select_all();
}

/// What a menu asks, at `t` (from the opening, it goes on to the projects first).
pub fn command(c: crate::workspace::Command, t: f64) {
    let mut l = live();
    l.moving = true;
    if l.gone.is_none() {
        l.gone = Some(t);
    }
    l.work.command(c, t);
}

/// The pointer scrolled by `d` (points; down and right positive) over `at`, at `t`.
pub fn scroll(d: (f64, f64), at: (f64, f64), t: f64) {
    let mut l = live();
    l.moving = true;
    if l.gone.is_some() {
        l.work.scroll(d, at, t);
    }
}

/// A pinch by `k` (above 0: apart) over `at`.
pub fn magnify(k: f64, at: (f64, f64)) {
    let mut l = live();
    l.moving = true;
    if l.gone.is_some() {
        l.work.magnify(k, at);
    }
}

/// Where the pointer was at `when`: the newest place in the trail no newer than that.
fn pointer_then(trail: &VecDeque<(f64, f64, f64)>, now: (f64, f64), when: f64) -> (f64, f64) {
    match trail.iter().rev().find(|s| s.0 <= when) {
        Some(s) => (s.1, s.2),
        None => trail.front().map_or(now, |s| (s.1, s.2)),
    }
}

impl Live {
    /// Moves everything on to `t`: the letters at `centres` (x, y, reach, typed) toward how
    /// near the pointer is, the layers toward where it is in a window `size` points big, the
    /// button (resting at `button`, if it is there) and the workspace.
    fn step(&mut self, t: f64, centres: &[(f64, f64, f64, bool)], size: (f64, f64), button: Option<Rect>) {
        let dt = self.last.map_or(0.0, |l| (t - l).clamp(0.0, 0.05));
        self.last = Some(t);
        self.size = size;
        while self.trail.front().is_some_and(|s| t - s.0 > TRAIL) {
            self.trail.pop_front();
        }
        // once it has gone in, the pointer moves nothing of the opening
        let pointer = if self.gone.is_some() { None } else { self.pointer };
        let mut moving = !self.trail.is_empty();
        self.hover.resize(centres.len(), Spring::new());
        for (h, &(cx, cy, r, typed)) in self.hover.iter_mut().zip(centres) {
            let aim = match pointer {
                Some(p) if typed => {
                    // the farther letters answer a moment later
                    let delay = STAGGER * ((p.0 - cx).hypot(p.1 - cy) / r).min(3.0);
                    let q = pointer_then(&self.trail, p, t - delay);
                    reach((q.0 - cx).hypot(q.1 - cy), r)
                }
                _ => 0.0,
            };
            moving |= h.toward(aim, dt, 170.0, 22.0);
        }
        let (ax, ay) = pointer.map_or((0.0, 0.0), |p| (((p.0 - size.0 / 2.0) / (size.0 / 2.0)).clamp(-1.0, 1.0), ((p.1 - size.1 / 2.0) / (size.1 / 2.0)).clamp(-1.0, 1.0)));
        moving |= self.tilt[0].toward(ax, dt, 60.0, 15.5);
        moving |= self.tilt[1].toward(ay, dt, 60.0, 15.5);
        moving |= self.button.step(if self.gone.is_none() { button } else { None }, pointer, dt);
        if let Some(g) = self.gone {
            if t >= g + GO_NAME_AFTER + GO_NAME * WORK_AFTER {
                self.work.show(g + GO_NAME_AFTER + GO_NAME * WORK_AFTER);
            }
            moving |= self.work.step(t, dt, size, self.pointer, self.scale);
        }
        self.moving = moving;
    }

    /// Nothing the pointer does shows (it is away, and all is at rest).
    fn at_rest(&self) -> bool {
        self.pointer.is_none() && self.trail.is_empty() && self.tilt.iter().all(|s| s.x == 0.0 && s.still())
    }
}

/// Whether something is still moving at `t` (the window draws every frame until nothing is).
pub fn busy(t: f64) -> bool {
    let Some(s) = scene() else { return false };
    let caret_still = s.caret.last().map_or(0.0, |c| c.0 + CARET_WAIT + 8.0 * CARET_DOWN);
    let l = live();
    let going = l.gone.is_some_and(|g| t < g + GO_NAME_AFTER + GO_NAME + 0.2);
    // what the AI does, the preview it draws, the projects it works in
    let watching = l.gone.is_some() && l.work.wants_draw();
    t < (DOTS_FROM + DOTS_SPREAD).max(s.done + ENTER).max(caret_still).max(s.button_at() + ENTER) || l.moving || going || watching
}

/// Which blink a caret is in (-1 when none is blinking): the window draws again when it
/// changes.
pub fn blink(t: f64) -> i64 {
    let l = live();
    match l.gone {
        None => blink_at(t),
        Some(_) => l.work.blink(t),
    }
}

/// The opening's caret.
fn blink_at(t: f64) -> i64 {
    let start = scene().map_or(0.0, |s| s.done) + BLINK;
    if t < start { -1 } else { ((t - start) / BLINK).floor() as i64 }
}

/// The grid of dots at `t`, slid by `shift` (points) and `fade`d (1: all there): each comes in
/// as the front from the middle passes it and is fainter the farther it is from the middle;
/// `even` of the way (0 to 1) to the projects' paper, where they are the same all over the
/// page. Near the pointer, or where it has just been, a dot is darker and bigger.
fn dots(canvas: &mut Canvas, scale: f64, t: f64, live: &mut Live, shift: (f64, f64), fade: f64, even: f64) {
    let (w, h) = (canvas.width as f64, canvas.height as f64);
    if live.stamps.as_ref().is_none_or(|s| s.0 != scale) {
        let mut v = vec![];
        for size in 0..8 {
            let r = DOT_RADIUS * scale * (1.0 + DOT_GROW * size as f64 / 7.0);
            for phase in 0..16 {
                v.push(Dot::new(r, (phase % 4) as f64 / 4.0, (phase / 4) as f64 / 4.0));
            }
        }
        live.stamps = Some((scale, v));
    }
    let spacing = (DOT_SPACING * scale).round().max(4.0);
    let (cx, cy) = ((w / 2.0).floor(), (h / 2.0).floor());
    let front = (t - DOTS_FROM) / DOTS_SPREAD;
    let (nx, ny) = ((cx / spacing).ceil() as i64 + 1, (cy / spacing).ceil() as i64 + 1);
    let pointer = live.pointer;
    let Some((_, stamps)) = &live.stamps else { return };
    for j in -ny..=ny {
        for i in -nx..=nx {
            let (gx, gy) = (cx + i as f64 * spacing, cy + j as f64 * spacing);
            let d = ((gx - cx) / (0.62 * w)).hypot((gy - cy) / (0.66 * h));
            let opening = (1.0 - d).max(0.0).powf(1.4) * smooth((front * (1.0 + DOTS_EDGE) - d) / DOTS_EDGE);
            let rest = opening + (HOME_DOTS - opening) * even;
            // where the dot is now, and how much the pointer lights it (in points)
            let (x, y) = (gx + shift.0 * scale, gy + shift.1 * scale);
            let (px, py) = (x / scale, y / scale);
            let mut lit = pointer.map_or(0.0, |p| reach((p.0 - px).hypot(p.1 - py), DOT_REACH));
            for s in &live.trail {
                let f = (1.0 - (t - s.0) / TRAIL).max(0.0);
                lit = lit.max(reach((s.1 - px).hypot(s.2 - py), DOT_REACH) * f * f);
            }
            let alpha = (rest + (1.0 - rest) * lit * 0.9) * fade;
            if alpha <= 0.01 {
                continue;
            }
            // the stamp for its size and for where it falls within a pixel
            let (xi, yi) = (x.floor(), y.floor());
            let (mut fx, mut fy) = (((x - xi) * 4.0).round() as i64, ((y - yi) * 4.0).round() as i64);
            let (mut xi, mut yi) = (xi as i64, yi as i64);
            if fx == 4 {
                (xi, fx) = (xi + 1, 0);
            }
            if fy == 4 {
                (yi, fy) = (yi + 1, 0);
            }
            let size = (lit * 7.0).round() as usize;
            canvas.stamp(&stamps[size * 16 + (fy * 4 + fx) as usize], xi, yi, DOTS.mix(DOT_LIT, lit), alpha as f32);
        }
    }
}

/// The paper with its dots: drawn fresh while they come out, answer the pointer or fade, else
/// kept.
fn background(width: usize, height: usize, scale: f64, t: f64, live: &mut Live, shift: (f64, f64)) -> Canvas {
    // going in, the dots become the projects' paper; inside a project they fade
    let even = live.gone.map_or(0.0, |g| smooth((t - g) / GO_DOTS));
    let fade = if live.gone.is_some() { live.work.paper(t) } else { 1.0 };
    if fade <= 0.0 {
        return Canvas::new(width, height, PAPER);
    }
    let still = t >= DOTS_FROM + DOTS_SPREAD && live.at_rest() && fade >= 1.0 && (even == 0.0 || even >= 1.0);
    let look = even >= 1.0;
    if still
        && let Some((s, home, c)) = &live.kept
        && *s == scale
        && *home == look
        && c.width == width
        && c.height == height
    {
        return c.clone();
    }
    let mut c = Canvas::new(width, height, PAPER);
    dots(&mut c, scale, t, live, shift, fade, even);
    if still {
        live.kept = Some((scale, look, c.clone()));
    }
    c
}

/// A rectangle as a path.
fn rect(x: f64, y: f64, w: f64, h: f64) -> Path {
    let mut p = Path::default();
    p.move_to(x, y);
    p.line_to(x + w, y);
    p.line_to(x + w, y + h);
    p.line_to(x, y + h);
    p.close();
    p
}

/// The window's picture `t` seconds after it opened, `width` × `height` pixels at `scale`
/// pixels to the point.
pub fn paint(width: usize, height: usize, scale: f64, t: f64) -> Canvas {
    let Some(s) = scene() else { return Canvas::new(width, height, PAPER) };
    let (wp, hp) = (width as f64 / scale, height as f64 / scale);
    // the lines and the button as a block in the middle, each in the middle across (points)
    let (ascent, descent, units) = (s.sans.ascent, s.sans.descent, s.sans.units);
    let tall = |size: f64| (ascent - descent) * size / units;
    let block: f64 = s.lines.iter().map(|l| tall(l.1)).sum::<f64>() + LEADING * (s.lines.len() - 1) as f64 + BUTTON_GAP + crate::button::HEIGHT;
    let mut top = (hp - block) / 2.0;
    let mut baselines = vec![];
    let mut lefts = vec![];
    for &(wide, size) in &s.lines {
        baselines.push(top + ascent * size / units);
        lefts.push((wp - wide) / 2.0);
        top += tall(size) + LEADING;
    }
    let button_w = s.button.wide + 2.0 * crate::button::PAD;
    let rest = ((wp - button_w) / 2.0, top - LEADING + BUTTON_GAP, button_w, crate::button::HEIGHT);
    // the springs move on; each letter feels the pointer where it rests
    let mut live = live();
    let centres: Vec<(f64, f64, f64, bool)> = s
        .letters
        .iter()
        .map(|l| {
            let size = LINES[l.line].runs[l.run].size;
            (lefts[l.line] + l.x + l.adv / 2.0, baselines[l.line] - 0.36 * size, HOVER_REACH + HOVER_REACH_SIZE * size, t >= l.at)
        })
        .collect();
    let shown = (t - s.button_at()) / ENTER;
    live.scale = scale;
    live.step(t, &centres, (wp, hp), (shown > 0.4).then_some(rest));
    let gone = live.gone;
    let (tx, ty) = (live.tilt[0].x, live.tilt[1].x);
    let words = (-tx * SHIFT_WORDS, -ty * SHIFT_WORDS * 0.6);
    let mut canvas = background(width, height, scale, t, &mut live, (-tx * SHIFT_DOTS, -ty * SHIFT_DOTS * 0.6));
    // the workspace, under the name gliding to its place
    if gone.is_some() {
        let shift = live.work.name_shift();
        live.work.paint(&mut canvas, scale, t, (wp, hp), (NAME_AT.0 + shift + s.name_wide, NAME_AT.1));
    }
    // going in: how far (0 to 1) the name has glided
    let glide = gone.map_or(0.0, |g| settle((t - g - GO_NAME_AFTER) / GO_NAME));
    // the letters typed so far, each rising into place as it fades in; heavier and higher
    // near the pointer, kept centred where it rests
    let mut faded = 0;
    // (the app's name shows whole once gone in, typed or not: the AI may start working early)
    for (i, l) in s.letters.iter().enumerate().filter(|(_, l)| t >= l.at || (gone.is_some() && (l.line, l.run) == NAME)) {
        let run = &LINES[l.line].runs[l.run];
        let is_name = (l.line, l.run) == NAME;
        // going in, the other words fade and drift up one after another
        let away = match gone {
            Some(g) if !is_name => {
                faded += 1;
                smooth((t - g - GO_STAGGER * faded as f64) / GO_FADE)
            }
            _ => 0.0,
        };
        if away >= 1.0 {
            continue;
        }
        let near = live.hover.get(i).map_or(0.0, |h| h.x.max(0.0));
        let f = if near > 0.002 {
            let base = if run.face == Face::Serif { s.serif } else { s.sans };
            base.at(&[("wght", run.weight + HOVER_WEIGHT * near), ("opsz", run.size)])
        } else {
            s.fonts[l.line][l.run].clone()
        };
        // (gone in before it was typed: the name is there already)
        let p = if is_name && t < l.at && gone.is_some() { 1.0 } else { (t - l.at) / ENTER };
        let rise = (1.0 - settle(p)) * 0.24 * run.size - near * HOVER_LIFT * run.size - away * 8.0;
        let mut x0 = lefts[l.line] + l.x + (l.adv - f.advance(l.glyph) * run.size / f.units) / 2.0 + words.0;
        let mut y0 = baselines[l.line] + rise + words.1;
        let mut size = run.size;
        if is_name && glide > 0.0 {
            // the name glides to the top left, shrinking to its size there
            let first = s.letters.iter().find(|m| (m.line, m.run) == NAME).map_or(0.0, |m| m.x);
            let to = (NAME_AT.0 + live.work.name_shift() + (l.x - first) * NAME_SIZE / run.size, NAME_AT.1);
            x0 += (to.0 - x0) * glide;
            y0 += (to.1 - y0) * glide;
            size += (NAME_SIZE - size) * glide;
        }
        let k = size / f.units;
        let mut path = Path::default();
        f.outline(l.glyph, &mut path, &|u, v| ((x0 + u * k) * scale, (y0 - v * k) * scale));
        canvas.fill_faded(&path, run.ink, (smooth(p / 0.45) * (1.0 - away)) as f32);
    }
    // the caret: it follows where the typing is, catching up quickly; going in, it goes out
    let mut on = if t < s.done + BLINK { smooth((t - CARET_IN) / 0.25) } else if blink_at(t) % 2 == 1 { 1.0 } else { 0.0 };
    if let Some(g) = gone {
        on = if t < s.done + BLINK { on } else { 1.0 } * (1.0 - smooth((t - g) / 0.15));
    }
    if on > 0.0 {
        let place = |line: usize, x: f64| -> [f64; 3] {
            let size = s.lines[line].1;
            let gap = if x > 0.0 { 0.045 * size } else { 0.0 };
            [lefts[line] + x + gap, baselines[line] - 0.8 * size, 0.98 * size]
        };
        let mut now = place(s.caret[0].1, s.caret[0].2);
        let (mut aim, mut since, mut quick) = (now, s.caret[0].0, CARET_ALONG);
        let catch = |now: [f64; 3], aim: [f64; 3], dt: f64, quick: f64| -> [f64; 3] {
            let left = |dt: f64| if quick > 0.0 { (-dt.max(0.0) / quick).exp() } else { 0.0 };
            // going down, across waits until the drop is mostly done
            let kx = if quick == CARET_DOWN { if dt < CARET_WAIT { 1.0 } else { left(dt - CARET_WAIT) } } else { left(dt) };
            let k = left(dt);
            [aim[0] + (now[0] - aim[0]) * kx, aim[1] + (now[1] - aim[1]) * k, aim[2] + (now[2] - aim[2]) * k]
        };
        for &(at, line, x, q) in s.caret.iter().skip(1).filter(|c| c.0 <= t) {
            now = catch(now, aim, at - since, quick);
            (aim, since, quick) = (place(line, x), at, q);
        }
        now = catch(now, aim, t - since, quick);
        let biggest = s.lines.iter().map(|l| l.1).fold(0.0, f64::max);
        let thick = (0.028 * biggest).min(0.035 * now[2]).max(1.5);
        canvas.fill_faded(&rect((now[0] + words.0) * scale, (now[1] + words.1) * scale, thick * scale, now[2] * scale), INK, on as f32);
    }
    // the button: it rises in after the typing, and sinks away going in
    if shown > 0.0 {
        let leave = gone.map_or(0.0, |g| smooth((t - g) / GO_BUTTON));
        let shift = (words.0 * 0.6, words.1 * 0.6);
        live.button.draw(&mut canvas, scale, &s.button, rest, smooth(shown / 0.45), (1.0 - settle(shown)) * 14.0, leave, shift);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_letters_come_in_order_at_an_uneven_pace() {
        let Some(s) = scene() else { return };
        let times: Vec<f64> = s.letters.iter().map(|l| l.at).collect();
        assert!(times.windows(2).all(|w| w[1] > w[0]));
        assert!((times[0] - LINES[0].pause).abs() < 1e-9);
        let gaps: Vec<f64> = times.windows(2).take(4).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().any(|g| (g - gaps[0]).abs() > 0.01), "not all the same: {gaps:?}");
        assert_eq!(s.letters.len(), "hello".len() + "welcome to veymelo".len());
    }

    #[test]
    fn the_caret_goes_down_before_the_second_line_and_blinks_at_the_end() {
        let Some(s) = scene() else { return };
        let first_of_second = s.letters.iter().find(|l| l.line == 1).unwrap().at;
        let down = s.caret.iter().find(|c| c.1 == 1).unwrap().0;
        assert!(down < first_of_second);
        assert!(busy(0.5) && busy(s.done));
        assert_eq!(blink_at(s.done), -1);
        assert_ne!(blink_at(s.done + BLINK + 0.01), blink_at(s.done + 2.0 * BLINK + 0.01));
    }

    #[test]
    fn a_letter_answers_where_the_pointer_was_a_moment_ago() {
        let trail: VecDeque<(f64, f64, f64)> = [(1.0, 10.0, 0.0), (1.1, 20.0, 0.0), (1.2, 30.0, 0.0)].into();
        assert_eq!(pointer_then(&trail, (30.0, 0.0), 1.15), (20.0, 0.0));
        assert_eq!(pointer_then(&trail, (30.0, 0.0), 2.0), (30.0, 0.0));
        assert_eq!(pointer_then(&trail, (30.0, 0.0), 0.5), (10.0, 0.0));
        assert_eq!(pointer_then(&VecDeque::new(), (5.0, 5.0), 0.5), (5.0, 5.0));
    }

    #[test]
    fn every_moment_can_be_drawn() {
        // the opening, the blinking, and going in (a small picture, so it is quick); drawing
        // must never wait on itself
        for t in [0.0, 1.0, 3.0, 6.0, 9.0] {
            let c = paint(320, 200, 1.0, t);
            assert_eq!(c.pixels.len(), 320 * 200 * 4);
        }
    }

    /// How long a frame takes to draw at the size of a Retina window, with the pointer over it
    /// (run by hand: cargo test paint_time -- --ignored --nocapture).
    #[test]
    #[ignore]
    fn paint_time() {
        let start = std::time::Instant::now();
        let frames = 60;
        for i in 0..frames {
            let t = 0.5 + i as f64 * 0.06;
            pointer(Some((500.0 + i as f64 * 4.0, 380.0)), t);
            std::hint::black_box(paint(2560, 1600, 2.0, t));
        }
        println!("{:.1} ms a frame", start.elapsed().as_secs_f64() * 1000.0 / frames as f64);
    }

    /// Pictures of the opening, going in, the workspace, the pointer near the plates, naming a
    /// project and going into it, written as BMP files to the folder in VEYMELO_FRAMES (run by
    /// hand: VEYMELO_FRAMES=/tmp/f cargo test frames -- --ignored).
    #[test]
    #[ignore]
    fn frames() {
        let Some(dir) = std::env::var_os("VEYMELO_FRAMES") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h, scale) = (2560, 1600, 2.0);
        let save = |c: &Canvas, name: &str| {
            let mut bmp = Vec::with_capacity(54 + w * h * 3);
            bmp.extend_from_slice(b"BM");
            bmp.extend_from_slice(&((54 + w * h * 3) as u32).to_le_bytes());
            bmp.extend_from_slice(&[0, 0, 0, 0, 54, 0, 0, 0, 40, 0, 0, 0]);
            bmp.extend_from_slice(&(w as i32).to_le_bytes());
            bmp.extend_from_slice(&(-(h as i32)).to_le_bytes());
            bmp.extend_from_slice(&[1, 0, 24, 0]);
            bmp.extend_from_slice(&[0; 24]);
            for px in c.pixels.chunks(4) {
                bmp.extend_from_slice(&[px[2], px[1], px[0]]);
            }
            std::fs::write(dir.join(format!("{name}.bmp")), bmp).unwrap();
        };
        // draws every frame for `secs` from `t`, calling `each` before each one
        let run = |t: &mut f64, secs: f64, each: &mut dyn FnMut(f64)| -> Canvas {
            let n = ((secs * 60.0) as usize).max(1);
            let mut last = None;
            for _ in 0..n {
                each(*t);
                last = Some(paint(w, h, scale, *t));
                *t += 1.0 / 60.0;
            }
            last.unwrap()
        };
        let mut t = 0.0;
        save(&run(&mut t, 6.0, &mut |_| {}), "a-rest");
        let button = live().button.rest.unwrap();
        let (bx, by) = (button.0 + button.2 / 2.0, button.1 + button.3 / 2.0);
        press(true, (bx, by), t);
        press(false, (bx, by), t);
        save(&run(&mut t, 0.5, &mut |_| {}), "b-going");
        save(&run(&mut t, 2.0, &mut |_| {}), "c-workspace");
        let size = live().size;
        save(&run(&mut t, 0.8, &mut |t| {
            pointer(Some((size.0 / 2.0 + 60.0, size.1 * 0.36)), t);
        }), "d-near-plates");
        key(36, "\r", t);
        save(&run(&mut t, 0.6, &mut |_| {}), "e-naming");
        for c in "Lesson one".chars() {
            key(0, &c.to_string(), t);
            run(&mut t, 0.08, &mut |_| {});
        }
        save(&run(&mut t, 0.3, &mut |_| {}), "f-named");
        key(36, "\r", t);
        save(&run(&mut t, 0.25, &mut |_| {}), "g-going-in");
        save(&run(&mut t, 1.2, &mut |_| {}), "h-inside");
    }
}

#[cfg(test)]
mod watching {
    use super::*;

    /// A real project inside, as it shows once its timeline's pictures and sound are made
    /// (run by hand: VEYMELO_FRAMES=/tmp/f VEYMELO_PROJECT=/path/to.veymelo cargo test
    /// a_real_project_shows).
    #[test]
    fn a_real_project_shows() {
        let (Some(dir), Some(file)) = (std::env::var_os("VEYMELO_FRAMES"), std::env::var_os("VEYMELO_PROJECT")) else { return };
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::remove_dir_all(dir.join("real"));
        std::fs::create_dir_all(dir.join("real")).unwrap();
        crate::live::use_library(dir.join("real/projects.json"));
        let (w, h, scale) = (2560, 1600, 2.0);
        let save = |c: &Canvas, name: &str| {
            let png = veymelo_render::to_png(&veymelo_raster::Image {
                width: w,
                height: h,
                px: c.pixels.chunks(4).map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, 1.0]).collect(),
            });
            std::fs::write(dir.join(format!("{name}.png")), png).unwrap();
        };
        let frames = |t: &mut f64, secs: f64, wait: bool| -> Canvas {
            let mut c = paint(w, h, scale, *t);
            for _ in 0..(secs * 60.0) as usize {
                *t += 1.0 / 60.0;
                c = paint(w, h, scale, *t);
                if wait {
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }
            }
            c
        };
        let mut t = 0.0;
        frames(&mut t, 5.5, false);
        let b = live().button.rest.unwrap();
        press(true, (b.0 + 10.0, b.1 + 10.0), t);
        press(false, (b.0 + 10.0, b.1 + 10.0), t);
        frames(&mut t, 1.5, false);
        answer(crate::workspace::Ask::OpenProject, Some(std::path::PathBuf::from(file)), t);
        // another of its compositions, when asked (VEYMELO_COMP); only looked at, not saved
        if let Some(comp) = std::env::var_os("VEYMELO_COMP") {
            let (_, session) = crate::live::shown().unwrap();
            let params = veymelo_json::Value::object([("comp", veymelo_json::Value::from(comp.to_string_lossy().as_ref()))]);
            session.lock().unwrap().run("view.set", &params).unwrap();
        }
        // its pictures and sound made meanwhile
        frames(&mut t, 12.0, true);
        save(&frames(&mut t, 0.2, true), "real-inside");
        command(crate::workspace::Command::TogglePanel, t);
        save(&frames(&mut t, 0.8, true), "real-folded");
        // the pointer over the Show in Finder button
        let size = live().size;
        pointer(Some((size.0 - 28.0 - 95.0 - 10.0 - 15.0, 36.0)), t);
        save(&frames(&mut t, 0.5, true), "real-finder");
        // a file of the folder under the pointer: its layers light up on the timeline
        pointer(Some((120.0, 70.0 + 56.0 + 6.0 * 26.0 + 13.0)), t);
        save(&frames(&mut t, 0.5, true), "real-file-lit");
        // the files folded too
        command(crate::workspace::Command::ToggleFiles, t);
        save(&frames(&mut t, 0.8, true), "real-both-folded");
        command(crate::workspace::Command::ToggleFiles, t);
        command(crate::workspace::Command::TogglePanel, t);
        frames(&mut t, 0.8, true);
        // the bands further down
        pointer(Some((400.0, size.1 - 120.0)), t);
        scroll((0.0, -320.0), (400.0, size.1 - 120.0), t);
        save(&frames(&mut t, 0.3, true), "real-scrolled");
        // the timeline dragged taller by its top edge
        let edge = size.1 - (size.1 * 0.44).clamp(220.0, 420.0) - 4.0 - 6.0;
        pointer(Some((400.0, edge)), t);
        press(true, (400.0, edge), t);
        pointer(Some((400.0, 230.0)), t);
        press(false, (400.0, 230.0), t);
        scroll((0.0, 2000.0), (400.0, 500.0), t);
        save(&frames(&mut t, 0.3, true), "real-tall");
    }

    /// The projects page as it is first met (no projects, no AI: the card that connects one
    /// lies under the plates), then with projects: the card opened from the status, its
    /// Claude Desktop tab, and the plate in dashes under the pointer (run by hand:
    /// VEYMELO_FRAMES=/tmp/f cargo test home_shows).
    #[test]
    fn home_shows() {
        let Some(dir) = std::env::var_os("VEYMELO_FRAMES") else { return };
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::remove_dir_all(dir.join("home"));
        std::fs::create_dir_all(dir.join("home")).unwrap();
        crate::live::use_library(dir.join("home/projects.json"));
        let (w, h, scale) = (2560, 1600, 2.0);
        let save = |c: &Canvas, name: &str| {
            let png = veymelo_render::to_png(&veymelo_raster::Image {
                width: w,
                height: h,
                px: c.pixels.chunks(4).map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, 1.0]).collect(),
            });
            std::fs::write(dir.join(format!("{name}.png")), png).unwrap();
        };
        let frames = |t: &mut f64, secs: f64| -> Canvas {
            let mut c = paint(w, h, scale, *t);
            for _ in 0..(secs * 60.0) as usize {
                *t += 1.0 / 60.0;
                c = paint(w, h, scale, *t);
            }
            c
        };
        let mut t = 0.0;
        frames(&mut t, 5.5);
        let b = live().button.rest.unwrap();
        press(true, (b.0 + 10.0, b.1 + 10.0), t);
        press(false, (b.0 + 10.0, b.1 + 10.0), t);
        save(&frames(&mut t, 3.0), "home-empty");
        // two projects, made as the AI makes them
        let mut agent = veymelo_mcp::Agent::new(crate::live::library());
        let folder = dir.join("home").display().to_string();
        for name in ["Logo sting", "Teaser"] {
            let _ = agent.call("project_new", &veymelo_json::Value::parse(&format!(r#"{{"name": "{name}", "folder": "{folder}"}}"#)).unwrap());
        }
        pointer(None, t);
        save(&frames(&mut t, 2.0), "home-projects");
        // the status pressed: the card under it, then its second tab
        let size = live().size;
        let at = (size.0 - 60.0, 34.0);
        pointer(Some(at), t);
        press(true, at, t);
        press(false, at, t);
        save(&frames(&mut t, 0.6), "home-connect");
        let tab = (size.0 - 20.0 - 400.0 + 18.0 + 120.0, 34.0 + 22.0 + 7.0 + 76.0);
        pointer(Some(tab), t);
        press(true, tab, t);
        press(false, tab, t);
        save(&frames(&mut t, 0.3), "home-connect-desktop");
        // away from it: it closes; the plate in dashes under the pointer
        press(true, (60.0, size.1 - 40.0), t);
        press(false, (60.0, size.1 - 40.0), t);
        assert!(!live().work.connect_open());
        let ghost = (28.0 + 220.0 * 2.5 + 20.0 * 2.0, 140.0 + 90.0);
        pointer(Some(ghost), t);
        save(&frames(&mut t, 0.8), "home-ghost");
    }

    /// Two projects an agent made through the library, one it works in: the page shows both,
    /// that one with "AI is working"; going into it shows its frame and its steps; back out.
    #[test]
    fn projects_and_what_the_ai_does_show() {
        let Some(dir) = std::env::var_os("VEYMELO_FRAMES") else { return };
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::remove_dir_all(dir.join("work"));
        std::fs::create_dir_all(&dir).unwrap();
        crate::live::use_library(dir.join("work/projects.json"));
        let mut agent = veymelo_mcp::Agent::new(crate::live::library());
        let mut call = |name: &str, args: &str| {
            let _ = agent.call(name, &veymelo_json::Value::parse(args).unwrap());
        };
        let folder = dir.join("work").display().to_string();
        call("project_new", &format!(r#"{{"name": "Logo sting", "folder": "{folder}"}}"#));
        call("project_new", &format!(r#"{{"name": "Teaser", "folder": "{folder}"}}"#));
        call("command_run", r##"{"command": "comp.new", "params": {"name": "Teaser", "width": 1080, "height": 1920, "fps": 30, "duration": 5, "background": "#1d2b3a"}}"##);
        call("command_run", r##"{"command": "layer.add", "params": {"type": "solid", "color": "#f2b134", "width": 600, "height": 600, "name": "Sun"}}"##);
        call("command_run", r##"{"command": "layer.add", "params": {"type": "text", "text": "hello", "name": "Title"}}"##);
        call("command_run", r##"{"command": "layer.add", "params": {"type": "text", "text": "a caption", "name": "Caption"}}"##);
        call("command_run", r##"{"command": "layer.add", "params": {"type": "shape", "name": "Ring"}}"##);
        call("command_run", r##"{"command": "layer.add", "params": {"type": "null", "name": "Rig"}}"##);
        call("command_run", r##"{"command": "layer.add", "params": {"type": "camera", "name": "Camera"}}"##);
        call("command_run", r##"{"command": "layer.set", "params": {"layer": "Caption", "in": 1.5, "out": 4}}"##);
        call("render_preview", r#"{"time": 1}"#);
        call("command_run", r##"{"command": "layer.remove", "params": {"layer": "Moon"}}"##);
        let (w, h, scale) = (2560, 1600, 2.0);
        let save = |c: &Canvas, name: &str| {
            let png = veymelo_render::to_png(&veymelo_raster::Image {
                width: w,
                height: h,
                px: c.pixels.chunks(4).map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, 1.0]).collect(),
            });
            std::fs::write(dir.join(format!("{name}.png")), png).unwrap();
        };
        let frames = |t: &mut f64, secs: f64, wait: bool| -> Canvas {
            let mut c = paint(w, h, scale, *t);
            for _ in 0..(secs * 60.0) as usize {
                *t += 1.0 / 60.0;
                c = paint(w, h, scale, *t);
                if wait {
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }
            }
            c
        };
        // a render going on in "Teaser", as one made from the command line tells it
        let renders = dir.join("work/renders");
        std::fs::create_dir_all(&renders).unwrap();
        let teaser_file = crate::live::library().find("Teaser").unwrap();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        let told = veymelo_json::Value::object([
            ("project", veymelo_json::Value::from(teaser_file.display().to_string())),
            ("comp", "Teaser".into()),
            ("out", "renders/teaser.mp4".into()),
            ("done", veymelo_json::Value::Number(84.0)),
            ("total", veymelo_json::Value::Number(150.0)),
            ("started", veymelo_json::Value::Number(now)),
            ("updated", veymelo_json::Value::Number(now + 60.0)),
        ]);
        std::fs::write(renders.join("test.json"), told.to_json()).unwrap();
        // and one made from the command line, of no project on the page
        let other = veymelo_json::Value::object([
            ("project", veymelo_json::Value::Null),
            ("comp", "Intro".into()),
            ("out", "renders/intro.mp4".into()),
            ("done", veymelo_json::Value::Number(30.0)),
            ("total", veymelo_json::Value::Number(120.0)),
            ("started", veymelo_json::Value::Number(now)),
            ("updated", veymelo_json::Value::Number(now + 60.0)),
        ]);
        std::fs::write(renders.join("test2.json"), other.to_json()).unwrap();
        unsafe { std::env::set_var("VEYMELO_RENDERS", &renders) };
        // past the opening, onto the page of projects
        let mut t = 0.0;
        frames(&mut t, 5.5, false);
        let b = live().button.rest.unwrap();
        press(true, (b.0 + 10.0, b.1 + 10.0), t);
        press(false, (b.0 + 10.0, b.1 + 10.0), t);
        save(&frames(&mut t, 3.0, true), "projects");
        // the pointer over the second plate: it lifts and shows its bin
        pointer(Some((400.0, 200.0)), t);
        save(&frames(&mut t, 1.0, false), "projects-bin");
        // into the one the agent works in
        let teaser = crate::live::library().find("Teaser").unwrap();
        answer(crate::workspace::Ask::OpenProject, Some(teaser), t);
        assert_eq!(live().work.mode, crate::workspace::Mode::Inside);
        save(&frames(&mut t, 2.0, true), "inside");
        // the Render button: rendering, then done
        let out = dir.join("work/renders-out/Teaser.mp4");
        answer(crate::workspace::Ask::RenderTo, Some(out.clone()), t);
        save(&frames(&mut t, 0.12, true), "inside-rendering");
        for _ in 0..600 {
            if !crate::export::going() {
                break;
            }
            frames(&mut t, 0.05, true);
        }
        assert!(out.exists(), "the render made its file");
        save(&frames(&mut t, 0.5, true), "inside-rendered");
        // the composition's settings: the card open, then 1080 × 1080 and 30 s typed and applied
        command(crate::workspace::Command::Settings, t);
        save(&frames(&mut t, 0.5, true), "settings");
        for (code, text) in [(48, ""), (0, "1080"), (48, ""), (0, "1080"), (48, ""), (48, ""), (0, "30")] {
            key(code, text, t);
        }
        save(&frames(&mut t, 0.3, true), "settings-typed");
        key(36, "\r", t);
        frames(&mut t, 0.5, true);
        {
            let (_, session) = crate::live::shown().unwrap();
            let s = session.lock().unwrap();
            let c = s.project().comp(s.view.comp.unwrap()).unwrap();
            assert_eq!((c.width, c.height, c.duration.seconds()), (1080, 1080, 30.0), "the settings were made");
        }
        assert!(veymelo_mcp::activity::steps().iter().any(|s| s.person && s.title == "comp.crop"), "as a step of the person's");
        save(&frames(&mut t, 1.0, true), "settings-applied");
        // the AI's steps folded away, and back
        command(crate::workspace::Command::TogglePanel, t);
        save(&frames(&mut t, 0.8, false), "inside-folded");
        command(crate::workspace::Command::TogglePanel, t);
        frames(&mut t, 0.8, false);
        // back out
        key(53, "", t);
        assert_eq!(live().work.mode, crate::workspace::Mode::Home);
        save(&frames(&mut t, 1.0, false), "back");
    }
}
