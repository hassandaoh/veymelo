//! Shaping for the scripts of India (Devanagari, Bengali, Gurmukhi, Gujarati, Oriya, Tamil,
//! Telugu, Kannada, Malayalam), as OpenType's Indic shaping lays down.
//!
//! Vowel signs in two parts are taken apart first (and a consonant written with its nukta kept
//! whole where the font has it). The text is cut into syllables, and each syllable's characters
//! are put in the order their glyphs are formed: its base consonant found (the last consonant
//! without a form below or after it, as the font itself says), vowel signs drawn before it
//! moved there, a Ra and virama that start it marked to become a reph. The font's forms then
//! come one feature at a time, each only on the glyphs it is for (nukta forms, akhands, reph,
//! rakar, pre-base forms, forms below and above, half forms, forms after; the half forms on the
//! consonants before the base, the forms below and after on those after it). Then the reph and
//! the pre-base forms move to where they are drawn, and a vowel sign before the base moves up
//! to it past the half forms, before the font's other forms and the usual rules. A zero-width
//! non-joiner keeps a half form from forming; a joiner asks for one.
//!
//! Fonts with the newer rules (`dev2`, `bng2`…) and the older ones (`deva`, `beng`…) are both
//! shaped, as the shaping engines of the systems shape them.

mod table;

use super::{Apply, G, LIGATED, Layout, MULTIPLIED, Options, SUBSTITUTED, merge};
use crate::Font;

/// The scripts shaped this way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indic {
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Oriya,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
}

// Categories: what a character is in a syllable.
const X: u8 = 0;
const C: u8 = 1;
const V: u8 = 2;
const N: u8 = 3;
const H: u8 = 4;
const ZWNJ: u8 = 5;
const ZWJ: u8 = 6;
const M: u8 = 7;
const SM: u8 = 8;
const A: u8 = 9;
const PLACEHOLDER: u8 = 10;
const DOTTED: u8 = 11;
const RS: u8 = 12;
const REPHA: u8 = 14;
const RA: u8 = 15;
const CM: u8 = 16;
const SYMBOL: u8 = 17;
const CS: u8 = 18;

// Positions: where a glyph goes in its syllable, in this order.
const START: u8 = 0;
const RA_TO_BECOME_REPH: u8 = 1;
const PRE_M: u8 = 2;
const PRE_C: u8 = 3;
const BASE_C: u8 = 4;
const AFTER_MAIN: u8 = 5;
const BEFORE_SUB: u8 = 7;
const BELOW_C: u8 = 8;
const AFTER_SUB: u8 = 9;
const BEFORE_POST: u8 = 10;
const POST_C: u8 = 11;
const AFTER_POST: u8 = 12;
const SMVD: u8 = 14;
const END: u8 = 15;

// Kinds of syllable.
const CONSONANT: u8 = 0;
const VOWEL: u8 = 1;
const STANDALONE: u8 = 2;
const SYMBOL_CLUSTER: u8 = 3;
const BROKEN: u8 = 4;
const OTHER: u8 = 5;

/// The features applied one at a time after the first reordering, each with whether it applies
/// to every glyph (else only to those the reordering marks for it).
const BASIC: [(&[u8; 4], bool); 11] = [
    (b"nukt", true),
    (b"akhn", true),
    (b"rphf", false),
    (b"rkrf", true),
    (b"pref", false),
    (b"blwf", false),
    (b"abvf", false),
    (b"half", false),
    (b"pstf", false),
    (b"vatu", true),
    (b"cjct", true),
];
/// The features applied together after the final reordering.
const LATER: [(&[u8; 4], bool); 6] = [(b"init", false), (b"pres", true), (b"abvs", true), (b"blws", true), (b"psts", true), (b"haln", true)];

// Each feature's bit in a glyph's mask: BASIC's in order, then LATER's.
const RPHF: u32 = 1 << 2;
const PREF: u32 = 1 << 4;
const BLWF: u32 = 1 << 5;
const ABVF: u32 = 1 << 6;
const HALF: u32 = 1 << 7;
const PSTF: u32 = 1 << 8;
const INIT: u32 = 1 << 11;

/// The bits of the features that apply to every glyph.
fn global() -> u32 {
    let basic = BASIC.iter().enumerate().filter(|(_, f)| f.1).map(|(k, _)| 1u32 << k);
    let later = LATER.iter().enumerate().filter(|(_, f)| f.1).map(|(k, _)| 1u32 << (BASIC.len() + k));
    basic.chain(later).fold(0, |a, b| a | b)
}

/// How a reph is written: Ra and virama (`Implicit`), Ra, virama and a joiner (`Explicit`), or
/// a character of its own (`Logical`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reph {
    Implicit,
    Explicit,
    Logical,
}

/// What shaping each script needs: its virama, where its reph goes, how a reph is written,
/// whether forms below come only after the base, and its two script tags (older, newer).
struct Config {
    virama: char,
    reph_pos: u8,
    reph: Reph,
    below_after_only: bool,
    old_tag: &'static [u8; 4],
    new_tag: &'static [u8; 4],
}

impl Indic {
    /// The script of a character of these scripts' blocks.
    pub(super) fn of(c: char) -> Option<Indic> {
        Some(match c as u32 {
            0x0900..=0x097F | 0xA8E0..=0xA8FF => Indic::Devanagari,
            0x0980..=0x09FF => Indic::Bengali,
            0x0A00..=0x0A7F => Indic::Gurmukhi,
            0x0A80..=0x0AFF => Indic::Gujarati,
            0x0B00..=0x0B7F => Indic::Oriya,
            0x0B80..=0x0BFF => Indic::Tamil,
            0x0C00..=0x0C7F => Indic::Telugu,
            0x0C80..=0x0CFF => Indic::Kannada,
            0x0D00..=0x0D7F => Indic::Malayalam,
            _ => return None,
        })
    }

    fn config(self) -> Config {
        use Reph::*;
        let (virama, reph_pos, reph, below_after_only, old_tag, new_tag) = match self {
            Indic::Devanagari => ('\u{094D}', BEFORE_POST, Implicit, false, b"deva", b"dev2"),
            Indic::Bengali => ('\u{09CD}', AFTER_SUB, Implicit, false, b"beng", b"bng2"),
            Indic::Gurmukhi => ('\u{0A4D}', BEFORE_SUB, Implicit, false, b"guru", b"gur2"),
            Indic::Gujarati => ('\u{0ACD}', BEFORE_POST, Implicit, false, b"gujr", b"gjr2"),
            Indic::Oriya => ('\u{0B4D}', AFTER_MAIN, Implicit, false, b"orya", b"ory2"),
            Indic::Tamil => ('\u{0BCD}', AFTER_POST, Implicit, false, b"taml", b"tml2"),
            Indic::Telugu => ('\u{0C4D}', AFTER_POST, Explicit, true, b"telu", b"tel2"),
            Indic::Kannada => ('\u{0CCD}', AFTER_POST, Implicit, true, b"knda", b"knd2"),
            Indic::Malayalam => ('\u{0D4D}', AFTER_MAIN, Logical, false, b"mlym", b"mlm2"),
        };
        Config { virama, reph_pos, reph, below_after_only, old_tag, new_tag }
    }

    /// Its newer script tag.
    pub(super) fn tag(self) -> &'static [u8; 4] {
        self.config().new_tag
    }

    /// The script tag a table's rules are under: the newer one where the table has it, else
    /// the older.
    pub(super) fn tag_in(self, layout: &Layout, table: usize) -> [u8; 4] {
        let cfg = self.config();
        if layout.has_script(table, cfg.new_tag) { *cfg.new_tag } else { *cfg.old_tag }
    }
}

/// A character's category and position.
fn properties(c: char) -> (u8, u8) {
    let u = c as u32;
    let v = match u {
        0x0900..=0x0D7F => table::BLOCKS[(u - 0x0900) as usize],
        0x1CD0..=0x1CFF => table::VEDIC[(u - 0x1CD0) as usize],
        0xA8E0..=0xA8FF => table::EXTENDED[(u - 0xA8E0) as usize],
        0x200C => ZWNJ as u16 | (END as u16) << 8,
        0x200D => ZWJ as u16 | (END as u16) << 8,
        0x00A0 | 0x2010 | 0x2011 | 0x2015 | 0x2022 | 0x25FB..=0x25FE => PLACEHOLDER as u16 | (BASE_C as u16) << 8,
        0x25CC => DOTTED as u16 | (BASE_C as u16) << 8,
        // Grantha marks Tamil may use
        0x11301..=0x11303 => SM as u16 | (SMVD as u16) << 8,
        0x1133B | 0x1133C => N as u16 | (END as u16) << 8,
        _ => X as u16 | (END as u16) << 8,
    };
    ((v & 0xFF) as u8, (v >> 8) as u8)
}

/// A letter, mark or format character (one a vowel sign at a word's start cannot follow).
fn is_letter(c: char) -> bool {
    c.is_alphabetic() || properties(c).0 != X
}

fn combining_class(c: char) -> u8 {
    let u = c as u32;
    table::COMBINING.binary_search_by_key(&u, |e| e.0).map_or(0, |k| table::COMBINING[k].1)
}

/// `c` taken apart into what the font has glyphs for, when it has them.
fn parts(font: &Font, c: char) -> Option<Vec<char>> {
    // kept whole however they are written
    if matches!(c, '\u{0931}' | '\u{09DC}' | '\u{09DD}' | '\u{0B94}') {
        return None;
    }
    let u = c as u32;
    let k = table::DECOMPOSED.binary_search_by_key(&u, |e| e.0).ok()?;
    let (a, b) = (char::from_u32(table::DECOMPOSED[k].1)?, char::from_u32(table::DECOMPOSED[k].2)?);
    if font.glyph(b) == 0 {
        return None;
    }
    match parts(font, a) {
        Some(mut first) => {
            first.push(b);
            Some(first)
        }
        None => (font.glyph(a) != 0).then(|| vec![a, b]),
    }
}

/// A consonant and nukta (or a letter and length mark) the font has as one character.
fn composed(font: &Font, a: char, b: char) -> Option<char> {
    let c = match (a, b) {
        ('\u{0928}', '\u{093C}') => '\u{0929}',
        ('\u{0930}', '\u{093C}') => '\u{0931}',
        ('\u{0933}', '\u{093C}') => '\u{0934}',
        ('\u{09AF}', '\u{09BC}') => '\u{09DF}',
        ('\u{0B92}', '\u{0BD7}') => '\u{0B94}',
        _ => return None,
    };
    (font.glyph(c) != 0).then_some(c)
}

/// The text as Indic shaping expects it: characters taken apart where the font has their parts
/// (two-part vowel signs always), marks in their canonical order, a consonant and its nukta made
/// one where the font has that character. Each with the index it came from.
pub(super) fn prepare(font: &Font, text: &[char], first: usize) -> Vec<(char, usize)> {
    let mut out: Vec<(char, usize)> = Vec::with_capacity(text.len() + 4);
    for (i, &c) in text.iter().enumerate() {
        match parts(font, c) {
            Some(p) => out.extend(p.into_iter().map(|c| (c, first + i))),
            None => out.push((c, first + i)),
        }
    }
    order_marks(&mut out, combining_class);
    let mut k = 1;
    while k < out.len() {
        match composed(font, out[k - 1].0, out[k].0) {
            Some(c) => {
                out[k - 1].0 = c;
                out.remove(k);
            }
            None => k += 1,
        }
    }
    out
}

/// A pattern of categories (the syllables of OpenType's Indic, Khmer and Myanmar shaping),
/// matched by every place it can end.
pub(super) enum Pat {
    One(u64),
    Seq(Vec<Pat>),
    Alt(Vec<Pat>),
    Opt(Box<Pat>),
    Many(Box<Pat>),
}

pub(super) fn one(cats: &[u8]) -> Pat {
    Pat::One(cats.iter().fold(0, |m, &c| m | 1 << c))
}

pub(super) fn opt(p: Pat) -> Pat {
    Pat::Opt(Box::new(p))
}

pub(super) fn many(p: Pat) -> Pat {
    Pat::Many(Box::new(p))
}

impl Pat {
    /// Where it can end, starting from each of `from`.
    fn ends(&self, cats: &[u8], from: Vec<usize>) -> Vec<usize> {
        let mut out = match self {
            Pat::One(set) => from.into_iter().filter(|&i| i < cats.len() && set >> cats[i] & 1 != 0).map(|i| i + 1).collect(),
            Pat::Seq(ps) => ps.iter().fold(from, |at, p| p.ends(cats, at)),
            Pat::Alt(ps) => ps.iter().flat_map(|p| p.ends(cats, from.clone())).collect(),
            Pat::Opt(p) => {
                let mut out = p.ends(cats, from.clone());
                out.extend(from);
                out
            }
            Pat::Many(p) => {
                let mut all = from.clone();
                let mut edge = from;
                loop {
                    let next: Vec<usize> = p.ends(cats, edge).into_iter().filter(|e| !all.contains(e)).collect();
                    if next.is_empty() {
                        break all;
                    }
                    all.extend(&next);
                    edge = next;
                }
            }
        };
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The kinds of syllable, in the order they are tried (the longest match wins; on a tie, the
/// first).
fn syllable_patterns() -> Vec<(Pat, u8)> {
    use Pat::{Alt, Seq};
    let c = || one(&[C, RA]);
    let n = || Seq(vec![opt(Seq(vec![opt(one(&[ZWNJ])), one(&[RS])])), opt(Seq(vec![one(&[N]), opt(one(&[N]))]))]);
    let z = || one(&[ZWJ, ZWNJ]);
    let reph = || Alt(vec![Seq(vec![one(&[RA]), one(&[H])]), one(&[REPHA])]);
    let cn = || Seq(vec![c(), opt(one(&[ZWJ])), opt(n())]);
    let matra_group = || Seq(vec![many(z()), one(&[M]), opt(one(&[N])), opt(one(&[H]))]);
    let syllable_tail = || Seq(vec![opt(Seq(vec![opt(z()), one(&[SM]), opt(one(&[SM])), opt(one(&[ZWNJ]))])), many(one(&[A]))]);
    let halant_group = || Seq(vec![opt(z()), one(&[H]), opt(Seq(vec![one(&[ZWJ]), opt(one(&[N]))]))]);
    let final_halant_group = || Alt(vec![halant_group(), Seq(vec![one(&[H]), one(&[ZWNJ])])]);
    let tail = || Seq(vec![many(Seq(vec![halant_group(), cn()])), opt(one(&[CM])), Alt(vec![final_halant_group(), many(matra_group())]), syllable_tail()]);
    vec![
        (Seq(vec![opt(one(&[REPHA, CS])), cn(), tail()]), CONSONANT),
        (Seq(vec![opt(reph()), one(&[V]), opt(n()), Alt(vec![one(&[ZWJ]), tail()])]), VOWEL),
        (Seq(vec![Alt(vec![Seq(vec![opt(one(&[REPHA, CS])), one(&[PLACEHOLDER])]), Seq(vec![opt(reph()), one(&[DOTTED])])]), opt(n()), tail()]), STANDALONE),
        (Seq(vec![one(&[SYMBOL]), opt(one(&[N])), syllable_tail()]), SYMBOL_CLUSTER),
        (Seq(vec![opt(reph()), opt(n()), tail()]), BROKEN),
    ]
}

/// The syllables of a run of categories, by `patterns` (the longest match; on a tie, the first;
/// else one character of kind `other`): where each starts and ends, and its kind.
pub(super) fn cut(cats: &[u8], patterns: &[(Pat, u8)], other: u8) -> Vec<(usize, usize, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < cats.len() {
        let mut best = (i + 1, other, 0);
        for (p, kind) in patterns {
            if let Some(&e) = p.ends(cats, vec![i]).last()
                && e > i
                && e - i > best.2
            {
                best = (e, *kind, e - i);
            }
        }
        out.push((i, best.0, best.1));
        i = best.0;
    }
    out
}

fn syllables(cats: &[u8]) -> Vec<(usize, usize, u8)> {
    cut(cats, &syllable_patterns(), OTHER)
}

/// Each glyph marked with the syllable it is in (counted from 1); the kind of each.
pub(super) fn mark_syllables(buf: &mut [G], found: Vec<(usize, usize, u8)>, other: u8) -> Vec<u8> {
    let mut kinds = vec![other];
    for (k, (a, b, kind)) in found.into_iter().enumerate() {
        for g in &mut buf[a..b] {
            g.syllable = (k + 1).min(u16::MAX as usize) as u16;
        }
        kinds.push(kind);
    }
    kinds
}

/// A dotted circle (glyph `dotted`, of category `cat` and position `pos`) put at the start of
/// each syllable of kind `broken` (after a reph of category `repha`), for what has nothing to
/// sit on.
pub(super) fn dotted_circles(buf: &mut Vec<G>, kinds: &[u8], broken: u8, dotted: u16, cat: u8, pos: u8, repha: Option<u8>) {
    if dotted == 0 {
        return;
    }
    let mut i = 0;
    let mut last = 0;
    while i < buf.len() {
        let s = buf[i].syllable;
        if s != last && kinds.get(s as usize) == Some(&broken) {
            last = s;
            let like = buf[i].clone();
            while i < buf.len() && buf[i].syllable == s && Some(buf[i].cat) == repha {
                i += 1;
            }
            buf.insert(i, G { id: dotted, cat, pos, props: 0, joiner: 0, ..like });
        }
        i += 1;
    }
}

/// The glyphs from `start` to `end` in order of `key`, each that moves back sharing a cluster
/// with those it moves past.
pub(super) fn sort_merging(buf: &mut [G], start: usize, end: usize, key: impl Fn(&G) -> u8) {
    for i in start + 1..end {
        let mut j = i;
        while j > start && key(&buf[j - 1]) > key(&buf[i]) {
            j -= 1;
        }
        if j < i {
            merge(buf, j, i + 1);
            buf[j..=i].rotate_right(1);
        }
    }
}

/// Marks in their canonical order (by combining class, `class`).
pub(super) fn order_marks(out: &mut [(char, usize)], class: impl Fn(char) -> u8) {
    let mut i = 0;
    while i < out.len() {
        let j = (i..out.len()).find(|&k| class(out[k].0) == 0).unwrap_or(out.len());
        if j > i + 1 {
            out[i..j].sort_by_key(|(c, _)| class(*c));
        }
        i = j.max(i + 1);
    }
}

fn is_consonant(g: &G) -> bool {
    matches!(g.cat, C | CS | RA | CM | V | PLACEHOLDER | DOTTED)
}

fn is_joiner(g: &G) -> bool {
    matches!(g.cat, ZWJ | ZWNJ)
}

fn is_halant(g: &G) -> bool {
    g.cat == H
}

/// Made of several by a ligature, and not split since.
fn ligated_only(g: &G) -> bool {
    g.props & LIGATED != 0 && g.props & MULTIPLIED == 0
}

/// The start and end of each syllable of the glyphs.
pub(super) fn ranges(buf: &[G]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut a = 0;
    for i in 1..=buf.len() {
        if i == buf.len() || buf[i].syllable != buf[a].syllable {
            out.push((a, i));
            a = i;
        }
    }
    out
}

/// What the shaping of one run knows of its font and script.
struct Plan<'a, 'f> {
    layout: &'a Layout<'f>,
    script: Indic,
    cfg: Config,
    old_spec: bool,
    /// Rules that look at glyphs around do not count in asking what the font would form.
    zero_context: bool,
    virama: u16,
    rphf: Vec<u16>,
    pref: Vec<u16>,
    blwf: Vec<u16>,
    pstf: Vec<u16>,
    vatu: Vec<u16>,
}

impl Plan<'_, '_> {
    fn would(&self, lookups: &[u16], glyphs: &[u16]) -> bool {
        !lookups.is_empty() && self.layout.would_substitute(lookups, glyphs, self.zero_context)
    }

    /// Where a consonant goes as the font forms it: below the base, after it, or the base.
    fn consonant_position(&self, consonant: u16) -> u8 {
        let (vc, cv) = ([self.virama, consonant], [consonant, self.virama]);
        let either = |lookups: &[u16]| self.would(lookups, &vc) || self.would(lookups, &cv);
        if either(&self.blwf) || either(&self.vatu) {
            BELOW_C
        } else if either(&self.pstf) || either(&self.pref) {
            POST_C
        } else {
            BASE_C
        }
    }

    /// The characters of one syllable put in the order their glyphs are formed, and each glyph
    /// marked with the features it is for.
    fn initial(&self, buf: &mut [G], start: usize, end: usize) {
        // Kannada's Ra, virama, joiner acts as Ra, joiner, virama
        if self.script == Indic::Kannada && start + 3 <= end && buf[start].cat == RA && buf[start + 1].cat == H && buf[start + 2].cat == ZWJ {
            merge(buf, start + 1, start + 3);
            buf.swap(start + 1, start + 2);
        }
        // a Ra and virama at the start become a reph when the font forms one (and the syllable
        // has another consonant); the base is then looked for after them
        let mut base = end;
        let mut has_reph = false;
        let mut limit = start;
        let explicit = self.cfg.reph == Reph::Explicit;
        if !self.rphf.is_empty() && start + 3 <= end && ((self.cfg.reph == Reph::Implicit && !is_joiner(&buf[start + 2])) || (explicit && buf[start + 2].cat == ZWJ)) {
            let glyphs = [buf[start].id, buf[start + 1].id, buf[start + 2].id];
            if self.would(&self.rphf, &glyphs[..2]) || (explicit && self.would(&self.rphf, &glyphs)) {
                limit += 2;
                while limit < end && is_joiner(&buf[limit]) {
                    limit += 1;
                }
                base = start;
                has_reph = true;
            }
        } else if self.cfg.reph == Reph::Logical && buf[start].cat == REPHA {
            limit += 1;
            while limit < end && is_joiner(&buf[limit]) {
                limit += 1;
            }
            base = start;
            has_reph = true;
        }
        // the base: from the end back, the first consonant with no form below or after it (forms
        // after come after those below), else the first consonant
        let mut i = end;
        let mut seen_below = false;
        loop {
            i -= 1;
            if is_consonant(&buf[i]) {
                if buf[i].pos != BELOW_C && (buf[i].pos != POST_C || seen_below) {
                    base = i;
                    break;
                }
                if buf[i].pos == BELOW_C {
                    seen_below = true;
                }
                base = i;
            } else if start < i && buf[i].cat == ZWJ && buf[i - 1].cat == H {
                // a virama and joiner ask for a half form: the base is after them
                break;
            }
            if i <= limit {
                break;
            }
        }
        if has_reph && base == start && limit - base <= 2 {
            // no other consonant: the Ra is the base
            has_reph = false;
        }
        for g in &mut buf[start..base] {
            g.pos = g.pos.min(PRE_C);
        }
        if base < end {
            buf[base].pos = BASE_C;
        }
        if has_reph {
            buf[start].pos = RA_TO_BECOME_REPH;
        }
        // older fonts: the first virama after the base goes after the last consonant
        if self.old_spec {
            let no_double = self.script == Indic::Kannada;
            if let Some(i) = (base + 1..end).find(|&i| buf[i].cat == H) {
                let mut j = end - 1;
                while j > i && !(is_consonant(&buf[j]) || (no_double && buf[j].cat == H)) {
                    j -= 1;
                }
                if buf[j].cat != H && j > i {
                    buf[i..=j].rotate_left(1);
                }
            }
        }
        // joiners, nuktas, viramas and medials go with what is before them
        let mut last_pos = START;
        for i in start..end {
            if matches!(buf[i].cat, ZWJ | ZWNJ | N | RS | CM | H) {
                buf[i].pos = last_pos;
                if buf[i].cat == H && buf[i].pos == PRE_M {
                    // a virama does not go with a vowel sign drawn before the base
                    if let Some(j) = (start..i).rev().find(|&j| buf[j].pos != PRE_M) {
                        buf[i].pos = buf[j].pos;
                    }
                }
            } else if buf[i].pos != SMVD {
                last_pos = buf[i].pos;
            }
        }
        // consonants after the base take what comes before them since the last consonant or
        // vowel sign
        let mut last = base;
        for i in base.saturating_add(1)..end {
            if is_consonant(&buf[i]) {
                for j in last + 1..i {
                    if buf[j].pos < SMVD {
                        buf[j].pos = buf[i].pos;
                    }
                }
                last = i;
            } else if buf[i].cat == M {
                last = i;
            }
        }
        // in order (each glyph keeping, for now, where it was)
        let syllable = buf[start].syllable;
        for (k, g) in buf[start..end].iter_mut().enumerate() {
            g.syllable = k as u16;
        }
        buf[start..end].sort_by_key(|g| g.pos);
        let (mut first_left, mut last_left) = (end, end);
        base = end;
        for (i, g) in buf.iter().enumerate().take(end).skip(start) {
            if g.pos == BASE_C {
                base = i;
                break;
            }
            if g.pos == PRE_M {
                if first_left == end {
                    first_left = i;
                }
                last_left = i;
            }
        }
        // several vowel signs before the base: in the order they were written, each with its marks
        if first_left < last_left {
            buf[first_left..=last_left].reverse();
            let mut i = first_left;
            for j in first_left..=last_left {
                if buf[j].cat == M {
                    buf[i..=j].reverse();
                    i = j + 1;
                }
            }
        }
        // what moved round after the base shares a cluster with where it went
        if self.old_spec || end - start > 127 {
            merge(buf, base, end);
        } else {
            let mut seen = vec![false; end - start];
            for i in base..end {
                if seen[i - start] {
                    continue;
                }
                let (mut lo, mut hi) = (i, i);
                let mut j = start + buf[i].syllable as usize;
                while j != i {
                    lo = lo.min(j);
                    hi = hi.max(j);
                    seen[j - start] = true;
                    j = start + buf[j].syllable as usize;
                }
                merge(buf, base.max(lo), hi + 1);
            }
        }
        for g in &mut buf[start..end] {
            g.syllable = syllable;
        }
        // the features each glyph is for
        if !self.rphf.is_empty() {
            for g in buf[start..end].iter_mut().take_while(|g| g.pos == RA_TO_BECOME_REPH) {
                g.mask |= RPHF;
            }
        }
        let before = HALF | if !self.old_spec && !self.cfg.below_after_only { BLWF } else { 0 };
        for g in &mut buf[start..base] {
            g.mask |= before;
        }
        if base < end {
            for g in &mut buf[base + 1..end] {
                g.mask |= BLWF | ABVF | PSTF;
            }
        }
        // older Devanagari fonts: a Ra and virama before the base may take the form below too
        if self.old_spec && self.script == Indic::Devanagari {
            for i in start..base.saturating_sub(1) {
                if buf[i].cat == RA && buf[i + 1].cat == H && (i + 2 == base || buf[i + 2].cat != ZWJ) {
                    buf[i].mask |= BLWF;
                    buf[i + 1].mask |= BLWF;
                }
            }
        }
        // the virama and Ra after the base that the font draws before it
        if !self.pref.is_empty() && base + 2 < end {
            for i in base + 1..end - 1 {
                if self.would(&self.pref, &[buf[i].id, buf[i + 1].id]) {
                    buf[i].mask |= PREF;
                    buf[i + 1].mask |= PREF;
                    break;
                }
            }
        }
        // a non-joiner keeps the consonant before it from its half form
        for i in start + 1..end {
            if buf[i].cat == ZWNJ {
                let mut j = i;
                loop {
                    j -= 1;
                    buf[j].mask &= !HALF;
                    if j <= start || is_consonant(&buf[j]) {
                        break;
                    }
                }
            }
        }
    }

    /// One syllable's reph and pre-base forms moved to where they are drawn, and a vowel sign
    /// before the base moved up to it, once the font's forms are made.
    fn finish(&self, buf: &mut [G], start: usize, end: usize, letter_before: bool) {
        // a virama the font joined into a glyph and split out again is still a virama
        if self.virama != 0 {
            for g in &mut buf[start..end] {
                if g.id == self.virama && g.props & LIGATED != 0 && g.props & MULTIPLIED != 0 {
                    g.cat = H;
                    g.props &= !(LIGATED | MULTIPLIED);
                }
            }
        }
        let mut try_pref = !self.pref.is_empty();
        // the base again
        let mut base = start;
        while base < end {
            if buf[base].pos >= BASE_C {
                if try_pref && base + 1 < end {
                    if let Some(i) = (base + 1..end).find(|&i| buf[i].mask & PREF != 0)
                        && !(buf[i].props & SUBSTITUTED != 0 && ligated_only(&buf[i]))
                    {
                        // a pre-base form that did not form: the base is around there
                        base = i;
                        while base < end && is_halant(&buf[base]) {
                            base += 1;
                        }
                        if base < end {
                            buf[base].pos = BASE_C;
                        }
                        try_pref = false;
                    }
                    if base == end {
                        break;
                    }
                }
                // Malayalam: past forms below that did not form
                if self.script == Indic::Malayalam {
                    let mut i = base + 1;
                    while i < end {
                        while i < end && is_joiner(&buf[i]) {
                            i += 1;
                        }
                        if i == end || !is_halant(&buf[i]) {
                            break;
                        }
                        i += 1;
                        while i < end && is_joiner(&buf[i]) {
                            i += 1;
                        }
                        if i < end && is_consonant(&buf[i]) && buf[i].pos == BELOW_C {
                            base = i;
                            buf[base].pos = BASE_C;
                        }
                        i += 1;
                    }
                }
                if start < base && buf[base].pos > BASE_C {
                    base -= 1;
                }
                break;
            }
            base += 1;
        }
        if base == end && start < base && buf[base - 1].cat == ZWJ {
            base -= 1;
        }
        if base < end {
            while start < base && matches!(buf[base].cat, N | H) {
                base -= 1;
            }
        }
        // a vowel sign before the base: after the last virama left standing before the base (one
        // a joiner follows does not count), so after the half forms
        if start + 1 < end && start < base {
            let mut new_pos = if base == end { base - 2 } else { base - 1 };
            if self.script != Indic::Malayalam && self.script != Indic::Tamil {
                loop {
                    while new_pos > start && !matches!(buf[new_pos].cat, M | H) {
                        new_pos -= 1;
                    }
                    if is_halant(&buf[new_pos]) && buf[new_pos].pos != PRE_M {
                        if new_pos + 1 < end && buf[new_pos + 1].cat == ZWJ && new_pos > start {
                            new_pos -= 1;
                            continue;
                        }
                    } else {
                        new_pos = start;
                    }
                    break;
                }
            }
            if start < new_pos && buf[new_pos].pos != PRE_M {
                let mut i = new_pos;
                while i > start {
                    if buf[i - 1].pos == PRE_M {
                        let old = i - 1;
                        if old < base && base <= new_pos {
                            base -= 1;
                        }
                        buf[old..=new_pos].rotate_left(1);
                        merge(buf, new_pos, end.min(base + 1));
                        new_pos -= 1;
                    }
                    i -= 1;
                }
            } else if let Some(i) = (start..base).find(|&i| buf[i].pos == PRE_M) {
                merge(buf, i, end.min(base + 1));
            }
        }
        // the reph, where the script draws it: written as Ra and virama, only if they formed one;
        // written as a character of its own, only if it did not join with anything
        if start + 1 < end && buf[start].pos == RA_TO_BECOME_REPH && ((buf[start].cat == REPHA) ^ ligated_only(&buf[start])) {
            let after_halant = |buf: &[G]| -> Option<usize> {
                let mut p = start + 1;
                while p < base && !is_halant(&buf[p]) {
                    p += 1;
                }
                (p < base && is_halant(&buf[p])).then(|| if p + 1 < base && is_joiner(&buf[p + 1]) { p + 1 } else { p })
            };
            let reph_pos = self.cfg.reph_pos;
            let found = if reph_pos == AFTER_POST {
                None
            } else if let Some(p) = after_halant(buf) {
                Some(p)
            } else if reph_pos == AFTER_MAIN {
                let mut p = base;
                while p + 1 < end && buf[p + 1].pos <= AFTER_MAIN {
                    p += 1;
                }
                (p < end).then_some(p)
            } else if reph_pos == AFTER_SUB {
                let mut p = base;
                while p + 1 < end && !matches!(buf[p + 1].pos, POST_C | AFTER_POST | SMVD) {
                    p += 1;
                }
                (p < end).then_some(p)
            } else {
                None
            };
            let new = found.or_else(|| after_halant(buf)).unwrap_or_else(|| {
                // at the end of the syllable, before its marks
                let mut p = end - 1;
                while p > start && buf[p].pos == SMVD {
                    p -= 1;
                }
                // and before a virama that follows a vowel sign, to go with the vowel sign
                if is_halant(&buf[p]) {
                    let mut i = base + 1;
                    while i < p {
                        if buf[i].cat == M {
                            p -= 1;
                        }
                        i += 1;
                    }
                }
                p
            });
            merge(buf, start, new + 1);
            buf[start..=new].rotate_left(1);
            if start < base && base <= new {
                base -= 1;
            }
        }
        // a pre-base form, where the vowel sign before the base would go
        if try_pref
            && base + 1 < end
            && let Some(i) = (base + 1..end).find(|&i| buf[i].mask & PREF != 0)
            && ligated_only(&buf[i])
        {
            let mut new_pos = base;
            if self.script != Indic::Malayalam && self.script != Indic::Tamil {
                while new_pos > start && !matches!(buf[new_pos - 1].cat, M | H) {
                    new_pos -= 1;
                }
            }
            if new_pos > start && is_halant(&buf[new_pos - 1]) && new_pos < end && is_joiner(&buf[new_pos]) {
                new_pos += 1;
            }
            merge(buf, new_pos, i + 1);
            buf[new_pos..=i].rotate_right(1);
        }
        // a vowel sign that starts a word takes its word-initial form
        if buf[start].pos == PRE_M && !letter_before {
            buf[start].mask |= INIT;
        }
    }
}

/// The substitutions of an Indic run, as the font's rules for its script say: the first
/// features, the reordering, the basic forms one by one, the final reordering, then the rest
/// (`first` and `usual` the features asked of every script before and after).
pub(super) fn substitute(layout: &Layout, buf: &mut Vec<G>, script: Indic, chars: &[(char, usize)], options: &Options, first: &[&[u8; 4]], usual: &[&[u8; 4]]) {
    let font = layout.font;
    let gsub = font.gsub;
    let cfg = script.config();
    let old_spec = !layout.has_script(gsub, cfg.new_tag);
    let tag = script.tag_in(layout, gsub);
    let feature = |t: &[u8; 4]| -> Vec<u16> {
        if !options.on(t, true) {
            return Vec::new();
        }
        layout.lookups(gsub, &tag, &[t]).into_iter().map(|l| l.0).collect()
    };
    let all = global();
    for (g, &(c, _)) in buf.iter_mut().zip(chars) {
        (g.cat, g.pos) = properties(c);
        g.mask = all;
    }
    let cats: Vec<u8> = buf.iter().map(|g| g.cat).collect();
    let kinds = mark_syllables(buf, syllables(&cats), OTHER);
    let kind_of = |g: &G| kinds.get(g.syllable as usize).copied().unwrap_or(OTHER);
    let per_syllable = Apply { per_syllable: true, ..Apply::default() };
    for (li, _) in layout.lookups(gsub, &tag, first) {
        layout.substitute(buf, li, &per_syllable);
    }
    let plan = Plan {
        layout,
        script,
        old_spec,
        zero_context: !old_spec && script != Indic::Malayalam,
        virama: font.glyph(cfg.virama),
        rphf: feature(b"rphf"),
        pref: feature(b"pref"),
        blwf: feature(b"blwf"),
        pstf: feature(b"pstf"),
        vatu: feature(b"vatu"),
        cfg,
    };
    // where each consonant goes, as the font forms it
    if plan.virama != 0 {
        for g in buf.iter_mut().filter(|g| g.pos == BASE_C) {
            g.pos = plan.consonant_position(g.id);
        }
    }
    // a syllable that is not one (a vowel sign alone…) is set on a dotted circle, after any reph
    dotted_circles(buf, &kinds, BROKEN, font.glyph('\u{25CC}'), DOTTED, END, Some(REPHA));
    for (a, b) in ranges(buf) {
        if matches!(kind_of(&buf[a]), CONSONANT | VOWEL | STANDALONE | BROKEN) {
            plan.initial(buf, a, b);
        }
    }
    for (k, (t, _)) in BASIC.iter().enumerate() {
        let ap = Apply { mask: 1 << k, per_syllable: true, manual_zwj: true, manual_zwnj: true, form: 0 };
        for li in feature(t) {
            layout.substitute(buf, li, &ap);
        }
    }
    let letter = |cluster: usize| chars.iter().find(|c| c.1 == cluster).is_some_and(|c| is_letter(c.0));
    for (a, b) in ranges(buf) {
        let before = a > 0 && letter(buf[a - 1].cluster);
        plan.finish(buf, a, b, before);
    }
    // the remaining forms and the usual rules, in the order the font lists their lookups
    let mut later: Vec<(u16, Apply)> = Vec::new();
    for (k, (t, _)) in LATER.iter().enumerate() {
        for li in feature(t) {
            later.push((li, Apply { mask: 1 << (BASIC.len() + k), per_syllable: true, manual_zwj: true, manual_zwnj: true, form: 0 }));
        }
    }
    later.extend(layout.lookups(gsub, &tag, usual).into_iter().map(|(li, _)| (li, Apply::default())));
    later.sort_by_key(|l| l.0);
    let mut k = 0;
    while k < later.len() {
        let (li, mut ap) = later[k];
        // a lookup two features share: on the glyphs either is for
        while k + 1 < later.len() && later[k + 1].0 == li {
            let other = later[k + 1].1;
            ap.mask = if ap.mask == 0 || other.mask == 0 { 0 } else { ap.mask | other.mask };
            ap.per_syllable &= other.per_syllable;
            ap.manual_zwj |= other.manual_zwj;
            ap.manual_zwnj |= other.manual_zwnj;
            k += 1;
        }
        layout.substitute(buf, li, &ap);
        k += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cuts(text: &str) -> Vec<(String, u8)> {
        let chars: Vec<char> = text.chars().collect();
        let cats: Vec<u8> = chars.iter().map(|&c| properties(c).0).collect();
        syllables(&cats).into_iter().map(|(a, b, kind)| (chars[a..b].iter().collect(), kind)).collect()
    }

    #[test]
    fn characters_sort_as_their_shaping_does() {
        assert_eq!(properties('क'), (C, BASE_C));
        assert_eq!(properties('र'), (RA, BASE_C));
        assert_eq!(properties('ि'), (M, PRE_M));
        assert_eq!(properties('ु'), (M, AFTER_SUB));
        assert_eq!(properties('्'), (H, BELOW_C));
        assert_eq!(properties('ं'), (SM, SMVD));
        // Bengali's vowel signs to the right go after the forms after the base
        assert_eq!(properties('া'), (M, AFTER_POST));
        // Malayalam's dot reph is a reph of its own
        assert_eq!(properties('\u{0D4E}').0, REPHA);
        assert_eq!(properties('\u{25CC}').0, DOTTED);
        assert_eq!(properties('a'), (X, END));
    }

    #[test]
    fn text_is_cut_into_syllables() {
        let s = |t: &str, k: u8| (t.to_string(), k);
        assert_eq!(cuts("नमस्ते"), vec![s("न", CONSONANT), s("म", CONSONANT), s("स्ते", CONSONANT)]);
        // a reph and a conjunct in one syllable; a vowel with its sign
        assert_eq!(cuts("र्क्षि"), vec![s("र्क्षि", CONSONANT)]);
        assert_eq!(cuts("ऋषि"), vec![s("ऋ", VOWEL), s("षि", CONSONANT)]);
        // a virama and non-joiner end a syllable; a lone vowel sign is not one
        assert_eq!(cuts("क्\u{200C}ष"), vec![s("क्\u{200C}", CONSONANT), s("ष", CONSONANT)]);
        assert_eq!(cuts("ि"), vec![s("ि", BROKEN)]);
        assert_eq!(cuts("a क"), vec![s("a", OTHER), s(" ", OTHER), s("क", CONSONANT)]);
    }
}
