//! The 3D scene's own layers and settings: cameras (where the composition is seen from, how
//! wide, what is in focus), lights (four kinds, with their cones, falloff and shadows), and the
//! material every 3D layer has (how it takes light and shadow).
//!
//! A camera or light is placed by its position and turned by its orientation and rotations; a
//! two-node one (auto-orient towards its point of interest) also aims at its point of interest.
//! Distances are in the composition's pixels.

use veymelo_keyframe::{Kind, Value};

use crate::{Body, DefNode, GroupDef, PropDef, Unit};

const fn num(id: &'static str, name: &'static str, v: f64) -> PropDef {
    PropDef::new(id, name, Kind::Number, Value::Number(v))
}

const fn pct(id: &'static str, name: &'static str, v: f64) -> PropDef {
    num(id, name, v).range(0.0, 100.0).unit(Unit::Percent)
}

const fn px(id: &'static str, name: &'static str, v: f64) -> PropDef {
    num(id, name, v).range(0.0, f64::INFINITY).unit(Unit::Pixels)
}

const fn deg(id: &'static str, name: &'static str, v: f64) -> PropDef {
    num(id, name, v).unit(Unit::Degrees)
}

const fn flag(id: &'static str, name: &'static str, v: bool) -> PropDef {
    PropDef::new(id, name, Kind::Bool, Value::Bool(v))
}

const fn choice(id: &'static str, name: &'static str, options: &'static [&'static str], v: u32) -> PropDef {
    PropDef::new(id, name, Kind::Choice(options), Value::Choice(v))
}

const fn p(def: PropDef) -> DefNode {
    DefNode::Prop(def)
}

const ZERO3: Value = Value::Vec3([0.0; 3]);

/// Where a camera or light is and which way it faces.
pub static SCENE_TRANSFORM: GroupDef = GroupDef {
    id: "transform",
    name: "Transform",
    body: Body::Fixed(&[
        p(PropDef::new("pointOfInterest", "Point of Interest", Kind::Vec3, ZERO3).unit(Unit::Pixels).spatial().doc("the point a two-node camera or light aims at")),
        p(PropDef::new("position", "Position", Kind::Vec3, ZERO3).unit(Unit::Pixels).spatial().doc("where it is (z: away from the viewer, so a camera in front has a z below 0)")),
        p(num("positionX", "X Position", 0.0).unit(Unit::Pixels).doc("x of the position, while separate")),
        p(num("positionY", "Y Position", 0.0).unit(Unit::Pixels)),
        p(num("positionZ", "Z Position", 0.0).unit(Unit::Pixels)),
        p(PropDef::new("orientation", "Orientation", Kind::Vec3, ZERO3).unit(Unit::Degrees).doc("which way it faces, turned about x, then y, then z (after aiming at its point of interest)")),
        p(deg("rotationX", "X Rotation", 0.0)),
        p(deg("rotationY", "Y Rotation", 0.0)),
        p(deg("rotation", "Z Rotation", 0.0).doc("turned about its view")),
    ]),
    doc: "where it is and which way it faces",
};

/// The iris shapes a camera's out-of-focus blur takes.
pub const IRIS_SHAPES: &[&str] = &["fastRectangle", "triangle", "square", "pentagon", "hexagon", "heptagon", "octagon", "nonagon", "decagon"];

pub static CAMERA_OPTIONS: GroupDef = GroupDef {
    id: "cameraOptions",
    name: "Camera Options",
    body: Body::Fixed(&[
        p(px("zoom", "Zoom", 1866.7).doc("how far the picture plane is from the eye: a composition-wide plane there fills the view (bigger: narrower, nearer-looking)")),
        p(flag("depthOfField", "Depth of Field", false)),
        p(px("focusDistance", "Focus Distance", 1866.7).doc("how far from the eye things are sharp")),
        p(px("aperture", "Aperture", 17.7).doc("how wide the lens opens: wider blurs more out of focus")),
        p(num("blurLevel", "Blur Level", 100.0).range(0.0, f64::INFINITY).unit(Unit::Percent)),
        p(choice("irisShape", "Iris Shape", IRIS_SHAPES, 0).doc("the shape a point out of focus spreads into")),
        p(deg("irisRotation", "Iris Rotation", 0.0)),
        p(pct("irisRoundness", "Iris Roundness", 0.0)),
        p(num("irisAspectRatio", "Iris Aspect Ratio", 1.0).range(0.01, 100.0)),
        p(num("irisDiffractionFringe", "Iris Diffraction Fringe", 0.0).range(0.0, 500.0)),
        p(pct("highlightGain", "Highlight Gain", 0.0).doc("bright parts out of focus made brighter, as light is")),
        p(num("highlightThreshold", "Highlight Threshold", 255.0).range(0.0, 255.0).doc("how bright a part has to be to count")),
        p(pct("highlightSaturation", "Highlight Saturation", 0.0)),
    ]),
    doc: "how the camera sees: how wide, and what is in focus",
};

/// How a light fades with distance.
pub const FALLOFFS: &[&str] = &["none", "smooth", "inverseSquareClamped"];

pub static LIGHT_OPTIONS: GroupDef = GroupDef {
    id: "lightOptions",
    name: "Light Options",
    body: Body::Fixed(&[
        p(num("intensity", "Intensity", 100.0).range(-100_000.0, 100_000.0).unit(Unit::Percent).doc("how bright (below 0 it takes light away)")),
        p(PropDef::new("color", "Color", Kind::Color, Value::Color(veymelo_color::Rgba::WHITE))),
        p(num("coneAngle", "Cone Angle", 90.0).range(0.0, 180.0).unit(Unit::Degrees).doc("a spot light's beam, across")),
        p(pct("coneFeather", "Cone Feather", 50.0).doc("how soft the beam's edge is")),
        p(choice("falloff", "Falloff", FALLOFFS, 0).doc("none; smooth: full within the radius, gone after the falloff distance more; or fading as the square of the distance past the radius")),
        p(px("radius", "Radius", 500.0)),
        p(px("falloffDistance", "Falloff Distance", 500.0)),
        p(flag("castsShadows", "Casts Shadows", false).doc("layers that cast shadows throw them by this light")),
        p(pct("shadowDarkness", "Shadow Darkness", 100.0)),
        p(px("shadowDiffusion", "Shadow Diffusion", 0.0).doc("how soft its shadows are, softer the farther they fall")),
        p(PropDef::new("source", "Source", Kind::Layer, Value::Layer(None)).doc("an environment light's picture: the layer whose picture is the panorama all round (none: the composition's environment layer)")),
        p(deg("rotation", "Environment Rotation", 0.0).doc("an environment light's panorama turned about the vertical")),
        p(pct("ambientOcclusion", "Ambient Occlusion", 0.0).doc("an ambient or environment light kept out of nooks and corners and from under what stands close (traced past every 3D layer and model), as much as this")),
        p(px("occlusionDistance", "Occlusion Distance", 60.0).doc("how far round a point the ambient occlusion looks for what hides it")),
    ]),
    doc: "what light it gives",
};

pub const SHADOW_CHOICES: &[&str] = &["off", "on", "only"];

/// The properties of how a 3D layer takes light and shadow, and any more given after them.
macro_rules! material_body {
    ($($more:expr),* $(,)?) => {
        Body::Fixed(&[
            p(choice("castsShadows", "Casts Shadows", SHADOW_CHOICES, 0).three_d().doc("throws shadows (only: throws them but is not seen)")),
            p(pct("lightTransmission", "Light Transmission", 0.0).three_d().doc("light through it from behind, coloured by it, and its shadow coloured too")),
            p(choice("acceptsShadows", "Accepts Shadows", SHADOW_CHOICES, 1).three_d().doc("shows shadows falling on it (only: just the shadows)")),
            p(flag("acceptsLights", "Accepts Lights", true).three_d()),
            p(pct("ambient", "Ambient", 100.0).three_d().doc("how much it takes of the ambient lights")),
            p(pct("diffuse", "Diffuse", 50.0).three_d().doc("how much it takes of the light falling on it")),
            p(pct("specularIntensity", "Specular Intensity", 50.0).three_d().doc("how bright its shine is")),
            p(pct("specularShininess", "Specular Shininess", 5.0).three_d().doc("how small and sharp its shine is")),
            p(pct("metal", "Metal", 100.0).three_d().doc("its shine coloured by it (0: by the light)")),
            p(pct("reflectionIntensity", "Reflection Intensity", 0.0).three_d().doc("how much it mirrors what is round it (traced to the nearest 3D layer or model each way, else the environment)")),
            p(pct("reflectionSharpness", "Reflection Sharpness", 100.0).three_d().doc("how sharp what it mirrors is (lower: blurred, as a rougher surface's)")),
            p(pct("reflectionRolloff", "Reflection Rolloff", 0.0).three_d().doc("how much less it mirrors where it faces the viewer than where it is seen edge on")),
            p(pct("transparency", "Transparency", 0.0).three_d().doc("a model seen through, as glass: what is behind it bent by its index of refraction and tinted by its colour")),
            p(pct("transparencyRolloff", "Transparency Rolloff", 0.0).three_d().doc("how much less clear it is seen edge on than face on")),
            p(num("indexOfRefraction", "Index of Refraction", 1.0).range(1.0, 3.0).three_d().doc("how much light bends going into it: 1 not at all, 1.33 water, 1.5 glass, 2.4 diamond")),
            p(choice("appearsInReflections", "Appears in Reflections", SHADOW_CHOICES, 1).three_d().doc("seen in what mirrors and glass show (only: seen there and nowhere else)")),
            $($more,)*
        ])
    };
}

/// How a 3D layer takes light and shadow (a 2D layer has it too, unused until it is 3D).
pub static MATERIAL_OPTIONS: GroupDef = GroupDef { id: "materialOptions", name: "Material Options", body: material_body!(), doc: "how the layer takes light and shadow when it is 3D" };

/// A 3D primitive's material: how it takes light and shadow, and its surface.
pub static PRIMITIVE_MATERIAL: GroupDef = GroupDef {
    id: "materialOptions",
    name: "Material Options",
    body: material_body!(
        p(PropDef::new("baseColor", "Base Color", Kind::Color, Value::Color(veymelo_color::Rgba { r: 0.8, g: 0.8, b: 0.8, a: 1.0 })).doc("its colour")),
        p(pct("metallic", "Metallic", 0.0).doc("how much it is metal (its shine coloured by it, no colour of its own beneath)")),
        p(pct("roughness", "Roughness", 50.0).doc("how rough its surface is (0: a mirror, 100: matte)")),
        p(PropDef::new("emissive", "Emissive", Kind::Color, Value::Color(veymelo_color::Rgba::BLACK)).doc("the light it gives off itself")),
    ),
    doc: "how the primitive takes light and shadow, and what its surface is",
};

const fn count(id: &'static str, name: &'static str, v: f64) -> PropDef {
    num(id, name, v).range(3.0, 512.0)
}

/// A cube's sizes.
pub static CUBE_GEOMETRY: GroupDef = GroupDef {
    id: "geometryOptions",
    name: "Geometry Options",
    body: Body::Fixed(&[p(px("width", "Width", 300.0)), p(px("height", "Height", 300.0)), p(px("depth", "Depth", 300.0))]),
    doc: "how big the cube is",
};

/// A plane's sizes.
pub static PLANE_GEOMETRY: GroupDef = GroupDef { id: "geometryOptions", name: "Geometry Options", body: Body::Fixed(&[p(px("width", "Width", 400.0)), p(px("height", "Height", 400.0))]), doc: "how big the plane is" };

/// A sphere's size and how finely it is made.
pub static SPHERE_GEOMETRY: GroupDef = GroupDef {
    id: "geometryOptions",
    name: "Geometry Options",
    body: Body::Fixed(&[p(px("radius", "Radius", 150.0)), p(count("segments", "Segments", 48.0).doc("pieces round it")), p(count("rings", "Rings", 24.0).doc("pieces from pole to pole"))]),
    doc: "how big the sphere is and how smooth",
};

/// A torus's sizes and how finely it is made.
pub static TORUS_GEOMETRY: GroupDef = GroupDef {
    id: "geometryOptions",
    name: "Geometry Options",
    body: Body::Fixed(&[
        p(px("radius", "Radius", 150.0).doc("from its middle to the middle of its tube")),
        p(px("tubeRadius", "Tube Radius", 50.0)),
        p(count("segments", "Segments", 48.0).doc("pieces round the ring")),
        p(count("rings", "Sides", 24.0).doc("pieces round the tube")),
    ]),
    doc: "how big the torus is and how smooth",
};

/// A cone's or cylinder's sizes and how finely it is made.
pub static ROUND_GEOMETRY: GroupDef = GroupDef {
    id: "geometryOptions",
    name: "Geometry Options",
    body: Body::Fixed(&[p(px("radius", "Radius", 150.0)), p(px("height", "Height", 300.0)), p(count("segments", "Segments", 48.0).doc("pieces round it"))]),
    doc: "how big it is and how smooth",
};

macro_rules! primitive_layer {
    ($name:ident, $geometry:ident) => {
        pub static $name: GroupDef = GroupDef {
            id: "layer",
            name: "Layer",
            body: Body::Fixed(&[DefNode::Group(&$geometry), DefNode::Group(&crate::defs::TRANSFORM), DefNode::Group(&PRIMITIVE_MATERIAL)]),
            doc: "",
        };
    };
}

primitive_layer!(CUBE_LAYER, CUBE_GEOMETRY);
primitive_layer!(PLANE_LAYER, PLANE_GEOMETRY);
primitive_layer!(SPHERE_LAYER, SPHERE_GEOMETRY);
primitive_layer!(TORUS_LAYER, TORUS_GEOMETRY);
primitive_layer!(ROUND_LAYER, ROUND_GEOMETRY);

/// A 3D primitive layer's properties.
pub fn primitive_layer(kind: crate::Primitive) -> &'static GroupDef {
    match kind {
        crate::Primitive::Cube => &CUBE_LAYER,
        crate::Primitive::Plane => &PLANE_LAYER,
        crate::Primitive::Sphere => &SPHERE_LAYER,
        crate::Primitive::Torus => &TORUS_LAYER,
        crate::Primitive::Cone | crate::Primitive::Cylinder => &ROUND_LAYER,
    }
}

/// A camera layer's properties.
pub static CAMERA_LAYER: GroupDef = GroupDef { id: "layer", name: "Layer", body: Body::Fixed(&[DefNode::Group(&SCENE_TRANSFORM), DefNode::Group(&CAMERA_OPTIONS)]), doc: "" };

/// A light layer's properties.
pub static LIGHT_LAYER: GroupDef = GroupDef { id: "layer", name: "Layer", body: Body::Fixed(&[DefNode::Group(&SCENE_TRANSFORM), DefNode::Group(&LIGHT_OPTIONS)]), doc: "" };

/// How a bevel's edge curves.
pub const BEVEL_STYLES: &[&str] = &["none", "angular", "concave", "convex"];

/// How a 3D text or shape layer is made solid: pushed back into depth, its edges bevelled.
pub static GEOMETRY_OPTIONS: GroupDef = GroupDef {
    id: "geometryOptions",
    name: "Geometry Options",
    body: Body::Fixed(&[
        p(choice("bevelStyle", "Bevel Style", BEVEL_STYLES, 0).doc("how its edges are rounded off: not at all, in a straight slope, hollowed in, or rounded out")),
        p(px("bevelDepth", "Bevel Depth", 2.0).doc("how far in its edges are rounded off")),
        p(pct("holeBevelDepth", "Hole Bevel Depth", 100.0).doc("the bevel round its holes (inside an O), in % of the bevel depth")),
        p(px("extrusionDepth", "Extrusion Depth", 0.0).doc("how deep it is made solid, back from its face (0: flat, as a picture)")),
    ]),
    doc: "how a 3D text or shape layer is made solid",
};

/// How a 3D model layer shows its model.
pub static MODEL_OPTIONS: GroupDef = GroupDef {
    id: "model",
    name: "3D Model",
    body: Body::Fixed(&[
        p(num("unitScale", "Unit Scale", 100.0).range(0.0, f64::INFINITY).unit(Unit::Pixels).doc("how many pixels one of the model's own units is (glTF's units are metres)")),
        p(num("animation", "Animation", 0.0).range(0.0, 10_000.0).doc("which of the model's animations plays, from 1 (0: none, the model as it stands)")),
        p(pct("speed", "Animation Speed", 100.0).range(-10_000.0, 10_000.0).doc("how fast it plays")),
        p(num("animationOffset", "Animation Offset", 0.0).unit(Unit::Seconds).doc("where in the animation the layer starts")),
        p(flag("loop", "Loop", true).doc("it plays again from the start when it ends")),
    ]),
    doc: "which model animation plays and how big the model is",
};

/// A 3D model layer's properties.
pub static MODEL_LAYER: GroupDef = GroupDef {
    id: "layer",
    name: "Layer",
    body: Body::Fixed(&[DefNode::Group(&MODEL_OPTIONS), DefNode::Group(&crate::defs::TRANSFORM), DefNode::Group(&MATERIAL_OPTIONS)]),
    doc: "",
};

/// The film width (mm) a camera's lens length is measured against.
pub const FILM_WIDTH_MM: f64 = 36.0;

/// The zoom of a lens `mm` long on a composition `width` pixels wide.
pub fn zoom_for_lens(width: f64, mm: f64) -> f64 {
    width * mm / FILM_WIDTH_MM
}

/// The aperture (pixels) a new camera has: 25.3 mm on a 1920-pixel-wide composition, in
/// proportion.
pub fn aperture_for(width: f64) -> f64 {
    25.3 * width / 1920.0
}

/// The zoom of the camera a composition is seen through when it has none: a 50 mm lens.
pub fn default_zoom(width: f64) -> f64 {
    zoom_for_lens(width, 50.0)
}
