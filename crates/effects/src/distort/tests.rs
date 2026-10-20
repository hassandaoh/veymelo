use super::*;
use crate::testing::{at as px_at, buf, instance, n, run, run_when};
use veymelo_keyframe::{Keyframe, Value};
use veymelo_raster::Px;

/// A picture with something different in every pixel.
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

fn near(a: Px, b: Px, tol: f32) -> bool {
    (0..4).all(|k| (a[k] - b[k]).abs() <= tol)
}

/// Red and green (across and down the picture) alike.
fn near_rg(a: Px, b: Px, tol: f32) -> bool {
    (0..2).all(|k| (a[k] - b[k]).abs() <= tol)
}

fn get(img: &Image, x: usize, y: usize) -> Px {
    img.px[y * img.width + x]
}

fn centred(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    let mut g = instance(spec, values);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap().set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, &[], None)
}

#[test]
fn twirl_reaches_by_the_longer_side() {
    let img = picture(60, 20);
    let out = centred(&TWIRL, &[("angle", n(90.0)), ("twirlRadius", n(50.0))], img.clone());
    // 50 % of 60 is 30: a pixel 20 right of the middle turns, the corners do not
    assert!(!near(get(&out.img, 50, 10), get(&img, 50, 10), 1e-4));
    assert_eq!(get(&out.img, 0, 0), get(&img, 0, 0));
    // the very middle hardly moves (a step down the picture is 0.05 of green)
    assert!(near_rg(get(&out.img, 30, 10), get(&img, 30, 10), 0.06), "{:?} {:?}", get(&out.img, 30, 10), get(&img, 30, 10));
    assert_eq!(centred(&TWIRL, &[], img.clone()).img.px, img.px, "no angle, no change");
}

#[test]
fn spheres_and_bulges() {
    let img = picture(40, 40);
    assert_eq!(centred(&SPHERIZE, &[], img.clone()).img.px, img.px, "radius 0");
    let ball = centred(&SPHERIZE, &[("radius", n(15.0))], img.clone());
    assert!(!near(get(&ball.img, 28, 20), get(&img, 28, 20), 1e-4) && get(&ball.img, 0, 0) == get(&img, 0, 0));
    // the ball's middle comes forward, bigger: a point 8 out shows what was nearer the middle
    assert!(get(&ball.img, 28, 20)[0] < get(&img, 28, 20)[0]);
    let big = [("horizontalRadius", n(40.0)), ("verticalRadius", n(40.0)), ("bulgeHeight", n(2.0))];
    let free = centred(&BULGE, &big, img.clone());
    let mut held = big.to_vec();
    held.push(("pinAllEdges", Value::Bool(true)));
    let pinned = centred(&BULGE, &held, img.clone());
    assert!(!near(get(&free.img, 20, 0), get(&img, 20, 0), 1e-4));
    assert!(near(get(&pinned.img, 20, 0), get(&img, 20, 0), 1e-4) && near(get(&pinned.img, 0, 20), get(&img, 0, 20), 1e-4), "edges held");
    let mut tapered = big.to_vec();
    tapered.push(("taperRadius", n(100.0)));
    assert!(!near(get(&centred(&BULGE, &tapered, img.clone()).img, 30, 20), get(&free.img, 30, 20), 1e-4));
    // sinking in: the middle shows less of the picture than swelling out does
    let sunk = centred(&BULGE, &[("horizontalRadius", n(40.0)), ("verticalRadius", n(40.0)), ("bulgeHeight", n(-1.0))], img.clone());
    assert!(get(&sunk.img, 26, 20)[0] > get(&img, 26, 20)[0] && get(&free.img, 26, 20)[0] < get(&img, 26, 20)[0]);
}

#[test]
fn waves_and_their_pins() {
    for k in 0..WAVE_TYPES.len() as u32 {
        for i in 0..300 {
            let v = wave(k, i as f64 * 0.137 - 5.0, 9);
            assert!((-1.0001..=1.0001).contains(&v), "{} {v}", WAVE_TYPES[k as usize]);
        }
    }
    assert!((wave(0, 1.0, 0) - 1f64.sin()).abs() < 1e-12);
    let img = picture(40, 30);
    // running up (0°) they push sideways; held on the left edge, the left column stays
    let held = run(&WAVE_WARP, &[("direction", n(0.0)), ("waveHeight", n(5.0)), ("pinning", Value::Choice(3))], img.clone());
    let pad = (held.img.width - 40) / 2;
    for y in 0..30 {
        assert!(near(get(&held.img, pad, y + pad), get(&img, 0, y), 0.01), "row {y}");
    }
    let free = run(&WAVE_WARP, &[("direction", n(0.0)), ("waveHeight", n(5.0))], img.clone());
    assert_ne!(free.img.px, held.img.px);
    // running right (90°) they push up and down; held at the top and the bottom
    for (pin, row) in [(4, 0), (6, 29)] {
        let out = run(&WAVE_WARP, &[("waveHeight", n(5.0)), ("pinning", Value::Choice(pin))], img.clone());
        let pad = (out.img.width - 40) / 2;
        for x in 0..40 {
            assert!(near(get(&out.img, x + pad, row + pad), get(&img, x, row), 0.01), "pin {pin} column {x}");
        }
        assert!((0..40).any(|x| !near(get(&out.img, x + pad, 15 + pad), get(&img, x, 15), 1e-3)), "the middle still moves");
    }
    // the waves run on with time
    let g = instance(&WAVE_WARP, &[("waveHeight", n(5.0))]);
    let (now, later) = (run_when(&WAVE_WARP, &g, img.clone(), Tick::ZERO, &[], None), run_when(&WAVE_WARP, &g, img.clone(), Tick::from_seconds(0.25), &[], None));
    assert_ne!(now.img.px, later.img.px);
    let whole = run_when(&WAVE_WARP, &g, img, Tick::from_seconds(1.0), &[], None);
    assert!(now.img.px.iter().zip(&whole.img.px).all(|(a, b)| near(*a, *b, 1e-3)), "a wave a second: back where it was");
}

#[test]
fn ripples() {
    let img = picture(40, 40);
    let base = [("radius", n(100.0)), ("waveHeight", n(4.0)), ("waveWidth", n(7.0))];
    let skew = centred(&RIPPLE, &base, img.clone());
    let mut even = base.to_vec();
    even.push(("typeOfConversion", Value::Choice(1)));
    let straight = centred(&RIPPLE, &even, img.clone());
    assert_ne!(skew.img.px, straight.img.px);
    assert_eq!(get(&skew.img, 0, 0), get(&img, 0, 0), "the corners are past half the layer");
    assert_ne!(get(&skew.img, 25, 20), get(&img, 25, 20));
}

#[test]
fn mirrors_and_offsets() {
    let img = picture(40, 20);
    let m = centred(&MIRROR, &[], img.clone());
    for x in 0..20 {
        assert!(near(get(&m.img, 39 - x, 7), get(&img, x, 7), 1e-5), "column {x} shows on the right");
        assert_eq!(get(&m.img, x, 7), get(&img, x, 7));
    }
    let down = centred(&MIRROR, &[("reflectionAngle", n(90.0))], img.clone());
    assert!(near(get(&down.img, 5, 19), get(&img, 5, 0), 1e-5), "the top shows at the bottom");
    let o = centred(&OFFSET, &[("shiftCenterTo", pt(30.0, 10.0))], img.clone());
    for x in 0..40 {
        assert!(near(get(&o.img, x, 3), get(&img, (x + 30) % 40, 3), 1e-5), "slid 10 right, round and round");
    }
    let half = centred(&OFFSET, &[("shiftCenterTo", pt(30.0, 10.0)), ("blendWithOriginal", n(100.0))], img.clone());
    assert!(half.img.px.iter().zip(&img.px).all(|(a, b)| near(*a, *b, 1e-6)));
}

#[test]
fn polar_coordinates() {
    let img = picture(40, 40);
    assert_eq!(run(&POLAR, &[], img.clone()).img.px, img.px);
    let round = run(&POLAR, &[("interpolation", n(100.0))], img.clone());
    // the top row goes to the middle, the bottom row out to the rim; across is round the circle
    assert!(get(&round.img, 20, 20)[1] < 0.1, "{:?}", get(&round.img, 20, 20));
    assert!(get(&round.img, 20, 1)[1] > 0.9 && get(&round.img, 39, 20)[1] > 0.9, "the rim");
    assert!(get(&round.img, 22, 10)[0] < 0.2 && get(&round.img, 17, 10)[0] > 0.8, "just right of the top is the left");
    // and back: the middle of the picture comes back much as it was
    let back = run(&POLAR, &[("interpolation", n(100.0)), ("typeOfConversion", Value::Choice(1))], round.img);
    for (x, y) in [(10, 12), (30, 25), (20, 33)] {
        assert!(near_rg(get(&back.img, x, y), get(&img, x, y), 0.02), "({x}, {y}): {:?} {:?}", get(&back.img, x, y), get(&img, x, y));
    }
}

#[test]
fn transform_moves_scales_and_grows_the_layer() {
    let img = picture(40, 20);
    let same = centred(&TRANSFORM, &[], img.clone());
    assert!(same.img.px.iter().zip(&img.px).all(|(a, b)| near(*a, *b, 1e-5)) && same.img.width == 40);
    // moved 30 right: the buffer grows to hold it
    let moved = centred(&TRANSFORM, &[("position", pt(50.0, 10.0))], img.clone());
    assert!(moved.img.width >= 70, "{}", moved.img.width);
    assert!(near(px_at(&moved, 45.5, 5.5), get(&img, 15, 5), 1e-4) && px_at(&moved, 5.5, 5.5)[3] == 0.0);
    let small = centred(&TRANSFORM, &[("scaleHeight", n(50.0))], img.clone());
    assert_eq!(px_at(&small, 2.5, 2.5)[3], 0.0);
    assert!(near(px_at(&small, 20.0, 10.0), get(&img, 20, 10), 0.05));
    let wide = centred(&TRANSFORM, &[("uniformScale", Value::Bool(false)), ("scaleWidth", n(200.0))], img.clone());
    assert!(wide.img.width >= 80 && wide.img.height == 20);
    let faint = centred(&TRANSFORM, &[("opacity", n(25.0))], img.clone());
    assert!((get(&faint.img, 5, 5)[3] - 0.25).abs() < 1e-5);
    let bilinear = centred(&TRANSFORM, &[("rotation", n(17.0))], img.clone());
    let bicubic = centred(&TRANSFORM, &[("rotation", n(17.0)), ("sampling", Value::Choice(1))], img.clone());
    assert_ne!(bilinear.img.px, bicubic.img.px);
    // under an adjustment layer the picture keeps its size
    let g = instance(&TRANSFORM, &[("position", pt(50.0, 10.0)), ("anchorPoint", pt(20.0, 10.0))]);
    let read = |p: &veymelo_project::Property| p.value_at(Tick::ZERO);
    let mut ctx = Ctx::new(&g, &read, Tick::ZERO, Rect::from_size(40.0, 20.0), 1);
    ctx.adjustment = true;
    assert_eq!((TRANSFORM.run)(&ctx, buf(img)).img.width, 40);
}

#[test]
fn transform_blurs_its_own_motion() {
    let mut sq = Image::new(40, 20);
    for y in 5..15 {
        for x in 15..25 {
            sq.px[y * 40 + x] = [1.0; 4];
        }
    }
    let partial = |b: &Buf| b.img.px.iter().filter(|p| p[3] > 0.02 && p[3] < 0.98).count();
    let mut g = instance(&TRANSFORM, &[("anchorPoint", pt(20.0, 10.0))]);
    // 200 a second to the right; 30 frames a second
    g.prop_mut("position").unwrap().set_keys(vec![Keyframe::new(Tick::ZERO, pt(20.0, 10.0)), Keyframe::new(Tick::from_seconds(1.0), pt(220.0, 10.0))]).unwrap();
    let at = |g: &veymelo_project::Group, shutter: Option<(f64, f64, u32)>| {
        let read = |p: &veymelo_project::Property| p.value_at(Tick::from_seconds(0.5));
        let read_at = |p: &veymelo_project::Property, t: Tick| p.value_at(t);
        let mut ctx = Ctx::new(g, &read, Tick::from_seconds(0.5), Rect::from_size(40.0, 20.0), 1);
        ctx.read_at = Some(&read_at);
        ctx.shutter = shutter;
        (TRANSFORM.run)(&ctx, buf(sq.clone()))
    };
    assert_eq!(partial(&at(&g, None)), 0, "the composition's blur is off: sharp");
    let comp = at(&g, Some((180.0, -90.0, 16)));
    assert!(partial(&comp) > 50, "{}", partial(&comp));
    g.prop_mut("useCompositionsShutterAngle").unwrap().set_value(Value::Bool(false)).unwrap();
    g.prop_mut("shutterAngle").unwrap().set_value(n(360.0)).unwrap();
    let own = at(&g, None);
    assert!(partial(&own) > partial(&comp), "a wider shutter, a longer blur");
    let total = |b: &Buf| b.img.px.iter().map(|p| p[3] as f64).sum::<f64>();
    assert!((total(&own) - 100.0).abs() < 2.0, "as much of it as before: {}", total(&own));
}

#[test]
fn corner_pin() {
    let img = picture(40, 20);
    let same = centred(&CORNER_PIN, &[], img.clone());
    assert!(same.img.px.iter().zip(&img.px).all(|(a, b)| near(*a, *b, 1e-5)));
    // the right side pulled out and in: a trapezoid, the buffer grown to hold it
    let pulled = centred(&CORNER_PIN, &[("upperRight", pt(60.0, 5.0)), ("lowerRight", pt(60.0, 15.0))], img.clone());
    assert!(pulled.img.width >= 60);
    assert!(px_at(&pulled, 2.0, 10.0)[3] > 0.99 && px_at(&pulled, 58.0, 10.0)[3] > 0.99 && px_at(&pulled, 58.0, 2.0)[3] == 0.0);
    assert!(near_rg(px_at(&pulled, 0.5, 10.0), get(&img, 0, 10), 0.03), "the left side stays");
    // crossed: nothing
    let crossed = centred(&CORNER_PIN, &[("upperLeft", pt(40.0, 0.0)), ("upperRight", pt(0.0, 0.0))], img.clone());
    assert!(crossed.img.px.iter().all(|p| p[3] == 0.0));
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "distort"), "{}", spec.def.id);
    }
}
