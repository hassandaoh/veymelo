//! What a warp stabilizer works out: how the camera moved from each frame to the next (once, by
//! following many features between them and fitting how most of them moved), and from that how
//! to move each frame so the camera seems to glide — each frame brought to the mean of its
//! neighbours' views, weighted by a Gaussian, so slow moves stay and shakes go (Matsushita et
//! al.'s motion smoothing) — or to stand still (every frame brought onto the first). Then how
//! much to crop and scale so the moved frames' edges do not show.
//!
//! For the Subspace Warp each frame's motion also keeps what is left of the features' moves
//! once the whole frame's is taken out, at the points of a small grid over the frame (the median
//! of the features round each): where parts of the picture move otherwise (nearer things moving
//! further than farther ones, a rolling shutter's wobble). Each grid point's path is smoothed as
//! the camera's is, and the frame is bent on the grid by what that changes, so each part is
//! steadied on its own.
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
    /// How the parts moved besides that, at each point of the grid over the frame (row by row,
    /// `LOCAL` points across and down; empty: not worked out).
    pub local: Vec<[f64; 2]>,
}

/// How many points across and down the grid of parts' own moves has.
pub const LOCAL: (usize, usize) = (8, 6);

impl Motion {
    pub const STILL: Motion = Motion { shift: [0.0; 2], similar: [1.0, 0.0, 0.0, 0.0], perspective: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0], agreed: 0, followed: 0, local: Vec::new() };

    /// The map of `method`.
    pub fn warp(&self, method: Method) -> Warp {
        match method {
            Method::Position => Warp([[1.0, 0.0, self.shift[0]], [0.0, 1.0, self.shift[1]], [0.0, 0.0, 1.0]]),
            Method::Similar => {
                let s = self.similar;
                Warp([[s[0], -s[1], s[2]], [s[1], s[0], s[3]], [0.0, 0.0, 1.0]])
            }
            Method::Perspective | Method::Subspace => {
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
    /// As text to keep: a first line of what it covers, then a line a frame (its parts' own
    /// moves after a `|`).
    pub fn to_text(&self) -> String {
        let mut out = format!("motion 1 {} {} {}\n", num(self.start), num(self.step), self.rect.map(num).join(" "));
        for m in &self.frames {
            let all: Vec<String> = m.shift.iter().chain(&m.similar).chain(&m.perspective).map(|v| num(*v)).collect();
            out.push_str(&format!("{} {} {}", all.join(" "), m.agreed, m.followed));
            if !m.local.is_empty() {
                out.push_str(" | ");
                out.push_str(&m.local.iter().flat_map(|p| p.iter().map(|v| num(*v))).collect::<Vec<_>>().join(" "));
            }
            out.push('\n');
        }
        out
    }

    pub fn from_text(text: &str) -> Option<Analysis> {
        let mut lines = text.lines();
        let head: Vec<f64> = lines.next()?.strip_prefix("motion 1 ")?.split_whitespace().map(|v| v.parse().ok()).collect::<Option<_>>()?;
        let [start, step, x0, y0, x1, y1] = head[..] else { return None };
        let mut frames = Vec::new();
        for line in lines.filter(|l| !l.trim().is_empty()) {
            let (whole, parts) = line.split_once('|').unwrap_or((line, ""));
            let v: Vec<f64> = whole.split_whitespace().map(|v| v.parse().ok()).collect::<Option<_>>()?;
            if v.len() != 16 {
                return None;
            }
            let l: Vec<f64> = parts.split_whitespace().map(|v| v.parse().ok()).collect::<Option<_>>()?;
            if !l.is_empty() && l.len() != 2 * LOCAL.0 * LOCAL.1 {
                return None;
            }
            let mut perspective = [0.0; 8];
            perspective.copy_from_slice(&v[6..14]);
            frames.push(Motion { shift: [v[0], v[1]], similar: [v[2], v[3], v[4], v[5]], perspective, agreed: v[14] as u32, followed: v[15] as u32, local: l.as_chunks::<2>().0.to_vec() });
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
            Some(was) => between(was, &now, self.analysis.rect),
        };
        self.analysis.frames.push(motion);
        self.last = Some(now);
    }

    pub fn finish(self) -> Analysis {
        self.analysis
    }
}

/// The points of the grid of parts' own moves over a frame covering `rect`, row by row.
pub fn grid_points(rect: [f64; 4]) -> Vec<[f64; 2]> {
    let [x0, y0, x1, y1] = rect;
    (0..LOCAL.1).flat_map(|j| (0..LOCAL.0).map(move |i| [x0 + (x1 - x0) * i as f64 / (LOCAL.0 - 1) as f64, y0 + (y1 - y0) * j as f64 / (LOCAL.1 - 1) as f64])).collect()
}

/// How the picture moved from `was` to `now` (frames covering `rect`).
fn between(was: &Gray, now: &Gray, rect: [f64; 4]) -> Motion {
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
        // what is left of each feature's move once the frame's is taken out (not far: a bad
        // match), the median of those round each grid point
        let [x0, y0, x1, y1] = rect;
        let cell = ((x1 - x0) / (LOCAL.0 - 1) as f64).max((y1 - y0) / (LOCAL.1 - 1) as f64);
        let far = 0.08 * (x1 - x0).hypot(y1 - y0);
        let left: Vec<([f64; 2], [f64; 2])> = pairs
            .iter()
            .map(|(p, q)| {
                let h = w.apply(*p);
                (*p, [q[0] - h[0], q[1] - h[1]])
            })
            .filter(|(_, r)| r[0].hypot(r[1]) < far)
            .collect();
        let median = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        m.local = grid_points(rect)
            .iter()
            .map(|g| {
                let near: Vec<[f64; 2]> = left.iter().filter(|(p, _)| (p[0] - g[0]).hypot(p[1] - g[1]) <= 1.5 * cell).map(|(_, r)| *r).collect();
                if near.len() < 3 {
                    return [0.0; 2];
                }
                [median(&mut near.iter().map(|r| r[0]).collect()), median(&mut near.iter().map(|r| r[1]).collect())]
            })
            .collect();
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
    /// In perspective, and bent on a grid so each part is steadied on its own.
    Subspace,
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
    /// Shown whole, what the moved frames leave empty filled from the frames round them.
    Synthesize,
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

/// What a stabilized clip is: each frame's map (framing included), the part of the frame kept
/// (a fraction of it about its middle; 1 keeps it all), and (Subspace Warp) how far each point
/// of the grid over each frame is moved besides, before the map.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub maps: Vec<Warp>,
    pub keep: f64,
    pub scale: f64,
    pub bends: Vec<Vec<[f64; 2]>>,
}

impl Plan {
    /// Where the picture of frame `k` shown at point `p` comes from in the frame (none where its
    /// map cannot be undone): back through its map, then back by its bend there (`rect`: the
    /// frame's).
    pub fn source(&self, k: usize, p: [f64; 2], rect: [f64; 4]) -> Option<[f64; 2]> {
        let q = self.maps.get(k)?.inverse()?.apply(p);
        let Some(b) = self.bends.get(k).filter(|b| b.len() == LOCAL.0 * LOCAL.1) else { return Some(q) };
        let [x0, y0, x1, y1] = rect;
        let x = ((q[0] - x0) / (x1 - x0).max(1e-9) * (LOCAL.0 - 1) as f64).clamp(0.0, (LOCAL.0 - 1) as f64);
        let y = ((q[1] - y0) / (y1 - y0).max(1e-9) * (LOCAL.1 - 1) as f64).clamp(0.0, (LOCAL.1 - 1) as f64);
        let (i0, j0) = ((x.floor() as usize).min(LOCAL.0 - 2), (y.floor() as usize).min(LOCAL.1 - 2));
        let (fx, fy) = (x - i0 as f64, y - j0 as f64);
        let at = |i: usize, j: usize| b[j * LOCAL.0 + i];
        let c: [f64; 2] = std::array::from_fn(|n| {
            let top = at(i0, j0)[n] + (at(i0 + 1, j0)[n] - at(i0, j0)[n]) * fx;
            let bottom = at(i0, j0 + 1)[n] + (at(i0 + 1, j0 + 1)[n] - at(i0, j0 + 1)[n]) * fx;
            top + (bottom - top) * fy
        });
        Some([q[0] - c[0], q[1] - c[1]])
    }
}

/// How far each grid point of each frame is moved (Subspace Warp): each point's path (the parts'
/// own moves added up frame after frame) smoothed as the camera's is (or held where it began),
/// less the path itself.
pub fn bends(a: &Analysis, s: &Settings) -> Vec<Vec<[f64; 2]>> {
    let n = LOCAL.0 * LOCAL.1;
    if s.method != Method::Subspace || a.frames.iter().skip(1).any(|m| m.local.len() != n) {
        return Vec::new();
    }
    // each point's path
    let mut path = vec![vec![[0.0; 2]; n]];
    for m in a.frames.iter().skip(1) {
        let last = path.last().expect("one at least");
        path.push((0..n).map(|v| [last[v][0] + m.local[v][0], last[v][1] + m.local[v][1]]).collect());
    }
    let rate = 1.0 / a.step.max(1e-9);
    let sigma = (s.smoothness / 100.0 * rate).max(1e-3);
    let reach = (3.0 * sigma).ceil() as isize;
    let last = path.len() as isize - 1;
    // past either end mirrored about it, so a steady drift goes on steadily
    let at = |j: isize, v: usize| -> [f64; 2] {
        let (e, k) = if j < 0 { (0, (-j).min(last)) } else if j > last { (last, (2 * last - j).max(0)) } else { return path[j as usize][v] };
        let (pe, pk) = (path[e as usize][v], path[k as usize][v]);
        [2.0 * pe[0] - pk[0], 2.0 * pe[1] - pk[1]]
    };
    (0..path.len())
        .map(|k| {
            (0..n)
                .map(|v| {
                    let target = match s.aim {
                        Aim::Still => [0.0; 2],
                        Aim::Smooth => {
                            let (mut sum, mut total) = ([0.0; 2], 0.0);
                            for j in k as isize - reach..=k as isize + reach {
                                let w = (-((j - k as isize) as f64).powi(2) / (2.0 * sigma * sigma)).exp();
                                let p = at(j, v);
                                sum = [sum[0] + w * p[0], sum[1] + w * p[1]];
                                total += w;
                            }
                            [sum[0] / total, sum[1] / total]
                        }
                    };
                    [target[0] - path[k][v][0], target[1] - path[k][v][1]]
                })
                .collect()
        })
        .collect()
}

pub fn plan(a: &Analysis, s: &Settings) -> Plan {
    let moved = corrections(a, s);
    let cover = covered(a, &moved);
    let extra = s.extra_scale / 100.0;
    let (scale, keep) = match s.framing {
        Framing::Stabilize | Framing::Synthesize => (extra, 1.0),
        Framing::Crop => (extra, cover),
        Framing::CropScale => {
            let up = (1.0 / cover.max(1e-6)).min(s.most_scale.max(100.0) / 100.0);
            (up * extra, (cover * up).min(1.0))
        }
    };
    let [x0, y0, x1, y1] = a.rect;
    let mid = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
    let zoom = [[scale, 0.0, mid[0] * (1.0 - scale)], [0.0, scale, mid[1] * (1.0 - scale)], [0.0, 0.0, 1.0]];
    Plan { maps: moved.iter().map(|w| Warp(times(&zoom, &w.0))).collect(), keep, scale, bends: bends(a, s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shaky(n: usize) -> Analysis {
        // a camera gliding right three pixels a frame, shaking up and down
        let shake = |k: usize| if k == 0 { 0.0 } else { 4.0 * ((k as f64) * 2.1).sin() - 4.0 * (((k - 1) as f64) * 2.1).sin() };
        let frames = (0..n).map(|k| if k == 0 { Motion::STILL } else { Motion { shift: [3.0, shake(k)], similar: [1.0, 0.0, 3.0, shake(k)], perspective: [1.0, 0.0, 3.0, 0.0, 1.0, shake(k), 0.0, 0.0], agreed: 100, followed: 100, local: Vec::new() } }).collect();
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

    /// A frame `w` × `h` of the waves seen moved by `cam`, its lowest third moved by `extra` more
    /// (nearer things, moving further).
    fn parted(w: usize, h: usize, cam: [f64; 2], extra: [f64; 2]) -> Image {
        let mut img = Image::new(w, h);
        for j in 0..h {
            for i in 0..w {
                let low = j * 3 >= 2 * h;
                let (px, py) = (i as f64 + 0.5 + cam[0] + if low { extra[0] } else { 0.0 }, j as f64 + 0.5 + cam[1] + if low { extra[1] } else { 0.0 });
                let t = 0.5 + 0.12 * (px * 0.21).sin() * (py * 0.17).cos() + 0.1 * ((px + py) * 0.093).sin() + 0.08 * (px * 0.051 - py * 0.13).cos() + 0.06 * ((px * 0.37).sin() * (py * 0.29).sin());
                img.px[j * w + i] = [t as f32, t as f32, t as f32, 1.0];
            }
        }
        img
    }

    #[test]
    fn subspace_warp_steadies_each_part_on_its_own() {
        let cam = |k: usize| [2.0 * (k as f64 * 1.3).sin(), 2.0 * (k as f64 * 1.7).cos()];
        let extra = |k: usize| [3.0 * (k as f64 * 2.1).sin(), 2.5 * (k as f64 * 0.8).cos()];
        let mut an = Analyzer::new(0.0, 0.04, [0.0, 0.0, 320.0, 240.0]);
        for k in 0..16 {
            an.add(&parted(320, 240, cam(k), extra(k)), [0.0; 2]);
        }
        let a = an.finish();
        assert!(a.frames[1..].iter().all(|m| m.local.len() == LOCAL.0 * LOCAL.1));
        let back = Analysis::from_text(&a.to_text()).unwrap();
        assert_eq!(back.frames[3].local.len(), LOCAL.0 * LOCAL.1);
        // a point of the near part, held where it was on the first frame
        let seen = |k: usize| {
            let (c, e) = (cam(k), extra(k));
            [200.0 - c[0] - e[0], 205.0 - c[1] - e[1]]
        };
        let held = |method: Method| {
            let p = plan(&a, &Settings { aim: Aim::Still, smoothness: 50.0, method, framing: Framing::Stabilize, most_scale: 150.0, extra_scale: 100.0 });
            (1..16).map(|k| {
                let q = p.source(k, seen(0), a.rect).unwrap();
                (q[0] - seen(k)[0]).hypot(q[1] - seen(k)[1])
            }).fold(0.0, f64::max)
        };
        let (whole, parts) = (held(Method::Perspective), held(Method::Subspace));
        assert!(whole > 2.0, "the frame's own motion leaves the near part shaking: {whole}");
        assert!(parts < 1.0 && parts < 0.25 * whole, "each part steadied: {parts} (whole frame {whole})");
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
