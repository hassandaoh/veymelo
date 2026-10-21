//! Following what moves in footage, the way a compositor's point tracker does: each track point
//! is a feature (a small patch of the picture to follow) and a search area round it (where to
//! look for it in the next frame).
//!
//! The patch is looked for by normalised cross-correlation — how alike two patches are, whatever
//! their brightness and contrast — first everywhere in the search area on a coarse copy of the
//! frame (halved a few times), then nearer and nearer on finer copies. The best place is then
//! found to a fraction of a pixel by Lucas–Kanade alignment (Gauss–Newton on the difference),
//! which also allows for the patch getting brighter or darker and, if asked, turning and growing.
//! How alike the patch and the place found are is the confidence; below a threshold the tracker
//! goes on, stops, carries the motion on, or takes the patch afresh from this frame.
//!
//! Points are in the layer's pixels: an image pixel is a layer point plus the frame's offset.

pub mod fit;
pub mod flow;
pub mod klt;
pub mod planar;
pub mod plane;
pub mod region;
pub mod solve;
pub mod stabilize;

pub use plane::{Channel, Plane};
use veymelo_raster::{Image, par};
pub use solve::{Warp, angle_and_length, nearest_turn};

/// What the tracker does where it is unsure of a match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Unsure {
    /// Takes it all the same.
    Continue,
    /// Stops before the frame.
    Stop,
    /// Leaves it, carrying on the motion it had.
    Extrapolate,
    /// Takes it, and the patch afresh from the frame.
    #[default]
    Adapt,
}

impl Unsure {
    pub const IDS: [&'static str; 4] = ["continue", "stop", "extrapolate", "adapt"];

    pub fn from_index(i: u32) -> Unsure {
        [Unsure::Continue, Unsure::Stop, Unsure::Extrapolate, Unsure::Adapt][(i as usize).min(3)]
    }
}

/// How the tracker matches.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub channel: Channel,
    /// A softening of the frames first, in pixels (0: none).
    pub blur: f64,
    /// Edges brought out first.
    pub enhance: bool,
    /// To a fraction of a pixel (else whole pixels).
    pub subpixel: bool,
    /// The patch taken afresh every frame.
    pub adapt_every_frame: bool,
    /// The confidence, in %, below which it is unsure.
    pub threshold: f64,
    pub unsure: Unsure,
    /// Follow each patch turning and growing as it is matched.
    pub follow_shape: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { channel: Channel::Luminance, blur: 0.0, enhance: false, subpixel: true, adapt_every_frame: false, threshold: 80.0, unsure: Unsure::Adapt, follow_shape: true }
    }
}

/// A track point: where its feature is, how big it is, and the search area round it (its size,
/// and how far its middle is from the feature's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub center: [f64; 2],
    pub feature: [f64; 2],
    pub search: [f64; 2],
    pub search_offset: [f64; 2],
}

/// A frame to track in.
#[derive(Clone, Copy)]
pub struct Frame<'a> {
    pub img: &'a Image,
    /// Where the layer's (0, 0) is in the image.
    pub offset: [f64; 2],
}

/// How a point was placed on a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    Tracked,
    /// Tracked, and its patch taken afresh here.
    Adapted,
    /// Unsure: the motion it had carried on.
    Extrapolated,
    /// Unsure, and told to stop: no place on this frame.
    Stopped,
}

/// Where a point was found on a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Found {
    /// The feature's middle, in the layer's pixels.
    pub center: [f64; 2],
    /// How alike it is to the patch, in %.
    pub confidence: f64,
    /// How far the patch has turned (degrees) and grown (1: not) since it was taken.
    pub rotation: f64,
    pub scale: f64,
    pub how: How,
}

/// A patch read on a grid one pixel apart (of its level) round a middle, turned and scaled by
/// `turn` = (s cos θ, s sin θ): its values, them less their mean, and how much those vary.
#[derive(Clone, Debug)]
struct Patch {
    r: [i32; 2],
    depth: usize,
    raw: Vec<f32>,
    centred: Vec<f32>,
    energy: f64,
}

/// Where the grid point `u` of a patch with middle `c` and turn `t` falls.
#[inline]
fn on(c: [f64; 2], t: [f64; 2], u: [f64; 2]) -> [f64; 2] {
    [c[0] + t[0] * u[0] - t[1] * u[1], c[1] + t[1] * u[0] + t[0] * u[1]]
}

impl Patch {
    fn read(plane: &Plane, c: [f64; 2], t: [f64; 2], r: [i32; 2]) -> Patch {
        let depth = plane.depth;
        let mut raw = Vec::with_capacity(((2 * r[0] + 1) * (2 * r[1] + 1)) as usize * depth);
        let mut v = [0.0; 3];
        for y in -r[1]..=r[1] {
            for x in -r[0]..=r[0] {
                let q = on(c, t, [x as f64, y as f64]);
                plane.read(q[0], q[1], &mut v);
                raw.extend_from_slice(&v[..depth]);
            }
        }
        let mean = raw.iter().map(|v| *v as f64).sum::<f64>() / raw.len().max(1) as f64;
        let centred: Vec<f32> = raw.iter().map(|v| (*v as f64 - mean) as f32).collect();
        let energy = centred.iter().map(|v| (*v as f64).powi(2)).sum();
        Patch { r, depth, raw, centred, energy }
    }
}

/// How alike `patch` is to `plane` read at middle `c` and turn `t`: their normalised
/// cross-correlation, −1 to 1.
fn alike(plane: &Plane, patch: &Patch, c: [f64; 2], t: [f64; 2]) -> f64 {
    if patch.energy <= 1e-12 {
        return 0.0;
    }
    let d = patch.depth;
    let (mut sum, mut squares, mut cross) = (0.0f64, 0.0f64, 0.0f64);
    let mut k = 0;
    let (fx, fy) = (c[0] - 0.5, c[1] - 0.5);
    let (bx, by) = (fx.floor() as i64, fy.floor() as i64);
    let (rx, ry) = (patch.r[0] as i64, patch.r[1] as i64);
    let plain = (t[0] - 1.0).abs() < 1e-9 && t[1].abs() < 1e-9;
    if plain && bx - rx >= 0 && by - ry >= 0 && bx + rx + 1 < plane.w as i64 && by + ry + 1 < plane.h as i64 {
        // moved only: every grid point shares the same weights between its four pixels
        let (tx, ty) = ((fx - bx as f64) as f32, (fy - by as f64) as f32);
        let w = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
        let row = plane.w * d;
        for gy in -ry..=ry {
            let base = (by + gy) as usize * row;
            for gx in -rx..=rx {
                let i = base + (bx + gx) as usize * d;
                for ch in 0..d {
                    let v = (w[0] * plane.v[i + ch] + w[1] * plane.v[i + d + ch] + w[2] * plane.v[i + row + ch] + w[3] * plane.v[i + row + d + ch]) as f64;
                    sum += v;
                    squares += v * v;
                    cross += v * patch.centred[k] as f64;
                    k += 1;
                }
            }
        }
    } else {
        let mut v = [0.0; 3];
        for gy in -ry..=ry {
            for gx in -rx..=rx {
                let q = on(c, t, [gx as f64, gy as f64]);
                plane.read(q[0], q[1], &mut v);
                for x in v.iter().take(d) {
                    let x = *x as f64;
                    sum += x;
                    squares += x * x;
                    cross += x * patch.centred[k] as f64;
                    k += 1;
                }
            }
        }
    }
    let n = patch.centred.len() as f64;
    let spread = squares - sum * sum / n;
    if spread <= 1e-12 {
        return 0.0;
    }
    (cross / (spread * patch.energy).sqrt()).clamp(-1.0, 1.0)
}

/// The frame's pixels round a point, ready to match in: a pyramid of a crop. A point of the
/// image is at `(p - origin) / 2^k` on level `k`.
struct Pyramid {
    origin: [f64; 2],
    levels: Vec<Plane>,
    slopes: (Plane, Plane),
}

impl Pyramid {
    /// The image's pixels `reach` either side of `c` (and a margin), in `levels` + 1 levels.
    fn new(frame: &Frame, options: &Options, c: [f64; 2], reach: [f64; 2], levels: usize) -> Pyramid {
        let sigma = options.blur.max(0.0) / 2.0;
        let margin = (3.0 * sigma).ceil() + 3.0 + if options.enhance { 6.0 } else { 0.0 } + (1usize << levels) as f64;
        let (x0, y0) = ((c[0] - reach[0] - margin).floor(), (c[1] - reach[1] - margin).floor());
        let (w, h) = (((2.0 * (reach[0] + margin)).ceil() as usize + 2).max(4), ((2.0 * (reach[1] + margin)).ceil() as usize + 2).max(4));
        let mut p = Plane::crop(frame.img, options.channel, x0 as i64, y0 as i64, w, h);
        if sigma > 0.05 {
            p = p.blurred(sigma);
        }
        if options.enhance {
            p = p.sharpened();
        }
        let slopes = p.slopes();
        let mut all = vec![p];
        for _ in 0..levels {
            let next = all.last().map(Plane::halved).unwrap_or_default();
            all.push(next);
        }
        Pyramid { origin: [x0, y0], levels: all, slopes }
    }

    fn down(&self, p: [f64; 2], k: usize) -> [f64; 2] {
        let s = (1u32 << k) as f64;
        [(p[0] - self.origin[0]) / s, (p[1] - self.origin[1]) / s]
    }

    fn up(&self, p: [f64; 2], k: usize) -> [f64; 2] {
        let s = (1u32 << k) as f64;
        [p[0] * s + self.origin[0], p[1] * s + self.origin[1]]
    }
}

/// One point being followed.
#[derive(Clone, Debug)]
struct Follow {
    point: Point,
    /// Its middle now, in image pixels.
    at: [f64; 2],
    /// How it has turned and grown since its patch was taken: (s cos θ, s sin θ).
    turn: [f64; 2],
    /// How far it moved last frame.
    moving: [f64; 2],
    patches: Vec<Patch>,
    levels: usize,
}

impl Follow {
    fn radius(&self) -> [i32; 2] {
        self.point.feature.map(|v| (v / 2.0).round().max(2.0) as i32)
    }

    fn radius_at(&self, k: usize) -> [i32; 2] {
        self.radius().map(|r| ((r as f64 / (1u32 << k) as f64).ceil() as i32).max(2))
    }

    /// How far round where it should be to look, across and down.
    fn range(&self) -> [f64; 2] {
        [0, 1].map(|i| (self.point.search[i] / 2.0 - self.point.feature[i] / 2.0).max(2.0))
    }

    /// How far the patch reaches from its middle, turned and grown.
    fn reach(&self) -> f64 {
        let r = self.radius();
        ((r[0] * r[0] + r[1] * r[1]) as f64).sqrt() * self.turn[0].hypot(self.turn[1]).max(1.0) + 2.0
    }

    fn new(point: Point, frame: &Frame, options: &Options) -> Follow {
        let at = [point.center[0] + frame.offset[0], point.center[1] + frame.offset[1]];
        let mut f = Follow { point, at, turn: [1.0, 0.0], moving: [0.0; 2], patches: Vec::new(), levels: 0 };
        // as many halvings as leave the search and the patch three pixels or more
        let (range, r) = (f.range(), f.radius());
        while f.levels < 4 && range[0].min(range[1]) / (2u32 << f.levels) as f64 >= 3.0 && r[0].min(r[1]) as f64 / (2u32 << f.levels) as f64 >= 3.0 {
            f.levels += 1;
        }
        let reach = f.reach();
        let pyramid = Pyramid::new(frame, options, at, [reach, reach], f.levels);
        f.take(&pyramid);
        f
    }

    /// The patch taken (afresh) where it is now.
    fn take(&mut self, p: &Pyramid) {
        self.patches = (0..=self.levels).map(|k| Patch::read(&p.levels[k], p.down(self.at, k), self.turn, self.radius_at(k))).collect();
    }

    fn found(&self, c: [f64; 2], frame: &Frame, confidence: f64, how: How) -> Found {
        Found { center: [c[0] - frame.offset[0], c[1] - frame.offset[1]], confidence, rotation: self.turn[1].atan2(self.turn[0]).to_degrees(), scale: self.turn[0].hypot(self.turn[1]), how }
    }

    fn step(&mut self, frame: &Frame, options: &Options) -> Found {
        let was = self.at;
        let guess = [was[0] + self.moving[0], was[1] + self.moving[1]];
        let middle = [guess[0] + self.point.search_offset[0], guess[1] + self.point.search_offset[1]];
        let (range, reach, top, turn) = (self.range(), self.reach(), self.levels, self.turn);
        let p = Pyramid::new(frame, options, middle, [range[0] + reach, range[1] + reach], top);
        // everywhere in the search area, on the coarsest level
        let s = (1u32 << top) as f64;
        let (rx, ry) = ((range[0] / s).ceil() as i64, (range[1] / s).ceil() as i64);
        let (plane, patch, m) = (&p.levels[top], &self.patches[top], p.down(middle, top));
        let width = (2 * rx + 1) as usize;
        let mut scores = vec![0.0f64; width * (2 * ry + 1) as usize];
        par::rows(&mut scores, width, |j, row| {
            for (i, score) in row.iter_mut().enumerate() {
                *score = alike(plane, patch, [m[0] + i as f64 - rx as f64, m[1] + j as f64 - ry as f64], turn);
            }
        });
        // the best few places, apart; of equals, the one nearest the guess (so plain areas stay)
        let g = p.down(guess, top);
        let mut places: Vec<(f64, [f64; 2])> = scores.iter().enumerate().map(|(k, s)| (*s, [m[0] + (k % width) as f64 - rx as f64, m[1] + (k / width) as f64 - ry as f64])).collect();
        let far = |c: &[f64; 2]| (c[0] - g[0]).powi(2) + (c[1] - g[1]).powi(2);
        places.sort_by(|a, b| if (a.0 - b.0).abs() < 1e-9 { far(&a.1).total_cmp(&far(&b.1)) } else { b.0.total_cmp(&a.0) });
        let mut seeds: Vec<[f64; 2]> = Vec::new();
        for (_, c) in &places {
            if seeds.iter().all(|q| (q[0] - c[0]).abs() > 1.5 || (q[1] - c[1]).abs() > 1.5) {
                seeds.push(*c);
                if seeds.len() == 3 {
                    break;
                }
            }
        }
        // each followed down the finer levels, looking two pixels round
        let mut best = (f64::NEG_INFINITY, middle);
        for seed in seeds {
            let (mut c, mut score) = (seed, alike(plane, patch, seed, turn));
            for k in (0..top).rev() {
                c = [c[0] * 2.0, c[1] * 2.0];
                let (pl, pa) = (&p.levels[k], &self.patches[k]);
                score = alike(pl, pa, c, turn);
                let mut here = c;
                for j in -2..=2 {
                    for i in -2..=2 {
                        let q = [c[0] + i as f64, c[1] + j as f64];
                        let v = alike(pl, pa, q, turn);
                        if v > score + 1e-9 {
                            (score, here) = (v, q);
                        }
                    }
                }
                c = here;
            }
            if score > best.0 + 1e-9 {
                best = (score, p.up(c, 0));
            }
        }
        let whole = p.down(best.1, 0);
        let (mut c, mut t) = (whole, turn);
        if options.subpixel {
            match refine(&p, &self.patches[0], whole, turn, options.follow_shape).or_else(|| refine(&p, &self.patches[0], whole, turn, false)) {
                Some(r) => (c, t) = r,
                None => c = peak(&p.levels[0], &self.patches[0], whole, turn),
            }
        }
        // a patch with nothing in it cannot be found: it stays where it should be
        if self.patches[0].energy < 1e-6 {
            (c, t) = (p.down(guess, 0), turn);
        }
        let mut confidence = alike(&p.levels[0], &self.patches[0], c, t);
        let coarse = alike(&p.levels[0], &self.patches[0], whole, turn);
        if coarse > confidence + 0.02 {
            (c, t, confidence) = (whole, turn, coarse);
        }
        let confidence = (confidence.max(0.0) * 100.0).min(100.0);
        let now = p.up(c, 0);
        let unsure = confidence < options.threshold;
        let mut how = How::Tracked;
        if unsure {
            match options.unsure {
                Unsure::Stop => return self.found(was, frame, confidence, How::Stopped),
                Unsure::Extrapolate => {
                    self.at = guess;
                    return self.found(guess, frame, confidence, How::Extrapolated);
                }
                Unsure::Continue => {}
                Unsure::Adapt => how = How::Adapted,
            }
        }
        // only a sure match says where to look next
        self.moving = if unsure { [0.0; 2] } else { [now[0] - was[0], now[1] - was[1]] };
        (self.at, self.turn) = (now, t);
        if options.adapt_every_frame || how == How::Adapted {
            how = How::Adapted;
            self.take(&p);
        }
        self.found(now, frame, confidence, how)
    }
}

/// The best place to a fraction of a pixel from a parabola through the likeness either side of
/// `c`, across and down.
fn peak(plane: &Plane, patch: &Patch, c: [f64; 2], t: [f64; 2]) -> [f64; 2] {
    let at = |dx: f64, dy: f64| alike(plane, patch, [c[0] + dx, c[1] + dy], t);
    let mid = at(0.0, 0.0);
    let shift = |before: f64, after: f64| {
        let bend = before - 2.0 * mid + after;
        if bend.abs() < 1e-12 { 0.0 } else { (0.5 * (before - after) / bend).clamp(-0.5, 0.5) }
    };
    [c[0] + shift(at(-1.0, 0.0), at(1.0, 0.0)), c[1] + shift(at(0.0, -1.0), at(0.0, 1.0))]
}

/// Lucas–Kanade: the middle `c` (and with `shape` the turn `t`) that make the frame read there
/// most like the patch, allowing it to be brighter or darker (gain `g`, bias `b`): the sum of
/// (I(c + t u) − g P(u) − b)² made least by Gauss–Newton steps. None when it runs away.
fn refine(p: &Pyramid, patch: &Patch, c0: [f64; 2], t0: [f64; 2], shape: bool) -> Option<([f64; 2], [f64; 2])> {
    let (plane, (sx, sy)) = (&p.levels[0], &p.slopes);
    let d = patch.depth;
    let (mut c, mut t) = (c0, t0);
    let (mut v, mut vx, mut vy) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
    // gain and bias to start from: matching the spread and mean of what is there
    let (mut g, mut b) = {
        let (mut si, mut sii, mut sp, mut spp, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
        let mut k = 0;
        for y in -patch.r[1]..=patch.r[1] {
            for x in -patch.r[0]..=patch.r[0] {
                let q = on(c, t, [x as f64, y as f64]);
                plane.read(q[0], q[1], &mut v);
                for vi in v.iter().take(d) {
                    let (i, pv) = (*vi as f64, patch.raw[k] as f64);
                    (si, sii, sp, spp, n) = (si + i, sii + i * i, sp + pv, spp + pv * pv, n + 1.0);
                    k += 1;
                }
            }
        }
        let (mi, mp) = (si / n, sp / n);
        let (vi, vp) = ((sii / n - mi * mi).max(0.0), (spp / n - mp * mp).max(0.0));
        let g = if vi > 1e-12 && vp > 1e-12 { (vi / vp).sqrt() } else { 1.0 };
        (g, mi - g * mp)
    };
    for _ in 0..30 {
        let mut h = [[0.0f64; 6]; 6];
        let mut rhs = [0.0f64; 6];
        let mut k = 0;
        for y in -patch.r[1]..=patch.r[1] {
            for x in -patch.r[0]..=patch.r[0] {
                let u = [x as f64, y as f64];
                let q = on(c, t, u);
                plane.read(q[0], q[1], &mut v);
                sx.read(q[0], q[1], &mut vx);
                sy.read(q[0], q[1], &mut vy);
                for ch in 0..d {
                    let pv = patch.raw[k] as f64;
                    k += 1;
                    let (ix, iy) = (vx[ch] as f64, vy[ch] as f64);
                    let e = v[ch] as f64 - g * pv - b;
                    // how the difference changes with each unknown: middle, turn, gain, bias
                    let j = [ix, iy, ix * u[0] + iy * u[1], iy * u[0] - ix * u[1], -pv, -1.0];
                    for m in 0..6 {
                        rhs[m] -= j[m] * e;
                        for n in m..6 {
                            h[m][n] += j[m] * j[n];
                        }
                    }
                }
            }
        }
        // the lower half as the upper, and a touch more on the diagonal to keep it solvable
        let upper = h;
        for (m, row) in h.iter_mut().enumerate() {
            for (n, v) in row.iter_mut().enumerate().take(m) {
                *v = upper[n][m];
            }
            row[m] = row[m] * (1.0 + 1e-6) + 1e-9;
        }
        let step = if shape {
            solve::linear(h, rhs)?
        } else {
            let pick = [0, 1, 4, 5];
            let x = solve::linear(pick.map(|m| pick.map(|n| h[m][n])), pick.map(|m| rhs[m]))?;
            [x[0], x[1], 0.0, 0.0, x[2], x[3]]
        };
        // no more than a pixel a step
        let len = step[0].hypot(step[1]);
        let k = if len > 1.0 { 1.0 / len } else { 1.0 };
        c = [c[0] + step[0] * k, c[1] + step[1] * k];
        t = [t[0] + step[2] * k, t[1] + step[3] * k];
        (g, b) = (g + step[4] * k, b + step[5] * k);
        if !(c[0].is_finite() && c[1].is_finite() && t[0].is_finite() && t[1].is_finite()) {
            return None;
        }
        if len < 1e-4 && step[2].abs() + step[3].abs() < 1e-6 {
            break;
        }
    }
    let grew = t[0].hypot(t[1]) / t0[0].hypot(t0[1]).max(1e-9);
    if (c[0] - c0[0]).hypot(c[1] - c0[1]) > 2.5 || !(0.75..=1.33).contains(&grew) || g <= 0.0 {
        return None;
    }
    Some((c, t))
}

/// The points of one tracker, followed frame after frame.
#[derive(Clone, Debug)]
pub struct Tracker {
    pub options: Options,
    follows: Vec<Follow>,
}

/// `f` on each of `items`, on threads of their own when there are several.
fn each<T: Send, R: Send>(items: &mut [T], f: impl Fn(&mut T) -> R + Sync) -> Vec<R> {
    if items.len() < 2 {
        return items.iter_mut().map(f).collect();
    }
    std::thread::scope(|s| {
        let f = &f;
        let handles: Vec<_> = items.iter_mut().map(|item| s.spawn(move || f(item))).collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
    })
}

impl Tracker {
    /// Begin following `points` as they are on `frame`.
    pub fn new(options: Options, points: &[Point], frame: &Frame) -> Tracker {
        let mut follows: Vec<Option<Follow>> = vec![None; points.len()];
        let mut work: Vec<(Point, &mut Option<Follow>)> = points.iter().copied().zip(follows.iter_mut()).collect();
        each(&mut work, |(p, slot)| **slot = Some(Follow::new(*p, frame, &options)));
        Tracker { options, follows: follows.into_iter().flatten().collect() }
    }

    /// Every point found on the next frame (later or earlier).
    pub fn step(&mut self, frame: &Frame) -> Vec<Found> {
        let options = &self.options;
        each(&mut self.follows, |f| f.step(frame, options))
    }

    pub fn len(&self) -> usize {
        self.follows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.follows.is_empty()
    }
}

#[cfg(test)]
mod tests;
