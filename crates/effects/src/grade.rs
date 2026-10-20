//! Colour correction, part two: curves, Levels with every control showing, corrections the
//! picture works out for itself (Auto Levels, Auto Contrast, Auto Color, Equalize,
//! Shadow/Highlight), Selective Color, Color Balance (HLS), Color Link, Broadcast Colors, and a
//! toner, a colour offset and a 3 × 3 kernel.

use veymelo_color::BlendMode;
use veymelo_keyframe::{Kind, Value};
use veymelo_project::{Body, DefNode, GroupDef, PropDef, Unit};
use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px};
use veymelo_time::Tick;

use crate::color::{Five, SAME, level, rgb3};
use crate::defs::{angle, choice, color, effect, flag, layer, pct, px, ranged};
use crate::util::{hsl, luma, map_px, map_straight, mix3, premul, rgb, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "color";

/// A curve's points, "x,y x,y …" from 0 to 1 (the identity is "0,0 1,1").
const fn curve(id: &'static str, name: &'static str) -> PropDef {
    PropDef::new(id, name, Kind::Text, Value::Text(String::new())).doc("its points, \"x,y x,y …\" from 0 to 1, like \"0,0 0.25,0.15 0.75,0.85 1,1\"")
}

effect!(
    CURVES_DEF,
    "curves",
    "Curves",
    "bends the tones along curves through points: one for all colours, then one for each channel",
    [
        choice("channel", "Channel", &["rgb", "red", "green", "blue", "alpha"], 0).doc("which curve the panel shows (all are kept)"),
        curve("rgb", "RGB"),
        curve("red", "Red"),
        curve("green", "Green"),
        curve("blue", "Blue"),
        curve("alpha", "Alpha"),
    ]
);
pub const CURVES: Spec = Spec::new(&CURVES_DEF, CATEGORY, curves);

/// A curve through points, smooth and never turning back between them (a monotone cubic,
/// Fritsch and Carlson), straight past the first and last.
pub struct Curve {
    xs: Vec<f32>,
    ys: Vec<f32>,
    slopes: Vec<f32>,
    table: Vec<f32>,
}

const TABLE: usize = 1024;

impl Curve {
    /// From "x,y x,y …" (spaces or ; between points). None for the identity or fewer than two.
    pub fn parse(s: &str) -> Option<Curve> {
        let mut pts: Vec<(f32, f32)> = s
            .split(|c: char| c.is_whitespace() || c == ';')
            .filter(|t| !t.is_empty())
            .filter_map(|t| {
                let (x, y) = t.split_once(',')?;
                let (x, y) = (x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?);
                (x.is_finite() && y.is_finite()).then_some((x, y))
            })
            .collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
        if pts.len() < 2 || pts.iter().all(|(x, y)| (x - y).abs() < 1e-6) {
            return None;
        }
        let (xs, ys): (Vec<f32>, Vec<f32>) = pts.into_iter().unzip();
        let n = xs.len();
        let d: Vec<f32> = (0..n - 1).map(|k| (ys[k + 1] - ys[k]) / (xs[k + 1] - xs[k])).collect();
        let mut m = vec![0f32; n];
        m[0] = d[0];
        m[n - 1] = d[n - 2];
        for k in 1..n - 1 {
            m[k] = if d[k - 1] * d[k] <= 0.0 { 0.0 } else { (d[k - 1] + d[k]) * 0.5 };
        }
        // slopes kept small enough that the curve never overshoots between points
        for k in 0..n - 1 {
            if d[k].abs() < 1e-12 {
                m[k] = 0.0;
                m[k + 1] = 0.0;
            } else {
                let (a, b) = (m[k] / d[k], m[k + 1] / d[k]);
                let s = a * a + b * b;
                if s > 9.0 {
                    let t = 3.0 / s.sqrt();
                    m[k] = t * a * d[k];
                    m[k + 1] = t * b * d[k];
                }
            }
        }
        let mut c = Curve { xs, ys, slopes: m, table: Vec::new() };
        c.table = (0..=TABLE).map(|i| c.exact(i as f32 / TABLE as f32)).collect();
        Some(c)
    }

    fn exact(&self, x: f32) -> f32 {
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0] + (x - self.xs[0]) * self.slopes[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1] + (x - self.xs[n - 1]) * self.slopes[n - 1];
        }
        let k = self.xs.partition_point(|v| *v <= x).saturating_sub(1).min(n - 2);
        let h = self.xs[k + 1] - self.xs[k];
        let t = (x - self.xs[k]) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * self.ys[k] + (t3 - 2.0 * t2 + t) * h * self.slopes[k] + (-2.0 * t3 + 3.0 * t2) * self.ys[k + 1] + (t3 - t2) * h * self.slopes[k + 1]
    }

    pub fn at(&self, x: f32) -> f32 {
        if !(0.0..=1.0).contains(&x) {
            return self.exact(x);
        }
        let f = x * TABLE as f32;
        let i = (f as usize).min(TABLE - 1);
        self.table[i] + (self.table[i + 1] - self.table[i]) * (f - i as f32)
    }
}

fn through(c: &Option<Curve>, v: f32) -> f32 {
    c.as_ref().map_or(v, |c| c.at(v))
}

fn curves(ctx: &Ctx, mut b: Buf) -> Buf {
    let text = |id: &str| ctx.value_in(ctx.effect, id).map(|v| v.as_text().to_string()).unwrap_or_default();
    let [all, r, g, bl, a] = ["rgb", "red", "green", "blue", "alpha"].map(|id| Curve::parse(&text(id)));
    if all.is_none() && r.is_none() && g.is_none() && bl.is_none() && a.is_none() {
        return b;
    }
    let chans = [r, g, bl];
    map_px(&mut b.img, |_, _, p| {
        if p[3] <= 0.0 && a.is_none() {
            return p;
        }
        let (c, al) = unpremul(p);
        premul([0, 1, 2].map(|i| through(&chans[i], through(&all, c[i]))), through(&a, al).clamp(0.0, 1.0))
    });
    b
}

const CLIPS: &[&str] = &["off", "on", "offFor32bpc"];

macro_rules! levels_individual_nodes {
    ($( [$p:literal, $name:literal] ),*) => {
        &[
            DefNode::Prop(ranged("inputBlack", "Input Black", 0.0, -1.0, 2.0)),
            DefNode::Prop(ranged("inputWhite", "Input White", 1.0, -1.0, 2.0)),
            DefNode::Prop(ranged("gamma", "Gamma", 1.0, 0.1, 10.0)),
            DefNode::Prop(ranged("outputBlack", "Output Black", 0.0, -1.0, 2.0)),
            DefNode::Prop(ranged("outputWhite", "Output White", 1.0, -1.0, 2.0)),
            $(
                DefNode::Prop(ranged(concat!($p, "InputBlack"), concat!($name, " Input Black"), 0.0, -1.0, 2.0)),
                DefNode::Prop(ranged(concat!($p, "InputWhite"), concat!($name, " Input White"), 1.0, -1.0, 2.0)),
                DefNode::Prop(ranged(concat!($p, "Gamma"), concat!($name, " Gamma"), 1.0, 0.1, 10.0)),
                DefNode::Prop(ranged(concat!($p, "OutputBlack"), concat!($name, " Output Black"), 0.0, -1.0, 2.0)),
                DefNode::Prop(ranged(concat!($p, "OutputWhite"), concat!($name, " Output White"), 1.0, -1.0, 2.0)),
            )*
            DefNode::Prop(choice("clipToOutputBlack", "Clip To Output Black", CLIPS, 2)),
            DefNode::Prop(choice("clipToOutputWhite", "Clip To Output White", CLIPS, 2)),
        ]
    };
}

pub static LEVELS_INDIVIDUAL_DEF: GroupDef = GroupDef {
    id: "levelsIndividual",
    name: "Levels (Individual Controls)",
    body: Body::Fixed(levels_individual_nodes!(["red", "Red"], ["green", "Green"], ["blue", "Blue"], ["alpha", "Alpha"])),
    doc: "Levels with every channel's values side by side: all colours first, then each channel",
};
pub const LEVELS_INDIVIDUAL: Spec = Spec::new(&LEVELS_INDIVIDUAL_DEF, CATEGORY, levels_individual);

fn levels_individual(ctx: &Ctx, mut b: Buf) -> Buf {
    let five = |p: &str| -> Five {
        let names = ["InputBlack", "InputWhite", "Gamma", "OutputBlack", "OutputWhite"];
        let v: Five = std::array::from_fn(|i| {
            let id = if p.is_empty() { names[i][..1].to_lowercase() + &names[i][1..] } else { format!("{p}{}", names[i]) };
            ctx.num(&id, SAME[i] as f64) as f32
        });
        [v[0], v[1], v[2].max(0.01), v[3], v[4]]
    };
    let clip = |id: &str| match ctx.choice(id) {
        0 => false,
        1 => true,
        _ => !ctx.float,
    };
    let clips = (clip("clipToOutputBlack"), clip("clipToOutputWhite"));
    let all = five("");
    let ch = ["red", "green", "blue", "alpha"].map(five);
    if all == SAME && ch.iter().all(|c| *c == SAME) {
        return b;
    }
    let step = |v: f32, f: Five| if f == SAME { v } else { level(v, f, clips) };
    map_px(&mut b.img, |_, _, p| {
        if p[3] <= 0.0 && ch[3] == SAME {
            return p;
        }
        let (c, a) = unpremul(p);
        premul([0, 1, 2].map(|i| step(step(c[i], all), ch[i])), step(a, ch[3]).clamp(0.0, 1.0))
    });
    b
}

/// How finely tones are counted.
const BINS: usize = 1024;

/// How many pixels (by their alpha) have each tone of red, green, blue and brightness.
fn histograms(img: &Image) -> [Vec<f64>; 4] {
    let mut h: [Vec<f64>; 4] = std::array::from_fn(|_| vec![0.0; BINS]);
    let bin = |v: f32| ((v.clamp(0.0, 1.0) * (BINS - 1) as f32).round() as usize).min(BINS - 1);
    for p in &img.px {
        let (c, a) = unpremul(*p);
        if a <= 1e-4 {
            continue;
        }
        for k in 0..3 {
            h[k][bin(c[k])] += a as f64;
        }
        h[3][bin(luma(c))] += a as f64;
    }
    h
}

/// The darkest and lightest tones once `black` and `white` of the pixels (fractions) are left out.
fn ends(h: &[f64], black: f64, white: f64) -> (f32, f32) {
    let total: f64 = h.iter().sum();
    if total <= 0.0 {
        return (0.0, 1.0);
    }
    let first = |iter: &mut dyn Iterator<Item = (usize, &f64)>, frac: f64| {
        let mut acc = 0.0;
        for (i, v) in iter {
            acc += v;
            if acc > total * frac {
                return Some(i);
            }
        }
        None
    };
    let lo = first(&mut h.iter().enumerate(), black).unwrap_or(0);
    let hi = first(&mut h.iter().enumerate().rev(), white).unwrap_or(BINS - 1);
    let (lo, hi) = (lo as f32 / (BINS - 1) as f32, hi as f32 / (BINS - 1) as f32);
    if hi - lo < 1e-3 { (0.0, 1.0) } else { (lo, hi) }
}

/// The mean tone of a histogram, 0 to 1.
fn mean(h: &[f64]) -> f64 {
    let (s, w) = h.iter().enumerate().fold((0.0, 0.0), |(s, w), (i, v)| (s + v * i as f64, w + v));
    if w > 0.0 { s / w / (BINS - 1) as f64 } else { 0.5 }
}

/// The histograms the automatic corrections look at: this frame's, or with temporal smoothing
/// the frames within that many seconds either side too (scene detect stops at a cut).
fn smoothed(ctx: &Ctx, img: &Image) -> [Vec<f64>; 4] {
    let mut h = histograms(img);
    let secs = ctx.num("temporalSmoothing", 0.0);
    let Some(host) = ctx.host.filter(|_| secs > 0.0) else { return h };
    let frame = ctx.frame.seconds().max(1e-6);
    let n = (secs / frame).round().clamp(0.0, 240.0) as i64;
    let scene = ctx.flag("sceneDetect");
    let m0 = mean(&h[3]);
    for dir in [-1i64, 1] {
        for k in 1..=n {
            let Some(other) = host.this_at(ctx.time + Tick::from_seconds((dir * k) as f64 * frame), ctx.index) else { break };
            let hn = histograms(&other.img);
            if scene && (mean(&hn[3]) - m0).abs() > 0.15 {
                break;
            }
            for c in 0..4 {
                for i in 0..BINS {
                    h[c][i] += hn[c][i];
                }
            }
        }
    }
    h
}

fn stretched(v: f32, lo: f32, hi: f32) -> f32 {
    ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
}

macro_rules! auto_effect {
    ($def:ident, $spec:ident, $id:literal, $name:literal, $doc:literal, $kind:expr, [$($extra:expr),*]) => {
        effect!(
            $def,
            $id,
            $name,
            $doc,
            [
                ranged("temporalSmoothing", "Temporal Smoothing (seconds)", 0.0, 0.0, 4.0).doc("look at the frames this many seconds either side too, so the correction does not flicker"),
                flag("sceneDetect", "Scene Detect", false).doc("with smoothing, stop at a cut"),
                ranged("blackClip", "Black Clip", 0.1, 0.0, 49.0).unit(Unit::Percent).doc("how much of the darkest is let go black"),
                ranged("whiteClip", "White Clip", 0.1, 0.0, 49.0).unit(Unit::Percent),
                $($extra,)*
                pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
            ]
        );
        pub const $spec: Spec = Spec::new(&$def, CATEGORY, |ctx, b| auto_correct(ctx, b, $kind));
    };
}

auto_effect!(AUTO_LEVELS_DEF, AUTO_LEVELS, "autoLevels", "Auto Levels", "stretches each channel from its own darkest to its lightest", 0, []);
auto_effect!(AUTO_CONTRAST_DEF, AUTO_CONTRAST, "autoContrast", "Auto Contrast", "stretches all colours together from the darkest to the lightest", 1, []);
auto_effect!(
    AUTO_COLOR_DEF,
    AUTO_COLOR,
    "autoColor",
    "Auto Color",
    "stretches each channel, and evens out a colour cast in the middle tones",
    2,
    [flag("snapNeutralMidtones", "Snap Neutral Midtones", true).doc("make the average colour grey")]
);

fn auto_correct(ctx: &Ctx, mut b: Buf, kind: u32) -> Buf {
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    let (black, white) = (ctx.num("blackClip", 0.1).clamp(0.0, 49.0) / 100.0, ctx.num("whiteClip", 0.1).clamp(0.0, 49.0) / 100.0);
    let h = smoothed(ctx, &b.img);
    let ranges: [(f32, f32); 3] = if kind == 1 {
        let all: Vec<f64> = (0..BINS).map(|i| h[0][i] + h[1][i] + h[2][i]).collect();
        [ends(&all, black, white); 3]
    } else {
        [0, 1, 2].map(|k| ends(&h[k], black, white))
    };
    // a gamma for each channel that brings its mean to the means' mean
    let mut gamma = [1.0f32; 3];
    if kind == 2 && ctx.flag("snapNeutralMidtones") {
        let mut sums = [0f64; 4];
        for p in &b.img.px {
            let (c, a) = unpremul(*p);
            if a > 1e-4 {
                for k in 0..3 {
                    sums[k] += (stretched(c[k], ranges[k].0, ranges[k].1) * a) as f64;
                }
                sums[3] += a as f64;
            }
        }
        if sums[3] > 0.0 {
            let means = [0, 1, 2].map(|k| (sums[k] / sums[3]) as f32);
            let target = (means[0] + means[1] + means[2]) / 3.0;
            for k in 0..3 {
                if means[k] > 1e-3 && means[k] < 0.999 && target > 1e-3 && target < 0.999 {
                    gamma[k] = (target.ln() / means[k].ln()).clamp(0.2, 5.0);
                }
            }
        }
    }
    map_straight(&mut b.img, |c| mix3([0, 1, 2].map(|k| stretched(c[k], ranges[k].0, ranges[k].1).powf(gamma[k])), c, keep));
    b
}

effect!(
    EQUALIZE_DEF,
    "equalize",
    "Equalize",
    "spreads the tones out evenly",
    [choice("equalize", "Equalize", &["rgb", "brightness", "photoshopStyle"], 0), pct("amountToEqualize", "Amount to Equalize", 100.0, 0.0, 100.0)]
);
pub const EQUALIZE: Spec = Spec::new(&EQUALIZE_DEF, CATEGORY, equalize);

/// The share of pixels at or below each tone.
fn running(h: &[f64]) -> Vec<f32> {
    let total = h.iter().sum::<f64>().max(1e-12);
    let mut acc = 0.0;
    h.iter()
        .map(|v| {
            acc += v;
            (acc / total) as f32
        })
        .collect()
}

fn equalize(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = (ctx.num("amountToEqualize", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    if k <= 0.0 {
        return b;
    }
    let h = histograms(&b.img);
    let look = |t: &[f32], v: f32| t[((v.clamp(0.0, 1.0) * (BINS - 1) as f32).round() as usize).min(BINS - 1)];
    match ctx.choice("equalize") {
        1 => {
            let t = running(&h[3]);
            map_straight(&mut b.img, |c| {
                let l = luma(c);
                let nl = look(&t, l);
                mix3(c, if l > 1e-5 { c.map(|v| v * nl / l) } else { [nl; 3] }, k)
            });
        }
        2 => {
            let t = running(&(0..BINS).map(|i| h[0][i] + h[1][i] + h[2][i]).collect::<Vec<_>>());
            map_straight(&mut b.img, |c| mix3(c, c.map(|v| look(&t, v)), k));
        }
        _ => {
            let t = [running(&h[0]), running(&h[1]), running(&h[2])];
            map_straight(&mut b.img, |c| mix3(c, [0, 1, 2].map(|i| look(&t[i], c[i])), k));
        }
    }
    b
}

effect!(
    SHADOW_HIGHLIGHT_DEF,
    "shadowHighlight",
    "Shadow/Highlight",
    "brings out what is in the shadows and pulls back blown highlights, by each area's own brightness",
    [
        flag("autoAmounts", "Auto Amounts", true).doc("work out the amounts from the picture"),
        pct("shadowAmount", "Shadow Amount", 50.0, 0.0, 100.0),
        pct("highlightAmount", "Highlight Amount", 0.0, 0.0, 100.0),
        ranged("temporalSmoothing", "Temporal Smoothing (seconds)", 0.0, 0.0, 4.0),
        flag("sceneDetect", "Scene Detect", false),
        pct("shadowTonalWidth", "Shadow Tonal Width", 50.0, 0.0, 100.0).doc("how far up the tones counts as shadow"),
        px("shadowRadius", "Shadow Radius", 30.0, 2500.0).doc("how big an area is judged as one"),
        pct("highlightTonalWidth", "Highlight Tonal Width", 50.0, 0.0, 100.0),
        px("highlightRadius", "Highlight Radius", 30.0, 2500.0),
        pct("colorCorrection", "Color Correction", 20.0, -100.0, 100.0).doc("how much the colour follows the change in brightness"),
        pct("midtoneContrast", "Midtone Contrast", 0.0, -100.0, 100.0),
        ranged("blackClip", "Black Clip", 0.01, 0.0, 49.0).unit(Unit::Percent),
        ranged("whiteClip", "White Clip", 0.01, 0.0, 49.0).unit(Unit::Percent),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
    ]
);
pub const SHADOW_HIGHLIGHT: Spec = Spec::new(&SHADOW_HIGHLIGHT_DEF, CATEGORY, shadow_highlight);

fn shadow_highlight(ctx: &Ctx, mut b: Buf) -> Buf {
    let (mut s_amt, mut h_amt) = (ctx.num("shadowAmount", 50.0) as f32 / 100.0, ctx.num("highlightAmount", 0.0) as f32 / 100.0);
    if ctx.flag("autoAmounts") {
        let m = mean(&smoothed(ctx, &b.img)[3]) as f32;
        s_amt = ((0.6 - m) * 1.5).clamp(0.0, 1.0) * 0.8 + 0.1;
        h_amt = ((m - 0.6) * 1.5).clamp(0.0, 1.0) * 0.8;
    }
    let (bc, wc) = (ctx.num("blackClip", 0.01).clamp(0.0, 49.0) / 100.0, ctx.num("whiteClip", 0.01).clamp(0.0, 49.0) / 100.0);
    let before = (bc > 0.0 || wc > 0.0).then(|| ends(&histograms(&b.img)[3], bc, wc));
    let (s_tw, h_tw) = ((ctx.num("shadowTonalWidth", 50.0) as f32 / 100.0).max(0.01), (ctx.num("highlightTonalWidth", 50.0) as f32 / 100.0).max(0.01));
    let (s_r, h_r) = (ctx.num("shadowRadius", 30.0).max(0.0) * b.scale / 2.0, ctx.num("highlightRadius", 30.0).max(0.0) * b.scale / 2.0);
    let (cc, mc) = (ctx.num("colorCorrection", 20.0) as f32 / 100.0, ctx.num("midtoneContrast", 0.0) as f32 / 100.0);
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    // each area's brightness: the picture's, blurred
    let mut bright = b.img.clone();
    map_px(&mut bright, |_, _, p| {
        let l = luma(unpremul(p).0);
        [l, l, l, 1.0]
    });
    let around_s = filter::gaussian(&bright, s_r, s_r, Edge::Repeat);
    let around_h = if (h_r - s_r).abs() < 1e-9 { around_s.clone() } else { filter::gaussian(&bright, h_r, h_r, Edge::Repeat) };
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let (c, a) = unpremul(p);
        if a <= 0.0 {
            return p;
        }
        let i = y * w + x;
        let ws = (1.0 - around_s.px[i][0] / s_tw).clamp(0.0, 1.0).powi(2);
        let wh = ((around_h.px[i][0] - (1.0 - h_tw)) / h_tw).clamp(0.0, 1.0).powi(2);
        let gain = (1.0 + s_amt * ws * 2.0) * (1.0 - h_amt * wh * 0.5);
        let l = luma(c);
        let nl = l * gain;
        let sat = gain.max(1e-3).powf(cc);
        let mut o = c.map(|v| nl + (v - l) * sat);
        if mc != 0.0 {
            let shift = (0.5 + (nl - 0.5) * (1.0 + mc)) - nl;
            o = o.map(|v| v + shift);
        }
        premul(mix3(o.map(|v| v.max(0.0)), c, keep), a)
    });
    // the ends of the tones put back where they were, so lifting shadows does not flatten them
    if let Some((lo0, hi0)) = before {
        let (lo1, hi1) = ends(&histograms(&b.img)[3], bc, wc);
        if (lo0, hi0) != (lo1, hi1) && hi1 - lo1 > 1e-3 {
            let k = (hi0 - lo0) / (hi1 - lo1);
            map_straight(&mut b.img, |c| c.map(|v| (lo0 + (v - lo1) * k).max(0.0)));
        }
    }
    b
}

/// Selective Color's ranges and their four inks.
macro_rules! selective_nodes {
    ($( [$p:literal, $name:literal] ),*) => {
        &[
            DefNode::Prop(choice("method", "Method", &["relative", "absolute"], 0).doc("relative: by how much of each ink there is already")),
            DefNode::Prop(choice("colors", "Colors", &["reds", "yellows", "greens", "cyans", "blues", "magentas", "whites", "neutrals", "blacks"], 0).doc("which range the panel shows (all are kept)")),
            $(
                DefNode::Prop(pct(concat!($p, "Cyan"), concat!($name, " Cyan"), 0.0, -100.0, 100.0)),
                DefNode::Prop(pct(concat!($p, "Magenta"), concat!($name, " Magenta"), 0.0, -100.0, 100.0)),
                DefNode::Prop(pct(concat!($p, "Yellow"), concat!($name, " Yellow"), 0.0, -100.0, 100.0)),
                DefNode::Prop(pct(concat!($p, "Black"), concat!($name, " Black"), 0.0, -100.0, 100.0)),
            )*
        ]
    };
}

const SELECTIVE_RANGES: [&str; 9] = ["reds", "yellows", "greens", "cyans", "blues", "magentas", "whites", "neutrals", "blacks"];

pub static SELECTIVE_COLOR_DEF: GroupDef = GroupDef {
    id: "selectiveColor",
    name: "Selective Color",
    body: Body::Fixed(selective_nodes!(
        ["reds", "Reds"],
        ["yellows", "Yellows"],
        ["greens", "Greens"],
        ["cyans", "Cyans"],
        ["blues", "Blues"],
        ["magentas", "Magentas"],
        ["whites", "Whites"],
        ["neutrals", "Neutrals"],
        ["blacks", "Blacks"]
    )),
    doc: "adds or takes away cyan, magenta, yellow and black inks in one range of colours at a time",
};
pub const SELECTIVE_COLOR: Spec = Spec::new(&SELECTIVE_COLOR_DEF, CATEGORY, selective_color);

/// How much a colour is in each range: the largest channel's lead for the primaries, the
/// smallest's lack for the secondaries, light, middling and dark for the neutrals.
fn selective_weights(c: [f32; 3]) -> [f32; 9] {
    let c = c.map(|v| v.clamp(0.0, 1.0));
    let mut order = [0usize, 1, 2];
    order.sort_by(|&a, &b| c[b].total_cmp(&c[a]));
    let (mx, md, mn) = (c[order[0]], c[order[1]], c[order[2]]);
    let mut w = [0f32; 9];
    w[[0, 2, 4][order[0]]] = mx - md;
    w[[3, 5, 1][order[2]]] = md - mn;
    w[6] = ((mn - 0.5) * 2.0).max(0.0);
    w[8] = ((0.5 - mx) * 2.0).max(0.0);
    w[7] = (1.0 - ((mx - 0.5).abs() + (mn - 0.5).abs())).clamp(0.0, 1.0);
    w
}

fn selective_color(ctx: &Ctx, mut b: Buf) -> Buf {
    let adj: [[f32; 4]; 9] = SELECTIVE_RANGES.map(|r| ["Cyan", "Magenta", "Yellow", "Black"].map(|k| ctx.num(&format!("{r}{k}"), 0.0) as f32 / 100.0));
    if adj.iter().all(|r| r.iter().all(|v| *v == 0.0)) {
        return b;
    }
    let relative = ctx.choice("method") == 0;
    map_straight(&mut b.img, |c| {
        let w = selective_weights(c);
        let mut o = c;
        for (r, wr) in w.iter().enumerate() {
            if *wr <= 0.0 {
                continue;
            }
            // more cyan is less red, more magenta less green, more yellow less blue; black darkens all
            for i in 0..3 {
                let more = adj[r][i] + adj[r][3];
                o[i] -= wr * if relative { more * (1.0 - c[i]).max(0.0) } else { more };
            }
        }
        o.map(|v| v.max(0.0))
    });
    b
}

effect!(
    COLOR_BALANCE_HLS_DEF,
    "colorBalanceHls",
    "Color Balance (HLS)",
    "turns the hue and changes lightness and saturation of everything",
    [angle("hue", "Hue", 0.0), pct("lightness", "Lightness", 0.0, -100.0, 100.0), pct("saturation", "Saturation", 0.0, -100.0, 100.0)]
);
pub const COLOR_BALANCE_HLS: Spec = Spec::new(&COLOR_BALANCE_HLS_DEF, CATEGORY, color_balance_hls);

fn color_balance_hls(ctx: &Ctx, mut b: Buf) -> Buf {
    let (dh, dl, ds) = (ctx.num("hue", 0.0) as f32 / 360.0, ctx.num("lightness", 0.0) as f32 / 100.0, ctx.num("saturation", 0.0) as f32 / 100.0);
    if dh == 0.0 && dl == 0.0 && ds == 0.0 {
        return b;
    }
    map_straight(&mut b.img, |c| {
        let (h, s, l) = hsl(c);
        let l = if dl > 0.0 { l + (1.0 - l) * dl } else { l + l * dl };
        let s = if ds > 0.0 { s + (1.0 - s) * ds } else { s + s * ds };
        rgb(h + dh, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0))
    });
    b
}

const LINK_MODES: [(&str, BlendMode); 13] = [
    ("normal", BlendMode::Normal),
    ("add", BlendMode::Add),
    ("multiply", BlendMode::Multiply),
    ("screen", BlendMode::Screen),
    ("overlay", BlendMode::Overlay),
    ("softLight", BlendMode::SoftLight),
    ("hardLight", BlendMode::HardLight),
    ("darken", BlendMode::Darken),
    ("lighten", BlendMode::Lighten),
    ("difference", BlendMode::Difference),
    ("hue", BlendMode::Hue),
    ("color", BlendMode::Color),
    ("luminosity", BlendMode::Luminosity),
];
const LINK_MODE_NAMES: [&str; 13] = {
    let mut out = [""; 13];
    let mut i = 0;
    while i < 13 {
        out[i] = LINK_MODES[i].0;
        i += 1;
    }
    out
};
const SAMPLES: &[&str] = &["average", "median", "brightest", "darkest", "maxRgb", "minRgb", "averageAlpha", "medianAlpha", "maxAlpha", "minAlpha"];

effect!(
    COLOR_LINK_DEF,
    "colorLink",
    "Color Link",
    "colours the layer with a colour taken from another layer (or itself): its average, middle, brightest…",
    [
        layer("sourceLayer", "Source Layer").doc("the layer the colour is taken from (none: this one); taken as it shows, before its masks and effects"),
        choice("sample", "Sample", SAMPLES, 0),
        pct("clip", "Clip", 0.0, 0.0, 100.0).doc("leave out this much at each extreme"),
        flag("stencilOriginalAlpha", "Stencil Original Alpha", true).doc("keep the layer's own alpha"),
        pct("opacity", "Opacity", 100.0, 0.0, 100.0),
        choice("blendingMode", "Blending Mode", &LINK_MODE_NAMES, 0),
    ]
);
pub const COLOR_LINK: Spec = Spec::new(&COLOR_LINK_DEF, CATEGORY, color_link);

/// The value at fraction `q` of a sorted list.
fn quantile(v: &[f32], q: f64) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v[((q * (v.len() - 1) as f64).round() as usize).min(v.len() - 1)]
}

/// The mean of a list without `clip` of it at each end.
fn trimmed_mean(v: &mut [f32], clip: f64) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    let cut = (v.len() as f64 * clip) as usize;
    let s = &v[cut..v.len() - cut];
    if s.is_empty() { quantile(v, 0.5) } else { (s.iter().map(|x| *x as f64).sum::<f64>() / s.len() as f64) as f32 }
}

/// A picture's colour (and alpha) by one of the ways to sample, leaving out `clip` at each end.
fn sample_of(img: &Image, how: u32, clip: f64) -> ([f32; 3], f32) {
    let clip = clip.clamp(0.0, 0.49);
    let alpha_only = how >= 6;
    let mut ch: [Vec<f32>; 3] = Default::default();
    // each colour with its brightness, to find the brightest and darkest
    let mut bright: Vec<(f32, [f32; 3])> = Vec::new();
    let mut alphas: Vec<f32> = Vec::new();
    let mut sums = [0f64; 4];
    for p in &img.px {
        let (c, a) = unpremul(*p);
        alphas.push(a);
        if alpha_only || a <= 1e-3 {
            continue;
        }
        for k in 0..3 {
            ch[k].push(c[k]);
            sums[k] += (c[k] * a) as f64;
        }
        sums[3] += a as f64;
        bright.push((luma(c), c));
    }
    if alpha_only {
        alphas.sort_by(f32::total_cmp);
        let a = match how {
            7 => quantile(&alphas, 0.5),
            8 => quantile(&alphas, 1.0 - clip),
            9 => quantile(&alphas, clip),
            _ => trimmed_mean(&mut alphas, clip),
        };
        return ([0.0; 3], a);
    }
    if bright.is_empty() {
        return ([0.0; 3], 1.0);
    }
    let c = match how {
        1 => ch.each_mut().map(|v| {
            v.sort_by(f32::total_cmp);
            quantile(v, 0.5)
        }),
        2 | 3 => {
            bright.sort_by(|a, b| a.0.total_cmp(&b.0));
            let q = if how == 2 { 1.0 - clip } else { clip };
            bright[((q * (bright.len() - 1) as f64).round() as usize).min(bright.len() - 1)].1
        }
        4 | 5 => ch.each_mut().map(|v| {
            v.sort_by(f32::total_cmp);
            quantile(v, if how == 4 { 1.0 - clip } else { clip })
        }),
        // a plain average counts each pixel by its alpha
        0 if clip <= 0.0 => [0, 1, 2].map(|k| (sums[k] / sums[3]) as f32),
        _ => ch.each_mut().map(|v| trimmed_mean(v, clip)),
    };
    (c, 1.0)
}

fn color_link(ctx: &Ctx, mut b: Buf) -> Buf {
    let how = ctx.choice("sample");
    let clip = ctx.num("clip", 0.0) / 100.0;
    let source = ctx.layer_over("sourceLayer", &b, Take::Source, true);
    let (c, sa) = sample_of(source.as_ref().unwrap_or(&b.img), how, clip);
    let op = (ctx.num("opacity", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let stencil = ctx.flag("stencilOriginalAlpha");
    if how >= 6 {
        // an alpha sample sets the layer's alpha
        map_px(&mut b.img, |_, _, p| {
            let (c0, a0) = unpremul(p);
            let mut na = a0 + (sa - a0) * op;
            if stencil {
                na = na.min(a0);
            }
            premul(c0, na.clamp(0.0, 1.0))
        });
        return b;
    }
    let mode = LINK_MODES[(ctx.choice("blendingMode") as usize).min(LINK_MODES.len() - 1)].1;
    let src: Px = [c[0] * op, c[1] * op, c[2] * op, op];
    map_px(&mut b.img, |_, _, p| {
        let mut o = veymelo_color::blend(mode, p, src, 0.5);
        o[3] = o[3].clamp(0.0, 1.0);
        if stencil {
            o = premul(unpremul(o).0, p[3]);
        }
        o
    });
    b
}

effect!(
    BROADCAST_DEF,
    "broadcastColors",
    "Broadcast Colors",
    "keeps colours within what television can show",
    [
        choice("broadcastLocale", "Broadcast Locale", &["ntsc", "pal"], 0),
        choice("howToMakeColorSafe", "How To Make Color Safe", &["reduceLuminance", "reduceSaturation", "keyOutUnsafe", "keyOutSafe"], 0),
        ranged("maximumSignal", "Maximum Signal Amplitude (IRE)", 110.0, 90.0, 120.0),
    ]
);
pub const BROADCAST: Spec = Spec::new(&BROADCAST_DEF, CATEGORY, broadcast);

fn broadcast(ctx: &Ctx, mut b: Buf) -> Buf {
    let pal = ctx.choice("broadcastLocale") == 1;
    let how = ctx.choice("howToMakeColorSafe");
    let (setup, span) = if pal { (0.0, 100.0) } else { (7.5, 92.5) };
    let most = (ctx.num("maximumSignal", 110.0) as f32 - setup) / span;
    // the signal's brightness and the swing of its colour on top
    let signal = |c: [f32; 3]| -> (f32, f32) {
        let y = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
        let swing = if pal {
            let (u, v) = (0.492 * (c[2] - y), 0.877 * (c[0] - y));
            u.hypot(v)
        } else {
            let (i, q) = (0.596 * c[0] - 0.274 * c[1] - 0.322 * c[2], 0.211 * c[0] - 0.523 * c[1] + 0.312 * c[2]);
            i.hypot(q)
        };
        (y, swing)
    };
    map_px(&mut b.img, |_, _, p| {
        let (c, a) = unpremul(p);
        if a <= 0.0 {
            return p;
        }
        let (y, swing) = signal(c);
        let unsafe_ = y + swing > most + 1e-5;
        match how {
            2 if unsafe_ => [0.0; 4],
            3 if !unsafe_ => [0.0; 4],
            1 if unsafe_ => {
                let s = if swing > 1e-6 { (most - y).max(0.0) / swing } else { 1.0 };
                let mut o = c.map(|v| y + (v - y) * s);
                if y > most {
                    o = o.map(|v| v * most / y);
                }
                premul(o, a)
            }
            0 if unsafe_ => premul(c.map(|v| v * most / (y + swing)), a),
            _ => p,
        }
    });
    b
}

effect!(
    TONER_DEF,
    "toner",
    "Toner",
    "maps the tones to two, three or five colours",
    [
        choice("tones", "Tones", &["duotone", "tritone", "pentone"], 1),
        color("highlights", "Highlights", 1.0, 1.0, 1.0),
        color("brights", "Brights", 0.86, 0.78, 0.64),
        color("midtones", "Midtones", 0.55, 0.43, 0.28),
        color("darktones", "Darktones", 0.26, 0.18, 0.1),
        color("shadows", "Shadows", 0.0, 0.0, 0.0),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
    ]
);
pub const TONER: Spec = Spec::new(&TONER_DEF, CATEGORY, toner);

fn toner(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = |id: &str| rgb3(ctx.color(id));
    let stops: Vec<[f32; 3]> = match ctx.choice("tones") {
        0 => vec![c("shadows"), c("highlights")],
        2 => vec![c("shadows"), c("darktones"), c("midtones"), c("brights"), c("highlights")],
        _ => vec![c("shadows"), c("midtones"), c("highlights")],
    };
    let keep = (ctx.num("blendWithOriginal", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    let n = stops.len() - 1;
    map_straight(&mut b.img, |col| {
        let t = luma(col).clamp(0.0, 1.0) * n as f32;
        let k = (t.floor() as usize).min(n - 1);
        mix3(mix3(stops[k], stops[k + 1], t - k as f32), col, keep)
    });
    b
}

effect!(
    COLOR_OFFSET_DEF,
    "colorOffset",
    "Color Offset",
    "turns each channel round, as if on a wheel",
    [angle("redPhase", "Red Phase", 0.0), angle("greenPhase", "Green Phase", 0.0), angle("bluePhase", "Blue Phase", 0.0), choice("overflow", "Overflow", &["wrap", "solarize", "polarize"], 0)]
);
pub const COLOR_OFFSET: Spec = Spec::new(&COLOR_OFFSET_DEF, CATEGORY, color_offset);

fn color_offset(ctx: &Ctx, mut b: Buf) -> Buf {
    let offs = ["redPhase", "greenPhase", "bluePhase"].map(|id| (ctx.num(id, 0.0) / 360.0).rem_euclid(1.0) as f32);
    if offs.iter().all(|o| *o == 0.0) {
        return b;
    }
    let overflow = ctx.choice("overflow");
    map_straight(&mut b.img, |c| {
        [0, 1, 2].map(|k| {
            let x = c[k] + offs[k];
            if offs[k] == 0.0 || x <= 1.0 {
                return if offs[k] == 0.0 { c[k] } else { x };
            }
            match overflow {
                1 => (2.0 - x).max(0.0),
                2 => {
                    if x - 1.0 < 0.5 {
                        0.0
                    } else {
                        1.0
                    }
                }
                _ => x.rem_euclid(1.0),
            }
        })
    });
    b
}

const KERNEL_NAMES: [&str; 9] = ["topLeft", "top", "topRight", "left", "center", "right", "bottomLeft", "bottom", "bottomRight"];

effect!(
    KERNEL_DEF,
    "kernel",
    "Kernel",
    "each pixel made from itself and its eight neighbours, by weights (blur, sharpen, emboss…)",
    [
        ranged("topLeft", "Top Left", 0.0, -100.0, 100.0),
        ranged("top", "Top", 0.0, -100.0, 100.0),
        ranged("topRight", "Top Right", 0.0, -100.0, 100.0),
        ranged("left", "Left", 0.0, -100.0, 100.0),
        ranged("center", "Center", 1.0, -100.0, 100.0),
        ranged("right", "Right", 0.0, -100.0, 100.0),
        ranged("bottomLeft", "Bottom Left", 0.0, -100.0, 100.0),
        ranged("bottom", "Bottom", 0.0, -100.0, 100.0),
        ranged("bottomRight", "Bottom Right", 0.0, -100.0, 100.0),
        ranged("scale", "Scale", 1.0, -100.0, 100.0),
        ranged("offset", "Offset", 0.0, -1.0, 1.0),
        flag("absoluteValue", "Absolute Value", false),
    ]
);
pub const KERNEL: Spec = Spec::new(&KERNEL_DEF, CATEGORY, kernel);

fn kernel(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = KERNEL_NAMES.map(|id| ctx.num(id, if id == "center" { 1.0 } else { 0.0 }) as f32);
    let (scale, offset, abs) = (ctx.num("scale", 1.0) as f32, ctx.num("offset", 0.0) as f32, ctx.flag("absoluteValue"));
    let same = k.iter().enumerate().all(|(i, v)| *v == if i == 4 { 1.0 } else { 0.0 });
    if same && scale == 1.0 && offset == 0.0 {
        return b;
    }
    let src = b.img.clone();
    let (w, h) = (src.width as i64, src.height as i64);
    map_px(&mut b.img, |x, y, p| {
        if p[3] <= 0.0 {
            return p;
        }
        let mut acc = [0f32; 3];
        for j in 0..3i64 {
            for i in 0..3i64 {
                let kw = k[(j * 3 + i) as usize];
                if kw == 0.0 {
                    continue;
                }
                let q = src.px[((y as i64 + j - 1).clamp(0, h - 1) * w + (x as i64 + i - 1).clamp(0, w - 1)) as usize];
                let (c, _) = unpremul(q);
                for ch in 0..3 {
                    acc[ch] += c[ch] * kw;
                }
            }
        }
        premul(
            acc.map(|v| {
                let v = v * scale + offset;
                (if abs { v.abs() } else { v }).max(0.0)
            }),
            p[3],
        )
    });
    b
}

effect!(
    CHANGE_COLOR_DEF,
    "changeColor",
    "Change Color",
    "turns the hue, and changes the lightness and saturation, of one colour and those near it",
    [
        choice("view", "View", &["correctedLayer", "colorCorrectionMask"], 0).doc("the result, or what is changed in white"),
        ranged("hueTransform", "Hue Transform", 0.0, -360.0, 360.0).unit(Unit::Degrees),
        pct("lightnessTransform", "Lightness Transform", 0.0, -100.0, 100.0),
        pct("saturationTransform", "Saturation Transform", 0.0, -100.0, 100.0),
        color("colorToChange", "Color To Change", 0.8, 0.2, 0.2),
        pct("matchingTolerance", "Matching Tolerance", 15.0, 0.0, 100.0),
        pct("matchingSoftness", "Matching Softness", 0.0, 0.0, 100.0),
        choice("matchColors", "Match Colors", &["usingRgb", "usingHue", "usingChroma"], 1),
        flag("invertColorCorrectionMask", "Invert Color Correction Mask", false),
    ]
);
pub const CHANGE_COLOR: Spec = Spec::new(&CHANGE_COLOR_DEF, CATEGORY, change_color);

/// How much colour `c` is like `key` (1 the same), by RGB, hue or chroma.
fn alike(c: [f32; 3], key: [f32; 3], using: u32, tol: f32, soft: f32) -> f32 {
    let d = match using {
        0 => ((c[0] - key[0]).powi(2) + (c[1] - key[1]).powi(2) + (c[2] - key[2]).powi(2)).sqrt() / 3f32.sqrt(),
        1 => {
            let ((h1, s1, _), (h2, _, _)) = (hsl(c), hsl(key));
            // greys have no hue to go by
            if s1 < 0.02 { 1.0 } else { ((h1 - h2 + 0.5).rem_euclid(1.0) - 0.5).abs() * 2.0 }
        }
        _ => {
            let chroma = |c: [f32; 3]| {
                let s = (c[0] + c[1] + c[2]).max(1e-6);
                [c[0] / s, c[1] / s]
            };
            let (a, k) = (chroma(c), chroma(key));
            (a[0] - k[0]).hypot(a[1] - k[1]) * 1.5
        }
    };
    if d <= tol {
        1.0
    } else if soft <= 1e-6 {
        0.0
    } else {
        (1.0 - (d - tol) / soft).clamp(0.0, 1.0)
    }
}

fn change_color(ctx: &Ctx, mut b: Buf) -> Buf {
    let mask = ctx.choice("view") == 1;
    let (dh, dl, ds) = (ctx.num("hueTransform", 0.0) as f32 / 360.0, ctx.num("lightnessTransform", 0.0) as f32 / 100.0, ctx.num("saturationTransform", 0.0) as f32 / 100.0);
    if !mask && dh == 0.0 && dl == 0.0 && ds == 0.0 {
        return b;
    }
    let key = rgb3(ctx.color("colorToChange"));
    let (tol, soft) = (ctx.num("matchingTolerance", 15.0) as f32 / 100.0, ctx.num("matchingSoftness", 0.0) as f32 / 100.0);
    let using = ctx.choice("matchColors");
    let invert = ctx.flag("invertColorCorrectionMask");
    map_straight(&mut b.img, |c| {
        let m = alike(c, key, using, tol, soft);
        let m = if invert { 1.0 - m } else { m };
        if mask {
            return [m; 3];
        }
        if m <= 0.0 {
            return c;
        }
        let (h, s, l) = hsl(c);
        let s2 = (s + ds * if ds > 0.0 { 1.0 - s } else { s }).clamp(0.0, 1.0);
        let l2 = (l + dl * if dl > 0.0 { 1.0 - l } else { l }).clamp(0.0, 1.0);
        mix3(c, rgb((h + dh).rem_euclid(1.0), s2, l2), m)
    });
    b
}

const CLIP_LEVELS: [f32; 5] = [0.9, 0.95, 1.0, 1.05, 1.09];

effect!(
    VIDEO_LIMITER_DEF,
    "videoLimiter",
    "Video Limiter",
    "keeps brightness and colour within a broadcast limit, easing into it",
    [
        choice("clipLevel", "Clip Level", &["90", "95", "100", "105", "109"], 2).doc("the most, in % of white"),
        choice("clipMethod", "Clip Method", &["luma", "chroma", "smartLimit"], 2),
        choice("compressionBeforeClipping", "Compression before Clipping", &["none", "3", "5", "10", "20"], 2).doc("how far below the limit (in %) it starts to ease in"),
        flag("gamutWarning", "Gamut Warning", false).doc("show what was changed in the warning colour"),
        color("gamutWarningColor", "Gamut Warning Color", 1.0, 0.0, 0.0),
    ]
);
pub const VIDEO_LIMITER: Spec = Spec::new(&VIDEO_LIMITER_DEF, CATEGORY, video_limiter);

/// Past `start`, a value eases toward `limit` instead of going on.
fn knee(v: f32, start: f32, limit: f32) -> f32 {
    if v <= start || limit <= start {
        return v.min(limit);
    }
    let r = limit - start;
    start + r * (1.0 - (-(v - start) / r).exp())
}

fn video_limiter(ctx: &Ctx, mut b: Buf) -> Buf {
    let level = CLIP_LEVELS[(ctx.choice("clipLevel") as usize).min(4)];
    let method = ctx.choice("clipMethod");
    let ease = [0.0, 0.03, 0.05, 0.1, 0.2][(ctx.choice("compressionBeforeClipping") as usize).min(4)];
    let warn = ctx.flag("gamutWarning").then(|| rgb3(ctx.color("gamutWarningColor")));
    let start = level * (1.0 - ease);
    map_straight(&mut b.img, |c| {
        let y = luma(c);
        let mut o = c;
        if method != 1 {
            // brightness eased in, the colour on top kept
            let y2 = knee(y.max(0.0), start, level);
            o = o.map(|v| v - y + y2);
        }
        if method != 0 {
            // colour drawn toward its grey until every channel is within 0 and the limit
            let yl = luma(o);
            let mut t = 1.0f32;
            for v in o {
                if v > level && v - yl > 1e-6 {
                    t = t.min((level - yl) / (v - yl));
                }
                if v < 0.0 && yl - v > 1e-6 {
                    t = t.min(yl / (yl - v));
                }
            }
            let t = t.clamp(0.0, 1.0);
            o = o.map(|v| yl + (v - yl) * t);
        }
        let o = o.map(|v| v.clamp(0.0, level));
        match warn {
            Some(w) if c.iter().zip(&o).any(|(a, b)| (a - b).abs() > 1e-4) => w,
            _ => o,
        }
    });
    b
}

effect!(
    NEUTRALIZER_DEF,
    "colorNeutralizer",
    "Color Neutralizer",
    "takes a colour cast out: what should be neutral in the shadows, midtones and highlights is moved to where it should be",
    [
        color("shadowsUnbalance", "Shadows Unbalance", 0.0, 0.0, 0.0).doc("the colour the shadows have"),
        color("shadowsBalance", "Shadows Balance", 0.0, 0.0, 0.0).doc("the colour they should have"),
        color("midtonesUnbalance", "Midtones Unbalance", 0.5, 0.5, 0.5),
        color("midtonesBalance", "Midtones Balance", 0.5, 0.5, 0.5),
        color("highlightsUnbalance", "Highlights Unbalance", 1.0, 1.0, 1.0),
        color("highlightsBalance", "Highlights Balance", 1.0, 1.0, 1.0),
        pct("pinning", "Pinning", 0.0, 0.0, 100.0).doc("keeps pure black and white where they are"),
        pct("blendWithOriginal", "Blend With Original", 0.0, 0.0, 100.0),
        pct("darks", "Darks", 0.0, -100.0, 100.0),
        pct("brights", "Brights", 0.0, -100.0, 100.0),
        pct("contrast", "Contrast", 0.0, -100.0, 100.0),
    ]
);
pub const NEUTRALIZER: Spec = Spec::new(&NEUTRALIZER_DEF, CATEGORY, neutralizer);

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn neutralizer(ctx: &Ctx, mut b: Buf) -> Buf {
    let shift = |u: &str, v: &str| {
        let (u, v) = (rgb3(ctx.color(u)), rgb3(ctx.color(v)));
        [v[0] - u[0], v[1] - u[1], v[2] - u[2]]
    };
    let off = [shift("shadowsUnbalance", "shadowsBalance"), shift("midtonesUnbalance", "midtonesBalance"), shift("highlightsUnbalance", "highlightsBalance")];
    let pin = ctx.num("pinning", 0.0) as f32 / 100.0;
    let keep = ctx.num("blendWithOriginal", 0.0) as f32 / 100.0;
    let (contrast, darks, brights) = (ctx.num("contrast", 0.0) as f32 / 100.0, ctx.num("darks", 0.0) as f32 / 100.0, ctx.num("brights", 0.0) as f32 / 100.0);
    map_straight(&mut b.img, |c| {
        let l = luma(c).clamp(0.0, 1.0);
        let (ws, wh) = (1.0 - smoothstep(0.0, 0.5, l), smoothstep(0.5, 1.0, l));
        let wm = 1.0 - ws - wh;
        let held = 1.0 - pin * (1.0 - 4.0 * l * (1.0 - l)).clamp(0.0, 1.0);
        let o = [0, 1, 2].map(|k| {
            let v = c[k] + (off[0][k] * ws + off[1][k] * wm + off[2][k] * wh) * held;
            let v = 0.5 + (v - 0.5) * (1.0 + contrast);
            (v + darks * 0.25 * (1.0 - l).powi(2) + brights * 0.25 * l * l).max(0.0)
        });
        mix3(o, c, keep)
    });
    b
}

effect!(
    ARBITRARY_MAP_DEF,
    "arbitraryMap",
    "PS Arbitrary Map",
    "maps each channel's tones through a table of 256 values",
    [
        ranged("phase", "Phase", 0.0, -1000.0, 1000.0).doc("turns the table round, in steps of 1/255"),
        flag("applyPhaseMapToAlpha", "Apply Phase Map To Alpha", false),
        PropDef::new("map", "Map", Kind::Text, Value::Text(String::new())).doc("the table: 256 values, 0 to 255 (or 0 to 1), split by commas or spaces; empty is no change"),
    ]
);
pub const ARBITRARY_MAP: Spec = Spec::new(&ARBITRARY_MAP_DEF, CATEGORY, arbitrary_map);

fn arbitrary_map(ctx: &Ctx, mut b: Buf) -> Buf {
    let text = ctx.value_in(ctx.effect, "map").map(|v| v.as_text().to_string()).unwrap_or_default();
    let vals: Vec<f32> = text.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|t| t.parse::<f32>().ok()).collect();
    let phase = ctx.num("phase", 0.0);
    if vals.len() < 2 && phase == 0.0 {
        return b;
    }
    let table: Vec<f32> = if vals.len() < 2 {
        (0..256).map(|i| i as f32 / 255.0).collect()
    } else {
        let k = if vals.iter().copied().fold(0f32, f32::max) > 1.0 { 1.0 / 255.0 } else { 1.0 };
        vals.iter().map(|v| (v * k).clamp(0.0, 1.0)).collect()
    };
    let alpha = ctx.flag("applyPhaseMapToAlpha");
    let n = table.len();
    let look = |v: f32| -> f32 {
        let x = (v.clamp(0.0, 1.0) as f64 * (n - 1) as f64 + phase * (n - 1) as f64 / 255.0).rem_euclid(n as f64);
        let i0 = x.floor() as usize % n;
        let i1 = (i0 + 1).min(n - 1);
        table[i0] + (table[i1] - table[i0]) * (x - x.floor()) as f32
    };
    map_px(&mut b.img, |_, _, p| {
        let (c, a) = unpremul(p);
        premul(c.map(look), if alpha { look(a) } else { a })
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 18] = [
    CURVES,
    LEVELS_INDIVIDUAL,
    AUTO_LEVELS,
    AUTO_CONTRAST,
    AUTO_COLOR,
    EQUALIZE,
    SHADOW_HIGHLIGHT,
    SELECTIVE_COLOR,
    COLOR_BALANCE_HLS,
    COLOR_LINK,
    BROADCAST,
    TONER,
    COLOR_OFFSET,
    KERNEL,
    CHANGE_COLOR,
    VIDEO_LIMITER,
    NEUTRALIZER,
    ARBITRARY_MAP,
];

#[cfg(test)]
mod tests;
