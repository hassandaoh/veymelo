//! A small 3D scene for effects that fold, flip and break the layer: flat textured pieces
//! (cards, shards, polygons) placed in 3D, seen through a camera (set by its position and turn,
//! or by where the layer's corners land), lit by a light and their material, drawn far to near.
//!
//! Points are in the picture's pixels, z away from the viewer (the layer's plane is z = 0).

use veymelo_geom::{Mat3, vec2};
use veymelo_raster::{Image, Px, Sampling, par};

use crate::util::unpremul;
use crate::{Buf, Ctx};

/// A 3 × 3 matrix, rows first.
pub type M3 = [[f64; 3]; 3];

pub const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn mul(a: &M3, b: &M3) -> M3 {
    [0, 1, 2].map(|i| [0, 1, 2].map(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

pub fn apply(m: &M3, p: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2])
}

pub fn invert(m: &M3) -> Option<M3> {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let k = 1.0 / det;
    Some([[(e * i - f * h) * k, (c * h - b * i) * k, (b * f - c * e) * k], [(f * g - d * i) * k, (a * i - c * g) * k, (c * d - a * f) * k], [(d * h - e * g) * k, (b * g - a * h) * k, (a * e - b * d) * k]])
}

fn rx(a: f64) -> M3 {
    let (s, c) = a.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}

fn ry(a: f64) -> M3 {
    let (s, c) = a.sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn rz(a: f64) -> M3 {
    let (s, c) = a.sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// Turning about x, y and z (radians), in `order` (0 XYZ, 1 XZY, 2 YXZ, 3 YZX, 4 ZXY, 5 ZYX;
/// the first done first).
pub fn turning(x: f64, y: f64, z: f64, order: u32) -> M3 {
    let (a, b, c) = (rx(x), ry(y), rz(z));
    let [p, q, r] = match order % 6 {
        1 => [a, c, b],
        2 => [b, a, c],
        3 => [b, c, a],
        4 => [c, a, b],
        5 => [c, b, a],
        _ => [a, b, c],
    };
    mul(&r, &mul(&q, &p))
}

/// Turning by `angle` (radians) about `axis`.
pub fn about(axis: [f64; 3], angle: f64) -> M3 {
    let l = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if l < 1e-12 || angle == 0.0 {
        return IDENTITY;
    }
    let (x, y, z) = (axis[0] / l, axis[1] / l, axis[2] / l);
    let (s, c) = angle.sin_cos();
    let t = 1.0 - c;
    [[t * x * x + c, t * x * y - s * z, t * x * z + s * y], [t * x * y + s * z, t * y * y + c, t * y * z - s * x], [t * x * z - s * y, t * y * z + s * x, t * z * z + c]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    if l < 1e-12 { [0.0, 0.0, -1.0] } else { a.map(|v| v / l) }
}

/// A camera: from scene points to the picture's pixels (homogeneous rows x, y, w; w is the
/// depth in front of it).
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub m: [[f64; 4]; 3],
    /// Points nearer than this (in w) are not drawn.
    pub near: f64,
}

impl Camera {
    /// Looking straight at the layer's plane from `d` pixels in front of pixel `c` (the plane
    /// itself seen as it is).
    pub fn plain(c: [f64; 2], d: f64) -> Camera {
        Camera { m: [[d, 0.0, c[0], 0.0], [0.0, d, c[1], 0.0], [0.0, 0.0, 1.0, d]], near: 0.05 * d }
    }

    /// The same camera on a scene moved first by p ↦ a·p + t.
    fn after(&self, a: &M3, t: [f64; 3]) -> Camera {
        let mut m = [[0.0; 4]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().take(3).enumerate() {
                *v = (0..3).map(|k| self.m[i][k] * a[k][j]).sum();
            }
            row[3] = (0..3).map(|k| self.m[i][k] * t[k]).sum::<f64>() + self.m[i][3];
        }
        Camera { m, near: self.near }
    }

    /// The same camera with the picture then bent by `h` (keeping w's scale).
    fn then(&self, h: &M3) -> Camera {
        let mut m = [[0.0; 4]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| h[i][k] * self.m[k][j]).sum();
            }
        }
        Camera { m, near: self.near * h[2][2].abs().max(1e-9) }
    }

    /// Where a scene point lands (none behind the camera).
    pub fn project(&self, p: [f64; 3]) -> Option<[f64; 2]> {
        let r = |i: usize| self.m[i][0] * p[0] + self.m[i][1] * p[1] + self.m[i][2] * p[2] + self.m[i][3];
        let w = r(2);
        (w > self.near).then(|| [r(0) / w, r(1) / w])
    }

    /// The composition's camera (`sc`, in the layer's units) on `b`'s pixels.
    pub fn through(sc: &crate::CompScene, b: &Buf) -> Camera {
        let (s, o) = (b.scale, b.origin);
        // from the buffer's pixels (and depth) to the layer's units, through the camera, and back
        let into = [[1.0 / s, 0.0, 0.0, o.x], [0.0, 1.0 / s, 0.0, o.y], [0.0, 0.0, 1.0 / s, 0.0], [0.0, 0.0, 0.0, 1.0]];
        let out = [[s, 0.0, -s * o.x], [0.0, s, -s * o.y], [0.0, 0.0, 1.0]];
        let seen: [[f64; 4]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| sc.camera[i][k] * into[k][j]).sum()));
        let m = std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| out[i][k] * seen[k][j]).sum()));
        Camera { m, near: 1.0 }
    }

    /// Where the camera is in the scene.
    pub fn eye(&self) -> [f64; 3] {
        let a: M3 = [0, 1, 2].map(|i| [self.m[i][0], self.m[i][1], self.m[i][2]]);
        match invert(&a) {
            Some(ai) => apply(&ai, [-self.m[0][3], -self.m[1][3], -self.m[2][3]]),
            None => [0.0, 0.0, -1e9],
        }
    }

    /// The camera the effect's Camera System settings describe, looking at `b`'s picture.
    pub fn of(ctx: &Ctx, b: &Buf) -> Camera {
        let r = ctx.bounds;
        let s = b.scale;
        let mid = b.pixel(r.center());
        let c = [mid.x, mid.y];
        let size = r.width().max(r.height()) * s;
        let px = |id: &str| {
            let p = b.pixel(ctx.point(id));
            [p.x, p.y]
        };
        match ctx.choice("cameraSystem") {
            1 => {
                // the layer's corners on the four pins; the focal length sets how strongly what
                // leaves the plane is foreshortened
                let focal = if ctx.flag("autoFocalLength") { 70.0 } else { ctx.num("cornerFocalLength", 70.0).max(1.0) };
                let base = Camera::plain(c, size * 2.0 * focal / 70.0);
                let v = |p: [f64; 2]| vec2(p[0], p[1]);
                let corner = |x: f64, y: f64| {
                    let p = b.pixel(vec2(if x == 0.0 { r.x0 } else { r.x1 }, if y == 0.0 { r.y0 } else { r.y1 }));
                    vec2(p.x, p.y)
                };
                let pins = [v(px("upperLeftCorner")), v(px("upperRightCorner")), v(px("lowerRightCorner")), v(px("lowerLeftCorner"))];
                match Mat3::quad_to_quad([corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0), corner(0.0, 1.0)], pins) {
                    Some(h) => base.then(&h.0),
                    None => base,
                }
            }
            2 => match ctx.host.and_then(|h| h.scene()) {
                Some(sc) => Camera::through(&sc, b),
                None => {
                    ctx.note("no composition camera to look through here: seen straight on".into());
                    Camera::plain(c, size * 2.0)
                }
            },
            _ => {
                let z = ctx.num("zPosition", 2.0).max(0.05);
                let focal = ctx.num("focalLength", 70.0).max(1.0);
                let base = Camera::plain(c, size * z * focal / 140.0);
                let order = ctx.choice("transformOrder");
                let t = turning(ctx.num("xRotation", 0.0).to_radians(), ctx.num("yRotation", 0.0).to_radians(), ctx.num("zRotation", 0.0).to_radians(), order);
                let at = px("xyPosition");
                let shift = [at[0] - c[0], at[1] - c[1], 0.0];
                // turned about the layer's middle, and moved by the X, Y position (before or after)
                let c3 = [c[0], c[1], 0.0];
                let tc = apply(&t, c3);
                let base_t = [c3[0] - tc[0], c3[1] - tc[1], c3[2] - tc[2]];
                let tr = if order >= 6 {
                    let ts = apply(&t, shift);
                    [base_t[0] + ts[0], base_t[1] + ts[1], base_t[2] + ts[2]]
                } else {
                    [base_t[0] + shift[0], base_t[1] + shift[1], base_t[2]]
                };
                base.after(&t, tr)
            }
        }
    }
}

/// The light and material the effect's settings describe.
#[derive(Clone, Copy, Debug)]
pub struct Light {
    /// 0 a distant source, 1 a point source, 2 none (ambient only).
    kind: u8,
    color: [f32; 3],
    at: [f64; 3],
    toward: [f64; 3],
    ambient: f32,
    diffuse: f32,
    specular: f32,
    sharpness: f32,
}

impl Light {
    pub fn of(ctx: &Ctx, b: &Buf) -> Light {
        let r = ctx.bounds;
        let s = b.scale;
        let mid = b.pixel(r.center());
        let intensity = ctx.num("lightIntensity", 1.0) as f32;
        let lc = ctx.color("lightColor");
        let p = b.pixel(ctx.point("lightPosition"));
        let at = [p.x, p.y, -ctx.num("lightDepth", 1.0) * r.width().max(r.height()) * s];
        let mut kind = if ctx.choice("lightType") == 1 { 1 } else { 0 };
        let mut color = [lc.r as f32 * intensity, lc.g as f32 * intensity, lc.b as f32 * intensity];
        let (mut at, mut toward) = (at, unit([at[0] - mid.x, at[1] - mid.y, at[2]]));
        if ctx.choice("lightType") == 2 {
            match ctx.host.and_then(|h| h.scene()).and_then(|sc| sc.light) {
                Some(l) => {
                    // the composition's first light, on the buffer's pixels
                    color = l.color;
                    at = [(l.at[0] - b.origin.x) * s, (l.at[1] - b.origin.y) * s, l.at[2] * s];
                    (kind, toward) = match l.kind {
                        0 => (0, [-l.dir[0], -l.dir[1], -l.dir[2]]),
                        3 => (2, toward),
                        _ => (1, toward),
                    };
                }
                None => {
                    ctx.note("the composition has no light: ambient light only".into());
                    kind = 2;
                }
            }
        }
        Light {
            kind,
            color,
            at,
            toward,
            ambient: ctx.num("ambientLight", 0.25) as f32,
            diffuse: ctx.num("diffuseReflection", 0.75) as f32,
            specular: ctx.num("specularReflection", 0.0) as f32,
            sharpness: ctx.num("highlightSharpness", 15.0).max(1.0) as f32,
        }
    }

    /// What a surface facing `n` at `at`, seen from `eye`, is multiplied by and has added (lit
    /// on whichever side faces the viewer).
    pub fn shade(&self, n: [f64; 3], at: [f64; 3], eye: [f64; 3]) -> ([f32; 3], [f32; 3]) {
        let v = unit([eye[0] - at[0], eye[1] - at[1], eye[2] - at[2]]);
        let n = if dot(n, v) < 0.0 { n.map(|c| -c) } else { n };
        if self.kind == 2 {
            return (self.color.map(|c| self.ambient + self.diffuse * c), [0.0; 3]);
        }
        let l = if self.kind == 1 { unit([self.at[0] - at[0], self.at[1] - at[1], self.at[2] - at[2]]) } else { self.toward };
        let lit = dot(n, l).max(0.0) as f32;
        let h = unit([l[0] + v[0], l[1] + v[1], l[2] + v[2]]);
        let shine = if self.specular > 0.0 && lit > 0.0 { self.specular * (dot(n, h).max(0.0) as f32).powf(self.sharpness) } else { 0.0 };
        (self.color.map(|c| self.ambient + self.diffuse * lit * c), self.color.map(|c| shine * c))
    }
}

/// A flat piece: an outline (any simple one) in the texture's pixels about its resting middle,
/// turned and set in the scene.
#[derive(Clone, Debug)]
pub struct Piece {
    pub outline: Vec<[f32; 2]>,
    pub rest: [f32; 2],
    pub turn: M3,
    pub at: [f64; 3],
    pub alpha: f32,
    /// One colour instead of the texture.
    pub flat: Option<[f32; 3]>,
    /// The back's texture mirrored about the piece's middle: 0 not, 1 top to bottom, 2 side to
    /// side.
    pub mirror: u8,
}

/// A piece seen through a camera, ready to draw.
#[derive(Clone, Debug)]
pub struct Placed {
    /// From the picture's pixels to the piece's own (homogeneous).
    back_to_piece: M3,
    rest: [f32; 2],
    outline: Vec<[f32; 2]>,
    reach: [f32; 4],
    pub tint: [f32; 3],
    pub shine: [f32; 3],
    alpha: f32,
    flat: Option<[f32; 3]>,
    /// Its back faces the camera.
    pub back: bool,
    mirror: u8,
    pub depth: f64,
    /// Where its middle is, and which way it faces.
    pub centre: [f64; 3],
    pub normal: [f64; 3],
}

impl Piece {
    /// The piece as `cam` sees it (none behind the camera or edge on).
    pub fn place(&self, cam: &Camera) -> Option<Placed> {
        if self.outline.len() < 3 {
            return None;
        }
        let (m, t, p) = (&cam.m, &self.turn, self.at);
        // from the piece's own (a, b) plane to the picture: the camera after the turn and move
        let row = |i: usize| [m[i][0] * t[0][0] + m[i][1] * t[1][0] + m[i][2] * t[2][0], m[i][0] * t[0][1] + m[i][1] * t[1][1] + m[i][2] * t[2][1], m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3]];
        let h: M3 = [row(0), row(1), row(2)];
        let mut reach = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for v in &self.outline {
            let (a, b) = ((v[0] - self.rest[0]) as f64, (v[1] - self.rest[1]) as f64);
            let w = h[2][0] * a + h[2][1] * b + h[2][2];
            if w < cam.near {
                return None;
            }
            let (x, y) = (((h[0][0] * a + h[0][1] * b + h[0][2]) / w) as f32, ((h[1][0] * a + h[1][1] * b + h[1][2]) / w) as f32);
            reach = [reach[0].min(x), reach[1].min(y), reach[2].max(x), reach[3].max(y)];
        }
        let back_to_piece = invert(&h)?;
        let mut outline = self.outline.clone();
        // wound so that inside is to the left of every edge
        let area: f32 = (0..outline.len()).map(|i| outline[i][0] * outline[(i + 1) % outline.len()][1] - outline[(i + 1) % outline.len()][0] * outline[i][1]).sum();
        if area < 0.0 {
            outline.reverse();
        }
        // the front faces -z at rest; it faces the camera when its normal points at the eye
        let normal = [-t[0][2], -t[1][2], -t[2][2]];
        let eye = cam.eye();
        let facing = dot(normal, [eye[0] - p[0], eye[1] - p[1], eye[2] - p[2]]);
        Some(Placed {
            back_to_piece,
            rest: self.rest,
            outline,
            reach: [reach[0] - 1.0, reach[1] - 1.0, reach[2] + 1.0, reach[3] + 1.0],
            tint: [1.0; 3],
            shine: [0.0; 3],
            alpha: self.alpha,
            flat: self.flat,
            back: facing < 0.0,
            mirror: self.mirror,
            depth: h[2][2],
            centre: p,
            normal,
        })
    }
}

impl Placed {
    /// One colour instead of its texture.
    pub fn flat(mut self, c: [f32; 3]) -> Placed {
        self.flat = Some(c);
        self
    }

    /// Its texture tinted by `c`.
    pub fn tinted(mut self, c: [f32; 3]) -> Placed {
        self.tint = [0, 1, 2].map(|k| self.tint[k] * c[k]);
        self
    }

    /// Lit by `light`, seen from `eye`.
    pub fn lit(mut self, light: &Light, eye: [f64; 3]) -> Placed {
        (self.tint, self.shine) = light.shade(self.normal, self.centre, eye);
        self
    }

    /// Its colour at the pixel centred on (x, y), premultiplied.
    fn at(&self, x: f32, y: f32, front: &Image, back: Option<&Image>) -> Option<Px> {
        let m = &self.back_to_piece;
        let (xf, yf) = (x as f64, y as f64);
        let w = m[2][0] * xf + m[2][1] * yf + m[2][2];
        if w.abs() < 1e-12 {
            return None;
        }
        let u = ((m[0][0] * xf + m[0][1] * yf + m[0][2]) / w) as f32 + self.rest[0];
        let v = ((m[1][0] * xf + m[1][1] * yf + m[1][2]) / w) as f32 + self.rest[1];
        // how far inside the outline (below 0: outside), any simple outline, round or not
        let n = self.outline.len();
        let (mut near, mut odd) = (f32::MAX, false);
        for i in 0..n {
            let (a, b) = (self.outline[i], self.outline[(i + 1) % n]);
            let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
            let t = (((u - a[0]) * ex + (v - a[1]) * ey) / (ex * ex + ey * ey).max(1e-12)).clamp(0.0, 1.0);
            near = near.min((u - a[0] - ex * t).hypot(v - a[1] - ey * t));
            if (a[1] > v) != (b[1] > v) && u < a[0] + (v - a[1]) / (b[1] - a[1]) * ex {
                odd = !odd;
            }
        }
        let inside = if odd { near } else { -near };
        // half a pixel past the edge, so pieces side by side leave no seam
        let cover = (inside + 1.0).clamp(0.0, 1.0);
        if cover <= 0.0 {
            return None;
        }
        let (c, a) = match self.flat {
            Some(c) => (c, 1.0),
            None => {
                let tex = if self.back { back.unwrap_or(front) } else { front };
                let (u, v) = match (self.back, self.mirror) {
                    (true, 1) => (u, 2.0 * self.rest[1] - v),
                    (true, 2) => (2.0 * self.rest[0] - u, v),
                    _ => (u, v),
                };
                unpremul(tex.sample(Sampling::Bilinear, u as f64, v as f64))
            }
        };
        let a = a * cover * self.alpha;
        if a <= 1e-6 {
            return None;
        }
        let o = [0, 1, 2].map(|k| c[k] * self.tint[k] + self.shine[k]);
        Some([o[0] * a, o[1] * a, o[2] * a, a])
    }
}

/// Rows a band has.
const BAND: usize = 16;

/// `pieces` drawn over `out` far to near, textured by `front` (and `back` for those turned away;
/// none: `front` again).
pub fn draw(out: &mut Image, pieces: &mut [Placed], front: &Image, back: Option<&Image>) {
    pieces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    let (w, h) = (out.width, out.height);
    if w == 0 || h == 0 {
        return;
    }
    let bands = h.div_ceil(BAND);
    let mut in_band: Vec<Vec<u32>> = vec![Vec::new(); bands];
    let boxes: Vec<Option<[usize; 4]>> = pieces
        .iter()
        .map(|p| {
            let r = p.reach;
            if !r.iter().all(|v| v.is_finite()) || r[2] < 0.0 || r[3] < 0.0 || r[0] >= w as f32 || r[1] >= h as f32 {
                return None;
            }
            Some([r[0].floor().max(0.0) as usize, r[1].floor().max(0.0) as usize, (r[2].ceil() as usize).min(w - 1), (r[3].ceil() as usize).min(h - 1)])
        })
        .collect();
    for (i, b) in boxes.iter().enumerate() {
        if let Some(b) = b {
            for band in in_band.iter_mut().take(b[3] / BAND + 1).skip(b[1] / BAND) {
                band.push(i as u32);
            }
        }
    }
    par::chunks(&mut out.px, w * BAND, |band, chunk| {
        let top = band * BAND;
        let rows = chunk.len() / w;
        for &i in &in_band[band] {
            let (p, b) = (&pieces[i as usize], boxes[i as usize].expect("in a band"));
            for y in b[1].max(top)..=b[3].min(top + rows - 1) {
                for x in b[0]..=b[2] {
                    let Some(c) = p.at(x as f32 + 0.5, y as f32 + 0.5, front, back) else { continue };
                    let d = &mut chunk[(y - top) * w + x];
                    let k = 1.0 - c[3];
                    *d = [c[0] + d[0] * k, c[1] + d[1] * k, c[2] + d[2] * k, c[3] + d[3] * k];
                }
            }
        }
    });
}

/// The points of the camera and light settings, where they start (parts of the layer).
pub const POINTS: [(&str, [f64; 2]); 6] = [("xyPosition", [0.5, 0.5]), ("upperLeftCorner", [0.0, 0.0]), ("upperRightCorner", [1.0, 0.0]), ("lowerLeftCorner", [0.0, 1.0]), ("lowerRightCorner", [1.0, 1.0]), ("lightPosition", [0.5, 0.5])];

/// An effect seen in 3D: its own settings, then the camera's, the light's and the material's.
macro_rules! scene_effect {
    ($def:ident, $id:literal, $name:literal, $doc:literal, z: $z:literal, light: $li:literal, ambient: $amb:literal, diffuse: $dif:literal, [$($prop:expr),* $(,)?]) => {
        pub static $def: veymelo_project::GroupDef = veymelo_project::GroupDef {
            id: $id,
            name: $name,
            doc: $doc,
            body: veymelo_project::Body::Fixed(&[
                $(veymelo_project::DefNode::Prop($prop),)*
                veymelo_project::DefNode::Prop(crate::defs::choice("cameraSystem", "Camera System", &["cameraPosition", "cornerPins", "compCamera"], 0)),
                veymelo_project::DefNode::Prop(crate::defs::angle("xRotation", "X Rotation", 0.0)),
                veymelo_project::DefNode::Prop(crate::defs::angle("yRotation", "Y Rotation", 0.0)),
                veymelo_project::DefNode::Prop(crate::defs::angle("zRotation", "Z Rotation", 0.0)),
                veymelo_project::DefNode::Prop(crate::defs::point("xyPosition", "X, Y Position")),
                veymelo_project::DefNode::Prop(crate::defs::ranged("zPosition", "Z Position", $z, 0.05, 10.0).doc("how far the camera is, in layer sizes")),
                veymelo_project::DefNode::Prop(crate::defs::ranged("focalLength", "Focal Length", 70.0, 1.0, 500.0)),
                veymelo_project::DefNode::Prop(crate::defs::choice("transformOrder", "Transform Order", &["rotateXyzPosition", "rotateXzyPosition", "rotateYxzPosition", "rotateYzxPosition", "rotateZxyPosition", "rotateZyxPosition", "positionRotateXyz", "positionRotateXzy", "positionRotateYxz", "positionRotateYzx", "positionRotateZxy", "positionRotateZyx"], 0)),
                veymelo_project::DefNode::Prop(crate::defs::point("upperLeftCorner", "Upper Left Corner")),
                veymelo_project::DefNode::Prop(crate::defs::point("upperRightCorner", "Upper Right Corner")),
                veymelo_project::DefNode::Prop(crate::defs::point("lowerLeftCorner", "Lower Left Corner")),
                veymelo_project::DefNode::Prop(crate::defs::point("lowerRightCorner", "Lower Right Corner")),
                veymelo_project::DefNode::Prop(crate::defs::flag("autoFocalLength", "Auto Focal Length", true)),
                veymelo_project::DefNode::Prop(crate::defs::ranged("cornerFocalLength", "Corner Pins Focal Length", 70.0, 1.0, 500.0)),
                veymelo_project::DefNode::Prop(crate::defs::choice("lightType", "Light Type", &["distantSource", "pointSource", "firstCompLight"], 0)),
                veymelo_project::DefNode::Prop(crate::defs::ranged("lightIntensity", "Light Intensity", $li, 0.0, 4.0)),
                veymelo_project::DefNode::Prop(crate::defs::color("lightColor", "Light Color", 1.0, 1.0, 1.0)),
                veymelo_project::DefNode::Prop(crate::defs::point("lightPosition", "Light Position")),
                veymelo_project::DefNode::Prop(crate::defs::ranged("lightDepth", "Light Depth", 1.0, -10.0, 10.0)),
                veymelo_project::DefNode::Prop(crate::defs::ranged("ambientLight", "Ambient Light", $amb, 0.0, 2.0)),
                veymelo_project::DefNode::Prop(crate::defs::ranged("diffuseReflection", "Diffuse Reflection", $dif, 0.0, 2.0)),
                veymelo_project::DefNode::Prop(crate::defs::ranged("specularReflection", "Specular Reflection", 0.0, 0.0, 2.0)),
                veymelo_project::DefNode::Prop(crate::defs::ranged("highlightSharpness", "Highlight Sharpness", 15.0, 1.0, 100.0)),
            ]),
        };
    };
}
pub(crate) use scene_effect;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_camera_sees_the_plane_as_it_is() {
        let c = Camera::plain([10.0, 20.0], 100.0);
        assert_eq!(c.project([3.0, 4.0, 0.0]), Some([3.0, 4.0]));
        let e = c.eye();
        assert!((e[0] - 10.0).abs() < 1e-9 && (e[1] - 20.0).abs() < 1e-9 && (e[2] + 100.0).abs() < 1e-9);
        let far = c.project([30.0, 20.0, 100.0]).unwrap();
        assert!((far[0] - 20.0).abs() < 1e-9, "farther, toward the middle");
        assert!(c.project([0.0, 0.0, -99.0]).is_none(), "behind the near plane");
    }

    #[test]
    fn turnings() {
        assert_eq!(turning(0.0, 0.0, 0.0, 3), IDENTITY);
        let (a, b) = (turning(0.5, 0.3, 0.2, 0), turning(0.5, 0.3, 0.2, 5));
        assert!((0..3).any(|i| (0..3).any(|j| (a[i][j] - b[i][j]).abs() > 1e-3)));
        let q = apply(&about([0.0, 0.0, 1.0], std::f64::consts::FRAC_PI_2), [1.0, 0.0, 0.0]);
        assert!((q[0]).abs() < 1e-12 && (q[1] - 1.0).abs() < 1e-12);
        assert!(invert(&a).is_some_and(|i| (0..3).all(|r| (0..3).all(|c| (mul(&i, &a)[r][c] - IDENTITY[r][c]).abs() < 1e-9))));
    }

    #[test]
    fn a_piece_at_rest_draws_its_texture() {
        let mut tex = Image::new(20, 20);
        for (i, p) in tex.px.iter_mut().enumerate() {
            let v = (i % 20) as f32 / 20.0;
            *p = [v, 0.5, 1.0 - v, 1.0];
        }
        let cam = Camera::plain([10.0, 10.0], 400.0);
        let piece = Piece { outline: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]], rest: [10.0, 10.0], turn: IDENTITY, at: [10.0, 10.0, 0.0], alpha: 1.0, flat: None, mirror: 0 };
        let mut placed = vec![piece.place(&cam).unwrap()];
        assert!(!placed[0].back);
        let mut out = Image::new(20, 20);
        draw(&mut out, &mut placed, &tex, None);
        for (a, b) in out.px.iter().zip(&tex.px) {
            assert!((0..4).all(|k| (a[k] - b[k]).abs() < 1e-3), "{a:?} {b:?}");
        }
        // turned over: its back toward the camera
        let flipped = Piece { turn: about([0.0, 1.0, 0.0], std::f64::consts::PI), ..piece };
        assert!(flipped.place(&cam).unwrap().back);
    }
}
