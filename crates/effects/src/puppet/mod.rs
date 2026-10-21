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
//! A mesh covers the part of the layer it was made over: its outline, kept from when it was made
//! (so the mesh stays the same while the layer's picture changes under it), or, for one without
//! an outline, the part of the layer's pixels under its seed each frame (the same pixels make
//! the same mesh, so it is kept). Its pins become vertices of it, it is bent as rigidly as it
//! can be (see [`arap`]) to where the pins are now, and the layer's pixels are drawn through its
//! triangles, back to front. What no mesh covers is not drawn.

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
        DefNode::Prop(
            PropDef::new("outline", "Outline", Kind::Path, Value::Path(veymelo_keyframe::ShapePath { points: Vec::new(), in_tangents: Vec::new(), out_tangents: Vec::new(), closed: true, feather: veymelo_keyframe::Feathers::NONE }))
                .setting()
                .doc("the part of the layer it covers, in the layer's pixels, as it was when the mesh was made (empty: the part under its seed, each frame)"),
        ),
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
    /// The part of the layer it covers, kept from when it was made (empty: none kept).
    pub outline: Vec<[f64; 2]>,
    pub pins: Vec<Pin>,
}

/// The instances in a list that are switched on.
pub fn groups(list: &Group) -> Vec<&Group> {
    list.children.iter().filter_map(|n| if let Node::Group(g) = n { Some(g) } else { None }).filter(|g| g.enabled).collect()
}

/// A pin, read now.
pub fn pin_at(ctx: &Ctx, g: &Group) -> Pin {
    pin_by(g, &|g, path| ctx.value_in(g, path))
}

/// A setting's value inside a group (as read at some moment).
type Get<'a> = dyn Fn(&Group, &str) -> Option<Value> + 'a;

/// A pin, its settings read by `get`.
fn pin_by(g: &Group, get: &Get) -> Pin {
    let num = |path: &str, d: f64| get(g, path).map_or(d, |v| v.as_f64());
    let v2 = |path: &str, d: [f64; 2]| get(g, path).map_or(d, |v| v.as_vec2());
    let kind = match g.def.id {
        "advancedPin" => PinKind::Advanced,
        "bendPin" => PinKind::Bend,
        "starchPin" => PinKind::Starch,
        "overlapPin" => PinKind::Overlap,
        _ => PinKind::Position,
    };
    let rest = v2("rest", [0.0; 2]);
    Pin {
        kind,
        rest,
        position: v2("position", rest),
        scale: num("scale", 100.0) / 100.0,
        rotation: num("rotation", 0.0),
        amount: num("amount", 50.0),
        extent: num("extent", 15.0),
        in_front: num("inFront", 50.0),
    }
}

/// A mesh and its pins (those switched on), read now.
pub fn mesh_at(ctx: &Ctx, m: &Group) -> MeshAt {
    mesh_by(m, &|g, path| ctx.value_in(g, path))
}

/// A mesh and its pins, their settings read by `get`.
fn mesh_by(m: &Group, get: &Get) -> MeshAt {
    let num = |path: &str, d: f64| get(m, path).map_or(d, |v| v.as_f64());
    MeshAt {
        seed: get(m, "seed").map_or([0.0; 2], |v| v.as_vec2()),
        options: Options { density: num("density", 50.0), expansion: num("expansion", 3.0), triangles: num("triangles", 350.0) },
        outline: match get(m, "outline") {
            Some(Value::Path(p)) => p.points,
            _ => Vec::new(),
        },
        pins: m.group("pins").map(|p| groups(p).into_iter().map(|g| pin_by(g, get)).collect()).unwrap_or_default(),
    }
}

/// The meshes (switched on) of a puppet, read now.
pub fn read(ctx: &Ctx) -> Vec<MeshAt> {
    ctx.effect.group("meshes").map(|list| groups(list).into_iter().map(|m| mesh_at(ctx, m)).collect()).unwrap_or_default()
}

/// The same read at layer time `t` (as now when the renderer cannot say).
fn read_at(ctx: &Ctx, t: veymelo_time::Tick) -> Vec<MeshAt> {
    let get = |g: &Group, path: &str| {
        g.prop(path).ok().map(|p| match ctx.read_at {
            Some(f) => f(p, t),
            None => (ctx.read)(p),
        })
    };
    ctx.effect.group("meshes").map(|list| groups(list).into_iter().map(|m| mesh_by(m, &get)).collect()).unwrap_or_default()
}

/// The most cells across the grid a mesh is found on (bigger layers are looked at coarser).
const GRID_MOST: f64 = 1024.0;

/// How much the turn and scale a bend or advanced pin asks of the edges round it count against
/// the mesh keeping its shape: ten times an edge's own, so the pin's turn wins where it is and
/// eases out into the mesh (see bend_pins_turn_about_themselves).
const TURN_WEIGHT: f64 = 10.0;

/// Meshes made lately, by what they were made from.
static MADE: Mutex<Vec<(u64, Arc<Mesh>)>> = Mutex::new(Vec::new());

fn fnv(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h = (*h ^ *b as u64).wrapping_mul(0x0100_0000_01B3);
    }
}

/// The cells of a grid `gw` × `gh`, `cells` to a layer unit from layer point (`x0`, `y0`), whose
/// middles are inside polygon `poly` (a row at a time, between where its sides cross the row).
fn filled(poly: &[[f64; 2]], gw: usize, gh: usize, cells: f64, x0: f64, y0: f64) -> Vec<bool> {
    let mut on = vec![false; gw * gh];
    let n = poly.len();
    let mut cuts = Vec::new();
    for y in 0..gh {
        let ly = y0 + (y as f64 + 0.5) / cells;
        cuts.clear();
        for i in 0..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            if (a[1] <= ly) != (b[1] <= ly) {
                cuts.push(a[0] + (ly - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
            }
        }
        cuts.sort_by(f64::total_cmp);
        for pair in cuts.as_chunks::<2>().0 {
            // the cells whose middles lie between the two crossings
            let from = (((pair[0] - x0) * cells - 0.5).ceil().max(0.0)) as usize;
            let to = (((pair[1] - x0) * cells - 0.5).floor()).min(gw as f64 - 1.0);
            if to < 0.0 {
                continue;
            }
            for x in from..=to as usize {
                on[y * gw + x] = true;
            }
        }
    }
    on
}

/// The outline of the part of `buf`'s pixels under `seed`, in layer units (as a mesh made now
/// would cover it, before its expansion): kept with the mesh, so it covers the same part later.
pub fn outline_of(buf: &Buf, seed: [f64; 2]) -> Vec<[f64; 2]> {
    let area = buf.area();
    let cells = (GRID_MOST / area.width().max(area.height()).max(1e-9)).min(1.0);
    let (w, h) = ((area.width() * cells).ceil().max(1.0) as usize, (area.height() * cells).ceil().max(1.0) as usize);
    let pad = 2;
    let (gw, gh) = (w + 2 * pad, h + 2 * pad);
    let mut on = vec![false; gw * gh];
    for y in 0..h {
        for x in 0..w {
            let p = buf.pixel(vec2(area.x0 + (x as f64 + 0.5) / cells, area.y0 + (y as f64 + 0.5) / cells));
            on[(y + pad) * gw + x + pad] = buf.img.sample(Sampling::Bilinear, p.x, p.y)[3] >= 0.5;
        }
    }
    let to_grid = |p: [f64; 2]| [(p[0] - area.x0) * cells + pad as f64, (p[1] - area.y0) * cells + pad as f64];
    let to_layer = |p: [f64; 2]| [(p[0] - pad as f64) / cells + area.x0, (p[1] - pad as f64) / cells + area.y0];
    match mesh::region(&mesh::Grid { w: gw, h: gh, on }, to_grid(seed)) {
        Some(part) => mesh::simplify(&mesh::trace(&part), 0.25).into_iter().map(to_layer).collect(),
        None => Vec::new(),
    }
}

/// The mesh over the part of the layer `m` covers: its kept outline, or the part of `buf`'s
/// pixels under its seed. Either is looked at on a grid of layer units (coarser for big layers),
/// not at the size it was drawn, so a preview drawn small bends the same way as the full
/// picture.
pub fn mesh_for(buf: &Buf, m: &MeshAt) -> Arc<Mesh> {
    let kept = m.outline.len() >= 3;
    let area = if kept {
        let (lo, hi) = m.outline.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
        veymelo_geom::Rect::new(lo[0], lo[1], hi[0], hi[1])
    } else {
        buf.area()
    };
    let cells = (GRID_MOST / area.width().max(area.height()).max(1e-9)).min(1.0);
    let (w, h) = ((area.width() * cells).ceil().max(1.0) as usize, (area.height() * cells).ceil().max(1.0) as usize);
    let pad = (m.options.expansion.max(0.0) * cells).ceil() as usize + 2;
    let (gw, gh) = (w + 2 * pad, h + 2 * pad);
    let on = if kept {
        filled(&m.outline, gw, gh, cells, area.x0 - pad as f64 / cells, area.y0 - pad as f64 / cells)
    } else {
        let mut on = vec![false; gw * gh];
        for y in 0..h {
            for x in 0..w {
                let p = buf.pixel(vec2(area.x0 + (x as f64 + 0.5) / cells, area.y0 + (y as f64 + 0.5) / cells));
                on[(y + pad) * gw + x + pad] = buf.img.sample(Sampling::Bilinear, p.x, p.y)[3] >= 0.5;
            }
        }
        on
    };
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
                holds.sides.push(arap::Side { i: v, j, to: [p.scale * (e[0] * c - e[1] * s), p.scale * (e[0] * s + e[1] * c)], weight: TURN_WEIGHT });
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

/// The meshes of a puppet as they are at one moment: each triangle where it is on the layer and
/// where it is bent to (layer units) and how far in front it is, back to front; and (for a
/// preview) the meshes' sides and their pins, as bent.
#[derive(Default)]
struct Laid {
    tris: Vec<([Vec2; 3], [Vec2; 3], f64)>,
    sides: Vec<(Vec2, Vec2)>,
    dots: Vec<(Vec2, PinKind)>,
}

/// `meshes` over `buf`, bent by their pins (`refine` as the puppet's refinement says); what
/// covers nothing said in `ctx`'s notes when `say`.
fn lay(ctx: &Ctx, buf: &Buf, meshes: &[MeshAt], refine: usize, say: bool) -> Laid {
    let mut laid = Laid::default();
    for m in meshes {
        let mesh = mesh_for(buf, m);
        if mesh.tris.is_empty() {
            if say {
                ctx.note(format!("a puppet mesh at [{}, {}] finds nothing of the layer to cover", m.seed[0], m.seed[1]));
            }
            continue;
        }
        let moved = bent(&mesh, &m.pins, refine);
        let depth = depths(&mesh, &m.pins);
        let v = |p: [f64; 2]| vec2(p[0], p[1]);
        if ctx.marks {
            let mut seen = std::collections::HashSet::new();
            for t in &mesh.tris {
                for e in 0..3 {
                    let (a, b) = (t[e].min(t[(e + 1) % 3]), t[e].max(t[(e + 1) % 3]));
                    if seen.insert((a, b)) {
                        laid.sides.push((v(moved[a]), v(moved[b])));
                    }
                }
            }
            // each pin on its vertex, as the mesh is bent
            laid.dots.extend(m.pins.iter().filter_map(|p| Some((v(moved[nearest(&mesh.verts, p.rest)?]), p.kind))));
        }
        for (k, t) in mesh.tris.iter().enumerate() {
            laid.tris.push((t.map(|i| v(mesh.verts[i])), t.map(|i| v(moved[i])), depth[k]));
        }
    }
    // back to front; the same depth keeps its order
    laid.tris.sort_by(|a, b| a.2.total_cmp(&b.2));
    laid
}

/// The most pictures a moving puppet is blurred from.
const MOST_MOMENTS: usize = 64;

fn run(ctx: &Ctx, buf: Buf) -> Buf {
    let meshes = read(ctx);
    if meshes.is_empty() {
        return buf;
    }
    let refine = (ctx.num("refinement", 20.0) / 10.0).round().clamp(0.0, 10.0) as usize;
    let now = lay(ctx, &buf, &meshes, refine, true);
    if now.tris.is_empty() {
        return Buf { img: Image::new(1, 1), origin: buf.origin, scale: buf.scale };
    }
    // motion blurred: as it is at moments over the shutter, as many as its triangles' corners
    // move pixels in it (at least the composition's fewest)
    let mut moments: Vec<Laid> = Vec::new();
    if let (Some((angle, phase, least)), Some(_)) = (ctx.shutter, ctx.read_at)
        && angle > 0.0
    {
        let frame = ctx.frame.seconds();
        let when = |f: f64| ctx.time + veymelo_time::Tick::from_seconds((phase + angle * f) / 360.0 * frame);
        let at = |f: f64| lay(ctx, &buf, &read_at(ctx, when(f)), refine, false);
        let (first, last) = (at(0.0), at(1.0));
        let travel = if first.tris.len() == last.tris.len() {
            first.tris.iter().zip(&last.tris).flat_map(|(a, b)| a.1.iter().zip(&b.1).map(|(p, q)| (*p - *q).length())).fold(0.0, f64::max) * buf.scale
        } else {
            f64::INFINITY
        };
        if travel >= 0.5 {
            let n = (travel.ceil() as usize).clamp((least as usize).max(2), MOST_MOMENTS);
            moments = (0..n).map(|i| if i == 0 { Laid { tris: first.tris.clone(), ..Laid::default() } } else if i == n - 1 { Laid { tris: last.tris.clone(), ..Laid::default() } } else { at(i as f64 / (n - 1) as f64) }).collect();
        }
    }
    // the result covers where the triangles went, in the same pixels to a unit
    let (mut lo, mut hi) = (vec2(f64::MAX, f64::MAX), vec2(f64::MIN, f64::MIN));
    for laid in moments.iter().chain([&now]) {
        for (_, to, _) in &laid.tris {
            for p in to {
                lo = vec2(lo.x.min(p.x), lo.y.min(p.y));
                hi = vec2(hi.x.max(p.x), hi.y.max(p.y));
            }
        }
    }
    let s = buf.scale;
    let origin = vec2((lo.x * s).floor() / s, (lo.y * s).floor() / s);
    let (w, h) = ((((hi.x - origin.x) * s).ceil() as usize + 1).min(16_384), (((hi.y - origin.y) * s).ceil() as usize + 1).min(16_384));
    let pieces = |laid: &Laid| -> Vec<Piece> { laid.tris.iter().map(|(from, to, _)| Piece::new(from.map(|p| buf.pixel(p)), to.map(|p| (p - origin) * s))).collect() };
    let mut img = if moments.is_empty() {
        draw(&buf.img, &pieces(&now), w, h)
    } else {
        // each moment's picture, the mean of them
        let k = 1.0 / moments.len() as f32;
        let mut sum = Image::new(w, h);
        for laid in &moments {
            let one = draw(&buf.img, &pieces(laid), w, h);
            par::chunks(&mut sum.px, 1 << 14, |c, part| {
                for (o, p) in part.iter_mut().zip(&one.px[c << 14..]) {
                    (0..4).for_each(|i| o[i] += p[i] * k);
                }
            });
        }
        sum
    };
    if ctx.marks {
        let at = |p: Vec2| (p - origin) * s;
        mark(&mut img, &now.sides.iter().map(|(a, b)| (at(*a), at(*b))).collect::<Vec<_>>(), &now.dots.iter().map(|(p, k)| (at(*p), *k)).collect::<Vec<_>>());
    }
    Buf { img, origin, scale: s }
}

/// What a pin is marked with, by what it does: holding pins yellow, bend orange, starch red,
/// overlap blue.
fn pin_colour(k: PinKind) -> [f32; 3] {
    match k {
        PinKind::Position | PinKind::Advanced => [1.0, 0.85, 0.1],
        PinKind::Bend => [1.0, 0.55, 0.1],
        PinKind::Starch => [0.9, 0.2, 0.2],
        PinKind::Overlap => [0.25, 0.6, 1.0],
    }
}

/// `img` with thin yellow lines along `sides` and a dot at each pin, in its pixels, over it.
fn mark(img: &mut Image, sides: &[(Vec2, Vec2)], dots: &[(Vec2, PinKind)]) {
    let (w, h) = (img.width as i64, img.height as i64);
    let mut over = |x: i64, y: i64, c: [f32; 3], a: f32| {
        if x < 0 || y < 0 || x >= w || y >= h || a <= 0.0 {
            return;
        }
        let p = &mut img.px[(y * w + x) as usize];
        *p = [c[0] * a + p[0] * (1.0 - a), c[1] * a + p[1] * (1.0 - a), c[2] * a + p[2] * (1.0 - a), a + p[3] * (1.0 - a)];
    };
    for &(a, b) in sides {
        let d = b - a;
        let len2 = d.x * d.x + d.y * d.y;
        let (x0, x1) = ((a.x.min(b.x) - 2.0).floor() as i64, (a.x.max(b.x) + 2.0).ceil() as i64);
        let (y0, y1) = ((a.y.min(b.y) - 2.0).floor() as i64, (a.y.max(b.y) + 2.0).ceil() as i64);
        for y in y0.max(0)..=y1.min(h - 1) {
            for x in x0.max(0)..=x1.min(w - 1) {
                let q = vec2(x as f64 + 0.5, y as f64 + 0.5);
                let t = if len2 > 0.0 { ((q - a).x * d.x + (q - a).y * d.y) / len2 } else { 0.0 }.clamp(0.0, 1.0);
                let e = q - (a + d * t);
                let dist = (e.x * e.x + e.y * e.y).sqrt();
                over(x, y, [1.0, 0.85, 0.1], ((1.25 - dist).clamp(0.0, 1.0) * 0.8) as f32);
            }
        }
    }
    for &(p, k) in dots {
        let r = 4.5;
        for y in (p.y - r - 1.0).floor() as i64..=(p.y + r + 1.0).ceil() as i64 {
            for x in (p.x - r - 1.0).floor() as i64..=(p.x + r + 1.0).ceil() as i64 {
                let d = ((x as f64 + 0.5 - p.x).powi(2) + (y as f64 + 0.5 - p.y).powi(2)).sqrt();
                // a dark ring round the colour
                over(x, y, [0.08, 0.08, 0.08], (r + 0.5 - d).clamp(0.0, 1.0) as f32);
                over(x, y, pin_colour(k), (r - 1.0 - d).clamp(0.0, 1.0) as f32);
            }
        }
    }
}

#[cfg(test)]
mod tests;
