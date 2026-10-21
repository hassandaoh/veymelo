//! Noise & Grain, part two: film grain added, matched to another layer's, or taken away (Add
//! Grain, Match Grain, Remove Grain), and the middle value round each pixel (Median, Median
//! (Legacy), Dust & Scratches).

use veymelo_raster::filter::{self, Edge};
use veymelo_raster::{Image, Px};

use crate::color::rgb3;
use crate::defs::{choice, color, effect, flag, layer, num, pct, point, ranged};
use crate::grain::value;
use crate::util::{luma, map_px, premul, unpremul};
use crate::{Buf, Ctx, Spec, Take};

const CATEGORY: &str = "noise";

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

const GRAIN_MODES: &[&str] = &["film", "multiply", "add", "screen", "overlay"];
const PRESETS: &[&str] = &["none", "fine35mm", "coarse16mm", "vintage8mm", "videoNoise", "monochromeHighSpeed"];

/// The settings every grain effect shares: how strong, big and soft the grain is, its colour,
/// how it is put on the picture, how it moves, and the preview box.
macro_rules! grain_effect {
    ($def:ident, $id:literal, $name:literal, $doc:literal, [$($head:expr),*], [$($tail:expr),*]) => {
        effect!(
            $def,
            $id,
            $name,
            $doc,
            [
                $($head,)*
                point("previewCenter", "Preview Center"),
                ranged("previewWidth", "Preview Width", 200.0, 1.0, 10_000.0),
                ranged("previewHeight", "Preview Height", 200.0, 1.0, 10_000.0),
                flag("showBox", "Show Box", true),
                color("boxColor", "Box Color", 1.0, 1.0, 1.0),
                $($tail,)*
                ranged("intensity", "Intensity", 1.0, 0.0, 100.0),
                ranged("redIntensity", "Red Intensity", 1.0, 0.0, 10.0),
                ranged("greenIntensity", "Green Intensity", 1.0, 0.0, 10.0),
                ranged("blueIntensity", "Blue Intensity", 1.0, 0.0, 10.0),
                ranged("size", "Size", 1.0, 0.05, 100.0),
                ranged("redSize", "Red Size", 1.0, 0.0, 10.0),
                ranged("greenSize", "Green Size", 1.0, 0.0, 10.0),
                ranged("blueSize", "Blue Size", 1.0, 0.0, 10.0),
                ranged("softness", "Softness", 0.0, 0.0, 100.0),
                ranged("aspectRatio", "Aspect Ratio", 1.0, 0.05, 20.0),
                flag("monochromatic", "Monochromatic", false),
                ranged("saturation", "Saturation", 1.0, 0.0, 10.0),
                ranged("tintAmount", "Tint Amount", 0.0, 0.0, 1.0),
                color("tintColor", "Tint Color", 1.0, 1.0, 1.0),
                ranged("redBalance", "Red Balance", 1.0, 0.0, 10.0),
                ranged("greenBalance", "Green Balance", 1.0, 0.0, 10.0),
                ranged("blueBalance", "Blue Balance", 1.0, 0.0, 10.0),
                choice("blendingMode", "Blending Mode", GRAIN_MODES, 0).doc("film: in the picture, stronger in its darker parts"),
                ranged("shadows", "Shadows", 1.0, 0.0, 10.0),
                ranged("midtones", "Midtones", 1.0, 0.0, 10.0),
                ranged("highlights", "Highlights", 1.0, 0.0, 10.0),
                ranged("midpoint", "Midpoint", 0.5, 0.0, 1.0),
                ranged("animationSpeed", "Animation Speed", 1.0, 0.0, 100.0).doc("1: new grain 24 times a second"),
                flag("animateSmoothly", "Animate Smoothly", true),
                num("randomSeed", "Random Seed", 0.0).range(0.0, 1_000_000.0),
            ]
        );
    };
}

/// The grain's look, worked out from the settings.
struct Grain {
    intensity: f32,
    channel_intensity: [f32; 3],
    size: f32,
    channel_size: [f32; 3],
    softness: f64,
    aspect: f32,
    mono: bool,
    saturation: f32,
    tint_amount: f32,
    tint: [f32; 3],
    mode: u32,
    tones: [f32; 3],
    midpoint: f32,
    frame: f32,
    seed: u64,
    balance: [f32; 3],
}

impl Grain {
    fn of(ctx: &Ctx, preset: u32) -> Grain {
        let three = |a: &str, b: &str, c: &str| [a, b, c].map(|k| ctx.num(k, 1.0) as f32);
        let frames = ctx.time.seconds() * 24.0 * ctx.num("animationSpeed", 1.0);
        let mut g = Grain {
            intensity: ctx.num("intensity", 1.0) as f32,
            channel_intensity: three("redIntensity", "greenIntensity", "blueIntensity"),
            size: ctx.num("size", 1.0).max(0.05) as f32,
            channel_size: three("redSize", "greenSize", "blueSize"),
            softness: ctx.num("softness", 0.0),
            aspect: ctx.num("aspectRatio", 1.0).max(0.05) as f32,
            mono: ctx.flag("monochromatic"),
            saturation: ctx.num("saturation", 1.0) as f32,
            tint_amount: ctx.num("tintAmount", 0.0).clamp(0.0, 1.0) as f32,
            tint: rgb3(ctx.color("tintColor")),
            mode: ctx.choice("blendingMode"),
            tones: [ctx.num("shadows", 1.0) as f32, ctx.num("midtones", 1.0) as f32, ctx.num("highlights", 1.0) as f32],
            midpoint: ctx.num("midpoint", 0.5).clamp(0.01, 0.99) as f32,
            frame: if ctx.flag("animateSmoothly") { frames as f32 } else { frames.floor() as f32 },
            seed: crate::noise::mix(&[0x6a1, ctx.num("randomSeed", 0.0).max(0.0) as u64]),
            balance: three("redBalance", "greenBalance", "blueBalance"),
        };
        // a film stock's look on top: strength, size, softness, colour, black and white. Grain
        // grows with the film's speed, and looks bigger the smaller the frame it is on (16 mm is
        // about 2.2 times smaller than 35 mm, 8 mm about 4.4 times); a video sensor's noise is
        // fine, sharp and fully coloured
        let (ki, ks, soft, ksat, mono) = match preset {
            // slow 35 mm colour film
            1 => (0.6, 0.75, 0.2, 0.6, false),
            // faster 16 mm colour film
            2 => (1.3, 0.75 * 2.2, 0.4, 0.75, false),
            // 8 mm, faded
            3 => (1.8, 0.6 * 4.4, 0.8, 0.5, false),
            4 => (1.0, 0.6, 0.0, 1.0, false),
            // fast 35 mm black and white
            5 => (1.6, 1.35, 0.3, 1.0, true),
            _ => return g,
        };
        g.intensity *= ki;
        g.size *= ks;
        g.softness += soft;
        g.saturation *= ksat;
        g.mono |= mono;
        g
    }

    /// Grain planes about 0 (one, or one a channel) over a buffer, in grains `size` units across
    /// (times each channel's size), softened and stretched.
    fn planes(&self, b: &Buf, size: f32, salt: u64) -> Vec<Vec<f32>> {
        let (w, h) = (b.img.width, b.img.height);
        let (s, o) = (b.scale, b.origin);
        let n = if self.mono { 1 } else { 3 };
        (0..n)
            .map(|k| {
                let gs = (size * if self.mono { 1.0 } else { self.channel_size[k].max(0.05) }).max(0.05);
                let seed = crate::noise::mix(&[self.seed, salt, k as u64]);
                let mut img = Image::new(w, h);
                map_px(&mut img, |x, y, _| {
                    let (lx, ly) = ((o.x + (x as f64 + 0.5) / s) as f32, (o.y + (y as f64 + 0.5) / s) as f32);
                    [value(lx / (gs * self.aspect), ly / gs, self.frame, seed, 1, None) - 0.5; 4]
                });
                let soft = self.softness * s;
                if soft > 0.05 {
                    img = filter::gaussian(&img, soft * self.aspect as f64, soft, Edge::Repeat);
                }
                img.px.iter().map(|p| p[0]).collect()
            })
            .collect()
    }

    /// How much grain a colour takes: shadows, midtones and highlights about the midpoint.
    fn weight(&self, c: [f32; 3]) -> f32 {
        let l = luma(c).clamp(0.0, 1.0);
        let dark = 1.0 - smoothstep(0.0, self.midpoint, l);
        let light = smoothstep(self.midpoint, 1.0, l);
        self.tones[0] * dark + self.tones[1] * (1.0 - dark - light) + self.tones[2] * light
    }

    /// The grain's colour from its planes' values: saturation, tint, balance.
    fn colour(&self, mut g: [f32; 3]) -> [f32; 3] {
        if !self.mono {
            let m = (g[0] + g[1] + g[2]) / 3.0;
            g = g.map(|v| m + (v - m) * self.saturation);
        }
        if self.tint_amount > 0.0 {
            let l = luma(self.tint).max(1e-3);
            g = [0, 1, 2].map(|k| g[k] * ((1.0 - self.tint_amount) + self.tint_amount * self.tint[k] / l));
        }
        [0, 1, 2].map(|k| g[k] * self.balance[k])
    }

    /// A channel `c` with grain `n` put in as the mode says.
    fn put(&self, c: f32, n: f32) -> f32 {
        match self.mode {
            1 => c * (1.0 + n),
            2 => c + n,
            3 => 1.0 - (1.0 - c) * (1.0 - n.abs()),
            4 => c + n * 4.0 * c * (1.0 - c),
            _ => c + n * (1.0 - 0.5 * c),
        }
        .max(0.0)
    }
}

/// The preview: the result inside the box, the layer as it was outside, the box drawn.
fn preview(ctx: &Ctx, b: &mut Buf, before: &Image) {
    let c = b.pixel(ctx.point("previewCenter"));
    let (hw, hh) = (ctx.num("previewWidth", 200.0) * b.scale * 0.5, ctx.num("previewHeight", 200.0) * b.scale * 0.5);
    let (x0, x1, y0, y1) = (c.x - hw, c.x + hw, c.y - hh, c.y + hh);
    let show = ctx.flag("showBox");
    let line = rgb3(ctx.color("boxColor"));
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let mut out = if px >= x0 && px < x1 && py >= y0 && py < y1 { p } else { before.px[y * w + x] };
        if show {
            let near = |v: f64, e: f64| (v - e).abs() < 0.75;
            let on = ((near(px, x0) || near(px, x1)) && py >= y0 - 0.75 && py < y1 + 0.75) || ((near(py, y0) || near(py, y1)) && px >= x0 - 0.75 && px < x1 + 0.75);
            if on {
                out = [line[0], line[1], line[2], 1.0];
            }
        }
        out
    });
}

grain_effect!(
    ADD_GRAIN_DEF,
    "addGrain",
    "Add Grain",
    "film grain over the picture",
    [choice("viewingMode", "Viewing Mode", &["preview", "blendingMatte", "finalOutput"], 2).doc("preview: only inside the box; blending matte: where the grain goes"), choice("preset", "Preset", PRESETS, 0)],
    []
);
pub const ADD_GRAIN: Spec = Spec::new(&ADD_GRAIN_DEF, CATEGORY, add_grain).points(&[("previewCenter", [0.5, 0.5])]);

fn add_grain(ctx: &Ctx, mut b: Buf) -> Buf {
    let g = Grain::of(ctx, ctx.choice("preset"));
    let view = ctx.choice("viewingMode");
    if g.intensity <= 0.0 && view != 1 {
        return b;
    }
    let before = b.img.clone();
    let planes = g.planes(&b, g.size, 101);
    let amp = g.intensity * 0.25;
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let (c, a) = unpremul(p);
        let weight = g.weight(c);
        if view == 1 {
            let v = weight.clamp(0.0, 1.0);
            return [v, v, v, 1.0];
        }
        if a <= 0.0 {
            return p;
        }
        let i = y * w + x;
        let raw = if g.mono { [planes[0][i]; 3] } else { [planes[0][i], planes[1][i], planes[2][i]] };
        let gc = g.colour(raw);
        premul([0, 1, 2].map(|k| g.put(c[k], gc[k] * amp * g.channel_intensity[k] * weight)), a)
    });
    if view == 0 {
        preview(ctx, &mut b, &before);
    }
    b
}

/// Square boxes (left, top, size; buffer pixels) where the picture is flattest, so grain shows
/// best there: up to `count` of them, `size` across.
fn flat_boxes(img: &Image, count: usize, size: usize) -> Vec<(usize, usize, usize)> {
    let (w, h) = (img.width, img.height);
    let size = size.clamp(3, w.min(h).max(3));
    if w < size || h < size || count == 0 {
        return Vec::new();
    }
    let l: Vec<f32> = img.px.iter().map(|p| luma(unpremul(*p).0)).collect();
    let step = (size / 2).max(1);
    let mut found: Vec<(f32, usize, usize)> = Vec::new();
    for y in (0..=h - size).step_by(step) {
        for x in (0..=w - size).step_by(step) {
            let (mut s, mut sq, mut solid) = (0f32, 0f32, true);
            for yy in y..y + size {
                for xx in x..x + size {
                    let v = l[yy * w + xx];
                    s += v;
                    sq += v * v;
                    solid &= img.px[yy * w + xx][3] > 0.5;
                }
            }
            if solid {
                let n = (size * size) as f32;
                found.push((sq / n - (s / n).powi(2), x, y));
            }
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)).then(a.1.cmp(&b.1)));
    found.into_iter().take(count).map(|(_, x, y)| (x, y, size)).collect()
}

fn draw_boxes(img: &mut Image, boxes: &[(usize, usize, usize)], c: [f32; 3]) {
    let w = img.width;
    for &(x, y, s) in boxes {
        for k in 0..s {
            for (px, py) in [(x + k, y), (x + k, y + s - 1), (x, y + k), (x + s - 1, y + k)] {
                if px < w && py < img.height {
                    img.px[py * w + px] = [c[0], c[1], c[2], 1.0];
                }
            }
        }
    }
}

/// How grainy a picture is where `inside` says (everywhere solid without it): each channel's
/// spread about its own softened self, and how big the grains are (from how alike neighbours
/// are).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Graininess {
    spread: [f32; 3],
    size: f32,
}

fn graininess(img: &Image, inside: Option<&[bool]>) -> Graininess {
    let (w, h) = (img.width, img.height);
    if w < 3 || h < 3 {
        return Graininess::default();
    }
    let take = |i: usize| inside.is_none_or(|m| m[i]) && img.px[i][3] > 0.5;
    let mut straight = Image::new(w, h);
    for (o, p) in straight.px.iter_mut().zip(&img.px) {
        let (c, _) = unpremul(*p);
        *o = [c[0], c[1], c[2], 1.0];
    }
    let soft = filter::gaussian(&straight, 1.5, 1.5, Edge::Repeat);
    let (mut spread, mut like, mut total) = ([0f32; 3], 0f64, 0f64);
    for (k, s) in spread.iter_mut().enumerate() {
        let fine: Vec<f32> = straight.px.iter().zip(&soft.px).map(|(a, b)| a[k] - b[k]).collect();
        let (mut sum, mut sq, mut n) = (0f64, 0f64, 0f64);
        for (i, v) in fine.iter().enumerate() {
            if take(i) {
                sum += *v as f64;
                sq += (*v as f64).powi(2);
                n += 1.0;
            }
        }
        if n > 1.0 {
            let mean = sum / n;
            *s = (sq / n - mean * mean).max(0.0).sqrt() as f32;
        }
        for y in 0..h {
            for x in 0..w - 1 {
                let i = y * w + x;
                if take(i) && take(i + 1) {
                    like += (fine[i] * fine[i + 1]) as f64;
                    total += (fine[i] * fine[i]) as f64;
                }
            }
        }
    }
    let rho = if total > 1e-12 { (like / total).clamp(0.0, 0.95) } else { 0.0 };
    Graininess { spread, size: (1.0 / (1.0 - rho as f32)).clamp(0.5, 20.0) * 0.6 }
}

fn box_mask(img: &Image, boxes: &[(usize, usize, usize)]) -> Option<Vec<bool>> {
    if boxes.is_empty() {
        return None;
    }
    let w = img.width;
    let mut m = vec![false; img.px.len()];
    for &(x, y, s) in boxes {
        for yy in y..y + s {
            for xx in x..x + s {
                m[yy * w + xx] = true;
            }
        }
    }
    Some(m)
}

fn boxes_of(ctx: &Ctx, img: &Image, scale: f64) -> Vec<(usize, usize, usize)> {
    let n = ctx.num("numberOfSamples", 80.0).clamp(1.0, 500.0) as usize;
    let s = (ctx.num("sampleSize", 9.0) * scale).round().max(3.0) as usize;
    flat_boxes(img, n, s)
}

grain_effect!(
    MATCH_GRAIN_DEF,
    "matchGrain",
    "Match Grain",
    "grain like another layer's added, so pictures from different sources sit together",
    [
        choice("viewingMode", "Viewing Mode", &["preview", "noiseSamples", "compensationSamples", "blendingMatte", "finalOutput"], 4),
        layer("noiseSourceLayer", "Noise Source Layer"),
        pct("compensateForExistingNoise", "Compensate for Existing Noise", 0.0, 0.0, 100.0)
    ],
    [ranged("numberOfSamples", "Number of Samples", 80.0, 1.0, 500.0), ranged("sampleSize", "Sample Size", 9.0, 3.0, 64.0), color("sampleBoxColor", "Sample Box Color", 1.0, 1.0, 0.0)]
);
pub const MATCH_GRAIN: Spec = Spec::new(&MATCH_GRAIN_DEF, CATEGORY, match_grain).points(&[("previewCenter", [0.5, 0.5])]);

fn match_grain(ctx: &Ctx, mut b: Buf) -> Buf {
    let Some(source) = ctx.layer_over("noiseSourceLayer", &b, Take::Source, false) else { return b };
    let view = ctx.choice("viewingMode");
    let (src_boxes, own_boxes) = (boxes_of(ctx, &source, b.scale), boxes_of(ctx, &b.img, b.scale));
    if view == 1 || view == 2 {
        let (mut img, boxes) = if view == 1 { (source, src_boxes) } else { (b.img.clone(), own_boxes) };
        draw_boxes(&mut img, &boxes, rgb3(ctx.color("sampleBoxColor")));
        b.img = img;
        return b;
    }
    let theirs = graininess(&source, box_mask(&source, &src_boxes).as_deref());
    let take_away = (ctx.num("compensateForExistingNoise", 0.0) / 100.0) as f32;
    let ours = if take_away > 0.0 { graininess(&b.img, box_mask(&b.img, &own_boxes).as_deref()) } else { Graininess::default() };
    let before = b.img.clone();
    let g = Grain::of(ctx, 0);
    // as much as the source has, less what is here already
    let amp: [f32; 3] = [0, 1, 2].map(|k| {
        let (s, e) = (theirs.spread[k], ours.spread[k] * take_away);
        (s * s - e * e).max(0.0).sqrt() * g.intensity * g.channel_intensity[k]
    });
    let size = (theirs.size.max(0.5) * g.size / b.scale as f32).max(0.05);
    let mut planes = g.planes(&b, size, 131);
    // each plane spread one, so the measured spread carries over
    for p in planes.iter_mut() {
        let n = p.len().max(1) as f64;
        let mean = p.iter().map(|v| *v as f64).sum::<f64>() / n;
        let var = p.iter().map(|v| (*v as f64 - mean).powi(2)).sum::<f64>() / n;
        let k = if var > 1e-12 { 1.0 / var.sqrt() } else { 0.0 };
        for v in p.iter_mut() {
            *v = ((*v as f64 - mean) * k) as f32;
        }
    }
    let w = b.img.width;
    map_px(&mut b.img, |x, y, p| {
        let (c, a) = unpremul(p);
        let weight = g.weight(c);
        if view == 3 {
            let v = weight.clamp(0.0, 1.0);
            return [v, v, v, 1.0];
        }
        if a <= 0.0 {
            return p;
        }
        let i = y * w + x;
        let raw = if g.mono { [planes[0][i]; 3] } else { [planes[0][i], planes[1][i], planes[2][i]] };
        let gc = g.colour(raw);
        premul([0, 1, 2].map(|k| g.put(c[k], gc[k] * amp[k] * weight)), a)
    });
    if view == 0 {
        preview(ctx, &mut b, &before);
    }
    b
}

effect!(
    REMOVE_GRAIN_DEF,
    "removeGrain",
    "Remove Grain",
    "grain and noise smoothed away, edges kept, with an unsharp mask after and a look across frames",
    [
        choice("viewingMode", "Viewing Mode", &["preview", "noiseSamples", "blendingMatte", "finalOutput"], 3),
        point("previewCenter", "Preview Center"),
        ranged("previewWidth", "Preview Width", 200.0, 1.0, 10_000.0),
        ranged("previewHeight", "Preview Height", 200.0, 1.0, 10_000.0),
        flag("showBox", "Show Box", true),
        color("boxColor", "Box Color", 1.0, 1.0, 1.0),
        ranged("noiseReduction", "Noise Reduction", 1.0, 0.0, 5.0),
        ranged("passes", "Passes", 1.0, 1.0, 8.0),
        choice("mode", "Mode", &["multichannel", "singleChannel"], 0).doc("multichannel: the colours smoothed together, by the brightness"),
        ranged("texture", "Texture", 0.0, 0.0, 1.0).doc("how much fine detail comes back"),
        ranged("unsharpAmount", "Unsharp Amount", 0.0, 0.0, 500.0),
        ranged("unsharpRadius", "Unsharp Radius", 1.0, 0.1, 100.0),
        ranged("unsharpThreshold", "Unsharp Threshold", 0.0, 0.0, 255.0),
        flag("temporalFiltering", "Temporal Filtering", false).doc("still parts are also smoothed with the frames before and after"),
        pct("temporalAmount", "Temporal Amount", 100.0, 0.0, 100.0),
        ranged("motionSensitivity", "Motion Sensitivity", 0.5, 0.0, 1.0),
        ranged("numberOfSamples", "Number of Samples", 80.0, 1.0, 500.0),
        ranged("sampleSize", "Sample Size", 9.0, 3.0, 64.0),
        color("sampleBoxColor", "Sample Box Color", 1.0, 1.0, 0.0),
    ]
);
pub const REMOVE_GRAIN: Spec = Spec::new(&REMOVE_GRAIN_DEF, CATEGORY, remove_grain).points(&[("previewCenter", [0.5, 0.5])]);

/// One plane smoothed but for its edges (a guided filter: each pixel a straight function of
/// `guide` fitted over a square `r` each way; changes bigger than about √`eps` stay).
pub(crate) fn guided(guide: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let size = (2 * r + 1) as f64;
    const PIECE: usize = 1 << 14;
    let mut img = Image::new(w, h);
    veymelo_raster::par::chunks(&mut img.px, PIECE, |n, part| {
        for (i, o) in part.iter_mut().enumerate() {
            let k = n * PIECE + i;
            *o = [guide[k], p[k], guide[k] * guide[k], guide[k] * p[k]];
        }
    });
    let m = filter::boxes(&img, size, size, 1, Edge::Repeat);
    let mut ab = Image::new(w, h);
    veymelo_raster::par::chunks(&mut ab.px, PIECE, |n, part| {
        for (o, &[mi, mp, mii, mip]) in part.iter_mut().zip(&m.px[n * PIECE..]) {
            let a = (mip - mi * mp) / ((mii - mi * mi).max(0.0) + eps);
            *o = [a, mp - a * mi, 0.0, 0.0];
        }
    });
    let mab = filter::boxes(&ab, size, size, 1, Edge::Repeat);
    let mut out = vec![0f32; w * h];
    veymelo_raster::par::chunks(&mut out, PIECE, |n, part| {
        for (i, o) in part.iter_mut().enumerate() {
            let k = n * PIECE + i;
            *o = mab.px[k][0] * guide[k] + mab.px[k][1];
        }
    });
    out
}

fn remove_grain(ctx: &Ctx, mut b: Buf) -> Buf {
    let amount = ctx.num("noiseReduction", 1.0) as f32;
    let sharpen = (ctx.num("unsharpAmount", 0.0) / 100.0) as f32;
    let view = ctx.choice("viewingMode");
    let boxes = boxes_of(ctx, &b.img, b.scale);
    if view == 1 {
        draw_boxes(&mut b.img, &boxes, rgb3(ctx.color("sampleBoxColor")));
        return b;
    }
    if view == 2 {
        b.img.px.fill([1.0; 4]);
        return b;
    }
    let temporal = ctx.flag("temporalFiltering");
    if amount <= 0.0 && sharpen <= 0.0 && !temporal {
        return b;
    }
    let before = b.img.clone();
    let (w, h) = (b.img.width, b.img.height);
    if temporal {
        // still pixels averaged with the frames on either side (the grain changes, the picture not)
        let at = |t: veymelo_time::Tick| ctx.host.and_then(|host| host.this_at(t, ctx.index)).filter(|o| o.img.width == w && o.img.height == h);
        let (prev, next) = (at(ctx.time - ctx.frame), at(ctx.time + ctx.frame));
        let k = (ctx.num("temporalAmount", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
        let limit = 0.02 + (1.0 - ctx.num("motionSensitivity", 0.5).clamp(0.0, 1.0) as f32) * 0.2;
        if prev.is_some() || next.is_some() {
            map_px(&mut b.img, |x, y, p| {
                let i = y * w + x;
                let (mut acc, mut n) = (p, 1.0);
                for o in [&prev, &next].into_iter().flatten() {
                    let q = o.img.px[i];
                    let d = (0..3).map(|c| (q[c] - p[c]).abs()).fold(0f32, f32::max);
                    if d < limit {
                        let wgt = 1.0 - d / limit;
                        for c in 0..4 {
                            acc[c] += q[c] * wgt;
                        }
                        n += wgt;
                    }
                }
                [0, 1, 2, 3].map(|c| p[c] + (acc[c] / n - p[c]) * k)
            });
        }
    }
    let alpha: Vec<f32> = b.img.px.iter().map(|p| p[3]).collect();
    let orig: Vec<Vec<f32>> = (0..3).map(|c| b.img.px.iter().map(|p| p[c]).collect()).collect();
    let mut ch = orig.clone();
    if amount > 0.0 {
        // smoothed by how grainy the flat parts are
        let level = if boxes.is_empty() {
            1.0
        } else {
            let g = graininess(&b.img, box_mask(&b.img, &boxes).as_deref());
            ((g.spread[0] + g.spread[1] + g.spread[2]) / 3.0 / 0.02).clamp(0.5, 2.0)
        };
        let passes = ctx.num("passes", 1.0).round().clamp(1.0, 8.0) as usize;
        let together = ctx.choice("mode") == 0;
        let texture = ctx.num("texture", 0.0).clamp(0.0, 1.0) as f32;
        let r = (2.0 * b.scale).round().max(1.0) as usize;
        let eps = (0.02 * amount * level).powi(2);
        for _ in 0..passes {
            let guide: Option<Vec<f32>> = together.then(|| (0..w * h).map(|i| luma([ch[0][i], ch[1][i], ch[2][i]])).collect());
            for c in ch.iter_mut() {
                *c = match &guide {
                    Some(g) => guided(g, c, w, h, r, eps),
                    None => guided(c, c, w, h, r, eps),
                };
            }
        }
        if texture > 0.0 {
            for (c, o) in ch.iter_mut().zip(&orig) {
                for (v, q) in c.iter_mut().zip(o) {
                    *v += (q - *v) * texture;
                }
            }
        }
    }
    if sharpen > 0.0 {
        let radius = (ctx.num("unsharpRadius", 1.0) * b.scale).max(0.1);
        let limit = (ctx.num("unsharpThreshold", 0.0) / 255.0) as f32;
        for c in ch.iter_mut() {
            let mut img = Image::new(w, h);
            for (o, v) in img.px.iter_mut().zip(c.iter()) {
                *o = [*v; 4];
            }
            let soft = filter::gaussian(&img, radius, radius, Edge::Repeat);
            for (v, s) in c.iter_mut().zip(&soft.px) {
                let d = *v - s[0];
                if d.abs() > limit {
                    *v += d * sharpen;
                }
            }
        }
    }
    for (i, p) in b.img.px.iter_mut().enumerate() {
        let a = alpha[i];
        *p = [ch[0][i].clamp(0.0, a.max(0.0)), ch[1][i].clamp(0.0, a.max(0.0)), ch[2][i].clamp(0.0, a.max(0.0)), a];
    }
    if view == 0 {
        preview(ctx, &mut b, &before);
    }
    b
}

/// Levels a channel is counted in for medians.
const BINS: usize = 512;

/// Each channel's middle value over a square `r` pixels each way (the edges going on), found by
/// sliding a count of the values along each row.
pub fn median_of(img: &Image, r: usize) -> Image {
    let (w, h) = (img.width, img.height);
    if r == 0 || w == 0 || h == 0 {
        return img.clone();
    }
    let level = |v: f32| (v.clamp(0.0, 1.0) * (BINS - 1) as f32).round() as usize;
    let q: Vec<[u16; 4]> = img.px.iter().map(|p| p.map(|v| level(v) as u16)).collect();
    let half = (2 * r + 1) * (2 * r + 1) / 2;
    let ri = r as i64;
    let mut out = Image::new(w, h);
    veymelo_raster::par::rows(&mut out.px, w, |y, row| {
        let at = |x: i64, y: i64| q[(y.clamp(0, h as i64 - 1) as usize) * w + x.clamp(0, w as i64 - 1) as usize];
        let mut count = vec![0u32; BINS * 4];
        // where each channel's middle is, and how many lie below it
        let (mut mid, mut below) = ([0usize; 4], [0usize; 4]);
        let column = |count: &mut Vec<u32>, below: &mut [usize; 4], mid: &[usize; 4], x: i64, add: bool| {
            for dy in -ri..=ri {
                let v = at(x, y as i64 + dy);
                for c in 0..4 {
                    let b = v[c] as usize;
                    if add {
                        count[c * BINS + b] += 1;
                        if b < mid[c] {
                            below[c] += 1;
                        }
                    } else {
                        count[c * BINS + b] -= 1;
                        if b < mid[c] {
                            below[c] -= 1;
                        }
                    }
                }
            }
        };
        let settle = |count: &[u32], mid: &mut [usize; 4], below: &mut [usize; 4]| {
            for c in 0..4 {
                let k = &count[c * BINS..(c + 1) * BINS];
                while below[c] > half {
                    mid[c] -= 1;
                    below[c] -= k[mid[c]] as usize;
                }
                while below[c] + k[mid[c]] as usize <= half {
                    below[c] += k[mid[c]] as usize;
                    mid[c] += 1;
                }
            }
        };
        for dx in -ri..=ri {
            column(&mut count, &mut below, &mid, dx, true);
        }
        settle(&count, &mut mid, &mut below);
        for (x, out) in row.iter_mut().enumerate() {
            *out = mid.map(|m| m as f32 / (BINS - 1) as f32);
            if x + 1 == w {
                break;
            }
            column(&mut count, &mut below, &mid, x as i64 - ri, false);
            column(&mut count, &mut below, &mid, x as i64 + ri + 1, true);
            settle(&count, &mut mid, &mut below);
        }
    });
    out
}

/// The median's pixel: all of it, or (not on alpha) its colour with the layer's own alpha.
fn median_pixel(orig: Px, m: Px, on_alpha: bool) -> Px {
    if on_alpha {
        let a = m[3].clamp(0.0, 1.0);
        return [m[0].min(a), m[1].min(a), m[2].min(a), a];
    }
    if m[3] <= 1e-4 {
        return orig;
    }
    premul([m[0] / m[3], m[1] / m[3], m[2] / m[3]].map(|v| v.min(1.0)), orig[3])
}

effect!(
    MEDIAN_DEF,
    "median",
    "Median",
    "each pixel the middle value round it: specks gone, the picture painterly at larger radii",
    [ranged("radius", "Radius", 0.0, 0.0, 255.0), flag("operateOnAlphaChannel", "Operate On Alpha Channel", false),]
);
pub const MEDIAN: Spec = Spec::new(&MEDIAN_DEF, CATEGORY, median);

fn median(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = (ctx.num("radius", 0.0) * b.scale).round().max(0.0) as usize;
    if r == 0 {
        return b;
    }
    let on_alpha = ctx.flag("operateOnAlphaChannel");
    let m = median_of(&b.img, r);
    for (o, q) in b.img.px.iter_mut().zip(&m.px) {
        *o = median_pixel(*o, *q, on_alpha);
    }
    b
}

effect!(
    MEDIAN_LEGACY_DEF,
    "medianLegacy",
    "Median (Legacy)",
    "the old median: the middle colour round each pixel",
    [ranged("radius", "Radius", 1.0, 0.0, 255.0), flag("operateOnAlphaChannel", "Operate On Alpha Channel", false),]
);
pub const MEDIAN_LEGACY: Spec = Spec::new(&MEDIAN_LEGACY_DEF, CATEGORY, median_legacy);

fn median_legacy(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = (ctx.num("radius", 1.0) * b.scale).round().max(0.0) as usize;
    if r == 0 {
        return b;
    }
    if ctx.flag("operateOnAlphaChannel") {
        b.img = median_of(&b.img, r);
        return b;
    }
    // the straight colours' median, the alpha kept
    let mut straight = b.img.clone();
    for p in &mut straight.px {
        let (c, _) = unpremul(*p);
        *p = [c[0], c[1], c[2], 1.0];
    }
    let m = median_of(&straight, r);
    for (p, q) in b.img.px.iter_mut().zip(&m.px) {
        *p = premul([q[0], q[1], q[2]], p[3]);
    }
    b
}

effect!(
    DUST_DEF,
    "dustAndScratches",
    "Dust & Scratches",
    "specks and scratches that differ from what is round them by more than the threshold, smoothed away",
    [ranged("radius", "Radius", 1.0, 0.0, 255.0), ranged("threshold", "Threshold", 0.0, 0.0, 255.0), flag("operateOnAlphaChannel", "Operate On Alpha Channel", false),]
);
pub const DUST: Spec = Spec::new(&DUST_DEF, CATEGORY, dust);

fn dust(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = (ctx.num("radius", 1.0) * b.scale).round().max(0.0) as usize;
    if r == 0 {
        return b;
    }
    let limit = (ctx.num("threshold", 0.0) / 255.0) as f32;
    let on_alpha = ctx.flag("operateOnAlphaChannel");
    let m = median_of(&b.img, r);
    for (o, q) in b.img.px.iter_mut().zip(&m.px) {
        let c = median_pixel(*o, *q, on_alpha);
        if (0..4).map(|k| (c[k] - o[k]).abs()).fold(0f32, f32::max) > limit {
            *o = c;
        }
    }
    b
}

/// Every effect here.
pub const ALL: [Spec; 6] = [ADD_GRAIN, MATCH_GRAIN, REMOVE_GRAIN, MEDIAN, MEDIAN_LEGACY, DUST];

#[cfg(test)]
mod tests;
