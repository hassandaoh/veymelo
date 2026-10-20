use super::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("veymelo-model-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// Bytes as base64 (to write test files with).
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            out.push(if k <= c.len() { ABC[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// One triangle (and its buffer's bytes): positions, u16 indices, then two key times and two
/// translations for an animation.
fn triangle() -> (String, Vec<u8>) {
    let mut bin = Vec::new();
    for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bin.extend(v.to_le_bytes());
    }
    for i in [0u16, 1, 2, 0] {
        bin.extend(i.to_le_bytes());
    }
    for v in [0.0f32, 1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0] {
        bin.extend(v.to_le_bytes());
    }
    let json = r#"{
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "Tri", "mesh": 0, "translation": [1, 2, 3], "children": [1]},
                  {"name": "Lamp", "extensions": {"KHR_lights_punctual": {"light": 0}}, "camera": 0}],
        "meshes": [{"name": "M", "primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
        "materials": [{"name": "Red", "pbrMetallicRoughness": {"baseColorFactor": [1, 0, 0, 1], "metallicFactor": 0.2}, "doubleSided": true}],
        "cameras": [{"type": "perspective", "perspective": {"yfov": 0.5, "znear": 0.1}}],
        "extensions": {"KHR_lights_punctual": {"lights": [{"type": "spot", "color": [1, 0.5, 0.25], "intensity": 3, "spot": {"outerConeAngle": 0.6}}]}},
        "animations": [{"name": "Slide", "samplers": [{"input": 2, "output": 3, "interpolation": "LINEAR"}], "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}]}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"},
            {"bufferView": 2, "componentType": 5126, "count": 2, "type": "SCALAR"},
            {"bufferView": 3, "componentType": 5126, "count": 2, "type": "VEC3"}
        ],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 6},
            {"buffer": 0, "byteOffset": 44, "byteLength": 8},
            {"buffer": 0, "byteOffset": 52, "byteLength": 24}
        ],
        "buffers": [BUFFER]
    }"#;
    (json.to_string(), bin)
}

fn check(m: &Model) {
    assert_eq!(m.triangles(), 1);
    assert_eq!(m.materials[0].base, [1.0, 0.0, 0.0, 1.0]);
    assert!(m.materials[0].two_sided && (m.materials[0].metallic - 0.2).abs() < 1e-6);
    // no normals given: the face's own
    assert_eq!(m.meshes[0].primitives[0].normals, vec![[0.0, 0.0, 1.0]; 3]);
    let still = m.pose(None, 0.0);
    assert_eq!(still.world[0].apply(vec3(1.0, 0.0, 0.0)), vec3(2.0, 2.0, 3.0));
    assert_eq!(still.world[1].apply(vec3(0.0, 0.0, 0.0)), vec3(1.0, 2.0, 3.0), "the child goes with its parent");
    let (lo, hi) = m.bounds(&still).unwrap();
    assert_eq!((lo, hi), (vec3(1.0, 2.0, 3.0), vec3(2.0, 3.0, 3.0)));
    // half way through the slide
    assert_eq!(m.animations[0].name, "Slide");
    assert_eq!(m.length(), 1.0);
    assert_eq!(m.pose(Some(0), 0.5).world[0].apply(vec3(0.0, 0.0, 0.0)), vec3(1.0, 0.0, 0.0));
    assert_eq!(m.pose(Some(0), 7.0).world[0].apply(vec3(0.0, 0.0, 0.0)), vec3(2.0, 0.0, 0.0), "held after its last key");
    assert_eq!(m.cameras[0].kind, CameraKind::Perspective { yfov: 0.5 });
    assert!(matches!(m.lights[0].kind, LightKind::Spot { outer, .. } if (outer - 0.6).abs() < 1e-9));
    assert_eq!((m.nodes[1].light, m.nodes[1].camera), (Some(0), Some(0)));
}

#[test]
fn gltf_with_its_buffer_inside_beside_or_bound_in() {
    let (json, bin) = triangle();
    // in a data URI
    let inside = json.replace("BUFFER", &format!(r#"{{"byteLength": {}, "uri": "data:application/octet-stream;base64,{}"}}"#, bin.len(), base64(&bin)));
    check(&gltf::read(inside.as_bytes(), Path::new(".")).unwrap());
    // in a file beside it
    std::fs::write(scratch("tri angle.bin"), &bin).unwrap();
    let beside = json.replace("BUFFER", &format!(r#"{{"byteLength": {}, "uri": "tri%20angle.bin"}}"#, bin.len()));
    std::fs::write(scratch("tri.gltf"), &beside).unwrap();
    check(&Model::open(&scratch("tri.gltf").to_string_lossy()).unwrap());
    // a .glb: the JSON and the buffer as chunks
    let text = json.replace("BUFFER", &format!(r#"{{"byteLength": {}}}"#, bin.len()));
    let pad = |mut v: Vec<u8>, with: u8| {
        while !v.len().is_multiple_of(4) {
            v.push(with);
        }
        v
    };
    let (j, b) = (pad(text.into_bytes(), b' '), pad(bin, 0));
    let mut glb = b"glTF".to_vec();
    glb.extend(2u32.to_le_bytes());
    glb.extend(((12 + 8 + j.len() + 8 + b.len()) as u32).to_le_bytes());
    glb.extend((j.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(&j);
    glb.extend((b.len() as u32).to_le_bytes());
    glb.extend(b"BIN\0");
    glb.extend(&b);
    check(&gltf::read(&glb, Path::new(".")).unwrap());
    assert!(gltf::read(br#"{"asset": {"version": "1.0"}}"#, Path::new(".")).unwrap_err().contains("2.0"));
}

#[test]
fn animations_between_keys() {
    let channel = |moves, interp, times: Vec<f32>, values: Vec<f32>, width| Channel { node: 0, moves, interp, times, values, width };
    // a quarter turn about y, half way: an eighth
    let s = std::f32::consts::FRAC_1_SQRT_2;
    let turn = channel(Moves::Rotation, Interp::Linear, vec![0.0, 2.0], vec![0.0, 0.0, 0.0, 1.0, 0.0, s, 0.0, s], 4);
    let q = turn.at(1.0);
    let half = (std::f64::consts::PI / 8.0).sin();
    assert!((q[1] - half).abs() < 1e-6 && (q[3] - (std::f64::consts::PI / 8.0).cos()).abs() < 1e-6, "{q:?}");
    let step = channel(Moves::Scale, Interp::Step, vec![0.0, 1.0], vec![1.0, 1.0, 1.0, 3.0, 3.0, 3.0], 3);
    assert_eq!(step.at(0.99), vec![1.0; 3]);
    // cubic with flat tangents: smooth, a half at the middle, an ease near the ends
    let cubic = channel(Moves::Translation, Interp::Cubic, vec![0.0, 1.0], vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 0.0, 0.0], 3);
    assert!((cubic.at(0.5)[0] - 5.0).abs() < 1e-9 && cubic.at(0.1)[0] < 1.0);
}

#[test]
fn obj_with_its_materials() {
    std::fs::write(scratch("box.mtl"), "newmtl Blue\nKd 0 0 1\nNs 98\nnewmtl Glass\nKd 1 1 1\nd 0.25\n").unwrap();
    let obj = "mtllib box.mtl\no Box\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\n\
               usemtl Blue\nf 1/1 2/2 3/3 4/4\nusemtl Glass\nf -5/1 -4/2 -1/4\n";
    std::fs::write(scratch("box.obj"), obj).unwrap();
    let m = Model::open(&scratch("box.obj").to_string_lossy()).unwrap();
    assert_eq!(m.nodes[0].name, "Box");
    assert_eq!(m.triangles(), 3, "the square cut in two, and a triangle");
    let prims = &m.meshes[0].primitives;
    assert_eq!(prims.len(), 2);
    let blue = &m.materials[prims[0].material.unwrap()];
    assert_eq!(blue.base, [0.0, 0.0, 1.0, 1.0]);
    assert!((blue.roughness - 0.1414).abs() < 1e-3, "shiny: {}", blue.roughness);
    assert!(matches!(m.materials[prims[1].material.unwrap()].alpha, AlphaMode::Blend));
    // flat faces, texture points turned the right way up
    assert_eq!(prims[0].normals[0], [0.0, 0.0, 1.0]);
    assert_eq!(prims[0].uvs[0][0], [0.0, 1.0]);
    assert!(obj::read("v 0 0 0\nf 1 2 3\n", Path::new(".")).unwrap_err().contains("corner"));
}

#[test]
fn skins_bend_by_their_joints() {
    let mut m = Model { nodes: vec![Node { children: vec![1], ..Node::default() }, Node { translation: [0.0, 2.0, 0.0], ..Node::default() }], roots: vec![0], ..Model::default() };
    m.skins.push(Skin { joints: vec![0, 1], inverse_bind: vec![Mat4::IDENTITY, Mat4::translate(vec3(0.0, -2.0, 0.0))] });
    m.animations.push(Animation { name: "Bend".into(), channels: vec![Channel { node: 1, moves: Moves::Translation, interp: Interp::Linear, times: vec![0.0, 1.0], values: vec![0.0, 2.0, 0.0, 3.0, 2.0, 0.0], width: 3 }] });
    let at_rest = m.joints(0, &m.pose(None, 0.0));
    assert_eq!(at_rest[1].apply(vec3(0.0, 2.0, 0.0)), vec3(0.0, 2.0, 0.0), "bound where it is: no bend");
    let moved = m.joints(0, &m.pose(Some(0), 1.0));
    assert_eq!(moved[1].apply(vec3(0.0, 2.0, 0.0)), vec3(3.0, 2.0, 0.0));
}

#[test]
fn solids_are_closed_and_face_out() {
    use shapes::{Shape, Sizes};
    let s = Sizes::default();
    for shape in [Shape::Cube, Shape::Sphere, Shape::Plane, Shape::Torus, Shape::Cone, Shape::Cylinder] {
        let p = shapes::mesh(shape, &s);
        assert!(p.triangles() > 0 && p.normals.len() == p.positions.len() && p.uvs[0].len() == p.positions.len() && p.tangents.len() == p.positions.len(), "{shape:?}");
        // each triangle turns anticlockwise seen from the side its normals face (out of a closed
        // solid: away from its middle)
        for t in p.indices.as_chunks::<3>().0 {
            let [a, b, c] = t.map(|i| p.positions[i as usize]);
            let face = cross(sub(b, a), sub(c, a));
            if face.iter().map(|v| v * v).sum::<f32>() < 1e-6 {
                continue;
            }
            let n = [0, 1, 2].map(|k| p.normals[t[0] as usize][k] + p.normals[t[1] as usize][k] + p.normals[t[2] as usize][k]);
            assert!(dot(face, n) > 0.0, "{shape:?}: a triangle turned the wrong way");
        }
    }
    let cube = shapes::model(Shape::Cube, &Sizes { size: [200.0, 100.0, 50.0], ..s }, Material::default());
    let (lo, hi) = cube.bounds(&cube.pose(None, 0.0)).unwrap();
    assert_eq!((lo, hi), (vec3(-100.0, -50.0, -25.0), vec3(100.0, 50.0, 25.0)));
    assert_eq!(cube.triangles(), 12);
    let sphere = shapes::mesh(Shape::Sphere, &Sizes { radius: 10.0, ..s });
    assert!(sphere.positions.iter().all(|p| ((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() - 10.0).abs() < 1e-4));
    assert!(shapes::model(Shape::Plane, &s, Material::default()).materials[0].two_sided);
}

#[test]
fn stl_binary_with_colours_and_text() {
    // a binary file: two facets, the second coloured red (VisCAM's way: blue in the low bits)
    let mut bin = vec![0u8; 80];
    bin.extend(2u32.to_le_bytes());
    let facet = |attr: u16, z: f32| {
        let mut f = Vec::new();
        for v in [0.0f32, 0.0, 1.0, 0.0, 0.0, z, 1.0, 0.0, z, 0.0, 1.0, z] {
            f.extend(v.to_le_bytes());
        }
        f.extend(attr.to_le_bytes());
        f
    };
    bin.extend(facet(0, 0.0));
    bin.extend(facet(0x8000 | 31 << 10, 5.0));
    let m = stl::read(&bin).unwrap();
    assert_eq!(m.triangles(), 2);
    assert_eq!(m.meshes[0].primitives.len(), 2, "by colour");
    let red = &m.materials[m.meshes[0].primitives[1].material.unwrap()];
    assert_eq!(red.base, [1.0, 0.0, 0.0, 1.0]);
    // Z up turned to Y up: the facet at z 5 is at y 5; it faces +Y
    let p = &m.meshes[0].primitives[1];
    assert!(p.positions.iter().all(|v| v[1] == 5.0));
    assert_eq!(p.normals[0], [0.0, 1.0, 0.0]);
    // text, a loop of four corners cut in two
    let text = "solid square\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 1 1 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid square\n";
    let m = stl::read(text.as_bytes()).unwrap();
    assert_eq!((m.triangles(), m.meshes[0].name.as_str()), (2, "square"));
    assert!(m.materials.is_empty(), "no colours: the default material");
    assert!(stl::read(b"solid nothing\nendsolid\n").unwrap_err().contains("no facets"));
    // opened by its name
    std::fs::write(scratch("part.stl"), &bin).unwrap();
    assert!(Model::is_model("part.STL"));
    assert_eq!(Model::open(&scratch("part.stl").to_string_lossy()).unwrap().triangles(), 2);
}
