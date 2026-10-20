//! Shaping for Myanmar, as OpenType's Myanmar shaping lays down (for fonts with its rules, `mym2`;
//! fonts made before them are shaped by their own rules alone): the text cut into syllables, and
//! in each the kinzi (Nga, asat, virama, drawn above after the base), the medial Ra and the vowel
//! sign drawn before the base moved before it, what follows the base in the order it is drawn;
//! then the font's forms one feature at a time (reph, pre-base, below, after), and the rest of
//! its rules.

use super::indic::{Pat, cut, dotted_circles, many, mark_syllables, one, opt, order_marks, ranges, sort_merging};
use super::khmer::run;
use super::{Apply, G, Layout, Options};

const X: u8 = 0;
const C: u8 = 1;
const IV: u8 = 2;
const DB: u8 = 3;
const H: u8 = 4;
const ZWNJ: u8 = 5;
const ZWJ: u8 = 6;
const SM: u8 = 8;
const A: u8 = 9;
const GB: u8 = 10;
const DOTTED: u8 = 11;
const RA: u8 = 15;
const CS: u8 = 18;
const VABV: u8 = 20;
const VBLW: u8 = 21;
const VPRE: u8 = 22;
const VPST: u8 = 23;
const AS: u8 = 32;
const MH: u8 = 35;
const MR: u8 = 36;
const MW: u8 = 37;
const MY: u8 = 38;
const PT: u8 = 39;
const VS: u8 = 40;
const ML: u8 = 41;

// Positions, in the order they are drawn.
const PRE_M: u8 = 2;
const PRE_C: u8 = 3;
const BASE_C: u8 = 4;
const AFTER_MAIN: u8 = 5;
const BEFORE_SUB: u8 = 7;
const BELOW_C: u8 = 8;
const AFTER_SUB: u8 = 9;

// Kinds of syllable.
const CONSONANT: u8 = 0;
const BROKEN: u8 = 1;
const OTHER: u8 = 2;

/// Each character's category, U+1000 to U+109F (made from Unicode's Indic syllabic and
/// positional categories, changed as shaping engines change them).
const MYANMAR: [u8; 160] = [
    C, C, C, C, RA, C, C, C, // 1000
    C, C, C, C, C, C, C, C, // 1008
    C, C, C, C, C, C, C, C, // 1010
    C, C, C, RA, C, C, C, C, // 1018
    C, IV, IV, IV, IV, IV, IV, IV, // 1020
    IV, IV, IV, VPST, VPST, VABV, VABV, VBLW, // 1028
    VBLW, VPRE, A, VABV, VABV, VABV, A, DB, // 1030
    SM, H, AS, MY, MR, MW, MH, C, // 1038
    GB, GB, GB, GB, GB, GB, GB, GB, // 1040
    GB, GB, GB, GB, X, X, C, X, // 1048
    C, C, IV, IV, IV, IV, VPST, VPST, // 1050
    VBLW, VBLW, RA, C, C, C, MY, MY, // 1058
    ML, C, VPST, PT, PT, C, C, VPST, // 1060
    VPST, PT, PT, PT, PT, PT, C, C, // 1068
    C, VABV, VABV, VABV, VABV, C, C, C, // 1070
    C, C, C, C, C, C, C, C, // 1078
    C, C, MW, VPST, VPRE, VABV, VABV, SM, // 1080
    SM, SM, SM, SM, SM, SM, C, SM, // 1088
    GB, GB, GB, GB, GB, GB, GB, GB, // 1090
    GB, GB, SM, SM, SM, VABV, X, X, // 1098
];
/// U+AA60 to U+AA7F (Myanmar extended A).
const EXTENDED_A: [u8; 32] = [
    C, C, C, C, C, C, C, C, // AA60
    C, C, C, C, C, C, C, C, // AA68
    X, C, C, C, GB, GB, GB, X, // AA70
    X, X, C, PT, DB, DB, C, C, // AA78
];
/// U+A9E0 to U+A9FF (Myanmar extended B).
const EXTENDED_B: [u8; 32] = [
    C, C, C, C, C, VABV, X, C, // A9E0
    C, C, C, C, C, C, C, C, // A9E8
    GB, GB, GB, GB, GB, GB, GB, GB, // A9F0
    GB, GB, C, C, C, C, C, X, // A9F8
];

/// A character's category.
fn category(c: char) -> u8 {
    let u = c as u32;
    match u {
        0x1000..=0x109F => MYANMAR[(u - 0x1000) as usize],
        0xAA60..=0xAA7F => EXTENDED_A[(u - 0xAA60) as usize],
        0xA9E0..=0xA9FF => EXTENDED_B[(u - 0xA9E0) as usize],
        0xFE00..=0xFE0F => VS,
        0x200C => ZWNJ,
        0x200D => ZWJ,
        0x00A0 | 0x2010 | 0x2011 | 0x2015 | 0x2022 | 0x25FB..=0x25FE => GB,
        0x25CC => DOTTED,
        _ => X,
    }
}

/// The text with its marks in canonical order.
pub(super) fn prepare(text: &[char], first: usize) -> Vec<(char, usize)> {
    let mut out: Vec<(char, usize)> = text.iter().enumerate().map(|(i, &c)| (c, first + i)).collect();
    order_marks(&mut out, |c| match c {
        '\u{1037}' => 7,
        '\u{1039}' | '\u{103A}' => 9,
        '\u{108D}' => 220,
        _ => 0,
    });
    out
}

fn syllable_patterns() -> Vec<(Pat, u8)> {
    use Pat::{Alt, Seq};
    let j = || one(&[ZWJ, ZWNJ]);
    let kinzi = || Seq(vec![one(&[RA]), one(&[AS]), one(&[H])]);
    let c = || one(&[C, RA]);
    let medial_group = || {
        let rest = Alt(vec![Seq(vec![one(&[MW]), opt(one(&[MH])), opt(one(&[ML]))]), Seq(vec![one(&[MH]), opt(one(&[ML]))]), one(&[ML])]);
        Seq(vec![opt(one(&[MY])), opt(one(&[AS])), opt(one(&[MR])), opt(Seq(vec![rest, opt(one(&[AS]))]))])
    };
    let db_as = || opt(Seq(vec![one(&[DB]), opt(one(&[AS]))]));
    let main_vowel_group = || Seq(vec![many(Seq(vec![one(&[VPRE]), opt(one(&[VS]))])), many(one(&[VABV])), many(one(&[VBLW])), many(one(&[A])), db_as()]);
    let post_vowel_group = || Seq(vec![one(&[VPST]), opt(one(&[MH])), opt(one(&[ML])), many(one(&[AS])), many(one(&[VABV])), many(one(&[A])), db_as()]);
    let tone_group = || Alt(vec![one(&[SM]), Seq(vec![one(&[PT]), many(one(&[A])), opt(one(&[DB])), opt(one(&[AS]))])]);
    let complex_tail = || Seq(vec![many(one(&[AS])), medial_group(), main_vowel_group(), many(post_vowel_group()), many(tone_group()), opt(j())]);
    let syllable_tail = || Seq(vec![many(Seq(vec![one(&[H]), one(&[C, RA, IV]), opt(one(&[VS]))])), Alt(vec![one(&[H]), complex_tail()])]);
    vec![
        (Seq(vec![opt(Alt(vec![kinzi(), one(&[CS])])), Alt(vec![c(), one(&[IV, GB, DOTTED])]), opt(one(&[VS])), syllable_tail()]), CONSONANT),
        (j(), OTHER),
        (Seq(vec![opt(kinzi()), opt(one(&[VS])), syllable_tail()]), BROKEN),
    ]
}

fn is_consonant(g: &G) -> bool {
    matches!(g.cat, C | CS | RA | IV | GB | DOTTED)
}

/// One syllable put in the order its glyphs are drawn.
fn reorder(buf: &mut [G], start: usize, end: usize) {
    // a kinzi starts it: the base is the consonant after
    let has_kinzi = start + 3 <= end && buf[start].cat == RA && buf[start + 1].cat == AS && buf[start + 2].cat == H;
    let limit = if has_kinzi { start + 3 } else { start };
    let mut base = if has_kinzi { start } else { limit };
    if let Some(i) = (limit..end).find(|&i| is_consonant(&buf[i])) {
        base = i;
    }
    let mut i = start;
    while i < limit {
        buf[i].pos = AFTER_MAIN;
        i += 1;
    }
    while i < base {
        buf[i].pos = PRE_C;
        i += 1;
    }
    if i < end {
        buf[i].pos = BASE_C;
        i += 1;
    }
    // what follows the base: the medial Ra and a vowel sign before it, the rest after it, below
    // it, or after what is below
    let mut pos = AFTER_MAIN;
    while i < end {
        let cat = buf[i].cat;
        buf[i].pos = if cat == MR {
            PRE_C
        } else if cat == VPRE {
            PRE_M
        } else if cat == VS {
            buf[i - 1].pos
        } else if pos == AFTER_MAIN && cat == VBLW {
            pos = BELOW_C;
            pos
        } else if pos == BELOW_C && cat == A {
            BEFORE_SUB
        } else if pos == BELOW_C && cat == VBLW {
            pos
        } else if pos == BELOW_C {
            pos = AFTER_SUB;
            pos
        } else {
            pos
        };
        i += 1;
    }
    sort_merging(buf, start, end, |g| g.pos);
    // several vowel signs before the base: in the order they were written, each with what
    // follows it
    let first = (start..end).find(|&i| buf[i].pos == PRE_M);
    let last = (start..end).rfind(|&i| buf[i].pos == PRE_M);
    if let (Some(first), Some(last)) = (first, last)
        && first < last
    {
        buf[first..=last].reverse();
        let mut i = first;
        for j in first..=last {
            if buf[j].cat == VPRE {
                buf[i..=j].reverse();
                i = j + 1;
            }
        }
    }
}

/// The substitutions of a Myanmar run in a font with Myanmar's rules (`first` and `usual` the
/// features asked of every script before and after its own).
pub(super) fn substitute(layout: &Layout, buf: &mut Vec<G>, chars: &[(char, usize)], options: &Options, first: &[&[u8; 4]], usual: &[&[u8; 4]]) {
    let gsub = layout.font.gsub;
    let tag = *b"mym2";
    for (g, &(c, _)) in buf.iter_mut().zip(chars) {
        g.cat = category(c);
    }
    let cats: Vec<u8> = buf.iter().map(|g| g.cat).collect();
    let kinds = mark_syllables(buf, cut(&cats, &syllable_patterns(), OTHER), OTHER);
    dotted_circles(buf, &kinds, BROKEN, layout.font.glyph('\u{25CC}'), DOTTED, 0, None);
    let per_syllable = Apply { per_syllable: true, ..Apply::default() };
    run(layout, buf, layout.lookups(gsub, &tag, first).into_iter().map(|(li, _)| (li, per_syllable)).collect());
    for (a, b) in ranges(buf) {
        if matches!(kinds.get(buf[a].syllable as usize), Some(&CONSONANT | &BROKEN)) {
            reorder(buf, a, b);
        }
    }
    let forms = Apply { per_syllable: true, manual_zwj: true, ..Apply::default() };
    for t in [b"rphf", b"pref", b"blwf", b"pstf"] {
        if options.on(t, true) {
            run(layout, buf, layout.lookups(gsub, &tag, &[t]).into_iter().map(|(li, _)| (li, forms)).collect());
        }
    }
    let rest: Vec<&[u8; 4]> = [b"pres", b"abvs", b"blws", b"psts"].into_iter().filter(|t| options.on(t, true)).collect();
    let mut stage: Vec<(u16, Apply)> = layout.lookups(gsub, &tag, &rest).into_iter().map(|(li, _)| (li, Apply { manual_zwj: true, ..Apply::default() })).collect();
    stage.extend(layout.lookups(gsub, &tag, usual).into_iter().map(|(li, _)| (li, Apply::default())));
    run(layout, buf, stage);
}
