use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;

/// A white square 12 across in the middle of a clear 24 × 24 layer.
fn square() -> Image {
    let mut img = Image::new(24, 24);
    for y in 6..18 {
        for x in 6..18 {
            img.px[y * 24 + x] = [1.0; 4];
        }
    }
    img
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, &[], None)
}

fn area(b: &Buf) -> f32 {
    b.img.px.iter().map(|p| p[3]).sum()
}

#[test]
fn choking_round() {
    let img = square();
    let shrunk = go(&SIMPLE_CHOKER, &[("chokeMatte", n(2.0))], img.clone());
    assert!((area(&shrunk) - 64.0).abs() < 4.0, "two in from each side: {}", area(&shrunk));
    let grown = go(&SIMPLE_CHOKER, &[("chokeMatte", n(-2.0))], img.clone());
    assert!(area(&grown) > 144.0 + 40.0);
    // grown round: the corners rounded, not square
    let pad = (grown.img.width - 24) / 2;
    let at = |x: usize, y: usize| grown.img.px[(y + pad) * grown.img.width + x + pad][3];
    assert_eq!(at(5, 12), 1.0);
    assert_eq!(at(4, 4), 0.0, "the corner two out diagonally is past a round brush");
    let half = go(&SIMPLE_CHOKER, &[("chokeMatte", n(1.5))], img.clone());
    assert!(area(&half) > area(&shrunk) && area(&half) < 144.0, "part pixels count");
    let view = go(&SIMPLE_CHOKER, &[("view", Value::Choice(1))], img.clone());
    assert!(view.img.px.iter().all(|p| p[3] == 1.0 && p[0] == p[1]));
    assert_eq!(disc_extreme(&[0.0, 0.0, 1.0, 0.0, 0.0], 5, 1, 1, true), vec![0.0, 1.0, 1.0, 1.0, 0.0]);
}

#[test]
fn matte_choker_cleans() {
    let mut img = square();
    // a hole and a speck
    img.px[12 * 24 + 12] = [0.0; 4];
    img.px[2 * 24 + 2] = [1.0; 4];
    let out = go(&MATTE_CHOKER, &[("choke1", n(0.0)), ("grayLevelSoftness1", n(50.0))], img.clone());
    let pad = (out.img.width - 24) / 2;
    let at = |x: usize, y: usize| out.img.px[(y + pad) * out.img.width + x + pad][3];
    assert!(at(12, 12) > 0.9, "the hole filled: {}", at(12, 12));
    assert!(at(2, 2) < 0.1, "the speck gone: {}", at(2, 2));
}

#[test]
fn refining_follows_the_picture() {
    // the picture: white on the left, black on the right; a rough matte that ends a little past
    // the white
    let mut img = Image::new(32, 16);
    for y in 0..16 {
        for x in 0..32 {
            let v = if x < 14 { 1.0 } else { 0.0 };
            let a = if x < 18 { 1.0 } else { 0.0 };
            img.px[y * 32 + x] = [v * a, v * a, v * a, a];
        }
    }
    let out = go(&REFINE_HARD, &[("decontaminateEdgeColors", Value::Bool(false))], img.clone());
    assert!(out.img.px[8 * 32 + 5][3] > 0.95, "inside stays");
    assert!(out.img.px[8 * 32 + 28][3] < 0.05, "outside stays");
    let soft = go(&REFINE_SOFT, &[], img.clone());
    assert_ne!(soft.img.px, img.px);
    let flipped = go(&REFINE_SOFT, &[("invert", Value::Bool(true))], img.clone());
    assert!(flipped.img.px[8 * 32 + 5][3] < 0.1);
    let edges = go(&REFINE_SOFT, &[("viewEdgeRegion", Value::Bool(true))], img.clone());
    assert!(edges.img.px[8 * 32 + 17][1] < edges.img.px[8 * 32 + 17][0], "the edge band tinted red");
    let grown = go(&REFINE_SOFT, &[("shiftEdge", n(60.0)), ("feather", n(0.0)), ("calculateEdgeDetails", Value::Bool(false))], img.clone());
    let base = go(&REFINE_SOFT, &[("feather", n(0.0)), ("calculateEdgeDetails", Value::Bool(false))], img.clone());
    assert!(area(&grown) >= area(&base));
    // the background's colour taken out of a see-through edge
    let mut fringe = Image::filled(16, 16, [1.0, 0.0, 0.0, 1.0]);
    for y in 0..16 {
        for x in 8..16 {
            fringe.px[y * 16 + x] = if x == 8 { [0.5 * 0.5, 0.5 * 0.5, 0.5 * 0.5, 0.5] } else { [0.0; 4] };
        }
    }
    let clean = go(&REFINE_SOFT, &[("calculateEdgeDetails", Value::Bool(false)), ("feather", n(0.0))], fringe.clone());
    let p = clean.img.px[8 * 16 + 8];
    assert!(p[3] > 0.0 && p[0] / p[3] > 0.6, "the edge takes the red of what is solid beside it: {p:?}");
    let map = go(&REFINE_SOFT, &[("viewDecontaminationMap", Value::Bool(true))], fringe);
    assert!(map.img.px[8 * 16 + 8][0] > 0.4 && map.img.px[8 * 16 + 2][0] < 0.1, "{:?} {:?}", map.img.px[8 * 16 + 8], map.img.px[8 * 16 + 2]);
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "matte"), "{}", spec.def.id);
    }
}
