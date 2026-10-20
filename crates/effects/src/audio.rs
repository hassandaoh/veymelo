//! Audio effects: Backwards, Bass & Treble, Compressor, Delay, Distortion, Flange & Chorus,
//! Gate, High-Low Pass, Modulator, Parametric EQ, Reverb, Stereo Mixer and Tone.
//!
//! They are effects on a layer like any other (their settings animate), leave its pictures as
//! they are, and change its sound where the composition's sound is mixed (`veymelo-render`'s
//! `audio`), which previews and exports share. Sound is mixed in blocks; so that a block sounds
//! the same wherever it starts, each effect runs from silence over a little of the sound
//! before the block ([`preroll`]) and only the block is kept.
//!
//! The arithmetic is the textbook kind: second-order filters from the RBJ "Audio EQ Cookbook",
//! the reverb a Schroeder–Moorer network (low-passed feedback combs into all-passes, with the
//! published Freeverb tunings), chorus and vibrato fractional delay lines, the compressor and
//! gate feed-forward on a stereo-linked peak with one-pole smoothing and a quadratic soft knee.

use std::f64::consts::PI;

use veymelo_keyframe::Value;
use veymelo_project::GroupDef;

use crate::defs::{choice, effect, flag, num, pct, ranged};
use crate::{Buf, Ctx, Spec};

pub const CATEGORY: &str = "audio";

fn pass(_: &Ctx, b: Buf) -> Buf {
    b
}

/// Distortion's kinds.
pub const DISTORTIONS: &[&str] = &["softClip", "hardClip", "saturation1", "saturation2", "tube", "fuzz"];
pub const PASSES: &[&str] = &["highPass", "lowPass"];
pub const LFOS: &[&str] = &["sine", "triangle"];
pub const WAVES: &[&str] = &["sine", "triangle", "saw", "square", "whiteNoise"];

effect!(BACKWARDS_DEF, "backwards", "Backwards", "plays the layer's sound backwards", [flag("swapChannels", "Swap Channels", false)]);
effect!(
    BASS_TREBLE_DEF,
    "bassTreble",
    "Bass & Treble",
    "turns the lows (under about 200 Hz) and the highs (over about 4 kHz) up or down, up to 12 dB",
    [ranged("bass", "Bass", 0.0, -100.0, 100.0), ranged("treble", "Treble", 0.0, -100.0, 100.0)]
);
effect!(
    COMPRESSOR_DEF,
    "compressor",
    "Compressor",
    "evens out loud and quiet: sound over the threshold is turned down by the ratio",
    [
        ranged("threshold", "Threshold (dB)", -16.0, -60.0, 0.0),
        ranged("ratio", "Ratio (x:1)", 3.0, 0.4, 30.0),
        ranged("knee", "Knee (dB)", 15.0, 0.0, 30.0),
        ranged("attack", "Attack (ms)", 6.0, 0.0, 400.0),
        ranged("release", "Release (ms)", 440.0, 1.0, 4000.0),
        flag("autoRelease", "Auto Release", true).doc("short peaks let go quickly, long loud stretches slowly"),
        ranged("makeupGain", "Makeup Gain (dB)", 0.0, -30.0, 30.0),
        ranged("outputLimit", "Output Limit (dB)", 0.0, -30.0, 0.0).doc("never louder than this"),
    ]
);
effect!(
    DELAY_DEF,
    "delay",
    "Delay",
    "echoes: the sound again after the delay, each echo quieter by the feedback",
    [
        ranged("delayTime", "Delay Time (ms)", 500.0, 1.0, 10_000.0),
        pct("delayAmount", "Delay Amount", 50.0, 0.0, 100.0),
        pct("feedback", "Feedback", 50.0, 0.0, 100.0),
        pct("dryOut", "Dry Out", 75.0, 0.0, 100.0),
        pct("wetOut", "Wet Out", 75.0, 0.0, 100.0),
    ]
);
effect!(
    DISTORTION_DEF,
    "distortion",
    "Distortion",
    "bends the sound's shape: clipping, saturation, tube warmth, fuzz; fewer bits; held samples",
    [
        choice("type", "Type", DISTORTIONS, 0),
        pct("drive", "Drive", 25.0, 0.0, 100.0),
        ranged("gain", "Gain", 25.0, 0.0, 300.0).doc("before it is bent (25: as it is)"),
        pct("mix", "Mix", 100.0, 0.0, 100.0),
        ranged("volume", "Volume", 11.0, 0.0, 100.0).doc("after (11: as it is)"),
        ranged("bitDepth", "Resolution (bit)", 24.0, 1.0, 24.0),
        ranged("downsample", "Downsample (x)", 1.0, 1.0, 64.0),
    ]
);
effect!(
    FLANGE_CHORUS_DEF,
    "flangeChorus",
    "Flange & Chorus",
    "copies of the sound a moment late, the moment swaying: a flange (short) or a chorus (longer, more voices)",
    [
        ranged("voiceSeparationTime", "Voice Separation Time (ms)", 3.0, 0.0, 100.0),
        ranged("voices", "Voices", 1.0, 1.0, 8.0),
        ranged("modulationRate", "Modulation Rate", 0.4, 0.0, 10.0).doc("sways a second"),
        pct("modulationDepth", "Modulation Depth", 50.0, 0.0, 100.0),
        num("voicePhaseChange", "Voice Phase Change", 0.0).unit(veymelo_project::Unit::Degrees),
        flag("invertPhase", "Invert Phase", false),
        flag("stereoVoices", "Stereo Voices", false),
        pct("dryOut", "Dry Out", 50.0, 0.0, 100.0),
        pct("wetOut", "Wet Out", 50.0, 0.0, 100.0),
    ]
);
effect!(
    GATE_DEF,
    "gate",
    "Gate",
    "silences the sound while it is quieter than the threshold",
    [ranged("threshold", "Threshold (dB)", -60.0, -60.0, 0.0), ranged("attack", "Attack (ms)", 10.0, 0.0, 1000.0), ranged("hold", "Hold (ms)", 40.0, 0.0, 2500.0), ranged("release", "Release (ms)", 220.0, 5.0, 4000.0)]
);
effect!(
    HIGH_LOW_DEF,
    "highLowPass",
    "High-Low Pass",
    "lets through only the sound above (high pass) or below (low pass) the cutoff",
    [choice("filter", "Filter Options", PASSES, 0), ranged("cutoffFrequency", "Cutoff Frequency", 2000.0, 20.0, 20_000.0), pct("dryOut", "Dry Out", 0.0, 0.0, 100.0), pct("wetOut", "Wet Out", 100.0, 0.0, 100.0)]
);
effect!(
    MODULATOR_DEF,
    "modulator",
    "Modulator",
    "sways the sound's pitch (vibrato) and loudness (tremolo)",
    [choice("modulationType", "Modulation Type", LFOS, 0), ranged("modulationRate", "Modulation Rate", 10.0, 0.0, 100.0), pct("modulationDepth", "Modulation Depth", 5.0, 0.0, 100.0), pct("amplitudeModulation", "Amplitude Modulation", 0.0, 0.0, 100.0)]
);
effect!(
    EQ_DEF,
    "parametricEq",
    "Parametric EQ",
    "three bands, each turning the sound round its frequency up or down",
    [
        flag("band1Enable", "Band 1 Enabled", true),
        ranged("band1Frequency", "Band 1 Frequency", 200.0, 20.0, 20_000.0),
        ranged("band1Bandwidth", "Band 1 Bandwidth", 30.0, 1.0, 100.0),
        ranged("band1BoostCut", "Band 1 Boost/Cut", 0.0, -24.0, 24.0),
        flag("band2Enable", "Band 2 Enabled", false),
        ranged("band2Frequency", "Band 2 Frequency", 1000.0, 20.0, 20_000.0),
        ranged("band2Bandwidth", "Band 2 Bandwidth", 30.0, 1.0, 100.0),
        ranged("band2BoostCut", "Band 2 Boost/Cut", 0.0, -24.0, 24.0),
        flag("band3Enable", "Band 3 Enabled", false),
        ranged("band3Frequency", "Band 3 Frequency", 5000.0, 20.0, 20_000.0),
        ranged("band3Bandwidth", "Band 3 Bandwidth", 30.0, 1.0, 100.0),
        ranged("band3BoostCut", "Band 3 Boost/Cut", 0.0, -24.0, 24.0),
    ]
);
effect!(
    REVERB_DEF,
    "reverb",
    "Reverb",
    "the sound as in a room: echoes so many and close they blur into a tail",
    [
        ranged("reverbTime", "Reverb Time (ms)", 100.0, 1.0, 5000.0).doc("before the tail starts"),
        pct("diffusion", "Diffusion", 75.0, 0.0, 100.0),
        pct("decay", "Decay", 25.0, 0.0, 100.0).doc("how long the tail lasts"),
        pct("brightness", "Brightness", 10.0, 0.0, 100.0),
        pct("dryOut", "Dry Out", 90.0, 0.0, 100.0),
        pct("wetOut", "Wet Out", 10.0, 0.0, 100.0),
    ]
);
effect!(
    STEREO_MIXER_DEF,
    "stereoMixer",
    "Stereo Mixer",
    "each channel's level and where it sits from left to right",
    [
        ranged("leftLevel", "Left Level", 100.0, 0.0, 400.0),
        ranged("rightLevel", "Right Level", 100.0, 0.0, 400.0),
        ranged("leftPan", "Left Pan", -100.0, -100.0, 100.0),
        ranged("rightPan", "Right Pan", 100.0, -100.0, 100.0),
        flag("invertPhase", "Invert Phase", false),
    ]
);
effect!(
    TONE_DEF,
    "tone",
    "Tone",
    "makes a sound: up to five tones (sine, triangle, saw, square) or white noise; a layer with it is heard",
    [
        choice("waveform", "Waveform Options", WAVES, 0),
        ranged("frequency1", "Frequency 1", 440.0, 0.0, 22_000.0),
        ranged("frequency2", "Frequency 2", 493.88, 0.0, 22_000.0),
        ranged("frequency3", "Frequency 3", 587.33, 0.0, 22_000.0),
        ranged("frequency4", "Frequency 4", 659.26, 0.0, 22_000.0),
        ranged("frequency5", "Frequency 5", 783.99, 0.0, 22_000.0),
        pct("level", "Level", 20.0, 0.0, 100.0),
    ]
);

pub const ALL: [Spec; 13] = [
    Spec::new(&BACKWARDS_DEF, CATEGORY, pass),
    Spec::new(&BASS_TREBLE_DEF, CATEGORY, pass),
    Spec::new(&COMPRESSOR_DEF, CATEGORY, pass),
    Spec::new(&DELAY_DEF, CATEGORY, pass),
    Spec::new(&DISTORTION_DEF, CATEGORY, pass),
    Spec::new(&FLANGE_CHORUS_DEF, CATEGORY, pass),
    Spec::new(&GATE_DEF, CATEGORY, pass),
    Spec::new(&HIGH_LOW_DEF, CATEGORY, pass),
    Spec::new(&MODULATOR_DEF, CATEGORY, pass),
    Spec::new(&EQ_DEF, CATEGORY, pass),
    Spec::new(&REVERB_DEF, CATEGORY, pass),
    Spec::new(&STEREO_MIXER_DEF, CATEGORY, pass),
    Spec::new(&TONE_DEF, CATEGORY, pass),
];

/// Whether effect `id` changes sound (not pictures).
pub fn is_audio(id: &str) -> bool {
    ALL.iter().any(|s| s.def.id == id)
}

/// Whether effect `id` makes sound of its own (a layer without sound is heard with it).
pub fn generates(id: &str) -> bool {
    id == "tone"
}

/// Whether effect `id` plays its layer's sound backwards (the mixer reads it so).
pub fn reverses(id: &str) -> bool {
    id == "backwards"
}

/// An audio effect's settings at a moment: each read through `get`, or its default.
pub struct Params<'a> {
    pub def: &'static GroupDef,
    pub get: &'a dyn Fn(&str) -> Option<Value>,
}

impl Params<'_> {
    fn value(&self, id: &str) -> Value {
        if let Some(v) = (self.get)(id) {
            return v;
        }
        if let veymelo_project::Body::Fixed(nodes) = &self.def.body {
            for n in nodes.iter() {
                if let veymelo_project::DefNode::Prop(p) = n
                    && p.id == id
                {
                    return p.default.clone();
                }
            }
        }
        Value::Number(0.0)
    }

    pub fn f(&self, id: &str) -> f64 {
        let v = self.value(id).as_f64();
        if v.is_finite() { v } else { 0.0 }
    }

    pub fn b(&self, id: &str) -> bool {
        self.value(id).as_bool()
    }

    pub fn e(&self, id: &str) -> u32 {
        self.value(id).as_choice()
    }
}

const AUTO_FAST_MS: f64 = 60.0;
const AUTO_SLOW_ATTACK_MS: f64 = 150.0;
const AUTO_SLOW_MS: f64 = 900.0;

/// Seconds of sound before a block effect `id` needs to be as it would be after running on
/// and on.
pub fn preroll(id: &str, p: &Params) -> f64 {
    match id {
        "bassTreble" | "highLowPass" | "parametricEq" => 0.05,
        "flangeChorus" | "modulator" => 0.06,
        "distortion" => 0.01,
        // an envelope settles within a few of its time constants
        "compressor" | "gate" => {
            let auto = id == "compressor" && p.b("autoRelease");
            let release = if auto { AUTO_SLOW_MS } else { p.f("release").max(1.0) } / 1000.0;
            let attack = p.f("attack").max(0.0) / 1000.0;
            let hold = if id == "gate" { p.f("hold").max(0.0) / 1000.0 } else { 0.0 };
            (9.0 * release + 9.0 * attack + hold + 0.01).min(10.0)
        }
        "delay" => {
            let d = p.f("delayTime").max(1.0) / 1000.0;
            let fb = (p.f("feedback") / 100.0).clamp(0.0, 0.99);
            // echoes until they are a thousandth as loud
            let echoes = if fb > 1e-3 { (1e-3f64.ln() / fb.ln()).ceil() + 1.0 } else { 1.0 };
            (d * echoes).min(10.0)
        }
        "reverb" => {
            let (fb, _, _, pre) = reverb_settings(p);
            let t60 = if fb > 1e-3 { -3.0 * 0.0367 / fb.log10() } else { 0.05 };
            (t60 + pre).min(4.0)
        }
        _ => 0.0,
    }
}

/// Effect `id` on stereo sound `buf` (left, right, left…) at `rate`, from silence; `start` is
/// the layer time of its first sample, in seconds (where swaying and tones are in their cycle).
pub fn process(id: &str, p: &Params, buf: &mut [f32], rate: u32, start: f64) {
    if rate == 0 || buf.len() < 2 {
        return;
    }
    let sr = rate as f64;
    match id {
        "backwards" if p.b("swapChannels") => buf.as_chunks_mut::<2>().0.iter_mut().for_each(|f| f.swap(0, 1)),
        "bassTreble" => {
            let (bass, treble) = (p.f("bass") / 100.0 * 12.0, p.f("treble") / 100.0 * 12.0);
            if bass != 0.0 {
                Biquad::low_shelf(sr, 200.0, bass).run(buf);
            }
            if treble != 0.0 {
                Biquad::high_shelf(sr, 4000.0, treble).run(buf);
            }
        }
        "highLowPass" => {
            let fc = p.f("cutoffFrequency").clamp(1.0, sr * 0.49);
            let q = std::f64::consts::FRAC_1_SQRT_2;
            let f = if p.e("filter") == 1 { Biquad::low_pass(sr, fc, q) } else { Biquad::high_pass(sr, fc, q) };
            let dry = buf.to_vec();
            f.run(buf);
            dry_wet(buf, &dry, p.f("dryOut") / 100.0, p.f("wetOut") / 100.0);
        }
        "parametricEq" => {
            for n in 1..=3 {
                let gain = p.f(&format!("band{n}BoostCut"));
                if !p.b(&format!("band{n}Enable")) || gain == 0.0 {
                    continue;
                }
                let f = p.f(&format!("band{n}Frequency")).clamp(1.0, sr * 0.49);
                let octaves = (p.f(&format!("band{n}Bandwidth")) / 100.0 * 3.0).max(0.05);
                Biquad::peaking(sr, f, octaves, gain).run(buf);
            }
        }
        "delay" => delay(p, buf, sr),
        "compressor" => compressor(p, buf, sr),
        "gate" => gate(p, buf, sr),
        "distortion" => distortion(p, buf, sr, start),
        "flangeChorus" => flange_chorus(p, buf, sr, start),
        "modulator" => modulator(p, buf, sr, start),
        "reverb" => reverb(p, buf, sr),
        "stereoMixer" => stereo_mixer(p, buf),
        "tone" => tone(p, buf, sr, start),
        _ => {}
    }
}

fn dry_wet(buf: &mut [f32], dry: &[f32], d: f64, w: f64) {
    let (d, w) = (d as f32, w as f32);
    for (o, x) in buf.iter_mut().zip(dry) {
        *o = *o * w + x * d;
    }
}

/// A second-order filter, normalised (a0 = 1).
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    pub b: [f64; 3],
    pub a: [f64; 2],
}

impl Biquad {
    fn new(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Biquad {
        Biquad { b: [b0 / a0, b1 / a0, b2 / a0], a: [a1 / a0, a2 / a0] }
    }

    pub fn low_pass(sr: f64, f0: f64, q: f64) -> Biquad {
        let (s, c) = (2.0 * PI * f0 / sr).sin_cos();
        let al = s / (2.0 * q);
        Biquad::new((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al)
    }

    pub fn high_pass(sr: f64, f0: f64, q: f64) -> Biquad {
        let (s, c) = (2.0 * PI * f0 / sr).sin_cos();
        let al = s / (2.0 * q);
        Biquad::new((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al)
    }

    /// A peak `octaves` wide, `db` up or down.
    pub fn peaking(sr: f64, f0: f64, octaves: f64, db: f64) -> Biquad {
        let a = 10f64.powf(db / 40.0);
        let w = 2.0 * PI * f0 / sr;
        let (s, c) = w.sin_cos();
        let al = s * (2f64.ln() / 2.0 * octaves * w / s).sinh();
        Biquad::new(1.0 + al * a, -2.0 * c, 1.0 - al * a, 1.0 + al / a, -2.0 * c, 1.0 - al / a)
    }

    /// Shelves of slope 1.
    pub fn low_shelf(sr: f64, f0: f64, db: f64) -> Biquad {
        let a = 10f64.powf(db / 40.0);
        let (s, c) = (2.0 * PI * f0 / sr).sin_cos();
        let k = 2.0 * a.sqrt() * s / 2.0 * std::f64::consts::SQRT_2;
        Biquad::new(a * ((a + 1.0) - (a - 1.0) * c + k), 2.0 * a * ((a - 1.0) - (a + 1.0) * c), a * ((a + 1.0) - (a - 1.0) * c - k), (a + 1.0) + (a - 1.0) * c + k, -2.0 * ((a - 1.0) + (a + 1.0) * c), (a + 1.0) + (a - 1.0) * c - k)
    }

    pub fn high_shelf(sr: f64, f0: f64, db: f64) -> Biquad {
        let a = 10f64.powf(db / 40.0);
        let (s, c) = (2.0 * PI * f0 / sr).sin_cos();
        let k = 2.0 * a.sqrt() * s / 2.0 * std::f64::consts::SQRT_2;
        Biquad::new(a * ((a + 1.0) + (a - 1.0) * c + k), -2.0 * a * ((a - 1.0) + (a + 1.0) * c), a * ((a + 1.0) + (a - 1.0) * c - k), (a + 1.0) - (a - 1.0) * c + k, 2.0 * ((a - 1.0) - (a + 1.0) * c), (a + 1.0) - (a - 1.0) * c - k)
    }

    /// Run over both channels (direct form I).
    pub fn run(&self, buf: &mut [f32]) {
        for ch in 0..2 {
            let (mut x1, mut x2, mut y1, mut y2) = (0.0f64, 0.0, 0.0, 0.0);
            for i in (ch..buf.len()).step_by(2) {
                let x = buf[i] as f64;
                let y = self.b[0] * x + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
                (x2, x1, y2, y1) = (x1, x, y1, y);
                buf[i] = y as f32;
            }
        }
    }
}

/// Channel `ch` of `src` `d` samples (a fraction) before sample `i`, between the two nearest.
fn behind(src: &[f32], i: usize, ch: usize, d: f64) -> f32 {
    let at = i as f64 - d;
    if at < 0.0 {
        return 0.0;
    }
    let j = at.floor() as usize;
    let f = (at - j as f64) as f32;
    let a = src[j * 2 + ch];
    let b = if j < i { src[(j + 1) * 2 + ch] } else { a };
    a + (b - a) * f
}

fn delay(p: &Params, buf: &mut [f32], sr: f64) {
    let d = ((p.f("delayTime").max(0.0) / 1000.0 * sr).round() as usize).max(1);
    let amount = (p.f("delayAmount") / 100.0) as f32;
    let fb = (p.f("feedback") / 100.0).clamp(0.0, 0.99) as f32;
    let (dry, wet) = ((p.f("dryOut") / 100.0) as f32, (p.f("wetOut") / 100.0) as f32);
    let n = buf.len() / 2;
    // the echo line: e[i] = x[i - d] + fb · e[i - d]
    let mut e = vec![0.0f32; buf.len()];
    for i in d..n {
        for ch in 0..2 {
            e[i * 2 + ch] = buf[(i - d) * 2 + ch] + fb * e[(i - d) * 2 + ch];
        }
    }
    for (o, v) in buf.iter_mut().zip(&e) {
        *o = *o * dry + v * wet * amount;
    }
}

fn db_to_lin(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

fn lin_to_db(v: f64) -> f64 {
    20.0 * v.max(1e-12).log10()
}

/// The one-pole smoothing for a time constant of `ms`.
fn smoothing(ms: f64, sr: f64) -> f64 {
    if ms <= 0.0 { 0.0 } else { (-1.0 / (ms / 1000.0 * sr)).exp() }
}

/// How much (dB, at most 0) the compressor turns down sound at `x` dB: over the threshold by
/// the ratio, a quadratic knee `knee` dB wide round it.
pub fn compressor_gain_db(x: f64, threshold: f64, ratio: f64, knee: f64) -> f64 {
    // under 1:1 it widens instead
    let r = ratio.max(0.1);
    let over = x - threshold;
    let y = if knee > 0.0 && 2.0 * over.abs() <= knee {
        x + (1.0 / r - 1.0) * (over + knee / 2.0).powi(2) / (2.0 * knee)
    } else if over > 0.0 {
        threshold + over / r
    } else {
        x
    };
    y - x
}

fn compressor(p: &Params, buf: &mut [f32], sr: f64) {
    let (th, ratio, knee) = (p.f("threshold"), p.f("ratio").max(0.1), p.f("knee").max(0.0));
    let attack = smoothing(p.f("attack"), sr);
    let auto = p.b("autoRelease");
    let release = smoothing(if auto { AUTO_FAST_MS } else { p.f("release").max(0.001) }, sr);
    let (slow_attack, slow_release) = (smoothing(AUTO_SLOW_ATTACK_MS, sr), smoothing(AUTO_SLOW_MS, sr));
    let makeup = p.f("makeupGain");
    let ceiling = db_to_lin(p.f("outputLimit").min(0.0)) as f32;
    // the turn-down in dB, smoothed: attacking as it deepens, releasing as it eases; with auto
    // release a slow one too, the deeper of the two used (short peaks let go fast, long ones slowly)
    let (mut g, mut slow) = (0.0f64, 0.0f64);
    for f in buf.as_chunks_mut::<2>().0.iter_mut() {
        let peak = f[0].abs().max(f[1].abs()) as f64;
        let target = compressor_gain_db(lin_to_db(peak), th, ratio, knee);
        let k = if target < g { attack } else { release };
        g = target + (g - target) * k;
        let mut used = g;
        if auto {
            let k = if target < slow { slow_attack } else { slow_release };
            slow = target + (slow - target) * k;
            used = g.min(slow);
        }
        let lin = db_to_lin(used + makeup) as f32;
        for v in f.iter_mut() {
            *v = (*v * lin).clamp(-ceiling, ceiling);
        }
    }
}

fn gate(p: &Params, buf: &mut [f32], sr: f64) {
    let th = db_to_lin(p.f("threshold"));
    let (attack, release) = (smoothing(p.f("attack"), sr), smoothing(p.f("release").max(0.001), sr));
    let hold = (p.f("hold").max(0.0) / 1000.0 * sr) as usize;
    // how open it is, 0 to 1: the sound's gain
    let (mut g, mut held) = (0.0f64, 0usize);
    for f in buf.as_chunks_mut::<2>().0.iter_mut() {
        let peak = f[0].abs().max(f[1].abs()) as f64;
        let open = if peak >= th {
            held = hold;
            true
        } else if held > 0 {
            held -= 1;
            true
        } else {
            false
        };
        let target = if open { 1.0 } else { 0.0 };
        let k = if target > g { attack } else { release };
        g = target + (g - target) * k;
        f[0] *= g as f32;
        f[1] *= g as f32;
    }
}

/// One sample through Distortion's curve `kind`, bent harder the more `drive` (0 to 1); never
/// past ±1.
pub fn distort(kind: u32, x: f32, drive: f32) -> f32 {
    let v = x * (1.0 + drive.clamp(0.0, 1.0) * 24.0);
    match kind {
        1 => v,
        2 => v / (1.0 + v.abs()),
        3 => {
            let c = v.clamp(-1.5, 1.5);
            c - c * c * c * 4.0 / 27.0
        }
        // tube: the upper half bent harder than the lower
        4 => {
            if v >= 0.0 {
                v.tanh()
            } else {
                (0.6 * v).tanh()
            }
        }
        5 => v.signum() * (1.0 - (-3.0 * v.abs()).exp()),
        _ => v.tanh(),
    }
    .clamp(-1.0, 1.0)
}

fn distortion(p: &Params, buf: &mut [f32], sr: f64, start: f64) {
    let kind = p.e("type");
    let drive = (p.f("drive") / 100.0) as f32;
    let pre = (p.f("gain").max(0.0) / 25.0) as f32;
    let mix = (p.f("mix") / 100.0).clamp(0.0, 1.0) as f32;
    let volume = (p.f("volume").max(0.0) / 11.0) as f32;
    let bits = p.f("bitDepth").round().clamp(1.0, 24.0);
    let steps = (bits < 24.0).then(|| 2f32.powf(bits as f32 - 1.0));
    let down = p.f("downsample").round().clamp(1.0, 64.0) as i64;
    // samples held by where they are in the layer, so blocks agree with one long run
    let first = (start * sr).round() as i64;
    let mut held = [0.0f32; 2];
    for (i, f) in buf.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let take = i == 0 || (first + i as i64).rem_euclid(down) == 0;
        for ch in 0..2 {
            let x = f[ch];
            let mut y = distort(kind, x * pre, drive);
            if let Some(q) = steps {
                y = (y * q).round() / q;
            }
            if take {
                held[ch] = y;
            }
            f[ch] = (x * (1.0 - mix) + held[ch] * mix) * volume;
        }
    }
}

fn sway(kind: u32, phase: f64) -> f64 {
    let ph = phase.rem_euclid(1.0);
    if kind == 1 { 1.0 - 4.0 * (ph - 0.5).abs() } else { (2.0 * PI * ph).sin() }
}

fn flange_chorus(p: &Params, buf: &mut [f32], sr: f64, start: f64) {
    let sep = p.f("voiceSeparationTime").max(0.0) / 1000.0 * sr;
    let voices = p.f("voices").round().clamp(1.0, 8.0) as usize;
    let rate = p.f("modulationRate").max(0.0);
    let depth = (p.f("modulationDepth") / 100.0).clamp(0.0, 1.0);
    let phase = p.f("voicePhaseChange") / 360.0;
    let invert = if p.b("invertPhase") { -1.0 } else { 1.0 };
    let stereo = p.b("stereoVoices");
    let (dry, wet) = ((p.f("dryOut") / 100.0) as f32, (p.f("wetOut") / 100.0) as f32);
    let src = buf.to_vec();
    for i in 0..buf.len() / 2 {
        let t = start + i as f64 / sr;
        let (mut acc, mut count) = ([0.0f32; 2], [0usize; 2]);
        for v in 0..voices {
            let d = (sep * (v + 1) as f64 * (1.0 + depth * sway(0, rate * t + phase * v as f64))).max(0.0);
            for ch in 0..2 {
                if stereo && voices > 1 && v % 2 != ch {
                    continue;
                }
                acc[ch] += behind(&src, i, ch, d);
                count[ch] += 1;
            }
        }
        for ch in 0..2 {
            let w = if count[ch] > 0 { acc[ch] / count[ch] as f32 } else { 0.0 };
            buf[i * 2 + ch] = src[i * 2 + ch] * dry + invert * w * wet;
        }
    }
}

fn modulator(p: &Params, buf: &mut [f32], sr: f64, start: f64) {
    let kind = p.e("modulationType");
    let rate = p.f("modulationRate").max(0.0);
    // vibrato: a swaying delay of up to 5 ms; tremolo: a swaying gain
    let depth = (p.f("modulationDepth") / 100.0).clamp(0.0, 1.0) * 0.005 * sr;
    let am = (p.f("amplitudeModulation") / 100.0).clamp(0.0, 1.0);
    let src = buf.to_vec();
    for i in 0..buf.len() / 2 {
        let l = sway(kind, rate * (start + i as f64 / sr));
        let d = depth * (1.0 + l) * 0.5;
        let g = (1.0 - am * (1.0 - l) * 0.5) as f32;
        for ch in 0..2 {
            let x = if depth > 0.0 { behind(&src, i, ch, d) } else { src[i * 2 + ch] };
            buf[i * 2 + ch] = x * g;
        }
    }
}

/// The comb feedback, damping, all-pass gain and pre-delay (seconds).
fn reverb_settings(p: &Params) -> (f64, f64, f64, f64) {
    let decay = (p.f("decay") / 100.0).clamp(0.0, 1.0);
    let damp = (1.0 - p.f("brightness") / 100.0).clamp(0.0, 1.0) * 0.6;
    let ap = 0.3 + 0.4 * (p.f("diffusion") / 100.0).clamp(0.0, 1.0);
    (0.7 + decay * 0.28, damp, ap, p.f("reverbTime").max(0.0) / 1000.0)
}

fn reverb(p: &Params, buf: &mut [f32], sr: f64) {
    // Freeverb's lengths (at 44.1 kHz), the right channel a little longer
    const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
    const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
    const SPREAD: usize = 23;
    let (fb, damp, apg, pre) = reverb_settings(p);
    let (dry, wet) = ((p.f("dryOut") / 100.0) as f32, (p.f("wetOut") / 100.0) as f32);
    let k = sr / 44_100.0;
    let pre = (pre * sr).round() as usize;
    let n = buf.len() / 2;
    let src = buf.to_vec();
    for ch in 0..2 {
        let spread = if ch == 1 { SPREAD } else { 0 };
        let line = |l: usize| vec![0.0f64; (((l + spread) as f64 * k) as usize).max(1)];
        let mut combs: Vec<(Vec<f64>, usize, f64)> = COMBS.iter().map(|&l| (line(l), 0, 0.0)).collect();
        let mut alls: Vec<(Vec<f64>, usize)> = ALLPASSES.iter().map(|&l| (line(l), 0)).collect();
        for i in 0..n {
            let x = if i >= pre { (src[(i - pre) * 2] as f64 + src[(i - pre) * 2 + 1] as f64) * 0.5 * 0.03 } else { 0.0 };
            let mut out = 0.0;
            for (l, at, low) in combs.iter_mut() {
                let y = l[*at];
                *low = y * (1.0 - damp) + *low * damp;
                l[*at] = x + *low * fb;
                *at = (*at + 1) % l.len();
                out += y;
            }
            for (l, at) in alls.iter_mut() {
                let b = l[*at];
                let y = -out + b;
                l[*at] = out + b * apg;
                *at = (*at + 1) % l.len();
                out = y;
            }
            buf[i * 2 + ch] = src[i * 2 + ch] * dry + out as f32 * wet * 3.0;
        }
    }
}

fn stereo_mixer(p: &Params, buf: &mut [f32]) {
    let (ll, rl) = ((p.f("leftLevel") / 100.0) as f32, (p.f("rightLevel") / 100.0) as f32);
    // a straight pan: −100 all left, 0 both full, +100 all right
    let pan = |v: f64| -> (f32, f32) {
        let v = (v / 100.0).clamp(-1.0, 1.0);
        ((1.0 - v).min(1.0) as f32, (1.0 + v).min(1.0) as f32)
    };
    let ((l_l, l_r), (r_l, r_r)) = (pan(p.f("leftPan")), pan(p.f("rightPan")));
    let invert = if p.b("invertPhase") { -1.0 } else { 1.0 };
    for f in buf.as_chunks_mut::<2>().0.iter_mut() {
        let (l, r) = (f[0] * ll, f[1] * rl);
        *f = [(l * l_l + r * r_l) * invert, (l * l_r + r * r_r) * invert];
    }
}

fn tone(p: &Params, buf: &mut [f32], sr: f64, start: f64) {
    let kind = p.e("waveform");
    let level = (p.f("level") / 100.0) as f32;
    let freqs: Vec<f64> = (1..=5).map(|i| p.f(&format!("frequency{i}"))).filter(|f| *f > 0.0).collect();
    for i in 0..buf.len() / 2 {
        let t = start + i as f64 / sr;
        let mut v = 0.0f64;
        for f in &freqs {
            let ph = (f * t).rem_euclid(1.0);
            v += match kind {
                1 => sway(1, ph - 0.25),
                2 => 2.0 * ph - 1.0,
                3 => {
                    if ph < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                // white noise: from where the sample is, the same each time
                4 => {
                    let h = ((t * sr).round() as i64 as u64 ^ f.to_bits()).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                    let h = (h ^ (h >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                    ((h ^ (h >> 32)) >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
                }
                _ => (2.0 * PI * ph).sin(),
            };
        }
        let v = v as f32 * level;
        buf[i * 2] += v;
        buf[i * 2 + 1] += v;
    }
}

#[cfg(test)]
mod tests;
