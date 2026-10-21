//! Immersive video: pictures of everything round the camera, laid out as equirectangular frames
//! (longitude across, latitude down, the front in the middle). They are blurred, sharpened,
//! cleaned and made to glow as on the sphere — across the seam at the sides and over the poles,
//! the same size everywhere on the sphere rather than on the frame — coloured and noised by
//! direction, turned, split into colours round a point, broken by glitches, and converted to and
//! from cube maps, fisheyes and flat views. Stereo frames (one eye over the other, or side by
//! side) are done an eye at a time.
//!
//! Directions: x to the right, y up, z to the front. A view is turned right by its pan, up by its
//! tilt and about its middle by its roll.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use veymelo_project::PropDef;
use veymelo_raster::{Image, Px};

use crate::defs::{angle, choice, color, effect, flag, point, ranged};
use crate::generate::{MODES, put_with};
use crate::util::{luma, map_px, premul, unpremul};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "immersive";

type V3 = [f64; 3];
type M3 = [[f64; 3]; 3];

pub const LAYOUTS: &[&str] = &["monoscopic", "stereoOverUnder", "stereoSideBySide"];

const fn frame_layout() -> PropDef {
    choice("frameLayout", "Frame Layout", LAYOUTS, 0).doc("stereo frames are done an eye at a time")
}

// ---------------------------------------------------------------- eyes

/// The eyes' rectangles in a `w` × `h` frame: x, y, width, height.
fn eyes(w: usize, h: usize, layout: u32) -> Vec<[usize; 4]> {
    match layout {
        1 if h >= 2 => vec![[0, 0, w, h / 2], [0, h / 2, w, h - h / 2]],
        2 if w >= 2 => vec![[0, 0, w / 2, h], [w / 2, 0, w - w / 2, h]],
        _ => vec![[0, 0, w, h]],
    }
}

/// `f` done to each eye of `img` on its own (with the eye's number).
fn per_eye(img: &Image, layout: u32, f: impl Fn(&Image, usize) -> Image) -> Image {
    let rects = eyes(img.width, img.height, layout);
    if rects.len() == 1 {
        return f(img, 0);
    }
    let mut out = Image::new(img.width, img.height);
    for (i, &[x0, y0, w, h]) in rects.iter().enumerate() {
        let mut eye = Image::new(w, h);
        for y in 0..h {
            eye.px[y * w..(y + 1) * w].copy_from_slice(&img.px[(y0 + y) * img.width + x0..][..w]);
        }
        let done = f(&eye, i);
        for y in 0..h {
            out.px[(y0 + y) * img.width + x0..][..w].copy_from_slice(&done.px[y * w..(y + 1) * w]);
        }
    }
    out
}

// ---------------------------------------------------------------- directions

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(v: V3) -> V3 {
    let l = dot(v, v).sqrt().max(1e-300);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// The angle between two directions (radians).
fn apart(a: V3, b: V3) -> f64 {
    dot(a, b).clamp(-1.0, 1.0).acos()
}

/// The direction seen at point (x, y) of a `w` × `h` equirectangular frame.
fn dir(x: f64, y: f64, w: f64, h: f64) -> V3 {
    let (lon, lat) = ((x / w - 0.5) * TAU, (0.5 - y / h) * PI);
    [lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos()]
}

/// Where direction `d` is in a `w` × `h` equirectangular frame.
fn spot(d: V3, w: f64, h: f64) -> (f64, f64) {
    let (lon, lat) = (d[0].atan2(d[2]), d[1].clamp(-1.0, 1.0).asin());
    ((lon / TAU + 0.5) * w, (0.5 - lat / PI) * h)
}

fn times(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn turn(m: &M3, v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

fn turned_back(m: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| m[j][i]))
}

/// A view turned right by `pan`, up by `tilt` and about its middle (clockwise) by `roll`,
/// degrees: what it sees in direction d of its own is direction `m · d` of the scene.
fn rotation(tilt: f64, pan: f64, roll: f64) -> M3 {
    let (a, b, c) = (-tilt.to_radians(), pan.to_radians(), -roll.to_radians());
    let x = [[1.0, 0.0, 0.0], [0.0, a.cos(), -a.sin()], [0.0, a.sin(), a.cos()]];
    let y = [[b.cos(), 0.0, b.sin()], [0.0, 1.0, 0.0], [-b.sin(), 0.0, b.cos()]];
    let z = [[c.cos(), -c.sin(), 0.0], [c.sin(), c.cos(), 0.0], [0.0, 0.0, 1.0]];
    times(&y, &times(&x, &z))
}

fn view_of(ctx: &Ctx) -> M3 {
    rotation(ctx.num("tilt", 0.0), ctx.num("pan", 0.0), ctx.num("roll", 0.0))
}

/// Pixel (x, y) of an equirectangular frame, going on round the sides and over the poles (past
/// a pole, down the other side of the sphere).
fn texel(img: &Image, x: i64, y: i64) -> Px {
    let (w, h) = (img.width as i64, img.height as i64);
    let (mut x, mut y) = (x, y);
    if y < 0 {
        (x, y) = (x + w / 2, -1 - y);
    } else if y >= h {
        (x, y) = (x + w / 2, 2 * h - 1 - y);
    }
    img.px[(y.clamp(0, h - 1) * w + x.rem_euclid(w)) as usize]
}

/// An equirectangular frame at point (x, y), between pixels.
fn sample(img: &Image, x: f64, y: f64) -> Px {
    if img.px.is_empty() {
        return [0.0; 4];
    }
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let (a, b, c, d) = (texel(img, x0, y0), texel(img, x0 + 1, y0), texel(img, x0, y0 + 1), texel(img, x0 + 1, y0 + 1));
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * tx;
        top + (c[k] + (d[k] - c[k]) * tx - top) * ty
    })
}

/// An equirectangular frame at point (x, y) by a cubic through 4 × 4 pixels (sharper than
/// [`sample`] when turned again and again), held within the four nearest so it never rings.
fn sample_sharp(img: &Image, x: f64, y: f64) -> Px {
    if img.px.is_empty() {
        return [0.0; 4];
    }
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let cubic = |a: f32, b: f32, c: f32, d: f32, t: f32| b + 0.5 * t * (c - a + t * (2.0 * a - 5.0 * b + 4.0 * c - d + t * (3.0 * (b - c) + d - a)));
    let p: [[Px; 4]; 4] = std::array::from_fn(|j| std::array::from_fn(|i| texel(img, x0 - 1 + i as i64, y0 - 1 + j as i64)));
    std::array::from_fn(|k| {
        let rows: [f32; 4] = std::array::from_fn(|j| cubic(p[j][0][k], p[j][1][k], p[j][2][k], p[j][3][k], tx));
        let v = cubic(rows[0], rows[1], rows[2], rows[3], ty);
        let near = [p[1][1][k], p[1][2][k], p[2][1][k], p[2][2][k]];
        v.clamp(near.iter().fold(f32::MAX, |a, &b| a.min(b)), near.iter().fold(f32::MIN, |a, &b| a.max(b)))
    })
}

// ---------------------------------------------------------------- projections

/// The layouts frames are converted between.
pub const PROJECTIONS: &[&str] = &["equirectangular", "cubeMapCross4x3", "cubeMap3x2", "cubeMapStrip6x1", "sphereMap", "flat", "cubeMapFrontRow3x2", "cubeMapEqualAngle3x2", "dome"];

/// A cube's faces: which way each looks, its right and its up (+x, -x, +y, -y, +z, -z).
const FACES: [(V3, V3, V3); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
    ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
    ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
];

/// A cube map's grid, and where each face is in it (column, row).
fn cube_grid(kind: u32) -> ((usize, usize), [(usize, usize); 6]) {
    match kind {
        // a cross: the top over the front, left, front, right and back across, the bottom under
        1 => ((4, 3), [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (3, 1)]),
        // right, left, top over bottom, front, back
        2 => ((3, 2), [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)]),
        // left, front, right over bottom, back, top
        6 | 7 => ((3, 2), [(2, 0), (0, 0), (2, 1), (0, 1), (1, 0), (1, 1)]),
        // a strip in the faces' order
        _ => ((6, 1), [(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (5, 0)]),
    }
}

/// A projection, and its field of view (degrees across, for the sphere map and flat views).
#[derive(Clone, Copy, Debug)]
struct Proj {
    kind: u32,
    fov: f64,
}

impl Proj {
    const EQUI: Proj = Proj { kind: 0, fov: 360.0 };

    fn cube(&self) -> bool {
        matches!(self.kind, 1..=3 | 6 | 7)
    }

    /// The direction seen at point (x, y) of a `w` × `h` frame (none where nothing is).
    fn dir(&self, x: f64, y: f64, w: f64, h: f64) -> Option<V3> {
        match self.kind {
            0 => Some(dir(x, y, w, h)),
            _ if self.cube() => {
                let ((gw, gh), cells) = cube_grid(self.kind);
                let (cw, ch) = (w / gw as f64, h / gh as f64);
                let (cx, cy) = ((x / cw).floor(), (y / ch).floor());
                let face = cells.iter().position(|&(c, r)| c as f64 == cx && r as f64 == cy)?;
                let (mut a, mut b) = ((x - cx * cw) / cw * 2.0 - 1.0, (y - cy * ch) / ch * 2.0 - 1.0);
                if self.kind == 7 {
                    // equal angles across the face
                    (a, b) = ((a * FRAC_PI_4).tan(), (b * FRAC_PI_4).tan());
                }
                let (f, r, u) = FACES[face];
                Some(unit(std::array::from_fn(|k| f[k] + a * r[k] - b * u[k])))
            }
            // the dome: half the sphere looking up, the front at the bottom
            8 => {
                let s = w.min(h) / 2.0;
                let (u, v) = ((x - w / 2.0) / s, (y - h / 2.0) / s);
                let r = (u * u + v * v).sqrt();
                (r <= 1.0).then(|| {
                    let th = r * FRAC_PI_2;
                    let (cu, cv) = if r > 1e-12 { (u / r, v / r) } else { (0.0, 0.0) };
                    [th.sin() * cu, th.cos(), th.sin() * cv]
                })
            }
            // a sphere map: the angle from the front grows evenly out from the middle
            4 => {
                let half = (self.fov.clamp(1.0, 360.0) / 2.0).to_radians();
                let s = w.min(h) / 2.0;
                let (u, v) = ((x - w / 2.0) / s, (h / 2.0 - y) / s);
                let r = (u * u + v * v).sqrt();
                (r <= 1.0).then(|| {
                    let th = r * half;
                    let (cu, cv) = if r > 1e-12 { (u / r, v / r) } else { (0.0, 0.0) };
                    [th.sin() * cu, th.sin() * cv, th.cos()]
                })
            }
            // a flat view
            _ => {
                let t = (self.fov.clamp(1.0, 179.0) / 2.0).to_radians().tan();
                Some(unit([(x / w * 2.0 - 1.0) * t, (1.0 - y / h * 2.0) * t * h / w, 1.0]))
            }
        }
    }

    /// Where direction `d` is in a `w` × `h` frame, and for a cube map its face's rectangle
    /// (samples stay in it); none where it is not seen.
    fn spot(&self, d: V3, w: f64, h: f64) -> Option<(f64, f64, Option<[f64; 4]>)> {
        match self.kind {
            0 => {
                let (x, y) = spot(d, w, h);
                Some((x, y, None))
            }
            _ if self.cube() => {
                let face = (0..6).max_by(|&i, &j| dot(d, FACES[i].0).total_cmp(&dot(d, FACES[j].0)))?;
                let (f, r, u) = FACES[face];
                let k = dot(d, f);
                let (mut a, mut b) = (dot(d, r) / k, -dot(d, u) / k);
                if self.kind == 7 {
                    (a, b) = (a.atan() / FRAC_PI_4, b.atan() / FRAC_PI_4);
                }
                let ((gw, gh), cells) = cube_grid(self.kind);
                let (cw, ch) = (w / gw as f64, h / gh as f64);
                let (x0, y0) = (cells[face].0 as f64 * cw, cells[face].1 as f64 * ch);
                Some((x0 + (a + 1.0) * 0.5 * cw, y0 + (b + 1.0) * 0.5 * ch, Some([x0, y0, x0 + cw, y0 + ch])))
            }
            8 => {
                let th = d[1].clamp(-1.0, 1.0).acos();
                (th <= FRAC_PI_2 + 1e-9).then(|| {
                    let (s, r, l) = (w.min(h) / 2.0, th / FRAC_PI_2, (d[0] * d[0] + d[2] * d[2]).sqrt());
                    let (cu, cv) = if l > 1e-12 { (d[0] / l, d[2] / l) } else { (0.0, 0.0) };
                    (w / 2.0 + cu * r * s, h / 2.0 + cv * r * s, None)
                })
            }
            4 => {
                let half = (self.fov.clamp(1.0, 360.0) / 2.0).to_radians();
                let th = d[2].clamp(-1.0, 1.0).acos();
                (th <= half + 1e-9).then(|| {
                    let (s, r, l) = (w.min(h) / 2.0, th / half, (d[0] * d[0] + d[1] * d[1]).sqrt());
                    let (cu, cv) = if l > 1e-12 { (d[0] / l, d[1] / l) } else { (0.0, 0.0) };
                    (w / 2.0 + cu * r * s, h / 2.0 - cv * r * s, None)
                })
            }
            _ => {
                if d[2] <= 1e-9 {
                    return None;
                }
                let t = (self.fov.clamp(1.0, 179.0) / 2.0).to_radians().tan();
                let (u, v) = (d[0] / d[2] / t, d[1] / d[2] / (t * h / w));
                (u.abs() <= 1.0 && v.abs() <= 1.0).then_some(((u + 1.0) * 0.5 * w, (1.0 - v) * 0.5 * h, None))
            }
        }
    }

    /// About how many of a `w` × `h` frame's pixels a radian spans (in its middle).
    fn density(&self, w: f64, h: f64) -> f64 {
        match self.kind {
            0 => h / PI,
            _ if self.cube() => {
                let (gw, _) = cube_grid(self.kind).0;
                w / gw as f64 / FRAC_PI_2
            }
            8 => w.min(h) / 2.0 / FRAC_PI_2,
            4 => w.min(h) / 2.0 / (self.fov.clamp(1.0, 360.0) / 2.0).to_radians(),
            _ => w / 2.0 / (self.fov.clamp(1.0, 179.0) / 2.0).to_radians().tan(),
        }
    }

    /// Frame `img` (in this projection) in direction `d`.
    fn sample(&self, img: &Image, d: V3, sharp: bool) -> Px {
        let (w, h) = (img.width as f64, img.height as f64);
        match self.spot(d, w, h) {
            None => [0.0; 4],
            Some((x, y, None)) if self.kind == 0 => {
                if sharp { sample_sharp(img, x, y) } else { sample(img, x, y) }
            }
            Some((x, y, face)) => {
                // within the pixels wholly inside the face (a face need not end on a pixel's edge)
                let [x0, y0, x1, y1] = face.unwrap_or([0.0, 0.0, w, h]);
                let (lx, ly) = (x0.ceil() + 0.5, y0.ceil() + 0.5);
                img.sample(veymelo_raster::Sampling::Bilinear, x.clamp(lx, (x1.floor() - 0.5).max(lx)), y.clamp(ly, (y1.floor() - 0.5).max(ly)))
            }
        }
    }
}

/// `src` (in projection `from`) seen as a `w` × `h` frame in projection `to`, its direction d
/// showing the source's `m · d`. Where a pixel spans several of the source's, it takes as many
/// samples (up to 4 × 4), so a big frame made small does not shimmer.
fn convert(src: &Image, from: Proj, to: Proj, w: usize, h: usize, m: &M3, sharp: bool) -> Image {
    let (fw, fh) = (w as f64, h as f64);
    let density = from.density(src.width as f64, src.height as f64);
    let mut out = Image::new(w, h);
    map_px(&mut out, |x, y, _| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let Some(d) = to.dir(px, py, fw, fh) else { return [0.0; 4] };
        let step = [to.dir(px + 1.0, py, fw, fh), to.dir(px, py + 1.0, fw, fh)].into_iter().flatten().map(|e| apart(d, e)).fold(0.0, f64::max);
        let n = ((step * density).ceil() as usize).clamp(1, 4);
        if n == 1 {
            return from.sample(src, turn(m, d), sharp);
        }
        let (mut sum, mut count) = ([0f32; 4], 0);
        for j in 0..n {
            for i in 0..n {
                let (sx, sy) = (px - 0.5 + (i as f64 + 0.5) / n as f64, py - 0.5 + (j as f64 + 0.5) / n as f64);
                if let Some(e) = to.dir(sx, sy, fw, fh) {
                    let p = from.sample(src, turn(m, e), false);
                    (0..4).for_each(|k| sum[k] += p[k]);
                    count += 1;
                }
            }
        }
        // the part of the pixel nothing is seen in counts as clear
        sum.map(|v| v / (n * n) as f32 * if count == 0 { 0.0 } else { 1.0 })
    });
    out
}

// ---------------------------------------------------------------- blurring on the sphere

/// A line blurred by a box `width` pixels wide going round it (its end meets its start; a
/// fractional end counts for the part it covers); as wide as the line or more, its mean.
fn round_box(line: &mut [Px], width: f64) {
    let n = line.len();
    if n < 2 || width <= 1.0 + 1e-9 {
        return;
    }
    let mut sum = vec![[0f64; 4]; n + 1];
    for (k, p) in line.iter().enumerate() {
        sum[k + 1] = std::array::from_fn(|c| sum[k][c] + p[c] as f64);
    }
    let total = sum[n];
    if width >= n as f64 {
        let mean = total.map(|v| (v / n as f64) as f32);
        line.fill(mean);
        return;
    }
    // the sum of pixels a (inclusive) to b (exclusive), round the line
    let upto = |k: i64, c: usize| (k.div_euclid(n as i64)) as f64 * total[c] + sum[k.rem_euclid(n as i64) as usize][c];
    let at = |k: i64| line[k.rem_euclid(n as i64) as usize];
    let out: Vec<Px> = (0..n as i64)
        .map(|i| {
            let (a, b) = (i as f64 + 0.5 - width / 2.0, i as f64 + 0.5 + width / 2.0);
            let (fa, fb) = (a.floor(), b.floor());
            let (pa, pb) = (at(fa as i64), at(fb as i64));
            std::array::from_fn(|c| {
                let inside = upto(fb as i64, c) - upto(fa as i64 + 1, c);
                (((fa + 1.0 - a) * pa[c] as f64 + inside + (b - fb) * pb[c] as f64) / width) as f32
            })
        })
        .collect();
    line.copy_from_slice(&out);
}

/// The width of each of three boxes that blur as a Gaussian of deviation `sigma` does.
fn box_width(sigma: f64) -> f64 {
    (4.0 * sigma * sigma + 1.0).sqrt()
}

/// An equirectangular frame blurred as a Gaussian of deviation `sigma` pixels (at the equator)
/// would blur it on the sphere: across, by more toward the poles (where a pixel is narrower on
/// the sphere) and round past the sides; down, round each whole circle through the poles (a
/// meridian and the one opposite), so it goes on over the poles.
fn sphere_blur(img: &Image, sigma: f64) -> Image {
    let (w, h) = (img.width, img.height);
    if sigma < 0.1 || w < 2 || h < 2 {
        return img.clone();
    }
    let mut out = img.clone();
    veymelo_raster::par::rows(&mut out.px, w, |y, row| {
        let lat = (0.5 - (y as f64 + 0.5) / h as f64) * PI;
        let width = box_width(sigma / lat.cos().max(1e-4));
        for _ in 0..3 {
            round_box(row, width);
        }
    });
    // the circles through the poles: meridian x down, then the one opposite back up
    let mut circles = vec![[0f32; 4]; w * 2 * h];
    for x in 0..w {
        let back = (x + w / 2) % w;
        for y in 0..h {
            circles[x * 2 * h + y] = out.px[y * w + x];
            circles[x * 2 * h + 2 * h - 1 - y] = out.px[y * w + back];
        }
    }
    let width = box_width(sigma);
    veymelo_raster::par::chunks(&mut circles, 2 * h, |_, line| {
        for _ in 0..3 {
            round_box(line, width);
        }
    });
    for x in 0..w {
        for y in 0..h {
            out.px[y * w + x] = circles[x * 2 * h + y];
        }
    }
    out
}

/// An equirectangular frame with `pad` pixels more all round, from round the sides and over the
/// poles.
fn padded(img: &Image, pad: usize) -> Image {
    let (w, h) = (img.width + 2 * pad, img.height + 2 * pad);
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            out.px[y * w + x] = texel(img, x as i64 - pad as i64, y as i64 - pad as i64);
        }
    }
    out
}

/// The middle of a padded frame (without its `pad`).
fn unpadded(img: &Image, pad: usize, w: usize, h: usize) -> Image {
    let mut out = Image::new(w, h);
    for y in 0..h {
        out.px[y * w..(y + 1) * w].copy_from_slice(&img.px[(y + pad) * img.width + pad..][..w]);
    }
    out
}

// ---------------------------------------------------------------- the effects

effect!(BLUR_DEF, "vrBlur", "VR Blur", "a blur the same size everywhere on the sphere, across the seam and over the poles", [
    frame_layout(),
    ranged("blurriness", "Blurriness", 10.0, 0.0, 1000.0),
]);
pub const BLUR: Spec = Spec::new(&BLUR_DEF, CATEGORY, blur);

fn blur(ctx: &Ctx, mut b: Buf) -> Buf {
    let sigma = ctx.num("blurriness", 10.0).max(0.0) * b.scale * 0.5;
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, _| sphere_blur(e, sigma));
    b
}

effect!(SHARPEN_DEF, "vrSharpen", "VR Sharpen", "sharpened as on the sphere, across the seam and over the poles", [
    frame_layout(),
    ranged("sharpenAmount", "Sharpen Amount", 20.0, 0.0, 100.0),
]);
pub const SHARPEN: Spec = Spec::new(&SHARPEN_DEF, CATEGORY, sharpen);

fn sharpen(ctx: &Ctx, mut b: Buf) -> Buf {
    let amount = ctx.num("sharpenAmount", 20.0) as f32 / 100.0 * 2.0;
    let sigma = (1.5 * b.scale).max(0.5);
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, _| {
        let soft = sphere_blur(e, sigma);
        let mut o = e.clone();
        for (p, q) in o.px.iter_mut().zip(&soft.px) {
            for c in 0..3 {
                p[c] = (p[c] + (p[c] - q[c]) * amount).max(0.0);
            }
        }
        o
    });
    b
}

effect!(DENOISE_DEF, "vrDeNoise", "VR De-Noise", "noise taken out as on the sphere: smoothed where flat, edges kept (random noise), or specks removed (salt and pepper)", [
    frame_layout(),
    choice("noiseType", "Noise Type", &["randomValued", "saltAndPepper"], 0),
    ranged("noiseLevel", "Noise Level", 20.0, 0.0, 100.0),
    ranged("detail", "Detail", 0.0, 0.0, 100.0).doc("how much of the original's fine detail comes back"),
]);
pub const DENOISE: Spec = Spec::new(&DENOISE_DEF, CATEGORY, denoise);

fn denoise(ctx: &Ctx, mut b: Buf) -> Buf {
    let level = (ctx.num("noiseLevel", 20.0) / 100.0).max(0.0);
    if level <= 0.0 {
        return b;
    }
    let s = b.scale;
    let specks = ctx.choice("noiseType") == 1;
    let detail = (ctx.num("detail", 0.0) / 100.0).clamp(0.0, 1.0) as f32;
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, _| {
        let (w, h) = (e.width, e.height);
        if specks {
            // a median takes out lone specks altogether
            let r = ((level * 3.0 * s).round() as usize).max(1);
            return unpadded(&crate::film::median_of(&padded(e, r), r), r, w, h);
        }
        // smoothed by the frame itself (a guided filter): flat places evened, edges kept; the
        // frame padded from round the sides and over the poles, so they are done like the rest
        let r = ((1.0 + level * 4.0) * s).max(1.0) as usize;
        let pad = 2 * r + 1;
        let big = padded(e, pad);
        let (pw, ph) = (big.width, big.height);
        let guide: Vec<f32> = big.px.iter().map(|p| luma([p[0], p[1], p[2]])).collect();
        let eps = ((level * level * 0.02) as f32).max(1e-6);
        let smooth: Vec<Vec<f32>> = (0..4).map(|c| {
            let plane: Vec<f32> = big.px.iter().map(|p| p[c]).collect();
            crate::film::guided(&guide, &plane, pw, ph, r, eps)
        }).collect();
        let mut out = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = (y + pad) * pw + x + pad;
                let o = e.px[y * w + x];
                let p: Px = std::array::from_fn(|c| smooth[c][i] + (o[c] - smooth[c][i]) * detail);
                out.px[y * w + x] = [p[0].max(0.0), p[1].max(0.0), p[2].max(0.0), p[3].clamp(0.0, 1.0)];
            }
        }
        out
    });
    b
}

effect!(GLOW_DEF, "vrGlow", "VR Glow", "the bright parts glowing, spread as on the sphere", [
    frame_layout(),
    ranged("luminanceThreshold", "Luma Threshold", 60.0, 0.0, 100.0),
    ranged("glowRadius", "Glow Radius", 20.0, 0.0, 1000.0),
    ranged("glowBrightness", "Glow Brightness", 100.0, 0.0, 1000.0),
    ranged("glowSaturation", "Glow Saturation", 100.0, 0.0, 400.0),
    flag("useTintColor", "Use Tint Color", false),
    color("tintColor", "Tint Color", 1.0, 0.8, 0.5),
]);
pub const GLOW: Spec = Spec::new(&GLOW_DEF, CATEGORY, glow);

fn glow(ctx: &Ctx, mut b: Buf) -> Buf {
    let threshold = ctx.num("luminanceThreshold", 60.0) as f32 / 100.0;
    let sigma = (ctx.num("glowRadius", 20.0).max(0.0) * b.scale * 0.5).max(0.5);
    let bright = ctx.num("glowBrightness", 100.0) as f32 / 100.0;
    let saturation = ctx.num("glowSaturation", 100.0) as f32 / 100.0;
    let tint = ctx.flag("useTintColor").then(|| {
        let c = ctx.color("tintColor");
        [c.r as f32, c.g as f32, c.b as f32]
    });
    if bright <= 0.0 {
        return b;
    }
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, _| {
        // what is brighter than the threshold, by how much
        let mut hot = e.clone();
        for p in hot.px.iter_mut() {
            let (c, a) = unpremul(*p);
            let l = luma(c);
            let k = if l > threshold { ((l - threshold) / (1.0 - threshold).max(1e-3)).min(4.0) } else { 0.0 };
            let c = c.map(|v| v * k);
            let lc = luma(c);
            let c = match tint {
                Some(t) => t.map(|v| v * lc),
                None => c.map(|v| (lc + (v - lc) * saturation).max(0.0)),
            };
            *p = [c[0] * a, c[1] * a, c[2] * a, a * k.min(1.0)];
        }
        let spread = sphere_blur(&hot, sigma);
        let mut o = e.clone();
        for (p, q) in o.px.iter_mut().zip(&spread.px) {
            for c in 0..3 {
                p[c] += q[c] * bright;
            }
            p[3] = (p[3] + q[3] * bright).min(1.0);
        }
        o
    });
    b
}

effect!(ROTATE_DEF, "vrRotateSphere", "VR Rotate Sphere", "the whole sphere turned: right by the pan, up by the tilt, about the front by the roll", [
    frame_layout(),
    flag("invertRotation", "Invert Rotation", false).doc("turned back the other way (undoes the same settings)"),
    angle("tilt", "Tilt (X Axis)", 0.0),
    angle("pan", "Pan (Y Axis)", 0.0),
    angle("roll", "Roll (Z Axis)", 0.0),
]);
pub const ROTATE: Spec = Spec::new(&ROTATE_DEF, CATEGORY, rotate);

fn rotate(ctx: &Ctx, mut b: Buf) -> Buf {
    if ["tilt", "pan", "roll"].iter().all(|k| ctx.num(k, 0.0) == 0.0) {
        return b;
    }
    let m = view_of(ctx);
    let m = if ctx.flag("invertRotation") { turned_back(&m) } else { m };
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, _| convert(e, Proj::EQUI, Proj::EQUI, e.width, e.height, &m, true));
    b
}

effect!(CONVERTER_DEF, "vrConverter", "VR Converter", "a frame converted between equirectangular, cube maps, sphere maps, a dome and a flat view, turned on the way", [
    frame_layout(),
    choice("sourceProjection", "Source Projection", PROJECTIONS, 0),
    ranged("sourceHorizontalFov", "Source Horizontal FOV", 90.0, 1.0, 360.0).doc("for a sphere map or a flat view"),
    choice("targetProjection", "Target Projection", PROJECTIONS, 2),
    ranged("targetHorizontalFov", "Target Horizontal FOV", 90.0, 1.0, 360.0),
    angle("tilt", "Tilt (X Axis)", 0.0),
    angle("pan", "Pan (Y Axis)", 0.0),
    angle("roll", "Roll (Z Axis)", 0.0),
]);
pub const CONVERTER: Spec = Spec::new(&CONVERTER_DEF, CATEGORY, converter);

fn converter(ctx: &Ctx, mut b: Buf) -> Buf {
    let from = Proj { kind: ctx.choice("sourceProjection"), fov: ctx.num("sourceHorizontalFov", 90.0) };
    let to = Proj { kind: ctx.choice("targetProjection"), fov: ctx.num("targetHorizontalFov", 90.0) };
    let still = ["tilt", "pan", "roll"].iter().all(|k| ctx.num(k, 0.0) == 0.0);
    if from.kind == to.kind && still && (!matches!(from.kind, 4 | 5) || from.fov == to.fov) {
        return b;
    }
    let m = view_of(ctx);
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, _| convert(e, from, to, e.width, e.height, &m, true));
    b
}

effect!(TO_PLANE_DEF, "vrSphereToPlane", "VR Sphere to Plane", "a flat view out of the sphere, looking where the pan and tilt turn it", [
    frame_layout(),
    ranged("horizontalFov", "Horizontal FOV", 90.0, 1.0, 179.0),
    angle("tilt", "Tilt (X Axis)", 0.0),
    angle("pan", "Pan (Y Axis)", 0.0),
    angle("roll", "Roll (Z Axis)", 0.0),
]);
pub const TO_PLANE: Spec = Spec::new(&TO_PLANE_DEF, CATEGORY, to_plane);

fn to_plane(ctx: &Ctx, mut b: Buf) -> Buf {
    let flat = Proj { kind: 5, fov: ctx.num("horizontalFov", 90.0) };
    let m = view_of(ctx);
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, _| convert(e, Proj::EQUI, flat, e.width, e.height, &m, true));
    b
}

effect!(TO_SPHERE_DEF, "vrPlaneToSphere", "VR Plane to Sphere", "the layer laid flat on the sphere, so many degrees wide, where the pan and tilt put it", [
    frame_layout(),
    ranged("scale", "Scale (Degrees)", 90.0, 1.0, 179.0),
    ranged("feather", "Feather", 0.0, 0.0, 100.0),
    angle("tilt", "Tilt (X Axis)", 0.0),
    angle("pan", "Pan (Y Axis)", 0.0),
    angle("roll", "Roll (Z Axis)", 0.0),
]);
pub const TO_SPHERE: Spec = Spec::new(&TO_SPHERE_DEF, CATEGORY, to_sphere);

fn to_sphere(ctx: &Ctx, mut b: Buf) -> Buf {
    let fov = ctx.num("scale", 90.0).clamp(1.0, 179.0);
    let flat = Proj { kind: 5, fov };
    let back = turned_back(&view_of(ctx));
    let feather = (ctx.num("feather", 0.0) / 100.0).clamp(0.0, 1.0);
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, _| {
        let (w, h) = (e.width as f64, e.height as f64);
        let mut out = convert(e, flat, Proj::EQUI, e.width, e.height, &back, false);
        if feather > 0.0 {
            // softened toward its edges, a part of its half-width in
            let t = (fov / 2.0).to_radians().tan();
            map_px(&mut out, |x, y, p| {
                let d = turn(&back, dir(x as f64 + 0.5, y as f64 + 0.5, w, h));
                if d[2] <= 0.0 {
                    return p;
                }
                let (u, v) = ((d[0] / d[2] / t).abs(), (d[1] / d[2] / (t * h / w)).abs());
                let k = ((1.0 - u.max(v)) / feather).clamp(0.0, 1.0) as f32;
                p.map(|c| c * k)
            });
        }
        out
    });
    b
}

effect!(CHROMATIC_DEF, "vrChromaticAberrations", "VR Chromatic Aberrations", "red, green and blue drawn toward or away from a point on the sphere by their own amounts, more the farther from it", [
    frame_layout(),
    angle("centerTilt", "Center Tilt", 0.0),
    angle("centerPan", "Center Pan", 0.0),
    ranged("aberrationRed", "Aberration (Red)", 10.0, -100.0, 100.0),
    ranged("aberrationGreen", "Aberration (Green)", 0.0, -100.0, 100.0),
    ranged("aberrationBlue", "Aberration (Blue)", -10.0, -100.0, 100.0),
    ranged("falloff", "Falloff Distance", 50.0, 0.0, 100.0),
    flag("falloffInvert", "Falloff Invert", false).doc("the most near the point instead"),
]);
pub const CHROMATIC: Spec = Spec::new(&CHROMATIC_DEF, CATEGORY, chromatic);

fn chromatic(ctx: &Ctx, mut b: Buf) -> Buf {
    let k = ["aberrationRed", "aberrationGreen", "aberrationBlue"].map(|id| ctx.num(id, 0.0) / 100.0 * 0.1);
    if k.iter().all(|v| *v == 0.0) {
        return b;
    }
    let centre = turn(&rotation(ctx.num("centerTilt", 0.0), ctx.num("centerPan", 0.0), 0.0), [0.0, 0.0, 1.0]);
    let falloff = (ctx.num("falloff", 50.0) / 100.0).clamp(0.0, 1.0);
    let invert = ctx.flag("falloffInvert");
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, _| {
        let (w, h) = (e.width as f64, e.height as f64);
        let mut o = e.clone();
        map_px(&mut o, |x, y, p| {
            let d = dir(x as f64 + 0.5, y as f64 + 0.5, w, h);
            let th = apart(d, centre);
            // the way from the point to here, along the sphere
            let across = [d[0] - centre[0] * th.cos(), d[1] - centre[1] * th.cos(), d[2] - centre[2] * th.cos()];
            if dot(across, across) < 1e-18 {
                return p;
            }
            let across = unit(across);
            let far = (th / PI).powf(1.0 + falloff * 3.0);
            let far = if invert { 1.0 - far } else { far };
            let mut out = [0f32; 4];
            for c in 0..3 {
                let t = th * (1.0 - k[c] * far);
                let nd: V3 = std::array::from_fn(|i| centre[i] * t.cos() + across[i] * t.sin());
                let (sx, sy) = spot(nd, w, h);
                let s = sample(e, sx, sy);
                out[c] = s[c];
                out[3] = out[3].max(s[3]);
            }
            out
        });
        o
    });
    b
}

effect!(GRADIENTS_DEF, "vrColorGradients", "VR Color Gradients", "colours at up to five points on the sphere, blended between them by their distance round it", [
    frame_layout(),
    flag("enablePoint1", "Enable Point 1", true),
    point("point1", "Point 1"),
    color("color1", "Color 1", 1.0, 0.0, 0.0),
    flag("enablePoint2", "Enable Point 2", true),
    point("point2", "Point 2"),
    color("color2", "Color 2", 1.0, 1.0, 0.0),
    flag("enablePoint3", "Enable Point 3", true),
    point("point3", "Point 3"),
    color("color3", "Color 3", 0.0, 0.0, 1.0),
    flag("enablePoint4", "Enable Point 4", true),
    point("point4", "Point 4"),
    color("color4", "Color 4", 0.0, 1.0, 0.0),
    flag("enablePoint5", "Enable Point 5", false),
    point("point5", "Point 5"),
    color("color5", "Color 5", 1.0, 0.0, 1.0),
    ranged("blend", "Gradient Power", 2.0, 0.1, 10.0).doc("higher: each colour holds farther before giving way"),
    ranged("opacity", "Opacity", 100.0, 0.0, 100.0),
    choice("blendingMode", "Blending Mode", &MODES, 0),
]);
pub const GRADIENTS: Spec = Spec::new(&GRADIENTS_DEF, CATEGORY, gradients).points(&[("point1", [0.25, 0.5]), ("point2", [0.5, 0.25]), ("point3", [0.75, 0.5]), ("point4", [0.5, 0.75]), ("point5", [0.0, 0.5])]);

fn gradients(ctx: &Ctx, mut b: Buf) -> Buf {
    let r = ctx.bounds;
    let points: Vec<(V3, [f32; 3])> = (1..=5)
        .filter(|i| ctx.flag(&format!("enablePoint{i}")))
        .map(|i| {
            let p = ctx.point(&format!("point{i}"));
            let c = ctx.color(&format!("color{i}"));
            (dir(p.x - r.x0, p.y - r.y0, r.width().max(1.0), r.height().max(1.0)), [c.r as f32, c.g as f32, c.b as f32])
        })
        .collect();
    if points.is_empty() {
        return b;
    }
    let power = ctx.num("blend", 2.0).max(0.1);
    let opacity = (ctx.num("opacity", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let mode = ctx.choice("blendingMode");
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, _| {
        let (w, h) = (e.width as f64, e.height as f64);
        let mut o = e.clone();
        map_px(&mut o, |x, y, p| {
            let d = dir(x as f64 + 0.5, y as f64 + 0.5, w, h);
            // weighed by how near each point is round the sphere
            let (mut sum, mut total) = ([0f64; 3], 0f64);
            for (q, c) in &points {
                let a = apart(d, *q);
                if a < 1e-6 {
                    return put_with(p, premul(*c, opacity * if mode == 0 { p[3] } else { 1.0 }), mode);
                }
                let k = 1.0 / a.powf(power);
                (0..3).for_each(|i| sum[i] += c[i] as f64 * k);
                total += k;
            }
            let c = sum.map(|v| (v / total) as f32);
            put_with(p, premul(c, opacity * if mode == 0 { p[3] } else { 1.0 }), mode)
        });
        o
    });
    b
}

effect!(FRACTAL_DEF, "vrFractalNoise", "VR Fractal Noise", "fractal noise on the sphere: no seam and no pinching at the poles", [
    frame_layout(),
    choice("fractalType", "Fractal Type", &["basic", "turbulentSharp", "max", "strings"], 0),
    ranged("contrast", "Contrast", 100.0, 0.0, 10_000.0),
    ranged("brightness", "Brightness", 0.0, -10_000.0, 10_000.0),
    flag("invert", "Invert", false),
    ranged("complexity", "Complexity", 6.0, 1.0, 20.0),
    angle("evolution", "Evolution", 0.0),
    ranged("scale", "Scale", 100.0, 1.0, 10_000.0),
    angle("tilt", "Tilt (X Axis)", 0.0),
    angle("pan", "Pan (Y Axis)", 0.0),
    angle("roll", "Roll (Z Axis)", 0.0),
    ranged("subInfluence", "Sub Influence", 50.0, 0.0, 100.0),
    ranged("subScaling", "Sub Scaling", 50.0, 10.0, 100.0),
    ranged("randomSeed", "Random Seed", 0.0, 0.0, 10_000.0),
    ranged("opacity", "Opacity", 100.0, 0.0, 100.0),
    choice("blendingMode", "Blending Mode", &MODES, 0),
]);
pub const FRACTAL: Spec = Spec::new(&FRACTAL_DEF, CATEGORY, fractal);

/// Smooth noise at point `q` that changes in place as `evolution` (turns) goes on: between two
/// unrelated fields, kept as strong in between.
fn evolving(q: V3, evolution: f64, seed: u64) -> f64 {
    let (k, t) = (evolution.floor(), evolution - evolution.floor());
    let t = t * t * (3.0 - 2.0 * t);
    let field = |n: f64| {
        let o = [n * 31.7, n * 17.3, n * 23.9];
        crate::noise::smooth(q[0] + o[0], q[1] + o[1], q[2] + o[2], seed, None)
    };
    (field(k) * (1.0 - t) + field(k + 1.0) * t) / ((1.0 - t) * (1.0 - t) + t * t).sqrt()
}

fn fractal(ctx: &Ctx, mut b: Buf) -> Buf {
    let kind = ctx.choice("fractalType");
    let (contrast, brightness) = (ctx.num("contrast", 100.0) / 100.0, ctx.num("brightness", 0.0) / 100.0);
    let invert = ctx.flag("invert");
    let scale = (ctx.num("scale", 100.0) / 100.0).max(0.01);
    let m = view_of(ctx);
    let influence = (ctx.num("subInfluence", 50.0) / 100.0).clamp(0.0, 1.0);
    let sub = (ctx.num("subScaling", 50.0) / 100.0).clamp(0.1, 1.0);
    let layers = ctx.num("complexity", 6.0).clamp(1.0, 20.0);
    let evolution = ctx.num("evolution", 0.0) / 360.0;
    let opacity = (ctx.num("opacity", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let mode = ctx.choice("blendingMode");
    let seed = crate::noise::mix(&[ctx.num("randomSeed", 0.0).max(0.0) as u64, 0x5e3f]);
    // at 100%, four noise cells to the sphere's radius
    let base = 4.0 / scale;
    if let Some(img) = fractal_on_gpu(&b.img, ctx.choice("frameLayout"), &m, kind, contrast, brightness, invert, influence, sub, layers, evolution, opacity, mode, seed, base) {
        b.img = img;
        return b;
    }
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, _| {
        let (w, h) = (e.width as f64, e.height as f64);
        let mut o = e.clone();
        map_px(&mut o, |x, y, p| {
            let d = turn(&m, dir(x as f64 + 0.5, y as f64 + 0.5, w, h));
            let count = layers.ceil() as usize;
            let (mut sum, mut weights, mut most, mut strength, mut fine) = (0f64, 0f64, 0f64, 1f64, base);
            for i in 0..count {
                // a part layer counts by its part
                let part = if i + 1 == count && layers.fract() > 0.0 { layers.fract() } else { 1.0 };
                // finer layers evolve faster, as they would on a flat picture
                let n = evolving([d[0] * fine, d[1] * fine, d[2] * fine], evolution * (1.0 + i as f64 * 0.5), seed ^ i as u64);
                let s = n.clamp(-1.0, 1.0);
                let v = match kind {
                    1 => (1.0 - s.abs()).powi(2),
                    3 => 1.0 - (s.abs() / 0.12).clamp(0.0, 1.0).powi(2),
                    _ => s * 0.5 + 0.5,
                };
                most = most.max(v * strength);
                sum += v * strength * part;
                weights += strength * part;
                strength *= influence;
                fine /= sub;
            }
            let v = if kind == 2 { most } else { sum / weights.max(1e-9) };
            let mut v = (v - 0.5) * contrast + 0.5 + brightness;
            if invert {
                v = 1.0 - v;
            }
            let v = v.clamp(0.0, 1.0) as f32;
            // alone it fills the layer's shape; blended it covers all of the layer
            put_with(p, premul([v; 3], opacity * if mode == 0 { p[3] } else { 1.0 }), mode)
        });
        o
    });
    b
}

/// A layer of VR Fractal Noise as the GPU takes it (sphere.metal's VrLayer): its seed and size,
/// where the two fields it evolves between are, how far between, and its strength.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct GpuVrLayer {
    seed: u64,
    fine: [f32; 2],
    at_k: [[f32; 2]; 3],
    at_next: [[f32; 2]; 3],
    t: f32,
    before: f32,
    inv_den: f32,
    strength: f32,
    part: f32,
    pad: f32,
}

/// The most layers the GPU takes.
const GPU_LAYERS: usize = 20;

/// VR Fractal Noise's settings as the GPU takes them (sphere.metal's Vr).
#[repr(C)]
#[derive(Clone, Copy)]
struct GpuVr {
    w: u32,
    h: u32,
    eyes: u32,
    kind: u32,
    mode: u32,
    invert: u32,
    count: u32,
    pad: u32,
    eye: [[u32; 4]; 2],
    at: [u32; 4],
    contrast: f32,
    brightness: f32,
    opacity: f32,
    weights: f32,
    m: [[f32; 2]; 9],
    layer: [GpuVrLayer; GPU_LAYERS],
    // (as long as the GPU's, which rounds it to sixteen bytes)
    tail: [u32; 2],
}

unsafe impl veymelo_raster::gpu::Plain for GpuVr {}

/// VR Fractal Noise on the GPU (None: leave it to the processor). Each eye's columns' longitudes
/// and rows' latitudes (their sines and cosines) and each layer's settings are worked out here,
/// as [`fractal`] works them out.
#[allow(clippy::too_many_arguments)]
fn fractal_on_gpu(img: &Image, layout: u32, m: &M3, kind: u32, contrast: f64, brightness: f64, invert: bool, influence: f64, sub: f64, layers: f64, evolution: f64, opacity: f32, mode: u32, seed: u64, base: f64) -> Option<Image> {
    use crate::noise::split;
    use veymelo_raster::gpu;
    let count = layers.ceil() as usize;
    if !gpu::wanted(img.px.len()) || count > GPU_LAYERS {
        return None;
    }
    let rects = eyes(img.width, img.height, layout);
    let mut trig: Vec<[f32; 4]> = Vec::new();
    let mut eye = [[0u32; 4]; 2];
    let mut at = [0u32; 4];
    for (i, &[x0, y0, ew, eh]) in rects.iter().enumerate() {
        eye[i] = [x0 as u32, y0 as u32, ew as u32, eh as u32];
        at[i * 2] = trig.len() as u32;
        for x in 0..ew {
            let (s, c) = (((x as f64 + 0.5) / ew as f64 - 0.5) * TAU).sin_cos();
            let (s, c) = (split(s), split(c));
            trig.push([s[0], s[1], c[0], c[1]]);
        }
        at[i * 2 + 1] = trig.len() as u32;
        for y in 0..eh {
            let (s, c) = ((0.5 - (y as f64 + 0.5) / eh as f64) * PI).sin_cos();
            let (s, c) = (split(s), split(c));
            trig.push([s[0], s[1], c[0], c[1]]);
        }
    }
    // each layer as fractal() works it out
    let mut layer = [GpuVrLayer::default(); GPU_LAYERS];
    let (mut weights, mut strength, mut fine) = (0f64, 1f64, base);
    for (i, l) in layer.iter_mut().enumerate().take(count) {
        let part = if i + 1 == count && layers.fract() > 0.0 { layers.fract() } else { 1.0 };
        let e = evolution * (1.0 + i as f64 * 0.5);
        let (k, t) = (e.floor(), e - e.floor());
        let t = t * t * (3.0 - 2.0 * t);
        let at = |n: f64| [split(n * 31.7), split(n * 17.3), split(n * 23.9)];
        *l = GpuVrLayer {
            seed: seed ^ i as u64,
            fine: split(fine),
            at_k: at(k),
            at_next: at(k + 1.0),
            t: t as f32,
            before: (1.0 - t) as f32,
            inv_den: (1.0 / ((1.0 - t) * (1.0 - t) + t * t).sqrt()) as f32,
            strength: strength as f32,
            part: part as f32,
            pad: 0.0,
        };
        weights += strength * part;
        strength *= influence;
        fine /= sub;
    }
    let p = GpuVr {
        w: img.width as u32,
        h: img.height as u32,
        eyes: rects.len() as u32,
        kind,
        mode: if mode == 0 { 0 } else { crate::generate::MODE_OF[(mode as usize).min(19)] as u32 + 1 },
        invert: invert as u32,
        count: count as u32,
        pad: 0,
        eye,
        at,
        contrast: contrast as f32,
        brightness: brightness as f32,
        opacity,
        weights: weights as f32,
        m: std::array::from_fn(|k| split(m[k / 3][k % 3])),
        layer,
        tail: [0; 2],
    };
    let kernel = gpu::Kernel { source: gpu::program(&[gpu::COMMON, include_str!("noise.metal"), include_str!("sphere.metal")]), name: "vr_fractal" };
    gpu::apply(img, &[gpu::Op::new(kernel, &p, [img.width, img.height]).with(&trig)])
}

effect!(GLITCH_DEF, "vrDigitalGlitch", "VR Digital Glitch", "bands of the sphere jumping sideways round it, their colours split, scan lines over them; changing so many times a second, everywhere or round one point", [
    frame_layout(),
    ranged("masterAmplitude", "Master Amplitude", 100.0, 0.0, 500.0),
    ranged("distortionRate", "Distortion Rate", 5.0, 0.0, 60.0).doc("new patterns a second"),
    ranged("distortionComplexity", "Distortion Complexity", 16.0, 1.0, 100.0).doc("how many bands"),
    angle("distortionEvolution", "Distortion Evolution", 0.0),
    flag("cycleEvolution", "Cycle Evolution", false),
    ranged("cycle", "Cycle (in Revolutions)", 1.0, 1.0, 1000.0),
    ranged("geometricDistortion", "Geometric Distortion", 50.0, 0.0, 100.0),
    ranged("horizontalDisplacement", "Horizontal Displacement", 100.0, 0.0, 400.0),
    ranged("verticalDisplacement", "Vertical Displacement", 0.0, 0.0, 400.0),
    ranged("colorDistortion", "Color Distortion", 50.0, 0.0, 100.0),
    ranged("redOffset", "Red Offset", -1.0, -10.0, 10.0),
    ranged("greenOffset", "Green Offset", 0.0, -10.0, 10.0),
    ranged("blueOffset", "Blue Offset", 1.0, -10.0, 10.0),
    ranged("scanlines", "Scanlines", 20.0, 0.0, 100.0),
    ranged("scanlineSpacing", "Scanline Spacing", 2.0, 2.0, 100.0),
    angle("targetTilt", "Target Tilt", 0.0),
    angle("targetPan", "Target Pan", 0.0),
    ranged("targetRadius", "Target Radius", 0.0, 0.0, 180.0).doc("degrees round the target (0: everywhere)"),
    ranged("targetFeather", "Target Feather", 0.0, 0.0, 180.0),
    ranged("randomSeed", "Random Seed", 0.0, 0.0, 10_000.0),
]);
pub const GLITCH: Spec = Spec::new(&GLITCH_DEF, CATEGORY, glitch);

fn glitch(ctx: &Ctx, mut b: Buf) -> Buf {
    let amp = (ctx.num("masterAmplitude", 100.0) / 100.0).max(0.0);
    if amp <= 0.0 {
        return b;
    }
    let rate = ctx.num("distortionRate", 5.0).max(0.0);
    let geo = ctx.num("geometricDistortion", 50.0) / 100.0 * amp;
    let (across, down) = (ctx.num("horizontalDisplacement", 100.0) / 100.0, ctx.num("verticalDisplacement", 0.0) / 100.0);
    let bands = ctx.num("distortionComplexity", 16.0).clamp(1.0, 100.0).round() as u64;
    let split = ctx.num("colorDistortion", 50.0) / 100.0 * amp;
    let offsets = ["redOffset", "greenOffset", "blueOffset"].map(|k| ctx.num(k, 0.0));
    let scan = (ctx.num("scanlines", 20.0) / 100.0 * amp) as f32;
    let spacing = (ctx.num("scanlineSpacing", 2.0).round().max(2.0) * b.scale).round().max(2.0) as usize;
    let seed = crate::noise::mix(&[ctx.seed, ctx.num("randomSeed", 0.0).max(0.0) as u64, 0x611c]);
    let mut evolution = ctx.num("distortionEvolution", 0.0) / 360.0;
    if ctx.flag("cycleEvolution") {
        evolution = evolution.rem_euclid(ctx.num("cycle", 1.0).round().max(1.0));
    }
    // the pattern now: a new one each 1/rate seconds, evolution moving smoothly between them
    let when = if rate > 0.0 { ctx.time.seconds() * rate } else { 0.0 } + evolution;
    let (slot, part) = (when.floor() as i64, when - when.floor());
    let target = turn(&rotation(ctx.num("targetTilt", 0.0), ctx.num("targetPan", 0.0), 0.0), [0.0, 0.0, 1.0]);
    let (radius, feather) = (ctx.num("targetRadius", 0.0).to_radians(), ctx.num("targetFeather", 0.0).to_radians());
    b.img = per_eye(&b.img, ctx.choice("frameLayout"), |e, eye| {
        let (w, h) = (e.width as f64, e.height as f64);
        let mut o = e.clone();
        map_px(&mut o, |x, y, p| {
            let band = ((y as f64 / h) * bands as f64) as u64;
            // a band's jump and split in pattern `s`: a third of the bands move
            let pattern = |s: i64| {
                let r = |k: u64| crate::sprites::rand(band, k ^ (s as u64).wrapping_mul(0x9e37), seed ^ eye as u64) as f64;
                if r(1) < 0.35 { ((r(2) * 2.0 - 1.0) * geo, split * w * 0.01 * (1.0 + r(3))) } else { (0.0, 0.0) }
            };
            let (j0, s0) = pattern(slot);
            let (j1, s1) = pattern(slot + 1);
            let (jump, apart_by) = (j0 + (j1 - j0) * part, s0 + (s1 - s0) * part);
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            let k = if radius > 0.0 {
                let a = apart(dir(fx, fy, w, h), target);
                if a <= radius { 1.0 } else if feather > 0.0 { (1.0 - (a - radius) / feather).clamp(0.0, 1.0) } else { 0.0 }
            } else {
                1.0
            };
            if k <= 0.0 {
                return p;
            }
            let (dx, dy) = (jump * w * 0.15 * across * k, jump * h * 0.15 * down * k);
            let (sx, sy) = (fx - dx, (fy - dy).clamp(0.5, h - 0.5));
            // jumps go round past the sides (the frame is a whole turn)
            let c: [Px; 3] = std::array::from_fn(|i| sample(e, sx + offsets[i] * apart_by * k, sy));
            let dark = if y % spacing == 0 { 1.0 - scan * 0.5 * k as f32 } else { 1.0 };
            [c[0][0] * dark, c[1][1] * dark, c[2][2] * dark, c[0][3].max(c[1][3]).max(c[2][3])]
        });
        o
    });
    b
}

pub const ALL: [Spec; 12] = [BLUR, CHROMATIC, GRADIENTS, CONVERTER, DENOISE, GLITCH, FRACTAL, GLOW, TO_SPHERE, ROTATE, SHARPEN, TO_PLANE];

#[cfg(test)]
mod tests;
