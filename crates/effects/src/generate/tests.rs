use super::*;
use crate::testing::{at as px_at, n, run, run_on};
use veymelo_geom::{Rect, vec2};
use veymelo_keyframe::Value;
use veymelo_raster::Image;

fn red(w: usize, h: usize) -> Image {
    Image::filled(w, h, [1.0, 0.0, 0.0, 1.0])
}

fn pt(x: f64, y: f64) -> Value {
    Value::Vec2([x, y])
}

#[test]
fn a_ramp_fills_the_shape_only() {
    // left quarter see-through: the ramp keeps the layer's alpha
    let mut img = red(40, 24);
    for (i, p) in img.px.iter_mut().enumerate() {
        if i % 40 < 10 {
            *p = [0.0; 4];
        }
    }
    let out = run(&GRADIENT_RAMP, &[("startOfRamp", pt(0.0, 0.0)), ("endOfRamp", pt(40.0, 0.0))], img.clone());
    for (a, b) in img.px.iter().zip(&out.img.px) {
        assert!((a[3] - b[3]).abs() < 1e-6 && b[..3].iter().all(|c| *c <= b[3] + 1e-6));
    }
    let (l, r) = (px_at(&out, 10.5, 5.0), px_at(&out, 39.5, 5.0));
    assert!(l[0] < 0.3 && r[0] > 0.95, "black to white across: {l:?} {r:?}");
    let radial = run(&GRADIENT_RAMP, &[("startOfRamp", pt(20.0, 12.0)), ("endOfRamp", pt(30.0, 12.0)), ("rampShape", Value::Choice(1))], red(40, 24));
    assert!(px_at(&radial, 20.5, 12.5)[0] < 0.1 && px_at(&radial, 35.5, 12.5)[0] > 0.99);
}

#[test]
fn four_colours_meet() {
    let pts = [("point1", pt(0.0, 0.0)), ("point2", pt(32.0, 0.0)), ("point3", pt(0.0, 32.0)), ("point4", pt(32.0, 32.0))];
    let out = run(&FOUR_COLOR, &pts, red(32, 32));
    let corner = px_at(&out, 0.5, 0.5);
    assert!(corner[0] > 0.9 && corner[1] > 0.9 && corner[2] < 0.1, "yellow at point 1: {corner:?}");
    let mut half = pts.to_vec();
    half.push(("opacity", n(50.0)));
    assert!((run(&FOUR_COLOR, &half, red(32, 32)).img.px[16 * 32 + 16][3] - 0.5).abs() < 1e-3);
}

#[test]
fn patterns() {
    let board = run(&CHECKERBOARD, &[("width", n(8.0))], red(32, 32));
    assert!(px_at(&board, 3.5, 3.5)[3] > 0.99 && px_at(&board, 11.5, 3.5)[3] < 0.01, "squares of 8, alone");
    let blended = run(&CHECKERBOARD, &[("width", n(8.0)), ("blendingMode", Value::Choice(1))], red(32, 32));
    assert!(px_at(&blended, 11.5, 3.5)[0] > 0.99, "with normal the layer shows between");
    let lines = run(&GRID, &[("width", n(10.0)), ("border", n(2.0))], red(32, 32));
    assert!(px_at(&lines, 0.5, 5.5)[3] > 0.9 && px_at(&lines, 5.5, 5.5)[3] < 0.01);
    let stencil = run(&GRID, &[("width", n(10.0)), ("border", n(2.0)), ("blendingMode", Value::Choice(18))], red(32, 32));
    assert!(px_at(&stencil, 0.5, 5.5)[0] > 0.9 && px_at(&stencil, 5.5, 5.5)[3] < 0.01, "the layer through the lines");
    let a = run(&CELL_PATTERN, &[("size", n(10.0))], Image::new(40, 30));
    let b = run(&CELL_PATTERN, &[("size", n(10.0))], Image::new(40, 30));
    let c = run(&CELL_PATTERN, &[("size", n(10.0)), ("randomSeed", n(5.0))], Image::new(40, 30));
    assert_eq!(a.img.px, b.img.px, "the same every time");
    assert_ne!(a.img.px, c.img.px, "its own for each seed");
    assert!(a.img.px.iter().any(|p| p[0] > 0.5) && a.img.px.iter().any(|p| p[0] < 0.3));
}

#[test]
fn shapes() {
    let c = ("center", pt(16.0, 16.0));
    let disc = run(&CIRCLE, &[c.clone(), ("radius", n(10.0))], red(32, 32));
    assert!(px_at(&disc, 16.5, 16.5)[3] > 0.99 && px_at(&disc, 30.5, 16.5)[3] < 0.01);
    let ring = run(&CIRCLE, &[c.clone(), ("radius", n(10.0)), ("edge", Value::Choice(2)), ("thickness", n(3.0))], red(32, 32));
    assert!(px_at(&ring, 16.5, 16.5)[3] < 0.01 && px_at(&ring, 24.5, 16.5)[3] > 0.99);
    let oval = run(&ELLIPSE, &[("center", pt(32.0, 32.0)), ("width", n(40.0)), ("height", n(40.0)), ("thickness", n(6.0))], Image::new(64, 64));
    assert!(px_at(&oval, 52.0, 32.0)[3] > 0.9 && px_at(&oval, 32.0, 32.0)[3] < 0.01, "a ring of the size asked");
    let beam = run(&BEAM, &[("startingPoint", pt(8.0, 16.0)), ("endingPoint", pt(56.0, 16.0)), ("length", n(100.0)), ("softness", n(0.0)), ("compositeOnOriginal", Value::Bool(false))], Image::new(64, 32));
    assert!(px_at(&beam, 32.0, 16.0)[3] > 0.99 && px_at(&beam, 32.0, 28.0)[3] < 0.01);
}

#[test]
fn light() {
    let flare = run(&LENS_FLARE, &[("flareCenter", pt(16.0, 16.0))], Image::filled(64, 64, [0.0, 0.0, 0.0, 1.0]));
    assert!(px_at(&flare, 16.5, 16.5)[0] > 0.9, "bright at the light");
    assert!(px_at(&flare, 60.5, 60.5)[0] < px_at(&flare, 16.5, 16.5)[0]);
    // a bright spot sends rays outward
    let mut spot = Image::filled(64, 64, [0.0, 0.0, 0.0, 1.0]);
    for y in 30..34 {
        for x in 30..34 {
            spot.px[y * 64 + x] = [1.0; 4];
        }
    }
    let rays = run(&LIGHT_RAYS, &[("center", pt(32.0, 32.0)), ("radius", n(60.0)), ("intensity", n(180.0))], spot.clone());
    // at 180 the rays reach ten times as far out as the spot
    assert!(px_at(&rays, 42.5, 32.5)[0] > 0.05, "light streams out");
    let burst = run(&LIGHT_BURST, &[("center", pt(32.0, 32.0))], spot);
    assert!(px_at(&burst, 35.5, 32.5)[0] > 0.5 && px_at(&burst, 44.5, 32.5)[0] < 0.1, "the spot is streaked outward, as far as the rays reach");
    let sweep = run(&LIGHT_SWEEP, &[("center", pt(16.0, 16.0)), ("direction", n(0.0)), ("width", n(8.0))], Image::filled(32, 32, [0.2, 0.2, 0.2, 1.0]));
    assert!(px_at(&sweep, 16.5, 5.5)[0] > 0.25 && (px_at(&sweep, 30.5, 5.5)[0] - 0.2).abs() < 1e-6, "lit within the band only");
}

#[test]
fn a_shape_layer_is_drawn_over_all_of_it() {
    // a small picture of a layer whose rectangle is much bigger: a generator fills the rectangle
    let b = Buf { img: red(4, 4), origin: vec2(-2.0, -2.0), scale: 1.0 };
    let mut b2 = b.clone();
    b2.cover(Rect::new(-20.0, -10.0, 20.0, 10.0));
    assert_eq!((b2.img.width, b2.img.height, b2.origin), (40, 20, vec2(-20.0, -10.0)));
    assert_eq!(px_at(&b2, -1.5, -1.5), [1.0, 0.0, 0.0, 1.0], "what was there stays in its place");
    let board = run_on(&CHECKERBOARD, &[("width", n(5.0))], b2, Rect::new(-20.0, -10.0, 20.0, 10.0), None);
    assert!(px_at(&board, -17.5, -7.5)[3] > 0.99);
}

/// The GPU makes Light Burst, Light Rays and Cell Pattern as the processor does: every mode,
/// shape and pattern, tiled and evolved.
#[test]
fn the_gpu_makes_light_and_cells_as_the_processor_does() {
    if !veymelo_raster::gpu::available() {
        return;
    }
    let (w, h) = (330, 250);
    let mut img = Image::new(w, h);
    for (i, p) in img.px.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        let a = if (x / 30 + y / 20) % 3 == 0 { 0.0 } else { 1.0 };
        *p = [(x as f32 / w as f32) * a, if (x / 7 + y / 7) % 2 == 0 { 0.9 * a } else { 0.2 * a }, (y as f32 / h as f32) * a, a];
    }
    let c = |k: u32| Value::Choice(k);
    let centre = Value::Vec2([140.5, 110.25]);
    let mut cases: Vec<(&Spec, Vec<(&str, Value)>)> = vec![];
    for mode in 0..3 {
        cases.push((&LIGHT_BURST, vec![("burst", c(mode)), ("center", centre.clone()), ("rayLength", n(70.0))]));
    }
    cases.push((&LIGHT_BURST, vec![("setColor", Value::Bool(true)), ("intensity", n(180.0)), ("center", centre.clone())]));
    for mode in 0..4 {
        cases.push((&LIGHT_RAYS, vec![("transferMode", c(mode)), ("center", centre.clone()), ("radius", n(60.0))]));
    }
    cases.push((&LIGHT_RAYS, vec![("shape", c(1)), ("colorFromSource", Value::Bool(false)), ("intensity", n(250.0)), ("center", centre.clone())]));
    for pattern in 0..12 {
        cases.push((&CELL_PATTERN, vec![("cellPattern", c(pattern)), ("size", n(37.0)), ("evolution", n(130.0))]));
    }
    cases.push((&CELL_PATTERN, vec![("enableTiling", Value::Bool(true)), ("cellsHorizontal", n(5.0)), ("cellsVertical", n(3.0)), ("overflow", c(1)), ("contrast", n(260.0)), ("offset", Value::Vec2([-40.5, 900.0]))]));
    cases.push((&CELL_PATTERN, vec![("overflow", c(2)), ("contrast", n(400.0)), ("invert", Value::Bool(true)), ("disperse", n(1.4)), ("cycleEvolution", Value::Bool(true)), ("evolution", n(3000.0))]));
    for (spec, values) in cases {
        let before = veymelo_raster::gpu::ran();
        let gpu = veymelo_raster::gpu::with_mode(false, || run(spec, &values, img.clone()));
        assert!(veymelo_raster::gpu::ran() > before, "drawn by the GPU: {} {values:?}", spec.def.id);
        let cpu = veymelo_raster::gpu::on_processor(|| run(spec, &values, img.clone()));
        let most = gpu.img.px.iter().zip(&cpu.img.px).flat_map(|(p, q)| (0..4).map(move |k| (p[k] - q[k]).abs())).fold(0f32, f32::max);
        assert!(most <= 1e-5, "{} {values:?}: {most}", spec.def.id);
    }
}

