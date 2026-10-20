//! A composition's sound, mixed: every layer heard — footage with sound, compositions inside
//! (mixed the same way at their layer's time), layers that make sound (Tone) — at its Audio
//! Levels, from its in to its out, its audio switch on (and only soloed ones while any is),
//! through its audio effects. A layer stretched, remapped or played backwards is read at its
//! own time sample by sample, between samples blended.
//!
//! Sound is mixed in blocks (a frame, a player's buffer). Levels and effects' settings are
//! read at a block's two ends and eased across it; an effect runs from silence over a little
//! of the sound before the block, so a block sounds the same wherever blocks fall — previews
//! and exports are the same sound.
//!
//! Loudness is measured as ITU-R BS.1770 says (K-weighted, gated: LUFS) and as peaks.

use veymelo_effects::audio::{self as fx, Params};
use veymelo_expr::{Eval, Place};
use veymelo_project::{Comp, FootageKind, Group, ItemKind, Layer, Node, Project, Source};
use veymelo_raster::Image;
use veymelo_time::Tick;

/// The rate sound is mixed at unless asked another.
pub const RATE: u32 = 48_000;

const MAX_DEPTH: usize = 16;

/// The gain of a level in dB: −96 and under is silence.
pub fn db_to_gain(db: f64) -> f32 {
    if db <= -96.0 { 0.0 } else { 10f64.powf(db / 20.0) as f32 }
}

/// `frames` stereo frames (left, right, left…) of composition `comp`'s sound from its time
/// `start`, at `rate`.
pub fn mix(project: &Project, comp: u64, start: Tick, frames: usize, rate: u32) -> Vec<f32> {
    let m = Mixer { eval: Eval::new(project), project, rate: rate.max(1) };
    let i0 = (start.seconds() * rate as f64).round() as i64;
    m.comp(comp, i0, frames, 0)
}

/// `frames` stereo frames of layer `layer` of composition `comp` alone, from composition time
/// `start`, at `rate`: as the mix hears it, the others left out.
pub fn layer_mix(project: &Project, comp: u64, layer: u64, start: Tick, frames: usize, rate: u32) -> Vec<f32> {
    let Some(c) = project.comp(comp) else { return vec![0.0; frames * 2] };
    let Some(l) = c.layer(layer) else { return vec![0.0; frames * 2] };
    let mut alone = c.clone();
    alone.layers = vec![l.clone()];
    let m = Mixer { eval: Eval::new(project), project, rate: rate.max(1) };
    let i0 = (start.seconds() * rate as f64).round() as i64;
    m.comp_as(&alone, comp, i0, frames, 0)
}

/// Whether anything in composition `comp` is heard.
pub fn heard(project: &Project, comp: u64) -> bool {
    fn walk(project: &Project, comp: u64, depth: usize) -> bool {
        let Some(c) = project.comp(comp) else { return false };
        depth < MAX_DEPTH && c.layers.iter().any(|l| audible(project, l, depth))
    }
    walk(project, comp, 0)
}

fn audio_effects(l: &Layer) -> Vec<&Group> {
    if !l.switches.effects {
        return Vec::new();
    }
    l.props.group("effects").map(|g| g.children.iter().filter_map(Node::as_group).filter(|g| g.enabled && fx::is_audio(g.def.id)).collect()).unwrap_or_default()
}

/// Whether a layer is heard: its audio switch on, and sound to hear.
fn audible(project: &Project, l: &Layer, depth: usize) -> bool {
    l.switches.audio
        && (audio_effects(l).iter().any(|g| fx::generates(g.def.id))
            || match &l.source {
                Source::Footage(id) => matches!(project.item(*id).map(|i| &i.kind), Some(ItemKind::Footage(f)) if f.has_audio),
                Source::Comp(id) => depth + 1 < MAX_DEPTH && project.comp(*id).is_some_and(|c| c.layers.iter().any(|l| audible(project, l, depth + 1))),
                _ => false,
            })
}

/// The first sample at or after time `t`.
fn sample_at(t: Tick, rate: u32) -> i64 {
    (t.seconds() * rate as f64 - 1e-9).ceil() as i64
}

struct Mixer<'a> {
    project: &'a Project,
    eval: Eval<'a>,
    rate: u32,
}

impl<'a> Mixer<'a> {
    fn time(&self, i: i64) -> Tick {
        Tick::from_seconds(i as f64 / self.rate as f64)
    }

    /// Samples `i0` … `i0 + n` of a composition (sample `i` at its time `i` / rate).
    fn comp(&self, id: u64, i0: i64, n: usize, depth: usize) -> Vec<f32> {
        match self.project.comp(id) {
            Some(comp) => self.comp_as(comp, id, i0, n, depth),
            None => vec![0.0f32; n * 2],
        }
    }

    /// Composition `comp` (by id `id`; as a layer showing it sets it) mixed.
    fn comp_as(&self, comp: &Comp, id: u64, i0: i64, n: usize, depth: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; n * 2];
        if depth >= MAX_DEPTH || n == 0 {
            return out;
        }
        let solo = comp.layers.iter().any(|l| l.switches.solo && audible(self.project, l, depth));
        for l in &comp.layers {
            if !audible(self.project, l, depth) || (solo && !l.switches.solo) {
                continue;
            }
            // the part of the block inside the layer's in and out
            let a = (sample_at(l.in_point, self.rate) - i0).clamp(0, n as i64) as usize;
            let b = (sample_at(l.out_point, self.rate) - i0).clamp(0, n as i64) as usize;
            if a >= b {
                continue;
            }
            let at = Place { layer: l, comp, comp_id: id };
            let (g0, g1) = (self.levels(at, i0 + a as i64), self.levels(at, i0 + b as i64));
            if g0 == [0.0; 2] && g1 == [0.0; 2] {
                continue;
            }
            let sound = self.layer(at, i0 + a as i64, b - a, depth);
            let len = (b - a).max(1) as f32;
            for (k, f) in sound.as_chunks::<2>().0.iter().enumerate() {
                // the levels eased across the block, so a fade does not step
                let w = k as f32 / len;
                out[(a + k) * 2] += f[0] * (g0[0] + (g1[0] - g0[0]) * w);
                out[(a + k) * 2 + 1] += f[1] * (g0[1] + (g1[1] - g0[1]) * w);
            }
        }
        out
    }

    fn levels(&self, at: Place<'a>, i: i64) -> [f32; 2] {
        match at.layer.props.prop("audio/levels") {
            Ok(p) => {
                let v = self.eval.value(p, at, self.time(i)).as_vec2();
                [db_to_gain(v[0]), db_to_gain(v[1])]
            }
            Err(_) => [1.0; 2],
        }
    }

    /// A layer's sound for samples `i0` … `i0 + n`, through its audio effects (not yet at its
    /// levels).
    fn layer(&self, at: Place<'a>, i0: i64, n: usize, depth: usize) -> Vec<f32> {
        let effects = audio_effects(at.layer);
        if effects.is_empty() {
            return self.raw(at, i0, n, false, depth);
        }
        let t = self.time(i0);
        let get_for = |g: &'a Group| move |id: &str| g.prop(id).ok().map(|p| self.eval.value(p, at, t));
        let reversed = effects.iter().any(|g| fx::reverses(g.def.id));
        let pre_s: f64 = effects
            .iter()
            .map(|g| {
                let get = get_for(g);
                fx::preroll(g.def.id, &Params { def: g.def, get: &get })
            })
            .sum();
        let pre = (pre_s * self.rate as f64).ceil() as usize;
        let mut buf = self.raw(at, i0 - pre as i64, n + pre, reversed, depth);
        let start = at.layer.layer_time(self.time(i0 - pre as i64)).seconds();
        for g in &effects {
            let get = get_for(g);
            fx::process(g.def.id, &Params { def: g.def, get: &get }, &mut buf, self.rate, start);
        }
        buf.drain(..pre * 2);
        buf
    }

    /// A layer's own sound for samples `i0` … `i0 + n`, silent outside its in and out; with
    /// `reversed`, its stretch from in to out the other way round.
    fn raw(&self, at: Place<'a>, i0: i64, n: usize, reversed: bool, depth: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; n * 2];
        let (a_in, a_out) = (sample_at(at.layer.in_point, self.rate), sample_at(at.layer.out_point, self.rate));
        let (a, b) = (i0.max(a_in), (i0 + n as i64).min(a_out));
        if a >= b {
            return out;
        }
        if reversed {
            // the mirrored stretch, read forwards, laid down backwards
            let (ma, mb) = (a_in + a_out - b, a_in + a_out - a);
            let fwd = self.source(at, ma, (mb - ma) as usize, depth);
            for k in a..b {
                let j = (a_in + a_out - 1 - k - ma) as usize;
                let o = (k - i0) as usize;
                out[o * 2] = fwd[j * 2];
                out[o * 2 + 1] = fwd[j * 2 + 1];
            }
            return out;
        }
        let s = self.source(at, a, (b - a) as usize, depth);
        out[(a - i0) as usize * 2..(b - i0) as usize * 2].copy_from_slice(&s);
        out
    }

    /// What a layer shows' sound at the layer's own time for samples `i0` … `i0 + n`.
    fn source(&self, at: Place<'a>, i0: i64, n: usize, depth: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; n * 2];
        if n == 0 {
            return out;
        }
        let rate = self.rate as f64;
        // the source time of each sample: at most ten milliseconds apart, straight between
        let step = (self.rate as usize / 100).max(1);
        let mut knots: Vec<(usize, f64)> = (0..n).step_by(step).map(|k| (k, self.eval.source_time(at, self.time(i0 + k as i64)).seconds())).collect();
        knots.push((n, self.eval.source_time(at, self.time(i0 + n as i64)).seconds()));
        let times: Vec<f64> = (0..n)
            .map(|k| {
                let j = (k / step).min(knots.len() - 2);
                let ((k0, s0), (k1, s1)) = (knots[j], knots[j + 1]);
                s0 + (s1 - s0) * (k - k0) as f64 / (k1 - k0).max(1) as f64
            })
            .collect();
        let (lo, hi) = times.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), t| (a.min(*t), b.max(*t)));
        if (hi - lo).is_nan() || hi - lo >= 600.0 {
            return out;
        }
        // the source's samples over that span, one either side to blend with
        let first = (lo * rate).floor() as i64 - 1;
        let m = ((hi * rate).ceil() as i64 - first + 2).max(1) as usize;
        let buf = match &at.layer.source {
            Source::Footage(id) => match self.project.item(*id).map(|i| &i.kind) {
                Some(ItemKind::Footage(f)) if f.has_audio && f.kind != FootageKind::Image => veymelo_media::sound(f, first as f64 / rate, m, self.rate).unwrap_or_else(|_| vec![0.0; m * 2]),
                _ => return out,
            },
            Source::Comp(id) => {
                if contains(self.project, *id, at.comp_id, 0) {
                    return out;
                }
                // what the layer sets of its essential properties (a swapped clip, its levels)
                match veymelo_project::essential::instance(self.project, *id, at.layer, &|p| self.eval.value(p, at, self.time(first))) {
                    Some(c) => self.comp_as(&c, *id, first, m, depth + 1),
                    None => self.comp(*id, first, m, depth + 1),
                }
            }
            _ => return out,
        };
        for (k, t) in times.iter().enumerate() {
            let x = t * rate - first as f64;
            let j = x.floor();
            if j < 0.0 || j as usize + 1 >= m {
                continue;
            }
            let (j, f) = (j as usize, (x - j) as f32);
            // a sample landing on a source sample is that sample, as it is
            let f = if f < 1e-4 { 0.0 } else { f };
            for c in 0..2 {
                let (p, q) = (buf[j * 2 + c], buf[(j + 1) * 2 + c]);
                out[k * 2 + c] = p + (q - p) * f;
            }
        }
        out
    }
}

/// Whether composition `outer` shows `inner` (itself, or somewhere inside).
fn contains(project: &Project, outer: u64, inner: u64, depth: usize) -> bool {
    outer == inner || (depth < MAX_DEPTH && project.comp(outer).is_some_and(|c| c.layers.iter().any(|l| matches!(l.source, Source::Comp(id) if contains(project, id, inner, depth + 1)))))
}

/// A summary of stereo sound for drawing it: for each `bin` frames, the lowest and highest
/// left and right samples.
pub fn peaks(samples: &[f32], bin: usize) -> Vec<[f32; 4]> {
    samples
        .chunks(bin.max(1) * 2)
        .map(|c| {
            let mut s = [f32::MAX, f32::MIN, f32::MAX, f32::MIN];
            for f in c.as_chunks::<2>().0 {
                s = [s[0].min(f[0]), s[1].max(f[0]), s[2].min(f[1]), s[3].max(f[1])];
            }
            if s[0] > s[1] { [0.0; 4] } else { s }
        })
        .collect()
}

/// How loud sound is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loudness {
    /// Integrated loudness (LUFS, gated as BS.1770 says); none when it is all silence.
    pub integrated: Option<f64>,
    /// The loudest three seconds (short-term LUFS).
    pub short_max: Option<f64>,
    /// The highest sample of each channel, in dBFS.
    pub peak: [f64; 2],
    /// The square-mean level of each channel, in dBFS.
    pub rms: [f64; 2],
}

fn to_db(v: f64) -> f64 {
    if v > 0.0 { 20.0 * v.log10() } else { f64::NEG_INFINITY }
}

/// BS.1770's K-weighting as the recommendation gives it at 48 kHz: a high shelf (+4 dB above
/// about 1.5 kHz, as a head hears), then a high pass (under about 38 Hz); each as (b, a).
const SHELF_48K: ([f64; 3], [f64; 2]) = ([1.535_124_859_586_97, -2.691_696_189_406_38, 1.198_392_810_852_85], [-1.690_659_293_182_41, 0.732_480_774_215_85]);
const HIGH_PASS_48K: [f64; 2] = [-1.990_047_454_833_98, 0.990_072_250_366_21];

/// A biquad's poles (its a1 and a2) taken back to the analogue filter the bilinear transform made
/// them from: the warped frequency k = tan(π·f0 / 48000), Q, and the shared divisor a0
/// (1 + k/Q + k²). With 1 + a1 + a2 = 4k²/a0 and 1 − a1 + a2 = 4/a0 they come out exactly.
fn analogue(a: [f64; 2]) -> (f64, f64, f64) {
    let a0 = 4.0 / (1.0 - a[0] + a[1]);
    let k = ((1.0 + a[0] + a[1]) / (1.0 - a[0] + a[1])).sqrt();
    let q = 2.0 * k / ((1.0 - a[1]) * a0);
    (k, q, a0)
}

/// The K-weighting at `rate`: the two analogue filters found from the 48 kHz ones (the shelf's
/// gains from its zeros: b1 gives the high gain, b0 − b2 the band's), made again for `rate`.
fn k_weighting(rate: f64) -> [([f64; 3], [f64; 2]); 2] {
    let again = |k48: f64| (k48.atan() * 48_000.0 / rate).tan();
    let poles = |k: f64, q: f64| {
        let a0 = 1.0 + k / q + k * k;
        (a0, [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0])
    };
    let (b, a) = SHELF_48K;
    let (k48, q, a0) = analogue(a);
    let high = k48 * k48 - b[1] * a0 / 2.0;
    let band = (b[0] - b[2]) * a0 * q / (2.0 * k48);
    let k = again(k48);
    let (d, shelf_a) = poles(k, q);
    let shelf = ([(high + band * k / q + k * k) / d, 2.0 * (k * k - high) / d, (high - band * k / q + k * k) / d], shelf_a);
    let (k48, q, _) = analogue(HIGH_PASS_48K);
    let (_, pass_a) = poles(again(k48), q);
    [shelf, ([1.0, -2.0, 1.0], pass_a)]
}

/// How loud stereo sound at `rate` is.
pub fn loudness(samples: &[f32], rate: u32) -> Loudness {
    let n = samples.len() / 2;
    let mut peak = [0.0f64; 2];
    let mut sq = [0.0f64; 2];
    for f in samples.as_chunks::<2>().0 {
        for c in 0..2 {
            peak[c] = peak[c].max(f[c].abs() as f64);
            sq[c] += (f[c] as f64).powi(2);
        }
    }
    let rms = sq.map(|s| to_db((s / n.max(1) as f64).sqrt()));
    // K-weighted, each channel
    let mut weighted = vec![[0.0f64; 2]; n];
    for c in 0..2 {
        let mut x: Vec<f64> = samples.iter().skip(c).step_by(2).map(|v| *v as f64).collect();
        for (b, a) in k_weighting(rate as f64) {
            let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
            for v in x.iter_mut() {
                let y = b[0] * *v + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
                (x2, x1, y2, y1) = (x1, *v, y1, y);
                *v = y;
            }
        }
        for (w, v) in weighted.iter_mut().zip(&x) {
            w[c] = v * v;
        }
    }
    // the mean square of windows `len` long, every `hop`
    let windows = |len: usize, hop: usize| -> Vec<f64> {
        if n < len {
            return Vec::new();
        }
        let mut sums = vec![0.0f64; n + 1];
        for (i, w) in weighted.iter().enumerate() {
            sums[i + 1] = sums[i] + w[0] + w[1];
        }
        (0..=(n - len) / hop).map(|k| (sums[k * hop + len] - sums[k * hop]) / len as f64).collect()
    };
    let lufs = |z: f64| -0.691 + 10.0 * z.max(1e-30).log10();
    let r = rate as usize;
    // gated: blocks of 400 ms every 100 ms, those over −70 LUFS, then those within 10 LU of their mean
    let blocks: Vec<f64> = windows(r * 4 / 10, r / 10).into_iter().filter(|z| lufs(*z) > -70.0).collect();
    let integrated = (!blocks.is_empty()).then(|| {
        let mean = blocks.iter().sum::<f64>() / blocks.len() as f64;
        let kept: Vec<f64> = blocks.iter().copied().filter(|z| lufs(*z) > lufs(mean) - 10.0).collect();
        lufs(kept.iter().sum::<f64>() / kept.len().max(1) as f64)
    });
    let short_max = windows(r * 3, r / 10).into_iter().map(lufs).filter(|l| *l > -70.0).fold(None, |m: Option<f64>, l| Some(m.map_or(l, |m| m.max(l))));
    Loudness { integrated, short_max, peak: peak.map(to_db), rms }
}

/// A picture of stereo sound, `w` × `h`: the left channel above the right, each its lows and
/// highs at every column, lines at 0, ±6 dB and the middle.
pub fn waveform(samples: &[f32], w: usize, h: usize) -> Image {
    let (w, h) = (w.max(8), h.max(8));
    let mut img = Image::filled(w, h, [0.07, 0.08, 0.1, 1.0]);
    let n = samples.len() / 2;
    let bin = n.div_ceil(w).max(1);
    let p = peaks(samples, bin);
    let half = h / 2;
    let mut set = |x: usize, y: usize, c: [f32; 4]| {
        if x < w && y < h {
            img.px[y * w + x] = c;
        }
    };
    for ch in 0..2 {
        let (top, height) = (ch * half, half);
        let mid = top + height / 2;
        let y_of = |v: f32| (mid as f32 - v.clamp(-1.0, 1.0) * (height as f32 / 2.0 - 1.0)).round() as usize;
        for x in 0..w {
            set(x, mid, [0.25, 0.27, 0.3, 1.0]);
            for g in [0.5f32, -0.5] {
                set(x, y_of(g), [0.16, 0.17, 0.2, 1.0]);
            }
        }
        for (x, s) in p.iter().enumerate().take(w) {
            let (lo, hi) = if ch == 0 { (s[0], s[1]) } else { (s[2], s[3]) };
            let clip = lo <= -1.0 || hi >= 1.0;
            let colour = if clip { [0.95, 0.3, 0.25, 1.0] } else { [0.35, 0.75, 0.95, 1.0] };
            for y in y_of(hi)..=y_of(lo) {
                set(x, y, colour);
            }
        }
    }
    for x in 0..w {
        set(x, half, [0.3, 0.3, 0.33, 1.0]);
    }
    img
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod weighting {
    use super::*;

    #[test]
    fn at_48_khz_it_is_the_recommendations_own() {
        let [(b, a), (hb, ha)] = k_weighting(48_000.0);
        for (x, y) in b.iter().chain(&a).zip(SHELF_48K.0.iter().chain(&SHELF_48K.1)) {
            assert!((x - y).abs() < 1e-12, "{x} {y}");
        }
        assert_eq!(hb, [1.0, -2.0, 1.0]);
        for (x, y) in ha.iter().zip(&HIGH_PASS_48K) {
            assert!((x - y).abs() < 1e-12, "{x} {y}");
        }
    }

    #[test]
    fn the_shelf_lifts_highs_by_four_db_at_any_rate() {
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            let [(b, a), _] = k_weighting(rate);
            // the gain at a frequency, from the filter's response there
            let gain = |f: f64| {
                let w = std::f64::consts::TAU * f / rate;
                let z = |c: &[f64]| {
                    let (mut re, mut im) = (0.0, 0.0);
                    for (n, v) in c.iter().enumerate() {
                        re += v * (w * n as f64).cos();
                        im -= v * (w * n as f64).sin();
                    }
                    (re * re + im * im).sqrt()
                };
                20.0 * (z(&b) / z(&[1.0, a[0], a[1]])).log10()
            };
            assert!(gain(100.0).abs() < 0.1, "{rate}: {}", gain(100.0));
            assert!((gain(12_000.0) - 4.0).abs() < 0.2, "{rate}: {}", gain(12_000.0));
        }
    }
}
