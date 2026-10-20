//! Lottie: the JSON animation format players on the web, on phones and in apps draw. A
//! composition goes out as a Lottie document and a Lottie document comes in as a composition.
//!
//! **Out** ([`export`]): compositions inside it become precomp assets, image footage becomes
//! image assets (in the file or beside it), sound footage sound assets (in the file), and shape,
//! solid, null, composition, image, sound, text and camera layers become Lottie's own kinds (text as text, which needs its fonts where it
//! plays, or as the outlines of its letters). Transforms with parents, separate dimensions and
//! anchors, keyframes (speed and influence as Lottie's ease curves, holds, the curves positions
//! move on), shape contents, masks, track mattes, blend modes, time remapping, layer timing,
//! the effects Lottie players draw and markers all go along. What Lottie cannot hold (lights,
//! layer styles, most effects, video…) is said in the warnings.
//!
//! **In** ([`import`]): a Lottie document (`.json`, or a `.lottie` archive) as a new
//! composition, its precomps, images and sounds as items in a folder of their own.
//!
//! Lottie counts time in frames: a key's time is a frame of its layer's own time, a layer's
//! in, out and start are frames of its composition, `sr` is its stretch over 100.
//!
//! Written from the Lottie format's public description (the lottie-spec schema and the
//! LottieFiles documentation).

mod base64;
mod export;
mod import;
mod shapes;
mod text;
mod values;

use veymelo_color::BlendMode;

pub use export::{Exported, Options, dotlottie, export};
pub use import::{Imported, Picture, document, import};

/// Lottie's blend modes (`bm`), in its order.
const BLENDS: [BlendMode; 18] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::HardLight,
    BlendMode::SoftLight,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
    BlendMode::Add,
    BlendMode::HardMix,
];

/// Lottie's `bm` for a blend mode (none: Lottie has nothing like it).
fn blend_out(m: BlendMode) -> Option<u32> {
    BLENDS.iter().position(|b| *b == m).map(|i| i as u32).or(match m {
        BlendMode::LinearDodge => Some(16),
        BlendMode::ClassicColorDodge => Some(6),
        BlendMode::ClassicColorBurn => Some(7),
        BlendMode::ClassicDifference => Some(10),
        _ => None,
    })
}

fn blend_in(bm: f64) -> BlendMode {
    BLENDS.get(bm.max(0.0) as usize).copied().unwrap_or(BlendMode::Normal)
}

/// Lottie's mask modes, in our order (none, add, subtract, intersect, lighten, darken,
/// difference).
const MASK_MODES: [&str; 7] = ["n", "a", "s", "i", "l", "d", "f"];

/// One setting of an effect Lottie players draw: our property, its Lottie kind (0 slider,
/// 1 angle, 2 colour, 3 point, 4 checkbox, 7 menu, 10 layer) and what ours is multiplied by to
/// be Lottie's.
struct Param {
    id: &'static str,
    ty: u8,
    scale: f64,
}

const fn p(id: &'static str, ty: u8) -> Param {
    Param { id, ty, scale: 1.0 }
}

const fn scaled(id: &'static str, ty: u8, scale: f64) -> Param {
    Param { id, ty, scale }
}

/// An effect Lottie players draw: our effect, its Lottie type and its settings in Lottie's order.
struct Fx {
    id: &'static str,
    ty: u8,
    params: &'static [Param],
}

const EFFECTS: &[Fx] = &[
    Fx { id: "gaussianBlur", ty: 29, params: &[p("blurriness", 0), p("dimensions", 7), p("repeatEdge", 4)] },
    Fx { id: "tint", ty: 20, params: &[p("mapBlackTo", 2), p("mapWhiteTo", 2), p("amountToTint", 0)] },
    Fx { id: "tritone", ty: 23, params: &[p("highlights", 2), p("midtones", 2), p("shadows", 2), p("blendWithOriginal", 0)] },
    Fx { id: "dropShadow", ty: 25, params: &[p("shadowColor", 2), scaled("opacity", 0, 2.55), p("direction", 1), p("distance", 0), p("softness", 0), p("shadowOnly", 4)] },
    Fx {
        id: "fill",
        ty: 21,
        params: &[p("fillMask", 7), p("allMasks", 4), p("color", 2), p("invert", 4), p("horizontalFeather", 0), p("verticalFeather", 0), scaled("opacity", 0, 0.01)],
    },
    // expression controls: Lottie's one kind of effect holding one value of a kind
    Fx { id: "sliderControl", ty: 5, params: &[p("slider", 0)] },
    Fx { id: "angleControl", ty: 5, params: &[p("angle", 1)] },
    Fx { id: "colorControl", ty: 5, params: &[p("color", 2)] },
    Fx { id: "pointControl", ty: 5, params: &[p("point", 3)] },
    Fx { id: "point3dControl", ty: 5, params: &[p("point", 3)] },
    Fx { id: "checkboxControl", ty: 5, params: &[p("checkbox", 4)] },
    Fx { id: "menuControl", ty: 5, params: &[p("menu", 7)] },
    Fx { id: "layerControl", ty: 5, params: &[p("layer", 10)] },
];

fn fx_out(id: &str) -> Option<&'static Fx> {
    EFFECTS.iter().find(|f| f.id == id)
}

/// The effect a Lottie effect is: by our id (`mn`, when we wrote it), else by its type (and
/// for a control, its value's kind and how many numbers a point has).
fn fx_in(mn: Option<&str>, ty: u64, inner: Option<u64>, dims: usize) -> Option<&'static Fx> {
    if let Some(f) = mn.and_then(fx_out) {
        return Some(f);
    }
    if ty == 5 {
        let inner = inner? as u8;
        if inner == 3 && dims == 3 {
            return fx_out("point3dControl");
        }
        return EFFECTS.iter().find(|f| f.ty == 5 && f.params[0].ty == inner);
    }
    EFFECTS.iter().find(|f| u64::from(f.ty) == ty)
}

#[cfg(test)]
mod tests;
