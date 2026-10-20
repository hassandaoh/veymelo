//! Blurs: a box blur run three times is close to a Gaussian, and costs the same however wide.

use crate::par;

/// Blur one value a pixel (`w` × `h`, row by row) by boxes `rx` and `ry` pixels wide, three
/// times over: an edge spreads over about `rx` (and `ry`) pixels in all.
pub fn soften(values: &mut [f32], w: usize, h: usize, rx: f64, ry: f64) {
    for _ in 0..3 {
        if rx > 0.0 {
            par::rows(values, w, |_, row| box_line(row, rx / 3.0));
        }
        if ry > 0.0 {
            // columns: turned into rows and back
            let mut t = transpose(values, w, h);
            par::rows(&mut t, h, |_, col| box_line(col, ry / 3.0));
            values.copy_from_slice(&transpose(&t, h, w));
        }
    }
}

/// Blur one value a pixel by one box `rx` and `ry` pixels wide: an edge becomes a straight ramp
/// that wide.
pub fn ramp(values: &mut [f32], w: usize, h: usize, rx: f64, ry: f64) {
    if rx > 0.0 {
        par::rows(values, w, |_, row| box_line(row, rx));
    }
    if ry > 0.0 {
        let mut t = transpose(values, w, h);
        par::rows(&mut t, h, |_, col| box_line(col, ry));
        values.copy_from_slice(&transpose(&t, h, w));
    }
}

fn transpose(v: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0; v.len()];
    for y in 0..h {
        for x in 0..w {
            out[x * h + y] = v[y * w + x];
        }
    }
    out
}

/// One line blurred by a box `width` pixels wide: each pixel becomes the mean of what the box
/// centred on it covers, end pixels counting for the part of them it covers; past the ends is
/// nothing.
fn box_line(line: &mut [f32], width: f64) {
    let n = line.len();
    if n == 0 || width <= 1.0 {
        return;
    }
    // prefix sums make every run of whole pixels one subtraction
    let mut sum = vec![0f64; n + 1];
    for i in 0..n {
        sum[i + 1] = sum[i] + line[i] as f64;
    }
    let at = |j: i64| -> f64 { if j < 0 || j >= n as i64 { 0.0 } else { line[j as usize] as f64 } };
    let run = |a: i64, b: i64| -> f64 {
        let (a, b) = (a.clamp(0, n as i64) as usize, b.clamp(0, n as i64) as usize);
        if b > a { sum[b] - sum[a] } else { 0.0 }
    };
    let out: Vec<f32> = (0..n as i64)
        .map(|i| {
            let (a, b) = (i as f64 + 0.5 - width / 2.0, i as f64 + 0.5 + width / 2.0);
            let (fa, fb) = (a.floor(), b.floor());
            let first = (fa + 1.0 - a) * at(fa as i64);
            let last = (b - fb) * at(fb as i64);
            ((first + run(fa as i64 + 1, fb as i64) + last) / width) as f32
        })
        .collect();
    line.copy_from_slice(&out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blur_keeps_the_total_and_softens_the_edge() {
        let (w, h) = (40, 3);
        let mut v = vec![0f32; w * h];
        for y in 0..h {
            for x in 10..30 {
                v[y * w + x] = 1.0;
            }
        }
        let before: f32 = v.iter().sum();
        soften(&mut v, w, h, 6.0, 0.0);
        let after: f32 = v.iter().sum();
        assert!((before - after).abs() < 1e-3, "{before} {after}");
        let row = &v[w..2 * w];
        assert!(row[10] > 0.2 && row[10] < 0.8, "the edge is soft: {}", row[10]);
        assert!((row[20] - 1.0).abs() < 1e-4, "the middle stays");
        assert!(row[5] < 1e-3, "far out stays clear");
    }
}
