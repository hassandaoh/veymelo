use veymelo_keyframe::ShapePath;

use crate::bezier::{self, length, piece, pieces, reversed, starting_at, with_vertices};
use crate::morph::{self, Morph};
use crate::trace::{Trace, trace};

/// A circle of radius `r` about (cx, cy): four vertices, the usual handle length.
fn circle(cx: f64, cy: f64, r: f64) -> ShapePath {
    let k = 0.552_284_75 * r;
    ShapePath {
        points: vec![[cx, cy - r], [cx + r, cy], [cx, cy + r], [cx - r, cy]],
        in_tangents: vec![[-k, 0.0], [0.0, -k], [k, 0.0], [0.0, k]],
        out_tangents: vec![[k, 0.0], [0.0, k], [-k, 0.0], [0.0, -k]],
        closed: true,
        feather: veymelo_keyframe::Feathers::NONE,
    }
}

fn square(c: [f64; 2], half: f64, turn: f64) -> ShapePath {
    let pts: Vec<[f64; 2]> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
        .iter()
        .map(|p| {
            let (s, co) = turn.sin_cos();
            [c[0] + (p[0] * co - p[1] * s) * half, c[1] + (p[0] * s + p[1] * co) * half]
        })
        .collect();
    ShapePath::polygon(&pts, true)
}

/// Points along a path, every so far.
fn samples(p: &ShapePath, n: usize) -> Vec<[f64; 2]> {
    (0..pieces(p)).flat_map(|i| (0..n).map(move |k| (i, k as f64 / n as f64))).map(|(i, t)| bezier::at(&piece(p, i), t)).collect()
}

#[test]
fn vertices_added_keep_the_outline() {
    let c = circle(50.0, 50.0, 20.0);
    let more = with_vertices(&c, 13);
    assert_eq!(more.points.len(), 13);
    assert!((length(&more) - length(&c)).abs() < 0.05, "{} {}", length(&more), length(&c));
    for q in samples(&more, 8) {
        let r = ((q[0] - 50.0).powi(2) + (q[1] - 50.0).powi(2)).sqrt();
        assert!((r - 20.0).abs() < 0.06, "on the circle: {r}");
    }
    // spread by length: the pieces about as long as each other
    let lens: Vec<f64> = (0..13).map(|i| bezier::piece_length(&piece(&more, i))).collect();
    let (lo, hi) = lens.iter().fold((f64::MAX, 0.0f64), |(a, b), l| (a.min(*l), b.max(*l)));
    assert!(hi - lo < 0.5 * lo, "{lens:?}");
    // started elsewhere, and turned round: the same outline
    let r = reversed(&starting_at(&c, 2));
    assert_eq!(r.points[0], c.points[2]);
    assert!((length(&r) - length(&c)).abs() < 1e-9);
}

#[test]
fn shapes_matched_and_moved_between() {
    // the same square started at another corner and drawn the other way: matched corner to corner
    let a = square([0.0, 0.0], 10.0, 0.0);
    let b = reversed(&starting_at(&a, 2));
    let m = Morph { spacing: None, ..Morph::default() };
    let (aa, bb) = morph::correspond(&a, &b, &m);
    for (p, q) in aa.points.iter().zip(&bb.points) {
        assert!(bezier::dist(*p, *q) < 1e-9, "{p:?} {q:?}");
    }
    // a square turned a quarter: half way, whole, it stays its size; straight lines shrink it
    let turned = square([0.0, 0.0], 10.0, std::f64::consts::FRAC_PI_2 * 0.999);
    let (aa, bb) = morph::correspond(&a, &turned, &Morph { spacing: None, first_vertices_match: true, ..Morph::default() });
    let rigid = morph::between(&aa, &bb, 0.5, &Morph { bending_resistance: 100.0, ..m });
    let straight = morph::between(&aa, &bb, 0.5, &Morph { linear: true, ..m });
    let area = |p: &ShapePath| bezier::signed_area(&p.points).abs();
    assert!((area(&rigid) - 400.0).abs() < 2.0, "{}", area(&rigid));
    assert!(area(&straight) < 300.0, "{}", area(&straight));
    // the ends are the shapes themselves
    assert_eq!(morph::between(&aa, &bb, 0.0, &m).points, aa.points);
    let end = morph::between(&aa, &bb, 1.0, &m);
    assert!(end.points.iter().zip(&bb.points).all(|(p, q)| bezier::dist(*p, *q) < 1e-9));
    // different counts: as many as the larger has, or by spacing
    let (x, y) = morph::correspond(&a, &circle(0.0, 0.0, 12.0), &Morph { spacing: Some(5.0), ..Morph::default() });
    assert_eq!(x.points.len(), y.points.len());
    assert!(x.points.len() >= 15);
}

#[test]
fn a_disc_and_a_square_with_a_hole_traced() {
    let (w, h) = (120, 80);
    let mut field = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            let disc = (fx - 30.0).powi(2) + (fy - 40.0).powi(2) <= 400.0;
            let square = (60.0..110.0).contains(&fx) && (15.0..65.0).contains(&fy);
            let hole = (75.0..95.0).contains(&fx) && (30.0..50.0).contains(&fy);
            field[y * w + x] = if disc || (square && !hole) { 1.0 } else { 0.0 };
        }
    }
    let t = trace(&field, w, h, &Trace::default());
    assert_eq!(t.len(), 3, "disc, square, hole");
    assert_eq!(t.iter().filter(|o| o.depth == 1).count(), 1, "one hole");
    let areas: Vec<f64> = t.iter().map(|o| o.area.abs()).collect();
    assert!((areas[0] - 2500.0).abs() < 60.0 && (areas[1] - std::f64::consts::PI * 400.0).abs() < 40.0 && (areas[2] - 400.0).abs() < 30.0, "{areas:?}");
    // sharp: the square four corners (or near it), the disc a handful of smooth vertices
    let sharp = trace(&field, w, h, &Trace { corner_roundness: 0.0, ..Trace::default() });
    assert!(sharp[0].points.len() <= 8, "{:?}", sharp[0].points);
    let smooth = trace(&field, w, h, &Trace { corner_roundness: 1.0, tolerance: 1.5, ..Trace::default() });
    let disc = &smooth[1];
    assert!(disc.points.len() < 40);
    let as_path = ShapePath { points: disc.points.clone(), in_tangents: disc.in_tangents.clone(), out_tangents: disc.out_tangents.clone(), closed: true, feather: veymelo_keyframe::Feathers::NONE };
    for q in samples(&as_path, 6) {
        let r = ((q[0] - 30.0).powi(2) + (q[1] - 40.0).powi(2)).sqrt();
        assert!((r - 20.0).abs() < 1.6, "on the disc's edge: {r}");
    }
    // too small, and above the threshold
    let mut speck = vec![0.0f32; 400];
    speck[21] = 1.0;
    assert!(trace(&speck, 20, 20, &Trace::default()).is_empty());
    assert_eq!(trace(&vec![1.0f32; 100], 10, 10, &Trace::default()).len(), 1, "touching the edges, it closes along them");
}
