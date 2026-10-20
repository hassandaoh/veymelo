//! Text laid out: each character's look (the document's, or its run's), its font found (the
//! look's, or for characters it lacks, a font of their script), capitals and small figures made,
//! each paragraph's directions worked out (right-to-left and left-to-right writing mixed, by the
//! bidirectional algorithm), runs of one font, size and direction shaped, paragraphs broken into
//! lines (wrapped in the box for paragraph text, the evenest breaks for the whole paragraph or
//! each line as full as it goes), lines set left, centred, right or justified within their
//! indents, one leading apart and paragraphs spaced, underlines and lines through drawn under
//! them. Vertical text goes down in columns, the columns right to left.
//!
//! Positions are in the layer's pixels, y down; a glyph's position is its origin on the baseline.

pub mod bidi;
pub mod breaks;

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock};

use veymelo_font::shape::{Options, shape_with};
use veymelo_font::{Font, system};
use veymelo_keyframe::{Caps, CharStyle, Composer, Direction, Justify, Kerning, ParaStyle, Position, TextDoc};

/// A font, shared.
pub type FontRef = Arc<Font>;

/// Fonts opened, by what was asked.
type Cache = Mutex<HashMap<String, Option<FontRef>>>;

/// The font a name stands for: a font file's path, or an installed family, full or PostScript
/// name, at `weight` (a variable font is set to it exactly).
pub fn resolve(name: &str, weight: f64, italic: bool) -> Option<FontRef> {
    resolve_axes(name, weight, italic, &[])
}

/// As [`resolve`], a variable font's other axes set too (those it has).
pub fn resolve_axes(name: &str, weight: f64, italic: bool, axes: &[(String, f64)]) -> Option<FontRef> {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let key = format!("{name}|{}|{italic}|{axes:?}", weight.to_bits());
    let cache = CACHE.get_or_init(Default::default);
    if let Some(f) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return f.clone();
    }
    let lower = name.to_lowercase();
    let file = name.contains('/') || lower.ends_with(".ttf") || lower.ends_with(".ttc") || lower.ends_with(".otf");
    let font = if file {
        Font::open(name).ok()
    } else {
        system::find(name, weight.round().clamp(1.0, 1000.0) as u16, italic).and_then(|f| Font::open_face(&f.path, f.index).ok())
    };
    let font = font.map(|f| {
        let has: Vec<String> = f.axes().into_iter().map(|a| a.0).collect();
        let mut settings: Vec<(&str, f64)> = Vec::new();
        if has.iter().any(|t| t == "wght") {
            settings.push(("wght", weight));
        }
        for (t, v) in axes {
            if has.iter().any(|h| h == t) {
                settings.push((t.as_str(), *v));
            }
        }
        if settings.is_empty() { Arc::new(f) } else { Arc::new(f.at(&settings)) }
    });
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, font.clone());
    font
}

/// A style's axes with the optical size set to the size of the text, as the system sets it,
/// unless the style sets it itself (to half a point: closer is not seen, and fewer instances
/// are kept).
fn sized(axes: &[(String, f64)], size: f64) -> Vec<(String, f64)> {
    let mut out = axes.to_vec();
    if !out.iter().any(|(t, _)| t == "opsz") {
        out.push(("opsz".into(), (size * 2.0).round() / 2.0));
    }
    out
}

/// A glyph laid out.
#[derive(Clone)]
pub struct Placed {
    pub font: FontRef,
    pub glyph: u16,
    /// Its origin, on the baseline (layer pixels, y down).
    pub x: f64,
    pub y: f64,
    /// Pixels to a font unit, up and across (they differ when the look stretches the letters).
    pub scale: f64,
    pub scale_x: f64,
    /// How far it moves the pen (pixels, tracking included).
    pub advance: f64,
    /// The character it came from (an index into the document's words), and that character's
    /// word and line.
    pub char_index: usize,
    pub word: usize,
    pub line: usize,
    /// Its look: an index into [`Layout::styles`].
    pub style: usize,
    /// Set right to left.
    pub rtl: bool,
    /// Turned a quarter turn clockwise (a letter that lies on its side in vertical text).
    pub sideways: bool,
    /// Slanted by drawing (the look's faux italic).
    pub slant: bool,
}

impl Placed {
    /// Where a point of the glyph (font units, y up) goes in the layer.
    pub fn map(&self, fx: f64, fy: f64) -> (f64, f64) {
        let (mut x, y) = (fx * self.scale_x, -fy * self.scale);
        if self.slant {
            // twelve degrees
            x -= y * 0.212_556_56;
        }
        if self.sideways { (self.x - y, self.y + x) } else { (self.x + x, self.y + y) }
    }
}

impl std::fmt::Debug for Placed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Placed {{ glyph: {}, x: {:.2}, y: {:.2}, char: {}, word: {}, line: {} }}", self.glyph, self.x, self.y, self.char_index, self.word, self.line)
    }
}

/// A line of text (a column, in vertical text).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Its baseline, its left end and its width (spaces at its end left out). In vertical text:
    /// its middle across, its top, and its length down.
    pub y: f64,
    pub x: f64,
    pub width: f64,
    /// Its characters (indices into the document's words), and its paragraph.
    pub chars: Range<usize>,
    pub paragraph: usize,
    /// How far its letters reach above and below the baseline.
    pub ascent: f64,
    pub descent: f64,
}

/// An underline or a line through letters.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoration {
    pub x0: f64,
    pub x1: f64,
    /// Its middle, and how thick it is.
    pub y: f64,
    pub thickness: f64,
    /// The look of the letters it is under (their colour).
    pub style: usize,
    pub line: usize,
}

/// A document laid out.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub glyphs: Vec<Placed>,
    pub lines: Vec<Line>,
    /// The looks the glyphs have.
    pub styles: Vec<CharStyle>,
    pub decorations: Vec<Decoration>,
    /// How many characters, words and lines there are.
    pub chars: usize,
    pub words: usize,
    /// Characters no font could show.
    pub missing: Vec<char>,
    /// A look's font could not be found.
    pub no_font: bool,
}

fn is_space(c: char) -> bool {
    c == ' ' || c == '\u{3000}' || c == '\t' || c == '\u{A0}'
}

/// Punctuation that may hang past the end of a line, and past its start.
fn hangs_after(c: char) -> bool {
    ".,;:!?、。’”»›)]}-–—".contains(c)
}

fn hangs_before(c: char) -> bool {
    "‘“«‹([{\"'".contains(c)
}

/// Upright in vertical text (CJK and full-width forms); the rest lie on their side.
fn upright(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x2E80..=0x2FFF | 0x3000..=0x303F | 0x3040..=0x30FF | 0x3100..=0x31FF | 0x3200..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// A character as it is drawn: its glyph source and size.
#[derive(Clone)]
struct Ch {
    /// The character drawn (capitals made).
    c: char,
    style: usize,
    font: FontRef,
    /// Its size in pixels (small capitals and small figures smaller).
    size: f64,
    /// Raised (above zero) from the baseline, in pixels.
    rise: f64,
}

/// What a run is shaped with, and what it is set with.
#[derive(Clone, PartialEq)]
struct Key {
    font: *const Font,
    size: u64,
    options: Options,
    level: u8,
    style: usize,
}

/// One glyph of a line before it is placed.
struct Glyph {
    font: FontRef,
    id: u16,
    dx: f64,
    dy: f64,
    advance: f64,
    scale: f64,
    scale_x: f64,
    /// The character (an index into the document's words).
    ch: usize,
    rtl: bool,
}

/// What to shape a style with.
fn options_of(s: &CharStyle, rtl: bool) -> Options {
    let mut features: Vec<([u8; 4], bool)> = s.features.iter().filter_map(|f| f.as_bytes().try_into().ok().map(|t: [u8; 4]| (t, true))).collect();
    if !s.ligatures {
        features.push((*b"liga", false));
        features.push((*b"clig", false));
    }
    Options { features, no_kerning: s.kerning != Kerning::Metrics, rtl: Some(rtl), size: None }
}

/// Glyph `g`'s side bearings in font units: (left, right).
fn bearings(font: &Font, g: u16) -> Option<(f64, f64)> {
    let b = font.bounds(g)?;
    Some((b[0], font.advance(g) - b[2]))
}

/// The layout of one paragraph's characters before lines are made: each character's width
/// (with its spacing), the bidirectional levels, and where lines may break.
struct Measured {
    widths: Vec<f64>,
    levels: bidi::Levels,
    breaks: Vec<usize>,
}

struct Ctx<'a> {
    chars: &'a [Ch],
    styles: &'a [CharStyle],
}

impl Ctx<'_> {
    /// The room after character `i` (its tracking) and before it (kerning set by hand).
    fn after(&self, i: usize) -> f64 {
        let ch = &self.chars[i];
        self.styles[ch.style].tracking / 1000.0 * ch.size
    }

    fn before(&self, i: usize) -> f64 {
        let ch = &self.chars[i];
        match self.styles[ch.style].kerning {
            Kerning::Manual(v) => v / 1000.0 * ch.size,
            _ => 0.0,
        }
    }

    /// Kerning by the shapes of the letters between characters `a` and `b` (side by side).
    fn optical(&self, a: usize, b: usize, ga: &Glyph, gb: &Glyph) -> f64 {
        let (ca, cb) = (&self.chars[a], &self.chars[b]);
        if self.styles[cb.style].kerning != Kerning::Optical || is_space(ca.c) || is_space(cb.c) {
            return 0.0;
        }
        let (Some((_, ra)), Some((lb, _))) = (bearings(&ga.font, ga.id), bearings(&gb.font, gb.id)) else { return 0.0 };
        let gap = ra * ga.scale_x + lb * gb.scale_x;
        let target = 0.07 * cb.size;
        (target - gap).clamp(-0.2 * cb.size, 0.2 * cb.size)
    }

    /// Characters `range` of the paragraph cut into runs of one font, size, look and direction,
    /// each shaped: the runs in the order they are seen, left to right, each its glyphs in the
    /// order they are seen.
    fn shape(&self, range: Range<usize>, levels: &[u8]) -> Vec<Vec<Glyph>> {
        let mut runs: Vec<(Key, Range<usize>)> = Vec::new();
        for i in range.clone() {
            let ch = &self.chars[i];
            let level = levels[i - range.start];
            let s = &self.styles[ch.style];
            let mut options = options_of(s, level % 2 == 1);
            options.size = Some(ch.size);
            let key = Key { font: Arc::as_ptr(&ch.font), size: ch.size.to_bits(), options, level, style: ch.style };
            match runs.last_mut() {
                Some((k, r)) if *k == key => r.end = i + 1,
                _ => runs.push((key, i..i + 1)),
            }
        }
        let run_levels: Vec<u8> = runs.iter().map(|(k, _)| k.level).collect();
        let order = bidi::visual_order(&run_levels);
        let mut out = Vec::new();
        for k in order {
            let (key, r) = &runs[k];
            let ch = &self.chars[r.start];
            let s = &self.styles[ch.style];
            let rtl = key.level % 2 == 1;
            let text: Vec<char> = r.clone().map(|i| if rtl { bidi::mirror(self.chars[i].c) } else { self.chars[i].c }).collect();
            let scale = ch.size / ch.font.units * s.v_scale / 100.0;
            let scale_x = ch.size / ch.font.units * s.h_scale / 100.0;
            let glyphs = shape_with(&ch.font, &text, &key.options);
            let mut run = Vec::with_capacity(glyphs.len());
            for g in glyphs {
                let mut advance = g.advance * scale_x;
                let mut dx = g.dx * scale_x;
                if s.tsume > 0.0
                    && g.advance > 0.0
                    && let Some((l, rb)) = bearings(&ch.font, g.id)
                {
                    let k = s.tsume / 100.0;
                    advance -= (l + rb).max(0.0) * k * scale_x;
                    dx -= l.max(0.0) * k * scale_x;
                }
                run.push(Glyph { font: ch.font.clone(), id: g.id, dx, dy: g.dy * scale, advance, scale, scale_x, ch: r.start + g.cluster, rtl });
            }
            out.push(run);
        }
        out
    }

    /// Lay the glyphs of runs side by side, the room between characters added: each glyph's
    /// x from the line's start, and the line's width.
    fn place(&self, runs: &[Vec<Glyph>]) -> (Vec<f64>, f64) {
        let mut xs = Vec::new();
        let mut pen = 0.0;
        let mut prev: Option<(usize, usize, usize)> = None;
        for (r, run) in runs.iter().enumerate() {
            for (k, g) in run.iter().enumerate() {
                if let Some((pc, pr, pk)) = prev
                    && pc != g.ch
                {
                    // between two characters: the earlier one's tracking and the later one's
                    // kerning, whichever side each is on
                    let (early, late) = if pc < g.ch { (pc, g.ch) } else { (g.ch, pc) };
                    pen += self.after(early) + self.before(late) + self.optical(pc, g.ch, &runs[pr][pk], g);
                }
                xs.push(pen + g.dx);
                pen += g.advance;
                prev = Some((g.ch, r, k));
            }
        }
        (xs, pen)
    }
}

/// The lines a paragraph of `n` characters breaks into: as full as each goes (`single`), or the
/// evenest for the whole paragraph. `room(k)` is the width line `k` has.
fn compose(widths: &[f64], spaces: &[bool], breaks: &[usize], room: &dyn Fn(usize) -> f64, single: bool) -> Vec<Range<usize>> {
    let n = widths.len();
    if n == 0 {
        return std::iter::once(0..0).collect();
    }
    // the width of characters a..b, spaces at its end left out
    let mut pre = vec![0.0; n + 1];
    for i in 0..n {
        pre[i + 1] = pre[i] + widths[i];
    }
    let width = |a: usize, b: usize| {
        let mut e = b;
        while e > a && spaces[e - 1] {
            e -= 1;
        }
        pre[e] - pre[a]
    };
    let mut points: Vec<usize> = breaks.iter().copied().filter(|&b| b > 0 && b < n).collect();
    points.push(n);
    points.dedup();
    if single {
        let mut out = Vec::new();
        let mut start = 0;
        let mut last_ok: Option<usize> = None;
        let mut k = 0;
        for &p in &points {
            if width(start, p) <= room(k) + 1e-9 {
                last_ok = Some(p);
                continue;
            }
            match last_ok {
                Some(b) if b > start => {
                    out.push(start..b);
                    start = b;
                    k += 1;
                    last_ok = (width(start, p) <= room(k) + 1e-9).then_some(p);
                    if last_ok.is_none() {
                        // still too wide on a line of its own: it stands alone
                        out.push(start..p);
                        start = p;
                        k += 1;
                    }
                }
                _ => {
                    out.push(start..p);
                    start = p;
                    k += 1;
                    last_ok = None;
                }
            }
        }
        if start < n {
            out.push(start..n);
        }
        return out;
    }
    // the evenest: the least sum of how loose each line is (squared), the last line free
    let mut nodes = vec![0usize];
    nodes.extend(points.iter().copied());
    let m = nodes.len();
    let mut best: Vec<(f64, usize, usize)> = vec![(f64::INFINITY, 0, 0); m];
    best[0] = (0.0, 0, 0);
    for j in 1..m {
        for i in (0..j).rev() {
            if best[i].0.is_infinite() {
                continue;
            }
            let line = best[i].2;
            let r = room(line).max(1e-9);
            let w = width(nodes[i], nodes[j]);
            let cost = if w > r + 1e-9 {
                // too wide: only when nothing shorter fits
                if j == i + 1 { 1e9 + (w - r) } else { break }
            } else if j == m - 1 {
                0.0
            } else {
                let slack = (r - w) / r;
                slack * slack * 100.0 + 1.0
            };
            let total = best[i].0 + cost;
            if total < best[j].0 {
                best[j] = (total, i, line + 1);
            }
        }
    }
    let mut cuts = vec![m - 1];
    while *cuts.last().expect("not empty") != 0 {
        let at = *cuts.last().expect("not empty");
        cuts.push(best[at].1);
    }
    cuts.reverse();
    cuts.windows(2).map(|w| nodes[w[0]]..nodes[w[1]]).collect()
}

/// Lay out `doc`.
pub fn lay_out(doc: &TextDoc) -> Layout {
    let mut out = Layout::default();
    let text: Vec<char> = doc.text.chars().collect();
    let per_char = doc.styles();
    // the looks, each once
    let mut style_of = Vec::with_capacity(text.len());
    for s in &per_char {
        let k = match out.styles.iter().position(|x| x == s) {
            Some(k) => k,
            None => {
                out.styles.push(s.clone());
                out.styles.len() - 1
            }
        };
        style_of.push(k);
    }
    if out.styles.is_empty() {
        out.styles.push(doc.base_style());
    }
    // each look's font
    let mut primary: Vec<Option<FontRef>> = Vec::new();
    for s in &out.styles {
        let f = resolve_axes(&s.font, s.weight, s.italic, &sized(&s.axes, s.size)).or_else(|| {
            out.no_font = true;
            resolve("System Font", s.weight, s.italic).or_else(|| resolve("Helvetica", s.weight, s.italic))
        });
        primary.push(f);
    }
    if primary.iter().all(Option::is_none) {
        return out;
    }
    // each character as it is drawn
    let mut chars: Vec<Ch> = Vec::with_capacity(text.len());
    for (i, &c0) in text.iter().enumerate() {
        let st = style_of[i];
        let s = &out.styles[st];
        let font = primary[st].clone().or_else(|| primary.iter().flatten().next().cloned()).expect("a font");
        let (c, small) = match s.caps {
            Caps::Normal => (c0, false),
            Caps::AllCaps => (c0.to_uppercase().next().unwrap_or(c0), false),
            Caps::SmallCaps => (c0.to_uppercase().next().unwrap_or(c0), c0.is_lowercase()),
        };
        let (k, rise) = match s.position {
            Position::Normal => (1.0, 0.0),
            Position::Superscript => (0.583, 0.333 * s.size),
            Position::Subscript => (0.583, -0.333 * s.size),
        };
        let size = s.size * k * if small { 0.7 } else { 1.0 };
        // what joins or dresses an emoji (a joiner, a variation selector, a keycap, a skin tone,
        // a tag) goes in the font of what it follows, so the emoji font can join them; joiners
        // and non-joiners always do (they only steer how letters join, drawn as nothing)
        let steers = matches!(c, '\u{200C}' | '\u{200D}');
        let joins = steers || matches!(c as u32, 0xFE0E | 0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F);
        let wants_emoji = text.get(i + 1) == Some(&'\u{FE0F}');
        let emoji = || system::fallback_for('\u{1F600}').and_then(|f| resolve_axes(&f.full, s.weight, false, &sized(&[], s.size))).filter(|f| f.glyph(c) != 0);
        let font = if let (true, Some(prev)) = (joins, chars.last()) {
            if steers || prev.font.glyph(c) != 0 { prev.font.clone() } else { font }
        } else if let Some(e) = wants_emoji.then(emoji).flatten() {
            e
        } else if font.glyph(c) != 0 || is_space(c) || c.is_control() {
            font
        } else {
            match system::fallback_for(c).and_then(|f| resolve_axes(&f.full, s.weight, false, &sized(&[], s.size))).filter(|f| f.glyph(c) != 0) {
                Some(f) => f,
                None => {
                    out.missing.push(c);
                    font
                }
            }
        };
        chars.push(Ch { c, style: st, font, size, rise: rise + s.baseline_shift });
    }
    out.chars = chars.len();
    // words: runs of what is not space
    let mut word_of = vec![0usize; chars.len()];
    let mut in_word = false;
    for (i, ch) in chars.iter().enumerate() {
        let space = is_space(ch.c) || ch.c == '\n' || ch.c == '\r';
        if !space && !in_word {
            out.words += 1;
        }
        in_word = !space;
        word_of[i] = out.words.saturating_sub(1);
    }
    let styles = std::mem::take(&mut out.styles);
    let cx = Ctx { chars: &chars, styles: &styles };
    let paras = doc.para_styles();
    let ranges = doc.paragraph_ranges();
    if doc.vertical {
        lay_out_vertical(doc, &cx, &ranges, &paras, &word_of, &mut out);
    } else {
        lay_out_lines(doc, &cx, &ranges, &paras, &word_of, &mut out);
    }
    out.styles = styles;
    out.missing.sort();
    out.missing.dedup();
    out
}

/// A paragraph's characters measured.
fn measure(cx: &Ctx, range: Range<usize>, para: &ParaStyle) -> Measured {
    let body: Vec<char> = cx.chars[range.clone()].iter().map(|c| c.c).collect();
    let dir = match para.direction {
        Direction::Auto => bidi::Direction::Auto,
        Direction::Ltr => bidi::Direction::Ltr,
        Direction::Rtl => bidi::Direction::Rtl,
    };
    let levels = bidi::resolve(&body, dir);
    let mut widths = vec![0.0; body.len()];
    let runs = cx.shape(range.clone(), &levels.levels);
    for run in &runs {
        for g in run {
            widths[g.ch - range.start] += g.advance;
        }
    }
    for (k, w) in widths.iter_mut().enumerate() {
        *w += cx.after(range.start + k) + if k > 0 { cx.before(range.start + k) } else { 0.0 };
    }
    let breaks = breaks::opportunities(&body);
    Measured { widths, levels, breaks }
}

fn lay_out_lines(doc: &TextDoc, cx: &Ctx, ranges: &[Range<usize>], paras: &[ParaStyle], word_of: &[usize], out: &mut Layout) {
    let chars = cx.chars;
    let box_w = doc.box_size.map(|b| b[0]);
    let mut y = 0.0;
    let mut first_line = true;
    let mut line_no = 0;
    for (p, range) in ranges.iter().enumerate() {
        let para = &paras[p];
        // the line break that ends a paragraph is not laid out
        let end = if range.end > range.start && matches!(chars[range.end - 1].c, '\n' | '\r') { range.end - 1 } else { range.end };
        let body = range.start..end;
        let m = measure(cx, body.clone(), para);
        let spaces: Vec<bool> = body.clone().map(|i| is_space(chars[i].c)).collect();
        let room = |k: usize| match box_w {
            Some(w) => (w - para.indent_left - para.indent_right - if k == 0 { para.indent_first } else { 0.0 }).max(1.0),
            None => f64::INFINITY,
        };
        let lines = if box_w.is_some() { compose(&m.widths, &spaces, &m.breaks, &room, para.composer == Composer::SingleLine) } else { std::iter::once(0..body.len()).collect() };
        let rtl = m.levels.base == 1;
        for (k, l) in lines.iter().enumerate() {
            let a = body.start + l.start;
            let b = body.start + l.end;
            let levels = m.levels.line(l.clone());
            let runs = cx.shape(a..b, &levels);
            let (xs, _) = cx.place(&runs);
            // what the line holds, spaces at its end (in reading order) left out
            let flat: Vec<&Glyph> = runs.iter().flatten().collect();
            let mut trail = b;
            while trail > a && is_space(chars[trail - 1].c) {
                trail -= 1;
            }
            let shown = |g: &Glyph| g.ch < trail;
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for (g, &x) in flat.iter().zip(&xs) {
                if shown(g) {
                    lo = lo.min(x - g.dx);
                    hi = hi.max(x - g.dx + g.advance);
                }
            }
            if lo > hi {
                (lo, hi) = (0.0, 0.0);
            }
            // punctuation hanging past the edges
            if para.hanging && box_w.is_some() {
                let first_seen = flat.iter().zip(&xs).find(|(g, _)| shown(g));
                if let Some((g, &x)) = first_seen
                    && hangs_before(chars[g.ch].c)
                {
                    lo = x - g.dx + g.advance;
                }
                let last_seen = flat.iter().zip(&xs).rev().find(|(g, _)| shown(g));
                if let Some((g, &x)) = last_seen
                    && hangs_after(chars[g.ch].c)
                {
                    hi = x - g.dx;
                }
            }
            let width = hi - lo;
            let last = k + 1 == lines.len();
            let r = room(k);
            let indent = if k == 0 { para.indent_first } else { 0.0 };
            let (left, right) = if rtl { (para.indent_left, para.indent_right + indent) } else { (para.indent_left + indent, para.indent_right) };
            let stretch = matches!(para.justify, Justify::JustifyLeft | Justify::JustifyCenter | Justify::JustifyRight | Justify::JustifyAll) && box_w.is_some() && (!last || para.justify == Justify::JustifyAll);
            let fill_spaces = (a..trail).filter(|&i| is_space(chars[i].c)).count();
            let gap = if stretch && fill_spaces > 0 { ((r - width) / fill_spaces as f64).max(0.0) } else { 0.0 };
            let align = match para.justify {
                Justify::Left | Justify::JustifyLeft => 0,
                Justify::Center | Justify::JustifyCenter => 1,
                Justify::Right | Justify::JustifyRight => 2,
                Justify::JustifyAll => 0,
            };
            let x0 = match (box_w, align) {
                (Some(_), 0) => left,
                (Some(w), 1) => left + (w - left - right - width) / 2.0,
                (Some(w), _) => w - right - width,
                (None, 0) => left,
                (None, 1) => -width / 2.0,
                (None, _) => -width - right,
            };
            let shift = if stretch { left } else { x0 } - lo;
            // the line's height: its tallest letters and leading
            let (mut ascent, mut descent, mut leading) = (0.0f64, 0.0f64, 0.0f64);
            for i in a..b.max(a + 1).min(chars.len().max(1)) {
                let Some(ch) = chars.get(i) else { break };
                let s = &cx.styles[ch.style];
                let sc = ch.size / ch.font.units;
                ascent = ascent.max(ch.font.ascent * sc + ch.rise.max(0.0));
                descent = descent.max(-ch.font.descent * sc - ch.rise.min(0.0));
                leading = leading.max(s.leading.unwrap_or(s.size * 1.2));
            }
            if a == b {
                // an empty line still has its look's height
                let s = &cx.styles[chars.get(a.min(chars.len().saturating_sub(1))).map_or(0, |c| c.style)];
                leading = s.leading.unwrap_or(s.size * 1.2);
                ascent = s.size * 0.8;
                descent = s.size * 0.2;
            }
            if first_line {
                y = if doc.box_size.is_some() { ascent } else { 0.0 };
                first_line = false;
            } else {
                y += leading;
                if k == 0 {
                    y += paras[p - 1].space_after + para.space_before;
                }
            }
            let visible = doc.box_size.is_none_or(|bx| y + descent <= bx[1] + 1e-6 || line_no == 0);
            if visible {
                out.lines.push(Line { y, x: lo + shift, width, chars: a..b, paragraph: p, ascent, descent });
                let mut extra = 0.0;
                let mut prev_ch: Option<usize> = None;
                let mut deco: Vec<(usize, f64, f64)> = Vec::new();
                for (g, &x) in flat.iter().zip(&xs) {
                    let ci = g.ch;
                    if prev_ch.is_some_and(|pc| pc != ci && is_space(chars[pc].c) && pc < trail) {
                        extra += gap;
                    }
                    prev_ch = Some(ci);
                    let ch = &chars[ci];
                    let s = &cx.styles[ch.style];
                    let gx = x + shift + extra;
                    out.glyphs.push(Placed {
                        font: g.font.clone(),
                        glyph: g.id,
                        x: gx,
                        y: y - g.dy - ch.rise,
                        scale: g.scale,
                        scale_x: g.scale_x,
                        advance: g.advance,
                        char_index: ci,
                        word: word_of.get(ci).copied().unwrap_or(0),
                        line: line_no,
                        style: ch.style,
                        rtl: g.rtl,
                        sideways: false,
                        slant: s.faux_italic,
                    });
                    if (s.underline || s.strikethrough) && g.advance > 0.0 && ci < trail {
                        let (x0, x1) = (gx - g.dx, gx - g.dx + g.advance);
                        match deco.last_mut() {
                            Some(d) if d.0 == ch.style && (d.2 - x0).abs() < 1.0 => d.2 = x1,
                            _ => deco.push((ch.style, x0, x1)),
                        }
                    }
                }
                for (st, x0, x1) in deco {
                    let s = &cx.styles[st];
                    let font = &chars[a].font;
                    let sc = s.size / font.units;
                    if s.underline {
                        let (pos, thick) = font.underline();
                        out.decorations.push(Decoration { x0, x1, y: y - pos * sc, thickness: thick * sc, style: st, line: line_no });
                    }
                    if s.strikethrough {
                        let (pos, thick) = font.strikeout();
                        out.decorations.push(Decoration { x0, x1, y: y - pos * sc, thickness: thick * sc, style: st, line: line_no });
                    }
                }
            }
            line_no += 1;
        }
    }
}

/// Vertical text: each paragraph down in columns, the columns right to left; letters of CJK
/// upright, the others on their side.
fn lay_out_vertical(doc: &TextDoc, cx: &Ctx, ranges: &[Range<usize>], paras: &[ParaStyle], word_of: &[usize], out: &mut Layout) {
    let chars = cx.chars;
    let box_h = doc.box_size.map(|b| b[1]);
    let mut column_x: Option<f64> = None;
    let mut line_no = 0;
    for (p, range) in ranges.iter().enumerate() {
        let para = &paras[p];
        let end = if range.end > range.start && matches!(chars[range.end - 1].c, '\n' | '\r') { range.end - 1 } else { range.end };
        let body = range.start..end;
        // each character's length down the column
        let mut lengths = Vec::with_capacity(body.len());
        let mut shaped: Vec<Vec<Glyph>> = Vec::new();
        for i in body.clone() {
            let ch = &chars[i];
            let s = &cx.styles[ch.style];
            let up = upright(ch.c);
            let mut options = options_of(s, false);
            options.size = Some(ch.size);
            if up {
                options.features.push((*b"vert", true));
            }
            let glyphs = shape_with(&ch.font, &[ch.c], &options);
            let scale = ch.size / ch.font.units * s.v_scale / 100.0;
            let scale_x = ch.size / ch.font.units * s.h_scale / 100.0;
            let run: Vec<Glyph> = glyphs.iter().map(|g| Glyph { font: ch.font.clone(), id: g.id, dx: g.dx * scale_x, dy: g.dy * scale, advance: g.advance * scale_x, scale, scale_x, ch: i, rtl: false }).collect();
            let len = if up { ch.font.vertical_advance(run.first().map_or(0, |g| g.id)) * scale } else { run.iter().map(|g| g.advance).sum() };
            lengths.push(len + cx.after(i));
            shaped.push(run);
        }
        let spaces: Vec<bool> = body.clone().map(|i| is_space(chars[i].c)).collect();
        let breaks = breaks::opportunities(&body.clone().map(|i| chars[i].c).collect::<Vec<_>>());
        let room = |k: usize| match box_h {
            Some(h) => (h - para.indent_left - para.indent_right - if k == 0 { para.indent_first } else { 0.0 }).max(1.0),
            None => f64::INFINITY,
        };
        let columns = if box_h.is_some() { compose(&lengths, &spaces, &breaks, &room, para.composer == Composer::SingleLine) } else { std::iter::once(0..body.len()).collect() };
        for (k, c) in columns.iter().enumerate() {
            let (a, b) = (body.start + c.start, body.start + c.end);
            let leading = (a..b).map(|i| cx.styles[chars[i].style].leading.unwrap_or(cx.styles[chars[i].style].size * 1.2)).fold(cx.styles[chars.get(a).map_or(0, |c| c.style)].size * 1.2, f64::max);
            let x = match column_x {
                None => match doc.box_size {
                    Some(bx) => bx[0] - leading / 2.0,
                    None => 0.0,
                },
                Some(prev) => prev - leading - if k == 0 { paras[p.saturating_sub(1)].space_after + para.space_before } else { 0.0 },
            };
            column_x = Some(x);
            let length: f64 = lengths[c.clone()].iter().sum();
            let r = room(k);
            let top = para.indent_left + if k == 0 { para.indent_first } else { 0.0 } + match (box_h, para.justify) {
                (Some(_), Justify::Center | Justify::JustifyCenter) => (r - length) / 2.0,
                (Some(_), Justify::Right | Justify::JustifyRight) => r - length,
                _ => 0.0,
            };
            out.lines.push(Line { y: x, x: top, width: length, chars: a..b, paragraph: p, ascent: leading / 2.0, descent: leading / 2.0 });
            let mut pen = top;
            for i in a..b {
                let ch = &chars[i];
                let s = &cx.styles[ch.style];
                let up = upright(ch.c);
                for g in &shaped[i - body.start] {
                    let (gx, gy, sideways) = if up {
                        // upright: centred across the column, its top at the pen
                        let sc = g.scale;
                        (x - g.advance / 2.0 + g.dx, pen + ch.font.ascent * sc, false)
                    } else {
                        // on its side: its baseline down the column, its middle on the column's
                        let sc = g.scale;
                        let mid = (ch.font.ascent + ch.font.descent) / 2.0 * sc;
                        (x - mid, pen + g.dx, true)
                    };
                    out.glyphs.push(Placed {
                        font: g.font.clone(),
                        glyph: g.id,
                        x: gx,
                        y: gy,
                        scale: g.scale,
                        scale_x: g.scale_x,
                        advance: g.advance,
                        char_index: i,
                        word: word_of.get(i).copied().unwrap_or(0),
                        line: line_no,
                        style: ch.style,
                        rtl: false,
                        sideways,
                        slant: s.faux_italic,
                    });
                    if !up {
                        pen += g.advance;
                    }
                }
                if up {
                    pen += lengths[i - body.start] - cx.after(i);
                }
                pen += cx.after(i);
            }
            line_no += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> TextDoc {
        TextDoc { font: "Tahoma".into(), size: 100.0, ..TextDoc::of(text) }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_line_of_latin_and_thai() {
        let l = lay_out(&doc("Hi สวัสดี"));
        assert!(!l.no_font && l.missing.is_empty());
        assert_eq!(l.lines.len(), 1);
        assert_eq!(l.chars, 9);
        assert_eq!(l.words, 2);
        assert_eq!(l.glyphs[0].x, 0.0);
        assert!(l.glyphs.iter().all(|g| g.line == 0));
        assert_eq!(l.glyphs.last().unwrap().word, 1);
        let w = l.lines[0].width;
        let tracked = lay_out(&TextDoc { tracking: 100.0, ..doc("Hi สวัสดี") });
        assert!((tracked.lines[0].width - (w + 8.0 * 10.0)).abs() < 0.5, "a tenth of the size after each of 8 characters: {} vs {w}", tracked.lines[0].width);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn lines_break_and_align() {
        let l = lay_out(&doc("one\ntwo"));
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[1].y, 120.0, "1.2 times the size apart");
        let centred = lay_out(&TextDoc { justify: Justify::Center, ..doc("middle") });
        assert!((centred.lines[0].x + centred.lines[0].width / 2.0).abs() < 1e-9, "point text centres on its origin");
        // a box wraps at spaces, and between Thai words
        let wrapped = lay_out(&TextDoc { box_size: Some([400.0, 1000.0]), ..doc("one two three four five six") });
        assert!(wrapped.lines.len() >= 3, "{:?}", wrapped.lines);
        assert!(wrapped.lines.iter().all(|l| l.width <= 400.0 + 1e-6));
        let thai = lay_out(&TextDoc { box_size: Some([350.0, 1000.0]), ..doc("สวัสดีครับวันนี้อากาศดีมาก") });
        assert!(thai.lines.len() >= 2 && thai.lines.iter().all(|l| l.width <= 350.0 + 1e-6), "{:?}", thai.lines);
        // justified lines fill the box but the last
        let words = "aa bb cc dd ee ff gg hh ii";
        let just = lay_out(&TextDoc { box_size: Some([500.0, 1000.0]), justify: Justify::JustifyLeft, ..doc(words) });
        let text: Vec<char> = words.chars().collect();
        let last_glyph_end = |line: usize| just.glyphs.iter().filter(|g| g.line == line && text[g.char_index] != ' ').map(|g| g.x + g.advance).fold(0.0, f64::max);
        assert!((last_glyph_end(0) - 500.0).abs() < 1.0, "{}", last_glyph_end(0));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn fonts_fall_back_and_caps() {
        // Georgia has no Thai: it comes from a Thai font
        let l = lay_out(&TextDoc { font: "Georgia".into(), ..TextDoc::of("ก a") });
        assert!(l.missing.is_empty());
        assert!(!Arc::ptr_eq(&l.glyphs[0].font, &l.glyphs[2].font));
        let caps = lay_out(&TextDoc { caps: Caps::AllCaps, ..doc("ab") });
        let font = resolve("Tahoma", 400.0, false).unwrap();
        assert_eq!(caps.glyphs[0].glyph, font.glyph('A'));
        assert!(lay_out(&TextDoc { font: "No Such Font".into(), ..TextDoc::of("x") }).no_font);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn runs_of_their_own_look() {
        let mut d = doc("big small");
        d.style_range(4..9, &mut |s| s.size = 50.0);
        let l = lay_out(&d);
        assert_eq!(l.styles.len(), 2);
        let big = l.glyphs.iter().find(|g| g.char_index == 0).unwrap();
        let small = l.glyphs.iter().find(|g| g.char_index == 5).unwrap();
        assert!((big.scale / small.scale - 2.0).abs() < 1e-9);
        assert_eq!(small.style, 1);
        // stretched across: the advance grows, not the height
        let wide = lay_out(&TextDoc { h_scale: 200.0, ..doc("H") });
        let plain = lay_out(&doc("H"));
        assert!((wide.glyphs[0].advance / plain.glyphs[0].advance - 2.0).abs() < 1e-9);
        assert_eq!(wide.glyphs[0].scale, plain.glyphs[0].scale);
        // a small figure, raised and smaller
        let mut sup = doc("x2");
        sup.style_range(1..2, &mut |s| s.position = Position::Superscript);
        let l = lay_out(&sup);
        assert!(l.glyphs[1].y < l.glyphs[0].y - 20.0 && l.glyphs[1].scale < l.glyphs[0].scale);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn right_to_left_paragraphs() {
        // an Arabic paragraph: its first letter at the right, the number left to right inside it
        let d = TextDoc { font: "Helvetica".into(), size: 60.0, ..TextDoc::of("مرحبا 123") };
        let l = lay_out(&d);
        assert!(l.missing.is_empty(), "{:?}", l.missing);
        // from an Arabic font, its letters joined: the first in its form that joins on
        let first = l.glyphs.iter().find(|g| g.char_index == 0).unwrap();
        assert_ne!(first.glyph, first.font.glyph('م'), "the first letter joins the next");
        let at = |i: usize| l.glyphs.iter().find(|g| g.char_index == i).map(|g| g.x).unwrap();
        assert!(at(0) > at(4), "the first letter is right of the fifth");
        assert!(at(6) < at(7) && at(7) < at(8), "digits go left to right");
        assert!(at(8) < at(4), "the number is left of the word");
        assert!(l.glyphs.iter().filter(|g| g.char_index < 5).all(|g| g.rtl));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn indents_spacing_and_the_evenest_lines() {
        let mut d = TextDoc { box_size: Some([600.0, 2000.0]), ..doc("one two\nthree") };
        d.indent_first = 50.0;
        d.space_before = 30.0;
        let l = lay_out(&d);
        assert!((l.lines[0].x - 50.0).abs() < 1e-6, "the first line indented: {:?}", l.lines[0]);
        assert!((l.lines[1].y - l.lines[0].y - 150.0).abs() < 1e-6, "a paragraph's space before it: {:?}", l.lines);
        // the evenest breaks against as-full-as-it-goes
        let words = "aaa bb cccc ddd ee f gggg hh iii";
        let greedy = lay_out(&TextDoc { composer: Composer::SingleLine, box_size: Some([700.0, 5000.0]), ..doc(words) });
        let even = lay_out(&TextDoc { composer: Composer::EveryLine, box_size: Some([700.0, 5000.0]), ..doc(words) });
        let spread = |l: &Layout| {
            let w: Vec<f64> = l.lines[..l.lines.len() - 1].iter().map(|x| x.width).collect();
            w.iter().cloned().fold(0.0, f64::max) - w.iter().cloned().fold(f64::INFINITY, f64::min)
        };
        assert!(spread(&even) <= spread(&greedy) + 1e-6, "even {} vs greedy {}", spread(&even), spread(&greedy));
        assert!(even.lines.iter().all(|l| l.width <= 700.0 + 1e-6));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn underlines_and_vertical_columns() {
        let mut d = doc("under line");
        d.underline = true;
        let l = lay_out(&d);
        assert_eq!(l.decorations.len(), 1);
        let dec = &l.decorations[0];
        assert!(dec.y > 0.0 && dec.x0 < 1.0 && dec.x1 > 300.0, "{dec:?}");
        // vertical: CJK upright down the column, the next column to the left
        let v = lay_out(&TextDoc { font: "Hiragino Sans".into(), size: 50.0, vertical: true, ..TextDoc::of("日本\n語") });
        assert!(v.missing.is_empty(), "{:?}", v.missing);
        let g0 = &v.glyphs[0];
        let g1 = &v.glyphs[1];
        assert!(g1.y > g0.y + 30.0 && (g1.x - g0.x).abs() < 1.0, "down the column: {g0:?} {g1:?}");
        let g2 = &v.glyphs[2];
        assert!(g2.x < g0.x - 30.0, "the second column to the left: {g2:?}");
        assert!(!g0.sideways);
        let latin = lay_out(&TextDoc { font: "Hiragino Sans".into(), size: 50.0, vertical: true, ..TextDoc::of("ab") });
        assert!(latin.glyphs.iter().all(|g| g.sideways));
    }
}
