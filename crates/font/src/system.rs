//! The fonts installed on the computer, found by family and style. Only each file's names are
//! read when looking; a font is opened when it is used.

use std::io::{Read, Seek, SeekFrom};
use std::sync::OnceLock;

/// One installed font.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub path: String,
    /// Which font of the file (collections hold several).
    pub index: usize,
    pub family: String,
    pub style: String,
    /// The full name ("Tahoma Bold") and the PostScript name ("Tahoma-Bold").
    pub full: String,
    pub postscript: String,
    /// 100 (thin) to 900 (black).
    pub weight: u16,
    pub italic: bool,
    /// It can be any weight along a weight axis.
    pub variable: bool,
}

/// The folders fonts are installed in.
fn folders() -> Vec<String> {
    let mut f = vec!["/System/Library/Fonts".to_string(), "/System/Library/Fonts/Supplemental".to_string(), "/Library/Fonts".to_string()];
    if let Some(home) = std::env::var_os("HOME") {
        f.push(format!("{}/Library/Fonts", home.to_string_lossy()));
    }
    f
}

fn read_at(file: &mut std::fs::File, at: u64, len: usize) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(at)).ok()?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn be16(b: &[u8], at: usize) -> u16 {
    b.get(at..at + 2).map_or(0, |x| u16::from_be_bytes([x[0], x[1]]))
}

fn be32(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).map_or(0, |x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]))
}

/// A name from a `name` table's bytes, English first.
fn name_in(t: &[u8], id: u16) -> Option<String> {
    let (count, strings) = (be16(t, 2) as usize, be16(t, 4) as usize);
    let mut best: Option<(u8, String)> = None;
    for i in 0..count {
        let r = 6 + 12 * i;
        let (platform, encoding, lang, nid) = (be16(t, r), be16(t, r + 2), be16(t, r + 4), be16(t, r + 6));
        if nid != id {
            continue;
        }
        let (len, off) = (be16(t, r + 8) as usize, be16(t, r + 10) as usize);
        let Some(bytes) = t.get(strings + off..strings + off + len) else { continue };
        let (rank, text) = match (platform, encoding) {
            (3, 1) | (3, 10) | (0, _) => {
                let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                (if platform == 3 && lang == 0x409 { 0 } else if platform == 0 { 1 } else { 2 }, String::from_utf16_lossy(&units))
            }
            // the Mac's own names, English when the language is 0 (some fonts have their
            // English names only here, and the others in other languages)
            (1, 0) => (if lang == 0 { 1 } else { 3 }, bytes.iter().map(|&b| b as char).collect()),
            _ => continue,
        };
        if best.as_ref().is_none_or(|(r, _)| rank < *r) {
            best = Some((rank, text));
        }
    }
    best.map(|(_, t)| t)
}

/// The fonts of one file.
fn faces_of(path: &str) -> Vec<Face> {
    let Ok(mut file) = std::fs::File::open(path) else { return Vec::new() };
    let Some(head) = read_at(&mut file, 0, 12) else { return Vec::new() };
    let starts: Vec<u32> = if &head[..4] == b"ttcf" {
        let n = be32(&head, 8) as usize;
        read_at(&mut file, 12, 4 * n).map(|b| (0..n).map(|i| be32(&b, 4 * i)).collect()).unwrap_or_default()
    } else {
        vec![0]
    };
    let mut out = Vec::new();
    for (index, start) in starts.into_iter().enumerate() {
        let Some(dir) = read_at(&mut file, start as u64, 12) else { continue };
        let n = be16(&dir, 4) as usize;
        let Some(records) = read_at(&mut file, start as u64 + 12, 16 * n) else { continue };
        let table = |tag: &[u8; 4]| (0..n).map(|i| &records[16 * i..16 * i + 16]).find(|r| &r[..4] == tag).map(|r| (be32(r, 8) as u64, be32(r, 12) as usize));
        // outlines we draw: TrueType, or PostScript (CFF, CFF2)
        if table(b"glyf").is_none() && table(b"CFF ").is_none() && table(b"CFF2").is_none() {
            continue;
        }
        let Some((at, len)) = table(b"name") else { continue };
        let Some(names) = read_at(&mut file, at, len) else { continue };
        let os2 = table(b"OS/2").and_then(|(at, len)| read_at(&mut file, at, len.min(96)));
        let family = name_in(&names, 16).or_else(|| name_in(&names, 1)).unwrap_or_default();
        if family.is_empty() {
            continue;
        }
        out.push(Face {
            path: path.to_string(),
            index,
            style: name_in(&names, 17).or_else(|| name_in(&names, 2)).unwrap_or_else(|| "Regular".into()),
            full: name_in(&names, 4).unwrap_or_else(|| family.clone()),
            postscript: name_in(&names, 6).unwrap_or_default(),
            weight: os2.as_ref().map_or(400, |o| be16(o, 4)),
            italic: os2.as_ref().is_some_and(|o| be16(o, 62) & 0x201 != 0),
            variable: table(b"fvar").is_some(),
            family,
        });
    }
    out
}

/// Every installed font (looked for once).
pub fn faces() -> &'static [Face] {
    static ALL: OnceLock<Vec<Face>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = Vec::new();
        for dir in folders() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let mut paths: Vec<String> = entries.flatten().map(|e| e.path().to_string_lossy().into_owned()).collect();
            paths.sort();
            for p in paths {
                let lower = p.to_lowercase();
                if lower.ends_with(".ttf") || lower.ends_with(".ttc") || lower.ends_with(".otf") {
                    all.extend(faces_of(&p));
                }
            }
        }
        all
    })
}

/// The names of the installed families, each once.
pub fn families() -> Vec<String> {
    let mut f: Vec<String> = faces().iter().filter(|f| !f.family.starts_with('.')).map(|f| f.family.clone()).collect();
    f.sort();
    f.dedup();
    f
}

/// The font of `family` (any case) nearest `weight` and `italic`; a family name may also be a
/// full or PostScript name.
pub fn find(family: &str, weight: u16, italic: bool) -> Option<&'static Face> {
    let all = faces();
    let named = |f: &&Face| f.family.eq_ignore_ascii_case(family);
    let mut of_family: Vec<&Face> = all.iter().filter(named).collect();
    if of_family.is_empty() {
        of_family = all.iter().filter(|f| f.full.eq_ignore_ascii_case(family) || f.postscript.eq_ignore_ascii_case(family)).collect();
    }
    // among faces as near, the family's usual style before others of its weight (ornaments,
    // display cuts, small capitals)
    let usual = |f: &Face| ["regular", "roman", "normal", "book", "plain", "medium"].contains(&f.style.to_lowercase().as_str()) || f.full.eq_ignore_ascii_case(&f.family);
    of_family.into_iter().min_by_key(|f| (u16::from(f.italic != italic), (f.weight as i32 - weight as i32).unsigned_abs(), u16::from(!usual(f)), f.index))
}

/// A font that has `c`, for text whose own font lacks it: the usual one of its script first.
pub fn fallback_for(c: char) -> Option<&'static Face> {
    let preferred: &[&str] = match c as u32 {
        0x0E00..=0x0E7F => &[".ThonburiUI", "Thonburi", "Tahoma"],
        // kana, CJK ideographs and their punctuation, full-width forms
        0x3000..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF => &["Hiragino Sans", "Hiragino Kaku Gothic ProN", "PingFang SC", "Noto Sans CJK JP", "Arial Unicode MS"],
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => &["Apple SD Gothic Neo", "AppleGothic", "Noto Sans CJK KR", "Arial Unicode MS"],
        // fonts with OpenType forms first (Geeza Pro's joining is in Apple's own tables)
        0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => &[".SF Arabic", "SF Arabic", "Arial", "Damascus", "Geeza Pro", "Arial Unicode MS"],
        0x0590..=0x05FF => &["Arial Hebrew", "Arial", "Arial Unicode MS"],
        0x0900..=0x097F | 0xA8E0..=0xA8FF | 0x1CD0..=0x1CFF => &["Kohinoor Devanagari", "Devanagari Sangam MN", "Arial Unicode MS"],
        0x0980..=0x09FF => &["Kohinoor Bangla", "Bangla Sangam MN", "Bangla MN"],
        0x0A00..=0x0A7F => &["Gurmukhi Sangam MN", "Gurmukhi MN", "Mukta Mahee"],
        0x0A80..=0x0AFF => &["Kohinoor Gujarati", "Gujarati Sangam MN"],
        0x0B00..=0x0B7F => &["Noto Sans Oriya", "Oriya Sangam MN"],
        0x0B80..=0x0BFF => &["Tamil Sangam MN", "Tamil MN"],
        0x0C00..=0x0C7F => &["Kohinoor Telugu", "Telugu Sangam MN"],
        0x0C80..=0x0CFF => &["Noto Sans Kannada", "Kannada Sangam MN"],
        0x0D00..=0x0D7F => &["Malayalam Sangam MN", "Malayalam MN"],
        0x0D80..=0x0DFF => &["Sinhala Sangam MN", "Sinhala MN"],
        0x0E80..=0x0EFF => &["Lao Sangam MN", "Arial Unicode MS"],
        0x1780..=0x17FF => &["Khmer Sangam MN", "Arial Unicode MS"],
        0x1000..=0x109F => &["Myanmar Sangam MN", "Noto Sans Myanmar", "Arial Unicode MS"],
        0x0370..=0x04FF => &["System Font", "Helvetica", "Arial"],
        // emoji, and what joins and dresses them (joiners, variation selectors, keycaps, tags)
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x200D | 0xFE0F | 0x20E3 | 0xE0020..=0xE007F | 0x2300..=0x23FF => &["Apple Color Emoji", "Noto Color Emoji", "Apple Symbols"],
        _ => &["System Font", "Helvetica", "Arial Unicode MS", "Apple Symbols"],
    };
    preferred.iter().find_map(|f| find(f, 400, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_fonts_are_found_by_name() {
        let t = find("Tahoma", 700, false).expect("Tahoma comes with macOS");
        assert_eq!((t.style.as_str(), t.weight), ("Bold", 700));
        let regular = find("tahoma", 400, false).unwrap();
        assert_eq!(regular.style, "Regular");
        assert_eq!(find("Tahoma-Bold", 400, false).map(|f| f.weight), Some(700), "found by PostScript name too");
        assert!(families().iter().any(|f| f == "Georgia"));
        let thai = fallback_for('ก').expect("a font for Thai");
        let font = crate::Font::open_face(&thai.path, thai.index).unwrap();
        assert_ne!(font.glyph('ก'), 0);
        assert!(find("No Such Family", 400, false).is_none());
        // a family's usual style before others as heavy (Hoefler Text before its ornaments), and
        // a font whose English names are only the Mac's own
        if let Some(h) = find("Hoefler Text", 400, false) {
            assert_eq!(h.style, "Regular");
        }
        if std::path::Path::new("/System/Library/Fonts/GeezaPro.ttc").exists() {
            assert_eq!(find("Geeza Pro", 400, false).map(|f| f.family.as_str()), Some("Geeza Pro"));
        }
    }
}
