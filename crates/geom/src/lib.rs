//! Geometry: 2D and 3D vectors, rectangles, 3×3 matrices (2D, with perspective) and 4×4
//! matrices (3D), quaternions, the layer transform, and the camera.
//!
//! The conventions are the industry's for compositing: y grows downward, angles are degrees
//! and turn clockwise on the screen, 3D is left-handed with +z going into the screen, and a 3D
//! layer turns by its orientation (X, then Y, then Z) and then by its X, Y and Z rotations.

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

pub const fn vec2(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

pub const fn vec3(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

impl Vec2 {
    pub const ZERO: Vec2 = vec2(0.0, 0.0);

    pub fn dot(self, o: Vec2) -> f64 {
        self.x * o.x + self.y * o.y
    }

    /// The z of the cross product: above zero when `o` turns clockwise from `self` on screen.
    pub fn cross(self, o: Vec2) -> f64 {
        self.x * o.y - self.y * o.x
    }

    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// The same direction, of length 1 (zero stays zero).
    pub fn normalize(self) -> Vec2 {
        let l = self.length();
        if l > 0.0 { self / l } else { self }
    }

    pub fn lerp(self, o: Vec2, t: f64) -> Vec2 {
        self + (o - self) * t
    }

    /// Turned `deg` degrees clockwise on screen.
    pub fn rotate_deg(self, deg: f64) -> Vec2 {
        let (s, c) = deg.to_radians().sin_cos();
        vec2(c * self.x - s * self.y, s * self.x + c * self.y)
    }

    /// Each part times the other's.
    pub fn mul_each(self, o: Vec2) -> Vec2 {
        vec2(self.x * o.x, self.y * o.y)
    }

    pub fn extend(self, z: f64) -> Vec3 {
        vec3(self.x, self.y, z)
    }

    /// The angle from the x axis, degrees clockwise.
    pub fn angle_deg(self) -> f64 {
        self.y.atan2(self.x).to_degrees()
    }
}

impl Vec3 {
    pub const ZERO: Vec3 = vec3(0.0, 0.0, 0.0);

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        vec3(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn normalize(self) -> Vec3 {
        let l = self.length();
        if l > 0.0 { self / l } else { self }
    }

    pub fn lerp(self, o: Vec3, t: f64) -> Vec3 {
        self + (o - self) * t
    }

    pub fn mul_each(self, o: Vec3) -> Vec3 {
        vec3(self.x * o.x, self.y * o.y, self.z * o.z)
    }

    pub fn xy(self) -> Vec2 {
        vec2(self.x, self.y)
    }
}

macro_rules! vector_ops {
    ($t:ident { $($f:ident),* }) => {
        impl Add for $t {
            type Output = $t;
            fn add(self, o: $t) -> $t {
                $t { $($f: self.$f + o.$f),* }
            }
        }
        impl Sub for $t {
            type Output = $t;
            fn sub(self, o: $t) -> $t {
                $t { $($f: self.$f - o.$f),* }
            }
        }
        impl Mul<f64> for $t {
            type Output = $t;
            fn mul(self, k: f64) -> $t {
                $t { $($f: self.$f * k),* }
            }
        }
        impl Div<f64> for $t {
            type Output = $t;
            fn div(self, k: f64) -> $t {
                $t { $($f: self.$f / k),* }
            }
        }
        impl Neg for $t {
            type Output = $t;
            fn neg(self) -> $t {
                $t { $($f: -self.$f),* }
            }
        }
        impl AddAssign for $t {
            fn add_assign(&mut self, o: $t) {
                $(self.$f += o.$f;)*
            }
        }
        impl SubAssign for $t {
            fn sub_assign(&mut self, o: $t) {
                $(self.$f -= o.$f;)*
            }
        }
    };
}
vector_ops!(Vec2 { x, y });
vector_ops!(Vec3 { x, y, z });

impl From<[f64; 3]> for Vec3 {
    fn from(a: [f64; 3]) -> Vec3 {
        vec3(a[0], a[1], a[2])
    }
}

impl From<Vec3> for [f64; 3] {
    fn from(v: Vec3) -> [f64; 3] {
        [v.x, v.y, v.z]
    }
}

/// A rectangle from (`x0`, `y0`) to (`x1`, `y1`); empty when either is not after the start.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub const EMPTY: Rect = Rect { x0: f64::INFINITY, y0: f64::INFINITY, x1: f64::NEG_INFINITY, y1: f64::NEG_INFINITY };

    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    pub fn from_size(w: f64, h: f64) -> Rect {
        Rect::new(0.0, 0.0, w, h)
    }

    pub fn width(&self) -> f64 {
        (self.x1 - self.x0).max(0.0)
    }

    pub fn height(&self) -> f64 {
        (self.y1 - self.y0).max(0.0)
    }

    pub fn is_empty(&self) -> bool {
        !(self.x1 > self.x0 && self.y1 > self.y0)
    }

    pub fn center(&self) -> Vec2 {
        vec2((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }

    pub fn union(&self, o: &Rect) -> Rect {
        Rect::new(self.x0.min(o.x0), self.y0.min(o.y0), self.x1.max(o.x1), self.y1.max(o.y1))
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        Rect::new(self.x0.max(o.x0), self.y0.max(o.y0), self.x1.min(o.x1), self.y1.min(o.y1))
    }

    /// Grown to take in `p`.
    pub fn include(&mut self, p: Vec2) {
        (self.x0, self.y0, self.x1, self.y1) = (self.x0.min(p.x), self.y0.min(p.y), self.x1.max(p.x), self.y1.max(p.y));
    }

    /// Grown by `d` on every side.
    pub fn inflate(&self, d: f64) -> Rect {
        Rect::new(self.x0 - d, self.y0 - d, self.x1 + d, self.y1 + d)
    }

    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x0 && p.x < self.x1 && p.y >= self.y0 && p.y < self.y1
    }

    /// Top left, top right, bottom right, bottom left.
    pub fn corners(&self) -> [Vec2; 4] {
        [vec2(self.x0, self.y0), vec2(self.x1, self.y0), vec2(self.x1, self.y1), vec2(self.x0, self.y1)]
    }

    /// Grown out to whole numbers.
    pub fn round_out(&self) -> Rect {
        Rect::new(self.x0.floor(), self.y0.floor(), self.x1.ceil(), self.y1.ceil())
    }
}

/// A 3×3 matrix (rows), mapping 2D points, with perspective when its last row is not (0, 0, 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3(pub [[f64; 3]; 3]);

impl Default for Mat3 {
    fn default() -> Mat3 {
        Mat3::IDENTITY
    }
}

impl Mul for Mat3 {
    type Output = Mat3;
    fn mul(self, o: Mat3) -> Mat3 {
        let (a, b) = (&self.0, &o.0);
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        Mat3(m)
    }
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    pub fn translate(t: Vec2) -> Mat3 {
        Mat3([[1.0, 0.0, t.x], [0.0, 1.0, t.y], [0.0, 0.0, 1.0]])
    }

    pub fn scale(s: Vec2) -> Mat3 {
        Mat3([[s.x, 0.0, 0.0], [0.0, s.y, 0.0], [0.0, 0.0, 1.0]])
    }

    /// A turn of `deg` degrees, clockwise on screen.
    pub fn rotate_deg(deg: f64) -> Mat3 {
        let (s, c) = deg.to_radians().sin_cos();
        Mat3([[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]])
    }

    /// A slant of `skew` degrees along the direction `axis` degrees from x.
    pub fn skew_deg(skew: f64, axis: f64) -> Mat3 {
        let k = skew.to_radians().tan();
        Mat3::rotate_deg(axis) * Mat3([[1.0, k, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]) * Mat3::rotate_deg(-axis)
    }

    /// A 2D layer's transform, from the layer's own space to its parent's: the anchor point
    /// goes to the position, scaled (percent) and turned (degrees) about it.
    pub fn layer_2d(anchor: Vec2, position: Vec2, scale_pct: Vec2, rotation: f64) -> Mat3 {
        Mat3::translate(position) * Mat3::rotate_deg(rotation) * Mat3::scale(scale_pct / 100.0) * Mat3::translate(-anchor)
    }

    pub fn apply(&self, p: Vec2) -> Vec2 {
        let m = &self.0;
        let x = m[0][0] * p.x + m[0][1] * p.y + m[0][2];
        let y = m[1][0] * p.x + m[1][1] * p.y + m[1][2];
        let w = m[2][0] * p.x + m[2][1] * p.y + m[2][2];
        if w != 1.0 && w != 0.0 { vec2(x / w, y / w) } else { vec2(x, y) }
    }

    /// A direction, without the move (for matrices without perspective).
    pub fn apply_vec(&self, v: Vec2) -> Vec2 {
        let m = &self.0;
        vec2(m[0][0] * v.x + m[0][1] * v.y, m[1][0] * v.x + m[1][1] * v.y)
    }

    pub fn is_affine(&self) -> bool {
        self.0[2][0] == 0.0 && self.0[2][1] == 0.0 && (self.0[2][2] - 1.0).abs() < 1e-12
    }

    pub fn determinant(&self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    pub fn inverse(&self) -> Option<Mat3> {
        let d = self.determinant();
        if !d.is_finite() || d.abs() < 1e-300 {
            return None;
        }
        let m = &self.0;
        let c = |a: usize, b: usize, e: usize, f: usize| m[a][b] * m[e][f] - m[a][f] * m[e][b];
        Some(Mat3([
            [c(1, 1, 2, 2) / d, -c(0, 1, 2, 2) / d, c(0, 1, 1, 2) / d],
            [-c(1, 0, 2, 2) / d, c(0, 0, 2, 2) / d, -c(0, 0, 1, 2) / d],
            [c(1, 0, 2, 1) / d, -c(0, 0, 2, 1) / d, c(0, 0, 1, 1) / d],
        ]))
    }

    /// The box round a rectangle once mapped.
    pub fn map_rect(&self, r: &Rect) -> Rect {
        let mut out = Rect::EMPTY;
        for p in r.corners() {
            out.include(self.apply(p));
        }
        out
    }

    /// How much it scales lengths, on average (the square root of its area scale).
    pub fn mean_scale(&self) -> f64 {
        let m = &self.0;
        (m[0][0] * m[1][1] - m[0][1] * m[1][0]).abs().sqrt()
    }

    /// The perspective map from the unit square ((0,0), (1,0), (1,1), (0,1)) onto the corners
    /// `q` in that order.
    pub fn square_to_quad(q: [Vec2; 4]) -> Mat3 {
        let [p0, p1, p2, p3] = q;
        let (dx1, dx2, dx3) = (p1.x - p2.x, p3.x - p2.x, p0.x - p1.x + p2.x - p3.x);
        let (dy1, dy2, dy3) = (p1.y - p2.y, p3.y - p2.y, p0.y - p1.y + p2.y - p3.y);
        if dx3.abs() < 1e-12 && dy3.abs() < 1e-12 {
            // a parallelogram: no perspective
            return Mat3([[p1.x - p0.x, p3.x - p0.x, p0.x], [p1.y - p0.y, p3.y - p0.y, p0.y], [0.0, 0.0, 1.0]]);
        }
        let den = dx1 * dy2 - dx2 * dy1;
        let g = (dx3 * dy2 - dx2 * dy3) / den;
        let h = (dx1 * dy3 - dx3 * dy1) / den;
        Mat3([[p1.x - p0.x + g * p1.x, p3.x - p0.x + h * p3.x, p0.x], [p1.y - p0.y + g * p1.y, p3.y - p0.y + h * p3.y, p0.y], [g, h, 1.0]])
    }

    /// The perspective map taking the corners `from` onto the corners `to`.
    pub fn quad_to_quad(from: [Vec2; 4], to: [Vec2; 4]) -> Option<Mat3> {
        Some(Mat3::square_to_quad(to) * Mat3::square_to_quad(from).inverse()?)
    }
}

/// A 4×4 matrix (rows), mapping 3D points, with perspective when its last row is not
/// (0, 0, 0, 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [[f64; 4]; 4]);

impl Default for Mat4 {
    fn default() -> Mat4 {
        Mat4::IDENTITY
    }
}

impl Mul for Mat4 {
    type Output = Mat4;
    fn mul(self, o: Mat4) -> Mat4 {
        let (a, b) = (&self.0, &o.0);
        let mut m = [[0.0; 4]; 4];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..4).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        Mat4(m)
    }
}

impl Mat4 {
    pub const IDENTITY: Mat4 = Mat4([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);

    pub fn translate(t: Vec3) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        (m.0[0][3], m.0[1][3], m.0[2][3]) = (t.x, t.y, t.z);
        m
    }

    pub fn scale(s: Vec3) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        (m.0[0][0], m.0[1][1], m.0[2][2]) = (s.x, s.y, s.z);
        m
    }

    pub fn rotate_x(deg: f64) -> Mat4 {
        let (s, c) = deg.to_radians().sin_cos();
        Mat4([[1.0, 0.0, 0.0, 0.0], [0.0, c, -s, 0.0], [0.0, s, c, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }

    pub fn rotate_y(deg: f64) -> Mat4 {
        let (s, c) = deg.to_radians().sin_cos();
        Mat4([[c, 0.0, s, 0.0], [0.0, 1.0, 0.0, 0.0], [-s, 0.0, c, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }

    pub fn rotate_z(deg: f64) -> Mat4 {
        let (s, c) = deg.to_radians().sin_cos();
        Mat4([[c, -s, 0.0, 0.0], [s, c, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }

    /// An orientation (degrees): turned about X, then Y, then Z.
    pub fn orientation(o: Vec3) -> Mat4 {
        Mat4::rotate_z(o.z) * Mat4::rotate_y(o.y) * Mat4::rotate_x(o.x)
    }

    /// A 3D layer's transform, from its own space to its parent's: the anchor goes to the
    /// position, scaled (percent), turned by the X, Y, Z rotations, then by the orientation.
    pub fn layer_3d(anchor: Vec3, position: Vec3, scale_pct: Vec3, orientation: Vec3, rotation: Vec3) -> Mat4 {
        Mat4::translate(position) * Mat4::orientation(orientation) * Mat4::rotate_z(rotation.z) * Mat4::rotate_y(rotation.y) * Mat4::rotate_x(rotation.x) * Mat4::scale(scale_pct / 100.0) * Mat4::translate(-anchor)
    }

    /// A 2D transform in 3D (z left as it is).
    pub fn from_mat3(m: &Mat3) -> Mat4 {
        let a = &m.0;
        Mat4([[a[0][0], a[0][1], 0.0, a[0][2]], [a[1][0], a[1][1], 0.0, a[1][2]], [0.0, 0.0, 1.0, 0.0], [a[2][0], a[2][1], 0.0, a[2][2]]])
    }

    pub fn apply(&self, p: Vec3) -> Vec3 {
        let m = &self.0;
        let r = |i: usize| m[i][0] * p.x + m[i][1] * p.y + m[i][2] * p.z + m[i][3];
        let (x, y, z, w) = (r(0), r(1), r(2), r(3));
        if w != 1.0 && w != 0.0 { vec3(x / w, y / w, z / w) } else { vec3(x, y, z) }
    }

    /// A direction, without the move.
    pub fn apply_vec(&self, v: Vec3) -> Vec3 {
        let m = &self.0;
        let r = |i: usize| m[i][0] * v.x + m[i][1] * v.y + m[i][2] * v.z;
        vec3(r(0), r(1), r(2))
    }

    pub fn transpose(&self) -> Mat4 {
        let mut m = [[0.0; 4]; 4];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = self.0[j][i];
            }
        }
        Mat4(m)
    }

    pub fn inverse(&self) -> Option<Mat4> {
        // Gauss–Jordan elimination with the largest pivot in each column
        let mut a = self.0;
        let mut inv = Mat4::IDENTITY.0;
        for col in 0..4 {
            let pivot = (col..4).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
            if !a[pivot][col].is_finite() || a[pivot][col].abs() < 1e-300 {
                return None;
            }
            a.swap(col, pivot);
            inv.swap(col, pivot);
            let k = a[col][col];
            for j in 0..4 {
                a[col][j] /= k;
                inv[col][j] /= k;
            }
            for i in 0..4 {
                if i != col {
                    let f = a[i][col];
                    for j in 0..4 {
                        a[i][j] -= f * a[col][j];
                        inv[i][j] -= f * inv[col][j];
                    }
                }
            }
        }
        Some(Mat4(inv))
    }

    /// The turn it makes, as an orientation (X, Y, Z degrees, turned in that order), for a
    /// matrix that only turns.
    pub fn to_orientation(&self) -> Vec3 {
        let m = &self.0;
        let y = (-m[2][0]).clamp(-1.0, 1.0).asin();
        if y.cos().abs() > 1e-9 {
            vec3(m[2][1].atan2(m[2][2]).to_degrees(), y.to_degrees(), m[1][0].atan2(m[0][0]).to_degrees())
        } else {
            // straight up or down: X and Z turn about the same axis; put it all in X
            vec3((-m[1][2]).atan2(m[1][1]).to_degrees(), y.to_degrees(), 0.0)
        }
    }
}

/// A turn in 3D, as a unit quaternion (for turning smoothly between orientations).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub w: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Mul for Quat {
    type Output = Quat;
    /// `self` after `o`.
    fn mul(self, o: Quat) -> Quat {
        Quat {
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
        }
    }
}

impl Quat {
    pub const IDENTITY: Quat = Quat { w: 1.0, x: 0.0, y: 0.0, z: 0.0 };

    /// A turn of `deg` degrees about `axis` (as `Mat4::rotate_x` and its kin turn).
    pub fn from_axis_deg(axis: Vec3, deg: f64) -> Quat {
        let a = axis.normalize();
        let (s, c) = (deg.to_radians() / 2.0).sin_cos();
        Quat { w: c, x: a.x * s, y: a.y * s, z: a.z * s }
    }

    /// An orientation (X, then Y, then Z, degrees).
    pub fn from_orientation(o: Vec3) -> Quat {
        Quat::from_axis_deg(vec3(0.0, 0.0, 1.0), o.z) * Quat::from_axis_deg(vec3(0.0, 1.0, 0.0), o.y) * Quat::from_axis_deg(vec3(1.0, 0.0, 0.0), o.x)
    }

    pub fn length(self) -> f64 {
        (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn normalize(self) -> Quat {
        let l = self.length();
        if l > 0.0 { Quat { w: self.w / l, x: self.x / l, y: self.y / l, z: self.z / l } } else { Quat::IDENTITY }
    }

    fn dot(self, o: Quat) -> f64 {
        self.w * o.w + self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// `t` of the way from `self` to `o` (0 to 1), turning the short way at an even speed.
    pub fn slerp(self, o: Quat, t: f64) -> Quat {
        let mut d = self.dot(o);
        let o = if d < 0.0 {
            d = -d;
            Quat { w: -o.w, x: -o.x, y: -o.y, z: -o.z }
        } else {
            o
        };
        if d > 0.9995 {
            // nearly the same: a straight blend is as good
            return Quat { w: self.w + (o.w - self.w) * t, x: self.x + (o.x - self.x) * t, y: self.y + (o.y - self.y) * t, z: self.z + (o.z - self.z) * t }.normalize();
        }
        let th = d.clamp(-1.0, 1.0).acos();
        let (a, b) = (((1.0 - t) * th).sin() / th.sin(), (t * th).sin() / th.sin());
        Quat { w: a * self.w + b * o.w, x: a * self.x + b * o.x, y: a * self.y + b * o.y, z: a * self.z + b * o.z }
    }

    pub fn to_mat4(self) -> Mat4 {
        let Quat { w, x, y, z } = self.normalize();
        Mat4([
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y), 0.0],
            [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x), 0.0],
            [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y), 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// As an orientation (X, then Y, then Z, degrees).
    pub fn to_orientation(self) -> Vec3 {
        self.to_mat4().to_orientation()
    }
}

/// A camera looking at a comp: where it is, which way it looks (its own right, down and
/// forward, as rows), and its zoom (the distance from the eye to where one unit is one pixel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    pub turn: Mat4,
    pub zoom: f64,
    /// The comp's size (the camera's centre is its middle).
    pub size: Vec2,
}

impl Camera {
    /// The zoom of a 50 mm lens on 36 mm film, for a comp `width` wide: what is seen when a
    /// comp has 3D layers and no camera.
    pub fn default_zoom(width: f64) -> f64 {
        width * 50.0 / 36.0
    }

    /// The camera a comp `size` big has when it has none: in front of its middle, looking at
    /// it, so a layer at z 0 looks exactly as it would in 2D.
    pub fn default_for(size: Vec2) -> Camera {
        let zoom = Camera::default_zoom(size.x);
        Camera::looking_at(vec3(size.x / 2.0, size.y / 2.0, -zoom), vec3(size.x / 2.0, size.y / 2.0, 0.0), zoom, size)
    }

    /// A camera at `position` looking at `target`, upright (its down is the world's down, as
    /// near as it can be).
    pub fn looking_at(position: Vec3, target: Vec3, zoom: f64, size: Vec2) -> Camera {
        let f = (target - position).normalize();
        let f = if f.length() == 0.0 { vec3(0.0, 0.0, 1.0) } else { f };
        let up = if f.cross(vec3(0.0, -1.0, 0.0)).length() < 1e-9 { vec3(0.0, 0.0, -1.0) } else { vec3(0.0, -1.0, 0.0) };
        let right = f.cross(up).normalize();
        let down = f.cross(right);
        let turn = Mat4([[right.x, right.y, right.z, 0.0], [down.x, down.y, down.z, 0.0], [f.x, f.y, f.z, 0.0], [0.0, 0.0, 0.0, 1.0]]);
        Camera { position, turn, zoom, size }
    }

    /// From the world into the camera's own space (x right, y down, z forward).
    pub fn view(&self) -> Mat4 {
        self.turn * Mat4::translate(-self.position)
    }

    /// Where a point of the world falls in the comp (pixels), and how far ahead of the camera
    /// it is; None when it is not in front of the camera.
    pub fn project(&self, p: Vec3) -> Option<(Vec2, f64)> {
        let c = self.view().apply(p);
        if c.z <= 1e-6 {
            return None;
        }
        Some((vec2(self.zoom * c.x / c.z + self.size.x / 2.0, self.zoom * c.y / c.z + self.size.y / 2.0), c.z))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near2(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-9
    }

    fn near3(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-9
    }

    fn near_m4(a: &Mat4, b: &Mat4) -> bool {
        (0..4).all(|i| (0..4).all(|j| (a.0[i][j] - b.0[i][j]).abs() < 1e-9))
    }

    #[test]
    fn a_layer_puts_its_anchor_at_its_position() {
        let m = Mat3::layer_2d(vec2(50.0, 25.0), vec2(960.0, 540.0), vec2(200.0, 50.0), 30.0);
        assert!(near2(m.apply(vec2(50.0, 25.0)), vec2(960.0, 540.0)));
        let p = vec2(12.0, -7.0);
        assert!(near2(m.inverse().unwrap().apply(m.apply(p)), p));
        // scaled about the anchor: a point 10 right of it lands 20 right (200%) before turning
        let m = Mat3::layer_2d(vec2(0.0, 0.0), vec2(100.0, 100.0), vec2(200.0, 100.0), 0.0);
        assert!(near2(m.apply(vec2(10.0, 0.0)), vec2(120.0, 100.0)));
    }

    #[test]
    fn turning_is_clockwise_on_screen() {
        assert!(near2(Mat3::rotate_deg(90.0).apply(vec2(1.0, 0.0)), vec2(0.0, 1.0)));
        assert!(near2(vec2(1.0, 0.0).rotate_deg(90.0), vec2(0.0, 1.0)));
        assert!((vec2(0.0, 1.0).angle_deg() - 90.0).abs() < 1e-9);
    }

    #[test]
    fn a_square_maps_onto_any_quad_corner_for_corner() {
        let q = [vec2(10.0, 10.0), vec2(110.0, 20.0), vec2(120.0, 140.0), vec2(5.0, 100.0)];
        let m = Mat3::square_to_quad(q);
        for (c, p) in [vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0)].iter().zip(q) {
            assert!(near2(m.apply(*c), p), "{c:?}");
        }
        let r = [vec2(0.0, 0.0), vec2(200.0, 0.0), vec2(200.0, 100.0), vec2(0.0, 100.0)];
        let qq = Mat3::quad_to_quad(r, q).unwrap();
        for (a, b) in r.iter().zip(q) {
            assert!(near2(qq.apply(*a), b));
        }
    }

    #[test]
    fn inverses_undo() {
        let m = Mat4::layer_3d(vec3(10.0, 20.0, 5.0), vec3(300.0, 200.0, -50.0), vec3(150.0, 80.0, 100.0), vec3(10.0, 20.0, 30.0), vec3(-15.0, 40.0, 5.0));
        let p = vec3(3.0, -4.0, 7.0);
        assert!(near3(m.inverse().unwrap().apply(m.apply(p)), p));
        assert!(near_m4(&(m * m.inverse().unwrap()), &Mat4::IDENTITY));
        assert!(Mat4::scale(vec3(1.0, 0.0, 1.0)).inverse().is_none());
    }

    #[test]
    fn a_3d_layer_without_turns_is_its_2d_self() {
        let m3 = Mat4::layer_3d(vec3(50.0, 25.0, 0.0), vec3(960.0, 540.0, 0.0), vec3(200.0, 50.0, 100.0), Vec3::ZERO, vec3(0.0, 0.0, 30.0));
        let m2 = Mat3::layer_2d(vec2(50.0, 25.0), vec2(960.0, 540.0), vec2(200.0, 50.0), 30.0);
        let p = vec2(17.0, -3.0);
        assert!(near2(m3.apply(p.extend(0.0)).xy(), m2.apply(p)));
        assert!(near_m4(&Mat4::from_mat3(&m2), &m3));
    }

    #[test]
    fn quaternions_turn_like_the_matrices() {
        for (axis, m) in [(vec3(1.0, 0.0, 0.0), Mat4::rotate_x(35.0)), (vec3(0.0, 1.0, 0.0), Mat4::rotate_y(35.0)), (vec3(0.0, 0.0, 1.0), Mat4::rotate_z(35.0))] {
            assert!(near_m4(&Quat::from_axis_deg(axis, 35.0).to_mat4(), &m), "{axis:?}");
        }
        let o = vec3(20.0, -35.0, 60.0);
        assert!(near_m4(&Quat::from_orientation(o).to_mat4(), &Mat4::orientation(o)));
        assert!(near3(Quat::from_orientation(o).to_orientation(), o));
        // half way from none to 90° about z is 45°
        let half = Quat::IDENTITY.slerp(Quat::from_axis_deg(vec3(0.0, 0.0, 1.0), 90.0), 0.5);
        assert!(near_m4(&half.to_mat4(), &Mat4::rotate_z(45.0)));
        assert!((half.length() - 1.0).abs() < 1e-12);
        // the short way round: from 350° to 10° passes 0°
        let a = Quat::from_axis_deg(vec3(0.0, 0.0, 1.0), 350.0);
        let b = Quat::from_axis_deg(vec3(0.0, 0.0, 1.0), 10.0);
        assert!((a.slerp(b, 0.5).to_orientation().z).abs() < 1e-6);
    }

    #[test]
    fn the_default_camera_sees_z_zero_as_2d() {
        let cam = Camera::default_for(vec2(1920.0, 1080.0));
        for p in [vec2(0.0, 0.0), vec2(960.0, 540.0), vec2(1500.0, 200.0)] {
            let (q, depth) = cam.project(p.extend(0.0)).unwrap();
            assert!(near2(q, p), "{p:?} → {q:?}");
            assert!((depth - Camera::default_zoom(1920.0)).abs() < 1e-6);
        }
        // nearer things look bigger, farther smaller, behind it nothing
        let (near, _) = cam.project(vec3(1060.0, 540.0, -1000.0)).unwrap();
        assert!(near.x - 960.0 > 100.0);
        let (far, _) = cam.project(vec3(1060.0, 540.0, 2000.0)).unwrap();
        assert!(far.x - 960.0 < 100.0);
        assert!(cam.project(vec3(960.0, 540.0, -5000.0)).is_none());
    }

    #[test]
    fn rects() {
        let mut r = Rect::EMPTY;
        assert!(r.is_empty());
        r.include(vec2(1.0, 2.0));
        r.include(vec2(-3.0, 5.0));
        assert_eq!(r, Rect::new(-3.0, 2.0, 1.0, 5.0));
        assert_eq!(Rect::new(0.2, 0.7, 3.1, 4.0).round_out(), Rect::new(0.0, 0.0, 4.0, 4.0));
        let m = Mat3::rotate_deg(90.0);
        let b = m.map_rect(&Rect::from_size(10.0, 20.0));
        assert!((b.width() - 20.0).abs() < 1e-9 && (b.height() - 10.0).abs() < 1e-9);
    }
}
