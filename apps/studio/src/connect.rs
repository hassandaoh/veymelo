//! The card that connects an AI to the app, on paper with a hairline edge: "Connect an AI",
//! then three tabs (Claude Code, Claude Desktop, Other apps). For the two Claudes, a "Set up
//! for me" button (the window asks first, then [`crate::setup`] does it) and how that app
//! stands; under it, the line to run or add by hand, by the tool's full path, with Copy. Other
//! apps get the command alone.
//!
//! It hangs under the AI's status at the top of the projects (with a point toward it), or lies
//! in the page: in the middle of the projects while there are none and no AI is connected, and
//! in the steps' column of a project while none is.

use veymelo_font::Font;
use veymelo_path::Path;
use veymelo_raster::{Canvas, Rgb};

use crate::button::{self, Icon, Label, Rect};
use crate::motion::{settle, smooth};
use crate::setup::{self, App, Standing};

const PAPER: Rgb = Rgb(255, 255, 255);
const INK: Rgb = Rgb(10, 10, 10);
const GREY: Rgb = Rgb(118, 118, 122);
const FAINT: Rgb = Rgb(168, 168, 174);
const RED: Rgb = Rgb(196, 43, 43);
const DONE: Rgb = Rgb(36, 150, 84);
const EDGE: Rgb = Rgb(161, 161, 170);
const RULE: Rgb = Rgb(228, 228, 232);
const WELL: Rgb = Rgb(249, 249, 250);

/// The room inside the card's edge, and the size of the point toward the status (points).
const PAD: f64 = 18.0;
const POINT: f64 = 7.0;
const TABS: [&str; 3] = ["Claude Code", "Claude Desktop", "Other apps"];

/// The faces the card writes in.
struct Faces {
    title: Font,
    body: Font,
    tab: Font,
    strong: Font,
    mono: Font,
}

fn faces() -> Option<&'static Faces> {
    static FACES: std::sync::OnceLock<Option<Faces>> = std::sync::OnceLock::new();
    FACES
        .get_or_init(|| {
            Some(Faces {
                title: crate::faces::sans_at(620.0, 14.0)?,
                body: crate::faces::sans_at(430.0, 12.0)?,
                tab: crate::faces::sans_at(540.0, 12.5)?,
                strong: crate::faces::sans_at(560.0, 12.0)?,
                mono: crate::faces::mono()?.at(&[("wght", 430.0)]),
            })
        })
        .as_ref()
}

/// Where the card is: hanging under a point at (x, y) with its right edge at `right`, or lying
/// in the page with its top left at (x, y); either `wide` points wide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    Under { point: (f64, f64), right: f64, wide: f64 },
    Lying { at: (f64, f64), wide: f64 },
}

/// What a press on the card did.
#[derive(Clone, Debug, PartialEq)]
pub enum Press {
    /// It was not on the card.
    Away,
    /// On the card, nothing more.
    On,
    /// Set this app up (once the person has said yes).
    SetUp(App),
    /// Put these words on the clipboard.
    Copy(String),
}

/// Where the card's parts are (points).
struct Parts {
    card: Rect,
    tabs: [Rect; 3],
    set_up: Option<Rect>,
    copy: Rect,
    /// What it says under the button (words, colour), the line over the well, the well and
    /// the lines in it, each with its baseline.
    said: Vec<(String, f64, Rgb)>,
    over: (String, f64),
    well: Rect,
    lines: Vec<(String, f64)>,
}

/// The words in `text` broken into lines no wider than `wide` points in `font` at `size`: at
/// spaces, and in a word too long for a line after a `/` or anywhere.
fn wrap(font: &Font, size: f64, text: &str, wide: f64) -> Vec<String> {
    let fits = |s: &str| Label::new(font.clone(), size, s).wide <= wide;
    let mut lines: Vec<String> = vec![];
    let mut now = String::new();
    for word in text.split(' ') {
        let joined = if now.is_empty() { word.to_string() } else { format!("{now} {word}") };
        if fits(&joined) {
            now = joined;
            continue;
        }
        if !now.is_empty() {
            lines.push(std::mem::take(&mut now));
        }
        // a word too long for a line by itself: broken after a slash where it can be
        let mut rest = word.to_string();
        while !fits(&rest) {
            let chars: Vec<char> = rest.chars().collect();
            let mut cut = chars.len();
            while cut > 1 && !fits(&chars[..cut].iter().collect::<String>()) {
                cut -= 1;
            }
            if let Some(slash) = chars[..cut].iter().rposition(|&c| c == '/').filter(|&s| s > 0) {
                cut = slash + 1;
            }
            lines.push(chars[..cut].iter().collect());
            rest = chars[cut..].iter().collect();
        }
        now = rest;
    }
    if !now.is_empty() {
        lines.push(now);
    }
    lines
}

/// A check mark of `size` points with its middle at (x, y).
fn check(canvas: &mut Canvas, scale: f64, (x, y): (f64, f64), size: f64, ink: Rgb, alpha: f32) {
    let k = size / 10.0;
    let pts = [(x - 4.5 * k, y + 0.2 * k), (x - 1.4 * k, y + 3.2 * k), (x + 4.5 * k, y - 3.4 * k)].map(|(u, v)| (u * scale, v * scale));
    canvas.stroke(&pts, false, 1.3 * scale, ink, alpha);
}

pub struct Connect {
    /// Whether it is shown (one lying in the page is shown while its owner places it there).
    pub open: bool,
    tab: usize,
    place: Place,
    /// When it opened, where the pointer is, when the line was copied (and from which tab).
    since: f64,
    pointer: Option<(f64, f64)>,
    copied: Option<(usize, f64)>,
    /// How Claude Code and Claude Desktop stand, as last read (and when it was read).
    standing: [Standing; 2],
    read: Option<std::time::Instant>,
    /// When the last setting up this card asked for began (what it said after shows here).
    asked: Option<std::time::Instant>,
}

impl Connect {
    pub const fn new() -> Connect {
        Connect { open: false, tab: 0, place: Place::Lying { at: (0.0, 0.0), wide: 400.0 }, since: 0.0, pointer: None, copied: None, standing: [Standing::No, Standing::No], read: None, asked: None }
    }

    /// Opens it at `t` (or keeps it open), placed at `place`.
    pub fn show(&mut self, place: Place, t: f64) {
        if !self.open {
            self.since = t;
            self.read = None;
            self.asked = None;
        }
        self.open = true;
        self.place = place;
    }

    /// Moves it to `place` (the page changed size).
    pub fn put(&mut self, place: Place) {
        self.place = place;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn point(&mut self, p: Option<(f64, f64)>) {
        self.pointer = p;
    }

    /// How the apps stand, read again now and then (a setting up may have changed them).
    fn refresh(&mut self) {
        let done = setup::last().is_some_and(|l| l.2 < 1.0);
        if self.read.is_none_or(|r| r.elapsed().as_secs_f64() > 4.0) || done {
            self.standing = setup::standing();
            self.read = Some(std::time::Instant::now());
        }
    }

    /// The setting up asked for from this card: going on now, or how it went.
    fn asked(&self, app: App) -> Option<Option<Result<String, String>>> {
        let asked = self.asked?;
        if setup::busy() == Some(app) {
            return Some(None);
        }
        let (which, said, ago) = setup::last()?;
        (which == app && ago < asked.elapsed().as_secs_f64()).then_some(Some(said))
    }

    /// The person said yes to setting `app` up: it starts, and how it goes shows here.
    pub fn started(&mut self) {
        self.asked = Some(std::time::Instant::now());
    }

    /// The command's lines for the tab shown, and what is copied.
    fn line(&self) -> String {
        match self.tab {
            0 => setup::code_line(),
            1 => setup::desktop_snippet(),
            _ => setup::command(),
        }
    }

    fn parts(&self) -> Option<Parts> {
        let f = faces()?;
        let (x, y, w) = match self.place {
            Place::Under { point, right, wide } => (right - wide, point.1 + POINT, wide),
            Place::Lying { at, wide } => (at.0, at.1, wide),
        };
        let inner = w - 2.0 * PAD;
        let mut tabs = [(0.0, 0.0, 0.0, 0.0); 3];
        let mut tx = x + PAD;
        for (i, name) in TABS.iter().enumerate() {
            let l = Label::new(f.tab.clone(), 12.5, name);
            tabs[i] = (tx - 6.0, y + 62.0, l.wide + 12.0, 30.0);
            tx += l.wide + 20.0;
        }
        let mut top = y + 108.0;
        let mut said = vec![];
        let mut set_up = None;
        let mut say = |words: &str, ink: Rgb, top: &mut f64| {
            for l in wrap(&f.body, 12.0, words, inner) {
                said.push((l, *top + 12.0, ink));
                *top += 17.0;
            }
        };
        let app = [App::Code, App::Desktop].get(self.tab).copied();
        if let Some(app) = app {
            set_up = Some((x + PAD, top, button::line_wide(&Label::new(f.strong.clone(), 12.0, "Set up for me"), Icon::None).max(118.0), 30.0));
            top += 30.0 + 14.0;
            match self.asked(app) {
                Some(Some(Ok(words))) => say(&words, DONE, &mut top),
                Some(Some(Err(why))) => say(&why, RED, &mut top),
                _ => say(
                    match app {
                        App::Code => "Adds Veymelo to Claude Code for all your projects.",
                        App::Desktop => "Adds Veymelo to Claude Desktop's settings, keeping a copy of them. Then quit Claude Desktop and open it again.",
                    },
                    GREY,
                    &mut top,
                ),
            }
            top += 10.0;
        } else {
            say("For any AI app that takes MCP servers: add this command as one.", GREY, &mut top);
            top += 10.0;
        }
        let over = match self.tab {
            0 => "Or run this in a terminal:",
            1 => "Or add this under \"mcpServers\" by hand:",
            _ => "The command:",
        };
        let copy = (x + w - PAD - 58.0, top - 4.0, 58.0, 24.0);
        let over = (over.to_string(), top + 12.0);
        top += 24.0;
        let lines: Vec<String> = wrap(&f.mono, 11.0, &self.line(), inner - 24.0);
        let well = (x + PAD, top, inner, lines.len() as f64 * 16.0 + 16.0);
        let lines = lines.into_iter().enumerate().map(|(i, l)| (l, well.1 + 20.0 + 16.0 * i as f64)).collect();
        let card = (x, y, w, well.1 + well.3 + PAD - y);
        Some(Parts { card, tabs, set_up, copy, said, over, well, lines })
    }

    /// The card's room on the page (points), while it is shown.
    pub fn rect(&self) -> Option<Rect> {
        self.open.then(|| self.parts().map(|p| p.card)).flatten()
    }

    /// Whether `p` is on the card.
    pub fn over(&self, p: (f64, f64)) -> bool {
        self.rect().is_some_and(|r| button::inside(r, p))
    }

    /// Whether `p` is on something to press.
    pub fn hand(&self, p: (f64, f64)) -> bool {
        let Some(parts) = self.parts().filter(|_| self.open) else { return false };
        parts.tabs.iter().any(|r| button::inside(*r, p)) || parts.set_up.is_some_and(|r| button::inside(r, p)) || button::inside(parts.copy, p)
    }

    /// A press at `p` at `t`.
    pub fn press(&mut self, p: (f64, f64), t: f64) -> Press {
        let Some(parts) = self.parts().filter(|_| self.open) else { return Press::Away };
        if !button::inside(parts.card, p) {
            return Press::Away;
        }
        if let Some(i) = parts.tabs.iter().position(|r| button::inside(*r, p)) {
            self.tab = i;
            return Press::On;
        }
        if let Some(r) = parts.set_up
            && button::inside(r, p)
            && setup::busy().is_none()
        {
            return Press::SetUp([App::Code, App::Desktop][self.tab]);
        }
        if button::inside(parts.copy, p) {
            self.copied = Some((self.tab, t));
            return Press::Copy(self.line());
        }
        Press::On
    }

    /// Whether it is still moving at `t` (coming in, a copy just made, a setting up going on).
    pub fn busy(&self, t: f64) -> bool {
        self.open && (t - self.since < 0.4 || self.copied.is_some_and(|c| t - c.1 < 2.0) || setup::busy().is_some() || setup::last().is_some_and(|l| l.2 < 1.0))
    }

    /// Draws it at `t`, `alpha` opaque.
    pub fn paint(&mut self, canvas: &mut Canvas, scale: f64, t: f64, alpha: f32) {
        if !self.open {
            return;
        }
        self.refresh();
        let (Some(f), Some(parts)) = (faces(), self.parts()) else { return };
        let px = |v: f64| v * scale;
        let lying = matches!(self.place, Place::Lying { .. });
        let come = if lying { 1.0 } else { smooth((t - self.since) / 0.18) };
        let a = alpha * come as f32;
        if a <= 0.0 {
            return;
        }
        let rise = if lying { 0.0 } else { (1.0 - settle((t - self.since) / 0.3)) * 6.0 };
        let (x, y, w, h) = parts.card;
        let (y, dy) = (y + rise, rise);
        canvas.shadow(px(x), px(y + 6.0), px(w), px(h), px(14.0), px(20.0), INK, a * if lying { 0.06 } else { 0.12 });
        canvas.fill_faded(&Path::rounded(px(x), px(y), px(w), px(h), px(14.0)), PAPER, a);
        canvas.stroke(&Path::rounded_points(px(x), px(y), px(w), px(h), px(14.0)), true, crate::iso::STROKE * scale, EDGE, a);
        if let Place::Under { point, .. } = self.place {
            // the point toward the status: paper over the edge, its two sides in hairlines
            let (cx, top) = (point.0, y);
            let mut fill = Path::default();
            fill.move_to(px(cx - POINT - 1.0), px(top + 1.0));
            fill.line_to(px(cx), px(top - POINT));
            fill.line_to(px(cx + POINT + 1.0), px(top + 1.0));
            fill.close();
            canvas.fill_faded(&fill, PAPER, a);
            canvas.stroke(&[(px(cx - POINT), px(top)), (px(cx), px(top - POINT)), (px(cx + POINT), px(top))], false, crate::iso::STROKE * scale, EDGE, a);
        }
        Label::new(f.title.clone(), 14.0, "Connect an AI").draw(canvas, x + PAD, y + 30.0, 1.0, scale, INK, a);
        Label::new(f.body.clone(), 12.0, "Let an AI app make videos with you in Veymelo.").draw(canvas, x + PAD, y + 48.0, 1.0, scale, GREY, a);
        // the tabs: the one shown in ink, underlined, on a hairline across
        canvas.fill_faded(&Path::rounded(px(x + PAD), px(y + 88.5), px(w - 2.0 * PAD), px(crate::iso::STROKE), 0.0), RULE, a);
        for (i, name) in TABS.iter().enumerate() {
            let (rx, _, rw, _) = parts.tabs[i];
            let l = Label::new(f.tab.clone(), 12.5, name);
            let near = self.pointer.is_some_and(|p| button::inside(parts.tabs[i], p));
            let ink = if i == self.tab { INK } else if near { GREY.mix(INK, 0.5) } else { GREY };
            l.draw(canvas, rx + 6.0, y + 81.0, 1.0, scale, ink, a);
            if i == self.tab {
                canvas.fill_faded(&Path::rounded(px(rx + 6.0), px(y + 87.6), px(rw - 12.0), px(1.4), 0.0), INK, a);
            }
        }
        // set up for me, and how the app stands beside it
        if let Some((bx, by, bw, bh)) = parts.set_up {
            let app = [App::Code, App::Desktop][self.tab];
            let going = setup::busy() == Some(app);
            let near = self.pointer.is_some_and(|p| button::inside((bx, by, bw, bh), p)) && !going;
            let words = Label::new(f.strong.clone(), 12.0, if going { "Setting up…" } else { "Set up for me" });
            button::line_button(canvas, scale, (bx, by + dy, bw, bh), &words, Icon::None, if near { 1.0 } else { 0.0 }, true, a);
            if going {
                crate::workspace::pen_dot(canvas, scale, t, (bx + 12.0, by + dy + bh / 2.0), 6.0, a);
            }
            let (word, ink, mark) = match &self.standing[self.tab] {
                Standing::Here => ("Set up", GREY, true),
                Standing::Elsewhere(_) => ("Set up for another copy of Veymelo", FAINT, false),
                Standing::No => ("", GREY, false),
            };
            if !word.is_empty() {
                let mut sx = bx + bw + 14.0;
                if mark {
                    check(canvas, scale, (sx + 5.0, by + dy + bh / 2.0), 10.0, DONE, a);
                    sx += 15.0;
                }
                let l = crate::workspace::fit(&f.body, 12.0, word, x + w - PAD - sx);
                l.draw(canvas, sx, by + dy + bh / 2.0 + 4.0, 1.0, scale, ink, a);
            }
        }
        for (line, base, ink) in &parts.said {
            Label::new(f.body.clone(), 12.0, line).draw(canvas, x + PAD, base + dy, 1.0, scale, *ink, a);
        }
        Label::new(f.body.clone(), 12.0, &parts.over.0).draw(canvas, x + PAD, parts.over.1 + dy, 1.0, scale, GREY, a);
        // Copy, and the line in its well
        let (cx, cy, cw, ch) = parts.copy;
        let fresh = self.copied.is_some_and(|c| c.0 == self.tab && t - c.1 < 1.8);
        let near = self.pointer.is_some_and(|p| button::inside(parts.copy, p));
        let word = Label::new(f.strong.clone(), 12.0, if fresh { "Copied" } else { "Copy" });
        button::line_button(canvas, scale, (cx, cy + dy, cw, ch), &word, Icon::None, if near { 1.0 } else { 0.0 }, false, a);
        let (wx, wy, ww, wh) = parts.well;
        canvas.fill_faded(&Path::rounded(px(wx), px(wy + dy), px(ww), px(wh), px(8.0)), WELL, a);
        canvas.stroke(&Path::rounded_points(px(wx), px(wy + dy), px(ww), px(wh), px(8.0)), true, crate::iso::STROKE * scale, RULE, a);
        for (line, base) in &parts.lines {
            Label::new(f.mono.clone(), 11.0, line).draw(canvas, wx + 12.0, base + dy, 1.0, scale, INK, a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_break_at_spaces_then_after_slashes() {
        let Some(f) = faces() else { return };
        let lines = wrap(&f.mono, 11.0, "claude mcp add --scope user veymelo -- /Applications/Veymelo.app/Contents/MacOS/veymelo mcp --bridge", 260.0);
        assert!(lines.len() >= 2, "{lines:?}");
        assert!(lines.iter().all(|l| Label::new(f.mono.clone(), 11.0, l).wide <= 260.0), "{lines:?}");
        assert_eq!(lines.join(" ").replace("/ ", "/"), "claude mcp add --scope user veymelo -- /Applications/Veymelo.app/Contents/MacOS/veymelo mcp --bridge");
    }

    #[test]
    fn a_press_picks_a_tab_asks_to_set_up_or_copies() {
        let Some(_) = faces() else { return };
        let mut c = Connect::new();
        c.show(Place::Lying { at: (100.0, 100.0), wide: 400.0 }, 0.0);
        let parts = c.parts().unwrap();
        let mid = |r: Rect| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        assert_eq!(c.press(mid(parts.set_up.unwrap()), 1.0), Press::SetUp(App::Code));
        assert_eq!(c.press(mid(parts.tabs[1]), 1.0), Press::On);
        assert_eq!(c.press(mid(c.parts().unwrap().set_up.unwrap()), 1.0), Press::SetUp(App::Desktop));
        c.press(mid(parts.tabs[2]), 1.0);
        let parts = c.parts().unwrap();
        assert!(parts.set_up.is_none());
        assert_eq!(c.press(mid(parts.copy), 1.0), Press::Copy(setup::command()));
        assert_eq!(c.press((10.0, 10.0), 1.0), Press::Away);
    }
}
