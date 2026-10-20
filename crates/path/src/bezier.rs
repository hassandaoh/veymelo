//! Bezier paths as their cubic pieces: lengths along them, pieces split where asked (the
//! outline kept exactly), vertices added by length, the first vertex moved, the direction
//! turned round.

use veymelo_keyframe::{FeatherPoint, ShapePath};

pub type P = [f64; 2];

pub fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}

pub fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

pub fn mul(a: P, k: f64) -> P {
    [a[0] * k, a[1] * k]
}

pub fn lerp(a: P, b: P, t: f64) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

pub fn dist(a: P, b: P) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// How many pieces a path has.
pub fn pieces(p: &ShapePath) -> usize {
    match p.points.len() {
        0 | 1 => 0,
        n if p.closed => n,
        n => n - 1,
    }
}

/// Piece `i`: its two ends and two control points.
pub fn piece(p: &ShapePath, i: usize) -> [P; 4] {
    let n = p.points.len();
    let j = (i + 1) % n;
    let (a, b) = (p.points[i], p.points[j]);
    [a, add(a, p.out_tangents[i]), add(b, p.in_tangents[j]), b]
}

pub fn at(c: &[P; 4], t: f64) -> P {
    let s = 1.0 - t;
    let (k0, k1, k2, k3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
    [c[0][0] * k0 + c[1][0] * k1 + c[2][0] * k2 + c[3][0] * k3, c[0][1] * k0 + c[1][1] * k1 + c[2][1] * k2 + c[3][1] * k3]
}

/// A piece's length (in short straight steps).
pub fn piece_length(c: &[P; 4]) -> f64 {
    let mut len = 0.0;
    let mut last = c[0];
    for k in 1..=48 {
        let q = at(c, k as f64 / 48.0);
        len += dist(last, q);
        last = q;
    }
    len
}

pub fn length(p: &ShapePath) -> f64 {
    (0..pieces(p)).map(|i| piece_length(&piece(p, i))).sum()
}

/// A piece split at `t` (de Casteljau): the two halves.
pub fn split(c: &[P; 4], t: f64) -> ([P; 4], [P; 4]) {
    let (ab, bc, cd) = (lerp(c[0], c[1], t), lerp(c[1], c[2], t), lerp(c[2], c[3], t));
    let (abc, bcd) = (lerp(ab, bc, t), lerp(bc, cd, t));
    let m = lerp(abc, bcd, t);
    ([c[0], ab, abc, m], [m, bcd, cd, c[3]])
}

/// The parameters that cut a piece into `k` parts of equal length.
fn even_cuts(c: &[P; 4], k: usize) -> Vec<f64> {
    if k <= 1 {
        return Vec::new();
    }
    const N: usize = 64;
    let mut acc = vec![0.0f64; N + 1];
    let mut last = c[0];
    for i in 1..=N {
        let q = at(c, i as f64 / N as f64);
        acc[i] = acc[i - 1] + dist(last, q);
        last = q;
    }
    let total = acc[N].max(1e-12);
    (1..k)
        .map(|j| {
            let want = total * j as f64 / k as f64;
            let i = acc.partition_point(|a| *a < want).clamp(1, N);
            let f = (want - acc[i - 1]) / (acc[i] - acc[i - 1]).max(1e-12);
            ((i - 1) as f64 + f) / N as f64
        })
        .collect()
}

/// A piece cut at parameters `ts` (rising): its parts.
fn split_many(c: &[P; 4], ts: &[f64]) -> Vec<[P; 4]> {
    let mut out = Vec::with_capacity(ts.len() + 1);
    let (mut rest, mut done) = (*c, 0.0);
    for &t in ts {
        let local = ((t - done) / (1.0 - done)).clamp(0.0, 1.0);
        let (a, b) = split(&rest, local);
        out.push(a);
        rest = b;
        done = t;
    }
    out.push(rest);
    out
}

/// A path made of pieces end to end.
pub fn from_pieces(cs: &[[P; 4]], closed: bool) -> ShapePath {
    let n = cs.len();
    let mut p = ShapePath { closed, ..ShapePath::default() };
    if n == 0 {
        return p;
    }
    for c in cs {
        p.points.push(c[0]);
        p.out_tangents.push(sub(c[1], c[0]));
        p.in_tangents.push([0.0; 2]);
    }
    // a vertex arrives as the piece before it ends
    for i in 1..n {
        p.in_tangents[i] = sub(cs[i - 1][2], cs[i - 1][3]);
    }
    if closed {
        p.in_tangents[0] = sub(cs[n - 1][2], cs[n - 1][3]);
    } else {
        p.points.push(cs[n - 1][3]);
        p.in_tangents.push(sub(cs[n - 1][2], cs[n - 1][3]));
        p.out_tangents.push([0.0; 2]);
    }
    p
}

/// The same outline with `total` vertices (at least as many as it has): its pieces cut so the
/// new vertices fall evenly by length.
pub fn with_vertices(p: &ShapePath, total: usize) -> ShapePath {
    let n = pieces(p);
    let have = p.points.len();
    if n == 0 || total <= have {
        return p.clone();
    }
    let lens: Vec<f64> = (0..n).map(|i| piece_length(&piece(p, i))).collect();
    let all: f64 = lens.iter().sum::<f64>().max(1e-12);
    // each piece's share of the vertices to add, largest remainders first
    let extra = total - have;
    let mut give: Vec<usize> = lens.iter().map(|l| (l / all * extra as f64).floor() as usize).collect();
    let left = extra - give.iter().sum::<usize>();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| (lens[b] / all * extra as f64).fract().total_cmp(&(lens[a] / all * extra as f64).fract()));
    for &i in order.iter().cycle().take(left) {
        give[i] += 1;
    }
    let mut cs = Vec::new();
    for (i, &g) in give.iter().enumerate() {
        let c = piece(p, i);
        cs.extend(split_many(&c, &even_cuts(&c, g + 1)));
    }
    from_pieces(&cs, p.closed)
}

/// The same outline starting at vertex `k`.
pub fn starting_at(p: &ShapePath, k: usize) -> ShapePath {
    let n = p.points.len();
    if n == 0 || !p.closed {
        return p.clone();
    }
    let r = |v: &[P]| (0..n).map(|i| v[(i + k) % n]).collect();
    // softening points stay where they are on the outline: their pieces numbered from `k`
    let feather = p.feather.iter().map(|f| FeatherPoint { piece: (f.piece + n - k % n) % n, ..*f }).collect();
    ShapePath { points: r(&p.points), in_tangents: r(&p.in_tangents), out_tangents: r(&p.out_tangents), closed: true, feather }
}

/// The same outline the other way round.
pub fn reversed(p: &ShapePath) -> ShapePath {
    let n = p.points.len();
    // each piece the other way round: its softening points from its other end
    let feather = p.feather.iter().filter(|_| n >= 2).map(|f| FeatherPoint { piece: if p.closed { (2 * n - 2 - f.piece) % n } else { n - 2 - f.piece.min(n - 2) }, at: 1.0 - f.at, ..*f }).collect();
    let mut q = ShapePath { points: p.points.iter().rev().copied().collect(), in_tangents: p.out_tangents.iter().rev().copied().collect(), out_tangents: p.in_tangents.iter().rev().copied().collect(), closed: p.closed, feather };
    if p.closed && !q.points.is_empty() {
        // keep the first vertex first
        q = starting_at(&q, q.points.len() - 1);
    }
    q
}

/// Twice the area a polygon encloses (above zero: clockwise on screen, y down).
pub fn signed_area(v: &[P]) -> f64 {
    let n = v.len();
    (0..n).map(|i| v[i][0] * v[(i + 1) % n][1] - v[(i + 1) % n][0] * v[i][1]).sum::<f64>() / 2.0
}

/// The point `f` (0 to 1) of the way along a path by its length, and the unit direction it
/// goes there; none for a path with no length.
pub fn along(p: &ShapePath, f: f64) -> Option<(P, P)> {
    let n = pieces(p);
    if n == 0 {
        return None;
    }
    let lens: Vec<f64> = (0..n).map(|i| piece_length(&piece(p, i))).collect();
    let total: f64 = lens.iter().sum();
    if total <= 1e-12 {
        return Some((p.points[0], [1.0, 0.0]));
    }
    let mut want = f.clamp(0.0, 1.0) * total;
    let mut i = 0;
    while i + 1 < n && want > lens[i] {
        want -= lens[i];
        i += 1;
    }
    let c = piece(p, i);
    // a straight piece: exactly that far along it
    if c[1] == c[0] && c[2] == c[3] {
        let len = lens[i].max(1e-12);
        let d = sub(c[3], c[0]);
        let l = d[0].hypot(d[1]);
        let dir = if l > 1e-12 { [d[0] / l, d[1] / l] } else { [1.0, 0.0] };
        return Some((lerp(c[0], c[3], (want / len).clamp(0.0, 1.0)), dir));
    }
    // the parameter at that length, from a table of lengths along the piece
    const N: usize = 256;
    let (mut acc, mut last, mut t) = (0.0, c[0], 1.0);
    for k in 1..=N {
        let q = at(&c, k as f64 / N as f64);
        let step = dist(last, q);
        if acc + step >= want {
            t = ((k - 1) as f64 + if step > 0.0 { (want - acc) / step } else { 0.0 }) / N as f64;
            break;
        }
        acc += step;
        last = q;
    }
    let s = 1.0 - t;
    let d = add(add(mul(sub(c[1], c[0]), 3.0 * s * s), mul(sub(c[2], c[1]), 6.0 * s * t)), mul(sub(c[3], c[2]), 3.0 * t * t));
    let l = d[0].hypot(d[1]);
    let dir = if l > 1e-12 { [d[0] / l, d[1] / l] } else { [1.0, 0.0] };
    Some((at(&c, t), dir))
}
