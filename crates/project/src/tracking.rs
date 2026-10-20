//! Motion trackers on a layer showing footage or a composition: what each follows and how, and
//! where it found its track points frame by frame.
//!
//! ```text
//! motionTrackers/Tracker 1    kind, position, rotation, scale, target, targetProperty, and how
//!                             it matches: channel, blur, enhance, subpixel, adaptEveryFrame,
//!                             threshold, lowConfidence, followShape
//!   points/Track Point 1      featureCenter, featureSize, searchOffset, searchSize, confidence,
//!                             attachPoint, attachPointOffset
//! ```
//!
//! Analysis gives Feature Center, Confidence and Attach Point a key for each frame analysed (in
//! the layer's time); applying a tracker writes keys on what it moves.

use veymelo_keyframe::{Kind, Value};

use crate::{Body, DefNode, GroupDef, PropDef, Unit};

/// What a tracker is for: moving another layer with what it follows (transform), holding the
/// tracked layer still (stabilize), pinning another layer's corners to three or four points
/// (parallel or perspective corner pin), or only the data.
pub const KINDS: &[&str] = &["transform", "stabilize", "parallel", "perspective", "raw"];

/// What the tracker compares.
pub const CHANNELS: &[&str] = &["rgb", "luminance", "saturation"];

/// What it does where it is unsure of a match.
pub const LOW_CONFIDENCE: &[&str] = &["continue", "stop", "extrapolate", "adapt"];

const fn p(def: PropDef) -> DefNode {
    DefNode::Prop(def)
}

const fn pair(id: &'static str, name: &'static str, v: [f64; 2]) -> PropDef {
    PropDef::new(id, name, Kind::Vec2, Value::Vec2(v)).unit(Unit::Pixels)
}

pub static TRACK_POINT: GroupDef = GroupDef {
    id: "trackPoint",
    name: "Track Point",
    body: Body::Fixed(&[
        p(pair("featureCenter", "Feature Center", [0.0; 2]).spatial().doc("the middle of what it follows, in the layer's pixels")),
        p(pair("featureSize", "Feature Size", [32.0; 2]).range(4.0, 4000.0).doc("how big the patch it follows is")),
        p(pair("searchOffset", "Search Offset", [0.0; 2]).doc("where the search area's middle is from the feature's")),
        p(pair("searchSize", "Search Size", [64.0; 2]).range(4.0, 8000.0).doc("how far round it looks for it on the next frame")),
        p(PropDef::new("confidence", "Confidence", Kind::Number, Value::Number(0.0)).range(0.0, 100.0).unit(Unit::Percent).doc("how sure it was of the match there")),
        p(pair("attachPoint", "Attach Point", [0.0; 2]).spatial().doc("the point that is moved to where it goes")),
        p(pair("attachPointOffset", "Attach Point Offset", [0.0; 2]).doc("where the attach point is from the feature")),
    ]),
    doc: "a patch of the picture followed frame by frame",
};

static POINTS: GroupDef = GroupDef { id: "points", name: "Track Points", body: Body::List("trackPoints"), doc: "what it follows" };

pub static TRACKER: GroupDef = GroupDef {
    id: "tracker",
    name: "Tracker",
    body: Body::Fixed(&[
        p(PropDef::new("kind", "Track Type", Kind::Choice(KINDS), Value::Choice(0)).setting().doc("transform: another layer moves with it; stabilize: the tracked layer is held still; parallel or perspective: another layer's corners pinned to its points; raw: the data alone")),
        p(PropDef::new("position", "Position", Kind::Bool, Value::Bool(true)).setting().doc("it follows where things go")),
        p(PropDef::new("rotation", "Rotation", Kind::Bool, Value::Bool(false)).setting().doc("and how they turn (two points)")),
        p(PropDef::new("scale", "Scale", Kind::Bool, Value::Bool(false)).setting().doc("and how they grow (two points)")),
        p(PropDef::new("target", "Motion Target", Kind::Layer, Value::Layer(None)).setting().doc("the layer it moves")),
        p(PropDef::new("targetProperty", "Target Property", Kind::Text, Value::Text(String::new())).setting().doc("a point of the target to move instead of its position, like \"effects/Lens Flare/center\"")),
        p(PropDef::new("channel", "Channel", Kind::Choice(CHANNELS), Value::Choice(1)).setting().doc("what it compares")),
        p(PropDef::new("blur", "Blur Before Match", Kind::Number, Value::Number(0.0)).range(0.0, 100.0).unit(Unit::Pixels).setting().doc("the frames softened first (noisy footage)")),
        p(PropDef::new("enhance", "Enhance Before Match", Kind::Bool, Value::Bool(false)).setting().doc("edges brought out first (soft footage)")),
        p(PropDef::new("subpixel", "Subpixel Positioning", Kind::Bool, Value::Bool(true)).setting().doc("to a fraction of a pixel")),
        p(PropDef::new("adaptEveryFrame", "Adapt Feature on Every Frame", Kind::Bool, Value::Bool(false)).setting().doc("the patch taken afresh on every frame (it changes as it goes)")),
        p(PropDef::new("threshold", "Confidence Threshold", Kind::Number, Value::Number(80.0)).range(0.0, 100.0).unit(Unit::Percent).setting().doc("below it, a match is unsure")),
        p(PropDef::new("lowConfidence", "If Confidence Is Below", Kind::Choice(LOW_CONFIDENCE), Value::Choice(3)).setting().doc("continue; stop; extrapolate the motion it had; or adapt: take the patch afresh")),
        p(PropDef::new("followShape", "Follow Shape", Kind::Bool, Value::Bool(true)).setting().doc("follow each patch turning and growing as it matches")),
        DefNode::Group(&POINTS),
    ]),
    doc: "follows patches of the picture through the layer's frames",
};

pub static MOTION_TRACKERS: GroupDef = GroupDef { id: "motionTrackers", name: "Motion Trackers", body: Body::List("motionTrackers"), doc: "trackers following what moves in the layer" };

/// How many track points a tracker of `kind` (an index of [`KINDS`]) has: two when it follows
/// turning or growing.
pub fn point_count(kind: u32, turns: bool) -> usize {
    match KINDS.get(kind as usize) {
        Some(&"parallel") => 3,
        Some(&"perspective") => 4,
        _ if turns => 2,
        _ => 1,
    }
}
