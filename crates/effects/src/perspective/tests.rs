use super::*;
use crate::testing::{at as px_at, instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_time::Tick;

fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, &[], None)
}

/// A white square 10 across in the middle of a clear 30 × 30 layer.
fn square_img() -> Image {
    let mut img = Image::new(30, 30);
    for y in 10..20 {
        for x in 10..20 {
            img.px[y * 30 + x] = [1.0; 4];
        }
    }
    img
}

fn picture(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.px[y * w + x] = [x as f32 / w as f32, y as f32 / h as f32, 0.4, 1.0];
        }
    }
    img
}

#[test]
fn shadows() {
    let img = square_img();
    let out = go(&DROP_SHADOW, &[("distance", n(4.0))], img.clone());
    // falling down and right (135°): beside the square's bottom right, a shadow
    assert!(px_at(&out, 21.0, 21.0)[3] > 0.4 && px_at(&out, 21.0, 21.0)[0] < 0.01, "{:?}", px_at(&out, 21.0, 21.0));
    assert!(px_at(&out, 8.0, 8.0)[3] == 0.0, "none up and left");
    assert_eq!(px_at(&out, 15.0, 15.0), [1.0; 4], "the layer over its shadow");
    let alone = go(&DROP_SHADOW, &[("distance", n(4.0)), ("shadowOnly", Value::Bool(true))], img.clone());
    assert!(px_at(&alone, 15.0, 15.0)[0] < 0.01);
    let soft = go(&DROP_SHADOW, &[("distance", n(4.0)), ("softness", n(6.0))], img.clone());
    assert!(px_at(&soft, 26.0, 26.0)[3] > px_at(&out, 26.0, 26.0)[3]);
    let radial = go(&RADIAL_SHADOW, &[("lightSource", Value::Vec2([15.0, 15.0])), ("projectionDistance", n(50.0)), ("shadowOnly", Value::Bool(true))], img.clone());
    // from the middle: a shadow 1.5 times the square all round
    assert!(px_at(&radial, 8.5, 15.0)[3] > 0.3 && px_at(&radial, 6.0, 15.0)[3] == 0.0, "{:?}", px_at(&radial, 8.5, 15.0));
}

#[test]
fn bevels_light_one_side() {
    // a grey square on nothing
    let mut img = Image::new(30, 30);
    for y in 10..20 {
        for x in 10..20 {
            img.px[y * 30 + x] = [0.5, 0.5, 0.5, 1.0];
        }
    }
    let out = go(&BEVEL_ALPHA, &[("lightAngle", n(-90.0)), ("lightIntensity", n(1.0))], img);
    // lit from the left: the left edge brighter, the right darker
    let (l, r) = (px_at(&out, 10.5, 15.0)[0], px_at(&out, 19.5, 15.0)[0]);
    assert!(l > 0.5 && r < 0.5, "{l} {r}");
    let mid = Image::filled(40, 40, [0.5, 0.5, 0.5, 1.0]);
    let edges = go(&BEVEL_EDGES, &[("lightAngle", n(-90.0)), ("lightIntensity", n(1.0))], mid);
    assert!(px_at(&edges, 1.0, 20.0)[0] > 0.5 && px_at(&edges, 39.0, 20.0)[0] < 0.5 && (px_at(&edges, 20.0, 20.0)[0] - 0.5).abs() < 1e-6);
}

#[test]
fn round_things() {
    let img = picture(60, 40);
    let ball = go(&SPHERE, &[("radius", n(15.0))], img.clone());
    assert_eq!(px_at(&ball, 2.0, 2.0)[3], 0.0, "nothing past the ball");
    assert!(px_at(&ball, 30.0, 20.0)[3] > 0.99);
    // lit from the top left: brighter there
    assert!(px_at(&ball, 24.0, 14.0)[2] > px_at(&ball, 36.0, 26.0)[2]);
    let turned = go(&SPHERE, &[("radius", n(15.0)), ("rotationY", n(90.0))], img.clone());
    assert_ne!(turned.img.px, ball.img.px);
    let cyl = go(&CYLINDER, &[], img.clone());
    // the layer 60 wide once round: about 19 across
    assert!(px_at(&cyl, 30.0, 20.0)[3] > 0.99 && px_at(&cyl, 41.0, 20.0)[3] == 0.0 && px_at(&cyl, 2.0, 20.0)[3] == 0.0);
    let tipped = go(&CYLINDER, &[("rotationX", n(30.0))], img.clone());
    assert_ne!(tipped.img.px, cyl.img.px);
    let inside = go(&CYLINDER, &[("render", Value::Choice(2))], img.clone());
    assert_ne!(inside.img.px, cyl.img.px);
}

#[test]
fn spotlight_and_environment() {
    let img = Image::filled(60, 60, [0.8, 0.8, 0.8, 1.0]);
    let out = go(&SPOTLIGHT, &[], img.clone());
    assert!(px_at(&out, 30.0, 30.0)[0] > 0.7, "lit where it points");
    assert!(px_at(&out, 58.0, 58.0)[0] < 0.1, "dark round it");
    let only = go(&SPOTLIGHT, &[("render", Value::Choice(1))], img.clone());
    assert!(px_at(&only, 58.0, 58.0)[3] < 0.1 && px_at(&only, 30.0, 30.0)[3] > 0.9);
    let env = go(&ENVIRONMENT, &[], picture(40, 40));
    assert_ne!(env.img.px, picture(40, 40).px);
    assert!(env.img.px.iter().all(|p| p[3] == 1.0));
    for mapping in 0..3 {
        let (u, v) = env_at(mapping, [0.0, 0.0, 1.0]);
        assert!((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v), "{mapping}");
    }
}

#[test]
fn glasses() {
    let img = picture(40, 20);
    let side = go(&GLASSES, &[("view3d", Value::Choice(0))], img.clone());
    assert!(near(px_at(&side, 5.0, 10.0), px_at(&side, 25.0, 10.0)), "the same view each side");
    let anaglyph = go(&GLASSES, &[("view3d", Value::Choice(3))], img.clone());
    assert!(anaglyph.img.px.iter().all(|p| p[2] == 0.0 && (p[0] - p[1]).abs() < 1e-6), "red and green the same brightness");
    let diff = go(&GLASSES, &[("view3d", Value::Choice(8))], img.clone());
    assert!(diff.img.px.iter().all(|p| p[0] < 1e-6 && p[1] < 1e-6), "no difference between the same views");
    let apart = go(&GLASSES, &[("view3d", Value::Choice(8)), ("sceneConvergence", n(4.0))], img);
    assert!(apart.img.px.iter().any(|p| p[0] > 0.01));
}

fn near(a: Px, b: Px) -> bool {
    (0..4).all(|k| (a[k] - b[k]).abs() < 0.03)
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "perspective"), "{}", spec.def.id);
    }
}
