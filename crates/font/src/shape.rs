//! Shaping: text made into glyphs, placed the way the font itself says (its OpenType tables).
//!
//! Characters become glyphs through the character map; `GSUB` then swaps glyphs (forms that join
//! up, ligatures, glyphs split in two, marks chosen to fit), and `GPOS` moves them (kerning
//! between pairs, marks set on the glyph they sit on, marks on marks, letters joined up in a
//! line). Thai text is prepared as its shaping expects first: SARA AM splits into NIKHAHIT and
//! SARA AA, and NIKHAHIT goes before the tone marks above it. Arabic letters take the form that
//! joins them to their neighbours (alone, first, middle, last) before the font's other rules,
//! and Arabic and Hebrew come out right to left (glyphs in the order they are seen, left to
//! right). Where a font has no `GPOS` kerning, its old `kern` table is used. Features beyond the
//! usual ones (old-style figures, small capitals, stylistic sets…) are asked for by [`Options`].
//! Fonts made for the Mac with none of OpenType's tables are shaped by Apple's own (`morx`,
//! `kerx`, `kern`, `trak`; see [`aat`]), which do for themselves what the shaping of each script
//! would (Arabic joining, Thai marks, the reordering of Indic letters).
//!
//! The scripts of India are shaped as OpenType's Indic shaping lays down (see [`indic`]), and
//! Khmer and Myanmar as their own shaping does (see [`khmer`] and [`myanmar`]).
//!
//! Positions are in font units, x to the right and y up.

mod aat;
mod indic;
mod khmer;
mod myanmar;

pub use indic::Indic;

use crate::{Font, i16_at, u16_at, u32_at};

/// A glyph placed: where the pen moves after it (`advance`), and how far it is drawn from where
/// the pen is (`dx`, `dy`). `cluster` is the index of the first character it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    pub id: u16,
    pub cluster: usize,
    pub advance: f64,
    pub dx: f64,
    pub dy: f64,
}

/// Writing systems with their own shaping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Latin,
    Thai,
    Arabic,
    Hebrew,
    /// One of the scripts of India.
    Indic(Indic),
    Khmer,
    Myanmar,
    /// Anything else, shaped by the font's default rules.
    Other,
}

impl Script {
    /// The script tag a table's rules are under for this script (the newer of two where the
    /// table has it).
    fn tag_in(self, layout: &Layout, table: usize) -> [u8; 4] {
        match self {
            Script::Indic(s) => s.tag_in(layout, table),
            Script::Myanmar if !layout.has_script(table, b"mym2") => *b"mymr",
            _ => *self.tag(),
        }
    }

    fn tag(self) -> &'static [u8; 4] {
        match self {
            Script::Latin => b"latn",
            Script::Thai => b"thai",
            Script::Arabic => b"arab",
            Script::Hebrew => b"hebr",
            Script::Indic(s) => s.tag(),
            Script::Khmer => b"khmr",
            Script::Myanmar => b"mym2",
            Script::Other => b"DFLT",
        }
    }

    /// Written right to left.
    pub fn rtl(self) -> bool {
        matches!(self, Script::Arabic | Script::Hebrew)
    }
}

/// The script a character belongs to; none for those every script shares (spaces, digits,
/// punctuation).
pub fn script_of(c: char) -> Option<Script> {
    match c as u32 {
        0x0E00..=0x0E7F => Some(Script::Thai),
        0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => Some(Script::Latin),
        0x0600..=0x06FF | 0x0750..=0x077F | 0x0870..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Some(Script::Arabic),
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => Some(Script::Hebrew),
        0x0900..=0x0963 | 0x0966..=0x0D7F | 0xA8E0..=0xA8FF => Indic::of(c).map(Script::Indic),
        0x1780..=0x17FF | 0x19E0..=0x19FF => Some(Script::Khmer),
        0x1000..=0x109F | 0xAA60..=0xAA7F | 0xA9E0..=0xA9FF => Some(Script::Myanmar),
        0x30..=0x39 | 0x20..=0x2F | 0x3A..=0x40 | 0x5B..=0x60 | 0x7B..=0x7E | 0xA0..=0xBF | 0x2000..=0x206F | 0x0964 | 0x0965 | 0x1CD0..=0x1CFF | 0x25CC => None,
        _ => Some(Script::Other),
    }
}

/// What shaping is asked for beyond the font's usual rules.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    /// Features switched on (true) or off (false) by tag: `liga` off leaves letters apart,
    /// `smcp`, `onum`, `ss01`… on asks for what the font has of them.
    pub features: Vec<([u8; 4], bool)>,
    /// Leave out the font's own kerning (kerning set by hand, or by the shapes, comes after).
    pub no_kerning: bool,
    /// The text goes right to left (true) or left to right (false), as the bidirectional
    /// order says; none: as its script goes.
    pub rtl: Option<bool>,
    /// How big the text is set (points, or pixels, to an em), for fonts drawn for each size
    /// that space their letters by size too (`trak`, as the system does); none: as the font is
    /// drawn.
    pub size: Option<f64>,
}

impl Options {
    fn on(&self, tag: &[u8; 4], default: bool) -> bool {
        self.features.iter().rev().find(|(t, _)| t == tag).map_or(default, |(_, v)| *v)
    }
}

/// Features applied before the joining forms, the joining forms themselves (Arabic), and those
/// after; then positions.
const FIRST: [&[u8; 4]; 2] = [b"ccmp", b"locl"];
const FORMS: [&[u8; 4]; 7] = [b"isol", b"fina", b"fin2", b"fin3", b"medi", b"med2", b"init"];
const USUAL: [&[u8; 4]; 6] = [b"rlig", b"rclt", b"calt", b"liga", b"clig", b"mset"];
/// Features people may ask for (off unless asked).
pub const ASKED: [&[u8; 4]; 26] = [
    b"dlig", b"hlig", b"salt", b"swsh", b"cswh", b"titl", b"ordn", b"frac", b"numr", b"dnom", b"smcp", b"c2sc", b"pcap", b"c2pc", b"onum", b"lnum", b"pnum", b"tnum", b"case", b"zero", b"sups", b"subs",
    b"sinf", b"unic", b"ss01", b"ss02",
];
const POSITIONS: [&[u8; 4]; 7] = [b"abvm", b"blwm", b"kern", b"mark", b"mkmk", b"dist", b"curs"];

/// How an Arabic letter joins: not at all, only to the letter before it (right-joining), both
/// ways, by passing joining through (a tatweel, a zero-width joiner), or not counted (a mark).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Joining {
    None,
    Right,
    Dual,
    Causing,
    Transparent,
}

/// The joining of a character (Unicode's ArabicShaping table, the letters in use).
fn joining(c: char) -> Joining {
    use Joining::*;
    let u = c as u32;
    match u {
        0x200D | 0x0640 | 0x07FA => Causing,
        0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4 | 0x06E7 | 0x06E8 | 0x06EA..=0x06ED | 0x08CA..=0x08FF | 0x0898..=0x089F => Transparent,
        0x0300..=0x036F | 0x05B0..=0x05C7 => Transparent,
        0x0622..=0x0625 | 0x0627 | 0x0629 | 0x062F..=0x0632 | 0x0648 | 0x0671..=0x0673 | 0x0675..=0x0677 | 0x0688..=0x0699 | 0x06C0 | 0x06C3..=0x06CB | 0x06CD | 0x06CF | 0x06D2 | 0x06D3 | 0x06D5 | 0x06EE | 0x06EF
        | 0x0759..=0x075B | 0x076B | 0x076C | 0x0771 | 0x0773 | 0x0774 | 0x0778 | 0x0779 | 0x08AA..=0x08AC | 0x08AE | 0x08B1 | 0x08B2 | 0x08B9 => Right,
        0x0620 | 0x0626 | 0x0628 | 0x062A..=0x062E | 0x0633..=0x063F | 0x0641..=0x0647 | 0x0649 | 0x064A | 0x066E | 0x066F | 0x0678..=0x0687 | 0x069A..=0x06BF | 0x06C1 | 0x06C2 | 0x06CC | 0x06CE | 0x06D0 | 0x06D1
        | 0x06FA..=0x06FC | 0x06FF | 0x0750..=0x077F | 0x08A0..=0x08C9 => Dual,
        _ => None,
    }
}

/// The joining form of each character of an Arabic run: 0 none, 1 alone, 2 first, 3 middle,
/// 4 last.
fn forms(text: &[char]) -> Vec<u8> {
    let kinds: Vec<Joining> = text.iter().map(|&c| joining(c)).collect();
    let mut out = vec![0u8; text.len()];
    let joins_after = |k: Joining| matches!(k, Joining::Dual | Joining::Causing);
    let joins_before = |k: Joining| matches!(k, Joining::Dual | Joining::Right | Joining::Causing);
    for i in 0..text.len() {
        let k = kinds[i];
        if matches!(k, Joining::None | Joining::Transparent) {
            continue;
        }
        let prev = (0..i).rev().map(|j| kinds[j]).find(|k| *k != Joining::Transparent);
        let next = (i + 1..text.len()).map(|j| kinds[j]).find(|k| *k != Joining::Transparent);
        let from_prev = joins_before(k) && prev.is_some_and(joins_after);
        let to_next = joins_after(k) && next.is_some_and(joins_before);
        out[i] = match (from_prev, to_next) {
            (false, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
            (true, false) => 4,
        };
        if k == Joining::Causing {
            out[i] = 0;
        }
    }
    out
}

/// The form a joining feature is for.
fn form_of(tag: &[u8; 4]) -> u8 {
    match tag {
        b"isol" => 1,
        b"init" => 2,
        b"medi" | b"med2" => 3,
        b"fina" | b"fin2" | b"fin3" => 4,
        _ => 0,
    }
}

/// One glyph while shaping.
#[derive(Clone, Debug)]
struct G {
    id: u16,
    cluster: usize,
    advance: f64,
    dx: f64,
    dy: f64,
    /// Set on another glyph: which, and where from it (its anchor less this glyph's).
    attach: Option<(usize, f64, f64)>,
    /// Its Arabic joining form (0: none), which only the matching form feature changes.
    form: u8,
    /// Joined in a line to the glyph after it in the text (cursive attachment): that glyph and
    /// how far up it sits from this one.
    chain: Option<(usize, f64)>,
    /// Set on another glyph by Apple's tables, which leave it its own advance.
    keep: bool,
    /// The features that may change it (bits Indic shaping sets; every one otherwise).
    mask: u32,
    /// The syllable it is in (Indic shaping; 0: none), which a syllable's features keep to.
    syllable: u16,
    /// Its Indic category and position (see `indic`).
    cat: u8,
    pos: u8,
    /// What substitution made of it: [`SUBSTITUTED`], [`LIGATED`], [`MULTIPLIED`].
    props: u8,
    /// A zero-width non-joiner (1) or joiner (2), which matching may pass over.
    joiner: u8,
}

/// A glyph swapped for another.
const SUBSTITUTED: u8 = 1;
/// A glyph made of several.
const LIGATED: u8 = 2;
/// One of the glyphs one was split into.
const MULTIPLIED: u8 = 4;

impl G {
    /// Glyph `id` of character `c`, the `cluster`th of the text, in joining form `form`.
    fn new(id: u16, cluster: usize, form: u8, c: char) -> G {
        let joiner = match c {
            '\u{200C}' => 1,
            '\u{200D}' => 2,
            _ => 0,
        };
        G { id, cluster, advance: 0.0, dx: 0.0, dy: 0.0, attach: None, form, chain: None, keep: false, mask: u32::MAX, syllable: 0, cat: 0, pos: 0, props: 0, joiner }
    }
}

/// Text made into placed glyphs with `font`, the font's usual rules.
pub fn shape(font: &Font, text: &[char]) -> Vec<Glyph> {
    shape_with(font, text, &Options::default())
}

/// Text made into placed glyphs with `font`, with `options`. A run of right-to-left script comes
/// out reversed: its glyphs in the order they are seen.
pub fn shape_with(font: &Font, text: &[char], options: &Options) -> Vec<Glyph> {
    // runs of one script; shared characters go with the script around them
    let mut runs: Vec<(Script, usize, usize)> = Vec::new();
    for (i, &c) in text.iter().enumerate() {
        match (script_of(c), runs.last_mut()) {
            (None, Some(r)) => r.2 = i + 1,
            (None, None) => runs.push((Script::Latin, i, i + 1)),
            (Some(s), Some(r)) if r.0 == s => r.2 = i + 1,
            (Some(s), Some(r)) if r.2 - r.1 > 0 && text[r.1..r.2].iter().all(|&c| script_of(c).is_none()) => {
                r.0 = s;
                r.2 = i + 1;
            }
            (Some(s), _) => runs.push((s, i, i + 1)),
        }
    }
    let mut out = Vec::new();
    // right to left as a whole: the runs too in the order they are seen
    if options.rtl == Some(true) {
        runs.reverse();
    }
    for (script, a, b) in runs {
        out.extend(shape_run(font, &text[a..b], a, script, options));
    }
    out
}

/// The OpenType features a font has (in its substitutions and positions, or that stand for the
/// settings of Apple's tables), each once.
pub fn features(font: &Font) -> Vec<String> {
    let d = font.data();
    let mut out: Vec<String> = aat::features(font);
    if font.gpos == 0 && (font.kerx != 0 || font.kern != 0) {
        out.push("kern".into());
    }
    out.dedup();
    for table in [font.gsub, font.gpos] {
        if table == 0 {
            continue;
        }
        let list = table + u16_at(d, table + 6) as usize;
        for k in 0..u16_at(d, list) as usize {
            let r = list + 2 + 6 * k;
            if let Some(tag) = d.get(r..r + 4) {
                let t = String::from_utf8_lossy(tag).to_string();
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Thai text as its shaping expects it: SARA AM as NIKHAHIT and SARA AA, NIKHAHIT moved before
/// the marks above that come before it. Each character with the index it came from.
fn prepare_thai(text: &[char], first: usize) -> Vec<(char, usize)> {
    let above = |c: char| matches!(c as u32, 0x0E31 | 0x0E34..=0x0E37 | 0x0E47..=0x0E4E);
    let mut out: Vec<(char, usize)> = Vec::with_capacity(text.len() + 2);
    for (i, &c) in text.iter().enumerate() {
        if c == '\u{0E33}' {
            let at = out.iter().rposition(|(p, _)| !above(*p)).map_or(0, |k| k + 1);
            out.insert(at, ('\u{0E4D}', first + i));
            out.push(('\u{0E32}', first + i));
        } else {
            out.push((c, first + i));
        }
    }
    out
}

/// The order Hebrew points are drawn in (shin and sin dots first, then dagesh, rafe, holam, the
/// vowels below): their combining class as the shapers adjust it (0 for a letter).
fn hebrew_order(c: char) -> u8 {
    match c as u32 {
        0x05C1 => 10,
        0x05C2 => 11,
        0x05BC => 12,
        0x05BF => 13,
        0x05B9 | 0x05BA => 14,
        0x05B1 => 15,
        0x05B2 => 16,
        0x05B3 => 17,
        0x05B5 => 18,
        0x05B6 => 19,
        0x05B7 => 20,
        0x05B8 | 0x05C7 => 21,
        0x05B0 => 22,
        0x05B4 => 23,
        0x05BB => 24,
        0x05BD => 25,
        0xFB1E => 26,
        0x0591..=0x05AF | 0x05C4 | 0x05C5 => 230,
        _ => 0,
    }
}

/// A Hebrew letter and point made one of the font's precomposed forms, where there is one.
fn hebrew_compose(a: char, b: char) -> Option<char> {
    const DAGESH: [u32; 27] = [
        0xFB30, 0xFB31, 0xFB32, 0xFB33, 0xFB34, 0xFB35, 0xFB36, 0, 0xFB38, 0xFB39, 0xFB3A, 0xFB3B, 0xFB3C, 0, 0xFB3E, 0, 0xFB40, 0xFB41, 0, 0xFB43, 0xFB44, 0, 0xFB46, 0xFB47, 0xFB48, 0xFB49, 0xFB4A,
    ];
    let (a, b) = (a as u32, b as u32);
    let out = match (b, a) {
        (0x05B4, 0x05D9) => 0xFB1D,
        (0x05B7, 0x05F2) => 0xFB1F,
        (0x05B7, 0x05D0) => 0xFB2E,
        (0x05B8, 0x05D0) => 0xFB2F,
        (0x05B9, 0x05D5) => 0xFB4B,
        (0x05BC, 0x05D0..=0x05EA) => DAGESH[(a - 0x05D0) as usize],
        (0x05BC, 0xFB2A) => 0xFB2C,
        (0x05BC, 0xFB2B) => 0xFB2D,
        (0x05BF, 0x05D1) => 0xFB4C,
        (0x05BF, 0x05DB) => 0xFB4D,
        (0x05BF, 0x05E4) => 0xFB4E,
        (0x05C1, 0x05E9) => 0xFB2A,
        (0x05C1, 0xFB49) => 0xFB2C,
        (0x05C2, 0x05E9) => 0xFB2B,
        (0x05C2, 0xFB49) => 0xFB2D,
        _ => 0,
    };
    char::from_u32(out).filter(|_| out != 0)
}

/// Hebrew text as its shaping expects it: the points after each letter in the order they are
/// drawn, and, for a font that cannot place points itself, letter and point made one where the
/// font has that form. Each character with the index it came from.
fn prepare_hebrew(font: &Font, layout: &Layout, text: &[char], first: usize) -> Vec<(char, usize)> {
    let mut out: Vec<(char, usize)> = text.iter().enumerate().map(|(i, &c)| (c, first + i)).collect();
    let mut i = 0;
    while i < out.len() {
        let j = (i..out.len()).find(|&k| hebrew_order(out[k].0) == 0).unwrap_or(out.len());
        if j == i {
            i += 1;
            continue;
        }
        out[i..j].sort_by_key(|(c, _)| hebrew_order(*c));
        i = j;
    }
    let places_marks = font.gpos != 0 && !layout.lookups(font.gpos, b"hebr", &[b"mark"]).is_empty();
    if places_marks {
        return out;
    }
    let mut k = 0;
    while k + 1 < out.len() {
        match hebrew_compose(out[k].0, out[k + 1].0).filter(|c| font.glyph(*c) != 0) {
            Some(c) => {
                out[k].0 = c;
                out.remove(k + 1);
            }
            None => k += 1,
        }
    }
    out
}

fn shape_run(font: &Font, text: &[char], first: usize, script: Script, options: &Options) -> Vec<Glyph> {
    let rtl = options.rtl.unwrap_or(script.rtl());
    // a font with Apple's tables does each script's shaping itself (by them before OpenType's,
    // as the system does), but for Hebrew points made one with their letter where it has that
    // form
    let apple = font.morx != 0;
    let chars: Vec<(char, usize)> = match script {
        Script::Hebrew => prepare_hebrew(font, &Layout { font, d: font.data() }, text, first),
        _ if apple => text.iter().enumerate().map(|(i, &c)| (c, first + i)).collect(),
        Script::Thai => prepare_thai(text, first),
        Script::Indic(_) => indic::prepare(font, text, first),
        Script::Khmer => khmer::prepare(font, text, first),
        Script::Myanmar => myanmar::prepare(text, first),
        _ => text.iter().enumerate().map(|(i, &c)| (c, first + i)).collect(),
    };
    let joined = if script == Script::Arabic && !apple { forms(&chars.iter().map(|c| c.0).collect::<Vec<_>>()) } else { vec![0; chars.len()] };
    let mut buf: Vec<G> = chars.iter().zip(&joined).map(|(&(c, cluster), &form)| G::new(font.glyph(c), cluster, form, c)).collect();
    let layout = Layout { font, d: font.data() };
    if apple {
        aat::substitute(font, &mut buf, options, rtl, options.on(b"vert", false));
    } else if font.gsub != 0 {
        let pick = |list: &[&'static [u8; 4]]| -> Vec<&'static [u8; 4]> { list.iter().copied().filter(|t| options.on(t, true)).collect() };
        let mut usual = pick(&USUAL);
        if !options.on(b"liga", true) {
            usual.retain(|t| *t != b"clig");
        }
        usual.extend(ASKED.iter().copied().filter(|t| options.on(t, false)));
        // stylistic sets beyond the first two, asked by name
        let extra: Vec<[u8; 4]> = options.features.iter().filter(|(t, on)| *on && t.starts_with(b"ss") && !ASKED.contains(&t)).map(|(t, _)| *t).collect();
        let mut usual: Vec<&[u8; 4]> = usual;
        usual.extend(extra.iter());
        let tag = script.tag_in(&layout, font.gsub);
        // the shaping of a script, for a font with rules of its own for it (else the default)
        let own = |t: &[u8; 4]| layout.has_script(font.gsub, t);
        match script {
            Script::Indic(s) if own(&tag) => indic::substitute(&layout, &mut buf, s, &chars, options, &pick(&FIRST), &usual),
            Script::Khmer => khmer::substitute(&layout, &mut buf, &chars, options, &pick(&FIRST), &usual),
            Script::Myanmar if &tag == b"mym2" => myanmar::substitute(&layout, &mut buf, &chars, options, &pick(&FIRST), &usual),
            _ => {
                for (li, _) in layout.lookups(font.gsub, &tag, &pick(&FIRST)) {
                    layout.substitute(&mut buf, li, &Apply::default());
                }
                if script == Script::Arabic {
                    for (li, t) in layout.lookups(font.gsub, &tag, &FORMS) {
                        layout.substitute(&mut buf, li, &Apply { form: form_of(&t), ..Apply::default() });
                    }
                }
                for (li, _) in layout.lookups(font.gsub, &tag, &usual) {
                    layout.substitute(&mut buf, li, &Apply::default());
                }
            }
        }
    }
    for g in &mut buf {
        g.advance = font.advance(g.id);
    }
    // kerning from GPOS; where it has none, from Apple's kerx, or the old kern table in either
    // its Apple or its Microsoft form
    let mut kerned = false;
    // a font shaped by Apple's tables is spaced by them too
    let apple_kerning = apple && font.kerx != 0;
    if font.gpos != 0 && !apple_kerning {
        let positions: Vec<&[u8; 4]> = POSITIONS.iter().copied().filter(|t| !(options.no_kerning && *t == b"kern")).collect();
        let tag = script.tag_in(&layout, font.gpos);
        let lookups = layout.lookups(font.gpos, &tag, &positions);
        kerned = !layout.lookups(font.gpos, &tag, &[b"kern"]).is_empty();
        for (li, _) in lookups {
            layout.position(&mut buf, li, rtl);
        }
    }
    if !kerned {
        if font.kerx != 0 && (font.gpos == 0 || apple_kerning) {
            aat::kern(font, &mut buf, aat::Kerning::Kerx, rtl, !options.no_kerning);
        } else if font.kern != 0 && u16_at(font.data(), font.kern) == 1 {
            aat::kern(font, &mut buf, aat::Kerning::Kern, rtl, !options.no_kerning);
        } else if font.kern != 0 && !options.no_kerning {
            layout.old_kern(&mut buf);
        }
    }
    // spaced by size as the system spaces it: for fonts drawn for each size (an optical size
    // axis), as they are set to the size
    if let Some(size) = options.size
        && font.trak != 0
        && font.axes.iter().any(|a| &a.0 == b"opsz")
    {
        aat::track(font, &mut buf, size);
    }
    // marks set on others by OpenType's tables take no room of their own, and are drawn from
    // the glyph they sit on (Indic marks keep the room the font gives them)
    if !matches!(script, Script::Indic(_) | Script::Khmer) {
        for g in buf.iter_mut().filter(|g| g.attach.is_some() && !g.keep) {
            g.advance = 0.0;
        }
    }
    // joiners only steer the shaping: nothing drawn, no room taken
    let space = font.glyph(' ');
    for g in buf.iter_mut().filter(|g| g.joiner != 0) {
        g.id = space;
        g.advance = 0.0;
    }
    // letters joined in a line sit on each other: each up from the one it hangs on
    for i in (0..buf.len()).rev() {
        if let Some((j, dy)) = buf[i].chain {
            buf[i].dy += buf[j].dy + dy;
        }
    }
    // right to left: in the order they are seen
    if rtl {
        let n = buf.len();
        buf.reverse();
        for g in &mut buf {
            if let Some((t, x, y)) = g.attach {
                g.attach = Some((n - 1 - t, x, y));
            }
        }
    }
    let mut pen = Vec::with_capacity(buf.len());
    let mut x = 0.0;
    for g in &buf {
        pen.push(x);
        x += g.advance;
    }
    for i in 0..buf.len() {
        if let Some((t, ox, oy)) = buf[i].attach {
            buf[i].dx = pen[t] + buf[t].dx + ox - pen[i];
            buf[i].dy = buf[t].dy + oy;
        }
    }
    buf.into_iter().map(|g| Glyph { id: g.id, cluster: g.cluster, advance: g.advance, dx: g.dx, dy: g.dy }).collect()
}

/// What a context rule matched: the positions it matched, and the lookups to apply then
/// (place in the match, lookup).
type Matched = (Vec<usize>, Vec<(usize, u16)>);

/// How a match goes along the glyphs: those it passes over (by its lookup's flag and mark set,
/// and the joiners it may pass over), and those it may take (with a feature's bit, in one
/// syllable).
#[derive(Clone, Copy)]
struct Walk {
    flag: u16,
    set: u16,
    /// The feature bits a glyph needs to be taken (0: any glyph).
    mask: u32,
    /// The syllable the match keeps to (0: any).
    syllable: u16,
    /// Zero-width joiners, and non-joiners, may be passed over.
    zwj: bool,
    zwnj: bool,
}

impl Walk {
    /// As positioning goes: any glyph taken, joiners passed over.
    fn any(flag: u16, set: u16) -> Walk {
        Walk { flag, set, mask: 0, syllable: 0, zwj: true, zwnj: true }
    }
}

/// How a substitution lookup is applied: only to glyphs of one joining form (Arabic; 0: any),
/// only to glyphs with a feature's bit (0: any), within each syllable, and with joiners and
/// non-joiners counted as glyphs (Indic features match them) rather than passed over.
#[derive(Clone, Copy, Default)]
struct Apply {
    form: u8,
    mask: u32,
    per_syllable: bool,
    manual_zwj: bool,
    manual_zwnj: bool,
}

impl Apply {
    /// How a match from glyph `start` goes: through the glyphs it changes, and through those
    /// before and after them that it looks at.
    fn walks(&self, flag: u16, set: u16, start: &G) -> (Walk, Walk) {
        let syllable = if self.per_syllable { start.syllable } else { 0 };
        (Walk { flag, set, mask: self.mask, syllable, zwj: !self.manual_zwj, zwnj: false }, Walk { flag, set, mask: 0, syllable, zwj: true, zwnj: !self.manual_zwnj })
    }
}

/// Glyphs `a` to `b` made one cluster (the first character any of them came from), with the
/// glyphs around that shared a cluster with them.
fn merge(buf: &mut [G], mut a: usize, mut b: usize) {
    if b <= a + 1 || b > buf.len() {
        return;
    }
    let cluster = buf[a..b].iter().map(|g| g.cluster).min().unwrap_or(0);
    if cluster != buf[b - 1].cluster {
        while b < buf.len() && buf[b - 1].cluster == buf[b].cluster {
            b += 1;
        }
    }
    if cluster != buf[a].cluster {
        while a > 0 && buf[a - 1].cluster == buf[a].cluster {
            a -= 1;
        }
    }
    for g in &mut buf[a..b] {
        g.cluster = cluster;
    }
}

/// The font's layout tables, read as needed.
struct Layout<'a> {
    font: &'a Font,
    d: &'a [u8],
}

/// Lookup flags.
const IGNORE_BASES: u16 = 0x2;
const IGNORE_LIGATURES: u16 = 0x4;
const IGNORE_MARKS: u16 = 0x8;
const MARK_FILTERING: u16 = 0x10;

impl Layout<'_> {
    fn u16(&self, at: usize) -> u16 {
        u16_at(self.d, at)
    }

    fn i16(&self, at: usize) -> i16 {
        i16_at(self.d, at)
    }

    fn tag(&self, at: usize) -> &[u8] {
        self.d.get(at..at + 4).unwrap_or(&[])
    }

    /// Which place `g` has in a coverage table, if it is in it.
    fn coverage(&self, at: usize, g: u16) -> Option<usize> {
        match self.u16(at) {
            1 => {
                let n = self.u16(at + 2) as usize;
                let (mut lo, mut hi) = (0, n);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let v = self.u16(at + 4 + 2 * mid);
                    match v.cmp(&g) {
                        std::cmp::Ordering::Equal => return Some(mid),
                        std::cmp::Ordering::Less => lo = mid + 1,
                        std::cmp::Ordering::Greater => hi = mid,
                    }
                }
                None
            }
            2 => {
                let n = self.u16(at + 2) as usize;
                for k in 0..n {
                    let r = at + 4 + 6 * k;
                    let (first, last, start) = (self.u16(r), self.u16(r + 2), self.u16(r + 4));
                    if g < first {
                        return None;
                    }
                    if g <= last {
                        return Some((start + g - first) as usize);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// The class `g` has in a class table (0 when it has none).
    fn class(&self, at: usize, g: u16) -> u16 {
        if at == 0 {
            return 0;
        }
        match self.u16(at) {
            1 => {
                let (start, n) = (self.u16(at + 2), self.u16(at + 4));
                if g >= start && g - start < n { self.u16(at + 6 + 2 * (g - start) as usize) } else { 0 }
            }
            2 => {
                let n = self.u16(at + 2) as usize;
                for k in 0..n {
                    let r = at + 4 + 6 * k;
                    if g >= self.u16(r) && g <= self.u16(r + 2) {
                        return self.u16(r + 4);
                    }
                }
                0
            }
            _ => 0,
        }
    }

    /// A glyph's kind in `GDEF`: 1 base, 2 ligature, 3 mark, 4 component (0: unknown).
    fn kind(&self, g: u16) -> u16 {
        let gdef = self.font.gdef;
        if gdef == 0 {
            return 0;
        }
        let off = self.u16(gdef + 4) as usize;
        if off == 0 { 0 } else { self.class(gdef + off, g) }
    }

    fn is_mark(&self, g: u16) -> bool {
        self.kind(g) == 3
    }

    /// Whether a lookup with `flag` (and mark set `set`) passes over glyph `g`.
    fn skips(&self, g: u16, flag: u16, set: u16) -> bool {
        let k = self.kind(g);
        if (k == 1 && flag & IGNORE_BASES != 0) || (k == 2 && flag & IGNORE_LIGATURES != 0) || (k == 3 && flag & IGNORE_MARKS != 0) {
            return true;
        }
        if k == 3 {
            let gdef = self.font.gdef;
            let wanted = flag >> 8;
            if wanted != 0 {
                let off = self.u16(gdef + 10) as usize;
                if off == 0 || self.class(gdef + off, g) != wanted {
                    return true;
                }
            }
            if flag & MARK_FILTERING != 0 {
                // the mark glyph sets come with GDEF 1.2
                let sets = if u32_at(self.d, gdef) >= 0x0001_0002 { self.u16(gdef + 12) as usize } else { 0 };
                if sets == 0 {
                    return true;
                }
                let base = gdef + sets;
                if set >= self.u16(base + 2) {
                    return true;
                }
                let cov = base + u32_at(self.d, base + 4 + 4 * set as usize) as usize;
                return self.coverage(cov, g).is_none();
            }
        }
        false
    }

    /// The lookups for the features in `wanted`, of the script `script` (else the default one),
    /// in the order they are listed, each with the feature it is for.
    fn lookups(&self, table: usize, script: &[u8; 4], wanted: &[&[u8; 4]]) -> Vec<(u16, [u8; 4])> {
        let scripts = table + self.u16(table + 4) as usize;
        let features = table + self.u16(table + 6) as usize;
        let n = self.u16(scripts) as usize;
        let find = |tag: &[u8; 4]| (0..n).map(|k| scripts + 2 + 6 * k).find(|&r| self.tag(r) == tag).map(|r| scripts + self.u16(r + 4) as usize);
        let Some(script_table) = find(script).or_else(|| find(b"DFLT")).or_else(|| find(b"latn")) else { return Vec::new() };
        let lang = self.u16(script_table) as usize;
        if lang == 0 {
            return Vec::new();
        }
        let lang = script_table + lang;
        let mut indices: Vec<u16> = (0..self.u16(lang + 4) as usize).map(|k| self.u16(lang + 6 + 2 * k)).collect();
        let required = self.u16(lang + 2);
        if required != 0xFFFF {
            indices.push(required);
        }
        let mut out: Vec<(u16, [u8; 4])> = Vec::new();
        for fi in indices {
            let r = features + 2 + 6 * fi as usize;
            if fi >= self.u16(features) || !wanted.iter().any(|w| self.tag(r) == *w) {
                continue;
            }
            let mut tag = [0u8; 4];
            tag.copy_from_slice(self.tag(r));
            let f = features + self.u16(r + 4) as usize;
            out.extend((0..self.u16(f + 2) as usize).map(|k| (self.u16(f + 4 + 2 * k), tag)));
        }
        out.sort();
        out.dedup();
        out
    }

    /// Lookup `index` of a table: its type, flag, mark set and subtables.
    fn lookup(&self, table: usize, index: u16) -> Option<(u16, u16, u16, Vec<usize>)> {
        let list = table + self.u16(table + 8) as usize;
        if index >= self.u16(list) {
            return None;
        }
        let at = list + self.u16(list + 2 + 2 * index as usize) as usize;
        let (mut ty, flag, n) = (self.u16(at), self.u16(at + 2), self.u16(at + 4) as usize);
        let set = if flag & MARK_FILTERING != 0 { self.u16(at + 6 + 2 * n) } else { 0 };
        let mut subs: Vec<usize> = (0..n).map(|k| at + self.u16(at + 6 + 2 * k) as usize).collect();
        // an extension lookup points on to subtables of its real type
        let ext = if table == self.font.gsub { 7 } else { 9 };
        if ty == ext {
            ty = subs.first().map_or(0, |&s| self.u16(s + 2));
            subs = subs.iter().map(|&s| s + u32_at(self.d, s + 4) as usize).collect();
        }
        Some((ty, flag, set, subs))
    }

    /// The glyph after (or before) `i` that a match goes on to, passing over what `w` passes
    /// over: none when the next one it cannot pass over is not one `w` takes or fails `test`.
    fn step(&self, buf: &[G], i: usize, w: &Walk, forward: bool, test: &dyn Fn(u16) -> bool) -> Option<usize> {
        let mut j = i;
        loop {
            j = if forward { j + 1 } else { j.checked_sub(1)? };
            let g = buf.get(j)?;
            if self.skips(g.id, w.flag, w.set) {
                continue;
            }
            if (w.mask == 0 || g.mask & w.mask != 0) && (w.syllable == 0 || g.syllable == w.syllable) && test(g.id) {
                return Some(j);
            }
            if (g.joiner == 2 && w.zwj) || (g.joiner == 1 && w.zwnj) {
                continue;
            }
            return None;
        }
    }

    /// Whether the table has its own rules for script `tag`.
    fn has_script(&self, table: usize, tag: &[u8; 4]) -> bool {
        if table == 0 {
            return false;
        }
        let scripts = table + self.u16(table + 4) as usize;
        (0..self.u16(scripts) as usize).any(|k| self.tag(scripts + 2 + 6 * k) == tag)
    }

    /// Whether one of substitution `lookups` changes exactly `glyphs` with nothing before or
    /// after them (`zero_context`: rules that look at glyphs around them do not count).
    fn would_substitute(&self, lookups: &[u16], glyphs: &[u16], zero_context: bool) -> bool {
        let Some(&g) = glyphs.first() else { return false };
        lookups.iter().any(|&li| {
            let Some((ty, _, _, subs)) = self.lookup(self.font.gsub, li) else { return false };
            subs.iter().any(|&s| {
                let cov = s + self.u16(s + 2) as usize;
                match ty {
                    1..=3 => glyphs.len() == 1 && self.coverage(cov, g).is_some(),
                    4 => {
                        let Some(k) = self.coverage(cov, g) else { return false };
                        let lset = s + self.u16(s + 6 + 2 * k) as usize;
                        (0..self.u16(lset) as usize).any(|l| {
                            let lig = lset + self.u16(lset + 2 + 2 * l) as usize;
                            let comps = self.u16(lig + 2) as usize;
                            comps == glyphs.len() && (1..comps).all(|c| self.u16(lig + 4 + 2 * (c - 1)) == glyphs[c])
                        })
                    }
                    5 | 6 => self.would_match(ty, s, glyphs, zero_context),
                    _ => false,
                }
            })
        })
    }

    /// Whether a context (5) or chained context (6) subtable at `s` has a rule whose input is
    /// exactly `glyphs` (`zero_context`: and that looks at nothing before or after).
    fn would_match(&self, ty: u16, s: usize, glyphs: &[u16], zero_context: bool) -> bool {
        let format = self.u16(s);
        let g = glyphs[0];
        let n = glyphs.len();
        let cov = s + self.u16(s + 2) as usize;
        match (ty, format) {
            (5, 1) | (5, 2) | (6, 1) | (6, 2) => {
                let chained = ty == 6;
                let classes = match (chained, format) {
                    (false, 2) => s + self.u16(s + 4) as usize,
                    (true, 2) => s + self.u16(s + 6) as usize,
                    _ => 0,
                };
                let sets_at = match (chained, format) {
                    (false, 2) => s + 6,
                    (true, 2) => s + 10,
                    _ => s + 4,
                };
                let pick = if format == 2 {
                    self.class(classes, g) as usize
                } else {
                    let Some(k) = self.coverage(cov, g) else { return false };
                    k
                };
                if pick >= self.u16(sets_at) as usize {
                    return false;
                }
                let off = self.u16(sets_at + 2 + 2 * pick) as usize;
                if off == 0 {
                    return false;
                }
                let rset = s + off;
                let same = |value: u16, g: u16| if format == 2 { self.class(classes, g) == value } else { g == value };
                (0..self.u16(rset) as usize).any(|r| {
                    let rule = rset + self.u16(rset + 2 + 2 * r) as usize;
                    let (around, inp) = if chained {
                        let nb = self.u16(rule) as usize;
                        let at = rule + 2 + 2 * nb;
                        let ni = self.u16(at) as usize;
                        let na = self.u16(at + 2 + 2 * ni.saturating_sub(1)) as usize;
                        (nb + na, (ni, at + 2))
                    } else {
                        (0, (self.u16(rule) as usize, rule + 4))
                    };
                    (!zero_context || around == 0) && inp.0 == n && (1..n).all(|k| same(self.u16(inp.1 + 2 * (k - 1)), glyphs[k]))
                })
            }
            (5, 3) => {
                let count = self.u16(s + 2) as usize;
                count == n && (0..n).all(|k| self.coverage(s + self.u16(s + 6 + 2 * k) as usize, glyphs[k]).is_some())
            }
            (6, 3) => {
                let nb = self.u16(s + 2) as usize;
                let at = s + 4 + 2 * nb;
                let ni = self.u16(at) as usize;
                let na = self.u16(at + 2 + 2 * ni) as usize;
                (!zero_context || nb + na == 0) && ni == n && (0..n).all(|k| self.coverage(s + self.u16(at + 2 + 2 * k) as usize, glyphs[k]).is_some())
            }
            _ => false,
        }
    }

    // ---- substitution

    /// Apply substitution lookup `li` as `ap` says.
    fn substitute(&self, buf: &mut Vec<G>, li: u16, ap: &Apply) {
        let Some((ty, flag, set, subs)) = self.lookup(self.font.gsub, li) else { return };
        let mut i = 0;
        while i < buf.len() {
            let g = &buf[i];
            if self.skips(g.id, flag, set) || (ap.form != 0 && g.form != ap.form) || (ap.mask != 0 && g.mask & ap.mask == 0) {
                i += 1;
                continue;
            }
            match self.subst_at(buf, i, ty, flag, set, &subs, 0, ap) {
                Some(next) => i = next.max(i + 1),
                None => i += 1,
            }
        }
    }

    /// Apply one lookup's subtables at `i`; where to go on, if one applied.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn subst_at(&self, buf: &mut Vec<G>, i: usize, ty: u16, flag: u16, set: u16, subs: &[usize], depth: u32, ap: &Apply) -> Option<usize> {
        let (inp, around) = ap.walks(flag, set, &buf[i]);
        for &s in subs {
            let format = self.u16(s);
            let cov = s + self.u16(s + 2) as usize;
            let g = buf[i].id;
            match ty {
                1 => {
                    let Some(k) = self.coverage(cov, g) else { continue };
                    buf[i].id = if format == 1 { g.wrapping_add(self.u16(s + 4)) } else { self.u16(s + 6 + 2 * k) };
                    buf[i].props |= SUBSTITUTED;
                    return Some(i + 1);
                }
                2 => {
                    let Some(k) = self.coverage(cov, g) else { continue };
                    let seq = s + self.u16(s + 6 + 2 * k) as usize;
                    let n = self.u16(seq) as usize;
                    // split in more than one: each is one of the parts
                    let props = buf[i].props | SUBSTITUTED | if n > 1 { MULTIPLIED } else { 0 };
                    let glyphs: Vec<G> = (0..n).map(|j| G { id: self.u16(seq + 2 + 2 * j), props, ..buf[i].clone() }).collect();
                    buf.splice(i..i + 1, glyphs);
                    return Some(i + n);
                }
                3 => {
                    let Some(k) = self.coverage(cov, g) else { continue };
                    let alts = s + self.u16(s + 6 + 2 * k) as usize;
                    if self.u16(alts) > 0 {
                        buf[i].id = self.u16(alts + 2);
                        buf[i].props |= SUBSTITUTED;
                    }
                    return Some(i + 1);
                }
                4 => {
                    let Some(k) = self.coverage(cov, g) else { continue };
                    let lset = s + self.u16(s + 6 + 2 * k) as usize;
                    for l in 0..self.u16(lset) as usize {
                        let lig = lset + self.u16(lset + 2 + 2 * l) as usize;
                        let comps = self.u16(lig + 2) as usize;
                        let mut at = i;
                        let mut matched = vec![i];
                        let ok = (1..comps).all(|c| match self.step(buf, at, &inp, true, &|g| g == self.u16(lig + 4 + 2 * (c - 1))) {
                            Some(j) => {
                                at = j;
                                matched.push(j);
                                true
                            }
                            None => false,
                        });
                        if ok {
                            buf[i].id = self.u16(lig);
                            // made of several, no longer one of a split's parts
                            buf[i].props = (buf[i].props | SUBSTITUTED | LIGATED) & !MULTIPLIED;
                            buf[i].cluster = matched.iter().map(|&j| buf[j].cluster).min().unwrap_or(buf[i].cluster);
                            for &j in matched[1..].iter().rev() {
                                buf.remove(j);
                            }
                            return Some(i + 1);
                        }
                    }
                }
                5 | 6 => {
                    if let Some((positions, records)) = self.context(buf, i, ty, s, &inp, &around) {
                        let mut positions = positions;
                        for (seq, lookup) in records {
                            if depth > 8 || seq >= positions.len() {
                                continue;
                            }
                            let before = buf.len();
                            if let Some((t2, f2, s2, subs2)) = self.lookup(self.font.gsub, lookup)
                                && !self.skips(buf[positions[seq]].id, f2, s2)
                            {
                                self.subst_at(buf, positions[seq], t2, f2, s2, &subs2, depth + 1, ap);
                            }
                            // a nested lookup that changed the length moves what follows
                            let grew = buf.len() as isize - before as isize;
                            if grew != 0 {
                                for p in positions.iter_mut().skip(seq + 1) {
                                    *p = (*p as isize + grew).max(0) as usize;
                                }
                            }
                        }
                        return Some(positions.last().map_or(i + 1, |l| l + 1));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// A context (5) or chained context (6) subtable matching at `i`: the matched positions,
    /// and the nested lookups to apply (sequence index, lookup).
    /// (`inp` is how the match goes through the glyphs it changes, `around` through those before
    /// and after.)
    fn context(&self, buf: &[G], i: usize, ty: u16, s: usize, inp: &Walk, around: &Walk) -> Option<Matched> {
        let format = self.u16(s);
        let g = buf[i].id;
        // match `n` glyphs from `i` on with `test(k, glyph)`, passing over skipped glyphs
        let input = |n: usize, test: &dyn Fn(usize, u16) -> bool| -> Option<Vec<usize>> {
            let mut out = vec![i];
            let mut at = i;
            for k in 1..n {
                let j = self.step(buf, at, inp, true, &|g| test(k, g))?;
                out.push(j);
                at = j;
            }
            Some(out)
        };
        let behind = |n: usize, test: &dyn Fn(usize, u16) -> bool| -> bool {
            let mut at = i;
            (0..n).all(|k| match self.step(buf, at, around, false, &|g| test(k, g)) {
                Some(j) => {
                    at = j;
                    true
                }
                None => false,
            })
        };
        let ahead = |from: usize, n: usize, test: &dyn Fn(usize, u16) -> bool| -> bool {
            let mut at = from;
            (0..n).all(|k| match self.step(buf, at, around, true, &|g| test(k, g)) {
                Some(j) => {
                    at = j;
                    true
                }
                None => false,
            })
        };
        let records = |at: usize, n: usize| -> Vec<(usize, u16)> { (0..n).map(|k| (self.u16(at + 4 * k) as usize, self.u16(at + 4 * k + 2))).collect() };
        match (ty, format) {
            // by glyphs and by classes: rule sets picked by the first glyph or its class
            (5, 1) | (5, 2) => {
                let cov = s + self.u16(s + 2) as usize;
                let k = self.coverage(cov, g)?;
                let classes = if format == 2 { s + self.u16(s + 4) as usize } else { 0 };
                let (sets_at, pick) = if format == 2 { (s + 6, self.class(classes, g) as usize) } else { (s + 4, k) };
                if pick >= self.u16(sets_at) as usize {
                    return None;
                }
                let off = self.u16(sets_at + 2 + 2 * pick) as usize;
                if off == 0 {
                    return None;
                }
                let rset = s + off;
                for r in 0..self.u16(rset) as usize {
                    let rule = rset + self.u16(rset + 2 + 2 * r) as usize;
                    let (n, nrec) = (self.u16(rule) as usize, self.u16(rule + 2) as usize);
                    let value = |k: usize| self.u16(rule + 4 + 2 * (k - 1));
                    let test = |k: usize, g: u16| if format == 2 { self.class(classes, g) == value(k) } else { g == value(k) };
                    if let Some(pos) = input(n, &test) {
                        return Some((pos, records(rule + 4 + 2 * (n - 1), nrec)));
                    }
                }
                None
            }
            (5, 3) => {
                let (n, nrec) = (self.u16(s + 2) as usize, self.u16(s + 4) as usize);
                let cov = |k: usize| s + self.u16(s + 6 + 2 * k) as usize;
                self.coverage(cov(0), g)?;
                let pos = input(n, &|k, g| self.coverage(cov(k), g).is_some())?;
                Some((pos, records(s + 6 + 2 * n, nrec)))
            }
            (6, 1) | (6, 2) => {
                let cov = s + self.u16(s + 2) as usize;
                let k = self.coverage(cov, g)?;
                let (back_cls, in_cls, ahead_cls) = if format == 2 {
                    (s + self.u16(s + 4) as usize, s + self.u16(s + 6) as usize, s + self.u16(s + 8) as usize)
                } else {
                    (0, 0, 0)
                };
                let (sets_at, pick) = if format == 2 { (s + 10, self.class(in_cls, g) as usize) } else { (s + 4, k) };
                if pick >= self.u16(sets_at) as usize {
                    return None;
                }
                let off = self.u16(sets_at + 2 + 2 * pick) as usize;
                if off == 0 {
                    return None;
                }
                let rset = s + off;
                for r in 0..self.u16(rset) as usize {
                    let mut at = rset + self.u16(rset + 2 + 2 * r) as usize;
                    let nb = self.u16(at) as usize;
                    let back = at + 2;
                    at = back + 2 * nb;
                    let ni = self.u16(at) as usize;
                    let inp = at + 2;
                    at = inp + 2 * ni.saturating_sub(1);
                    let na = self.u16(at) as usize;
                    let ahd = at + 2;
                    at = ahd + 2 * na;
                    let nrec = self.u16(at) as usize;
                    let same = |table: usize, value: u16, g: u16| if format == 2 { self.class(table, g) == value } else { g == value };
                    if !behind(nb, &|k, g| same(back_cls, self.u16(back + 2 * k), g)) {
                        continue;
                    }
                    let Some(pos) = input(ni.max(1), &|k, g| same(in_cls, self.u16(inp + 2 * (k - 1)), g)) else { continue };
                    if !ahead(*pos.last().unwrap_or(&i), na, &|k, g| same(ahead_cls, self.u16(ahd + 2 * k), g)) {
                        continue;
                    }
                    return Some((pos, records(at + 2, nrec)));
                }
                None
            }
            (6, 3) => {
                let mut at = s + 2;
                let nb = self.u16(at) as usize;
                let back = at + 2;
                at = back + 2 * nb;
                let ni = self.u16(at) as usize;
                let inp = at + 2;
                at = inp + 2 * ni;
                let na = self.u16(at) as usize;
                let ahd = at + 2;
                at = ahd + 2 * na;
                let nrec = self.u16(at) as usize;
                let cov = |base: usize, k: usize| s + self.u16(base + 2 * k) as usize;
                if ni == 0 || self.coverage(cov(inp, 0), g).is_none() {
                    return None;
                }
                if !behind(nb, &|k, g| self.coverage(cov(back, k), g).is_some()) {
                    return None;
                }
                let pos = input(ni, &|k, g| self.coverage(cov(inp, k), g).is_some())?;
                if !ahead(*pos.last().unwrap_or(&i), na, &|k, g| self.coverage(cov(ahd, k), g).is_some()) {
                    return None;
                }
                Some((pos, records(at + 2, nrec)))
            }
            _ => None,
        }
    }

    // ---- positioning

    fn position(&self, buf: &mut [G], li: u16, rtl: bool) {
        let Some((ty, flag, set, subs)) = self.lookup(self.font.gpos, li) else { return };
        let mut i = 0;
        while i < buf.len() {
            if self.skips(buf[i].id, flag, set) {
                i += 1;
                continue;
            }
            match self.pos_at(buf, i, ty, flag, set, &subs, 0, rtl) {
                Some(next) => i = next.max(i + 1),
                None => i += 1,
            }
        }
    }

    /// The size of a value record of `format`, in bytes.
    fn value_size(format: u16) -> usize {
        2 * format.count_ones() as usize
    }

    /// How much a device table moves a value at the font's instance (a variation index into
    /// GDEF's variation store; size-by-size device tables are for pixels on screen, left out).
    fn device(&self, at: usize) -> f64 {
        if !self.font.varied() || self.u16(at + 4) != 0x8000 {
            return 0.0;
        }
        let gdef = self.font.gdef;
        if gdef == 0 || u32_at(self.d, gdef) < 0x0001_0003 {
            return 0.0;
        }
        let off = u32_at(self.d, gdef + 14) as usize;
        if off == 0 {
            return 0.0;
        }
        self.font.item_delta(gdef + off, self.u16(at) as usize, self.u16(at + 2) as usize)
    }

    /// Apply a value record at `at` (of a subtable at `base`) to a glyph: x and y placement, x
    /// advance, each moved at a variable font's instance.
    fn apply_value(&self, g: &mut G, format: u16, at: usize, base: usize) {
        let mut r = at;
        let mut take = |bit: u16| -> f64 {
            if format & bit != 0 {
                let v = self.i16(r) as f64;
                r += 2;
                v
            } else {
                0.0
            }
        };
        let (mut xp, mut yp, mut xa, _ya) = (take(1), take(2), take(4), take(8));
        let mut dev = |bit: u16| -> f64 {
            if format & bit != 0 {
                let off = self.u16(r) as usize;
                r += 2;
                if off != 0 { self.device(base + off) } else { 0.0 }
            } else {
                0.0
            }
        };
        xp += dev(0x10);
        yp += dev(0x20);
        xa += dev(0x40);
        g.dx += xp;
        g.dy += yp;
        g.advance += xa;
    }

    fn anchor(&self, at: usize) -> (f64, f64) {
        let (mut x, mut y) = (self.i16(at + 2) as f64, self.i16(at + 4) as f64);
        if self.u16(at) == 3 {
            let (xo, yo) = (self.u16(at + 6) as usize, self.u16(at + 8) as usize);
            if xo != 0 {
                x += self.device(at + xo);
            }
            if yo != 0 {
                y += self.device(at + yo);
            }
        }
        (x, y)
    }

    #[allow(clippy::too_many_arguments)]
    fn pos_at(&self, buf: &mut [G], i: usize, ty: u16, flag: u16, set: u16, subs: &[usize], depth: u32, rtl: bool) -> Option<usize> {
        for &s in subs {
            let format = self.u16(s);
            let cov = s + self.u16(s + 2) as usize;
            let g = buf[i].id;
            match ty {
                1 => {
                    let Some(k) = self.coverage(cov, g) else { continue };
                    let vf = self.u16(s + 4);
                    let at = if format == 1 { s + 6 } else { s + 8 + Self::value_size(vf) * k };
                    self.apply_value(&mut buf[i], vf, at, s);
                    return Some(i + 1);
                }
                2 => {
                    let Some(k) = self.coverage(cov, g) else { continue };
                    let j = self.step(buf, i, &Walk::any(flag, set), true, &|_| true)?;
                    let (vf1, vf2) = (self.u16(s + 4), self.u16(s + 6));
                    let (s1, s2) = (Self::value_size(vf1), Self::value_size(vf2));
                    let second = buf[j].id;
                    let found = if format == 1 {
                        let pset = s + self.u16(s + 10 + 2 * k) as usize;
                        let n = self.u16(pset) as usize;
                        let rec = 2 + s1 + s2;
                        (0..n).map(|p| pset + 2 + rec * p).find(|&r| self.u16(r) == second).map(|r| (r + 2, r + 2 + s1))
                    } else {
                        let (c1, c2) = (self.class(s + self.u16(s + 8) as usize, g) as usize, self.class(s + self.u16(s + 10) as usize, second) as usize);
                        let (n1, n2) = (self.u16(s + 12) as usize, self.u16(s + 14) as usize);
                        (c1 < n1 && c2 < n2).then(|| {
                            let r = s + 16 + (c1 * n2 + c2) * (s1 + s2);
                            (r, r + s1)
                        })
                    };
                    let Some((v1, v2)) = found else { continue };
                    // format 1's value records' devices are measured from the pair set
                    let base = if format == 1 { s + self.u16(s + 10 + 2 * k) as usize } else { s };
                    self.apply_value(&mut buf[i], vf1, v1, base);
                    self.apply_value(&mut buf[j], vf2, v2, base);
                    return Some(if vf2 != 0 { j + 1 } else { j });
                }
                3 => {
                    // this glyph's exit joined to the next one's entry
                    let Some(k) = self.coverage(cov, g) else { continue };
                    let n = self.u16(s + 4) as usize;
                    if k >= n {
                        continue;
                    }
                    let exit_off = self.u16(s + 6 + 4 * k + 2) as usize;
                    if exit_off == 0 {
                        continue;
                    }
                    let j = self.step(buf, i, &Walk::any(flag, set), true, &|_| true)?;
                    let Some(kj) = self.coverage(cov, buf[j].id).filter(|kj| *kj < n) else { continue };
                    let entry_off = self.u16(s + 6 + 4 * kj) as usize;
                    if entry_off == 0 {
                        continue;
                    }
                    let (ex, ey) = self.anchor(s + exit_off);
                    let (nx, ny) = self.anchor(s + entry_off);
                    if rtl {
                        // seen right to left: this glyph's exit is on its left, at the next one's entry
                        let d = ex + buf[i].dx;
                        buf[i].advance -= d;
                        buf[i].dx -= d;
                        buf[j].advance = nx + buf[j].dx;
                    } else {
                        buf[i].advance = ex + buf[i].dx;
                        let d = nx + buf[j].dx;
                        buf[j].advance -= d;
                        buf[j].dx -= d;
                    }
                    // the next glyph hangs on this one (or, flagged right to left, this on the next)
                    if flag & 1 != 0 {
                        buf[i].chain = Some((j, ny - ey));
                    } else {
                        buf[j].chain = Some((i, ey - ny));
                    }
                    return Some(j);
                }
                4..=6 => {
                    // the mark's class and anchor, then the glyph it sits on and that glyph's anchor
                    let Some(mk) = self.coverage(cov, g) else { continue };
                    let base_cov = s + self.u16(s + 4) as usize;
                    let classes = self.u16(s + 6) as usize;
                    let marks = s + self.u16(s + 8) as usize;
                    let bases = s + self.u16(s + 10) as usize;
                    let rec = marks + 2 + 4 * mk;
                    let (class, m_anchor) = (self.u16(rec) as usize, marks + self.u16(rec + 2) as usize);
                    // on a base (or ligature): the glyph before that is not a mark; on a mark: the one just before
                    let target = if ty == 6 {
                        let j = self.step(buf, i, &Walk::any(flag & !IGNORE_MARKS, set), false, &|_| true)?;
                        self.is_mark(buf[j].id).then_some(j)?
                    } else {
                        (0..i).rev().find(|&j| !self.is_mark(buf[j].id))?
                    };
                    let Some(bk) = self.coverage(base_cov, buf[target].id) else { continue };
                    let b_anchor_off = if ty == 5 {
                        // a ligature: the anchor of its last component
                        let lig = bases + self.u16(bases + 2 + 2 * bk) as usize;
                        let comps = self.u16(lig) as usize;
                        if comps == 0 {
                            continue;
                        }
                        let r = lig + 2 + 2 * ((comps - 1) * classes + class);
                        let off = self.u16(r) as usize;
                        if off == 0 {
                            continue;
                        }
                        lig + off
                    } else {
                        let off = self.u16(bases + 2 + 2 * (bk * classes + class)) as usize;
                        if off == 0 {
                            continue;
                        }
                        bases + off
                    };
                    let (bx, by) = self.anchor(b_anchor_off);
                    let (mx, my) = self.anchor(m_anchor);
                    buf[i].attach = Some((target, bx - mx, by - my));
                    buf[i].dx = 0.0;
                    buf[i].dy = 0.0;
                    return Some(i + 1);
                }
                7 | 8 => {
                    let ctx_ty = if ty == 7 { 5 } else { 6 };
                    let w = Walk::any(flag, set);
                    if let Some((positions, records)) = self.context(buf, i, ctx_ty, s, &w, &w) {
                        for (seq, lookup) in records {
                            if depth > 8 || seq >= positions.len() {
                                continue;
                            }
                            if let Some((t2, f2, s2, subs2)) = self.lookup(self.font.gpos, lookup)
                                && !self.skips(buf[positions[seq]].id, f2, s2)
                            {
                                self.pos_at(buf, positions[seq], t2, f2, s2, &subs2, depth + 1, rtl);
                            }
                        }
                        return Some(positions.last().map_or(i + 1, |l| l + 1));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Kerning from the old `kern` table (its first subtable of pairs).
    fn old_kern(&self, buf: &mut [G]) {
        let k = self.font.kern;
        if self.u16(k) != 0 || self.u16(k + 2) == 0 {
            return;
        }
        let sub = k + 4;
        if self.u16(sub + 4) >> 8 != 0 {
            return;
        }
        let n = self.u16(sub + 6) as usize;
        let pairs = sub + 14;
        let value = |a: u16, b: u16| -> Option<f64> {
            let key = (a as u32) << 16 | b as u32;
            let (mut lo, mut hi) = (0, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let r = pairs + 6 * mid;
                let here = u32_at(self.d, r);
                match here.cmp(&key) {
                    std::cmp::Ordering::Equal => return Some(self.i16(r + 4) as f64),
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                }
            }
            None
        };
        for i in 0..buf.len().saturating_sub(1) {
            if self.is_mark(buf[i].id) {
                continue;
            }
            if let Some(j) = (i + 1..buf.len()).find(|&j| !self.is_mark(buf[j].id))
                && let Some(v) = value(buf[i].id, buf[j].id)
            {
                buf[i].advance += v;
            }
        }
    }
}

#[cfg(test)]
mod tests;
