use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;

fn go(spec: &Spec, values: &[(&str, Value)], img: Image, t: f64) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::from_seconds(t), &[], None)
}

/// How much is drawn, and the middle of it.
fn mass(img: &Image) -> (f32, f32, f32) {
    let (mut m, mut sx, mut sy) = (0f32, 0f32, 0f32);
    for (i, p) in img.px.iter().enumerate() {
        m += p[3];
        sx += p[3] * (i % img.width) as f32;
        sy += p[3] * (i / img.width) as f32;
    }
    (m, sx / m.max(1e-9), sy / m.max(1e-9))
}

#[test]
fn kept_states_change_nothing() {
    let kept: Stepped<Swarm> = Stepped::new();
    let p = Physics { rate: 300.0, life: 1.0, at: [0.0; 3], radius: [0.1; 3], motion: 9, speed: 0.5, gravity: [0.0, 0.5, 0.0], resistance: 0.1, axis: [0.0, -1.0, 0.0], extra: 0.5, cone: 30.0, floor: Some((0.3, 1, 0.5)), seed: 5, flat: false, most: 10_000 };
    let fresh = |n: u64| {
        let mut s = Swarm::default();
        (0..n).for_each(|k| p.step(&mut s, k));
        s
    };
    let a = kept.run(1, 100, Swarm::default, |s, k| p.step(s, k));
    let b = kept.run(1, 47, Swarm::default, |s, k| p.step(s, k));
    let c = kept.run(1, 100, Swarm::default, |s, k| p.step(s, k));
    for (got, n) in [(&a, 100), (&b, 47), (&c, 100)] {
        let want = fresh(n);
        assert_eq!(got.parts.len(), want.parts.len(), "{n}");
        assert!(got.parts.iter().zip(&want.parts).all(|(x, y)| x.p == y.p && x.id == y.id), "{n}");
    }
    assert!(a.parts.iter().all(|q| q.p[1] <= 0.3 + 1e-6), "the floor holds");
}

#[test]
fn particle_world_flies() {
    let blank = Image::filled(80, 60, [0.2, 0.2, 0.2, 1.0]);
    let start = go(&WORLD_SPEC, &[], blank.clone(), 0.0);
    assert!(start.img.px.iter().all(|p| p[3] == 0.0), "nothing born yet, the layer replaced");
    let v = [("birthRate", n(1.0)), ("particleType", Value::Choice(1)), ("birthSize", n(0.05)), ("deathSize", n(0.05))];
    let a = go(&WORLD_SPEC, &v, blank.clone(), 0.5);
    assert_eq!(a.img.px, go(&WORLD_SPEC, &v, blank.clone(), 0.5).img.px, "the same each time");
    let (m0, _, y0) = mass(&a.img);
    assert!(m0 > 1.0, "particles drawn: {m0}");
    let (_, _, y1) = mass(&go(&WORLD_SPEC, &v, blank.clone(), 0.9).img);
    assert!(y1 > y0, "falling: {y0} then {y1}");
    // nearer the camera, bigger
    let near = go(&WORLD_SPEC, &[v[0].clone(), v[1].clone(), v[2].clone(), v[3].clone(), ("cameraDistance", n(1.0))], blank.clone(), 0.5);
    assert!(mass(&near.img).0 > m0 * 1.5);
    let fogged = go(&WORLD_SPEC, &[v[0].clone(), v[1].clone(), v[2].clone(), v[3].clone(), ("depthCue", Value::Choice(1)), ("depthCueDistance", n(0.01))], blank, 0.5);
    assert!(mass(&fogged.img).0 < m0, "faded with depth");
}

#[test]
fn particle_systems_steer() {
    let blank = Image::new(120, 80);
    let right = [("animation", Value::Choice(1)), ("direction", n(90.0)), ("gravity", n(0.0)), ("birthRate", n(4.0)), ("particleType", Value::Choice(2)), ("birthSize", n(0.6)), ("deathSize", n(0.6))];
    let (m, x, _) = mass(&go(&SYSTEMS_SPEC, &right, blank.clone(), 0.8).img);
    assert!(m > 1.0 && x > 62.0, "blown right of the producer: {x}");
    let up = [("animation", Value::Choice(1)), ("direction", n(0.0)), ("gravity", n(0.0)), ("birthRate", n(4.0)), ("particleType", Value::Choice(2)), ("birthSize", n(0.6)), ("deathSize", n(0.6))];
    let (_, _, y) = mass(&go(&SYSTEMS_SPEC, &up, blank.clone(), 0.8).img);
    assert!(y < 38.0, "up: {y}");
    let added = go(&SYSTEMS_SPEC, &[right[0].clone(), right[1].clone(), ("transferMode", Value::Choice(2))], blank.clone(), 0.8);
    assert!(added.img.px.iter().any(|p| p[3] > 0.0));
    let black = go(&SYSTEMS_SPEC, &[("transferMode", Value::Choice(3))], blank, 0.8);
    assert!(black.img.px.iter().all(|p| p[3] == 1.0), "on black");
}

#[test]
fn mercury_runs_together() {
    let mut img = Image::new(80, 80);
    for (i, p) in img.px.iter_mut().enumerate() {
        let v = (i % 80) as f32 / 80.0;
        *p = [v, 0.5, 1.0 - v, 1.0];
    }
    let a = go(&MERCURY_SPEC, &[("birthRate", n(4.0)), ("blobBirthSize", n(1.0)), ("blobDeathSize", n(1.0))], img.clone(), 1.0);
    assert_eq!(a.img.px, go(&MERCURY_SPEC, &[("birthRate", n(4.0)), ("blobBirthSize", n(1.0)), ("blobDeathSize", n(1.0))], img.clone(), 1.0).img.px);
    let (m, _, _) = mass(&a.img);
    assert!(m > 20.0, "drops: {m}");
    assert!(a.img.px.iter().any(|p| p[3] == 0.0), "clear between them");
    assert!(go(&MERCURY_SPEC, &[], img, 0.0).img.px.iter().all(|p| p[3] == 0.0), "none at the start");
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "simulation"), "{}", spec.def.id);
    }
}
