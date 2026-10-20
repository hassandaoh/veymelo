use std::collections::HashMap;

use super::*;

/// Effect `id`'s settings, its defaults but for `set`.
fn run(id: &str, set: &[(&str, Value)], buf: &mut [f32], rate: u32, start: f64) {
    let def = crate::find(id).unwrap().def;
    let values: HashMap<String, Value> = set.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    let get = |k: &str| values.get(k).cloned();
    process(id, &Params { def, get: &get }, buf, rate, start);
}

fn n(v: f64) -> Value {
    Value::Number(v)
}

fn sine(freq: f64, sr: f64, frames: usize) -> Vec<f32> {
    (0..frames).flat_map(|i| {
        let v = (2.0 * PI * freq * i as f64 / sr).sin() as f32;
        [v, v]
    }).collect()
}

fn rms(b: &[f32], from: usize) -> f32 {
    let tail = &b[from * 2..];
    (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt()
}

#[test]
fn all_are_sound_effects() {
    for s in ALL {
        assert_eq!(s.category, CATEGORY);
        assert!(is_audio(s.def.id) && crate::find(s.def.id).is_some(), "{}", s.def.id);
    }
    assert!(!is_audio("gaussianBlur"));
    assert!(generates("tone") && reverses("backwards"));
}

#[test]
fn filters_let_through_what_they_should() {
    let sr = 48_000.0;
    let low = [("filter", Value::Choice(1)), ("cutoffFrequency", n(500.0))];
    let mut hi = sine(8000.0, sr, 4800);
    run("highLowPass", &low, &mut hi, 48_000, 0.0);
    let mut lo = sine(100.0, sr, 4800);
    run("highLowPass", &low, &mut lo, 48_000, 0.0);
    assert!(rms(&hi, 1000) < 0.01 && rms(&lo, 1000) > 0.6, "{} {}", rms(&hi, 1000), rms(&lo, 1000));
    let mut deep = sine(50.0, sr, 9600);
    run("highLowPass", &[("cutoffFrequency", n(500.0))], &mut deep, 48_000, 0.0);
    assert!(rms(&deep, 2000) < 0.02, "a high pass takes the lows away");
    // bass up raises 60 Hz, treble down lowers 10 kHz
    let tb = [("bass", n(100.0)), ("treble", n(-100.0))];
    let mut b = sine(60.0, sr, 9600);
    run("bassTreble", &tb, &mut b, 48_000, 0.0);
    assert!(rms(&b, 4000) > 0.707 * 3.0);
    let mut t = sine(10_000.0, sr, 9600);
    run("bassTreble", &tb, &mut t, 48_000, 0.0);
    assert!(rms(&t, 4000) < 0.707 * 0.4);
    // 12 dB up at a band's own frequency: four times as loud
    let mut e = sine(1000.0, sr, 9600);
    run("parametricEq", &[("band1BoostCut", n(12.0)), ("band1Frequency", n(1000.0))], &mut e, 48_000, 0.0);
    assert!((rms(&e, 4000) * std::f32::consts::SQRT_2 - 3.98).abs() < 0.2);
}

#[test]
fn echoes_and_rooms() {
    let mut b = vec![0.0f32; 2000];
    (b[0], b[1]) = (1.0, 1.0);
    run("delay", &[("delayTime", n(10.0)), ("feedback", n(0.0))], &mut b, 10_000, 0.0);
    // the dry 75 %, and 100 samples on the echo: wet 75 % of amount 50 %
    assert!((b[0] - 0.75).abs() < 1e-6 && (b[200] - 0.375).abs() < 1e-6 && b[2..200].iter().all(|v| *v == 0.0));
    let def = crate::find("delay").unwrap().def;
    let get = |_: &str| None;
    assert!(preroll("delay", &Params { def, get: &get }) >= 0.5, "half a second, and its echoes");
    let mut r = vec![0.0f32; 48_000];
    (r[0], r[1]) = (1.0, 1.0);
    run("reverb", &[], &mut r, 48_000, 0.0);
    assert!(r[24_000..].iter().any(|v| v.abs() > 1e-5), "a tail after a quarter of a second");
    assert!(r.iter().all(|v| v.is_finite() && v.abs() < 2.0));
}

#[test]
fn loudness_kept_in_hand() {
    // 10 dB over a −20 dB threshold at 4:1 comes out 7.5 dB down; under it, as it is
    assert!((compressor_gain_db(-10.0, -20.0, 4.0, 0.0) + 7.5).abs() < 1e-9);
    assert_eq!(compressor_gain_db(-30.0, -20.0, 4.0, 0.0), 0.0);
    assert!(compressor_gain_db(-23.0, -20.0, 4.0, 6.0).abs() < 1e-9, "the knee meets the straight line at its edges");
    let set = [("attack", n(1.0)), ("threshold", n(-20.0)), ("ratio", n(4.0)), ("knee", n(0.0)), ("release", n(100.0)), ("autoRelease", Value::Bool(false))];
    let mut loud = sine(1000.0, 48_000.0, 9600);
    run("compressor", &set, &mut loud, 48_000, 0.0);
    let peak = loud[18_000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!((peak - db_to_lin(-15.0) as f32).abs() < 0.05, "{peak}");
    let mut ceiling = sine(1000.0, 48_000.0, 4800);
    run("compressor", &[("outputLimit", n(-12.0)), ("makeupGain", n(20.0))], &mut ceiling, 48_000, 0.0);
    assert!(ceiling.iter().all(|v| v.abs() <= db_to_lin(-12.0) as f32 + 1e-6));
    // a gate shuts on quiet sound, opens on loud
    let mut quiet: Vec<f32> = sine(500.0, 48_000.0, 9600).iter().map(|v| v * 0.001).collect();
    run("gate", &[("threshold", n(-40.0))], &mut quiet, 48_000, 0.0);
    assert!(rms(&quiet, 4800) < 1e-6);
    let mut open: Vec<f32> = sine(500.0, 48_000.0, 9600).iter().map(|v| v * 0.5).collect();
    let before = rms(&open, 4800);
    run("gate", &[("threshold", n(-40.0))], &mut open, 48_000, 0.0);
    assert!((rms(&open, 4800) - before).abs() < 0.01);
}

#[test]
fn bent_mixed_and_made() {
    for k in 0..DISTORTIONS.len() as u32 {
        for x in [-2.0f32, -0.3, 0.0, 0.2, 1.7] {
            let y = distort(k, x, 0.5);
            assert!(y.abs() <= 1.0 && y.signum() * x.signum() >= 0.0, "{k} {x} {y}");
        }
    }
    assert!(distort(4, 0.1, 0.5) > -distort(4, -0.1, 0.5), "a tube bends one way more");
    let src: Vec<f32> = sine(300.0, 48_000.0, 480).iter().map(|v| v * 0.5).collect();
    let mut held = src.clone();
    run("distortion", &[("downsample", n(4.0))], &mut held, 48_000, 0.0);
    assert!(held[0] == held[6] && held[6] != held[8], "each sample held for four");
    // panning: the left channel moved right
    let mut b = vec![1.0, 0.0];
    run("stereoMixer", &[("leftPan", n(100.0))], &mut b, 48_000, 0.0);
    assert_eq!(b, vec![0.0, 1.0]);
    let mut same = vec![1.0, 0.0, 1.0, 0.0];
    run("stereoMixer", &[], &mut same, 48_000, 0.0);
    assert_eq!(same, vec![1.0, 0.0, 1.0, 0.0], "as it starts, it changes nothing");
    // a 1 kHz tone at a fifth of full
    let mut t = vec![0.0f32; 96_000];
    run("tone", &[("frequency1", n(1000.0)), ("frequency2", n(0.0)), ("frequency3", n(0.0)), ("frequency4", n(0.0)), ("frequency5", n(0.0))], &mut t, 48_000, 0.0);
    let left: Vec<f32> = t.iter().step_by(2).copied().collect();
    let up = left.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    assert!((up as i64 - 1000).abs() <= 1, "{up}");
    assert!((left.iter().fold(0.0f32, |m, v| m.max(v.abs())) - 0.2).abs() < 1e-3);
    // swapped channels
    let mut s = vec![1.0, 2.0];
    run("backwards", &[("swapChannels", Value::Bool(true))], &mut s, 48_000, 0.0);
    assert_eq!(s, vec![2.0, 1.0]);
    // vibrato and tremolo stay in bounds; tremolo dips
    let mut m = vec![1.0f32; 9600];
    run("modulator", &[("amplitudeModulation", n(100.0))], &mut m, 48_000, 0.0);
    assert!(m.iter().any(|v| *v < 0.05) && m.iter().any(|v| *v > 0.95));
    let mut f = sine(440.0, 48_000.0, 4800);
    run("flangeChorus", &[], &mut f, 48_000, 0.0);
    assert!(f.iter().all(|v| v.abs() <= 1.0 + 1e-4));
}
