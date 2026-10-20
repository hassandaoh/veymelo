use super::*;
use crate::testing::{at as px_at, instance, n, run_when, square};
use veymelo_geom::Rect;
use veymelo_keyframe::{Keyframe, Value};
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

fn near_rg(a: Px, b: Px, tol: f32) -> bool {
    (0..2).all(|k| (a[k] - b[k]).abs() <= tol)
}

fn go(spec: &Spec, values: &[(&str, Value)], img: Image) -> Buf {
    go_masked(spec, values, img, &[])
}

fn go_masked(spec: &Spec, values: &[(&str, Value)], img: Image, masks: &[crate::MaskShape]) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(img.width as f64, img.height as f64));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(spec, &g, img, Tick::ZERO, masks, None)
}

#[test]
fn bender_leans_from_the_base() {
    let img = picture(40, 60);
    assert_eq!(go(&BENDER, &[], img.clone()).img.px, img.px);
    let out = go(&BENDER, &[("amount", n(40.0))], img.clone());
    // the base stays, the top leans right (it shows what was to its left)
    assert!(near_rg(px_at(&out, 20.0, 59.5), get(&img, 20, 59), 0.02));
    assert!(px_at(&out, 30.0, 1.0)[0] < get(&img, 30, 1)[0] - 0.1, "{:?}", px_at(&out, 30.0, 1.0));
    let mut kinds = Vec::new();
    for style in 0..4 {
        kinds.push(go(&BENDER, &[("amount", n(40.0)), ("style", Value::Choice(style))], img.clone()).img.px);
    }
    for i in 0..4 {
        for j in i + 1..4 {
            assert_ne!(kinds[i], kinds[j], "styles {i} and {j}");
        }
    }
}

#[test]
fn blobs_are_lit() {
    // a white disc on nothing
    let mut img = Image::new(60, 60);
    for y in 0..60 {
        for x in 0..60 {
            if (x as f64 - 29.5).hypot(y as f64 - 29.5) < 18.0 {
                img.px[y * 60 + x] = [0.6, 0.6, 0.6, 1.0];
            }
        }
    }
    let out = go(&BLOBBYLIZE, &[], img.clone());
    assert!(get(&out.img, 30, 30)[3] > 0.9 && get(&out.img, 2, 2)[3] == 0.0);
    // lit from the top left: brighter there than at the bottom right
    let (tl, br) = (get(&out.img, 18, 18), get(&out.img, 41, 41));
    assert!(tl[3] > 0.5 && br[3] > 0.5 && tl[0] / tl[3] > br[0] / br[3] + 0.05, "{tl:?} {br:?}");
    let cut = go(&BLOBBYLIZE, &[("cutAway", n(60.0))], img.clone());
    let area = |b: &Buf| b.img.px.iter().filter(|p| p[3] > 0.5).count();
    assert!(area(&cut) < area(&out));
}

#[test]
fn power_pin() {
    let img = picture(40, 30);
    let same = go(&POWER_PIN, &[], img.clone());
    assert!(same.img.px.iter().zip(&img.px).all(|(a, b)| near_rg(*a, *b, 1e-3)));
    let out = go(&POWER_PIN, &[("topRight", pt(60.0, -10.0))], img.clone());
    assert!(out.img.width >= 60 && out.origin.y <= -10.0, "{} {:?}", out.img.width, out.origin);
    assert!(px_at(&out, 55.0, -6.0)[3] > 0.5);
    let flat = go(&POWER_PIN, &[("topRight", pt(60.0, -10.0)), ("perspective", n(0.0))], img.clone());
    assert_ne!(flat.img.px, out.img.px);
    // unstretched: the middle quarter between the pins fills the layer
    let back = go(&POWER_PIN, &[("unstretch", Value::Bool(true)), ("topLeft", pt(10.0, 7.5)), ("topRight", pt(30.0, 7.5)), ("bottomLeft", pt(10.0, 22.5)), ("bottomRight", pt(30.0, 22.5))], img.clone());
    assert!(near_rg(px_at(&back, 0.5, 0.5), get(&img, 10, 7), 0.03), "{:?}", px_at(&back, 0.5, 0.5));
    assert!(near_rg(px_at(&back, 39.5, 29.5), get(&img, 29, 22), 0.03));
}

#[test]
fn ripple_pulses_follow_the_level() {
    let img = picture(60, 60);
    let mut g = instance(&RIPPLE_PULSE, &[("center", pt(30.0, 30.0))]);
    let still = run_when(&RIPPLE_PULSE, &g, img.clone(), Tick::from_seconds(0.5), &[], None);
    assert_eq!(still.img.px, img.px, "no pulse, no ripples");
    g.prop_mut("pulseLevel").unwrap().set_keys(vec![Keyframe::new(Tick::ZERO, n(100.0)), Keyframe::new(Tick::from_seconds(0.2), n(0.0))]).unwrap();
    let out = run_when(&RIPPLE_PULSE, &g, img.clone(), Tick::from_seconds(0.5), &[], None);
    assert_ne!(out.img.px, img.px);
    // the pulse was 0.3 to 0.5 s ago: the ripples are 9 to 15 out (30 a second), not in the middle
    assert!(near_rg(get(&out.img, 30, 30), get(&img, 30, 30), 1e-4));
    assert!((0..60).any(|x| get(&out.img, x, 30) != get(&img, x, 30)));
    let later = run_when(&RIPPLE_PULSE, &g, img.clone(), Tick::from_seconds(3.0), &[], None);
    assert_eq!(later.img.px, img.px, "they have died away");
}

#[test]
fn slant_and_smudge() {
    let img = picture(40, 40);
    let out = go(&SLANT, &[("slant", n(50.0))], img.clone());
    assert!(near_rg(px_at(&out, 20.0, 39.5), get(&img, 20, 39), 0.02), "the floor stays");
    assert!(px_at(&out, 30.0, 0.5)[0] < get(&img, 30, 0)[0] - 0.2, "the top leans right");
    let tall = go(&SLANT, &[("height", n(50.0))], img.clone());
    assert_eq!(px_at(&tall, 20.0, 5.0)[3], 0.0, "half as tall");
    let s = go(&SMUDGE, &[("reach", n(100.0)), ("radius", n(10.0))], img.clone());
    // the point at To shows what was at From
    assert!(near_rg(px_at(&s, 26.0, 20.0), get(&img, 14, 20), 0.03), "{:?} {:?}", px_at(&s, 26.0, 20.0), get(&img, 14, 20));
    assert!(near_rg(px_at(&s, 20.0, 2.0), get(&img, 20, 2), 1e-4), "far off it stays");
}

#[test]
fn splits_open() {
    let img = picture(40, 40);
    let out = go(&SPLIT, &[], img.clone());
    assert_eq!(get(&out.img, 20, 20)[3], 0.0, "open in the middle");
    assert_eq!(get(&out.img, 2, 20), get(&img, 2, 20), "past the ends it stays");
    let one = go(&SPLIT2, &[("split2", n(0.0))], img.clone());
    // only one side (below the line from A to B, going right) opens
    assert!(get(&one.img, 20, 17)[3] > 0.99 && get(&one.img, 20, 21)[3] == 0.0, "{:?}", get(&one.img, 20, 21));
}

#[test]
fn smear_moves_a_masked_part() {
    let img = picture(60, 60);
    let masks = [square(20.0, 20.0, 10.0), square(5.0, 5.0, 50.0)];
    let v = [("sourceMask", n(1.0)), ("boundaryMask", n(2.0)), ("maskOffset", pt(10.0, 0.0))];
    let out = go_masked(&SMEAR, &v, img.clone(), &masks);
    // the source square now sits 10 to the right
    assert!(near_rg(get(&out.img, 34, 25), get(&img, 24, 25), 0.02), "{:?}", get(&out.img, 34, 25));
    assert_eq!(get(&out.img, 2, 2), get(&img, 2, 2), "past the boundary it stays");
    assert_eq!(go_masked(&SMEAR, &[("sourceMask", n(1.0))], img.clone(), &masks).img.px, img.px, "no boundary, nothing");
}

#[test]
fn reshape_fits_one_outline_to_another() {
    let img = picture(60, 60);
    let masks = [square(20.0, 20.0, 10.0), square(15.0, 15.0, 20.0), square(2.0, 2.0, 56.0)];
    let v = [("sourceMask", n(1.0)), ("destinationMask", n(2.0)), ("boundaryMask", n(3.0))];
    let out = go_masked(&RESHAPE, &v, img.clone(), &masks);
    // the small square's corner is now at the big one's
    assert!(near_rg(get(&out.img, 15, 15), get(&img, 20, 20), 0.04), "{:?} {:?}", get(&out.img, 15, 15), get(&img, 20, 20));
    assert!(near_rg(get(&out.img, 34, 34), get(&img, 29, 29), 0.04));
    assert_eq!(get(&out.img, 0, 0), get(&img, 0, 0));
    let pairs = pairs_of("0.5,0.5 0,0 bad 1.25,0.75");
    assert_eq!(pairs, vec![(0.0, 0.0), (0.5, 0.5), (0.25, 0.75)]);
    assert!((source_fraction(&pairs, 0.25, true) - 0.25).abs() < 1e-12);
}

#[test]
fn old_twirl_and_warps() {
    let img = picture(40, 40);
    let old = go(&TWIRL_LEGACY, &[("angle", n(90.0))], img.clone());
    let new = run_when(&crate::distort::TWIRL, &{
        let mut g = instance(&crate::distort::TWIRL, &[("angle", n(90.0))]);
        crate::distort::TWIRL.start(&mut g, Rect::from_size(40.0, 40.0));
        g
    }, img.clone(), Tick::ZERO, &[], None);
    assert_ne!(old.img.px, new.img.px);
    assert_eq!(get(&old.img, 0, 0), get(&img, 0, 0));
    assert_eq!(go(&WARP, &[], img.clone()).img.px, img.px);
    let mut seen = Vec::new();
    for style in 0..WARP_STYLES.len() as u32 {
        let out = go(&WARP, &[("warpStyle", Value::Choice(style)), ("bend", n(50.0))], img.clone());
        assert_ne!(out.img.px.len(), 0);
        assert!(!seen.contains(&out.img.px), "{} looks like another", WARP_STYLES[style as usize]);
        seen.push(out.img.px);
    }
    let arc = go(&WARP, &[("bend", n(50.0))], img.clone());
    let pad = (arc.img.width - 40) / 2;
    // an arc bends the middle up: the middle column shows what was lower
    assert!(get(&arc.img, 20 + pad, 10 + pad)[1] > get(&img, 20, 10)[1] + 0.05);
}

#[test]
fn upscale() {
    let img = picture(20, 20);
    assert_eq!(go(&UPSCALE, &[], img.clone()).img.px, img.px);
    let out = go(&UPSCALE, &[("scale", n(200.0))], img.clone());
    assert_eq!((out.img.width, out.img.height), (40, 40));
    assert!((out.origin.x + 10.0).abs() < 1e-9 && (out.origin.y + 10.0).abs() < 1e-9, "about the middle: {:?}", out.origin);
    assert!(out.img.px.iter().all(|p| p[3] > 0.999), "edges kept solid");
    // the same place of the picture, now twice as far from the middle
    assert!(near_rg(px_at(&out, 17.0, 7.0), get(&img, 13, 8), 0.06), "{:?} {:?}", px_at(&out, 17.0, 7.0), get(&img, 13, 8));
    let plain = go(&UPSCALE, &[("scale", n(200.0)), ("detail", n(0.0))], img.clone());
    assert_ne!(plain.img.px, out.img.px);
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "distort"), "{}", spec.def.id);
    }
}
