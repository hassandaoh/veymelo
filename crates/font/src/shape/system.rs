//! The scripts whose letters are put together inside each syllable (those of India, Khmer and
//! Myanmar): their runs shaped by the Mac's own text engine (CoreText) from the same font, so
//! they come out as the system sets them. Where there is no such engine they take the font's
//! default rules.

use super::{Glyph, Options};
use crate::Font;

/// `text` (characters from `first` on) shaped by the system with `font` and what `options` asks:
/// the glyphs in the order they are drawn. None where the system cannot take the font.
#[cfg(target_os = "macos")]
pub(super) fn shape(font: &Font, text: &[char], first: usize, options: &Options) -> Option<Vec<Glyph>> {
    mac::shape(font, text, first, options)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn shape(_: &Font, _: &[char], _: usize, _: &Options) -> Option<Vec<Glyph>> {
    None
}

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::{Arc, Mutex, OnceLock};

    use super::super::{Glyph, Options};
    use crate::Font;

    type Ref = *const c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Range {
        start: isize,
        len: isize,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Point {
        x: f64,
        y: f64,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDataCreateWithBytesNoCopy(a: Ref, bytes: *const u8, len: isize, dealloc: Ref) -> Ref;
        fn CFStringCreateWithCharacters(a: Ref, chars: *const u16, len: isize) -> Ref;
        fn CFDictionaryCreate(a: Ref, keys: *const Ref, values: *const Ref, n: isize, kc: Ref, vc: Ref) -> Ref;
        fn CFDictionaryGetValue(d: Ref, k: Ref) -> Ref;
        fn CFAttributedStringCreate(a: Ref, s: Ref, attrs: Ref) -> Ref;
        fn CFArrayCreate(a: Ref, values: *const Ref, n: isize, cb: Ref) -> Ref;
        fn CFArrayGetCount(a: Ref) -> isize;
        fn CFArrayGetValueAtIndex(a: Ref, i: isize) -> Ref;
        fn CFNumberCreate(a: Ref, kind: isize, value: *const c_void) -> Ref;
        fn CFEqual(a: Ref, b: Ref) -> bool;
        fn CFRelease(r: Ref);
        static kCFAllocatorNull: Ref;
        static kCFTypeArrayCallBacks: c_void;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }

    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontManagerCreateFontDescriptorsFromData(data: Ref) -> Ref;
        fn CTFontDescriptorCreateCopyWithAttributes(d: Ref, attrs: Ref) -> Ref;
        fn CTFontCreateWithFontDescriptor(d: Ref, size: f64, m: Ref) -> Ref;
        fn CTLineCreateWithAttributedString(s: Ref) -> Ref;
        fn CTLineGetGlyphRuns(l: Ref) -> Ref;
        fn CTRunGetGlyphCount(r: Ref) -> isize;
        fn CTRunGetGlyphs(r: Ref, range: Range, out: *mut u16);
        fn CTRunGetPositions(r: Ref, range: Range, out: *mut Point);
        fn CTRunGetAdvances(r: Ref, range: Range, out: *mut Point);
        fn CTRunGetStringIndices(r: Ref, range: Range, out: *mut isize);
        fn CTRunGetAttributes(r: Ref) -> Ref;
        static kCTFontAttributeName: Ref;
        static kCTFontFeatureSettingsAttribute: Ref;
        static kCTFontOpenTypeFeatureTag: Ref;
        static kCTFontOpenTypeFeatureValue: Ref;
        static kCTFontVariationAttribute: Ref;
    }

    /// A system font made from one of ours, at one size with one set of features. The font's
    /// bytes are kept with it: the system reads them where they are.
    struct Made {
        font: Ref,
        _bytes: Arc<Vec<u8>>,
    }

    // the system's fonts may be used from any thread; they are never changed once made
    unsafe impl Send for Made {}

    /// The fonts made so far, by the bytes they were made from, the face, where it is on its
    /// axes, its size and its features.
    fn made() -> &'static Mutex<HashMap<String, Made>> {
        static MADE: OnceLock<Mutex<HashMap<String, Made>>> = OnceLock::new();
        MADE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    unsafe fn dict(keys: &[Ref], values: &[Ref]) -> Ref {
        unsafe { CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), values.as_ptr(), keys.len() as isize, &raw const kCFTypeDictionaryKeyCallBacks as Ref, &raw const kCFTypeDictionaryValueCallBacks as Ref) }
    }

    unsafe fn number_i32(v: i32) -> Ref {
        unsafe { CFNumberCreate(std::ptr::null(), 3, &raw const v as *const c_void) }
    }

    unsafe fn number_f64(v: f64) -> Ref {
        unsafe { CFNumberCreate(std::ptr::null(), 13, &raw const v as *const c_void) }
    }

    /// The system's font for `font` at `size` with `features` (tag and on), made once.
    fn system_font(font: &Font, size: f64, features: &[([u8; 4], bool)]) -> Option<Ref> {
        let bytes = font.shared_data();
        let user = font.user_coords();
        let key = format!("{:p}/{}/{}/{:?}/{size}/{features:?}", Arc::as_ptr(&bytes), bytes.len(), font.face, user);
        let mut all = made().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(m) = all.get(&key) {
            return Some(m.font);
        }
        unsafe {
            let data = CFDataCreateWithBytesNoCopy(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, kCFAllocatorNull);
            let faces = CTFontManagerCreateFontDescriptorsFromData(data);
            CFRelease(data);
            if faces.is_null() {
                return None;
            }
            if font.face as isize >= CFArrayGetCount(faces) {
                CFRelease(faces);
                return None;
            }
            let face = CFArrayGetValueAtIndex(faces, font.face as isize);
            let mut keys = vec![];
            let mut values = vec![];
            let settings: Vec<Ref> = features
                .iter()
                .map(|(tag, on)| {
                    let t: Vec<u16> = String::from_utf8_lossy(tag).encode_utf16().collect();
                    let tag = CFStringCreateWithCharacters(std::ptr::null(), t.as_ptr(), t.len() as isize);
                    let value = number_i32(i32::from(*on));
                    dict(&[kCTFontOpenTypeFeatureTag, kCTFontOpenTypeFeatureValue], &[tag, value])
                })
                .collect();
            if !settings.is_empty() {
                keys.push(kCTFontFeatureSettingsAttribute);
                values.push(CFArrayCreate(std::ptr::null(), settings.as_ptr(), settings.len() as isize, &raw const kCFTypeArrayCallBacks as Ref));
            }
            if !user.is_empty() {
                let (axes, at): (Vec<Ref>, Vec<Ref>) = user.iter().map(|(tag, v)| (number_i32(u32::from_be_bytes(*tag) as i32), number_f64(*v))).unzip();
                keys.push(kCTFontVariationAttribute);
                values.push(dict(&axes, &at));
            }
            let descriptor = if keys.is_empty() { face } else { CTFontDescriptorCreateCopyWithAttributes(face, dict(&keys, &values)) };
            let made_font = CTFontCreateWithFontDescriptor(descriptor, size, std::ptr::null());
            if made_font.is_null() {
                return None;
            }
            all.insert(key, Made { font: made_font, _bytes: bytes });
            Some(made_font)
        }
    }

    pub(super) fn shape(font: &Font, text: &[char], first: usize, options: &Options) -> Option<Vec<Glyph>> {
        if text.is_empty() {
            return Some(Vec::new());
        }
        let size = options.size.filter(|s| *s > 0.0).unwrap_or(font.units);
        let mut features = options.features.clone();
        if options.no_kerning {
            features.push((*b"kern", false));
        }
        let ct = system_font(font, size, &features)?;
        // each UTF-16 unit back to the character it is part of
        let mut utf16 = Vec::with_capacity(text.len() * 2);
        let mut owner = Vec::with_capacity(text.len() * 2);
        for (i, c) in text.iter().enumerate() {
            let mut buf = [0u16; 2];
            for u in c.encode_utf16(&mut buf) {
                utf16.push(*u);
                owner.push(i);
            }
        }
        let k = font.units / size;
        let mut out = Vec::with_capacity(text.len());
        unsafe {
            let string = CFStringCreateWithCharacters(std::ptr::null(), utf16.as_ptr(), utf16.len() as isize);
            let attrs = dict(&[kCTFontAttributeName], &[ct]);
            let attributed = CFAttributedStringCreate(std::ptr::null(), string, attrs);
            let line = CTLineCreateWithAttributedString(attributed);
            let runs = CTLineGetGlyphRuns(line);
            let mut pen = 0.0;
            for r in 0..CFArrayGetCount(runs) {
                let run = CFArrayGetValueAtIndex(runs, r);
                let n = CTRunGetGlyphCount(run) as usize;
                let (mut ids, mut at, mut adv, mut index) = (vec![0u16; n], vec![Point::default(); n], vec![Point::default(); n], vec![0isize; n]);
                let all = Range { start: 0, len: 0 };
                CTRunGetGlyphs(run, all, ids.as_mut_ptr());
                CTRunGetPositions(run, all, at.as_mut_ptr());
                CTRunGetAdvances(run, all, adv.as_mut_ptr());
                CTRunGetStringIndices(run, all, index.as_mut_ptr());
                // a run the system drew in another font (for what this one lacks) is ours as
                // missing glyphs
                let theirs = CFDictionaryGetValue(CTRunGetAttributes(run), kCTFontAttributeName);
                let own = theirs.is_null() || CFEqual(theirs, ct);
                for g in 0..n {
                    let cluster = first + owner.get(index[g].max(0) as usize).copied().unwrap_or(0);
                    let (x, y) = (at[g].x * k, at[g].y * k);
                    let advance = adv[g].x * k;
                    let id = if own { ids[g] } else { 0 };
                    out.push(Glyph { id, cluster, advance, dx: x - pen, dy: y });
                    pen += advance;
                }
            }
            for r in [line, attributed, attrs, string] {
                CFRelease(r);
            }
        }
        Some(out)
    }
}
