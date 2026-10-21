//! A scene's flat layers arranged for rays (a scene of many of them): a box round each layer's
//! picture, the boxes in a tree, so a ray looks only at the layers whose boxes it passes
//! through instead of at every one. Each is then met as it always is (its plane, its picture),
//! in the layers' order, so what a ray finds is the same.

use veymelo_geom::{Vec3, vec3};

/// How many flat layers a scene has before its rays look at them through the tree.
pub(super) const FLAT_TREE: usize = 32;

/// The boxes: lowest and highest corners, then either the two below (`count` 0) or the run of
/// layers from `first` in `items`.
pub(super) struct Flats {
    nodes: Vec<(Vec3, Vec3, u32, u32, u32)>,
    items: Vec<(Vec3, Vec3, u32)>,
}

impl Flats {
    /// A tree of the boxes `boxes` (lowest and highest corners), each its layer's.
    pub fn new(boxes: &[(Vec3, Vec3)]) -> Flats {
        let mut items: Vec<(Vec3, Vec3, u32)> = boxes.iter().enumerate().map(|(i, &(lo, hi))| (lo, hi, i as u32)).collect();
        let mut nodes = Vec::new();
        let n = items.len();
        if n > 0 {
            Flats::split(&mut items, 0, n, &mut nodes);
        }
        Flats { nodes, items }
    }

    fn split(items: &mut [(Vec3, Vec3, u32)], first: usize, count: usize, nodes: &mut Vec<(Vec3, Vec3, u32, u32, u32)>) -> u32 {
        let part = &items[first..first + count];
        let lo = part.iter().fold(vec3(f64::INFINITY, f64::INFINITY, f64::INFINITY), |a, b| vec3(a.x.min(b.0.x), a.y.min(b.0.y), a.z.min(b.0.z)));
        let hi = part.iter().fold(vec3(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |a, b| vec3(a.x.max(b.1.x), a.y.max(b.1.y), a.z.max(b.1.z)));
        let at = nodes.len() as u32;
        if count <= 4 {
            nodes.push((lo, hi, 0, first as u32, count as u32));
            return at;
        }
        nodes.push((lo, hi, 0, 0, 0));
        // halved along the longest side, by the boxes' middles
        let size = hi - lo;
        let axis = if size.x >= size.y && size.x >= size.z { 0 } else if size.y >= size.z { 1 } else { 2 };
        let key = |b: &(Vec3, Vec3, u32)| {
            let c = b.0 + b.1;
            [c.x, c.y, c.z][axis]
        };
        items[first..first + count].sort_by(|a, b| key(a).total_cmp(&key(b)));
        let half = count / 2;
        let left = Flats::split(items, first, half, nodes);
        let right = Flats::split(items, first + half, count - half, nodes);
        nodes[at as usize] = (lo, hi, left, right, 0);
        at
    }

    /// The layers whose boxes the ray from `from` along `dir` passes through between `near` and
    /// `far`, in their order.
    pub fn along(&self, from: Vec3, dir: Vec3, near: f64, far: f64) -> Vec<usize> {
        let mut out = Vec::new();
        if self.nodes.is_empty() {
            return out;
        }
        let inv = vec3(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
        let crosses = |lo: Vec3, hi: Vec3| {
            let (mut t0, mut t1) = (near, far);
            for (o, d, l, h) in [(from.x, inv.x, lo.x, hi.x), (from.y, inv.y, lo.y, hi.y), (from.z, inv.z, lo.z, hi.z)] {
                // (along an axis the ray does not move on: inside the slab or not at all)
                if !d.is_finite() {
                    if o < l || o > h {
                        return false;
                    }
                    continue;
                }
                let (mut ta, mut tb) = ((l - o) * d, (h - o) * d);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            t0 <= t1
        };
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let (lo, hi, a, b, count) = self.nodes[i as usize];
            if !crosses(lo, hi) {
                continue;
            }
            if count == 0 {
                stack.extend([a, b]);
                continue;
            }
            for &(lo, hi, k) in &self.items[b as usize..(b + count) as usize] {
                if crosses(lo, hi) {
                    out.push(k as usize);
                }
            }
        }
        out.sort_unstable();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_finds_the_boxes_it_passes_through() {
        // a row of unit boxes along x, one far off
        let mut boxes: Vec<(Vec3, Vec3)> = (0..40).map(|i| (vec3(i as f64 * 2.0, 0.0, 0.0), vec3(i as f64 * 2.0 + 1.0, 1.0, 1.0))).collect();
        boxes.push((vec3(0.0, 50.0, 0.0), vec3(1.0, 51.0, 1.0)));
        let t = Flats::new(&boxes);
        // along the row through all of them, between 10 and 30 only those there
        let all = t.along(vec3(-1.0, 0.5, 0.5), vec3(1.0, 0.0, 0.0), 0.0, f64::INFINITY);
        assert_eq!(all, (0..40).collect::<Vec<_>>());
        let some = t.along(vec3(-1.0, 0.5, 0.5), vec3(1.0, 0.0, 0.0), 10.0, 30.0);
        assert_eq!(some, (4..15).collect::<Vec<_>>());
        // straight up through the first and the far one
        assert_eq!(t.along(vec3(0.5, -5.0, 0.5), vec3(0.0, 1.0, 0.0), 0.0, f64::INFINITY), vec![0, 40]);
    }
}
