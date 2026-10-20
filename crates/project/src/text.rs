//! What a text layer is made of: its document (the words and their look), and animators that
//! change some of the characters, words or lines (those their selectors pick), more or less.

use veymelo_keyframe::{Kind, Value};

use crate::defs::{Body, DefNode, GroupDef, PropDef, Unit};

pub const GROUPINGS: &[&str] = &["character", "word", "line", "all"];
pub const BASED_ON: &[&str] = &["characters", "charactersExcludingSpaces", "words", "lines"];
pub const SELECTOR_MODES: &[&str] = &["add", "subtract", "intersect", "min", "max", "difference"];
pub const SHAPES: &[&str] = &["square", "rampUp", "rampDown", "triangle", "round", "smooth"];
pub const UNITS: &[&str] = &["percentage", "index"];
pub const TRACKING_TYPES: &[&str] = &["beforeAndAfter", "before", "after"];
pub const CHAR_ALIGNMENT: &[&str] = &["left", "center", "right", "adjustKerning"];
pub const CHAR_RANGE: &[&str] = &["preserveCaseAndDigits", "fullUnicode"];
pub const FILL_STROKE: &[&str] = &["perCharacterPalette", "allFillsOverAllStrokes", "allStrokesOverAllFills"];
/// How characters are put over each other.
pub const INTER_CHAR_BLEND: &[&str] = &[
    "normal", "darken", "multiply", "colorBurn", "linearBurn", "add", "lighten", "screen", "colorDodge", "linearDodge", "overlay", "softLight", "hardLight", "linearLight", "vividLight", "pinLight", "hardMix", "difference", "exclusion", "hue",
    "saturation", "color", "luminosity",
];

const fn num(id: &'static str, name: &'static str, v: f64) -> PropDef {
    PropDef::new(id, name, Kind::Number, Value::Number(v))
}

const fn choice(id: &'static str, name: &'static str, options: &'static [&'static str], v: u32) -> PropDef {
    PropDef::new(id, name, Kind::Choice(options), Value::Choice(v))
}

const fn flag(id: &'static str, name: &'static str, v: bool) -> PropDef {
    PropDef::new(id, name, Kind::Bool, Value::Bool(v))
}

pub static MORE_OPTIONS: GroupDef = GroupDef {
    id: "moreOptions",
    name: "More Options",
    body: Body::Fixed(&[
        DefNode::Prop(choice("anchorGrouping", "Anchor Point Grouping", GROUPINGS, 0).doc("what animators turn and scale about: each character, word or line, or all of it")),
        DefNode::Prop(PropDef::new("groupingAlignment", "Grouping Alignment", Kind::Vec2, Value::Vec2([0.0, 0.0])).unit(Unit::Percent).doc("moves that point across and up its group, in % of its size")),
        DefNode::Prop(choice("fillStroke", "Fill & Stroke", FILL_STROKE, 0).doc("each character with its own fill and stroke order, or every fill over every stroke, or the other way")),
        DefNode::Prop(choice("interCharBlend", "Inter-Character Blending", INTER_CHAR_BLEND, 0).doc("how each character is put over the ones before it")),
    ]),
    doc: "how animators place what they change",
};

pub static PATH_OPTIONS: GroupDef = GroupDef {
    id: "pathOptions",
    name: "Path Options",
    body: Body::Fixed(&[
        DefNode::Prop(num("path", "Path", 0.0).range(0.0, 1000.0).doc("the mask the text runs along, from 1 (0: none)")),
        DefNode::Prop(flag("reversePath", "Reverse Path", false)),
        DefNode::Prop(flag("perpendicular", "Perpendicular To Path", true).doc("characters turned to stand on the path")),
        DefNode::Prop(flag("forceAlignment", "Force Alignment", false).doc("the characters spread from the first margin to the last")),
        DefNode::Prop(num("firstMargin", "First Margin", 0.0).unit(Unit::Pixels)),
        DefNode::Prop(num("lastMargin", "Last Margin", 0.0).unit(Unit::Pixels)),
    ]),
    doc: "the text set along a mask's path",
};

pub static ANIMATORS: GroupDef = GroupDef { id: "animators", name: "Animators", body: Body::List("animators"), doc: "animators, applied top to bottom" };

pub static TEXT: GroupDef = GroupDef {
    id: "text",
    name: "Text",
    body: Body::Fixed(&[
        DefNode::Prop(PropDef::new("sourceText", "Source Text", Kind::Doc, Value::Text(String::new())).doc("the words and their look; its keys hold until the next")),
        DefNode::Group(&PATH_OPTIONS),
        DefNode::Group(&MORE_OPTIONS),
        DefNode::Prop(flag("perChar3d", "Per-character 3D", false).setting().doc("on a 3D layer, each character a plane of its own in the scene (animators move, turn and scale them in depth)")),
        DefNode::Group(&ANIMATORS),
    ]),
    doc: "the words, and what animates them",
};

pub static SELECTORS: GroupDef = GroupDef { id: "selectors", name: "Selectors", body: Body::List("selectors"), doc: "which characters the animator changes, and how much" };

pub static ANIMATOR: GroupDef = GroupDef {
    id: "animator",
    name: "Animator",
    body: Body::Fixed(&[
        DefNode::Group(&SELECTORS),
        DefNode::Prop(PropDef::new("anchor", "Anchor Point", Kind::Vec3, Value::Vec3([0.0; 3])).unit(Unit::Pixels).doc("moves the point characters turn and scale about")),
        DefNode::Prop(PropDef::new("position", "Position", Kind::Vec3, Value::Vec3([0.0; 3])).unit(Unit::Pixels).doc("moves the selected characters by this much (z: in depth, with per-character 3D)")),
        DefNode::Prop(PropDef::new("scale", "Scale", Kind::Vec3, Value::Vec3([100.0; 3])).unit(Unit::Percent)),
        DefNode::Prop(num("skew", "Skew", 0.0).range(-85.0, 85.0).unit(Unit::Degrees)),
        DefNode::Prop(num("skewAxis", "Skew Axis", 0.0).unit(Unit::Degrees)),
        DefNode::Prop(num("rotationX", "X Rotation", 0.0).unit(Unit::Degrees).three_d().doc("turn about the x axis (per-character 3D)")),
        DefNode::Prop(num("rotationY", "Y Rotation", 0.0).unit(Unit::Degrees).three_d().doc("turn about the y axis (per-character 3D)")),
        DefNode::Prop(num("rotation", "Rotation", 0.0).unit(Unit::Degrees)),
        DefNode::Prop(num("opacity", "Opacity", 100.0).range(0.0, 100.0).unit(Unit::Percent).doc("the opacity the selected characters take")),
        DefNode::Prop(PropDef::new("fillColor", "Fill Color", Kind::Color, Value::Color(veymelo_color::Rgba::CLEAR)).doc("the colour they take; its alpha is how much (default clear: no change)")),
        DefNode::Prop(num("fillHue", "Fill Hue", 0.0).unit(Unit::Degrees).doc("the fill's hue turned")),
        DefNode::Prop(num("fillSaturation", "Fill Saturation", 0.0).range(-100.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(num("fillBrightness", "Fill Brightness", 0.0).range(-100.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(num("fillOpacity", "Fill Opacity", 100.0).range(0.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(PropDef::new("strokeColor", "Stroke Color", Kind::Color, Value::Color(veymelo_color::Rgba::CLEAR)).doc("the stroke colour they take; its alpha is how much")),
        DefNode::Prop(num("strokeHue", "Stroke Hue", 0.0).unit(Unit::Degrees)),
        DefNode::Prop(num("strokeSaturation", "Stroke Saturation", 0.0).range(-100.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(num("strokeBrightness", "Stroke Brightness", 0.0).range(-100.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(num("strokeOpacity", "Stroke Opacity", 100.0).range(0.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(num("strokeWidth", "Stroke Width", 0.0).unit(Unit::Pixels).doc("added to the stroke's width")),
        DefNode::Prop(choice("trackingType", "Tracking Type", TRACKING_TYPES, 0).setting().doc("where the room tracking adds goes: half each side, before, or after each character")),
        DefNode::Prop(num("tracking", "Tracking Amount", 0.0).doc("room added at each selected character, in thousandths of the size")),
        DefNode::Prop(num("lineAnchor", "Line Anchor", 0.0).range(0.0, 100.0).unit(Unit::Percent).doc("where a line grows from as tracking spreads it: 0 its left, 50 its middle, 100 its right (set: else as the paragraph is aligned)")),
        DefNode::Prop(PropDef::new("lineSpacing", "Line Spacing", Kind::Vec2, Value::Vec2([0.0; 2])).unit(Unit::Pixels).doc("each line moved this much more than the one before")),
        DefNode::Prop(choice("characterAlignment", "Character Alignment", CHAR_ALIGNMENT, 1).setting().doc("where a changed character sits in the room of the one it replaces; adjust kerning: the line set again")),
        DefNode::Prop(choice("characterRange", "Character Range", CHAR_RANGE, 0).setting().doc("letters stay letters and digits digits, or any character")),
        DefNode::Prop(num("characterValue", "Character Value", 0.0).range(0.0, 1_114_111.0).doc("the selected characters become this one (by its number; 0: unset)")),
        DefNode::Prop(num("characterOffset", "Character Offset", 0.0).doc("the selected characters moved on so many in the alphabet")),
        DefNode::Prop(PropDef::new("blur", "Blur", Kind::Vec2, Value::Vec2([0.0; 2])).unit(Unit::Pixels).doc("the selected characters blurred across and down")),
    ]),
    doc: "changes the characters its selectors pick, as much as they pick them",
};

pub static RANGE: GroupDef = GroupDef {
    id: "range",
    name: "Range Selector",
    body: Body::Fixed(&[
        DefNode::Prop(num("start", "Start", 0.0).doc("where the range starts: % of the text, or a count with units index")),
        DefNode::Prop(num("end", "End", 100.0)),
        DefNode::Prop(num("offset", "Offset", 0.0).doc("moves the range along (animate it to sweep through the text)")),
        DefNode::Prop(choice("units", "Units", UNITS, 0).setting()),
        DefNode::Prop(choice("basedOn", "Based On", BASED_ON, 0).setting()),
        DefNode::Prop(choice("mode", "Mode", SELECTOR_MODES, 0).doc("how it joins the selectors above it")),
        DefNode::Prop(num("amount", "Amount", 100.0).range(-100.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(choice("shape", "Shape", SHAPES, 0).doc("how much is picked across the range: all of it (square), more and more (ramp up), less and less, peaking in the middle (triangle, round, smooth)")),
        DefNode::Prop(num("easeHigh", "Ease High", 0.0).range(-100.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(num("easeLow", "Ease Low", 0.0).range(-100.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(PropDef::new("randomize", "Randomize Order", Kind::Bool, Value::Bool(false)).doc("pick the characters in a random order")),
        DefNode::Prop(num("seed", "Random Seed", 0.0)),
    ]),
    doc: "picks a range of the text",
};

pub static WIGGLY: GroupDef = GroupDef {
    id: "wiggly",
    name: "Wiggly Selector",
    body: Body::Fixed(&[
        DefNode::Prop(choice("mode", "Mode", SELECTOR_MODES, 2).doc("how it joins the selectors above it (default intersect)")),
        DefNode::Prop(num("maxAmount", "Max Amount", 100.0).range(-100.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(num("minAmount", "Min Amount", -100.0).range(-100.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(choice("basedOn", "Based On", BASED_ON, 0).setting()),
        DefNode::Prop(num("speed", "Wiggles/Second", 2.0)),
        DefNode::Prop(num("correlation", "Correlation", 50.0).range(0.0, 100.0).unit(Unit::Percent).doc("how alike the characters wiggle")),
        DefNode::Prop(num("temporalPhase", "Temporal Phase", 0.0).unit(Unit::Degrees)),
        DefNode::Prop(num("spatialPhase", "Spatial Phase", 0.0).unit(Unit::Degrees)),
        DefNode::Prop(flag("lockDimensions", "Lock Dimensions", false).doc("x, y and z picked alike (else each its own)")),
        DefNode::Prop(num("seed", "Random Seed", 0.0)),
    ]),
    doc: "picks characters more or less at random, changing over time",
};

pub static EXPRESSION: GroupDef = GroupDef {
    id: "expression",
    name: "Expression Selector",
    body: Body::Fixed(&[
        DefNode::Prop(choice("basedOn", "Based On", BASED_ON, 0).setting()),
        DefNode::Prop(num("amount", "Amount", 100.0).range(-100.0, 100.0).unit(Unit::Percent).doc("how much each character is picked: give it an expression reading textIndex (from 1), textTotal and selectorValue (what the selectors above picked, in %)")),
    ]),
    doc: "picks each character as an expression says",
};
