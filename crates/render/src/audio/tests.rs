use super::*;
use veymelo_keyframe::{Keyframe, Value};
use veymelo_project::{Comp, Footage, ItemKind};
use veymelo_time::FrameRate;

const R: u32 = 48_000;

fn t(s: f64) -> Tick {
    Tick::from_seconds(s)
}

/// A solid making a tone of `hz` at a fifth of full (Tone's default level).
fn tone_layer(p: &mut Project, hz: f64, length: f64) -> Layer {
    let mut l = Layer::new(p.alloc(), "Tone", Source::Solid { color: veymelo_color::Rgba::WHITE, width: 10, height: 10 }, t(length));
    let mut g = Group::instance(veymelo_effects::find("tone").unwrap().def, p.alloc(), "Tone");
    g.prop_mut("frequency1").unwrap().set_value(Value::Number(hz)).unwrap();
    for i in 2..=5 {
        g.prop_mut(&format!("frequency{i}")).unwrap().set_value(Value::Number(0.0)).unwrap();
    }
    l.props.group_mut("effects").unwrap().children.push(Node::Group(g));
    l
}

fn comp_with(p: &mut Project, name: &str, layers: Vec<Layer>, length: f64) -> u64 {
    let mut c = Comp::new(64, 64, FrameRate::new(30, 1).unwrap(), t(length));
    c.layers = layers;
    p.add_comp(name, c)
}

fn left(s: &[f32]) -> Vec<f32> {
    s.iter().step_by(2).copied().collect()
}

fn ups(s: &[f32]) -> usize {
    s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
}

fn peak(s: &[f32]) -> f32 {
    s.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

#[test]
fn a_tone_and_a_composition_of_it() {
    let mut p = Project::new();
    let l = tone_layer(&mut p, 1000.0, 4.0);
    let inner = comp_with(&mut p, "Inner", vec![l], 4.0);
    assert!(heard(&p, inner));
    let s = mix(&p, inner, t(0.5), R as usize, R);
    assert!((ups(&left(&s)) as i64 - 1000).abs() <= 1 && (peak(&s) - 0.2).abs() < 1e-3);
    // shown inside another, as it is: the same sound
    let mut shown = Layer::new(p.alloc(), "Inner", Source::Comp(inner), t(4.0));
    let outer = comp_with(&mut p, "Outer", vec![shown.clone()], 4.0);
    let o = mix(&p, outer, t(0.5), R as usize, R);
    let worst = s.iter().zip(&o).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1e-5, "{worst}");
    // stretched twice as long: half the pitch
    shown.stretch = 200.0;
    shown.out_point = t(8.0);
    let slow = comp_with(&mut p, "Slow", vec![shown.clone()], 8.0);
    let s2 = mix(&p, slow, t(1.0), R as usize, R);
    assert!((ups(&left(&s2)) as i64 - 500).abs() <= 1, "{}", ups(&left(&s2)));
    // half as loud (−6 dB), and a fade to nothing with no steps in it
    shown.stretch = 100.0;
    shown.out_point = t(4.0);
    shown.props.prop_mut("audio/levels").unwrap().set_value(Value::Vec2([-6.0206, -6.0206])).unwrap();
    let quiet = comp_with(&mut p, "Quiet", vec![shown.clone()], 4.0);
    assert!((peak(&mix(&p, quiet, t(0.5), 4800, R)) - 0.1).abs() < 1e-3);
    shown.props.prop_mut("audio/levels").unwrap().set_keys(vec![Keyframe::new(t(0.0), Value::Vec2([0.0; 2])), Keyframe::new(t(1.0), Value::Vec2([-40.0; 2]))]).unwrap();
    let fade = comp_with(&mut p, "Fade", vec![shown], 4.0);
    // made of frames, as an export makes it: smooth all the same
    let mut whole = Vec::new();
    for f in 0..30 {
        whole.extend(mix(&p, fade, t(f as f64 / 30.0), 1600, R));
    }
    let tops: Vec<f32> = whole.chunks(96).map(peak).collect();
    assert!(tops.windows(2).all(|w| w[1] <= w[0] * 1.02 + 1e-4), "it only gets quieter");
    assert!(tops.windows(2).all(|w| (w[0] - w[1]).abs() < 0.02), "with no steps");
}

#[test]
fn who_is_heard() {
    let mut p = Project::new();
    let (mut a, mut b) = (tone_layer(&mut p, 440.0, 4.0), tone_layer(&mut p, 880.0, 4.0));
    a.in_point = t(1.0);
    let c = comp_with(&mut p, "C", vec![a.clone(), b.clone()], 4.0);
    // before a's in point only b; after, both
    assert!((ups(&left(&mix(&p, c, t(0.0), R as usize / 2, R))) as i64 - 440).abs() <= 1);
    assert!(peak(&mix(&p, c, t(2.0), R as usize, R)) > 0.3);
    // a soloed: only a; b's audio switched off: only a
    a.switches.solo = true;
    let solo = comp_with(&mut p, "Solo", vec![a.clone(), b.clone()], 4.0);
    assert!((ups(&left(&mix(&p, solo, t(2.0), R as usize, R))) as i64 - 440).abs() <= 1);
    a.switches.solo = false;
    b.switches.audio = false;
    let off = comp_with(&mut p, "Off", vec![a, b], 4.0);
    assert!((ups(&left(&mix(&p, off, t(2.0), R as usize, R))) as i64 - 440).abs() <= 1);
    let silent = comp_with(&mut p, "Silent", vec![], 1.0);
    assert!(!heard(&p, silent) && mix(&p, silent, t(0.0), 100, R).iter().all(|v| *v == 0.0));
}

#[test]
fn blocks_sound_the_same_as_one_long_run() {
    let mut p = Project::new();
    let mut l = tone_layer(&mut p, 330.0, 4.0);
    for (id, set) in [("delay", vec![("delayTime", 120.0), ("feedback", 60.0)]), ("reverb", vec![("decay", 60.0)])] {
        let mut g = Group::instance(veymelo_effects::find(id).unwrap().def, p.alloc(), id);
        for (k, v) in set {
            g.prop_mut(k).unwrap().set_value(Value::Number(v)).unwrap();
        }
        l.props.group_mut("effects").unwrap().children.push(Node::Group(g));
    }
    let c = comp_with(&mut p, "Echo", vec![l], 4.0);
    let one = mix(&p, c, t(1.0), R as usize, R);
    let mut pieces = Vec::new();
    let mut at = 0usize;
    for len in [1601usize, 1599, 1600, 7000, 36_200] {
        pieces.extend(mix(&p, c, t(1.0 + at as f64 / R as f64), len, R));
        at += len;
    }
    let worst = one.iter().zip(&pieces).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1e-4, "{worst}");
}

#[test]
fn loudness_as_broadcasters_measure_it() {
    // a 1 kHz sine at −23 dBFS in both channels reads −23 LUFS (EBU Tech 3341, the first case)
    let a = 10f64.powf(-23.0 / 20.0) as f32;
    let sine: Vec<f32> = (0..R as usize * 20).flat_map(|i| {
        let v = (std::f64::consts::TAU * 1000.0 * i as f64 / R as f64).sin() as f32 * a;
        [v, v]
    }).collect();
    let l = loudness(&sine, R);
    assert!((l.integrated.unwrap() + 23.0).abs() < 0.1, "{:?}", l.integrated);
    assert!((l.peak[0] + 23.0).abs() < 0.01 && (l.rms[0] + 26.01).abs() < 0.02, "{l:?}");
    // the same at 44.1 kHz
    let other: Vec<f32> = (0..44_100 * 10).flat_map(|i| {
        let v = (std::f64::consts::TAU * 1000.0 * i as f64 / 44_100.0).sin() as f32 * a;
        [v, v]
    }).collect();
    assert!((loudness(&other, 44_100).integrated.unwrap() + 23.0).abs() < 0.1);
    // silence has no loudness
    assert_eq!(loudness(&vec![0.0; 96_000], R).integrated, None);
    // a picture of it: the sine as a band, a clipped stretch red
    let mut loud = sine[..R as usize].to_vec();
    loud.iter_mut().take(4800).for_each(|v| *v *= 40.0);
    let img = waveform(&loud, 200, 80);
    assert_eq!((img.width, img.height), (200, 80));
    assert!(img.px[..200 * 40].iter().any(|p| p[0] > 0.9 && p[1] < 0.4), "clipping shows red");
}

#[cfg(target_os = "macos")]
#[test]
fn footage_sound_forwards_and_backwards() {
    // a movie whose left channel rises with time: its sample at t is t / 10
    let dir = std::env::temp_dir().join(format!("veymelo-render-audio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ramp.mov").to_string_lossy().into_owned();
    let mut v = veymelo_os::Video::new(&path, 16, 16, 10, 1, veymelo_os::Codec::ProRes(0), 1.0, 0, 0, Some(veymelo_os::Sound { rate: R, channels: 2, aac_bitrate: 0 })).unwrap();
    for f in 0..20 {
        v.frame(&vec![0u8; 16 * 16 * 4]).unwrap();
        let n = R as usize / 10;
        let s: Vec<f32> = (0..n).flat_map(|k| [((f * n + k) as f64 / R as f64 / 10.0) as f32, 0.0]).collect();
        v.sound(&s).unwrap();
    }
    v.finish().unwrap();
    let mut p = Project::new();
    let f = veymelo_media::probe(&path, false).unwrap();
    assert!(f.has_audio);
    let item = p.add("ramp", None, ItemKind::Footage(Footage { ..f }));
    let mut l = Layer::new(p.alloc(), "Ramp", Source::Footage(item), t(2.0));
    let c = comp_with(&mut p, "C", vec![l.clone()], 2.0);
    let s = mix(&p, c, t(0.5), 10, R);
    assert!((s[0] - 0.05).abs() < 2e-4, "at 0.5 s: {}", s[0]);
    // backwards: at 0.5 s it plays what is at 1.5 s
    l.props.group_mut("effects").unwrap().children.push(Node::Group(Group::instance(veymelo_effects::find("backwards").unwrap().def, p.alloc(), "Backwards")));
    let back = comp_with(&mut p, "Back", vec![l], 2.0);
    let b = mix(&p, back, t(0.5), 10, R);
    assert!((b[0] - 0.15).abs() < 2e-4, "backwards at 0.5 s: {}", b[0]);
    assert!(b[2] < b[0], "and falling");
}
