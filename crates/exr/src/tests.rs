use super::*;

/// FNV-1a over a plane's floats, as the fixtures' index keeps them.
fn hash(v: &[f32]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in v.iter().flat_map(|f| f.to_le_bytes()) {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}")
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// Files written by OpenEXR itself (through OpenImageIO, and a Blender render), in every
/// compression, scanlines and tiles, one part and many: every channel of every part reads as
/// OpenEXR reads it, to the bit.
#[test]
fn files_read_as_openexr_reads_them() {
    let index = veymelo_json::Value::parse(&String::from_utf8(fixture("index.json")).unwrap()).unwrap();
    let mut wrong = Vec::new();
    for (file, entry) in index.as_object().unwrap().iter() {
        let bytes = fixture(file);
        let exr = parse(&bytes).unwrap_or_else(|e| panic!("{file}: {e}"));
        let parts = entry.get("parts").and_then(|p| p.as_array()).unwrap();
        assert_eq!(exr.parts.len(), parts.len(), "{file}");
        for (k, part) in parts.iter().enumerate() {
            let planes = exr.part_planes(&bytes, k).unwrap_or_else(|e| panic!("{file} part {k}: {e}"));
            let p = &exr.parts[k];
            let n = |key: &str| part.get(key).and_then(|v| v.as_i64()).unwrap() as i32;
            assert_eq!((p.data.x0, p.data.y0, p.data.width() as i32, p.data.height() as i32), (n("x"), n("y"), n("width"), n("height")), "{file}");
            let names = part.get("channels").and_then(|c| c.as_array()).unwrap();
            let hashes = part.get("hashes").and_then(|c| c.as_array()).unwrap();
            for (name, want) in names.iter().zip(hashes) {
                let name = name.as_str().unwrap();
                let plane = planes.iter().find(|q| q.name == name).unwrap_or_else(|| panic!("{file}: no {name}"));
                if name == "id" {
                    // OpenImageIO reads whole numbers scaled to 0…1: they are checked as written
                    let w = plane.window.width();
                    let ok = plane.samples.iter().enumerate().all(|(i, v)| *v == ((i % w / 4 + i / w / 4) % 3 + 1) as f32);
                    if !ok {
                        wrong.push(format!("{file} {name}"));
                    }
                    continue;
                }
                if hash(&plane.samples) != want.as_str().unwrap() {
                    wrong.push(format!("{file} {name}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "read otherwise than OpenEXR: {wrong:?}");
}

#[test]
fn headers_read() {
    let bytes = fixture("tiles-zip-mip.exr");
    let exr = parse(&bytes).unwrap();
    let p = &exr.parts[0];
    assert_eq!((p.kind, p.compression, p.tiles), (Kind::Tiles, Compression::Zip, Some(Tiles { width: 16, height: 8, levels: Levels::Mipmap, round_up: false })));
    assert_eq!((p.data, p.display), (Window { x0: 3, y0: 2, x1: 36, y1: 26 }, Window { x0: 0, y0: 0, x1: 39, y1: 29 }));
    assert_eq!(p.text("Comment"), Some("fixture"));
    let names: Vec<&str> = p.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["A", "B", "G", "R", "Y", "Z", "id"]);
    assert_eq!(p.channels[5].sample, Sample::Float);
    assert_eq!(p.channels[6].sample, Sample::Uint);
    // six levels of 34 × 25 (17 × 12, 8 × 6…): 3 × 4 tiles, then 2 × 2, then one each
    assert_eq!(p.chunks.len(), 12 + 4 + 1 + 1 + 1 + 1);
    let exr = parse(&fixture("lines-zip-down.exr")).unwrap();
    assert!(exr.parts[0].decreasing);
    assert!(parse(b"GIF89a").unwrap_err().contains("not an OpenEXR"));
}

/// Blender 5 writes a part a pass, the channels carrying the pass's name; the planes keep it
/// once, and the Cryptomatte manifests are in the parts' headers.
#[test]
fn layers_of_a_render() {
    let bytes = fixture("render-zip.exr");
    let (exr, planes) = read(&bytes).unwrap();
    assert!(exr.multipart);
    assert_eq!(exr.parts[0].name.as_deref(), Some("ViewLayer.Combined"));
    let names: Vec<&str> = planes.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"ViewLayer.Depth.Z") && names.contains(&"ViewLayer.CryptoObject00.r"), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("Depth.ViewLayer")));
    let crypto = exr.parts.iter().find_map(|p| p.attributes.iter().find(|a| a.name.ends_with("/name") && matches!(&a.value, Value::Text(t) if t == "ViewLayer.CryptoObject")));
    assert!(crypto.is_some());
    // the picture is the Combined pass, not a data pass
    let pic = picture(&exr, &planes).unwrap();
    assert_eq!((pic.layer.as_str(), pic.width, pic.height, pic.alpha), ("ViewLayer.Combined", 48, 32, true));
    // two parts named in a file of OpenImageIO's: the beauty's channels take its name
    let bytes = fixture("parts.exr");
    let (exr, planes) = read(&bytes).unwrap();
    let names: Vec<&str> = planes.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["beauty.A", "beauty.B", "beauty.G", "beauty.R", "depth.Z"]);
    assert_eq!(picture(&exr, &planes).unwrap().layer, "beauty");
    // the picture alone, from the parts it is in
    assert_eq!(read_picture(&bytes).unwrap(), picture(&exr, &planes).unwrap());
    let bytes = fixture("render-dwaa.exr");
    let (exr, planes) = read(&bytes).unwrap();
    assert_eq!(read_picture(&bytes).unwrap(), picture(&exr, &planes).unwrap());
}

/// The picture over the display window: nothing outside the data window.
#[test]
fn a_picture_in_its_display_window() {
    let bytes = fixture("lines-piz.exr");
    let (exr, planes) = read(&bytes).unwrap();
    let pic = picture(&exr, &planes).unwrap();
    assert_eq!((pic.width, pic.height, pic.layer.as_str(), pic.alpha), (40, 30, "", true));
    assert_eq!(pic.px[0], [0.0; 4]);
    let r = planes.iter().find(|p| p.name == "R").unwrap();
    // data pixel (0, 0) is display pixel (3, 2)
    assert_eq!(pic.px[2 * 40 + 3][0], r.samples[0]);
    assert_eq!(pic.px[(2 + 10) * 40 + 3 + 20][0], 37.5, "the bright highlight");
}

/// A file of our own writer's read back, halves and floats.
#[test]
fn our_own_files_read_back() {
    let (w, h) = (37, 21);
    let ramp: Vec<f32> = (0..w * h).map(|i| i as f32 * 0.37 - 20.0).collect();
    let channels = vec![write::Channel { name: "R".into(), samples: ramp.clone() }, write::Channel { name: "depth.Z".into(), samples: ramp.iter().map(|v| v * 3.0).collect() }];
    for halves in [false, true] {
        let bytes = write::encode(w, h, &channels, halves);
        let (exr, planes) = read(&bytes).unwrap();
        assert_eq!(exr.display(), Window { x0: 0, y0: 0, x1: w as i32 - 1, y1: h as i32 - 1 });
        let r = &planes.iter().find(|p| p.name == "R").unwrap().samples;
        let want: Vec<f32> = if halves { ramp.iter().map(|v| half::to_f32(half::from_f32(*v))).collect() } else { ramp.clone() };
        assert_eq!(r, &want);
    }
}

/// A file laid out by hand, uncompressed: two parts, one with channels kept every other pixel
/// (luminance and chroma), the other with its chunk table broken.
struct Hand {
    out: Vec<u8>,
}

impl Hand {
    fn attr(&mut self, name: &str, kind: &str, v: &[u8]) {
        self.out.extend(name.as_bytes());
        self.out.push(0);
        self.out.extend(kind.as_bytes());
        self.out.push(0);
        self.out.extend((v.len() as i32).to_le_bytes());
        self.out.extend(v);
    }

    fn header(&mut self, name: &str, channels: &[(&str, i32, [i32; 2])], data: [i32; 4], chunks: i32) {
        let mut list = Vec::new();
        for (n, kind, s) in channels {
            list.extend(n.as_bytes());
            list.push(0);
            list.extend(kind.to_le_bytes());
            list.extend([0, 0, 0, 0]);
            list.extend(s[0].to_le_bytes());
            list.extend(s[1].to_le_bytes());
        }
        list.push(0);
        let b = |v: [i32; 4]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
        self.attr("channels", "chlist", &list);
        self.attr("compression", "compression", &[0]);
        self.attr("dataWindow", "box2i", &b(data));
        self.attr("displayWindow", "box2i", &b([0, 0, 7, 5]));
        self.attr("lineOrder", "lineOrder", &[0]);
        self.attr("name", "string", name.as_bytes());
        self.attr("type", "string", b"scanlineimage");
        self.attr("chunkCount", "int", &chunks.to_le_bytes());
        self.out.push(0);
    }
}

#[test]
fn channels_at_a_lower_rate_and_a_broken_chunk_table() {
    let mut f = Hand { out: MAGIC.to_vec() };
    f.out.extend(0x1002u32.to_le_bytes());
    // part 0: Y every pixel, RY and BY every other pixel both ways, over 8 × 6 at (0, 0)
    f.header("yc", &[("BY", 1, [2, 2]), ("RY", 1, [2, 2]), ("Y", 1, [1, 1])], [0, 0, 7, 5], 6);
    // part 1: a depth over 4 × 2 at (2, 3)
    f.header("depth", &[("Z", 2, [1, 1])], [2, 3, 5, 4], 2);
    f.out.push(0);
    let table = f.out.len();
    f.out.extend([0u8; 8 * 8]);
    let h = |v: f32| half::from_f32(v).to_le_bytes();
    let mut at = Vec::new();
    for y in 0..6i32 {
        at.push(f.out.len() as u64);
        let mut line = Vec::new();
        if y % 2 == 0 {
            (0..4).for_each(|_| line.extend(h(-1.0)));
            (0..4).for_each(|_| line.extend(h(1.0)));
        }
        (0..8).for_each(|x| line.extend(h(0.25 + x as f32 / 32.0)));
        f.out.extend(0i32.to_le_bytes());
        f.out.extend(y.to_le_bytes());
        f.out.extend((line.len() as i32).to_le_bytes());
        f.out.extend(line);
    }
    for y in 3..5i32 {
        at.push(f.out.len() as u64);
        f.out.extend(1i32.to_le_bytes());
        f.out.extend(y.to_le_bytes());
        f.out.extend(16i32.to_le_bytes());
        (0..4).for_each(|x| f.out.extend((y as f32 * 10.0 + x as f32).to_le_bytes()));
    }
    for (k, v) in at.iter().enumerate() {
        f.out[table + 8 * k..table + 8 * k + 8].copy_from_slice(&v.to_le_bytes());
    }
    let check = |bytes: &[u8]| {
        let (exr, planes) = read(bytes).unwrap();
        let names: Vec<&str> = planes.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["yc.BY", "yc.RY", "yc.Y", "depth.Z"]);
        // the chroma widened to every pixel
        assert!(planes[1].samples.iter().all(|v| *v == 1.0) && planes[1].samples.len() == 48);
        let z = &planes[3];
        assert_eq!((z.window, z.samples[5]), (Window { x0: 2, y0: 3, x1: 5, y1: 4 }, 41.0));
        assert_eq!(z.placed(exr.display(), -1.0)[4 * 8 + 3], 41.0);
        assert_eq!(z.placed(exr.display(), -1.0)[0], -1.0);
        // luminance and chroma made red, green and blue: red twice the luminance, no blue
        let pic = picture(&exr, &planes).unwrap();
        assert_eq!(pic.layer, "yc");
        let [r, g, b, a] = pic.px[8 + 4];
        let y = 0.25 + 4.0 / 32.0;
        assert!((r - 2.0 * y).abs() < 1e-3 && b.abs() < 1e-3 && a == 1.0, "{:?}", pic.px[12]);
        assert!((0.2126 * r + 0.7152 * g + 0.0722 * b - y).abs() < 1e-3);
    };
    check(&f.out);
    // the table zeroed, as when writing stopped short: the chunks are found one after another
    let mut broken = f.out.clone();
    broken[table..table + 64].fill(0);
    check(&broken);
}

#[test]
fn what_is_not_read_says_so() {
    let mut bytes = fixture("lines-zip.exr");
    let at = bytes.windows(12).position(|w| w == b"compression\0").unwrap() + 12 + 12 + 4;
    assert_eq!(bytes[at], 3);
    bytes[at] = 10;
    let e = parse(&bytes).unwrap_err();
    assert!(e.contains("HTJ2K"), "{e}");
}

/// The picture of layered files that have no unnamed layer with colour.
#[test]
fn the_picture_is_the_beauty_pass() {
    let of = |names: &[&str]| picture_layer(names);
    let mut blender = Vec::new();
    for l in ["CryptoMaterial00", "CryptoMaterial01", "GlossCol", "GlossDir", "Image", "VolDir"] {
        for c in ["R", "G", "B", "A"] {
            blender.push(format!("{l}.{}", if l.starts_with("Crypto") { c.to_lowercase() } else { c.to_string() }));
        }
    }
    let names: Vec<&str> = blender.iter().map(String::as_str).collect();
    assert_eq!(of(&names).as_deref(), Some("Image"));
    let no_image: Vec<&str> = names.iter().copied().filter(|n| !n.starts_with("Image")).collect();
    assert_eq!(of(&no_image).as_deref(), Some("GlossCol"), "not Cryptomatte");
    assert_eq!(of(&["ViewLayer.CryptoObject00.r", "ViewLayer.CryptoObject00.g", "ViewLayer.CryptoObject00.b", "ViewLayer.Combined.R", "ViewLayer.Combined.G", "ViewLayer.Combined.B"]).as_deref(), Some("ViewLayer.Combined"));
    assert_eq!(of(&["Depth.Z", "AO.Y"]).as_deref(), Some("AO"), "a grey pass, not depth");
    assert_eq!(of(&["R", "G", "B", "beauty.R", "beauty.G", "beauty.B"]).as_deref(), Some(""));
}

/// Deep files OpenImageIO wrote (`deep.py`): their samples as written, and flattened from the
/// nearest (OpenImageIO's own flattening keeps the order they are stored in; OpenEXR's guide to
/// deep pixels sorts them by depth): the files in `VEYMELO_EXR_DIR`.
#[test]
#[ignore]
fn deep_files_are_read_and_flattened_by_depth() {
    let Some(dir) = std::env::var_os("VEYMELO_EXR_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    // pixel (x, y): (x + y) % 3 samples, sample s at depth 10 − 3s + x/10, half seen, its
    // colour (0.2 + 0.1s, x/20, y/10) times its alpha
    let sample = |x: usize, y: usize, s: usize| -> [f32; 6] {
        let (z, a) = (10.0 - 3.0 * s as f32 + 0.1 * x as f32, 0.5);
        [(0.2 + 0.1 * s as f32) * a, 0.05 * x as f32 * a, 0.1 * y as f32 * a, a, z, z + 0.5]
    };
    for name in ["deep_scan_zip", "deep_scan_none", "deep_scan_rle"] {
        let bytes = std::fs::read(dir.join(format!("{name}.exr"))).unwrap();
        let exr = parse(&bytes).unwrap();
        let deep = exr.deep(&bytes, 0).unwrap();
        let starts = deep.starts();
        let names: Vec<&str> = deep.channels.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, ["A", "B", "G", "R", "Z", "ZBack"], "{name}");
        let order = [3, 2, 1, 0, 4, 5];
        for y in 0..6 {
            for x in 0..8 {
                let p = y * 8 + x;
                assert_eq!(deep.counts[p] as usize, (x + y) % 3, "{name} ({x}, {y})");
                for s in 0..deep.counts[p] as usize {
                    let want = sample(x, y, s);
                    for (k, &c) in order.iter().enumerate() {
                        assert!((deep.channels[c].1[starts[p] + s] - want[k]).abs() < 1e-5, "{name} ({x}, {y}) sample {s} {}", names[c]);
                    }
                }
            }
        }
        // as a picture: the file's size
        let pic = read_picture(&bytes).unwrap();
        assert_eq!((pic.width, pic.height), (8, 6));
        // flattened: the nearer sample (the later one) over the farther
        let (_, planes) = read(&bytes).unwrap();
        let plane = |n: &str| &planes.iter().find(|p| p.name == n).unwrap().samples;
        let (r, a, z) = (plane("R"), plane("A"), plane("Z"));
        for (x, y) in [(1, 0), (2, 0), (5, 1), (0, 0)] {
            let p = y * 8 + x;
            let n = (x + y) % 3;
            let (mut want_r, mut want_a, mut through) = (0.0, 0.0, 1.0);
            for s in (0..n).rev() {
                let v = sample(x, y, s);
                want_r += through * v[0];
                want_a += through * v[3];
                through *= 1.0 - v[3];
            }
            assert!((r[p] - want_r).abs() < 1e-5 && (a[p] - want_a).abs() < 1e-5, "{name} ({x}, {y}): {} {}", r[p], a[p]);
            if n > 0 {
                assert!((z[p] - sample(x, y, n - 1)[4]).abs() < 1e-5, "the nearest depth");
            }
        }
    }
}

/// A deep file 4 × 4 of channels A, R, Z (floats) as the layout says: scanlines in ZIP chunks of
/// sixteen lines (each line channel by channel), or 2 × 2 tiles (each channel by channel),
/// pixel (x, y) with (x + y) % 2 + 1 samples, sample s: alpha 0.5, red (x + s) / 10, depth
/// 5 − s.
fn deep_file(tiled: bool) -> Vec<u8> {
    let attr = |out: &mut Vec<u8>, name: &str, kind: &str, v: &[u8]| {
        out.extend(name.as_bytes());
        out.push(0);
        out.extend(kind.as_bytes());
        out.push(0);
        out.extend((v.len() as i32).to_le_bytes());
        out.extend(v);
    };
    let ints = |v: &[i32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let mut chlist = Vec::new();
    for n in ["A", "R", "Z"] {
        chlist.extend(n.as_bytes());
        chlist.push(0);
        chlist.extend(ints(&[2, 0, 1, 1]));
    }
    chlist.push(0);
    let count = |x: i32, y: i32| ((x + y) % 2 + 1) as usize;
    let value = |c: usize, x: i32, s: usize| -> f32 {
        match c {
            0 => 0.5,
            1 => (x as f32 + s as f32) / 10.0,
            _ => 5.0 - s as f32,
        }
    };
    // the chunks' windows
    let windows: Vec<(i32, i32, i32, i32)> = if tiled { vec![(0, 0, 1, 1), (2, 0, 3, 1), (0, 2, 1, 3), (2, 2, 3, 3)] } else { vec![(0, 0, 3, 3)] };
    let mut chunks = Vec::new();
    for (k, &(x0, y0, x1, y1)) in windows.iter().enumerate() {
        let mut table = Vec::new();
        let mut so_far = 0i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                so_far += count(x, y) as i32;
                table.extend(so_far.to_le_bytes());
            }
        }
        let mut samples = Vec::new();
        if tiled {
            for c in 0..3 {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        for s in 0..count(x, y) {
                            samples.extend(value(c, x, s).to_le_bytes());
                        }
                    }
                }
            }
        } else {
            for y in y0..=y1 {
                for c in 0..3 {
                    for x in x0..=x1 {
                        for s in 0..count(x, y) {
                            samples.extend(value(c, x, s).to_le_bytes());
                        }
                    }
                }
            }
        }
        let (t, s) = if tiled { (table.clone(), samples.clone()) } else { (crate::write::zip_bytes(&table), crate::write::zip_bytes(&samples)) };
        let mut chunk = if tiled { ints(&[(k % 2) as i32, (k / 2) as i32, 0, 0]) } else { ints(&[y0]) };
        chunk.extend((t.len() as u64).to_le_bytes());
        chunk.extend((s.len() as u64).to_le_bytes());
        chunk.extend((samples.len() as u64).to_le_bytes());
        chunk.extend(t);
        chunk.extend(s);
        chunks.push(chunk);
    }
    let mut out = vec![0x76, 0x2f, 0x31, 0x01];
    out.extend((2u32 | 0x800).to_le_bytes());
    attr(&mut out, "channels", "chlist", &chlist);
    attr(&mut out, "compression", "compression", &[if tiled { 0 } else { 3 }]);
    attr(&mut out, "dataWindow", "box2i", &ints(&[0, 0, 3, 3]));
    attr(&mut out, "displayWindow", "box2i", &ints(&[0, 0, 3, 3]));
    attr(&mut out, "lineOrder", "lineOrder", &[0]);
    attr(&mut out, "pixelAspectRatio", "float", &1f32.to_le_bytes());
    attr(&mut out, "screenWindowCenter", "v2f", &[0; 8]);
    attr(&mut out, "screenWindowWidth", "float", &1f32.to_le_bytes());
    attr(&mut out, "name", "string", b"deep");
    attr(&mut out, "type", "string", if tiled { b"deeptile" } else { b"deepscanline" });
    attr(&mut out, "version", "int", &ints(&[1]));
    attr(&mut out, "chunkCount", "int", &ints(&[chunks.len() as i32]));
    attr(&mut out, "maxSamplesPerPixel", "int", &ints(&[2]));
    if tiled {
        let mut t = ints(&[2, 2]);
        t.push(0);
        attr(&mut out, "tiles", "tiledesc", &t);
    }
    out.push(0);
    let mut at = out.len() + 8 * chunks.len();
    for c in &chunks {
        out.extend((at as u64).to_le_bytes());
        at += c.len();
    }
    for c in chunks {
        out.extend(c);
    }
    out
}

#[test]
fn deep_tiles_and_zip_scanlines_are_read() {
    for tiled in [false, true] {
        let bytes = deep_file(tiled);
        let exr = parse(&bytes).unwrap();
        let deep = exr.deep(&bytes, 0).unwrap();
        let starts = deep.starts();
        for y in 0..4 {
            for x in 0..4 {
                let p = (y * 4 + x) as usize;
                let n = ((x + y) % 2 + 1) as usize;
                assert_eq!(deep.counts[p] as usize, n, "tiled {tiled} ({x}, {y})");
                for s in 0..n {
                    assert_eq!(deep.channels[1].1[starts[p] + s], (x as f32 + s as f32) / 10.0, "tiled {tiled} ({x}, {y}) {s}");
                    assert_eq!(deep.channels[2].1[starts[p] + s], 5.0 - s as f32);
                }
            }
        }
        // flattened: the second sample (nearer) over the first
        let (_, planes) = read(&bytes).unwrap();
        let r = &planes.iter().find(|p| p.name == "R").unwrap().samples;
        assert!((r[1] - (0.2 + 0.5 * 0.1)).abs() < 1e-6, "{}", r[1]);
        assert!((r[0] - 0.0).abs() < 1e-6);
    }
}
