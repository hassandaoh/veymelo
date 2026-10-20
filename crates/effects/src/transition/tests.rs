use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_time::Tick;

fn picture(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.px[y * w + x] = [x as f32 / w as f32, y as f32 / h as f32, 0.4, 1.0];
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

fn alpha(b: &Buf, x: usize, y: usize) -> f32 {
    b.img.px[y * b.img.width + x][3]
}

fn left(b: &Buf) -> f32 {
    b.img.px.iter().map(|p| p[3]).sum::<f32>() / b.img.px.len() as f32
}

/// Every transition: nothing at 0, everything gone at 100 (those that go all the way), and about
/// half way at 50.
#[test]
fn from_nothing_to_all() {
    let img = picture(40, 30);
    let cases: &[(&Spec, &str)] = &[
        (&BLOCK, "transitionCompletion"),
        (&GRADIENT_WIPE, "transitionCompletion"),
        (&LINEAR_WIPE, "transitionCompletion"),
        (&RADIAL_WIPE, "transitionCompletion"),
        (&VENETIAN, "transitionCompletion"),
        (&GLASS_WIPE, "completion"),
        (&GRID_WIPE, "completion"),
        (&IMAGE_WIPE, "completion"),
        (&JAWS, "completion"),
        (&LIGHT_WIPE, "completion"),
        (&LINE_SWEEP, "completion"),
        (&RADIAL_SCALE, "completion"),
        (&TWISTER, "completion"),
        (&WARP_REVEAL, "completion"),
    ];
    for (spec, id) in cases {
        let at = |c: f64| go(spec, &[(id, n(c))], img.clone());
        assert_eq!(at(0.0).img.px, img.px, "{}: 0 leaves it", spec.def.id);
        let mid = left(&at(50.0));
        assert!(mid < 0.99, "{}: some gone half way ({mid})", spec.def.id);
        if spec.def.id != "twister" {
            assert!(left(&at(100.0)) < 0.01, "{}: all gone at 100 ({})", spec.def.id, left(&at(100.0)));
        }
    }
}

#[test]
fn wipes_go_their_way() {
    let img = picture(40, 30);
    let lin = go(&LINEAR_WIPE, &[("transitionCompletion", n(50.0))], img.clone());
    assert!(alpha(&lin, 5, 15) == 0.0 && alpha(&lin, 35, 15) == 1.0, "left to right: the left gone first");
    let iris = go(&IRIS, &[("outerRadius", n(10.0))], img.clone());
    assert!(alpha(&iris, 20, 15) == 1.0 && alpha(&iris, 2, 2) == 0.0);
    let star = go(&IRIS, &[("outerRadius", n(14.0)), ("useInnerRadius", Value::Bool(true)), ("innerRadius", n(4.0))], img.clone());
    assert!(left(&star) < left(&go(&IRIS, &[("outerRadius", n(14.0))], img.clone())), "a star is less than its polygon");
    let radial = go(&RADIAL_WIPE, &[("transitionCompletion", n(25.0))], img.clone());
    assert!(alpha(&radial, 28, 5) == 0.0 && alpha(&radial, 10, 25) == 1.0, "a quarter round from the top, clockwise");
    let jaws = go(&JAWS, &[("completion", n(30.0)), ("width", n(10.0))], img.clone());
    assert!(alpha(&jaws, 20, 15) == 0.0 && alpha(&jaws, 20, 1) > 0.9, "split open in the middle");
    let stretched = go(&SCALE_WIPE, &[("stretch", n(1.0))], img.clone());
    assert_eq!(stretched.img.px[15 * 40 + 5], img.px[15 * 40 + 5], "before the line as it was");
    assert_ne!(stretched.img.px[15 * 40 + 35], img.px[15 * 40 + 35]);
    let twist = go(&TWISTER, &[("completion", n(50.0))], img.clone());
    assert_ne!(twist.img.px, img.px);
}
