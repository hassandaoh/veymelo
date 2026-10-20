use super::*;
use crate::testing::{at as px_at, buf, instance, n, run_when};
use crate::{Host, noise};
use veymelo_geom::Rect;
use veymelo_keyframe::Value;

const FPS: f64 = 30.0;

/// A clip the layer plays: each frame drawn by `draw` from its frame number.
struct Clip {
    w: usize,
    h: usize,
    draw: fn(usize, usize, f64) -> Px,
    /// Another layer (a map) and its size, if any.
    other: Option<fn(usize, usize, f64) -> Px>,
}

impl Clip {
    fn frame(&self, f: f64, draw: fn(usize, usize, f64) -> Px) -> Image {
        let mut img = Image::new(self.w, self.h);
        map_px(&mut img, |x, y, _| draw(x, y, f));
        img
    }
}

impl Host for Clip {
    fn layer(&self, _: u64, _: Take) -> Option<(Buf, Rect)> {
        let d = self.other?;
        Some((buf(self.frame(0.0, d)), Rect::from_size(self.w as f64, self.h as f64)))
    }

    fn this_at(&self, t: Tick, _: usize) -> Option<Buf> {
        Some(buf(self.frame(t.seconds() * FPS, self.draw)))
    }

    fn layer_at(&self, _: u64, shift: Tick, _: Take) -> Option<(Buf, Rect)> {
        let d = self.other?;
        Some((buf(self.frame(shift.seconds() * FPS, d)), Rect::from_size(self.w as f64, self.h as f64)))
    }
}

/// A white dot 2 across moving 4 to the right a frame from x = 4, on nothing (on whole frames
/// it covers whole pixels).
fn dot(x: usize, y: usize, f: f64) -> Px {
    let (x0, cx) = (4.0 + 4.0 * f, x as f64 + 0.5);
    let k = ((cx - x0 + 0.5).clamp(0.0, 1.0) * (x0 + 2.5 - cx).clamp(0.0, 1.0)) as f32;
    if (3..5).contains(&y) { [k; 4] } else { [0.0; 4] }
}

/// Soft grey blobs panning 2 to the right a frame (sub-pixel between frames).
fn pan(x: usize, y: usize, f: f64) -> Px {
    let v = 0.5 + 0.45 * noise::smooth((x as f64 + 0.5 - 2.0 * f) / 6.0 + 0.37, (y as f64 + 0.5) / 6.0 + 0.61, 0.5, 9, None) as f32;
    [v, v, v, 1.0]
}

fn blobs(x: f64, y: f64, seed: u64) -> f32 {
    0.5 + 0.45 * noise::smooth(x / 4.0 + 0.37, y / 4.0 + 0.61, 0.5, seed, None) as f32
}

/// A light, mottled square 12 across (rows 10 to 24) moving 6 to the right a frame from x = 8,
/// over a still, dark, mottled ground.
fn thing(x: usize, y: usize, f: f64) -> Px {
    let (x0, cx, cy) = (8.0 + 6.0 * f, x as f64 + 0.5, y as f64 + 0.5);
    let k = ((cx - x0).clamp(0.0, 1.0) * (x0 + 12.0 - cx).clamp(0.0, 1.0) * (cy - 10.0).clamp(0.0, 1.0) * (24.0 - cy).clamp(0.0, 1.0)) as f32;
    let (fg, bg) = (0.5 + 0.5 * blobs(cx - x0, cy, 3), 0.5 * blobs(cx, cy, 5));
    let g = bg + (fg - bg) * k;
    [g, g, g, 1.0]
}

/// The frame's number in red.
fn counter(_: usize, _: usize, f: f64) -> Px {
    [f.round() as f32 / 100.0, 0.0, 0.0, 1.0]
}

/// Black on the left half of 40, white on the right.
fn halves_map(x: usize, _: usize, _: f64) -> Px {
    if x < 20 { [0.0, 0.0, 0.0, 1.0] } else { [1.0; 4] }
}

fn clip(w: usize, h: usize, draw: fn(usize, usize, f64) -> Px) -> Clip {
    Clip { w, h, draw, other: None }
}

/// `spec` with `values` at frame `f` on `c`.
fn go(spec: &Spec, values: &[(&str, Value)], c: &Clip, f: f64) -> Buf {
    let g = instance(spec, values);
    run_when(spec, &g, c.frame(f, c.draw), Tick::from_seconds(f / FPS), &[], Some(c))
}

#[test]
fn echoes_trail_behind() {
    let c = clip(40, 8, dot);
    // frame 5: the dot at 24, before it at 20 and 16
    let out = go(&ECHO, &[("numberOfEchoes", n(2.0)), ("decay", n(0.5))], &c, 5.0);
    assert_eq!(px_at(&out, 24.5, 3.5), [1.0; 4]);
    assert!((px_at(&out, 20.5, 3.5)[3] - 0.5).abs() < 1e-5 && (px_at(&out, 16.5, 3.5)[3] - 0.25).abs() < 1e-5);
    assert_eq!(px_at(&out, 12.5, 3.5)[3], 0.0, "no more than two");
    let ahead = go(&ECHO, &[("numberOfEchoes", n(1.0)), ("echoTime", n(1.0 / FPS))], &c, 5.0);
    assert!(px_at(&ahead, 28.5, 3.5)[3] > 0.99, "from after");
    let faint = go(&ECHO, &[("numberOfEchoes", n(2.0)), ("startingIntensity", n(0.5)), ("echoOperator", Value::Choice(1))], &c, 5.0);
    assert!((px_at(&faint, 24.5, 3.5)[3] - 0.5).abs() < 1e-5, "the layer itself fainter too");
    let behind = go(&ECHO, &[("numberOfEchoes", n(1.0)), ("echoOperator", Value::Choice(4))], &c, 5.0);
    assert_eq!(px_at(&behind, 20.5, 3.5), [1.0; 4]);
    let none = go(&ECHO, &[("numberOfEchoes", n(0.0))], &c, 5.0);
    assert_eq!(none.img.px, c.frame(5.0, dot).px);
}

#[test]
fn posterized_time_holds() {
    let c = clip(4, 4, counter);
    // at 10 frames a second, frames 3 to 5 show frame 3
    for f in [3.0, 4.0, 5.0] {
        let out = go(&POSTERIZE_TIME, &[("frameRate", n(10.0))], &c, f);
        assert!((px_at(&out, 1.0, 1.0)[0] - 0.03).abs() < 1e-6, "{f}: {:?}", px_at(&out, 1.0, 1.0));
    }
    assert!((held(0.2, 10.0) - 0.2).abs() < 1e-9, "a frame on the boundary is its own");
}

#[test]
fn time_difference_lights_changes() {
    let c = clip(40, 8, dot);
    let out = go(&TIME_DIFF, &[("timeOffsetSec", n(-1.0 / FPS)), ("alphaChannel", Value::Choice(4))], &c, 5.0);
    assert!(px_at(&out, 24.5, 3.5)[0] > 0.99 && px_at(&out, 20.5, 3.5)[0] > 0.99, "where the dot is now and was");
    assert_eq!(px_at(&out, 10.5, 3.5)[0], 0.0, "still: black");
    let signed = go(&TIME_DIFF, &[("timeOffsetSec", n(-1.0 / FPS)), ("absoluteDifference", Value::Bool(false)), ("alphaChannel", Value::Choice(4))], &c, 5.0);
    assert!((px_at(&signed, 10.5, 3.5)[0] - 0.5).abs() < 1e-6 && px_at(&signed, 24.5, 3.5)[0] > 0.99 && px_at(&signed, 20.5, 3.5)[0] < 0.01);
    // against another layer: its own frames
    let other = Clip { other: Some(counter), ..clip(4, 4, counter) };
    let vs = go(&TIME_DIFF, &[("target", Value::Layer(Some(2))), ("timeOffsetSec", n(2.0 / FPS)), ("alphaChannel", Value::Choice(4))], &other, 0.0);
    assert!((px_at(&vs, 1.0, 1.0)[0] - 0.02).abs() < 1e-5, "{:?}", px_at(&vs, 1.0, 1.0));
}

#[test]
fn displaced_in_time_by_a_map() {
    let c = Clip { other: Some(halves_map), ..clip(40, 4, counter) };
    let out = go(&TIME_DISPLACEMENT, &[("timeDisplacementLayer", Value::Layer(Some(2))), ("maxDisplacementTimeSec", n(0.1)), ("timeResolutionFps", n(30.0))], &c, 10.0);
    // black: 0.1 s before (3 frames), white: 0.1 s after
    assert!((px_at(&out, 5.0, 1.0)[0] - 0.07).abs() < 1e-5 && (px_at(&out, 35.0, 1.0)[0] - 0.13).abs() < 1e-5, "{:?} {:?}", px_at(&out, 5.0, 1.0), px_at(&out, 35.0, 1.0));
    let coarse = go(&TIME_DISPLACEMENT, &[("timeDisplacementLayer", Value::Layer(Some(2))), ("maxDisplacementTimeSec", n(0.1)), ("timeResolutionFps", n(1.0))], &c, 10.0);
    assert!((px_at(&coarse, 5.0, 1.0)[0] - 0.10).abs() < 1e-5, "steps of a second: none");
}

/// How unlike two pictures are (mean absolute difference of grey) inside a margin.
fn unlike(a: &Image, b: &Image, margin: usize) -> f32 {
    let (mut s, mut k) = (0f32, 0f32);
    for y in margin..a.height - margin {
        for x in margin..a.width - margin {
            s += (a.px[y * a.width + x][0] - b.px[y * b.width + x][0]).abs();
            k += 1.0;
        }
    }
    s / k
}

#[test]
fn timewarp_builds_frames_between() {
    let c = clip(96, 64, pan);
    // at half speed, frame 3 shows source frame 1.5
    let truth = c.frame(1.5, pan);
    let whole = go(&TIMEWARP, &[("method", Value::Choice(0))], &c, 3.0);
    assert!(unlike(&whole.img, &c.frame(1.0, pan), 0) < 1e-5, "the frame before");
    let mix = go(&TIMEWARP, &[("method", Value::Choice(1))], &c, 3.0);
    let motion = go(&TIMEWARP, &[], &c, 3.0);
    let (em, ep) = (unlike(&mix.img, &truth, 8), unlike(&motion.img, &truth, 8));
    assert!(ep < em * 0.3, "following the motion is nearer the true frame: {ep} vs mixing {em}");
    // on a whole frame, that frame
    let on = go(&TIMEWARP, &[], &c, 4.0);
    assert!(unlike(&on.img, &c.frame(2.0, pan), 0) < 1e-5);
    let frame = go(&TIMEWARP, &[("adjustTimeBy", Value::Choice(1)), ("sourceFrame", n(4.0))], &c, 0.0);
    assert!(unlike(&frame.img, &c.frame(4.0, pan), 0) < 1e-5);
}

/// Pixel Motion on the small moving square: a vector every 6 pixels.
const FINE: (&str, f64) = ("vectorDetail", 60.0);

#[test]
fn timewarp_moves_a_solid_thing() {
    // half way, one whole square where it was then, the ground where it had been, no ghosts
    let c = clip(80, 34, thing);
    let truth = c.frame(1.5, thing);
    let out = go(&TIMEWARP, &[(FINE.0, n(FINE.1))], &c, 3.0);
    let mix = go(&TIMEWARP, &[("method", Value::Choice(1))], &c, 3.0);
    let (ep, em) = (unlike(&out.img, &truth, 4), unlike(&mix.img, &truth, 4));
    assert!(ep < em * 0.4, "{ep} vs mixing {em}");
    // source frame 1.5: the square from 17 to 29; before, from 14 to 26, after, from 20 to 32
    for x in [15.5, 22.5, 30.5] {
        let (got, want) = (px_at(&out, x, 17.5)[0], px_at(&buf(truth.clone()), x, 17.5)[0]);
        assert!((got - want).abs() < 0.08, "at {x}: {got} for {want}");
    }
}

#[test]
fn timewarp_blurs_and_mattes() {
    let c = clip(80, 34, thing);
    let sharp = go(&TIMEWARP, &[("speed", n(100.0)), (FINE.0, n(FINE.1))], &c, 2.0);
    let blurred = go(&TIMEWARP, &[("speed", n(100.0)), (FINE.0, n(FINE.1)), ("enableMotionBlur", Value::Bool(true))], &c, 2.0);
    // frame 2: the square from 20 to 32; blurred over half a frame, it reaches on toward 35
    let ground = c.frame(0.0, thing);
    let (s, bl, g) = (px_at(&sharp, 33.5, 17.5)[0], px_at(&blurred, 33.5, 17.5)[0], px_at(&buf(ground), 33.5, 17.5)[0]);
    assert!((s - g).abs() < 0.02 && bl - g > 0.08, "{s} {bl} on {g}");
    let matted = Clip { other: Some(halves_map), ..clip(40, 34, thing) };
    let shown = go(&TIMEWARP, &[("matteLayer", Value::Layer(Some(2))), ("matteChannel", Value::Choice(2)), ("show", Value::Choice(3)), ("method", Value::Choice(0))], &matted, 3.0);
    assert!(px_at(&shown, 5.0, 5.0)[0] < 0.01 && px_at(&shown, 35.0, 5.0)[0] > 0.99, "the matte");
    let front = go(&TIMEWARP, &[("matteLayer", Value::Layer(Some(2))), ("matteChannel", Value::Choice(2)), ("show", Value::Choice(1))], &matted, 3.0);
    assert_eq!(px_at(&front, 5.0, 25.0)[3], 0.0, "nothing outside the matte");
    assert!(px_at(&front, 35.0, 25.0)[3] > 0.99);
}

#[test]
fn motion_blurs() {
    let c = clip(40, 8, dot);
    let forced = go(&FORCE_BLUR, &[("motionBlurLevels", n(4.0)), ("shutterAngle", n(360.0))], &c, 5.0);
    // across a whole frame in 4 pictures: the dot from 24, 25, 26 and 27
    assert!((px_at(&forced, 26.5, 3.5)[3] - 0.5).abs() < 1e-5 && (px_at(&forced, 24.5, 3.5)[3] - 0.25).abs() < 1e-5, "{:?}", px_at(&forced, 26.5, 3.5));
    assert_eq!(px_at(&forced, 29.5, 3.5)[3], 0.0);
    let wide = go(&WIDE_TIME, &[("forwardSteps", n(1.0)), ("backwardSteps", n(1.0))], &c, 5.0);
    for x in [20.5, 24.5, 28.5] {
        assert!((px_at(&wide, x, 3.5)[3] - 1.0 / 3.0).abs() < 1e-5, "{x}");
    }
    let s = clip(80, 34, thing);
    let pixel = go(&PIXEL_BLUR, &[(FINE.0, n(FINE.1)), ("shutterControl", Value::Choice(1)), ("shutterAngle", n(360.0))], &s, 2.0);
    let still = buf(s.frame(2.0, thing));
    // the square (20 to 32) smeared half a frame each way: a third of it a pixel past each edge
    for x in [19.5, 32.5] {
        let (got, was) = (px_at(&pixel, x, 17.5)[0], px_at(&still, x, 17.5)[0]);
        assert!(got - was > 0.06, "at {x}: {got} from {was}");
    }
    assert!((px_at(&pixel, 60.0, 17.5)[0] - px_at(&still, 60.0, 17.5)[0]).abs() < 0.01, "the still ground kept");
}

#[test]
fn without_a_renderer_nothing_changes() {
    let img = crate::testing::halves(10, 10);
    for spec in ALL {
        let g = instance(&spec, &[]);
        let out = run_when(&spec, &g, img.clone(), Tick::from_seconds(0.5), &[], None);
        assert_eq!(out.img.px, img.px, "{}", spec.def.id);
    }
}

#[test]
fn every_one_is_registered() {
    for spec in ALL {
        assert!(crate::find(spec.def.id).is_some_and(|s| s.category == "time"), "{}", spec.def.id);
    }
}


/// Cells of flat colour with dark lines, 56 across, moving 7.3 a frame over clouds.
fn cells(x: usize, y: usize, f: f64) -> Px {
    let mut acc = [0f32; 4];
    for j in 0..4 {
        for i in 0..4 {
            let p = cell_at(x as f64 + (i as f64 + 0.5) / 4.0, y as f64 + (j as f64 + 0.5) / 4.0, f);
            (0..4).for_each(|k| acc[k] += p[k] / 16.0);
        }
    }
    acc
}

fn cell_at(cx: f64, cy: f64, f: f64) -> Px {
    let (x0, y0) = (30.0 + 7.3 * f - 28.0, 86.0 - 28.0);
    let g = 0.15 + 0.3 * (0.5 + 0.5 * noise::fractal(cx / 30.0, cy / 30.0, 0.0, 4, 5.0, 0.5, None)) as f32;
    let (u, v) = (cx - x0, cy - y0);
    if !(0.0..56.0).contains(&u) || !(0.0..56.0).contains(&v) {
        return [g, g, g, 1.0];
    }
    let line = u % 14.0 < 2.0 || v % 14.0 < 2.0 || u > 54.0 || v > 54.0;
    if line {
        return [0.06, 0.06, 0.06, 1.0];
    }
    let (i, j) = ((u / 14.0) as usize, (v / 14.0) as usize);
    [1.0 - i as f32 / 3.0 * 0.8, 1.0 - j as f32 / 3.0, 0.2 + 0.25 * (i + j) as f32 / 3.0, 1.0]
}

#[test]
fn timewarp_moves_flat_colours() {
    // cells of flat colour say little of how they move but at their edges: still no ghosts
    let c = clip(192, 192, cells);
    let truth = c.frame(7.5, cells);
    let out = go(&TIMEWARP, &[("vectorDetail", n(50.0)), ("speed", n(25.0))], &c, 30.0);
    let (mut bad, mut sum, mut count) = (0, 0f32, 0);
    for y in 40..130 {
        for x in 10..130 {
            let (p, q) = (out.img.px[y * 192 + x], truth.px[y * 192 + x]);
            let e = (0..3).map(|k| (p[k] - q[k]).abs()).fold(0f32, f32::max);
            bad += (e > 0.3) as usize;
            sum += e;
            count += 1;
        }
    }
    assert!(bad < 10 && sum / (count as f32) < 0.01, "{bad} pixels far off, {} on average", sum / count as f32);
}
