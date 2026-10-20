//! Line drawings seen from the one isometric angle, in hairline strokes: a thin plate with
//! round corners is drawn as its outline seen from above (the silhouette, the brighter line)
//! and the one crease where its top meets its side (dimmer), never its vertical corners; it is
//! filled with the paper, so it hides what is behind it. A picture may lie on its top, and a
//! pen may go round its edge (how far a render of it has come).

use veymelo_raster::{Canvas, Rgb};

/// How thick every line is (points).
pub const STROKE: f64 = 0.9;
const COS: f64 = 0.866_025_403_784_438_6;

/// Where a point of the room (`u` across, `v` deep, `z` up; points) falls on the screen,
/// `origin` being where the room's (0, 0, 0) is.
pub fn project(origin: (f64, f64), u: f64, v: f64, z: f64) -> (f64, f64) {
    (origin.0 + (u - v) * COS, origin.1 + (u + v) * 0.5 - z)
}

/// A thin plate: its width (`u`), depth (`v`), corner radius and thickness (points).
#[derive(Clone, Copy)]
pub struct Plate {
    pub w: f64,
    pub d: f64,
    pub round: f64,
    pub thick: f64,
}

impl Plate {
    /// Its outline seen from above, centred on (0, 0): the corners are quarter circles.
    fn outline(&self) -> Vec<(f64, f64)> {
        let (a, b) = (self.w / 2.0, self.d / 2.0);
        let r = self.round.min(a).min(b);
        let mut pts = vec![];
        for (cx, cy, from) in [(a - r, -b + r, -90.0f64), (a - r, b - r, 0.0), (-a + r, b - r, 90.0), (-a + r, -b + r, 180.0)] {
            for k in 0..=8 {
                let t = (from + 90.0 * k as f64 / 8.0).to_radians();
                pts.push((cx + r * t.cos(), cy + r * t.sin()));
            }
        }
        pts
    }
}

/// The convex hull of `pts` (they are all on a convex outline), going round.
fn hull(mut pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    pts.sort_by(|p, q| p.0.total_cmp(&q.0).then(p.1.total_cmp(&q.1)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<(f64, f64)> = vec![];
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<(f64, f64)> = vec![];
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Draws `plate` with its bottom at height `z` over `origin` (points; `scale` pixels to the
/// point): filled with `paper`, its silhouette in `edge`, its crease in `crease`, all `alpha`
/// opaque. `mark` puts a small upright plus of that size (points) over the middle of its top,
/// in that colour (drawn flat on the top, a plus would look like a cross).
#[allow(clippy::too_many_arguments)]
pub fn plate(canvas: &mut Canvas, scale: f64, origin: (f64, f64), z: f64, plate: &Plate, paper: Rgb, edge: Rgb, crease: Rgb, alpha: f32, mark: Option<(f64, Rgb)>) {
    let outline = plate.outline();
    let px = |p: (f64, f64)| (p.0 * scale, p.1 * scale);
    let top: Vec<(f64, f64)> = outline.iter().map(|&(u, v)| px(project(origin, u, v, z + plate.thick))).collect();
    let bottom: Vec<(f64, f64)> = outline.iter().map(|&(u, v)| px(project(origin, u, v, z))).collect();
    let mut all = top.clone();
    all.extend(bottom);
    let shape = hull(all);
    // the paper hides what is behind; the crease, then the silhouette over it
    let mut fill = veymelo_path::Path::default();
    if let Some(&(x, y)) = shape.first() {
        fill.move_to(x, y);
        for &(x, y) in &shape[1..] {
            fill.line_to(x, y);
        }
        fill.close();
    }
    canvas.fill_faded(&fill, paper, alpha);
    let width = STROKE * scale;
    canvas.stroke(&top, true, width, crease, alpha);
    if let Some((size, colour)) = mark {
        let (cx, cy) = project(origin, 0.0, 0.0, z + plate.thick);
        let h = size / 2.0;
        canvas.stroke(&[px((cx - h, cy)), px((cx + h, cy))], false, width * 1.2, colour, alpha);
        canvas.stroke(&[px((cx, cy - h)), px((cx, cy + h))], false, width * 1.2, colour, alpha);
    }
    canvas.stroke(&shape, true, width, edge, alpha);
}

/// A plate not there yet: its top at height `z` over `origin`, filled with `paper`, its
/// outline in dashes of `edge` (`dash` points drawn, then as much left out), with an upright
/// plus of `mark` points over its middle; all `alpha` opaque.
#[allow(clippy::too_many_arguments)]
pub fn ghost(canvas: &mut Canvas, scale: f64, origin: (f64, f64), z: f64, plate: &Plate, paper: Rgb, edge: Rgb, dash: f64, mark: f64, alpha: f32) {
    let outline = plate.outline();
    let mut ring: Vec<(f64, f64)> = outline.iter().map(|&(u, v)| project(origin, u, v, z + plate.thick)).collect();
    let mut fill = veymelo_path::Path::default();
    if let Some(&(x, y)) = ring.first() {
        fill.move_to(x * scale, y * scale);
        for &(x, y) in &ring[1..] {
            fill.line_to(x * scale, y * scale);
        }
        fill.close();
    }
    canvas.fill_faded(&fill, paper, alpha);
    if let Some(&first) = ring.first() {
        ring.push(first);
    }
    let width = STROKE * scale;
    // along the outline, a dash drawn then a gap, carried over from one side to the next
    let (mut along, mut piece): (f64, Vec<(f64, f64)>) = (0.0, vec![]);
    let on = |a: f64| (a / dash).floor() as i64 % 2 == 0;
    for w in ring.windows(2) {
        let len = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        let steps = (len / 0.5).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let f = k as f64 / steps as f64;
            let p = (w[0].0 + (w[1].0 - w[0].0) * f, w[0].1 + (w[1].1 - w[0].1) * f);
            if on(along + len * f) {
                piece.push((p.0 * scale, p.1 * scale));
            } else if piece.len() > 1 {
                canvas.stroke(&std::mem::take(&mut piece), false, width, edge, alpha);
            } else {
                piece.clear();
            }
        }
        along += len;
    }
    if piece.len() > 1 {
        canvas.stroke(&piece, false, width, edge, alpha);
    }
    let (cx, cy) = project(origin, 0.0, 0.0, z + plate.thick);
    let h = mark / 2.0;
    canvas.stroke(&[((cx - h) * scale, cy * scale), ((cx + h) * scale, cy * scale)], false, width * 1.2, edge, alpha);
    canvas.stroke(&[(cx * scale, (cy - h) * scale), (cx * scale, (cy + h) * scale)], false, width * 1.2, edge, alpha);
}

/// A pen going round the top edge of `plate` (its bottom at height `z` over `origin`): the
/// line it has drawn, `done` of the way round (0 to 1) from the plate's far corner, in `ink`
/// `width` points thick, and its tip, a dot of `pen` `dot` points across; all `alpha` opaque.
#[allow(clippy::too_many_arguments)]
pub fn trace(canvas: &mut Canvas, scale: f64, origin: (f64, f64), z: f64, plate: &Plate, done: f64, ink: Rgb, width: f64, pen: Rgb, dot: f64, alpha: f32) {
    let outline = plate.outline();
    // starting at the corner farthest back (the highest on the screen), going round
    let pts: Vec<(f64, f64)> = outline.iter().map(|&(u, v)| project(origin, u, v, z + plate.thick)).collect();
    let start = (0..pts.len()).min_by(|&a, &b| pts[a].1.total_cmp(&pts[b].1)).unwrap_or(0);
    let ring: Vec<(f64, f64)> = (0..=pts.len()).map(|k| pts[(start + k) % pts.len()]).collect();
    let total: f64 = ring.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum();
    let mut left = total * done.clamp(0.0, 1.0);
    let mut drawn = vec![ring[0]];
    for w in ring.windows(2) {
        let len = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        if len >= left {
            let k = if len > 0.0 { left / len } else { 0.0 };
            drawn.push((w[0].0 + (w[1].0 - w[0].0) * k, w[0].1 + (w[1].1 - w[0].1) * k));
            break;
        }
        left -= len;
        drawn.push(w[1]);
    }
    let px: Vec<(f64, f64)> = drawn.iter().map(|p| (p.0 * scale, p.1 * scale)).collect();
    if px.len() > 1 {
        canvas.stroke(&px, false, width * scale, ink, alpha);
    }
    if let Some(&(x, y)) = drawn.last() {
        let r = dot / 2.0;
        canvas.fill_faded(&veymelo_path::Path::rounded((x - r) * scale, (y - r) * scale, dot * scale, dot * scale, r * scale), pen, alpha);
    }
}

/// Draws a picture lying on the top of `plate` (its bottom at height `z` over `origin`;
/// points, `scale` pixels to the point): as big as fits `inset` points inside its edges at its
/// own shape (`aspect`: width over height), its top edge going back along the plate's width and
/// its left edge along its depth, coloured by `at` (from 0 to 1 across and down), its edges
/// smooth, a hairline in `edge` round it, all `alpha` opaque.
#[allow(clippy::too_many_arguments)]
pub fn picture(canvas: &mut Canvas, scale: f64, origin: (f64, f64), z: f64, plate: &Plate, inset: f64, aspect: f64, at: &dyn Fn(f64, f64) -> [f32; 3], edge: Rgb, alpha: f32) {
    let (room_w, room_d) = (plate.w - 2.0 * inset, plate.d - 2.0 * inset);
    if room_w <= 0.0 || room_d <= 0.0 || aspect <= 0.0 || alpha <= 0.0 {
        return;
    }
    let pw = room_w.min(room_d * aspect);
    let ph = pw / aspect;
    let (u0, v0) = (-pw / 2.0, -ph / 2.0);
    let top = z + plate.thick;
    let corners = [(u0, v0), (u0 + pw, v0), (u0 + pw, v0 + ph), (u0, v0 + ph)].map(|(u, v)| project(origin, u, v, top));
    let (x0, x1) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.0), b.max(c.0)));
    let (y0, y1) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.1), b.max(c.1)));
    let clampx = |v: f64| (v * scale).clamp(0.0, canvas.width as f64) as usize;
    let clampy = |v: f64| (v * scale).clamp(0.0, canvas.height as f64) as usize;
    let (px0, px1, py0, py1) = (clampx(x0 - 1.0), clampx(x1 + 1.0), clampy(y0 - 1.0), clampy(y1 + 1.0));
    // how far across and deep one pixel reaches (points), for the smooth edges
    let span = (0.5 / COS + 1.0) / scale;
    let soft = |d: f64| (d / span + 0.5).clamp(0.0, 1.0);
    for py in py0..py1 {
        for px in px0..px1 {
            // back from the screen to the plate's top
            let (x, y) = ((px as f64 + 0.5) / scale, (py as f64 + 0.5) / scale);
            let (a, b) = ((x - origin.0) / COS, 2.0 * (y - origin.1 + top));
            let (u, v) = ((a + b) / 2.0, (b - a) / 2.0);
            let cover = soft(u - u0) * soft(u0 + pw - u) * soft(v - v0) * soft(v0 + ph - v);
            if cover <= 0.0 {
                continue;
            }
            let c = at(((u - u0) / pw).clamp(0.0, 1.0), ((v - v0) / ph).clamp(0.0, 1.0));
            let k = cover as f32 * alpha;
            let i = (py * canvas.width + px) * 4;
            for (ch, value) in c.iter().enumerate() {
                let old = canvas.pixels[i + ch] as f32;
                canvas.pixels[i + ch] = (old + (value - old) * k).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    let ring: Vec<(f64, f64)> = corners.iter().map(|p| (p.0 * scale, p.1 * scale)).collect();
    canvas.stroke(&ring, true, STROKE * scale * 0.8, edge, alpha * 0.7);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_room_is_seen_from_the_usual_angle() {
        let o = (100.0, 100.0);
        assert_eq!(project(o, 0.0, 0.0, 0.0), (100.0, 100.0));
        // across goes right and down, deep goes left and down, up goes up
        let (x, y) = project(o, 10.0, 0.0, 0.0);
        assert!(x > 100.0 && y > 100.0);
        let (x, y) = project(o, 0.0, 10.0, 0.0);
        assert!(x < 100.0 && y > 100.0);
        assert_eq!(project(o, 0.0, 0.0, 10.0), (100.0, 90.0));
    }

    #[test]
    fn a_hull_goes_round_the_outside() {
        let h = hull(vec![(0.0, 0.0), (2.0, 0.0), (1.0, 1.0), (2.0, 2.0), (0.0, 2.0)]);
        assert_eq!(h.len(), 4);
        assert!(!h.contains(&(1.0, 1.0)));
    }
}
