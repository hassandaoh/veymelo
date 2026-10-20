//! Footage made up for the tests: a textured square moving along a known path (turning and
//! growing too) over a textured ground, with a little noise; where it truly is, is known.

use super::*;

/// A smooth texture: a few waves of different ways and widths added up.
fn texture(x: f64, y: f64, seed: u64) -> [f32; 3] {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s % 10_000) as f64 / 10_000.0
    };
    let mut v = [0.5f64; 3];
    for _ in 0..8 {
        let (way, width, phase) = (next() * std::f64::consts::TAU, 8.0 + next() * 30.0, next() * std::f64::consts::TAU);
        let w = ((x * way.cos() + y * way.sin()) * std::f64::consts::TAU / width + phase).sin();
        for c in &mut v {
            *c += w * (0.04 + 0.08 * next());
        }
    }
    v.map(|c| c.clamp(0.0, 1.0) as f32)
}

/// Noise from −0.5 to 0.5, the same for the same pixel and frame.
fn noise(x: u64, y: u64, f: u64) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B9).wrapping_add(y.wrapping_mul(0x85EB_CA6B)).wrapping_add(f.wrapping_mul(0xC2B2_AE35));
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h % 1000) as f32 / 1000.0 - 0.5
}

/// Where the square is on a frame: its middle, turn (degrees) and size (1: as it started).
#[derive(Clone, Copy)]
struct Pose {
    c: [f64; 2],
    turn: f64,
    size: f64,
}

impl Pose {
    fn map(&self, u: [f64; 2]) -> [f64; 2] {
        let (s, c) = self.turn.to_radians().sin_cos();
        [self.c[0] + self.size * (c * u[0] - s * u[1]), self.c[1] + self.size * (s * u[0] + c * u[1])]
    }

    fn local(&self, p: [f64; 2]) -> [f64; 2] {
        let (s, c) = self.turn.to_radians().sin_cos();
        let d = [(p[0] - self.c[0]) / self.size, (p[1] - self.c[1]) / self.size];
        [c * d[0] + s * d[1], -s * d[0] + c * d[1]]
    }
}

/// A `w` × `h` frame `f`: the ground, the square (`half` from its middle to its sides) at
/// `pose`, noise of `amount`, and a grey block over `cover` if given.
fn frame(w: usize, h: usize, pose: Pose, half: f64, amount: f32, f: u64, cover: Option<[f64; 4]>) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f32; 3];
            for (sx, sy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let p = [x as f64 + sx, y as f64 + sy];
                let l = pose.local(p);
                let t = if l[0].abs() <= half && l[1].abs() <= half { texture(l[0], l[1], 11) } else { texture(p[0] * 0.8, p[1] * 0.8, 5).map(|v| v * 0.5) };
                for i in 0..3 {
                    sum[i] += t[i] / 4.0;
                }
            }
            let n = noise(x as u64, y as u64, f) * amount;
            let mut px = [sum[0] + n, sum[1] + n, sum[2] + n, 1.0];
            if let Some([cx, cy, cw, ch]) = cover {
                let (mx, my) = (x as f64 + 0.5, y as f64 + 0.5);
                if mx >= cx && mx < cx + cw && my >= cy && my < cy + ch {
                    px = [0.4, 0.4, 0.4, 1.0];
                }
            }
            img.px[y * w + x] = px;
        }
    }
    img
}

fn point(c: [f64; 2], feature: f64, search: f64) -> Point {
    Point { center: c, feature: [feature; 2], search: [search; 2], search_offset: [0.0; 2] }
}

fn apart(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

#[test]
fn a_moving_patch_followed_to_a_fraction_of_a_pixel() {
    let path = |f: usize| {
        let t = f as f64;
        Pose { c: [90.0 + 4.3 * t + 5.0 * (t * 0.35).sin(), 80.0 + 2.6 * t - 3.0 * (t * 0.2).cos()], turn: 0.0, size: 1.0 }
    };
    let first = frame(320, 240, path(0), 40.0, 0.02, 0, None);
    let mut tracker = Tracker::new(Options::default(), &[point(path(0).c, 31.0, 71.0)], &Frame { img: &first, offset: [0.0; 2] });
    let mut worst = 0.0f64;
    for f in 1..24 {
        let img = frame(320, 240, path(f), 40.0, 0.02, f as u64, None);
        let r = tracker.step(&Frame { img: &img, offset: [0.0; 2] })[0];
        worst = worst.max(apart(r.center, path(f).c));
        assert!(r.confidence > 90.0 && r.how == How::Tracked, "frame {f}: {r:?}");
    }
    assert!(worst < 0.25, "the furthest off: {worst} pixels");
}

#[test]
fn offsets_channels_and_softening() {
    let pose = |f: usize| Pose { c: [120.0 + 3.1 * f as f64, 100.0 - 1.9 * f as f64], turn: 0.0, size: 1.0 };
    for channel in [Channel::Rgb, Channel::Luminance, Channel::Saturation] {
        for (blur, enhance) in [(0.0, false), (3.0, false), (0.0, true)] {
            let options = Options { channel, blur, enhance, ..Default::default() };
            // the layer's (0, 0) at image pixel (10, −5)
            let offset = [10.0, -5.0];
            let first = frame(240, 200, pose(0), 36.0, 0.01, 0, None);
            let start = [pose(0).c[0] - offset[0], pose(0).c[1] - offset[1]];
            let mut tracker = Tracker::new(options, &[point(start, 29.0, 61.0)], &Frame { img: &first, offset });
            for f in 1..6 {
                let img = frame(240, 200, pose(f), 36.0, 0.01, f as u64, None);
                let r = tracker.step(&Frame { img: &img, offset })[0];
                let want = [pose(f).c[0] - offset[0], pose(f).c[1] - offset[1]];
                assert!(apart(r.center, want) < 0.3, "{channel:?} blur {blur} enhance {enhance} frame {f}: {:?} {want:?}", r.center);
            }
        }
    }
}

#[test]
fn turning_and_growing_followed() {
    let pose = |f: usize| Pose { c: [160.0 + 1.5 * f as f64, 120.0 + 0.8 * f as f64], turn: 2.0 * f as f64, size: 1.0 + 0.012 * f as f64 };
    let first = frame(320, 240, pose(0), 60.0, 0.01, 0, None);
    // two points on the square, as a tracker following turning and scale has
    let (a, b) = ([-30.0, 0.0], [30.0, 0.0]);
    let options = Options { unsure: Unsure::Continue, ..Default::default() };
    let mut tracker = Tracker::new(options, &[point(pose(0).map(a), 25.0, 61.0), point(pose(0).map(b), 25.0, 61.0)], &Frame { img: &first, offset: [0.0; 2] });
    for f in 1..16 {
        let img = frame(320, 240, pose(f), 60.0, 0.01, f as u64, None);
        let r = tracker.step(&Frame { img: &img, offset: [0.0; 2] });
        for (got, u) in r.iter().zip([a, b]) {
            assert!(apart(got.center, pose(f).map(u)) < 0.5, "frame {f}: {got:?} {:?}", pose(f).map(u));
            assert!(got.confidence > 85.0, "frame {f}: its shape followed, it stays sure: {got:?}");
        }
        // the patch has turned and grown as the square has
        assert!((r[0].rotation - pose(f).turn).abs() < 1.5 && (r[0].scale - pose(f).size).abs() < 0.03, "frame {f}: {:?}", r[0]);
        let (angle, length) = angle_and_length(r[0].center, r[1].center);
        assert!((angle - pose(f).turn).abs() < 1.0 && (length / 60.0 - pose(f).size).abs() < 0.02);
    }
}

#[test]
fn what_it_does_when_unsure() {
    let pose = |f: usize| Pose { c: [100.0 + 5.0 * f as f64, 100.0], turn: 0.0, size: 1.0 };
    // frame 4 has the square hidden
    let cover = |f: usize| (f == 4).then_some([60.0, 60.0, 160.0, 80.0]);
    let run = |unsure: Unsure| -> Vec<Found> {
        let first = frame(260, 200, pose(0), 36.0, 0.01, 0, None);
        let mut tracker = Tracker::new(Options { unsure, ..Default::default() }, &[point(pose(0).c, 29.0, 71.0)], &Frame { img: &first, offset: [0.0; 2] });
        (1..7).map(|f| tracker.step(&Frame { img: &frame(260, 200, pose(f), 36.0, 0.01, f as u64, cover(f)), offset: [0.0; 2] })[0]).collect()
    };
    let stopped = run(Unsure::Stop);
    assert_eq!(stopped[3].how, How::Stopped);
    assert!(stopped[3].confidence < 80.0);
    // carried on at five pixels a frame, then found again
    let carried = run(Unsure::Extrapolate);
    assert_eq!(carried[3].how, How::Extrapolated);
    assert!(apart(carried[3].center, pose(4).c) < 0.5, "{:?}", carried[3]);
    assert!(carried[4].how == How::Tracked && apart(carried[4].center, pose(5).c) < 0.3);
    // backwards works as forwards does
    let mut tracker = Tracker::new(Options::default(), &[point(pose(6).c, 29.0, 71.0)], &Frame { img: &frame(260, 200, pose(6), 36.0, 0.01, 6, None), offset: [0.0; 2] });
    for f in (0..6).rev() {
        let r = tracker.step(&Frame { img: &frame(260, 200, pose(f), 36.0, 0.01, f as u64, None), offset: [0.0; 2] })[0];
        assert!(apart(r.center, pose(f).c) < 0.3, "frame {f}: {r:?}");
    }
}
