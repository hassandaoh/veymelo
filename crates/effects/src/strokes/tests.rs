use veymelo_keyframe::{ShapePath, Value};
use veymelo_project::{Group, Node};
use veymelo_raster::Image;
use veymelo_time::Tick;

use super::*;
use crate::testing::{at, halves, instance, run_when};

/// A stroke of `kind` along `points`, with `values` set (by path).
fn stroke(kind: StrokeKind, points: &[[f64; 2]], values: &[(&str, Value)]) -> Group {
    let mut g = Group::instance(kind.def(), 2, kind.def().name);
    let path = ShapePath { points: points.to_vec(), in_tangents: vec![[0.0; 2]; points.len()], out_tangents: vec![[0.0; 2]; points.len()], closed: false, feather: Default::default() };
    g.prop_mut("path").unwrap().set_value(Value::Path(path)).unwrap();
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
    g
}

/// Paint with `strokes` (and `values` of its own) at layer time `t` on `img`.
fn painted(strokes: Vec<Group>, values: &[(&str, Value)], img: Image, t: f64) -> Buf {
    let mut g = instance(&PAINT, values);
    g.group_mut("strokes").unwrap().children.extend(strokes.into_iter().map(Node::Group));
    run_when(&PAINT, &g, img, Tick::from_seconds(t), &[], None)
}

fn red() -> Value {
    Value::Color(veymelo_color::Rgba { r: 1.0, g: 0.0, b: 0.0, a: 1.0 })
}

const ACROSS: &[[f64; 2]] = &[[10.0, 30.0], [90.0, 30.0]];

#[test]
fn a_brush_paints_along_its_path() {
    let b = painted(vec![stroke(StrokeKind::Brush, ACROSS, &[("options/color", red())])], &[], Image::new(100, 60), 0.0);
    assert_eq!(at(&b, 50.0, 30.0), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(at(&b, 50.0, 33.0)[3], 1.0, "inside its 10 pixels");
    assert_eq!(at(&b, 50.0, 37.0)[3], 0.0, "outside them");
    assert_eq!(at(&b, 4.0, 30.0)[3], 0.0, "not before its start");
    // half of it drawn: the first half
    let b = painted(vec![stroke(StrokeKind::Brush, ACROSS, &[("options/color", red()), ("options/end", Value::Number(50.0))])], &[], Image::new(100, 60), 0.0);
    assert_eq!(at(&b, 30.0, 30.0)[3], 1.0);
    assert_eq!(at(&b, 70.0, 30.0)[3], 0.0);
    // moved by its transform, and painted alone on nothing
    let moved = stroke(StrokeKind::Brush, ACROSS, &[("options/color", red()), ("transform/position", Value::Vec2([0.0, 15.0]))]);
    let b = painted(vec![moved], &[("paintOnTransparent", Value::Bool(true))], halves(100, 60), 0.0);
    assert_eq!(at(&b, 50.0, 45.0), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(at(&b, 50.0, 30.0)[3], 0.0, "the layer not shown");
}

#[test]
fn flow_builds_up_and_opacity_caps_it() {
    let middle = |values: &[(&str, Value)]| at(&painted(vec![stroke(StrokeKind::Brush, ACROSS, values)], &[], Image::new(100, 60), 0.0), 50.0, 30.0)[3];
    let thin = middle(&[("options/flow", Value::Number(20.0))]);
    // dabs 2.5 pixels apart on a tip 10 across: four or so land on each pixel of the middle
    assert!(thin > 0.5 && thin < 0.75, "{thin}");
    let capped = middle(&[("options/opacity", Value::Number(50.0))]);
    assert!((capped - 0.5).abs() < 1e-6, "{capped}");
    // a soft tip fades out from its middle
    let soft = painted(vec![stroke(StrokeKind::Brush, &[[50.0, 30.0], [50.0, 30.0]], &[("options/hardness", Value::Number(0.0)), ("options/diameter", Value::Number(30.0))])], &[], Image::new(100, 60), 0.0);
    let (c, half, edge) = (at(&soft, 50.0, 30.0)[3], at(&soft, 57.0, 30.0)[3], at(&soft, 64.0, 30.0)[3]);
    assert!(c > 0.95 && half > 0.3 && half < 0.8 && edge < 0.05, "{c} {half} {edge}");
}

#[test]
fn erasers_rub_out_the_layer_or_only_paint() {
    let grey = || halves(100, 60);
    let brush = |y: f64| stroke(StrokeKind::Brush, &[[10.0, y], [90.0, y]], &[("options/color", red()), ("options/diameter", Value::Number(20.0))]);
    let rubber = |mode: u32| stroke(StrokeKind::Eraser, &[[50.0, 0.0], [50.0, 60.0]], &[("options/eraseMode", Value::Choice(mode))]);
    // the layer and paint under it
    let b = painted(vec![brush(30.0), rubber(0)], &[], grey(), 0.0);
    assert_eq!(at(&b, 50.0, 30.0)[3], 0.0);
    assert_eq!(at(&b, 50.0, 5.0)[3], 0.0, "the layer too");
    // only paint: the layer stays
    let b = painted(vec![brush(30.0), rubber(1)], &[], grey(), 0.0);
    let p = at(&b, 50.0, 30.0);
    assert!((p[0] - 0.9).abs() < 1e-6 && p[3] == 1.0, "the layer shows through: {p:?}");
    assert_eq!(at(&b, 30.0, 30.0)[1], 0.0, "the paint elsewhere stays");
    // only the stroke just before it
    let b = painted(vec![brush(20.0), brush(40.0), rubber(2)], &[], grey(), 0.0);
    assert_eq!(at(&b, 50.0, 20.0), [1.0, 0.0, 0.0, 1.0], "the first stroke stays");
    assert!((at(&b, 50.0, 40.0)[0] - 0.9).abs() < 1e-6, "the last one is gone");
}

#[test]
fn a_clone_copies_from_elsewhere() {
    // copying the light right half onto the dark left one
    let c = stroke(StrokeKind::Clone, &[[10.0, 30.0], [30.0, 30.0]], &[("options/clonePosition", Value::Vec2([70.0, 30.0])), ("options/diameter", Value::Number(16.0))]);
    let b = painted(vec![c], &[], halves(100, 60), 0.0);
    assert!((at(&b, 20.0, 30.0)[0] - 0.9).abs() < 1e-6, "{:?}", at(&b, 20.0, 30.0));
    assert!((at(&b, 20.0, 50.0)[0] - 0.1).abs() < 1e-6, "only along the stroke");
}

#[test]
fn strokes_show_for_their_own_time() {
    let timed = || vec![stroke(StrokeKind::Brush, ACROSS, &[("from", Value::Number(1.0)), ("length", Value::Number(0.5))])];
    let shows = |t: f64| at(&painted(timed(), &[], Image::new(100, 60), t), 50.0, 30.0)[3] > 0.0;
    assert_eq!([shows(0.5), shows(1.0), shows(1.4), shows(1.5), shows(9.0)], [false, true, true, false, false]);
    let forever = |t: f64| at(&painted(vec![stroke(StrokeKind::Brush, ACROSS, &[("from", Value::Number(1.0))])], &[], Image::new(100, 60), t), 50.0, 30.0)[3] > 0.0;
    assert!(!forever(0.9) && forever(100.0));
}

#[test]
fn channels_pressure_and_modes() {
    // black into the opacity alone: the layer rubbed out there, its colour kept elsewhere
    let black = Value::Color(veymelo_color::Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 });
    let b = painted(vec![stroke(StrokeKind::Brush, ACROSS, &[("options/color", black.clone()), ("options/channels", Value::Choice(2))])], &[], halves(100, 60), 0.0);
    assert_eq!(at(&b, 50.0, 30.0)[3], 0.0);
    // colour only: no paint where the layer is not
    let b = painted(vec![stroke(StrokeKind::Brush, ACROSS, &[("options/color", red()), ("options/channels", Value::Choice(1))])], &[], Image::new(100, 60), 0.0);
    assert_eq!(at(&b, 50.0, 30.0)[3], 0.0);
    // pressure from none to full: thin where it starts, the full tip where it ends
    let b = painted(vec![stroke(StrokeKind::Brush, ACROSS, &[("options/color", red()), ("options/diameter", Value::Number(20.0)), ("pressure", Value::Text("0, 1".into()))])], &[], Image::new(100, 60), 0.0);
    let thick = |x: f64| (0..60).filter(|y| at(&b, x, *y as f64 + 0.5)[3] > 0.5).count();
    assert!(thick(15.0) < 5 && thick(85.0) >= 17, "{} {}", thick(15.0), thick(85.0));
    // multiplied onto the light half: red times its grey
    let b = painted(vec![stroke(StrokeKind::Brush, ACROSS, &[("options/color", red()), ("mode", Value::Choice(2))])], &[], halves(100, 60), 0.0);
    let p = at(&b, 80.0, 30.0);
    assert!((p[0] - 0.9).abs() < 1e-6 && p[1] == 0.0, "{p:?}");
}
