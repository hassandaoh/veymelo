//! Drawing helpers for effects that paint: lines along outlines (round dabs and segments, or
//! filled convex pieces), lines found in a picture (contours), and how much each pixel is
//! covered.
//!
//! Coverage is a plane of numbers from 0 to 1, one a pixel, row by row.

use veymelo_raster::par;

/// A round-ended line from `a` to `b` (buffer pixels), `r` wide on each side, as strong as `v`.
#[derive(Clone, Copy, Debug)]
pub struct Seg {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub r: f64,
    pub v: f32,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much each pixel of a `w` × `h` picture the segments cover (the strongest where they
/// cross). `hardness` (0 to 1) is how much of each one's width is solid; the rest fades out.
pub fn segments(w: usize, h: usize, segs: &[Seg], hardness: f64) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    if w == 0 || h == 0 || segs.is_empty() {
        return out;
    }
    // which segments reach each row
    let mut rows: Vec<Vec<u32>> = vec![Vec::new(); h];
    for (i, s) in segs.iter().enumerate() {
        let pad = s.r + 1.0;
        let (y0, y1) = ((s.a[1].min(s.b[1]) - pad).floor().max(0.0), (s.a[1].max(s.b[1]) + pad).ceil().min(h as f64));
        if y1 <= 0.0 || y0 >= h as f64 {
            continue;
        }
        for row in rows.iter_mut().take(y1 as usize).skip(y0 as usize) {
            row.push(i as u32);
        }
    }
    let hard = hardness.clamp(0.0, 1.0);
    par::rows(&mut out, w, |y, row| {
        let py = y as f64 + 0.5;
        for &i in &rows[y] {
            let s = segs[i as usize];
            let pad = s.r + 1.0;
            let (x0, x1) = ((s.a[0].min(s.b[0]) - pad).floor().max(0.0) as usize, ((s.a[0].max(s.b[0]) + pad).ceil().max(0.0) as usize).min(w));
            let (dx, dy) = (s.b[0] - s.a[0], s.b[1] - s.a[1]);
            let l2 = dx * dx + dy * dy;
            let (inner, outer) = (s.r * hard - 0.5, s.r + 0.5);
            for (x, o) in row.iter_mut().enumerate().take(x1).skip(x0) {
                let px = x as f64 + 0.5;
                let t = if l2 > 1e-12 { (((px - s.a[0]) * dx + (py - s.a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                let d = (px - s.a[0] - dx * t).hypot(py - s.a[1] - dy * t);
                if d >= outer {
                    continue;
                }
                let c = if d <= inner { 1.0 } else { 1.0 - smoothstep(inner as f32, outer as f32, d as f32) } * s.v;
                if c > *o {
                    *o = c;
                }
            }
        }
    });
    out
}

/// How much each pixel the union of convex `polys` (buffer pixels) covers (16 points a pixel).
pub fn convex(w: usize, h: usize, polys: &[Vec<[f64; 2]>]) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    let inside = |poly: &[[f64; 2]], x: f64, y: f64| {
        let n = poly.len();
        let mut sign = 0.0;
        for i in 0..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let c = (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
            if c.abs() < 1e-12 {
                continue;
            }
            if sign == 0.0 {
                sign = c.signum();
            } else if c.signum() != sign {
                return false;
            }
        }
        true
    };
    for poly in polys.iter().filter(|p| p.len() >= 3) {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in poly {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let (xa, xb) = (lo[0].floor().max(0.0) as usize, (hi[0].ceil().max(0.0) as usize).min(w));
        let (ya, yb) = (lo[1].floor().max(0.0) as usize, (hi[1].ceil().max(0.0) as usize).min(h));
        for y in ya..yb {
            for x in xa..xb {
                let hits = (0..16).filter(|k| inside(poly, x as f64 + ((k % 4) as f64 + 0.5) / 4.0, y as f64 + ((k / 4) as f64 + 0.5) / 4.0)).count();
                let v = &mut out[y * w + x];
                *v = v.max(hits as f32 / 16.0);
            }
        }
    }
    out
}

/// Whether `q` is inside the outline (crossings counted).
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

/// How much each pixel the outlines cover (4 points a pixel), each inverted where asked.
pub fn filled(w: usize, h: usize, polys: &[(Vec<[f64; 2]>, bool)]) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    par::rows(&mut out, w, |y, row| {
        for (x, v) in row.iter_mut().enumerate() {
            let hits = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)].iter().filter(|(sx, sy)| polys.iter().any(|(p, inv)| inside(p, [x as f64 + sx, y as f64 + sy]) != *inv)).count();
            *v = hits as f32 / 4.0;
        }
    });
    out
}

/// The length of a line through `pts` (round to the start when `closed`).
pub fn length(pts: &[[f64; 2]], closed: bool) -> f64 {
    let n = pts.len();
    if n < 2 {
        return 0.0;
    }
    let sides = if closed { n } else { n - 1 };
    (0..sides).map(|i| (pts[(i + 1) % n][0] - pts[i][0]).hypot(pts[(i + 1) % n][1] - pts[i][1])).sum()
}

/// How far `q` is from the line through `pts` (round to the start when `closed`).
pub fn distance(pts: &[[f64; 2]], closed: bool, q: [f64; 2]) -> f64 {
    let n = pts.len();
    if n == 0 {
        return f64::INFINITY;
    }
    if n == 1 {
        return (q[0] - pts[0][0]).hypot(q[1] - pts[0][1]);
    }
    let sides = if closed { n } else { n - 1 };
    (0..sides)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let t = (((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / (dx * dx + dy * dy).max(1e-12)).clamp(0.0, 1.0);
            (q[0] - a[0] - dx * t).hypot(q[1] - a[1] - dy * t)
        })
        .fold(f64::INFINITY, f64::min)
}

/// `n` points evenly along the line (the last at its end, unless `closed`).
pub fn even(pts: &[[f64; 2]], closed: bool, n: usize) -> Vec<[f64; 2]> {
    let len = length(pts, closed);
    let parts = if closed { n as f64 } else { (n.max(2) - 1) as f64 };
    (0..n).map(|i| point_at(pts, closed, len * i as f64 / parts).0).collect()
}

/// The point `s` along the line, and the way it goes there.
pub fn point_at(pts: &[[f64; 2]], closed: bool, s: f64) -> ([f64; 2], [f64; 2]) {
    let n = pts.len();
    if n == 0 {
        return ([0.0; 2], [1.0, 0.0]);
    }
    if n == 1 {
        return (pts[0], [1.0, 0.0]);
    }
    let sides = if closed { n } else { n - 1 };
    let mut left = s.max(0.0);
    for i in 0..sides {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        if left <= l || i == sides - 1 {
            let t = if l > 0.0 { (left / l).clamp(0.0, 1.0) } else { 0.0 };
            let d = if l > 0.0 { [(b[0] - a[0]) / l, (b[1] - a[1]) / l] } else { [1.0, 0.0] };
            return ([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], d);
        }
        left -= l;
    }
    (pts[n - 1], [1.0, 0.0])
}

/// The part of the line from `s0` to `s1` along it.
pub fn trim(pts: &[[f64; 2]], closed: bool, s0: f64, s1: f64) -> Vec<[f64; 2]> {
    let n = pts.len();
    let mut out: Vec<[f64; 2]> = Vec::new();
    if n < 2 || s1 <= s0 {
        return out;
    }
    let sides = if closed { n } else { n - 1 };
    let mut acc = 0.0;
    for i in 0..sides {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        let (e0, e1) = (acc, acc + l);
        acc = e1;
        if e1 < s0 || e0 > s1 || l <= 0.0 {
            continue;
        }
        let at = |s: f64| {
            let t = ((s - e0) / l).clamp(0.0, 1.0);
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
        };
        let p0 = at(s0.max(e0));
        if out.last() != Some(&p0) {
            out.push(p0);
        }
        out.push(at(s1.min(e1)));
    }
    out
}

/// Segments along a line.
pub fn along(pts: &[[f64; 2]], closed: bool, r: f64, v: f32, out: &mut Vec<Seg>) {
    let n = pts.len();
    if n == 1 {
        out.push(Seg { a: pts[0], b: pts[0], r, v });
    }
    if n < 2 {
        return;
    }
    let sides = if closed { n } else { n - 1 };
    for i in 0..sides {
        out.push(Seg { a: pts[i], b: pts[(i + 1) % n], r, v });
    }
}

/// Lines where a plane of values crosses `level` (marching squares), joined end to end: each
/// line and whether it closes on itself.
pub fn contours(plane: &[f32], w: usize, h: usize, level: f32) -> Vec<(Vec<[f64; 2]>, bool)> {
    use std::collections::{HashMap, HashSet};
    if w < 2 || h < 2 {
        return Vec::new();
    }
    // an edge between pixel middles: across from (x, y) is 2·i, down is 2·i + 1
    let across = |x: usize, y: usize| 2 * (y * w + x) as u64;
    let down = |x: usize, y: usize| 2 * (y * w + x) as u64 + 1;
    let (mut at, mut next): (HashMap<u64, [f64; 2]>, HashMap<u64, Vec<u64>>) = (HashMap::new(), HashMap::new());
    let v = |x: usize, y: usize| plane[y * w + x];
    let part = |a: f32, b: f32| if (b - a).abs() > 1e-9 { ((level - a) / (b - a)).clamp(0.0, 1.0) as f64 } else { 0.5 };
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let (a, b, c, d) = (v(x, y), v(x + 1, y), v(x + 1, y + 1), v(x, y + 1));
            let case = (a > level) as u8 | ((b > level) as u8) << 1 | ((c > level) as u8) << 2 | ((d > level) as u8) << 3;
            if case == 0 || case == 15 {
                continue;
            }
            let e = [
                (across(x, y), [x as f64 + 0.5 + part(a, b), y as f64 + 0.5]),
                (down(x + 1, y), [x as f64 + 1.5, y as f64 + 0.5 + part(b, c)]),
                (across(x, y + 1), [x as f64 + 0.5 + part(d, c), y as f64 + 1.5]),
                (down(x, y), [x as f64 + 0.5, y as f64 + 0.5 + part(a, d)]),
            ];
            let pairs: &[(usize, usize)] = match case {
                1 | 14 => &[(3, 0)],
                2 | 13 => &[(0, 1)],
                3 | 12 => &[(3, 1)],
                4 | 11 => &[(1, 2)],
                5 => &[(3, 0), (1, 2)],
                6 | 9 => &[(0, 2)],
                7 | 8 => &[(3, 2)],
                10 => &[(0, 1), (2, 3)],
                _ => &[],
            };
            for &(i, j) in pairs {
                at.insert(e[i].0, e[i].1);
                at.insert(e[j].0, e[j].1);
                next.entry(e[i].0).or_default().push(e[j].0);
                next.entry(e[j].0).or_default().push(e[i].0);
            }
        }
    }
    let mut keys: Vec<u64> = next.keys().copied().collect();
    keys.sort_unstable();
    // open lines start at their ends first, then the loops
    keys.sort_by_key(|k| next[k].len() != 1);
    let mut used: HashSet<(u64, u64)> = HashSet::new();
    let pair = |a: u64, b: u64| (a.min(b), a.max(b));
    let mut out = Vec::new();
    for &k0 in &keys {
        for &first in &next[&k0] {
            if used.contains(&pair(k0, first)) {
                continue;
            }
            let mut line = vec![at[&k0]];
            let (mut prev, mut cur) = (k0, first);
            used.insert(pair(prev, cur));
            let mut closed = false;
            loop {
                if cur == k0 {
                    closed = true;
                    break;
                }
                line.push(at[&cur]);
                match next[&cur].iter().copied().find(|&n| n != prev && !used.contains(&pair(cur, n))) {
                    Some(n) => {
                        used.insert(pair(cur, n));
                        (prev, cur) = (cur, n);
                    }
                    None => break,
                }
            }
            if line.len() >= 2 {
                out.push((line, closed));
            }
        }
    }
    out
}

/// Convex pieces whose union is the stroke of `lines`, `r` wide on each side: a quad for each
/// side, the joins (round discs, mitres up to `miter` times the half width, bevels), the ends
/// (round, butt or projecting squares).
pub fn outline_pieces(lines: &[Vec<[f64; 2]>], r: f64, cap: u32, join: u32, miter: f64) -> Vec<Vec<[f64; 2]>> {
    let disc = |c: [f64; 2]| -> Vec<[f64; 2]> { (0..16).map(|k| (k as f64 / 16.0 * std::f64::consts::TAU).sin_cos()).map(|(s, co)| [c[0] + co * r, c[1] + s * r]).collect() };
    let dir = |a: [f64; 2], b: [f64; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy).max(1e-12);
        [dx / l, dy / l]
    };
    let mut out = Vec::new();
    for l in lines {
        let mut pts: Vec<[f64; 2]> = Vec::new();
        for p in l {
            if pts.last().is_none_or(|q: &[f64; 2]| (q[0] - p[0]).hypot(q[1] - p[1]) > 1e-9) {
                pts.push(*p);
            }
        }
        if pts.len() < 2 {
            if let (Some(p), 0) = (pts.first(), cap) {
                out.push(disc(*p));
            }
            continue;
        }
        let n = pts.len();
        for k in 0..n - 1 {
            let (mut a, mut b) = (pts[k], pts[k + 1]);
            let d = dir(a, b);
            if cap == 2 && k == 0 {
                a = [a[0] - d[0] * r, a[1] - d[1] * r];
            }
            if cap == 2 && k == n - 2 {
                b = [b[0] + d[0] * r, b[1] + d[1] * r];
            }
            let nn = [-d[1] * r, d[0] * r];
            out.push(vec![[a[0] + nn[0], a[1] + nn[1]], [b[0] + nn[0], b[1] + nn[1]], [b[0] - nn[0], b[1] - nn[1]], [a[0] - nn[0], a[1] - nn[1]]]);
        }
        for k in 1..n - 1 {
            let (p, d0, d1) = (pts[k], dir(pts[k - 1], pts[k]), dir(pts[k], pts[k + 1]));
            let cross = d0[0] * d1[1] - d0[1] * d1[0];
            if cross.abs() < 1e-9 && d0[0] * d1[0] + d0[1] * d1[1] > 0.0 {
                continue;
            }
            if join == 0 {
                out.push(disc(p));
                continue;
            }
            // the outer side of the turn
            let s = if cross > 0.0 { -1.0 } else { 1.0 };
            let (o0, o1) = ([p[0] - d0[1] * r * s, p[1] + d0[0] * r * s], [p[0] - d1[1] * r * s, p[1] + d1[0] * r * s]);
            if join == 1 {
                let cos_half = ((1.0 + d0[0] * d1[0] + d0[1] * d1[1]) * 0.5).max(0.0).sqrt();
                let len = if cos_half > 1e-6 { r / cos_half } else { f64::INFINITY };
                if len <= miter * r {
                    let m = [(o0[0] + o1[0]) * 0.5 - p[0], (o0[1] + o1[1]) * 0.5 - p[1]];
                    let ml = m[0].hypot(m[1]).max(1e-12);
                    out.push(vec![p, o0, [p[0] + m[0] / ml * len, p[1] + m[1] / ml * len], o1]);
                    continue;
                }
            }
            out.push(vec![p, o0, o1]);
        }
        if cap == 0 {
            out.push(disc(pts[0]));
            out.push(disc(pts[n - 1]));
        }
    }
    out
}

/// A line's corners cut off twice (Chaikin), its ends kept.
pub fn rounded(pts: &[[f64; 2]], times: usize) -> Vec<[f64; 2]> {
    let mut cur = pts.to_vec();
    for _ in 0..times {
        if cur.len() < 3 {
            break;
        }
        let mut next = vec![cur[0]];
        for w in cur.windows(2) {
            let (a, b) = (w[0], w[1]);
            next.push([a[0] * 0.75 + b[0] * 0.25, a[1] * 0.75 + b[1] * 0.25]);
            next.push([a[0] * 0.25 + b[0] * 0.75, a[1] * 0.25 + b[1] * 0.75]);
        }
        next.push(*cur.last().expect("some points"));
        cur = next;
    }
    cur
}

/// A plane of 0 and 1 grown (or, below zero, shrunk) by `r` pixels, its edge soft over a pixel.
pub fn grown(plane: &[f32], w: usize, h: usize, r: f64) -> Vec<f32> {
    if r.abs() < 1e-9 {
        return plane.to_vec();
    }
    if r > 0.0 {
        // how far each pixel is from the part that is on
        let on: Vec<bool> = plane.iter().map(|v| *v > 0.5).collect();
        crate::puppet::mesh::distance_sq(w, h, &on).iter().map(|d| (r - d.sqrt() + 0.5).clamp(0.0, 1.0) as f32).collect()
    } else {
        // how far each pixel is from the part that is off
        let off: Vec<bool> = plane.iter().map(|v| *v <= 0.5).collect();
        crate::puppet::mesh::distance_sq(w, h, &off).iter().map(|d| (d.sqrt() + r + 0.5).clamp(0.0, 1.0) as f32).collect()
    }
}
