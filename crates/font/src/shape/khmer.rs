//! Shaping for Khmer, as OpenType's Khmer shaping lays down: vowel signs in two parts taken apart
//! (their first part drawn before the consonant), the text cut into syllables, and in each the
//! coeng and Ro (drawn before the base) and the vowel sign drawn before it moved to its start.
//! Then the font's forms, each on the glyphs it is for (the Ro's pre-base form, forms below,
//! above and after, and the forms of what follows a pre-base Ro), and the rest of its rules.

use super::indic::{Pat, cut, dotted_circles, many, mark_syllables, one, opt, order_marks, ranges};
use super::{Apply, G, Layout, Options, merge};
use crate::Font;

const X: u8 = 0;
const C: u8 = 1;
const V: u8 = 2;
const H: u8 = 4;
const ZWNJ: u8 = 5;
const ZWJ: u8 = 6;
const PLACEHOLDER: u8 = 10;
const DOTTED: u8 = 11;
const RA: u8 = 15;
const VABV: u8 = 20;
const VBLW: u8 = 21;
const VPRE: u8 = 22;
const VPST: u8 = 23;
const ROBATIC: u8 = 25;
const XGROUP: u8 = 26;
const YGROUP: u8 = 27;

// Kinds of syllable.
const CONSONANT: u8 = 0;
const BROKEN: u8 = 1;
const OTHER: u8 = 2;

// The bits of the features that apply only to the glyphs the reordering marks.
const PREF: u32 = 1;
const BLWF: u32 = 2;
const ABVF: u32 = 4;
const PSTF: u32 = 8;
const CFAR: u32 = 16;
/// The bit every glyph has.
const ALL: u32 = 1 << 31;

/// Each character's category, U+1780 to U+17FF (made from Unicode's Indic syllabic and
/// positional categories, changed as shaping engines change them).
const KHMER: [u8; 128] = [
    C, C, C, C, C, C, C, C, // 1780
    C, C, C, C, C, C, C, C, // 1788
    C, C, C, C, C, C, C, C, // 1790
    C, C, RA, C, C, C, C, C, // 1798
    C, C, C, V, V, V, V, V, // 17A0
    V, V, V, V, V, V, V, V, // 17A8
    V, V, V, V, X, X, VPST, VABV, // 17B0
    VABV, VABV, VABV, VBLW, VBLW, VBLW, VABV, VPST, // 17B8
    VPST, VPRE, VPRE, VPRE, VPST, VPST, XGROUP, YGROUP, // 17C0
    YGROUP, ROBATIC, ROBATIC, XGROUP, ROBATIC, XGROUP, XGROUP, XGROUP, // 17C8
    XGROUP, XGROUP, H, YGROUP, X, X, X, X, // 17D0
    X, PLACEHOLDER, X, X, X, YGROUP, X, X, // 17D8
    PLACEHOLDER, PLACEHOLDER, PLACEHOLDER, PLACEHOLDER, PLACEHOLDER, PLACEHOLDER, PLACEHOLDER, PLACEHOLDER, // 17E0
    PLACEHOLDER, PLACEHOLDER, X, X, X, X, X, X, // 17E8
    X, X, X, X, X, X, X, X, // 17F0
    X, X, X, X, X, X, X, X, // 17F8
];

/// A character's category.
fn category(c: char) -> u8 {
    let u = c as u32;
    match u {
        0x1780..=0x17FF => KHMER[(u - 0x1780) as usize],
        0x200C => ZWNJ,
        0x200D => ZWJ,
        0x00A0 | 0x2010 | 0x2011 | 0x2015 | 0x2022 | 0x25FB..=0x25FE => PLACEHOLDER,
        0x25CC => DOTTED,
        _ => X,
    }
}

/// The text as Khmer shaping expects it: a vowel sign in two parts as its first part (the sign
/// drawn before the consonant) and itself, where the font has both; marks in canonical order.
pub(super) fn prepare(font: &Font, text: &[char], first: usize) -> Vec<(char, usize)> {
    let mut out = Vec::with_capacity(text.len() + 2);
    for (i, &c) in text.iter().enumerate() {
        if matches!(c, '\u{17BE}' | '\u{17BF}' | '\u{17C0}' | '\u{17C4}' | '\u{17C5}') && font.glyph('\u{17C1}') != 0 && font.glyph(c) != 0 {
            out.push(('\u{17C1}', first + i));
        }
        out.push((c, first + i));
    }
    order_marks(&mut out, |c| match c {
        '\u{17D2}' => 9,
        '\u{17DD}' => 230,
        _ => 0,
    });
    out
}

fn syllable_patterns() -> Vec<(Pat, u8)> {
    use Pat::{Alt, Seq};
    let c = || one(&[C, RA, V]);
    let joiner = || one(&[ZWJ, ZWNJ]);
    let cn = || Seq(vec![c(), opt(Seq(vec![opt(joiner()), one(&[ROBATIC])]))]);
    let xgroup = || many(Seq(vec![many(joiner()), one(&[XGROUP])]));
    let matra_group = || Seq(vec![opt(one(&[VPRE])), xgroup(), opt(one(&[VBLW])), xgroup(), opt(Seq(vec![opt(joiner()), one(&[VABV])])), xgroup(), opt(one(&[VPST]))]);
    let syllable_tail = || Seq(vec![xgroup(), matra_group(), xgroup(), opt(Seq(vec![one(&[H]), c()])), many(one(&[YGROUP]))]);
    let broken = || Seq(vec![many(Seq(vec![one(&[H]), cn()])), Alt(vec![one(&[H]), syllable_tail()])]);
    vec![(Seq(vec![Alt(vec![cn(), one(&[PLACEHOLDER, DOTTED])]), broken()]), CONSONANT), (broken(), BROKEN)]
}

/// One syllable put in the order its glyphs are formed, each glyph marked with the features it
/// is for.
fn reorder(buf: &mut [G], start: usize, end: usize, cfar: bool) {
    for g in &mut buf[start + 1..end] {
        g.mask |= BLWF | ABVF | PSTF;
    }
    let mut coengs = 0;
    for i in start + 1..end {
        if buf[i].cat == H && coengs <= 2 && i + 1 < end {
            coengs += 1;
            if buf[i + 1].cat == RA {
                // a coeng and Ro go before the base, in their pre-base form; what follows them
                // takes the forms that follow one
                buf[i].mask |= PREF;
                buf[i + 1].mask |= PREF;
                merge(buf, start, i + 2);
                buf[start..i + 2].rotate_right(2);
                if cfar {
                    for g in &mut buf[i + 2..end] {
                        g.mask |= CFAR;
                    }
                }
                coengs = 2;
            }
        } else if buf[i].cat == VPRE {
            merge(buf, start, i + 1);
            buf[start..=i].rotate_right(1);
        }
    }
}

/// The substitutions of a Khmer run (`first` and `usual` the features asked of every script
/// before and after its own).
pub(super) fn substitute(layout: &Layout, buf: &mut Vec<G>, chars: &[(char, usize)], options: &Options, first: &[&[u8; 4]], usual: &[&[u8; 4]]) {
    let gsub = layout.font.gsub;
    let tag = *b"khmr";
    for (g, &(c, _)) in buf.iter_mut().zip(chars) {
        g.cat = category(c);
        g.mask = ALL;
    }
    let cats: Vec<u8> = buf.iter().map(|g| g.cat).collect();
    let kinds = mark_syllables(buf, cut(&cats, &syllable_patterns(), OTHER), OTHER);
    dotted_circles(buf, &kinds, BROKEN, layout.font.glyph('\u{25CC}'), DOTTED, 0, None);
    let cfar = options.on(b"cfar", true) && !layout.lookups(gsub, &tag, &[b"cfar"]).is_empty();
    for (a, b) in ranges(buf) {
        if matches!(kinds.get(buf[a].syllable as usize), Some(&CONSONANT | &BROKEN)) {
            reorder(buf, a, b, cfar);
        }
    }
    // the first features and the forms together, each syllable on its own
    let mut stage: Vec<(u16, Apply)> = layout.lookups(gsub, &tag, first).into_iter().map(|(li, _)| (li, Apply { per_syllable: true, ..Apply::default() })).collect();
    for (t, bit) in [(b"pref", PREF), (b"blwf", BLWF), (b"abvf", ABVF), (b"pstf", PSTF), (b"cfar", CFAR)] {
        if options.on(t, true) {
            for (li, _) in layout.lookups(gsub, &tag, &[t]) {
                stage.push((li, Apply { mask: bit, per_syllable: true, manual_zwj: true, manual_zwnj: true, form: 0 }));
            }
        }
    }
    run(layout, buf, stage);
    // then the rest
    let rest: Vec<&[u8; 4]> = [b"pres", b"abvs", b"blws", b"psts"].into_iter().filter(|t| options.on(t, true)).collect();
    let mut stage: Vec<(u16, Apply)> = layout.lookups(gsub, &tag, &rest).into_iter().map(|(li, _)| (li, Apply { manual_zwj: true, manual_zwnj: true, ..Apply::default() })).collect();
    stage.extend(layout.lookups(gsub, &tag, usual).into_iter().map(|(li, _)| (li, Apply::default())));
    run(layout, buf, stage);
}

/// A stage's lookups applied in the order the font lists them (a lookup several features share
/// once, on the glyphs any of them is for).
pub(super) fn run(layout: &Layout, buf: &mut Vec<G>, mut stage: Vec<(u16, Apply)>) {
    stage.sort_by_key(|l| l.0);
    let mut k = 0;
    while k < stage.len() {
        let (li, mut ap) = stage[k];
        while k + 1 < stage.len() && stage[k + 1].0 == li {
            let other = stage[k + 1].1;
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
