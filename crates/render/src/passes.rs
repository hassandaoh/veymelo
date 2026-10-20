//! A composition's render passes, as a 3D render writes them beside its picture, for the 3D
//! Channel effects on a layer that shows it: the depth from the camera (`Z`), each pixel's
//! layer by its number in the composition (`ObjectID`, 1 the top) and its source by its item's
//! (`MaterialID`), how much of the pixel is covered (`Coverage`), the surface's normal in the
//! camera's space (`N.X/Y/Z`, facing the camera), where on the layer it is (`UV.U/V`), and
//! Cryptomatte by layer names (`CryptoObject`) and by source names, a model's own materials by
//! theirs (`CryptoMaterial`), four ranks each, with their manifests.
//!
//! 3D layers (our planes, extruded text and shapes, primitives and models) are where the
//! camera sees them; 2D layers lie flat where the composition's plane is; a layer above
//! another is in front of it, as it is drawn. Each pixel's pieces are taken from the front:
//! its depth, numbers, normal and place are the first piece that covers at least half of it
//! (else the one that shows most).

use veymelo_geom::{Mat3, vec2, vec3};
use veymelo_project::{Comp, ItemKind, Layer, Source};
use veymelo_raster::Image;
use veymelo_raster::passes::{FAR, Passes, crypto_id};
use veymelo_time::Tick;

use crate::{Renderer, scene};

/// A piece of a layer at a pixel.
#[derive(Clone, Copy)]
struct Piece {
    z: f32,
    a: f32,
    object: f32,
    material: f32,
    normal: [f32; 3],
    uv: [f32; 2],
    object_id: u32,
    material_id: u32,
}

/// The Cryptomatte ranks kept of each kind.
const RANKS: usize = 4;

/// Names with their Cryptomatte IDs, as a manifest keeps them.
type Named = Vec<(String, u32)>;

/// A run member's numbers and names: its layer's number (the one in this composition it is
/// in), its source's, and the IDs of its name, its model's materials and its source's name.
struct Member {
    object: f32,
    material: f32,
    object_id: u32,
    own: Vec<u32>,
    material_id: u32,
}

impl Renderer<'_> {
    /// The passes of `comp` at its time `t`, `scale` of their pixels to its pixels.
    pub(crate) fn comp_passes(&self, comp: &Comp, t: Tick, scale: f64, depth: usize) -> Passes {
        let (w, h) = (((comp.width as f64 * scale).round() as usize).max(1), ((comp.height as f64 * scale).round() as usize).max(1));
        let view = Mat3::scale(vec2(scale, scale));
        let shown: Vec<&Layer> = self.shown(comp, t, depth).into_iter().filter(|l| !l.switches.adjustment && !matches!(l.source, Source::Camera | Source::Light(_) | Source::Null)).collect();
        // names for Cryptomatte, kept in their manifests
        let (mut objects, mut materials): (Named, Named) = (Vec::new(), Vec::new());
        let named = |list: &mut Named, name: &str| -> u32 {
            let id = crypto_id(name);
            if !list.iter().any(|(n, _)| n == name) {
                list.push((name.to_string(), id));
            }
            id
        };
        let number = |l: &Layer| comp.layers.iter().position(|x| x.id == l.id).map_or(0.0, |i| i as f32 + 1.0);
        let source_of = |l: &Layer| match l.source {
            Source::Footage(i) | Source::Comp(i) | Source::Model(i) => self.project.item(i).map(|it| (it.name.clone(), i as f32)),
            _ => None,
        };
        // where the composition's plane is, as the camera sees its middle: where 2D layers lie
        let cam = scene::camera(&self.eval, comp, t);
        let flat_z = cam.view.apply(vec3(comp.width as f64 / 2.0, comp.height as f64 / 2.0, 0.0)).z as f32;
        // runs of 3D layers and lone 2D ones, bottom first (as they are drawn)
        let mut groups: Vec<Vec<&Layer>> = Vec::new();
        for l in &shown {
            match groups.last_mut() {
                Some(g) if scene::in_3d(l) && scene::in_3d(g[0]) => g.push(l),
                _ => groups.push(vec![l]),
            }
        }
        // each pixel's pieces, the front first: the top group's first
        let mut pieces: Vec<Vec<Piece>> = vec![Vec::new(); w * h];
        for g in groups.iter().rev() {
            if !scene::in_3d(g[0]) {
                let l = g[0];
                let img = self.alone(comp, l, t, view, w, h, depth);
                let (name, mat) = source_of(l).unwrap_or_else(|| (l.name.clone(), 0.0));
                let (oid, mid) = (named(&mut objects, &l.name), named(&mut materials, &name));
                for (i, (cell, p)) in pieces.iter_mut().zip(&img.px).enumerate() {
                    if p[3] > 1e-4 {
                        let uv = [((i % w) as f32 + 0.5) / w as f32, ((i / w) as f32 + 0.5) / h as f32];
                        cell.push(Piece { z: flat_z, a: p[3].min(1.0), object: number(l), material: mat, normal: [0.0, 0.0, -1.0], uv, object_id: oid, material_id: mid });
                    }
                }
                continue;
            }
            let mut members = Vec::new();
            for l in g {
                self.members(comp, l, t, &[], &mut members);
            }
            let mut hits = vec![Vec::new(); w * h];
            let mut canvas = Image::new(w, h);
            self.run_3d(comp, &members, t, view, &mut canvas, 1.0, depth, Some(&mut hits));
            // each member's numbers and names (inside a collapsed composition: the layer of this one it is in)
            let facts: Vec<Member> = members
                .iter()
                .map(|m| {
                    let top = m.chain.first().map_or(m.layer, |(_, l)| *l);
                    let (name, mat) = source_of(m.layer).unwrap_or_else(|| (m.layer.name.clone(), 0.0));
                    // a model's own materials by their names
                    let own: Vec<u32> = match m.layer.source {
                        Source::Model(i) => match self.project.item(i).map(|it| &it.kind) {
                            Some(ItemKind::Footage(f)) => veymelo_media::model(&f.path).map(|md| md.materials.iter().map(|x| named(&mut materials, &x.name)).collect()).unwrap_or_default(),
                            _ => Vec::new(),
                        },
                        _ => Vec::new(),
                    };
                    Member { object: number(top), material: mat, object_id: named(&mut objects, &m.layer.name), own, material_id: named(&mut materials, &name) }
                })
                .collect();
            for (cell, list) in pieces.iter_mut().zip(hits) {
                for hit in list {
                    let Some(m) = facts.get(hit.member) else { continue };
                    let material_id = hit.material.and_then(|k| m.own.get(k).copied()).unwrap_or(m.material_id);
                    cell.push(Piece { z: hit.z, a: hit.a, object: m.object, material: m.material, normal: hit.normal, uv: hit.uv, object_id: m.object_id, material_id });
                }
            }
        }
        // each pixel, from the front
        const NAMES: [&str; 9] = ["Z", "ObjectID", "MaterialID", "Coverage", "N.X", "N.Y", "N.Z", "UV.U", "UV.V"];
        let mut planes: Vec<Vec<f32>> = (0..NAMES.len() + 2 * RANKS * 2).map(|_| vec![0.0; w * h]).collect();
        planes[0].fill(FAR);
        for (i, cell) in pieces.iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            let mut left = 1.0f32;
            // each ID and how much of the pixel it covers
            let mut objs: Vec<(u32, f32)> = Vec::new();
            let mut mats: Vec<(u32, f32)> = Vec::new();
            let (mut front, mut best) = (None, None::<(usize, f32)>);
            for (k, p) in cell.iter().enumerate() {
                let c = p.a * left;
                if front.is_none() && p.a >= 0.5 {
                    front = Some(k);
                }
                if best.is_none_or(|(_, b)| c > b) {
                    best = Some((k, c));
                }
                for (list, id) in [(&mut objs, p.object_id), (&mut mats, p.material_id)] {
                    match list.iter_mut().find(|(x, _)| *x == id) {
                        Some(e) => e.1 += c,
                        None => list.push((id, c)),
                    }
                }
                left *= 1.0 - p.a;
                if left <= 1e-5 {
                    break;
                }
            }
            let p = cell[front.or(best.map(|b| b.0)).unwrap_or(0)];
            for (k, v) in [p.z, p.object, p.material, 1.0 - left, p.normal[0], p.normal[1], p.normal[2], p.uv[0], p.uv[1]].into_iter().enumerate() {
                planes[k][i] = v;
            }
            for (kind, list) in [&mut objs, &mut mats].into_iter().enumerate() {
                list.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                for (rank, (id, c)) in list.iter().take(RANKS).enumerate() {
                    let at = NAMES.len() + kind * RANKS * 2 + rank * 2;
                    planes[at][i] = f32::from_bits(*id);
                    planes[at + 1][i] = *c;
                }
            }
        }
        let mut p = Passes::new(w, h, scale);
        let mut planes = planes.into_iter();
        for name in NAMES {
            p.channels.push((name.to_string(), planes.next().expect("a plane")));
        }
        for kind in ["CryptoObject", "CryptoMaterial"] {
            for file in 0..RANKS / 2 {
                for c in ["R", "G", "B", "A"] {
                    p.channels.push((format!("{kind}{file:02}.{c}"), planes.next().expect("a plane")));
                }
            }
        }
        p.manifests = vec![("CryptoMaterial".into(), materials), ("CryptoObject".into(), objects)];
        p
    }
}
