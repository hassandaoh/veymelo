//! A 3D text or shape layer made solid: the outlines of its picture (where it is at least half
//! opaque) pushed back into depth, its front and back faces showing its picture, its edges
//! bevelled — in a straight slope, hollowed in or rounded out — and its sides coloured as its
//! edges are. Made as a model in the layer's own space (y down, depth away from the viewer), to
//! be drawn, lit and shadowed as models are.

use veymelo_color::srgb_to_linear;
use veymelo_effects::Buf;
use veymelo_model::{Map, Material, Mesh, Model, Node, Primitive, Texture};
use veymelo_path::caps::{area, triangulate};
use veymelo_path::trace::{Trace, trace};

/// How a layer is made solid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    /// 0 none, 1 angular, 2 concave, 3 convex.
    pub style: u32,
    pub bevel: f64,
    /// The bevel round holes, as a part of `bevel`.
    pub hole_bevel: f64,
    pub depth: f64,
}

/// The ring `r` moved `d` toward its solid side (to the left as it goes, its solid side there
/// once rings are turned as `area` wants them), corners mitred (no further than three times).
fn inset(r: &[[f64; 2]], d: f64) -> Vec<[f64; 2]> {
    if d == 0.0 {
        return r.to_vec();
    }
    let n = r.len();
    (0..n)
        .map(|i| {
            let (a, p, b) = (r[(i + n - 1) % n], r[i], r[(i + 1) % n]);
            let left = |u: [f64; 2], v: [f64; 2]| {
                let (dx, dy) = (v[0] - u[0], v[1] - u[1]);
                let l = dx.hypot(dy).max(1e-12);
                [-dy / l, dx / l]
            };
            let (n1, n2) = (left(a, p), left(p, b));
            let m = [n1[0] + n2[0], n1[1] + n2[1]];
            let ml = m[0].hypot(m[1]);
            if ml < 1e-9 {
                return [p[0] + n1[0] * d, p[1] + n1[1] * d];
            }
            let m = [m[0] / ml, m[1] / ml];
            let k = (1.0 / (m[0] * n1[0] + m[1] * n1[1]).max(1.0 / 3.0)) * d;
            [p[0] + m[0] * k, p[1] + m[1] * k]
        })
        .collect()
}

/// Where segments a–b and c–d cross (not at their ends), if they do.
fn crossing(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<[f64; 2]> {
    let (r, q) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let den = r[0] * q[1] - r[1] * q[0];
    if den.abs() < 1e-12 {
        return None;
    }
    let (ax, ay) = (c[0] - a[0], c[1] - a[1]);
    let (t, u) = ((ax * q[1] - ay * q[0]) / den, (ax * r[1] - ay * r[0]) / den);
    (t > 1e-9 && t < 1.0 - 1e-9 && u > 1e-9 && u < 1.0 - 1e-9).then(|| [a[0] + r[0] * t, a[1] + r[1] * t])
}

/// A ring moved in so far it crosses itself (the little loops an inward offset makes round a
/// hollow curve) made simple again: each smaller loop's corners gathered onto where it crosses,
/// so the ring keeps its corners (one to one with the rings beside it in a bevel).
fn untangle(mut r: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    let n = r.len();
    for _ in 0..n {
        let mut found = None;
        'search: for i in 0..n {
            for j in i + 2..n {
                if i == 0 && j == n - 1 {
                    continue;
                }
                if let Some(x) = crossing(r[i], r[(i + 1) % n], r[j], r[(j + 1) % n]) {
                    found = Some((i, j, x));
                    break 'search;
                }
            }
        }
        let Some((i, j, x)) = found else { break };
        // the loop between them (i + 1 ..= j) or the rest, whichever has fewer corners
        let inner: Vec<usize> = (i + 1..=j).collect();
        let outer: Vec<usize> = (j + 1..n).chain(0..=i).collect();
        for k in if inner.len() <= outer.len() { inner } else { outer } {
            r[k] = x;
        }
    }
    r
}

/// Along a bevel at step `t` (0: the face's edge, 1: the full outline): how far in and how deep.
fn profile(style: u32, t: f64) -> (f64, f64) {
    let th = t * std::f64::consts::FRAC_PI_2;
    match style {
        2 => (th.cos(), th.sin()),
        3 => (1.0 - th.sin(), 1.0 - th.cos()),
        _ => (1.0 - t, t),
    }
}

/// The solid `buf` (a layer's picture) makes as `g` says; none when it shows nothing.
pub(crate) fn solid(buf: &Buf, g: &Geometry, linear: bool) -> Option<Model> {
    let (w, h) = (buf.img.width, buf.img.height);
    if w == 0 || h == 0 || g.depth <= 0.0 {
        return None;
    }
    let field: Vec<f32> = buf.img.px.iter().map(|p| p[3]).collect();
    let traced = trace(&field, w, h, &Trace { threshold: 0.5, tolerance: 0.5, min_area: 2.0, corner_roundness: 0.0 });
    if traced.is_empty() {
        return None;
    }
    let to_layer = |p: [f64; 2]| [buf.origin.x + p[0] / buf.scale, buf.origin.y + p[1] / buf.scale];
    // each outline (lying in an even number of others) with its holes (one deeper, inside it)
    let rings: Vec<(Vec<[f64; 2]>, usize)> = traced.iter().map(|t| (t.points.iter().map(|p| to_layer(*p)).collect(), t.depth)).collect();
    let inside = |p: [f64; 2], poly: &[[f64; 2]]| {
        let mut odd = false;
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
                odd = !odd;
            }
        }
        odd
    };
    let mut shapes: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
    for (i, (r, d)) in rings.iter().enumerate() {
        if d % 2 == 1 {
            continue;
        }
        let mut outer = r.clone();
        if area(&outer) < 0.0 {
            outer.reverse();
        }
        let mut shape = vec![outer];
        for (j, (hr, hd)) in rings.iter().enumerate() {
            if j != i && *hd == d + 1 && inside(hr[0], &rings[i].0) {
                let mut hole = hr.clone();
                if area(&hole) > 0.0 {
                    hole.reverse();
                }
                shape.push(hole);
            }
        }
        shapes.push(shape);
    }
    // the bevel, no deeper than half the solid
    let bevel = if g.style == 0 { 0.0 } else { g.bevel.clamp(0.0, g.depth / 2.0) };
    let steps = match g.style {
        0 => 0,
        1 => 1,
        _ => 6,
    };
    // the picture, its colours straight, for the faces; its edges' colours for the sides
    let tex = Texture { width: w, height: h, px: buf.img.px.iter().map(|p| if p[3] > 1e-6 { [p[0] / p[3], p[1] / p[3], p[2] / p[3], 1.0] } else { [0.0, 0.0, 0.0, 0.0] }).collect(), wrap: [veymelo_model::Wrap::Clamp; 2], smooth: true };
    let uv = |p: [f64; 2]| [((p[0] - buf.origin.x) * buf.scale / w as f64) as f32, ((p[1] - buf.origin.y) * buf.scale / h as f64) as f32];
    let edge_color = |p: [f64; 2]| -> [f32; 4] {
        let c = tex.at(uv(p)[0] as f64, uv(p)[1] as f64);
        let lin = |v: f32| if linear { v } else { srgb_to_linear(v.max(0.0) as f64) as f32 };
        [lin(c[0]), lin(c[1]), lin(c[2]), 1.0]
    };
    let mut faces = Primitive { material: Some(0), ..Default::default() };
    let mut sides = Primitive { material: Some(1), ..Default::default() };
    let push = |p: &mut Primitive, pos: [f64; 3], n: [f64; 3]| -> u32 {
        p.positions.push(pos.map(|v| v as f32));
        p.normals.push(n.map(|v| v as f32));
        (p.positions.len() - 1) as u32
    };
    for shape in &shapes {
        // how far in each ring is at the face (holes by their own bevel)
        let at_face: Vec<Vec<[f64; 2]>> = shape.iter().enumerate().map(|(k, r)| untangle(inset(r, bevel * if k == 0 { 1.0 } else { g.hole_bevel }))).collect();
        // the front face at depth 0 facing the viewer, the back one at the depth facing away
        let tris = triangulate(&at_face);
        let flat: Vec<[f64; 2]> = at_face.iter().flatten().copied().collect();
        for (z, n, flip) in [(0.0, [0.0, 0.0, -1.0], false), (g.depth, [0.0, 0.0, 1.0], true)] {
            let base = faces.positions.len() as u32;
            for p in &flat {
                push(&mut faces, [p[0], p[1], z], n);
                faces.uvs[0].push(uv(*p));
            }
            for t in &tris {
                faces.indices.extend(if flip { [base + t[0], base + t[2], base + t[1]] } else { [base + t[0], base + t[1], base + t[2]] });
            }
        }
        // round each ring: the rings of the front bevel, the sides, the rings of the back bevel
        for (k, r) in shape.iter().enumerate() {
            let b = bevel * if k == 0 { 1.0 } else { g.hole_bevel };
            let mut profile_rings: Vec<(Vec<[f64; 2]>, f64)> = Vec::new();
            for s in 0..=steps {
                let (i, z) = profile(g.style, s as f64 / steps.max(1) as f64);
                profile_rings.push((untangle(inset(r, b * i)), b * z));
            }
            if steps == 0 {
                profile_rings = vec![(r.clone(), 0.0)];
            }
            let back: Vec<(Vec<[f64; 2]>, f64)> = profile_rings.iter().rev().map(|(ring, z)| (ring.clone(), g.depth - z)).collect();
            let all: Vec<(Vec<[f64; 2]>, f64)> = profile_rings.into_iter().chain(back).collect();
            let n = r.len();
            let colors: Vec<[f32; 4]> = inset(r, 1.5).iter().map(|p| edge_color(*p)).collect();
            // each band between two rings, its faces' normals (out from the solid: to the right of the ring's way)
            for pair in all.windows(2) {
                let ((ra, za), (rb, zb)) = (&pair[0], &pair[1]);
                if (za - zb).abs() < 1e-9 && ra == rb {
                    continue;
                }
                for i in 0..n {
                    let j = (i + 1) % n;
                    let q = [[ra[i][0], ra[i][1], *za], [ra[j][0], ra[j][1], *za], [rb[j][0], rb[j][1], *zb], [rb[i][0], rb[i][1], *zb]];
                    let (u, v) = ([q[1][0] - q[0][0], q[1][1] - q[0][1], q[1][2] - q[0][2]], [q[3][0] - q[0][0], q[3][1] - q[0][1], q[3][2] - q[0][2]]);
                    let mut nn = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                    let l = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt();
                    if l < 1e-12 {
                        continue;
                    }
                    nn = nn.map(|c| c / l);
                    let ids = q.map(|p| push(&mut sides, p, nn));
                    for ci in [i, j, j, i] {
                        sides.colors.push(colors[ci]);
                    }
                    sides.indices.extend([ids[0], ids[1], ids[2], ids[0], ids[2], ids[3]]);
                }
            }
        }
    }
    smooth(&mut sides, 40f64.to_radians());
    let material = |m: Option<Map>| Material { metallic: 0.0, roughness: 0.55, base_map: m, two_sided: true, ..Material::default() };
    Some(Model {
        meshes: vec![Mesh { name: "Solid".into(), primitives: vec![faces, sides], weights: Vec::new() }],
        nodes: vec![Node { mesh: Some(0), ..Node::default() }],
        roots: vec![0],
        materials: vec![material(Some(Map { texture: 0, set: 0 })), material(None)],
        textures: vec![tex],
        ..Model::default()
    })
}

/// Each vertex's normal made the mean of those at the same place whose faces turn less than
/// `angle` from it (so curves look round and corners stay sharp).
fn smooth(p: &mut Primitive, angle: f64) {
    use std::collections::HashMap;
    let key = |v: [f32; 3]| (v[0].to_bits(), v[1].to_bits(), v[2].to_bits());
    let mut at: HashMap<(u32, u32, u32), Vec<usize>> = HashMap::new();
    for (i, v) in p.positions.iter().enumerate() {
        at.entry(key(*v)).or_default().push(i);
    }
    let cos = angle.cos() as f32;
    let old = p.normals.clone();
    for (i, v) in p.positions.iter().enumerate() {
        let own = old[i];
        let mut sum = [0.0f32; 3];
        for &j in &at[&key(*v)] {
            let o = old[j];
            if own[0] * o[0] + own[1] * o[1] + own[2] * o[2] >= cos {
                (0..3).for_each(|c| sum[c] += o[c]);
            }
        }
        let l = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
        if l > 1e-9 {
            p.normals[i] = sum.map(|c| c / l);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veymelo_geom::vec2;
    use veymelo_raster::Image;

    /// A five-pointed star filled into a picture.
    fn star(w: usize) -> Buf {
        let (cx, cy) = (w as f64 / 2.0, w as f64 / 2.0);
        let poly: Vec<[f64; 2]> = (0..10)
            .map(|k| {
                let r = if k % 2 == 0 { 0.45 * w as f64 } else { 0.2 * w as f64 };
                let a = k as f64 * std::f64::consts::PI / 5.0 - std::f64::consts::FRAC_PI_2;
                [cx + r * a.cos(), cy + r * a.sin()]
            })
            .collect();
        let mut img = Image::new(w, w);
        for y in 0..w {
            for x in 0..w {
                let p = [x as f64 + 0.5, y as f64 + 0.5];
                let mut odd = false;
                for i in 0..10 {
                    let (a, b) = (poly[i], poly[(i + 1) % 10]);
                    if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
                        odd = !odd;
                    }
                }
                if odd {
                    img.px[y * w + x] = [1.0, 0.2, 0.2, 1.0];
                }
            }
        }
        Buf { img, origin: vec2(0.0, 0.0), scale: 1.0 }
    }

    fn face_area(m: &Model) -> f64 {
        let p = &m.meshes[0].primitives[0];
        p.indices.as_chunks::<3>().0.iter().take(p.indices.len() / 6).map(|t| {
            let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
            (((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) as f64 / 2.0).abs()
        }).sum()
    }

    /// A thick C: a ring with a gap, its inside a hollow curve (where moving in makes loops).
    fn c_shape(w: usize) -> Buf {
        let mut img = Image::new(w, w);
        let c = w as f64 / 2.0;
        for y in 0..w {
            for x in 0..w {
                let (dx, dy) = (x as f64 + 0.5 - c, y as f64 + 0.5 - c);
                let (r, a) = (dx.hypot(dy), dy.atan2(dx).to_degrees());
                if (0.15 * w as f64..0.42 * w as f64).contains(&r) && a.abs() > 35.0 {
                    img.px[y * w + x] = [0.9, 0.7, 0.1, 1.0];
                }
            }
        }
        Buf { img, origin: vec2(0.0, 0.0), scale: 1.0 }
    }

    #[test]
    fn hollow_curves_bevelled_without_spilling() {
        let b = c_shape(200);
        let inked = b.img.px.iter().filter(|p| p[3] > 0.5).count() as f64;
        for style in [1, 2, 3] {
            let m = solid(&b, &Geometry { style, bevel: 14.0, hole_bevel: 1.0, depth: 40.0 }, false).unwrap();
            let faces = face_area(&m);
            assert!(faces <= inked * 1.01 && faces > inked * 0.3, "style {style}: {faces} of {inked}");
        }
    }

    #[test]
    fn a_star_made_solid_keeps_its_shape() {
        let b = star(200);
        let inked = b.img.px.iter().filter(|p| p[3] > 0.5).count() as f64;
        for style in [0, 1, 3] {
            let m = solid(&b, &Geometry { style, bevel: 6.0, hole_bevel: 1.0, depth: 30.0 }, false).unwrap();
            let faces = face_area(&m);
            // the front face no bigger than the star (smaller by its bevel), every triangle in it
            assert!(faces <= inked * 1.02 && faces > inked * if style == 0 { 0.95 } else { 0.6 }, "style {style}: {faces} of {inked}");
        }
    }
}
