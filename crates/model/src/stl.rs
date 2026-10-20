//! STL meshes (3D printing and CAD), binary and text, read from 3D Systems' description of the
//! format and its common colour extensions.
//!
//! Binary: an 80-byte header, the number of facets, then 50 bytes a facet (a normal, three
//! corners, a 16-bit attribute). Text: `solid`, then `facet normal …`, `outer loop`, `vertex …`
//! corners, `endloop`, `endfacet`, `endsolid` (a loop of more corners is cut into a fan). A binary
//! file's colours: with `COLOR=` in the header (Materialise Magics) that is the part's colour, and
//! a facet whose attribute's top bit is clear has its own (red in the low five bits); without it
//! (VisCAM, SolidView) a facet with the top bit set has its own (blue in the low bits). Each
//! colour is a plain material; a file without colours has none.
//!
//! STL has no units, no texture points and no shared corners: each facet keeps its own, facing
//! the way its corners turn (the stored normal only for a facet with no area), so it shows
//! faceted. Files are Z-up; they are turned to the +Y-up of models (x, z, −y: a turn, so the
//! corners keep their order).

use std::collections::HashMap;

use crate::{Material, Mesh, Model, Node, Primitive, cross, sub, unit};

/// The header and the facet count.
const HEADER: usize = 84;
/// A binary facet's bytes.
const FACET: usize = 50;

/// An STL file's model (binary or text, told apart by its contents).
pub fn read(bytes: &[u8]) -> Result<Model, String> {
    let count = bytes.get(80..HEADER).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let needed = count.and_then(|n| (n as usize).checked_mul(FACET)).and_then(|n| n.checked_add(HEADER));
    // a binary file is exactly its facets long (many binary headers start with "solid" too)
    if needed == Some(bytes.len()) {
        return binary(bytes);
    }
    if looks_like_text(bytes) {
        return text(bytes);
    }
    if needed.is_some_and(|n| n <= bytes.len()) {
        return binary(bytes);
    }
    Err("not an STL file: its facets are cut short, and it does not start with \"solid\"".into())
}

fn looks_like_text(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_string();
    let t = head.trim_start();
    t.get(..5).is_some_and(|k| k.eq_ignore_ascii_case("solid")) && (t.contains("facet") || t.contains("endsolid"))
}

/// Z-up turned to Y-up.
fn y_up(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[2], -v[1]]
}

/// sRGB to linear light.
fn linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Facets gathered into primitives by colour, in the order the colours come.
#[derive(Default)]
struct Build {
    prims: Vec<(Option<[u8; 3]>, Primitive)>,
    slots: HashMap<Option<[u8; 3]>, usize>,
    facets: usize,
}

impl Build {
    /// One facet (the file's corners), `stored` its normal as the file gives it.
    fn facet(&mut self, color: Option<[u8; 3]>, stored: [f32; 3], corners: [[f32; 3]; 3]) {
        if corners.iter().flatten().any(|c| !c.is_finite()) {
            return;
        }
        let v = corners.map(y_up);
        let n = unit(cross(sub(v[1], v[0]), sub(v[2], v[0]))).or_else(|| unit(y_up(stored)).filter(|n| n.iter().all(|c| c.is_finite()))).unwrap_or([0.0, 1.0, 0.0]);
        let slot = *self.slots.entry(color).or_insert_with(|| {
            self.prims.push((color, Primitive::default()));
            self.prims.len() - 1
        });
        let p = &mut self.prims[slot].1;
        for c in v {
            p.indices.push(p.positions.len() as u32);
            p.positions.push(c);
            p.normals.push(n);
        }
        self.facets += 1;
    }

    fn finish(self, name: String) -> Result<Model, String> {
        if self.facets == 0 {
            return Err("the STL file has no facets".into());
        }
        let plain = |name: String, base: [f32; 4]| Material { name, base, metallic: 0.0, roughness: 0.5, ..Material::default() };
        let mut materials = Vec::new();
        let mut primitives = Vec::new();
        for (color, mut p) in self.prims {
            if let Some(c) = color {
                let l = c.map(|v| linear(v as f32 / 255.0));
                materials.push(plain(format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]), [l[0], l[1], l[2], 1.0]));
                p.material = Some(materials.len() - 1);
            }
            primitives.push(p);
        }
        // facets without a colour beside ones with: a grey of their own
        if !materials.is_empty() {
            for p in primitives.iter_mut().filter(|p| p.material.is_none()) {
                materials.push(plain("STL".into(), [0.6, 0.6, 0.6, 1.0]));
                p.material = Some(materials.len() - 1);
            }
        }
        Ok(Model {
            meshes: vec![Mesh { name: name.clone(), primitives, weights: Vec::new() }],
            nodes: vec![Node { name, mesh: Some(0), ..Node::default() }],
            roots: vec![0],
            materials,
            ..Model::default()
        })
    }
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    b.get(at..at + 4).map_or(f32::NAN, |s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Five bits made eight.
fn five(x: u16) -> u8 {
    let x = (x & 31) as u8;
    (x << 3) | (x >> 2)
}

/// The part's colour a Magics header gives (`COLOR=` and four bytes).
fn header_color(header: &[u8]) -> Option<[u8; 3]> {
    let at = header.windows(6).position(|w| w == b"COLOR=")?;
    let c = header.get(at + 6..at + 10)?;
    Some([c[0], c[1], c[2]])
}

fn binary(bytes: &[u8]) -> Result<Model, String> {
    let header = bytes.get(..80).ok_or("the STL header is cut short")?;
    let magics = header_color(header);
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    let mut b = Build::default();
    for f in bytes[HEADER..].as_chunks::<FACET>().0.iter().take(count) {
        let n = [f32_at(f, 0), f32_at(f, 4), f32_at(f, 8)];
        let corners = [0usize, 1, 2].map(|k| [f32_at(f, 12 + 12 * k), f32_at(f, 16 + 12 * k), f32_at(f, 20 + 12 * k)]);
        let attr = u16::from_le_bytes([f[48], f[49]]);
        let color = match magics {
            Some(part) => Some(if attr & 0x8000 == 0 { [five(attr), five(attr >> 5), five(attr >> 10)] } else { part }),
            None => (attr & 0x8000 != 0).then(|| [five(attr >> 10), five(attr >> 5), five(attr)]),
        };
        b.facet(color, n, corners);
    }
    let text = String::from_utf8_lossy(header);
    let name = text.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    b.finish(name.strip_prefix("solid").unwrap_or(name).trim().to_string())
}

/// Three numbers from what is left of a line.
fn three<'a>(it: &mut impl Iterator<Item = &'a str>) -> Option<[f32; 3]> {
    Some([it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?])
}

fn text(bytes: &[u8]) -> Result<Model, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut b = Build::default();
    let mut name: Option<String> = None;
    let mut normal = [0.0f32; 3];
    let mut corners: Vec<[f32; 3]> = Vec::new();
    for (ln, line) in text.lines().enumerate() {
        let line = line.trim();
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { continue };
        match key.to_ascii_lowercase().as_str() {
            "solid" if name.is_none() => name = Some(line.get(5..).unwrap_or("").trim().to_string()),
            "facet" => {
                normal = it.next().and_then(|_| three(&mut it)).unwrap_or([0.0; 3]);
                corners.clear();
            }
            "vertex" => corners.push(three(&mut it).ok_or_else(|| format!("STL line {}: a vertex needs three numbers", ln + 1))?),
            "endloop" | "endfacet" => {
                if let Some((&first, rest)) = corners.split_first() {
                    for w in rest.windows(2) {
                        b.facet(None, normal, [first, w[0], w[1]]);
                    }
                }
                corners.clear();
            }
            _ => {}
        }
    }
    b.finish(name.unwrap_or_default())
}
