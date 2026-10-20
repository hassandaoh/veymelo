//! Filling outlines by a fill rule. Each row of pixels is crossed at sixteen heights; along
//! each, the outlines' crossings give the stretches that are inside (by how many times the
//! outlines wind round them), and those stretches are added in exactly, to the fraction of a
//! pixel. Outlines may cross themselves and each other: the rule decides what is inside.

use crate::{Mask, Path};

/// How the inside of outlines that cross or lie in each other is decided.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rule {
    /// Inside where the outlines wind round a point any number of times but zero.
    #[default]
    NonZero,
    /// Inside where they wind round it an odd number of times (holes in holes are filled).
    EvenOdd,
}

/// Heights each row of pixels is crossed at.
const SUB: usize = 16;

struct Edge {
    top: f64,
    bottom: f64,
    /// x at `top`, and how much x changes a unit down.
    x: f64,
    slope: f64,
    /// +1 going down, -1 going up.
    dir: i32,
}

/// How much of each pixel `path` (its outlines taken as closed) covers by `rule`, in the box it
/// touches of a picture `width` × `height`.
pub fn fill(path: &Path, rule: Rule, width: usize, height: usize) -> Option<Mask> {
    let (x0, y0, x1, y1) = path.extent()?;
    let left = x0.floor().max(0.0) as usize;
    let top = y0.floor().max(0.0) as usize;
    let right = (x1.ceil().max(0.0) as usize).min(width);
    let bottom = (y1.ceil().max(0.0) as usize).min(height);
    if left >= right || top >= bottom {
        return None;
    }
    let mut edges: Vec<Edge> = Vec::new();
    for c in path.contours() {
        let n = c.len();
        if n < 2 {
            continue;
        }
        for k in 0..n {
            let (a, b) = (c[k], c[(k + 1) % n]);
            if a.1 == b.1 || !(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite()) {
                continue;
            }
            let (p, q, dir) = if a.1 < b.1 { (a, b, 1) } else { (b, a, -1) };
            edges.push(Edge { top: p.1, bottom: q.1, x: p.0, slope: (q.0 - p.0) / (q.1 - p.1), dir });
        }
    }
    edges.sort_by(|a, b| a.top.total_cmp(&b.top));
    let (w, h) = (right - left, bottom - top);
    let mut cover = vec![0f32; w * h];
    let mut run = vec![0f32; w + 1];
    let mut crossings: Vec<(f64, i32)> = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    let mut next = 0;
    let k = 1.0 / SUB as f32;
    for row in 0..h {
        let y = (top + row) as f64;
        // edges that reach into this row
        while next < edges.len() && edges[next].top < y + 1.0 {
            active.push(next);
            next += 1;
        }
        active.retain(|&e| edges[e].bottom > y);
        if active.is_empty() {
            continue;
        }
        run.iter_mut().for_each(|v| *v = 0.0);
        let line = &mut cover[row * w..(row + 1) * w];
        for s in 0..SUB {
            let ys = y + (s as f64 + 0.5) / SUB as f64;
            crossings.clear();
            for &e in &active {
                let e = &edges[e];
                if e.top <= ys && ys < e.bottom {
                    crossings.push((e.x + (ys - e.top) * e.slope, e.dir));
                }
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            for pair in crossings.windows(2) {
                winding += pair[0].1;
                let inside = match rule {
                    Rule::NonZero => winding != 0,
                    Rule::EvenOdd => winding % 2 != 0,
                };
                if inside {
                    span(line, &mut run, pair[0].0 - left as f64, pair[1].0 - left as f64, k);
                }
            }
        }
        // the whole pixels of each stretch were counted at its ends; add them up along the row
        let mut acc = 0.0;
        for (x, c) in line.iter_mut().enumerate() {
            acc += run[x];
            *c = (*c + acc).clamp(0.0, 1.0);
        }
    }
    Some(Mask { left, top, width: w, height: h, cover })
}

/// Add the stretch from `a` to `b` (in the box's pixels) at weight `k`: its ends exactly, the
/// whole pixels between as a step up and down in `run`.
fn span(line: &mut [f32], run: &mut [f32], a: f64, b: f64, k: f32) {
    let w = line.len() as f64;
    let (a, b) = (a.clamp(0.0, w), b.clamp(0.0, w));
    if b <= a {
        return;
    }
    let (ia, ib) = (a.floor() as usize, b.floor() as usize);
    if ia == ib {
        line[ia.min(line.len() - 1)] += (b - a) as f32 * k;
        return;
    }
    line[ia] += (ia as f64 + 1.0 - a) as f32 * k;
    run[ia + 1] += k;
    run[ib] -= k;
    if ib < line.len() {
        line[ib] += (b - ib as f64) as f32 * k;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(p: &mut Path, x: f64, y: f64, s: f64, clockwise: bool) {
        p.move_to(x, y);
        if clockwise {
            p.line_to(x + s, y);
            p.line_to(x + s, y + s);
            p.line_to(x, y + s);
        } else {
            p.line_to(x, y + s);
            p.line_to(x + s, y + s);
            p.line_to(x + s, y);
        }
        p.close();
    }

    fn area(m: &Mask) -> f32 {
        m.cover.iter().sum()
    }

    #[test]
    fn a_square_between_pixels() {
        let mut p = Path::default();
        square(&mut p, 2.5, 3.0, 4.0, true);
        let m = fill(&p, Rule::NonZero, 20, 20).unwrap();
        assert!((area(&m) - 16.0).abs() < 1e-4, "{}", area(&m));
        assert!((m.at(2, 4) - 0.5).abs() < 1e-5);
        assert!((m.at(3, 4) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn overlaps_and_holes_by_rule() {
        // two squares overlapping the same way round: their union either way
        let mut p = Path::default();
        square(&mut p, 0.0, 0.0, 10.0, true);
        square(&mut p, 5.0, 0.0, 10.0, true);
        assert!((area(&fill(&p, Rule::NonZero, 30, 30).unwrap()) - 150.0).abs() < 1e-3);
        assert!((area(&fill(&p, Rule::EvenOdd, 30, 30).unwrap()) - 100.0).abs() < 1e-3, "even-odd leaves the overlap out");
        // a square in a square, the same way round: filled by non-zero, a hole by even-odd
        let mut q = Path::default();
        square(&mut q, 0.0, 0.0, 10.0, true);
        square(&mut q, 2.0, 2.0, 6.0, true);
        assert!((area(&fill(&q, Rule::NonZero, 30, 30).unwrap()) - 100.0).abs() < 1e-3);
        assert!((area(&fill(&q, Rule::EvenOdd, 30, 30).unwrap()) - 64.0).abs() < 1e-3);
        // the other way round, it is a hole by both
        let mut r = Path::default();
        square(&mut r, 0.0, 0.0, 10.0, true);
        square(&mut r, 2.0, 2.0, 6.0, false);
        assert!((area(&fill(&r, Rule::NonZero, 30, 30).unwrap()) - 64.0).abs() < 1e-3);
    }

    #[test]
    fn a_circle_is_round() {
        let mut p = Path::default();
        p.move_to(60.0, 30.0);
        let k = 0.552_284_749_8 * 30.0;
        p.cubic_to(60.0, 30.0 + k, 30.0 + k, 60.0, 30.0, 60.0);
        p.cubic_to(30.0 - k, 60.0, 0.0, 30.0 + k, 0.0, 30.0);
        p.cubic_to(0.0, 30.0 - k, 30.0 - k, 0.0, 30.0, 0.0);
        p.cubic_to(30.0 + k, 0.0, 60.0, 30.0 - k, 60.0, 30.0);
        let a = area(&fill(&p, Rule::NonZero, 64, 64).unwrap());
        let want = std::f32::consts::PI * 900.0;
        assert!((a - want).abs() / want < 0.003, "{a} vs {want}");
    }

    #[test]
    fn clipped_to_the_picture() {
        let mut p = Path::default();
        square(&mut p, -5.0, -5.0, 10.0, true);
        let m = fill(&p, Rule::NonZero, 20, 20).unwrap();
        assert_eq!((m.left, m.top, m.width, m.height), (0, 0, 5, 5));
        assert!((area(&m) - 25.0).abs() < 1e-3);
        assert!(fill(&Path::default(), Rule::NonZero, 10, 10).is_none());
    }
}
