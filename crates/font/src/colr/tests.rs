use super::*;

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend(x.to_be_bytes());
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend(x.to_be_bytes());
}

/// A glyph of one rectangle, its four corners on the curve.
fn rect(x0: i16, y0: i16, x1: i16, y1: i16) -> Vec<u8> {
    let mut g = Vec::new();
    for v in [1, x0, y0, x1, y1] {
        g.extend(v.to_be_bytes());
    }
    g.extend(3u16.to_be_bytes());
    g.extend(0u16.to_be_bytes());
    g.extend([1u8; 4]);
    for (a, b) in [(0, x0), (x0, x0), (x0, x1), (x1, x1)] {
        g.extend((b - a).to_be_bytes());
    }
    for (a, b) in [(0, y0), (y0, y1), (y1, y1), (y1, y0)] {
        g.extend((b - a).to_be_bytes());
    }
    g
}

/// A font of 1000 units an em whose glyphs 1, 2 and 3 are the rectangles (100, 0)–(500, 400),
/// (0, 0)–(300, 600) and (300, 0)–(600, 600) (none with `outlines` false), "A" glyph 1, with
/// `more` tables.
pub(crate) fn font_with(more: Vec<([u8; 4], Vec<u8>)>, outlines: bool) -> Font {
    let glyphs = [Vec::new(), rect(100, 0, 500, 400), rect(0, 0, 300, 600), rect(300, 0, 600, 600)];
    let (mut glyf, mut loca) = (Vec::new(), Vec::new());
    for g in &glyphs {
        be16(&mut loca, (glyf.len() / 2) as u16);
        glyf.extend(g);
    }
    be16(&mut loca, (glyf.len() / 2) as u16);
    let mut head = vec![0u8; 54];
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&4u16.to_be_bytes());
    let mut maxp = vec![0, 0, 0x50, 0];
    be16(&mut maxp, 4);
    let mut hmtx = Vec::new();
    for _ in 0..4 {
        be16(&mut hmtx, 700);
        be16(&mut hmtx, 0);
    }
    let mut cmap = Vec::new();
    for v in [0u16, 1, 3, 1] {
        be16(&mut cmap, v);
    }
    be32(&mut cmap, 12);
    for v in [4u16, 32, 0, 4, 4, 1, 0, 0x41, 0xFFFF, 0, 0x41, 0xFFFF, 1u16.wrapping_sub(0x41), 1, 0, 0] {
        be16(&mut cmap, v);
    }
    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![(*b"cmap", cmap), (*b"head", head), (*b"hhea", hhea), (*b"hmtx", hmtx), (*b"maxp", maxp)];
    if outlines {
        tables.extend([(*b"glyf", glyf), (*b"loca", loca)]);
    }
    tables.extend(more);
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    for v in [tables.len() as u16, 128, 3, 16] {
        be16(&mut out, v);
    }
    let mut body = Vec::new();
    for (tag, t) in &tables {
        out.extend(tag);
        be32(&mut out, 0);
        be32(&mut out, (12 + 16 * tables.len() + body.len()) as u32);
        be32(&mut out, t.len() as u32);
        body.extend(t);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }
    out.extend(body);
    Font::from_data(out, 0).unwrap()
}

/// A palette of red, green and blue.
pub(crate) fn cpal() -> Vec<u8> {
    let mut c = Vec::new();
    for v in [0u16, 3, 1, 3] {
        be16(&mut c, v);
    }
    be32(&mut c, 14);
    be16(&mut c, 0);
    c.extend([0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255]);
    c
}

/// A paint table: its own bytes `head`, then each of `kids`, the three-byte offset at each's
/// place in `head` set to where it is put.
pub(crate) fn with_kids(mut head: Vec<u8>, kids: Vec<(usize, Vec<u8>)>) -> Vec<u8> {
    for (at, kid) in kids {
        let off = head.len() as u32;
        head[at..at + 3].copy_from_slice(&off.to_be_bytes()[1..]);
        head.extend(kid);
    }
    head
}

/// A table of offsets to `items` (each four bytes, from the table's start) after `count`
/// (four bytes) and with each record's own bytes before its offset.
fn list(records: Vec<(Vec<u8>, Vec<u8>)>) -> Vec<u8> {
    let size: usize = records.iter().map(|(r, _)| r.len() + 4).sum();
    let mut out = Vec::new();
    be32(&mut out, records.len() as u32);
    let mut body = Vec::new();
    for (r, item) in records {
        out.extend(r);
        be32(&mut out, (4 + size + body.len()) as u32);
        body.extend(item);
    }
    out.extend(body);
    out
}

/// A `COLR` version 1 table of base glyphs `bases` (glyph, its paint), layers `layers`, and a
/// clip list `clips` (bytes, or none).
pub(crate) fn colr(bases: Vec<(u16, Vec<u8>)>, layers: Vec<Vec<u8>>, clips: Option<Vec<u8>>, map: Option<Vec<u8>>, store: Option<Vec<u8>>) -> Vec<u8> {
    let base_list = list(bases.into_iter().map(|(g, p)| (g.to_be_bytes().to_vec(), p)).collect());
    let layer_list = list(layers.into_iter().map(|p| (Vec::new(), p)).collect());
    let mut out = Vec::new();
    for v in [1u16, 0] {
        be16(&mut out, v);
    }
    be32(&mut out, 0);
    be32(&mut out, 0);
    be16(&mut out, 0);
    let mut body = Vec::new();
    let mut place = |t: Option<Vec<u8>>, out: &mut Vec<u8>| match t {
        Some(t) => {
            be32(out, (34 + body.len()) as u32);
            body.extend(t);
        }
        None => be32(out, 0),
    };
    place(Some(base_list), &mut out);
    place(Some(layer_list), &mut out);
    place(clips, &mut out);
    place(map, &mut out);
    place(store, &mut out);
    out.extend(body);
    out
}

fn solid(index: u16, alpha: f64) -> Vec<u8> {
    let mut p = vec![2];
    be16(&mut p, index);
    be16(&mut p, (alpha * 16384.0) as i16 as u16);
    p
}

pub(crate) fn line(extend: u8, stops: &[(f64, u16, f64)]) -> Vec<u8> {
    let mut l = vec![extend];
    be16(&mut l, stops.len() as u16);
    for &(o, i, a) in stops {
        be16(&mut l, (o * 16384.0) as i16 as u16);
        be16(&mut l, i);
        be16(&mut l, (a * 16384.0) as i16 as u16);
    }
    l
}

fn words(format: u8, values: &[i16]) -> Vec<u8> {
    let mut p = vec![format, 0, 0, 0];
    for &v in values {
        be16(&mut p, v as u16);
    }
    p
}

pub(crate) fn glyph(g: u16, kid: Vec<u8>) -> Vec<u8> {
    let mut p = vec![10, 0, 0, 0];
    be16(&mut p, g);
    with_kids(p, vec![(1, kid)])
}

fn red() -> Ink {
    Ink { colour: Some([1.0, 0.0, 0.0, 1.0]), alpha: 1.0 }
}

fn blue() -> Ink {
    Ink { colour: Some([0.0, 0.0, 1.0, 1.0]), alpha: 1.0 }
}

fn close(a: [f64; 6], b: [f64; 6]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

#[test]
fn every_kind_of_paint_is_read() {
    let stops = [(1.0, 2, 1.0), (0.0, 0, 1.0)];
    // glyph 1: two layers, a linear gradient (its stops given backwards) moved by a matrix in
    // glyph 2, and a radial gradient in glyph 3 turned a quarter about (300, 300)
    let mut affine = Vec::new();
    for v in [2.0f64, 0.0, 0.0, 1.0, 10.0, -5.0] {
        be32(&mut affine, (v * 65536.0) as i32 as u32);
    }
    let linear = with_kids(words(4, &[0, 0, 300, 0, 0, 300]), vec![(1, line(1, &stops))]);
    let transform = with_kids(vec![12, 0, 0, 0, 0, 0, 0], vec![(1, linear), (4, affine)]);
    let radial = with_kids(words(6, &[100, 100, 0, 100, 100, 200]), vec![(1, line(0, &stops))]);
    let rotate = with_kids(words(26, &[0x2000, 300, 300]), vec![(1, glyph(3, radial))]);
    let layers = vec![glyph(2, transform), rotate];
    let mut first = vec![1, 2];
    be32(&mut first, 0);
    // glyph 2: a sweep blended by multiplying over glyph 1's colour, and the rest of the
    // transforms each round a solid colour
    let sweep = with_kids(words(8, &[50, 60, -0x4000, 0]), vec![(1, line(2, &stops))]);
    let composite = with_kids(vec![32, 0, 0, 0, 23, 0, 0, 0], vec![(1, sweep), (5, vec![11, 0, 1])]);
    let wrapped = |format: u8, values: &[i16]| with_kids(words(format, values), vec![(1, solid(1, 0.5))]);
    let transforms = [
        wrapped(14, &[7, -3]),
        wrapped(16, &[0x8000u16 as i16, 0x2000]),
        wrapped(18, &[0x2000, 0x2000, 100, 200]),
        wrapped(20, &[0x6000]),
        wrapped(22, &[0x6000, 10, 20]),
        wrapped(24, &[0x2000]),
        wrapped(28, &[0x1000, 0]),
        wrapped(30, &[0, 0x1000, 50, 50]),
    ];
    // glyph 3: itself, through a glyph of its own
    let itself = glyph(1, vec![11, 0, 3]);
    let mut all = layers;
    all.extend(transforms);
    let font = font_with(vec![(*b"COLR", colr(vec![(1, first), (2, glyph(2, composite)), (3, itself)], all, None, None, None)), (*b"CPAL", cpal())], true);
    let g1 = font.color_glyph(1).unwrap();
    let sorted = ColorLine { extend: Extend::Repeat, stops: vec![(0.0, red()), (1.0, blue())] };
    let Paint::Layers(l) = &g1.paint else { panic!("{:?}", g1.paint) };
    assert_eq!(l.len(), 2);
    let Paint::Glyph { glyph: 2, paint } = &l[0] else { panic!("{:?}", l[0]) };
    let Paint::Transform { m, paint } = &**paint else { panic!() };
    assert!(close(*m, [2.0, 0.0, 0.0, 1.0, 10.0, -5.0]), "{m:?}");
    assert_eq!(**paint, Paint::Linear { line: sorted.clone(), p0: [0.0, 0.0], p1: [300.0, 0.0], p2: [0.0, 300.0] });
    let Paint::Transform { m, paint } = &l[1] else { panic!("{:?}", l[1]) };
    // a quarter turn counter-clockwise about (300, 300): (600, 300) to (300, 600)
    assert!(close(*m, [0.0, 1.0, -1.0, 0.0, 600.0, 0.0]), "{m:?}");
    let Paint::Glyph { glyph: 3, paint } = &**paint else { panic!() };
    assert_eq!(**paint, Paint::Radial { line: ColorLine { extend: Extend::Pad, ..sorted.clone() }, c0: [100.0, 100.0], r0: 0.0, c1: [100.0, 100.0], r1: 200.0 });
    // painted in the box both rectangles reach, the second turned
    assert_eq!(g1.bounds, [0.0, 0.0, 600.0, 600.0]);

    let g2 = font.color_glyph(2).unwrap();
    let Paint::Glyph { glyph: 2, paint } = &g2.paint else { panic!() };
    let Paint::Composite { source, mode: 23, backdrop } = &**paint else { panic!("{paint:?}") };
    // the angles stored less one half turn: -1 is 0°, 0 is 180°
    assert_eq!(**source, Paint::Sweep { line: ColorLine { extend: Extend::Reflect, ..sorted.clone() }, centre: [50.0, 60.0], start: 0.0, end: 180.0 });
    assert_eq!(**backdrop, g1.paint);
    assert_eq!(g2.bounds, [0.0, 0.0, 300.0, 600.0]);
    // the transforms (layers 2 on) as matrices, each round a half-seen green
    let mut r = Reader { font: &font, d: font.data(), bases: 0, layers: 0, map: 0, store: 0, read: 0 };
    let t = font.colr + u32_at(font.data(), font.colr + 18) as usize;
    let half_green = Paint::Solid(Ink { colour: Some([0.0, 1.0, 0.0, 1.0]), alpha: 0.5 });
    let want: [[f64; 6]; 8] = [
        [1.0, 0.0, 0.0, 1.0, 7.0, -3.0],
        [-2.0, 0.0, 0.0, 0.5, 0.0, 0.0],
        [0.5, 0.0, 0.0, 0.5, 50.0, 100.0],
        [1.5, 0.0, 0.0, 1.5, 0.0, 0.0],
        [1.5, 0.0, 0.0, 1.5, -5.0, -10.0],
        [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, -(std::f64::consts::PI / 4.0).tan(), 1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0, 1.0, 0.0, -50.0],
    ];
    for (k, want) in want.iter().enumerate() {
        let at = t + u32_at(font.data(), t + 4 + 4 * (2 + k)) as usize;
        let Paint::Transform { m, paint } = r.paint(at, &mut Vec::new()) else { panic!("{k}") };
        assert!(close(m, *want), "{k}: {m:?}");
        assert_eq!(*paint, half_green);
    }
    // a colour glyph that would hold itself paints nothing there
    let g3 = font.color_glyph(3).unwrap();
    assert_eq!(g3.paint, Paint::Glyph { glyph: 1, paint: Box::new(Paint::Nothing) });
}

#[test]
fn a_clip_box_bounds_the_glyph_and_unbounded_paint_is_not_drawn() {
    // glyph 1 a solid colour everywhere (no bounds), glyph 2 the same but with a clip box
    let mut clips = vec![1];
    be32(&mut clips, 1);
    for v in [2u16, 3] {
        be16(&mut clips, v);
    }
    clips.extend([0, 0, 12]);
    clips.push(1);
    for v in [-10i16, -20, 30, 40] {
        be16(&mut clips, v as u16);
    }
    let table = colr(vec![(1, solid(0, 1.0)), (2, solid(1, 1.0))], vec![], Some(clips), None, None);
    let font = font_with(vec![(*b"COLR", table), (*b"CPAL", cpal())], true);
    assert_eq!(font.color_glyph(1), None);
    assert_eq!(font.color_glyph(2).unwrap().bounds, [-10.0, -20.0, 30.0, 40.0]);
    assert_eq!(font.color_glyph(3), None);
}

#[test]
fn variable_paints_move_with_the_instance() {
    // an axis "wght" 0 to 1 to 2, and a store whose one region peaks at the far end: the stop
    // offsets and alpha of a variable solid move by its deltas
    let mut fvar = Vec::new();
    for v in [1u16, 0, 16, 2, 1, 20, 0, 0] {
        be16(&mut fvar, v);
    }
    fvar.extend(b"wght");
    for v in [0.0f64, 1.0, 2.0] {
        be32(&mut fvar, (v * 65536.0) as i32 as u32);
    }
    be16(&mut fvar, 0);
    be16(&mut fvar, 256);
    // the store: one region (0 → 1 → 1), one data set of two rows (alpha, then x of a point)
    let mut store = Vec::new();
    be16(&mut store, 1);
    be32(&mut store, 12);
    be16(&mut store, 1);
    be32(&mut store, 22);
    // region list
    for v in [1u16, 1, 0, 0x4000, 0x4000] {
        be16(&mut store, v);
    }
    // the data: 2 rows, 1 word column, 1 region (index 0)
    for v in [2u16, 1, 1, 0] {
        be16(&mut store, v);
    }
    be16(&mut store, (-8192i16) as u16);
    be16(&mut store, 100);
    let mut var_solid = vec![3];
    be16(&mut var_solid, 0);
    be16(&mut var_solid, 0x4000);
    be32(&mut var_solid, 0);
    let mut translate = words(15, &[10, 0]);
    be32(&mut translate, 0);
    let table = colr(vec![(1, glyph(1, var_solid)), (2, glyph(2, with_kids(translate, vec![(1, solid(1, 1.0))])))], vec![], None, None, Some(store));
    let font = font_with(vec![(*b"COLR", table), (*b"CPAL", cpal()), (*b"fvar", fvar)], true);
    let solid_at = |f: &Font| match f.color_glyph(1).unwrap().paint {
        Paint::Glyph { paint, .. } => *paint,
        p => p,
    };
    assert_eq!(solid_at(&font), Paint::Solid(Ink { colour: Some([1.0, 0.0, 0.0, 1.0]), alpha: 1.0 }));
    let heavy = font.at(&[("wght", 2.0)]);
    assert_eq!(solid_at(&heavy), Paint::Solid(Ink { colour: Some([1.0, 0.0, 0.0, 1.0]), alpha: 0.5 }));
    // the implicit map: index 0 is set 0 row 0 (the alpha's), so the translate's x (index 0)
    // moves by the same -8192 units, a whole -8192 font units
    let Paint::Glyph { paint, .. } = heavy.color_glyph(2).unwrap().paint else { panic!() };
    let Paint::Transform { m, .. } = *paint else { panic!() };
    assert_eq!(m[4], 10.0 - 8192.0);
}

/// `CBLC` and `CBDT` with two strikes: 20 pixels an em (glyph 1 listed by offsets, small
/// metrics before its PNG) and 40 (glyphs 1 to 3 the same size each, big metrics in the index).
fn bitmap_tables() -> (Vec<u8>, Vec<u8>) {
    let mut cbdt = vec![0, 3, 0, 0];
    // strike 20, glyph 1: small metrics (h 16, w 12, x 1, y 14, advance 14), its PNG
    let first = cbdt.len();
    cbdt.extend([16, 12, 1, 14, 14]);
    be32(&mut cbdt, 4);
    cbdt.extend(b"pic1");
    let first_end = cbdt.len();
    // strike 40, glyphs 1–3: each its length then 4 bytes
    let second = cbdt.len();
    for name in [b"big1", b"big2", b"big3"] {
        be32(&mut cbdt, 4);
        cbdt.extend(name);
    }
    let mut cblc = vec![0, 3, 0, 0];
    be32(&mut cblc, 2);
    let strike = |cblc: &mut Vec<u8>, array: u32, ppem: u8, start: u16, end: u16| {
        be32(cblc, array);
        be32(cblc, 0);
        be32(cblc, 1);
        be32(cblc, 0);
        cblc.extend([0u8; 24]);
        be16(cblc, start);
        be16(cblc, end);
        cblc.extend([ppem, ppem, 32, 1]);
    };
    let arrays = 8 + 48 * 2;
    // the bigger strike first: the order they are chosen in is by size
    strike(&mut cblc, arrays as u32, 40, 1, 3);
    strike(&mut cblc, arrays as u32 + 8 + 20, 20, 1, 1);
    // strike 40's index: one range 1–3, format 2 (each 8 bytes), image format 19
    for v in [1u16, 3] {
        be16(&mut cblc, v);
    }
    be32(&mut cblc, 8);
    be16(&mut cblc, 2);
    be16(&mut cblc, 19);
    be32(&mut cblc, second as u32);
    be32(&mut cblc, 8);
    cblc.extend([32, 30, 2, 28, 34, 0, 0, 0]);
    // strike 20's index: glyph 1, format 1 (offsets), image format 17
    for v in [1u16, 1] {
        be16(&mut cblc, v);
    }
    be32(&mut cblc, 8);
    be16(&mut cblc, 1);
    be16(&mut cblc, 17);
    be32(&mut cblc, first as u32);
    be32(&mut cblc, 0);
    be32(&mut cblc, (first_end - first) as u32);
    (cblc, cbdt)
}

#[test]
fn bitmap_glyphs_are_found_in_the_nearest_strike() {
    let (cblc, cbdt) = bitmap_tables();
    // a font of pictures only, no outlines
    let font = font_with(vec![(*b"CBDT", cbdt), (*b"CBLC", cblc)], false);
    assert!(font.has_color());
    let p = font.picture(1, 18.0).unwrap();
    assert_eq!((p.bytes, p.origin, p.ppem, &p.kind), (&b"pic1"[..], (1.0, -2.0), 20.0, b"png "));
    let p = font.picture(1, 30.0).unwrap();
    assert_eq!((p.bytes, p.origin, p.ppem), (&b"big1"[..], (2.0, -4.0), 40.0));
    // glyph 3 only in the bigger strike, found there when a small one is asked for
    let p = font.picture(3, 10.0).unwrap();
    assert_eq!((p.bytes, p.ppem), (&b"big3"[..], 40.0));
    assert_eq!(font.picture(0, 20.0), None);
}
