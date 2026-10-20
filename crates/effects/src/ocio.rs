//! Colour management: colours moved between colour spaces (OCIO Color Space Transform, Color
//! Profile Converter), shown on a display through a view (OCIO Display Transform), changed by a
//! colour correction (OCIO CDL Transform), a file of one (OCIO File Transform) or a
//! configuration's looks (OCIO Look Transform); and colours held to a frame's (Color
//! Stabilizer).
//!
//! The OCIO effects read an OpenColorIO configuration file, or with none named our own built
//! from the colour spaces we know (the camera logs, ACES, the display standards).

pub mod config;
mod yaml;

use veymelo_color::space::{self, Curve, Space};
use veymelo_keyframe::{Kind, Value};
use veymelo_project::PropDef;
use veymelo_time::Tick;

use crate::colorfile::{self, Cdl, Step, Transform};
use crate::defs::{choice, effect, flag, point, px, ranged};
use crate::lut::Between;
use crate::util::{luma, map_straight, unpremul};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "color";

/// A words setting (empty by default).
const fn words(id: &'static str, name: &'static str, doc: &'static str) -> PropDef {
    PropDef::new(id, name, Kind::Text, Value::Text(String::new())).doc(doc)
}

fn text(ctx: &Ctx, id: &str) -> String {
    ctx.value_in(ctx.effect, id).map(|v| v.as_text().trim().to_string()).unwrap_or_default()
}

const DIRECTION: &[&str] = &["forward", "inverse"];

/// The pixels through `t` (backwards if the direction says), colours as they are (not
/// premultiplied); what the transform could not do told to whoever renders.
fn run(ctx: &Ctx, mut b: Buf, t: &Transform, between: Between, what: &str) -> Buf {
    if !t.skipped.is_empty() {
        ctx.note(format!("{what}: passed over {}", t.skipped.join(", ")));
    }
    let back = ctx.choice("direction") == 1;
    map_straight(&mut b.img, |c| t.run(c, between, back));
    b
}

/// The configuration the effect names (our own when it names none).
fn config(ctx: &Ctx) -> Option<std::sync::Arc<config::Config>> {
    match config::load(&text(ctx, "config")) {
        Ok(c) => Some(c),
        Err(e) => {
            ctx.note(e);
            None
        }
    }
}

/// A space setting's name, or the first of `roles` the configuration has, or its first space.
fn space_name(cfg: &config::Config, given: &str, roles: &[&str]) -> String {
    if !given.is_empty() {
        return given.to_string();
    }
    roles.iter().find_map(|r| cfg.space(r).map(|s| s.name.clone())).unwrap_or_else(|| cfg.spaces[0].name.clone())
}

const CONFIG_DOC: &str = "an OpenColorIO configuration file (.ocio); empty: our own, from the colour spaces we know";

effect!(
    CDL_DEF,
    "ocioCdlTransform",
    "OCIO CDL Transform",
    "an ASC colour decision: slope, offset and power for each channel, then saturation",
    [
        ranged("slopeRed", "Slope Red", 1.0, 0.0, 100.0),
        ranged("slopeGreen", "Slope Green", 1.0, 0.0, 100.0),
        ranged("slopeBlue", "Slope Blue", 1.0, 0.0, 100.0),
        ranged("offsetRed", "Offset Red", 0.0, -100.0, 100.0),
        ranged("offsetGreen", "Offset Green", 0.0, -100.0, 100.0),
        ranged("offsetBlue", "Offset Blue", 0.0, -100.0, 100.0),
        ranged("powerRed", "Power Red", 1.0, 0.0, 100.0),
        ranged("powerGreen", "Power Green", 1.0, 0.0, 100.0),
        ranged("powerBlue", "Power Blue", 1.0, 0.0, 100.0),
        ranged("saturation", "Saturation", 1.0, 0.0, 100.0),
        choice("style", "Style", &["ascCdl12", "noClamp"], 0).doc("ASC CDL 1.2 holds colours from 0 to 1; no clamp lets them past"),
        choice("direction", "Direction", DIRECTION, 0),
    ]
);
pub const CDL: Spec = Spec::new(&CDL_DEF, CATEGORY, ocio_cdl);

fn ocio_cdl(ctx: &Ctx, b: Buf) -> Buf {
    let rgb = |n: &str, d: f64| ["Red", "Green", "Blue"].map(|c| ctx.num(&format!("{n}{c}"), d));
    let cdl = Cdl { slope: rgb("slope", 1.0), offset: rgb("offset", 0.0), power: rgb("power", 1.0), sat: ctx.num("saturation", 1.0) };
    let t = Transform::of(vec![Step::Cdl { cdl, clamp: ctx.choice("style") == 0 }]);
    run(ctx, b, &t, Between::Tetrahedral, "OCIO CDL Transform")
}

effect!(
    CST_DEF,
    "ocioColorSpaceTransform",
    "OCIO Color Space Transform",
    "colours moved from one colour space to another (camera logs, ACES, display standards…)",
    [
        words("config", "Configuration", CONFIG_DOC),
        words("source", "Source", "the colour space the colours are in, by name; empty: the scene-linear one (ACEScg in ours)"),
        words("destination", "Destination", "the colour space to put them in; empty: sRGB"),
        choice("direction", "Direction", DIRECTION, 0),
    ]
);
pub const CST: Spec = Spec::new(&CST_DEF, CATEGORY, ocio_cst);

fn ocio_cst(ctx: &Ctx, b: Buf) -> Buf {
    let Some(cfg) = config(ctx) else { return b };
    let src = space_name(&cfg, &text(ctx, "source"), &["scene_linear", "reference"]);
    let dst = space_name(&cfg, &text(ctx, "destination"), &["sRGB - Texture", "sRGB - Display", "color_picking", "matte_paint", "texture_paint"]);
    match cfg.convert(&src, &dst) {
        Ok(t) => run(ctx, b, &t, Between::Tetrahedral, "OCIO Color Space Transform"),
        Err(e) => {
            ctx.note(e);
            b
        }
    }
}

effect!(
    DISPLAY_DEF,
    "ocioDisplayTransform",
    "OCIO Display Transform",
    "colours made ready for a display through one of its views (plain, tone-mapped, or the configuration's)",
    [
        words("config", "Configuration", CONFIG_DOC),
        words("source", "Input Color Space", "the colour space the colours are in; empty: the scene-linear one"),
        words("display", "Display", "the display, by name (in ours: sRGB, Rec.1886 Rec.709, Display P3, Rec.2100-PQ…); empty: the first"),
        words("view", "View", "the view, by name (in ours: Standard, Tone Mapped, Raw); empty: the display's first"),
        flag("bypassLooks", "Bypass Looks", false),
        choice("direction", "Direction", DIRECTION, 0),
    ]
);
pub const DISPLAY: Spec = Spec::new(&DISPLAY_DEF, CATEGORY, ocio_display);

fn ocio_display(ctx: &Ctx, b: Buf) -> Buf {
    let Some(cfg) = config(ctx) else { return b };
    let src = space_name(&cfg, &text(ctx, "source"), &["scene_linear", "reference"]);
    match cfg.display(&src, &text(ctx, "display"), &text(ctx, "view"), ctx.flag("bypassLooks")) {
        Ok(t) => run(ctx, b, &t, Between::Tetrahedral, "OCIO Display Transform"),
        Err(e) => {
            ctx.note(e);
            b
        }
    }
}

effect!(
    FILE_DEF,
    "ocioFileTransform",
    "OCIO File Transform",
    "colours through a file: a LUT (.cube .3dl .csp .spi1d .spi3d), a matrix (.spimtx), a CDL (.cc .ccc .cdl) or a Common LUT Format list (.clf)",
    [
        words("file", "File", "the file (a full path)"),
        words("cccId", "CCC ID", "which correction of a .ccc or .cdl file; empty: the first"),
        choice("interpolation", "Interpolation", &["nearest", "linear", "tetrahedral", "best"], 3),
        choice("direction", "Direction", DIRECTION, 0),
    ]
);
pub const FILE: Spec = Spec::new(&FILE_DEF, CATEGORY, ocio_file);

fn ocio_file(ctx: &Ctx, b: Buf) -> Buf {
    let path = text(ctx, "file");
    if path.is_empty() {
        return b;
    }
    let between = match ctx.choice("interpolation") {
        0 => Between::Nearest,
        1 => Between::Trilinear,
        _ => Between::Tetrahedral,
    };
    match colorfile::load(&path, &text(ctx, "cccId")) {
        Ok(t) => run(ctx, b, &t, between, "OCIO File Transform"),
        Err(e) => {
            ctx.note(e);
            b
        }
    }
}

effect!(
    LOOK_DEF,
    "ocioLookTransform",
    "OCIO Look Transform",
    "a configuration's looks done in their own colour spaces, from one colour space to another",
    [
        words("config", "Configuration", "an OpenColorIO configuration file (.ocio) with looks"),
        words("source", "Source", "the colour space the colours are in; empty: the scene-linear one"),
        words("destination", "Destination", "the colour space to leave them in; empty: the source"),
        words("looks", "Looks", "the looks by name, \"+A, -B\" (- undoes one; A | B: the first the configuration has)"),
        choice("direction", "Direction", DIRECTION, 0),
    ]
);
pub const LOOK: Spec = Spec::new(&LOOK_DEF, CATEGORY, ocio_look);

fn ocio_look(ctx: &Ctx, b: Buf) -> Buf {
    let Some(cfg) = config(ctx) else { return b };
    if cfg.looks.is_empty() {
        ctx.note("OCIO Look Transform: the configuration has no looks".into());
        return b;
    }
    let src = space_name(&cfg, &text(ctx, "source"), &["scene_linear", "reference"]);
    let dst = { let d = text(ctx, "destination"); if d.is_empty() { src.clone() } else { d } };
    match cfg.looks(&src, &dst, &text(ctx, "looks")) {
        Ok(t) => run(ctx, b, &t, Between::Tetrahedral, "OCIO Look Transform"),
        Err(e) => {
            ctx.note(e);
            b
        }
    }
}

const PROFILES: &[&str] = &["projectWorkingSpace", "srgb", "rec709", "rec2020", "displayP3", "acesCg", "aces2065", "adobeRgb", "proPhotoRgb", "dciP3"];

/// A profile choice as a colour space (the project's working space is sRGB).
fn profile(i: u32) -> Space {
    let s = |n: &str| *space::find(n).expect("one of ours");
    match i {
        2 => Space { name: "HDTV (Rec. 709)", curve: Curve::Rec709, ..s("Linear Rec.709 (sRGB)") },
        3 => Space { name: "Rec. 2020", curve: Curve::Rec709, ..s("Linear Rec.2020") },
        4 => s("Display P3 - Display"),
        5 => s("ACEScg"),
        6 => s("ACES2065-1"),
        7 => s("Adobe RGB (1998)"),
        8 => s("ProPhoto RGB"),
        9 => s("P3-DCI - Display"),
        _ => s("sRGB - Texture"),
    }
}

effect!(
    PROFILE_DEF,
    "colorProfileConverter",
    "Color Profile Converter",
    "colours moved from one colour profile to another (sRGB, Rec. 709, Rec. 2020, Display P3, ACES, Adobe RGB, ProPhoto, DCI-P3)",
    [
        choice("inputProfile", "Input Profile", PROFILES, 0),
        flag("linearizeInputProfile", "Linearize Input Profile", false).doc("the colours are its light, not written with its curve"),
        choice("outputProfile", "Output Profile", PROFILES, 1),
        flag("linearizeOutputProfile", "Linearize Output Profile", false),
        choice("intent", "Intent", &["perceptual", "relativeColorimetric", "saturation", "absoluteColorimetric"], 1)
            .doc("perceptual: the input's whole gamut drawn softly into the output's; relative: whites matched; saturation: clipped; absolute: whites as measured"),
        flag("useBlackPointCompensation", "Use Black Point Compensation", true).doc("(every profile here has the same black: no change)"),
    ]
);
pub const PROFILE: Spec = Spec::new(&PROFILE_DEF, "utility", profile_converter);

fn profile_converter(ctx: &Ctx, mut b: Buf) -> Buf {
    let (mut a, mut z) = (profile(ctx.choice("inputProfile")), profile(ctx.choice("outputProfile")));
    if ctx.flag("linearizeInputProfile") {
        a.curve = Curve::Linear;
    }
    if ctx.flag("linearizeOutputProfile") {
        z.curve = Curve::Linear;
    }
    let intent = ctx.choice("intent");
    let m = if intent == 3 { space::mul(&space::invert(&z.to_xyz(false)), &a.to_xyz(false)) } else { space::matrix(&a, &z) };
    if a.curve == z.curve && (0..3).all(|i| (0..3).all(|j| (m[i][j] - space::IDENTITY[i][j]).abs() < 1e-12)) {
        return b;
    }
    let limit = reach(&m);
    map_straight(&mut b.img, |c| {
        let lin = c.map(|v| a.curve.decode(v as f64));
        let mut o = space::apply(&m, lin);
        match intent {
            0 => o = space::compress_gamut_with(o, limit, [0.815, 0.803, 0.880].map(|t: f64| t.min(1.0)), 1.2, false),
            2 => o = o.map(|v| v.max(0.0)),
            _ => {}
        }
        o.map(|v| z.curve.encode(v) as f32)
    });
    b
}

/// How far past the destination's edge (as the gamut compression measures it, per channel)
/// the source's primaries and secondaries land through `m`: the limits that put the source's
/// edge on the destination's.
fn reach(m: &space::M3) -> [f64; 3] {
    let mut lim = [1.0f64; 3];
    for c in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 0.0]] {
        let o = space::apply(m, c);
        let ach = o[0].max(o[1]).max(o[2]);
        if ach > 0.0 {
            for i in 0..3 {
                lim[i] = lim[i].max((ach - o[i]) / ach);
            }
        }
    }
    // a source inside the destination needs none (a limit past the threshold that is never met)
    lim.map(|l| if l <= 1.0 + 1e-9 { 1e9 } else { l })
}

effect!(
    STABILIZER_DEF,
    "colorStabilizer",
    "Color Stabilizer",
    "the layer's colours held to one frame's, measured at a few points (for flicker and drifting exposure)",
    [
        ranged("referenceFrame", "Reference Frame", 0.0, -100_000.0, 100_000.0).doc("the layer time (seconds) whose colours the others are held to"),
        choice("stabilize", "Stabilize", &["brightness", "levels", "curves"], 0).doc("brightness: the mid point's; levels: the black and white points'; curves: all three"),
        point("blackPoint", "Black Point"),
        point("midPoint", "Mid Point"),
        point("whitePoint", "White Point"),
        px("sampleSize", "Sample Size", 5.0, 100.0).doc("how far round each point is measured"),
    ]
);
pub const STABILIZER: Spec = Spec::new(&STABILIZER_DEF, CATEGORY, color_stabilizer).points(&[("blackPoint", [0.25, 0.75]), ("midPoint", [0.5, 0.5]), ("whitePoint", [0.75, 0.25])]);

/// The mean colour (as it is) in the square `r` layer units round `p`.
fn sample(b: &Buf, p: veymelo_geom::Vec2, r: f64) -> [f64; 3] {
    let (a, z) = (b.pixel(p - veymelo_geom::vec2(r, r)), b.pixel(p + veymelo_geom::vec2(r, r)));
    let (x0, x1) = (a.x.floor().max(0.0) as usize, (z.x.ceil().max(0.0) as usize).min(b.img.width));
    let (y0, y1) = (a.y.floor().max(0.0) as usize, (z.y.ceil().max(0.0) as usize).min(b.img.height));
    let (mut s, mut n) = ([0f64; 3], 0f64);
    for y in y0..y1.max(y0) {
        for x in x0..x1.max(x0) {
            let (c, al) = unpremul(b.img.px[y * b.img.width + x]);
            if al > 0.0 {
                (0..3).for_each(|k| s[k] += c[k] as f64);
                n += 1.0;
            }
        }
    }
    if n > 0.0 { s.map(|v| v / n) } else { [0.0; 3] }
}

/// A smooth rising curve through points (x rising), straight past its ends (Fritsch and
/// Carlson's monotone cubic).
fn through(pts: &[(f64, f64)], x: f64) -> f64 {
    let n = pts.len();
    if n == 0 {
        return x;
    }
    if n == 1 {
        return x - pts[0].0 + pts[0].1;
    }
    let slope = |i: usize| (pts[i + 1].1 - pts[i].1) / (pts[i + 1].0 - pts[i].0).max(1e-9);
    let d: Vec<f64> = (0..n - 1).map(slope).collect();
    let mut m: Vec<f64> = (0..n).map(|i| if i == 0 { d[0] } else if i == n - 1 { d[n - 2] } else if d[i - 1] * d[i] <= 0.0 { 0.0 } else { (d[i - 1] + d[i]) * 0.5 }).collect();
    for i in 0..n - 1 {
        if d[i].abs() < 1e-12 {
            (m[i], m[i + 1]) = (0.0, 0.0);
        } else {
            let (a, b) = (m[i] / d[i], m[i + 1] / d[i]);
            let h = a.hypot(b);
            if h > 3.0 {
                (m[i], m[i + 1]) = (3.0 * a / h * d[i], 3.0 * b / h * d[i]);
            }
        }
    }
    if x <= pts[0].0 {
        return pts[0].1 + (x - pts[0].0) * m[0];
    }
    if x >= pts[n - 1].0 {
        return pts[n - 1].1 + (x - pts[n - 1].0) * m[n - 1];
    }
    let i = pts.partition_point(|p| p.0 <= x).clamp(1, n - 1) - 1;
    let (p0, p1) = (pts[i], pts[i + 1]);
    let h = (p1.0 - p0.0).max(1e-12);
    let t = (x - p0.0) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * p0.1 + (t3 - 2.0 * t2 + t) * h * m[i] + (-2.0 * t3 + 3.0 * t2) * p1.1 + (t3 - t2) * h * m[i + 1]
}

fn color_stabilizer(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(host) = ctx.host else { return b };
    let Some(reference) = host.this_at(Tick::from_seconds(ctx.num("referenceFrame", 0.0)), ctx.index) else { return b };
    let r = ctx.num("sampleSize", 5.0).max(0.5);
    let pts = [ctx.point("blackPoint"), ctx.point("midPoint"), ctx.point("whitePoint")];
    let now = pts.map(|p| sample(&b, p, r));
    let then = pts.map(|p| sample(&reference, p, r));
    match ctx.choice("stabilize") {
        1 => map_straight(&mut b.img, |c| {
            [0, 1, 2].map(|k| {
                let (x0, x1, y0, y1) = (now[0][k], now[2][k], then[0][k], then[2][k]);
                let span = x1 - x0;
                (if span.abs() < 1e-6 { c[k] as f64 - x0 + y0 } else { y0 + (c[k] as f64 - x0) * (y1 - y0) / span }) as f32
            })
        }),
        2 => {
            let curves = [0, 1, 2].map(|k| {
                let mut p: Vec<(f64, f64)> = (0..3).map(|i| (now[i][k], then[i][k])).collect();
                p.sort_by(|a, b| a.0.total_cmp(&b.0));
                p.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
                p
            });
            map_straight(&mut b.img, |c| [0, 1, 2].map(|k| through(&curves[k], c[k] as f64) as f32));
        }
        _ => {
            let (l0, l1) = (luma(now[1].map(|v| v as f32)), luma(then[1].map(|v| v as f32)));
            let k = if l0 > 1e-6 { l1 / l0 } else { 1.0 };
            map_straight(&mut b.img, |c| c.map(|v| v * k));
        }
    }
    b
}

/// Every effect here.
pub const ALL: [Spec; 7] = [CDL, CST, DISPLAY, FILE, LOOK, PROFILE, STABILIZER];

#[cfg(test)]
mod tests;
