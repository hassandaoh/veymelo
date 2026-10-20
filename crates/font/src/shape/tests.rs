//! The shaper checked against the system's own (CoreText), on fonts that come with macOS.

use super::*;

/// The system's shaping of `text` with the font file at `path` (its first face), at one unit a
/// font unit: each glyph and where it is drawn (x, y).
#[cfg(target_os = "macos")]
fn system_shaping(path: &str, text: &str) -> Vec<(u16, f64, f64)> {
    system_shaping_with(path, text, &[])
}

/// The same with OpenType features switched on or off.
#[cfg(target_os = "macos")]
fn system_shaping_with(path: &str, text: &str, features: &[(&str, bool)]) -> Vec<(u16, f64, f64)> {
    system_shaping_at(path, text, features, None)
}

/// The same at `size` points, positions given back in font units.
#[cfg(target_os = "macos")]
fn system_shaping_at(path: &str, text: &str, features: &[(&str, bool)], size: Option<f64>) -> Vec<(u16, f64, f64)> {
    use std::ffi::c_void;
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
        fn CFDataCreate(a: Ref, bytes: *const u8, len: isize) -> Ref;
        fn CFStringCreateWithCharacters(a: Ref, chars: *const u16, len: isize) -> Ref;
        fn CFDictionaryCreate(a: Ref, keys: *const Ref, values: *const Ref, n: isize, kc: Ref, vc: Ref) -> Ref;
        fn CFAttributedStringCreate(a: Ref, s: Ref, attrs: Ref) -> Ref;
        fn CFArrayGetCount(a: Ref) -> isize;
        fn CFArrayCreate(a: Ref, values: *const Ref, n: isize, cb: Ref) -> Ref;
        fn CFNumberCreate(a: Ref, kind: isize, value: *const c_void) -> Ref;
        static kCFTypeArrayCallBacks: c_void;
        fn CFArrayGetValueAtIndex(a: Ref, i: isize) -> Ref;
        fn CFRelease(r: Ref);
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGDataProviderCreateWithCFData(d: Ref) -> Ref;
        fn CGFontCreateWithDataProvider(p: Ref) -> Ref;
    }
    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontCreateWithGraphicsFont(f: Ref, size: f64, m: Ref, attrs: Ref) -> Ref;
        fn CTFontDescriptorCreateWithAttributes(attrs: Ref) -> Ref;
        static kCTFontFeatureSettingsAttribute: Ref;
        static kCTFontOpenTypeFeatureTag: Ref;
        static kCTFontOpenTypeFeatureValue: Ref;
        static kCTFontFeatureTypeIdentifierKey: Ref;
        static kCTFontFeatureSelectorIdentifierKey: Ref;
        fn CTLineCreateWithAttributedString(s: Ref) -> Ref;
        fn CTLineGetGlyphRuns(l: Ref) -> Ref;
        fn CTRunGetGlyphCount(r: Ref) -> isize;
        fn CTRunGetGlyphs(r: Ref, range: Range, out: *mut u16);
        fn CTRunGetPositions(r: Ref, range: Range, out: *mut Point);
        static kCTFontAttributeName: Ref;
    }
    let bytes = std::fs::read(path).unwrap();
    let units = Font::open(path).unwrap().units;
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let mut out = Vec::new();
    unsafe {
        let data = CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize);
        let provider = CGDataProviderCreateWithCFData(data);
        let cg = CGFontCreateWithDataProvider(provider);
        let dict = |k: &[Ref], v: &[Ref]| CFDictionaryCreate(std::ptr::null(), k.as_ptr(), v.as_ptr(), k.len() as isize, &raw const kCFTypeDictionaryKeyCallBacks as Ref, &raw const kCFTypeDictionaryValueCallBacks as Ref);
        let settings: Vec<Ref> = features
            .iter()
            .map(|(tag, on)| {
                // an Apple setting named outright: "aat 37 1"
                if let Some(rest) = tag.strip_prefix("aat ") {
                    let n: Vec<i32> = rest.split(' ').map(|v| v.parse().unwrap()).collect();
                    let (ty, sel) = (CFNumberCreate(std::ptr::null(), 3, &raw const n[0] as *const c_void), CFNumberCreate(std::ptr::null(), 3, &raw const n[1] as *const c_void));
                    return dict(&[kCTFontFeatureTypeIdentifierKey, kCTFontFeatureSelectorIdentifierKey], &[ty, sel]);
                }
                let t: Vec<u16> = tag.encode_utf16().collect();
                let tag = CFStringCreateWithCharacters(std::ptr::null(), t.as_ptr(), t.len() as isize);
                let v: i32 = *on as i32;
                let value = CFNumberCreate(std::ptr::null(), 3, &raw const v as *const c_void);
                dict(&[kCTFontOpenTypeFeatureTag, kCTFontOpenTypeFeatureValue], &[tag, value])
            })
            .collect();
        let descriptor = if settings.is_empty() {
            std::ptr::null()
        } else {
            let list = CFArrayCreate(std::ptr::null(), settings.as_ptr(), settings.len() as isize, &raw const kCFTypeArrayCallBacks as Ref);
            CTFontDescriptorCreateWithAttributes(dict(&[kCTFontFeatureSettingsAttribute], &[list]))
        };
        let font = CTFontCreateWithGraphicsFont(cg, size.unwrap_or(units), std::ptr::null(), descriptor);
        let keys = [kCTFontAttributeName];
        let values = [font];
        let attrs = CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), values.as_ptr(), 1, &raw const kCFTypeDictionaryKeyCallBacks as Ref, &raw const kCFTypeDictionaryValueCallBacks as Ref);
        let string = CFStringCreateWithCharacters(std::ptr::null(), utf16.as_ptr(), utf16.len() as isize);
        let attributed = CFAttributedStringCreate(std::ptr::null(), string, attrs);
        let line = CTLineCreateWithAttributedString(attributed);
        let runs = CTLineGetGlyphRuns(line);
        for r in 0..CFArrayGetCount(runs) {
            let run = CFArrayGetValueAtIndex(runs, r);
            let n = CTRunGetGlyphCount(run) as usize;
            let mut glyphs = vec![0u16; n];
            let mut points = vec![Point::default(); n];
            CTRunGetGlyphs(run, Range { start: 0, len: 0 }, glyphs.as_mut_ptr());
            CTRunGetPositions(run, Range { start: 0, len: 0 }, points.as_mut_ptr());
            let k = units / size.unwrap_or(units);
            out.extend(glyphs.iter().zip(&points).map(|(g, p)| (*g, p.x * k, p.y * k)));
        }
        for r in [line, attributed, string, attrs, font, cg, provider, data] {
            CFRelease(r);
        }
    }
    out
}

/// Our shaping: each glyph and where it is drawn (a font with optical sizes at the size the
/// system's is asked at, one point a font unit, as the system sets it by itself; spaced for
/// that size too).
fn ours(path: &str, text: &str) -> Vec<(u16, f64, f64)> {
    ours_with(path, text, &[])
}

fn ours_with(path: &str, text: &str, features: &[(&str, bool)]) -> Vec<(u16, f64, f64)> {
    let font = Font::open(path).unwrap();
    let font = if font.axes().iter().any(|a| a.0 == "opsz") { font.at(&[("opsz", font.units)]) } else { font };
    let chars: Vec<char> = text.chars().collect();
    let mut pen = 0.0;
    let features = features.iter().map(|(t, on)| (t.as_bytes().try_into().unwrap(), *on)).collect();
    shape_with(&font, &chars, &Options { features, size: Some(font.units), ..Default::default() })
        .into_iter()
        .map(|g| {
            let at = (g.id, pen + g.dx, g.dy);
            pen += g.advance;
            at
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn same_as_system(path: &str, text: &str) {
    if !std::path::Path::new(path).exists() {
        return;
    }
    let (sys, mine) = (system_shaping(path, text), ours(path, text));
    let ids = |v: &[(u16, f64, f64)]| v.iter().map(|g| g.0).collect::<Vec<_>>();
    assert_eq!(ids(&mine), ids(&sys), "glyphs of {text:?} in {path}");
    for (k, (a, b)) in mine.iter().zip(&sys).enumerate() {
        assert!((a.1 - b.1).abs() <= 1.0 && (a.2 - b.2).abs() <= 1.0, "glyph {k} of {text:?} in {path}: ours {a:?}, the system's {b:?}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn thai_marks_sit_where_the_system_puts_them() {
    for text in ["สวัสดีครับ", "น้ำ ปี่ ฟ้า ฝั่ง", "ที่นี่ ป้ำ ญี่ปุ่น", "กรุงเทพมหานคร", "ฤๅษี ฦๅ ปฏิบัติ ฐาน", "ผู้ใหญ่ อยู่ เป็นต้น ๆ"] {
        same_as_system("/System/Library/Fonts/ThonburiUI.ttc", text);
        same_as_system("/System/Library/Fonts/Supplemental/Tahoma.ttf", text);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn latin_kerning_matches_the_system() {
    for text in ["AVATAR Wave To, yo", "Typography: fi fl office"] {
        same_as_system("/System/Library/Fonts/Supplemental/Arial.ttf", text);
        same_as_system("/System/Library/Fonts/Supplemental/Tahoma.ttf", text);
    }
}

#[test]
fn sara_am_splits_and_goes_before_tone_marks() {
    let thai: Vec<char> = "น้ำ".chars().collect();
    let prepared: Vec<char> = prepare_thai(&thai, 0).into_iter().map(|(c, _)| c).collect();
    assert_eq!(prepared, ['น', '\u{0E4D}', '\u{0E49}', '\u{0E32}']);
    assert!(prepare_thai(&thai, 0).iter().skip(1).all(|(_, k)| *k >= 1), "the new characters keep where they came from");
}

#[test]
fn scripts_and_runs() {
    assert_eq!(script_of('ก'), Some(Script::Thai));
    assert_eq!(script_of('a'), Some(Script::Latin));
    assert_eq!(script_of(' '), None);
    assert_eq!(script_of('1'), None);
}

#[cfg(target_os = "macos")]
#[test]
fn arabic_joins_as_the_system_joins() {
    for text in ["بسم الله", "مرحبا بالعالم", "كتاب", "لا إله إلا", "سلامٌ عليكم", "ـبـ"] {
        same_as_system("/System/Library/Fonts/Supplemental/Arial.ttf", text);
        same_as_system("/System/Library/Fonts/SFArabic.ttf", text);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn hebrew_comes_out_right_to_left() {
    for text in ["שלום עולם", "בְּרֵאשִׁית"] {
        same_as_system("/System/Library/Fonts/Supplemental/Arial.ttf", text);
    }
}

#[test]
fn arabic_letters_take_their_joining_forms() {
    let word: Vec<char> = "بيت".chars().collect();
    assert_eq!(forms(&word), vec![2, 3, 4], "first, middle, last");
    // alef joins only to what comes before it: the beh after it starts again
    let w2: Vec<char> = "باب".chars().collect();
    assert_eq!(forms(&w2), vec![2, 4, 1]);
    // marks are passed over
    let w3: Vec<char> = "بَب".chars().collect();
    assert_eq!(forms(&w3), vec![2, 0, 4]);
}


#[cfg(target_os = "macos")]
#[test]
fn features_on_and_off() {
    let path = "/System/Library/Fonts/Avenir Next Condensed.ttc";
    if !std::path::Path::new(path).exists() {
        return;
    }
    let font = Font::open(path).unwrap();
    let text: Vec<char> = "office fit".chars().collect();
    let usual = shape(&font, &text);
    let apart = shape_with(&font, &text, &Options { features: vec![(*b"liga", false)], ..Default::default() });
    assert!(usual.len() < apart.len(), "ligatures join letters: {} glyphs, {} without", usual.len(), apart.len());
    assert_eq!(apart.len(), text.len());
}


/// Where a font that comes with macOS is (the main folder or the supplemental one).
#[cfg(target_os = "macos")]
fn system_font(name: &str) -> Option<String> {
    ["/System/Library/Fonts/", "/System/Library/Fonts/Supplemental/"].iter().map(|d| format!("{d}{name}")).find(|p| std::path::Path::new(p).exists())
}

/// How our shaping and the system's differ (none when they agree to a font unit).
#[cfg(target_os = "macos")]
fn differs(path: &str, text: &str, features: &[(&str, bool)]) -> Option<String> {
    differs_asked(path, text, features, features)
}

/// The same, the system asked in its own terms (`theirs`) for what we are asked (`features`).
#[cfg(target_os = "macos")]
fn differs_asked(path: &str, text: &str, features: &[(&str, bool)], theirs: &[(&str, bool)]) -> Option<String> {
    let (sys, mine) = (system_shaping_with(path, text, theirs), ours_with(path, text, features));
    let ids = |v: &[(u16, f64, f64)]| v.iter().map(|g| g.0).collect::<Vec<_>>();
    if ids(&mine) != ids(&sys) {
        return Some(format!("glyphs: ours {:?}, the system's {:?}", ids(&mine), ids(&sys)));
    }
    for (k, (a, b)) in mine.iter().zip(&sys).enumerate() {
        if (a.1 - b.1).abs() > 1.0 || (a.2 - b.2).abs() > 1.0 {
            return Some(format!("glyph {k}: ours ({:.0}, {:.0}), the system's ({:.0}, {:.0})", a.1, a.2, b.1, b.2));
        }
    }
    None
}

/// Fonts made for the Mac with Apple's tables and none of OpenType's: each shapes as the system
/// shapes it.
#[cfg(target_os = "macos")]
#[test]
fn apple_tables_shape_as_the_system_does() {
    let latin = ["AVATAR Wave To, yo", "Typography: fi fl office", "Quick brown fox, the end. 1/2 \"st\""];
    let thai = ["สวัสดีครับ", "น้ำ ปี่ ฟ้า ฝั่ง", "ที่นี่ ป้ำ ญี่ปุ่น", "ฤๅษี ฦๅ ปฏิบัติ ฐาน", "ผู้ใหญ่ อยู่ เป็นต้น ๆ"];
    let arabic = ["بسم الله", "مرحبا بالعالم", "كتاب", "لا إله إلا", "سلامٌ عليكم", "محمد"];
    let hebrew = ["שלום עולם", "בְּרֵאשִׁית"];
    let fonts: &[(&str, &[&str])] = &[
        ("Helvetica.ttc", &latin),
        ("HelveticaNeue.ttc", &latin),
        ("Times.ttc", &latin),
        ("Courier.ttc", &latin),
        ("Menlo.ttc", &latin),
        ("Monaco.ttf", &latin),
        ("Geneva.ttf", &latin),
        ("LucidaGrande.ttc", &latin),
        ("Optima.ttc", &latin),
        ("Palatino.ttc", &latin),
        ("Hoefler Text.ttc", &latin),
        ("Zapfino.ttf", &latin),
        ("Apple Chancery.ttf", &latin),
        ("Skia.ttf", &latin),
        ("Didot.ttc", &latin),
        ("Baskerville.ttc", &latin),
        ("BigCaslon.ttf", &latin),
        ("AmericanTypewriter.ttc", &latin),
        ("Copperplate.ttc", &latin),
        ("Herculanum.ttf", &latin),
        ("Chalkboard.ttc", &latin),
        ("MarkerFelt.ttc", &latin),
        ("Cochin.ttc", &latin),
        ("Athelas.ttc", &latin),
        ("Marion.ttc", &latin),
        ("Thonburi.ttc", &thai),
        ("Sathu.ttf", &thai),
        ("GeezaPro.ttc", &arabic),
        ("Al Nile.ttc", &arabic),
        ("Baghdad.ttc", &arabic),
        ("Beirut.ttc", &arabic),
        ("DecoTypeNaskh.ttc", &arabic),
        ("Farah.ttc", &arabic),
        ("KufiStandardGK.ttc", &arabic),
        ("Nadeem.ttc", &arabic),
        ("Muna.ttc", &arabic),
        ("Sana.ttc", &arabic),
        ("Diwan Kufi.ttc", &arabic),
        ("Diwan Thuluth.ttf", &arabic),
        ("Farisi.ttf", &arabic),
        ("Mishafi.ttf", &arabic),
        ("Waseem.ttc", &arabic),
        ("AlBayan.ttc", &arabic),
        ("Al Tarikh.ttc", &arabic),
        ("Raanana.ttc", &hebrew),
        ("NewPeninimMT.ttc", &hebrew),
        ("Corsiva.ttc", &hebrew),
        ("DevanagariMT.ttc", &["नमस्ते दुनिया", "क्षत्रिय हिन्दी", "प्रेम कृष्ण"]),
        ("GujaratiMT.ttc", &["નમસ્તે ગુજરાતી"]),
        ("Gurmukhi.ttf", &["ਪੰਜਾਬੀ ਗੁਰਮੁਖੀ"]),
        ("Malayalam MN.ttc", &["മലയാളം ഭാഷ"]),
        ("Mshtakan.ttc", &["Բարեւ աշխարհ"]),
        ("PlantagenetCherokee.ttf", &["ᏣᎳᎩ ᎦᏬᏂᎯᏍᏗ"]),
        ("AppleGothic.ttf", &["한국어 글자"]),
        ("Kailasa.ttc", &["བོད་ཡིག"]),
    ];
    let mut wrong = Vec::new();
    let mut checked = 0;
    for (name, texts) in fonts {
        let Some(path) = system_font(name) else { continue };
        for text in texts.iter() {
            checked += 1;
            if let Some(why) = differs(&path, text, &[]) {
                wrong.push(format!("{name} {text:?}: {why}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{} of {checked} differ:\n{}", wrong.len(), wrong.join("\n"));
}

/// Fonts drawn for each size are spaced by size as the system spaces them (`trak`); others are
/// not, as the system leaves them.
#[cfg(target_os = "macos")]
#[test]
fn letters_spaced_by_size_as_the_system_spaces_them() {
    for name in ["SFNS.ttf", "NewYork.ttf", "SFCompact.ttf", "Hoefler Text.ttc", "Skia.ttf"] {
        let Some(path) = system_font(name) else { continue };
        let font = Font::open(&path).unwrap();
        for size in [3.0, 5.0, 8.0, 12.0, 17.0, 24.0, 36.0, 100.0, 400.0] {
            let sys = system_shaping_at(&path, "HHHH", &[], Some(size));
            let f = if font.axes().iter().any(|a| a.0 == "opsz") { font.at(&[("opsz", size)]) } else { font.clone() };
            let glyphs = shape_with(&f, &['H'; 4], &Options { size: Some(size), ..Default::default() });
            let (ours, theirs) = (glyphs[0].advance, sys[1].1 - sys[0].1);
            assert!((ours - theirs).abs() <= 1.0, "{name} at {size}: ours {ours}, the system's {theirs}");
        }
    }
}

/// Features by tag, on or off.
type Features<'a> = &'a [(&'a str, bool)];

/// Features asked of fonts with Apple's tables (their settings): as the system gives them.
#[cfg(target_os = "macos")]
#[test]
fn apple_settings_as_the_system_sets_them() {
    let text = "Office fit 1234/5678 Th 1st Hello";
    // the old "letter case" small capitals some fonts still have, and historical ligatures,
    // which the system leaves out when asked by OpenType's names: asked of it as Apple's own
    let old_small_caps: &[(&str, bool)] = &[("aat 3 3", true)];
    let historical: &[(&str, bool)] = &[("aat 1 20", true)];
    let asked: &[(&str, Features, Features)] = &[
        ("Hoefler Text.ttc", &[("smcp", true)], old_small_caps),
        ("Apple Chancery.ttf", &[("smcp", true)], old_small_caps),
        ("BigCaslon.ttf", &[("smcp", true)], old_small_caps),
        ("Baskerville.ttc", &[("hlig", true)], historical),
        ("Hoefler Text.ttc", &[("smcp", true), ("onum", true)], &[("aat 3 3", true), ("onum", true)]),
    ];
    let cases: &[(&str, &[(&str, bool)])] = &[
        ("Hoefler Text.ttc", &[("onum", true)]),
        ("Hoefler Text.ttc", &[("lnum", true)]),
        ("Hoefler Text.ttc", &[("dlig", true)]),
        ("Hoefler Text.ttc", &[("liga", false)]),
        ("Hoefler Text.ttc", &[("tnum", true)]),
        ("Hoefler Text.ttc", &[("onum", true), ("dlig", true)]),
        ("Zapfino.ttf", &[("liga", false)]),
        ("Zapfino.ttf", &[("frac", true)]),
        ("Zapfino.ttf", &[("dlig", true)]),
        ("Zapfino.ttf", &[("sups", true)]),
        ("Zapfino.ttf", &[("onum", true)]),
        ("Apple Chancery.ttf", &[("dlig", true)]),
        ("Apple Chancery.ttf", &[("frac", true)]),
        ("Apple Chancery.ttf", &[("ordn", true)]),
        ("Apple Chancery.ttf", &[("tnum", true)]),
        ("Baskerville.ttc", &[("smcp", true)]),
        ("Skia.ttf", &[("frac", true)]),
        ("Skia.ttf", &[("afrc", true)]),
        ("Skia.ttf", &[("subs", true)]),
        ("Didot.ttc", &[("smcp", true)]),
        ("Didot.ttc", &[("onum", true)]),
        ("Helvetica.ttc", &[("tnum", true)]),
        ("Helvetica.ttc", &[("liga", false)]),
        ("Helvetica.ttc", &[("kern", false)]),
    ];
    let mut wrong = Vec::new();
    for (name, features) in cases {
        let Some(path) = system_font(name) else { continue };
        if let Some(why) = differs(&path, text, features) {
            wrong.push(format!("{name} {features:?}: {why}"));
        }
    }
    for (name, features, theirs) in asked {
        let Some(path) = system_font(name) else { continue };
        if let Some(why) = differs_asked(&path, text, features, theirs) {
            wrong.push(format!("{name} {features:?}: {why}"));
        }
        // and they do something
        assert_ne!(ours_with(&path, text, features), ours(&path, text), "{name} {features:?} changes nothing");
    }
    assert!(wrong.is_empty(), "{} of {} differ:\n{}", wrong.len(), cases.len() + asked.len(), wrong.join("\n"));
}

/// Words of the nine scripts of India (conjuncts, reph, pre-base vowel signs and forms, two-part
/// vowel signs, nuktas written either way, joiners and non-joiners, marks with nothing to sit
/// on) in the fonts that come with macOS, OpenType's and Apple's.
#[cfg(target_os = "macos")]
#[test]
fn indic_scripts_shape_as_the_system_does() {
    let deva: &[&str] = &["नमस्ते", "हिन्दी", "क्षत्रिय", "र्क", "कि", "प्रेम", "द्ध", "श्री", "अर्थ", "धर्म", "कार्य", "राष्ट्र", "स्त्री", "ज्ञान", "क़िला", "क़िला", "क्\u{200D}ष", "क्\u{200C}ष", "रु", "रू", "हृदय", "ऋषि", "संस्कृत", "विद्यालय", "उद्घाटन", "ट्रक", "ड्र", "क्र", "त्र", "श्र", "द्व", "ह्म", "ह्य", "पँ", "कॅ", "ॐ", "्", "ि", "कि्", "र्कि", "र्किं", "र्\u{200D}क", "कर्\u{200D}", "स्त्र्य", "ऩ", "ऩ", "र्य", "ह्र", "न्न", "ट्ट", "क्त", "द्य", "त्त्व", "कें", "र्कें", "र्\u{200C}क", "१२३", "क।"];
    let beng: &[&str] = &["বাংলা", "কি", "র্ক", "ক্ষ", "কো", "কৌ", "রবীন্দ্রনাথ", "ঠাকুর", "শ্রী", "স্ত্রী", "ক্র", "প্র", "ব্যবহার", "জ্ঞান", "য়", "য়", "ড়", "ৎ", "র\u{200D}্য", "কার্য", "সূর্য", "ক্ক", "ন্ত", "ম্প", "কে", "কৈ", "র্কি", "ক্\u{200C}ষ"];
    let guru: &[&str] = &["ਪੰਜਾਬੀ", "ਕਿ", "ਗੁਰਮੁਖੀ", "ਸ਼", "ਸ਼", "ਖ਼ਾਲਸਾ", "ਪ੍ਰ", "ਕ੍ਰ", "ਸਿੱਖ", "ਦਿੱਲੀ", "ਕੈ", "ਕੌ", "ਸ੍ਵ", "ੴ"];
    let gujr: &[&str] = &["ગુજરાતી", "કિ", "ર્ક", "ક્ષ", "પ્ર", "શ્રી", "સ્ત્રી", "દ્ધ", "કૉ", "ર્કિ", "ક્ત", "દ્ય"];
    let orya: &[&str] = &["ଓଡ଼ିଆ", "କି", "ର୍କ", "କୋ", "କୈ", "କୌ", "ପ୍ର", "କ୍ଷ", "ଶ୍ରୀ", "କଁ", "ର୍କି", "ସ୍ତ୍ର"];
    let taml: &[&str] = &["தமிழ்", "கொ", "கை", "ஸ்ரீ", "கோ", "கௌ", "க்ஷ", "வணக்கம்", "தெரியும்", "கி", "கீ", "டி", "ஔ", "ஶ்ரீ"];
    let telu: &[&str] = &["తెలుగు", "క్ష", "కి", "ర్క", "ర్\u{200D}క", "శ్రీ", "ప్ర", "కై", "కొ", "స్త్రీ", "క్క", "ద్ద", "త్య", "కృ", "కౌ", "ర్కి"];
    let knda: &[&str] = &["ಕನ್ನಡ", "ಕಿ", "ರ್ಕ", "ಕೀ", "ಕೆ", "ಕೇ", "ಕೈ", "ಕೊ", "ಕೋ", "ಕೌ", "ಶ್ರೀ", "ಪ್ರ", "ಸ್ತ್ರೀ", "ರ್\u{200D}ಕ", "ಕ್ಕ", "ತ್ಯ", "ರ್ಕೆ", "ಕೃ"];
    let mlym: &[&str] = &["മലയാളം", "കൊ", "കോ", "കൌ", "ക്ര", "പ്ര", "ന്\u{200D}", "ൻ", "ക്ഷ", "ശ്രീ", "സ്ത്രീ", "ൎക", "ക്രി", "ക്കൊ", "ന്ന", "ല്ല", "ക്യ", "ക്വ", "ണ്ട", "ന്റെ", "ൾ", "കെ", "കൈ"];
    let cases: &[(&str, &[&str])] = &[
        ("/System/Library/Fonts/Kohinoor.ttc", deva),
        ("/System/Library/Fonts/Supplemental/Devanagari Sangam MN.ttc", deva),
        ("/System/Library/Fonts/Supplemental/ITFDevanagari.ttc", deva),
        ("/System/Library/Fonts/KohinoorBangla.ttc", beng),
        ("/System/Library/Fonts/Supplemental/Bangla Sangam MN.ttc", beng),
        ("/System/Library/Fonts/Supplemental/Bangla MN.ttc", beng),
        ("/System/Library/Fonts/Supplemental/Gurmukhi Sangam MN.ttc", guru),
        ("/System/Library/Fonts/Supplemental/Gurmukhi MN.ttc", guru),
        ("/System/Library/Fonts/KohinoorGujarati.ttc", gujr),
        ("/System/Library/Fonts/Supplemental/Gujarati Sangam MN.ttc", gujr),
        ("/System/Library/Fonts/NotoSansOriya.ttc", orya),
        ("/System/Library/Fonts/Supplemental/Oriya Sangam MN.ttc", orya),
        ("/System/Library/Fonts/Supplemental/Tamil Sangam MN.ttc", taml),
        ("/System/Library/Fonts/Supplemental/Tamil MN.ttc", taml),
        ("/System/Library/Fonts/KohinoorTelugu.ttc", telu),
        ("/System/Library/Fonts/Supplemental/Telugu Sangam MN.ttc", telu),
        ("/System/Library/Fonts/NotoSansKannada.ttc", knda),
        ("/System/Library/Fonts/Supplemental/Kannada Sangam MN.ttc", knda),
        ("/System/Library/Fonts/Supplemental/Malayalam Sangam MN.ttc", mlym),
        ("/System/Library/Fonts/Supplemental/Malayalam MN.ttc", mlym),
    ];
    let mut bad = Vec::new();
    for (path, texts) in cases {
        if !std::path::Path::new(path).exists() {
            continue;
        }
        for t in *texts {
            let (sys, mine) = (system_shaping(path, t), ours(path, t));
            let same_ids = sys.iter().map(|g| g.0).eq(mine.iter().map(|g| g.0));
            let same_pos = same_ids && sys.iter().zip(&mine).all(|(a, b)| (a.1 - b.1).abs() <= 1.0 && (a.2 - b.2).abs() <= 1.0);
            if !same_pos {
                bad.push(format!("{t} in {path}: ours {mine:?}, the system's {sys:?}"));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Khmer and Myanmar words (coeng and Ro, two-part vowel signs, register shifters, kinzi, medials,
/// stacked consonants, vowel signs with nothing to sit on), and Sinhala, in the fonts that come
/// with macOS.
#[cfg(target_os = "macos")]
#[test]
fn khmer_myanmar_and_sinhala_shape_as_the_system_does() {
    let khmr: &[&str] = &["ខ្មែរ", "កើ", "ក្រុម", "ស្រី", "ព្រះ", "កៅ", "ក្ក", "ស្ត្រី", "កុំ", "ភាសា", "ប្រទេសកម្ពុជា", "សួស្តី", "អរគុណ", "ខ្ញុំ", "ស្រឡាញ់", "ព្រះរាជាណាចក្រ", "ក្រៅ", "គ្រូ", "ឆ្នាំ", "ចំណេះ", "ក៏", "ក៊ី", "ម៉ោង", "ប៉ុន្តែ", "ក្សត្រ", "ឡើង", "ើ", "ក្រោយ"];
    let mymr: &[&str] = &["မြန်မာ", "ကို", "မြို့", "ကျွန်", "င်္ဂ", "ကြေး", "ပြော", "သီ", "နေ", "မင်္ဂလာပါ", "ကျေးဇူးတင်ပါတယ်", "မြန်မာနိုင်ငံ", "သင်္ချိုင်း", "ကြွ", "ဂြိုဟ်", "နှင့်", "ဆွေး", "ပြည်", "လှ", "ေ", "စ္စ", "ဿ", "ကော်"];
    let sinh: &[&str] = &["සිංහල", "ක්\u{200D}ෂ", "කෙ", "කො", "ශ්\u{200D}රී", "ක්\u{200D}ය", "ර්\u{200D}ක", "කෛ", "කෞ", "ස්ත්\u{200D}රී", "ශ්\u{200D}රී ලංකාව"];
    let cases: &[(&str, &[&str])] = &[
        ("/System/Library/Fonts/Supplemental/Khmer Sangam MN.ttf", khmr),
        ("/System/Library/Fonts/Supplemental/Khmer MN.ttc", khmr),
        ("/System/Library/Fonts/NotoSansMyanmar.ttc", mymr),
        ("/System/Library/Fonts/Supplemental/Myanmar Sangam MN.ttc", mymr),
        ("/System/Library/Fonts/Supplemental/Myanmar MN.ttc", mymr),
        ("/System/Library/Fonts/Supplemental/Sinhala Sangam MN.ttc", sinh),
        ("/System/Library/Fonts/Supplemental/Sinhala MN.ttc", sinh),
    ];
    let mut bad = Vec::new();
    for (path, texts) in cases {
        if !std::path::Path::new(path).exists() {
            continue;
        }
        for t in *texts {
            let (sys, mine) = (system_shaping(path, t), ours(path, t));
            let same = sys.iter().map(|g| g.0).eq(mine.iter().map(|g| g.0)) && sys.iter().zip(&mine).all(|(a, b)| (a.1 - b.1).abs() <= 1.0 && (a.2 - b.2).abs() <= 1.0);
            if !same {
                bad.push(format!("{t} in {path}: ours {mine:?}, the system's {sys:?}"));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
