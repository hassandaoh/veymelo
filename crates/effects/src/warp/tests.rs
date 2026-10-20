use super::*;
use crate::testing::{at as px_at, buf, instance, n, run_when};
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

fn pt(x: f64, y: f64) -> Value {
    Value::Vec2([x, y])
}

fn get(img: &Image, x: usize, y: usize) -> Px {
    img.px[y * img.width + x]
}

fn near(a: Px, b: Px, tol: f32) -> bool {
    (0..4).all(|k| (a[k] - b[k]).abs() <= tol)
}

/// Red and green (across and down the picture) alike.
fn near_rg(a: Px, b: Px, tol: f32) -> bool {
    (0..2).all(|k| (a[k] - b[k]).abs() <= tol)
}

/// `spec` on `img` (the whole layer), its points started on the layer, then `values` set.
fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    go_with(spec, values, img, None)
}

fn go_with(spec: &Spec, values: &[(&str, Value)], img: Image, host: Option<&dyn crate::Host>) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, &[], host)
}

#[test]
fn smooth_noise() {
    let mut lo: f64 = 0.0;
    let mut hi: f64 = 0.0;
    for i in 0..4000 {
        let (x, y, z) = (i as f64 * 0.173, i as f64 * 0.311 - 40.0, (i % 37) as f64 * 0.21);
        let v = crate::noise::smooth(x, y, z, 5, None);
        lo = lo.min(v);
        hi = hi.max(v);
        // it changes little over a little way
        assert!((crate::noise::smooth(x + 0.01, y, z, 5, None) - v).abs() < 0.05);
    }
    assert!(lo < -0.4 && hi > 0.4 && lo >= -1.0 && hi <= 1.0, "{lo} {hi}");
    assert_eq!(crate::noise::smooth(3.0, -2.0, 7.0, 5, None), 0.0, "nothing on the lattice");
    assert_eq!(crate::noise::smooth(0.3, 0.6, 0.25, 5, Some(3)), crate::noise::smooth(0.3, 0.6, 3.25, 5, Some(3)), "round again every 3");
    assert_ne!(crate::noise::smooth(0.3, 0.6, 0.25, 5, None), crate::noise::smooth(0.3, 0.6, 0.25, 6, None));
    let f = crate::noise::fractal(0.37, 0.71, 0.0, 9, 4.5, 0.5, None);
    assert!((-1.0..=1.0).contains(&f));
}

#[test]
fn turbulence_pushes_but_holds_its_pins() {
    let img = picture(60, 40);
    assert_eq!(go(&TURBULENT, &[("amount", n(0.0))], img.clone()).img.px, img.px);
    let out = go(&TURBULENT, &[("size", n(20.0))], img.clone());
    assert_ne!(out.img.px, img.px);
    // pinned all round (the default): the edges stay exactly
    for x in 0..60 {
        assert!(near(get(&out.img, x, 0), get(&img, x, 0), 1e-4) && near(get(&out.img, x, 39), get(&img, x, 39), 1e-4), "column {x}");
    }
    let free = go(&TURBULENT, &[("size", n(20.0)), ("pinning", Value::Choice(0))], img.clone());
    assert!((0..60).any(|x| !near(get(&free.img, x, 0), get(&img, x, 0), 1e-3)));
    // locked: nothing is taken from past the layer, so it stays solid
    let locked = go(&TURBULENT, &[("size", n(20.0)), ("amount", n(200.0)), ("pinning", Value::Choice(8))], img.clone());
    assert!(locked.img.px.iter().all(|p| p[3] > 0.999));
    let other = go(&TURBULENT, &[("size", n(20.0)), ("randomSeed", n(3.0))], img.clone());
    assert_ne!(other.img.px, out.img.px);
    let turned = go(&TURBULENT, &[("size", n(20.0)), ("evolution", n(90.0))], img.clone());
    assert_ne!(turned.img.px, out.img.px);
    // cycling over two turns: two turns on is where it began
    let cyc = |e: f64| go(&TURBULENT, &[("size", n(20.0)), ("cycleEvolution", Value::Bool(true)), ("cycle", n(2.0)), ("evolution", n(e))], img.clone());
    assert_eq!(cyc(30.0).img.px, cyc(30.0 + 720.0).img.px);
    for kind in 0..KINDS.len() as u32 {
        let k = go(&TURBULENT, &[("size", n(20.0)), ("displacement", Value::Choice(kind))], img.clone());
        assert_ne!(k.img.px, img.px, "{}", KINDS[kind as usize]);
    }
    let grown = go(&TURBULENT, &[("resizeLayer", Value::Bool(true))], img.clone());
    assert!(grown.img.width > 60 + 50);
}

/// A map 40 × 20: black on the left half, white on the right.
struct Halves;

impl crate::Host for Halves {
    fn layer(&self, _: u64, _: Take) -> Option<(Buf, Rect)> {
        let mut m = Image::filled(40, 20, [0.0, 0.0, 0.0, 1.0]);
        for y in 0..20 {
            for x in 20..40 {
                m.px[y * 40 + x] = [1.0; 4];
            }
        }
        Some((buf(m), Rect::from_size(40.0, 20.0)))
    }
}

#[test]
fn displacement_by_a_map() {
    let img = picture(40, 20);
    let none = [("expandOutput", Value::Bool(false))];
    // the layer as its own map, half way: nothing moves; all the way: everything moves by the most
    let half = go(&DISPLACEMENT, &[none[0].clone(), ("useForHorizontalDisplacement", Value::Choice(9)), ("useForVerticalDisplacement", Value::Choice(10))], img.clone());
    assert_eq!(half.img.px, img.px);
    let full = go(&DISPLACEMENT, &[none[0].clone(), ("useForHorizontalDisplacement", Value::Choice(8)), ("useForVerticalDisplacement", Value::Choice(10)), ("maxHorizontalDisplacement", n(3.0))], img.clone());
    assert!(near(get(&full.img, 10, 5), get(&img, 13, 5), 1e-5), "takes from 3 to the right");
    let with = [none[0].clone(), ("displacementMapLayer", Value::Layer(Some(4))), ("maxHorizontalDisplacement", n(4.0)), ("useForVerticalDisplacement", Value::Choice(10))];
    let out = go_with(&DISPLACEMENT, &with, img.clone(), Some(&Halves));
    assert!(near(get(&out.img, 10, 5), get(&img, 6, 5), 1e-5) && near(get(&out.img, 30, 5), get(&img, 34, 5), 1e-5), "black pushes one way, white the other");
    let mut wrapped = with.to_vec();
    wrapped.push(("wrapPixelsAround", Value::Bool(true)));
    let w = go_with(&DISPLACEMENT, &wrapped, img.clone(), Some(&Halves));
    assert!(near(get(&w.img, 38, 5), get(&img, 2, 5), 1e-5), "round to the other side");
    let grown = go_with(&DISPLACEMENT, &with[1..], img.clone(), Some(&Halves));
    assert_eq!(grown.img.width, 48, "room for the push");
}

/// A map 20 × 10, all white.
struct Small;

impl crate::Host for Small {
    fn layer(&self, _: u64, _: Take) -> Option<(Buf, Rect)> {
        Some((buf(Image::filled(20, 10, [1.0; 4])), Rect::from_size(20.0, 10.0)))
    }
}

#[test]
fn a_smaller_map_centred_stretched_or_tiled() {
    let img = picture(40, 20);
    let base = [("expandOutput", Value::Bool(false)), ("displacementMapLayer", Value::Layer(Some(4))), ("maxHorizontalDisplacement", n(2.0)), ("useForVerticalDisplacement", Value::Choice(10))];
    let centred = go_with(&DISPLACEMENT, &base, img.clone(), Some(&Small));
    assert_eq!(get(&centred.img, 2, 2), get(&img, 2, 2), "outside the map nothing moves");
    assert!(near(get(&centred.img, 20, 10), get(&img, 22, 10), 1e-5));
    for behaviour in [1, 2] {
        let mut v = base.to_vec();
        v.push(("displacementMapBehavior", Value::Choice(behaviour)));
        let all = go_with(&DISPLACEMENT, &v, img.clone(), Some(&Small));
        assert!(near(get(&all.img, 2, 2), get(&img, 4, 2), 1e-5), "behaviour {behaviour}: everywhere");
    }
}

#[test]
fn optics() {
    let img = picture(60, 40);
    assert_eq!(go(&OPTICS, &[], img.clone()).img.px, img.px);
    let wide = go(&OPTICS, &[("fieldOfView", n(90.0))], img.clone());
    assert!(near(get(&wide.img, 30, 20), get(&img, 30, 20), 0.02), "the middle stays");
    // put in, the picture is pushed out toward the corners (they show what was nearer the middle)
    assert!(get(&wide.img, 55, 20)[0] < get(&img, 55, 20)[0]);
    let back = go(&OPTICS, &[("fieldOfView", n(90.0)), ("reverseLensDistortion", Value::Bool(true))], img.clone());
    assert!(get(&back.img, 55, 20)[0] > get(&img, 55, 20)[0]);
    // taking out what was put in brings it back, near the middle
    let again = go(&OPTICS, &[("fieldOfView", n(60.0)), ("reverseLensDistortion", Value::Bool(true))], go(&OPTICS, &[("fieldOfView", n(60.0))], img.clone()).img);
    for (x, y) in [(20, 15), (40, 25)] {
        assert!((0..2).all(|k| (get(&again.img, x, y)[k] - get(&img, x, y)[k]).abs() < 0.02), "({x}, {y})");
    }
    let fits = go(&OPTICS, &[("fieldOfView", n(120.0)), ("optimalPixels", Value::Bool(true))], img.clone());
    assert!(fits.img.px.iter().all(|p| p[3] > 0.99), "optimal: no empty corners");
    let grown = go(&OPTICS, &[("fieldOfView", n(90.0)), ("reverseLensDistortion", Value::Bool(true)), ("resize", Value::Choice(1))], img.clone());
    assert!(grown.img.width > 60);
}

#[test]
fn magnify() {
    let img = picture(60, 60);
    let out = go(&MAGNIFY, &[("size", n(20.0))], img.clone());
    // twice: 10 out shows 5 out; past the glass the layer as it was
    assert!(near(px_at(&out, 40.5, 30.5), get(&img, 35, 30), 1e-5));
    assert_eq!(get(&out.img, 2, 2), get(&img, 2, 2));
    let alone = go(&MAGNIFY, &[("size", n(20.0)), ("blendingMode", Value::Choice(0))], img.clone());
    assert_eq!(get(&alone.img, 2, 2)[3], 0.0);
    let square = go(&MAGNIFY, &[("size", n(20.0)), ("shape", Value::Choice(1))], img.clone());
    assert!(near(px_at(&square, 46.5, 46.5), get(&img, 38, 38), 1e-5), "a corner of the square glass");
    assert_eq!(get(&out.img, 46, 46), get(&img, 46, 46), "outside the round one");
    let soft = go(&MAGNIFY, &[("size", n(20.0)), ("scaling", Value::Choice(1))], img.clone());
    assert_ne!(soft.img.px, out.img.px);
    let faint = go(&MAGNIFY, &[("size", n(20.0)), ("opacity", n(0.0))], img.clone());
    assert_eq!(faint.img.px, img.px);
}

#[test]
fn mesh_warp() {
    let img = picture(40, 40);
    assert_eq!(go(&MESH_WARP, &[], img.clone()).img.px, img.px);
    let one = |m: &str| go(&MESH_WARP, &[("rows", n(2.0)), ("columns", n(2.0)), ("distortionMesh", Value::Text(m.into()))], img.clone());
    // the middle point of a 2 × 2 grid moved 6 right
    let out = one("0,0 0,0 0,0  0,0 6,0 0,0  0,0 0,0 0,0");
    assert!(near_rg(get(&out.img, 26, 20), get(&img, 20, 20), 0.03), "{:?} {:?}", get(&out.img, 26, 20), get(&img, 20, 20));
    // the corners stay (the pixels by them move a hair, the bend being smooth)
    assert!(near_rg(get(&out.img, 0, 0), get(&img, 0, 0), 0.01) && near_rg(get(&out.img, 39, 39), get(&img, 39, 39), 0.01));
    // smoothly: no kink half way between grid points
    let (a, b, c) = (get(&out.img, 20, 10)[0], get(&out.img, 20, 15)[0], get(&out.img, 20, 20)[0]);
    assert!(a >= b - 1e-4 && b >= c - 1e-4);
    // a corner pulled out: the buffer grows to hold it
    let out = one("-10,-10 0,0 0,0  0,0 0,0 0,0  0,0 0,0 0,0");
    assert!(out.img.width >= 50 && out.origin.x <= -10.0);
    assert_eq!(one("1,1 2,2").img.px, img.px, "the wrong number of points: left alone");
}

#[test]
fn bezier_warp() {
    let img = picture(40, 30);
    let same = go(&BEZIER_WARP, &[], img.clone());
    assert!(same.img.px.iter().zip(&img.px).all(|(a, b)| near(*a, *b, 1e-3)), "as it was");
    // the top's handles pulled up: the top bows out above the layer
    let bowed = go(&BEZIER_WARP, &[("topLeftTangent", pt(13.0, -12.0)), ("topRightTangent", pt(27.0, -12.0))], img.clone());
    assert!(bowed.origin.y < -5.0, "{:?}", bowed.origin);
    assert!(px_at(&bowed, 20.0, -5.0)[3] > 0.99 && px_at(&bowed, 1.0, -5.0)[3] == 0.0);
    assert!(near(px_at(&bowed, 20.0, 29.5), get(&img, 20, 29), 0.05), "the bottom stays: {:?} {:?} {:?}", px_at(&bowed, 20.0, 29.5), get(&img, 20, 29), bowed.origin);
}

#[test]
fn bend() {
    let img = picture(40, 60);
    assert_eq!(go(&BEND, &[], img.clone()).img.px, img.px);
    let out = go(&BEND, &[("bend", n(90.0))], img.clone());
    // before the start (above 12) as it was; after it, bent away
    assert_eq!(get(&out.img, 10, 5), get(&img, 10, 5));
    assert_ne!(out.img.px, img.px);
    let none = go(&BEND, &[("bend", n(90.0)), ("renderPrestart", Value::Choice(0))], img.clone());
    assert_eq!(get(&none.img, 10, 5)[3], 0.0);
    let other = go(&BEND, &[("bend", n(-90.0))], img.clone());
    assert_ne!(other.img.px, out.img.px, "the other way");
}

#[test]
fn lens() {
    let img = picture(40, 40);
    let out = go(&LENS, &[], img.clone());
    assert_eq!(get(&out.img, 0, 0)[3], 0.0, "nothing round the lens");
    assert!(near_rg(get(&out.img, 20, 20), get(&img, 20, 20), 0.03));
    let conv = go(&LENS, &[("convergence", n(60.0))], img.clone());
    assert_ne!(conv.img.px, out.img.px);
}

#[test]
fn page_turn() {
    let img = picture(60, 60);
    let out = go(&PAGE_TURN, &[("foldRadius", n(5.0))], img.clone());
    // far behind the fold the page lies flat as it was; past the curl nothing
    assert_eq!(get(&out.img, 5, 5), get(&img, 5, 5));
    assert_eq!(get(&out.img, 59, 59)[3], 0.0);
    assert_ne!(out.img.px, img.px);
    let front = go(&PAGE_TURN, &[("foldRadius", n(5.0)), ("render", Value::Choice(2))], img.clone());
    let backs = out.img.px.iter().zip(&front.img.px).filter(|(a, b)| a != b).count();
    assert!(backs > 50, "the back shows over part of the front: {backs}");
    let paper = go(&PAGE_TURN, &[("backOpacity", n(0.0)), ("paperColor", Value::Color(veymelo_color::rgba(1.0, 0.0, 0.0, 1.0))), ("render", Value::Choice(1)), ("foldRadius", n(5.0))], img.clone());
    assert!(paper.img.px.iter().any(|p| p[3] > 0.9 && p[0] > 0.5 && p[1] < 0.01 && p[2] < 0.01), "red paper on the back");
}

#[test]
fn tiles() {
    let img = picture(40, 40);
    let out = go(&TILER, &[], img.clone());
    // half the size, a tile centred: the picture over again every 20 pixels
    for (x, y) in [(5, 7), (12, 3), (3, 16)] {
        assert!(near(get(&out.img, x, y), get(&out.img, x + 20, y), 1e-5) && near(get(&out.img, x, y), get(&out.img, x, y + 20), 1e-5));
    }
    assert!(near_rg(get(&out.img, 20, 20), get(&img, 20, 20), 0.04), "{:?}", get(&out.img, 20, 20));
    let kept = go(&TILER, &[("blendWithOriginal", n(100.0))], img.clone());
    assert!(kept.img.px.iter().zip(&img.px).all(|(a, b)| near(*a, *b, 1e-6)));
    assert_eq!(go(&GRIDDLER, &[], img.clone()).img.px, img.px);
    let cut = go(&GRIDDLER, &[("cutTiles", Value::Bool(true)), ("tileSize", n(50.0))], img.clone());
    assert_eq!(get(&cut.img, 0, 0)[3], 0.0, "a gap at each tile's edge");
    assert!(near(get(&cut.img, 2, 2), get(&img, 2, 2), 1e-5));
    let turned = go(&GRIDDLER, &[("rotation", n(45.0))], img.clone());
    assert_ne!(turned.img.px, img.px);
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "distort"), "{}", spec.def.id);
    }
    assert_eq!(mesh_moves("1,2 3.5,-4;5,6 bad 7"), vec![[1.0, 2.0], [3.5, -4.0], [5.0, 6.0]]);
}
