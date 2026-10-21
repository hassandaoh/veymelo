//! A black button with round corners and a soft shadow: it leans toward the pointer when it
//! is near, lifts and casts a deeper shadow, goes a little lighter, and sinks when pressed. A
//! press that starts and ends on it counts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use veymelo_path::Path;
use veymelo_raster::{Canvas, Rgb};
use veymelo_font::Font;
use crate::motion::Spring;

const FILL: Rgb = Rgb(10, 10, 10);
const FILL_NEAR: Rgb = Rgb(40, 40, 44);
const WORDS: Rgb = Rgb(255, 255, 255);
const SHADOW: Rgb = Rgb(0, 0, 0);

/// Its height, the room either side of the words and its corners (points).
pub const HEIGHT: f64 = 50.0;
pub const PAD: f64 = 28.0;
pub const ROUND: f64 = 14.0;
/// How far it feels the pointer beyond its edge, how far it leans toward it at most and how
/// high it lifts under it (points).
const FEEL: f64 = 48.0;
const LEAN: f64 = 6.0;
const LIFT: f64 = 2.0;

/// Words set once: each run of them in a face that has their letters (the words' own face, or
/// for a script it has none of, the system's face for it: Thai, Arabic, Hebrew, Japanese,
/// Chinese, Korean, those of India, emoji…), shaped in it (letters joined, marks set where
/// they belong, syllables put together), right-to-left writing turned the way it is read, and
/// the runs side by side as they are seen. How wide they are (points).
pub struct Label {
    pub font: Font,
    pub size: f64,
    pub wide: f64,
    set: Arc<Set>,
}

/// Runs of glyphs as they are seen, left to right: each its face and its glyphs, and how wide
/// they all are.
struct Set {
    runs: Vec<(Font, Vec<Placed>)>,
    wide: f64,
}

/// A glyph, where across and how far up it is (points).
type Placed = (u16, f64, f64);

/// Words already set, by their face, size and words.
type Kept = Mutex<HashMap<(usize, u64, u64, u64, String), Arc<Set>>>;

impl Label {
    pub fn new(font: Font, size: f64, text: &str) -> Label {
        static KEPT: OnceLock<Kept> = OnceLock::new();
        let bits = |tag: &str| font.setting(tag).map_or(0, f64::to_bits);
        let key = (font.id(), bits("wght"), bits("opsz"), size.to_bits(), text.to_string());
        let kept = KEPT.get_or_init(Default::default);
        let found = kept.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned();
        let set = found.unwrap_or_else(|| {
            let set = Arc::new(set_words(&font, size, text));
            let mut k = kept.lock().unwrap_or_else(|e| e.into_inner());
            if k.len() > 4096 {
                k.clear();
            }
            k.insert(key, set.clone());
            set
        });
        Label { font, size, wide: set.wide, set }
    }

    /// The words as a path with their baseline starting at (`x`, `y`) points, `grow` times
    /// their size, at `scale` pixels to the point (the glyphs in colour left out: see
    /// [`Label::draw`]).
    pub fn path(&self, x: f64, y: f64, grow: f64, scale: f64) -> Path {
        let mut path = Path::default();
        for (face, glyphs) in &self.set.runs {
            let k = self.size * grow / face.units;
            for &(g, at, up) in glyphs {
                if coloured(face, g) {
                    continue;
                }
                let (gx, gy) = (x + at * grow, y - up * grow);
                face.outline(g, &mut path, &|u, v| ((gx + u * k) * scale, (gy - v * k) * scale));
            }
        }
        path
    }

    /// Draws the words as [`Label::path`] has them, in `ink`, `alpha` opaque, and the glyphs in
    /// colour (emoji) in their own colours.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, canvas: &mut Canvas, x: f64, y: f64, grow: f64, scale: f64, ink: Rgb, alpha: f32) {
        canvas.fill_faded(&self.path(x, y, grow, scale), ink, alpha);
        for (face, glyphs) in self.set.runs.iter().filter(|(f, _)| f.has_color()) {
            let em = self.size * grow;
            let k = em / face.units;
            for &(g, at, up) in glyphs {
                let (gx, gy) = (x + at * grow, y - up * grow);
                if let Some(layers) = face.color_layers(g) {
                    // its layers bottom first, each in its colour (none: the ink)
                    for (lg, colour) in layers {
                        let mut part = Path::default();
                        face.outline(lg, &mut part, &|u, v| ((gx + u * k) * scale, (gy - v * k) * scale));
                        let (c, a) = colour.map_or((ink, 1.0), |c| (Rgb((c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8), c[3]));
                        canvas.fill_faded(&part, c, alpha * a);
                    }
                } else if let Some(p) = face.picture(g, em * scale)
                    && let Some(pic) = glyph_picture(face, g, &p)
                {
                    // where its picture's bottom left is from the glyph's origin, and how big it
                    // is (points)
                    let per = em / p.ppem;
                    let (w, h) = (pic.width as f64 * per, pic.height as f64 * per);
                    let (left, bottom) = (gx + p.origin.0 * per, gy - p.origin.1 * per);
                    put_picture(canvas, &pic, (left * scale, (bottom - h) * scale, w * scale, h * scale), alpha);
                }
            }
        }
    }

    /// How far below the middle of a box the baseline goes so the words sit in its middle.
    pub fn drop(&self, grow: f64) -> f64 {
        (self.font.ascent + self.font.descent) / 2.0 * self.size * grow / self.font.units * 0.92
    }
}

/// Whether glyph `g` of `face` is drawn in colour (its layers, or its picture).
fn coloured(face: &Font, g: u16) -> bool {
    face.has_color() && (face.color_layers(g).is_some() || face.picture(g, 64.0).is_some())
}

/// `text` set in `font` at `size` points: each character's face (what joins or dresses an
/// emoji, and what steers how letters join, in the face of what it follows; what the font
/// lacks in the system's face for it), the levels of right-to-left writing in it, the runs of
/// one face and level shaped and put in the order they are seen. The same as a text layer of
/// the video sets them.
fn set_words(font: &Font, size: f64, text: &str) -> Set {
    let chars: Vec<char> = text.chars().collect();
    let weight = font.setting("wght").unwrap_or(font.weight() as f64);
    let mut faces: Vec<Font> = vec![font.clone()];
    let mut face_of: Vec<usize> = Vec::with_capacity(chars.len());
    let index = |f: Font, faces: &mut Vec<Font>| match faces.iter().position(|g| g.id() == f.id() && g.setting("wght") == f.setting("wght")) {
        Some(i) => i,
        None => {
            faces.push(f);
            faces.len() - 1
        }
    };
    for (i, &c) in chars.iter().enumerate() {
        let steers = matches!(c, '\u{200C}' | '\u{200D}');
        let joins = steers || matches!(c as u32, 0xFE0E | 0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F);
        let emoji = || crate::faces::fallback('\u{1F600}', weight, size).filter(|f| f.glyph(c) != 0);
        let face = match face_of.last() {
            Some(&prev) if joins && (steers || faces[prev].glyph(c) != 0) => prev,
            _ if chars.get(i + 1) == Some(&'\u{FE0F}') && emoji().is_some() => index(emoji().unwrap(), &mut faces),
            _ if font.glyph(c) != 0 || c.is_whitespace() || c.is_control() => 0,
            _ => match crate::faces::fallback(c, weight, size) {
                Some(f) => index(f, &mut faces),
                None => 0,
            },
        };
        face_of.push(face);
    }
    // the levels of right-to-left writing, each part between the dots the app sets between
    // things (a name · a size · a length) read on its own: a name in Arabic does not take the
    // numbers after it into its run
    let mut levels = vec![0u8; chars.len()];
    let mut start = 0;
    for end in (0..=chars.len()).filter(|&i| i == chars.len() || chars[i] == '·') {
        if end > start {
            levels[start..end].copy_from_slice(&veymelo_text::bidi::resolve(&chars[start..end], veymelo_text::bidi::Direction::Ltr).levels);
        }
        start = end + 1;
    }
    // runs of one face and level (what only steers the writing and has no glyph left out), in
    // the order they are seen
    let mut runs: Vec<(usize, u8, Vec<char>)> = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        if ignorable(c) && faces[face_of[i]].glyph(c) == 0 {
            continue;
        }
        let rtl = levels[i] % 2 == 1;
        let c = if rtl { veymelo_text::bidi::mirror(c) } else { c };
        match runs.last_mut() {
            Some((f, l, r)) if *f == face_of[i] && *l == levels[i] => r.push(c),
            _ => runs.push((face_of[i], levels[i], vec![c])),
        }
    }
    let order = veymelo_text::bidi::visual_order(&runs.iter().map(|r| r.1).collect::<Vec<u8>>());
    let mut pen = 0.0;
    let mut out = Vec::new();
    for k in order {
        let (f, level, text) = &runs[k];
        let face = &faces[*f];
        let options = veymelo_font::shape::Options { rtl: Some(level % 2 == 1), size: Some(size), ..Default::default() };
        let kf = size / face.units;
        let mut glyphs = Vec::new();
        for g in veymelo_font::shape::shape_with(face, text, &options) {
            glyphs.push((g.id, pen + g.dx * kf, g.dy * kf));
            pen += g.advance * kf;
        }
        out.push((face.clone(), glyphs));
    }
    Set { runs: out, wide: pen }
}

/// Characters drawn as nothing that only steer the writing (direction marks and isolates,
/// joiners, variation selectors, invisible spaces and hyphens, tags).
fn ignorable(c: char) -> bool {
    matches!(c as u32, 0x00AD | 0x034F | 0x061C | 0x115F..=0x1160 | 0x17B4..=0x17B5 | 0x180B..=0x180F | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F | 0xFEFF | 0xFFA0 | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0000..=0xE0FFF)
}

/// A colour glyph's picture read (premultiplied), kept by its face, glyph and size.
fn glyph_picture(face: &Font, g: u16, p: &veymelo_font::GlyphPicture) -> Option<Arc<veymelo_os::Picture>> {
    type Pictures = Mutex<HashMap<(usize, u16, u64), Option<Arc<veymelo_os::Picture>>>>;
    static KEPT: OnceLock<Pictures> = OnceLock::new();
    let mut kept = KEPT.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if kept.len() > 1024 {
        kept.clear();
    }
    kept.entry((face.id(), g, p.ppem.to_bits())).or_insert_with(|| veymelo_os::read_picture_bytes(p.bytes).ok().map(Arc::new)).clone()
}

/// A premultiplied picture drawn into the canvas pixels `(x, y, w, h)`, `alpha` opaque: each
/// pixel the mean of the picture's under it.
fn put_picture(canvas: &mut Canvas, pic: &veymelo_os::Picture, (x, y, w, h): (f64, f64, f64, f64), alpha: f32) {
    if w <= 0.0 || h <= 0.0 || pic.width == 0 || pic.height == 0 {
        return;
    }
    let (cw, ch) = (canvas.width as i64, canvas.height as i64);
    let (kx, ky) = (pic.width as f64 / w, pic.height as f64 / h);
    for py in (y.floor() as i64).max(0)..((y + h).ceil() as i64).min(ch) {
        let (s0, s1) = (((py as f64 - y) * ky).max(0.0), ((py as f64 + 1.0 - y) * ky).min(pic.height as f64));
        for px in (x.floor() as i64).max(0)..((x + w).ceil() as i64).min(cw) {
            let (t0, t1) = (((px as f64 - x) * kx).max(0.0), ((px as f64 + 1.0 - x) * kx).min(pic.width as f64));
            let mut sum = [0f32; 4];
            let mut n = 0.0;
            for sy in s0.floor() as usize..(s1.ceil() as usize).max(s0.floor() as usize + 1).min(pic.height) {
                for sx in t0.floor() as usize..(t1.ceil() as usize).max(t0.floor() as usize + 1).min(pic.width) {
                    let v = pic.px[sy * pic.width + sx];
                    (0..4).for_each(|c| sum[c] += v[c]);
                    n += 1.0;
                }
            }
            if n == 0.0 {
                continue;
            }
            // how much of the pixel the picture covers, at its edges
            let cover = (((px as f64 + 1.0).min(x + w) - (px as f64).max(x)) * ((py as f64 + 1.0).min(y + h) - (py as f64).max(y))).clamp(0.0, 1.0) as f32;
            let k = alpha * cover / n;
            let a = (sum[3] * k).clamp(0.0, 1.0);
            let d = ((py * cw + px) as usize) * 4;
            for (v, s) in canvas.pixels[d..d + 3].iter_mut().zip(sum) {
                *v = (*v as f32 * (1.0 - a) + (s * k).clamp(0.0, 1.0) * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Where a button rests (points: x, y, width, height).
pub type Rect = (f64, f64, f64, f64);

pub fn inside(r: Rect, p: (f64, f64)) -> bool {
    p.0 >= r.0 && p.0 <= r.0 + r.2 && p.1 >= r.1 && p.1 <= r.1 + r.3
}

/// A button's state: where it rests (once it can be pressed), how near the pointer is (0 to
/// 1), how far it leans (points: across, down), how far it is pressed (0 to 1), and whether it
/// is held down.
pub struct Button {
    pub rest: Option<Rect>,
    pub near: Spring,
    lean: [Spring; 2],
    press: Spring,
    held: bool,
}

impl Button {
    pub const fn new() -> Button {
        Button { rest: None, near: Spring::new(), lean: [Spring::new(); 2], press: Spring::new(), held: false }
    }

    pub fn hit(&self, p: (f64, f64)) -> bool {
        self.rest.is_some_and(|r| inside(r, p))
    }

    /// The mouse went down at `p`: held if it is on the button.
    pub fn down(&mut self, p: (f64, f64)) {
        self.held = self.hit(p);
    }

    /// The mouse came up at `p`: gives whether that pressed the button.
    pub fn up(&mut self, p: (f64, f64)) -> bool {
        let pressed = self.held && self.hit(p);
        self.held = false;
        pressed
    }

    /// Moves on `dt` seconds, resting at `rest` (None: it is not there to press), with the
    /// pointer at `pointer`. Gives whether it is still moving.
    pub fn step(&mut self, rest: Option<Rect>, pointer: Option<(f64, f64)>, dt: f64) -> bool {
        self.rest = rest;
        if rest.is_none() {
            self.held = false;
        }
        let (mut near, mut lean) = (0.0, (0.0, 0.0));
        if let (Some(p), Some((x, y, w, h))) = (pointer, rest) {
            let (dx, dy) = ((x - p.0).max(p.0 - x - w).max(0.0), (y - p.1).max(p.1 - y - h).max(0.0));
            near = (1.0 - dx.hypot(dy) / FEEL).max(0.0);
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            let pull = |d: f64, half: f64| (d / (half + FEEL)).clamp(-1.0, 1.0) * LEAN * near;
            lean = (pull(p.0 - cx, w / 2.0), pull(p.1 - cy, h / 2.0));
        }
        let mut moving = self.near.toward(near, dt, 200.0, 24.0);
        moving |= self.lean[0].toward(lean.0, dt, 220.0, 24.0);
        moving |= self.lean[1].toward(lean.1, dt, 220.0, 24.0);
        moving |= self.press.toward(if self.held { 1.0 } else { 0.0 }, dt, 600.0, 40.0);
        moving
    }

    /// Draws it resting at `rest`, `alpha` opaque, `rise` points below its place (coming in),
    /// `leave` of the way (0 to 1) sunk away, moved by `shift` points; at `scale` pixels to the
    /// point.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, canvas: &mut Canvas, scale: f64, label: &Label, rest: Rect, alpha: f64, rise: f64, leave: f64, shift: (f64, f64)) {
        let alpha = alpha * (1.0 - leave);
        if alpha <= 0.0 {
            return;
        }
        let (near, press) = (self.near.x.clamp(0.0, 1.0), self.press.x.clamp(0.0, 1.0));
        let grow = 1.0 - 0.035 * press - 0.08 * leave;
        let (bw, bh) = (rest.2 * grow, rest.3 * grow);
        let bx = rest.0 + (rest.2 - bw) / 2.0 + self.lean[0].x + shift.0;
        let by = rest.1 + (rest.3 - bh) / 2.0 + self.lean[1].x + rise - near * LIFT + press * 1.5 + leave * 6.0 + shift.1;
        let r = ROUND * grow;
        let lift = near * (1.0 - press);
        let a = alpha as f32;
        canvas.shadow(bx * scale, (by + 8.0 + 5.0 * lift) * scale, bw * scale, bh * scale, r * scale, (16.0 + 10.0 * lift) * scale, SHADOW, a * (0.16 + 0.1 * lift as f32));
        let fill = FILL.mix(FILL_NEAR, near).mix(FILL, press);
        canvas.fill_faded(&Path::rounded(bx * scale, by * scale, bw * scale, bh * scale, r * scale), fill, a);
        let x0 = bx + (bw - label.wide * grow) / 2.0;
        let y0 = by + bh / 2.0 + label.drop(grow);
        label.draw(canvas, x0, y0, grow, scale, WORDS, a);
    }
}

/// The height of a button drawn in hairlines, and the room either side of its words (points).
pub const LINE_HEIGHT: f64 = 32.0;
pub const LINE_PAD: f64 = 16.0;

/// What a button in hairlines shows before its words.
#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    None,
    Plus,
    Folder,
}

impl Icon {
    /// The room it takes before the words (points).
    pub fn room(self) -> f64 {
        if self == Icon::None { 0.0 } else { 20.0 }
    }
}

/// How wide a button in hairlines is for `label` after `icon` (points).
pub fn line_wide(label: &Label, icon: Icon) -> f64 {
    label.wide + 2.0 * LINE_PAD + icon.room()
}

const PAPER: Rgb = Rgb(255, 255, 255);
const LINE: Rgb = Rgb(161, 161, 170);
const LINE_NEAR: Rgb = Rgb(40, 40, 46);
const TINT: Rgb = Rgb(241, 241, 244);

/// A capsule in hairlines at `rect` (points): paper inside (a faint tint `near` the pointer, 0
/// to 1), its edge grey going to ink near the pointer (ink always when `strong`), its words in
/// ink after `icon`; `alpha` opaque, at `scale` pixels to the point.
#[allow(clippy::too_many_arguments)]
pub fn line_button(canvas: &mut Canvas, scale: f64, rect: Rect, label: &Label, icon: Icon, near: f64, strong: bool, alpha: f32) {
    let (x, y, w, h) = rect;
    let px = |v: f64| v * scale;
    let near = near.clamp(0.0, 1.0);
    canvas.fill_faded(&Path::rounded(px(x), px(y), px(w), px(h), px(h / 2.0)), PAPER.mix(TINT, near), alpha);
    let edge = if strong { LINE_NEAR } else { LINE.mix(LINE_NEAR, near) };
    canvas.stroke(&Path::rounded_points(px(x), px(y), px(w), px(h), px(h / 2.0)), true, crate::iso::STROKE * scale, edge, alpha);
    let mut tx = x + (w - label.wide - icon.room()) / 2.0;
    let cy = y + h / 2.0;
    let ink = Rgb(10, 10, 10);
    match icon {
        Icon::None => {}
        Icon::Plus => {
            let c = tx + 6.0;
            canvas.stroke(&[(px(c - 5.0), px(cy)), (px(c + 5.0), px(cy))], false, 1.1 * scale, ink, alpha);
            canvas.stroke(&[(px(c), px(cy - 5.0)), (px(c), px(cy + 5.0))], false, 1.1 * scale, ink, alpha);
        }
        Icon::Folder => {
            let (fx, fy) = (tx + 6.5, cy + 0.5);
            let folder: Vec<(f64, f64)> = [(-6.5, 4.5), (-6.5, -5.0), (-2.3, -5.0), (-0.9, -3.2), (6.5, -3.2), (6.5, 4.5)].iter().map(|(dx, dy)| (px(fx + dx), px(fy + dy))).collect();
            canvas.stroke(&folder, true, 1.1 * scale, ink, alpha);
        }
    }
    tx += icon.room();
    label.draw(canvas, tx, cy + label.drop(1.0), 1.0, scale, ink, alpha);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_counts_only_when_it_starts_and_ends_on_the_button() {
        let mut b = Button::new();
        let rest = Some((100.0, 100.0, 120.0, 50.0));
        b.step(rest, None, 0.0);
        b.down((150.0, 120.0));
        assert!(b.up((160.0, 130.0)));
        b.down((150.0, 120.0));
        assert!(!b.up((20.0, 20.0)), "let go away from it");
        b.down((20.0, 20.0));
        assert!(!b.up((150.0, 120.0)), "started away from it");
    }

    #[test]
    fn it_leans_toward_a_pointer_near_it() {
        let mut b = Button::new();
        let rest = Some((100.0, 100.0, 120.0, 50.0));
        for _ in 0..60 {
            b.step(rest, Some((240.0, 125.0)), 1.0 / 60.0);
        }
        assert!(b.near.x > 0.2 && b.lean[0].x > 0.5, "near {} lean {}", b.near.x, b.lean[0].x);
    }

    /// Words in many scripts set as labels, to be put beside the system's own drawing of them
    /// (run by hand: VEYMELO_FRAMES=/tmp/f cargo test labels_in_many_scripts).
    #[test]
    fn labels_in_many_scripts() {
        let Some(dir) = std::env::var_os("VEYMELO_FRAMES") else { return };
        let Some(font) = crate::faces::sans_at(430.0, 22.0) else { return };
        let lines = [
            "“Hello 👋🌍✨ 👍🏽 👨‍👩‍👧 🇹🇭 ❤️ 1️⃣”",
            "“မင်္ဂလာပါ” “សួស្តីពិភពលោក” “ສະບາຍດີ”",
            "“नमस्ते दुनिया क्षत्रिय” “ওহে বিশ্ব” “வணக்கம் உலகம்”",
            "“ሰላም ልዑል” “བཀྲ་ཤིས་བདེ་ལེགས” “ආයුබෝවන්”",
            "“નમસ્તે દુનિયા” “നമസ്കാരം ലോകം” “გამარჯობა”",
            "“مرحبا بالعالم” “שלום עולם” “سلام دنیا ۱۲۳”",
            "“สวัสดีครับ ยินดีต้อนรับ ผู้ใหญ่ ฤๅษี”",
        ];
        let (w, h) = (1100, 44 * lines.len() + 20);
        let mut canvas = Canvas::new(w, h, Rgb(255, 255, 255));
        for (i, line) in lines.iter().enumerate() {
            Label::new(font.clone(), 22.0, line).draw(&mut canvas, 10.0, 10.0 + 44.0 * (i + 1) as f64 - 14.0, 1.0, 1.0, Rgb(0, 0, 0), 1.0);
        }
        let png = veymelo_render::to_png(&veymelo_raster::Image { width: w, height: h, px: canvas.pixels.chunks(4).map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, 1.0]).collect() });
        std::fs::write(std::path::Path::new(&dir).join("labels.png"), png).unwrap();
    }
}
