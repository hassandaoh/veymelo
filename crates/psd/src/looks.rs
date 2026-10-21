//! What a layer looks like beyond its pixels: its layer effects (`lfx2`, or `lmfx` where a kind
//! may be there more than once), each a descriptor, and a fill layer's fill (`SoCo` a solid
//! colour, `GdFl` a gradient, `PtFl` a pattern), and a shape's vector stroke (`vstk`).

use crate::Psd;
use crate::descriptor::{self, Desc, Item};

/// A layer's effects: whether they are shown at all, the scale they were made for (percent),
/// and each one (its kind and settings), in the order Photoshop draws them.
#[derive(Clone, Debug, PartialEq)]
pub struct Effects {
    pub shown: bool,
    pub scale: f64,
    pub list: Vec<(&'static str, Desc)>,
}

/// The kinds of effect: our name, the key a single one is kept under, and the key of a list of
/// them (in `lmfx`).
const KINDS: [(&str, &str, &str); 10] = [
    ("dropShadow", "DrSh", "dropShadowMulti"),
    ("innerShadow", "IrSh", "innerShadowMulti"),
    ("outerGlow", "OrGl", ""),
    ("innerGlow", "IrGl", ""),
    ("bevelEmboss", "ebbl", ""),
    ("satin", "ChFX", ""),
    ("colorOverlay", "SoFi", "solidFillMulti"),
    ("gradientOverlay", "GrFl", "gradientFillMulti"),
    ("patternOverlay", "patternFill", ""),
    ("stroke", "FrFX", "frameFXMulti"),
];

/// A fill layer's fill.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    /// A colour (straight sRGB, 0 to 1).
    Solid([f64; 3]),
    /// A gradient fill's settings (`Grad`, `Angl`, `Type`, `Rvrs`, `Algn`, `Scl `, `Ofst`…).
    Gradient(Desc),
    /// A pattern fill's settings.
    Pattern(Desc),
}

/// A descriptor block: its version (4 bytes, 16), then the descriptor.
fn versioned(d: &[u8]) -> Option<Desc> {
    descriptor::read(d, true).ok()
}

impl Psd {
    /// Layer `i`'s effects (none when it has none).
    pub fn effects(&self, i: usize) -> Option<Effects> {
        // `lmfx` holds what `lfx2` does and more (several of a kind); either starts with the
        // effects' version (4 bytes) before the descriptor's
        let d = self.block(i, b"lmfx").or_else(|| self.block(i, b"lfx2"))?;
        let desc = descriptor::read(d.get(4..)?, true).ok()?;
        let mut list = Vec::new();
        for (name, single, multi) in KINDS {
            let many = (!multi.is_empty()).then(|| desc.list(multi)).flatten();
            match many {
                Some(items) => list.extend(items.iter().filter_map(|it| if let Item::Desc(e) = it { Some((name, e.clone())) } else { None })),
                None => {
                    if let Some(e) = desc.desc(single) {
                        list.push((name, e.clone()));
                    }
                }
            }
        }
        list.retain(|(_, e)| e.flag("present") != Some(false));
        (!list.is_empty()).then(|| Effects { shown: desc.flag("masterFXSwitch").unwrap_or(true), scale: desc.num("Scl ").unwrap_or(100.0), list })
    }

    /// Layer `i`'s fill, where it is a fill layer.
    pub fn fill(&self, i: usize) -> Option<Fill> {
        if let Some(d) = self.block(i, b"SoCo") {
            return versioned(d).and_then(|d| d.color("Clr ")).map(Fill::Solid);
        }
        if let Some(d) = self.block(i, b"GdFl") {
            return versioned(d).map(Fill::Gradient);
        }
        self.block(i, b"PtFl").and_then(versioned).map(Fill::Pattern)
    }

    /// Layer `i`'s vector stroke and fill switches (a shape layer's `vstk`): its descriptor.
    pub fn vector_stroke(&self, i: usize) -> Option<Desc> {
        self.block(i, b"vstk").and_then(versioned)
    }
}

/// A gradient's stops: each colour's place (0 to 1) and colour, each opacity's place and
/// opacity, and how smooth it is (0 to 1).
pub type Stops = (Vec<(f64, [f64; 3])>, Vec<(f64, f64)>, f64);

/// A gradient descriptor (`Grdn`) as its stops.
pub fn gradient_stops(g: &Desc) -> Stops {
    let place = |d: &Desc| d.num("Lctn").unwrap_or(0.0) / 4096.0;
    let colors: Vec<(f64, [f64; 3])> =
        g.list("Clrs").unwrap_or(&[]).iter().filter_map(|it| if let Item::Desc(d) = it { Some((place(d), d.color("Clr ").unwrap_or([0.0; 3]))) } else { None }).collect();
    let opacities: Vec<(f64, f64)> =
        g.list("Trns").unwrap_or(&[]).iter().filter_map(|it| if let Item::Desc(d) = it { Some((place(d), d.num("Opct").unwrap_or(100.0) / 100.0)) } else { None }).collect();
    (colors, opacities, g.num("Intr").unwrap_or(4096.0) / 4096.0)
}
