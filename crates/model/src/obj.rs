//! Wavefront OBJ: lines of vertex positions (`v`, colours may follow), texture points (`vt`),
//! normals (`vn`) and faces (`f`, of position/texture/normal numbers, counted from 1 or back
//! from the end), split into objects and groups (`o`, `g`) and materials (`usemtl`) from MTL
//! files (`mtllib`): their colour (`Kd`), opacity (`d`, or `Tr` the other way), light given
//! off (`Ke`), shininess (`Ns`, as a roughness), PBR roughness and metalness (`Pr`, `Pm`) and
//! pictures (`map_Kd`, `map_Ke`, `map_d`). Faces of more than three corners are cut into
//! triangles from their first corner. Without normals, faces are flat.

use std::collections::HashMap;
use std::path::Path;

use crate::{AlphaMode, Map, Material, Mesh, Model, Node, Primitive, Texture};

/// sRGB to linear (OBJ colours are as seen; glTF's, linear).
fn linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn numbers(rest: &[&str]) -> Vec<f32> {
    rest.iter().map_while(|v| v.parse::<f32>().ok()).collect()
}

/// The materials of MTL text, by name; pictures they name read from `dir` into `textures`.
fn materials(text: &str, dir: &Path, textures: &mut Vec<Texture>, notes: &mut Vec<String>) -> Vec<Material> {
    let mut out: Vec<Material> = Vec::new();
    let mut picture = |name: &str, textures: &mut Vec<Texture>| -> Option<Map> {
        let path = dir.join(name.replace('\\', "/"));
        match veymelo_os::read_picture(&path.to_string_lossy(), 0) {
            Ok(p) => {
                textures.push(Texture::from_picture(p));
                Some(Map { texture: textures.len() - 1, set: 0 })
            }
            Err(e) => {
                notes.push(format!("{name} cannot be read ({e})"));
                None
            }
        }
    };
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((&key, rest)) = words.split_first() else { continue };
        if key == "newmtl" {
            out.push(Material { name: rest.join(" "), metallic: 0.0, roughness: 0.6, ..Material::default() });
            continue;
        }
        let Some(m) = out.last_mut() else { continue };
        let n = numbers(rest);
        // a picture's file is its last word (options come before it)
        let file = rest.last().copied().unwrap_or("");
        match key {
            "Kd" if n.len() >= 3 => (m.base[0], m.base[1], m.base[2]) = (linear(n[0]), linear(n[1]), linear(n[2])),
            "d" if !n.is_empty() => m.base[3] = n[0],
            "Tr" if !n.is_empty() => m.base[3] = 1.0 - n[0],
            "Ke" if n.len() >= 3 => m.emissive = [linear(n[0]), linear(n[1]), linear(n[2])],
            "Ns" if !n.is_empty() => m.roughness = (2.0 / (n[0].max(0.0) + 2.0)).sqrt(),
            "Pr" if !n.is_empty() => m.roughness = n[0],
            "Pm" if !n.is_empty() => m.metallic = n[0],
            "map_Kd" => m.base_map = picture(file, textures),
            "map_Ke" => {
                m.emissive_map = picture(file, textures);
                if m.emissive == [0.0; 3] {
                    m.emissive = [1.0; 3];
                }
            }
            _ => {}
        }
        if m.base[3] < 1.0 {
            m.alpha = AlphaMode::Blend;
        }
    }
    out
}

/// A primitive being made for a material, with the numbers of its corners so far (position,
/// texture point, normal) and the vertex each became.
type Part = (Option<usize>, Primitive, HashMap<(usize, usize, usize), u32>);

/// The mesh made so far, called `name`, put into `model` (none when it has no faces).
fn flush(name: &str, parts: &mut Vec<Part>, model: &mut Model) {
    let prims: Vec<Primitive> = parts.drain(..).map(|(_, p, _)| p).filter(|p| !p.indices.is_empty()).collect();
    if !prims.is_empty() {
        model.nodes.push(Node { name: name.to_string(), mesh: Some(model.meshes.len()), ..Node::default() });
        model.roots.push(model.nodes.len() - 1);
        model.meshes.push(Mesh { name: name.to_string(), primitives: prims, weights: Vec::new() });
    }
}

/// A model from OBJ text (`dir`: where the files it names are).
pub fn read(text: &str, dir: &Path) -> Result<Model, String> {
    let mut model = Model::default();
    let (mut pos, mut tex, mut nor) = (Vec::<[f32; 3]>::new(), Vec::<[f32; 2]>::new(), Vec::<[f32; 3]>::new());
    let mut colors: Vec<Option<[f32; 4]>> = Vec::new();
    let mut by_name: HashMap<String, usize> = HashMap::new();
    // the mesh being made: its name, and a primitive for each material
    let mut name = String::from("Object");
    let mut current: Option<usize> = None;
    let mut parts: Vec<Part> = Vec::new();
    let resolve = |v: &str, n: usize| -> Option<usize> {
        let i: i64 = v.parse().ok()?;
        let k = if i < 0 { n as i64 + i } else { i - 1 };
        (k >= 0 && (k as usize) < n).then_some(k as usize)
    };
    for (line_no, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("");
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((&key, rest)) = words.split_first() else { continue };
        let n = numbers(rest);
        match key {
            "v" if n.len() >= 3 => {
                pos.push([n[0], n[1], n[2]]);
                colors.push((n.len() >= 6).then(|| [linear(n[3]), linear(n[4]), linear(n[5]), 1.0]));
            }
            "vt" if !n.is_empty() => tex.push([n[0], n.get(1).copied().unwrap_or(0.0)]),
            "vn" if n.len() >= 3 => nor.push([n[0], n[1], n[2]]),
            "o" | "g" if !rest.is_empty() => {
                flush(&name, &mut parts, &mut model);
                name = rest.join(" ");
            }
            "mtllib" => {
                for file in rest {
                    let path = dir.join(file);
                    match std::fs::read_to_string(&path) {
                        Ok(mtl) => {
                            for m in materials(&mtl, path.parent().unwrap_or(dir), &mut model.textures, &mut model.notes) {
                                by_name.insert(m.name.clone(), model.materials.len());
                                model.materials.push(m);
                            }
                        }
                        Err(e) => model.notes.push(format!("{file} cannot be read ({e})")),
                    }
                }
            }
            "usemtl" => current = by_name.get(&rest.join(" ")).copied(),
            "f" => {
                let corners: Vec<(usize, Option<usize>, Option<usize>)> = rest
                    .iter()
                    .map(|c| {
                        let mut it = c.split('/');
                        let p = resolve(it.next().unwrap_or(""), pos.len())?;
                        let t = it.next().and_then(|t| resolve(t, tex.len()));
                        let nn = it.next().and_then(|t| resolve(t, nor.len()));
                        Some((p, t, nn))
                    })
                    .collect::<Option<_>>()
                    .ok_or_else(|| format!("line {}: a face names a corner there is not", line_no + 1))?;
                if corners.len() < 3 {
                    continue;
                }
                let part = match parts.iter().position(|(m, _, _)| *m == current) {
                    Some(i) => i,
                    None => {
                        parts.push((current, Primitive { material: current, ..Default::default() }, HashMap::new()));
                        parts.len() - 1
                    }
                };
                let (_, prim, seen) = &mut parts[part];
                let mut index_of = |(p, t, nn): (usize, Option<usize>, Option<usize>)| -> u32 {
                    *seen.entry((p, t.unwrap_or(usize::MAX), nn.unwrap_or(usize::MAX))).or_insert_with(|| {
                        prim.positions.push(pos[p]);
                        if let Some(t) = t {
                            // OBJ's texture points go up from the bottom; ours go down from the top
                            prim.uvs[0].push([tex[t][0], 1.0 - tex[t][1]]);
                        }
                        if let Some(nn) = nn {
                            prim.normals.push(nor[nn]);
                        }
                        if let Some(c) = colors[p] {
                            prim.colors.push(c);
                        }
                        (prim.positions.len() - 1) as u32
                    })
                };
                let ids: Vec<u32> = corners.into_iter().map(&mut index_of).collect();
                for k in 1..ids.len() - 1 {
                    prim.indices.extend([ids[0], ids[k], ids[k + 1]]);
                }
            }
            _ => {}
        }
    }
    flush(&name, &mut parts, &mut model);
    // a primitive whose corners do not all have normals (or texture points) has none
    for p in model.meshes.iter_mut().flat_map(|m| &mut m.primitives) {
        let n = p.positions.len();
        if p.uvs[0].len() != n {
            p.uvs[0].clear();
        }
        if p.colors.len() != n {
            p.colors.clear();
        }
        if p.normals.len() != n {
            p.normals.clear();
            p.flat();
        }
    }
    if model.meshes.is_empty() {
        return Err("the OBJ has no faces".into());
    }
    Ok(model)
}
