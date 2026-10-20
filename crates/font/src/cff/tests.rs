//! PostScript outlines checked against the system's own (CoreText), on fonts that come with macOS.

use crate::{Font, Pen};

/// The bounding box of the outline a pen is given (curves by their points along them).
#[derive(Default)]
struct Bounds {
    min: [f64; 2],
    max: [f64; 2],
    any: bool,
    at: [f64; 2],
}

impl Bounds {
    fn add(&mut self, x: f64, y: f64) {
        if !self.any {
            (self.min, self.max, self.any) = ([x, y], [x, y], true);
        }
        self.min = [self.min[0].min(x), self.min[1].min(y)];
        self.max = [self.max[0].max(x), self.max[1].max(y)];
    }
}

impl Pen for Bounds {
    fn move_to(&mut self, x: f64, y: f64) {
        self.add(x, y);
        self.at = [x, y];
    }
    fn line_to(&mut self, x: f64, y: f64) {
        self.add(x, y);
        self.at = [x, y];
    }
    fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
        let [x0, y0] = self.at;
        for k in 1..=16 {
            let t = k as f64 / 16.0;
            let s = 1.0 - t;
            self.add(s * s * x0 + 2.0 * s * t * cx + t * t * x, s * s * y0 + 2.0 * s * t * cy + t * t * y);
        }
        self.at = [x, y];
    }
    fn cubic_to(&mut self, ax: f64, ay: f64, bx: f64, by: f64, x: f64, y: f64) {
        let [x0, y0] = self.at;
        for k in 1..=32 {
            let t = k as f64 / 32.0;
            let s = 1.0 - t;
            self.add(s * s * s * x0 + 3.0 * s * s * t * ax + 3.0 * s * t * t * bx + t * t * t * x, s * s * s * y0 + 3.0 * s * s * t * ay + 3.0 * s * t * t * by + t * t * t * y);
        }
        self.at = [x, y];
    }
    fn close(&mut self) {}
}

/// The system's outline bounds of glyph `g` of the first font in `path`, at one point a unit.
#[cfg(target_os = "macos")]
fn system_bounds(path: &str, g: u16) -> Option<[f64; 4]> {
    use std::ffi::c_void;
    type Ref = *const c_void;
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDataCreate(a: Ref, bytes: *const u8, len: isize) -> Ref;
        fn CFRelease(r: Ref);
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGDataProviderCreateWithCFData(d: Ref) -> Ref;
        fn CGFontCreateWithDataProvider(p: Ref) -> Ref;
        fn CGPathGetPathBoundingBox(p: Ref) -> Rect;
        fn CGPathIsEmpty(p: Ref) -> bool;
    }
    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontCreateWithGraphicsFont(f: Ref, size: f64, m: Ref, attrs: Ref) -> Ref;
        fn CTFontCreatePathForGlyph(f: Ref, g: u16, m: Ref) -> Ref;
    }
    let bytes = std::fs::read(path).ok()?;
    let units = Font::open(path).ok()?.units;
    unsafe {
        let data = CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize);
        let provider = CGDataProviderCreateWithCFData(data);
        let cg = CGFontCreateWithDataProvider(provider);
        if cg.is_null() {
            return None;
        }
        let font = CTFontCreateWithGraphicsFont(cg, units, std::ptr::null(), std::ptr::null());
        let p = CTFontCreatePathForGlyph(font, g, std::ptr::null());
        let out = if p.is_null() || CGPathIsEmpty(p) {
            None
        } else {
            let r = CGPathGetPathBoundingBox(p);
            Some([r.x, r.y, r.x + r.w, r.y + r.h])
        };
        if !p.is_null() {
            CFRelease(p);
        }
        for r in [font, cg, provider, data] {
            CFRelease(r);
        }
        out
    }
}

#[cfg(target_os = "macos")]
#[test]
fn postscript_outlines_match_the_system() {
    let fonts = ["/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", "/System/Library/Fonts/AppleSDGothicNeo.ttc", "/System/Library/Fonts/LastResort.otf"];
    let mut checked = 0;
    for path in fonts {
        if !std::path::Path::new(path).exists() {
            continue;
        }
        let font = Font::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let text: Vec<char> = "Aaあ漢字한글?&@é".chars().collect();
        for c in text {
            let g = font.glyph(c);
            if g == 0 {
                continue;
            }
            let Some(sys) = system_bounds(path, g) else { continue };
            let mut b = Bounds::default();
            font.outline(g, &mut b, &|x, y| (x, y));
            assert!(b.any, "{c:?} in {path} drew nothing");
            let ours = [b.min[0], b.min[1], b.max[0], b.max[1]];
            for k in 0..4 {
                assert!((ours[k] - sys[k]).abs() <= 1.5, "{c:?} (glyph {g}) in {path}: ours {ours:?}, the system's {sys:?}");
            }
            checked += 1;
        }
    }
    assert!(checked >= 10, "only {checked} glyphs checked");
}
