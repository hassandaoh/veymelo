use super::*;

use crate::write::{layer, write};

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


