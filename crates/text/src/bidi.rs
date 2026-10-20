//! The order text is seen in when right-to-left and left-to-right writing meet (Unicode's
//! bidirectional algorithm, UAX #9): each character's direction class, the paragraph's
//! direction, explicit embeddings, overrides and isolates, the weak and neutral rules (paired
//! brackets included), each character's level, and a line's characters put in the order they
//! are seen. Characters that are mirrored when written right to left (brackets) are mirrored.

use std::ops::Range;

/// A character's bidirectional class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    L,
    R,
    AL,
    EN,
    ES,
    ET,
    AN,
    CS,
    NSM,
    BN,
    B,
    S,
    WS,
    ON,
    LRE,
    LRO,
    RLE,
    RLO,
    PDF,
    LRI,
    RLI,
    FSI,
    PDI,
}

use Class::*;

/// The class of a character (the scripts and symbols in use; the rest left to right).
pub fn class_of(c: char) -> Class {
    let u = c as u32;
    match u {
        0x0A | 0x0D | 0x1C..=0x1E | 0x85 | 0x2029 => B,
        0x09 | 0x0B | 0x1F => S,
        0x0C | 0x20 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x205F | 0x3000 => WS,
        0x202A => LRE,
        0x202B => RLE,
        0x202C => PDF,
        0x202D => LRO,
        0x202E => RLO,
        0x2066 => LRI,
        0x2067 => RLI,
        0x2068 => FSI,
        0x2069 => PDI,
        0x200E => L,
        0x200F => R,
        0x061C => AL,
        0x00..=0x08 | 0x0E..=0x1B | 0x7F..=0x84 | 0x86..=0x9F | 0xAD | 0x180E | 0x200B..=0x200D | 0x2060..=0x2064 | 0x206A..=0x206F | 0xFEFF => BN,
        0x30..=0x39 | 0xB2 | 0xB3 | 0xB9 | 0x06F0..=0x06F9 | 0x2070 | 0x2074..=0x2079 | 0x2080..=0x2089 | 0x2488..=0x249B | 0xFF10..=0xFF19 | 0x1D7CE..=0x1D7FF => EN,
        0x2B | 0x2D | 0x207A | 0x207B | 0x208A | 0x208B | 0x2212 | 0xFB29 | 0xFE62 | 0xFE63 | 0xFF0B | 0xFF0D => ES,
        0x23..=0x25 | 0xA2..=0xA5 | 0xB0 | 0xB1 | 0x0609 | 0x060A | 0x066A | 0x09F2 | 0x09F3 | 0x0AF1 | 0x0BF9 | 0x0E3F | 0x17DB | 0x2030..=0x2034 | 0x20A0..=0x20CF | 0x212E | 0x2213 | 0xFE5F | 0xFE69 | 0xFE6A
        | 0xFF03..=0xFF05 | 0xFFE0 | 0xFFE1 | 0xFFE5 | 0xFFE6 => ET,
        0x0600..=0x0605 | 0x0660..=0x0669 | 0x066B | 0x066C | 0x06DD | 0x0890 | 0x0891 | 0x08E2 | 0x10E60..=0x10E7E => AN,
        0x2C | 0x2E | 0x2F | 0x3A | 0xA0 | 0x060C | 0x202F | 0x2044 | 0xFE50 | 0xFE52 | 0xFE55 | 0xFF0C | 0xFF0E | 0xFF0F | 0xFF1A => CS,
        // marks: they take the direction of what they sit on
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x05BF | 0x05C1 | 0x05C2 | 0x05C4 | 0x05C5 | 0x05C7 | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4 | 0x06E7 | 0x06E8
        | 0x06EA..=0x06ED | 0x0711 | 0x0730..=0x074A | 0x07A6..=0x07B0 | 0x07EB..=0x07F3 | 0x07FD | 0x0816..=0x0819 | 0x081B..=0x0823 | 0x0825..=0x0827 | 0x0829..=0x082D | 0x0859..=0x085B | 0x0898..=0x089F | 0x08CA..=0x08E1
        | 0x08E3..=0x0902 | 0x093A | 0x093C | 0x0941..=0x0948 | 0x094D | 0x0951..=0x0957 | 0x0962 | 0x0963 | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E | 0x0EB1 | 0x0EB4..=0x0EBC | 0x0EC8..=0x0ECE | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF | 0x20D0..=0x20F0 | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F => NSM,
        // right to left: Hebrew and its neighbours, then Arabic and the scripts written like it
        0x0590..=0x05FF | 0x07C0..=0x085F | 0xFB1D..=0xFB4F | 0x10800..=0x10CFF | 0x10D40..=0x10EBF | 0x10F00..=0x10F2F | 0x10F70..=0x10FFF | 0x1E800..=0x1EC6F | 0x1ED00..=0x1EDFF | 0x1EF00..=0x1EFFF => R,
        0x0606 | 0x0607 | 0x060E | 0x060F | 0x06DE | 0x06E9 => ON,
        0x0600..=0x07BF | 0x0860..=0x08FF | 0xFB50..=0xFD3D | 0xFD40..=0xFDFF | 0xFE70..=0xFEFE | 0x10D00..=0x10D3F | 0x10EC0..=0x10EFF | 0x10F30..=0x10F6F | 0x1EC70..=0x1ECFF | 0x1EE00..=0x1EEFF => AL,
        // neither way: punctuation and symbols
        0x21 | 0x22 | 0x26..=0x2A | 0x3B..=0x40 | 0x5B..=0x60 | 0x7B..=0x7E | 0xA1 | 0xA6..=0xA9 | 0xAB | 0xAC | 0xAE | 0xAF | 0xB4 | 0xB6..=0xB8 | 0xBB..=0xBF | 0xD7 | 0xF7 | 0x02B9 | 0x02BA | 0x02C2..=0x02CF | 0x02D2..=0x02DF
        | 0x02E5..=0x02ED | 0x02EF..=0x02FF | 0x0374 | 0x0375 | 0x037E | 0x0384 | 0x0385 | 0x0387 | 0x03F6 | 0x058A | 0x058D | 0x058E | 0x0BF3..=0x0BF8 | 0x0BFA | 0x0F3A..=0x0F3D | 0x1400 | 0x169B | 0x169C
        | 0x1800..=0x180A | 0x1FBD | 0x1FBF..=0x1FC1 | 0x1FCD..=0x1FCF | 0x1FDD..=0x1FDF | 0x1FED..=0x1FEF | 0x1FFD | 0x1FFE | 0x2010..=0x2027 | 0x2035..=0x2043 | 0x2045..=0x205E | 0x207C..=0x207E | 0x208C..=0x208E
        | 0x2100 | 0x2101 | 0x2103..=0x2106 | 0x2108 | 0x2109 | 0x2114 | 0x2116..=0x2118 | 0x211E..=0x2123 | 0x2125 | 0x2127 | 0x2129 | 0x213A | 0x213B | 0x2140..=0x2144 | 0x214A..=0x214D | 0x2150..=0x215F | 0x2189..=0x218B
        | 0x2190..=0x2211 | 0x2214..=0x2335 | 0x237B..=0x2394 | 0x2396..=0x2429 | 0x2440..=0x244A | 0x2460..=0x2487 | 0x24EA..=0x26AB | 0x26AD..=0x27FF | 0x2900..=0x2B73 | 0x2B76..=0x2BFF | 0x2CE5..=0x2CEA | 0x2CF9..=0x2CFF
        | 0x2E00..=0x2E5D | 0x2E80..=0x2FFF | 0x3001..=0x3004 | 0x3008..=0x3020 | 0x3030 | 0x303D..=0x303F | 0x309B | 0x309C | 0x30A0 | 0x30FB | 0x31C0..=0x31E3 | 0x321D | 0x321E | 0x3250..=0x325F | 0x327C..=0x327E
        | 0x32B1..=0x32BF | 0x32CC..=0x32CF | 0x3377..=0x337A | 0x33DE | 0x33DF | 0x33FF | 0x4DC0..=0x4DFF | 0xA490..=0xA4C6 | 0xA60D..=0xA60F | 0xA673 | 0xA67E | 0xA67F | 0xA700..=0xA721 | 0xA788 | 0xFD3E | 0xFD3F
        | 0xFE10..=0xFE19 | 0xFE30..=0xFE4F | 0xFE51 | 0xFE54 | 0xFE56..=0xFE5E | 0xFE60 | 0xFE61 | 0xFE64..=0xFE66 | 0xFE68 | 0xFE6B | 0xFF01 | 0xFF02 | 0xFF06..=0xFF0A | 0xFF1B..=0xFF20 | 0xFF3B..=0xFF40 | 0xFF5B..=0xFF65
        | 0xFFE2..=0xFFE4 | 0xFFE8..=0xFFEE | 0xFFF9..=0xFFFD | 0x1F000..=0x1FAFF => ON,
        _ => L,
    }
}

/// The character drawn for `c` when written right to left (brackets and the like turned).
pub fn mirror(c: char) -> char {
    const PAIRS: [(char, char); 22] = [
        ('(', ')'),
        ('[', ']'),
        ('{', '}'),
        ('<', '>'),
        ('«', '»'),
        ('‹', '›'),
        ('⁅', '⁆'),
        ('⁽', '⁾'),
        ('₍', '₎'),
        ('≤', '≥'),
        ('≪', '≫'),
        ('⊂', '⊃'),
        ('⊆', '⊇'),
        ('〈', '〉'),
        ('《', '》'),
        ('「', '」'),
        ('『', '』'),
        ('【', '】'),
        ('〔', '〕'),
        ('（', '）'),
        ('［', '］'),
        ('｛', '｝'),
    ];
    for (a, b) in PAIRS {
        if c == a {
            return b;
        }
        if c == b {
            return a;
        }
    }
    c
}

/// The bracket a character opens (its closing one) or closes (its opening one): (pair, opens).
fn bracket(c: char) -> Option<(char, bool)> {
    const OPEN: [(char, char); 12] = [('(', ')'), ('[', ']'), ('{', '}'), ('⁅', '⁆'), ('⁽', '⁾'), ('₍', '₎'), ('〈', '〉'), ('《', '》'), ('「', '」'), ('『', '』'), ('【', '】'), ('〔', '〕')];
    for (a, b) in OPEN {
        if c == a {
            return Some((b, true));
        }
        if c == b {
            return Some((a, false));
        }
    }
    None
}

/// Which way a paragraph goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    /// As its first letter that has a direction goes.
    #[default]
    Auto,
    Ltr,
    Rtl,
}

/// A paragraph's levels: even left to right, odd right to left.
#[derive(Clone, Debug)]
pub struct Levels {
    /// The paragraph's own level (0 or 1).
    pub base: u8,
    pub levels: Vec<u8>,
    /// Each character's class as written (for the rules at line ends).
    classes: Vec<Class>,
}

const MAX_DEPTH: u8 = 125;

fn is_isolate_start(c: Class) -> bool {
    matches!(c, LRI | RLI | FSI)
}

fn is_removed(c: Class) -> bool {
    matches!(c, LRE | RLE | LRO | RLO | PDF | BN)
}

fn is_ni(c: Class) -> bool {
    matches!(c, B | S | WS | ON | LRI | RLI | FSI | PDI)
}

/// The direction of the first letter that has one in `classes[from..to]` (skipping isolates).
fn first_strong(classes: &[Class], from: usize, to: usize) -> Option<u8> {
    let mut depth = 0;
    for &c in &classes[from..to] {
        match c {
            LRI | RLI | FSI => depth += 1,
            PDI if depth > 0 => depth -= 1,
            PDI => {}
            L if depth == 0 => return Some(0),
            R | AL if depth == 0 => return Some(1),
            B => break,
            _ => {}
        }
    }
    None
}

/// Where each isolate initiator's matching PDI is.
fn matching_pdis(classes: &[Class]) -> Vec<Option<usize>> {
    let mut out = vec![None; classes.len()];
    let mut open: Vec<usize> = Vec::new();
    for (i, &c) in classes.iter().enumerate() {
        if is_isolate_start(c) {
            open.push(i);
        } else if c == PDI
            && let Some(s) = open.pop()
        {
            out[s] = Some(i);
        } else if c == B {
            open.clear();
        }
    }
    out
}

/// The levels of one paragraph of `text` (no paragraph separators inside but at its end).
pub fn resolve(text: &[char], direction: Direction) -> Levels {
    let n = text.len();
    let orig: Vec<Class> = text.iter().map(|&c| class_of(c)).collect();
    let mut cls = orig.clone();
    let pdi = matching_pdis(&orig);
    let base = match direction {
        Direction::Ltr => 0,
        Direction::Rtl => 1,
        Direction::Auto => first_strong(&orig, 0, n).unwrap_or(0),
    };
    // explicit levels and directions (X1–X8)
    let mut levels = vec![base; n];
    struct Entry {
        level: u8,
        over: Option<Class>,
        isolate: bool,
    }
    let mut stack = vec![Entry { level: base, over: None, isolate: false }];
    let (mut over_iso, mut over_emb, mut valid_iso) = (0usize, 0usize, 0usize);
    for i in 0..n {
        let top = stack.last().expect("never empty");
        let (level, over) = (top.level, top.over);
        match orig[i] {
            RLE | LRE | RLO | LRO => {
                let rtl = matches!(orig[i], RLE | RLO);
                let next = if rtl { (level + 1) | 1 } else { (level + 2) & !1 };
                levels[i] = level;
                if next <= MAX_DEPTH && over_iso == 0 && over_emb == 0 {
                    let o = match orig[i] {
                        RLO => Some(R),
                        LRO => Some(L),
                        _ => None,
                    };
                    stack.push(Entry { level: next, over: o, isolate: false });
                } else if over_iso == 0 {
                    over_emb += 1;
                }
            }
            RLI | LRI | FSI => {
                levels[i] = level;
                if let Some(o) = over {
                    cls[i] = o;
                }
                let rtl = match orig[i] {
                    RLI => true,
                    LRI => false,
                    _ => first_strong(&orig, i + 1, pdi[i].unwrap_or(n)) == Some(1),
                };
                let next = if rtl { (level + 1) | 1 } else { (level + 2) & !1 };
                if next <= MAX_DEPTH && over_iso == 0 && over_emb == 0 {
                    valid_iso += 1;
                    stack.push(Entry { level: next, over: None, isolate: true });
                } else {
                    over_iso += 1;
                }
            }
            PDI => {
                if over_iso > 0 {
                    over_iso -= 1;
                } else if valid_iso > 0 {
                    over_emb = 0;
                    while stack.last().is_some_and(|e| !e.isolate) {
                        stack.pop();
                    }
                    stack.pop();
                    valid_iso -= 1;
                }
                let top = stack.last().expect("never empty");
                levels[i] = top.level;
                if let Some(o) = top.over {
                    cls[i] = o;
                }
            }
            PDF => {
                levels[i] = level;
                if over_iso > 0 {
                } else if over_emb > 0 {
                    over_emb -= 1;
                } else if stack.last().is_some_and(|e| !e.isolate) && stack.len() >= 2 {
                    stack.pop();
                }
            }
            B => levels[i] = base,
            BN => levels[i] = level,
            _ => {
                levels[i] = level;
                if let Some(o) = over {
                    cls[i] = o;
                }
            }
        }
    }
    // removed characters (X9) take the level of what is before them
    let kept: Vec<usize> = (0..n).filter(|&i| !is_removed(orig[i])).collect();
    // level runs, then the isolating run sequences (X10)
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for &i in &kept {
        match runs.last_mut() {
            Some(r) if levels[*r.last().expect("not empty")] == levels[i] => r.push(i),
            _ => runs.push(vec![i]),
        }
    }
    let mut sequences: Vec<Vec<usize>> = Vec::new();
    let mut used = vec![false; runs.len()];
    for k in 0..runs.len() {
        if used[k] {
            continue;
        }
        // a run starting with a PDI that matches an initiator joins that initiator's sequence
        if orig[runs[k][0]] == PDI && (0..runs[k][0]).any(|s| pdi[s] == Some(runs[k][0]) && is_isolate_start(orig[s])) {
            continue;
        }
        let mut seq = Vec::new();
        let mut at = k;
        loop {
            used[at] = true;
            seq.extend(&runs[at]);
            let last = *runs[at].last().expect("not empty");
            match (is_isolate_start(orig[last]), pdi[last]) {
                (true, Some(p)) => match runs.iter().position(|r| r[0] == p) {
                    Some(next) if !used[next] => at = next,
                    _ => break,
                },
                _ => break,
            }
        }
        sequences.push(seq);
    }
    for seq in &sequences {
        resolve_sequence(seq, &orig, &mut cls, &levels, base, text, &pdi, n);
    }
    // implicit levels (I1, I2)
    for &i in &kept {
        let l = levels[i];
        levels[i] = match (l % 2, cls[i]) {
            (0, R) => l + 1,
            (0, AN | EN) => l + 2,
            (1, L | EN | AN) => l + 1,
            _ => l,
        };
    }
    for i in 0..n {
        if is_removed(orig[i]) {
            levels[i] = if i == 0 { base } else { levels[i - 1] };
        }
    }
    Levels { base, levels, classes: orig }
}

/// The weak and neutral rules over one isolating run sequence (W1–W7, N0–N2).
#[allow(clippy::too_many_arguments)]
fn resolve_sequence(seq: &[usize], orig: &[Class], cls: &mut [Class], levels: &[u8], base: u8, text: &[char], pdi: &[Option<usize>], n: usize) {
    let level = levels[seq[0]];
    let before = (0..seq[0]).rev().find(|&j| !is_removed(orig[j])).map_or(base, |j| levels[j]);
    let last = *seq.last().expect("not empty");
    let after = if is_isolate_start(orig[last]) { base } else { (last + 1..n).find(|&j| !is_removed(orig[j])).map_or(base, |j| levels[j]) };
    let dir = |l: u8| if l % 2 == 1 { R } else { L };
    let sos = dir(level.max(before));
    let eos = dir(level.max(after));
    let e = dir(level);
    let m = seq.len();
    let at = |k: usize| seq[k];
    // W1: marks take what they follow
    for k in 0..m {
        if cls[at(k)] == NSM {
            cls[at(k)] = if k == 0 {
                sos
            } else {
                match cls[at(k - 1)] {
                    LRI | RLI | FSI | PDI => ON,
                    c => c,
                }
            };
        }
    }
    // W2, W3: numbers after Arabic letters are Arabic numbers; Arabic letters are right to left
    let mut strong = sos;
    for k in 0..m {
        match cls[at(k)] {
            L | R => strong = cls[at(k)],
            AL => {
                strong = AL;
                cls[at(k)] = R;
            }
            EN if strong == AL => cls[at(k)] = AN,
            _ => {}
        }
    }
    // W4: one separator between two numbers of a kind
    for k in 1..m.saturating_sub(1) {
        let (p, c, q) = (cls[at(k - 1)], cls[at(k)], cls[at(k + 1)]);
        if c == ES && p == EN && q == EN {
            cls[at(k)] = EN;
        } else if c == CS && p == q && matches!(p, EN | AN) {
            cls[at(k)] = p;
        }
    }
    // W5: terminators next to European numbers
    let mut k = 0;
    while k < m {
        if cls[at(k)] == ET {
            let start = k;
            while k < m && cls[at(k)] == ET {
                k += 1;
            }
            let touches = (start > 0 && cls[at(start - 1)] == EN) || (k < m && cls[at(k)] == EN);
            if touches {
                for j in start..k {
                    cls[at(j)] = EN;
                }
            }
        } else {
            k += 1;
        }
    }
    // W6: what separates or terminates nothing is neutral
    for k in 0..m {
        if matches!(cls[at(k)], ES | ET | CS) {
            cls[at(k)] = ON;
        }
    }
    // W7: European numbers after left-to-right letters are left to right
    let mut strong = sos;
    for k in 0..m {
        match cls[at(k)] {
            L | R => strong = cls[at(k)],
            EN if strong == L => cls[at(k)] = L,
            _ => {}
        }
    }
    // N0: paired brackets
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut open: Vec<(char, usize)> = Vec::new();
    for k in 0..m {
        if cls[at(k)] != ON {
            continue;
        }
        let Some((other, opens)) = bracket(text[at(k)]) else { continue };
        if opens {
            if open.len() >= 63 {
                break;
            }
            open.push((other, k));
        } else if let Some(p) = open.iter().rposition(|(want, _)| *want == text[at(k)] || (*want == '〉' && text[at(k)] == '〉')) {
            pairs.push((open[p].1, k));
            open.truncate(p);
        }
    }
    pairs.sort();
    let strong_of = |c: Class| match c {
        L => Some(L),
        R | AN | EN => Some(R),
        _ => None,
    };
    for (o, c) in pairs {
        let inside: Vec<Class> = (o + 1..c).filter_map(|k| strong_of(cls[at(k)])).collect();
        let set = if inside.contains(&e) {
            Some(e)
        } else if let Some(&opp) = inside.first() {
            let context = (0..o).rev().find_map(|k| strong_of(cls[at(k)])).unwrap_or(sos);
            Some(if context == opp { opp } else { e })
        } else {
            None
        };
        if let Some(d) = set {
            cls[at(o)] = d;
            cls[at(c)] = d;
            // marks right after a bracket that changed go with it
            for b in [o, c] {
                let mut k = b + 1;
                while k < m && orig[at(k)] == NSM {
                    cls[at(k)] = d;
                    k += 1;
                }
            }
        }
    }
    // N1, N2: neutrals between letters of one direction go that way, else the embedding's
    let mut k = 0;
    while k < m {
        if !is_ni(cls[at(k)]) {
            k += 1;
            continue;
        }
        let start = k;
        while k < m && is_ni(cls[at(k)]) {
            k += 1;
        }
        let side = |c: Class| match c {
            L => L,
            _ => R,
        };
        let prev = if start == 0 { sos } else { side(cls[at(start - 1)]) };
        let next = if k == m { eos } else { side(cls[at(k)]) };
        let d = if prev == next { prev } else { e };
        for j in start..k {
            cls[at(j)] = d;
        }
    }
    let _ = pdi;
}

impl Levels {
    /// The levels of the characters `range` of the paragraph set as one line: spaces before a
    /// tab or the line's end at the paragraph's level (L1).
    pub fn line(&self, range: Range<usize>) -> Vec<u8> {
        let mut out = self.levels[range.clone()].to_vec();
        let cls = &self.classes[range];
        let trailing = |c: Class| matches!(c, WS | LRI | RLI | FSI | PDI) || is_removed(c);
        let mut end = out.len();
        for k in (0..out.len()).rev() {
            match cls[k] {
                S | B => {
                    out[k] = self.base;
                    for j in (0..k).rev() {
                        if !trailing(cls[j]) {
                            break;
                        }
                        out[j] = self.base;
                    }
                    end = k;
                }
                _ => {}
            }
        }
        for j in (0..end).rev() {
            if !trailing(cls[j]) {
                break;
            }
            out[j] = self.base;
        }
        out
    }
}

/// The order a line's characters are seen in, left to right: indices into `levels` (L2).
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(&high) = levels.iter().max() else { return order };
    let low_odd = levels.iter().copied().filter(|l| l % 2 == 1).min().unwrap_or(high + 1);
    let mut lv = levels.to_vec();
    let mut level = high;
    while level >= low_odd && level > 0 {
        let mut k = 0;
        while k < lv.len() {
            if lv[k] >= level {
                let start = k;
                while k < lv.len() && lv[k] >= level {
                    k += 1;
                }
                order[start..k].reverse();
                lv[start..k].reverse();
            } else {
                k += 1;
            }
        }
        level -= 1;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(s: &str, d: Direction) -> Vec<u8> {
        let t: Vec<char> = s.chars().collect();
        resolve(&t, d).levels
    }

    #[test]
    fn hebrew_inside_english() {
        let s = "car is מכונית in Hebrew";
        let l = levels(s, Direction::Auto);
        let t: Vec<char> = s.chars().collect();
        for (k, c) in t.iter().enumerate() {
            let want = if ('\u{05D0}'..='\u{05EA}').contains(c) { 1 } else { 0 };
            assert_eq!(l[k], want, "{c} at {k}");
        }
        let order = visual_order(&resolve(&t, Direction::Auto).line(0..t.len()));
        let seen: String = order.iter().map(|&i| t[i]).collect();
        assert_eq!(seen, "car is תינוכמ in Hebrew");
    }

    #[test]
    fn arabic_with_numbers() {
        // a right-to-left paragraph: the number stays left to right inside it
        let s = "السعر 123 دولار";
        let t: Vec<char> = s.chars().collect();
        let lv = resolve(&t, Direction::Auto);
        assert_eq!(lv.base, 1);
        assert_eq!(&lv.levels[6..9], &[2, 2, 2]);
        assert_eq!(lv.levels[5], 1, "the space between them goes right to left");
        let order = visual_order(&lv.line(0..t.len()));
        let seen: String = order.iter().map(|&i| t[i]).collect();
        assert_eq!(seen, "رالود 123 رعسلا");
    }

    #[test]
    fn brackets_and_mirrors() {
        // brackets round right-to-left text inside left-to-right text go with what is inside
        let s = "a (ב) c";
        let l = levels(s, Direction::Ltr);
        assert_eq!(l, vec![0, 0, 0, 1, 0, 0, 0]);
        let s2 = "א (b) ג";
        let l2 = levels(s2, Direction::Rtl);
        assert_eq!(l2[2], 1, "the bracket goes with the paragraph");
        assert_eq!(l2[3], 2, "the letter inside is left to right");
        assert_eq!(mirror('('), ')');
        assert_eq!(mirror('»'), '«');
    }

    #[test]
    fn explicit_embeddings_and_isolates() {
        // an isolate keeps what is inside from its surroundings
        let s = "\u{2067}abc\u{2069} def";
        let l = levels(s, Direction::Ltr);
        assert_eq!(l[1], 2, "left-to-right letters inside a right-to-left isolate");
        assert_eq!(l[0], 0);
        // an override makes Latin letters right to left
        let s2 = "\u{202E}abc\u{202C}";
        let t: Vec<char> = s2.chars().collect();
        let lv = resolve(&t, Direction::Ltr);
        assert_eq!(&lv.levels[1..4], &[1, 1, 1]);
        let order = visual_order(&lv.line(0..t.len()));
        let seen: String = order.iter().filter(|&&i| class_of(t[i]) == L).map(|&i| t[i]).collect();
        assert_eq!(seen, "cba");
    }

    #[test]
    fn spaces_at_the_end_of_a_line_go_with_the_paragraph() {
        let s = "שלום ";
        let t: Vec<char> = s.chars().collect();
        let lv = resolve(&t, Direction::Ltr);
        assert_eq!(lv.line(0..t.len())[4], 0);
    }

    /// The system's own bidirectional levels (ICU), for checking ours.
    #[cfg(target_os = "macos")]
    fn icu_levels(text: &[char], para: u8) -> (Vec<u8>, Vec<usize>) {
        use std::ffi::c_void;
        #[link(name = "icucore")]
        unsafe extern "C" {
            fn ubidi_openSized(max_length: i32, max_runs: i32, err: *mut i32) -> *mut c_void;
            fn ubidi_setPara(b: *mut c_void, text: *const u16, length: i32, para_level: u8, embedding: *mut u8, err: *mut i32);
            fn ubidi_getLevels(b: *mut c_void, err: *mut i32) -> *const u8;
            fn ubidi_getVisualMap(b: *mut c_void, map: *mut i32, err: *mut i32);
            fn ubidi_close(b: *mut c_void);
        }
        let utf16: Vec<u16> = text.iter().map(|&c| c as u32 as u16).collect();
        let mut err = 0i32;
        unsafe {
            let b = ubidi_openSized(utf16.len() as i32 + 1, 0, &mut err);
            ubidi_setPara(b, utf16.as_ptr(), utf16.len() as i32, para, std::ptr::null_mut(), &mut err);
            let l = ubidi_getLevels(b, &mut err);
            let out = std::slice::from_raw_parts(l, utf16.len()).to_vec();
            let mut map = vec![0i32; utf16.len()];
            ubidi_getVisualMap(b, map.as_mut_ptr(), &mut err);
            ubidi_close(b);
            assert!(err <= 0, "ICU error {err}");
            (out, map.into_iter().map(|i| i as usize).collect())
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_same_levels_as_the_system() {
        let pool: Vec<char> = "abcXYZ שלוםאב سلامبت 0123٤٥ +-%$:,./ ()[]<>!?\u{202B}\u{202A}\u{202E}\u{202D}\u{202C}\u{2067}\u{2066}\u{2068}\u{2069}\u{064E}\u{05B0}\u{0301}".chars().collect();
        let mut seed: u64 = 7;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        let mut checked = 0;
        for round in 0..3000 {
            let len = 1 + next() % 24;
            let text: Vec<char> = (0..len).map(|_| pool[next() % pool.len()]).collect();
            for (dir, para) in [(Direction::Auto, 0xFEu8), (Direction::Ltr, 0), (Direction::Rtl, 1)] {
                let ours = resolve(&text, dir);
                let mine = ours.line(0..text.len());
                let (sys, sys_order) = icu_levels(&text, para);
                // (UAX #9's N0 has a mark right after a bracket go with it; this ICU does not)
                let shown = |k: usize| !is_removed(class_of(text[k])) && !(class_of(text[k]) == NSM && k > 0 && bracket(text[k - 1]).is_some());
                for k in (0..text.len()).filter(|&k| shown(k)) {
                    assert_eq!(mine[k] % 2, sys[k] % 2, "round {round} {dir:?}: {:?} at {k} ({:?}); ours {mine:?}, the system's {sys:?}", text.iter().collect::<String>(), text[k]);
                }
                let a: Vec<usize> = visual_order(&mine).into_iter().filter(|&k| shown(k)).collect();
                let b: Vec<usize> = sys_order.into_iter().filter(|&k| shown(k)).collect();
                assert_eq!(a, b, "round {round} {dir:?}: the order of {:?}; levels ours {mine:?}, the system's {sys:?}", text.iter().collect::<String>());
                checked += 1;
            }
        }
        assert_eq!(checked, 9000);
    }
}
