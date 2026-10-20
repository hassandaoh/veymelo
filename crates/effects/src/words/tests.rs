use super::*;
use crate::testing::{at as px_at, instance, n};
use veymelo_keyframe::Value;

/// `spec` set up as `values`, read at layer time `t` (composition time `comp`, at `rate`).
fn with<R>(spec: &Spec, values: &[(&str, Value)], t: f64, comp: f64, rate: FrameRate, f: impl FnOnce(&Ctx) -> R) -> R {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(100.0, 60.0));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    let read = |p: &veymelo_project::Property| p.value_at(Tick::ZERO);
    let mut ctx = Ctx::new(&g, &read, Tick::from_seconds(t), Rect::from_size(100.0, 60.0), 7);
    ctx.comp_time = Tick::from_seconds(comp);
    ctx.rate = rate;
    ctx.frame = rate.frame_duration();
    f(&ctx)
}

fn fps(n: u32, d: u32) -> FrameRate {
    FrameRate::new(n, d).unwrap()
}

fn shown(values: &[(&str, Value)]) -> String {
    with(&NUMBERS, values, 0.0, 3723.5, fps(30, 1), numbers_text)
}

#[test]
fn dates() {
    assert_eq!(date_of(0), (2000, 1, 1, 0), "a Saturday");
    assert_eq!(date_of(59), (2000, 2, 29, 3), "a Tuesday");
    assert_eq!(date_of(366), (2001, 1, 1, 2), "a Monday");
    assert_eq!(date_of(-1), (1999, 12, 31, 6), "a Friday");
    assert_eq!(date_of(9_132), (2025, 1, 1, 4), "a Wednesday");
}

#[test]
fn numbers_say() {
    let kind = |k: u32| ("type", Value::Choice(k));
    assert_eq!(shown(&[("value", n(-12.345)), ("decimalPlaces", n(2.0))]), "-12.35");
    assert_eq!(shown(&[kind(1), ("value", n(42.5)), ("decimalPlaces", n(1.0))]), "00042.5");
    assert_eq!(shown(&[kind(2), ("value", n(61.5))]), "00:01:01:15");
    assert_eq!(shown(&[kind(3), ("value", n(61.5))]), "00:01:01:12");
    assert_eq!(shown(&[kind(2), ("currentTimeDate", Value::Bool(true))]), "01:02:03:15", "the composition's time");
    assert_eq!(shown(&[kind(5), ("value", n(3723.0))]), "01:02:03");
    assert_eq!(shown(&[kind(6), ("value", n(59.0))]), "02/29/00");
    assert_eq!(shown(&[kind(7), ("value", n(366.0))]), "Jan 1, 2001");
    assert_eq!(shown(&[kind(8), ("value", n(0.0))]), "Saturday, January 1, 2000");
    assert_eq!(shown(&[kind(9), ("value", n(255.0))]), "FF");
    assert_eq!(shown(&[kind(9), ("value", n(-26.0))]), "-1A");
    // today: some date, and the same one again
    let today = shown(&[kind(8), ("currentTimeDate", Value::Bool(true))]);
    assert!(today.contains(", 20"), "{today}");
    // random: from 0 to the value, the same for the same frame, another for another
    let draw = |t: f64| with(&NUMBERS, &[("randomValues", Value::Bool(true)), ("value", n(100.0)), ("decimalPlaces", n(3.0))], t, t, fps(30, 1), numbers_text);
    let (a, b, c) = (draw(1.0), draw(1.0), draw(1.5));
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert!((0.0..=100.0).contains(&a.parse::<f64>().unwrap()));
}

#[test]
fn timecode_says() {
    let tc = |values: &[(&str, Value)], t: f64, comp: f64, rate: FrameRate| with(&TIMECODE, values, t, comp, rate, timecode_text);
    assert_eq!(tc(&[], 1.5, 11.5, fps(30, 1)), "00:00:01:15", "the layer's own time");
    assert_eq!(tc(&[("timeSource", Value::Choice(1))], 1.5, 3611.5, fps(30, 1)), "01:00:11:15", "the composition's");
    assert_eq!(tc(&[("timeSource", Value::Choice(2)), ("timeUnits", n(25.0)), ("startingFrame", n(100.0))], 1.0, 0.0, fps(30, 1)), "00:00:05:00");
    assert_eq!(tc(&[("displayFormat", Value::Choice(1))], 2.0, 0.0, fps(24, 1)), "48");
    assert_eq!(tc(&[("displayFormat", Value::Choice(2))], 45.0 / 24.0, 0.0, fps(24, 1)), "2+13", "feet and frames, 16 to a foot");
    assert_eq!(tc(&[("displayFormat", Value::Choice(3))], 45.0 / 24.0, 0.0, fps(24, 1)), "1+05", "40 to a foot");
    // drop-frame: a minute in, the first two numbers skipped
    let ntsc = fps(30_000, 1001);
    let minute = 1800.0 * 1001.0 / 30_000.0;
    assert_eq!(tc(&[("dropFrame", Value::Bool(true))], minute, 0.0, ntsc), "00;01;00;02");
    assert_eq!(tc(&[], minute, 0.0, ntsc), "00:01:00:00");
}

#[test]
fn even_digits_stand_still() {
    let Some(face) = font_for("", 400.0) else { return };
    let width = |s: &str, even: bool| set_line(s, &face, 40.0, 0.0, even, 1, vec2(0.0, 0.0)).1.width();
    assert!((width("1111", true) - width("8888", true)).abs() < 1e-9);
    assert!((width("1:11", true) - width("8:88", true)).abs() < 1e-9);
    // placed by its middle; its capitals' middle on the point
    let (_, r) = set_line("80", &face, 40.0, 0.0, false, 1, vec2(50.0, 30.0));
    assert!(((r.x0 + r.x1) * 0.5 - 50.0).abs() < 1e-9 && ((r.y0 + r.y1) * 0.5 - 30.0).abs() < 1e-9);
    let (_, l) = set_line("80", &face, 40.0, 0.0, false, 0, vec2(50.0, 30.0));
    assert!((l.x0 - 50.0).abs() < 1e-9);
}

/// `spec` with `values` run on a grey 100 × 60 layer.
fn drawn(spec: &Spec, values: &[(&str, Value)]) -> Buf {
    let mut g = instance(spec, &[]);
    spec.start(&mut g, Rect::from_size(100.0, 60.0));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    crate::testing::run_when(spec, &g, Image::filled(100, 60, [0.5, 0.5, 0.5, 1.0]), Tick::ZERO, &[], None)
}

#[test]
fn numbers_drawn() {
    if font_for("", 400.0).is_none() {
        return;
    }
    // a big 8 in the middle: red where its middle bar is, the layer round it
    let eight = drawn(&NUMBERS, &[("value", n(8.0)), ("size", n(50.0))]);
    let red = eight.img.px.iter().filter(|p| p[0] > 0.9 && p[1] < 0.1).count();
    assert!(red > 100, "{red}");
    assert_eq!(px_at(&eight, 3.0, 3.0), [0.5, 0.5, 0.5, 1.0]);
    let alone = drawn(&NUMBERS, &[("value", n(8.0)), ("size", n(50.0)), ("compositeOnOriginal", Value::Bool(false))]);
    assert_eq!(px_at(&alone, 3.0, 3.0), [0.0; 4]);
    // outlined only: white about the edges, no red
    let ring = drawn(&NUMBERS, &[("value", n(8.0)), ("size", n(50.0)), ("displayOptions", Value::Choice(1)), ("strokeWidth", n(3.0))]);
    assert!(ring.img.px.iter().any(|p| p[1] > 0.95) && !ring.img.px.iter().any(|p| p[0] > 0.9 && p[1] < 0.1));
    let both = drawn(&NUMBERS, &[("value", n(8.0)), ("size", n(50.0)), ("displayOptions", Value::Choice(2)), ("strokeWidth", n(3.0))]);
    assert!(both.img.px.iter().filter(|p| p[0] > 0.9 && p[1] < 0.1).count() > red / 3);
}

#[test]
fn timecode_drawn() {
    if font_for("", 400.0).is_none() {
        return;
    }
    let tc = drawn(&TIMECODE, &[("textSize", n(12.0))]);
    // a black box low in the middle, white figures in it, the layer above
    let low: Vec<Px> = (0..100).map(|x| px_at(&tc, x as f64 + 0.5, 54.0)).collect();
    assert!(low.iter().any(|p| p[0] < 0.05) && low.iter().any(|p| p[0] > 0.9), "{low:?}");
    assert_eq!(px_at(&tc, 50.0, 20.0), [0.5, 0.5, 0.5, 1.0]);
    let faint = drawn(&TIMECODE, &[("textSize", n(12.0)), ("opacity", n(50.0)), ("showBox", Value::Bool(false))]);
    assert!(faint.img.px.iter().all(|p| p[0] < 0.76), "half-seen white over grey");
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "text"), "{}", spec.def.id);
    }
}
