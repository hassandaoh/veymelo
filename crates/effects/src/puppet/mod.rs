//! Puppet: the layer bent like a puppet by pins on a mesh over it.
//!
//! ```text
//! Puppet                    refinement
//!   meshes/Mesh 1           seed (where on the layer it is), density, expansion, triangles
//!     pins/Puppet Pin 1     rest (where it was put), position (where it is moved to)
//!     pins/Puppet Pin 2     advanced: position, scale and rotation about itself
//!     pins/Puppet Pin 3     bend: scale and rotation about itself, its place free
//!     pins/Starch 1         stiffens the mesh round it: amount, extent
//!     pins/Overlap 1        what is in front where parts cross: inFront, extent
//! ```
//!
//! Each frame, each mesh is made over the part of the layer's pixels under its seed (the same
//! pixels make the same mesh, so it is kept), its pins become vertices of it, it is bent as
//! rigidly as it can be (see [`arap`]) to where the pins are now, and the layer's pixels are
//! drawn through its triangles, back to front. What no mesh covers is not drawn.

pub mod arap;
pub mod mesh;

use std::sync::{Arc, Mutex};

use veymelo_geom::{Vec2, vec2};
use veymelo_keyframe::{Kind, Value};
use veymelo_project::{Body, DefNode, Group, GroupDef, Node, PropDef, Unit};
use veymelo_raster::{Image, Sampling, par};

use crate::{Buf, Ctx, Spec};
pub use mesh::{Mesh, Options};

const fn num(id: &'static str, name: &'static str, v: f64) -> PropDef {
    PropDef::new(id, name, Kind::Number, Value::Number(v))
}

const fn point(id: &'static str, name: &'static str) -> PropDef {
    PropDef::new(id, name, Kind::Vec2, Value::Vec2([0.0; 2])).unit(Unit::Pixels)
}

const REST: PropDef = point("rest", "Rest").setting().doc("where on the layer the pin was put, in its pixels");
const POSITION: PropDef = point("position", "Position").spatial().doc("where the pin is moved to, in the layer's pixels");
const SCALE: PropDef = num("scale", "Scale", 100.0).range(0.0, f64::INFINITY).unit(Unit::Percent).doc("how much the mesh round it grows");
const ROTATION: PropDef = num("rotation", "Rotation", 0.0).unit(Unit::Degrees).doc("how far the mesh round it turns");
const EXTENT: PropDef = num("extent", "Extent", 15.0).range(0.0, f64::INFINITY).unit(Unit::Pixels).doc("how far round it it reaches");

pub static PIN: GroupDef = GroupDef { id: "pin", name: "Puppet Pin", body: Body::Fixed(&[DefNode::Prop(REST), DefNode::Prop(POSITION)]), doc: "holds the mesh where it is moved to" };

pub static ADVANCED_PIN: GroupDef = GroupDef {
    id: "advancedPin",
    name: "Puppet Pin",
    body: Body::Fixed(&[DefNode::Prop(REST), DefNode::Prop(POSITION), DefNode::Prop(SCALE), DefNode::Prop(ROTATION)]),
    doc: "holds the mesh where it is moved to, turned and grown about itself",
};

pub static BEND_PIN: GroupDef = GroupDef {
    id: "bendPin",
    name: "Puppet Pin",
    body: Body::Fixed(&[DefNode::Prop(REST), DefNode::Prop(SCALE), DefNode::Prop(ROTATION)]),
    doc: "turns and grows the mesh about itself, going wherever the other pins take it",
};

pub static STARCH_PIN: GroupDef = GroupDef {
    id: "starchPin",
    name: "Starch",
    body: Body::Fixed(&[
        DefNode::Prop(REST),
        DefNode::Prop(POSITION),
        DefNode::Prop(num("amount", "Amount", 50.0).range(0.0, 100.0).unit(Unit::Percent).doc("how stiff the mesh is round it")),
        DefNode::Prop(EXTENT),
    ]),
    doc: "stiffens the mesh round it, so it bends less there",
};

pub static OVERLAP_PIN: GroupDef = GroupDef {
    id: "overlapPin",
    name: "Overlap",
    body: Body::Fixed(&[
        DefNode::Prop(REST),
        DefNode::Prop(POSITION),
        DefNode::Prop(num("inFront", "In Front", 50.0).range(-100.0, 100.0).doc("where parts cross, the higher is drawn in front (the rest are at 0)")),
        DefNode::Prop(EXTENT),
    ]),
    doc: "which part is drawn in front where parts of the mesh cross",
};

/// The pins' types, in the order of [`Pin::kind`].
pub const PIN_TYPES: [&GroupDef; 5] = [&PIN, &ADVANCED_PIN, &BEND_PIN, &STARCH_PIN, &OVERLAP_PIN];

pub static PINS: GroupDef = GroupDef { id: "pins", name: "Pins", body: Body::List("puppetPins"), doc: "the pins on the mesh" };

pub static MESH: GroupDef = GroupDef {
    id: "mesh",
    name: "Mesh",
    body: Body::Fixed(&[
        DefNode::Prop(point("seed", "Seed").setting().doc("a point of the part of the layer it covers (where its first pin was put)")),
        DefNode::Prop(num("density", "Density", 50.0).range(0.0, 100.0).setting().doc("how fine it is: each 25 above 50 doubles the triangles, each 25 below halves them")),
        DefNode::Prop(num("expansion", "Expansion", 3.0).range(-100.0, 1000.0).unit(Unit::Pixels).setting().doc("how far it reaches past the layer's pixels")),
        DefNode::Prop(num("triangles", "Triangles", 350.0).range(10.0, 10_000.0).setting().doc("about how many triangles it has")),
        DefNode::Group(&PINS),
    ]),
    doc: "triangles over a part of the layer, bent by its pins",
};

pub static MESHES: GroupDef = GroupDef { id: "meshes", name: "Meshes", body: Body::List("puppetMeshes"), doc: "a mesh for each part of the layer that moves" };

pub static PUPPET: GroupDef = GroupDef {
    id: "puppet",
    name: "Puppet",
    body: Body::Fixed(&[
        DefNode::Prop(num("refinement", "Mesh Rotation Refinement", 20.0).range(0.0, 100.0).doc("how much more the parts that turn far are made to keep their shape")),
        DefNode::Group(&MESHES),
    ]),
    doc: "bends the layer like a puppet, by pins on a mesh of triangles over it",
};

pub const SPEC: Spec = Spec {
    lists: &[("puppetMeshes", &MESH), ("puppetPins", &PIN), ("puppetPins", &ADVANCED_PIN), ("puppetPins", &BEND_PIN), ("puppetPins", &STARCH_PIN), ("puppetPins", &OVERLAP_PIN)],
    ..Spec::new(&PUPPET, "distort", run)
};

/// What a pin does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinKind {
    Position,
    Advanced,
    Bend,
    Starch,
    Overlap,
}

/// A pin, read at the frame's time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pin {
    pub kind: PinKind,
    pub rest: [f64; 2],
    pub position: [f64; 2],
    /// 1 is as it was.
    pub scale: f64,
    pub rotation: f64,
    pub amount: f64,
    pub extent: f64,
    pub in_front: f64,
}

impl Pin {
    /// Whether it holds the mesh at a place.
    pub fn holds(&self) -> bool {
        matches!(self.kind, PinKind::Position | PinKind::Advanced)
    }
}

/// A mesh's settings and pins, read at the frame's time.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshAt {
    pub seed: [f64; 2],
    pub options: Options,
    pub pins: Vec<Pin>,
}

/// The instances in a list that are switched on.
pub fn groups(list: &Group) -> Vec<&Group> {
    list.children.iter().filter_map(|n| if let Node::Group(g) = n { Some(g) } else { None }).filter(|g| g.enabled).collect()
}

/// A pin, read now.
pub fn pin_at(ctx: &Ctx, g: &Group) -> Pin {
    let kind = match g.def.id {
        "advancedPin" => PinKind::Advanced,
        "bendPin" => PinKind::Bend,
        "starchPin" => PinKind::Starch,
        "overlapPin" => PinKind::Overlap,
        _ => PinKind::Position,
    };
    let rest = ctx.v2_in(g, "rest", [0.0; 2]);
    Pin {
        kind,
        rest,
        position: ctx.v2_in(g, "position", rest),
        scale: ctx.num_in(g, "scale", 100.0) / 100.0,
        rotation: ctx.num_in(g, "rotation", 0.0),
        amount: ctx.num_in(g, "amount", 50.0),
        extent: ctx.num_in(g, "extent", 15.0),
        in_front: ctx.num_in(g, "inFront", 50.0),
    }
}

/// A mesh and its pins (those switched on), read now.
pub fn mesh_at(ctx: &Ctx, m: &Group) -> MeshAt {
    MeshAt {
        seed: ctx.v2_in(m, "seed", [0.0; 2]),
        options: Options { density: ctx.num_in(m, "density", 50.0), expansion: ctx.num_in(m, "expansion", 3.0), triangles: ctx.num_in(m, "triangles", 350.0) },
        pins: m.group("pins").map(|p| groups(p).into_iter().map(|g| pin_at(ctx, g)).collect()).unwrap_or_default(),
    }
}

/// The meshes (switched on) of a puppet, read now.
pub fn read(ctx: &Ctx) -> Vec<MeshAt> {
    ctx.effect.group("meshes").map(|list| groups(list).into_iter().map(|m| mesh_at(ctx, m)).collect()).unwrap_or_default()
}

/// The most cells across the grid a mesh is found on (bigger layers are looked at coarser).
const GRID_MOST: f64 = 1024.0;

/// Meshes made lately, by what they were made from.
static MADE: Mutex<Vec<(u64, Arc<Mesh>)>> = Mutex::new(Vec::new());

fn fnv(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h = (*h ^ *b as u64).wrapping_mul(0x0100_0000_01B3);
    }
}

/// The mesh over the part of `buf`'s pixels under `m`'s seed. The pixels are looked at on a
/// grid of layer units (coarser for big layers), not at the size they were drawn, so a preview
/// drawn small bends the same way as the full picture.
pub fn mesh_for(buf: &Buf, m: &MeshAt) -> Arc<Mesh> {
    let area = buf.area();
    let cells = (GRID_MOST / area.width().max(area.height()).max(1e-9)).min(1.0);
    let (w, h) = ((area.width() * cells).ceil().max(1.0) as usize, (area.height() * cells).ceil().max(1.0) as usize);
    let pad = (m.options.expansion.max(0.0) * cells).ceil() as usize + 2;
    let (gw, gh) = (w + 2 * pad, h + 2 * pad);
    let mut on = vec![false; gw * gh];
    for y in 0..h {
        for x in 0..w {
            let p = buf.pixel(vec2(area.x0 + (x as f64 + 0.5) / cells, area.y0 + (y as f64 + 0.5) / cells));
            on[(y + pad) * gw + x + pad] = buf.img.sample(Sampling::Bilinear, p.x, p.y)[3] >= 0.5;
        }
    }
    // what the mesh is made from, as one number
    let mut key = 0xCBF2_9CE4_8422_2325u64;
    for v in [area.x0, area.y0, cells, m.seed[0], m.seed[1], m.options.density, m.options.expansion, m.options.triangles] {
        fnv(&mut key, &v.to_bits().to_le_bytes());
    }
    fnv(&mut key, &(gw as u64).to_le_bytes());
    for chunk in on.chunks(8) {
        fnv(&mut key, &[chunk.iter().fold(0u8, |b, o| (b << 1) | *o as u8)]);
    }
    for p in m.pins.iter().filter(|p| p.kind != PinKind::Starch && p.kind != PinKind::Overlap) {
        fnv(&mut key, &p.rest[0].to_bits().to_le_bytes());
        fnv(&mut key, &p.rest[1].to_bits().to_le_bytes());
    }
    if let Ok(made) = MADE.lock()
        && let Some((_, mesh)) = made.iter().find(|(k, _)| *k == key)
    {
        return mesh.clone();
    }
    let grid = mesh::grow(&mesh::Grid { w: gw, h: gh, on }, m.options.expansion * cells);
    let to_grid = |p: [f64; 2]| [(p[0] - area.x0) * cells + pad as f64, (p[1] - area.y0) * cells + pad as f64];
    let to_layer = |p: [f64; 2]| [(p[0] - pad as f64) / cells + area.x0, (p[1] - pad as f64) / cells + area.y0];
    let outline = match mesh::region(&grid, to_grid(m.seed)) {
        Some(part) => mesh::simplify(&mesh::trace(&part), 0.75).into_iter().map(to_layer).collect(),
        None => Vec::new(),
    };
    let pins: Vec<[f64; 2]> = m.pins.iter().filter(|p| p.kind != PinKind::Starch && p.kind != PinKind::Overlap).map(|p| p.rest).collect();
    let made = Arc::new(mesh::triangulate(&outline, &pins, &m.options));
    if let Ok(mut all) = MADE.lock() {
        all.push((key, made.clone()));
        if all.len() > 32 {
            all.remove(0);
        }
    }
    made
}

fn nearest(verts: &[[f64; 2]], p: [f64; 2]) -> Option<usize> {
    (0..verts.len()).min_by(|a, b| {
        let d = |i: usize| (verts[i][0] - p[0]).powi(2) + (verts[i][1] - p[1]).powi(2);
        d(*a).total_cmp(&d(*b))
    })
}

fn middle(mesh: &Mesh, t: &[usize; 3]) -> [f64; 2] {
    let v = &mesh.verts;
    [(v[t[0]][0] + v[t[1]][0] + v[t[2]][0]) / 3.0, (v[t[0]][1] + v[t[1]][1] + v[t[2]][1]) / 3.0]
}

/// How stiff each triangle is: 1, more near starch pins.
pub fn stiffness(mesh: &Mesh, pins: &[Pin]) -> Vec<f64> {
    mesh.tris
        .iter()
        .map(|t| {
            let c = middle(mesh, t);
            1.0 + pins
                .iter()
                .filter(|p| p.kind == PinKind::Starch)
                .map(|p| {
                    let d = ((c[0] - p.rest[0]).powi(2) + (c[1] - p.rest[1]).powi(2)).sqrt();
                    (1.0 - d / (2.0 * p.extent.max(0.0) + 1e-9)).clamp(0.0, 1.0) * p.amount.max(0.0) * 2.0
                })
                .sum::<f64>()
        })
        .collect()
}

/// Where the mesh's vertices go for its pins as they are now.
pub fn bent(mesh: &Mesh, pins: &[Pin], refine: usize) -> Vec<[f64; 2]> {
    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); mesh.verts.len()];
    for t in &mesh.tris {
        for e in 0..3 {
            let (a, b) = (t[e], t[(e + 1) % 3]);
            if !neighbours[a].contains(&b) {
                neighbours[a].push(b);
            }
            if !neighbours[b].contains(&a) {
                neighbours[b].push(a);
            }
        }
    }
    let mut holds = arap::Holds::default();
    let mut taken = Vec::new();
    for p in pins {
        let Some(v) = nearest(&mesh.verts, p.rest) else { continue };
        let r = mesh.verts[v];
        // a pin off its vertex (outside the mesh) moves the vertex by as much as it moves
        if p.holds() && !taken.contains(&v) {
            taken.push(v);
            holds.at.push((v, [p.position[0] + r[0] - p.rest[0], p.position[1] + r[1] - p.rest[1]]));
        }
        if matches!(p.kind, PinKind::Advanced | PinKind::Bend) {
            let (s, c) = p.rotation.to_radians().sin_cos();
            for &j in &neighbours[v] {
                let e = [mesh.verts[j][0] - r[0], mesh.verts[j][1] - r[1]];
                holds.sides.push(arap::Side { i: v, j, to: [p.scale * (e[0] * c - e[1] * s), p.scale * (e[0] * s + e[1] * c)], weight: 10.0 });
            }
        }
    }
    if holds.at.is_empty() && holds.sides.is_empty() {
        return mesh.verts.clone();
    }
    arap::bend(&mesh.verts, &mesh.tris, &stiffness(mesh, pins), &holds, refine)
}

/// How far in front each triangle is (overlap pins): 0, or the most of the pins that reach it.
pub fn depths(mesh: &Mesh, pins: &[Pin]) -> Vec<f64> {
    mesh.tris
        .iter()
        .map(|t| {
            let c = middle(mesh, t);
            pins.iter()
                .filter(|p| p.kind == PinKind::Overlap && ((c[0] - p.rest[0]).powi(2) + (c[1] - p.rest[1]).powi(2)).sqrt() <= p.extent.max(0.0) + 1e-9)
                .map(|p| p.in_front)
                .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))))
                .unwrap_or(0.0)
        })
        .collect()
}

/// Where `p` is in triangle `a b c`, as weights of its corners (None for a flat triangle).
fn weights_in(a: Vec2, b: Vec2, c: Vec2, p: Vec2) -> Option<[f64; 3]> {
    let d = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
    if d.abs() < 1e-12 {
        return None;
    }
    let l0 = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / d;
    let l1 = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / d;
    Some([l0, l1, 1.0 - l0 - l1])
}

/// A triangle to draw: its corners in the source's pixels and in the result's, back to front.
pub(crate) struct Piece {
    from: [Vec2; 3],
    to: [Vec2; 3],
    rows: (f64, f64),
    cols: (f64, f64),
}

impl Piece {
    pub(crate) fn new(from: [Vec2; 3], to: [Vec2; 3]) -> Piece {
        Piece { from, rows: (to[0].y.min(to[1].y).min(to[2].y), to[0].y.max(to[1].y).max(to[2].y)), cols: (to[0].x.min(to[1].x).min(to[2].x), to[0].x.max(to[1].x).max(to[2].x)), to }
    }
}

/// Where each pixel is looked at for how much the triangles cover it.
const SUBS: [(f64, f64); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];

/// `src` drawn through `pieces` into a `w` × `h` picture: each pixel takes the colour of the
/// front-most triangle over its middle (or, at an edge, over any of its sample points), as much
/// as the triangles together cover it, so the triangles meet without seams.
pub(crate) fn draw(src: &Image, pieces: &[Piece], w: usize, h: usize) -> Image {
    let mut out = Image::new(w, h);
    par::rows(&mut out.px, w, |y, row| {
        let fy = y as f64;
        let (mut cover, mut front, mut any) = (vec![0u8; w], vec![usize::MAX; w], vec![usize::MAX; w]);
        for (k, p) in pieces.iter().enumerate() {
            if p.rows.1 < fy || p.rows.0 > fy + 1.0 {
                continue;
            }
            let (x0, x1) = ((p.cols.0.floor().max(0.0)) as usize, (p.cols.1.ceil().max(0.0) as usize + 1).min(w));
            for x in x0..x1 {
                let fx = x as f64;
                let on = |dx: f64, dy: f64| weights_in(p.to[0], p.to[1], p.to[2], vec2(fx + dx, fy + dy)).is_some_and(|l| l.iter().all(|v| *v >= -1e-9));
                for (s, (dx, dy)) in SUBS.iter().enumerate() {
                    if on(*dx, *dy) {
                        cover[x] |= 1 << s;
                        any[x] = k;
                    }
                }
                if on(0.5, 0.5) {
                    front[x] = k;
                }
            }
        }
        for (x, px) in row.iter_mut().enumerate() {
            let k = if front[x] != usize::MAX { front[x] } else { any[x] };
            if k == usize::MAX {
                continue;
            }
            let p = &pieces[k];
            let at = vec2(x as f64 + 0.5, fy + 0.5);
            let Some(l) = weights_in(p.to[0], p.to[1], p.to[2], at) else { continue };
            let from = p.from[0] * l[0] + p.from[1] * l[1] + p.from[2] * l[2];
            // the edges are soft by how much the triangles cover, so the picture's own edge
            // pixels go on rather than fading again
            let c = src.sample(Sampling::Bilinear, from.x.clamp(0.5, src.width as f64 - 0.5), from.y.clamp(0.5, src.height as f64 - 0.5));
            let k = if front[x] != usize::MAX && cover[x] == 0b1111 { 1.0 } else { cover[x].count_ones().max(1) as f32 / 4.0 };
            *px = c.map(|v| v * k);
        }
    });
    out
}

fn run(ctx: &Ctx, buf: Buf) -> Buf {
    let meshes = read(ctx);
    if meshes.is_empty() {
        return buf;
    }
    let refine = (ctx.num("refinement", 20.0) / 10.0).round().clamp(0.0, 10.0) as usize;
    let mut tris: Vec<([Vec2; 3], [Vec2; 3], f64)> = Vec::new();
    for m in &meshes {
        let mesh = mesh_for(&buf, m);
        if mesh.tris.is_empty() {
            ctx.note(format!("a puppet mesh at [{}, {}] finds nothing of the layer to cover", m.seed[0], m.seed[1]));
            continue;
        }
        let moved = bent(&mesh, &m.pins, refine);
        let depth = depths(&mesh, &m.pins);
        let v = |p: [f64; 2]| vec2(p[0], p[1]);
        for (k, t) in mesh.tris.iter().enumerate() {
            tris.push((t.map(|i| v(mesh.verts[i])), t.map(|i| v(moved[i])), depth[k]));
        }
    }
    if tris.is_empty() {
        return Buf { img: Image::new(1, 1), origin: buf.origin, scale: buf.scale };
    }
    // back to front; the same depth keeps its order
    tris.sort_by(|a, b| a.2.total_cmp(&b.2));
    // the result covers where the triangles went, in the same pixels to a unit
    let (mut lo, mut hi) = (vec2(f64::MAX, f64::MAX), vec2(f64::MIN, f64::MIN));
    for (_, to, _) in &tris {
        for p in to {
            lo = vec2(lo.x.min(p.x), lo.y.min(p.y));
            hi = vec2(hi.x.max(p.x), hi.y.max(p.y));
        }
    }
    let s = buf.scale;
    let origin = vec2((lo.x * s).floor() / s, (lo.y * s).floor() / s);
    let (w, h) = ((((hi.x - origin.x) * s).ceil() as usize + 1).min(16_384), (((hi.y - origin.y) * s).ceil() as usize + 1).min(16_384));
    let pieces: Vec<Piece> = tris
        .iter()
        .map(|(from, to, _)| Piece::new(from.map(|p| buf.pixel(p)), to.map(|p| (p - origin) * s)))
        .collect();
    let img = draw(&buf.img, &pieces, w, h);
    Buf { img, origin, scale: s }
}

#[cfg(test)]
mod tests;
