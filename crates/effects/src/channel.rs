//! Channels: turned inside out (Invert), moved between each other or taken from other layers (Set
//! Channels, Shift Channels, Channel Combiner, Set Matte), grown and shrunk (Minimax), worked on
//! with numbers (Arithmetic, Compound Arithmetic), mixed with a colour, another layer or the
//! layer as it was before its effects (Solid Composite, Blend, Calculations, Composite), and
//! a colour matte taken out (Remove Color Matting).

use veymelo_color::BlendMode;
use veymelo_raster::Px;

use crate::color::rgb3;
use crate::defs::{choice, color, effect, flag, layer, pct, ranged};
use crate::util::{hsl, luma, map_px, premul, rgb, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "channel";

fn mix4(a: Px, b: Px, t: f32) -> Px {
    [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// What a channel can be set from: a channel of a pixel, or all, half or nothing.
const SOURCES: &[&str] = &["red", "green", "blue", "alpha", "luminance", "hue", "lightness", "saturation", "fullOn", "halfOn", "off"];

/// A pixel's straight colour `c` and alpha `a` by `source` of [`SOURCES`].
fn pick(source: u32, c: [f32; 3], a: f32) -> f32 {
    match source {
        0 => c[0],
        1 => c[1],
        2 => c[2],
        3 => a,
        4 => luma(c),
        5 => hsl(c).0,
        6 => hsl(c).2,
        7 => hsl(c).1,
        8 => 1.0,
        9 => 0.5,
        _ => 0.0,
    }
}

/// Another layer laid over this one (stretched or centred), or this layer itself.
fn other_or_self(ctx: &Ctx, b: &Buf, path: &str, stretch: bool) -> Option<veymelo_raster::Image> {
    ctx.layer_over(path, b, Take::All, stretch)
}

const INVERT_CHANNELS: &[&str] = &["rgb", "red", "green", "blue", "hls", "hue", "lightness", "saturation", "yiq", "luminance", "inPhaseChrominance", "quadratureChrominance", "alpha"];

effect!(
    INVERT_DEF,
    "invert",
    "Invert",
    "a channel (or a set of them) turned inside out: dark to light, a hue to its opposite",
    [choice("channel", "Channel", INVERT_CHANNELS, 0), pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),]
);
pub const INVERT: Spec = Spec::new(&INVERT_DEF, CATEGORY, invert);

fn to_yiq(c: [f32; 3]) -> [f32; 3] {
    [0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2], 0.596 * c[0] - 0.274 * c[1] - 0.322 * c[2], 0.211 * c[0] - 0.523 * c[1] + 0.312 * c[2]]
}

fn from_yiq(c: [f32; 3]) -> [f32; 3] {
    [c[0] + 0.956 * c[1] + 0.621 * c[2], c[0] - 0.272 * c[1] - 0.647 * c[2], c[0] - 1.106 * c[1] + 1.703 * c[2]]
}

fn inverted(ch: u32, s: [f32; 3]) -> [f32; 3] {
    match ch {
        1 => [1.0 - s[0], s[1], s[2]],
        2 => [s[0], 1.0 - s[1], s[2]],
        3 => [s[0], s[1], 1.0 - s[2]],
        4..=7 => {
            let (mut h, mut sat, mut l) = hsl(s);
            if matches!(ch, 4 | 5) {
                h = 1.0 - h;
            }
            if matches!(ch, 4 | 6) {
                l = 1.0 - l;
            }
            if matches!(ch, 4 | 7) {
                sat = 1.0 - sat;
            }
            rgb(h, sat, l)
        }
        8..=11 => {
            // the brightness turned about white, the colour axes about nothing
            let mut q = to_yiq(s);
            if matches!(ch, 8 | 9) {
                q[0] = 1.0 - q[0];
            }
            if matches!(ch, 8 | 10) {
                q[1] = -q[1];
            }
            if matches!(ch, 8 | 11) {
                q[2] = -q[2];
            }
            from_yiq(q).map(|v| v.max(0.0))
        }
        _ => s.map(|v| 1.0 - v),
    }
}

fn invert(ctx: &Ctx, mut b: Buf) -> Buf {
    let ch = ctx.choice("channel");
    let k = 1.0 - (ctx.num("blendWithOriginal", 0.0) / 100.0) as f32;
    map_px(&mut b.img, |_, _, p| {
        let a = p[3];
        if ch == 12 {
            // the alpha turned inside out, the colour kept
            let na = 1.0 - a;
            let f = if a > 0.0 { na / a } else { 0.0 };
            return mix4(p, [p[0] * f, p[1] * f, p[2] * f, na], k);
        }
        if a <= 0.0 {
            return p;
        }
        let s = [p[0] / a, p[1] / a, p[2] / a];
        let o = inverted(ch, s);
        [(s[0] + (o[0] - s[0]) * k) * a, (s[1] + (o[1] - s[1]) * k) * a, (s[2] + (o[2] - s[2]) * k) * a, a]
    });
    b
}

effect!(
    SET_CHANNELS_DEF,
    "setChannels",
    "Set Channels",
    "each of red, green, blue and alpha taken from a channel of this or another layer",
    [
        layer("sourceLayer1", "Source Layer 1").doc("none: this layer"),
        choice("setRedToSource1s", "Set Red To Source 1's", SOURCES, 0),
        layer("sourceLayer2", "Source Layer 2"),
        choice("setGreenToSource2s", "Set Green To Source 2's", SOURCES, 1),
        layer("sourceLayer3", "Source Layer 3"),
        choice("setBlueToSource3s", "Set Blue To Source 3's", SOURCES, 2),
        layer("sourceLayer4", "Source Layer 4"),
        choice("setAlphaToSource4s", "Set Alpha To Source 4's", SOURCES, 3),
        flag("stretchLayersToFit", "Stretch Layers to Fit", true),
    ]
);
pub const SET_CHANNELS: Spec = Spec::new(&SET_CHANNELS_DEF, CATEGORY, set_channels);

fn set_channels(ctx: &Ctx, mut b: Buf) -> Buf {
    let take = ["setRedToSource1s", "setGreenToSource2s", "setBlueToSource3s", "setAlphaToSource4s"].map(|id| ctx.choice(id));
    let stretch = ctx.flag("stretchLayersToFit");
    let layers: Vec<Option<veymelo_raster::Image>> = ["sourceLayer1", "sourceLayer2", "sourceLayer3", "sourceLayer4"].iter().map(|id| other_or_self(ctx, &b, id, stretch)).collect();
    if take == [0, 1, 2, 3] && layers.iter().all(Option::is_none) {
        return b;
    }
    let own = b.img.clone();
    let w = b.img.width;
    map_px(&mut b.img, |x, y, _| {
        let i = y * w + x;
        let v: [f32; 4] = std::array::from_fn(|k| {
            let (c, a) = unpremul(layers[k].as_ref().map_or(own.px[i], |l| l.px[i]));
            pick(take[k], c, a)
        });
        premul([v[0], v[1], v[2]], v[3].clamp(0.0, 1.0))
    });
    b
}

const SHIFT_SOURCES: &[&str] = &["alpha", "red", "green", "blue", "luminance", "hue", "lightness", "saturation", "fullOn", "halfOn", "fullOff"];
const SHIFT_AS: [u32; 11] = [3, 0, 1, 2, 4, 5, 6, 7, 8, 9, 10];

effect!(
    SHIFT_CHANNELS_DEF,
    "shiftChannels",
    "Shift Channels",
    "each channel taken from another of the same layer",
    [
        choice("takeAlphaFrom", "Take Alpha From", SHIFT_SOURCES, 0),
        choice("takeRedFrom", "Take Red From", SHIFT_SOURCES, 1),
        choice("takeGreenFrom", "Take Green From", SHIFT_SOURCES, 2),
        choice("takeBlueFrom", "Take Blue From", SHIFT_SOURCES, 3),
    ]
);
pub const SHIFT_CHANNELS: Spec = Spec::new(&SHIFT_CHANNELS_DEF, CATEGORY, shift_channels);

fn shift_channels(ctx: &Ctx, mut b: Buf) -> Buf {
    let take = ["takeRedFrom", "takeGreenFrom", "takeBlueFrom", "takeAlphaFrom"].map(|id| SHIFT_AS[ctx.choice(id).min(10) as usize]);
    if take == [0, 1, 2, 3] {
        return b;
    }
    map_px(&mut b.img, |_, _, p| {
        let (c, a) = unpremul(p);
        let v = take.map(|s| pick(s, c, a));
        premul([v[0], v[1], v[2]], v[3].clamp(0.0, 1.0))
    });
    b
}

const COMBINER_FROM: &[&str] = &["rgbToHls", "hlsToRgb", "rgbToYuv", "yuvToRgb", "red", "green", "blue", "alpha", "lightness", "hue", "saturation", "luminance", "maxRgb", "minRgb"];
const COMBINER_TO: &[&str] = &["redOnly", "greenOnly", "blueOnly", "alphaOnly", "lightnessOnly", "hueOnly", "saturationOnly", "grayscale", "saturationMultiplied"];

effect!(
    COMBINER_DEF,
    "channelCombiner",
    "Channel Combiner",
    "colours turned into another colour space and back, or one value put into a channel",
    [
        flag("useSecondLayer", "Use 2nd Layer", false),
        layer("sourceLayer", "Source Layer"),
        choice("from", "From", COMBINER_FROM, 8),
        choice("to", "To", COMBINER_TO, 4).doc("where a single value goes"),
        flag("invert", "Invert", false),
        flag("solidAlpha", "Solid Alpha", false),
    ]
);
pub const COMBINER: Spec = Spec::new(&COMBINER_DEF, CATEGORY, combiner);

fn to_yuv(c: [f32; 3]) -> [f32; 3] {
    let y = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    [y, 0.492 * (c[2] - y) + 0.5, 0.877 * (c[0] - y) + 0.5]
}

fn from_yuv(c: [f32; 3]) -> [f32; 3] {
    let (y, u, v) = (c[0], c[1] - 0.5, c[2] - 0.5);
    let (r, b) = (y + v / 0.877, y + u / 0.492);
    [r, (y - 0.299 * r - 0.114 * b) / 0.587, b]
}

fn combiner(ctx: &Ctx, mut b: Buf) -> Buf {
    let (from, to) = (ctx.choice("from"), ctx.choice("to"));
    let flip = ctx.flag("invert");
    let solid = ctx.flag("solidAlpha");
    let second = if ctx.flag("useSecondLayer") { Some(other_or_self(ctx, &b, "sourceLayer", true).unwrap_or_else(|| b.img.clone())) } else { None };
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let (c, mut a) = unpremul(p);
        let (sc, sa) = second.as_ref().map_or((c, a), |img| unpremul(img.px[y * w + x]));
        if sa <= 0.0 && from != 7 && second.is_none() {
            return if solid { premul(c, 1.0) } else { p };
        }
        let iv = |v: f32| if flip { 1.0 - v } else { v };
        let out = if from <= 3 {
            let o = match from {
                0 => {
                    let (h, s, l) = hsl(sc);
                    [h, l, s]
                }
                1 => rgb(sc[0], sc[2], sc[1]),
                2 => to_yuv(sc),
                _ => from_yuv(sc),
            };
            o.map(iv)
        } else {
            let (h, s, l) = hsl(sc);
            let v = iv(match from {
                4 => sc[0],
                5 => sc[1],
                6 => sc[2],
                7 => sa,
                8 => l,
                9 => h,
                10 => s,
                11 => luma(sc),
                12 => sc[0].max(sc[1]).max(sc[2]),
                _ => sc[0].min(sc[1]).min(sc[2]),
            });
            // the one value into one channel; the rest set as that kind of picture has them
            match to {
                0 => [v, 0.0, 0.0],
                1 => [0.0, v, 0.0],
                2 => [0.0, 0.0, v],
                3 => {
                    a = v.clamp(0.0, 1.0);
                    [1.0; 3]
                }
                5 => rgb(v, 1.0, 0.5),
                6 => rgb(0.0, v.clamp(0.0, 1.0), 0.5),
                8 => {
                    let (h0, s0, l0) = hsl(c);
                    rgb(h0, (s0 * v).clamp(0.0, 1.0), l0)
                }
                _ => [v; 3],
            }
        };
        if solid {
            a = 1.0;
        }
        premul(out.map(|v| v.max(0.0)), a)
    });
    b
}

effect!(
    MINIMAX_DEF,
    "minimax",
    "Minimax",
    "each pixel the darkest (minimum) or lightest (maximum) round it: shapes grown or shrunk",
    [
        choice("operation", "Operation", &["minimum", "maximum", "minimumThenMaximum", "maximumThenMinimum"], 1),
        ranged("radius", "Radius", 0.0, 0.0, 500.0),
        choice("channel", "Channel", &["color", "alpha", "alphaAndColor", "red", "green", "blue"], 0),
        choice("direction", "Direction", &["horizontalAndVertical", "justHorizontal", "justVertical"], 0),
        flag("dontShrinkEdges", "Don't Shrink Edges", false).doc("past the layer's edge counts as its edge pixels, not as nothing"),
    ]
);
pub const MINIMAX: Spec = Spec::new(&MINIMAX_DEF, CATEGORY, minimax);

/// The least (or, `most`, the greatest) of each `r` either side along a line, past its ends
/// `outside` (or, None, its end values). By van Herk and Gil-Werman: the line (with `r` more each
/// side) cut into blocks a window long, each block's running extreme from its start and from its
/// end; a window spans the end of one block and the start of the next, so its extreme is the
/// better of those two.
pub(crate) fn line_extreme(v: &[f32], r: usize, most: bool, outside: Option<f32>) -> Vec<f32> {
    let n = v.len();
    if n == 0 || r == 0 {
        return v.to_vec();
    }
    let at = |i: i64| -> f32 {
        if i >= 0 && (i as usize) < n {
            v[i as usize]
        } else {
            outside.unwrap_or(v[i.clamp(0, n as i64 - 1) as usize])
        }
    };
    let pick = |a: f32, b: f32| if most { a.max(b) } else { a.min(b) };
    let k = 2 * r + 1;
    let len = n + 2 * r;
    let padded: Vec<f32> = (0..len as i64).map(|i| at(i - r as i64)).collect();
    let (mut from_start, mut from_end) = (padded.clone(), padded.clone());
    for i in 1..len {
        if i % k != 0 {
            from_start[i] = pick(from_start[i - 1], padded[i]);
        }
    }
    for i in (0..len - 1).rev() {
        if (i + 1) % k != 0 {
            from_end[i] = pick(from_end[i + 1], padded[i]);
        }
    }
    // the window of pixel x is padded[x .. x + k]
    (0..n).map(|x| pick(from_end[x], from_start[x + k - 1])).collect()
}

/// A plane's extremes over a box `rx` across and `ry` down each way.
pub(crate) fn box_extreme(plane: &[f32], w: usize, h: usize, rx: usize, ry: usize, most: bool, outside: Option<f32>) -> Vec<f32> {
    let mut out = plane.to_vec();
    // the rows on the cores apart, then the columns (as the rows of the plane turned)
    if rx > 0 {
        veymelo_raster::par::rows(&mut out, w, |_, row| {
            let done = line_extreme(row, rx, most, outside);
            row.copy_from_slice(&done);
        });
    }
    if ry > 0 {
        let mut cols = turned(&out, w, h);
        veymelo_raster::par::rows(&mut cols, h, |_, col| {
            let done = line_extreme(col, ry, most, outside);
            col.copy_from_slice(&done);
        });
        out = turned(&cols, h, w);
    }
    out
}

/// A plane `w` × `h` turned on its diagonal (its rows its columns).
pub(crate) fn turned(plane: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    veymelo_raster::par::rows(&mut out, h, |x, row| {
        for (y, o) in row.iter_mut().enumerate() {
            *o = plane[y * w + x];
        }
    });
    out
}

fn minimax(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = (ctx.num("radius", 0.0) * b.scale).round().max(0.0) as usize;
    if r == 0 {
        return b;
    }
    let op = ctx.choice("operation");
    let (rx, ry) = match ctx.choice("direction") {
        1 => (r, 0),
        2 => (0, r),
        _ => (r, r),
    };
    let which: [bool; 4] = match ctx.choice("channel") {
        1 => [false, false, false, true],
        2 => [true; 4],
        3 => [true, false, false, false],
        4 => [false, true, false, false],
        5 => [false, false, true, false],
        _ => [true, true, true, false],
    };
    if which[3] && op != 0 && !ctx.adjustment {
        b.pad(rx.max(ry));
    }
    let outside = if ctx.flag("dontShrinkEdges") { None } else { Some(0.0) };
    let (w, h) = (b.img.width, b.img.height);
    let mut planes: Vec<Vec<f32>> = (0..4).map(|k| b.img.px.iter().map(|p| if k == 3 { p[3] } else { unpremul(*p).0[k] }).collect()).collect();
    for (k, plane) in planes.iter_mut().enumerate() {
        if !which[k] {
            continue;
        }
        let ext = |p: &[f32], most: bool| box_extreme(p, w, h, rx, ry, most, outside);
        *plane = match op {
            0 => ext(plane, false),
            1 => ext(plane, true),
            2 => ext(&ext(plane, false), true),
            _ => ext(&ext(plane, true), false),
        };
    }
    for (i, p) in b.img.px.iter_mut().enumerate() {
        *p = premul([planes[0][i], planes[1][i], planes[2][i]], planes[3][i].clamp(0.0, 1.0));
    }
    b
}

const OPERATORS: &[&str] = &["and", "or", "xor", "add", "subtract", "difference", "min", "max", "blockAbove", "blockBelow", "slice", "multiply", "screen"];

effect!(
    ARITHMETIC_DEF,
    "arithmetic",
    "Arithmetic",
    "each colour channel worked with a number of its own: added, multiplied, cut off above or below…",
    [
        choice("operator", "Operator", OPERATORS, 3),
        ranged("redValue", "Red Value", 0.0, 0.0, 255.0),
        ranged("greenValue", "Green Value", 0.0, 0.0, 255.0),
        ranged("blueValue", "Blue Value", 0.0, 0.0, 255.0),
        flag("clipResultValues", "Clip Result Values", true),
    ]
);
pub const ARITHMETIC: Spec = Spec::new(&ARITHMETIC_DEF, CATEGORY, arithmetic);

fn arithmetic(ctx: &Ctx, mut b: Buf) -> Buf {
    let op = ctx.choice("operator");
    let k = ["redValue", "greenValue", "blueValue"].map(|id| (ctx.num(id, 0.0) / 255.0) as f32);
    let clip = ctx.flag("clipResultValues");
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    crate::util::map_straight(&mut b.img, |c| {
        [0, 1, 2].map(|i| {
            let (v, kv) = (c[i], k[i]);
            let r = match op {
                0 => (byte(v) & byte(kv)) as f32 / 255.0,
                1 => (byte(v) | byte(kv)) as f32 / 255.0,
                2 => (byte(v) ^ byte(kv)) as f32 / 255.0,
                3 => v + kv,
                4 => v - kv,
                5 => (v - kv).abs(),
                6 => v.min(kv),
                7 => v.max(kv),
                8 if v > kv => 0.0,
                9 if v < kv => 0.0,
                10 => {
                    if v >= kv {
                        1.0
                    } else {
                        0.0
                    }
                }
                11 => v * kv,
                12 => 1.0 - (1.0 - v) * (1.0 - kv),
                _ => v,
            };
            if clip { r.clamp(0.0, 1.0) } else { r.max(0.0) }
        })
    });
    b
}

const SOLID_MODES: &[&str] = &["normal", "add", "multiply", "screen", "overlay", "softLight", "hardLight", "darken", "lighten", "difference", "color", "luminosity"];
const SOLID_OF: [BlendMode; 12] = [
    BlendMode::Normal,
    BlendMode::Add,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
    BlendMode::Color,
    BlendMode::Luminosity,
];

effect!(
    SOLID_DEF,
    "solidComposite",
    "Solid Composite",
    "the layer over a solid colour, mixed by a blend mode",
    [
        pct("sourceOpacity", "Source Opacity", 100.0, 0.0, 100.0),
        color("color", "Color", 1.0, 1.0, 1.0),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0).doc("of the colour"),
        choice("blendingMode", "Blending Mode", SOLID_MODES, 0),
    ]
);
pub const SOLID: Spec = Spec::new(&SOLID_DEF, CATEGORY, solid_composite);

fn solid_composite(ctx: &Ctx, mut b: Buf) -> Buf {
    let keep = (ctx.num("sourceOpacity", 100.0) / 100.0) as f32;
    let c = ctx.color("color");
    let op = ((ctx.num("opacity", 100.0) / 100.0) as f32 * c.a as f32).clamp(0.0, 1.0);
    let mode = SOLID_OF[ctx.choice("blendingMode").min(11) as usize];
    let under = premul(rgb3(c), op);
    map_px(&mut b.img, |_, _, p| {
        let mut o = veymelo_color::blend(mode, under, p.map(|v| v * keep), 0.5);
        o[3] = o[3].clamp(0.0, 1.0);
        [o[0].max(0.0), o[1].max(0.0), o[2].max(0.0), o[3]]
    });
    b
}

effect!(
    UNMATTE_DEF,
    "removeColorMatting",
    "Remove Color Matting",
    "the colour a see-through picture was mixed with (premultiplied with) taken out of its edges",
    [color("backgroundColor", "Background Color", 0.0, 0.0, 0.0), flag("clipHdrResults", "Clip HDR Results", true),]
);
pub const UNMATTE: Spec = Spec::new(&UNMATTE_DEF, CATEGORY, remove_color_matting);

fn remove_color_matting(ctx: &Ctx, mut b: Buf) -> Buf {
    let bg = rgb3(ctx.color("backgroundColor"));
    let clip = ctx.flag("clipHdrResults");
    map_px(&mut b.img, |_, _, p| {
        let a = p[3];
        if a <= 1e-6 {
            return [0.0; 4];
        }
        let c = [0, 1, 2].map(|i| {
            let v = (p[i] / a - bg[i] * (1.0 - a)).max(0.0);
            if clip { v.min(a) } else { v }
        });
        [c[0], c[1], c[2], a]
    });
    b
}

const MATTE_FROM: &[&str] = &["red", "green", "blue", "alpha", "luminance", "hue", "lightness", "saturation", "full", "off"];
const MATTE_AS: [u32; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 10];

effect!(
    SET_MATTE_DEF,
    "setMatte",
    "Set Matte",
    "the layer's alpha taken from a channel of another layer (or this one)",
    [
        layer("takeMatteFromLayer", "Take Matte From Layer").doc("none: this layer"),
        choice("useForMatte", "Use For Matte", MATTE_FROM, 3),
        flag("invertMatte", "Invert Matte", false),
        flag("stretchMatteToFit", "Stretch Matte to Fit", true),
        flag("compositeMatteWithOriginal", "Composite Matte with Original", true).doc("the new matte times the layer's own alpha"),
        flag("premultiplyMatteLayer", "Premultiply Matte Layer", true),
    ]
);
pub const SET_MATTE: Spec = Spec::new(&SET_MATTE_DEF, CATEGORY, set_matte);

fn set_matte(ctx: &Ctx, mut b: Buf) -> Buf {
    let from = MATTE_AS[ctx.choice("useForMatte").min(9) as usize];
    let flip = ctx.flag("invertMatte");
    let with_own = ctx.flag("compositeMatteWithOriginal");
    let premultiplied = ctx.flag("premultiplyMatteLayer");
    let matte = other_or_self(ctx, &b, "takeMatteFromLayer", ctx.flag("stretchMatteToFit"));
    let own = matte.is_none();
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let (c, a) = unpremul(p);
        let (mc, ma) = matte.as_ref().map_or((c, a), |m| unpremul(m.px[y * w + x]));
        let mut m = pick(from, mc, ma);
        if premultiplied && !matches!(from, 3 | 8 | 10) {
            m *= ma;
        }
        let m = m.clamp(0.0, 1.0);
        let m = if flip { 1.0 - m } else { m };
        // the layer's own alpha is the matte it has already: not taken twice
        let na = if with_own && (!own || from != 3 || flip) { a * m } else { m };
        premul(c, na.clamp(0.0, 1.0))
    });
    b
}

effect!(
    BLEND_DEF,
    "blend",
    "Blend",
    "the layer mixed with another: crossfaded, given its colour or tint, or the darker or lighter of the two",
    [
        layer("blendWithLayer", "Blend With Layer").doc("none: this layer"),
        choice("mode", "Mode", &["crossfade", "colorOnly", "tintOnly", "darkenOnly", "lightenOnly"], 0),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
        choice("ifLayerSizesDiffer", "If Layer Sizes Differ", &["center", "stretchToFit"], 0),
    ]
);
pub const BLEND: Spec = Spec::new(&BLEND_DEF, CATEGORY, blend);

fn blend(ctx: &Ctx, mut b: Buf) -> Buf {
    let mode = ctx.choice("mode");
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    if keep >= 1.0 {
        return b;
    }
    let other = other_or_self(ctx, &b, "blendWithLayer", ctx.choice("ifLayerSizesDiffer") == 1).unwrap_or_else(|| b.img.clone());
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let o = other.px[y * w + x];
        let mixed = match mode {
            0 => o,
            // the other's hue (and saturation), this one's lightness
            1 | 2 => {
                let (c, a) = unpremul(p);
                let (oc, oa) = unpremul(o);
                if a <= 0.0 || oa <= 0.0 {
                    p
                } else {
                    let (_, s, l) = hsl(c);
                    let (oh, os, _) = hsl(oc);
                    let (h2, s2) = if mode == 1 {
                        (oh, os)
                    } else if s > 1e-4 {
                        (oh, s)
                    } else {
                        (0.0, 0.0)
                    };
                    mix4(p, premul(rgb(h2, s2, l), a), oa)
                }
            }
            3 => [p[0].min(o[0]), p[1].min(o[1]), p[2].min(o[2]), p[3]],
            _ => [p[0].max(o[0]), p[1].max(o[1]), p[2].max(o[2]), p[3].max(o[3])],
        };
        mix4(mixed, p, keep)
    });
    b
}

const CALC_MODES: &[&str] = &[
    "normal",
    "darken",
    "multiply",
    "colorBurn",
    "linearBurn",
    "add",
    "lighten",
    "screen",
    "colorDodge",
    "linearDodge",
    "overlay",
    "softLight",
    "hardLight",
    "linearLight",
    "vividLight",
    "pinLight",
    "hardMix",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "luminosity",
];
const CALC_OF: [BlendMode; 22] = [
    BlendMode::Normal,
    BlendMode::Darken,
    BlendMode::Multiply,
    BlendMode::ColorBurn,
    BlendMode::LinearBurn,
    BlendMode::Add,
    BlendMode::Lighten,
    BlendMode::Screen,
    BlendMode::ColorDodge,
    BlendMode::LinearDodge,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::LinearLight,
    BlendMode::VividLight,
    BlendMode::PinLight,
    BlendMode::HardMix,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Luminosity,
];
const CALC_CHANNELS: &[&str] = &["rgba", "gray", "red", "green", "blue", "alpha"];

effect!(
    CALCULATIONS_DEF,
    "calculations",
    "Calculations",
    "a channel of this layer mixed with a channel of another by a blend mode",
    [
        choice("inputChannel", "Input Channel", CALC_CHANNELS, 0),
        flag("invertInput", "Invert Input", false),
        layer("secondLayer", "Second Layer"),
        choice("secondLayerChannel", "Second Layer Channel", CALC_CHANNELS, 0),
        pct("secondLayerOpacity", "Second Layer Opacity", 100.0, 0.0, 100.0),
        flag("invertSecondLayer", "Invert Second Layer", false),
        flag("stretchSecondLayerToFit", "Stretch Second Layer to Fit", true),
        choice("blendingMode", "Blending Mode", CALC_MODES, 0),
        flag("preserveTransparency", "Preserve Transparency", false),
    ]
);
pub const CALCULATIONS: Spec = Spec::new(&CALCULATIONS_DEF, CATEGORY, calculations);

/// A channel of a pixel as a pixel: all of it, or one channel as solid grey.
fn channel_of(p: Px, ch: u32, flip: bool) -> Px {
    let (c, a) = unpremul(p);
    let g = match ch {
        0 => return if flip { premul(c.map(|v| 1.0 - v), a) } else { p },
        1 => luma(c) * a,
        2 => c[0] * a,
        3 => c[1] * a,
        4 => c[2] * a,
        _ => a,
    };
    let g = if flip { 1.0 - g } else { g };
    [g, g, g, 1.0]
}

fn calculations(ctx: &Ctx, mut b: Buf) -> Buf {
    let (in_ch, in_flip) = (ctx.choice("inputChannel"), ctx.flag("invertInput"));
    let (sec_ch, sec_flip) = (ctx.choice("secondLayerChannel"), ctx.flag("invertSecondLayer"));
    let sec_op = (ctx.num("secondLayerOpacity", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let mode = CALC_OF[ctx.choice("blendingMode").min(21) as usize];
    let keep_alpha = ctx.flag("preserveTransparency");
    let other = other_or_self(ctx, &b, "secondLayer", ctx.flag("stretchSecondLayerToFit"));
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let mut o = channel_of(p, in_ch, in_flip);
        if let Some(other) = &other {
            o = veymelo_color::blend(mode, o, channel_of(other.px[y * w + x], sec_ch, sec_flip).map(|v| v * sec_op), 0.5);
        }
        if keep_alpha && o[3] > 1e-6 {
            o = premul(unpremul(o).0, p[3]);
        }
        o[3] = o[3].clamp(0.0, 1.0);
        o
    });
    b
}

const COMPOSITE_MODES: &[&str] = &[
    "inFront",
    "inBack",
    "add",
    "multiply",
    "screen",
    "overlay",
    "softLight",
    "hardLight",
    "darken",
    "lighten",
    "difference",
    "hue",
    "saturation",
    "color",
    "luminosity",
    "stencilAlpha",
];
const COMPOSITE_OF: [BlendMode; 16] = [
    BlendMode::Normal,
    BlendMode::Normal,
    BlendMode::Add,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
    BlendMode::StencilAlpha,
];

effect!(
    COMPOSITE_DEF,
    "composite",
    "Composite",
    "the layer as it was before its effects put back with what the effects above made: in front, behind, or by a blend mode",
    [pct("opacity", "Opacity", 100.0, 0.0, 100.0), choice("compositeOriginal", "Composite Original", COMPOSITE_MODES, 0), flag("rgbOnly", "RGB Only", false).doc("the alpha left as the effects made it"),]
);
pub const COMPOSITE: Spec = Spec::new(&COMPOSITE_DEF, CATEGORY, composite);

fn composite(ctx: &Ctx, mut b: Buf) -> Buf {
    let op = (ctx.num("opacity", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let m = ctx.choice("compositeOriginal").min(15);
    let rgb_only = ctx.flag("rgbOnly");
    // the layer before any effect, laid on this buffer's pixels
    let Some(first) = ctx.host.and_then(|h| h.this_at(ctx.time, 0)) else { return b };
    let w = b.img.width;
    let before: Vec<Px> = (0..b.img.px.len())
        .map(|i| {
            let p = b.origin + veymelo_geom::vec2((i % w) as f64 + 0.5, (i / w) as f64 + 0.5) / b.scale;
            let q = first.pixel(p);
            first.img.sample(veymelo_raster::Sampling::Bilinear, q.x, q.y)
        })
        .collect();
    map_px(&mut b.img, |x, y, now| {
        let was = before[y * w + x].map(|v| v * op);
        let mut o = if m == 1 { veymelo_color::blend(BlendMode::Normal, was, now, 0.5) } else { veymelo_color::blend(COMPOSITE_OF[m as usize], now, was, 0.5) };
        o[3] = o[3].clamp(0.0, 1.0);
        if rgb_only {
            o = premul(unpremul(o).0, now[3]);
        }
        o
    });
    b
}

const COMPOUND_OPERATORS: &[&str] = &["copy", "add", "subtract", "multiply", "difference", "and", "or", "xor", "lighten", "darken", "minimum", "maximum", "screen", "overlay", "hardLight"];

effect!(
    COMPOUND_DEF,
    "compoundArithmetic",
    "Compound Arithmetic",
    "this layer's channels worked with another layer's: added, multiplied, the lighter…",
    [
        layer("secondSourceLayer", "Second Source Layer").doc("none: this layer"),
        choice("operator", "Operator", COMPOUND_OPERATORS, 0),
        choice("operateOnChannels", "Operate On Channels", &["rgb", "argb", "alpha"], 0),
        choice("overflowBehavior", "Overflow Behavior", &["clip", "wrap", "scale"], 0),
        flag("stretchSecondSourceToFit", "Stretch Second Source to Fit", true),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
    ]
);
pub const COMPOUND: Spec = Spec::new(&COMPOUND_DEF, CATEGORY, compound);

fn operate(op: u32, a: f32, b: f32) -> f32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    let overlay = |base: f32, top: f32| if base < 0.5 { 2.0 * base * top } else { 1.0 - 2.0 * (1.0 - base) * (1.0 - top) };
    match op {
        0 => b,
        1 => a + b,
        2 => a - b,
        3 => a * b,
        4 => (a - b).abs(),
        5 => (byte(a) & byte(b)) as f32 / 255.0,
        6 => (byte(a) | byte(b)) as f32 / 255.0,
        7 => (byte(a) ^ byte(b)) as f32 / 255.0,
        8 | 11 => a.max(b),
        9 | 10 => a.min(b),
        12 => a + b - a * b,
        13 => overlay(a, b),
        _ => overlay(b, a),
    }
}

/// A result past 0 to 1 brought back: cut off, wrapped round, or (scale) the operator's whole
/// range fitted to 0 to 1.
fn brought_back(op: u32, v: f32, how: u32) -> f32 {
    match how {
        1 => v.rem_euclid(1.0001).min(1.0),
        2 => match op {
            1 => v * 0.5,
            2 => (v + 1.0) * 0.5,
            _ => v.clamp(0.0, 1.0),
        },
        _ => v.clamp(0.0, 1.0),
    }
}

fn compound(ctx: &Ctx, mut b: Buf) -> Buf {
    let op = ctx.choice("operator");
    let chans = ctx.choice("operateOnChannels");
    let how = ctx.choice("overflowBehavior");
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    if keep >= 1.0 {
        return b;
    }
    let other = other_or_self(ctx, &b, "secondSourceLayer", ctx.flag("stretchSecondSourceToFit")).unwrap_or_else(|| b.img.clone());
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let (c, a) = unpremul(p);
        let (oc, oa) = unpremul(other.px[y * w + x]);
        let rc = if chans != 2 { [0, 1, 2].map(|k| brought_back(op, operate(op, c[k], oc[k]), how)) } else { c };
        let ra = if chans != 0 { brought_back(op, operate(op, a, oa), how) } else { a };
        mix4(premul(rc, ra), p, keep)
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 13] = [INVERT, SET_CHANNELS, SHIFT_CHANNELS, COMBINER, MINIMAX, ARITHMETIC, SOLID, UNMATTE, SET_MATTE, BLEND, CALCULATIONS, COMPOSITE, COMPOUND];

#[cfg(test)]
mod tests;

#[cfg(test)]
mod extreme_tests {
    /// The extremes over each window, by blocks, are the ones found by looking at every pixel of
    /// it: every radius, both ways, past the ends nothing or the end pixel.
    #[test]
    fn line_extremes_are_each_windows() {
        let v: Vec<f32> = (0..97).map(|i| (((i * 37) % 23) as f32 - 9.0) * 0.17).collect();
        for r in [1, 2, 3, 7, 30, 120] {
            for most in [true, false] {
                for outside in [Some(0.0), None, Some(-5.0)] {
                    let got = super::line_extreme(&v, r, most, outside);
                    for (x, g) in got.iter().enumerate() {
                        let window = (x as i64 - r as i64..=x as i64 + r as i64).map(|i| if i >= 0 && (i as usize) < v.len() { v[i as usize] } else { outside.unwrap_or(v[i.clamp(0, v.len() as i64 - 1) as usize]) });
                        let want = if most { window.fold(f32::MIN, f32::max) } else { window.fold(f32::MAX, f32::min) };
                        assert_eq!(*g, want, "r {r} most {most} outside {outside:?} at {x}");
                    }
                }
            }
        }
    }
}
