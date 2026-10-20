//! A mask's edge softened by different amounts along it. Points on the outline say how far the
//! softening reaches there, outward (above zero) or inward (below); between them the reach goes
//! over along the outline's length, smoothly or straight as their tension says (round the start
//! of a closed outline too). Each pixel near the outline takes the reaches at its nearest place
//! on it, and is seen from whole at the inward reach to nothing at the outward one: outward only,
//! whole at the edge fading out; inward only, fading in to the edge; both, across the edge.
//! Where nothing reaches, the edge stays sharp.

use veymelo_keyframe::ShapePath;

/// The outline flattened in pixels: its points, how far along it each is, and for each piece
/// the (curve measure, length so far) of its steps, to find where a softening point is.
struct Flat {
    pts: Vec<[f64; 2]>,
    along: Vec<f64>,
    steps: Vec<Vec<(f64, f64)>>,
    length: f64,
}

fn cubic(c: &[[f64; 2]; 4], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let (a, b, d, e) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [a * c[0][0] + b * c[1][0] + d * c[2][0] + e * c[3][0], a * c[0][1] + b * c[1][1] + d * c[2][1] + e * c[3][1]]
}

fn flatten(path: &ShapePath, place: &dyn Fn([f64; 2]) -> [f64; 2]) -> Flat {
    let n = path.points.len();
    let mut flat = Flat { pts: Vec::new(), along: Vec::new(), steps: Vec::new(), length: 0.0 };
    for i in 0..path.pieces() {
        let j = (i + 1) % n;
        let (a, b) = (path.points[i], path.points[j]);
        let o = path.out_tangents.get(i).copied().unwrap_or([0.0; 2]);
        let t = path.in_tangents.get(j).copied().unwrap_or([0.0; 2]);
        let c = [place(a), place([a[0] + o[0], a[1] + o[1]]), place([b[0] + t[0], b[1] + t[1]]), place(b)];
        let hull = (0..3).map(|k| (c[k + 1][0] - c[k][0]).hypot(c[k + 1][1] - c[k][1])).sum::<f64>();
        let k = ((hull / 1.5).ceil() as usize).clamp(4, 1024);
        if flat.pts.is_empty() {
            flat.pts.push(c[0]);
            flat.along.push(0.0);
        }
        let mut steps = vec![(0.0, flat.length)];
        let mut last = c[0];
        for q in 1..=k {
            let u = q as f64 / k as f64;
            let p = cubic(&c, u);
            flat.length += (p[0] - last[0]).hypot(p[1] - last[1]);
            flat.pts.push(p);
            flat.along.push(flat.length);
            steps.push((u, flat.length));
            last = p;
        }
        flat.steps.push(steps);
    }
    flat
}

/// How far along the outline a place on piece `piece`, `at` on it, is.
fn along(flat: &Flat, piece: usize, at: f64) -> Option<f64> {
    let steps = flat.steps.get(piece)?;
    let at = at.clamp(0.0, 1.0);
    let k = steps.partition_point(|s| s.0 <= at).clamp(1, steps.len() - 1);
    let (a, b) = (steps[k - 1], steps[k]);
    let f = if b.0 > a.0 { (at - a.0) / (b.0 - a.0) } else { 0.0 };
    Some(a.1 + (b.1 - a.1) * f)
}

/// The reaches along the outline: (how far along, outward, inward, tension), in order.
struct Reaches {
    marks: Vec<(f64, f64, f64, f64)>,
    length: f64,
    closed: bool,
}

impl Reaches {
    /// The outward and inward reach `u` along the outline.
    fn at(&self, u: f64) -> (f64, f64) {
        let m = &self.marks;
        match m.len() {
            0 => return (0.0, 0.0),
            1 => return (m[0].1, m[0].2),
            _ => {}
        }
        // from one mark to the next: smooth, or straight as much as their tension says
        let between = |a: &(f64, f64, f64, f64), b: &(f64, f64, f64, f64), x: f64| {
            let x = x.clamp(0.0, 1.0);
            let smooth = x * x * (3.0 - 2.0 * x);
            let w = smooth + (x - smooth) * ((a.3 + b.3) / 2.0).clamp(0.0, 1.0);
            (a.1 + (b.1 - a.1) * w, a.2 + (b.2 - a.2) * w)
        };
        let (first, last) = (m[0], m[m.len() - 1]);
        if u < first.0 || u >= last.0 {
            if !self.closed {
                return if u < first.0 { (first.1, first.2) } else { (last.1, last.2) };
            }
            // round the start of the outline, from the last mark to the first
            let span = self.length - last.0 + first.0;
            let x = if u >= last.0 { u - last.0 } else { u + self.length - last.0 };
            return between(&last, &first, if span > 1e-9 { x / span } else { 0.0 });
        }
        let k = m.partition_point(|e| e.0 <= u).clamp(1, m.len() - 1);
        let span = m[k].0 - m[k - 1].0;
        between(&m[k - 1], &m[k], if span > 1e-9 { (u - m[k - 1].0) / span } else { 0.0 })
    }
}

/// How much of each pixel of a `w` × `h` picture the outline `path` covers with its softening
/// points: `place` takes the path's units to pixels, `scale` pixels to a unit; `hard` is how much
/// the outline covers each pixel unsoftened; `grow` (pixels) grows the outline first (below zero:
/// shrinks it); `linear` softens in a straight ramp instead of a smooth one.
#[allow(clippy::too_many_arguments)]
pub fn coverage(path: &ShapePath, place: &dyn Fn([f64; 2]) -> [f64; 2], scale: f64, w: usize, h: usize, grow: f64, linear: bool, hard: &[f32]) -> Vec<f32> {
    let flat = flatten(path, place);
    if flat.pts.len() < 2 {
        return hard.to_vec();
    }
    let mut marks: Vec<(f64, f64, f64, f64)> = path
        .feather
        .iter()
        .filter_map(|f| {
            let r = f.width * scale;
            Some((along(&flat, f.piece, f.at)?, r.max(0.0), (-r).max(0.0), f.tension))
        })
        .collect();
    marks.sort_by(|a, b| a.0.total_cmp(&b.0));
    let reaches = Reaches { marks, length: flat.length, closed: path.closed };
    let most = reaches.marks.iter().map(|m| m.1.max(m.2)).fold(0.0, f64::max);
    let reach = most + grow.abs() + 2.0;
    // each pixel's distance to the outline, and how far along the outline the nearest place is
    let mut dist = vec![f32::INFINITY; w * h];
    let mut at = vec![0f32; w * h];
    for k in 0..flat.pts.len() - 1 {
        let (a, b) = (flat.pts[k], flat.pts[k + 1]);
        let (sa, sb) = (flat.along[k], flat.along[k + 1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let x0 = (a[0].min(b[0]) - reach).floor().max(0.0) as usize;
        let y0 = (a[1].min(b[1]) - reach).floor().max(0.0) as usize;
        let x1 = (a[0].max(b[0]) + reach).ceil().min(w as f64 - 1.0);
        let y1 = (a[1].max(b[1]) + reach).ceil().min(h as f64 - 1.0);
        if x1 < 0.0 || y1 < 0.0 {
            continue;
        }
        for y in y0..=y1 as usize {
            for x in x0..=x1 as usize {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let t = if l2 > 0.0 { (((px - a[0]) * dx + (py - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                let d = (a[0] + dx * t - px).hypot(a[1] + dy * t - py) as f32;
                let i = y * w + x;
                if d < dist[i] {
                    dist[i] = d;
                    at[i] = (sa + (sb - sa) * t) as f32;
                }
            }
        }
    }
    let ramp = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        if linear { x } else { x * x * (3.0 - 2.0 * x) }
    };
    (0..w * h)
        .map(|i| {
            let inside = hard.get(i).copied().unwrap_or(0.0) >= 0.5;
            let d = dist[i] as f64;
            if !d.is_finite() || d > reach {
                return if inside { 1.0 } else { 0.0 };
            }
            // how far outside the grown edge (below zero: inside it)
            let out = if inside { -d } else { d } - grow;
            let (r_out, r_in) = reaches.at(at[i] as f64);
            // one ramp across the soft stretch, whole at the inward reach to nothing at the
            // outward one (so where both reach, the edge goes over without a step)
            let soft = r_out + r_in;
            let v = if soft > 0.5 { ramp((r_out - out) / soft) } else { (0.5 - out).clamp(0.0, 1.0) };
            v as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use veymelo_keyframe::FeatherPoint;

    /// A square from (50, 50) to (150, 150), clockwise: its pieces the top, right, bottom, left.
    fn square() -> ShapePath {
        ShapePath::polygon(&[[50.0, 50.0], [150.0, 50.0], [150.0, 150.0], [50.0, 150.0]], true)
    }

    fn hard(path: &ShapePath) -> Vec<f32> {
        let (w, h) = (200, 200);
        let mut out = vec![0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                if px >= path.points[0][0] && px <= path.points[1][0] && py >= path.points[0][1] && py <= path.points[2][1] {
                    out[y * w + x] = 1.0;
                }
            }
        }
        out
    }

    fn cover(path: &ShapePath, grow: f64, linear: bool) -> Vec<f32> {
        coverage(path, &|p| p, 1.0, 200, 200, grow, linear, &hard(path))
    }

    fn at(c: &[f32], x: usize, y: usize) -> f32 {
        c[y * 200 + x]
    }

    #[test]
    fn without_points_the_edge_is_sharp() {
        let c = cover(&square(), 0.0, false);
        assert!(at(&c, 100, 100) > 0.99 && at(&c, 40, 100) < 0.01);
    }

    #[test]
    fn one_point_softens_the_whole_outline() {
        let mut p = square();
        p.feather = vec![FeatherPoint { piece: 0, at: 0.5, width: 20.0, tension: 0.0 }].into();
        let c = cover(&p, 0.0, true);
        // 10.5 px above the top edge of a 20 px straight ramp: about half
        assert!((at(&c, 100, 39) - 0.475).abs() < 0.06, "{}", at(&c, 100, 39));
        assert!(at(&c, 100, 28) < 0.01);
        assert!(at(&c, 100, 60) > 0.99, "inside stays whole");
        assert!((at(&c, 160, 100) - 0.475).abs() < 0.06, "the right edge too: {}", at(&c, 160, 100));
    }

    #[test]
    fn outward_and_inward_points_change_along_the_outline() {
        let mut p = square();
        p.feather = vec![FeatherPoint { piece: 0, at: 0.5, width: 20.0, tension: 1.0 }, FeatherPoint { piece: 2, at: 0.5, width: -20.0, tension: 1.0 }].into();
        let c = cover(&p, 0.0, true);
        // the top softens outward, sharp inside
        assert!(at(&c, 100, 45) > 0.6 && at(&c, 100, 45) < 0.8, "{}", at(&c, 100, 45));
        assert!(at(&c, 100, 52) > 0.99);
        // the bottom softens inward: nothing at the edge, whole 20 px in
        assert!(at(&c, 100, 152) < 0.01);
        assert!((at(&c, 100, 140) - 0.475).abs() < 0.06, "{}", at(&c, 100, 140));
        // half way round, both reaches half, 10 px each way: one ramp 20 px across the edge,
        // half at the edge itself
        assert!((at(&c, 154, 100) - 0.275).abs() < 0.06, "{}", at(&c, 154, 100));
        assert!((at(&c, 145, 100) - 0.725).abs() < 0.06, "{}", at(&c, 145, 100));
        assert!((at(&c, 149, 100) - 0.525).abs() < 0.06, "{}", at(&c, 149, 100));
    }

    #[test]
    fn grown_and_scaled() {
        let mut p = square();
        p.feather = vec![FeatherPoint { piece: 0, at: 0.5, width: 10.0, tension: 0.0 }].into();
        let grown = cover(&p, 10.0, true);
        assert!(at(&grown, 100, 41) > 0.99, "grown by 10 px: {}", at(&grown, 100, 41));
        assert!((at(&grown, 100, 34) - 0.45).abs() < 0.06, "{}", at(&grown, 100, 34));
        // at half the pixels, a 10 px softening is 5
        let half = coverage(&p, &|q| [q[0] / 2.0, q[1] / 2.0], 0.5, 100, 100, 0.0, true, &{
            let mut v = vec![0f32; 100 * 100];
            for y in 25..75 {
                for x in 25..75 {
                    v[y * 100 + x] = 1.0;
                }
            }
            v
        });
        let v = half[22 * 100 + 50];
        assert!(v > 0.4 && v < 0.6, "{v}");
    }
}
