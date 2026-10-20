//! The app's icon, drawn by our own code at any size: a white tile with the rounded square
//! corners of the system's icons, the grid of dots of the opening, a serif italic v (as the
//! name is set), and after it the dot a pen leaves where it lifts.

use veymelo_path::Path;
use veymelo_raster::{Canvas, Dot, Rgb};

use crate::button::Label;
use crate::faces;

const TILE: Rgb = Rgb(252, 252, 250);
const EDGE: Rgb = Rgb(218, 218, 214);
const DOTS: Rgb = Rgb(190, 190, 198);
const INK: Rgb = Rgb(14, 14, 16);
const PEN: Rgb = Rgb(240, 84, 44);

/// The tile: a squircle (|x|⁵ + |y|⁵ = r⁵, as the system's icons are cut) round (`cx`, `cy`).
fn tile(cx: f64, cy: f64, r: f64) -> Path {
    let mut p = Path::default();
    let n = 360;
    for i in 0..n {
        let a = i as f64 / n as f64 * std::f64::consts::TAU;
        let (c, s) = (a.cos(), a.sin());
        let (x, y) = (c.signum() * c.abs().powf(0.4) * r, s.signum() * s.abs().powf(0.4) * r);
        if i == 0 {
            p.move_to(cx + x, cy + y);
        } else {
            p.line_to(cx + x, cy + y);
        }
    }
    p.close();
    p
}

fn disc(cx: f64, cy: f64, r: f64) -> Path {
    let mut p = Path::default();
    let n = 96;
    for i in 0..n {
        let a = i as f64 / n as f64 * std::f64::consts::TAU;
        let (x, y) = (cx + a.cos() * r, cy + a.sin() * r);
        if i == 0 {
            p.move_to(x, y);
        } else {
            p.line_to(x, y);
        }
    }
    p.close();
    p
}

/// The icon on paper of colour `paper`, `size` pixels square.
fn on(size: usize, paper: Rgb) -> Canvas {
    let k = size as f64 / 1024.0;
    let mut c = Canvas::new(size, size, paper);
    // a soft shadow under the tile, as the system's icons have
    if size >= 64 {
        c.shadow(100.0 * k, 112.0 * k, 824.0 * k, 824.0 * k, 190.0 * k, 34.0 * k, Rgb(0, 0, 0), 0.22);
    }
    let (mid, r) = (512.0 * k, 412.0 * k);
    // the tile, with a hairline edge so it holds on white
    let edge = (2.0 * k).max(if size >= 32 { 0.75 } else { 0.0 });
    if edge > 0.0 {
        c.fill_faded(&tile(mid, mid, r), EDGE, 1.0);
    }
    c.fill_faded(&tile(mid, mid, r - edge), TILE, 1.0);
    // the dots, fainter toward the tile's edge
    if size >= 64 {
        let step = 64.0 * k;
        let dot = Dot::new(4.6 * k, 0.0, 0.0);
        for j in -6..=6 {
            for i in -6..=6 {
                let (x, y) = (mid + i as f64 * step, mid + j as f64 * step);
                let d = ((x - mid) / (0.9 * r)).hypot((y - mid) / (0.9 * r));
                let a = (1.0 - d).max(0.0).powf(1.1) as f32;
                if a > 0.02 {
                    c.stamp(&dot, x.round() as i64, y.round() as i64, DOTS, a);
                }
            }
        }
    }
    // the v and the pen's dot after it, as a full stop, together in the middle
    if let Some(serif) = faces::serif() {
        let label = Label::new(serif.clone(), 1.0, "v");
        let shape = label.path(0.0, 0.0, 1.0, 1.0);
        if let Some((x0, y0, x1, y1)) = shape.extent() {
            let tall = 470.0 * k;
            let s = tall / (y1 - y0);
            let (w, h) = ((x1 - x0) * s, (y1 - y0) * s);
            let (dot, gap) = (46.0 * k, 34.0 * k);
            let (left, top) = (mid - (w + gap + 2.0 * dot) / 2.0, mid - h / 2.0 + 8.0 * k);
            let v = label.path(left / s - x0, top / s - y0, 1.0, s);
            c.fill_faded(&v, INK, 1.0);
            // where the pen lifted: on the line the v stands on
            c.fill_faded(&disc(left + w + gap + dot, top + h - dot, dot), PEN, 1.0);
        }
    }
    c
}

/// The icon, `size` pixels square, as straight RGBA: drawn on white and on black, its alpha
/// what tells the two apart.
pub fn rgba(size: usize) -> Vec<u8> {
    let (white, black) = (on(size, Rgb(255, 255, 255)), on(size, Rgb(0, 0, 0)));
    let mut out = Vec::with_capacity(size * size * 4);
    for (w, b) in white.pixels.chunks(4).zip(black.pixels.chunks(4)) {
        // on white a pixel is a·c + (1 − a), on black a·c
        let gap = (0..3).map(|i| w[i] as f32 - b[i] as f32).sum::<f32>() / 3.0;
        let a = (1.0 - gap / 255.0).clamp(0.0, 1.0);
        if a <= 0.0 {
            out.extend([0, 0, 0, 0]);
            continue;
        }
        let c = |i: usize| (b[i] as f32 / a).round().clamp(0.0, 255.0) as u8;
        out.extend([c(0), c(1), c(2), (a * 255.0).round() as u8]);
    }
    out
}

/// The icon at every size a macOS icon set holds, as PNG files in folder `dir` (for `iconutil`).
pub fn write_set(dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    for base in [16usize, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let size = base * scale;
            let png = veymelo_engine::render::png::encode(size as u32, size as u32, &rgba(size));
            let path = dir.join(format!("icon_{base}x{base}{suffix}.png"));
            std::fs::write(&path, png).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_clear_round_its_tile_and_solid_inside() {
        let size = 128;
        let px = rgba(size);
        let at = |x: usize, y: usize| &px[(y * size + x) * 4..(y * size + x) * 4 + 4];
        assert_eq!(at(1, 1)[3], 0, "a corner is clear");
        assert_eq!(at(64, 30)[3], 255, "the tile is solid");
        assert!(at(64, 30)[0] > 240, "and white: {:?}", at(64, 30));
    }
}
