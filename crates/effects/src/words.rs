//! Text: a number (a count, a time, a date, in hexadecimal) or the timecode drawn on the layer
//! in a font — filled, outlined or both, over the layer or alone.
//!
//! The text is set by the same font engine as text layers: any installed font or font file,
//! the system's own when none is named.

use veymelo_color::Rgba;
use veymelo_geom::{Rect, Vec2, vec2};
use veymelo_keyframe::{Kind, Value};
use veymelo_path::shape::Sub;
use veymelo_path::{Path, Rule, Stroke};
use veymelo_project::PropDef;
use veymelo_raster::{Image, Px};
use veymelo_text::FontRef;
use veymelo_time::{FrameRate, Tick};

use crate::defs::{choice, color, effect, flag, pct, point, px, ranged};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "text";

/// A font setting: an installed font's name or a font file (empty: the system's own).
const fn font(id: &'static str, name: &'static str) -> PropDef {
    PropDef::new(id, name, Kind::Text, Value::Text(String::new())).doc("an installed font's name (family, full or PostScript) or a font file; empty: the system's own")
}

/// The font a name stands for, or the system's own when it names none that is installed.
pub(crate) fn font_for(name: &str, weight: f64) -> Option<FontRef> {
    let name = name.trim();
    (!name.is_empty())
        .then(|| veymelo_text::resolve(name, weight, false))
        .flatten()
        .or_else(|| veymelo_text::resolve("System Font", weight, false))
        .or_else(|| veymelo_text::resolve("Helvetica", weight, false))
        .or_else(|| veymelo_text::resolve("Arial", weight, false))
}

/// Character `c`'s glyph in `font`, or in another font of its script where this one lacks it.
pub(crate) fn glyph_in(font: &FontRef, c: char) -> (FontRef, u16) {
    let g = font.glyph(c);
    if g == 0 && !c.is_whitespace()
        && let Some(other) = veymelo_font::system::fallback_for(c).and_then(|f| veymelo_text::resolve(&f.full, 400.0, false)).filter(|f| f.glyph(c) != 0)
    {
        let g = other.glyph(c);
        return (other, g);
    }
    (font.clone(), g)
}

/// A line of text set in `font`, `size` layer units to an em, `tracking` more between
/// characters; with `even`, each digit in a cell as wide as the widest (so a changing number
/// stands still). Placed by `align` (0: its left end, 1: its middle, 2: its right end) at `at`
/// across, the middle of its capitals at `at` down. Its outlines (layer units) and the box from
/// its capitals' top to its baseline.
pub(crate) fn set_line(text: &str, font: &FontRef, size: f64, tracking: f64, even: bool, align: u32, at: Vec2) -> (Path, Rect) {
    let scale_of = |f: &FontRef| size / f.units.max(1.0);
    let scale = scale_of(font);
    let cell = ('0'..='9').map(|c| font.advance(font.glyph(c))).fold(0.0, f64::max) * scale;
    // each character's font (another of its script where this one lacks it), glyph, start and
    // advance
    let mut pen = 0.0;
    let mut placed: Vec<(FontRef, u16, f64)> = Vec::new();
    for c in text.chars() {
        let (face, g) = glyph_in(font, c);
        let adv = face.advance(g) * scale_of(&face);
        let (x, step) = if even && c.is_ascii_digit() { (pen + (cell - adv) * 0.5, cell) } else { (pen, adv) };
        placed.push((face, g, x));
        pen += step + tracking;
    }
    let width = (pen - if placed.is_empty() { 0.0 } else { tracking }).max(0.0);
    let cap = font.cap_height().unwrap_or(font.ascent * 0.7) * scale;
    let left = at.x - width * [0.0, 0.5, 1.0][align.min(2) as usize];
    let base = at.y + cap * 0.5;
    let mut outline = Path::default();
    for (face, g, x) in placed {
        let k = scale_of(&face);
        face.outline(g, &mut outline, &|fx, fy| (left + x + fx * k, base - fy * k));
    }
    (outline, Rect::new(left, base - cap, left + width, base))
}

/// How set text is painted: fill only (0), outline only (1), fill over outline (2) or outline
/// over fill (3); the outline `width` wide (layer units) about the letters' edges.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Paint {
    pub display: u32,
    pub fill: Rgba,
    pub stroke: Rgba,
    pub width: f64,
    pub opacity: f32,
}

fn over(dst: Px, src: Px) -> Px {
    let k = 1.0 - src[3];
    [src[0] + dst[0] * k, src[1] + dst[1] * k, src[2] + dst[2] * k, src[3] + dst[3] * k]
}

/// A colour as a pixel, `k` of it.
fn tinted(c: Rgba, k: f32) -> Px {
    let a = (c.a as f32 * k).clamp(0.0, 1.0);
    [c.r as f32 * a, c.g as f32 * a, c.b as f32 * a, a]
}

/// `outline` (layer units) painted over `img`, whose pixels lie as `b`'s do.
pub(crate) fn paint(b: &Buf, img: &mut Image, outline: &Path, look: &Paint) {
    let (w, h) = (img.width, img.height);
    // the outline in the picture's pixels
    let mut p = Path::default();
    for c in outline.contours() {
        for (k, q) in c.iter().enumerate() {
            let r = b.pixel(vec2(q.0, q.1));
            if k == 0 {
                p.move_to(r.x, r.y);
            } else {
                p.line_to(r.x, r.y);
            }
        }
        p.close();
    }
    let fill = (look.display != 1).then(|| veymelo_path::fill(&p, Rule::NonZero, w, h)).flatten();
    let ring = (look.display != 0 && look.width > 0.0)
        .then(|| {
            let runs: Vec<Sub> = p.contours().iter().map(|c| Sub::polygon(&c.iter().map(|q| [q.0, q.1]).collect::<Vec<_>>(), true)).collect();
            veymelo_path::fill(&veymelo_path::stroke(&runs, &Stroke { width: look.width * b.scale, ..Stroke::default() }), Rule::NonZero, w, h)
        })
        .flatten();
    let order: [(Option<&veymelo_path::Mask>, Rgba); 2] = match look.display {
        3 => [(fill.as_ref(), look.fill), (ring.as_ref(), look.stroke)],
        _ => [(ring.as_ref(), look.stroke), (fill.as_ref(), look.fill)],
    };
    for (mask, c) in order {
        let Some(m) = mask else { continue };
        for y in m.top..(m.top + m.height).min(h) {
            for x in m.left..(m.left + m.width).min(w) {
                let k = m.at(x, y) * look.opacity;
                if k > 0.0 {
                    let d = &mut img.px[y * w + x];
                    *d = over(*d, tinted(c, k));
                }
            }
        }
    }
}

/// A box (layer units) filled over `img`, its edges softened to the pixel.
fn paint_box(b: &Buf, img: &mut Image, r: Rect, c: Rgba, opacity: f32) {
    let (p0, p1) = (b.pixel(vec2(r.x0, r.y0)), b.pixel(vec2(r.x1, r.y1)));
    let w = img.width;
    let (x0, x1) = ((p0.x.floor().max(0.0)) as usize, (p1.x.ceil().max(0.0) as usize).min(w));
    let (y0, y1) = ((p0.y.floor().max(0.0)) as usize, (p1.y.ceil().max(0.0) as usize).min(img.height));
    for y in y0..y1 {
        let cy = ((y as f64 + 1.0).min(p1.y) - (y as f64).max(p0.y)).clamp(0.0, 1.0);
        for x in x0..x1 {
            let cx = ((x as f64 + 1.0).min(p1.x) - (x as f64).max(p0.x)).clamp(0.0, 1.0);
            let d = &mut img.px[y * w + x];
            *d = over(*d, tinted(c, (cx * cy) as f32 * opacity));
        }
    }
}

const DISPLAY: &[&str] = &["fillOnly", "strokeOnly", "fillOverStroke", "strokeOverFill"];
const NUMBER_TYPES: &[&str] = &["number", "numberLeadingZeros", "timecode30", "timecode25", "timecode24", "time", "numericalDate", "shortDate", "longDate", "hexadecimal"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Saturday", "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];

effect!(
    NUMBERS_DEF,
    "numbers",
    "Numbers",
    "a number drawn on the layer: a value, a random one each frame, the time as timecode or a clock, a date, or hexadecimal",
    [
        choice("type", "Type", NUMBER_TYPES, 0),
        flag("randomValues", "Random Values", false).doc("a new value each frame, from 0 to the value"),
        ranged("value", "Value/Offset/Random Max", 0.0, -30_000.0, 30_000.0).doc("the number; for dates, days from 1 January 2000; with Current Time/Date, added to it (dates: days)"),
        ranged("decimalPlaces", "Decimal Places", 0.0, 0.0, 10.0),
        flag("currentTimeDate", "Current Time/Date", false).doc("times: the composition's time; dates: today's"),
        point("position", "Position"),
        choice("displayOptions", "Display Options", DISPLAY, 0),
        color("fillColor", "Fill Color", 1.0, 0.0, 0.0),
        color("strokeColor", "Stroke Color", 1.0, 1.0, 1.0),
        px("strokeWidth", "Stroke Width", 2.0, 50.0),
        px("size", "Size", 36.0, 512.0),
        ranged("tracking", "Tracking", 0.0, -100.0, 100.0).doc("more room between characters, in layer pixels"),
        flag("proportionalSpacing", "Proportional Spacing", true).doc("off: every digit as wide as the widest, so the number stands still as it changes"),
        flag("compositeOnOriginal", "Composite On Original", true).doc("off: the number alone"),
        font("font", "Font"),
        ranged("fontWeight", "Font Weight", 400.0, 100.0, 900.0),
        choice("alignment", "Alignment", &["left", "center", "right"], 1),
    ]
);
pub const NUMBERS: Spec = Spec::new(&NUMBERS_DEF, CATEGORY, numbers).points(&[("position", [0.5, 0.5])]);

/// The date `days` after 1 January 2000: year, month (1 to 12), day, day of the week (0:
/// Saturday).
pub fn date_of(days: i64) -> (i64, u32, u32, usize) {
    // the days counted from 1 March of year 0, in 400-year eras
    let z = days + 730_425;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, days.rem_euclid(7) as usize)
}

/// Today, as days after 1 January 2000 where this computer is.
fn today() -> i64 {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    (secs + local_offset(secs)).div_euclid(86_400) - 10_957
}

/// How far this computer's clock is ahead of UTC, in seconds, at `secs` after 1970.
#[cfg(unix)]
fn local_offset(secs: i64) -> i64 {
    // the C library's broken-down local time (its leading fields, the same on the unixes we run
    // on: nine ints, then the offset)
    #[repr(C)]
    struct Tm {
        fields: [i32; 9],
        gmtoff: std::ffi::c_long,
        zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    let mut tm = Tm { fields: [0; 9], gmtoff: 0, zone: std::ptr::null() };
    // SAFETY: both pointers are to live values of the types the C function takes.
    let ok = unsafe { !localtime_r(&secs, &mut tm).is_null() };
    if ok { tm.gmtoff as i64 } else { 0 }
}

#[cfg(not(unix))]
fn local_offset(_secs: i64) -> i64 {
    0
}

/// Frame `frame` as timecode at a whole rate `fps`.
fn timecode_at(frame: i64, fps: i64) -> String {
    FrameRate::new(fps.max(1) as u32, 1).unwrap_or_default().timecode(frame, false)
}

/// What Numbers shows.
pub(crate) fn numbers_text(ctx: &Ctx) -> String {
    let kind = ctx.choice("type");
    let places = ctx.num("decimalPlaces", 0.0).round().clamp(0.0, 10.0) as usize;
    let mut v = ctx.num("value", 0.0);
    if ctx.flag("randomValues") {
        let frame = (ctx.time.0 as f64 / ctx.frame.0.max(1) as f64).floor() as i64;
        v *= crate::noise::unit(crate::noise::hash(ctx.seed ^ (frame as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)));
    }
    let now = ctx.flag("currentTimeDate");
    let t = if now { ctx.comp_time.seconds() + v } else { v };
    match kind {
        1 => {
            let s = format!("{:.*}", places, v.abs());
            let (whole, part) = s.split_once('.').map_or((s.as_str(), String::new()), |(a, b)| (a, format!(".{b}")));
            format!("{}{whole:0>5}{part}", if v < 0.0 { "-" } else { "" })
        }
        2..=4 => {
            let fps = [30, 25, 24][(kind - 2) as usize];
            timecode_at((t * fps as f64 + 1e-6).floor() as i64, fps)
        }
        5 => {
            let s = (t + 1e-6).floor() as i64;
            let a = s.abs();
            format!("{}{:02}:{:02}:{:02}", if s < 0 { "-" } else { "" }, a / 3600, (a / 60) % 60, a % 60)
        }
        6..=8 => {
            let days = if now { today() + v.floor() as i64 } else { v.floor() as i64 };
            let (y, m, d, wd) = date_of(days);
            let month = MONTHS[m as usize - 1];
            match kind {
                6 => format!("{m:02}/{d:02}/{:02}", y.rem_euclid(100)),
                7 => format!("{} {d}, {y}", &month[..3]),
                _ => format!("{}, {month} {d}, {y}", DAYS[wd]),
            }
        }
        9 => {
            let i = v.round() as i64;
            if i < 0 { format!("-{:X}", -i) } else { format!("{i:X}") }
        }
        _ => format!("{:.*}", places, t),
    }
}

fn numbers(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(face) = font_for(&value_text(ctx, "font"), ctx.num("fontWeight", 400.0)) else {
        ctx.note("Numbers found no font to draw with".into());
        return b;
    };
    let size = ctx.num("size", 36.0).max(0.0);
    let (outline, _) = set_line(&numbers_text(ctx), &face, size, ctx.num("tracking", 0.0), !ctx.flag("proportionalSpacing"), ctx.choice("alignment"), ctx.point("position"));
    if !ctx.flag("compositeOnOriginal") {
        b.img.px.iter_mut().for_each(|p| *p = [0.0; 4]);
    }
    let look = Paint { display: ctx.choice("displayOptions"), fill: ctx.color("fillColor"), stroke: ctx.color("strokeColor"), width: ctx.num("strokeWidth", 2.0), opacity: 1.0 };
    let mut img = std::mem::replace(&mut b.img, Image::new(0, 0));
    paint(&b, &mut img, &outline, &look);
    b.img = img;
    b
}

/// A text setting's words.
fn value_text(ctx: &Ctx, path: &str) -> String {
    ctx.value_in(ctx.effect, path).map(|v| v.as_text().to_string()).unwrap_or_default()
}

effect!(
    TIMECODE_DEF,
    "timecode",
    "Timecode",
    "the time drawn on the layer as timecode, frames or feet and frames, in a box",
    [
        choice("displayFormat", "Display Format", &["smpte", "frames", "feetFrames35", "feetFrames16"], 0),
        choice("timeSource", "Time Source", &["layerSource", "composition", "custom"], 0).doc("the layer's own time, the composition's (from its start), or the layer's counted at Time Units from Starting Frame"),
        ranged("timeUnits", "Time Units", 30.0, 1.0, 120.0).doc("frames a second, for Custom"),
        flag("dropFrame", "Drop Frame", false).doc("at 29.97 or 59.94 frames a second"),
        ranged("startingFrame", "Starting Frame", 0.0, -1_000_000.0, 1_000_000.0).doc("for Custom"),
        point("textPosition", "Text Position"),
        px("textSize", "Text Size", 36.0, 512.0),
        color("textColor", "Text Color", 1.0, 1.0, 1.0),
        flag("showBox", "Show Box", true),
        color("boxColor", "Box Color", 0.0, 0.0, 0.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        flag("compositeOnOriginal", "Composite On Original", true).doc("off: the timecode alone"),
        font("font", "Font"),
    ]
);
pub const TIMECODE: Spec = Spec::new(&TIMECODE_DEF, CATEGORY, timecode).points(&[("textPosition", [0.5, 0.9])]);

/// What Timecode shows.
pub(crate) fn timecode_text(ctx: &Ctx) -> String {
    let source = ctx.choice("timeSource");
    let units = ctx.num("timeUnits", 30.0).clamp(1.0, 120.0);
    let rate = if source == 2 { FrameRate::from_f64(units).or_else(|| FrameRate::new(units.round() as u32, 1)).unwrap_or_default() } else { ctx.rate };
    let t = if source == 1 { ctx.comp_time } else { ctx.time };
    let mut frame = rate.frame_at(t + Tick::from_seconds(1e-6));
    if source == 2 {
        frame += ctx.num("startingFrame", 0.0).round() as i64;
    }
    let feet = |per: i64| format!("{}{}+{:02}", if frame < 0 { "-" } else { "" }, frame.abs() / per, frame.abs() % per);
    match ctx.choice("displayFormat") {
        1 => frame.to_string(),
        2 => feet(16),
        3 => feet(40),
        _ => rate.timecode(frame, ctx.flag("dropFrame")),
    }
}

fn timecode(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(face) = font_for(&value_text(ctx, "font"), 400.0) else {
        ctx.note("Timecode found no font to draw with".into());
        return b;
    };
    let size = ctx.num("textSize", 36.0).max(0.0);
    let (outline, area) = set_line(&timecode_text(ctx), &face, size, 0.0, true, 1, ctx.point("textPosition"));
    let opacity = (ctx.num("opacity", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    if !ctx.flag("compositeOnOriginal") {
        b.img.px.iter_mut().for_each(|p| *p = [0.0; 4]);
    }
    let mut img = std::mem::replace(&mut b.img, Image::new(0, 0));
    if ctx.flag("showBox") {
        let pad = size * 0.2;
        paint_box(&b, &mut img, Rect::new(area.x0 - pad, area.y0 - pad, area.x1 + pad, area.y1 + pad), ctx.color("boxColor"), opacity);
    }
    let look = Paint { display: 0, fill: ctx.color("textColor"), stroke: Rgba::WHITE, width: 0.0, opacity };
    paint(&b, &mut img, &outline, &look);
    b.img = img;
    b
}

/// Every effect here.
pub const ALL: [Spec; 2] = [NUMBERS, TIMECODE];

#[cfg(test)]
mod tests;
