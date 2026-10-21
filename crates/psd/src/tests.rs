use super::*;

use crate::adjust::Adjustment;
use crate::descriptor::{Desc, Item};
use crate::vector::Join;
use crate::write::{desc_block, effects_block, layer, linked_block, placed_block, type_block, vector_block, write, write_with};

fn plane(n: usize, v: u8) -> Vec<u8> {
    vec![v; n]
}

#[test]
fn layers_groups_and_masks() {
    let mut back = layer("Back", [0, 0, 20, 30]);
    back.channels = vec![(0, 0, plane(600, 255)), (1, 0, plane(600, 0)), (2, 0, plane(600, 0))];
    let mut end = layer("</Layer group>", [0, 0, 0, 0]);
    end.section = Some(3);
    end.hidden = true;
    // a green square, its alpha a ramp, packed with RLE
    let mut inside = layer("Inside ไทย", [5, 5, 15, 15]);
    let ramp: Vec<u8> = (0..100).map(|k| (k % 10 * 25) as u8).collect();
    inside.channels = vec![(-1, 1, ramp.clone()), (0, 1, plane(100, 0)), (1, 1, plane(100, 255)), (2, 2, plane(100, 0))];
    let mut folder = layer("Group", [0, 0, 0, 0]);
    folder.section = Some(1);
    folder.blend = b"pass";
    folder.opacity = 128;
    // a blue layer, hidden, half shown by its mask
    let mut top = layer("Top", [0, 0, 20, 30]);
    top.hidden = true;
    top.channels = vec![(0, 3, plane(600, 0)), (1, 3, plane(600, 0)), (2, 3, plane(600, 255)), (-2, 0, plane(150, 255))];
    top.mask = Some(([0, 0, 10, 15], 0));
    let file = write(30, 20, 8, &[back, end, inside, folder, top]);
    let doc = parse(file).unwrap();
    assert_eq!((doc.width, doc.height, doc.depth, doc.mode, doc.layers.len()), (30, 20, 8, 3, 5));
    // top first, the group holding its layer
    assert_eq!(doc.tree(), vec![Node::Layer(4), Node::Group(3, vec![Node::Layer(2)]), Node::Layer(0)]);
    let g = &doc.layers[3];
    assert_eq!((g.name.as_str(), &g.blend, g.opacity, g.kind), ("Group", b"pass", 128, Kind::Group));
    assert_eq!(doc.layers[2].name, "Inside ไทย");
    assert!(doc.layers[4].hidden && !doc.layers[0].hidden);
    // pixels: red; green with its ramp of alpha (RLE and ZIP); blue under its mask
    let p = doc.pixels(0).unwrap();
    assert_eq!((p.left, p.top, p.width, p.height, p.px[0]), (0, 0, 30, 20, [1.0, 0.0, 0.0, 1.0]));
    let p = doc.pixels(2).unwrap();
    assert_eq!((p.left, p.top, p.width, p.height), (5, 5, 10, 10));
    assert_eq!(p.px[3], [0.0, 1.0, 0.0, 75.0 / 255.0]);
    let p = doc.pixels(4).unwrap();
    let (w, px) = (p.width, p.px);
    assert_eq!((px[0], px[16][3], px[14 * w][3]), ([0.0, 0.0, 1.0, 1.0], 0.0, 0.0), "the mask covers the top left 15 × 10, nothing outside it");
    assert_eq!(px[5 * w + 5][3], 1.0);
    assert_eq!(doc.pixels(1), None, "a divider has no pixels");
}

#[test]
fn sixteen_bits_predicted() {
    // a 16-bit grey ramp in a ZIP with prediction
    let mut s = Vec::new();
    for _y in 0..4 {
        for x in 0..8u16 {
            s.extend((x * 8000).to_be_bytes());
        }
    }
    let mut l = layer("Ramp", [0, 0, 4, 8]);
    l.channels = vec![(0, 3, s.clone()), (1, 3, s.clone()), (2, 3, s)];
    let doc = parse(write(8, 4, 16, &[l])).unwrap();
    let px = doc.pixels(0).unwrap().px;
    assert!((px[7][0] - 56000.0 / 65535.0).abs() < 1e-6 && (px[3][1] - 24000.0 / 65535.0).abs() < 1e-6);
    assert!(parse(b"GIF89a".to_vec()).unwrap_err().contains("not a Photoshop"));
}




fn desc(class: &str, items: Vec<(&str, Item)>) -> Desc {
    Desc { class: class.into(), items: items.into_iter().map(|(k, v)| (k.to_string(), v)).collect() }
}

fn unit(u: &str, v: f64) -> Item {
    Item::Unit(u.into(), v)
}

fn rgb(r: f64, g: f64, b: f64) -> Item {
    Item::Desc(desc("RGBC", vec![("Rd  ", Item::Double(r)), ("Grn ", Item::Double(g)), ("Bl  ", Item::Double(b))]))
}

fn en(t: &str, v: &str) -> Item {
    Item::Enum(t.into(), v.into())
}

/// A square's knots (corners, no curves).
fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<[[f64; 2]; 3]> {
    [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|p| [p, p, p]).to_vec()
}

#[test]
fn vector_masks_feathers_and_the_light() {
    // a ring (a square with a square hole merged into it), then a square taken away, inverted
    let paths = vec![(square(10.0, 10.0, 90.0, 90.0), true, 1), (square(30.0, 30.0, 70.0, 70.0), true, -1), (square(0.0, 0.0, 20.0, 20.0), true, 2), (square(5.0, 5.0, 8.0, 8.0), false, 3)];
    let mut l = layer("Shape", [0, 0, 50, 100]);
    l.info = vec![(*b"vmsk", vector_block(&paths, 100.0, 50.0, 1))];
    // a mask drawn from it (left to the vector mask), and a layer with a feathered mask of its own
    l.channels = vec![(0, 0, plane(5000, 255)), (1, 0, plane(5000, 255)), (2, 0, plane(5000, 255)), (-2, 0, plane(5000, 0))];
    (l.mask, l.mask_rendered) = (Some(([0, 0, 50, 100], 0)), true);
    let mut soft = layer("Soft", [0, 0, 40, 60]);
    let half: Vec<u8> = (0..2400).map(|k| if k % 60 < 30 { 255 } else { 0 }).collect();
    soft.channels = vec![(0, 0, plane(2400, 255)), (1, 0, plane(2400, 255)), (2, 0, plane(2400, 255)), (-2, 0, half)];
    (soft.mask, soft.mask_feather) = (Some(([0, 0, 40, 60], 0)), 8.0);
    let file = write_with(100, 50, 8, &[l, soft], &[(1037, 75i32.to_be_bytes().to_vec()), (1049, 40i32.to_be_bytes().to_vec())], &[]);
    let doc = parse(file).unwrap();
    assert_eq!(doc.global_light(), (75.0, 40.0));
    let v = doc.vector_mask(0).unwrap();
    assert!(v.inverted && !v.disabled);
    let joins: Vec<(Join, bool, bool)> = v.paths.iter().map(|p| (p.join, p.merged, p.closed)).collect();
    assert_eq!(joins, [(Join::Add, false, true), (Join::Add, true, true), (Join::Subtract, false, true), (Join::Intersect, false, false)]);
    let k = v.paths[0].knots[2];
    assert!((k.at[0] - 90.0).abs() < 1e-5 && (k.at[1] - 90.0).abs() < 1e-5, "{k:?}");
    // the drawn mask hides nothing: the vector mask is read apart
    assert_eq!(doc.pixels(0).unwrap().px[0][3], 1.0);
    // the feathered mask soft across its edge, whole far from it
    let m = doc.layers[1].mask.clone().unwrap();
    assert_eq!((m.feather, m.channel, m.rendered), (8.0, -2, false));
    let px = doc.pixels(1).unwrap().px;
    let row: Vec<f32> = (0..60).map(|x| px[20 * 60 + x][3]).collect();
    assert!(row[15] > 0.95 && row[45] < 0.05 && (row[29] - 0.5).abs() < 0.1 && row[25] > row[29] && row[29] > row[33], "{row:?}");
}

#[test]
fn effects_fills_and_adjustments() {
    let shadow = desc("DrSh", vec![("enab", Item::Bool(true)), ("Md  ", en("BlnM", "Mltp")), ("Clr ", rgb(255.0, 0.0, 0.0)), ("Opct", unit("#Prc", 60.0)), ("uglg", Item::Bool(false)), ("lagl", unit("#Ang", 45.0)), ("Dstn", unit("#Pxl", 7.0)), ("blur", unit("#Pxl", 9.0))]);
    let stroke = desc("FrFX", vec![("enab", Item::Bool(false)), ("Styl", en("FStl", "InsF")), ("Sz  ", unit("#Pxl", 4.0)), ("Clr ", rgb(0.0, 0.0, 255.0))]);
    let fx = desc("null", vec![("Scl ", unit("#Prc", 100.0)), ("masterFXSwitch", Item::Bool(true)), ("DrSh", Item::Desc(shadow)), ("FrFX", Item::Desc(stroke))]);
    let mut a = layer("Styled", [0, 0, 4, 4]);
    a.info = vec![(*b"lfx2", effects_block(&fx)), (*b"SoCo", desc_block(&desc("null", vec![("Clr ", rgb(0.0, 128.0, 255.0))])))];
    // levels: version, then the composite and red's records
    let mut levl = 2u16.to_be_bytes().to_vec();
    for v in [10u16, 245, 0, 255, 150, 0, 255, 20, 230, 100] {
        levl.extend(v.to_be_bytes());
    }
    // curves, version 1: the composite (bit 0) and blue (bit 3), each its points (out, in)
    let mut curv = vec![0u8];
    curv.extend(1u16.to_be_bytes());
    curv.extend(0b1001u32.to_be_bytes());
    for pts in [&[(0u16, 0u16), (200, 128), (255, 255)][..], &[(30, 0), (255, 255)]] {
        curv.extend((pts.len() as u16).to_be_bytes());
        for (o, i) in pts {
            curv.extend(o.to_be_bytes());
            curv.extend(i.to_be_bytes());
        }
    }
    // hue/saturation: master +30° and −20 saturation, reds' range and lightness
    let mut hue = 2u16.to_be_bytes().to_vec();
    hue.extend([0, 0]);
    for v in [0i16, 25, 0, 30, -20, 0, 315, 345, 15, 45, 0, 0, 40] {
        hue.extend(v.to_be_bytes());
    }
    let mut blnc = Vec::new();
    for v in [10i16, 0, 0, 0, -20, 0, 0, 0, 30] {
        blnc.extend(v.to_be_bytes());
    }
    blnc.push(1);
    let mut expa = 1u16.to_be_bytes().to_vec();
    for v in [1.5f32, 0.1, 0.8] {
        expa.extend(v.to_be_bytes());
    }
    let blocks: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"levl", levl),
        (*b"curv", curv),
        (*b"hue2", hue),
        (*b"blnc", blnc),
        (*b"thrs", [128u16.to_be_bytes(), [0, 0]].concat()),
        (*b"expA", expa),
        (*b"nvrt", Vec::new()),
        (*b"vibA", desc_block(&desc("null", vec![("vibrance", Item::Int(40)), ("Strt", Item::Int(-10))]))),
    ];
    let mut layers = vec![a];
    for (key, b) in &blocks {
        let mut l = layer("Adjust", [0, 0, 0, 0]);
        l.info = vec![(*key, b.clone())];
        layers.push(l);
    }
    let doc = parse(write(4, 4, 8, &layers)).unwrap();
    let e = doc.effects(0).unwrap();
    assert!(e.shown && e.scale == 100.0);
    assert_eq!(e.list.iter().map(|(k, d)| (*k, d.flag("enab"))).collect::<Vec<_>>(), [("dropShadow", Some(true)), ("stroke", Some(false))]);
    let ds = &e.list[0].1;
    assert_eq!((ds.color("Clr "), ds.num("lagl"), ds.enumeration("Md  ")), (Some([1.0, 0.0, 0.0]), Some(45.0), Some("Mltp")));
    let crate::looks::Fill::Solid(c) = doc.fill(0).unwrap() else { panic!() };
    assert!((c[1] - 128.0 / 255.0).abs() < 1e-9 && c[2] == 1.0);
    let got: Vec<Adjustment> = (1..=blocks.len()).map(|i| doc.adjustment(i).unwrap()).collect();
    assert!(doc.layers[1].adjustment);
    let Adjustment::Levels(lv) = &got[0] else { panic!("{:?}", got[0]) };
    assert_eq!(lv.len(), 2);
    assert!((lv[0][0] - 10.0 / 255.0).abs() < 1e-9 && lv[0][4] == 1.5 && (lv[1][3] - 230.0 / 255.0).abs() < 1e-9);
    assert_eq!(got[1], Adjustment::Curves(vec![(0, vec![[0.0, 0.0], [128.0 / 255.0, 200.0 / 255.0], [1.0, 1.0]]), (3, vec![[0.0, 30.0 / 255.0], [1.0, 1.0]])]));
    let Adjustment::HueSaturation { colorize, master, ranges } = &got[2] else { panic!() };
    assert_eq!((colorize, *master, ranges[0].range, ranges[0].hsl), (&None, [30.0, -20.0, 0.0], [315.0, 345.0, 15.0, 45.0], [0.0, 0.0, 40.0]));
    assert_eq!(got[3], Adjustment::ColorBalance { shadows: [10.0, 0.0, 0.0], midtones: [0.0, -20.0, 0.0], highlights: [0.0, 0.0, 30.0], luminosity: true });
    assert_eq!(got[4], Adjustment::Threshold(128.0));
    let Adjustment::Exposure { exposure, offset, gamma } = got[5] else { panic!() };
    assert!((exposure - 1.5).abs() < 1e-6 && (offset - 0.1).abs() < 1e-6 && (gamma - 0.8).abs() < 1e-6);
    assert_eq!((got[6].clone(), got[7].clone()), (Adjustment::Invert, Adjustment::Vibrance { vibrance: 40.0, saturation: -10.0 }));
}

/// A string as EngineData keeps it: UTF-16 after its byte order mark, `(`, `)` and `\` escaped.
fn engine_string(s: &str) -> Vec<u8> {
    let mut out = b"(".to_vec();
    for b in [0xfe, 0xff].into_iter().chain(s.encode_utf16().flat_map(|u| u.to_be_bytes())) {
        if matches!(b, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(b);
    }
    out.push(b')');
    out
}

#[test]
fn type_layers_and_smart_objects() {
    // two runs (a red "Hi (" in one font, then a blue line in another), the second paragraph
    // centred, point text
    let mut e = b"\n\n<<\n\t/EngineDict\n\t<<\n\t\t/Editor\n\t\t<<\n\t\t\t/Text ".to_vec();
    e.extend(engine_string("Hi (\rสวัสดี\r"));
    e.extend(
        b"\n\t\t>>\n\t\t/ParagraphRun << /RunArray [ << /ParagraphSheet << /Properties << /Justification 0 >> >> >> << /ParagraphSheet << /Properties << /Justification 2 /SpaceBefore 6.0 >> >> >> ] /RunLengthArray [ 5 7 ] >>\n\t\t/StyleRun << /RunArray [ << /StyleSheet << /StyleSheetData << /Font 0 /FontSize 48.0 /AutoLeading false /Leading 60.0 /Tracking 50 /FillColor << /Type 1 /Values [ 1.0 1.0 0.0 0.0 ] >> /FauxBold true >> >> >> << /StyleSheet << /StyleSheetData << /Font 1 /FontSize 24.0 /FillColor << /Type 1 /Values [ 1.0 0.0 0.0 1.0 ] >> /FontCaps 2 >> >> >> ] /RunLengthArray [ 4 8 ] >>\n\t\t/Rendered << /Shapes << /WritingDirection 0 /Children [ << /ShapeType 0 /Cookie << /Photoshop << /ShapeType 0 /PointBase [ 0.0 0.0 ] >> >> >> ] >> >>\n\t>>\n\t/ResourceDict\n\t<<\n\t\t/FontSet [ << /Name ",
    );
    e.extend(engine_string("Helvetica-Bold"));
    e.extend(b" /Script 0 >> << /Name ");
    e.extend(engine_string("Thonburi"));
    e.extend(b" >> ]\n\t\t/StyleSheetSet [ << /Name (Normal) /StyleSheetData << /FontSize 12.0 /Ligatures false >> >> ]\n\t>>\n>>");
    let text = desc("TxLr", vec![("Txt ", Item::Text("Hi (\rสวัสดี\r".into())), ("Ornt", en("Ornt", "Hrzn")), ("EngineData", Item::Data(e))]);
    let mut t = layer("Hello", [0, 0, 10, 10]);
    t.info = vec![(*b"TySh", type_block([2.0, 0.0, 0.0, 2.0, 30.0, 70.0], &text))];
    // a smart object holding a picture, pinned by its corners
    let place = desc(
        "null",
        vec![
            ("Idnt", Item::Text("abc-123".into())),
            ("Trnf", Item::List([10.0, 10.0, 90.0, 20.0, 80.0, 60.0, 15.0, 50.0].map(Item::Double).to_vec())),
            ("Sz  ", Item::Desc(desc("Pnt ", vec![("Wdth", Item::Double(32.0)), ("Hght", Item::Double(16.0))]))),
        ],
    );
    let mut so = layer("Placed", [10, 10, 60, 90]);
    so.info = vec![(*b"SoLd", placed_block(&place))];
    let picture = b"\x89PNG not really".to_vec();
    let file = write_with(100, 100, 8, &[t, so], &[], &[(*b"lnk2", linked_block("abc-123", "photo.png", b"png ", &picture))]);
    let doc = parse(file).unwrap();
    assert!(doc.layers[0].text && doc.layers[1].smart);
    let ty = doc.type_layer(0).unwrap().unwrap();
    assert_eq!((ty.transform, ty.text.as_str(), ty.vertical, ty.boxed), ([2.0, 0.0, 0.0, 2.0, 30.0, 70.0], "Hi (\nสวัสดี\n", false, None));
    assert_eq!(ty.runs.iter().map(|r| r.0).collect::<Vec<_>>(), [4, 8]);
    let (a, b) = (&ty.runs[0].1, &ty.runs[1].1);
    assert_eq!((a.font.as_deref(), a.size, a.leading, a.tracking, a.fill, a.faux_bold, a.ligatures), (Some("Helvetica-Bold"), 48.0, Some(60.0), 50.0, Some([1.0, 0.0, 0.0, 1.0]), true, false));
    assert_eq!((b.font.as_deref(), b.size, b.leading, b.caps, b.fill), (Some("Thonburi"), 24.0, None, 2, Some([0.0, 0.0, 1.0, 1.0])));
    assert_eq!(ty.paragraphs.iter().map(|p| (p.0, p.1.justification, p.1.space_before)).collect::<Vec<_>>(), [(5, 0, 0.0), (7, 2, 6.0)]);
    let p = doc.placed(1).unwrap();
    assert_eq!((p.id.as_str(), p.corners, p.size, p.warped), ("abc-123", [[10.0, 10.0], [90.0, 20.0], [80.0, 60.0], [15.0, 50.0]], Some([32.0, 16.0]), false));
    let files = doc.linked_files();
    assert_eq!(files.len(), 1);
    assert_eq!((files[0].id.as_str(), files[0].name.as_str(), &files[0].kind), ("abc-123", "photo.png", b"png "));
    assert_eq!(doc.linked_data(&files[0]), Some(&picture[..]));
}

#[test]
fn sixteen_bit_layers_in_their_own_block() {
    // a 16-bit document keeps its layers in `Lr16`, the layer info itself empty
    let mut l = layer("Deep", [0, 0, 2, 2]);
    l.channels = vec![(0, 0, [0xffu8, 0xff].repeat(4)), (1, 0, vec![0; 8]), (2, 0, vec![0; 8])];
    let inner = write(2, 2, 16, &[l]);
    // the layer info from the written document: after its header (26), mode data and resources
    // (4 + 4), the section's length (4) and the layer info's (4)
    let n = u32::from_be_bytes(inner[38..42].try_into().unwrap()) as usize;
    let lr16 = inner[42..42 + n].to_vec();
    let mut doc = write_with(2, 2, 16, &[], &[], &[(*b"Lr16", lr16)]);
    doc.truncate(doc.len());
    let d = parse(doc).unwrap();
    assert_eq!(d.layers.len(), 1);
    assert_eq!(d.pixels(0).unwrap().px[3], [1.0, 0.0, 0.0, 1.0]);
}
