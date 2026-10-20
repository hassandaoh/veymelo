//! A puppet's mesh: the outline of the part of the layer under its seed point, filled with
//! triangles of about the same size.
//!
//! 1. The layer's alpha, on a grid of layer units, is seen as in (half or more) or out, grown or
//!    shrunk by the expansion with an exact distance transform (Felzenszwalb and Huttenlocher's
//!    lower envelope of parabolas).
//! 2. The part connected to the seed (sides touching, not corners), its holes filled, is walked
//!    round along the cell edges into a polygon, made simpler (Douglas–Peucker), and its sides
//!    cut so none is longer than the triangles should be.
//! 3. Points on a slightly shaken hexagonal lattice fill the inside, and the pins' own points
//!    are put in so they sit exactly on a vertex. All are joined by Delaunay triangles
//!    (Bowyer–Watson); sides of the outline the triangles cut across are brought back by
//!    flipping edges; triangles outside the outline are dropped.

use std::collections::{BTreeMap, HashMap, VecDeque};

/// Triangles over a part of a layer, in layer units. Each triangle turns the same way:
/// `cross(b - a, c - a) > 0`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub verts: Vec<[f64; 2]>,
    pub tris: Vec<[usize; 3]>,
    /// The outline, in layer units.
    pub outline: Vec<[f64; 2]>,
}

/// How the mesh is made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// About how many triangles.
    pub triangles: f64,
    /// 0 to 100: 50 makes `triangles`, each 25 more doubles them, each 25 less halves them.
    pub density: f64,
    /// How far the outline is grown past the layer's pixels (below zero: shrunk), in layer units.
    pub expansion: f64,
}

impl Default for Options {
    fn default() -> Options {
        Options { triangles: 350.0, density: 50.0, expansion: 3.0 }
    }
}

/// Cells that are in or out.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub w: usize,
    pub h: usize,
    pub on: Vec<bool>,
}

impl Grid {
    pub fn at(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h && self.on[y as usize * self.w + x as usize]
    }
}

/// Which way `o → a → b` turns (above zero: the same way as the mesh's triangles).
pub fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn dist2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// The squared distance from each of `f`'s places to the nearest, each place starting at
/// `f[q]` (0 where something is, very far where nothing is).
fn envelope(f: &[f64], out: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    // the parabolas that make the lower envelope, and where each takes over
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];
    let (mut k, mut i) = (0usize, 1usize);
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    let meet = |q: usize, p: usize| ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
    while i < n {
        let mut s = meet(i, v[k]);
        while s <= z[k] {
            k -= 1;
            s = meet(i, v[k]);
        }
        k += 1;
        v[k] = i;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
        i += 1;
    }
    let mut k = 0;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        *o = (q as f64 - v[k] as f64).powi(2) + f[v[k]];
    }
}

/// The squared distance from every cell to the nearest cell that is on.
pub fn distance_sq(w: usize, h: usize, on: &[bool]) -> Vec<f64> {
    const FAR: f64 = 1e20;
    let mut cols = vec![FAR; w * h];
    let (mut col, mut got) = (vec![0.0; h], vec![0.0; h]);
    for x in 0..w {
        for y in 0..h {
            col[y] = if on[y * w + x] { 0.0 } else { FAR };
        }
        envelope(&col, &mut got);
        for y in 0..h {
            cols[y * w + x] = got[y];
        }
    }
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        envelope(&cols[y * w..(y + 1) * w], &mut out[y * w..(y + 1) * w]);
    }
    out
}

/// The grid grown by `e` cells (below zero: shrunk).
pub fn grow(g: &Grid, e: f64) -> Grid {
    if e.abs() < 1e-9 {
        return g.clone();
    }
    if e > 0.0 {
        let d = distance_sq(g.w, g.h, &g.on);
        Grid { w: g.w, h: g.h, on: d.iter().map(|v| *v <= e * e).collect() }
    } else {
        let off: Vec<bool> = g.on.iter().map(|b| !b).collect();
        let d = distance_sq(g.w, g.h, &off);
        Grid { w: g.w, h: g.h, on: d.iter().zip(&g.on).map(|(v, on)| *on && *v > e * e).collect() }
    }
}

/// Fill from `start` through cells for which `open` holds, sides touching.
fn flood(w: usize, h: usize, starts: &[(usize, usize)], open: impl Fn(usize) -> bool) -> Vec<bool> {
    let mut seen = vec![false; w * h];
    let mut queue = VecDeque::new();
    for &(x, y) in starts {
        if open(y * w + x) && !seen[y * w + x] {
            seen[y * w + x] = true;
            queue.push_back((x, y));
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        let mut go = |nx: usize, ny: usize| {
            let i = ny * w + nx;
            if !seen[i] && open(i) {
                seen[i] = true;
                queue.push_back((nx, ny));
            }
        };
        if x > 0 {
            go(x - 1, y);
        }
        if x + 1 < w {
            go(x + 1, y);
        }
        if y > 0 {
            go(x, y - 1);
        }
        if y + 1 < h {
            go(x, y + 1);
        }
    }
    seen
}

/// The part of the grid connected to `seed` (or, off it, to the nearest cell on), its holes
/// filled.
pub fn region(g: &Grid, seed: [f64; 2]) -> Option<Grid> {
    let (w, h) = (g.w, g.h);
    if w == 0 || h == 0 {
        return None;
    }
    let (sx, sy) = ((seed[0].floor() as i64).clamp(0, w as i64 - 1), (seed[1].floor() as i64).clamp(0, h as i64 - 1));
    let start = if g.at(sx, sy) {
        (sx as usize, sy as usize)
    } else {
        let mut best: Option<(i64, usize, usize)> = None;
        for y in 0..h {
            for x in 0..w {
                if g.on[y * w + x] {
                    let d = (x as i64 - sx).pow(2) + (y as i64 - sy).pow(2);
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, x, y));
                    }
                }
            }
        }
        let (_, x, y) = best?;
        (x, y)
    };
    let part = flood(w, h, &[start], |i| g.on[i]);
    // what can be reached from the edge without crossing the part is outside; the rest is in
    let edge: Vec<(usize, usize)> = (0..w).flat_map(|x| [(x, 0), (x, h - 1)]).chain((0..h).flat_map(|y| [(0, y), (w - 1, y)])).collect();
    let outside = flood(w, h, &edge, |i| !part[i]);
    Some(Grid { w, h, on: outside.iter().map(|o| !o).collect() })
}

/// The outline of a part with no holes, along the edges of its cells: corner points, the part
/// on the right of the way round (in y-down units).
pub fn trace(g: &Grid) -> Vec<[f64; 2]> {
    // the top row's leftmost cell: its top edge is on the outline, going right
    let Some(first) = g.on.iter().position(|o| *o) else { return Vec::new() };
    let start = ((first % g.w) as i64, (first / g.w) as i64);
    let (mut c, mut d) = (start, (1i64, 0i64));
    let mut pts: Vec<[f64; 2]> = Vec::new();
    // the start is touched by its cell alone, so the way round passes it once
    for _ in 0..4 * (g.w + 1) * (g.h + 1) {
        // the two cells ahead of corner `c`, left and right of the way `d`
        let r = (-d.1, d.0);
        let cell = |side: i64| {
            let (x2, y2) = (2 * c.0 + d.0 + side * r.0, 2 * c.1 + d.1 + side * r.1);
            g.at(x2.div_euclid(2), y2.div_euclid(2))
        };
        let turn = if !cell(1) {
            // round the corner, keeping cells that only touch at a corner apart
            r
        } else if cell(-1) {
            (d.1, -d.0)
        } else {
            d
        };
        if turn != d || pts.is_empty() {
            pts.push([c.0 as f64, c.1 as f64]);
        }
        d = turn;
        c = (c.0 + d.0, c.1 + d.1);
        if c == start {
            break;
        }
    }
    pts
}

/// The area a polygon encloses, signed by the way it turns.
pub fn area(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum::<f64>() * 0.5
}

fn seg_dist2(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let l = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l > 0.0 { (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l).clamp(0.0, 1.0) } else { 0.0 };
    dist2(p, [a[0] + ab[0] * t, a[1] + ab[1] * t])
}

/// A closed polygon with the points it can do without dropped: none of what is left strays
/// more than `tol` from it (Douglas–Peucker, the loop cut at its first point and the point
/// farthest from that).
pub fn simplify(p: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    let n = p.len();
    if n <= 4 {
        return p.to_vec();
    }
    let far = (1..n).max_by(|a, b| dist2(p[0], p[*a]).total_cmp(&dist2(p[0], p[*b]))).unwrap_or(n / 2);
    let at = |i: usize| p[i % n];
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[far] = true;
    let mut stack = vec![(0usize, far), (far, n)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (mut worst, mut at_worst) = (0.0, a);
        for i in a + 1..b {
            let d = seg_dist2(at(i), at(a), at(b));
            if d > worst {
                (worst, at_worst) = (d, i);
            }
        }
        if worst > tol * tol {
            keep[at_worst] = true;
            stack.push((a, at_worst));
            stack.push((at_worst, b));
        }
    }
    (0..n).filter(|i| keep[*i]).map(|i| p[i]).collect()
}

/// The polygon with its sides cut so none is longer than `most`.
pub fn densify(p: &[[f64; 2]], most: f64) -> Vec<[f64; 2]> {
    let n = p.len();
    let mut out = Vec::new();
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        out.push(a);
        let k = (dist2(a, b).sqrt() / most.max(1e-6)).ceil() as usize;
        for j in 1..k {
            let t = j as f64 / k as f64;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    out
}

/// Whether `q` is inside the polygon (crossings counted).
pub fn inside(poly: &[[f64; 2]], q: [f64; 2]) -> bool {
    let n = poly.len();
    let mut odd = false;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a[1] > q[1]) != (b[1] > q[1]) && q[0] < (b[0] - a[0]) * (q[1] - a[1]) / (b[1] - a[1]) + a[0] {
            odd = !odd;
        }
    }
    odd
}

fn outline_dist2(poly: &[[f64; 2]], q: [f64; 2]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| seg_dist2(q, poly[i], poly[(i + 1) % n])).fold(f64::MAX, f64::min)
}

/// Whether `d` is inside the circle through triangle `a b c` (which turns the mesh's way).
fn in_circle(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let (ax, ay, bx, by, cx, cy) = (a[0] - d[0], a[1] - d[1], b[0] - d[0], b[1] - d[1], c[0] - d[0], c[1] - d[1]);
    let (la, lb, lc) = (ax * ax + ay * ay, bx * bx + by * by, cx * cx + cy * cy);
    let det = la * (bx * cy - cx * by) - lb * (ax * cy - cx * ay) + lc * (ax * by - bx * ay);
    let m = la.max(lb).max(lc);
    det > 1e-12 * m * m
}

/// The triangle turned the mesh's way.
fn turned(v: &[[f64; 2]], t: [usize; 3]) -> [usize; 3] {
    if cross(v[t[0]], v[t[1]], v[t[2]]) < 0.0 { [t[0], t[2], t[1]] } else { t }
}

/// Delaunay triangles joining `pts` (Bowyer–Watson: each point in turn takes out the triangles
/// whose circles hold it and fills the hole with triangles to itself).
pub fn delaunay(pts: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in pts {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    // a triangle far round everything to start from
    let size = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0) * 20.0;
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let mut v = pts.to_vec();
    v.extend([[mid[0] - size, mid[1] - size], [mid[0] + size, mid[1] - size], [mid[0], mid[1] + size]]);
    let mut tris = vec![turned(&v, [n, n + 1, n + 2])];
    for i in 0..n {
        let p = v[i];
        let mut bad: Vec<usize> = (0..tris.len()).filter(|&k| in_circle(v[tris[k][0]], v[tris[k][1]], v[tris[k][2]], p)).collect();
        if bad.is_empty() {
            // on a circle exactly: the triangle it is in
            match tris.iter().position(|t| (0..3).all(|e| cross(v[t[e]], v[t[(e + 1) % 3]], p) >= 0.0)) {
                Some(k) => bad.push(k),
                None => continue,
            }
        }
        // the hole's sides: sides of the bad triangles that only one of them has
        let mut count: HashMap<(usize, usize), u32> = HashMap::new();
        for &k in &bad {
            let t = tris[k];
            for e in 0..3 {
                let (a, b) = (t[e], t[(e + 1) % 3]);
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let mut sides = Vec::new();
        for &k in &bad {
            let t = tris[k];
            for e in 0..3 {
                let (a, b) = (t[e], t[(e + 1) % 3]);
                if count[&(a.min(b), a.max(b))] == 1 {
                    sides.push((a, b));
                }
            }
        }
        bad.sort_unstable_by(|a, b| b.cmp(a));
        for k in bad {
            tris.swap_remove(k);
        }
        for (a, b) in sides {
            if cross(v[a], v[b], p).abs() > 1e-12 {
                tris.push(turned(&v, [a, b, i]));
            }
        }
    }
    tris.retain(|t| t.iter().all(|i| *i < n));
    tris
}

/// Whether segments `a b` and `c d` cross (not just touch).
fn crosses(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let side = |x: f64| if x > 1e-12 { 1 } else if x < -1e-12 { -1 } else { 0 };
    let (d1, d2, d3, d4) = (side(cross(a, b, c)), side(cross(a, b, d)), side(cross(c, d, a)), side(cross(c, d, b)));
    d1 * d2 < 0 && d3 * d4 < 0
}

fn edge(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

/// Bring back `sides` the triangles cut across, by flipping the edges that cross them.
pub fn recover(v: &[[f64; 2]], tris: &mut [[usize; 3]], sides: &[(usize, usize)]) {
    let mut by_edge: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (k, t) in tris.iter().enumerate() {
        for e in 0..3 {
            by_edge.entry(edge(t[e], t[(e + 1) % 3])).or_default().push(k);
        }
    }
    let kept: std::collections::HashSet<(usize, usize)> = sides.iter().map(|&(a, b)| edge(a, b)).collect();
    for &(a, b) in sides {
        if by_edge.get(&edge(a, b)).is_some_and(|t| !t.is_empty()) {
            continue;
        }
        let mut queue: VecDeque<(usize, usize)> = by_edge.iter().filter(|(e, t)| t.len() == 2 && crosses(v[a], v[b], v[e.0], v[e.1])).map(|(e, _)| *e).collect();
        let mut tries = 0;
        while let Some(e) = queue.pop_front() {
            tries += 1;
            if tries > 20_000 {
                break;
            }
            let Some(ts) = by_edge.get(&e).cloned() else { continue };
            if ts.len() != 2 || kept.contains(&e) {
                continue;
            }
            let (t0, t1) = (tris[ts[0]], tris[ts[1]]);
            let (Some(p), Some(q)) = (t0.iter().copied().find(|x| *x != e.0 && *x != e.1), t1.iter().copied().find(|x| *x != e.0 && *x != e.1)) else { continue };
            // only a four-sided shape that bulges both ways can have its diagonal flipped
            if !crosses(v[p], v[q], v[e.0], v[e.1]) {
                if queue.is_empty() {
                    break;
                }
                queue.push_back(e);
                continue;
            }
            for (k, t) in [(ts[0], t0), (ts[1], t1)] {
                for j in 0..3 {
                    if let Some(l) = by_edge.get_mut(&edge(t[j], t[(j + 1) % 3])) {
                        l.retain(|x| *x != k);
                    }
                }
            }
            let (n0, n1) = (turned(v, [p, q, e.0]), turned(v, [p, q, e.1]));
            tris[ts[0]] = n0;
            tris[ts[1]] = n1;
            for (k, t) in [(ts[0], n0), (ts[1], n1)] {
                for j in 0..3 {
                    by_edge.entry(edge(t[j], t[(j + 1) % 3])).or_default().push(k);
                }
            }
            by_edge.retain(|_, l| !l.is_empty());
            if crosses(v[a], v[b], v[p], v[q]) {
                queue.push_back(edge(p, q));
            }
        }
    }
}

/// A number from -0.5 to 0.5 for `i`, the same every time.
fn shake(i: u64) -> f64 {
    let mut x = i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x2545_F491_4F6C_DD1D;
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64 - 0.5
}

/// Triangles over the polygon `outline` (layer units), with `extra` points (pins) inside it as
/// vertices of their own.
pub fn triangulate(outline: &[[f64; 2]], extra: &[[f64; 2]], o: &Options) -> Mesh {
    let mut poly = outline.to_vec();
    if area(&poly) < 0.0 {
        poly.reverse();
    }
    let size = area(&poly);
    if poly.len() < 3 || size <= 0.0 {
        return Mesh::default();
    }
    // the side of an equilateral triangle that makes about the number wanted
    let wanted = (o.triangles.max(2.0) * 2f64.powf((o.density - 50.0) / 25.0)).clamp(2.0, 20_000.0);
    let side = (4.0 * size / (3f64.sqrt() * wanted)).sqrt().max(0.5);
    let edge_pts = densify(&poly, side);
    let nb = edge_pts.len();
    let mut pts = edge_pts.clone();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in &poly {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let pins: Vec<[f64; 2]> = extra.iter().copied().filter(|p| inside(&poly, *p) && outline_dist2(&edge_pts, *p) > (0.2 * side).powi(2)).collect();
    let row_step = side * 3f64.sqrt() / 2.0;
    let (mut row, mut y) = (0u64, lo[1] + row_step / 2.0);
    while y < hi[1] {
        let (mut col, mut x) = (0u64, lo[0] + side * 0.25 + if row % 2 == 1 { side / 2.0 } else { 0.0 });
        while x < hi[0] {
            let k = row * 100_003 + col;
            let p = [x + shake(2 * k) * side * 0.08, y + shake(2 * k + 1) * side * 0.08];
            // away from the outline, and from the pins, which take their places
            if inside(&poly, p) && outline_dist2(&edge_pts, p) >= (0.55 * side).powi(2) && pins.iter().all(|q| dist2(*q, p) >= (0.55 * side).powi(2)) {
                pts.push(p);
            }
            x += side;
            col += 1;
        }
        y += row_step;
        row += 1;
    }
    for q in &pins {
        if pts.iter().all(|p| dist2(*p, *q) > 1e-12) {
            pts.push(*q);
        }
    }
    let mut tris = delaunay(&pts);
    let sides: Vec<(usize, usize)> = (0..nb).map(|i| (i, (i + 1) % nb)).collect();
    recover(&pts, &mut tris, &sides);
    tris.retain(|t| {
        let c = [(pts[t[0]][0] + pts[t[1]][0] + pts[t[2]][0]) / 3.0, (pts[t[0]][1] + pts[t[1]][1] + pts[t[2]][1]) / 3.0];
        cross(pts[t[0]], pts[t[1]], pts[t[2]]) > 1e-9 && inside(&poly, c)
    });
    // only the points the triangles use
    let mut index = vec![usize::MAX; pts.len()];
    let mut verts = Vec::new();
    for t in tris.iter_mut() {
        for i in t.iter_mut() {
            if index[*i] == usize::MAX {
                index[*i] = verts.len();
                verts.push(pts[*i]);
            }
            *i = index[*i];
        }
    }
    Mesh { verts, tris, outline: poly }
}
