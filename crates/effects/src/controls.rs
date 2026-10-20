//! Expression controls: effects that change nothing, holding a value (a number, a point, a
//! colour, a layer…) for expressions to read: `effect("Slider Control")("Slider")`.

use veymelo_color::Rgba;
use veymelo_keyframe::{Kind, Value};
use veymelo_project::{Body, DefNode, GroupDef, PropDef, Unit};

use crate::{Buf, Ctx, Spec};

fn pass(_: &Ctx, b: Buf) -> Buf {
    b
}

const CATEGORY: &str = "expressionControls";

macro_rules! control {
    ($def:ident, $spec:ident, $id:literal, $name:literal, $doc:literal, [$($prop:expr),*]) => {
        control!($def, $spec, $id, $name, $doc, [$($prop),*], &[]);
    };
    ($def:ident, $spec:ident, $id:literal, $name:literal, $doc:literal, [$($prop:expr),*], $points:expr) => {
        pub static $def: GroupDef = GroupDef { id: $id, name: $name, body: Body::Fixed(&[$(DefNode::Prop($prop)),*]), doc: $doc };
        pub const $spec: Spec = Spec::new(&$def, CATEGORY, pass).points($points);
    };
}

control!(SLIDER_DEF, SLIDER, "sliderControl", "Slider Control", "a number for expressions to read", [PropDef::new("slider", "Slider", Kind::Number, Value::Number(0.0))]);
control!(ANGLE_DEF, ANGLE, "angleControl", "Angle Control", "an angle for expressions to read", [PropDef::new("angle", "Angle", Kind::Number, Value::Number(0.0)).unit(Unit::Degrees)]);
control!(CHECKBOX_DEF, CHECKBOX, "checkboxControl", "Checkbox Control", "on or off, for expressions to read", [PropDef::new("checkbox", "Checkbox", Kind::Bool, Value::Bool(false))]);
control!(COLOR_DEF, COLOR, "colorControl", "Color Control", "a colour for expressions to read", [PropDef::new("color", "Color", Kind::Color, Value::Color(Rgba { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }))]);
control!(
    POINT_DEF,
    POINT,
    "pointControl",
    "Point Control",
    "a point on the layer for expressions to read",
    [PropDef::new("point", "Point", Kind::Vec2, Value::Vec2([0.0; 2])).unit(Unit::Pixels).spatial()],
    &[("point", [0.5, 0.5])]
);
control!(POINT_3D_DEF, POINT_3D, "point3dControl", "3D Point Control", "a point in space for expressions to read", [PropDef::new("point", "3D Point", Kind::Vec3, Value::Vec3([0.0; 3])).unit(Unit::Pixels).spatial()]);
control!(LAYER_DEF, LAYER, "layerControl", "Layer Control", "a layer for expressions to read", [PropDef::new("layer", "Layer", Kind::Layer, Value::Layer(None))]);
control!(
    MENU_DEF,
    MENU,
    "menuControl",
    "Dropdown Menu Control",
    "a choice from a list of its own, for expressions to read (the first is 1)",
    [
        PropDef::new("menu", "Menu", Kind::Number, Value::Number(1.0)).range(1.0, 1000.0).doc("which item, from 1"),
        PropDef::new("items", "Items", Kind::Text, Value::Text(String::new())).setting().doc("the items' names, one after another split by |, like \"Small|Medium|Large\"")
    ]
);

/// How a stereo rig's eyes stand: either side of its camera, or one of them the camera itself.
pub const STEREO_CONFIGURATIONS: &[&str] = &["stereoPair", "centerRight", "centerLeft"];

/// Where a stereo rig's eyes meet when they turn in.
pub const CONVERGE_TO: &[&str] = &["pointOfInterest", "zoom", "focusDistance"];

control!(
    STEREO_DEF,
    STEREO,
    "stereoControls",
    "Stereo 3D Controls",
    "how far apart a stereo rig's two eye cameras stand and where they meet, for their expressions to read (camera.stereoRig makes it)",
    [
        PropDef::new("configuration", "Configuration", Kind::Choice(STEREO_CONFIGURATIONS), Value::Choice(0))
            .doc("stereoPair: an eye either side of the camera; centerRight: the left eye is the camera, the right beside it; centerLeft: the right eye is the camera"),
        PropDef::new("sceneDepth", "Stereo Scene Depth", Kind::Number, Value::Number(3.0)).range(0.0, 100.0).unit(Unit::Percent).doc("how far apart the eyes are, in % of the composition's width"),
        PropDef::new("convergence", "Enable Convergence", Kind::Bool, Value::Bool(false)).doc("the eyes turn in to meet (else they look straight ahead, side by side)"),
        PropDef::new("convergeTo", "Converge To", Kind::Choice(CONVERGE_TO), Value::Choice(0)).doc("where they meet: as far ahead as the camera's point of interest, its zoom or its focus distance"),
        PropDef::new("zOffset", "Convergence Z Offset", Kind::Number, Value::Number(0.0)).unit(Unit::Pixels).doc("where they meet moved farther (or nearer, below 0)")
    ]
);

/// Every effect here.
pub const ALL: [Spec; 9] = [SLIDER, ANGLE, CHECKBOX, COLOR, POINT, POINT_3D, LAYER, MENU, STEREO];
