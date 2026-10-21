//! The timeline under the preview, to look at (it changes nothing): the play button and the
//! time, a ruler of seconds with the composition's markers, then the layers by kind, each kind
//! one band (Sound, Text, Footage, Graphics, Camera & Lights, Helpers; those with none left
//! out). In a band the layers lie side by side as blocks from where each comes in to where it
//! goes, stacked only where they overlap (every row drawn whole; scroll for more), and each
//! shows what suits its kind:
//!
//! - Sound: its waveform.
//! - Text: its words, as they show.
//! - Footage, and compositions: pictures of it along its time.
//! - Graphics: a small picture of it, and its name.
//! - Camera & Lights: a line with a diamond at each keyframe and, between them, how it moves
//!   ("zoom in", "move", "turn").
//! - Helpers: a thin bar (the band starts folded).
//!
//! The waveforms and pictures are made on a thread of their own when the project has rested a
//! moment, each layer's again only when it has changed. A band's heading folds and opens it.
//! Blocks the AI has just worked on get the pen's dot (the newest brightest); the bands follow
//! it as it works, unless the person has just scrolled. The playhead runs down across them all.
//!
//! Drag on the ruler to look at a time; press a block to go to where it comes in. Scroll for
//! more bands (sideways: along the time); pinch, or = and -, to zoom the time, 0 for all of it.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use veymelo_engine::project::{FootageKind, ItemKind, Layer, Node, Project, Source};
use veymelo_font::Font;
use veymelo_path::Path;
use veymelo_raster::{Canvas, Rgb};
use veymelo_time::Tick;

use crate::button::{self, Label};
use crate::motion::Spring;

const INK: Rgb = Rgb(10, 10, 10);
const GREY: Rgb = Rgb(118, 118, 122);
const FAINT: Rgb = Rgb(168, 168, 174);
const RULE: Rgb = Rgb(232, 232, 236);
const PAPER: Rgb = Rgb(255, 255, 255);
/// What the AI has just worked on: the pen's dot.
const PEN: Rgb = crate::workspace::PEN;

/// The names' column, the header, the ruler, the room between a band's rows and round a band
/// (points).
const NAMES: f64 = 150.0;
const HEAD: f64 = 40.0;
const RULER: f64 = 22.0;
const GAP: f64 = 3.0;
const PAD: f64 = 7.0;
/// A folded band's height.
const FOLDED: f64 = 18.0;
/// The peaks of a sound kept a second, and the rate it is read at for them.
const PEAKS: f64 = 50.0;
const PEAK_RATE: u32 = 8000;
/// How tall a layer's pictures are made (pixels), and how many at most along a layer.
const PICTURE: f64 = 56.0;
const MOST_PICTURES: usize = 8;
/// Seconds after the person scrolls before the bands follow the AI again.
const HANDS_OFF: f64 = 4.0;

/// What kind of layer it is, which is the band it falls in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Sound,
    Text,
    Footage,
    Graphics,
    Camera,
    Helpers,
}

impl Kind {
    /// The bands, in the order they are listed.
    pub const ALL: [Kind; 6] = [Kind::Sound, Kind::Text, Kind::Footage, Kind::Graphics, Kind::Camera, Kind::Helpers];

    fn index(self) -> usize {
        Kind::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Sound => "Sound",
            Kind::Text => "Text",
            Kind::Footage => "Footage",
            Kind::Graphics => "Graphics",
            Kind::Camera => "Camera & Lights",
            Kind::Helpers => "Helpers",
        }
    }

    /// Its blocks' fill, their edge (also its mark), and the ink of words on them.
    fn colours(self) -> (Rgb, Rgb, Rgb) {
        match self {
            Kind::Sound => (Rgb(222, 242, 233), Rgb(29, 158, 117), Rgb(8, 80, 65)),
            Kind::Text => (Rgb(226, 237, 250), Rgb(55, 138, 221), Rgb(12, 68, 124)),
            Kind::Footage => (Rgb(236, 235, 253), Rgb(127, 119, 221), Rgb(60, 52, 137)),
            Kind::Graphics => (Rgb(250, 234, 226), Rgb(216, 90, 48), Rgb(113, 43, 19)),
            Kind::Camera => (Rgb(238, 237, 254), Rgb(83, 74, 183), Rgb(60, 52, 137)),
            Kind::Helpers => (Rgb(236, 236, 239), Rgb(176, 176, 184), Rgb(90, 90, 96)),
        }
    }

    /// Where its row `r` is in a band (from the band's top) and how tall it is (points).
    fn row_place(self, r: usize) -> (f64, f64) {
        (PAD + r as f64 * (self.row() + GAP), self.row())
    }

    /// How tall a band of `rows` rows is (points).
    fn band_tall(self, rows: usize) -> f64 {
        match rows.checked_sub(1) {
            None => FOLDED,
            Some(last) => {
                let (at, tall) = self.row_place(last);
                (at + tall + PAD).max(FOLDED)
            }
        }
    }

    /// How tall a row of its blocks is (points).
    fn row(self) -> f64 {
        match self {
            Kind::Sound => 30.0,
            Kind::Text => 20.0,
            Kind::Footage => 30.0,
            Kind::Graphics => 22.0,
            Kind::Camera => 34.0,
            Kind::Helpers => 10.0,
        }
    }
}

/// A stretch where a camera or light (or a helper) moves: from, to (seconds), and what it does.
#[derive(Clone, Debug, PartialEq)]
pub struct Move {
    pub from: f64,
    pub to: f64,
    pub what: String,
}

/// A layer as the timeline shows it: its id and name, kind, when it is there (seconds), its
/// words (a text layer's), its keyframes' times and moves, and how many pictures of it to make.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
    pub from: f64,
    pub to: f64,
    pub words: Option<String>,
    pub keys: Vec<f64>,
    pub moves: Vec<Move>,
    pub pictures: usize,
    /// A light (else, in the Camera & Lights band, a camera).
    pub light: bool,
}

/// The kind of layer `l` is in `project`.
fn kind_of(project: &Project, l: &Layer) -> Kind {
    let tone = l.props.group("effects").is_ok_and(|g| g.children.iter().filter_map(Node::as_group).any(|e| e.def.id == "tone"));
    if tone {
        return Kind::Sound;
    }
    if l.switches.adjustment || l.switches.guide {
        return Kind::Helpers;
    }
    match &l.source {
        Source::Text => Kind::Text,
        Source::Null => Kind::Helpers,
        Source::Camera | Source::Light(_) => Kind::Camera,
        Source::Footage(id) => match project.item(*id).map(|i| &i.kind) {
            Some(ItemKind::Footage(f)) if f.kind == FootageKind::Audio => Kind::Sound,
            _ => Kind::Footage,
        },
        _ => Kind::Graphics,
    }
}

/// A text layer's words, its first line as it first shows.
fn words_of(l: &Layer) -> Option<String> {
    let p = l.props.prop("text/sourceText").ok()?;
    let doc = p.keys.first().map_or(&p.value, |k| &k.value).doc()?;
    let line = doc.text.lines().map(str::trim).find(|s| !s.is_empty())?.to_string();
    Some(line)
}

/// The properties whose keys tell how a camera, light or helper moves, and the word for each.
const MOVING: [(&str, &str); 9] = [
    ("cameraOptions/zoom", "zoom"),
    ("transform/position", "move"),
    ("transform/pointOfInterest", "aim"),
    ("transform/orientation", "turn"),
    ("transform/rotationX", "turn"),
    ("transform/rotationY", "turn"),
    ("transform/rotationZ", "turn"),
    ("cameraOptions/focusDistance", "focus"),
    ("lightOptions/intensity", "light"),
];

/// A layer's keyframes' times (composition seconds) and the moves between them.
fn moves_of(l: &Layer) -> (Vec<f64>, Vec<Move>) {
    let mut keys: Vec<f64> = Vec::new();
    let mut moves: Vec<Move> = Vec::new();
    for (path, word) in MOVING {
        let Ok(p) = l.props.prop(path) else { continue };
        for k in &p.keys {
            let t = l.comp_time(k.time).seconds();
            if !keys.iter().any(|x| (x - t).abs() < 1e-6) {
                keys.push(t);
            }
        }
        for w in p.keys.windows(2) {
            if w[0].value == w[1].value {
                continue;
            }
            let (a, b) = (l.comp_time(w[0].time).seconds(), l.comp_time(w[1].time).seconds());
            let what = if word == "zoom" { if w[1].value.as_f64() > w[0].value.as_f64() { "zoom in" } else { "zoom out" } } else { word };
            match moves.iter_mut().find(|m| (m.from - a).abs() < 1e-6 && (m.to - b).abs() < 1e-6) {
                Some(m) if !m.what.split(" + ").any(|x| x == what) => m.what = format!("{} + {what}", m.what),
                Some(_) => {}
                None => moves.push(Move { from: a, to: b, what: what.to_string() }),
            }
        }
    }
    keys.sort_by(f64::total_cmp);
    moves.sort_by(|a, b| a.from.total_cmp(&b.from));
    (keys, moves)
}

/// The layers of composition `comp`, top first, and its markers' times.
pub fn lanes(project: &Project, comp: u64) -> (Vec<Lane>, Vec<f64>) {
    let Some(c) = project.comp(comp) else { return (Vec::new(), Vec::new()) };
    let lanes = c
        .layers
        .iter()
        .map(|l| {
            let kind = kind_of(project, l);
            let (from, to) = (l.in_point.seconds(), l.out_point.seconds());
            let (keys, moves) = if matches!(kind, Kind::Camera | Kind::Helpers) { moves_of(l) } else { (Vec::new(), Vec::new()) };
            let pictures = match (&l.source, kind) {
                (_, Kind::Sound | Kind::Text | Kind::Camera | Kind::Helpers) => 0,
                // what changes along its time: several, as many as fit its length
                (Source::Comp(_), _) => ((to - from) / 3.0).ceil().clamp(1.0, MOST_PICTURES as f64) as usize,
                (Source::Footage(id), _) => match project.item(*id).map(|i| &i.kind) {
                    Some(ItemKind::Footage(f)) if matches!(f.kind, FootageKind::Video | FootageKind::Sequence) => ((to - from) / 3.0).ceil().clamp(1.0, MOST_PICTURES as f64) as usize,
                    _ => 1,
                },
                _ => 1,
            };
            Lane { id: l.id, name: l.name.clone(), kind, from, to, words: if kind == Kind::Text { words_of(l) } else { None }, keys, moves, pictures, light: matches!(l.source, Source::Light(_)) }
        })
        .collect();
    (lanes, c.markers.iter().map(|m| m.time.seconds()).collect())
}

/// The layers of one kind laid in rows: each into the first row where it does not overlap
/// what is there, the earliest first.
fn pack(lanes: &[Lane], members: &[usize]) -> Vec<Vec<usize>> {
    let mut order = members.to_vec();
    order.sort_by(|&a, &b| lanes[a].from.total_cmp(&lanes[b].from).then(a.cmp(&b)));
    let mut rows: Vec<(f64, Vec<usize>)> = Vec::new();
    for i in order {
        match rows.iter_mut().find(|(end, _)| *end <= lanes[i].from + 1e-9) {
            Some((end, row)) => {
                *end = lanes[i].to;
                row.push(i);
            }
            None => rows.push((lanes[i].to, vec![i])),
        }
    }
    rows.into_iter().map(|(_, r)| r).collect()
}

// ---- what the blocks show, made on a thread of their own -----------------------------------

/// A small picture of a layer at a time: straight RGB.
#[derive(Clone, Debug)]
pub struct Pic {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
    pub at: f64,
}

/// What a layer's block shows that has to be made: its sound's peaks (`PEAKS` a second from
/// where it comes in, 0 to 1), its pictures.
#[derive(Clone, Debug, Default)]
pub struct Art {
    pub peaks: Vec<f32>,
    pub pics: Vec<Pic>,
}

struct Artist {
    ask: Mutex<Option<(u64, u64, Arc<Project>)>>,
    wake: Condvar,
    /// Each layer's art, and what the layer was when it was made.
    done: Mutex<HashMap<u64, (u64, Arc<Art>)>>,
    count: AtomicU64,
}

fn artist() -> &'static Artist {
    static A: OnceLock<Artist> = OnceLock::new();
    A.get_or_init(|| {
        std::thread::spawn(art_loop);
        Artist { ask: Mutex::new(None), wake: Condvar::new(), done: Mutex::new(HashMap::new()), count: AtomicU64::new(0) }
    })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ask for what the blocks of `comp` of `project` show, as it is at `revision`.
pub fn want_art(revision: u64, comp: u64, project: Arc<Project>) {
    let a = artist();
    let mut ask = lock(&a.ask);
    if ask.as_ref().is_none_or(|x| (x.0, x.1) != (revision, comp)) {
        *ask = Some((revision, comp, project));
        a.wake.notify_one();
    }
}

/// What layer `id`'s block shows, once made.
pub fn art(id: u64) -> Option<Arc<Art>> {
    lock(&artist().done).get(&id).map(|d| d.1.clone())
}

/// A count that goes up whenever more is made.
pub fn art_count() -> u64 {
    artist().count.load(Ordering::Relaxed)
}

/// What a layer is, to tell when it changed: itself, and the composition it shows if it shows
/// one.
fn fingerprint(project: &Project, l: &Layer) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{l:?}").hash(&mut h);
    if let Source::Comp(id) | Source::Footage(id) = &l.source {
        if let Some(c) = project.comp(*id) {
            format!("{c:?}").hash(&mut h);
        }
        if let Some(i) = project.item(*id) {
            format!("{:?}", i.kind).hash(&mut h);
        }
    }
    h.finish()
}

fn art_loop() {
    let a = artist();
    let mut made: Option<(u64, u64)> = None;
    loop {
        // the newest ask, once the project has rested a moment
        let (revision, comp, project) = {
            let mut ask = lock(&a.ask);
            loop {
                if let Some(x) = ask.as_ref().filter(|x| Some((x.0, x.1)) != made) {
                    break x.clone();
                }
                ask = a.wake.wait(ask).unwrap_or_else(|e| e.into_inner());
            }
        };
        std::thread::sleep(std::time::Duration::from_millis(350));
        if lock(&a.ask).as_ref().is_some_and(|x| (x.0, x.1) != (revision, comp)) {
            continue;
        }
        made = Some((revision, comp));
        let Some(c) = project.comp(comp) else { continue };
        let (lanes, _) = lanes(&project, comp);
        // the sound first, then the pictures, each layer told as soon as it is made
        let mut order: Vec<&Lane> = lanes.iter().filter(|l| l.kind == Kind::Sound || l.pictures > 0).collect();
        order.sort_by_key(|l| if l.kind == Kind::Sound { 0 } else { 1 });
        for lane in order {
            // a newer ask: start again with it
            if lock(&a.ask).as_ref().is_some_and(|x| (x.0, x.1) != (revision, comp)) {
                made = None;
                break;
            }
            let Some(layer) = c.layer(lane.id) else { continue };
            let print = fingerprint(&project, layer);
            if lock(&a.done).get(&lane.id).is_some_and(|d| d.0 == print) {
                continue;
            }
            let art = if lane.kind == Kind::Sound { Art { peaks: layer_peaks(&project, comp, lane), pics: Vec::new() } } else { Art { peaks: Vec::new(), pics: pictures(&project, comp, layer, lane) } };
            lock(&a.done).insert(lane.id, (print, Arc::new(art)));
            a.count.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The loudest of each `1 / PEAKS` second of a layer's sound, from where it comes in (0 to 1).
fn layer_peaks(project: &Project, comp: u64, lane: &Lane) -> Vec<f32> {
    let per = (PEAK_RATE as f64 / PEAKS) as usize;
    let frames = ((lane.to - lane.from).max(0.0) * PEAK_RATE as f64) as usize;
    let mut out = Vec::with_capacity(frames / per + 1);
    let mut at = 0;
    while at < frames {
        let n = (per * 200).min(frames - at);
        let s = veymelo_engine::render::audio::layer_mix(project, comp, lane.id, Tick::from_seconds(lane.from + at as f64 / PEAK_RATE as f64), n, PEAK_RATE);
        for chunk in s.chunks(per * 2) {
            out.push(chunk.iter().fold(0.0f32, |m, v| m.max(v.abs())).min(1.0));
        }
        at += n;
    }
    out
}

/// Pictures of a layer: one in the middle of its time, or several along it.
fn pictures(project: &Project, comp: u64, layer: &Layer, lane: &Lane) -> Vec<Pic> {
    let n = lane.pictures.max(1);
    let span = (lane.to - lane.from).max(1e-3);
    // small: about as tall as a picture is shown, from what its source says it is
    let scale = veymelo_engine::render::source_extent(project, layer).map_or(0.12, |(w, h)| (PICTURE / h.max(1.0)).min(PICTURE * 2.0 / w.max(1.0)).min(1.0));
    (0..n)
        .filter_map(|k| {
            let at = lane.from + span * (k as f64 + 0.5) / n as f64;
            let buf = veymelo_engine::render::source_pixels_at(project, comp, layer.id, Tick::from_seconds(at), scale)?;
            let img = buf.img;
            if img.width == 0 || img.height == 0 {
                return None;
            }
            // over paper, at most twice as tall as it shows
            let (w, h) = (img.width, img.height);
            let shrink = (h as f64 / (PICTURE * 2.0)).max(1.0);
            let (ow, oh) = (((w as f64 / shrink).round() as usize).max(1), ((h as f64 / shrink).round() as usize).max(1));
            let mut rgb = Vec::with_capacity(ow * oh * 3);
            for y in 0..oh {
                for x in 0..ow {
                    let p = img.px[((y as f64 * shrink) as usize).min(h - 1) * w + ((x as f64 * shrink) as usize).min(w - 1)];
                    let k = 1.0 - p[3].clamp(0.0, 1.0);
                    rgb.extend([0, 1, 2].map(|c| ((p[c] + k).clamp(0.0, 1.0) * 255.0).round() as u8));
                }
            }
            Some(Pic { w: ow, h: oh, rgb, at })
        })
        .collect()
}

// ---- where things are ----------------------------------------------------------------------

/// Where the parts are in the timeline's room (points).
pub struct Parts {
    pub play: (f64, f64),
    /// The time's track (x), the ruler's top and bottom, and where the bands are shown (y).
    pub track: (f64, f64),
    pub ruler: (f64, f64),
    pub bands: (f64, f64),
    pub room: (f64, f64, f64, f64),
}

pub fn parts(room: (f64, f64, f64, f64)) -> Parts {
    let (x, y, w, h) = room;
    let ruler = (y + HEAD, y + HEAD + RULER);
    Parts { play: (x + 22.0, y + HEAD / 2.0), track: (x + NAMES + 8.0, x + w - 12.0), ruler, bands: (ruler.1 + 4.0, y + h), room }
}

/// A band laid out: its kind, how many layers, its rows (when open), where it starts (from the
/// top of all the bands) and how tall it is.
struct Band {
    kind: Kind,
    count: usize,
    rows: Vec<Vec<usize>>,
    top: f64,
    tall: f64,
}

/// What is under a point: a band's heading, or a layer's block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Head(Kind),
    Block(usize),
}

/// What the timeline shows: the stretch of time (seconds), how far down the bands are scrolled
/// (points), which bands are folded (and their chevrons turning), when the person last
/// scrolled, and the AI step last followed.
#[derive(Clone, Debug)]
pub struct Timeline {
    pub from: f64,
    pub to: f64,
    pub down: f64,
    folded: [bool; 6],
    turn: [Spring; 6],
    touched: f64,
    followed: u64,
    /// Whether all of the composition's time is shown (it stays so when the length changes).
    all: bool,
}

impl Timeline {
    pub const fn new() -> Timeline {
        let mut turn = [Spring::new(); 6];
        // Helpers start folded (its chevron points along)
        turn[5] = Spring { x: 1.0, v: 0.0 };
        Timeline { from: 0.0, to: 0.0, down: 0.0, folded: [false, false, false, false, false, true], turn, touched: f64::NEG_INFINITY, followed: 0, all: true }
    }

    /// The bands for `lanes`, top to bottom.
    fn bands(&self, lanes: &[Lane]) -> Vec<Band> {
        let mut out = Vec::new();
        let mut top = 0.0;
        for k in Kind::ALL {
            let members: Vec<usize> = (0..lanes.len()).filter(|&i| lanes[i].kind == k).collect();
            if members.is_empty() {
                continue;
            }
            let folded = self.folded[k.index()];
            let rows = if folded { Vec::new() } else { pack(lanes, &members) };
            let tall = if folded { FOLDED } else { k.band_tall(rows.len()) };
            out.push(Band { kind: k, count: members.len(), rows, top, tall });
            top += tall + 1.0;
        }
        out
    }

    /// How tall all the bands are (points).
    pub fn height(&self, lanes: &[Lane]) -> f64 {
        self.bands(lanes).last().map_or(0.0, |b| b.top + b.tall)
    }

    /// Moves the springs on `dt` seconds. Gives whether any still moves.
    pub fn step(&mut self, dt: f64) -> bool {
        let mut moving = false;
        for (i, s) in self.turn.iter_mut().enumerate() {
            moving |= s.toward(if self.folded[i] { 1.0 } else { 0.0 }, dt, 300.0, 30.0);
        }
        moving
    }

    /// Keeps the stretch shown inside a composition `length` seconds long (all of it at first).
    pub fn fit(&mut self, length: f64) {
        if self.all || self.to <= self.from || self.to > length + 1e-9 && self.from <= 0.0 {
            self.from = 0.0;
            self.to = length.max(1e-3);
        }
        let span = (self.to - self.from).clamp(0.25_f64.min(length), length.max(1e-3));
        self.from = self.from.clamp(0.0, (length - span).max(0.0));
        self.to = self.from + span;
        self.all = self.from <= 1e-9 && self.to >= length - 1e-9;
    }

    /// The time at `x` on the track.
    pub fn time_at(&self, p: &Parts, x: f64) -> f64 {
        let k = ((x - p.track.0) / (p.track.1 - p.track.0).max(1.0)).clamp(0.0, 1.0);
        self.from + k * (self.to - self.from)
    }

    /// The x of time `s` on the track.
    pub fn x_of(&self, p: &Parts, s: f64) -> f64 {
        p.track.0 + (s - self.from) / (self.to - self.from).max(1e-9) * (p.track.1 - p.track.0)
    }

    /// Whether `pt` is on the ruler (where dragging looks at a time).
    pub fn on_time(&self, p: &Parts, pt: (f64, f64)) -> bool {
        pt.0 >= p.track.0 - 6.0 && pt.0 <= p.track.1 + 6.0 && pt.1 >= p.ruler.0 && pt.1 <= p.ruler.1 + 4.0
    }

    /// A layer's block in the bands: x, y, width, height (points), on a row at `y`, `tall` high.
    fn block(&self, p: &Parts, lane: &Lane, y: f64, tall: f64) -> (f64, f64, f64, f64) {
        let (a, b) = (self.x_of(p, lane.from.max(self.from)), self.x_of(p, lane.to.min(self.to)));
        (a, y, (b - a).max(4.0), tall)
    }

    /// What is under `pt`, if anything.
    pub fn hit(&self, p: &Parts, pt: (f64, f64), lanes: &[Lane]) -> Option<Hit> {
        let (x, _, w, _) = p.room;
        if pt.0 < x || pt.0 > x + w || pt.1 < p.bands.0 || pt.1 > p.bands.1 {
            return None;
        }
        let top = p.bands.0 - self.down;
        for band in self.bands(lanes) {
            let (y0, y1) = (top + band.top, top + band.top + band.tall);
            if pt.1 < y0 || pt.1 >= y1 {
                continue;
            }
            if pt.0 < p.track.0 - 4.0 {
                return Some(Hit::Head(band.kind));
            }
            for (r, row) in band.rows.iter().enumerate() {
                let (at, tall) = band.kind.row_place(r);
                for &i in row {
                    let (bx, by, bw, bh) = self.block(p, &lanes[i], y0 + at, tall);
                    if lanes[i].to > self.from && lanes[i].from < self.to && button::inside((bx, by - GAP / 2.0, bw, bh + GAP), pt) {
                        return Some(Hit::Block(i));
                    }
                }
            }
            return None;
        }
        None
    }

    /// Folds or opens band `k`.
    pub fn toggle(&mut self, k: Kind) {
        let i = k.index();
        self.folded[i] = !self.folded[i];
    }

    /// The pointer scrolled by `d` (points; down and right positive) at `t`: the bands up and
    /// down (`room` of them shown, `all` there), the time along when zoomed in.
    pub fn scroll(&mut self, d: (f64, f64), all: f64, room: f64, length: f64, track: f64, t: f64) {
        self.touched = t;
        self.down = (self.down - d.1).clamp(0.0, (all - room).max(0.0));
        if d.0 != 0.0 {
            let shift = -d.0 / track.max(1.0) * (self.to - self.from);
            self.from += shift;
            self.to += shift;
            self.fit(length);
        }
    }

    /// Zooms the time by `k` (above 1: in) round the time at `at`.
    pub fn zoom(&mut self, k: f64, at: f64, length: f64) {
        self.all = false;
        let span = ((self.to - self.from) / k).clamp(0.25_f64.min(length), length.max(1e-3));
        let w = if self.to > self.from { (at - self.from) / (self.to - self.from) } else { 0.5 };
        self.from = at - w * span;
        self.to = self.from + span;
        self.fit(length);
    }

    /// All the time again.
    pub fn whole(&mut self, length: f64) {
        self.all = true;
        self.from = 0.0;
        self.to = length.max(1e-3);
    }

    /// Follows the AI: the newest step that worked on layers here opens their band and brings
    /// it into view (unless the person has just scrolled). `steps` are (step number, layer
    /// names), oldest first; `room` is how tall the bands' view is.
    pub fn follow(&mut self, steps: &[(u64, Vec<String>)], lanes: &[Lane], room: f64, t: f64) {
        let Some((n, names)) = steps.last() else { return };
        if *n == self.followed {
            return;
        }
        self.followed = *n;
        if t - self.touched < HANDS_OFF || room <= 0.0 {
            return;
        }
        let Some(i) = lanes.iter().position(|l| names.contains(&l.name)) else { return };
        self.folded[lanes[i].kind.index()] = false;
        let all = self.height(lanes);
        if let Some(b) = self.bands(lanes).into_iter().find(|b| b.kind == lanes[i].kind)
            && (b.top < self.down || b.top + b.tall > self.down + room)
        {
            self.down = b.top.min((all - room).max(0.0)).max(0.0);
        }
    }
}

/// How fresh each layer the AI worked on is (1 the newest), from the steps that named them.
pub fn freshness(steps: &[(u64, Vec<String>)]) -> Vec<(String, f32)> {
    let mut out: Vec<(String, f32)> = Vec::new();
    for (k, (_, names)) in steps.iter().rev().take(10).enumerate() {
        let f = 1.0 - k as f32 / 10.0;
        for n in names {
            if !out.iter().any(|(m, _)| m == n) {
                out.push((n.clone(), f));
            }
        }
    }
    out
}

/// A step between labelled ticks, so they stand at least `apart` points from each other.
fn tick_step(span: f64, track: f64, apart: f64) -> f64 {
    let per = span / (track / apart).max(1.0);
    *[0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0].iter().find(|s| **s >= per).unwrap_or(&300.0)
}

/// Words in `font` at `size`, cut to fit `wide` points.
fn fit_label(font: &Font, size: f64, text: &str, wide: f64) -> Label {
    let full = Label::new(font.clone(), size, text);
    if full.wide <= wide {
        return full;
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().collect::<String>() + "…";
        if Label::new(font.clone(), size, &s).wide <= wide { lo = mid } else { hi = mid - 1 }
    }
    Label::new(font.clone(), size, &(chars[..lo].iter().collect::<String>() + "…"))
}

/// A picture drawn into the canvas pixels `(x, y, w, h)`, only inside columns `clip` (pixels).
fn put_pic(canvas: &mut Canvas, pic: &Pic, (x, y, w, h): (f64, f64, f64, f64), clip: (f64, f64), alpha: f32) {
    let (cw, ch) = (canvas.width as i64, canvas.height as i64);
    let (x0, x1) = ((x.max(clip.0)).floor() as i64, ((x + w).min(clip.1)).ceil() as i64);
    let (y0, y1) = (y.floor() as i64, (y + h).ceil() as i64);
    for py in y0.max(0)..y1.min(ch) {
        let sy = (((py as f64 + 0.5 - y) / h) * pic.h as f64).clamp(0.0, (pic.h - 1) as f64) as usize;
        for px in x0.max(0)..x1.min(cw) {
            let sx = (((px as f64 + 0.5 - x) / w) * pic.w as f64).clamp(0.0, (pic.w - 1) as f64) as usize;
            let s = (sy * pic.w + sx) * 3;
            let d = ((py * cw + px) as usize) * 4;
            for c in 0..3 {
                let (a, b) = (canvas.pixels[d + c] as f32, pic.rgb[s + c] as f32);
                canvas.pixels[d + c] = (a + (b - a) * alpha).round() as u8;
            }
        }
    }
}

/// What the timeline is drawn from.
pub struct Show<'a> {
    pub lanes: &'a [Lane],
    pub markers: &'a [f64],
    pub fresh: &'a [(String, f32)],
    /// Layers to light up (those using the file under the pointer); the others go faint.
    pub lit: &'a [u64],
    pub time: f64,
    pub length: f64,
    pub fps: f64,
    pub playing: bool,
    pub pointer: Option<(f64, f64)>,
    pub body: &'a Font,
    pub small: &'a Font,
    pub strong: &'a Font,
}

impl Timeline {
    /// Draws the timeline in `p.room` on `canvas` (`scale` pixels to the point), `alpha` faded in.
    pub fn paint(&self, canvas: &mut Canvas, scale: f64, p: &Parts, s: &Show, alpha: f32) {
        let px = |v: f64| v * scale;
        let rect = |x: f64, y: f64, w: f64, h: f64| Path::rounded(px(x), px(y), px(w), px(h), 0.0);
        let round = |x: f64, y: f64, w: f64, h: f64, r: f64| Path::rounded(px(x), px(y), px(w), px(h), px(r));
        let (x, y, w, h) = p.room;
        let (x0, x1) = p.track;
        let over = |r: (f64, f64, f64, f64)| s.pointer.is_some_and(|q| button::inside(r, q));
        // the play button: a round ink button, a triangle or two bars in white
        let (bx, by) = p.play;
        let near = over((bx - 16.0, by - 16.0, 32.0, 32.0));
        canvas.fill_faded(&round(bx - 13.0, by - 13.0, 26.0, 26.0, 13.0), if near { Rgb(48, 48, 54) } else { INK }, alpha);
        if s.playing {
            canvas.fill_faded(&round(bx - 4.5, by - 5.0, 3.0, 10.0, 1.0), PAPER, alpha);
            canvas.fill_faded(&round(bx + 1.5, by - 5.0, 3.0, 10.0, 1.0), PAPER, alpha);
        } else {
            let mut tri = Path::default();
            tri.move_to(px(bx - 3.0), px(by - 5.5));
            tri.line_to(px(bx + 5.5), px(by));
            tri.line_to(px(bx - 3.0), px(by + 5.5));
            tri.close();
            canvas.fill_faded(&tri, PAPER, alpha);
        }
        // the time: where, and of how long
        let now = Label::new(s.strong.clone(), 14.0, &crate::watch::timecode(s.time, s.fps));
        now.draw(canvas, bx + 24.0, by + 5.0, 1.0, scale, INK, alpha);
        let all = Label::new(s.body.clone(), 12.0, &format!("/  {}", crate::watch::timecode(s.length, s.fps)));
        all.draw(canvas, bx + 24.0 + now.wide + 8.0, by + 5.0, 1.0, scale, FAINT, alpha);
        // the ruler: ticks and seconds, and the composition's markers
        let span = (self.to - self.from).max(1e-9);
        let step = tick_step(span, x1 - x0, 70.0);
        let minor = step / 5.0;
        let head = self.x_of(p, s.time);
        let mut k = (self.from / minor).ceil() as i64;
        while (k as f64) * minor <= self.to + 1e-9 {
            let tt = k as f64 * minor;
            let tx = self.x_of(p, tt);
            let major = (tt / step - (tt / step).round()).abs() < 1e-6;
            let tall = if major { 7.0 } else { 3.5 };
            canvas.fill_faded(&rect(tx, p.ruler.1 - tall, 1.0, tall), if major { FAINT } else { RULE }, alpha);
            if major {
                let words = if step < 1.0 { format!("{tt:.1}s") } else { format!("{}s", tt.round()) };
                let l = Label::new(s.small.clone(), 10.0, &words);
                // faint under the playhead's knob, which covers it
                let under = head > tx - 8.0 && head < tx + 3.0 + l.wide + 6.0;
                l.draw(canvas, tx + 3.0, p.ruler.0 + 11.0, 1.0, scale, FAINT, if under { alpha * 0.2 } else { alpha });
            }
            k += 1;
        }
        canvas.fill_faded(&rect(x0, p.ruler.1, x1 - x0, 1.0), RULE, alpha);
        for &m in s.markers {
            if m >= self.from && m <= self.to {
                let mx = self.x_of(p, m);
                canvas.fill_faded(&round(mx - 3.0, p.ruler.1 - 9.0, 6.0, 6.0, 1.5), Rgb(240, 122, 61), alpha);
            }
        }
        // the bands, as far down as scrolled, within their room
        let (top_view, bottom_view) = p.bands;
        let top = top_view - self.down;
        let fresh_of = |name: &str| s.fresh.iter().find(|(n, _)| n == name).map(|f| f.1);
        let mut hovered: Option<usize> = None;
        for band in self.bands(s.lanes) {
            let (y0, y1) = (top + band.top, top + band.top + band.tall);
            if y1 < top_view || y0 > bottom_view {
                continue;
            }
            let (fill, mark, words_ink) = band.kind.colours();
            // its heading: a chevron turning (open: down, folded: along), its mark, name, count
            let head_y = y0.max(top_view) + 13.0;
            if head_y < bottom_view {
                let turn = self.turn[band.kind.index()].x.clamp(0.0, 1.0) * -std::f64::consts::FRAC_PI_2;
                let (cx, cy) = (x + 14.0, head_y - 4.0);
                let pt = |dx: f64, dy: f64| (px(cx + dx * turn.cos() - dy * turn.sin()), px(cy + dx * turn.sin() + dy * turn.cos()));
                canvas.stroke(&[pt(-3.5, -1.8), pt(0.0, 1.8), pt(3.5, -1.8)], false, 1.5 * scale, GREY, alpha);
                canvas.fill_faded(&round(x + 25.0, cy - 4.0, 8.0, 8.0, 4.0), mark, alpha);
                // (cameras alone, or lights alone, by that name)
                let said = match band.kind {
                    Kind::Camera if s.lanes.iter().filter(|l| l.kind == Kind::Camera).all(|l| !l.light) => "Camera",
                    Kind::Camera if s.lanes.iter().filter(|l| l.kind == Kind::Camera).all(|l| l.light) => "Lights",
                    k => k.name(),
                };
                let name = fit_label(s.strong, 12.0, said, NAMES - 66.0);
                name.draw(canvas, x + 38.0, head_y, 1.0, scale, INK, alpha);
                let count = Label::new(s.small.clone(), 11.0, &band.count.to_string());
                count.draw(canvas, x + 44.0 + name.wide, head_y, 1.0, scale, FAINT, alpha);
                let fresh = s.lanes.iter().filter(|l| l.kind == band.kind).filter_map(|l| fresh_of(&l.name)).fold(0.0f32, f32::max);
                if fresh > 0.0 {
                    canvas.fill_faded(&round(x + 51.0 + name.wide + count.wide, head_y - 7.0, 6.0, 6.0, 3.0), PEN, alpha * fresh);
                }
            }
            // folded: one thin bar from the first start to the last end
            if band.rows.is_empty() {
                let members: Vec<&Lane> = s.lanes.iter().filter(|l| l.kind == band.kind).collect();
                let (a0, b0) = members.iter().fold((f64::MAX, f64::MIN), |(a, b), l| (a.min(l.from), b.max(l.to)));
                let (a1, b1) = (a0.max(self.from), b0.min(self.to));
                if b1 > a1 && y0 + FOLDED / 2.0 > top_view {
                    let (l0, l1) = (self.x_of(p, a1), self.x_of(p, b1));
                    canvas.fill_faded(&round(l0, y0 + FOLDED / 2.0 - 2.5, (l1 - l0).max(5.0), 5.0, 2.5), fill.mix(mark, 0.25), alpha);
                }
            }
            for (r, row) in band.rows.iter().enumerate() {
                let (at, tall) = band.kind.row_place(r);
                let ry = y0 + at;
                if ry + tall < top_view || ry > bottom_view {
                    continue;
                }
                for &i in row {
                    let lane = &s.lanes[i];
                    if lane.to <= self.from || lane.from >= self.to {
                        continue;
                    }
                    let (bx, by, bw, bh) = self.block(p, lane, ry, tall);
                    if by < top_view - 2.0 || by + bh > bottom_view + 2.0 {
                        continue;
                    }
                    let hover = over((bx, by - 1.0, bw, bh + 2.0));
                    if hover {
                        hovered = Some(i);
                    }
                    let fresh = fresh_of(&lane.name);
                    let lit = s.lit.contains(&lane.id);
                    let a = if s.lit.is_empty() || lit { alpha } else { alpha * 0.35 };
                    self.paint_block(canvas, scale, p, s, lane, (bx, by, bw, bh), (fill, mark, words_ink), hover || lit, a);
                    if lit {
                        canvas.stroke(&Path::rounded_points(px(bx - 1.0), px(by - 1.0), px(bw + 2.0), px(bh + 2.0), px(5.0)), true, 1.4 * scale, INK, alpha);
                    }
                    if let Some(f) = fresh {
                        canvas.fill_faded(&round(bx + bw - 7.0, by + 2.0, 5.0, 5.0, 2.5), PEN, alpha * f.max(0.35));
                    }
                }
            }
            // a hairline under the band
            if y1 > top_view && y1 < bottom_view {
                canvas.fill_faded(&rect(x + 8.0, y1, w - 16.0, 1.0), RULE, alpha);
            }
        }
        // where the bands shown are among all
        let all = self.height(s.lanes);
        let room = bottom_view - top_view;
        if all > room && room > 0.0 {
            let (k0, k1) = (self.down / all, ((self.down + room) / all).min(1.0));
            canvas.fill_faded(&round(x + w - 5.0, top_view + room * k0, 3.0, (room * (k1 - k0)).max(10.0), 1.5), FAINT, alpha);
        }
        // the playhead, down across everything, with a head on the ruler
        if s.time >= self.from && s.time <= self.to {
            canvas.fill_faded(&rect(head - 0.75, p.ruler.0 + 6.0, 1.5, y + h - p.ruler.0 - 6.0), INK, alpha);
            canvas.fill_faded(&round(head - 5.0, p.ruler.0 + 1.0, 10.0, 10.0, 5.0), INK, alpha);
        }
        // the block under the pointer: its name and when it is there
        if let (Some(i), Some(q)) = (hovered, s.pointer) {
            let lane = &s.lanes[i];
            let quiet = lane.kind == Kind::Sound && art(lane.id).is_some_and(|a| a.peaks.iter().all(|v| *v < 1e-4));
            let words = format!("{}  ·  {:.1}–{:.1} s{}", lane.name, lane.from, lane.to, if quiet { "  ·  nothing heard here" } else { "" });
            let l = fit_label(s.small, 11.0, &words, 320.0);
            let (tw, th) = (l.wide + 18.0, 22.0);
            let tx = (q.0 + 12.0).min(x + w - tw - 4.0);
            let ty = (q.1 - th - 8.0).max(p.ruler.1 + 2.0);
            canvas.shadow(px(tx), px(ty + 2.0), px(tw), px(th), px(6.0), px(8.0), INK, alpha * 0.14);
            canvas.fill_faded(&round(tx, ty, tw, th, 6.0), PAPER, alpha);
            canvas.stroke(&Path::rounded_points(px(tx), px(ty), px(tw), px(th), px(6.0)), true, 0.8 * scale, RULE.mix(INK, 0.15), alpha);
            l.draw(canvas, tx + 9.0, ty + 15.0, 1.0, scale, INK, alpha);
        }
    }

    /// One layer's block, as its kind shows it.
    #[allow(clippy::too_many_arguments)]
    fn paint_block(&self, canvas: &mut Canvas, scale: f64, p: &Parts, s: &Show, lane: &Lane, (bx, by, bw, bh): (f64, f64, f64, f64), (fill, mark, ink): (Rgb, Rgb, Rgb), hover: bool, alpha: f32) {
        let px = |v: f64| v * scale;
        let round = |x: f64, y: f64, w: f64, h: f64, r: f64| Path::rounded(px(x), px(y), px(w), px(h), px(r));
        let art = art(lane.id);
        let edge = |canvas: &mut Canvas| {
            canvas.stroke(&Path::rounded_points(px(bx), px(by), px(bw), px(bh), px(4.0)), true, if hover { 1.4 * scale } else { 0.8 * scale }, mark, alpha * if hover { 1.0 } else { 0.55 });
        };
        let clip = (px(bx + 1.0), px(bx + bw - 1.0));
        match lane.kind {
            Kind::Sound => {
                canvas.fill_faded(&round(bx, by, bw, bh, 4.0), fill, alpha);
                // its waveform, mirrored about the middle, as loud as the block allows (a quiet
                // sound still shows its shape); none heard there: said so
                let silent = art.as_ref().is_some_and(|a| a.peaks.iter().all(|v| *v < 1e-4));
                if silent {
                    canvas.fill_faded(&round(bx + 3.0, by + bh / 2.0 + 2.5, (bw - 6.0).max(1.0), 1.0, 0.5), mark, alpha * 0.4);
                    if bw > 46.0 {
                        let l = Label::new(s.small.clone(), 9.5, "silent");
                        l.draw(canvas, bx + bw - l.wide - 5.0, by + 10.0, 1.0, scale, GREY, alpha);
                    }
                } else if let Some(a) = art.as_ref().filter(|a| !a.peaks.is_empty()) {
                    let mid = by + bh / 2.0 + 3.0;
                    let loudest = a.peaks.iter().fold(0.0f32, |m, v| m.max(*v)).max(0.02);
                    let half = (bh / 2.0 - 6.0) / loudest as f64;
                    let mut bars = Path::default();
                    let cols = (bw * scale) as usize;
                    for c in 0..cols {
                        let xa = bx + c as f64 / scale;
                        let (ta, tb) = (self.time_at(p, xa) - lane.from, self.time_at(p, xa + 1.0 / scale) - lane.from);
                        let (i0, i1) = ((ta * PEAKS).max(0.0) as usize, ((tb * PEAKS).ceil().max(0.0) as usize).max((ta * PEAKS).max(0.0) as usize + 1));
                        let v = a.peaks.get(i0.min(a.peaks.len())..i1.min(a.peaks.len())).map_or(0.0, |r| r.iter().fold(0.0f32, |m, v| m.max(*v)));
                        let tall = (v as f64 * half).max(0.3) * scale;
                        let (cx, cy) = (px(bx) + c as f64, px(mid));
                        bars.move_to(cx, cy - tall);
                        bars.line_to(cx + 1.0, cy - tall);
                        bars.line_to(cx + 1.0, cy + tall);
                        bars.line_to(cx, cy + tall);
                        bars.close();
                    }
                    canvas.fill_faded(&bars, mark, alpha * 0.85);
                }
                edge(canvas);
                if bw > 40.0 {
                    fit_label(s.small, 9.5, &lane.name, bw - 10.0).draw(canvas, bx + 5.0, by + 10.0, 1.0, scale, ink, alpha);
                }
            }
            Kind::Text => {
                canvas.fill_faded(&round(bx, by, bw, bh, 4.0), fill, alpha);
                edge(canvas);
                let said = lane.words.as_deref().map_or_else(|| lane.name.clone(), |w| format!("“{w}”"));
                if bw > 18.0 {
                    fit_label(s.small, 10.5, &said, bw - 10.0).draw(canvas, bx + 5.0, by + bh / 2.0 + 3.8, 1.0, scale, ink, alpha);
                }
            }
            Kind::Footage | Kind::Graphics => {
                canvas.fill_faded(&round(bx, by, bw, bh, 4.0), fill, alpha);
                let pics = art.as_ref().map(|a| a.pics.as_slice()).unwrap_or(&[]);
                let (ph, py) = (bh - 4.0, by + 2.0);
                let mut text_from = bx + 6.0;
                if lane.pictures > 1 || lane.kind == Kind::Footage {
                    // pictures along it: each where its time is, side by side, the nearest shown
                    if let Some(first) = pics.first() {
                        let pw = (ph * first.w as f64 / first.h.max(1) as f64).clamp(ph * 0.4, ph * 2.5);
                        let mut at = bx + 2.0;
                        while at < bx + bw - 2.0 {
                            let t = self.time_at(p, at + pw / 2.0);
                            let pic = pics.iter().min_by(|a, b| (a.at - t).abs().total_cmp(&(b.at - t).abs())).unwrap_or(first);
                            put_pic(canvas, pic, (px(at), px(py), px(pw), px(ph)), clip, alpha);
                            at += pw + 1.0;
                        }
                        // its name on a small paper label, where it starts
                        if bw > 50.0 {
                            let l = fit_label(s.small, 9.5, &lane.name, (bw - 16.0).min(160.0));
                            canvas.fill_faded(&round(bx + 4.0, by + bh - 15.0, l.wide + 10.0, 12.0, 3.0), PAPER, alpha * 0.92);
                            l.draw(canvas, bx + 9.0, by + bh - 6.0, 1.0, scale, ink, alpha);
                        }
                        text_from = f64::MAX;
                    }
                } else if let Some(pic) = pics.first() {
                    // a small picture of it at its start, then its name
                    let side = ph;
                    let (pw, phh) = if pic.w >= pic.h { (side, side * pic.h as f64 / pic.w.max(1) as f64) } else { (side * pic.w as f64 / pic.h.max(1) as f64, side) };
                    canvas.fill_faded(&round(bx + 2.0, py, side, side, 3.0), PAPER, alpha);
                    put_pic(canvas, pic, (px(bx + 2.0 + (side - pw) / 2.0), px(py + (side - phh) / 2.0), px(pw), px(phh)), clip, alpha);
                    text_from = bx + side + 7.0;
                }
                edge(canvas);
                if text_from < bx + bw - 14.0 {
                    fit_label(s.small, 10.5, &lane.name, bx + bw - 5.0 - text_from).draw(canvas, text_from, by + bh / 2.0 + 3.8, 1.0, scale, ink, alpha);
                }
            }
            Kind::Camera | Kind::Helpers => {
                let mid = if lane.kind == Kind::Camera { by + 12.0 } else { by + bh / 2.0 };
                if lane.kind == Kind::Helpers {
                    canvas.fill_faded(&round(bx, by + 1.0, bw, bh - 2.0, (bh - 2.0) / 2.0), fill.mix(mark, if hover { 0.5 } else { 0.25 }), alpha);
                } else {
                    // a line where it is, its name at its start
                    canvas.fill_faded(&round(bx, mid - 0.6, bw, 1.2, 0.6), mark, alpha * 0.6);
                    if bw > 40.0 {
                        fit_label(s.small, 9.5, &lane.name, bw.min(120.0)).draw(canvas, bx, mid - 5.0, 1.0, scale, ink, alpha * 0.8);
                    }
                }
                // how it moves: an arc over each move, what it does under it (where there is
                // room for the words, past the last ones)
                let mut said_to = f64::MIN;
                for m in &lane.moves {
                    if m.to <= self.from || m.from >= self.to || lane.kind == Kind::Helpers {
                        continue;
                    }
                    let (a, b) = (self.x_of(p, m.from), self.x_of(p, m.to));
                    let lift = ((b - a) * 0.18).clamp(3.0, 8.0);
                    let n = 16;
                    let pts: Vec<(f64, f64)> = (0..=n).map(|k| {
                        let u = k as f64 / n as f64;
                        (px(a + (b - a) * u), px(mid - lift * 4.0 * u * (1.0 - u)))
                    }).collect();
                    canvas.stroke(&pts, false, 1.2 * scale, mark, alpha);
                    let l = Label::new(s.small.clone(), 9.5, &m.what);
                    let lx = ((a + b) / 2.0 - l.wide / 2.0).max(bx);
                    if lx > said_to + 6.0 && lx + l.wide < p.track.1 + 4.0 {
                        l.draw(canvas, lx, mid + 13.0, 1.0, scale, ink, alpha);
                        said_to = lx + l.wide;
                    }
                }
                // a diamond at each keyframe
                let d = if lane.kind == Kind::Camera { 4.5 } else { 3.0 };
                for &k in &lane.keys {
                    if k < self.from || k > self.to {
                        continue;
                    }
                    let kx = self.x_of(p, k);
                    let mut dia = Path::default();
                    dia.move_to(px(kx), px(mid - d));
                    dia.line_to(px(kx + d), px(mid));
                    dia.line_to(px(kx), px(mid + d));
                    dia.line_to(px(kx - d), px(mid));
                    dia.close();
                    canvas.fill_faded(&dia, if lane.kind == Kind::Camera { mark } else { GREY }, alpha);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(name: &str, kind: Kind, from: f64, to: f64) -> Lane {
        Lane { id: 0, name: name.into(), kind, from, to, words: None, keys: Vec::new(), moves: Vec::new(), pictures: 0, light: false }
    }

    #[test]
    fn time_and_place_go_both_ways_and_zoom_stays_inside() {
        let p = parts((0.0, 0.0, 850.0, 240.0));
        let mut tl = Timeline::new();
        tl.fit(99.0);
        assert_eq!((tl.from, tl.to), (0.0, 99.0));
        let x = tl.x_of(&p, 33.0);
        assert!((tl.time_at(&p, x) - 33.0).abs() < 1e-9);
        tl.zoom(4.0, 90.0, 99.0);
        assert!(tl.to <= 99.0 + 1e-9 && (tl.to - tl.from - 24.75).abs() < 1e-9, "{} {}", tl.from, tl.to);
        tl.scroll((-10_000.0, 0.0), 0.0, 0.0, 99.0, 600.0, 0.0);
        assert!((tl.to - 99.0).abs() < 1e-9, "panned to the end and no further");
        tl.whole(99.0);
        assert_eq!((tl.from, tl.to), (0.0, 99.0));
        assert_eq!(tick_step(99.0, 700.0, 70.0), 10.0);
        // all of it shown: still all of it when the composition gets longer or shorter
        tl.fit(120.0);
        assert_eq!((tl.from, tl.to), (0.0, 120.0));
        tl.zoom(4.0, 10.0, 120.0);
        tl.fit(150.0);
        assert!(tl.to - tl.from < 100.0, "zoomed in: kept so");
    }

    #[test]
    fn layers_lie_side_by_side_and_stack_only_where_they_overlap() {
        let lanes = vec![lane("A", Kind::Text, 0.0, 2.0), lane("B", Kind::Text, 2.0, 4.0), lane("C", Kind::Text, 1.0, 3.0), lane("D", Kind::Text, 3.5, 5.0)];
        let rows = pack(&lanes, &[0, 1, 2, 3]);
        assert_eq!(rows, vec![vec![0, 1], vec![2, 3]], "A then B in one row; C over them, then D after C");
        // a crowd at one time: a row each, every one whole
        let crowd: Vec<Lane> = (0..20).map(|i| lane(&format!("L{i}"), Kind::Graphics, 0.0, 1.0)).collect();
        let rows = pack(&crowd, &(0..20).collect::<Vec<_>>());
        assert_eq!(rows.len(), 20);
    }

    #[test]
    fn the_bands_are_by_kind_and_fold() {
        let lanes = vec![lane("Cap 2", Kind::Text, 0.0, 1.0), lane("Rig", Kind::Helpers, 0.0, 1.0), lane("Voice", Kind::Sound, 0.0, 1.0), lane("Cap 1", Kind::Text, 0.5, 1.5), lane("Card", Kind::Graphics, 0.0, 1.0)];
        let mut tl = Timeline::new();
        let kinds: Vec<Kind> = tl.bands(&lanes).iter().map(|b| b.kind).collect();
        assert_eq!(kinds, [Kind::Sound, Kind::Text, Kind::Graphics, Kind::Helpers], "kinds with none are left out");
        let text = tl.bands(&lanes).into_iter().find(|b| b.kind == Kind::Text).unwrap();
        assert_eq!(text.rows.len(), 2, "the captions overlap: two rows");
        assert!(tl.bands(&lanes).last().unwrap().rows.is_empty(), "Helpers start folded");
        let before = tl.height(&lanes);
        tl.toggle(Kind::Text);
        assert!(tl.height(&lanes) < before);
    }

    #[test]
    fn the_bands_follow_the_ai_unless_the_person_scrolls() {
        let lanes: Vec<Lane> = (0..40).map(|i| lane(&format!("L{i}"), if i < 30 { Kind::Graphics } else { Kind::Text }, i as f64 * 0.1, 50.0)).collect();
        let mut tl = Timeline::new();
        let room = 120.0;
        tl.follow(&[(1, vec!["L35".into()])], &lanes, room, 10.0);
        let text = tl.bands(&lanes).into_iter().find(|b| b.kind == Kind::Text).unwrap();
        assert!(tl.down <= text.top && text.top < tl.down + room, "brought into view: {} {}", tl.down, text.top);
        tl.scroll((0.0, 200.0), tl.height(&lanes), room, 99.0, 600.0, 11.0);
        let mine = tl.down;
        tl.follow(&[(2, vec!["L3".into()])], &lanes, room, 12.0);
        assert_eq!(tl.down, mine, "the person's scroll is kept a moment");
        let fresh = freshness(&[(1, vec!["A".into()]), (2, vec!["B".into(), "A".into()])]);
        assert_eq!(fresh[0], ("B".to_string(), 1.0));
        assert_eq!(fresh.iter().find(|f| f.0 == "A").unwrap().1, 1.0);
    }
}

