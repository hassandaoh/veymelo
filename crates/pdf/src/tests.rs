use super::*;

/// A picture drawn `scale` times its size, premultiplied sRGB.
fn drawn(doc: &Doc, scale: f64) -> veymelo_raster::Image {
    let (w, h) = ((doc.width * scale).round() as usize, (doc.height * scale).round() as usize);
    let plain = |c: [f64; 4]| [(c[0] * c[3]) as f32, (c[1] * c[3]) as f32, (c[2] * c[3]) as f32, c[3] as f32];
    veymelo_svg::draw(doc, Mat3([[scale, 0.0, 0.0], [0.0, scale, 0.0], [0.0, 0.0, 1.0]]), w, h, &plain)
}

/// PDFs made by the system (`make.swift`), each drawn by us and compared with the system's own
/// drawing of it (`render.swift`, twice its size): the files in `VEYMELO_PDF_DIR`.
#[test]
#[ignore]
fn pages_match_the_systems_drawing() {
    let Some(dir) = std::env::var_os("VEYMELO_PDF_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    let mut worst = Vec::new();
    for name in ["shapes", "paint", "text", "locked", "more", "rc4"] {
        let bytes = std::fs::read(dir.join(format!("{name}.pdf"))).unwrap();
        let doc = read(bytes, 0).unwrap_or_else(|e| panic!("{name}: {e}"));
        let ours = drawn(&doc, 2.0);
        let rgba: Vec<u8> = ours.px.iter().flat_map(|p| {
            let a = p[3].max(1e-6);
            let s = |v: f32| ((v / a).clamp(0.0, 1.0) * 255.0).round() as u8;
            [s(p[0]), s(p[1]), s(p[2]), (p[3].clamp(0.0, 1.0) * 255.0).round() as u8]
        }).collect();
        veymelo_os::write_picture(&dir.join(format!("{name}-ours.png")).to_string_lossy(), ours.width, ours.height, &rgba, "public.png", true, 1.0).unwrap();
        let theirs = veymelo_os::read_picture(&dir.join(format!("{name}-ref.png")).to_string_lossy(), 0).unwrap();
        assert_eq!((theirs.width, theirs.height), (ours.width, ours.height), "{name}");
        // how far apart, on average and where most
        let mut sum = 0.0;
        let mut far = 0usize;
        for (a, b) in ours.px.iter().zip(&theirs.px) {
            let d = (0..4).map(|k| (a[k] - b[k]).abs()).fold(0.0f32, f32::max);
            sum += d as f64;
            if d > 0.25 {
                far += 1;
            }
        }
        let mean = sum / ours.px.len() as f64;
        let share = far as f64 / ours.px.len() as f64;
        println!("{name}: mean {mean:.4}, far {:.3}%, notes {:?}", share * 100.0, doc.skipped);
        worst.push((name, mean, share));
    }
    for (name, mean, share) in worst {
        // glyphs' edges are smoothed a little differently
        assert!(mean < 0.02 && share < 0.02, "{name}: mean {mean}, far {share}");
    }
}

/// A PDF file of `objects` (numbered from 1, each what goes between `obj` and `endobj`), its
/// catalogue object 1, with a cross-reference table.
fn build(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(o);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in &offsets {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut s = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    s.extend(data);
    s.extend(b"\nendstream");
    s
}

/// The objects of a one-page file `w` × `h` with `resources` and `content`, more objects from 5.
fn page(w: u32, h: u32, resources: &str, content: &[u8], more: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut o = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Resources << {resources} >> /Contents 4 0 R >>").into_bytes(),
        stream("", content),
    ];
    o.extend(more.iter().cloned());
    o
}

/// Pixel (`x`, `y`) of a picture (`y` from its top), straight.
fn at(img: &veymelo_raster::Image, x: usize, y: usize) -> [f32; 4] {
    let p = img.px[y * img.width + x];
    if p[3] <= 0.0 { [0.0; 4] } else { [p[0] / p[3], p[1] / p[3], p[2] / p[3], p[3]] }
}

fn near(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

#[test]
fn shapes_paint_and_cut_as_written() {
    let content = b"1 0 0 rg 10 10 30 30 re f q 0 0 1 RG 4 w 50 10 m 90 10 l S Q q 60 50 30 30 re W n 0 1 0 rg 50 40 50 50 re f Q";
    let doc = read(build(&page(100, 100, "", content, &[])), 0).unwrap();
    assert_eq!((doc.width, doc.height), (100.0, 100.0));
    let img = drawn(&doc, 1.0);
    // y up in the file, down in the picture
    assert!(near(at(&img, 25, 75), [1.0, 0.0, 0.0, 1.0], 0.01));
    assert!(near(at(&img, 70, 90), [0.0, 0.0, 1.0, 1.0], 0.01));
    // the green square cut to the clip
    assert!(near(at(&img, 75, 35), [0.0, 1.0, 0.0, 1.0], 0.01));
    assert_eq!(at(&img, 55, 55)[3], 0.0);
}

#[test]
fn cross_reference_streams_object_streams_and_updates_are_read() {
    // objects 1 to 3 packed in object stream 5, found through a cross-reference stream (6)
    let content = b"0 0 1 rg 0 0 20 20 re f";
    let bodies = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Contents 4 0 R >>"];
    let mut place = 0;
    let mut head = String::new();
    for (k, b) in bodies.iter().enumerate() {
        head.push_str(&format!("{} {place} ", k + 1));
        place += b.len() + 1;
    }
    let first = head.len();
    let packed = format!("{head}{}", bodies.join(" ")).into_bytes();
    let mut out = b"%PDF-1.5\n".to_vec();
    let o4 = out.len();
    out.extend(b"4 0 obj\n");
    out.extend(stream("", content));
    out.extend(b"\nendobj\n");
    let o5 = out.len();
    let z = veymelo_zip::zlib(&packed);
    out.extend(b"5 0 obj\n");
    out.extend(stream(&format!("/Type /ObjStm /N 3 /First {first} /Filter /FlateDecode"), &z));
    out.extend(b"\nendobj\n");
    let o6 = out.len();
    // entries: type (1), offset or stream (2), index (1)
    let mut rows = vec![0u8, 0, 0, 0];
    for (kind, a, b) in [(2u8, 5u16, 0u8), (2, 5, 1), (2, 5, 2), (1, o4 as u16, 0), (1, o5 as u16, 0), (1, o6 as u16, 0)] {
        rows.push(kind);
        rows.extend(a.to_be_bytes());
        rows.push(b);
    }
    out.extend(b"6 0 obj\n");
    out.extend(stream("/Type /XRef /Size 7 /W [1 2 1] /Root 1 0 R", &rows));
    out.extend(format!("\nendobj\nstartxref\n{o6}\n%%EOF\n").as_bytes());
    let doc = read(out.clone(), 0).unwrap();
    assert!(near(at(&drawn(&doc, 1.0), 10, 10), [0.0, 0.0, 1.0, 1.0], 0.01));
    // an update: object 4 again, now red, with its own table pointing back
    let o4b = out.len();
    out.extend(b"4 0 obj\n");
    out.extend(stream("", b"1 0 0 rg 0 0 20 20 re f"));
    out.extend(b"\nendobj\n");
    let x = out.len();
    out.extend(format!("xref\n4 1\n{o4b:010} 00000 n \ntrailer\n<< /Size 7 /Root 1 0 R /Prev {o6} >>\nstartxref\n{x}\n%%EOF\n").as_bytes());
    let doc = read(out, 0).unwrap();
    assert!(near(at(&drawn(&doc, 1.0), 10, 10), [1.0, 0.0, 0.0, 1.0], 0.01));
}

#[test]
fn a_file_whose_table_is_wrong_is_found_again() {
    let mut f = build(&page(20, 20, "", b"0 1 0 rg 0 0 20 20 re f", &[]));
    // the table's places all moved by 3
    let x = f.windows(4).rposition(|w| w == b"xref").unwrap();
    let text = String::from_utf8_lossy(&f[x..]).to_string();
    let moved: String = text
        .lines()
        .map(|l| {
            let parts: Vec<&str> = l.split(' ').collect();
            if parts.len() >= 3 && parts[2] == "n" { format!("{:010} 00000 n ", parts[0].parse::<usize>().unwrap() + 3) } else { l.to_string() }
        })
        .collect::<Vec<_>>()
        .join("\n");
    f.truncate(x);
    f.extend(moved.as_bytes());
    let doc = read(f, 0).unwrap();
    assert!(near(at(&drawn(&doc, 1.0), 10, 10), [0.0, 1.0, 0.0, 1.0], 0.01));
}

#[test]
fn filters_inline_pictures_and_colour_spaces_are_read() {
    // the content in LZW then ASCII85: a 2 × 2 inline grey picture, an indexed colour, a
    // separation through a calculator function, and DeviceN
    let content = b"q 20 0 0 20 0 0 cm BI /W 2 /H 2 /BPC 8 /CS /G ID \x00\xff\xff\x00 EI Q /Ix cs 1 sc 20 0 10 10 re f /Sep cs 0.5 sc 30 0 10 10 re f";
    let doc = read(build(&page(40, 20, "/ColorSpace << /Ix [/Indexed /DeviceRGB 1 <ff000000ff00>] /Sep [/Separation /Spot /DeviceRGB 5 0 R] >>", content, &[stream("/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1]", b"{ dup 0 exch }")])), 0).unwrap();
    let img = drawn(&doc, 1.0);
    // the picture's first row black then white (its top row at the top)
    // (smoothed between its pixels, as the system draws pictures)
    assert!(near(at(&img, 2, 2), [0.0, 0.0, 0.0, 1.0], 0.01), "{:?}", at(&img, 2, 2));
    assert!(near(at(&img, 17, 2), [1.0, 1.0, 1.0, 1.0], 0.01));
    // index 1: green
    assert!(near(at(&img, 25, 15), [0.0, 1.0, 0.0, 1.0], 0.01));
    // tint 0.5 to (0.5, 0, 0.5)
    assert!(near(at(&img, 35, 15), [0.5, 0.0, 0.5, 1.0], 0.01), "{:?}", at(&img, 35, 15));
}

#[test]
fn meshes_patterns_and_layers_are_drawn() {
    // a triangle mesh (type 4) shaded red, green, blue, and a Coons patch; an uncoloured tiling
    // pattern of squares in blue; a layer that is shown and one that is hidden
    let mut mesh = Vec::new();
    for (x, y, c) in [(0u8, 0u8, [255u8, 0, 0]), (255, 0, [0, 255, 0]), (0, 255, [0, 0, 255])] {
        mesh.extend([0u8, x, y]);
        mesh.extend(c);
    }
    let content = b"/Tri sh q 0 0 1 rg /P1 cs 0 0 1 /T scn 40 0 40 40 re f Q /OC /L1 BDC 0 0 0 rg 80 0 10 10 re f EMC /OC /L2 BDC 1 0 0 rg 90 0 10 10 re f EMC";
    let objects = page(
        100,
        40,
        "/Shading << /Tri 5 0 R >> /ColorSpace << /P1 [/Pattern /DeviceRGB] >> /Pattern << /T 6 0 R >> /Properties << /L1 7 0 R /L2 8 0 R >>",
        content,
        &[
            stream("/ShadingType 4 /ColorSpace /DeviceRGB /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 40 0 40 0 1 0 1 0 1]", &mesh),
            stream("/PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>", b"0 0 5 5 re f"),
            b"<< /Type /OCG /Name (Shown) >>".to_vec(),
            b"<< /Type /OCG /Name (Hidden) >>".to_vec(),
        ],
    );
    let mut objects = objects;
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [7 0 R 8 0 R] /D << /OFF [8 0 R] >> >> >>".to_vec();
    let doc = read(build(&objects), 0).unwrap();
    let img = drawn(&doc, 1.0);
    // near the mesh's red corner (bottom left of the page)
    let c = at(&img, 2, 37);
    assert!(c[0] > 0.8 && c[1] < 0.2 && c[2] < 0.2, "{c:?}");
    // the pattern: blue in its squares' lower left quarters, nothing between
    assert!(near(at(&img, 42, 37), [0.0, 0.0, 1.0, 1.0], 0.01), "{:?}", at(&img, 42, 37));
    assert_eq!(at(&img, 48, 32)[3], 0.0);
    // the shown layer, named; the hidden one left out
    assert!(near(at(&img, 85, 35), [0.0, 0.0, 0.0, 1.0], 0.01));
    assert_eq!(at(&img, 95, 35)[3], 0.0);
    let names: Vec<String> = doc.root.children.iter().filter_map(|n| if let veymelo_svg::Node::Group(g) = n { Some(g.name.clone()) } else { None }).collect();
    assert!(names.contains(&"Shown".to_string()), "{names:?}");
}

#[test]
fn type3_glyphs_soft_masks_and_turned_pages() {
    // a Type 3 font whose "a" is a filled square in the text's colour; a soft mask of a white
    // square on black over a red fill; the page turned a quarter
    let content = b"BT /T3 10 Tf 0 0 1 rg 0 0 Td (a) Tj ET q /M gs 1 0 0 rg 20 0 20 20 re f Q";
    let objects = page(
        40,
        20,
        "/Font << /T3 5 0 R >> /ExtGState << /M 7 0 R >>",
        content,
        &[
            b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /sq 6 0 R >> /Encoding << /Differences [97 /sq] >> /FirstChar 97 /LastChar 97 /Widths [1000] >>".to_vec(),
            stream("", b"1000 0 0 0 1000 1000 d1 0 0 1000 1000 re f"),
            b"<< /Type /ExtGState /SMask << /S /Luminosity /G 8 0 R >> >>".to_vec(),
            stream("/Type /XObject /Subtype /Form /BBox [0 0 40 20] /Group << /S /Transparency /CS /DeviceGray >>", b"0 g 0 0 40 20 re f 1 g 25 5 10 10 re f"),
        ],
    );
    let doc = read(build(&objects), 0).unwrap();
    let img = drawn(&doc, 1.0);
    // the glyph: a 10 × 10 blue square at the corner
    assert!(near(at(&img, 5, 15), [0.0, 0.0, 1.0, 1.0], 0.01), "{:?}", at(&img, 5, 15));
    // red only where the mask is white
    assert!(near(at(&img, 30, 10), [1.0, 0.0, 0.0, 1.0], 0.01), "{:?}", at(&img, 30, 10));
    assert_eq!(at(&img, 22, 10)[3], 0.0);
    // turned: the page 20 wide and 40 tall, its left edge at the top
    let mut objects = objects;
    objects[2] = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Rotate 90 /Resources << /Font << /T3 5 0 R >> /ExtGState << /M 7 0 R >> >> /Contents 4 0 R >>".to_vec();
    let doc = read(build(&objects), 0).unwrap();
    assert_eq!((doc.width, doc.height), (20.0, 40.0));
    let img = drawn(&doc, 1.0);
    // the glyph (the page's bottom left) now at the top left
    assert!(near(at(&img, 5, 5), [0.0, 0.0, 1.0, 1.0], 0.01), "{:?}", at(&img, 5, 5));
}

#[test]
fn text_clips_and_fonts_not_in_the_file() {
    // Helvetica, not in the file, from the system; then a clip from text (mode 7)
    let content = b"BT /F1 20 Tf 0 0 0 rg 2 4 Td (H) Tj ET BT /F1 20 Tf 7 Tr 40 4 Td (H) Tj ET 1 0 0 rg 40 0 20 20 re f";
    let doc = read(build(&page(60, 20, "/Font << /F1 5 0 R >>", content, &[b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec()])), 0).unwrap();
    let img = drawn(&doc, 2.0);
    let ink = |x0: usize, x1: usize| (0..40).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| at(&img, x, y)[3] > 0.5).count();
    assert!(ink(0, 40) > 100, "Helvetica's H drawn: {}", ink(0, 40));
    // the red square only inside the H
    let red = ink(80, 120);
    assert!(red > 100 && red < 40 * 40 / 2, "{red}");
}
