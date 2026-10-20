//! A flat shape with holes cut into triangles (the faces of a solid made from it): each hole
//! joined to the outline by a cut to a vertex it can see (so the outline and its holes become
//! one ring), then ears clipped off one by one — a corner whose triangle holds no other vertex.

/// Twice the signed area of a ring (above zero: anticlockwise with y up, clockwise on screen).
pub fn area(ring: &[[f64; 2]]) -> f64 {
    (0..ring.len()).map(|i| {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        a[0] * b[1] - b[0] * a[1]
    })
    .sum()
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Whether segments a–b and c–d cross (touching at an end does not count).
fn crosses(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    ((d1 > 1e-12 && d2 < -1e-12) || (d1 < -1e-12 && d2 > 1e-12)) && ((d3 > 1e-12 && d4 < -1e-12) || (d3 < -1e-12 && d4 > 1e-12))
}

/// Whether `p` is inside the anticlockwise triangle a, b, c and not on its edges.
fn inside_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let e = 1e-9 * (cross(a, b, c).abs() + 1.0);
    cross(a, b, p) > e && cross(b, c, p) > e && cross(c, a, p) > e
}

/// Whether `p` lies on segment a–c, between its ends.
fn on_segment(p: [f64; 2], a: [f64; 2], c: [f64; 2]) -> bool {
    let len2 = (c[0] - a[0]).powi(2) + (c[1] - a[1]).powi(2);
    if len2 < 1e-24 || cross(a, c, p).abs() > 1e-9 * len2.sqrt() {
        return false;
    }
    let t = ((p[0] - a[0]) * (c[0] - a[0]) + (p[1] - a[1]) * (c[1] - a[1])) / len2;
    t > 1e-9 && t < 1.0 - 1e-9
}

/// The triangles of an outline (`rings[0]`) with holes (the rest), as corners into all the
/// rings' points one after another; each triangle anticlockwise as `area` counts.
pub fn triangulate(rings: &[Vec<[f64; 2]>]) -> Vec<[u32; 3]> {
    if rings.first().is_none_or(|r| r.len() < 3) {
        return Vec::new();
    }
    let pts: Vec<[f64; 2]> = rings.iter().flatten().copied().collect();
    let mut start = 0;
    let mut loops: Vec<Vec<u32>> = Vec::new();
    for (k, r) in rings.iter().enumerate() {
        let mut ids: Vec<u32> = (start..start + r.len() as u32).collect();
        start += r.len() as u32;
        if r.len() < 3 {
            continue;
        }
        // the outline anticlockwise, the holes clockwise
        if (area(r) > 0.0) != (k == 0) {
            ids.reverse();
        }
        loops.push(ids);
    }
    let mut ring = loops.remove(0);
    // holes from the farthest right in, each joined to the ring at a vertex it can see
    loops.sort_by(|a, b| {
        let most = |l: &Vec<u32>| l.iter().map(|&i| pts[i as usize][0]).fold(f64::NEG_INFINITY, f64::max);
        most(b).total_cmp(&most(a))
    });
    for hole in loops {
        let (hi, &m) = hole.iter().enumerate().max_by(|a, b| pts[*a.1 as usize][0].total_cmp(&pts[*b.1 as usize][0])).expect("a hole has points");
        let mp = pts[m as usize];
        // the nearest vertex of the ring the cut to which crosses no edge (of the ring or hole)
        let edges: Vec<([f64; 2], [f64; 2])> = ring.iter().enumerate().map(|(i, &a)| (pts[a as usize], pts[ring[(i + 1) % ring.len()] as usize])).chain(hole.iter().enumerate().map(|(i, &a)| (pts[a as usize], pts[hole[(i + 1) % hole.len()] as usize]))).collect();
        let mut best: Option<(usize, f64)> = None;
        for (i, &v) in ring.iter().enumerate() {
            let vp = pts[v as usize];
            let d = (vp[0] - mp[0]).powi(2) + (vp[1] - mp[1]).powi(2);
            if best.is_some_and(|(_, bd)| d >= bd) {
                continue;
            }
            if edges.iter().any(|(a, b)| crosses(mp, vp, *a, *b)) {
                continue;
            }
            best = Some((i, d));
        }
        let Some((at, _)) = best else { continue };
        // the ring to that vertex, round the hole from its vertex and back, then on
        let mut joined: Vec<u32> = ring[..=at].to_vec();
        joined.extend(hole[hi..].iter().chain(hole[..=hi].iter()));
        joined.extend(ring[at..].iter());
        ring = joined;
    }
    // clip ears
    let mut out = Vec::new();
    // each turn clips an ear or drops a corner, so it ends in as many turns as there are corners
    let mut turns = ring.len() + 1;
    while ring.len() > 3 && turns > 0 {
        turns -= 1;
        let n = ring.len();
        let mut clipped = false;
        for i in 0..n {
            let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
            let (pa, pb, pc) = (pts[a as usize], pts[b as usize], pts[c as usize]);
            let turn = cross(pa, pb, pc);
            if turn.abs() <= 1e-12 {
                // no corner at all (on a straight line, or doubled back on itself): dropped, the
                // shape's area the same
                ring.remove(i);
                clipped = true;
                break;
            }
            if turn < 0.0 {
                continue;
            }
            // an ear holds no other corner that turns back in (those that turn out cannot be inside
            // without one that turns in)
            let blocked = (0..n).any(|k| {
                let o = ring[k];
                let po = pts[o as usize];
                if o == a || o == b || o == c || po == pa || po == pb || po == pc {
                    return false;
                }
                let reflex = cross(pts[ring[(k + n - 1) % n] as usize], po, pts[ring[(k + 1) % n] as usize]) <= 0.0;
                reflex && (inside_triangle(po, pa, pb, pc) || on_segment(po, pa, pc))
            });
            if !blocked {
                out.push([a, b, c]);
                ring.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            // nothing clips cleanly (a ring touching itself): take the most convex corner
            let i = (0..n).max_by(|&x, &y| {
                let t = |i: usize| cross(pts[ring[(i + n - 1) % n] as usize], pts[ring[i] as usize], pts[ring[(i + 1) % n] as usize]);
                t(x).total_cmp(&t(y))
            });
            let i = i.unwrap_or(0);
            out.push([ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]]);
            ring.remove(i);
        }
    }
    if ring.len() == 3 && cross(pts[ring[0] as usize], pts[ring[1] as usize], pts[ring[2] as usize]) > 1e-12 {
        out.push([ring[0], ring[1], ring[2]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(rings: &[Vec<[f64; 2]>], tris: &[[u32; 3]]) -> f64 {
        let pts: Vec<[f64; 2]> = rings.iter().flatten().copied().collect();
        tris.iter().map(|t| area(&[pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]]) / 2.0).sum()
    }

    #[test]
    fn shapes_with_and_without_holes() {
        let square = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let t = triangulate(std::slice::from_ref(&square));
        assert_eq!(t.len(), 2);
        assert!((total(std::slice::from_ref(&square), &t) - 100.0).abs() < 1e-9);
        // a ring: a square with a square hole, given either way round
        let hole = vec![[3.0, 3.0], [3.0, 7.0], [7.0, 7.0], [7.0, 3.0]];
        for h in [hole.clone(), hole.iter().rev().copied().collect()] {
            let rings = vec![square.clone(), h];
            let t = triangulate(&rings);
            assert!((total(&rings, &t) - 84.0).abs() < 1e-9, "{t:?}");
            assert!(t.iter().all(|tr| { let pts: Vec<[f64; 2]> = rings.iter().flatten().copied().collect(); area(&[pts[tr[0] as usize], pts[tr[1] as usize], pts[tr[2] as usize]]) > 0.0 }));
        }
        // a C traced pixel by pixel: many corners on its straight edges
        let mut c: Vec<[f64; 2]> = Vec::new();
        for x in 0..=20 {
            c.push([x as f64, 0.0]);
        }
        for y in 1..=4 {
            c.push([20.0, y as f64]);
        }
        for x in (5..20).rev() {
            c.push([x as f64, 4.0]);
        }
        for y in 5..=16 {
            c.push([5.0, y as f64]);
        }
        for x in 6..=20 {
            c.push([x as f64, 16.0]);
        }
        for y in 17..=20 {
            c.push([20.0, y as f64]);
        }
        for x in (0..20).rev() {
            c.push([x as f64, 20.0]);
        }
        for y in (1..20).rev() {
            c.push([0.0, y as f64]);
        }
        let want = 20.0 * 20.0 - 15.0 * 12.0;
        let t = triangulate(std::slice::from_ref(&c));
        let pts = c.clone();
        let abs: f64 = t.iter().map(|tr| (area(&[pts[tr[0] as usize], pts[tr[1] as usize], pts[tr[2] as usize]]) / 2.0).abs()).sum();
        assert!((abs - want).abs() < 1e-9, "every triangle inside, none over another: {abs} of {want}");
        // an L, given clockwise, and two holes
        let l = vec![[0.0, 0.0], [0.0, 10.0], [4.0, 10.0], [4.0, 4.0], [10.0, 4.0], [10.0, 0.0]];
        assert!((total(std::slice::from_ref(&l), &triangulate(std::slice::from_ref(&l))) - 64.0).abs() < 1e-9);
        let two = vec![square.clone(), vec![[1.0, 1.0], [1.0, 3.0], [3.0, 3.0], [3.0, 1.0]], vec![[6.0, 6.0], [6.0, 9.0], [9.0, 9.0], [9.0, 6.0]]];
        assert!((total(&two, &triangulate(&two)) - (100.0 - 4.0 - 9.0)).abs() < 1e-9);
    }
}
