//! Colour correction, part one: tinting (Tint, Tritone, Photo Filter), tone (Brightness &
//! Contrast, Levels, Exposure, Gamma/Pedestal/Gain), colour (Hue/Saturation, Vibrance, Color
//! Balance, Channel Mixer, Black & White), picking colours out (Change to Color, Leave Color) and
//! mapping them round a palette (Colorama).
//!
//! These work on each pixel's colour as it is (not premultiplied), in the project's working
//! space; see-through pixels keep their alpha.

use veymelo_keyframe::{Gradient, Kind, Value};
use veymelo_project::{Body, DefNode, GroupDef, PropDef, Unit};

use crate::defs::{angle, choice, color, effect, flag, layer, pct, ranged};
use crate::util::{hsl, luma, map_px, map_straight, mix3, premul, rgb, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "color";

pub(crate) fn rgb3(c: veymelo_color::Rgba) -> [f32; 3] {
    [c.r as f32, c.g as f32, c.b as f32]
}

effect!(
    TINT_DEF,
    "tint",
    "Tint",
    "maps dark to one colour and light to another",
    [color("mapBlackTo", "Map Black To", 0.0, 0.0, 0.0), color("mapWhiteTo", "Map White To", 1.0, 1.0, 1.0), pct("amountToTint", "Amount to Tint", 100.0, 0.0, 100.0)]
);
pub const TINT: Spec = Spec::new(&TINT_DEF, CATEGORY, tint);

fn tint(ctx: &Ctx, mut b: Buf) -> Buf {
    let (black, white) = (rgb3(ctx.color("mapBlackTo")), rgb3(ctx.color("mapWhiteTo")));
    let k = ctx.num("amountToTint", 100.0) as f32 / 100.0;
    map_straight(&mut b.img, |c| mix3(c, mix3(black, white, luma(c).clamp(0.0, 1.0)), k));
    b
}

effect!(
    TRITONE_DEF,
    "tritone",
    "Tritone",
    "maps shadows, midtones and highlights to three colours",
    [
        color("highlights", "Highlights", 1.0, 1.0, 1.0),
        color("midtones", "Midtones", 0.5, 0.4, 0.3),
        color("shadows", "Shadows", 0.0, 0.0, 0.0),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
    ]
);
pub const TRITONE: Spec = Spec::new(&TRITONE_DEF, CATEGORY, tritone);

fn tritone(ctx: &Ctx, mut b: Buf) -> Buf {
    let (hi, mid, sh) = (rgb3(ctx.color("highlights")), rgb3(ctx.color("midtones")), rgb3(ctx.color("shadows")));
    let k = 1.0 - ctx.num("blendWithOriginal", 0.0) as f32 / 100.0;
    map_straight(&mut b.img, |c| {
        let l = luma(c).clamp(0.0, 1.0);
        let to = if l < 0.5 { mix3(sh, mid, l * 2.0) } else { mix3(mid, hi, (l - 0.5) * 2.0) };
        mix3(c, to, k)
    });
    b
}

effect!(
    BRIGHTNESS_CONTRAST_DEF,
    "brightnessContrast",
    "Brightness & Contrast",
    "lightens or darkens, and spreads or gathers the tones",
    [
        ranged("brightness", "Brightness", 0.0, -150.0, 150.0),
        ranged("contrast", "Contrast", 0.0, -100.0, 100.0),
        flag("useLegacy", "Use Legacy", false).doc("the old way: brightness added, contrast scaled about middle grey (clips black and white)"),
    ]
);
pub const BRIGHTNESS_CONTRAST: Spec = Spec::new(&BRIGHTNESS_CONTRAST_DEF, CATEGORY, brightness_contrast);

/// One channel through Brightness & Contrast's modern curve: black and white stay where they
/// are. Brightness bends the tones by where it puts middle grey: up to 0.8 at +100, down to 0.2 at
/// −100 (brightening and darkening mirror each other). Contrast gives them an S about middle
/// grey whose slope there is 1.5 at +100 and 0.5 at −100.
fn bc_modern(v: f32, b: f32, c: f32) -> f32 {
    if v >= 1.0 {
        return v;
    }
    let mut v = v.max(0.0);
    if b != 0.0 {
        // the power that takes middle grey to 0.5 + 0.3·|b| (or as far down)
        let k = (1.0 / (0.5 - 0.3 * b.abs())).log2();
        v = if b > 0.0 { 1.0 - (1.0 - v).powf(k) } else { v.powf(k) };
    }
    if c != 0.0 {
        let g = (1.0 + 0.5 * c).max(0.05);
        v = if v < 0.5 { 0.5 * (2.0 * v).powf(g) } else { 1.0 - 0.5 * (2.0 - 2.0 * v).max(0.0).powf(g) };
    }
    v
}

fn brightness_contrast(ctx: &Ctx, mut b: Buf) -> Buf {
    let (br, ct) = (ctx.num("brightness", 0.0) as f32 / 100.0, ctx.num("contrast", 0.0) as f32 / 100.0);
    if br == 0.0 && ct == 0.0 {
        return b;
    }
    if ctx.flag("useLegacy") {
        let k = if ct >= 0.0 { 1.0 / (1.0 - ct * 0.99) } else { 1.0 + ct };
        map_straight(&mut b.img, |c| c.map(|v| ((v + br - 0.5) * k + 0.5).max(0.0)));
    } else {
        map_straight(&mut b.img, |c| c.map(|v| bc_modern(v, br, ct)));
    }
    b
}

const HUE_RANGES: [&str; 6] = ["red", "yellow", "green", "cyan", "blue", "magenta"];

/// Hue/Saturation's settings: the master's, each range's (hue, saturation and lightness, and the
/// hues it covers), and colorize.
macro_rules! hue_saturation_nodes {
    ($( [$p:literal, $name:literal, $centre:expr] ),*) => {
        &[
            DefNode::Prop(choice("channelControl", "Channel Control", &["master", "reds", "yellows", "greens", "cyans", "blues", "magentas"], 0).doc("which range the panel shows (all are kept)")),
            DefNode::Prop(angle("masterHue", "Master Hue", 0.0)),
            DefNode::Prop(pct("masterSaturation", "Master Saturation", 0.0, -100.0, 100.0)),
            DefNode::Prop(pct("masterLightness", "Master Lightness", 0.0, -100.0, 100.0)),
            $(
                DefNode::Prop(angle(concat!($p, "Hue"), concat!($name, " Hue"), 0.0)),
                DefNode::Prop(pct(concat!($p, "Saturation"), concat!($name, " Saturation"), 0.0, -100.0, 100.0)),
                DefNode::Prop(pct(concat!($p, "Lightness"), concat!($name, " Lightness"), 0.0, -100.0, 100.0)),
                DefNode::Prop(ranged(concat!($p, "RangeStart"), concat!($name, " Range Start"), $centre - 15.0, -360.0, 720.0).unit(Unit::Degrees)),
                DefNode::Prop(ranged(concat!($p, "RangeEnd"), concat!($name, " Range End"), $centre + 15.0, -360.0, 720.0).unit(Unit::Degrees)),
                DefNode::Prop(ranged(concat!($p, "StartFalloff"), concat!($name, " Start Falloff"), 30.0, 0.0, 180.0).unit(Unit::Degrees)),
                DefNode::Prop(ranged(concat!($p, "EndFalloff"), concat!($name, " End Falloff"), 30.0, 0.0, 180.0).unit(Unit::Degrees)),
            )*
            DefNode::Prop(flag("colorize", "Colorize", false).doc("gives everything one hue and saturation, keeping its lightness")),
            DefNode::Prop(angle("colorizeHue", "Colorize Hue", 0.0)),
            DefNode::Prop(pct("colorizeSaturation", "Colorize Saturation", 25.0, 0.0, 100.0)),
            DefNode::Prop(pct("colorizeLightness", "Colorize Lightness", 0.0, -100.0, 100.0)),
        ]
    };
}

pub static HUE_SATURATION_DEF: GroupDef = GroupDef {
    id: "hueSaturation",
    name: "Hue/Saturation",
    body: Body::Fixed(hue_saturation_nodes!(["red", "Red", 0.0], ["yellow", "Yellow", 60.0], ["green", "Green", 120.0], ["cyan", "Cyan", 180.0], ["blue", "Blue", 240.0], ["magenta", "Magenta", 300.0])),
    doc: "turns hues, and changes saturation and lightness, of everything or of one range of colours",
};
pub const HUE_SATURATION: Spec = Spec::new(&HUE_SATURATION_DEF, CATEGORY, hue_saturation);

/// One range of colour: the hues it holds wholly (from start to end, round the wheel), how far
/// it fades out either side, and what it changes.
struct Range {
    start: f32,
    end: f32,
    before: f32,
    after: f32,
    hue: f32,
    sat: f32,
    light: f32,
}

impl Range {
    /// How much hue `h` (degrees) belongs to the range: 1 within it, less across its fades.
    fn weight(&self, h: f32) -> f32 {
        let width = (self.end - self.start).rem_euclid(360.0);
        let d = (h - self.start).rem_euclid(360.0);
        if d <= width {
            return 1.0;
        }
        let (past, short) = (d - width, 360.0 - d);
        let wa = if self.after > 0.0 { 1.0 - past / self.after } else { 0.0 };
        let wb = if self.before > 0.0 { 1.0 - short / self.before } else { 0.0 };
        wa.max(wb).clamp(0.0, 1.0)
    }
}

fn more_saturated(s: f32, by: f32) -> f32 {
    if by >= 0.0 { s + (1.0 - s) * by * s.min(1.0) } else { s * (1.0 + by) }
}

fn lighter(l: f32, by: f32) -> f32 {
    if by >= 0.0 { l + (1.0 - l) * by } else { l * (1.0 + by) }
}

fn hue_saturation(ctx: &Ctx, mut b: Buf) -> Buf {
    let n = |k: &str, d: f64| ctx.num(k, d) as f32;
    let (hue, sat, light) = (n("masterHue", 0.0) / 360.0, n("masterSaturation", 0.0) / 100.0, n("masterLightness", 0.0) / 100.0);
    let colorize = ctx.flag("colorize");
    let (ch, cs, cl) = (n("colorizeHue", 0.0) / 360.0, n("colorizeSaturation", 25.0) / 100.0, n("colorizeLightness", 0.0) / 100.0);
    let centres = [0.0, 60.0, 120.0, 180.0, 240.0, 300.0];
    let ranges: Vec<Range> = HUE_RANGES
        .iter()
        .zip(centres)
        .map(|(r, c)| Range {
            start: n(&format!("{r}RangeStart"), c - 15.0),
            end: n(&format!("{r}RangeEnd"), c + 15.0),
            before: n(&format!("{r}StartFalloff"), 30.0).max(0.0),
            after: n(&format!("{r}EndFalloff"), 30.0).max(0.0),
            hue: n(&format!("{r}Hue"), 0.0) / 360.0,
            sat: n(&format!("{r}Saturation"), 0.0) / 100.0,
            light: n(&format!("{r}Lightness"), 0.0) / 100.0,
        })
        .filter(|r| r.hue != 0.0 || r.sat != 0.0 || r.light != 0.0)
        .collect();
    map_straight(&mut b.img, |c| {
        let (h, s, l) = hsl(c);
        if colorize {
            return rgb(ch, cs, (l + cl * if cl > 0.0 { 1.0 - l } else { l }).clamp(0.0, 1.0));
        }
        let (mut h2, mut s2, mut l2) = (h + hue, more_saturated(s, sat), lighter(l, light));
        for r in &ranges {
            // greys have no hue: they belong to no range
            let w = r.weight(h * 360.0) * (s * 20.0).min(1.0);
            if w > 0.0 {
                h2 += r.hue * w;
                s2 = more_saturated(s2.clamp(0.0, 1.0), r.sat * w);
                l2 = lighter(l2, r.light * w);
            }
        }
        rgb(h2.rem_euclid(1.0), s2.clamp(0.0, 1.0), l2)
    });
    b
}

const CLIPS: &[&str] = &["off", "on", "offFor32bpc"];
const LEVEL_CHANNELS: [&str; 4] = ["red", "green", "blue", "alpha"];

/// Levels' settings: for all colours, then for each channel.
macro_rules! levels_nodes {
    ($( [$p:literal, $name:literal] ),*) => {
        &[
            DefNode::Prop(choice("channel", "Channel", &["rgb", "red", "green", "blue", "alpha"], 0).doc("which channel's values the panel shows (all are kept)")),
            DefNode::Prop(ranged("inputBlack", "Input Black", 0.0, -1.0, 2.0).doc("this and darker becomes output black (0 is black, 1 white)")),
            DefNode::Prop(ranged("inputWhite", "Input White", 1.0, -1.0, 2.0).doc("this and lighter becomes output white")),
            DefNode::Prop(ranged("gamma", "Gamma", 1.0, 0.1, 10.0).doc("above 1 lightens the middle tones, below 1 darkens them")),
            DefNode::Prop(ranged("outputBlack", "Output Black", 0.0, -1.0, 2.0)),
            DefNode::Prop(ranged("outputWhite", "Output White", 1.0, -1.0, 2.0)),
            DefNode::Prop(choice("clipToOutputBlack", "Clip To Output Black", CLIPS, 2).doc("whether darker than input black stays at output black (offFor32bpc: only in 8 and 16-bit projects)")),
            DefNode::Prop(choice("clipToOutputWhite", "Clip To Output White", CLIPS, 2)),
            $(
                DefNode::Prop(ranged(concat!($p, "InputBlack"), concat!($name, " Input Black"), 0.0, -1.0, 2.0)),
                DefNode::Prop(ranged(concat!($p, "InputWhite"), concat!($name, " Input White"), 1.0, -1.0, 2.0)),
                DefNode::Prop(ranged(concat!($p, "Gamma"), concat!($name, " Gamma"), 1.0, 0.1, 10.0)),
                DefNode::Prop(ranged(concat!($p, "OutputBlack"), concat!($name, " Output Black"), 0.0, -1.0, 2.0)),
                DefNode::Prop(ranged(concat!($p, "OutputWhite"), concat!($name, " Output White"), 1.0, -1.0, 2.0)),
                DefNode::Prop(choice(concat!($p, "ClipToOutputBlack"), concat!($name, " Clip To Output Black"), CLIPS, 2)),
                DefNode::Prop(choice(concat!($p, "ClipToOutputWhite"), concat!($name, " Clip To Output White"), CLIPS, 2)),
            )*
        ]
    };
}

pub static LEVELS_DEF: GroupDef = GroupDef {
    id: "levels",
    name: "Levels",
    body: Body::Fixed(levels_nodes!(["red", "Red"], ["green", "Green"], ["blue", "Blue"], ["alpha", "Alpha"])),
    doc: "stretches the tones between an input black and white to an output black and white, with a gamma between; for all colours, then each channel",
};
pub const LEVELS: Spec = Spec::new(&LEVELS_DEF, CATEGORY, levels);

/// Input black, input white, gamma, output black, output white.
pub(crate) type Five = [f32; 5];

pub(crate) const SAME: Five = [0.0, 1.0, 1.0, 0.0, 1.0];

pub(crate) fn level(v: f32, [ib, iw, g, ob, ow]: Five, (clip_black, clip_white): (bool, bool)) -> f32 {
    let mut t = (v - ib) / (iw - ib).max(1e-6);
    if clip_black {
        t = t.max(0.0);
    }
    if clip_white {
        t = t.min(1.0);
    }
    ob + (ow - ob) * t.max(0.0).powf(1.0 / g)
}

/// The five values and clips of one channel (`""` for all colours).
pub(crate) fn level_settings(ctx: &Ctx, p: &str) -> (Five, (bool, bool)) {
    let key = |k: &str| if p.is_empty() { k[..1].to_lowercase() + &k[1..] } else { format!("{p}{k}") };
    let names = ["InputBlack", "InputWhite", "Gamma", "OutputBlack", "OutputWhite"];
    let v: Five = std::array::from_fn(|i| ctx.num(&key(names[i]), SAME[i] as f64) as f32);
    // off for 32 bpc: on in an 8 or 16-bit project, off in a float one
    let clip = |k: &str| match ctx.value_in(ctx.effect, &key(k)).map_or(2, |v| v.as_choice()) {
        0 => false,
        1 => true,
        _ => !ctx.float,
    };
    ([v[0], v[1], v[2].max(0.01), v[3], v[4]], (clip("ClipToOutputBlack"), clip("ClipToOutputWhite")))
}

fn levels(ctx: &Ctx, mut b: Buf) -> Buf {
    let all = level_settings(ctx, "");
    let chans = LEVEL_CHANNELS.map(|c| level_settings(ctx, c));
    if all.0 == SAME && chans.iter().all(|c| c.0 == SAME) {
        return b;
    }
    map_px(&mut b.img, |_, _, p| {
        if p[3] <= 0.0 && chans[3].0 == SAME {
            return p;
        }
        let (c, a) = unpremul(p);
        let o = [0, 1, 2].map(|i| {
            let v = level(c[i], all.0, all.1);
            if chans[i].0 == SAME { v } else { level(v, chans[i].0, chans[i].1) }
        });
        let a = if chans[3].0 == SAME { a } else { level(a, chans[3].0, chans[3].1).clamp(0.0, 1.0) };
        premul(o, a)
    });
    b
}

effect!(
    EXPOSURE_DEF,
    "exposure",
    "Exposure",
    "changes exposure in stops, as a camera does, in linear light",
    [
        choice("channels", "Channels", &["master", "individualChannels"], 0),
        ranged("exposure", "Exposure", 0.0, -20.0, 20.0).doc("stops: each one doubles (or halves) the light"),
        ranged("offset", "Offset", 0.0, -2.0, 2.0),
        ranged("gammaCorrection", "Gamma Correction", 1.0, 0.01, 9.99),
        ranged("redExposure", "Red Exposure", 0.0, -20.0, 20.0),
        ranged("redOffset", "Red Offset", 0.0, -2.0, 2.0),
        ranged("redGammaCorrection", "Red Gamma Correction", 1.0, 0.01, 9.99),
        ranged("greenExposure", "Green Exposure", 0.0, -20.0, 20.0),
        ranged("greenOffset", "Green Offset", 0.0, -2.0, 2.0),
        ranged("greenGammaCorrection", "Green Gamma Correction", 1.0, 0.01, 9.99),
        ranged("blueExposure", "Blue Exposure", 0.0, -20.0, 20.0),
        ranged("blueOffset", "Blue Offset", 0.0, -2.0, 2.0),
        ranged("blueGammaCorrection", "Blue Gamma Correction", 1.0, 0.01, 9.99),
        flag("bypassLinearLightConversion", "Bypass Linear Light Conversion", false).doc("work on the stored values instead of in linear light"),
    ]
);
pub const EXPOSURE: Spec = Spec::new(&EXPOSURE_DEF, CATEGORY, exposure);

/// `|v|^p` keeping `v`'s sign.
fn signed_pow(v: f32, p: f32) -> f32 {
    v.abs().powf(p).copysign(v)
}

fn exposure(ctx: &Ctx, mut b: Buf) -> Buf {
    let one = |p: &str| {
        let k = |s: &str| if p.is_empty() { s[..1].to_lowercase() + &s[1..] } else { format!("{p}{s}") };
        (2f32.powf(ctx.num(&k("Exposure"), 0.0) as f32), ctx.num(&k("Offset"), 0.0) as f32, ctx.num(&k("GammaCorrection"), 1.0).max(0.01) as f32)
    };
    let s = if ctx.choice("channels") == 1 { [one("red"), one("green"), one("blue")] } else { [one(""); 3] };
    if s.iter().all(|v| *v == (1.0, 0.0, 1.0)) {
        return b;
    }
    let bypass = ctx.flag("bypassLinearLightConversion");
    map_straight(&mut b.img, |c| {
        [0, 1, 2].map(|i| {
            let (e, off, g) = s[i];
            if bypass {
                return signed_pow(c[i] * e + off, 1.0 / g);
            }
            let lin = veymelo_color::srgb_to_linear(c[i].max(0.0) as f64) as f32;
            let o = signed_pow(lin * e + off, 1.0 / g);
            (veymelo_color::linear_to_srgb(o.abs() as f64) as f32).copysign(o)
        })
    });
    b
}

effect!(
    BLACK_WHITE_DEF,
    "blackWhite",
    "Black & White",
    "turns colours grey, each hue as light as its slider says",
    [
        ranged("reds", "Reds", 40.0, -200.0, 300.0).unit(Unit::Percent),
        ranged("yellows", "Yellows", 60.0, -200.0, 300.0).unit(Unit::Percent),
        ranged("greens", "Greens", 40.0, -200.0, 300.0).unit(Unit::Percent),
        ranged("cyans", "Cyans", 60.0, -200.0, 300.0).unit(Unit::Percent),
        ranged("blues", "Blues", 20.0, -200.0, 300.0).unit(Unit::Percent),
        ranged("magentas", "Magentas", 80.0, -200.0, 300.0).unit(Unit::Percent),
        flag("tint", "Tint", false),
        color("tintColor", "Tint Color", 0.88, 0.76, 0.6),
    ]
);
pub const BLACK_WHITE: Spec = Spec::new(&BLACK_WHITE_DEF, CATEGORY, black_white);

/// A pixel's grey: its smallest channel, and its colourfulness (largest minus smallest) as much
/// as its hue's slider says: a grey stays as it is, a pure hue becomes its slider's grey.
fn black_white(ctx: &Ctx, mut b: Buf) -> Buf {
    let w = ["reds", "yellows", "greens", "cyans", "blues", "magentas"].map(|k| ctx.num(k, 0.0) as f32 / 100.0);
    let tint = ctx.flag("tint").then(|| hsl(rgb3(ctx.color("tintColor"))));
    map_straight(&mut b.img, |c| {
        let h6 = hsl(c).0 * 6.0;
        let i = h6.floor() as usize % 6;
        let weight = w[i] + (w[(i + 1) % 6] - w[i]) * (h6 - h6.floor());
        let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        let grey = (mn + (mx - mn) * weight).max(0.0);
        match tint {
            Some((th, ts, _)) => rgb(th, ts, grey.min(1.0)),
            None => [grey; 3],
        }
    });
    b
}

effect!(
    CHANGE_TO_COLOR_DEF,
    "changeToColor",
    "Change to Color",
    "changes one colour (and those near it) into another",
    [
        color("from", "From", 1.0, 0.0, 0.0),
        color("to", "To", 0.0, 0.0, 1.0),
        choice("change", "Change", &["hue", "hueAndLightness", "hueAndSaturation", "hueLightnessAndSaturation"], 0),
        choice("changeBy", "Change By", &["settingToColor", "transformingToColor"], 0).doc("set what changes to To's, or move it by as much as To is from From"),
        pct("hueTolerance", "Hue Tolerance", 5.0, 0.0, 100.0),
        pct("lightnessTolerance", "Lightness Tolerance", 50.0, 0.0, 100.0),
        pct("saturationTolerance", "Saturation Tolerance", 50.0, 0.0, 100.0),
        pct("softness", "Softness", 50.0, 0.0, 100.0),
        flag("viewCorrectionMatte", "View Correction Matte", false).doc("show what is changed, white, in grey"),
    ]
);
pub const CHANGE_TO_COLOR: Spec = Spec::new(&CHANGE_TO_COLOR_DEF, CATEGORY, change_to_color);

/// How far apart two hues (0 to 1) are, 0 to 1 (opposite).
fn hue_apart(a: f32, b: f32) -> f32 {
    ((a - b + 0.5).rem_euclid(1.0) - 0.5).abs() * 2.0
}

fn change_to_color(ctx: &Ctx, mut b: Buf) -> Buf {
    let (from, to) = (rgb3(ctx.color("from")), rgb3(ctx.color("to")));
    let tol = ["hueTolerance", "lightnessTolerance", "saturationTolerance"].map(|k| ctx.num(k, 0.0) as f32 / 100.0);
    let soft = ctx.num("softness", 50.0) as f32 / 100.0;
    let change = ctx.choice("change");
    let (light_too, sat_too) = (matches!(change, 1 | 3), matches!(change, 2 | 3));
    let by = ctx.choice("changeBy") == 1;
    let matte = ctx.flag("viewCorrectionMatte");
    let ((fh, fs, fl), (th, ts, tl)) = (hsl(from), hsl(to));
    map_straight(&mut b.img, |c| {
        let (h, s, l) = hsl(c);
        let d = [hue_apart(h, fh), (l - fl).abs(), (s - fs).abs()];
        let mut k = 1.0f32;
        for i in 0..3 {
            let over = d[i] - tol[i];
            if over > 0.0 {
                k = k.min(if soft <= 1e-6 { 0.0 } else { (1.0 - over / soft).clamp(0.0, 1.0) });
            }
        }
        if matte {
            return [k; 3];
        }
        if k <= 0.0 {
            return c;
        }
        let shift = |a: f32, b: f32| ((b - a + 0.5).rem_euclid(1.0) - 0.5) * k;
        let (nh, ns, nl) = if by {
            ((h + shift(fh, th)).rem_euclid(1.0), if sat_too { s + (ts - fs) * k } else { s }, if light_too { l + (tl - fl) * k } else { l })
        } else {
            ((h + shift(h, th)).rem_euclid(1.0), if sat_too { s + (ts - s) * k } else { s }, if light_too { l + (tl - l) * k } else { l })
        };
        rgb(nh, ns.clamp(0.0, 1.0), nl.clamp(0.0, 1.0))
    });
    b
}

effect!(
    LEAVE_COLOR_DEF,
    "leaveColor",
    "Leave Color",
    "turns everything grey but one colour",
    [
        pct("amountToDecolor", "Amount to Decolor", 0.0, 0.0, 100.0),
        color("colorToLeave", "Color To Leave", 1.0, 0.0, 0.0),
        pct("tolerance", "Tolerance", 15.0, 0.0, 100.0),
        pct("edgeSoftness", "Edge Softness", 0.0, 0.0, 100.0),
        choice("matchColors", "Match Colors", &["usingRgb", "usingHue"], 0),
    ]
);
pub const LEAVE_COLOR: Spec = Spec::new(&LEAVE_COLOR_DEF, CATEGORY, leave_color);

fn leave_color(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = ctx.num("amountToDecolor", 0.0) as f32 / 100.0;
    if k <= 0.0 {
        return b;
    }
    let keep = rgb3(ctx.color("colorToLeave"));
    let (tol, soft) = (ctx.num("tolerance", 15.0) as f32 / 100.0, ctx.num("edgeSoftness", 0.0) as f32 / 100.0);
    let by_hue = ctx.choice("matchColors") == 1;
    let kh = hsl(keep).0;
    map_straight(&mut b.img, |c| {
        let d = if by_hue {
            let (h, s, _) = hsl(c);
            if s < 0.02 { 1.0 } else { hue_apart(h, kh) }
        } else {
            ((c[0] - keep[0]).powi(2) + (c[1] - keep[1]).powi(2) + (c[2] - keep[2]).powi(2)).sqrt() / 3f32.sqrt()
        };
        let kept = if tol >= 1.0 || d <= tol {
            1.0
        } else if soft <= 1e-6 {
            0.0
        } else {
            (1.0 - (d - tol) / soft).clamp(0.0, 1.0)
        };
        mix3(c, [luma(c); 3], k * (1.0 - kept))
    });
    b
}

/// Photo Filter's filters: camera filters and plain colours (the usual values), then Custom.
const FILTERS: [(&str, [u8; 3]); 21] = [
    ("warming85", [236, 138, 0]),
    ("warmingLba", [250, 150, 0]),
    ("warming81", [235, 177, 19]),
    ("cooling80", [0, 109, 255]),
    ("coolingLbb", [0, 93, 255]),
    ("cooling82", [0, 181, 255]),
    ("red", [234, 26, 26]),
    ("orange", [243, 132, 23]),
    ("yellow", [249, 227, 28]),
    ("green", [25, 201, 25]),
    ("cyan", [29, 203, 234]),
    ("blue", [29, 53, 234]),
    ("violet", [155, 29, 234]),
    ("magenta", [227, 24, 227]),
    ("sepia", [172, 122, 51]),
    ("deepRed", [255, 0, 0]),
    ("deepBlue", [0, 34, 205]),
    ("deepEmerald", [0, 140, 0]),
    ("deepYellow", [255, 213, 0]),
    ("underwater", [0, 194, 177]),
    ("custom", [0, 0, 0]),
];

const FILTER_NAMES: [&str; 21] = {
    let mut out = [""; 21];
    let mut i = 0;
    while i < 21 {
        out[i] = FILTERS[i].0;
        i += 1;
    }
    out
};

effect!(
    PHOTO_FILTER_DEF,
    "photoFilter",
    "Photo Filter",
    "as if shot through a coloured filter",
    [
        choice("filter", "Filter", &FILTER_NAMES, 0),
        color("color", "Color", 0.93, 0.54, 0.09).doc("the filter's colour, with filter custom"),
        pct("density", "Density", 25.0, 0.0, 100.0),
        flag("preserveLuminosity", "Preserve Luminosity", true).doc("keep how light each pixel is"),
    ]
);
pub const PHOTO_FILTER: Spec = Spec::new(&PHOTO_FILTER_DEF, CATEGORY, photo_filter);

fn photo_filter(ctx: &Ctx, mut b: Buf) -> Buf {
    let f = ctx.choice("filter") as usize;
    let tone = if f + 1 < FILTERS.len() { FILTERS[f].1.map(|v| v as f32 / 255.0) } else { rgb3(ctx.color("color")) };
    let d = ctx.num("density", 25.0) as f32 / 100.0;
    let keep = ctx.flag("preserveLuminosity");
    map_straight(&mut b.img, |c| {
        let mut o = mix3(c, [c[0] * tone[0], c[1] * tone[1], c[2] * tone[2]], d);
        if keep {
            let k = luma(c) / luma(o).max(1e-6);
            o = o.map(|v| v * k);
        }
        o
    });
    b
}

effect!(
    VIBRANCE_DEF,
    "vibrance",
    "Vibrance",
    "saturates dull colours more than vivid ones",
    [pct("vibrance", "Vibrance", 0.0, -100.0, 100.0), pct("saturation", "Saturation", 0.0, -100.0, 100.0)]
);
pub const VIBRANCE: Spec = Spec::new(&VIBRANCE_DEF, CATEGORY, vibrance);

fn vibrance(ctx: &Ctx, mut b: Buf) -> Buf {
    let (vib, sat) = (ctx.num("vibrance", 0.0) as f32 / 100.0, ctx.num("saturation", 0.0) as f32 / 100.0);
    if vib == 0.0 && sat == 0.0 {
        return b;
    }
    map_straight(&mut b.img, |c| {
        let (h, s, l) = hsl(c);
        rgb(h, (s * (1.0 + sat) + vib * (1.0 - s) * s.min(0.5)).clamp(0.0, 1.0), l)
    });
    b
}

effect!(
    COLOR_BALANCE_DEF,
    "colorBalance",
    "Color Balance",
    "shifts red, green and blue in the shadows, midtones and highlights",
    [
        pct("shadowRedBalance", "Shadow Red Balance", 0.0, -100.0, 100.0),
        pct("shadowGreenBalance", "Shadow Green Balance", 0.0, -100.0, 100.0),
        pct("shadowBlueBalance", "Shadow Blue Balance", 0.0, -100.0, 100.0),
        pct("midtoneRedBalance", "Midtone Red Balance", 0.0, -100.0, 100.0),
        pct("midtoneGreenBalance", "Midtone Green Balance", 0.0, -100.0, 100.0),
        pct("midtoneBlueBalance", "Midtone Blue Balance", 0.0, -100.0, 100.0),
        pct("highlightRedBalance", "Highlight Red Balance", 0.0, -100.0, 100.0),
        pct("highlightGreenBalance", "Highlight Green Balance", 0.0, -100.0, 100.0),
        pct("highlightBlueBalance", "Highlight Blue Balance", 0.0, -100.0, 100.0),
        flag("preserveLuminosity", "Preserve Luminosity", false),
    ]
);
pub const COLOR_BALANCE: Spec = Spec::new(&COLOR_BALANCE_DEF, CATEGORY, color_balance);

fn color_balance(ctx: &Ctx, mut b: Buf) -> Buf {
    let three = |p: &str| ["Red", "Green", "Blue"].map(|c| ctx.num(&format!("{p}{c}Balance"), 0.0) as f32 / 100.0);
    let (sh, mid, hi) = (three("shadow"), three("midtone"), three("highlight"));
    let keep = ctx.flag("preserveLuminosity");
    map_straight(&mut b.img, |c| {
        let l = luma(c).clamp(0.0, 1.0);
        let (ws, wh) = ((1.0 - l * 2.0).clamp(0.0, 1.0), (l * 2.0 - 1.0).clamp(0.0, 1.0));
        let wm = 1.0 - ws - wh;
        let mut o = [0, 1, 2].map(|i| (c[i] + (sh[i] * ws + mid[i] * wm + hi[i] * wh) * 0.5).max(0.0));
        if keep {
            let d = luma(c) - luma(o);
            o = o.map(|v| (v + d).max(0.0));
        }
        o
    });
    b
}

effect!(
    CHANNEL_MIXER_DEF,
    "channelMixer",
    "Channel Mixer",
    "makes each channel from the others, in %, plus a constant",
    [
        pct("redRed", "Red-Red", 100.0, -200.0, 200.0),
        pct("redGreen", "Red-Green", 0.0, -200.0, 200.0),
        pct("redBlue", "Red-Blue", 0.0, -200.0, 200.0),
        pct("redConst", "Red-Const", 0.0, -200.0, 200.0),
        pct("greenRed", "Green-Red", 0.0, -200.0, 200.0),
        pct("greenGreen", "Green-Green", 100.0, -200.0, 200.0),
        pct("greenBlue", "Green-Blue", 0.0, -200.0, 200.0),
        pct("greenConst", "Green-Const", 0.0, -200.0, 200.0),
        pct("blueRed", "Blue-Red", 0.0, -200.0, 200.0),
        pct("blueGreen", "Blue-Green", 0.0, -200.0, 200.0),
        pct("blueBlue", "Blue-Blue", 100.0, -200.0, 200.0),
        pct("blueConst", "Blue-Const", 0.0, -200.0, 200.0),
        flag("monochrome", "Monochrome", false).doc("red's mix for all three: grey"),
    ]
);
pub const CHANNEL_MIXER: Spec = Spec::new(&CHANNEL_MIXER_DEF, CATEGORY, channel_mixer);

fn channel_mixer(ctx: &Ctx, mut b: Buf) -> Buf {
    let row = |c: &str| ["Red", "Green", "Blue", "Const"].map(|k| ctx.num(&format!("{c}{k}"), 0.0) as f32 / 100.0);
    let m = [row("red"), row("green"), row("blue")];
    let mono = ctx.flag("monochrome");
    map_straight(&mut b.img, |c| {
        let out = |r: [f32; 4]| (c[0] * r[0] + c[1] * r[1] + c[2] * r[2] + r[3]).max(0.0);
        if mono { [out(m[0]); 3] } else { [out(m[0]), out(m[1]), out(m[2])] }
    });
    b
}

effect!(
    GAMMA_PEDESTAL_GAIN_DEF,
    "gammaPedestalGain",
    "Gamma/Pedestal/Gain",
    "each channel bent by a gamma, then stretched from its pedestal (black) to its gain (white)",
    [
        ranged("blackStretch", "Black Stretch", 0.0, 0.0, 4.0).doc("lifts the darkest tones"),
        ranged("redGamma", "Red Gamma", 1.0, 0.1, 10.0),
        ranged("redPedestal", "Red Pedestal", 0.0, -2.0, 2.0),
        ranged("redGain", "Red Gain", 1.0, 0.0, 4.0),
        ranged("greenGamma", "Green Gamma", 1.0, 0.1, 10.0),
        ranged("greenPedestal", "Green Pedestal", 0.0, -2.0, 2.0),
        ranged("greenGain", "Green Gain", 1.0, 0.0, 4.0),
        ranged("blueGamma", "Blue Gamma", 1.0, 0.1, 10.0),
        ranged("bluePedestal", "Blue Pedestal", 0.0, -2.0, 2.0),
        ranged("blueGain", "Blue Gain", 1.0, 0.0, 4.0),
    ]
);
pub const GAMMA_PEDESTAL_GAIN: Spec = Spec::new(&GAMMA_PEDESTAL_GAIN_DEF, CATEGORY, gamma_pedestal_gain);

fn gamma_pedestal_gain(ctx: &Ctx, mut b: Buf) -> Buf {
    let ch = |c: &str| (ctx.num(&format!("{c}Gamma"), 1.0).max(0.01) as f32, ctx.num(&format!("{c}Pedestal"), 0.0) as f32, ctx.num(&format!("{c}Gain"), 1.0) as f32);
    let s = [ch("red"), ch("green"), ch("blue")];
    let stretch = ctx.num("blackStretch", 0.0).max(0.0) as f32;
    map_straight(&mut b.img, |c| {
        // the toe fades out by middle grey
        let toe = |v: f32| if stretch > 0.0 && v > 0.0 && v < 1.0 { v + stretch * 0.25 * v * (1.0 - v).powi(4) } else { v };
        [0, 1, 2].map(|i| {
            let (g, p, k) = s[i];
            (p + (k - p) * toe(c[i]).max(0.0).powf(g)).max(0.0)
        })
    });
    b
}

const PHASES: &[&str] = &["intensity", "red", "green", "blue", "hue", "lightness", "saturation", "value", "alpha", "zero"];
const PALETTES: &[&str] = &["hueCycle", "rampGrayscale", "fire", "ice", "rampRed", "rampGreen", "rampBlue", "pastelCycle", "custom"];

effect!(
    COLORAMA_DEF,
    "colorama",
    "Colorama",
    "maps a pixel's brightness (or hue, red…) round a cycle of colours",
    [
        choice("getPhaseFrom", "Get Phase From", PHASES, 0),
        layer("addPhase", "Add Phase").doc("a layer whose phase is added"),
        choice("addPhaseFrom", "Add Phase From", PHASES, 0),
        choice("addMode", "Add Mode", &["wrap", "clamp", "average", "screen"], 0),
        angle("phaseShift", "Phase Shift", 0.0).doc("turns the phase round the cycle"),
        choice("usePresetPalette", "Use Preset Palette", PALETTES, 0),
        PropDef::new("outputCycle", "Output Cycle", Kind::Gradient, Value::Gradient(Gradient { colors: Vec::new(), opacities: Vec::new() }))
            .doc("the colours round the cycle (positions 0 to 1, going round), with preset custom"),
        ranged("cycleRepetitions", "Cycle Repetitions", 1.0, 0.0, 100.0),
        flag("interpolatePalette", "Interpolate Palette", true).doc("off: each colour holds until the next (posterized)"),
        choice("modify", "Modify", &["all", "red", "green", "blue", "hue", "lightness", "saturation", "value", "redAndGreen", "greenAndBlue", "blueAndRed", "none"], 0),
        flag("modifyAlpha", "Modify Alpha", false).doc("take the cycle's opacity too"),
        flag("changeEmptyPixels", "Change Empty Pixels", false),
        color("matchingColor", "Matching Color", 1.0, 1.0, 1.0),
        pct("matchingTolerance", "Matching Tolerance", 0.0, 0.0, 100.0),
        pct("matchingSoftness", "Matching Softness", 0.0, 0.0, 100.0),
        choice("matchingMode", "Matching Mode", &["off", "rgb", "hue", "chroma"], 0).doc("only colours near the matching colour are changed"),
        layer("maskLayer", "Mask Layer"),
        choice("maskingMode", "Masking Mode", &["off", "luminance", "invertedLuminance", "alpha", "invertedAlpha"], 0),
        flag("compositeOverLayer", "Composite Over Layer", true).doc("off: only what is changed is left"),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
    ]
);
pub const COLORAMA: Spec = Spec::new(&COLORAMA_DEF, CATEGORY, colorama);

/// A phase (0 to 1) from a colour and its alpha.
fn phase_of(c: [f32; 3], a: f32, from: u32) -> f32 {
    match from {
        1 => c[0],
        2 => c[1],
        3 => c[2],
        4 => hsl(c).0,
        5 => hsl(c).2,
        6 => hsl(c).1,
        7 => c[0].max(c[1]).max(c[2]),
        8 => a,
        9 => 0.0,
        _ => luma(c),
    }
    .clamp(0.0, 1.0)
}

/// The colours round a cycle: stops at positions 0 to 1, going round.
struct Cycle(Vec<(f32, [f32; 4])>);

impl Cycle {
    fn preset(i: u32) -> Cycle {
        let ramp = |v: &[(f32, [f32; 3])]| Cycle(v.iter().map(|(p, c)| (*p, [c[0], c[1], c[2], 1.0])).collect());
        match i {
            1 => ramp(&[(0.0, [0.0; 3]), (0.999, [1.0; 3])]),
            2 => ramp(&[(0.0, [0.0; 3]), (0.3, [0.7, 0.05, 0.0]), (0.6, [1.0, 0.55, 0.0]), (0.85, [1.0, 0.95, 0.4]), (0.999, [1.0; 3])]),
            3 => ramp(&[(0.0, [0.0, 0.0, 0.1]), (0.4, [0.1, 0.3, 0.8]), (0.75, [0.5, 0.85, 1.0]), (0.999, [1.0; 3])]),
            4 => ramp(&[(0.0, [0.0; 3]), (0.999, [1.0, 0.0, 0.0])]),
            5 => ramp(&[(0.0, [0.0; 3]), (0.999, [0.0, 1.0, 0.0])]),
            6 => ramp(&[(0.0, [0.0; 3]), (0.999, [0.0, 0.0, 1.0])]),
            7 => ramp(&[(0.0, [1.0, 0.7, 0.7]), (1.0 / 3.0, [0.7, 1.0, 0.7]), (2.0 / 3.0, [0.7, 0.7, 1.0])]),
            _ => ramp(&[(0.0, [1.0, 0.0, 0.0]), (1.0 / 6.0, [1.0, 1.0, 0.0]), (2.0 / 6.0, [0.0, 1.0, 0.0]), (0.5, [0.0, 1.0, 1.0]), (4.0 / 6.0, [0.0, 0.0, 1.0]), (5.0 / 6.0, [1.0, 0.0, 1.0])]),
        }
    }

    /// From a gradient's stops (its opacity read at each colour stop).
    fn from(g: &Gradient) -> Option<Cycle> {
        if g.colors.is_empty() {
            return None;
        }
        let mut stops: Vec<(f32, [f32; 4])> = g.colors.iter().map(|(p, c)| (p.rem_euclid(1.0) as f32, [c[0] as f32, c[1] as f32, c[2] as f32, g.at(*p)[3] as f32])).collect();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(Cycle(stops))
    }

    fn at(&self, ph: f32, smooth: bool) -> [f32; 4] {
        let n = self.0.len();
        let ph = ph.rem_euclid(1.0);
        let k = self.0.iter().rposition(|s| s.0 <= ph).unwrap_or(n - 1);
        let (p0, c0) = self.0[k];
        if !smooth || n == 1 {
            return c0;
        }
        let (p1, c1) = self.0[(k + 1) % n];
        let t = ((ph - p0).rem_euclid(1.0) / (p1 - p0).rem_euclid(1.0).max(1e-6)).clamp(0.0, 1.0);
        [0, 1, 2, 3].map(|i| c0[i] + (c1[i] - c0[i]) * t)
    }
}

/// How much colour `c` is like `key` (1 is the same), by RGB, hue or chroma.
fn matching(c: [f32; 3], key: [f32; 3], mode: u32, tol: f32, soft: f32) -> f32 {
    let d = match mode {
        2 => {
            let ((h1, s1, _), (h2, _, _)) = (hsl(c), hsl(key));
            hue_apart(h1, h2) + (1.0 - s1.min(1.0)) * 0.5
        }
        3 => {
            let chroma = |c: [f32; 3]| {
                let s = c[0] + c[1] + c[2];
                if s > 1e-5 { c.map(|v| v / s) } else { [1.0 / 3.0; 3] }
            };
            let (a, b) = (chroma(c), chroma(key));
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() * 1.5
        }
        _ => ((c[0] - key[0]).powi(2) + (c[1] - key[1]).powi(2) + (c[2] - key[2]).powi(2)).sqrt() / 3f32.sqrt(),
    };
    if d <= tol {
        1.0
    } else if soft > 0.0 {
        (1.0 - (d - tol) / soft).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn colorama(ctx: &Ctx, mut b: Buf) -> Buf {
    let shift = ctx.num("phaseShift", 0.0) as f32 / 360.0;
    let from = ctx.choice("getPhaseFrom");
    let cycles = ctx.num("cycleRepetitions", 1.0).max(0.0) as f32;
    let smooth = ctx.flag("interpolatePalette");
    let preset = ctx.choice("usePresetPalette");
    let custom = (preset as usize == PALETTES.len() - 1).then(|| ctx.value_in(ctx.effect, "outputCycle").and_then(|v| v.as_gradient().and_then(Cycle::from))).flatten();
    let cycle = custom.unwrap_or_else(|| Cycle::preset(preset));
    let blend = 1.0 - ctx.num("blendWithOriginal", 0.0) as f32 / 100.0;
    let add = ctx.layer_over("addPhase", &b, Take::All, true);
    let (add_from, add_mode) = (ctx.choice("addPhaseFrom"), ctx.choice("addMode"));
    let modify = ctx.choice("modify");
    let (modify_alpha, change_empty) = (ctx.flag("modifyAlpha"), ctx.flag("changeEmptyPixels"));
    let match_mode = ctx.choice("matchingMode");
    let key = rgb3(ctx.color("matchingColor"));
    let (tol, soft) = (ctx.num("matchingTolerance", 0.0) as f32 / 100.0, ctx.num("matchingSoftness", 0.0) as f32 / 100.0);
    let mask_mode = ctx.choice("maskingMode");
    let mask = if mask_mode != 0 { ctx.layer_over("maskLayer", &b, Take::All, true) } else { None };
    let composite = ctx.flag("compositeOverLayer");
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let a = p[3];
        if a <= 0.0 && !change_empty {
            return p;
        }
        let (c, _) = unpremul(p);
        let mut ph = phase_of(c, a, from);
        if let Some(img) = &add {
            let (ac, aa) = unpremul(img.px[y * w + x]);
            let q = phase_of(ac, aa, add_from);
            ph = match add_mode {
                1 => (ph + q).min(1.0),
                2 => (ph + q) * 0.5,
                3 => ph + q - ph * q,
                _ => (ph + q).rem_euclid(1.0),
            };
        }
        let pc = cycle.at((ph * cycles + shift).rem_euclid(1.0), smooth);
        let pc3 = [pc[0], pc[1], pc[2]];
        let (h, s, l) = hsl(c);
        let (qh, qs, ql) = hsl(pc3);
        let new = match modify {
            1 => [pc[0], c[1], c[2]],
            2 => [c[0], pc[1], c[2]],
            3 => [c[0], c[1], pc[2]],
            4 => rgb(qh, s, l),
            5 => rgb(h, s, ql),
            6 => rgb(h, qs, l),
            7 => {
                let (m, pm) = (c[0].max(c[1]).max(c[2]), pc3[0].max(pc3[1]).max(pc3[2]));
                if m > 1e-6 { c.map(|v| v * pm / m) } else { [pm; 3] }
            }
            8 => [pc[0], pc[1], c[2]],
            9 => [c[0], pc[1], pc[2]],
            10 => [pc[0], c[1], pc[2]],
            11 => c,
            _ => pc3,
        };
        let new_a = if modify_alpha { pc[3] } else if a > 0.0 { a } else { 1.0 };
        // where it applies: the matching colour, then the mask layer
        let mut k = if match_mode == 0 { 1.0 } else { matching(c, key, match_mode, tol, soft) };
        if let Some(m) = &mask {
            let (mc, ma) = unpremul(m.px[y * w + x]);
            let lum = luma(mc) * ma;
            k *= match mask_mode {
                1 => lum,
                2 => 1.0 - lum,
                3 => ma,
                _ => 1.0 - ma,
            }
            .clamp(0.0, 1.0);
        }
        let mixed = mix3(c, new, k * blend);
        let oa = a + (new_a - a) * k * blend;
        let oa = if composite { oa } else { oa * k };
        premul(mixed, oa.clamp(0.0, 1.0))
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 15] = [
    TINT,
    TRITONE,
    BRIGHTNESS_CONTRAST,
    HUE_SATURATION,
    LEVELS,
    EXPOSURE,
    BLACK_WHITE,
    CHANGE_TO_COLOR,
    LEAVE_COLOR,
    PHOTO_FILTER,
    VIBRANCE,
    COLOR_BALANCE,
    CHANNEL_MIXER,
    GAMMA_PEDESTAL_GAIN,
    COLORAMA,
];

#[cfg(test)]
mod tests;
