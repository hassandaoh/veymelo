//! How a layer's pixel mixes with the pixel under it, for each blend mode.
//!
//! Pixels are premultiplied RGBA. The general rule (the W3C compositing model) is
//! `out = src·(1 − αb) + dst·(1 − αs) + αs·αb·B(cb, cs)`, where `B` is the mode's formula on
//! colours that are not premultiplied: where only one of the two has colour it shows as it is,
//! and where both do, the formula decides. A few modes act differently (Dissolve, the stencils,
//! Alpha Add), and the classic ones fade toward a colour that changes nothing instead of
//! fading their result.

use crate::{BlendMode, luminance};

impl BlendMode {
    /// Modes that change what is under the layer (by its alpha or brightness) instead of
    /// drawing the layer.
    pub fn is_stencil(self) -> bool {
        matches!(self, BlendMode::StencilAlpha | BlendMode::StencilLuma | BlendMode::SilhouetteAlpha | BlendMode::SilhouetteLuma)
    }

    /// Modes whose formulas hold for colours brighter than white (in floating-point work).
    /// The others take their colours as 0 to 1.
    pub fn keeps_over_range(self) -> bool {
        !matches!(
            self,
            BlendMode::ColorDodge
                | BlendMode::ClassicColorDodge
                | BlendMode::SoftLight
                | BlendMode::HardLight
                | BlendMode::LinearLight
                | BlendMode::VividLight
                | BlendMode::PinLight
                | BlendMode::HardMix
                | BlendMode::Hue
                | BlendMode::Saturation
                | BlendMode::Color
        )
    }

    /// For a classic mode, the source colour that leaves what is under it as it was.
    fn classic_neutral(self) -> Option<f32> {
        match self {
            BlendMode::ClassicColorDodge | BlendMode::ClassicDifference => Some(0.0),
            BlendMode::ClassicColorBurn => Some(1.0),
            _ => None,
        }
    }
}

fn color_burn(b: f32, s: f32) -> f32 {
    if b >= 1.0 {
        1.0
    } else if s <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - b) / s).min(1.0)
    }
}

fn color_dodge(b: f32, s: f32) -> f32 {
    if b <= 0.0 {
        0.0
    } else if s >= 1.0 {
        1.0
    } else {
        (b / (1.0 - s)).min(1.0)
    }
}

fn multiply(b: f32, s: f32) -> f32 {
    b * s
}

fn screen(b: f32, s: f32) -> f32 {
    b + s - b * s
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 { multiply(b, 2.0 * s) } else { screen(b, 2.0 * s - 1.0) }
}

fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b - (1.0 - 2.0 * s) * b * (1.0 - b)
    } else {
        let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
        b + (2.0 * s - 1.0) * (d - b)
    }
}

/// A mode that works on each of red, green and blue alone.
fn channel(mode: BlendMode, b: f32, s: f32) -> f32 {
    match mode {
        BlendMode::Darken => b.min(s),
        BlendMode::Multiply => multiply(b, s),
        BlendMode::ColorBurn => color_burn(b, s),
        // the old burn, which lets results go below black
        BlendMode::ClassicColorBurn => {
            if s <= 0.0 {
                0.0
            } else {
                1.0 - (1.0 - b) / s
            }
        }
        BlendMode::LinearBurn => (b + s - 1.0).max(0.0),
        BlendMode::Add | BlendMode::LinearDodge => b + s,
        BlendMode::Lighten => b.max(s),
        BlendMode::Screen => screen(b, s),
        BlendMode::ColorDodge => color_dodge(b, s),
        BlendMode::ClassicColorDodge => {
            if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        BlendMode::Overlay => hard_light(s, b),
        BlendMode::SoftLight => soft_light(b, s),
        BlendMode::HardLight => hard_light(b, s),
        BlendMode::LinearLight => (b + 2.0 * s - 1.0).clamp(0.0, 1.0),
        BlendMode::VividLight => {
            if s <= 0.5 {
                color_burn(b, 2.0 * s)
            } else {
                color_dodge(b, 2.0 * s - 1.0)
            }
        }
        BlendMode::PinLight => {
            if s <= 0.5 {
                b.min(2.0 * s)
            } else {
                b.max(2.0 * s - 1.0)
            }
        }
        BlendMode::HardMix => {
            if b + s >= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        BlendMode::Difference | BlendMode::ClassicDifference => (b - s).abs(),
        BlendMode::Exclusion => b + s - 2.0 * b * s,
        BlendMode::Subtract => (b - s).max(0.0),
        BlendMode::Divide => {
            if s <= 0.0 {
                if b > 0.0 { 1.0 } else { 0.0 }
            } else {
                b / s
            }
        }
        _ => s,
    }
}

// The colour modes keep one colour's brightness and take hue or saturation from the other,
// with brightness weighed 0.3, 0.59, 0.11 (as the colour modes of compositing tools do).

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

/// Bring a colour back between black and white without changing its brightness.
fn clip(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let (lo, hi) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
    let mut o = c;
    if lo < 0.0 {
        o = o.map(|v| l + (v - l) * l / (l - lo).max(1e-9));
    }
    if hi > 1.0 {
        o = o.map(|v| l + (v - l) * (1.0 - l) / (hi - l).max(1e-9));
    }
    o
}

fn with_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip(c.map(|v| v + d))
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn with_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let (lo, hi) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
    if hi - lo <= 1e-9 {
        return [0.0; 3];
    }
    c.map(|v| (v - lo) * s / (hi - lo))
}

/// The mode's formula on colours that are not premultiplied.
fn formula(mode: BlendMode, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    match mode {
        BlendMode::Hue => with_lum(with_sat(s, sat(b)), lum(b)),
        BlendMode::Saturation => with_lum(with_sat(b, sat(s)), lum(b)),
        BlendMode::Color => with_lum(s, lum(b)),
        BlendMode::Luminosity => with_lum(b, lum(s)),
        BlendMode::DarkerColor => {
            if luminance(s[0] as f64, s[1] as f64, s[2] as f64) < luminance(b[0] as f64, b[1] as f64, b[2] as f64) {
                s
            } else {
                b
            }
        }
        BlendMode::LighterColor => {
            if luminance(s[0] as f64, s[1] as f64, s[2] as f64) > luminance(b[0] as f64, b[1] as f64, b[2] as f64) {
                s
            } else {
                b
            }
        }
        _ => [channel(mode, b[0], s[0]), channel(mode, b[1], s[1]), channel(mode, b[2], s[2])],
    }
}

/// Mix premultiplied pixel `src` (its opacity already in it) onto premultiplied `dst` with
/// `mode`. `noise` (0 to 1, a different one for each pixel) decides Dissolve.
#[inline]
pub fn blend(mode: BlendMode, dst: [f32; 4], src: [f32; 4], noise: f32) -> [f32; 4] {
    let (sa, da) = (src[3], dst[3]);
    match mode {
        BlendMode::Normal => {
            let k = 1.0 - sa;
            return [src[0] + dst[0] * k, src[1] + dst[1] * k, src[2] + dst[2] * k, sa + da * k];
        }
        // each pixel is all layer or all what is under it, the layer's alpha its chance
        BlendMode::Dissolve | BlendMode::DancingDissolve => {
            if sa <= 0.0 || noise >= sa {
                return dst;
            }
            return [src[0] / sa, src[1] / sa, src[2] / sa, 1.0];
        }
        BlendMode::StencilAlpha => return dst.map(|v| v * sa),
        BlendMode::SilhouetteAlpha => return dst.map(|v| v * (1.0 - sa)),
        BlendMode::StencilLuma => return dst.map(|v| v * luminance(src[0] as f64, src[1] as f64, src[2] as f64) as f32),
        BlendMode::SilhouetteLuma => return dst.map(|v| v * (1.0 - luminance(src[0] as f64, src[1] as f64, src[2] as f64) as f32)),
        // colours as Normal; alphas added, so two edges side by side leave no seam
        BlendMode::AlphaAdd => {
            let k = 1.0 - sa;
            return [src[0] + dst[0] * k, src[1] + dst[1] * k, src[2] + dst[2] * k, (sa + da).min(1.0)];
        }
        // colours as Normal without being held under the alpha
        BlendMode::LuminescentPremul => {
            let k = 1.0 - sa;
            return [src[0] + dst[0] * k, src[1] + dst[1] * k, src[2] + dst[2] * k, sa + da - sa * da];
        }
        _ => {}
    }
    if sa <= 0.0 {
        return dst;
    }
    if da <= 0.0 {
        return src;
    }
    let mut cs = [src[0] / sa, src[1] / sa, src[2] / sa];
    let mut cb = [dst[0] / da, dst[1] / da, dst[2] / da];
    if !mode.keeps_over_range() {
        cs = cs.map(|v| v.clamp(0.0, 1.0));
        cb = cb.map(|v| v.clamp(0.0, 1.0));
    }
    let mut out = [0.0f32; 4];
    if let Some(neutral) = mode.classic_neutral() {
        // the source fades toward the colour that changes nothing, then works at full strength
        let b = formula(mode, cb, cs.map(|v| neutral + sa * (v - neutral)));
        for i in 0..3 {
            out[i] = (1.0 - da) * src[i] + da * b[i];
        }
    } else {
        let b = formula(mode, cb, cs);
        for i in 0..3 {
            out[i] = (1.0 - da) * src[i] + (1.0 - sa) * dst[i] + sa * da * b[i];
        }
    }
    out[3] = sa + da - sa * da;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(v: f32) -> [f32; 4] {
        [v, v, v, 1.0]
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn formulas_on_solid_pixels() {
        let (b, s) = (0.6f32, 0.3f32);
        let r = |m| blend(m, solid(b), solid(s), 0.5)[0];
        for (mode, want) in [
            (BlendMode::Normal, s),
            (BlendMode::Darken, s),
            (BlendMode::Multiply, b * s),
            (BlendMode::ColorBurn, 1.0 - ((1.0 - b) / s).min(1.0)),
            (BlendMode::LinearBurn, 0.0),
            (BlendMode::Add, b + s),
            (BlendMode::LinearDodge, b + s),
            (BlendMode::Lighten, b),
            (BlendMode::Screen, b + s - b * s),
            (BlendMode::ColorDodge, b / (1.0 - s)),
            (BlendMode::Overlay, 0.3 + 0.2 - 0.06),
            (BlendMode::HardLight, b * 2.0 * s),
            (BlendMode::LinearLight, b + 2.0 * s - 1.0),
            (BlendMode::PinLight, b.min(2.0 * s)),
            (BlendMode::HardMix, 0.0),
            (BlendMode::Difference, b - s),
            (BlendMode::Exclusion, b + s - 2.0 * b * s),
            (BlendMode::Subtract, b - s),
            (BlendMode::Divide, b / s),
        ] {
            assert!(close(r(mode), want), "{mode:?}: {} vs {want}", r(mode));
        }
        assert!(close(blend(BlendMode::HardMix, solid(0.6), solid(0.5), 0.5)[0], 1.0));
    }

    #[test]
    fn nothing_over_something_changes_nothing() {
        let d = [0.2, 0.3, 0.4, 0.8];
        for m in BlendMode::ALL {
            if m.is_stencil() {
                continue;
            }
            let o = blend(m, d, [0.0; 4], 0.5);
            assert!((0..4).all(|i| close(o[i], d[i])), "{m:?}: {o:?}");
        }
    }

    #[test]
    fn something_over_nothing_is_itself() {
        let s = [0.25, 0.1, 0.05, 0.5];
        for m in BlendMode::ALL {
            if m.is_stencil() || matches!(m, BlendMode::Dissolve | BlendMode::DancingDissolve) {
                continue;
            }
            let o = blend(m, [0.0; 4], s, 0.5);
            assert!((0..4).all(|i| close(o[i], s[i])), "{m:?}: {o:?}");
        }
    }

    #[test]
    fn half_a_layer_is_half_the_change() {
        // with the general rule, a layer at half opacity lands halfway between below and its result
        let (b, s) = (solid(0.6), [0.15, 0.15, 0.15, 0.5]);
        let full = blend(BlendMode::Multiply, solid(0.6), solid(0.3), 0.5)[0];
        assert!(close(blend(BlendMode::Multiply, b, s, 0.5)[0], 0.5 * 0.6 + 0.5 * full));
    }

    #[test]
    fn stencils_cut_what_is_under() {
        let d = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(blend(BlendMode::StencilAlpha, d, [0.0; 4], 0.0), [0.0; 4]);
        assert_eq!(blend(BlendMode::SilhouetteAlpha, d, [0.0; 4], 0.0), d);
        assert_eq!(blend(BlendMode::StencilAlpha, d, [0.1, 0.1, 0.1, 0.25], 0.0), [0.125, 0.125, 0.125, 0.25]);
        let white = blend(BlendMode::StencilLuma, d, [1.0, 1.0, 1.0, 1.0], 0.0);
        assert!(close(white[3], 1.0) && close(blend(BlendMode::SilhouetteLuma, d, [1.0, 1.0, 1.0, 1.0], 0.0)[3], 0.0));
    }

    #[test]
    fn dissolve_is_all_or_nothing() {
        let (d, s) = (solid(0.2), [0.3, 0.3, 0.3, 0.6]);
        assert_eq!(blend(BlendMode::Dissolve, d, s, 0.59), solid(0.5));
        assert_eq!(blend(BlendMode::Dissolve, d, s, 0.61), d);
    }

    #[test]
    fn brighter_than_white() {
        let (d, s) = ([2.0, 2.0, 2.0, 1.0], solid(0.5));
        assert!(close(blend(BlendMode::Add, d, s, 0.5)[0], 2.5));
        assert!(close(blend(BlendMode::Screen, d, s, 0.5)[0], 1.5));
        assert!(blend(BlendMode::SoftLight, d, s, 0.5)[0] <= 1.0, "soft light takes colours as 0 to 1");
    }

    #[test]
    fn classic_modes_fade_toward_no_change() {
        // at opacity k a classic mode works as if its colour were k of the way from its neutral
        let at = |m, b: f32, o: f32, k: f32| blend(m, solid(b), [o * k, o * k, o * k, k], 0.5)[0];
        for (b, o) in [(0.4f32, 0.5f32), (0.8, 0.9), (0.1, 0.3)] {
            let k = 0.15;
            assert!(close(at(BlendMode::ClassicColorDodge, b, o, k), (b / (1.0 - k * o)).min(1.0)));
            assert!(close(at(BlendMode::ClassicDifference, b, o, k), (b - k * o).abs()));
            assert!(close(at(BlendMode::ClassicColorBurn, b, o, k), 1.0 - (1.0 - b) / (1.0 - k * (1.0 - o))));
        }
        assert!(close(at(BlendMode::ClassicColorBurn, 0.0, 0.1, 1.0), -9.0), "the classic burn goes below black");
    }

    #[test]
    fn colour_modes_keep_brightness() {
        let below = [0.2f32, 0.4, 0.6, 1.0];
        let red = [1.0f32, 0.0, 0.0, 1.0];
        let c = blend(BlendMode::Color, below, red, 0.5);
        assert!(close(lum([c[0], c[1], c[2]]), lum([0.2, 0.4, 0.6])), "{c:?}");
        assert!(c[0] > c[1] && c[0] > c[2], "and take the hue: {c:?}");
        let l = blend(BlendMode::Luminosity, below, solid(0.9), 0.5);
        assert!(close(lum([l[0], l[1], l[2]]), 0.9));
        let grey = blend(BlendMode::Saturation, below, solid(0.5), 0.5);
        assert!(close(grey[0], grey[1]) && close(grey[1], grey[2]), "no saturation makes grey: {grey:?}");
    }
}
