//! How effects' settings are written down (shared by every family).

use veymelo_color::Rgba;
use veymelo_keyframe::{Kind, Value};
use veymelo_project::{PropDef, Unit};

pub const fn num(id: &'static str, name: &'static str, v: f64) -> PropDef {
    PropDef::new(id, name, Kind::Number, Value::Number(v))
}

/// A number from `lo` to `hi`.
pub const fn ranged(id: &'static str, name: &'static str, v: f64, lo: f64, hi: f64) -> PropDef {
    num(id, name, v).range(lo, hi)
}

/// A percentage from `lo` to `hi`.
pub const fn pct(id: &'static str, name: &'static str, v: f64, lo: f64, hi: f64) -> PropDef {
    num(id, name, v).range(lo, hi).unit(Unit::Percent)
}

/// A distance in layer pixels, from 0 to `most`.
pub const fn px(id: &'static str, name: &'static str, v: f64, most: f64) -> PropDef {
    num(id, name, v).range(0.0, most).unit(Unit::Pixels)
}

pub const fn angle(id: &'static str, name: &'static str, v: f64) -> PropDef {
    num(id, name, v).unit(Unit::Degrees)
}

pub const fn choice(id: &'static str, name: &'static str, options: &'static [&'static str], v: u32) -> PropDef {
    PropDef::new(id, name, Kind::Choice(options), Value::Choice(v))
}

pub const fn flag(id: &'static str, name: &'static str, v: bool) -> PropDef {
    PropDef::new(id, name, Kind::Bool, Value::Bool(v))
}

/// A point on the layer, in its pixels (moved along a path when animated).
pub const fn point(id: &'static str, name: &'static str) -> PropDef {
    PropDef::new(id, name, Kind::Vec2, Value::Vec2([0.0; 2])).unit(Unit::Pixels).spatial()
}

pub const fn layer(id: &'static str, name: &'static str) -> PropDef {
    PropDef::new(id, name, Kind::Layer, Value::Layer(None))
}

pub const fn color(id: &'static str, name: &'static str, r: f64, g: f64, b: f64) -> PropDef {
    PropDef::new(id, name, Kind::Color, Value::Color(Rgba { r, g, b, a: 1.0 }))
}

/// An effect's definition: `effect!(DEF, "id", "Name", "what it does", [settings…])`.
macro_rules! effect {
    ($def:ident, $id:literal, $name:literal, $doc:literal, [$($prop:expr),* $(,)?]) => {
        pub static $def: veymelo_project::GroupDef =
            veymelo_project::GroupDef { id: $id, name: $name, body: veymelo_project::Body::Fixed(&[$(veymelo_project::DefNode::Prop($prop)),*]), doc: $doc };
    };
}
pub(crate) use effect;
