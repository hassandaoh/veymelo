//! Apple's own layout tables, for the fonts made for the Mac that have none of OpenType's
//! (Geeza Pro, Thonburi, Helvetica, Hoefler Text, Zapfino…).
//!
//! `morx` swaps, joins, reorders and puts in glyphs with little machines that read the glyphs
//! one by one: each glyph has a class, and the machine's state and that class give an entry
//! that says what to do and which state comes next. `kerx`, and `kern` in Apple's form, kern
//! pairs, kern by such machines (along the line and across it, raising letters joined in a
//! line), and set marks on glyphs. `trak` spaces the letters by size. Positions are in font
//! units, x to the right and y up.

use super::{G, Options};
use crate::{Font, i16_at, u16_at, u32_at};

/// A glyph taken out (a part of a ligature): left in place until the end, so the machines
/// still see where it was.
const DELETED: u16 = 0xFFFF;

/// The OpenType features and Apple's settings for them: the feature type, the setting that
/// turns it on, and the one that turns it off (none where the type's other settings are not an
/// "off": asking for it off leaves the font's own choice).
const SETTINGS: &[(&[u8; 4], u16, u16, Option<u16>)] = &[
    (b"rlig", 1, 0, Some(1)),
    (b"liga", 1, 2, Some(3)),
    (b"dlig", 1, 4, Some(5)),
    (b"clig", 1, 18, Some(19)),
    (b"hlig", 1, 20, Some(21)),
    (b"vert", 4, 0, Some(1)),
    (b"vrt2", 4, 0, Some(1)),
    (b"tnum", 6, 0, None),
    (b"pnum", 6, 1, None),
    (b"sups", 10, 1, None),
    (b"subs", 10, 2, None),
    (b"ordn", 10, 3, None),
    (b"sinf", 10, 4, None),
    (b"afrc", 11, 1, None),
    (b"frac", 11, 2, None),
    (b"zero", 14, 4, Some(5)),
    (b"mgrk", 15, 10, Some(11)),
    (b"titl", 19, 4, None),
    (b"trad", 20, 0, None),
    (b"smpl", 20, 1, None),
    (b"jp78", 20, 2, None),
    (b"jp83", 20, 3, None),
    (b"jp90", 20, 4, None),
    (b"expt", 20, 10, None),
    (b"jp04", 20, 11, None),
    (b"hojo", 20, 12, None),
    (b"nlck", 20, 13, None),
    (b"tnam", 20, 14, None),
    (b"onum", 21, 0, None),
    (b"lnum", 21, 1, None),
    (b"pwid", 22, 0, None),
    (b"fwid", 22, 1, None),
    (b"hwid", 22, 2, None),
    (b"twid", 22, 3, None),
    (b"qwid", 22, 4, None),
    (b"palt", 22, 5, None),
    (b"halt", 22, 6, None),
    (b"hngl", 23, 1, None),
    (b"ruby", 28, 2, Some(3)),
    (b"ital", 32, 2, Some(3)),
    (b"case", 33, 0, Some(1)),
    (b"cpsp", 33, 2, Some(3)),
    (b"hkna", 34, 0, Some(1)),
    (b"vkna", 34, 2, Some(3)),
    (b"ss01", 35, 2, Some(3)),
    (b"ss02", 35, 4, Some(5)),
    (b"ss03", 35, 6, Some(7)),
    (b"ss04", 35, 8, Some(9)),
    (b"ss05", 35, 10, Some(11)),
    (b"ss06", 35, 12, Some(13)),
    (b"ss07", 35, 14, Some(15)),
    (b"ss08", 35, 16, Some(17)),
    (b"ss09", 35, 18, Some(19)),
    (b"ss10", 35, 20, Some(21)),
    (b"ss11", 35, 22, Some(23)),
    (b"ss12", 35, 24, Some(25)),
    (b"ss13", 35, 26, Some(27)),
    (b"ss14", 35, 28, Some(29)),
    (b"ss15", 35, 30, Some(31)),
    (b"ss16", 35, 32, Some(33)),
    (b"ss17", 35, 34, Some(35)),
    (b"ss18", 35, 36, Some(37)),
    (b"ss19", 35, 38, Some(39)),
    (b"ss20", 35, 40, Some(41)),
    (b"calt", 36, 0, Some(1)),
    (b"swsh", 36, 2, Some(3)),
    (b"cswh", 36, 4, Some(5)),
    (b"smcp", 37, 1, None),
    (b"pcap", 37, 2, None),
    (b"c2sc", 38, 1, None),
    (b"c2pc", 38, 2, None),
];

/// The old "letter case" type's small capitals, which fonts still have: the same as `smcp`.
const OLD_SMALL_CAPS: (u16, u16) = (3, 3);

/// The settings asked for, as Apple's (type, setting).
fn asked(options: &Options) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    for (tag, on) in &options.features {
        if let Some(&(_, ty, yes, no)) = SETTINGS.iter().find(|s| s.0 == tag) {
            match (on, no) {
                (true, _) => out.push((ty, yes)),
                (false, Some(no)) => out.push((ty, no)),
                (false, None) => {}
            }
        }
    }
    out
}

/// The OpenType features that stand for settings the font's `morx` has.
pub(super) fn features(font: &Font) -> Vec<String> {
    let (d, at) = (font.data(), font.morx);
    let mut out = Vec::new();
    if at == 0 || u16_at(d, at) < 2 {
        return out;
    }
    let mut p = at + 8;
    for _ in 0..u32_at(d, at + 4) {
        let (length, n) = (u32_at(d, p + 4) as usize, u32_at(d, p + 8) as usize);
        for f in 0..n {
            let r = p + 16 + 12 * f;
            let mut setting = (u16_at(d, r), u16_at(d, r + 2));
            if setting == OLD_SMALL_CAPS {
                setting = (37, 1);
            }
            for s in SETTINGS.iter().filter(|s| (s.1, s.2) == setting) {
                out.push(String::from_utf8_lossy(s.0).to_string());
            }
        }
        if length == 0 {
            break;
        }
        p += length;
    }
    out
}

/// A value from an Apple lookup table at `at` (glyph to a 16-bit value); none for a glyph the
/// table leaves out.
fn lookup(d: &[u8], at: usize, g: u16, glyphs: u16) -> Option<u16> {
    // a binary-search header: how big each entry is and how many there are, the last one, an
    // end mark of 0xFFFF, left out
    let entries = || {
        let (size, n) = (u16_at(d, at + 2) as usize, u16_at(d, at + 4) as usize);
        let n = if n > 0 && u16_at(d, at + 12 + (n - 1) * size) == 0xFFFF { n - 1 } else { n };
        (size.max(1), n)
    };
    match u16_at(d, at) {
        // a value for every glyph
        0 => (g < glyphs).then(|| u16_at(d, at + 2 + 2 * g as usize)),
        // runs of glyphs with one value (2) or a value each (4), in order of their last glyph
        format @ (2 | 4) => {
            let (size, n) = entries();
            let (mut lo, mut hi) = (0, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let r = at + 12 + mid * size;
                let (last, first) = (u16_at(d, r), u16_at(d, r + 2));
                if g > last {
                    lo = mid + 1;
                } else if g < first {
                    hi = mid;
                } else if format == 2 {
                    return Some(u16_at(d, r + 4));
                } else {
                    return Some(u16_at(d, at + u16_at(d, r + 4) as usize + 2 * (g - first) as usize));
                }
            }
            None
        }
        // glyphs one by one, in order
        6 => {
            let (size, n) = entries();
            let (mut lo, mut hi) = (0, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let r = at + 12 + mid * size;
                match u16_at(d, r).cmp(&g) {
                    std::cmp::Ordering::Equal => return Some(u16_at(d, r + 2)),
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                }
            }
            None
        }
        // a value for each glyph of one run
        8 => {
            let (first, n) = (u16_at(d, at + 2), u16_at(d, at + 4));
            (g >= first && g - first < n).then(|| u16_at(d, at + 6 + 2 * (g - first) as usize))
        }
        // the same with values of any size (only their low 16 bits used)
        10 => {
            let (size, first, n) = (u16_at(d, at + 2) as usize, u16_at(d, at + 4), u16_at(d, at + 6));
            if g < first || g - first >= n {
                return None;
            }
            let r = at + 8 + size * (g - first) as usize;
            Some(match size {
                1 => d.get(r).copied().unwrap_or(0) as u16,
                2 => u16_at(d, r),
                4 => u32_at(d, r) as u16,
                _ => u32_at(d, r + size - 4) as u16,
            })
        }
        _ => None,
    }
}

/// A machine's tables: each glyph's class, a row of entries for each state, and the entries.
/// The newer form (`morx`, `kerx`) counts in 32 bits and looks classes up; the older (`kern`)
/// counts in 16, keeps classes in a plain array, and names states by where their rows are.
struct Machine<'a> {
    d: &'a [u8],
    at: usize,
    classes: u32,
    class_table: usize,
    states: usize,
    entries: usize,
    entry_size: usize,
    wide: bool,
    glyphs: u16,
}

/// The classes every machine has: the end of the text, a glyph its table leaves out, a glyph
/// taken out.
const END_OF_TEXT: u32 = 0;
const OUT_OF_BOUNDS: u32 = 1;
const DELETED_GLYPH: u32 = 2;

/// Entry flag (all machines): stay on this glyph.
const DONT_ADVANCE: u16 = 0x4000;

impl Machine<'_> {
    fn wide(d: &[u8], at: usize, entry_size: usize, glyphs: u16) -> Machine<'_> {
        let off = |k: usize| at + u32_at(d, at + 4 * k) as usize;
        Machine { d, at, classes: u32_at(d, at), class_table: off(1), states: off(2), entries: off(3), entry_size, wide: true, glyphs }
    }

    fn narrow(d: &[u8], at: usize, entry_size: usize, glyphs: u16) -> Machine<'_> {
        let off = |k: usize| at + u16_at(d, at + 2 * k) as usize;
        Machine { d, at, classes: u16_at(d, at) as u32, class_table: off(1), states: off(2), entries: off(3), entry_size, wide: false, glyphs }
    }

    /// The class of a glyph (none: the end of the text).
    fn class(&self, g: Option<u16>) -> u32 {
        let Some(g) = g else { return END_OF_TEXT };
        if g == DELETED {
            return DELETED_GLYPH;
        }
        let c = if self.wide {
            lookup(self.d, self.class_table, g, self.glyphs).map(u32::from)
        } else {
            let (first, n) = (u16_at(self.d, self.class_table), u16_at(self.d, self.class_table + 2));
            (g >= first && g - first < n).then(|| self.d.get(self.class_table + 4 + (g - first) as usize).copied().unwrap_or(1) as u32)
        };
        c.filter(|&c| c < self.classes).unwrap_or(OUT_OF_BOUNDS)
    }

    /// Where the entry is for a glyph of `class` met in `state`.
    fn entry(&self, state: u32, class: u32) -> usize {
        let cell = state as usize * self.classes as usize + class as usize;
        let index = if self.wide { u16_at(self.d, self.states + 2 * cell) as usize } else { self.d.get(self.states + cell).copied().unwrap_or(0) as usize };
        self.entries + index * self.entry_size
    }

    /// The state an entry goes to.
    fn next(&self, entry: usize) -> u32 {
        let s = u16_at(self.d, entry) as u32;
        if self.wide || self.classes == 0 { s } else { s.saturating_sub((self.states - self.at) as u32) / self.classes }
    }

    /// Runs the machine over the glyphs: `step` gets the glyphs, where the machine is (the
    /// glyph count at the end of the text), the entry it took and that entry's flags, and may
    /// change the glyphs and where it is; then it moves on, unless the entry says to stay.
    fn run(&self, buf: &mut Vec<G>, mut step: impl FnMut(&mut Vec<G>, &mut usize, usize, u16)) {
        let (mut state, mut i) = (0u32, 0usize);
        // how many times it may stay on a glyph in all, so a font cannot keep it there
        let mut stays = 64 * buf.len() + 256;
        loop {
            let class = self.class(buf.get(i).map(|g| g.id));
            let e = self.entry(state, class);
            let flags = u16_at(self.d, e + 2);
            state = self.next(e);
            step(buf, &mut i, e, flags);
            if i >= buf.len() {
                break;
            }
            if flags & DONT_ADVANCE == 0 || stays == 0 {
                i += 1;
            } else {
                stays -= 1;
            }
        }
    }
}

/// Every glyph from `a` to `b` given the first character any of them came from (they now stand
/// for those characters together).
fn merge(buf: &mut [G], a: usize, b: usize) {
    let b = b.min(buf.len());
    if a >= b {
        return;
    }
    let first = buf[a..b].iter().map(|g| g.cluster).min().unwrap_or(0);
    for g in &mut buf[a..b] {
        g.cluster = first;
    }
}

/// The glyphs put through the font's `morx`: its chains of subtables, each subtable run if
/// the features it belongs to are on (the font's own choices, changed by what `options` asks).
/// `rtl`: the text goes right to left, which turns round the subtables that read in the
/// direction of the line; `vertical`: the text is set in columns.
pub(super) fn substitute(font: &Font, buf: &mut Vec<G>, options: &Options, rtl: bool, vertical: bool) {
    let (d, at, glyphs) = (font.data(), font.morx, font.glyphs);
    if u16_at(d, at) < 2 {
        return;
    }
    let asked = asked(options);
    let mut p = at + 8;
    for _ in 0..u32_at(d, at + 4) {
        let (mut flags, length, features, subtables) = (u32_at(d, p), u32_at(d, p + 4) as usize, u32_at(d, p + 8) as usize, u32_at(d, p + 12));
        for f in 0..features {
            let r = p + 16 + 12 * f;
            let mut setting = (u16_at(d, r), u16_at(d, r + 2));
            if setting == OLD_SMALL_CAPS {
                setting = (37, 1);
            }
            if asked.contains(&setting) {
                flags = (flags & u32_at(d, r + 8)) | u32_at(d, r + 4);
            }
        }
        let mut q = p + 16 + 12 * features;
        for _ in 0..subtables {
            let (len, coverage, belongs) = (u32_at(d, q) as usize, u32_at(d, q + 4), u32_at(d, q + 8));
            // for text set either way (0x20000000), or the way this text is
            let fits = coverage & 0x2000_0000 != 0 || (coverage & 0x8000_0000 != 0) == vertical;
            if belongs & flags != 0 && fits && len > 12 {
                // read from the end (0x40000000); in the order of the text whichever way it
                // goes (0x10000000), or else in the direction of the line
                let backwards = coverage & 0x4000_0000 != 0;
                let reverse = if coverage & 0x1000_0000 != 0 { backwards } else { backwards != rtl };
                if reverse {
                    buf.reverse();
                }
                let s = q + 12;
                match coverage & 0xFF {
                    0 => rearrange(&Machine::wide(d, s, 4, glyphs), buf),
                    1 => contextual(&Machine::wide(d, s, 8, glyphs), s + u32_at(d, s + 16) as usize, buf),
                    2 => {
                        let off = |k: usize| s + u32_at(d, s + 16 + 4 * k) as usize;
                        ligature(&Machine::wide(d, s, 6, glyphs), off(0), off(1), off(2), buf);
                    }
                    4 => {
                        for g in buf.iter_mut().filter(|g| g.id != DELETED) {
                            if let Some(n) = lookup(d, s, g.id, glyphs) {
                                g.id = n;
                            }
                        }
                    }
                    5 => insertion(&Machine::wide(d, s, 8, glyphs), s + u32_at(d, s + 16) as usize, buf),
                    _ => {}
                }
                if reverse {
                    buf.reverse();
                }
            }
            if len == 0 {
                break;
            }
            q += len;
        }
        if length == 0 {
            break;
        }
        p += length;
    }
    buf.retain(|g| g.id != DELETED);
}

/// Glyphs reordered: the machine marks the first and the last of a stretch, and a verb
/// says which of the glyphs at its ends (A, B before the middle x; C, D after it) change
/// places.
fn rearrange(m: &Machine, buf: &mut Vec<G>) {
    // how many from the start and from the end move, as 0–2, or 3 for two that also swap
    const VERBS: [u8; 16] = [0x00, 0x10, 0x01, 0x11, 0x20, 0x30, 0x02, 0x03, 0x12, 0x13, 0x21, 0x31, 0x22, 0x32, 0x23, 0x33];
    let (mut start, mut end) = (0usize, 0usize);
    m.run(buf, |buf, i, _, flags| {
        if flags & 0x8000 != 0 {
            start = *i;
        }
        if flags & 0x2000 != 0 {
            end = (*i + 1).min(buf.len());
        }
        let verb = VERBS[(flags & 0xF) as usize];
        if verb == 0 || start >= end {
            return;
        }
        let (l, r) = ((verb >> 4).min(2) as usize, (verb & 0xF).min(2) as usize);
        let n = end - start;
        if n < l + r {
            return;
        }
        merge(buf, start, end);
        let span = &mut buf[start..end];
        let moved: Vec<G> = span[n - r..].iter().chain(&span[l..n - r]).chain(&span[..l]).cloned().collect();
        span.clone_from_slice(&moved);
        if verb >> 4 == 3 {
            span.swap(n - 1, n - 2);
        }
        if verb & 0xF == 3 {
            span.swap(0, 1);
        }
    });
}

/// Glyphs swapped for others by where they are: the entry may swap the glyph marked earlier,
/// the one it is on, or both, each by a lookup table of its own.
fn contextual(m: &Machine, tables: usize, buf: &mut Vec<G>) {
    let d = m.d;
    let swap = |g: u16, k: u16| -> Option<u16> {
        if g == DELETED {
            return None;
        }
        lookup(d, tables + u32_at(d, tables + 4 * k as usize) as usize, g, m.glyphs)
    };
    let (mut mark, mut marked) = (0usize, false);
    m.run(buf, |buf, i, e, flags| {
        // at the end of the text nothing changes unless a glyph was marked
        if *i >= buf.len() && !marked {
            return;
        }
        let (at_mark, at_current) = (u16_at(d, e + 4), u16_at(d, e + 6));
        if at_mark != 0xFFFF
            && mark < buf.len()
            && let Some(n) = swap(buf[mark].id, at_mark)
        {
            buf[mark].id = n;
        }
        if at_current != 0xFFFF && !buf.is_empty() {
            let k = (*i).min(buf.len() - 1);
            if let Some(n) = swap(buf[k].id, at_current) {
                buf[k].id = n;
            }
        }
        if flags & 0x8000 != 0 {
            marked = true;
            mark = *i;
        }
    });
}

/// Glyphs joined into ligatures: the machine keeps the glyphs it may join on a stack and,
/// when it has them all, takes them off one by one with an action each that adds up which
/// ligature they make; the ligature takes the first one's place and the rest are taken out.
fn ligature(m: &Machine, actions: usize, components: usize, ligatures: usize, buf: &mut Vec<G>) {
    let d = m.d;
    let mut stack: Vec<usize> = Vec::new();
    m.run(buf, |buf, i, e, flags| {
        if flags & 0x8000 != 0 && *i < buf.len() {
            // the same glyph is not kept twice (where the machine stayed on it)
            if stack.last() == Some(&*i) {
                stack.pop();
            }
            if stack.len() == 64 {
                stack.remove(0);
            }
            stack.push(*i);
        }
        if flags & 0x2000 == 0 || stack.is_empty() || *i >= buf.len() {
            return;
        }
        let mut at = actions + 4 * u16_at(d, e + 4) as usize;
        let mut cursor = stack.len();
        let mut index: u32 = 0;
        loop {
            if cursor == 0 {
                stack.clear();
                break;
            }
            cursor -= 1;
            let pos = stack[cursor];
            let action = u32_at(d, at);
            // the last 30 bits, signed: added to the glyph, where in the component table to look
            let offset = (((action & 0x3FFF_FFFF) << 2) as i32 >> 2) as i64;
            let k = buf[pos].id as i64 + offset;
            if k >= 0 {
                index = index.wrapping_add(u16_at(d, components + 2 * k as usize) as u32);
            }
            // stored, or the last: the ligature goes in this glyph's place
            if action & 0xC000_0000 != 0 {
                let last = *stack.last().unwrap_or(&pos);
                merge(buf, pos, last + 1);
                buf[pos].id = u16_at(d, ligatures + 2 * index as usize);
                while stack.len() > cursor + 1 {
                    if let Some(p) = stack.pop() {
                        buf[p].id = DELETED;
                    }
                }
            }
            at += 4;
            if action & 0x8000_0000 != 0 {
                break;
            }
        }
    });
}

/// Glyphs put in, before or after the glyph marked earlier and the one the machine is on.
fn insertion(m: &Machine, glyphs_at: usize, buf: &mut Vec<G>) {
    let d = m.d;
    let put = |buf: &mut Vec<G>, place: usize, from: u16, count: usize, next_to: usize| {
        let like = buf.get(next_to).or(buf.last()).cloned();
        let Some(like) = like else { return };
        for k in 0..count {
            let id = u16_at(d, glyphs_at + 2 * (from as usize + k));
            buf.insert(place + k, G { id, attach: None, chain: None, ..like.clone() });
        }
    };
    let (mut mark, mut marked) = (0usize, false);
    m.run(buf, |buf, i, e, flags| {
        let (current, at_mark) = (u16_at(d, e + 4), u16_at(d, e + 6));
        if at_mark != 0xFFFF && marked && mark <= buf.len() {
            let count = (flags & 0x1F) as usize;
            let before = flags & 0x0400 != 0;
            let place = if before || mark >= buf.len() { mark } else { mark + 1 };
            put(buf, place, at_mark, count, mark);
            *i += count;
        }
        if current != 0xFFFF {
            let count = ((flags & 0x03E0) >> 5) as usize;
            let before = flags & 0x0800 != 0;
            let place = if before || *i >= buf.len() { *i } else { *i + 1 };
            put(buf, place, current, count, *i);
            // on past them, unless it stays to read them
            if flags & DONT_ADVANCE == 0 {
                *i += count;
            }
        }
        if flags & 0x8000 != 0 {
            marked = true;
            mark = *i;
        }
    });
}

/// Which kerning table is read.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kerning {
    /// `kerx`.
    Kerx,
    /// `kern` in Apple's form (version 1).
    Kern,
}

/// One subtable of `kerx` or Apple's `kern`: where it starts, how long its header is, its
/// format, and whether it moves glyphs across the line, varies with the font, and is for text
/// set in columns; and how many values each of its kerning values has (one per variation).
struct Sub {
    at: usize,
    header: usize,
    format: u32,
    cross: bool,
    varies: bool,
    vertical: bool,
    tuples: usize,
}

/// The glyphs kerned by the font's `kerx`, or its `kern` in Apple's form, the glyphs read in
/// the order they are seen (left to right, whichever way the text goes). `kerning`: kern along
/// the line (the moves across it, which place letters and marks, always come).
pub(super) fn kern(font: &Font, buf: &mut [G], table: Kerning, rtl: bool, kerning: bool) {
    let d = font.data();
    let (at, count, first) = match table {
        Kerning::Kerx => (font.kerx, u32_at(d, font.kerx + 4), font.kerx + 8),
        Kerning::Kern => (font.kern, u32_at(d, font.kern + 4), font.kern + 8),
    };
    if at == 0 {
        return;
    }
    let n = buf.len();
    if rtl {
        buf.reverse();
        flip(buf);
    }
    // moves across the line: each glyph's own, and whether it hangs on the glyph before it
    // (all do once a subtable moves glyphs across, until a value of -32768 lets one go)
    let mut across = vec![0.0; n];
    let mut hangs = vec![false; n];
    let mut crossed = false;
    let mut p = first;
    for _ in 0..count {
        let sub = match table {
            Kerning::Kerx => {
                let c = u32_at(d, p + 4);
                Sub { at: p, header: 12, format: c & 0xFF, cross: c & 0x4000_0000 != 0, varies: false, vertical: c & 0x8000_0000 != 0, tuples: u32_at(d, p + 8) as usize }
            }
            Kerning::Kern => {
                let c = u16_at(d, p + 4);
                Sub { at: p, header: 8, format: (c & 0xFF) as u32, cross: c & 0x4000 != 0, varies: c & 0x2000 != 0, vertical: c & 0x8000 != 0, tuples: 0 }
            }
        };
        let length = u32_at(d, p) as usize;
        if !sub.vertical && !sub.varies && (kerning || sub.cross) {
            if sub.cross && !crossed {
                crossed = true;
                hangs.iter_mut().for_each(|h| *h = true);
            }
            let mut moved = Moves { buf: &mut *buf, across: &mut across, hangs: &mut hangs, cross: sub.cross };
            match sub.format {
                0 | 2 | 3 => pairs(font, &sub, table, &mut moved),
                1 => machine(font, &sub, table, &mut moved),
                4 if table == Kerning::Kerx => anchors(font, &sub, moved.buf),
                _ => {}
            }
        }
        if length == 0 {
            break;
        }
        p += length;
    }
    if crossed {
        let mut y = 0.0;
        for i in 0..n {
            y = across[i] + if hangs[i] && i > 0 { y } else { 0.0 };
            buf[i].dy += y;
        }
    }
    if rtl {
        buf.reverse();
        flip(buf);
    }
}

/// The glyphs a mark is set on, renumbered for the glyphs turned round.
fn flip(buf: &mut [G]) {
    let n = buf.len();
    for g in buf.iter_mut() {
        if let Some((t, x, y)) = g.attach {
            g.attach = Some((n - 1 - t, x, y));
        }
    }
}

/// Where kerning goes: along the line into the glyphs, across it into each glyph's own move.
struct Moves<'a> {
    buf: &'a mut [G],
    across: &'a mut [f64],
    hangs: &'a mut [bool],
    cross: bool,
}

/// Kerning between each glyph and the next: from a list of pairs (format 0), a table of
/// classes (2), or a small table of classes in `kern` (3).
fn pairs(font: &Font, sub: &Sub, table: Kerning, m: &mut Moves) {
    let (d, s, glyphs) = (font.data(), sub.at + sub.header, font.glyphs);
    let kerx = table == Kerning::Kerx;
    let value = |a: u16, b: u16| -> f64 {
        match sub.format {
            0 => {
                let (n, pairs) = if kerx { (u32_at(d, s) as usize, s + 16) } else { (u16_at(d, s) as usize, s + 8) };
                let key = (a as u32) << 16 | b as u32;
                let (mut lo, mut hi) = (0, n);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let r = pairs + 6 * mid;
                    match u32_at(d, r).cmp(&key) {
                        std::cmp::Ordering::Equal => return i16_at(d, r + 4) as f64,
                        std::cmp::Ordering::Less => lo = mid + 1,
                        std::cmp::Ordering::Greater => hi = mid,
                    }
                }
                0.0
            }
            2 if kerx => {
                // the classes as lookup tables, both in bytes: the left one to the row (a
                // multiple of the row's width), the right one along it
                let off = |k: usize| sub.at + u32_at(d, s + 4 * k) as usize;
                let l = lookup(d, off(1), a, glyphs).unwrap_or(0) as usize;
                let r = lookup(d, off(2), b, glyphs).unwrap_or(0) as usize;
                i16_at(d, off(3) + l + r) as f64
            }
            2 => {
                // the classes as byte offsets from the start of the subtable
                let off = |k: usize| sub.at + u16_at(d, s + 2 * k) as usize;
                let class = |t: usize, g: u16| -> usize {
                    let (first, n) = (u16_at(d, t), u16_at(d, t + 2));
                    if g >= first && g - first < n { u16_at(d, t + 4 + 2 * (g - first) as usize) as usize } else { 0 }
                };
                let (l, r) = (class(off(1), a), class(off(2), b));
                let at = sub.at + l + r;
                if l == 0 || at < off(3) { 0.0 } else { i16_at(d, at) as f64 }
            }
            3 => {
                let n = u16_at(d, s) as usize;
                let byte = |k: usize| d.get(k).copied().unwrap_or(0) as usize;
                let (values, lefts, rights) = (byte(s + 2), byte(s + 3), byte(s + 4));
                let (a, b) = (a as usize, b as usize);
                if a >= n || b >= n {
                    return 0.0;
                }
                let classes = s + 6 + 2 * values;
                let (l, r) = (byte(classes + a), byte(classes + n + b));
                if l >= lefts || r >= rights {
                    return 0.0;
                }
                let k = byte(classes + 2 * n + l * rights + r);
                if k < values { i16_at(d, s + 6 + 2 * k) as f64 } else { 0.0 }
            }
            _ => 0.0,
        }
    };
    let n = m.buf.len();
    for i in 0..n.saturating_sub(1) {
        if m.buf[i].id == DELETED {
            continue;
        }
        let Some(j) = (i + 1..n).find(|&j| m.buf[j].id != DELETED) else { break };
        let v = value(m.buf[i].id, m.buf[j].id);
        if v == 0.0 {
            continue;
        }
        if m.cross {
            m.across[j] = v;
        } else {
            m.buf[i].advance += v;
        }
    }
}

/// Kerning by a machine (format 1): glyphs are kept on a stack, and an action takes them off,
/// a value for each, until a value marked last (odd). Along the line the value moves the glyph
/// and those after it; across it, it raises the glyph and those that hang on it, and -32768
/// lets the glyph go of the one before it (and starts it again from the line).
fn machine(font: &Font, sub: &Sub, table: Kerning, m: &mut Moves) {
    let (d, s) = (font.data(), sub.at + sub.header);
    let (machine, values, step) = match table {
        Kerning::Kerx => (Machine::wide(d, s, 6, font.glyphs), s + u32_at(d, s + 16) as usize, 2 * sub.tuples.max(1)),
        Kerning::Kern => (Machine::narrow(d, s, 4, font.glyphs), s, 2),
    };
    let mut stack: Vec<usize> = Vec::new();
    let mut glyphs: Vec<G> = m.buf.to_vec();
    machine.run(&mut glyphs, |buf, i, e, flags| {
        if table == Kerning::Kerx && flags & 0x2000 != 0 {
            stack.clear();
        }
        if flags & 0x8000 != 0 {
            if stack.len() < 8 {
                stack.push(*i);
            } else {
                stack.clear();
            }
        }
        let first = match table {
            Kerning::Kerx => {
                let k = u16_at(d, e + 4);
                if k == 0xFFFF {
                    return;
                }
                values + k as usize * step
            }
            Kerning::Kern => {
                let k = flags & 0x3FFF;
                if k == 0 {
                    return;
                }
                values + k as usize
            }
        };
        let mut at = first;
        while let Some(idx) = stack.pop() {
            let raw = i16_at(d, at) as i32;
            at += step;
            if idx >= buf.len() {
                continue;
            }
            let (last, v) = (raw & 1 != 0, raw & !1);
            if m.cross {
                // -32768 lets the glyph go of the one before it, and starts it again at 0
                if v == -0x8000 {
                    m.hangs[idx] = false;
                    m.across[idx] = 0.0;
                } else {
                    m.across[idx] += v as f64;
                }
            } else {
                m.buf[idx].dx += v as f64;
                m.buf[idx].advance += v as f64;
            }
            if last {
                break;
            }
        }
    });
}

/// Marks set on glyphs by a machine (`kerx` format 4): the glyph it is on goes where a point of
/// it meets a point of the glyph marked earlier, the points as outline points (0), as anchors
/// in `ankr` (1), or as numbers given (2).
fn anchors(font: &Font, sub: &Sub, buf: &mut [G]) {
    let (d, s) = (font.data(), sub.at + sub.header);
    let machine = Machine::wide(d, s, 6, font.glyphs);
    let flags = u32_at(d, s + 16);
    let (kind, data) = (flags >> 30, s + (flags & 0x00FF_FFFF) as usize);
    let anchor = |g: u16, k: usize| -> Option<(f64, f64)> {
        let ankr = font.ankr;
        if ankr == 0 {
            return None;
        }
        let off = lookup(d, ankr + u32_at(d, ankr + 4) as usize, g, font.glyphs)? as usize;
        let at = ankr + u32_at(d, ankr + 8) as usize + off;
        (k < u32_at(d, at) as usize).then(|| (i16_at(d, at + 4 + 4 * k) as f64, i16_at(d, at + 6 + 4 * k) as f64))
    };
    let (mut mark, mut marked) = (0usize, false);
    let mut glyphs: Vec<G> = buf.to_vec();
    let mut set: Vec<(usize, usize, f64, f64)> = Vec::new();
    machine.run(&mut glyphs, |b, i, e, flags| {
        let k = u16_at(d, e + 4);
        if marked && k != 0xFFFF && *i < b.len() {
            let k = k as usize;
            let points = match kind {
                0 => font.contour_point(b[mark].id, u16_at(d, data + 4 * k) as usize).zip(font.contour_point(b[*i].id, u16_at(d, data + 4 * k + 2) as usize)),
                1 => anchor(b[mark].id, u16_at(d, data + 4 * k) as usize).zip(anchor(b[*i].id, u16_at(d, data + 4 * k + 2) as usize)),
                2 => {
                    let v = |j: usize| i16_at(d, data + 8 * k + 2 * j) as f64;
                    Some(((v(0), v(1)), (v(2), v(3))))
                }
                _ => None,
            };
            if let Some(((mx, my), (cx, cy))) = points {
                set.push((*i, mark, mx - cx, my - cy));
            }
        }
        if flags & 0x8000 != 0 {
            marked = true;
            mark = *i;
        }
    });
    for (i, on, x, y) in set {
        buf[i].attach = Some((on, x, y));
        buf[i].keep = true;
    }
}

/// The letters spaced as the font's `trak` says for text of `size` (points to an em), its usual
/// track: between the sizes it gives, in proportion; beyond them, as at the nearest. Each glyph
/// that takes room gets the space after it.
pub(super) fn track(font: &Font, buf: &mut [G], size: f64) {
    let (d, t) = (font.data(), font.trak);
    let horizontal = u16_at(d, t + 6) as usize;
    if t == 0 || horizontal == 0 {
        return;
    }
    let data = t + horizontal;
    let (tracks, sizes, size_table) = (u16_at(d, data) as usize, u16_at(d, data + 2) as usize, t + u32_at(d, data + 4) as usize);
    let Some(entry) = (0..tracks).map(|k| data + 8 + 8 * k).find(|&r| u32_at(d, r) == 0) else { return };
    let values = t + u16_at(d, entry + 6) as usize;
    let value = |k: usize| i16_at(d, values + 2 * k) as f64;
    let size_at = |k: usize| u32_at(d, size_table + 4 * k) as i32 as f64 / 65536.0;
    let v = match sizes {
        0 => return,
        1 => value(0),
        _ if size <= size_at(0) => value(0),
        n if size >= size_at(n - 1) => value(n - 1),
        n => {
            let k = (1..n).find(|&k| size_at(k) >= size).unwrap_or(n - 1) - 1;
            let (s0, s1) = (size_at(k), size_at(k + 1));
            let f = if s1 == s0 { 0.0 } else { (size - s0) / (s1 - s0) };
            (f * value(k + 1) + (1.0 - f) * value(k)).round()
        }
    };
    if v == 0.0 {
        return;
    }
    for g in buf.iter_mut().filter(|g| g.advance > 0.0 && g.attach.is_none()) {
        g.advance += v;
    }
}
