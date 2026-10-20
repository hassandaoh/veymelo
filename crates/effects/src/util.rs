//! Small things effects share.

use veymelo_raster::{Image, Px, par};

/// A colour as it is (not premultiplied) and its alpha.
pub fn unpremul(p: Px) -> ([f32; 3], f32) {
    let a = p[3];
    if a > 1e-7 { ([p[0] / a, p[1] / a, p[2] / a], a) } else { ([0.0; 3], 0.0) }
}

pub fn premul(c: [f32; 3], a: f32) -> Px {
    [c[0] * a, c[1] * a, c[2] * a, a]
}

/// How bright a colour looks (Rec. 709 weights).
pub fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// How bright a premultiplied pixel looks, as it is drawn (dark where it is see-through).
pub fn luma_px(p: Px) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// Every pixel through `f(x, y, pixel)`, rows shared among threads.
pub fn map_px(img: &mut Image, f: impl Fn(usize, usize, Px) -> Px + Sync) {
    let w = img.width;
    par::rows(&mut img.px, w, |y, row| {
        for (x, p) in row.iter_mut().enumerate() {
            *p = f(x, y, *p);
        }
    });
}

/// A pixel's channels kept to what a premultiplied pixel can be: alpha 0 to 1, colours not below
/// zero (32-bit colours may be brighter than white).
pub fn tidy(mut p: Px) -> Px {
    p[3] = p[3].clamp(0.0, 1.0);
    for c in &mut p[..3] {
        *c = c.max(0.0);
    }
    p
}

/// Red, green, blue as hue (0 to 1 round the wheel), saturation and lightness.
pub fn hsl(c: [f32; 3]) -> (f32, f32, f32) {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let l = (mx + mn) * 0.5;
    let d = mx - mn;
    if d.abs() < 1e-7 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
    let h = if mx == c[0] {
        (c[1] - c[2]) / d + if c[1] < c[2] { 6.0 } else { 0.0 }
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    (h / 6.0, s, l)
}

/// Hue (0 to 1), saturation and lightness as red, green, blue.
pub fn rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    if s <= 0.0 {
        return [l; 3];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let part = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [part(h + 1.0 / 3.0), part(h), part(h - 1.0 / 3.0)]
}

/// Every pixel with something in it through `f`, on its colour as it is (not premultiplied).
pub fn map_straight(img: &mut Image, f: impl Fn([f32; 3]) -> [f32; 3] + Sync) {
    map_px(img, |_, _, p| {
        if p[3] <= 0.0 {
            return p;
        }
        let (c, a) = unpremul(p);
        premul(f(c), a)
    });
}

pub fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
