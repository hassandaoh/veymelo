use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;

fn picture() -> Image {
    let mut img = Image::new(80, 60);
    for (i, p) in img.px.iter_mut().enumerate() {
        let (x, y) = ((i % 80) as f32 / 80.0, (i / 80) as f32 / 60.0);
        *p = [x, y, 1.0 - x, 1.0];
    }
    img
}

fn go(values: &[(&str, Value)], t: f64) -> Buf {
    let mut g = instance(&SHATTER, &[]);
    SHATTER.start(&mut g, Rect::from_size(80.0, 60.0));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(&SHATTER, &g, picture(), Tick::from_seconds(t), &[], None)
}

fn unlike(a: &Image, b: &Image) -> f32 {
    a.px.iter().zip(&b.px).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).fold(0f32, f32::max)).fold(0f32, f32::max)
}

#[test]
fn every_pattern_at_rest_is_the_layer() {
    for (k, name) in PATTERNS.iter().enumerate().take(PATTERNS.len() - 1) {
        let still = go(&[("pattern", Value::Choice(k as u32)), ("repetitions", n(6.0))], 0.0);
        let d = unlike(&still.img, &picture());
        assert!(d < 0.03, "{name}: {d}");
    }
}

#[test]
fn pieces_fly_where_the_force_reaches() {
    let later = go(&[], 1.0);
    assert!(unlike(&later.img, &picture()) > 0.3, "broken and moving");
    // the corners, out of the force's reach, stay
    let corner = |img: &Image| img.px[2 * 80 + 2];
    assert!((0..3).all(|k| (corner(&later.img)[k] - corner(&picture())[k]).abs() < 0.03));
    let left = go(&[("render", Value::Choice(1))], 1.0);
    assert!(left.img.px[30 * 80 + 40][3] < 0.1, "only what stayed: a hole in the middle");
    let none = go(&[("force1Radius", n(0.0))], 1.0);
    assert!(unlike(&none.img, &picture()) < 0.03, "no force, nothing breaks");
}

#[test]
fn a_gradient_holds_pieces() {
    // the layer as its own gradient, nothing allowed to break at a threshold of 0
    let held = go(&[("gradientLayer", Value::Layer(Some(1))), ("shatterThreshold", n(0.0))], 1.0);
    assert!(unlike(&held.img, &picture()) > 0.3, "no gradient layer to be found: the force alone");
}

#[test]
fn thick_pieces_and_lines() {
    let thick = go(&[("extrusionDepth", n(0.5)), ("sideMode", Value::Choice(0)), ("textureColor", Value::Color(veymelo_color::rgba(1.0, 1.0, 1.0, 1.0)))], 0.8);
    let thin = go(&[], 0.8);
    assert!(unlike(&thick.img, &thin.img) > 0.05, "sides drawn");
    let lines = go(&[("view", Value::Choice(1)), ("repetitions", n(4.0))], 0.0);
    let drawn = lines.img.px.iter().filter(|p| p[3] > 0.3).count();
    assert!(drawn > 200 && drawn < 80 * 60 / 2, "outlines: {drawn}");
    let forces = go(&[("view", Value::Choice(3))], 0.0);
    assert!(forces.img.px.iter().any(|p| p[0] > 0.9 && p[1] < 0.5), "the force's circle in red");
}

#[test]
fn registered() {
    assert!(crate::find("shatter").is_some_and(|s| s.category == "simulation"));
}
