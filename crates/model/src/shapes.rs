//! Solids made from a few numbers (a 3D primitive layer's cube, sphere, plane, torus, cone or
//! cylinder): one mesh with normals, texture points (0 to 1 across each face, or round each
//! surface) and tangents, centred on the origin, in the units it is given in, +Y up and +Z
//! toward the viewer as models have them.

use std::f32::consts::{PI, TAU};

use crate::{Material, Mesh, Model, Node, Primitive};

/// The solids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    Cube,
    Sphere,
    Plane,
    Torus,
    Cone,
    Cylinder,
}

/// A solid's sizes and how finely it is made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizes {
    /// Width, height and depth (a cube; a plane takes the width and height).
    pub size: [f32; 3],
    /// A sphere's, cone's or cylinder's radius; the radius of a torus's ring.
    pub radius: f32,
    /// A torus's tube's radius.
    pub tube: f32,
    /// A cone's or cylinder's height.
    pub height: f32,
    /// Pieces round it.
    pub segments: u32,
    /// A sphere's rings from pole to pole; the pieces round a torus's tube.
    pub rings: u32,
}

impl Default for Sizes {
    fn default() -> Sizes {
        Sizes { size: [300.0; 3], radius: 150.0, tube: 50.0, height: 300.0, segments: 48, rings: 24 }
    }
}

/// A mesh being made.
struct Build {
    p: Primitive,
}

impl Build {
    fn vertex(&mut self, pos: [f32; 3], n: [f32; 3], uv: [f32; 2]) -> u32 {
        self.p.positions.push(pos);
        self.p.normals.push(n);
        self.p.uvs[0].push(uv);
        (self.p.positions.len() - 1) as u32
    }

    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.p.indices.extend([a, b, c]);
    }

    /// Corners a, b, c, d anticlockwise seen from outside.
    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.tri(a, b, c);
        self.tri(a, c, d);
    }
}

/// The mesh of a solid.
pub fn mesh(shape: Shape, s: &Sizes) -> Primitive {
    let seg = s.segments.clamp(3, 512);
    let rings = s.rings.clamp(2, 512);
    let mut b = Build { p: Primitive { material: Some(0), ..Primitive::default() } };
    match shape {
        Shape::Cube => {
            let half = s.size.map(|v| v.abs() * 0.5);
            // each face's normal and the directions its texture runs across and down
            let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
                ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
                ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
                ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, -1.0, 0.0]),
                ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]),
                ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
                ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            ];
            for (n, u, v) in faces {
                let at = |su: f32, sv: f32| -> [f32; 3] { [0, 1, 2].map(|k| (n[k] + u[k] * su + v[k] * sv) * half[k]) };
                let i0 = b.vertex(at(-1.0, -1.0), n, [0.0, 0.0]);
                let i1 = b.vertex(at(1.0, -1.0), n, [1.0, 0.0]);
                let i2 = b.vertex(at(1.0, 1.0), n, [1.0, 1.0]);
                let i3 = b.vertex(at(-1.0, 1.0), n, [0.0, 1.0]);
                // across and down make a turn away from the normal: wound the other way
                b.quad(i0, i3, i2, i1);
            }
        }
        Shape::Plane => {
            let (w, h) = (s.size[0].abs() * 0.5, s.size[1].abs() * 0.5);
            let n = [0.0, 0.0, 1.0];
            let i0 = b.vertex([-w, h, 0.0], n, [0.0, 0.0]);
            let i1 = b.vertex([w, h, 0.0], n, [1.0, 0.0]);
            let i2 = b.vertex([w, -h, 0.0], n, [1.0, 1.0]);
            let i3 = b.vertex([-w, -h, 0.0], n, [0.0, 1.0]);
            b.quad(i0, i3, i2, i1);
        }
        Shape::Sphere => {
            let r = s.radius.abs();
            for j in 0..=rings {
                // the poles exactly, so they close
                let (st, ct) = if j == 0 {
                    (0.0, 1.0)
                } else if j == rings {
                    (0.0, -1.0)
                } else {
                    (PI * j as f32 / rings as f32).sin_cos()
                };
                for i in 0..=seg {
                    let ph = TAU * (i % seg) as f32 / seg as f32;
                    let n = [st * ph.sin(), ct, st * ph.cos()];
                    b.vertex(n.map(|c| c * r), n, [i as f32 / seg as f32, j as f32 / rings as f32]);
                }
            }
            let row = seg + 1;
            for j in 0..rings {
                for i in 0..seg {
                    let a = j * row + i;
                    let (below, below_next, next) = (a + row, a + row + 1, a + 1);
                    if j != 0 {
                        b.tri(a, below, next);
                    }
                    if j != rings - 1 {
                        b.tri(next, below, below_next);
                    }
                }
            }
        }
        Shape::Torus => {
            let (ring, tube) = (s.radius.abs(), s.tube.abs());
            for j in 0..=rings {
                let v = TAU * (j % rings) as f32 / rings as f32;
                for i in 0..=seg {
                    let u = TAU * (i % seg) as f32 / seg as f32;
                    // the ring lies flat (in x and z)
                    let (cu, su, cv, sv) = (u.cos(), u.sin(), v.cos(), v.sin());
                    let n = [cv * su, sv, cv * cu];
                    let p = [(ring + tube * cv) * su, tube * sv, (ring + tube * cv) * cu];
                    b.vertex(p, n, [i as f32 / seg as f32, j as f32 / rings as f32]);
                }
            }
            let row = seg + 1;
            for j in 0..rings {
                for i in 0..seg {
                    let a = j * row + i;
                    b.quad(a, a + 1, a + row + 1, a + row);
                }
            }
        }
        Shape::Cylinder | Shape::Cone => {
            let r = s.radius.abs();
            let h = s.height.abs() * 0.5;
            let top = if shape == Shape::Cone { 0.0 } else { r };
            // a cone's side leans: its normals tilt up
            let slope = (r - top) / (2.0 * h).max(1e-6);
            let (ny, nk) = (slope / (1.0 + slope * slope).sqrt(), 1.0 / (1.0 + slope * slope).sqrt());
            let first = b.p.positions.len() as u32;
            for i in 0..=seg {
                let a = TAU * (i % seg) as f32 / seg as f32;
                let (sa, ca) = a.sin_cos();
                let n = [sa * nk, ny, ca * nk];
                let u = i as f32 / seg as f32;
                b.vertex([sa * top, h, ca * top], n, [u, 0.0]);
                b.vertex([sa * r, -h, ca * r], n, [u, 1.0]);
            }
            for i in 0..seg {
                let t0 = first + 2 * i;
                let (b0, t1, b1) = (t0 + 1, t0 + 2, t0 + 3);
                if shape == Shape::Cone {
                    b.tri(t0, b0, b1);
                } else {
                    b.quad(t0, b0, b1, t1);
                }
            }
            let caps: &[f32] = if shape == Shape::Cone { &[-1.0] } else { &[1.0, -1.0] };
            for &side in caps {
                let n = [0.0, side, 0.0];
                let centre = b.vertex([0.0, h * side, 0.0], n, [0.5, 0.5]);
                let rim = b.p.positions.len() as u32;
                for i in 0..=seg {
                    let a = TAU * (i % seg) as f32 / seg as f32;
                    let (sa, ca) = a.sin_cos();
                    b.vertex([sa * r, h * side, ca * r], n, [0.5 + 0.5 * sa, 0.5 - 0.5 * ca * side]);
                }
                for i in 0..seg {
                    let (p0, p1) = (rim + i, rim + i + 1);
                    if side > 0.0 {
                        b.tri(centre, p0, p1);
                    } else {
                        b.tri(centre, p1, p0);
                    }
                }
            }
        }
    }
    b.p.make_tangents();
    b.p
}

/// A solid as a model of one mesh in one material (a plane seen from both sides).
pub fn model(shape: Shape, s: &Sizes, mut material: Material) -> Model {
    if shape == Shape::Plane {
        material.two_sided = true;
    }
    let name = format!("{shape:?}");
    Model {
        meshes: vec![Mesh { name: name.clone(), primitives: vec![mesh(shape, s)], weights: Vec::new() }],
        nodes: vec![Node { name, mesh: Some(0), ..Node::default() }],
        roots: vec![0],
        materials: vec![material],
        ..Model::default()
    }
}
