//! The typefaces, read once from the system: its sans serif (SF), its serif italic (New York
//! Italic) for the words that are set apart, and a face for the scripts SF has no letters for
//! (Thai: the system's own Thai face for its interface).

use veymelo_font::Font;
use std::sync::OnceLock;

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

/// The face for what the others have no letters for (Thai), if the system has one.
pub fn other() -> Option<&'static Font> {
    static OTHER: OnceLock<Option<Font>> = OnceLock::new();
    OTHER.get_or_init(|| ["/System/Library/Fonts/ThonburiUI.ttc", "/System/Library/Fonts/Supplemental/Thonburi.ttc", "/System/Library/Fonts/Thonburi.ttc", "/System/Library/Fonts/Supplemental/SukhumvitSet.ttc"].iter().find_map(|p| Font::open(p).ok())).as_ref()
}

/// The system's monospaced face, for commands (the sans serif when there is none).
pub fn mono() -> Option<&'static Font> {
    static MONO: OnceLock<Option<Font>> = OnceLock::new();
    MONO.get_or_init(|| ["/System/Library/Fonts/SFNSMono.ttf", "/System/Library/Fonts/Menlo.ttc"].iter().find_map(|p| Font::open(p).ok()).or_else(|| sans().cloned())).as_ref()
}
