use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_time::Tick;

fn picture() -> Image {
    let mut img = Image::new(60, 40);
    for (i, p) in img.px.iter_mut().enumerate() {
        let (x, y) = ((i % 60) as f32 / 60.0, (i / 60) as f32 / 40.0);
        *p = [x, y, 1.0 - x, 1.0];
    }
    img
}

fn go(spec: &Spec, values: &[(&str, Value)], host: Option<&dyn crate::Host>) -> Buf {
    let img = picture();
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(60.0, 40.0));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, &[], host)
}

fn unlike(a: &Image, b: &Image) -> f32 {
    a.px.iter().zip(&b.px).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).fold(0f32, f32::max)).fold(0f32, f32::max)
}

#[test]
fn cards_at_rest_are_the_layer() {
    let still = go(&DANCE, &[("ambientLight", n(0.0)), ("diffuseReflection", n(1.0)), ("lightType", Value::Choice(2))], None);
    assert!(unlike(&still.img, &picture()) < 0.02, "{}", unlike(&still.img, &picture()));
    // turned by the layer's brightness: the cards lean, and some show gaps
    let turned = go(&DANCE, &[("yRotationSource", Value::Choice(1)), ("yRotationMultiplier", n(80.0))], None);
    assert!(unlike(&turned.img, &picture()) > 0.2);
    assert!(turned.img.px.iter().any(|p| p[3] < 0.5), "gaps between leaning cards");
    // pushed back by red: smaller, toward the middle
    let back = go(&DANCE, &[("zPositionSource", Value::Choice(2)), ("zPositionMultiplier", n(3.0))], None);
    assert!(back.img.px[0][3] < 0.5 || back.img.px[59][3] < 0.5, "a far card leaves its corner");
}

#[test]
fn cards_flip_over() {
    let none = go(&WIPE, &[("lightType", Value::Choice(2)), ("ambientLight", n(0.0)), ("diffuseReflection", n(1.0))], None);
    assert!(unlike(&none.img, &picture()) < 0.02, "not started: the layer");
    let half = go(&WIPE, &[("transitionCompletion", n(50.0)), ("columns", n(6.0))], None);
    assert!(unlike(&half.img, &picture()) > 0.2, "turning");
    let done = go(&WIPE, &[("transitionCompletion", n(100.0)), ("lightType", Value::Choice(2)), ("ambientLight", n(0.0)), ("diffuseReflection", n(1.0))], None);
    assert!(unlike(&done.img, &picture()) < 0.02, "turned over, the back (the layer again, the right way up)");
    let sideways = go(&WIPE, &[("transitionCompletion", n(100.0)), ("flipAxis", Value::Choice(1)), ("lightType", Value::Choice(2)), ("ambientLight", n(0.0)), ("diffuseReflection", n(1.0))], None);
    assert!(unlike(&sideways.img, &picture()) < 0.02, "about y too");
}

/// A back layer of plain red.
struct Red;

impl crate::Host for Red {
    fn layer(&self, _: u64, _: crate::Take) -> Option<(Buf, Rect)> {
        Some((crate::testing::buf(Image::filled(60, 40, [1.0, 0.0, 0.0, 1.0])), Rect::from_size(60.0, 40.0)))
    }
}

#[test]
fn the_back_layer_shows_on_the_backs() {
    let done = go(&WIPE, &[("transitionCompletion", n(100.0)), ("backLayer", Value::Layer(Some(9))), ("lightType", Value::Choice(2)), ("ambientLight", n(0.0)), ("diffuseReflection", n(1.0))], Some(&Red));
    assert!(done.img.px.iter().all(|p| p[0] > 0.97 && p[1] < 0.03), "all red");
    let gradual = go(&WIPE, &[("transitionCompletion", n(50.0)), ("transitionWidth", n(10.0)), ("backLayer", Value::Layer(Some(9)))], Some(&Red));
    let (left, right) = (gradual.img.px[20 * 60 + 3], gradual.img.px[20 * 60 + 56]);
    assert!(left[0] > 0.9 && left[1] < 0.1 && right[1] > 0.4, "left to right: done on the left, not yet on the right: {left:?} {right:?}");
}

#[test]
fn every_one_is_registered() {
    assert!(crate::find("cardDance").is_some_and(|s| s.category == "simulation"));
    assert!(crate::find("cardWipe").is_some_and(|s| s.category == "transition"));
}
