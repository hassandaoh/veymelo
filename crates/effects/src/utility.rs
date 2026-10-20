//! Utility: room for effects to draw past the layer's edges (Grow Bounds), what lies past white
//! or below black shown (Overbrights), bright colours brought under white (HDR Highlight
//! Compression) or squeezed and let out again round 8- and 16-bit effects (HDR Compander), and
//! film scans' logarithmic code values turned to linear and back (Cineon Converter).
//!
//! Apply Color LUT lives with the looks (look.rs).

use crate::defs::{choice, color, effect, pct, px, ranged};
use crate::util::{luma, map_px, map_straight, premul, unpremul};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "utility";

effect!(
    GROW_BOUNDS_DEF,
    "growBounds",
    "Grow Bounds",
    "more room round the layer, so the effects after it can draw past its edges",
    [px("pixels", "Pixels", 0.0, 4000.0).doc("added on every side, in layer pixels")]
);
pub const GROW_BOUNDS: Spec = Spec::new(&GROW_BOUNDS_DEF, CATEGORY, grow_bounds);

fn grow_bounds(ctx: &Ctx, mut b: Buf) -> Buf {
    // under an adjustment layer the picture is already the whole composition
    if !ctx.adjustment {
        let n = (ctx.num("pixels", 0.0).max(0.0) * b.scale).ceil() as usize;
        let room = 16_384usize.saturating_sub(b.img.width.max(b.img.height)) / 2;
        b.pad(n.min(room));
    }
    b
}

effect!(
    OVERBRIGHTS_DEF,
    "overbrights",
    "Overbrights",
    "where the colours go past white (in the highlight colour) or below black (in its opposite); the rest dim grey",
    [choice("channel", "Channel", &["maxRgb", "red", "green", "blue", "alpha", "luminance"], 0), color("highlightColor", "Highlight Color", 1.0, 0.0, 0.0),]
);
pub const OVERBRIGHTS: Spec = Spec::new(&OVERBRIGHTS_DEF, CATEGORY, overbrights);

fn overbrights(ctx: &Ctx, mut b: Buf) -> Buf {
    let ch = ctx.choice("channel");
    let hc = ctx.color("highlightColor");
    let hc = [hc.r as f32, hc.g as f32, hc.b as f32];
    map_px(&mut b.img, |_, _, p| {
        let (c, a) = unpremul(p);
        if a <= 0.0 {
            return p;
        }
        let v = match ch {
            1 => c[0],
            2 => c[1],
            3 => c[2],
            4 => a,
            5 => luma(c),
            _ => c[0].max(c[1]).max(c[2]),
        };
        let o = if v > 1.0 {
            hc
        } else if v < 0.0 {
            hc.map(|k| 1.0 - k)
        } else {
            [v * 0.35; 3]
        };
        premul(o, a)
    });
    b
}

effect!(
    HDR_COMPRESSION_DEF,
    "hdrHighlightCompression",
    "HDR Highlight Compression",
    "colours brighter than white brought smoothly under it, hues kept",
    [pct("amount", "Amount", 100.0, 0.0, 100.0)]
);
pub const HDR_COMPRESSION: Spec = Spec::new(&HDR_COMPRESSION_DEF, CATEGORY, hdr_compression);

/// `m` above `knee` bent smoothly toward 1, never reaching it.
pub fn soft_ceiling(m: f32, knee: f32) -> f32 {
    if m <= knee {
        return m;
    }
    let w = 1.0 - knee;
    knee + w * (m - knee) / ((m - knee) + w)
}

fn hdr_compression(ctx: &Ctx, mut b: Buf) -> Buf {
    let t = (ctx.num("amount", 100.0) / 100.0).clamp(0.0, 1.0) as f32;
    if t <= 0.0 {
        return b;
    }
    // the more, the lower the bend starts (from white at none to half at all)
    let knee = 1.0 - 0.5 * t;
    map_straight(&mut b.img, |c| {
        let m = c[0].max(c[1]).max(c[2]);
        if m <= knee {
            return c;
        }
        let k = soft_ceiling(m, knee) / m;
        c.map(|v| v * k)
    });
    b
}

effect!(
    HDR_COMPANDER_DEF,
    "hdrCompander",
    "HDR Compander",
    "bright colours squeezed into 0 to 1 for effects that cannot hold more, then let out again the same way",
    [
        choice("mode", "Mode", &["compressRange", "expandRange"], 0),
        ranged("gain", "Gain", 1.0, 1.0, 100.0).doc("the brightest value squeezed to 1"),
        ranged("gamma", "Gamma", 1.0, 0.1, 10.0),
    ]
);
pub const HDR_COMPANDER: Spec = Spec::new(&HDR_COMPANDER_DEF, CATEGORY, hdr_compander);

fn hdr_compander(ctx: &Ctx, mut b: Buf) -> Buf {
    let expand = ctx.choice("mode") == 1;
    let gain = ctx.num("gain", 1.0).max(1e-6) as f32;
    let gamma = ctx.num("gamma", 1.0).max(1e-3) as f32;
    // a power that keeps the sign, so values below black come back too
    let pow = |v: f32, e: f32| v.signum() * v.abs().powf(e);
    map_straight(&mut b.img, |c| c.map(|v| if expand { pow(v, gamma) * gain } else { pow(v / gain, 1.0 / gamma) }));
    b
}

/// The Kodak Cineon encoding of film density: 0.002 density a code value, a negative gamma of
/// 0.6, black and white at code values `black` and `white` (of 1023), shown from `low` to
/// `high`, bent by `gamma` (1.7: none), the brightest `rolloff` per cent rolled off softly.
#[derive(Clone, Copy, Debug)]
pub struct Cineon {
    pub black: f32,
    pub white: f32,
    pub low: f32,
    pub high: f32,
    pub gamma: f32,
    pub rolloff: f32,
}

impl Cineon {
    /// The usual: black at 95, white at 685.
    pub const STANDARD: Cineon = Cineon { black: 95.0, white: 685.0, low: 0.0, high: 1.0, gamma: 1.7, rolloff: 0.0 };

    /// Black's light as a part of white's.
    fn floor(&self) -> f32 {
        10f32.powf((self.black - self.white) * 0.002 / 0.6)
    }

    fn knee(&self) -> f32 {
        1.0 - (self.rolloff / 100.0).clamp(0.0, 0.99)
    }

    /// A code value (as a part of 1023) as linear.
    pub fn to_linear(&self, v: f32) -> f32 {
        let f = self.floor();
        let mut lin = (10f32.powf((v * 1023.0 - self.white) * 0.002 / 0.6) - f) / (1.0 - f).max(1e-6);
        lin = lin.max(0.0).powf(1.7 / self.gamma.max(0.01));
        let k = self.knee();
        if k < 1.0 && lin > k {
            let w = 1.0 - k;
            lin = k + w * (1.0 - (-(lin - k) / w).exp());
        }
        self.low + (self.high - self.low) * lin
    }

    /// Linear as a code value (a part of 1023): `to_linear` undone.
    pub fn to_log(&self, v: f32) -> f32 {
        let f = self.floor();
        let span = self.high - self.low;
        let mut lin = (v - self.low) / if span.abs() < 1e-6 { 1e-6 } else { span };
        let k = self.knee();
        if k < 1.0 && lin > k {
            let w = 1.0 - k;
            lin = k - w * (1.0 - (lin - k) / w).max(1e-6).ln();
        }
        lin = lin.max(0.0).powf(self.gamma.max(0.01) / 1.7);
        (self.white + (lin * (1.0 - f) + f).max(1e-9).log10() * 0.6 / 0.002) / 1023.0
    }
}

effect!(
    CINEON_DEF,
    "cineonConverter",
    "Cineon Converter",
    "film scans' logarithmic (Cineon) code values made linear, linear made logarithmic, or one log setting made the standard one",
    [
        choice("conversionType", "Conversion Type", &["linearToLog", "logToLinear", "logToLog"], 1),
        ranged("tenBitBlackPoint", "10 Bit Black Point", 95.0, 0.0, 1023.0),
        ranged("internalBlackPoint", "Internal Black Point", 0.0, -1.0, 2.0),
        ranged("tenBitWhitePoint", "10 Bit White Point", 685.0, 0.0, 1023.0),
        ranged("internalWhitePoint", "Internal White Point", 1.0, -1.0, 2.0),
        ranged("gamma", "Gamma", 1.7, 0.1, 5.0),
        pct("highlightRolloff", "Highlight Rolloff", 20.0, 0.0, 100.0),
    ]
);
pub const CINEON: Spec = Spec::new(&CINEON_DEF, CATEGORY, cineon);

fn cineon(ctx: &Ctx, mut b: Buf) -> Buf {
    let c = Cineon {
        black: ctx.num("tenBitBlackPoint", 95.0) as f32,
        white: ctx.num("tenBitWhitePoint", 685.0) as f32,
        low: ctx.num("internalBlackPoint", 0.0) as f32,
        high: ctx.num("internalWhitePoint", 1.0) as f32,
        gamma: ctx.num("gamma", 1.7) as f32,
        rolloff: ctx.num("highlightRolloff", 20.0) as f32,
    };
    let standard = Cineon { rolloff: c.rolloff, ..Cineon::STANDARD };
    let kind = ctx.choice("conversionType");
    map_straight(&mut b.img, |v| {
        v.map(|x| match kind {
            0 => c.to_log(x),
            1 => c.to_linear(x),
            _ => standard.to_log(c.to_linear(x)),
        })
    });
    b
}

/// Every effect here.
pub const ALL: [Spec; 5] = [GROW_BOUNDS, OVERBRIGHTS, HDR_COMPRESSION, HDR_COMPANDER, CINEON];

#[cfg(test)]
mod tests;
