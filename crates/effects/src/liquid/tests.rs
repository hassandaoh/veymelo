use super::*;
use crate::testing::{instance, n, run_when, square};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_raster::Image;
use veymelo_time::Tick;

fn picture(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.px[y * w + x] = [x as f32 / w as f32, y as f32 / h as f32, ((x * 7 + y * 3) % 11) as f32 / 11.0, 1.0];
        }
    }
    img
}

fn get(img: &Image, x: usize, y: usize) -> Px {
    img.px[y * img.width + x]
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image, masks: &[crate::MaskShape]) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, masks, None)
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

#[test]
fn flow_from_knots() {
    let img = picture(64, 64);
    let out = go(&FLOW, &[("knot1", Value::Vec2([32.0, 32.0])), ("amount1", n(80.0)), ("amount2", n(0.0)), ("antialiasing", Value::Choice(0))], img.clone(), &[]);
    // the knot stays; near it the picture comes from nearer the knot (flowing out of it)
    assert!((get(&out.img, 32, 32)[0] - get(&img, 32, 32)[0]).abs() < 0.01);
    assert!(get(&out.img, 40, 32)[0] < get(&img, 40, 32)[0] - 0.01);
    assert_eq!(go(&FLOW, &[("amount1", n(0.0)), ("amount2", n(0.0))], img.clone(), &[]).img.px, img.px);
    let push = go(&FLOW, &[("knot1", Value::Vec2([32.0, 32.0])), ("amount1", n(-200.0)), ("amount2", n(0.0))], img.clone(), &[]);
    assert!(push.img.px.iter().any(|p| p[3] < 0.5), "pushed away: nothing comes in from past the edges");
    let tiled = go(&FLOW, &[("knot1", Value::Vec2([32.0, 32.0])), ("amount1", n(-200.0)), ("amount2", n(0.0)), ("tileEdges", Value::Bool(true))], img.clone(), &[]);
    assert!(tiled.img.px.iter().all(|p| p[3] > 0.99), "tiled: the layer over again");
}

#[test]
fn strokes_as_text() {
    let s = Stroke { tool: 9, size: 40.0, pressure: 60.0, jitter: 70.0, clone_offset: [3.0, -4.0], mode: 2, points: vec![[1.0, 2.5], [10.0, 2.5]] };
    assert_eq!(s.line(), "reconstruction 40 60 70 3 -4 amplitwist 1,2.5 10,2.5");
    assert_eq!(Stroke::parse(&s.line()), Some(s));
    assert_eq!(strokes("warp 20 50 70 0 0 5,5 9,5\nnonsense\nbloat 10 50 70 0 0").len(), 1, "a stroke needs a point");
}

#[test]
fn liquify_pushes_where_the_brush_went() {
    let img = picture(80, 60);
    assert_eq!(go(&LIQUIFY, &[], img.clone(), &[]).img.px, img.px);
    let warp = text("warp 30 100 70 0 0 20,30 50,30");
    let out = go(&LIQUIFY, &[("distortionMesh", warp.clone())], img.clone(), &[]);
    // dragged right: what was on the left shows further right
    assert!(get(&out.img, 40, 30)[0] < get(&img, 40, 30)[0] - 0.03, "{:?} {:?}", get(&out.img, 40, 30), get(&img, 40, 30));
    assert_eq!(get(&out.img, 40, 2), get(&img, 40, 2), "out of the brush's reach");
    let none = go(&LIQUIFY, &[("distortionMesh", warp.clone()), ("distortionPercentage", n(0.0))], img.clone(), &[]);
    assert_eq!(none.img.px, img.px);
    // the same strokes over again: the same picture (from the kept mesh)
    assert_eq!(go(&LIQUIFY, &[("distortionMesh", warp.clone())], img.clone(), &[]).img.px, out.img.px);
    // frozen inside a mask, nothing there moves
    let held = go(&LIQUIFY, &[("distortionMesh", warp.clone()), ("freezeAreaMask", n(1.0))], img.clone(), &[square(25.0, 15.0, 30.0)]);
    assert_eq!(get(&held.img, 40, 30), get(&img, 40, 30));
    // reconstructed (all the way, twice over) it goes back
    let undone = go(&LIQUIFY, &[("distortionMesh", text("warp 30 100 70 0 0 20,30 50,30\nreconstruction 60 100 70 0 0 35,30\nreconstruction 60 100 70 0 0 35,30\nreconstruction 60 100 70 0 0 35,30"))], img.clone(), &[]);
    let moved = |b: &Buf| b.img.px.iter().zip(&img.px).map(|(a, c)| (a[0] - c[0]).abs()).sum::<f32>();
    assert!(moved(&undone) < moved(&out) * 0.5, "{} {}", moved(&undone), moved(&out));
    // reconstructed toward how its start moved: what moved alike stays moved
    let kept = go(&LIQUIFY, &[("distortionMesh", text("warp 60 100 70 0 0 10,30 70,30\nreconstruction 30 100 70 0 0 displace 40,30"))], img.clone(), &[]);
    let gone = go(&LIQUIFY, &[("distortionMesh", text("warp 60 100 70 0 0 10,30 70,30\nreconstruction 30 100 70 0 0 40,30"))], img.clone(), &[]);
    assert!(moved(&kept) > moved(&gone));
    for tool in TOOLS {
        if *tool == "freeze" || *tool == "thaw" || *tool == "reconstruction" {
            continue;
        }
        let out = go(&LIQUIFY, &[("distortionMesh", text(&format!("{tool} 30 100 70 8 0 30,30 45,32")))], img.clone(), &[]);
        assert_ne!(out.img.px, img.px, "{tool}");
    }
    let shifted = go(&LIQUIFY, &[("distortionMesh", warp.clone()), ("distortionMeshOffset", Value::Vec2([0.0, 20.0]))], img.clone(), &[]);
    assert_eq!(get(&shifted.img, 40, 30), get(&img, 40, 30), "the distortion moved down");
    assert_ne!(get(&shifted.img, 40, 50), get(&img, 40, 50));
    let grid = go(&LIQUIFY, &[("viewMesh", Value::Bool(true))], img.clone(), &[]);
    assert!(grid.img.px.iter().any(|p| (p[0] - 0.5).abs() < 1e-6 && (p[1] - 0.5).abs() < 1e-6 && (p[2] - 0.5).abs() < 1e-6));
}
