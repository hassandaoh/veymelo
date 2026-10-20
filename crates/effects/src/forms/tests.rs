use super::*;
use crate::testing::{at as px_at, buf, instance, n, run, run_when, square};
use veymelo_geom::Rect;
use veymelo_keyframe::{Keyframe, Value};
use veymelo_raster::Image;

fn clear(w: usize, h: usize) -> Image {
    Image::new(w, h)
}

fn pt(x: f64, y: f64) -> Value {
    Value::Vec2([x, y])
}

fn secs(s: f64) -> Tick {
    Tick::from_seconds(s)
}

/// Radio waves from the middle of a 100 × 100 layer, one a second, growing 20 pixels a second.
fn waves(extra: &[(&str, Value)]) -> veymelo_project::Group {
    let mut values = vec![("producerPoint", pt(50.0, 50.0)), ("expansion", n(20.0)), ("lifespan", n(10.0)), ("startWidth", n(4.0)), ("endWidth", n(4.0))];
    values.extend(extra.iter().cloned());
    instance(&RADIO_WAVES, &values)
}

#[test]
fn waves_ripple_out_one_a_second() {
    let out = run_when(&RADIO_WAVES, &waves(&[]), clear(100, 100), secs(1.5), &[], None);
    // born at 0 s (30 out now) and at 1 s (10 out)
    assert!(px_at(&out, 79.5, 50.0)[3] > 0.9 && px_at(&out, 59.5, 50.0)[3] > 0.9, "two rings");
    assert!(px_at(&out, 50.0, 50.0)[3] < 0.01 && px_at(&out, 70.0, 50.0)[3] < 0.01 && px_at(&out, 90.0, 50.0)[3] < 0.01, "nothing between them");
    assert!((px_at(&out, 50.0, 20.5)[3] - px_at(&out, 79.5, 50.0)[3]).abs() < 0.05, "round");
    // short-lived: the first has gone
    let short = run_when(&RADIO_WAVES, &waves(&[("lifespan", n(1.0))]), clear(100, 100), secs(1.5), &[], None);
    assert!(px_at(&short, 79.5, 50.0)[3] < 0.01 && px_at(&short, 59.5, 50.0)[3] > 0.9);
    // fading out over the last of their 2 s
    let k = run_when(&RADIO_WAVES, &waves(&[("fadeOutTime", n(2.0)), ("lifespan", n(2.0))]), clear(100, 100), secs(1.5), &[], None);
    assert!(px_at(&k, 79.5, 50.0)[3] < px_at(&k, 59.5, 50.0)[3], "the older one is fainter");
}

#[test]
fn polygons_and_stars() {
    let square_wave = run_when(&RADIO_WAVES, &waves(&[("sides", n(4.0))]), clear(100, 100), secs(1.5), &[], None);
    // a diamond (a corner up) 30 to its corners: its sides cross the diagonals 21 out
    assert!(px_at(&square_wave, 65.0, 65.0)[3] > 0.5 && px_at(&square_wave, 79.5, 50.0)[3] > 0.5, "on the diamond");
    let ring = run_when(&RADIO_WAVES, &waves(&[]), clear(100, 100), secs(1.5), &[], None);
    assert!(px_at(&ring, 65.0, 65.0)[3] < 0.01, "not on the circle");
    // turned an eighth: a square standing on its side, its sides 30 × cos 45° out
    let turned = run_when(&RADIO_WAVES, &waves(&[("sides", n(4.0)), ("orientation", n(45.0))]), clear(100, 100), secs(1.5), &[], None);
    assert!(px_at(&turned, 71.2, 50.0)[3] > 0.5 && px_at(&turned, 79.5, 50.0)[3] < 0.01);
    let star = run_when(&RADIO_WAVES, &waves(&[("sides", n(5.0)), ("star", Value::Bool(true)), ("starDepth", n(-0.5))]), clear(100, 100), secs(1.5), &[], None);
    // a point straight up 30 out, the dip between points half way in
    assert!(px_at(&star, 50.0, 20.5)[3] > 0.5, "a point up");
    let dip = 54.0f64.to_radians().sin_cos();
    assert!(px_at(&star, 50.0 + dip.1 * 15.0, 50.0 - dip.0 * 15.0)[3] > 0.5, "the dip between points");
}

#[test]
fn birth_or_each_frame() {
    let mut g = waves(&[]);
    g.prop_mut("color").unwrap().set_keys(vec![Keyframe::new(secs(0.0), Value::Color(veymelo_color::rgba(1.0, 0.0, 0.0, 1.0))), Keyframe::new(secs(1.0), Value::Color(veymelo_color::rgba(0.0, 0.0, 1.0, 1.0)))]).unwrap();
    let now = run_when(&RADIO_WAVES, &g, clear(100, 100), secs(1.5), &[], None);
    let (outer, inner) = (px_at(&now, 79.5, 50.0), px_at(&now, 59.5, 50.0));
    assert!(outer[2] > 0.9 && inner[2] > 0.9 && outer[0] < 0.01, "each frame: both blue now");
    g.prop_mut("parametersAreSetAt").unwrap().set_value(Value::Choice(0)).unwrap();
    let born = run_when(&RADIO_WAVES, &g, clear(100, 100), secs(1.5), &[], None);
    let (outer, inner) = (px_at(&born, 79.5, 50.0), px_at(&born, 59.5, 50.0));
    assert!(outer[0] > 0.9 && outer[2] < 0.01 && inner[2] > 0.9, "at birth: the first stays red {outer:?} {inner:?}");
}

#[test]
fn moving_and_bouncing() {
    // heading right 40 a second: born at 0, 60 along by 1.5 s
    let moving = run_when(&RADIO_WAVES, &waves(&[("velocity", n(40.0)), ("lifespan", n(1.2))]), clear(200, 100), secs(1.5), &[], None);
    assert!(px_at(&moving, 70.0 + 9.5, 50.0)[3] > 0.9, "the one born at 1 s is 20 along, 10 out");
    assert_eq!(bounce(12.0, 0.0, 10.0), 8.0);
    assert_eq!(bounce(-3.0, 0.0, 10.0), 3.0);
    assert_eq!(bounce(25.0, 0.0, 10.0), 5.0);
    let back = run_when(&RADIO_WAVES, &waves(&[("velocity", n(40.0)), ("lifespan", n(1.2)), ("reflection", Value::Bool(true))]), clear(100, 100), secs(1.5), &[], None);
    // its middle went to 70: still inside, so the same as without reflection
    assert!(px_at(&back, 79.5, 50.0)[3] > 0.9);
    // 160 a second: its middle would be at 130, past the edge; bounced back it is at 70
    let far = |reflect: bool| run_when(&RADIO_WAVES, &waves(&[("velocity", n(160.0)), ("lifespan", n(1.2)), ("reflection", Value::Bool(reflect))]), clear(100, 100), secs(1.5), &[], None);
    assert!(px_at(&far(true), 79.5, 50.0)[3] > 0.9 && px_at(&far(true), 59.5, 50.0)[3] > 0.9, "bounced back");
    assert!(far(false).img.px.iter().all(|p| p[3] < 0.01), "gone past the edge");
}

#[test]
fn waves_from_a_mask() {
    let mask = square(40.0, 40.0, 20.0);
    let g = waves(&[("waveType", Value::Choice(2)), ("mask", n(1.0))]);
    let out = run_when(&RADIO_WAVES, &g, clear(100, 100), secs(1.0), &[mask], None);
    // born at 0 s: 20 out from the square's sides by now; born at 1 s: on them
    assert!(px_at(&out, 79.5, 50.0)[3] > 0.9 && px_at(&out, 50.0, 79.5)[3] > 0.9, "a rounded square 20 out");
    assert!(px_at(&out, 59.5, 50.0)[3] > 0.5, "the new one on the mask's edge");
    assert!(px_at(&out, 50.0, 50.0)[3] < 0.01 && px_at(&out, 70.0, 50.0)[3] < 0.01);
    // without a mask nothing is drawn
    let none = run_when(&RADIO_WAVES, &g, clear(100, 100), secs(1.0), &[], None);
    assert!(none.img.px.iter().all(|p| p[3] == 0.0));
}

/// Gives a white disc 15 across in the middle of a 100 × 100 layer.
struct Disc;

impl crate::Host for Disc {
    fn layer(&self, _: u64, _: Take) -> Option<(Buf, Rect)> {
        let mut m = Image::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                if (x as f64 + 0.5 - 50.0).hypot(y as f64 + 0.5 - 50.0) < 15.0 {
                    m.px[y * 100 + x] = [1.0; 4];
                }
            }
        }
        Some((buf(m), Rect::from_size(100.0, 100.0)))
    }
}

#[test]
fn waves_from_a_pictures_outline() {
    let g = waves(&[("waveType", Value::Choice(1)), ("sourceLayer", Value::Layer(Some(3))), ("sourceCenter", pt(50.0, 50.0)), ("preBlur", n(0.0)), ("tolerance", n(0.0))]);
    let out = run_when(&RADIO_WAVES, &g, clear(100, 100), secs(1.0), &[], Some(&Disc));
    // the disc's edge 15 out; the older wave 20 past it
    assert!(px_at(&out, 64.5, 50.0)[3] > 0.5 && px_at(&out, 84.5, 50.0)[3] > 0.5, "{:?} {:?}", px_at(&out, 64.5, 50.0), px_at(&out, 84.5, 50.0));
    assert!(px_at(&out, 75.0, 50.0)[3] < 0.01 && px_at(&out, 50.0, 50.0)[3] < 0.01);
    // its producer point moved: the outline goes with it
    let moved = waves(&[("waveType", Value::Choice(1)), ("sourceLayer", Value::Layer(Some(3))), ("sourceCenter", pt(50.0, 50.0)), ("producerPoint", pt(30.0, 50.0)), ("preBlur", n(0.0)), ("tolerance", n(0.0))]);
    let out = run_when(&RADIO_WAVES, &moved, clear(100, 100), secs(1.0), &[], Some(&Disc));
    assert!(px_at(&out, 44.5, 50.0)[3] > 0.5 && px_at(&out, 54.5, 50.0)[3] < 0.01);
}

#[test]
fn profiles_shape_the_stroke() {
    assert_eq!(profile(0, 0.7), 1.0);
    assert!(profile(1, -1.0) > 0.99 && profile(1, 1.0) < 0.01, "thin at the front (outside)");
    assert!(profile(2, -1.0) < 0.01 && profile(2, 1.0) > 0.99);
    assert!(profile(3, 0.0) > 0.99 && profile(3, 1.0) < 0.01 && profile(3, -1.0) < 0.01);
    assert!((profile(4, 0.5) - (PI / 4.0).cos()).abs() < 1e-12);
    let wide = |kind: f64| run_when(&RADIO_WAVES, &waves(&[("startWidth", n(10.0)), ("endWidth", n(10.0)), ("profile", Value::Choice(kind as u32))]), clear(100, 100), secs(1.5), &[], None);
    let (sq, both) = (wide(0.0), wide(3.0));
    assert!(px_at(&sq, 83.5, 50.0)[3] > 0.9 && px_at(&both, 83.5, 50.0)[3] < 0.5, "tapered at the edges");
    assert!(px_at(&both, 80.0, 50.0)[3] > 0.85, "full in the middle");
}

fn bolt(extra: &[(&str, Value)]) -> Buf {
    let mut values = vec![("origin", pt(10.0, 50.0)), ("direction", pt(110.0, 50.0)), ("glowRadius", n(0.0)), ("coreOpacity", n(100.0)), ("forking", n(0.0))];
    values.extend(extra.iter().cloned());
    run(&LIGHTNING, &values, Image::filled(120, 100, [0.0, 0.0, 0.0, 1.0]))
}

#[test]
fn lightning_strikes_from_origin_to_target() {
    let out = bolt(&[]);
    assert!(px_at(&out, 10.5, 50.5)[0] > 0.5 && px_at(&out, 109.5, 50.5)[0] > 0.5, "both ends lit");
    // it crosses the middle somewhere near the line
    let near = (0..100).filter(|y| px_at(&out, 60.5, *y as f64)[0] > 0.5).collect::<Vec<_>>();
    assert!(!near.is_empty() && near.iter().all(|y| (*y as f64 - 50.0).abs() < 35.0), "{near:?}");
    assert!(px_at(&out, 60.5, 2.5)[0] < 0.01 && px_at(&out, 2.5, 95.0)[0] < 0.01, "dark away from it");
    assert_eq!(out.img.px, bolt(&[]).img.px, "the same every time");
    assert_ne!(out.img.px, bolt(&[("conductivityState", n(11.0))]).img.px, "another bolt for another state");
    assert_eq!(out.img.px, bolt(&[("conductivityState", n(10.4))]).img.px, "within a whole number it stays");
    let calm = bolt(&[("turbulence", n(0.0))]);
    assert!((0..120).all(|x| px_at(&calm, x as f64 + 0.5, 50.5)[0] > 0.5 || !(11..109).contains(&x)), "no turbulence: straight");
}

#[test]
fn forks_glow_and_compositing() {
    let lit = |b: &Buf| b.img.px.iter().filter(|p| p[0] > 0.3).count();
    let plain = bolt(&[]);
    let forked = bolt(&[("forking", n(60.0))]);
    assert!(lit(&forked) > lit(&plain), "forks light more");
    let glow = bolt(&[("glowRadius", n(20.0)), ("glowOpacity", n(100.0))]);
    let g = px_at(&glow, 60.5, 2.5);
    assert!(lit(&glow) > lit(&plain) && g[2] >= g[0], "a bluish glow around it");
    let alone = run(&LIGHTNING, &[("origin", pt(10.0, 50.0)), ("direction", pt(110.0, 50.0)), ("compositeOnOriginal", Value::Bool(false))], Image::filled(120, 100, [0.0, 1.0, 0.0, 1.0]));
    assert!(px_at(&alone, 2.5, 2.5)[3] < 0.05 && px_at(&alone, 2.5, 2.5)[1] < 0.05, "the layer gone");
    let on = run(&LIGHTNING, &[("origin", pt(10.0, 50.0)), ("direction", pt(110.0, 50.0))], Image::filled(120, 100, [0.0, 1.0, 0.0, 1.0]));
    assert!(px_at(&on, 2.5, 2.5)[1] > 0.95 && px_at(&on, 2.5, 2.5)[3] > 0.99, "over the layer");
    for (kind, name) in LIGHTNING_TYPES.iter().enumerate() {
        let out = bolt(&[("lightningType", Value::Choice(kind as u32))]);
        assert!(lit(&out) > 50, "{name} draws");
    }
}

#[test]
fn obstacles_turn_the_bolt() {
    // the top half opaque: a bolt along the middle stays out of it
    let mut img = Image::new(120, 100);
    for p in &mut img.px[..120 * 50] {
        *p = [0.0, 0.0, 0.0, 1.0];
    }
    let values = |o: f64| vec![("origin", pt(10.0, 50.0)), ("direction", pt(110.0, 50.0)), ("glowRadius", n(0.0)), ("forking", n(0.0)), ("turbulence", n(3.0)), ("alphaObstacle", n(o))];
    let lit_in = |b: &Buf, top: bool| b.img.px.iter().enumerate().filter(|(i, p)| (i / 120 < 50) == top && p[0] > 0.3).count();
    let free = run(&LIGHTNING, &values(0.0), img.clone());
    let away = run(&LIGHTNING, &values(10.0), img.clone());
    let drawn = run(&LIGHTNING, &values(-10.0), img);
    assert!(lit_in(&away, true) < lit_in(&free, true) && lit_in(&drawn, true) > lit_in(&free, true), "{} {} {}", lit_in(&away, true), lit_in(&free, true), lit_in(&drawn, true));
}

#[test]
fn the_mandelbrot_set() {
    let set = |extra: &[(&str, Value)]| {
        let mut v = vec![("mandelbrotX", n(-0.5)), ("mandelbrotY", n(0.0))];
        v.extend(extra.iter().cloned());
        run(&FRACTAL, &v, Image::filled(100, 80, [1.0, 0.0, 0.0, 1.0]))
    };
    let out = set(&[]);
    let (inside, outside) = (px_at(&out, 50.5, 40.5), px_at(&out, 1.5, 40.5));
    assert!(inside == [0.0, 0.0, 0.0, 1.0], "black inside: {inside:?}");
    assert!(outside[3] == 1.0 && outside[..3].iter().any(|c| *c > 0.02), "coloured outside: {outside:?}");
    // the main cardioid's cusp at 0.25: just left of it inside, just right of it outside
    let x = |re: f64| 50.0 + (re + 0.5) / 3.5 * 100.0;
    assert!(px_at(&out, x(0.2), 40.5) == [0.0, 0.0, 0.0, 1.0] && px_at(&out, x(0.4), 40.5) != [0.0, 0.0, 0.0, 1.0]);
    let clear_inside = set(&[("transparency", Value::Bool(true))]);
    assert_eq!(px_at(&clear_inside, 50.5, 40.5)[3], 0.0);
    let over = set(&[("overlay", Value::Bool(true))]);
    assert_eq!(px_at(&over, 50.5, 40.5), [1.0, 0.0, 0.0, 1.0], "the layer shows through its inside");
    // zoomed: each step twice as close
    let zoomed = set(&[("mandelbrotMagnification", n(3.0)), ("mandelbrotX", n(-1.75)), ("mandelbrotY", n(0.0))]);
    assert!(zoomed.img.px.contains(&[0.0, 0.0, 0.0, 1.0]), "the little copy at −1.75");
    assert_ne!(set(&[("cycleOffset", n(90.0))]).img.px, out.img.px, "turning the colours");
    for palette in 1..4 {
        assert_ne!(set(&[("palette", Value::Choice(palette))]).img.px, out.img.px);
    }
    for eq in 1..3 {
        assert_ne!(set(&[("equation", Value::Choice(eq))]).img.px, out.img.px);
    }
}

#[test]
fn julia_sets_and_inversions() {
    // c = 0: the unit disc
    let disc = run(&FRACTAL, &[("setChoice", Value::Choice(4)), ("mandelbrotX", n(0.0)), ("mandelbrotY", n(0.0))], Image::filled(100, 100, [0.0; 4]));
    let x = |re: f64| 50.0 + re / 3.5 * 100.0;
    assert_eq!(px_at(&disc, x(0.9), 50.0), [0.0, 0.0, 0.0, 1.0]);
    assert_ne!(px_at(&disc, x(1.1), 50.0), [0.0, 0.0, 0.0, 1.0]);
    // inverted, the disc's outside comes in and its inside goes out
    let inv = run(&FRACTAL, &[("setChoice", Value::Choice(5)), ("mandelbrotX", n(0.0)), ("mandelbrotY", n(0.0))], Image::filled(100, 100, [0.0; 4]));
    assert_ne!(px_at(&inv, x(0.9), 50.0), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(px_at(&inv, x(1.1), 50.0), [0.0, 0.0, 0.0, 1.0]);
    // over Julia: starting from the Julia point instead of 0
    let plain = run(&FRACTAL, &[], Image::filled(60, 60, [0.0; 4]));
    let over = run(&FRACTAL, &[("setChoice", Value::Choice(2)), ("juliaX", n(0.3)), ("juliaY", n(0.2))], Image::filled(60, 60, [0.0; 4]));
    assert_ne!(plain.img.px, over.img.px);
}

#[test]
fn edges_are_smoothed() {
    let values = |f: f64, how: u32| vec![("mandelbrotX", n(-0.5)), ("mandelbrotY", n(0.0)), ("oversampleFactor", n(f)), ("oversampleMethod", Value::Choice(how))];
    let one = run(&FRACTAL, &values(1.0, 0), Image::new(80, 60));
    let three = run(&FRACTAL, &values(3.0, 0), Image::new(80, 60));
    let all = run(&FRACTAL, &values(3.0, 1), Image::new(80, 60));
    assert!(one.img.px != three.img.px && three.img.px != all.img.px);
    // only pixels at the set's edge are sampled more: most are the same
    let same = one.img.px.iter().zip(&three.img.px).filter(|(a, b)| a == b).count();
    assert!(same > one.img.px.len() / 2, "{same}");
}

#[test]
fn every_one_is_registered_with_its_points() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some(), "{}", spec.def.id);
    }
    let mut g = instance(&LIGHTNING, &[]);
    LIGHTNING.start(&mut g, Rect::from_size(200.0, 100.0));
    assert_eq!(g.prop("origin").unwrap().value_at(Tick::ZERO), pt(50.0, 20.0));
}
