use super::*;
use crate::testing::{at as px_at, instance, n, run_when};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
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

fn near_rg(a: Px, b: Px, tol: f32) -> bool {
    (0..2).all(|k| (a[k] - b[k]).abs() <= tol)
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, &[], None)
}

#[test]
fn brush_strokes_repaint() {
    let img = picture(40, 40);
    let out = go(&BRUSH, &[], img.clone());
    assert_ne!(out.img.px, img.px);
    assert!(out.img.px.iter().zip(&img.px).all(|(a, b)| near_rg(*a, *b, 0.25)), "near the colours it had");
    let none = go(&BRUSH, &[("strokeDensity", n(0.0)), ("paintSurface", Value::Choice(3))], img.clone());
    assert!(none.img.px.iter().all(|p| *p == [0.0, 0.0, 0.0, 1.0]), "no strokes on black: black");
    let kept = go(&BRUSH, &[("blendWithOriginal", n(100.0))], img.clone());
    assert!(kept.img.px.iter().zip(&img.px).all(|(a, b)| near_rg(*a, *b, 1e-6)));
}

#[test]
fn kaleidoscope_mirrors_wedges() {
    let img = picture(60, 60);
    let out = go(&KALEIDOSCOPE, &[], img.clone());
    // six mirrored wedges: a point and its mirror across a wedge's line match
    let at = |a: f64, r: f64| px_at(&out, 30.0 + r * a.to_radians().cos(), 30.0 + r * a.to_radians().sin());
    assert!(near_rg(at(10.0, 15.0), at(50.0, 15.0), 0.04), "{:?} {:?}", at(10.0, 15.0), at(50.0, 15.0));
    assert!(near_rg(at(10.0, 15.0), at(70.0, 15.0), 0.04), "one wedge on: the same");
    let wheel = go(&KALEIDOSCOPE, &[("mirroring", Value::Choice(3))], img.clone());
    assert_ne!(wheel.img.px, out.img.px);
}

#[test]
fn tiles_past_the_layer() {
    let img = picture(20, 10);
    let out = go(&REPEAT, &[("expandRight", n(20.0)), ("expandDown", n(10.0))], img.clone());
    assert_eq!((out.img.width, out.img.height), (40, 20));
    assert!(near_rg(px_at(&out, 25.5, 3.5), get(&img, 5, 3), 1e-4) && near_rg(px_at(&out, 5.5, 13.5), get(&img, 5, 3), 1e-4), "repeated");
    let unfold = go(&REPEAT, &[("expandRight", n(20.0)), ("tiling", Value::Choice(1))], img.clone());
    assert!(near_rg(px_at(&unfold, 25.5, 3.5), get(&img, 14, 3), 1e-4), "mirrored");
    let mt = go(&MOTION_TILE, &[("tileWidth", n(50.0)), ("tileHeight", n(50.0))], img.clone());
    // half-size tiles centred on the layer's middle: the middle of the picture there, and again a tile on
    assert!(near_rg(px_at(&mt, 10.0, 5.0), get(&img, 10, 5), 0.06), "{:?}", px_at(&mt, 10.0, 5.0));
    assert!(near_rg(px_at(&mt, 3.0, 2.0), px_at(&mt, 13.0, 2.0), 1e-4) && near_rg(px_at(&mt, 3.0, 2.0), px_at(&mt, 3.0, 7.0), 1e-4));
    let bigger = go(&MOTION_TILE, &[("outputWidth", n(300.0))], img.clone());
    assert_eq!(bigger.img.width, 60);
    assert!(near_rg(px_at(&bigger, -15.0, 5.0), get(&img, 5, 5), 1e-3), "a whole tile on the left");
}

#[test]
fn vignette_darkens_the_edges() {
    let img = Image::filled(60, 40, [0.8, 0.8, 0.8, 1.0]);
    let out = go(&VIGNETTE, &[("amount", n(100.0))], img.clone());
    assert!(get(&out.img, 30, 20)[0] > 0.79 && get(&out.img, 0, 0)[0] < 0.7, "{:?}", get(&out.img, 0, 0));
    let light = go(&VIGNETTE, &[("amount", n(-100.0))], img.clone());
    assert!(get(&light.img, 0, 0)[0] > 0.8);
    let pinned = go(&VIGNETTE, &[("amount", n(100.0)), ("pinHighlights", n(100.0))], img.clone());
    assert!(get(&pinned.img, 0, 0)[0] > get(&out.img, 0, 0)[0]);
}

#[test]
fn thresholds() {
    let img = picture(40, 40);
    let lum = go(&CHANNEL_THRESHOLD, &[], img.clone());
    assert!(lum.img.px.iter().all(|p| p[0] == p[1] && p[1] == p[2] && (p[0] == 0.0 || p[0] == 1.0)));
    let rgb = go(&CHANNEL_THRESHOLD, &[("channel", Value::Choice(1))], img.clone());
    assert_eq!(get(&rgb.img, 30, 5), [1.0, 0.0, get(&rgb.img, 30, 5)[2], 1.0]);
    let inv = go(&CHANNEL_THRESHOLD, &[("invert", Value::Bool(true))], img.clone());
    assert!(lum.img.px.iter().zip(&inv.img.px).all(|(a, b)| a[0] + b[0] == 1.0));
    let t = go(&THRESHOLD_RGB, &[("redThreshold", n(0.0)), ("invertGreenChannel", Value::Bool(true))], img.clone());
    assert!(t.img.px.iter().all(|p| p[0] == 1.0));
    assert_eq!(get(&t.img, 5, 35)[1], 0.0, "green high, inverted: off");
}

#[test]
fn blocks_load_in() {
    let img = picture(32, 32);
    assert_eq!(go(&BLOCK_LOAD, &[("completion", n(100.0))], img.clone()).img.px, img.px);
    let start = go(&BLOCK_LOAD, &[], img.clone());
    assert!(start.img.px.iter().all(|p| p[3] == 0.0), "cleared at the start");
    let mid = go(&BLOCK_LOAD, &[("completion", n(50.0)), ("scanlines", Value::Bool(false)), ("smoothing", Value::Bool(false))], img.clone());
    // half way, blocks: a pixel shares its colour with its neighbour
    assert_eq!(get(&mid.img, 0, 0), get(&mid.img, 1, 1));
    assert_ne!(mid.img.px, img.px);
}

#[test]
fn film_burns() {
    let img = picture(60, 60);
    assert_eq!(go(&BURN_FILM, &[], img.clone()).img.px, img.px);
    let out = go(&BURN_FILM, &[("burn", n(40.0))], img.clone());
    let holes = out.img.px.iter().filter(|p| p[3] == 0.0).count();
    assert!(holes > 50 && holes < 3600, "{holes}");
    assert_eq!(get(&out.img, 30, 30)[3], 0.0, "it starts in the middle");
    let more = go(&BURN_FILM, &[("burn", n(70.0))], img.clone());
    assert!(more.img.px.iter().filter(|p| p[3] == 0.0).count() > holes);
}

/// A disc of alpha: a bump in the middle of a 40 × 40 layer.
fn bump() -> Image {
    let mut img = Image::new(40, 40);
    for y in 0..40 {
        for x in 0..40 {
            let a = if (x as f64 - 19.5).hypot(y as f64 - 19.5) < 10.0 { 1.0 } else { 0.4 };
            img.px[y * 40 + x] = [0.5 * a, 0.5 * a, 0.5 * a, a];
        }
    }
    img
}

#[test]
fn glass_and_plastic_are_lit() {
    let img = bump();
    let p = go(&PLASTIC, &[], img.clone());
    // lit from the top left: the bump's top left brighter than its bottom right
    let (tl, br) = (get(&p.img, 12, 12), get(&p.img, 27, 27));
    assert!(tl[0] / tl[3] > br[0] / br[3] + 0.05, "{tl:?} {br:?}");
    let flat = Image::filled(20, 20, [0.4, 0.3, 0.2, 1.0]);
    let pf = go(&PLASTIC, &[("specular", n(0.0))], flat.clone());
    assert!(pf.img.px.iter().all(|q| near_rg(*q, [0.4, 0.3, 0.2, 1.0], 1e-4)), "a flat surface keeps its colour");
    let g = go(&GLASS, &[], img.clone());
    assert_ne!(g.img.px, img.px);
    assert!(g.img.px.iter().zip(&img.px).all(|(a, b)| (a[3] - b[3]).abs() < 1e-6), "glass keeps the alpha");
}

#[test]
fn hex_tiles_repeat_the_middle() {
    let img = picture(80, 80);
    let out = go(&HEX_TILE, &[("radius", n(10.0))], img.clone());
    // a neighbouring hexagon (√3 × 10 to the right) shows the middle one again
    let step = 3f64.sqrt() * 10.0;
    assert!(near_rg(px_at(&out, 42.0, 41.0), px_at(&out, 42.0 + step, 41.0), 0.03));
    assert!(near_rg(px_at(&out, 40.0, 40.0), get(&img, 40, 40), 0.03));
}

#[test]
fn smoothie_recolours_from_itself() {
    let mut img = Image::new(40, 20);
    for y in 0..20 {
        for x in 0..40 {
            img.px[y * 40 + x] = if x < 20 { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] };
        }
    }
    let out = go(&SMOOTHIE, &[], img.clone());
    // only colours found along the line from A to B (red, blue and what is between)
    assert!(out.img.px.iter().all(|p| p[1] < 1e-4 && (p[0] + p[2] - 1.0).abs() < 1e-3), "{:?}", out.img.px[0]);
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "stylize"), "{}", spec.def.id);
    }
}
