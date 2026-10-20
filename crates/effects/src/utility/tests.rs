use super::*;
use crate::testing::{buf, instance, n, run, run_on};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;
use veymelo_raster::Image;

#[test]
fn grows_bounds() {
    let out = run(&GROW_BOUNDS, &[("pixels", n(5.0))], Image::filled(10, 10, [1.0; 4]));
    assert_eq!((out.img.width, out.img.height), (20, 20));
    assert_eq!((out.origin.x, out.origin.y), (-5.0, -5.0));
    assert_eq!(out.img.px[0], [0.0; 4], "the new room clear");
    assert_eq!(out.img.px[10 * 20 + 10], [1.0; 4], "the layer where it was");
    // under an adjustment layer: nothing to grow
    let g = instance(&GROW_BOUNDS, &[("pixels", n(5.0))]);
    let read = |p: &veymelo_project::Property| p.value_at(veymelo_time::Tick::ZERO);
    let mut ctx = Ctx::new(&g, &read, veymelo_time::Tick::ZERO, Rect::from_size(10.0, 10.0), 1);
    ctx.adjustment = true;
    let same = (GROW_BOUNDS.run)(&ctx, buf(Image::filled(10, 10, [1.0; 4])));
    assert_eq!(same.img.width, 10);
    let _ = run_on;
}

#[test]
fn shows_overbrights() {
    let mut img = Image::new(3, 1);
    img.px = vec![[1.5, 0.2, 0.2, 1.0], [-0.2, 0.5, 0.5, 1.0], [0.5, 0.4, 0.3, 1.0]];
    let out = run(&OVERBRIGHTS, &[], img);
    assert_eq!(out.img.px[0], [1.0, 0.0, 0.0, 1.0], "past white: the highlight colour");
    assert!((out.img.px[1][0] - 0.175).abs() < 1e-6, "the brightest channel in range: dim grey");
    let neg = run(&OVERBRIGHTS, &[("channel", Value::Choice(1))], {
        let mut i = Image::new(1, 1);
        i.px[0] = [-0.2, 0.5, 0.5, 1.0];
        i
    });
    assert_eq!(neg.img.px[0], [0.0, 1.0, 1.0, 1.0], "below black: the opposite colour");
    assert!((out.img.px[2][0] - 0.175).abs() < 1e-6);
}

#[test]
fn compresses_highlights() {
    let mut img = Image::new(4, 1);
    img.px = vec![[0.3, 0.2, 0.1, 1.0], [2.0, 1.0, 0.5, 1.0], [8.0, 4.0, 2.0, 1.0], [0.9, 0.9, 0.9, 1.0]];
    let out = run(&HDR_COMPRESSION, &[], img.clone());
    assert_eq!(out.img.px[0], img.px[0], "below the bend: as it was");
    for p in &out.img.px[1..3] {
        assert!(p[0] < 1.0 && (p[1] / p[0] - 0.5).abs() < 1e-5, "under white, the hue kept: {p:?}");
    }
    assert!(out.img.px[2][0] > out.img.px[1][0], "brighter stays brighter");
    let none = run(&HDR_COMPRESSION, &[("amount", n(0.0))], img.clone());
    assert_eq!(none.img.px, img.px);
}

#[test]
fn compander_undoes_itself() {
    let mut img = Image::new(3, 1);
    img.px = vec![[3.0, 1.0, 0.25, 1.0], [-0.05, 0.0, 6.0, 0.5], [0.5, 0.5, 0.5, 1.0]];
    let set = [("gain", n(16.0)), ("gamma", n(2.2))];
    let squeezed = run(&HDR_COMPANDER, &set, img.clone());
    assert!(squeezed.img.px.iter().all(|p| p[0] <= p[3] + 1e-6 && p[2] <= p[3] + 1e-6), "all within 0 to 1");
    let back = run(&HDR_COMPANDER, &[set[0].clone(), set[1].clone(), ("mode", Value::Choice(1))], squeezed.img);
    for (a, b) in back.img.px.iter().zip(&img.px) {
        assert!((0..4).all(|k| (a[k] - b[k]).abs() < 1e-4), "{a:?} {b:?}");
    }
}

#[test]
fn cineon_both_ways() {
    let c = Cineon { rolloff: 0.0, ..Cineon::STANDARD };
    assert!((c.to_linear(685.0 / 1023.0) - 1.0).abs() < 1e-5, "white");
    assert!(c.to_linear(95.0 / 1023.0).abs() < 1e-5, "black");
    let soft = Cineon { rolloff: 20.0, ..Cineon::STANDARD };
    // (past about code 770 the roll-off is within a float of the internal white)
    for i in 0..=16 {
        let v = 0.1 + i as f32 * 0.04;
        assert!((soft.to_log(soft.to_linear(v)) - v).abs() < 1e-4, "{v}");
    }
    assert!(soft.to_linear(1.0) <= 1.0 && c.to_linear(1.0) > 10.0, "the brightest rolled off to the internal white, not past it");
    // the effect: log to linear, and log to log with the standard settings changes nothing
    let mut img = Image::new(2, 1);
    img.px = vec![[0.67, 0.4, 0.2, 1.0], [0.1, 0.5, 0.72, 1.0]];
    let lin = run(&CINEON, &[("highlightRolloff", n(0.0))], img.clone());
    assert!((lin.img.px[0][0] - c.to_linear(0.67)).abs() < 1e-6);
    let same = run(&CINEON, &[("conversionType", Value::Choice(2))], img.clone());
    for (a, b) in same.img.px.iter().zip(&img.px) {
        assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-4), "{a:?} {b:?}");
    }
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "utility"), "{}", spec.def.id);
    }
}
