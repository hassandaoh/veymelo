//! A colour grade in one effect (basic correction, a creative look, curves, colour wheels, a
//! secondary correction keyed on hue, saturation and lightness, a vignette), and colour lookup
//! tables from files.

use veymelo_keyframe::{Kind, Value};
use veymelo_project::{PropDef, Unit};
use veymelo_raster::filter::{self, Edge};
use veymelo_raster::Image;

use crate::color::rgb3;
use crate::defs::{angle, choice, color, effect, flag, pct, ranged};
use crate::grade::Curve;
use crate::lut::{self, Between};
use crate::util::{hsl, luma, map_px, premul, rgb, unpremul};
use crate::{Buf, Ctx, Spec};

const fn text(id: &'static str, name: &'static str, doc: &'static str) -> PropDef {
    PropDef::new(id, name, Kind::Text, Value::Text(String::new())).doc(doc)
}

const LOOKS: &[&str] = &["none", "custom", "warmFilm", "coolNight", "bleachBypass", "tealOrange", "fadedMatte", "monochrome"];
const CURVE_DOC: &str = "points \"x,y x,y …\" from 0 to 1";
const HS_DOC: &str = "points \"x,y …\" from 0 to 1; y 0.5 changes nothing";

effect!(
    COLOR_GRADE_DEF,
    "colorGrade",
    "Color Grade",
    "a whole grade: white balance and tone, a creative look, curves, colour wheels, a correction of one keyed range of colour, and a vignette",
    [
        flag("highDynamicRange", "High Dynamic Range", false).doc("the tone controls work up to HDR White"),
        choice("inputLut", "Input LUT", &["none", "custom"], 0),
        text("inputLutFile", "Input LUT File", "a .cube file, applied first (a full path)"),
        pct("temperature", "Temperature", 0.0, -100.0, 100.0).doc("warmer (orange) or cooler (blue)"),
        pct("tint", "Tint", 0.0, -100.0, 100.0).doc("toward magenta or green"),
        ranged("exposure", "Exposure", 0.0, -5.0, 5.0).doc("stops"),
        pct("contrast", "Contrast", 0.0, -100.0, 100.0),
        pct("highlights", "Highlights", 0.0, -100.0, 100.0),
        pct("shadows", "Shadows", 0.0, -100.0, 100.0),
        pct("whites", "Whites", 0.0, -100.0, 100.0),
        pct("blacks", "Blacks", 0.0, -100.0, 100.0),
        ranged("hdrWhite", "HDR White", 100.0, 100.0, 10000.0),
        pct("hdrSpecular", "HDR Specular", 0.0, -100.0, 100.0),
        pct("saturation", "Saturation", 100.0, 0.0, 200.0),
        choice("look", "Look", LOOKS, 0),
        text("lookFile", "Look File", "a .cube file, with look custom (a full path)"),
        pct("lookIntensity", "Intensity", 100.0, 0.0, 200.0).doc("how much of the look and the creative adjustments"),
        pct("fadedFilm", "Faded Film", 0.0, 0.0, 100.0),
        pct("sharpen", "Sharpen", 0.0, -100.0, 100.0),
        pct("vibrance", "Vibrance", 0.0, -100.0, 100.0),
        pct("creativeSaturation", "Creative Saturation", 100.0, 0.0, 200.0),
        color("shadowTint", "Shadow Tint", 0.5, 0.5, 0.5).doc("grey changes nothing; a colour tints the shadows"),
        color("highlightTint", "Highlight Tint", 0.5, 0.5, 0.5),
        pct("tintBalance", "Tint Balance", 0.0, -100.0, 100.0),
        pct("curveShadows", "RGB Curve Shadows", 0.0, -100.0, 100.0),
        pct("curveMidtones", "RGB Curve Midtones", 0.0, -100.0, 100.0),
        pct("curveHighlights", "RGB Curve Highlights", 0.0, -100.0, 100.0),
        pct("redShadows", "Red Curve Shadows", 0.0, -100.0, 100.0),
        pct("redMidtones", "Red Curve Midtones", 0.0, -100.0, 100.0),
        pct("redHighlights", "Red Curve Highlights", 0.0, -100.0, 100.0),
        pct("greenShadows", "Green Curve Shadows", 0.0, -100.0, 100.0),
        pct("greenMidtones", "Green Curve Midtones", 0.0, -100.0, 100.0),
        pct("greenHighlights", "Green Curve Highlights", 0.0, -100.0, 100.0),
        pct("blueShadows", "Blue Curve Shadows", 0.0, -100.0, 100.0),
        pct("blueMidtones", "Blue Curve Midtones", 0.0, -100.0, 100.0),
        pct("blueHighlights", "Blue Curve Highlights", 0.0, -100.0, 100.0),
        text("rgbCurve", "RGB Curve", CURVE_DOC),
        text("redCurve", "Red Curve", CURVE_DOC),
        text("greenCurve", "Green Curve", CURVE_DOC),
        text("blueCurve", "Blue Curve", CURVE_DOC),
        text("hueVsSat", "Hue vs Sat", HS_DOC),
        text("hueVsHue", "Hue vs Hue", HS_DOC),
        text("hueVsLuma", "Hue vs Luma", HS_DOC),
        text("lumaVsSat", "Luma vs Sat", HS_DOC),
        text("satVsSat", "Sat vs Sat", HS_DOC),
        color("shadowsWheel", "Shadows Wheel", 0.5, 0.5, 0.5),
        color("midtonesWheel", "Midtones Wheel", 0.5, 0.5, 0.5),
        color("highlightsWheel", "Highlights Wheel", 0.5, 0.5, 0.5),
        flag("keyHue", "Key Hue", true).doc("the secondary correction's key looks at hue"),
        angle("hueCenter", "Hue Center", 0.0),
        ranged("hueRange", "Hue Range", 180.0, 0.0, 180.0).unit(Unit::Degrees).doc("180 takes every hue"),
        ranged("hueSoftness", "Hue Softness", 0.0, 0.0, 180.0).unit(Unit::Degrees),
        flag("keySaturation", "Key Saturation", true),
        pct("saturationLow", "Saturation Low", 0.0, 0.0, 100.0),
        pct("saturationHigh", "Saturation High", 100.0, 0.0, 100.0),
        pct("saturationSoftness", "Saturation Softness", 0.0, 0.0, 100.0),
        flag("keyLightness", "Key Lightness", true),
        pct("lightnessLow", "Lightness Low", 0.0, 0.0, 100.0),
        pct("lightnessHigh", "Lightness High", 100.0, 0.0, 100.0),
        pct("lightnessSoftness", "Lightness Softness", 0.0, 0.0, 100.0),
        flag("showMask", "Show Mask", false),
        choice("maskStyle", "Mask Style", &["colorGray", "colorBlack", "whiteBlack"], 0),
        flag("invertMask", "Invert Mask", false),
        pct("denoise", "Denoise", 0.0, 0.0, 100.0),
        pct("blur", "Blur", 0.0, 0.0, 100.0),
        color("correction", "Correction", 0.5, 0.5, 0.5).doc("the secondary correction's colour wheel"),
        pct("correctionTemperature", "Correction Temperature", 0.0, -100.0, 100.0),
        pct("correctionTint", "Correction Tint", 0.0, -100.0, 100.0),
        pct("correctionContrast", "Correction Contrast", 0.0, -100.0, 100.0),
        pct("correctionSharpen", "Correction Sharpen", 0.0, -100.0, 100.0),
        pct("correctionSaturation", "Correction Saturation", 100.0, 0.0, 200.0),
        ranged("vignetteAmount", "Vignette Amount", 0.0, -5.0, 5.0).doc("below 0 darkens the edges, above lightens them"),
        pct("vignetteMidpoint", "Vignette Midpoint", 50.0, 0.0, 100.0),
        pct("vignetteRoundness", "Vignette Roundness", 0.0, -100.0, 100.0),
        pct("vignetteFeather", "Vignette Feather", 50.0, 0.0, 100.0),
    ]
);
pub const COLOR_GRADE: Spec = Spec::new(&COLOR_GRADE_DEF, "color", color_grade);

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A curve moving the shadows, midtones and highlights (-1 to 1 each), black and white held.
fn tones(x: f32, s: f32, m: f32, h: f32) -> f32 {
    if s == 0.0 && m == 0.0 && h == 0.0 {
        return x;
    }
    let t = x.clamp(0.0, 1.0);
    let bump = |c: f32| (1.0 - ((t - c) / 0.25).powi(2)).max(0.0).powi(2);
    x + 0.25 * (s * bump(0.25) + m * bump(0.5) + h * bump(0.75))
}

/// A colour's difference from its own grey (none for a grey).
fn tint_of(c: veymelo_color::Rgba) -> [f32; 3] {
    let c = rgb3(c);
    let l = (c[0] + c[1] + c[2]) / 3.0;
    c.map(|v| v - l)
}

/// A curve of offsets from neutral (y 0.5), smooth through its points: round the wheel for
/// hue curves, level past its ends for the others.
struct Offsets(Vec<f32>);

impl Offsets {
    const N: usize = 512;

    fn parse(s: &str, round: bool) -> Option<Offsets> {
        let mut pts: Vec<(f32, f32)> = s
            .split(|c: char| c.is_whitespace() || c == ';')
            .filter_map(|t| {
                let (x, y) = t.split_once(',')?;
                Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?))
            })
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
        if pts.is_empty() || pts.iter().all(|p| (p.1 - 0.5).abs() < 1e-6) {
            return None;
        }
        if pts.len() == 1 {
            return Some(Offsets(vec![pts[0].1 - 0.5; Self::N + 1]));
        }
        // the points, and copies either side so the ends have neighbours
        let mut all: Vec<(f32, f32)> = Vec::new();
        if round {
            for k in -1..=1 {
                all.extend(pts.iter().map(|&(x, y)| (x.rem_euclid(1.0) + k as f32, y)));
            }
            all.sort_by(|a, b| a.0.total_cmp(&b.0));
        } else {
            let ((x0, y0), (x1, y1)) = (pts[0], pts[pts.len() - 1]);
            all.extend([(x0.min(0.0) - 2.0, y0), (x0.min(0.0) - 1.0, y0)]);
            all.extend(pts.iter().copied());
            all.extend([(x1.max(1.0) + 1.0, y1), (x1.max(1.0) + 2.0, y1)]);
        }
        let n = all.len();
        let slope = |k: usize| (all[k + 1].1 - all[k].1) / (all[k + 1].0 - all[k].0).max(1e-6);
        let m: Vec<f32> = (0..n)
            .map(|k| {
                if k == 0 || k == n - 1 {
                    return 0.0;
                }
                let (a, b) = (slope(k - 1), slope(k));
                if a * b <= 0.0 { 0.0 } else { ((a + b) * 0.5).clamp(-3.0 * a.abs().min(b.abs()), 3.0 * a.abs().min(b.abs())) }
            })
            .collect();
        let at = |x: f32| {
            let k = all.partition_point(|p| p.0 <= x).saturating_sub(1).min(n - 2);
            let ((xa, ya), (xb, yb)) = (all[k], all[k + 1]);
            let h = (xb - xa).max(1e-6);
            let t = ((x - xa) / h).clamp(0.0, 1.0);
            let (t2, t3) = (t * t, t * t * t);
            (2.0 * t3 - 3.0 * t2 + 1.0) * ya + (t3 - 2.0 * t2 + t) * h * m[k] + (-2.0 * t3 + 3.0 * t2) * yb + (t3 - t2) * h * m[k + 1]
        };
        Some(Offsets((0..=Self::N).map(|i| at(i as f32 / Self::N as f32) - 0.5).collect()))
    }

    fn at(&self, x: f32) -> f32 {
        let f = x.clamp(0.0, 1.0) * Self::N as f32;
        let i = (f as usize).min(Self::N - 1);
        self.0[i] + (self.0[i + 1] - self.0[i]) * (f - i as f32)
    }
}

/// The looks made here (custom reads a file).
fn made_look(look: u32, c: [f32; 3]) -> [f32; 3] {
    let l = luma(c);
    match look {
        2 => [c[0] * 1.06 + 0.015, c[1] * 1.01 + 0.01, c[2] * 0.88],
        3 => [c[0] * 0.82, c[1] * 0.92, c[2] * 1.08 + 0.02].map(|v| v * 0.92),
        4 => {
            // silver kept: half the colour, the brightness laid over itself for contrast
            let over = |v: f32| if l < 0.5 { 2.0 * v * l } else { 1.0 - 2.0 * (1.0 - v) * (1.0 - l) };
            c.map(|v| l + (v - l) * 0.45).map(|v| v * 0.5 + over(v) * 0.5)
        }
        5 => {
            let t = (l - 0.5).clamp(-0.5, 0.5);
            [c[0] + 0.18 * t, c[1] + 0.03 * t, c[2] - 0.16 * t]
        }
        6 => c.map(|v| 0.08 + v * 0.84).map(|v| l + (v - l) * 0.8),
        7 => [l; 3],
        _ => c,
    }
}

/// How much one of the key's ranges (low, high, softness; 0 to 1) holds `v`.
fn within(v: f32, (lo, hi, soft): (f32, f32, f32)) -> f32 {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    let d = if v < lo { lo - v } else if v > hi { v - hi } else { 0.0 };
    if d <= 0.0 {
        1.0
    } else if soft > 0.0 {
        (1.0 - d / soft).max(0.0)
    } else {
        0.0
    }
}

fn color_grade(ctx: &Ctx, mut b: Buf) -> Buf {
    let f = |k: &str, d: f64| ctx.num(k, d) as f32;
    let hdr = ctx.flag("highDynamicRange");
    let white = if hdr { (f("hdrWhite", 100.0) / 100.0).max(0.01) } else { 1.0 };
    let specular = if hdr { f("hdrSpecular", 0.0) / 100.0 } else { 0.0 };
    let file = |key: &str| {
        let path = ctx.value_in(ctx.effect, key).map(|v| v.as_text().to_string()).unwrap_or_default();
        if path.trim().is_empty() {
            return None;
        }
        lut::load(&path).map_err(|e| ctx.note(e)).ok()
    };
    let input = if ctx.choice("inputLut") == 1 { file("inputLutFile") } else { None };
    let look = ctx.choice("look");
    let look_lut = if look == 1 { file("lookFile") } else { None };
    let (temp, tint) = (f("temperature", 0.0) / 100.0, f("tint", 0.0) / 100.0);
    let expo = 2f32.powf(f("exposure", 0.0));
    let contrast = f("contrast", 0.0) / 100.0;
    let (hi, sh, wh, bl) = (f("highlights", 0.0) / 100.0, f("shadows", 0.0) / 100.0, f("whites", 0.0) / 100.0, f("blacks", 0.0) / 100.0);
    let sat = f("saturation", 100.0) / 100.0;
    let intensity = f("lookIntensity", 100.0) / 100.0;
    let (faded, vib, csat) = (f("fadedFilm", 0.0) / 100.0, f("vibrance", 0.0) / 100.0, f("creativeSaturation", 100.0) / 100.0);
    let (st, ht) = (tint_of(ctx.color("shadowTint")), tint_of(ctx.color("highlightTint")));
    let balance = f("tintBalance", 0.0) / 100.0;
    let three = |p: &str| ["Shadows", "Midtones", "Highlights"].map(|k| f(&format!("{p}{k}"), 0.0) / 100.0);
    let (cm, cr, cg, cb) = (three("curve"), three("red"), three("green"), three("blue"));
    let text_of = |k: &str| ctx.value_in(ctx.effect, k).map(|v| v.as_text().to_string()).unwrap_or_default();
    let curves = ["rgbCurve", "redCurve", "greenCurve", "blueCurve"].map(|k| Curve::parse(&text_of(k)));
    let hs = |k: &str, round: bool| Offsets::parse(&text_of(k), round);
    let (hue_sat, hue_hue, hue_luma, luma_sat, sat_sat) = (hs("hueVsSat", true), hs("hueVsHue", true), hs("hueVsLuma", true), hs("lumaVsSat", false), hs("satVsSat", false));
    let any_hs = hue_sat.is_some() || hue_hue.is_some() || hue_luma.is_some() || luma_sat.is_some() || sat_sat.is_some();
    let wheels = [tint_of(ctx.color("shadowsWheel")), tint_of(ctx.color("midtonesWheel")), tint_of(ctx.color("highlightsWheel"))];
    let has_look = look >= 2 || look_lut.is_some();
    let creative = intensity != 0.0 && (has_look || faded != 0.0 || vib != 0.0 || csat != 1.0 || st != [0.0; 3] || ht != [0.0; 3]);
    map_px(&mut b.img, |_, _, p| {
        if p[3] <= 0.0 {
            return p;
        }
        let (mut c, a) = unpremul(p);
        if let Some(l) = &input {
            c = l.apply(c, Between::Tetrahedral);
        }
        // basic correction, its tones up to HDR white
        c = c.map(|v| v / white);
        if temp != 0.0 || tint != 0.0 {
            c = [c[0] * (1.0 + 0.25 * temp), c[1] * (1.0 - 0.25 * tint), c[2] * (1.0 - 0.25 * temp)];
        }
        c = c.map(|v| 0.5 + (v * expo - 0.5) * (1.0 + contrast));
        if hi != 0.0 || sh != 0.0 || wh != 0.0 || bl != 0.0 {
            let lt = luma(c).clamp(0.0, 1.0);
            let d = 0.25 * hi * smoothstep(0.5, 1.0, lt) + 0.25 * sh * (1.0 - smoothstep(0.0, 0.5, lt)) + 0.2 * wh * lt * lt + 0.2 * bl * (1.0 - lt).powi(2);
            c = c.map(|v| v + d);
        }
        if specular != 0.0 {
            c = c.map(|v| if v > 1.0 { 1.0 + (v - 1.0) * (1.0 + specular).max(0.0) } else { v });
        }
        c = c.map(|v| v * white);
        let l = luma(c);
        c = c.map(|v| l + (v - l) * sat);
        // the creative part, as much as its intensity
        if creative {
            let mut d = match &look_lut {
                Some(lut) => lut.apply(c, Between::Tetrahedral),
                None => made_look(look, c),
            };
            d = d.map(|v| v * (1.0 - 0.25 * faded) + 0.12 * faded);
            let l = luma(d);
            if vib != 0.0 {
                let (mx, mn) = (d[0].max(d[1]).max(d[2]), d[0].min(d[1]).min(d[2]));
                let s = if mx > 1e-6 { (mx - mn) / mx } else { 0.0 };
                let k = 1.0 + vib * (1.0 - s.clamp(0.0, 1.0));
                d = d.map(|v| l + (v - l) * k);
            }
            d = d.map(|v| l + (v - l) * csat);
            let pivot = (0.5 + 0.25 * balance).clamp(0.05, 0.95);
            let lt = l.clamp(0.0, 1.0);
            let (wsh, whi) = (1.0 - smoothstep(0.0, pivot, lt), smoothstep(pivot, 1.0, lt));
            for k in 0..3 {
                d[k] += 0.5 * (st[k] * wsh + ht[k] * whi);
            }
            c = [0, 1, 2].map(|k| c[k] + (d[k] - c[k]) * intensity);
        }
        // curves: sliders, points, hue and saturation curves
        c = c.map(|v| tones(v, cm[0], cm[1], cm[2]));
        c = [tones(c[0], cr[0], cr[1], cr[2]), tones(c[1], cg[0], cg[1], cg[2]), tones(c[2], cb[0], cb[1], cb[2])];
        if let Some(m) = &curves[0] {
            c = c.map(|v| m.at(v));
        }
        for k in 0..3 {
            if let Some(cv) = &curves[k + 1] {
                c[k] = cv.at(c[k]);
            }
        }
        if any_hs {
            let base = c.map(|v| v.clamp(0.0, 1.0));
            let (h, s, l) = hsl(base);
            let colourful = (s * 20.0).min(1.0);
            let off = |cv: &Option<Offsets>, x: f32| cv.as_ref().map_or(0.0, |cv| cv.at(x));
            let ms = ((1.0 + 2.0 * off(&hue_sat, h) * colourful) * (1.0 + 2.0 * off(&luma_sat, l)) * (1.0 + 2.0 * off(&sat_sat, s))).max(0.0);
            let (dh, dl) = (off(&hue_hue, h) * colourful, off(&hue_luma, h) * colourful);
            let y = luma(c);
            if dh != 0.0 {
                // what lies past 0 and 1 (HDR) is kept; only the part within is turned
                let turned = rgb((h + dh).rem_euclid(1.0), s, l);
                c = [0, 1, 2].map(|k| c[k] + turned[k] - base[k]);
            }
            c = c.map(|v| y + (v - y) * ms + dl);
        }
        if wheels.iter().any(|w| *w != [0.0; 3]) {
            let lt = luma(c).clamp(0.0, 1.0);
            let (ws, wh) = (1.0 - smoothstep(0.0, 0.5, lt), smoothstep(0.5, 1.0, lt));
            let wm = 1.0 - ws - wh;
            for k in 0..3 {
                c[k] += 0.5 * (wheels[0][k] * ws + wheels[1][k] * wm + wheels[2][k] * wh);
            }
        }
        premul(c, a)
    });
    secondary(ctx, &mut b);
    vignette(ctx, &mut b);
    let sharpen = f("sharpen", 0.0) / 100.0;
    if sharpen != 0.0 {
        sharpened(&mut b, sharpen, None);
    }
    map_px(&mut b.img, |_, _, p| [p[0].max(0.0), p[1].max(0.0), p[2].max(0.0), p[3]]);
    b
}

/// The secondary correction: a key on hue, saturation and lightness, softened, and the
/// correction through it (or the key shown).
fn secondary(ctx: &Ctx, b: &mut Buf) {
    let f = |k: &str, d: f64| ctx.num(k, d) as f32;
    let (temp, tint, contrast, sharpen, sat) = (f("correctionTemperature", 0.0) / 100.0, f("correctionTint", 0.0) / 100.0, f("correctionContrast", 0.0) / 100.0, f("correctionSharpen", 0.0) / 100.0, f("correctionSaturation", 100.0) / 100.0);
    let wheel = tint_of(ctx.color("correction"));
    let show = ctx.flag("showMask");
    if !show && temp == 0.0 && tint == 0.0 && contrast == 0.0 && sharpen == 0.0 && sat == 1.0 && wheel == [0.0; 3] {
        return;
    }
    let hue = (ctx.flag("keyHue") && f("hueRange", 180.0) < 180.0).then(|| (f("hueCenter", 0.0), f("hueRange", 180.0), f("hueSoftness", 0.0)));
    let band = |p: &str| (f(&format!("{p}Low"), 0.0) / 100.0, f(&format!("{p}High"), 100.0) / 100.0, f(&format!("{p}Softness"), 0.0) / 100.0);
    let (sat_band, light_band) = (ctx.flag("keySaturation").then(|| band("saturation")), ctx.flag("keyLightness").then(|| band("lightness")));
    let mut key = b.img.clone();
    map_px(&mut key, |_, _, p| {
        let (c, a) = unpremul(p);
        if a <= 0.0 {
            return [0.0; 4];
        }
        let (h, s, l) = hsl(c.map(|v| v.clamp(0.0, 1.0)));
        let mut w = 1.0;
        if let Some((centre, range, soft)) = hue {
            let d = (h * 360.0 - centre).rem_euclid(360.0).min((centre - h * 360.0).rem_euclid(360.0));
            let hw = if d <= range {
                1.0
            } else if soft > 0.0 {
                (1.0 - (d - range) / soft).max(0.0)
            } else {
                0.0
            };
            // a grey has no hue: it is in the key only as far as it has colour
            w *= hw * (s * 20.0).min(1.0);
        }
        if let Some(r) = sat_band {
            w *= within(s, r);
        }
        if let Some(r) = light_band {
            w *= within(l, r);
        }
        [w; 4]
    });
    let denoise = f("denoise", 0.0) / 100.0;
    if denoise > 0.0 {
        let s = (denoise * 3.0) as f64 * b.scale;
        key = filter::gaussian(&key, s, s, Edge::Repeat);
        let k = 1.0 + denoise * 2.0;
        map_px(&mut key, |_, _, p| [((p[0] - 0.5) * k + 0.5).clamp(0.0, 1.0); 4]);
    }
    let soften = f("blur", 0.0) / 100.0;
    if soften > 0.0 {
        let s = (soften * 10.0) as f64 * b.scale;
        key = filter::gaussian(&key, s, s, Edge::Repeat);
    }
    if ctx.flag("invertMask") {
        map_px(&mut key, |_, _, p| [1.0 - p[0]; 4]);
    }
    let style = ctx.choice("maskStyle");
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        if p[3] <= 0.0 {
            return p;
        }
        let (c, a) = unpremul(p);
        let k = key.px[y * w + x][0];
        let out = if show {
            match style {
                0 => {
                    let g = luma(c).clamp(0.0, 1.0) * 0.2 + 0.4;
                    c.map(|v| g + (v - g) * k)
                }
                1 => c.map(|v| v * k),
                _ => [k; 3],
            }
        } else {
            let mut d = [c[0] * (1.0 + 0.25 * temp), c[1] * (1.0 - 0.25 * tint), c[2] * (1.0 - 0.25 * temp)];
            d = d.map(|v| 0.5 + (v - 0.5) * (1.0 + contrast));
            let l = luma(d);
            d = [0, 1, 2].map(|i| l + (d[i] - l) * sat + 0.5 * wheel[i]);
            [0, 1, 2].map(|i| c[i] + (d[i] - c[i]) * k)
        };
        premul(out, a)
    });
    if !show && sharpen != 0.0 {
        sharpened(b, sharpen, Some(&key));
    }
}

/// The vignette: darker (or lighter) toward the layer's edges.
fn vignette(ctx: &Ctx, b: &mut Buf) {
    let amount = ctx.num("vignetteAmount", 0.0) as f32;
    if amount == 0.0 {
        return;
    }
    let (mid, round, feather) = (ctx.num("vignetteMidpoint", 50.0) as f32 / 100.0, ctx.num("vignetteRoundness", 0.0) as f32 / 100.0, ctx.num("vignetteFeather", 50.0) as f32 / 100.0);
    let r = ctx.bounds;
    let (o, s) = (b.origin, b.scale);
    let aspect = (r.width() / r.height().max(1e-6)) as f32;
    map_px(&mut b.img, |x, y, p| {
        if p[3] <= 0.0 {
            return p;
        }
        let (lx, ly) = (o.x + (x as f64 + 0.5) / s, o.y + (y as f64 + 0.5) / s);
        let (mut nx, mut ny) = (((lx - r.center().x) / (r.width() * 0.5).max(1e-6)) as f32, ((ly - r.center().y) / (r.height() * 0.5).max(1e-6)) as f32);
        // roundness: 0 follows the frame, 1 a circle, -1 squarer
        if round > 0.0 {
            nx *= 1.0 + (aspect.max(1.0) - 1.0) * round;
            ny *= 1.0 + ((1.0 / aspect).max(1.0) - 1.0) * round;
        }
        let pw = 2.0 + (-round).max(0.0) * 6.0;
        let dist = (nx.abs().powf(pw) + ny.abs().powf(pw)).powf(1.0 / pw) / std::f32::consts::SQRT_2;
        let start = mid * 0.9;
        let k = smoothstep(start, start + 0.05 + feather * (1.05 - start), dist);
        let (c, a) = unpremul(p);
        let c = if amount > 0.0 { c.map(|v| v + (1.0 - v) * (amount * 0.2 * k).min(1.0)) } else { c.map(|v| v * (1.0 + amount * 0.2 * k).max(0.0)) };
        premul(c, a)
    });
}

/// Brightness differences from a slight blur added back, by `amount` (times the key, if any).
fn sharpened(b: &mut Buf, amount: f32, key: Option<&Image>) {
    let mut bright = b.img.clone();
    map_px(&mut bright, |_, _, p| [luma(unpremul(p).0); 4]);
    let soft = filter::gaussian(&bright, b.scale, b.scale, Edge::Repeat);
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let (c, a) = unpremul(p);
        if a <= 0.0 {
            return p;
        }
        let i = y * w + x;
        let k = amount * key.map_or(1.0, |m| m.px[i][0]);
        let d = (bright.px[i][0] - soft.px[i][0]) * k * 2.0;
        premul(c.map(|v| (v + d).max(0.0)), a)
    });
}

effect!(
    APPLY_LUT_DEF,
    "applyColorLut",
    "Apply Color LUT",
    "maps colours through a lookup table from a .cube file",
    [
        PropDef::new("lut", "LUT", Kind::Text, Value::Text(String::new())).setting().doc("the .cube file (a full path)"),
        choice("interpolation", "Interpolation", &["nearest", "trilinear", "tetrahedral"], 2),
    ]
);
pub const APPLY_LUT: Spec = Spec::new(&APPLY_LUT_DEF, "utility", apply_lut);

fn apply_lut(ctx: &Ctx, mut b: Buf) -> Buf {
    let path = ctx.value_in(ctx.effect, "lut").map(|v| v.as_text().to_string()).unwrap_or_default();
    if path.trim().is_empty() {
        return b;
    }
    let lut = match lut::load(&path) {
        Ok(l) => l,
        Err(e) => {
            ctx.note(e);
            return b;
        }
    };
    let between = [Between::Nearest, Between::Trilinear, Between::Tetrahedral][(ctx.choice("interpolation") as usize).min(2)];
    map_px(&mut b.img, |_, _, p| {
        if p[3] <= 0.0 {
            return p;
        }
        let (c, a) = unpremul(p);
        premul(lut.apply(c, between), a)
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 2] = [COLOR_GRADE, APPLY_LUT];

#[cfg(test)]
mod tests;
