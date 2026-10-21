//! Colour: the sRGB curve and linear light, HSL and HSV, luminance, colours written as hex,
//! and the 38 blend modes, by name and by how each mixes a pixel with the one under it.

pub mod aces;
mod blend;
pub mod space;

pub use blend::blend;

/// A colour, red, green, blue and alpha, 0 to 1 each, not premultiplied.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

pub const fn rgba(r: f64, g: f64, b: f64, a: f64) -> Rgba {
    Rgba { r, g, b, a }
}

impl Rgba {
    pub const BLACK: Rgba = rgba(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Rgba = rgba(1.0, 1.0, 1.0, 1.0);
    pub const CLEAR: Rgba = rgba(0.0, 0.0, 0.0, 0.0);

    /// The colour of `#rgb`, `#rrggbb` or `#rrggbbaa` (the `#` may be left out).
    pub fn from_hex(text: &str) -> Option<Rgba> {
        let h = text.trim().trim_start_matches('#');
        if !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
        match h.len() {
            3 => {
                let nib = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|v| (v * 17) as f64 / 255.0);
                Some(rgba(nib(0)?, nib(1)?, nib(2)?, 1.0))
            }
            6 => Some(rgba(byte(0)?, byte(2)?, byte(4)?, 1.0)),
            8 => Some(rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
            _ => None,
        }
    }

    /// As `#rrggbb` (or `#rrggbbaa` when not opaque).
    pub fn to_hex(&self) -> String {
        let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        if b(self.a) == 255 { format!("#{:02x}{:02x}{:02x}", b(self.r), b(self.g), b(self.b)) } else { format!("#{:02x}{:02x}{:02x}{:02x}", b(self.r), b(self.g), b(self.b), b(self.a)) }
    }

    /// How bright it looks (Rec. 709 weights, on the values as they are).
    pub fn luminance(&self) -> f64 {
        luminance(self.r, self.g, self.b)
    }

    pub fn with_alpha(self, a: f64) -> Rgba {
        Rgba { a, ..self }
    }

    pub fn lerp(self, o: Rgba, t: f64) -> Rgba {
        rgba(self.r + (o.r - self.r) * t, self.g + (o.g - self.g) * t, self.b + (o.b - self.b) * t, self.a + (o.a - self.a) * t)
    }
}

/// How bright red, green and blue look together (Rec. 709 weights).
pub fn luminance(r: f64, g: f64, b: f64) -> f64 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// An sRGB value (0 to 1, as stored in pictures) in linear light.
pub fn srgb_to_linear(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// A value in linear light as sRGB.
pub fn linear_to_srgb(v: f64) -> f64 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Red, green, blue (0 to 1) as hue (degrees, 0 to 360), saturation and lightness (0 to 1).
pub fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d.abs() < 1e-12 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    (hue(r, g, b, max, d), s, l)
}

/// Hue (degrees), saturation and lightness as red, green, blue.
pub fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    if s <= 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let h = h.rem_euclid(360.0) / 360.0;
    let part = |t: f64| {
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
    (part(h + 1.0 / 3.0), part(h), part(h - 1.0 / 3.0))
}

/// Red, green, blue as hue (degrees), saturation and value (0 to 1).
pub fn rgb_to_hsv(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let s = if max > 0.0 { d / max } else { 0.0 };
    let h = if d.abs() < 1e-12 { 0.0 } else { hue(r, g, b, max, d) };
    (h, s, max)
}

/// Hue (degrees), saturation and value as red, green, blue.
pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (r + m, g + m, b + m)
}

fn hue(r: f64, g: f64, b: f64, max: f64, d: f64) -> f64 {
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h * 60.0
}

/// How a layer's colours mix with what is under it: the 38 modes of compositing tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlendMode {
    #[default]
    Normal,
    Dissolve,
    DancingDissolve,
    Darken,
    Multiply,
    ColorBurn,
    ClassicColorBurn,
    LinearBurn,
    DarkerColor,
    Add,
    Lighten,
    Screen,
    ColorDodge,
    ClassicColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    LinearLight,
    VividLight,
    PinLight,
    HardMix,
    Difference,
    ClassicDifference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
    StencilAlpha,
    StencilLuma,
    SilhouetteAlpha,
    SilhouetteLuma,
    AlphaAdd,
    LuminescentPremul,
}

impl BlendMode {
    /// Every mode, in the order menus list them.
    pub const ALL: [BlendMode; 38] = [
        BlendMode::Normal,
        BlendMode::Dissolve,
        BlendMode::DancingDissolve,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::ClassicColorBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColor,
        BlendMode::Add,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::ClassicColorDodge,
        BlendMode::LinearDodge,
        BlendMode::LighterColor,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::LinearLight,
        BlendMode::VividLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::ClassicDifference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
        BlendMode::StencilAlpha,
        BlendMode::StencilLuma,
        BlendMode::SilhouetteAlpha,
        BlendMode::SilhouetteLuma,
        BlendMode::AlphaAdd,
        BlendMode::LuminescentPremul,
    ];

    /// Every mode's name, in the order of [`BlendMode::ALL`].
    pub const IDS: [&'static str; 38] = [
        "normal",
        "dissolve",
        "dancingDissolve",
        "darken",
        "multiply",
        "colorBurn",
        "classicColorBurn",
        "linearBurn",
        "darkerColor",
        "add",
        "lighten",
        "screen",
        "colorDodge",
        "classicColorDodge",
        "linearDodge",
        "lighterColor",
        "overlay",
        "softLight",
        "hardLight",
        "linearLight",
        "vividLight",
        "pinLight",
        "hardMix",
        "difference",
        "classicDifference",
        "exclusion",
        "subtract",
        "divide",
        "hue",
        "saturation",
        "color",
        "luminosity",
        "stencilAlpha",
        "stencilLuma",
        "silhouetteAlpha",
        "silhouetteLuma",
        "alphaAdd",
        "luminescentPremul",
    ];

    /// Its name in files and commands: `normal`, `colorBurn`, `luminescentPremul`…
    pub fn id(self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Dissolve => "dissolve",
            BlendMode::DancingDissolve => "dancingDissolve",
            BlendMode::Darken => "darken",
            BlendMode::Multiply => "multiply",
            BlendMode::ColorBurn => "colorBurn",
            BlendMode::ClassicColorBurn => "classicColorBurn",
            BlendMode::LinearBurn => "linearBurn",
            BlendMode::DarkerColor => "darkerColor",
            BlendMode::Add => "add",
            BlendMode::Lighten => "lighten",
            BlendMode::Screen => "screen",
            BlendMode::ColorDodge => "colorDodge",
            BlendMode::ClassicColorDodge => "classicColorDodge",
            BlendMode::LinearDodge => "linearDodge",
            BlendMode::LighterColor => "lighterColor",
            BlendMode::Overlay => "overlay",
            BlendMode::SoftLight => "softLight",
            BlendMode::HardLight => "hardLight",
            BlendMode::LinearLight => "linearLight",
            BlendMode::VividLight => "vividLight",
            BlendMode::PinLight => "pinLight",
            BlendMode::HardMix => "hardMix",
            BlendMode::Difference => "difference",
            BlendMode::ClassicDifference => "classicDifference",
            BlendMode::Exclusion => "exclusion",
            BlendMode::Subtract => "subtract",
            BlendMode::Divide => "divide",
            BlendMode::Hue => "hue",
            BlendMode::Saturation => "saturation",
            BlendMode::Color => "color",
            BlendMode::Luminosity => "luminosity",
            BlendMode::StencilAlpha => "stencilAlpha",
            BlendMode::StencilLuma => "stencilLuma",
            BlendMode::SilhouetteAlpha => "silhouetteAlpha",
            BlendMode::SilhouetteLuma => "silhouetteLuma",
            BlendMode::AlphaAdd => "alphaAdd",
            BlendMode::LuminescentPremul => "luminescentPremul",
        }
    }

    /// The mode named `id`.
    pub fn from_id(id: &str) -> Option<BlendMode> {
        BlendMode::ALL.into_iter().find(|m| m.id() == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn hex_colours() {
        assert_eq!(Rgba::from_hex("#ff8000"), Some(rgba(1.0, 128.0 / 255.0, 0.0, 1.0)));
        assert_eq!(Rgba::from_hex("f80"), Some(rgba(1.0, 136.0 / 255.0, 0.0, 1.0)));
        assert_eq!(Rgba::from_hex("#00000080").map(|c| c.a), Some(128.0 / 255.0));
        assert_eq!(Rgba::from_hex("#12345"), None);
        assert_eq!(Rgba::from_hex("#gg0000"), None);
        assert_eq!(rgba(1.0, 0.5, 0.0, 1.0).to_hex(), "#ff8000");
        assert_eq!(rgba(0.0, 0.0, 0.0, 0.5).to_hex(), "#00000080");
    }

    #[test]
    fn the_srgb_curve_goes_both_ways() {
        for i in 0..=100 {
            let v = i as f64 / 100.0;
            assert!(near(linear_to_srgb(srgb_to_linear(v)), v), "{v}");
        }
        // middle grey on screen is about a fifth of the light
        assert!((srgb_to_linear(0.5) - 0.214).abs() < 0.001);
    }

    #[test]
    fn hsl_and_hsv_go_both_ways() {
        for &(r, g, b) in &[(1.0, 0.0, 0.0), (0.2, 0.6, 0.4), (0.9, 0.9, 0.1), (0.3, 0.3, 0.3), (0.05, 0.1, 0.95)] {
            let (h, s, l) = rgb_to_hsl(r, g, b);
            let (r2, g2, b2) = hsl_to_rgb(h, s, l);
            assert!(near(r, r2) && near(g, g2) && near(b, b2), "hsl {r} {g} {b}");
            let (h, s, v) = rgb_to_hsv(r, g, b);
            let (r2, g2, b2) = hsv_to_rgb(h, s, v);
            assert!(near(r, r2) && near(g, g2) && near(b, b2), "hsv {r} {g} {b}");
        }
        assert_eq!(rgb_to_hsl(0.0, 1.0, 0.0).0, 120.0);
        assert!(near(luminance(1.0, 1.0, 1.0), 1.0));
    }

    #[test]
    fn blend_modes_by_name() {
        assert_eq!(BlendMode::ALL.len(), 38);
        for (m, id) in BlendMode::ALL.into_iter().zip(BlendMode::IDS) {
            assert_eq!(BlendMode::from_id(m.id()), Some(m));
            assert_eq!(m.id(), id);
        }
        let mut ids: Vec<_> = BlendMode::ALL.iter().map(|m| m.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 38, "every name once");
        assert_eq!(BlendMode::from_id("nope"), None);
    }
}
