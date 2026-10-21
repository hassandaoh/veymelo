//! The typefaces, read once from the system: its sans serif (SF), its serif italic (New York
//! Italic) for the words that are set apart, and for the scripts these have no letters for
//! (Thai, Arabic, Hebrew, Japanese, Chinese, Korean, those of India, emoji…) the system's own
//! face for each, as heavy as the words around them.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use veymelo_font::{Font, system};

/// The sans serif at its usual design (None when the system has none we can read).
pub fn sans() -> Option<&'static Font> {
    static SANS: OnceLock<Option<Font>> = OnceLock::new();
    SANS.get_or_init(|| ["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc"].iter().find_map(|p| Font::open(p).ok())).as_ref()
}

/// The serif italic (the sans serif when there is none).
pub fn serif() -> Option<&'static Font> {
    static SERIF: OnceLock<Option<Font>> = OnceLock::new();
    SERIF.get_or_init(|| Font::open("/System/Library/Fonts/NewYorkItalic.ttf").ok().or_else(|| sans().cloned())).as_ref()
}

/// The sans serif at `weight`, drawn for `size` points.
pub fn sans_at(weight: f64, size: f64) -> Option<Font> {
    sans().map(|f| f.at(&[("wght", weight), ("opsz", size)]))
}

/// The system's face for `c`, for words whose face has no letter for it, `weight` heavy (its
/// family's nearest) and drawn for `size` points; None when the system has none that has it.
/// Each is found and opened once.
pub fn fallback(c: char, weight: f64, size: f64) -> Option<Font> {
    type Found = Mutex<HashMap<(char, u64), Option<&'static system::Face>>>;
    type Set = Mutex<HashMap<(String, usize, u64, u64), Font>>;
    static FOUND: OnceLock<Found> = OnceLock::new();
    static SET: OnceLock<Set> = OnceLock::new();
    let found = *FOUND.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).entry((c, weight.to_bits())).or_insert_with(|| {
        let usual = system::fallback_for(c)?;
        // the family's face nearest the weight, if it has the letter too
        let near = system::find(&usual.family, weight.round().clamp(1.0, 1000.0) as u16, false);
        Some(near.filter(|f| system::open(f).is_some_and(|font| font.glyph(c) != 0)).unwrap_or(usual))
    });
    let face = found?;
    let key = (face.path.clone(), face.index, weight.to_bits(), size.to_bits());
    let set = SET.get_or_init(Default::default);
    if let Some(f) = set.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Some(f.clone());
    }
    let font = system::open(face)?;
    let has: Vec<String> = font.axes().into_iter().map(|a| a.0).collect();
    let settings: Vec<(&str, f64)> = [("wght", weight), ("opsz", size)].into_iter().filter(|(t, _)| has.iter().any(|h| h == t)).collect();
    let font = if settings.is_empty() { font } else { font.at(&settings) };
    if font.glyph(c) == 0 {
        return None;
    }
    set.lock().unwrap_or_else(|e| e.into_inner()).insert(key, font.clone());
    Some(font)
}

/// The system's monospaced face, for commands (the sans serif when there is none).
pub fn mono() -> Option<&'static Font> {
    static MONO: OnceLock<Option<Font>> = OnceLock::new();
    MONO.get_or_init(|| ["/System/Library/Fonts/SFNSMono.ttf", "/System/Library/Fonts/Menlo.ttc"].iter().find_map(|p| Font::open(p).ok()).or_else(|| sans().cloned())).as_ref()
}
