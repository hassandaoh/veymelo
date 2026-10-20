//! What properties there are: their names, kinds, defaults and limits, written once in code.
//! A project file only holds what differs from these.

use veymelo_keyframe::{Kind, Value};

/// What a number of a property is measured in (for help and checks; values are plain numbers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    None,
    Pixels,
    Percent,
    Degrees,
    Seconds,
}

impl Unit {
    pub fn id(self) -> &'static str {
        match self {
            Unit::None => "",
            Unit::Pixels => "px",
            Unit::Percent => "%",
            Unit::Degrees => "°",
            Unit::Seconds => "s",
        }
    }
}

/// Which layers a property applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applies {
    Always,
    /// Only when the layer is 3D (orientation, X and Y rotation…).
    ThreeD,
    /// Only when the layer is 2D.
    TwoD,
}

/// A property: what it is called, what it holds, where it starts and how far it may go.
#[derive(Debug)]
pub struct PropDef {
    /// Its name in paths and files: `position`.
    pub id: &'static str,
    /// Its name for people: `Position`.
    pub name: &'static str,
    pub kind: Kind,
    pub default: Value,
    /// The least and most each of its numbers may be.
    pub min: f64,
    pub max: f64,
    pub unit: Unit,
    /// A point that moves along a path between keys (position, anchor point).
    pub spatial: bool,
    /// Can have keyframes and expressions (some settings cannot).
    pub animatable: bool,
    pub applies: Applies,
    pub doc: &'static str,
}

impl PropDef {
    pub const fn new(id: &'static str, name: &'static str, kind: Kind, default: Value) -> PropDef {
        PropDef { id, name, kind, default, min: f64::NEG_INFINITY, max: f64::INFINITY, unit: Unit::None, spatial: false, animatable: true, applies: Applies::Always, doc: "" }
    }

    pub const fn range(mut self, min: f64, max: f64) -> PropDef {
        self.min = min;
        self.max = max;
        self
    }

    pub const fn unit(mut self, unit: Unit) -> PropDef {
        self.unit = unit;
        self
    }

    pub const fn spatial(mut self) -> PropDef {
        self.spatial = true;
        self
    }

    /// A setting that cannot be animated.
    pub const fn setting(mut self) -> PropDef {
        self.animatable = false;
        self
    }

    pub const fn three_d(mut self) -> PropDef {
        self.applies = Applies::ThreeD;
        self
    }

    pub const fn doc(mut self, doc: &'static str) -> PropDef {
        self.doc = doc;
        self
    }

    /// Whether `v` is a value this property can hold.
    pub fn check(&self, v: &Value) -> Result<(), String> {
        if !v.is(self.kind) {
            return Err(format!("{} is {}", self.id, self.kind.shape()));
        }
        for i in 0..v.dims() {
            let x = v.component(i);
            if !x.is_finite() {
                return Err(format!("{} cannot be {x}", self.id));
            }
            // a colour's numbers are 0 to 1 by nature, its limits are for the others
            if !matches!(v, Value::Color(_)) && (x < self.min || x > self.max) {
                return Err(format!("{} goes from {} to {}{}, not {x}", self.id, self.min, self.max, self.unit.id()));
            }
        }
        Ok(())
    }

    /// One line on it, for help: `opacity (number, 0 to 100 %) = 100: how see-through…`.
    pub fn summary(&self) -> String {
        let mut s = format!("{} ({}", self.id, self.kind.name());
        if self.min.is_finite() || self.max.is_finite() {
            let end = |v: f64| if v.is_finite() { v.to_string() } else { "…".into() };
            s += &format!(", {} to {}", end(self.min), end(self.max));
        }
        if self.unit != Unit::None {
            s += &format!(" {}", self.unit.id());
        }
        if !self.animatable {
            s += ", not animated";
        }
        if self.applies == Applies::ThreeD {
            s += ", 3D layers";
        }
        s += &format!(") = {}", self.default.to_json(self.kind).to_json());
        if !self.doc.is_empty() {
            s += &format!(": {}", self.doc);
        }
        s
    }
}

/// What a group holds: always the same children, or a list of instances its user adds.
#[derive(Debug)]
pub enum Body {
    Fixed(&'static [DefNode]),
    /// Instances of the types a catalog has for this list (`masks`, `effects`, `contents`…).
    List(&'static str),
}

/// A group of properties (Transform), a list (Effects), or a type of instance in a list (a mask,
/// a blur).
#[derive(Debug)]
pub struct GroupDef {
    pub id: &'static str,
    pub name: &'static str,
    pub body: Body,
    pub doc: &'static str,
}

#[derive(Debug)]
pub enum DefNode {
    Prop(PropDef),
    Group(&'static GroupDef),
}

impl DefNode {
    pub fn id(&self) -> &'static str {
        match self {
            DefNode::Prop(p) => p.id,
            DefNode::Group(g) => g.id,
        }
    }
}

/// The types of instance each list can hold, by list name and type id.
#[derive(Debug, Default)]
pub struct Catalog {
    lists: Vec<(&'static str, Vec<&'static GroupDef>)>,
}

impl Catalog {
    /// The types this crate defines (masks); effects and the rest are added where they are made.
    pub fn base() -> Catalog {
        let mut c = Catalog::default();
        c.add("masks", &MASK);
        c.add("motionTrackers", &crate::tracking::TRACKER);
        c.add("trackPoints", &crate::tracking::TRACK_POINT);
        for t in crate::shapes::TYPES.iter().chain(crate::shapes::MORE_TYPES.iter()) {
            c.add("contents", t);
        }
        c.add("animators", &crate::text::ANIMATOR);
        c.add("selectors", &crate::text::RANGE);
        c.add("selectors", &crate::text::WIGGLY);
        c.add("selectors", &crate::text::EXPRESSION);
        c.add(crate::styles::LIST, &crate::styles::BLENDING_OPTIONS);
        for d in crate::styles::STYLES {
            c.add(crate::styles::LIST, d);
        }
        c
    }

    pub fn add(&mut self, list: &'static str, def: &'static GroupDef) {
        match self.lists.iter_mut().find(|(l, _)| *l == list) {
            Some((_, defs)) => {
                defs.retain(|d| d.id != def.id);
                defs.push(def);
            }
            None => self.lists.push((list, vec![def])),
        }
    }

    pub fn find(&self, list: &str, type_id: &str) -> Option<&'static GroupDef> {
        self.types(list).iter().copied().find(|d| d.id == type_id)
    }

    pub fn types(&self, list: &str) -> &[&'static GroupDef] {
        self.lists.iter().find(|(l, _)| *l == list).map_or(&[], |(_, d)| d)
    }
}

// The properties every layer is placed by. Positions are in the composition's pixels with y
// down, rotations in degrees clockwise, and +z goes away from the viewer.

const ZERO3: Value = Value::Vec3([0.0; 3]);

pub static TRANSFORM: GroupDef = GroupDef {
    id: "transform",
    name: "Transform",
    body: Body::Fixed(&[
        DefNode::Prop(PropDef::new("anchor", "Anchor Point", Kind::Vec3, ZERO3).unit(Unit::Pixels).spatial().doc("the point of the layer that sits at its position, and that it turns and scales around, in the layer's own pixels")),
        DefNode::Prop(PropDef::new("position", "Position", Kind::Vec3, ZERO3).unit(Unit::Pixels).spatial().doc("where the anchor point is in the composition (z is used by 3D layers)")),
        DefNode::Prop(PropDef::new("positionX", "X Position", Kind::Number, Value::Number(0.0)).unit(Unit::Pixels).doc("x of the position, used instead of it while the layer's position is in separate dimensions")),
        DefNode::Prop(PropDef::new("positionY", "Y Position", Kind::Number, Value::Number(0.0)).unit(Unit::Pixels).doc("y of the position, while separate")),
        DefNode::Prop(PropDef::new("positionZ", "Z Position", Kind::Number, Value::Number(0.0)).unit(Unit::Pixels).doc("z of the position, while separate (3D layers)")),
        DefNode::Prop(PropDef::new("scale", "Scale", Kind::Vec3, Value::Vec3([100.0; 3])).unit(Unit::Percent).doc("size, 100 % being the layer's own size; below zero flips it")),
        DefNode::Prop(PropDef::new("orientation", "Orientation", Kind::Vec3, ZERO3).unit(Unit::Degrees).three_d().doc("which way a 3D layer faces, turned about x, then y, then z")),
        DefNode::Prop(PropDef::new("rotationX", "X Rotation", Kind::Number, Value::Number(0.0)).unit(Unit::Degrees).three_d().doc("turn about the x axis, after orientation")),
        DefNode::Prop(PropDef::new("rotationY", "Y Rotation", Kind::Number, Value::Number(0.0)).unit(Unit::Degrees).three_d().doc("turn about the y axis, after X rotation")),
        DefNode::Prop(PropDef::new("rotation", "Rotation", Kind::Number, Value::Number(0.0)).unit(Unit::Degrees).doc("turn clockwise (about z for 3D layers); 360 is a whole turn")),
        DefNode::Prop(PropDef::new("opacity", "Opacity", Kind::Number, Value::Number(100.0)).range(0.0, 100.0).unit(Unit::Percent).doc("0 is invisible, 100 is solid")),
    ]),
    doc: "where the layer is, how big, which way it is turned, and how see-through",
};

pub static MASKS: GroupDef = GroupDef { id: "masks", name: "Masks", body: Body::List("masks"), doc: "shapes that cut out what of the layer is seen" };

/// How loud a layer with sound is heard.
pub static AUDIO: GroupDef = GroupDef {
    id: "audio",
    name: "Audio",
    body: Body::Fixed(&[DefNode::Prop(
        PropDef::new("levels", "Audio Levels", Kind::Vec2, Value::Vec2([0.0; 2])).range(-192.0, 24.0).doc("left and right, in dB: 0 as it is, +6 twice as loud, −6 half; −96 and under silent"),
    )]),
    doc: "how loud the layer's sound is",
};

pub static EFFECTS: GroupDef = GroupDef { id: "effects", name: "Effects", body: Body::List("effects"), doc: "effects, applied top to bottom" };

/// How a mask joins the masks above it.
pub const MASK_MODES: &[&str] = &["none", "add", "subtract", "intersect", "lighten", "darken", "difference"];

pub static MASK: GroupDef = GroupDef {
    id: "mask",
    name: "Mask",
    body: Body::Fixed(&[
        DefNode::Prop(PropDef::new("mode", "Mode", Kind::Choice(MASK_MODES), Value::Choice(1)).setting().doc("how it joins the masks above it")),
        DefNode::Prop(PropDef::new("inverted", "Inverted", Kind::Bool, Value::Bool(false)).setting().doc("keep the outside instead of the inside")),
        DefNode::Prop(PropDef::new("path", "Mask Path", Kind::Path, Value::Path(veymelo_keyframe::ShapePath { points: Vec::new(), in_tangents: Vec::new(), out_tangents: Vec::new(), closed: true, feather: veymelo_keyframe::Feathers::NONE })).doc("its outline, in the layer's pixels")),
        DefNode::Prop(PropDef::new("feather", "Mask Feather", Kind::Vec2, Value::Vec2([0.0; 2])).range(0.0, f64::INFINITY).unit(Unit::Pixels).doc("how soft its edge is, across and down")),
        DefNode::Prop(PropDef::new("opacity", "Mask Opacity", Kind::Number, Value::Number(100.0)).range(0.0, 100.0).unit(Unit::Percent)),
        DefNode::Prop(PropDef::new("expansion", "Mask Expansion", Kind::Number, Value::Number(0.0)).unit(Unit::Pixels).doc("grows (or below zero shrinks) the outline")),
        DefNode::Prop(PropDef::new("motionBlur", "Motion Blur", Kind::Choice(MASK_MOTION_BLUR), Value::Choice(0)).setting().doc("a moving outline blurred over the shutter: as the layer's switch says, always, or never")),
        DefNode::Prop(PropDef::new("featherFalloff", "Feather Falloff", Kind::Choice(FEATHER_FALLOFF), Value::Choice(0)).setting().doc("its softened edge goes over smoothly, or in a straight ramp")),
        DefNode::Prop(PropDef::new("rotoBezier", "RotoBezier", Kind::Bool, Value::Bool(false)).setting().doc("its curves follow its points by themselves (the tangents it has are not used)")),
    ]),
    doc: "a shape that cuts out what of the layer is seen",
};

/// When a mask's moving outline is blurred.
pub const MASK_MOTION_BLUR: &[&str] = &["sameAsLayer", "on", "off"];

/// How a mask's softened edge goes over.
pub const FEATHER_FALLOFF: &[&str] = &["smooth", "linear"];

/// The properties of a layer with pixels: masks, effects, transform.
pub static VISUAL_LAYER: GroupDef = GroupDef {
    id: "layer",
    name: "Layer",
    body: Body::Fixed(&[DefNode::Group(&MASKS), DefNode::Group(&EFFECTS), DefNode::Group(&TRANSFORM), DefNode::Group(&crate::styles::LAYER_STYLES), DefNode::Group(&crate::scene::MATERIAL_OPTIONS)]),
    doc: "",
};

/// The properties of a layer showing another composition or footage: time remap, motion
/// trackers, masks, effects, transform.
pub static TIMED_LAYER: GroupDef = GroupDef {
    id: "layer",
    name: "Layer",
    body: Body::Fixed(&[
        DefNode::Prop(PropDef::new("timeRemap", "Time Remap", Kind::Number, Value::Number(0.0)).unit(Unit::Seconds).doc("the time shown of what the layer shows, in seconds (used while its time remapping is on)")),
        DefNode::Group(&crate::tracking::MOTION_TRACKERS),
        DefNode::Group(&MASKS),
        DefNode::Group(&EFFECTS),
        DefNode::Group(&TRANSFORM),
        DefNode::Group(&AUDIO),
        DefNode::Group(&crate::styles::LAYER_STYLES),
        DefNode::Group(&crate::scene::MATERIAL_OPTIONS),
    ]),
    doc: "",
};

/// The properties of a layer without pixels of its own (a null): effects (for their controls)
/// and transform.
pub static NULL_LAYER: GroupDef =
    GroupDef { id: "layer", name: "Layer", body: Body::Fixed(&[DefNode::Group(&EFFECTS), DefNode::Group(&TRANSFORM)]), doc: "" };

/// The properties of a shape layer: its contents, masks, effects, transform.
pub static SHAPE_LAYER: GroupDef = GroupDef {
    id: "layer",
    name: "Layer",
    body: Body::Fixed(&[
        DefNode::Group(&crate::shapes::CONTENTS),
        DefNode::Group(&MASKS),
        DefNode::Group(&EFFECTS),
        DefNode::Group(&TRANSFORM),
        DefNode::Group(&crate::styles::LAYER_STYLES),
        DefNode::Group(&crate::scene::MATERIAL_OPTIONS),
        DefNode::Group(&crate::scene::GEOMETRY_OPTIONS),
    ]),
    doc: "",
};

/// The properties of a text layer: its text, masks, effects, transform.
pub static TEXT_LAYER: GroupDef = GroupDef {
    id: "layer",
    name: "Layer",
    body: Body::Fixed(&[
        DefNode::Group(&crate::text::TEXT),
        DefNode::Group(&MASKS),
        DefNode::Group(&EFFECTS),
        DefNode::Group(&TRANSFORM),
        DefNode::Group(&crate::styles::LAYER_STYLES),
        DefNode::Group(&crate::scene::MATERIAL_OPTIONS),
        DefNode::Group(&crate::scene::GEOMETRY_OPTIONS),
    ]),
    doc: "",
};


#[cfg(test)]
mod tests {
    use super::*;

    fn props(g: &GroupDef) -> Vec<&PropDef> {
        match &g.body {
            Body::Fixed(nodes) => nodes.iter().filter_map(|n| if let DefNode::Prop(p) = n { Some(p) } else { None }).collect(),
            Body::List(_) => Vec::new(),
        }
    }

    #[test]
    fn every_default_is_a_value_its_property_takes() {
        for g in [&TRANSFORM, &MASK] {
            let ps = props(g);
            for p in &ps {
                assert_eq!(p.check(&p.default), Ok(()), "{}", p.id);
            }
            let mut ids: Vec<_> = ps.iter().map(|p| p.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), ps.len(), "names in {} are each used once", g.id);
        }
    }

    #[test]
    fn checks_say_what_is_allowed() {
        let opacity = props(&TRANSFORM).into_iter().find(|p| p.id == "opacity").unwrap();
        assert_eq!(opacity.check(&Value::Number(50.0)), Ok(()));
        assert_eq!(opacity.check(&Value::Number(150.0)), Err("opacity goes from 0 to 100%, not 150".into()));
        assert_eq!(opacity.check(&Value::Vec2([1.0, 2.0])), Err("opacity is a number".into()));
        assert_eq!(opacity.check(&Value::Number(f64::NAN)), Err("opacity cannot be NaN".into()));
        assert_eq!(opacity.summary(), "opacity (number, 0 to 100 %) = 100: 0 is invisible, 100 is solid");
        let mode = props(&MASK).into_iter().find(|p| p.id == "mode").unwrap();
        assert_eq!(mode.check(&Value::Choice(9)), Err("mode is one of \"none\", \"add\", \"subtract\", \"intersect\", \"lighten\", \"darken\", \"difference\"".into()));
    }

    #[test]
    fn a_catalog_finds_types_by_list() {
        let mut c = Catalog::base();
        assert_eq!(c.find("masks", "mask").map(|d| d.name), Some("Mask"));
        assert!(c.find("effects", "mask").is_none());
        assert!(c.types("effects").is_empty());
        c.add("effects", &MASK);
        c.add("effects", &MASK);
        assert_eq!(c.types("effects").len(), 1, "adding a type again replaces it");
    }
}
