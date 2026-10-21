//! The composition's settings, in a card under the pencil after its line at the top of a
//! project: its name, size, frame rate, length and background, each typed in, with shortcuts
//! under it that fill it in (lit while what is typed is theirs). Return (or Apply) makes the
//! change in the project, as a step of the person's beside the AI's; Escape (or Cancel, or a
//! click away) leaves it as it was. A new size keeps what is in the picture in its middle.
//! While the AI is in the middle of a step, the change waits for it to end.

use veymelo_font::Font;
use veymelo_json::Value as Json;
use veymelo_path::Path;
use veymelo_raster::{Canvas, Rgb};

use crate::button::{self, Label, Rect};
use crate::faces;
use crate::motion::{settle, smooth};
use crate::watch::About;

const INK: Rgb = Rgb(10, 10, 10);
const GREY: Rgb = Rgb(118, 118, 122);
const FAINT: Rgb = Rgb(168, 168, 174);
const EDGE: Rgb = Rgb(196, 196, 202);
const WELL: Rgb = Rgb(244, 244, 246);
const PAPER: Rgb = Rgb(255, 255, 255);
const RED: Rgb = Rgb(196, 43, 43);

/// The card's width, and where its fields start across (points).
const WIDE: f64 = 510.0;
const FIELDS: f64 = 116.0;
/// The most pixels a side, and seconds, a composition may be.
const MOST_SIDE: u32 = 30_000;
const MOST_SECONDS: f64 = 86_400.0;

/// The shortcuts: sizes, frame rates, lengths (none: to the end of its last layer), colours.
const SIZES: [(&str, u32, u32); 5] = [("9:16", 1080, 1920), ("1:1", 1080, 1080), ("4:5", 1080, 1350), ("16:9", 1920, 1080), ("4K", 3840, 2160)];
const RATES: [f64; 6] = [24.0, 25.0, 30.0, 50.0, 60.0, 120.0];
const LENGTHS: [(&str, Option<f64>); 6] = [("15 s", Some(15.0)), ("30 s", Some(30.0)), ("60 s", Some(60.0)), ("90 s", Some(90.0)), ("3 min", Some(180.0)), ("to the last layer", None)];
const COLOURS: [&str; 5] = ["#ffffff", "#f4f1e8", "#111111", "#1d2b3a", "#f0542c"];

/// The fields, in the order Tab goes through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    Name,
    Width,
    Height,
    Rate,
    Length,
    Colour,
}

const ORDER: [Which; 6] = [Which::Name, Which::Width, Which::Height, Which::Rate, Which::Length, Which::Colour];

/// A field being typed in: its words, where the caret is (in letters), and whether all of it is
/// chosen (what is typed then takes its place).
#[derive(Clone, Debug, Default)]
struct Field {
    text: String,
    caret: usize,
    all: bool,
}

impl Field {
    fn new(text: String) -> Field {
        let caret = text.chars().count();
        Field { text, caret, all: false }
    }

    fn set(&mut self, text: String) {
        *self = Field::new(text);
    }

    fn put(&mut self, words: &str) {
        if self.all {
            self.text.clear();
            self.caret = 0;
            self.all = false;
        }
        let mut chars: Vec<char> = self.text.chars().collect();
        for c in words.chars().filter(|c| !c.is_control()) {
            chars.insert(self.caret.min(chars.len()), c);
            self.caret += 1;
        }
        self.text = chars.into_iter().collect();
    }

    fn back(&mut self) {
        if self.all {
            self.set(String::new());
            return;
        }
        if self.caret > 0 {
            let mut chars: Vec<char> = self.text.chars().collect();
            chars.remove(self.caret - 1);
            self.caret -= 1;
            self.text = chars.into_iter().collect();
        }
    }

    fn forward(&mut self) {
        if self.all {
            self.set(String::new());
            return;
        }
        let mut chars: Vec<char> = self.text.chars().collect();
        if self.caret < chars.len() {
            chars.remove(self.caret);
            self.text = chars.into_iter().collect();
        }
    }

    fn step(&mut self, by: i64) {
        let n = self.text.chars().count();
        self.all = false;
        self.caret = (self.caret as i64 + by).clamp(0, n as i64) as usize;
    }
}

/// A length typed as seconds (`99`, `12.5`) or minutes and seconds (`1:39`, `1:02:03`).
fn seconds(text: &str) -> Option<f64> {
    let t = text.trim().trim_end_matches('s').trim();
    let mut total = 0.0;
    for part in t.split(':') {
        total = total * 60.0 + part.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)?;
    }
    Some(total)
}

/// A length as people write it: `1:39`, or `12 s` / `12.5 s` under a minute.
fn length_text(secs: f64) -> String {
    if secs < 60.0 {
        let s = format!("{secs:.2}");
        return s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    let whole = secs.floor() as u64;
    let rest = secs - whole as f64;
    let tail = if rest > 1e-6 { format!("{:.2}", rest).trim_start_matches('0').trim_end_matches('0').to_string() } else { String::new() };
    format!("{}:{:02}{tail}", whole / 60, whole % 60)
}

/// A frame rate typed as a number (`29.97`) or a fraction (`30000/1001`).
fn rate(text: &str) -> Option<f64> {
    let t = text.trim().trim_end_matches("fps").trim();
    match t.split_once('/') {
        Some((a, b)) => Some(a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok().filter(|v| *v > 0.0)?),
        None => t.parse::<f64>().ok(),
    }
    .filter(|v| v.is_finite())
}

/// A colour typed as `#rrggbb` or `#rgb` (the # may be left out).
fn colour(text: &str) -> Option<[f64; 3]> {
    let t = text.trim().trim_start_matches('#');
    let full: String = match t.len() {
        3 => t.chars().flat_map(|c| [c, c]).collect(),
        6 => t.to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&full, 16).ok()?;
    Some([((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0])
}

fn hex(c: [f64; 3]) -> String {
    let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

fn number(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{v}").trim_end_matches('0').to_string() }
}

/// What a field says is wrong with what is typed, if anything.
fn problem(which: Which, text: &str) -> Option<&'static str> {
    match which {
        Which::Name => text.trim().is_empty().then_some("a name"),
        Which::Width | Which::Height => match text.trim().parse::<u32>() {
            Ok(v) if (1..=MOST_SIDE).contains(&v) => None,
            _ => Some("1 to 30,000 px"),
        },
        Which::Rate => match rate(text) {
            Some(v) if v > 0.0 && v <= 1000.0 => None,
            _ => Some("above 0, up to 1000"),
        },
        Which::Length => match seconds(text) {
            Some(v) if v > 0.0 && v <= MOST_SECONDS => None,
            _ => Some("like 1:39 or 99"),
        },
        Which::Colour => colour(text).is_none().then_some("like #ffffff"),
    }
}

/// The faces the card writes in.
struct Faces {
    title: Font,
    body: Font,
    small: Font,
    strong: Font,
}

fn faces() -> Option<&'static Faces> {
    static F: std::sync::OnceLock<Option<Faces>> = std::sync::OnceLock::new();
    F.get_or_init(|| Some(Faces { title: faces::sans_at(620.0, 14.0)?, body: faces::sans_at(450.0, 13.0)?, small: faces::sans_at(450.0, 11.5)?, strong: faces::sans_at(600.0, 12.0)? })).as_ref()
}

/// What can be pressed on the card.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Spot {
    Field(usize),
    Size(usize),
    Swap,
    Rate(usize),
    Length(usize),
    Colour(usize),
    Cancel,
    Apply,
}

/// The card: whether it is open, since when, which composition it was opened on and as what,
/// its fields, which one is typed in, the change waiting for the AI's step to end, and what
/// went wrong making it.
pub struct Settings {
    pub open: bool,
    since: f64,
    was: Option<About>,
    fields: [Field; 6],
    focus: usize,
    waiting: Option<Vec<(String, Json)>>,
    trouble: Option<String>,
    /// Where the card is (points: its top left), and the pointer.
    at: (f64, f64),
    pointer: Option<(f64, f64)>,
}

impl Settings {
    pub const fn new() -> Settings {
        Settings { open: false, since: 0.0, was: None, fields: [const { Field { text: String::new(), caret: 0, all: false } }; 6], focus: 0, waiting: None, trouble: None, at: (0.0, 0.0), pointer: None }
    }

    /// Opens the card on composition `about`, its top left at `at` (points), at `t`.
    pub fn show(&mut self, about: About, at: (f64, f64), t: f64) {
        self.fields = [
            Field::new(about.name.clone()),
            Field::new(about.width.to_string()),
            Field::new(about.height.to_string()),
            Field::new(number((about.fps * 1000.0).round() / 1000.0)),
            Field::new(length_text(about.duration)),
            Field::new(hex(about.background)),
        ];
        self.focus = 0;
        self.fields[0].all = true;
        self.was = Some(about);
        self.open = true;
        self.since = t;
        self.waiting = None;
        self.trouble = None;
        self.at = at;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.waiting = None;
    }

    /// Whether the card wants to be drawn again at `t` (it is coming in, its caret blinks, or a
    /// change waits).
    pub fn busy(&self, t: f64) -> bool {
        self.open && (t - self.since < 0.4 || self.waiting.is_some())
    }

    /// Which blink the caret is in (the window draws again when it changes).
    pub fn blink(&self, t: f64) -> i64 {
        if self.open { ((t - self.since) / 0.53).floor() as i64 } else { -1 }
    }

    fn rect(&self) -> Rect {
        (self.at.0, self.at.1, WIDE, 432.0)
    }

    fn field_rect(&self, i: usize) -> Rect {
        let (x, y, _, _) = self.rect();
        match ORDER[i] {
            Which::Name => (x + FIELDS, y + 50.0, 320.0, 28.0),
            Which::Width => (x + FIELDS, y + 96.0, 74.0, 28.0),
            Which::Height => (x + FIELDS + 94.0, y + 96.0, 74.0, 28.0),
            Which::Rate => (x + FIELDS, y + 174.0, 74.0, 28.0),
            Which::Length => (x + FIELDS, y + 252.0, 74.0, 28.0),
            Which::Colour => (x + FIELDS, y + 330.0, 108.0, 28.0),
        }
    }

    /// The shortcuts' places in a row starting at `(x, y)` (points), each as wide as its words.
    fn chips(f: &Faces, words: &[String], (x, y): (f64, f64)) -> Vec<Rect> {
        let mut at = x;
        words
            .iter()
            .map(|w| {
                let wide = Label::new(f.small.clone(), 11.5, w).wide + 22.0;
                let r = (at, y, wide, 24.0);
                at += wide + 6.0;
                r
            })
            .collect()
    }

    fn size_words() -> Vec<String> {
        SIZES.iter().map(|s| s.0.to_string()).collect()
    }

    fn rate_words() -> Vec<String> {
        RATES.iter().map(|r| number(*r)).collect()
    }

    fn length_words() -> Vec<String> {
        LENGTHS.iter().map(|l| l.0.to_string()).collect()
    }

    fn buttons(&self) -> (Rect, Rect) {
        let (x, y, w, h) = self.rect();
        ((x + w - 20.0 - 84.0 - 10.0 - 76.0, y + h - 46.0, 76.0, 30.0), (x + w - 20.0 - 84.0, y + h - 46.0, 84.0, 30.0))
    }

    fn swap_rect(&self) -> Rect {
        let (fx, fy, fw, _) = self.field_rect(2);
        (fx + fw + 34.0, fy + 1.0, 26.0, 26.0)
    }

    /// What is under `p`.
    fn spot(&self, p: (f64, f64)) -> Option<Spot> {
        let f = faces()?;
        let (x, _, _, _) = self.rect();
        for i in 0..ORDER.len() {
            if button::inside(self.field_rect(i), p) {
                return Some(Spot::Field(i));
            }
        }
        let rows = [
            (Self::chips(f, &Self::size_words(), (x + FIELDS, self.field_rect(1).1 + 36.0)), 0),
            (Self::chips(f, &Self::rate_words(), (x + FIELDS, self.field_rect(3).1 + 36.0)), 1),
            (Self::chips(f, &Self::length_words(), (x + FIELDS, self.field_rect(4).1 + 36.0)), 2),
        ];
        for (rects, kind) in rows {
            if let Some(i) = rects.iter().position(|r| button::inside(*r, p)) {
                return Some(match kind {
                    0 => Spot::Size(i),
                    1 => Spot::Rate(i),
                    _ => Spot::Length(i),
                });
            }
        }
        let (cx, cy, cw, _) = self.field_rect(5);
        for i in 0..COLOURS.len() {
            let r = (cx + cw + 14.0 + i as f64 * 28.0, cy + 2.0, 24.0, 24.0);
            if button::inside(r, p) {
                return Some(Spot::Colour(i));
            }
        }
        if button::inside(self.swap_rect(), p) {
            return Some(Spot::Swap);
        }
        let (cancel, apply) = self.buttons();
        if button::inside(cancel, p) {
            return Some(Spot::Cancel);
        }
        if button::inside(apply, p) {
            return Some(Spot::Apply);
        }
        None
    }

    pub fn point(&mut self, p: Option<(f64, f64)>) {
        self.pointer = p;
    }

    /// Whether the pointer at `p` is over something to press on the card.
    pub fn hand(&self, p: (f64, f64)) -> bool {
        self.open && matches!(self.spot(p), Some(s) if !matches!(s, Spot::Field(_)))
    }

    /// Whether `p` is on the card.
    pub fn over(&self, p: (f64, f64)) -> bool {
        self.open && button::inside(self.rect(), p)
    }

    /// A press at `p`: on a field, typing goes there; on a shortcut, it is filled in; away from
    /// the card, it closes. Gives the change to make, when Apply was pressed and it is right.
    pub fn press(&mut self, p: (f64, f64)) -> Option<Vec<(String, Json)>> {
        if !self.over(p) {
            self.close();
            return None;
        }
        match self.spot(p)? {
            Spot::Field(i) => {
                self.focus = i;
                let n = self.fields[i].text.chars().count();
                self.fields[i].caret = n;
                self.fields[i].all = false;
            }
            Spot::Size(i) => {
                let (_, w, h) = SIZES[i];
                self.fields[1].set(w.to_string());
                self.fields[2].set(h.to_string());
            }
            Spot::Swap => {
                let (w, h) = (self.fields[1].text.clone(), self.fields[2].text.clone());
                self.fields[1].set(h);
                self.fields[2].set(w);
            }
            Spot::Rate(i) => self.fields[3].set(number(RATES[i])),
            Spot::Length(i) => {
                let end = LENGTHS[i].1.or_else(|| self.was.as_ref().map(|a| a.last_end)).unwrap_or(1.0);
                self.fields[4].set(length_text(end));
            }
            Spot::Colour(i) => self.fields[5].set(COLOURS[i].to_string()),
            Spot::Cancel => self.close(),
            Spot::Apply => return self.change(),
        }
        None
    }

    /// A key: its code, what it types, and whether Shift or Command is down. Gives the change to
    /// make when Return was pressed and it is right.
    pub fn key(&mut self, code: u16, text: &str, shift: bool) -> Option<Vec<(String, Json)>> {
        let numeric = matches!(ORDER[self.focus], Which::Width | Which::Height | Which::Rate | Which::Length);
        match code {
            53 => self.close(),
            36 | 76 => return self.change(),
            48 => {
                let n = ORDER.len();
                self.focus = if shift { (self.focus + n - 1) % n } else { (self.focus + 1) % n };
                self.fields[self.focus].all = true;
            }
            51 => self.fields[self.focus].back(),
            117 => self.fields[self.focus].forward(),
            123 => self.fields[self.focus].step(-1),
            124 => self.fields[self.focus].step(1),
            115 => self.fields[self.focus].step(-(1 << 30)),
            119 => self.fields[self.focus].step(1 << 30),
            // up and down: a number one more or less (ten with Shift)
            126 | 125 if numeric => {
                let by = if shift { 10.0 } else { 1.0 } * if code == 126 { 1.0 } else { -1.0 };
                let f = &mut self.fields[self.focus];
                match ORDER[self.focus] {
                    Which::Length => {
                        if let Some(v) = seconds(&f.text) {
                            f.set(length_text((v + by).max(1.0)));
                        }
                    }
                    Which::Rate => {
                        if let Some(v) = rate(&f.text) {
                            f.set(number((v + by).max(1.0)));
                        }
                    }
                    _ => {
                        if let Ok(v) = f.text.trim().parse::<i64>() {
                            f.set((v + by as i64).clamp(1, MOST_SIDE as i64).to_string());
                        }
                    }
                }
            }
            _ => {
                if !text.is_empty() && text.chars().all(|c| !c.is_control()) {
                    self.fields[self.focus].put(text);
                }
            }
        }
        None
    }

    /// Words put in from the clipboard.
    pub fn paste(&mut self, words: &str) {
        let one: String = words.lines().next().unwrap_or("").trim().to_string();
        self.fields[self.focus].put(&one);
    }

    pub fn select_all(&mut self) {
        self.fields[self.focus].all = true;
    }

    /// The commands that make what is typed so (none to make when nothing changed), or none
    /// and what is wrong said by each field.
    fn change(&mut self) -> Option<Vec<(String, Json)>> {
        if ORDER.iter().zip(&self.fields).any(|(w, f)| problem(*w, &f.text).is_some()) {
            self.trouble = Some("Some of it is not right yet".into());
            return None;
        }
        let was = self.was.clone()?;
        let comp = Json::Number(was.comp as f64);
        let (w, h) = (self.fields[1].text.trim().parse::<u32>().ok()?, self.fields[2].text.trim().parse::<u32>().ok()?);
        let mut out = Vec::new();
        // a new size: what is in the picture kept in its middle
        if (w, h) != (was.width, was.height) {
            let (dx, dy) = ((was.width as f64 - w as f64) / 2.0, (was.height as f64 - h as f64) / 2.0);
            out.push(("comp.crop".to_string(), Json::object([("comp", comp.clone()), ("rect", Json::Array(vec![Json::Number(dx), Json::Number(dy), Json::Number(w as f64), Json::Number(h as f64)]))])));
        }
        let mut set: Vec<(&str, Json)> = vec![("comp", comp)];
        let name = self.fields[0].text.trim().to_string();
        if name != was.name {
            set.push(("name", Json::from(name.as_str())));
        }
        let r = rate(&self.fields[3].text)?;
        if (r - was.fps).abs() > 1e-6 {
            // a fraction as typed, else the number
            let typed = self.fields[3].text.trim().trim_end_matches("fps").trim().to_string();
            set.push(("fps", if typed.contains('/') { Json::from(typed.as_str()) } else { Json::Number(r) }));
        }
        let d = seconds(&self.fields[4].text)?;
        if (d - was.duration).abs() > 1e-6 {
            set.push(("duration", Json::Number(d)));
        }
        let c = colour(&self.fields[5].text)?;
        if hex(c) != hex(was.background) {
            set.push(("background", Json::from(hex(c).as_str())));
        }
        if set.len() > 1 {
            out.push(("comp.set".to_string(), Json::object(set)));
        }
        if out.is_empty() {
            self.close();
            return None;
        }
        Some(out)
    }

    /// The change waits for the AI's step to end.
    pub fn wait(&mut self, change: Vec<(String, Json)>) {
        self.waiting = Some(change);
    }

    /// The change waiting, taken (to try again).
    pub fn take_waiting(&mut self) -> Option<Vec<(String, Json)>> {
        self.waiting.take()
    }

    /// Making the change went wrong: why.
    pub fn failed(&mut self, why: String) {
        self.trouble = Some(why);
    }

    /// Draws the card at `t` on `canvas` (`scale` pixels to the point), `alpha` faded in.
    pub fn paint(&self, canvas: &mut Canvas, scale: f64, t: f64, alpha: f32) {
        if !self.open {
            return;
        }
        let Some(f) = faces() else { return };
        let px = |v: f64| v * scale;
        let round = |r: Rect, k: f64| Path::rounded(px(r.0), px(r.1), px(r.2), px(r.3), px(k));
        let come = smooth((t - self.since) / 0.18) as f32 * alpha;
        let rise = (1.0 - settle((t - self.since) / 0.3)) * 8.0;
        let (x, y, w, h) = self.rect();
        let y = y + rise;
        let near = |r: Rect| self.pointer.is_some_and(|p| button::inside(r, p));
        canvas.shadow(px(x), px(y + 8.0), px(w), px(h), px(14.0), px(26.0), INK, come * 0.16);
        canvas.fill_faded(&round((x, y, w, h), 14.0), PAPER, come);
        canvas.stroke(&Path::rounded_points(px(x), px(y), px(w), px(h), px(14.0)), true, 0.9 * scale, EDGE, come);
        Label::new(f.title.clone(), 14.0, "Composition").draw(canvas, x + 20.0, y + 31.0, 1.0, scale, INK, come);
        let lift = |r: Rect| (r.0, r.1 + rise, r.2, r.3);
        // each field: its label, its well, its words, the caret where it types, what is wrong
        let labels = ["Name", "Size", "", "Frame rate", "Length", "Background"];
        for (i, which) in ORDER.iter().enumerate() {
            let r = lift(self.field_rect(i));
            if !labels[i].is_empty() {
                Label::new(f.small.clone(), 11.5, labels[i]).draw(canvas, x + 20.0, r.1 + 18.0, 1.0, scale, GREY, come);
            }
            let focused = i == self.focus;
            canvas.fill_faded(&round(r, 7.0), WELL, come);
            if focused {
                canvas.stroke(&Path::rounded_points(px(r.0), px(r.1), px(r.2), px(r.3), px(7.0)), true, 1.2 * scale, INK, come * 0.7);
            }
            let field = &self.fields[i];
            let mut text_x = r.0 + 10.0;
            if *which == Which::Colour {
                let c = colour(&field.text).map_or(PAPER, |c| Rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8));
                canvas.fill_faded(&round((r.0 + 7.0, r.1 + 7.0, 14.0, 14.0), 7.0), c, come);
                canvas.stroke(&Path::rounded_points(px(r.0 + 7.0), px(r.1 + 7.0), px(14.0), px(14.0), px(7.0)), true, 0.7 * scale, EDGE, come);
                text_x += 18.0;
            }
            let words = Label::new(f.body.clone(), 13.0, &field.text);
            if focused && field.all && !field.text.is_empty() {
                canvas.fill_faded(&round((text_x - 2.0, r.1 + 5.0, words.wide + 4.0, r.3 - 10.0), 3.0), Rgb(205, 222, 250), come);
            }
            words.draw(canvas, text_x, r.1 + 19.0, 1.0, scale, INK, come);
            if focused && !field.all && ((t - self.since) / 0.53).floor() as i64 % 2 == 0 {
                let before: String = field.text.chars().take(field.caret).collect();
                let cx = text_x + Label::new(f.body.clone(), 13.0, &before).wide;
                canvas.fill_faded(&round((cx, r.1 + 6.0, 1.3, r.3 - 12.0), 0.6), INK, come);
            }
            if let Some(why) = problem(*which, &field.text) {
                let (ex, ey) = match which {
                    Which::Width | Which::Height => (lift(self.swap_rect()).0 + 36.0, r.1 + 18.0),
                    _ => (r.0 + r.2 + 12.0, r.1 + 18.0),
                };
                if !(*which == Which::Width && problem(Which::Height, &self.fields[2].text).is_some()) {
                    Label::new(f.small.clone(), 11.5, why).draw(canvas, ex, ey, 1.0, scale, RED, come);
                }
            }
        }
        // what goes after the fields: ×, px, fps, how lengths are typed
        let (_, wy, ww, _) = lift(self.field_rect(1));
        Label::new(f.body.clone(), 13.0, "×").draw(canvas, x + FIELDS + ww + 6.0, wy + 19.0, 1.0, scale, FAINT, come);
        let (hx, _, hw, _) = self.field_rect(2);
        Label::new(f.small.clone(), 11.5, "px").draw(canvas, hx + hw + 8.0, wy + 18.0, 1.0, scale, FAINT, come);
        let (rx, ry, rw, _) = lift(self.field_rect(3));
        Label::new(f.small.clone(), 11.5, "fps").draw(canvas, rx + rw + 8.0, ry + 18.0, 1.0, scale, FAINT, come);
        let (lx, ly, lw, _) = lift(self.field_rect(4));
        if problem(Which::Length, &self.fields[4].text).is_none() {
            Label::new(f.small.clone(), 11.5, "min:sec, or seconds").draw(canvas, lx + lw + 8.0, ly + 18.0, 1.0, scale, FAINT, come);
        }
        // the swap of width and height
        let s = lift(self.swap_rect());
        canvas.stroke(&Path::rounded_points(px(s.0), px(s.1), px(s.2), px(s.3), px(s.2 / 2.0)), true, 0.9 * scale, if near(self.swap_rect()) { INK } else { EDGE }, come);
        let (sx, sy) = (s.0 + s.2 / 2.0, s.1 + s.3 / 2.0);
        canvas.stroke(&[(px(sx - 5.0), px(sy - 2.5)), (px(sx + 5.0), px(sy - 2.5))], false, 1.1 * scale, INK, come);
        canvas.stroke(&[(px(sx + 2.5), px(sy - 5.0)), (px(sx + 5.0), px(sy - 2.5)), (px(sx + 2.5), px(sy))], false, 1.1 * scale, INK, come);
        canvas.stroke(&[(px(sx + 5.0), px(sy + 2.5)), (px(sx - 5.0), px(sy + 2.5))], false, 1.1 * scale, INK, come);
        canvas.stroke(&[(px(sx - 2.5), px(sy)), (px(sx - 5.0), px(sy + 2.5)), (px(sx - 2.5), px(sy + 5.0))], false, 1.1 * scale, INK, come);
        // the shortcuts, lit while what is typed is theirs
        let size_now = (self.fields[1].text.trim().parse::<u32>().ok(), self.fields[2].text.trim().parse::<u32>().ok());
        let rate_now = rate(&self.fields[3].text);
        let length_now = seconds(&self.fields[4].text);
        let last_end = self.was.as_ref().map_or(0.0, |a| a.last_end);
        let rows: [(Vec<String>, f64, Box<dyn Fn(usize) -> bool>); 3] = [
            (Self::size_words(), self.field_rect(1).1 + 36.0, Box::new(|i| size_now == (Some(SIZES[i].1), Some(SIZES[i].2)))),
            (Self::rate_words(), self.field_rect(3).1 + 36.0, Box::new(|i| rate_now.is_some_and(|r| (r - RATES[i]).abs() < 1e-6))),
            (Self::length_words(), self.field_rect(4).1 + 36.0, Box::new(|i| length_now.is_some_and(|l| (l - LENGTHS[i].1.unwrap_or(last_end)).abs() < 1e-6))),
        ];
        for (words, cy, lit) in rows {
            for (i, r) in Self::chips(f, &words, (x + FIELDS, cy)).into_iter().enumerate() {
                let on = lit(i);
                let rr = lift(r);
                if on {
                    canvas.fill_faded(&round(rr, rr.3 / 2.0), INK, come);
                } else {
                    if near(r) {
                        canvas.fill_faded(&round(rr, rr.3 / 2.0), WELL, come);
                    }
                    canvas.stroke(&Path::rounded_points(px(rr.0), px(rr.1), px(rr.2), px(rr.3), px(rr.3 / 2.0)), true, 0.9 * scale, EDGE, come);
                }
                let l = Label::new(f.small.clone(), 11.5, &words[i]);
                l.draw(canvas, rr.0 + (rr.2 - l.wide) / 2.0, rr.1 + 16.0, 1.0, scale, if on { PAPER } else { INK }, come);
            }
        }
        // the colours: a ring round the one typed
        let (cx, cy, cw, _) = lift(self.field_rect(5));
        let now = colour(&self.fields[5].text).map(hex);
        for (i, c) in COLOURS.iter().enumerate() {
            let v = colour(c).unwrap_or([1.0; 3]);
            let rgb = Rgb((v[0] * 255.0).round() as u8, (v[1] * 255.0).round() as u8, (v[2] * 255.0).round() as u8);
            let r = (cx + cw + 14.0 + i as f64 * 28.0, cy + 2.0, 24.0, 24.0);
            canvas.fill_faded(&round((r.0 + 3.0, r.1 + 3.0, 18.0, 18.0), 9.0), rgb, come);
            canvas.stroke(&Path::rounded_points(px(r.0 + 3.0), px(r.1 + 3.0), px(18.0), px(18.0), px(9.0)), true, 0.7 * scale, EDGE, come);
            if now.as_deref() == Some(*c) {
                canvas.stroke(&Path::rounded_points(px(r.0), px(r.1), px(24.0), px(24.0), px(12.0)), true, 1.4 * scale, INK, come);
            }
        }
        // the foot: how to use it, or what went wrong; Cancel, and Apply (waiting for the AI)
        let (cancel, apply) = self.buttons();
        let (cancel, apply) = (lift(cancel), lift(apply));
        let foot = match &self.trouble {
            Some(why) => (why.as_str(), RED),
            None => ("Tab: next  ·  Return: apply  ·  Esc: close", FAINT),
        };
        fit(&f.small, 11.0, foot.0, cancel.0 - x - 30.0).draw(canvas, x + 20.0, cancel.1 + 19.0, 1.0, scale, foot.1, come);
        if near(self.buttons().0) {
            canvas.fill_faded(&round(cancel, cancel.3 / 2.0), WELL, come);
        }
        canvas.stroke(&Path::rounded_points(px(cancel.0), px(cancel.1), px(cancel.2), px(cancel.3), px(cancel.3 / 2.0)), true, 0.9 * scale, EDGE, come);
        let l = Label::new(f.strong.clone(), 12.0, "Cancel");
        l.draw(canvas, cancel.0 + (cancel.2 - l.wide) / 2.0, cancel.1 + 19.5, 1.0, scale, INK, come);
        canvas.fill_faded(&round(apply, apply.3 / 2.0), if near(self.buttons().1) { Rgb(48, 48, 54) } else { INK }, come);
        let l = Label::new(f.strong.clone(), 12.0, if self.waiting.is_some() { "Waiting…" } else { "Apply" });
        l.draw(canvas, apply.0 + (apply.2 - l.wide) / 2.0, apply.1 + 19.5, 1.0, scale, PAPER, come);
    }
}

/// Words in a face at a size, cut to fit `wide` points (with an ellipsis).
fn fit(font: &Font, size: f64, text: &str, wide: f64) -> Label {
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

    #[test]
    fn what_is_typed_reads_as_it_should() {
        assert_eq!(seconds("1:39"), Some(99.0));
        assert_eq!(seconds("99"), Some(99.0));
        assert_eq!(seconds("12.5 s"), Some(12.5));
        assert_eq!(seconds("1:02:03"), Some(3723.0));
        assert_eq!(seconds("abc"), None);
        assert_eq!(length_text(99.0), "1:39");
        assert_eq!(length_text(12.5), "12.5");
        assert!((rate("30000/1001").unwrap() - 29.97).abs() < 0.001);
        assert_eq!(rate("120 fps"), Some(120.0));
        assert_eq!(colour("#fff"), Some([1.0; 3]));
        assert_eq!(colour("1d2b3a").map(hex), Some("#1d2b3a".into()));
        assert_eq!(problem(Which::Width, "0"), Some("1 to 30,000 px"));
        assert_eq!(problem(Which::Width, "1080"), None);
        assert_eq!(problem(Which::Rate, "1200"), Some("above 0, up to 1000"));
        assert_eq!(problem(Which::Name, "  "), Some("a name"));
    }

    #[test]
    fn typing_goes_where_the_caret_is() {
        let mut f = Field::new("1080".into());
        f.step(-2);
        f.put("5");
        assert_eq!(f.text, "10580");
        f.back();
        assert_eq!(f.text, "1080");
        f.all = true;
        f.put("7");
        assert_eq!((f.text.as_str(), f.caret), ("7", 1), "chosen all, what is typed takes its place");
    }

    fn about() -> About {
        About { comp: 7, name: "Teaser".into(), width: 1080, height: 1920, fps: 30.0, duration: 99.0, background: [1.0; 3], last_end: 98.5, three_d: false }
    }

    #[test]
    fn a_new_size_keeps_the_picture_in_the_middle_and_only_changes_are_made() {
        let mut s = Settings::new();
        s.show(about(), (0.0, 0.0), 0.0);
        s.fields[1].set("1080".into());
        s.fields[2].set("1080".into());
        s.fields[4].set("30".into());
        let change = s.change().unwrap();
        assert_eq!(change[0].0, "comp.crop");
        let rect: Vec<f64> = change[0].1.get("rect").unwrap().as_array().unwrap().iter().filter_map(Json::as_f64).collect();
        assert_eq!(rect, [0.0, 420.0, 1080.0, 1080.0], "the middle kept: 420 px off the top");
        assert_eq!(change[1].0, "comp.set");
        assert_eq!(change[1].1.get("duration").and_then(Json::as_f64), Some(30.0));
        assert!(change[1].1.get("fps").is_none() && change[1].1.get("name").is_none(), "what did not change is left alone");
        // a shortcut to the last layer's end
        let mut s = Settings::new();
        s.show(about(), (0.0, 0.0), 0.0);
        s.fields[4].set(length_text(about().last_end));
        assert_eq!(seconds(&s.fields[4].text), Some(98.5));
        // nothing changed: nothing to do, and it closes
        let mut s = Settings::new();
        s.show(about(), (0.0, 0.0), 0.0);
        assert!(s.change().is_none() && !s.open);
        // something wrong: nothing made, and said
        let mut s = Settings::new();
        s.show(about(), (0.0, 0.0), 0.0);
        s.fields[1].set("0".into());
        assert!(s.change().is_none() && s.trouble.is_some());
    }
}
