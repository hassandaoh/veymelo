//! What a shape layer is made of: groups of paths (rectangles, ellipses, stars, drawn paths),
//! what paints them (fills, strokes, gradients) and what changes them on the way (trim paths,
//! round corners, offset, pucker and bloat, twist, zig zag, wiggle, merge, repeater).
//!
//! Items are listed top first in a group's contents. A paint paints the paths above it in its
//! group (and in the groups inside), as changed by every operator in the group, above or below
//! it; items higher up are drawn on top.

use veymelo_keyframe::{Gradient, Kind, Value};

use crate::defs::{Body, DefNode, GroupDef, PropDef, Unit};

pub const DIRECTIONS: &[&str] = &["normal", "reversed"];
pub const STAR_TYPES: &[&str] = &["star", "polygon"];
pub const COMPOSITE: &[&str] = &["below", "above"];
pub const RULES: &[&str] = &["nonZero", "evenOdd"];
pub const CAPS: &[&str] = &["butt", "round", "square"];
pub const JOINS: &[&str] = &["miter", "round", "bevel"];
pub const GRADIENT_TYPES: &[&str] = &["linear", "radial"];
pub const TRIM_MODES: &[&str] = &["simultaneously", "individually"];
pub const POINT_TYPES: &[&str] = &["corner", "smooth"];
pub const MERGE_MODES: &[&str] = &["merge", "add", "subtract", "intersect", "exclude"];

const fn num(id: &'static str, name: &'static str, v: f64) -> PropDef {
    PropDef::new(id, name, Kind::Number, Value::Number(v))
}

const fn point(id: &'static str, name: &'static str, x: f64, y: f64) -> PropDef {
    PropDef::new(id, name, Kind::Vec2, Value::Vec2([x, y])).unit(Unit::Pixels)
}

const fn choice(id: &'static str, name: &'static str, options: &'static [&'static str], v: u32) -> PropDef {
    PropDef::new(id, name, Kind::Choice(options), Value::Choice(v))
}

const fn opacity() -> PropDef {
    num("opacity", "Opacity", 100.0).range(0.0, 100.0).unit(Unit::Percent)
}

const fn blend() -> PropDef {
    choice("blend", "Blend Mode", &veymelo_color::BlendMode::IDS, 0).setting().doc("how it mixes with what is drawn under it in the layer")
}

const fn direction() -> PropDef {
    choice("direction", "Path Direction", DIRECTIONS, 0).setting().doc("which way round the path goes (where trims start, how fills of crossing paths go)")
}

pub static GROUP_TRANSFORM: GroupDef = GroupDef {
    id: "transform",
    name: "Transform",
    body: Body::Fixed(&[
        DefNode::Prop(point("anchor", "Anchor Point", 0.0, 0.0).doc("the point the group turns and scales about")),
        DefNode::Prop(point("position", "Position", 0.0, 0.0)),
        DefNode::Prop(PropDef::new("scale", "Scale", Kind::Vec2, Value::Vec2([100.0; 2])).unit(Unit::Percent)),
        DefNode::Prop(num("skew", "Skew", 0.0).range(-85.0, 85.0).unit(Unit::Degrees).doc("a slant, along the skew axis")),
        DefNode::Prop(num("skewAxis", "Skew Axis", 0.0).unit(Unit::Degrees)),
        DefNode::Prop(num("rotation", "Rotation", 0.0).unit(Unit::Degrees)),
        DefNode::Prop(opacity()),
    ]),
    doc: "where the group's contents are, and how see-through",
};

pub static CONTENTS: GroupDef = GroupDef { id: "contents", name: "Contents", body: Body::List("contents"), doc: "shapes, paints and operators, top first" };

pub static GROUP: GroupDef = GroupDef {
    id: "group",
    name: "Group",
    body: Body::Fixed(&[DefNode::Prop(blend()), DefNode::Group(&CONTENTS), DefNode::Group(&GROUP_TRANSFORM)]),
    doc: "a group of shape items with its own transform; operators inside act only on it",
};

pub static RECT: GroupDef = GroupDef {
    id: "rect",
    name: "Rectangle",
    body: Body::Fixed(&[
        DefNode::Prop(direction()),
        DefNode::Prop(PropDef::new("size", "Size", Kind::Vec2, Value::Vec2([100.0; 2])).range(0.0, f64::INFINITY).unit(Unit::Pixels)),
        DefNode::Prop(point("position", "Position", 0.0, 0.0).doc("its centre")),
        DefNode::Prop(num("roundness", "Roundness", 0.0).range(0.0, f64::INFINITY).unit(Unit::Pixels).doc("the radius of its corners")),
    ]),
    doc: "a rectangle path",
};

pub static ELLIPSE: GroupDef = GroupDef {
    id: "ellipse",
    name: "Ellipse",
    body: Body::Fixed(&[
        DefNode::Prop(direction()),
        DefNode::Prop(PropDef::new("size", "Size", Kind::Vec2, Value::Vec2([100.0; 2])).range(0.0, f64::INFINITY).unit(Unit::Pixels)),
        DefNode::Prop(point("position", "Position", 0.0, 0.0).doc("its centre")),
    ]),
    doc: "an ellipse path",
};

pub static STAR: GroupDef = GroupDef {
    id: "star",
    name: "Polystar",
    body: Body::Fixed(&[
        DefNode::Prop(direction()),
        DefNode::Prop(choice("type", "Type", STAR_TYPES, 0).doc("a star (points in and out) or a polygon")),
        DefNode::Prop(num("points", "Points", 5.0).range(1.0, 100.0)),
        DefNode::Prop(point("position", "Position", 0.0, 0.0).doc("its centre")),
        DefNode::Prop(num("rotation", "Rotation", 0.0).unit(Unit::Degrees)),
        DefNode::Prop(num("innerRadius", "Inner Radius", 50.0).range(0.0, f64::INFINITY).unit(Unit::Pixels).doc("a star's inner points")),
        DefNode::Prop(num("outerRadius", "Outer Radius", 100.0).range(0.0, f64::INFINITY).unit(Unit::Pixels)),
        DefNode::Prop(num("innerRoundness", "Inner Roundness", 0.0).unit(Unit::Percent)),
        DefNode::Prop(num("outerRoundness", "Outer Roundness", 0.0).unit(Unit::Percent).doc("at 100 a polygon is a circle")),
    ]),
    doc: "a star or polygon path",
};

pub static PATH: GroupDef = GroupDef {
    id: "path",
    name: "Path",
    body: Body::Fixed(&[
        DefNode::Prop(direction()),
        DefNode::Prop(PropDef::new("path", "Path", Kind::Path, Value::Path(veymelo_keyframe::ShapePath { points: Vec::new(), in_tangents: Vec::new(), out_tangents: Vec::new(), closed: false, feather: veymelo_keyframe::Feathers::NONE })).doc("its points and tangents, in the layer's pixels")),
    ]),
    doc: "a path drawn point by point",
};

pub static FILL: GroupDef = GroupDef {
    id: "fill",
    name: "Fill",
    body: Body::Fixed(&[
        DefNode::Prop(blend()),
        DefNode::Prop(choice("composite", "Composite", COMPOSITE, 0).setting().doc("drawn below the paints above it in its group, or above them")),
        DefNode::Prop(choice("rule", "Fill Rule", RULES, 0).doc("for paths that cross or lie in each other: non-zero fills all of them, even-odd leaves holes")),
        DefNode::Prop(PropDef::new("color", "Color", Kind::Color, Value::Color(veymelo_color::Rgba::WHITE))),
        DefNode::Prop(opacity()),
    ]),
    doc: "fills the paths above it with a colour",
};

/// The settings every stroke has, `k` of them.
const fn stroke_look(k: usize) -> DefNode {
    DefNode::Prop(match k {
        0 => num("width", "Stroke Width", 2.0).range(0.0, f64::INFINITY).unit(Unit::Pixels),
        1 => choice("cap", "Line Cap", CAPS, 0).doc("how open ends end"),
        2 => choice("join", "Line Join", JOINS, 0).doc("how corners are turned"),
        _ => num("miter", "Miter Limit", 4.0).range(1.0, f64::INFINITY).doc("how far a pointed corner may reach, in stroke widths, before it is bevelled"),
    })
}

pub static DASHES: GroupDef = GroupDef {
    id: "dashes",
    name: "Dashes",
    body: Body::Fixed(&[
        DefNode::Prop(num("dash", "Dash", 0.0).range(0.0, f64::INFINITY).unit(Unit::Pixels).doc("0: a solid line")),
        DefNode::Prop(num("gap", "Gap", 0.0).range(0.0, f64::INFINITY).unit(Unit::Pixels).doc("0: the same as the dash")),
        DefNode::Prop(num("dash2", "Dash 2", 0.0).range(0.0, f64::INFINITY).unit(Unit::Pixels).doc("a second dash and gap, if not 0")),
        DefNode::Prop(num("gap2", "Gap 2", 0.0).range(0.0, f64::INFINITY).unit(Unit::Pixels)),
        DefNode::Prop(num("dash3", "Dash 3", 0.0).range(0.0, f64::INFINITY).unit(Unit::Pixels)),
        DefNode::Prop(num("gap3", "Gap 3", 0.0).range(0.0, f64::INFINITY).unit(Unit::Pixels)),
        DefNode::Prop(num("offset", "Offset", 0.0).unit(Unit::Pixels).doc("how far along the dashes start")),
    ]),
    doc: "dashes along the stroke",
};

/// How a taper's lengths are given.
pub const TAPER_UNITS: &[&str] = &["percent", "pixels"];

/// How a wave's length is given.
pub const WAVE_UNITS: &[&str] = &["pixels", "cycles"];

pub static TAPER: GroupDef = GroupDef {
    id: "taper",
    name: "Taper",
    body: Body::Fixed(&[
        DefNode::Prop(choice("lengthUnits", "Length Units", TAPER_UNITS, 0).setting().doc("the lengths as % of each line's length, or in pixels")),
        DefNode::Prop(num("startLength", "Start Length", 0.0).range(0.0, f64::INFINITY).doc("how far from its start it thins (0: not)")),
        DefNode::Prop(num("endLength", "End Length", 0.0).range(0.0, f64::INFINITY).doc("how far from its end it thins")),
        DefNode::Prop(num("startWidth", "Start Width", 100.0).range(0.0, 100.0).unit(Unit::Percent).doc("how wide it is at its start (0: a point)")),
        DefNode::Prop(num("endWidth", "End Width", 100.0).range(0.0, 100.0).unit(Unit::Percent).doc("how wide it is at its end")),
        DefNode::Prop(num("startEase", "Start Ease", 0.0).range(0.0, 100.0).unit(Unit::Percent).doc("0: thinning straight; 100: a round bulge")),
        DefNode::Prop(num("endEase", "End Ease", 0.0).range(0.0, 100.0).unit(Unit::Percent)),
    ]),
    doc: "the stroke thinner toward its ends",
};

pub static WAVE: GroupDef = GroupDef {
    id: "wave",
    name: "Wave",
    body: Body::Fixed(&[
        DefNode::Prop(num("amount", "Amount", 0.0).range(0.0, 100.0).unit(Unit::Percent).doc("how much thinner it gets between its widest (0: no wave)")),
        DefNode::Prop(choice("units", "Units", WAVE_UNITS, 0).setting().doc("a wave every so many pixels, or so many along each line")),
        DefNode::Prop(num("phase", "Phase", 0.0).unit(Unit::Degrees).doc("how far into a wave it starts")),
        DefNode::Prop(num("wavelength", "Wavelength", 40.0).range(0.0, f64::INFINITY).unit(Unit::Pixels).doc("pixels from one widest place to the next (units: pixels)")),
        DefNode::Prop(num("cycles", "Cycles", 4.0).range(0.0, f64::INFINITY).doc("waves along each line (units: cycles)")),
    ]),
    doc: "the stroke swelling and thinning along it",
};

pub static STROKE: GroupDef = GroupDef {
    id: "stroke",
    name: "Stroke",
    body: Body::Fixed(&[
        DefNode::Prop(blend()),
        DefNode::Prop(choice("composite", "Composite", COMPOSITE, 0).setting().doc("drawn below the paints above it in its group, or above them")),
        DefNode::Prop(PropDef::new("color", "Color", Kind::Color, Value::Color(veymelo_color::Rgba::WHITE))),
        DefNode::Prop(opacity()),
        stroke_look(0),
        stroke_look(1),
        stroke_look(2),
        stroke_look(3),
        DefNode::Group(&DASHES),
        DefNode::Group(&TAPER),
        DefNode::Group(&WAVE),
    ]),
    doc: "a line along the paths above it",
};

const GRADIENT_DEFAULT: Value = Value::Gradient(Gradient { colors: Vec::new(), opacities: Vec::new() });

/// The settings every gradient has, `k` of them.
const fn gradient_look(k: usize) -> DefNode {
    DefNode::Prop(match k {
        0 => choice("type", "Type", GRADIENT_TYPES, 0),
        1 => point("start", "Start Point", 0.0, 0.0).doc("where the gradient starts (a radial one: its centre)"),
        2 => point("end", "End Point", 100.0, 0.0).doc("where it ends (a radial one: how far out)"),
        3 => num("highlightLength", "Highlight Length", 0.0).range(-100.0, 100.0).unit(Unit::Percent).doc("a radial gradient's bright point, moved this far toward its edge"),
        4 => num("highlightAngle", "Highlight Angle", 0.0).unit(Unit::Degrees).doc("and turned this far from the line to its end point"),
        _ => PropDef::new("colors", "Colors", Kind::Gradient, GRADIENT_DEFAULT).doc("its colours (default white to black)"),
    })
}

pub static GRADIENT_FILL: GroupDef = GroupDef {
    id: "gradientFill",
    name: "Gradient Fill",
    body: Body::Fixed(&[
        DefNode::Prop(blend()),
        DefNode::Prop(choice("composite", "Composite", COMPOSITE, 0).setting().doc("drawn below the paints above it in its group, or above them")),
        DefNode::Prop(choice("rule", "Fill Rule", RULES, 0)),
        gradient_look(0),
        gradient_look(1),
        gradient_look(2),
        gradient_look(3),
        gradient_look(4),
        gradient_look(5),
        DefNode::Prop(opacity()),
    ]),
    doc: "fills the paths above it with a gradient",
};

pub static GRADIENT_STROKE: GroupDef = GroupDef {
    id: "gradientStroke",
    name: "Gradient Stroke",
    body: Body::Fixed(&[
        DefNode::Prop(blend()),
        DefNode::Prop(choice("composite", "Composite", COMPOSITE, 0).setting().doc("drawn below the paints above it in its group, or above them")),
        gradient_look(0),
        gradient_look(1),
        gradient_look(2),
        gradient_look(3),
        gradient_look(4),
        gradient_look(5),
        DefNode::Prop(opacity()),
        stroke_look(0),
        stroke_look(1),
        stroke_look(2),
        stroke_look(3),
        DefNode::Group(&DASHES),
        DefNode::Group(&TAPER),
        DefNode::Group(&WAVE),
    ]),
    doc: "a line along the paths above it, painted with a gradient",
};

pub static TRIM: GroupDef = GroupDef {
    id: "trim",
    name: "Trim Paths",
    body: Body::Fixed(&[
        DefNode::Prop(num("start", "Start", 0.0).range(0.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(num("end", "End", 100.0).range(0.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(num("offset", "Offset", 0.0).unit(Unit::Degrees).doc("turns the part shown along the path: 360 is once round")),
        DefNode::Prop(choice("mode", "Trim Multiple Shapes", TRIM_MODES, 0).doc("each path trimmed the same, or all of them as one long path")),
    ]),
    doc: "shows only a part of the paths above it (drawing them on, when animated)",
};

pub static ROUND: GroupDef = GroupDef {
    id: "round",
    name: "Round Corners",
    body: Body::Fixed(&[DefNode::Prop(num("radius", "Radius", 10.0).range(0.0, f64::INFINITY).unit(Unit::Pixels))]),
    doc: "rounds the sharp corners of the paths above it",
};

pub static OFFSET: GroupDef = GroupDef {
    id: "offset",
    name: "Offset Paths",
    body: Body::Fixed(&[
        DefNode::Prop(num("amount", "Amount", 10.0).unit(Unit::Pixels).doc("how far out (below zero: in)")),
        DefNode::Prop(choice("join", "Line Join", JOINS, 0)),
        DefNode::Prop(num("miter", "Miter Limit", 4.0).range(1.0, f64::INFINITY)),
        DefNode::Prop(num("copies", "Copies", 1.0).range(0.0, 100.0)),
        DefNode::Prop(num("copyOffset", "Copy Offset", 0.0)),
    ]),
    doc: "grows or shrinks the paths above it",
};

pub static PUCKER: GroupDef = GroupDef {
    id: "pucker",
    name: "Pucker & Bloat",
    body: Body::Fixed(&[DefNode::Prop(num("amount", "Amount", 0.0).range(-100.0, 100.0).unit(Unit::Percent).doc("above zero bloats, below zero puckers"))]),
    doc: "pulls the points in and pushes the curves out, or the other way",
};

pub static TWIST: GroupDef = GroupDef {
    id: "twist",
    name: "Twist",
    body: Body::Fixed(&[DefNode::Prop(num("angle", "Angle", 0.0).unit(Unit::Degrees)), DefNode::Prop(point("center", "Center", 0.0, 0.0))]),
    doc: "turns the paths more at the centre than at their edges",
};

pub static ZIGZAG: GroupDef = GroupDef {
    id: "zigzag",
    name: "Zig Zag",
    body: Body::Fixed(&[
        DefNode::Prop(num("size", "Size", 10.0).unit(Unit::Pixels)),
        DefNode::Prop(num("ridges", "Ridges per segment", 5.0).range(0.0, 100.0)),
        DefNode::Prop(choice("points", "Points", POINT_TYPES, 0)),
    ]),
    doc: "zig-zags the edges of the paths above it",
};

pub static WIGGLE: GroupDef = GroupDef {
    id: "wiggle",
    name: "Wiggle Paths",
    body: Body::Fixed(&[
        DefNode::Prop(num("size", "Size", 10.0).unit(Unit::Pixels)),
        DefNode::Prop(num("detail", "Detail", 10.0).range(0.0, 100.0).doc("points added to each curve")),
        DefNode::Prop(choice("points", "Points", POINT_TYPES, 1)),
        DefNode::Prop(num("speed", "Wiggles/Second", 2.0)),
        DefNode::Prop(num("correlation", "Correlation", 50.0).range(0.0, 100.0).unit(Unit::Percent).doc("how alike the points move")),
        DefNode::Prop(num("phase", "Temporal Phase", 0.0).unit(Unit::Degrees)),
        DefNode::Prop(num("seed", "Random Seed", 0.0)),
    ]),
    doc: "moves the points of the paths above it about, over time",
};

pub static MERGE: GroupDef = GroupDef {
    id: "merge",
    name: "Merge Paths",
    body: Body::Fixed(&[DefNode::Prop(choice("mode", "Mode", MERGE_MODES, 0).doc("merge (one compound path), add, subtract (the first less the rest), intersect, exclude"))]),
    doc: "makes the paths above it in its group one path; paints above it in the group are not drawn",
};

pub static REPEATER_TRANSFORM: GroupDef = GroupDef {
    id: "transform",
    name: "Transform",
    body: Body::Fixed(&[
        DefNode::Prop(point("anchor", "Anchor Point", 0.0, 0.0)),
        DefNode::Prop(point("position", "Position", 100.0, 0.0).doc("how far each copy moves from the one before")),
        DefNode::Prop(PropDef::new("scale", "Scale", Kind::Vec2, Value::Vec2([100.0; 2])).unit(Unit::Percent)),
        DefNode::Prop(num("rotation", "Rotation", 0.0).unit(Unit::Degrees)),
        DefNode::Prop(num("startOpacity", "Start Opacity", 100.0).range(0.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(num("endOpacity", "End Opacity", 100.0).range(0.0, 100.0).unit(Unit::Percent)),
    ]),
    doc: "how each copy is moved from the one before",
};

pub static REPEATER: GroupDef = GroupDef {
    id: "repeater",
    name: "Repeater",
    body: Body::Fixed(&[
        DefNode::Prop(num("copies", "Copies", 3.0).range(0.0, 1000.0).doc("a fraction draws the last copy part-way faded")),
        DefNode::Prop(num("offset", "Offset", 0.0).doc("copies to shift them all by")),
        DefNode::Prop(choice("composite", "Composite", COMPOSITE, 0).doc("each copy below the one before, or above")),
        DefNode::Group(&REPEATER_TRANSFORM),
    ]),
    doc: "copies everything above it in its group, each moved a step further",
};

/// Every type of shape item.
pub static TYPES: [&GroupDef; 17] = [&GROUP, &RECT, &ELLIPSE, &STAR, &PATH, &FILL, &STROKE, &GRADIENT_FILL, &GRADIENT_STROKE, &TRIM, &ROUND, &OFFSET, &PUCKER, &TWIST, &ZIGZAG, &WIGGLE, &MERGE];

/// …and the repeater.
pub static MORE_TYPES: [&GroupDef; 1] = [&REPEATER];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Catalog, Group, Layer, Node, Source};
    use veymelo_json::Value as Json;
    use veymelo_time::Tick;

    fn check(g: &GroupDef) {
        if let Body::Fixed(nodes) = &g.body {
            let mut ids: Vec<&str> = nodes.iter().map(DefNode::id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), nodes.len(), "names in {} are each used once", g.id);
            for n in nodes.iter() {
                match n {
                    DefNode::Prop(p) => assert_eq!(p.check(&p.default), Ok(()), "{}/{}", g.id, p.id),
                    DefNode::Group(sub) => check(sub),
                }
            }
        }
    }

    #[test]
    fn every_item_is_well_defined() {
        let c = Catalog::base();
        for t in TYPES.iter().chain(MORE_TYPES.iter()) {
            check(t);
            assert!(c.find("contents", t.id).is_some(), "{} is in the catalog", t.id);
        }
        assert_eq!(c.types("contents").len(), 18);
    }

    #[test]
    fn a_shape_layer_with_a_group_reads_back() {
        let mut l = Layer::new(1, "Shapes", Source::Shape, Tick::SECOND);
        let mut group = Group::instance(&GROUP, 2, "Group 1");
        let mut rect = Group::instance(&RECT, 3, "Rectangle 1");
        rect.prop_mut("size").unwrap().set_value(Value::Vec2([200.0, 80.0])).unwrap();
        let fill = Group::instance(&FILL, 4, "Fill 1");
        group.group_mut("contents").unwrap().children.extend([Node::Group(rect), Node::Group(fill)]);
        l.props.group_mut("contents").unwrap().children.push(Node::Group(group));
        let j = l.to_json();
        assert!(j.to_json().contains(r#""props":{"contents":[{"type":"group","id":2,"name":"Group 1","props":{"contents":[{"type":"rect","id":3,"name":"Rectangle 1","props":{"size":[200,80]}},{"type":"fill","id":4,"name":"Fill 1"}]}}]}"#), "{}", j.to_json());
        let back = Layer::from_json(&j, Tick::SECOND, &Catalog::base(), "").unwrap();
        assert_eq!(back, l);
        assert_eq!(back.props.prop("contents/Group 1/contents/Rectangle 1/size").unwrap().value, Value::Vec2([200.0, 80.0]));
        let bad = Json::parse(r#"{"type": "shape", "props": {"contents": [{"type": "square"}]}}"#).unwrap();
        assert!(Layer::from_json(&bad, Tick::SECOND, &Catalog::base(), "").unwrap_err().message.starts_with("contents have no type \"square\""));
    }
}
