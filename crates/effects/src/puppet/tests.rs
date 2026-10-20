use super::*;
use veymelo_geom::Rect;
use veymelo_time::Tick;

/// A disc of alpha `r` round `c` on a `w` × `h` grid.
fn disc(w: usize, h: usize, c: [f64; 2], r: f64) -> mesh::Grid {
    mesh::Grid { w, h, on: (0..w * h).map(|i| ((i % w) as f64 + 0.5 - c[0]).hypot((i / w) as f64 + 0.5 - c[1]) <= r).collect() }
}

/// Every triangle turns the right way, and together they cover the outline exactly.
fn sound(m: &Mesh) {
    assert!(!m.tris.is_empty());
    let mut sum = 0.0;
    for t in &m.tris {
        let a = mesh::cross(m.verts[t[0]], m.verts[t[1]], m.verts[t[2]]) / 2.0;
        assert!(a > 0.0, "a flipped or flat triangle {t:?}: {a}");
        sum += a;
    }
    let outline = mesh::area(&m.outline).abs();
    assert!((sum - outline).abs() < 1e-6 * outline.max(1.0), "triangles {sum}, outline {outline}");
}

fn covers(m: &Mesh, p: [f64; 2]) -> bool {
    m.tris.iter().any(|t| (0..3).all(|e| mesh::cross(m.verts[t[e]], m.verts[t[(e + 1) % 3]], p) >= -1e-9))
}

#[test]
fn distances_are_exact() {
    let (w, h) = (23, 17);
    let on: Vec<bool> = (0..w * h).map(|i| matches!(i % 37, 0 | 5) || i == 200).collect();
    let d = mesh::distance_sq(w, h, &on);
    for y in 0..h {
        for x in 0..w {
            let best = (0..w * h).filter(|i| on[*i]).map(|i| ((i % w) as f64 - x as f64).powi(2) + ((i / w) as f64 - y as f64).powi(2)).fold(f64::MAX, f64::min);
            assert_eq!(d[y * w + x], best, "({x}, {y})");
        }
    }
}

#[test]
fn outlines_go_round_cells() {
    // an L: the corner cells touching only at a corner stay apart
    let g = mesh::Grid { w: 4, h: 4, on: [1, 1, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 1].iter().map(|v| *v == 1).collect() };
    let part = mesh::region(&g, [0.5, 0.5]).unwrap();
    assert!(!part.at(3, 3), "the cell touching at a corner is another part");
    let p = mesh::trace(&part);
    assert_eq!(p, [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [3.0, 2.0], [3.0, 3.0], [0.0, 3.0]]);
    assert_eq!(mesh::area(&p), 6.0);
    // holes are filled
    let ring = mesh::Grid { w: 5, h: 5, on: (0..25).map(|i| { let (x, y) = (i % 5, i / 5); (1..4).contains(&x) && (1..4).contains(&y) && i != 12 }).collect() };
    assert!(mesh::region(&ring, [1.5, 1.5]).unwrap().at(2, 2));
}

#[test]
fn a_mesh_covers_its_part() {
    let g = disc(64, 64, [32.0, 32.0], 20.0);
    let part = mesh::region(&g, [32.0, 32.0]).unwrap();
    let outline = mesh::simplify(&mesh::trace(&part), 0.75);
    let m = mesh::triangulate(&outline, &[], &Options::default());
    sound(&m);
    assert!(m.tris.len() > 150 && m.tris.len() < 800, "{} triangles", m.tris.len());
    for y in 0..64 {
        for x in 0..64 {
            if g.on[y * 64 + x] && ((x as f64 + 0.5 - 32.0).hypot(y as f64 + 0.5 - 32.0) < 19.0) {
                assert!(covers(&m, [x as f64 + 0.5, y as f64 + 0.5]), "({x}, {y}) is not covered");
            }
        }
    }
    let fine = mesh::triangulate(&outline, &[], &Options { density: 75.0, ..Options::default() });
    assert!(fine.tris.len() > m.tris.len() * 3 / 2, "{} vs {}", fine.tris.len(), m.tris.len());
    // a pin's point is a vertex
    let pinned = mesh::triangulate(&outline, &[[30.3, 27.7]], &Options::default());
    sound(&pinned);
    assert!(pinned.verts.contains(&[30.3, 27.7]));
}

#[test]
fn a_hollow_side_keeps_its_edges() {
    // a U: the gap between its arms is not covered
    let g = mesh::Grid { w: 60, h: 60, on: (0..3600).map(|i| { let (x, y) = (i % 60, i / 60); (10..50).contains(&x) && (10..50).contains(&y) && !((25..35).contains(&x) && y < 40) }).collect() };
    let part = mesh::region(&g, [12.0, 45.0]).unwrap();
    let m = mesh::triangulate(&mesh::simplify(&mesh::trace(&part), 0.75), &[], &Options::default());
    sound(&m);
    assert!(!covers(&m, [30.0, 20.0]));
    for p in [[15.0, 15.0], [45.0, 15.0], [30.0, 45.0]] {
        assert!(covers(&m, p), "{p:?}");
    }
}

#[test]
fn expansion_grows_and_shrinks() {
    let g = disc(80, 80, [40.0, 40.0], 20.0);
    let area = |e: f64| mesh::area(&mesh::trace(&mesh::region(&mesh::grow(&g, e), [40.0, 40.0]).unwrap()));
    let pi = std::f64::consts::PI;
    assert!((area(0.0) - pi * 400.0).abs() < 60.0, "{}", area(0.0));
    assert!((area(5.0) - pi * 625.0).abs() < 90.0, "{}", area(5.0));
    assert!((area(-5.0) - pi * 225.0).abs() < 60.0, "{}", area(-5.0));
}

/// A rectangle `w` × `h` from the origin, meshed.
fn block(w: f64, h: f64, triangles: f64) -> Mesh {
    mesh::triangulate(&[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]], &[], &Options { triangles, ..Options::default() })
}

fn pin(kind: PinKind, rest: [f64; 2], position: [f64; 2]) -> Pin {
    Pin { kind, rest, position, scale: 1.0, rotation: 0.0, amount: 50.0, extent: 15.0, in_front: 50.0 }
}

fn near(a: [f64; 2], b: [f64; 2], tol: f64) -> bool {
    (a[0] - b[0]).abs() < tol && (a[1] - b[1]).abs() < tol
}

#[test]
fn pins_where_they_were_change_nothing() {
    let m = block(100.0, 40.0, 200.0);
    let moved = bent(&m, &[pin(PinKind::Position, [0.0, 0.0], [0.0, 0.0]), pin(PinKind::Position, [100.0, 40.0], [100.0, 40.0])], 2);
    for (a, b) in moved.iter().zip(&m.verts) {
        assert!(near(*a, *b, 1e-7), "{a:?} vs {b:?}");
    }
}

#[test]
fn one_pin_moves_it_all() {
    let m = block(100.0, 40.0, 200.0);
    let moved = bent(&m, &[pin(PinKind::Position, [50.0, 20.0], [80.0, -10.0])], 2);
    for (a, b) in moved.iter().zip(&m.verts) {
        assert!(near(*a, [b[0] + 30.0, b[1] - 30.0], 1e-7), "{a:?} vs {b:?}");
    }
}

#[test]
fn two_pins_turn_it_without_bending() {
    let m = block(100.0, 20.0, 150.0);
    // both ends' corners turned a quarter about the first
    let pins = [pin(PinKind::Position, [0.0, 0.0], [0.0, 0.0]), pin(PinKind::Position, [100.0, 0.0], [0.0, 100.0])];
    let moved = bent(&m, &pins, 4);
    for (p, r) in moved.iter().zip(&m.verts) {
        let want = [-r[1], r[0]];
        assert!(near(*p, want, 1e-3), "{p:?} vs {want:?}");
    }
}

/// How unevenly the triangles near `at` were stretched.
fn strain(m: &Mesh, moved: &[[f64; 2]], at: [f64; 2], radius: f64) -> f64 {
    let mut sum = 0.0;
    for t in &m.tris {
        let c = middle(m, t);
        if (c[0] - at[0]).hypot(c[1] - at[1]) > radius {
            continue;
        }
        for e in 0..3 {
            let (a, b) = (t[e], t[(e + 1) % 3]);
            let l0 = (m.verts[a][0] - m.verts[b][0]).hypot(m.verts[a][1] - m.verts[b][1]);
            let l1 = (moved[a][0] - moved[b][0]).hypot(moved[a][1] - moved[b][1]);
            sum += (l1 / l0 - 1.0).abs();
        }
    }
    sum
}

#[test]
fn starch_stiffens() {
    let m = block(120.0, 30.0, 300.0);
    let pins = vec![pin(PinKind::Position, [0.0, 15.0], [0.0, 15.0]), pin(PinKind::Position, [120.0, 15.0], [80.0, 60.0])];
    let plain = strain(&m, &bent(&m, &pins, 2), [60.0, 15.0], 15.0);
    let mut starched = pins.clone();
    starched.push(Pin { amount: 100.0, extent: 20.0, ..pin(PinKind::Starch, [60.0, 15.0], [60.0, 15.0]) });
    let stiff = strain(&m, &bent(&m, &starched, 2), [60.0, 15.0], 15.0);
    assert!(stiff < plain * 0.5, "starched {stiff}, plain {plain}");
}

#[test]
fn bend_pins_turn_about_themselves() {
    let m = mesh::triangulate(&[[0.0, 0.0], [20.0, 0.0], [20.0, 100.0], [0.0, 100.0]], &[[10.0, 50.0]], &Options { triangles: 200.0, ..Options::default() });
    let pins = [pin(PinKind::Position, [10.0, 0.0], [10.0, 0.0]), Pin { rotation: 30.0, ..pin(PinKind::Bend, [10.0, 50.0], [10.0, 50.0]) }];
    let moved = bent(&m, &pins, 2);
    let bottom = nearest(&m.verts, [10.0, 100.0]).unwrap();
    // turned clockwise on the screen about the middle: the bottom swings left
    assert!(moved[bottom][0] < 10.0 - 5.0, "{:?}", moved[bottom]);
}

#[test]
fn overlap_pins_bring_parts_forward() {
    let m = block(100.0, 100.0, 200.0);
    let d = depths(&m, &[Pin { in_front: 80.0, extent: 20.0, ..pin(PinKind::Overlap, [20.0, 20.0], [20.0, 20.0]) }]);
    assert!(d.contains(&80.0) && d.contains(&0.0));
}

/// A puppet on a 40 × 20 red block from (10, 10) in a 60 × 40 picture, with `pins`.
fn puppet(pins: &[(&'static GroupDef, [f64; 2], [f64; 2])]) -> Group {
    let mut fx = Group::new(&PUPPET);
    let mut mesh = Group::instance(&MESH, 2, "Mesh 1");
    mesh.prop_mut("seed").unwrap().set_value(Value::Vec2([30.0, 20.0])).unwrap();
    let list = mesh.group_mut("pins").unwrap();
    for (k, (def, rest, at)) in pins.iter().enumerate() {
        let mut p = Group::instance(def, 10 + k as u64, &format!("Puppet Pin {}", k + 1));
        p.prop_mut("rest").unwrap().set_value(Value::Vec2(*rest)).unwrap();
        if let Ok(q) = p.prop_mut("position") {
            q.set_value(Value::Vec2(*at)).unwrap();
        }
        list.children.push(Node::Group(p));
    }
    fx.group_mut("meshes").unwrap().children.push(Node::Group(mesh));
    fx
}

fn picture() -> Buf {
    let mut img = Image::new(60, 40);
    for y in 10..30 {
        for x in 10..50 {
            img.px[y * 60 + x] = [1.0, 0.0, 0.0, 1.0];
        }
    }
    Buf { img, origin: vec2(0.0, 0.0), scale: 1.0 }
}

fn apply(fx: &Group, buf: Buf) -> Buf {
    let read = |p: &veymelo_project::Property| p.value_at(Tick::ZERO);
    run(&Ctx::new(fx, &read, Tick::ZERO, Rect::new(0.0, 0.0, 60.0, 40.0), 1), buf)
}

/// The colour at a point of the layer.
fn at(b: &Buf, x: f64, y: f64) -> [f32; 4] {
    let p = b.pixel(vec2(x, y));
    b.img.get(p.x.floor() as i64, p.y.floor() as i64)
}

#[test]
fn drawn_through_the_mesh() {
    let still = apply(&puppet(&[(&PIN, [15.0, 20.0], [15.0, 20.0])]), picture());
    for (x, y) in [(12.5, 12.5), (30.5, 20.5), (47.5, 27.5)] {
        assert_eq!(at(&still, x, y), [1.0, 0.0, 0.0, 1.0], "({x}, {y}) inside stays as it was");
    }
    assert_eq!(at(&still, 5.5, 5.5)[3], 0.0);
    // one pin moved: all of it moves, as it was
    let moved = apply(&puppet(&[(&PIN, [15.0, 20.0], [25.0, 27.0])]), picture());
    for (x, y) in [(22.5, 19.5), (40.5, 27.5), (57.5, 34.5)] {
        let c = at(&moved, x, y);
        assert!((c[0] - 1.0).abs() < 1e-5 && (c[3] - 1.0).abs() < 1e-5, "({x}, {y}): {c:?}");
    }
    assert_eq!(at(&moved, 15.5, 15.5)[3], 0.0, "where it was is empty");
    // the same every time, soft at its edges
    let pins = [(&PIN, [15.0, 20.0], [15.0, 12.0]), (&PIN, [45.0, 20.0], [50.0, 35.0])];
    let (a, b) = (apply(&puppet(&pins), picture()), apply(&puppet(&pins), picture()));
    assert_eq!((a.img.px.clone(), a.origin), (b.img.px.clone(), b.origin));
    assert!(a.img.px.iter().any(|p| p[3] > 0.05 && p[3] < 0.95), "soft edges");
    assert!(a.img.px.iter().all(|p| p[3] <= 1.0001 && p.iter().all(|c| c.is_finite())));
}

#[test]
fn drawn_small_it_bends_the_same() {
    let pins = [(&PIN, [15.0, 20.0], [15.0, 12.0]), (&PIN, [45.0, 20.0], [50.0, 35.0])];
    let full = apply(&puppet(&pins), picture());
    // the same layer drawn at half the pixels to a unit
    let mut img = Image::new(30, 20);
    for y in 5..15 {
        for x in 5..25 {
            img.px[y * 30 + x] = [1.0, 0.0, 0.0, 1.0];
        }
    }
    let half = apply(&puppet(&pins), Buf { img, origin: vec2(0.0, 0.0), scale: 0.5 });
    for (x, y) in [(20.0, 15.0), (30.0, 20.0), (44.0, 30.0)] {
        assert_eq!(at(&full, x, y)[3] > 0.5, at(&half, x, y)[3] > 0.5, "({x}, {y})");
    }
}
