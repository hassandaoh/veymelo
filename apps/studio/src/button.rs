//! A black button with round corners and a soft shadow: it leans toward the pointer when it
//! is near, lifts and casts a deeper shadow, goes a little lighter, and sinks when pressed. A
//! press that starts and ends on it counts.

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

/// Words set once in a face: the glyphs with their places, and how wide they are (points).
/// Letters the face has none of (Thai, in SF) are set in the system's face for them, shaped so
/// their marks sit where they belong.
pub struct Label {
    pub font: Font,
    pub size: f64,
    pub glyphs: Vec<(u16, f64)>,
    pub wide: f64,
    /// The glyphs set in the other face: glyph, where across, and how far up (points).
    other: Vec<(u16, f64, f64)>,
}

impl Label {
    pub fn new(font: Font, size: f64, text: &str) -> Label {
        let k = size / font.units;
        let mut pen = 0.0;
        let mut glyphs = vec![];
        let mut other = vec![];
        let chars: Vec<char> = text.chars().collect();
        let lacks = |c: char| font.glyph(c) == 0 && !c.is_whitespace();
        let mut i = 0;
        while i < chars.len() {
            match crate::faces::other().filter(|_| lacks(chars[i])) {
                Some(f) => {
                    // a run the face lacks, set and shaped in the other face
                    let start = i;
                    while i < chars.len() && (lacks(chars[i]) || (chars[i] == ' ' && i + 1 < chars.len() && lacks(chars[i + 1]))) {
                        i += 1;
                    }
                    let kf = size / f.units;
                    for g in veymelo_font::shape::shape(f, &chars[start..i]) {
                        other.push((g.id, pen + g.dx * kf, g.dy * kf));
                        pen += g.advance * kf;
                    }
                }
                None => {
                    let g = font.glyph(chars[i]);
                    glyphs.push((g, pen));
                    pen += font.advance(g) * k;
                    i += 1;
                }
            }
        }
        Label { font, size, glyphs, wide: pen, other }
    }

    /// The words as a path with their baseline starting at (`x`, `y`) points, `grow` times
    /// their size, at `scale` pixels to the point.
    pub fn path(&self, x: f64, y: f64, grow: f64, scale: f64) -> Path {
        let k = self.size * grow / self.font.units;
        let mut path = Path::default();
        for &(g, at) in &self.glyphs {
            let gx = x + at * grow;
            self.font.outline(g, &mut path, &|u, v| ((gx + u * k) * scale, (y - v * k) * scale));
        }
        if let Some(f) = crate::faces::other().filter(|_| !self.other.is_empty()) {
            let kf = self.size * grow / f.units;
            for &(g, at, up) in &self.other {
                let (gx, gy) = (x + at * grow, y - up * grow);
                f.outline(g, &mut path, &|u, v| ((gx + u * kf) * scale, (gy - v * kf) * scale));
            }
        }
        path
    }

    /// How far below the middle of a box the baseline goes so the words sit in its middle.
    pub fn drop(&self, grow: f64) -> f64 {
        (self.font.ascent + self.font.descent) / 2.0 * self.size * grow / self.font.units * 0.92
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
        canvas.fill_faded(&label.path(x0, y0, grow, scale), WORDS, a);
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
    canvas.fill_faded(&label.path(tx, cy + label.drop(1.0), 1.0, scale), ink, alpha);
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
}
