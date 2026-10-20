//! Layer styles: the looks a layer can take around and over its shape — Drop Shadow, Inner
//! Shadow, Outer Glow, Inner Glow, Bevel and Emboss, Satin, Color Overlay, Gradient Overlay and
//! Stroke — with Blending Options (the light they share, how much the layer's own fill shows,
//! which channels it draws, knockout).
//!
//! They sit in the layer's `layerStyles` list after its transform, Blending Options first and the
//! styles in this order however they were added; each is switched on and off as effects are.
//! Their sizes and distances are in the layer's own pixels.
//!
//! The light (Global Light) is one for a composition: every layer's Blending Options holds it,
//! and [`share_global_light`] keeps them alike after any change.

use std::sync::Arc;

use veymelo_color::BlendMode;
use veymelo_keyframe::{Gradient, Kind, Value};

use crate::{Body, DefNode, GroupDef, ItemKind, Layer, PropDef, Project, Unit};

/// The list's id in a layer.
pub const LIST: &str = "layerStyles";
/// Blending Options' id.
pub const BLENDING: &str = "blendingOptions";

/// The modes a style mixes by (the order paint programs list them).
pub const MODES: &[&str] = &[
    "normal", "dissolve", "darken", "multiply", "colorBurn", "linearBurn", "darkerColor", "lighten", "screen", "colorDodge", "linearDodge", "lighterColor", "overlay", "softLight", "hardLight", "vividLight", "linearLight", "pinLight", "hardMix", "difference", "exclusion", "subtract", "divide", "hue", "saturation", "color", "luminosity",
];

/// The blend mode a style's mode setting stands for.
pub fn mode_of(choice: u32) -> BlendMode {
    MODES.get(choice as usize).and_then(|m| BlendMode::from_id(m)).unwrap_or(BlendMode::Normal)
}

/// The light's angle and height by default.
pub const ANGLE: f64 = 120.0;
pub const ALTITUDE: f64 = 30.0;

const fn num(id: &'static str, name: &'static str, v: f64) -> PropDef {
    PropDef::new(id, name, Kind::Number, Value::Number(v))
}

const fn pct(id: &'static str, name: &'static str, v: f64) -> PropDef {
    num(id, name, v).range(0.0, 100.0).unit(Unit::Percent)
}

const fn px(id: &'static str, name: &'static str, v: f64, most: f64) -> PropDef {
    num(id, name, v).range(0.0, most).unit(Unit::Pixels)
}

const fn angle(id: &'static str, name: &'static str, v: f64) -> PropDef {
    num(id, name, v).unit(Unit::Degrees)
}

const fn flag(id: &'static str, name: &'static str, v: bool) -> PropDef {
    PropDef::new(id, name, Kind::Bool, Value::Bool(v))
}

const fn choice(id: &'static str, name: &'static str, options: &'static [&'static str], v: u32) -> PropDef {
    PropDef::new(id, name, Kind::Choice(options), Value::Choice(v))
}

const fn color(id: &'static str, name: &'static str, r: f64, g: f64, b: f64) -> PropDef {
    PropDef::new(id, name, Kind::Color, Value::Color(veymelo_color::rgba(r, g, b, 1.0)))
}

/// A mode setting, `m` its default.
const fn mode(id: &'static str, name: &'static str, m: usize) -> PropDef {
    choice(id, name, MODES, m as u32)
}

const NORMAL: usize = 0;
const MULTIPLY: usize = 3;
const SCREEN: usize = 8;

/// A gradient setting (empty: the style's own default).
const fn colors(doc: &'static str) -> PropDef {
    PropDef::new("colors", "Colors", Kind::Gradient, Value::Gradient(Gradient { colors: Vec::new(), opacities: Vec::new() })).doc(doc)
}

const fn p(def: PropDef) -> DefNode {
    DefNode::Prop(def)
}

pub const KNOCKOUT: &[&str] = &["none", "shallow", "deep"];
pub const COLOR_TYPES: &[&str] = &["color", "gradient"];
pub const GLOW_TECHNIQUES: &[&str] = &["softer", "precise"];
pub const GLOW_SOURCES: &[&str] = &["center", "edge"];
pub const BEVEL_STYLES: &[&str] = &["outerBevel", "innerBevel", "emboss", "pillowEmboss", "strokeEmboss"];
pub const BEVEL_TECHNIQUES: &[&str] = &["smooth", "chiselHard", "chiselSoft"];
pub const BEVEL_DIRECTIONS: &[&str] = &["up", "down"];
pub const GRADIENT_STYLES: &[&str] = &["linear", "radial", "angle", "reflected", "diamond"];
pub const STROKE_POSITIONS: &[&str] = &["outside", "inside", "center"];

pub static LAYER_STYLES: GroupDef = GroupDef { id: LIST, name: "Layer Styles", body: Body::List(LIST), doc: "looks around and over the layer's shape: shadows, glows, bevel, satin, overlays, a stroke" };

static ADVANCED: GroupDef = GroupDef {
    id: "advancedBlending",
    name: "Advanced Blending",
    body: Body::Fixed(&[
        p(pct("fillOpacity", "Fill Opacity", 100.0).doc("how much of the layer itself shows, its styles staying (Opacity fades both)")),
        p(flag("red", "Red", true).doc("the layer draws into red")),
        p(flag("green", "Green", true)),
        p(flag("blue", "Blue", true)),
        p(choice("knockout", "Knockout", KNOCKOUT, 0).doc("the layer's fill cutting through what is under it (shallow: in its own composition; deep: to the background)")),
        p(flag("blendInteriorAsGroup", "Blend Interior Styles as Group", false).doc("the inner styles faded with the fill")),
        p(flag("useBlendRanges", "Use Blend Ranges from Source", true)),
    ]),
    doc: "",
};

pub static BLENDING_OPTIONS: GroupDef = GroupDef {
    id: BLENDING,
    name: "Blending Options",
    body: Body::Fixed(&[
        p(angle("globalLightAngle", "Global Light Angle", ANGLE).doc("where the light the styles share comes from, the same for every layer of the composition")),
        p(angle("globalLightAltitude", "Global Light Altitude", ALTITUDE).range(0.0, 90.0).doc("how high it is")),
        DefNode::Group(&ADVANCED),
    ]),
    doc: "the light the styles share, and how the layer itself is drawn",
};

pub static DROP_SHADOW: GroupDef = GroupDef {
    id: "dropShadow",
    name: "Drop Shadow",
    body: Body::Fixed(&[
        p(mode("blendMode", "Blend Mode", MULTIPLY)),
        p(color("color", "Color", 0.0, 0.0, 0.0)),
        p(pct("opacity", "Opacity", 75.0)),
        p(flag("useGlobalLight", "Use Global Light", true)),
        p(angle("angle", "Angle", ANGLE).doc("where the light comes from (the shadow falls the other way)")),
        p(num("distance", "Distance", 5.0).range(0.0, 30_000.0).unit(Unit::Pixels)),
        p(pct("spread", "Spread", 0.0).doc("how much of its size is solid before it softens")),
        p(px("size", "Size", 5.0, 250.0)),
        p(pct("noise", "Noise", 0.0)),
        p(flag("knocksOut", "Layer Knocks Out Drop Shadow", true).doc("no shadow under a see-through layer")),
    ]),
    doc: "a shadow under the layer, away from the light",
};

pub static INNER_SHADOW: GroupDef = GroupDef {
    id: "innerShadow",
    name: "Inner Shadow",
    body: Body::Fixed(&[
        p(mode("blendMode", "Blend Mode", MULTIPLY)),
        p(color("color", "Color", 0.0, 0.0, 0.0)),
        p(pct("opacity", "Opacity", 75.0)),
        p(flag("useGlobalLight", "Use Global Light", true)),
        p(angle("angle", "Angle", ANGLE)),
        p(num("distance", "Distance", 5.0).range(0.0, 30_000.0).unit(Unit::Pixels)),
        p(pct("choke", "Choke", 0.0)),
        p(px("size", "Size", 5.0, 250.0)),
        p(pct("noise", "Noise", 0.0)),
    ]),
    doc: "a shadow inside the layer's edge, as if it were cut into what is under it",
};

pub static OUTER_GLOW: GroupDef = GroupDef {
    id: "outerGlow",
    name: "Outer Glow",
    body: Body::Fixed(&[
        p(mode("blendMode", "Blend Mode", SCREEN)),
        p(pct("opacity", "Opacity", 75.0)),
        p(pct("noise", "Noise", 0.0)),
        p(choice("colorType", "Color Type", COLOR_TYPES, 0)),
        p(color("color", "Color", 1.0, 1.0, 190.0 / 255.0)),
        p(colors("from the edge out (default: the colour fading away)")),
        p(pct("gradientSmoothness", "Gradient Smoothness", 100.0)),
        p(choice("technique", "Technique", GLOW_TECHNIQUES, 0).doc("softer: blurred; precise: following the shape's distance")),
        p(pct("spread", "Spread", 0.0)),
        p(px("size", "Size", 5.0, 250.0)),
        p(num("range", "Range", 50.0).range(1.0, 100.0).unit(Unit::Percent)),
        p(pct("jitter", "Jitter", 0.0)),
    ]),
    doc: "light glowing out from the layer's edge",
};

pub static INNER_GLOW: GroupDef = GroupDef {
    id: "innerGlow",
    name: "Inner Glow",
    body: Body::Fixed(&[
        p(mode("blendMode", "Blend Mode", SCREEN)),
        p(pct("opacity", "Opacity", 75.0)),
        p(pct("noise", "Noise", 0.0)),
        p(choice("colorType", "Color Type", COLOR_TYPES, 0)),
        p(color("color", "Color", 1.0, 1.0, 190.0 / 255.0)),
        p(colors("from the edge in (default: the colour fading away)")),
        p(pct("gradientSmoothness", "Gradient Smoothness", 100.0)),
        p(choice("technique", "Technique", GLOW_TECHNIQUES, 0)),
        p(choice("source", "Source", GLOW_SOURCES, 1).doc("glowing in from the edge, or out from the middle")),
        p(pct("choke", "Choke", 0.0)),
        p(px("size", "Size", 5.0, 250.0)),
        p(num("range", "Range", 50.0).range(1.0, 100.0).unit(Unit::Percent)),
        p(pct("jitter", "Jitter", 0.0)),
    ]),
    doc: "light glowing inside the layer, from its edge or its middle",
};

pub static BEVEL_EMBOSS: GroupDef = GroupDef {
    id: "bevelEmboss",
    name: "Bevel and Emboss",
    body: Body::Fixed(&[
        p(choice("style", "Style", BEVEL_STYLES, 1)),
        p(choice("technique", "Technique", BEVEL_TECHNIQUES, 0)),
        p(num("depth", "Depth", 100.0).range(1.0, 1000.0).unit(Unit::Percent)),
        p(choice("direction", "Direction", BEVEL_DIRECTIONS, 0)),
        p(px("size", "Size", 5.0, 250.0)),
        p(px("soften", "Soften", 0.0, 16.0)),
        p(flag("useGlobalLight", "Use Global Light", true)),
        p(angle("angle", "Angle", ANGLE)),
        p(angle("altitude", "Altitude", ALTITUDE).range(0.0, 90.0)),
        p(mode("highlightMode", "Highlight Mode", SCREEN)),
        p(color("highlightColor", "Highlight Color", 1.0, 1.0, 1.0)),
        p(pct("highlightOpacity", "Highlight Opacity", 75.0)),
        p(mode("shadowMode", "Shadow Mode", MULTIPLY)),
        p(color("shadowColor", "Shadow Color", 0.0, 0.0, 0.0)),
        p(pct("shadowOpacity", "Shadow Opacity", 75.0)),
    ]),
    doc: "the layer's edge raised (or sunk) and lit",
};

pub static SATIN: GroupDef = GroupDef {
    id: "satin",
    name: "Satin",
    body: Body::Fixed(&[
        p(mode("blendMode", "Blend Mode", MULTIPLY)),
        p(color("color", "Color", 0.0, 0.0, 0.0)),
        p(pct("opacity", "Opacity", 50.0)),
        p(angle("angle", "Angle", 19.0)),
        p(px("distance", "Distance", 11.0, 250.0)),
        p(px("size", "Size", 14.0, 250.0)),
        p(flag("invert", "Invert", true)),
    ]),
    doc: "a soft sheen inside the layer, from its shape laid over itself",
};

pub static COLOR_OVERLAY: GroupDef = GroupDef {
    id: "colorOverlay",
    name: "Color Overlay",
    body: Body::Fixed(&[p(mode("blendMode", "Blend Mode", NORMAL)), p(color("color", "Color", 1.0, 0.0, 0.0)), p(pct("opacity", "Opacity", 100.0))]),
    doc: "the layer filled with one colour",
};

pub static GRADIENT_OVERLAY: GroupDef = GroupDef {
    id: "gradientOverlay",
    name: "Gradient Overlay",
    body: Body::Fixed(&[
        p(mode("blendMode", "Blend Mode", NORMAL)),
        p(pct("opacity", "Opacity", 100.0)),
        p(colors("its colours (default black to white)")),
        p(pct("gradientSmoothness", "Gradient Smoothness", 100.0)),
        p(angle("angle", "Angle", 90.0)),
        p(choice("style", "Style", GRADIENT_STYLES, 0)),
        p(flag("reverse", "Reverse", false)),
        p(flag("alignWithLayer", "Align with Layer", true).doc("spread over the layer's shape (otherwise its whole rectangle)")),
        p(num("scale", "Scale", 100.0).range(10.0, 150.0).unit(Unit::Percent)),
        p(PropDef::new("offset", "Offset", Kind::Vec2, Value::Vec2([0.0; 2])).unit(Unit::Pixels)),
    ]),
    doc: "the layer filled with a gradient",
};

pub static STROKE: GroupDef = GroupDef {
    id: "stroke",
    name: "Stroke",
    body: Body::Fixed(&[
        p(color("color", "Color", 1.0, 0.0, 0.0)),
        p(mode("blendMode", "Blend Mode", NORMAL)),
        p(num("size", "Size", 3.0).range(1.0, 250.0).unit(Unit::Pixels)),
        p(pct("opacity", "Opacity", 100.0)),
        p(choice("position", "Position", STROKE_POSITIONS, 0)),
    ]),
    doc: "a line round the layer's edge",
};

/// The styles in their order (after Blending Options).
pub static STYLES: [&GroupDef; 9] = [&DROP_SHADOW, &INNER_SHADOW, &OUTER_GLOW, &INNER_GLOW, &BEVEL_EMBOSS, &SATIN, &COLOR_OVERLAY, &GRADIENT_OVERLAY, &STROKE];

/// A style by its id or name (any case).
pub fn find(s: &str) -> Option<&'static GroupDef> {
    STYLES.iter().copied().find(|d| d.id.eq_ignore_ascii_case(s) || d.name.eq_ignore_ascii_case(s))
}

/// Where in the list an instance of `id` goes: Blending Options first, then the styles in
/// their order.
pub fn rank(id: &str) -> usize {
    if id == BLENDING { 0 } else { STYLES.iter().position(|d| d.id == id).map_or(usize::MAX, |i| i + 1) }
}

/// Whether a layer can have styles (one with pixels of its own, not an adjustment layer).
pub fn can_have(layer: &Layer) -> bool {
    !layer.switches.adjustment && layer.props.group(LIST).is_ok()
}

/// The (angle, altitude) properties of a layer's Global Light, if it has Blending Options.
fn light_of(layer: &Layer) -> Option<[&crate::Property; 2]> {
    let bo = layer.props.group(LIST).ok()?.children.iter().filter_map(|c| c.as_group()).find(|g| g.def.id == BLENDING)?;
    Some([bo.prop("globalLightAngle").ok()?, bo.prop("globalLightAltitude").ok()?])
}

fn light_of_mut(layer: &mut Layer) -> Option<[&mut crate::Property; 2]> {
    let bo = layer.props.group_mut(LIST).ok()?.children.iter_mut().filter_map(|c| c.as_group_mut()).find(|g| g.def.id == BLENDING)?;
    let [a, b] = bo.children.get_disjoint_mut([0, 1]).ok()?;
    Some([a.as_prop_mut()?, b.as_prop_mut()?])
}

/// The Global Light of a composition's layers (the first that has one), as it is held.
pub fn comp_light(comp: &crate::Comp) -> Option<[crate::Property; 2]> {
    comp.layers.iter().find_map(|l| light_of(l).map(|[a, b]| [a.clone(), b.clone()]))
}

/// Give `layer`'s Blending Options the composition's Global Light.
pub fn take_light(layer: &mut Layer, light: &[crate::Property; 2]) {
    if let Some(mine) = light_of_mut(layer) {
        for (m, l) in mine.into_iter().zip(light) {
            (m.value, m.keys, m.expression) = (l.value.clone(), l.keys.clone(), l.expression.clone());
        }
    }
}

/// After a change: where a layer's Global Light was changed (from `before`), every layer of its
/// composition takes it; where none was but the layers differ, they take the first one's.
pub fn share_global_light(before: &Project, after: &mut Project) {
    let comps: Vec<u64> = after
        .items
        .iter()
        .filter(|it| match (&it.kind, before.item(it.id).map(|b| &b.kind)) {
            (ItemKind::Comp(a), Some(ItemKind::Comp(b))) => !Arc::ptr_eq(a, b),
            (ItemKind::Comp(_), _) => true,
            _ => false,
        })
        .map(|it| it.id)
        .collect();
    for id in comps {
        let Some(comp) = after.comp(id) else { continue };
        let lights: Vec<(u64, [crate::Property; 2])> = comp.layers.iter().filter_map(|l| light_of(l).map(|[a, b]| (l.id, [a.clone(), b.clone()]))).collect();
        if lights.len() < 2 {
            continue;
        }
        let was = |layer: u64| before.comp(id).and_then(|c| c.layer(layer)).and_then(light_of).map(|[a, b]| [a.clone(), b.clone()]);
        // the light that was changed; or, where they differ, that of a layer that had one before
        // (a layer just given styles does not bring its own)
        let source = match lights.iter().find(|(l, light)| was(*l).is_some_and(|w| w != *light)) {
            Some((_, light)) => light.clone(),
            None if lights.windows(2).any(|w| w[0].1 != w[1].1) => lights.iter().find(|(l, _)| was(*l).is_some()).unwrap_or(&lights[0]).1.clone(),
            None => continue,
        };
        if let Some(comp) = after.comp_mut(id) {
            for layer in comp.layers.iter_mut() {
                take_light(layer, &source);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Group, Source};
    use veymelo_time::{FrameRate, Tick};

    #[test]
    fn every_style_is_whole() {
        for d in STYLES {
            let g = Group::new(d);
            assert!(!g.children.is_empty(), "{}", d.id);
            assert_eq!(find(d.name).map(|f| f.id), Some(d.id));
        }
        assert_eq!(mode_of(3), BlendMode::Multiply);
        assert_eq!(mode_of(10), BlendMode::LinearDodge);
        assert!(MODES.iter().all(|m| BlendMode::from_id(m).is_some()), "every mode is a blend mode");
        assert!(rank(BLENDING) < rank("dropShadow") && rank("bevelEmboss") < rank("stroke"));
    }

    fn with_styles(id: u64) -> Layer {
        let mut l = Layer::new(id, "L", Source::Solid { color: veymelo_color::Rgba::WHITE, width: 10, height: 10 }, Tick::from_seconds(1.0));
        let list = l.props.group_mut(LIST).unwrap();
        list.children.push(crate::Node::Group(Group::instance(&BLENDING_OPTIONS, id * 10, "Blending Options")));
        l
    }

    #[test]
    fn the_light_is_shared() {
        let mut p = Project::default();
        let mut comp = crate::Comp::new(100, 100, FrameRate::default(), Tick::from_seconds(1.0));
        comp.layers = vec![with_styles(1), with_styles(2)];
        p.items.push(crate::Item { id: 9, name: "C".into(), label: crate::Label::None, comment: String::new(), folder: None, kind: ItemKind::Comp(Arc::new(comp)), proxy: None });
        let before = p.clone();
        p.comp_mut(9).unwrap().layers[1].props.prop_mut("layerStyles/blendingOptions/globalLightAngle").unwrap().value = Value::Number(45.0);
        share_global_light(&before, &mut p);
        let first = p.comp(9).unwrap().layers[0].props.prop("layerStyles/blendingOptions/globalLightAngle").unwrap().value.clone();
        assert_eq!(first, Value::Number(45.0));
    }
}
