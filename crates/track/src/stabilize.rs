//! What a warp stabilizer works out: how the camera moved from each frame to the next (once, by
//! following many features between them and fitting how most of them moved), and from that how
//! to move each frame so the camera seems to glide — each frame brought to the mean of its
//! neighbours' views, weighted by a Gaussian, so slow moves stay and shakes go (Matsushita et
//! al.'s motion smoothing) — or to stand still (every frame brought onto the first). Then how
//! much to crop and scale so the moved frames' edges do not show.
//!
//! Points are the layer's pixels; a frame's map takes its pixels to where they are shown.

use veymelo_raster::Image;

use crate::fit::{Model, robust, times};
use crate::klt::{Gray, corners, follow};
use crate::solve::Warp;

/// How the camera moved from one frame to the next: as a move, as a move turned and scaled,
/// and in perspective; and how many of the features it followed agreed.
#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    pub shift: [f64; 2],
    /// (a, b, tx, ty): x' = a x − b y + tx, y' = b x + a y + ty.
    pub similar: [f64; 4],
    /// The perspective map's first eight numbers (the ninth is 1).
    pub perspective: [f64; 8],
    pub agreed: u32,
    pub followed: u32,
}

impl Motion {
    pub const STILL: Motion = Motion { shift: [0.0; 2], similar: [1.0, 0.0, 0.0, 0.0], perspective: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0], agreed: 0, followed: 0 };

    /// The map of `method`.
    pub fn warp(&self, method: Method) -> Warp {
        match method {
            Method::Position => Warp([[1.0, 0.0, self.shift[0]], [0.0, 1.0, self.shift[1]], [0.0, 0.0, 1.0]]),
            Method::Similar => {
                let s = self.similar;
                Warp([[s[0], -s[1], s[2]], [s[1], s[0], s[3]], [0.0, 0.0, 1.0]])
            }
            Method::Perspective => {
                let h = self.perspective;
                Warp([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]])
            }
        }
    }
}

/// A clip's camera motion: its first frame's time (the layer's seconds), the time between
/// frames, the frames' rectangle (the layer's pixels: x0, y0, x1, y1), and each frame's motion
/// from the one before (the first's none).
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub start: f64,
    pub step: f64,
    pub rect: [f64; 4],
    pub frames: Vec<Motion>,
}

/// Short text of a number (enough to keep a map to a thousandth of a pixel).
fn num(v: f64) -> String {
    let s = format!("{v:.7}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

impl Analysis {
    /// As text to keep: a first line of what it covers, then a line a frame.
    pub fn to_text(&self) -> String {
        let mut out = format!("motion 1 {} {} {}\n", num(self.start), num(self.step), self.rect.map(num).join(" "));
        for m in &self.frames {
            let all: Vec<String> = m.shift.iter().chain(&m.similar).chain(&m.perspective).map(|v| num(*v)).collect();
            out.push_str(&format!("{} {} {}\n", all.join(" "), m.agreed, m.followed));
        }
        out
    }

    pub fn from_text(text: &str) -> Option<Analysis> {
        let mut lines = text.lines();
        let head: Vec<f64> = lines.next()?.strip_prefix("motion 1 ")?.split_whitespace().map(|v| v.parse().ok()).collect::<Option<_>>()?;
        let [start, step, x0, y0, x1, y1] = head[..] else { return None };
        let mut frames = Vec::new();
        for line in lines.filter(|l| !l.trim().is_empty()) {
            let v: Vec<f64> = line.split_whitespace().map(|v| v.parse().ok()).collect::<Option<_>>()?;
            if v.len() != 16 {
                return None;
            }
            let mut perspective = [0.0; 8];
            perspective.copy_from_slice(&v[6..14]);
            frames.push(Motion { shift: [v[0], v[1]], similar: [v[2], v[3], v[4], v[5]], perspective, agreed: v[14] as u32, followed: v[15] as u32 });
        }
        Some(Analysis { start, step, rect: [x0, y0, x1, y1], frames })
    }

    /// The frame shown at layer time `t` (seconds).
    pub fn frame_at(&self, t: f64) -> usize {
        let k = ((t - self.start) / self.step.max(1e-9)).round();
        (k.max(0.0) as usize).min(self.frames.len().saturating_sub(1))
    }
}

/// Works out a clip's motion as its frames are given one after another.
pub struct Analyzer {
    last: Option<Gray>,
    analysis: Analysis,
}

impl Analyzer {
    /// For frames whose first is at layer time `start`, `step` seconds apart, covering `rect`.
    pub fn new(start: f64, step: f64, rect: [f64; 4]) -> Analyzer {
        Analyzer { last: None, analysis: Analysis { start, step, rect, frames: Vec::new() } }
    }

    /// The next frame (whose layer (0, 0) is at `offset` in it).
    pub fn add(&mut self, img: &Image, offset: [f64; 2]) {
        let now = Gray::new(img, offset, 3);
        let motion = match &self.last {
            None => Motion::STILL,
            Some(was) => between(was, &now),
        };
        self.analysis.frames.push(motion);
        self.last = Some(now);
    }

    pub fn finish(self) -> Analysis {
        self.analysis
    }
}

/// How the picture moved from `was` to `now`.
fn between(was: &Gray, now: &Gray) -> Motion {
    let apart = (was.width().max(was.height()) as f64 / 40.0).max(6.0);
    let points = corners(was, 500, apart, 0.01);
    let pairs: Vec<([f64; 2], [f64; 2])> = points.iter().zip(follow(was, now, &points)).filter_map(|(p, q)| Some((was.to_layer(*p), now.to_layer(q?)))).collect();
    let tolerance = 1.5 * was.step;
    let mut m = Motion { followed: pairs.len() as u32, ..Motion::STILL };
    if let Some((w, _)) = robust(Model::Move, &pairs, tolerance) {
        m.shift = [w.0[0][2], w.0[1][2]];
    }
    if let Some((w, _)) = robust(Model::Similar, &pairs, tolerance) {
        m.similar = [w.0[0][0], w.0[1][0], w.0[0][2], w.0[1][2]];
    }
    if let Some((w, n)) = robust(Model::Perspective, &pairs, tolerance) {
        m.perspective = [w.0[0][0], w.0[0][1], w.0[0][2], w.0[1][0], w.0[1][1], w.0[1][2], w.0[2][0], w.0[2][1]];
        m.agreed = n as u32;
    }
    m
}

/// What the stabilized clip is to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aim {
    /// The camera's moves kept, smoothed.
    Smooth,
    /// The camera held still.
    Still,
}

/// How a frame may be moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Position,
    /// Moved, turned and scaled.
    Similar,
    Perspective,
}

/// What is done about the moved frames' edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// They show.
    Stabilize,
    /// Cut to what every frame has.
    Crop,
    /// Cut so, and scaled up to fill the frame.
    CropScale,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub aim: Aim,
    /// How smooth, in %: 100 smooths over about a second either way.
    pub smoothness: f64,
    pub method: Method,
    pub framing: Framing,
    /// The most it scales up, in %.
    pub most_scale: f64,
    /// A scale on top, in %.
    pub extra_scale: f64,
}

/// Each frame's map from where it is to where it is shown (before framing): `rate` frames a
/// second.
pub fn corrections(a: &Analysis, s: &Settings) -> Vec<Warp> {
    // where each frame's camera is: frame 0's view taken to frame k's
    let mut path: Vec<[[f64; 3]; 3]> = Vec::with_capacity(a.frames.len());
    let mut c = Warp::NONE.0;
    for (k, m) in a.frames.iter().enumerate() {
        if k > 0 {
            c = times(&m.warp(s.method).0, &c);
            c = c.map(|r| r.map(|v| v / c[2][2]));
        }
        path.push(c);
    }
    let rate = 1.0 / a.step.max(1e-9);
    let sigma = (s.smoothness / 100.0 * rate).max(1e-3);
    let reach = (3.0 * sigma).ceil() as isize;
    let last = path.len() as isize - 1;
    // the path past either end mirrored about the end (C_e C_(e∓j)⁻¹ C_e), so a steady move goes
    // on steadily and the ends do not drift
    let mirrored = |e: usize, j: usize| -> [[f64; 3]; 3] {
        let m = Warp(path[j]).inverse().map_or(path[e], |back| times(&path[e], &times(&back.0, &path[e])));
        m.map(|r| r.map(|v| v / m[2][2]))
    };
    let at = |j: isize| -> [[f64; 3]; 3] {
        if j < 0 {
            mirrored(0, ((-j) as usize).min(last as usize))
        } else if j > last {
            mirrored(last as usize, (2 * last - j).max(0) as usize)
        } else {
            path[j as usize]
        }
    };
    (0..path.len())
        .map(|k| {
            let target = match s.aim {
                Aim::Still => path[0],
                Aim::Smooth => {
                    let mut sum = [[0.0; 3]; 3];
                    let mut total = 0.0;
                    for j in k as isize - reach..=k as isize + reach {
                        let w = (-((j - k as isize) as f64).powi(2) / (2.0 * sigma * sigma)).exp();
                        total += w;
                        let c = at(j);
                        for (r, row) in sum.iter_mut().enumerate() {
                            for (col, v) in row.iter_mut().enumerate() {
                                *v += w * c[r][col];
                            }
                        }
                    }
                    sum.map(|r| r.map(|v| v / total))
                }
            };
            Warp(path[k]).inverse().map_or(Warp::NONE, |back| {
                let m = times(&target, &back.0);
                Warp(m.map(|r| r.map(|v| v / m[2][2])))
            })
        })
        .collect()
}

/// Whether `p` is inside the four-sided outline `q` (in order round it).
fn inside(q: &[[f64; 2]; 4], p: [f64; 2]) -> bool {
    let mut sign = 0.0;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if cross.abs() < 1e-12 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// How much of the frame (a fraction of its width and height, about its middle) every moved
/// frame covers.
pub fn covered(a: &Analysis, moved: &[Warp]) -> f64 {
    let [x0, y0, x1, y1] = a.rect;
    let (mid, half) = ([(x0 + x1) / 2.0, (y0 + y1) / 2.0], [(x1 - x0) / 2.0, (y1 - y0) / 2.0]);
    let outlines: Vec<[[f64; 2]; 4]> = moved.iter().map(|w| [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|p| w.apply(p))).collect();
    let fits = |f: f64| {
        let r = [[mid[0] - f * half[0], mid[1] - f * half[1]], [mid[0] + f * half[0], mid[1] - f * half[1]], [mid[0] + f * half[0], mid[1] + f * half[1]], [mid[0] - f * half[0], mid[1] + f * half[1]]];
        outlines.iter().all(|q| r.iter().all(|p| inside(q, *p)))
    };
    let (mut lo, mut hi) = (0.0, 1.0);
    if fits(1.0) {
        return 1.0;
    }
    for _ in 0..30 {
        let m = (lo + hi) / 2.0;
        if fits(m) { lo = m } else { hi = m }
    }
    lo
}

/// What a stabilized clip is: each frame's map (framing included), and the part of the frame
/// kept (a fraction of it about its middle; 1 keeps it all).
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub maps: Vec<Warp>,
    pub keep: f64,
    pub scale: f64,
}

pub fn plan(a: &Analysis, s: &Settings) -> Plan {
    let moved = corrections(a, s);
    let cover = covered(a, &moved);
    let extra = s.extra_scale / 100.0;
    let (scale, keep) = match s.framing {
        Framing::Stabilize => (extra, 1.0),
        Framing::Crop => (extra, cover),
        Framing::CropScale => {
            let up = (1.0 / cover.max(1e-6)).min(s.most_scale.max(100.0) / 100.0);
            (up * extra, (cover * up).min(1.0))
        }
    };
    let [x0, y0, x1, y1] = a.rect;
    let mid = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
    let zoom = [[scale, 0.0, mid[0] * (1.0 - scale)], [0.0, scale, mid[1] * (1.0 - scale)], [0.0, 0.0, 1.0]];
    Plan { maps: moved.iter().map(|w| Warp(times(&zoom, &w.0))).collect(), keep, scale }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shaky(n: usize) -> Analysis {
        // a camera gliding right three pixels a frame, shaking up and down
        let shake = |k: usize| if k == 0 { 0.0 } else { 4.0 * ((k as f64) * 2.1).sin() - 4.0 * (((k - 1) as f64) * 2.1).sin() };
        let frames = (0..n).map(|k| if k == 0 { Motion::STILL } else { Motion { shift: [3.0, shake(k)], similar: [1.0, 0.0, 3.0, shake(k)], perspective: [1.0, 0.0, 3.0, 0.0, 1.0, shake(k), 0.0, 0.0], agreed: 100, followed: 100 } }).collect();
        Analysis { start: 0.0, step: 1.0 / 25.0, rect: [0.0, 0.0, 400.0, 300.0], frames }
    }

    /// A scene of waves, seen through a camera at `(x, y)` turned `turn` degrees about the
    /// middle of a `w` × `h` frame.
    fn view(w: usize, h: usize, x: f64, y: f64, turn: f64) -> Image {
        let (s, c) = turn.to_radians().sin_cos();
        let mut img = Image::new(w, h);
        for j in 0..h {
            for i in 0..w {
                let (u, v) = (i as f64 + 0.5 - w as f64 / 2.0, j as f64 + 0.5 - h as f64 / 2.0);
                let (px, py) = (c * u - s * v + w as f64 / 2.0 + x, s * u + c * v + h as f64 / 2.0 + y);
                let t = 0.5 + 0.12 * (px * 0.21).sin() * (py * 0.17).cos() + 0.1 * ((px + py) * 0.093).sin() + 0.08 * (px * 0.051 - py * 0.13).cos() + 0.06 * ((px * 0.37).sin() * (py * 0.29).sin());
                img.px[j * w + i] = [t as f32, t as f32, t as f32, 1.0];
            }
        }
        img
    }

    #[test]
    fn a_shaking_camera_worked_out_from_its_frames() {
        let cam = |k: usize| (2.5 * k as f64 + 3.0 * (k as f64 * 1.7).sin(), 2.0 * (k as f64 * 2.3).cos(), 0.4 * (k as f64 * 0.9).sin());
        let mut an = Analyzer::new(0.0, 0.04, [0.0, 0.0, 320.0, 240.0]);
        for k in 0..6 {
            let (x, y, t) = cam(k);
            an.add(&view(320, 240, x, y, t), [0.0; 2]);
        }
        let a = an.finish();
        assert_eq!(a.frames.len(), 6);
        for k in 1..6 {
            // the scene moves the other way to the camera: a scene point's place in frame k
            let at = |k: usize, p: [f64; 2]| {
                let (x, y, t) = cam(k);
                let (s, c) = t.to_radians().sin_cos();
                let (dx, dy) = (p[0] - 160.0 - x, p[1] - 120.0 - y);
                [c * dx + s * dy + 160.0, -s * dx + c * dy + 120.0]
            };
            let m = &a.frames[k];
            assert!(m.agreed as f64 > 0.8 * m.followed as f64 && m.followed > 100, "frame {k}: {m:?}");
            for p in [[100.0, 80.0], [250.0, 200.0]] {
                let (was, now) = (at(k - 1, p), at(k, p));
                let got = m.warp(Method::Perspective).apply(was);
                assert!((got[0] - now[0]).hypot(got[1] - now[1]) < 0.15, "frame {k}: {got:?} {now:?}");
            }
        }
    }

    #[test]
    fn kept_as_text() {
        let a = shaky(5);
        let back = Analysis::from_text(&a.to_text()).unwrap();
        assert_eq!(back.to_text(), a.to_text());
        assert!(back.frames.iter().zip(&a.frames).all(|(p, q)| p.perspective.iter().zip(&q.perspective).all(|(x, y)| (x - y).abs() < 1e-6)));
        assert_eq!(a.frame_at(0.079), 2);
        assert_eq!(a.frame_at(9.0), 4);
    }

    #[test]
    fn smoothed_or_held_still() {
        let a = shaky(60);
        let s = Settings { aim: Aim::Smooth, smoothness: 50.0, method: Method::Perspective, framing: Framing::Stabilize, most_scale: 150.0, extra_scale: 100.0 };
        let moved = corrections(&a, &s);
        // in the middle of the clip the shake is gone and the glide stays: a point of the scene
        // moves three pixels a frame across and nothing up or down once stabilized
        let scene = |k: usize| [100.0 + 3.0 * k as f64, 150.0 + 4.0 * ((k as f64) * 2.1).sin()];
        for k in 0..59 {
            let (p, q) = (moved[k].apply(scene(k)), moved[k + 1].apply(scene(k + 1)));
            assert!((q[0] - p[0] - 3.0).abs() < 0.05 && (q[1] - p[1]).abs() < 0.6, "frame {k}: {p:?} {q:?}");
        }
        // held still: every frame as the first
        let still = corrections(&a, &Settings { aim: Aim::Still, ..s });
        for k in [10, 33, 59] {
            let p = still[k].apply(scene(k));
            assert!((p[0] - 100.0).abs() < 1e-6 && (p[1] - 150.0).abs() < 1e-6, "{p:?}");
        }
        // cropped to what every frame covers, scaled back up to fill (no further than allowed)
        let p = plan(&a, &Settings { aim: Aim::Still, framing: Framing::CropScale, ..s });
        assert!(p.scale > 1.0 && p.scale <= 1.5, "{}", p.scale);
        let cropped = plan(&a, &Settings { aim: Aim::Still, framing: Framing::Crop, ..s });
        assert!(cropped.keep < 1.0 && cropped.scale == 1.0);
    }
}
