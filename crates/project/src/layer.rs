//! A layer: what it shows, when, how it mixes with what is under it, and its properties.

use veymelo_color::{BlendMode, Rgba};
use veymelo_json::{Map, Value as Json};
use veymelo_time::Tick;

use crate::defs::{Catalog, GroupDef, NULL_LAYER, SHAPE_LAYER, TEXT_LAYER, TIMED_LAYER, VISUAL_LAYER};
use crate::props::Group;
use crate::read::{LoadError, Obj, fail, named};

named! {
    /// A colour tag for layers, items, keys and markers (0 is none).
    #[derive(Default)]
    pub enum Label {
        #[default]
        None = "none",
        Red = "red",
        Yellow = "yellow",
        Aqua = "aqua",
        Pink = "pink",
        Lavender = "lavender",
        Peach = "peach",
        SeaFoam = "seaFoam",
        Blue = "blue",
        Green = "green",
        Purple = "purple",
        Orange = "orange",
        Brown = "brown",
        Fuchsia = "fuchsia",
        Cyan = "cyan",
        Sandstone = "sandstone",
        DarkGreen = "darkGreen",
    }
}

impl Label {
    /// The colour it shows as.
    pub fn color(self) -> Rgba {
        let hex = match self {
            Label::None => "#808080",
            Label::Red => "#b53838",
            Label::Yellow => "#e4d84c",
            Label::Aqua => "#a9cbc7",
            Label::Pink => "#e5bcc9",
            Label::Lavender => "#a9a9ca",
            Label::Peach => "#e7c19e",
            Label::SeaFoam => "#b3c7b3",
            Label::Blue => "#677de0",
            Label::Green => "#4aa44c",
            Label::Purple => "#8e2c9a",
            Label::Orange => "#e8920d",
            Label::Brown => "#7f452a",
            Label::Fuchsia => "#f46dd6",
            Label::Cyan => "#3da2a5",
            Label::Sandstone => "#a89677",
            Label::DarkGreen => "#1e401e",
        };
        Rgba::from_hex(hex).unwrap_or(Rgba::BLACK)
    }
}

/// A mark at a moment (or over a stretch of time) with a note.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marker {
    pub time: Tick,
    pub duration: Tick,
    pub comment: String,
    pub label: Label,
    pub chapter: String,
    pub url: String,
    /// The stretch it covers keeps its length when the time around it is stretched.
    pub protected: bool,
    /// A web page frame the link opens in.
    pub frame_target: String,
    /// A cue point for players that read them: its name, whether it is one to jump to (else
    /// an event), and its parameters.
    pub cue: Option<CuePoint>,
}

/// A cue point a marker carries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CuePoint {
    pub name: String,
    pub navigation: bool,
    pub params: Vec<(String, String)>,
}

impl CuePoint {
    pub fn to_json(&self) -> Json {
        let mut o = Map::new();
        o.insert("name", self.name.as_str());
        if self.navigation {
            o.insert("navigation", true);
        }
        if !self.params.is_empty() {
            o.insert("params", Json::array(self.params.iter().map(|(k, v)| Json::array([Json::from(k.as_str()), Json::from(v.as_str())]))));
        }
        Json::Object(o)
    }

    pub fn from_json(j: &Json, at: &str) -> Result<CuePoint, LoadError> {
        let o = Obj::new(j, at, "a cue point")?;
        o.only(&["name", "navigation", "params"])?;
        let mut params = Vec::new();
        for (i, p) in o.list("params")?.iter().enumerate() {
            let pair = p.as_array().filter(|a| a.len() == 2);
            match pair.map(|a| (a[0].as_str(), a[1].as_str())) {
                Some((Some(k), Some(v))) => params.push((k.to_string(), v.to_string())),
                _ => return fail(&format!("{at}.params[{i}]"), "a cue point's parameter is [name, value], both text"),
            }
        }
        Ok(CuePoint { name: o.text("name")?.unwrap_or("").into(), navigation: o.flag("navigation", false)?, params })
    }
}

impl Marker {
    pub fn to_json(&self) -> Json {
        let mut o = Map::new();
        o.insert("t", self.time.seconds());
        if self.duration != Tick::ZERO {
            o.insert("duration", self.duration.seconds());
        }
        for (key, text) in [("comment", &self.comment), ("chapter", &self.chapter), ("url", &self.url)] {
            if !text.is_empty() {
                o.insert(key, text.as_str());
            }
        }
        if self.label != Label::None {
            o.insert("label", self.label.id());
        }
        if self.protected {
            o.insert("protected", true);
        }
        if !self.frame_target.is_empty() {
            o.insert("frameTarget", self.frame_target.as_str());
        }
        if let Some(c) = &self.cue {
            o.insert("cuePoint", c.to_json());
        }
        Json::Object(o)
    }

    pub fn from_json(j: &Json, at: &str) -> Result<Marker, LoadError> {
        let o = Obj::new(j, at, "a marker")?;
        o.only(&["t", "duration", "comment", "label", "chapter", "url", "protected", "frameTarget", "cuePoint"])?;
        let Some(time) = o.time("t")? else { return fail(at, "a marker needs \"t\", its time in seconds") };
        let duration = o.time_or("duration", Tick::ZERO)?;
        if duration < Tick::ZERO {
            return fail(&o.at("duration"), "a marker cannot last less than nothing");
        }
        Ok(Marker {
            time,
            duration,
            comment: o.text("comment")?.unwrap_or("").into(),
            label: o.choice("label", Label::IDS, Label::from_id)?.unwrap_or_default(),
            chapter: o.text("chapter")?.unwrap_or("").into(),
            url: o.text("url")?.unwrap_or("").into(),
            protected: o.flag("protected", false)?,
            frame_target: o.text("frameTarget")?.unwrap_or("").into(),
            cue: o.get("cuePoint").map(|c| CuePoint::from_json(c, &o.at("cuePoint"))).transpose()?,
        })
    }
}

named! {
    /// The kinds of light.
    #[derive(Default)]
    pub enum LightKind {
        Parallel = "parallel",
        Spot = "spot",
        #[default]
        Point = "point",
        Ambient = "ambient",
        /// A picture all round (a layer's, mapped as a panorama) lighting the scene and seen
        /// in what shines.
        Environment = "environment",
    }
}

named! {
    /// The solids a 3D primitive layer can be.
    #[derive(Default)]
    pub enum Primitive {
        #[default]
        Cube = "cube",
        Sphere = "sphere",
        Plane = "plane",
        Torus = "torus",
        Cone = "cone",
        Cylinder = "cylinder",
    }
}

/// What a layer shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A rectangle of one colour.
    Solid { color: Rgba, width: u32, height: u32 },
    /// Nothing: a handle other layers can be parented to.
    Null,
    Text,
    Shape,
    /// A footage item, by id.
    Footage(u64),
    /// Another composition, by id.
    Comp(u64),
    /// A 3D model (footage of the model kind), by id: drawn in the 3D scene.
    Model(u64),
    /// A solid made from a few numbers (a cube, sphere…), drawn in the 3D scene as a model is.
    Primitive(Primitive),
    Camera,
    Light(LightKind),
}

impl Source {
    pub const IDS: &'static [&'static str] = &["solid", "null", "text", "shape", "footage", "comp", "model", "primitive", "camera", "light"];

    pub fn id(&self) -> &'static str {
        match self {
            Source::Solid { .. } => "solid",
            Source::Null => "null",
            Source::Text => "text",
            Source::Shape => "shape",
            Source::Footage(_) => "footage",
            Source::Comp(_) => "comp",
            Source::Model(_) => "model",
            Source::Primitive(_) => "primitive",
            Source::Camera => "camera",
            Source::Light(_) => "light",
        }
    }

    /// The item it shows, if it shows one.
    pub fn item(&self) -> Option<u64> {
        match self {
            Source::Footage(id) | Source::Comp(id) | Source::Model(id) => Some(*id),
            _ => None,
        }
    }

    /// Whether it has pixels to draw.
    pub fn is_visual(&self) -> bool {
        !matches!(self, Source::Null | Source::Camera | Source::Light(_))
    }

    /// The properties a layer of this kind has.
    pub fn props_def(&self) -> &'static GroupDef {
        match self {
            Source::Null => &NULL_LAYER,
            Source::Camera => &crate::scene::CAMERA_LAYER,
            Source::Light(_) => &crate::scene::LIGHT_LAYER,
            Source::Comp(_) | Source::Footage(_) => &TIMED_LAYER,
            Source::Model(_) => &crate::scene::MODEL_LAYER,
            Source::Primitive(kind) => crate::scene::primitive_layer(*kind),
            Source::Shape => &SHAPE_LAYER,
            Source::Text => &TEXT_LAYER,
            _ => &VISUAL_LAYER,
        }
    }
}

named! {
    #[derive(Default)]
    pub enum Quality {
        #[default]
        Best = "best",
        Draft = "draft",
        Wireframe = "wireframe",
    }
}

named! {
    /// How a layer's pixels are picked when it is moved, scaled or turned.
    #[derive(Default)]
    pub enum Sampling {
        #[default]
        Bilinear = "bilinear",
        Bicubic = "bicubic",
    }
}

named! {
    /// How frames of footage at another rate are made to fit.
    #[derive(Default)]
    pub enum FrameBlend {
        #[default]
        Off = "off",
        FrameMix = "frameMix",
        PixelMotion = "pixelMotion",
    }
}

named! {
    /// How a layer turns by itself.
    #[derive(Default)]
    pub enum AutoOrient {
        #[default]
        Off = "off",
        /// Along the path its position moves on.
        AlongPath = "alongPath",
        /// Toward the camera (3D layers).
        TowardsCamera = "towardsCamera",
        /// Toward its point of interest (a two-node camera or light).
        TowardsPointOfInterest = "towardsPointOfInterest",
    }
}

/// A layer's switches.
#[derive(Clone, Debug, PartialEq)]
pub struct Switches {
    /// Seen.
    pub video: bool,
    /// Heard.
    pub audio: bool,
    /// Only soloed layers are seen while any layer is.
    pub solo: bool,
    pub locked: bool,
    /// Hidden from the layer list while the composition hides shy layers.
    pub shy: bool,
    /// A composition layer draws its layers straight into this one (and keeps vectors sharp).
    pub collapse: bool,
    pub quality: Quality,
    pub sampling: Sampling,
    /// Its effects are applied.
    pub effects: bool,
    pub frame_blend: FrameBlend,
    pub motion_blur: bool,
    /// It changes the layers under it instead of covering them.
    pub adjustment: bool,
    pub three_d: bool,
    /// Seen while working, left out of renders.
    pub guide: bool,
}

impl Default for Switches {
    fn default() -> Switches {
        Switches {
            video: true,
            audio: true,
            solo: false,
            locked: false,
            shy: false,
            collapse: false,
            quality: Quality::Best,
            sampling: Sampling::Bilinear,
            effects: true,
            frame_blend: FrameBlend::Off,
            motion_blur: false,
            adjustment: false,
            three_d: false,
            guide: false,
        }
    }
}

impl Switches {
    const FLAGS: [&'static str; 11] = ["video", "audio", "solo", "locked", "shy", "collapse", "effects", "motionBlur", "adjustment", "threeD", "guide"];

    fn flags(&mut self) -> [&mut bool; 11] {
        [
            &mut self.video,
            &mut self.audio,
            &mut self.solo,
            &mut self.locked,
            &mut self.shy,
            &mut self.collapse,
            &mut self.effects,
            &mut self.motion_blur,
            &mut self.adjustment,
            &mut self.three_d,
            &mut self.guide,
        ]
    }

    /// The switches not as they start, or nothing.
    pub fn to_json(&self) -> Option<Json> {
        let (mut me, mut start) = (self.clone(), Switches::default());
        let mut o = Map::new();
        for ((name, on), was) in Switches::FLAGS.iter().zip(me.flags()).zip(start.flags()) {
            if *on != *was {
                o.insert(*name, *on);
            }
        }
        if self.quality != start.quality {
            o.insert("quality", self.quality.id());
        }
        if self.sampling != start.sampling {
            o.insert("sampling", self.sampling.id());
        }
        if self.frame_blend != start.frame_blend {
            o.insert("frameBlend", self.frame_blend.id());
        }
        (!o.is_empty()).then_some(Json::Object(o))
    }

    pub fn from_json(j: &Json, at: &str) -> Result<Switches, LoadError> {
        let o = Obj::new(j, at, "switches")?;
        let mut known: Vec<&str> = Switches::FLAGS.to_vec();
        known.extend(["quality", "sampling", "frameBlend"]);
        o.only(&known)?;
        let mut s = Switches::default();
        for (name, flag) in Switches::FLAGS.iter().zip(s.flags()) {
            *flag = o.flag(name, *flag)?;
        }
        s.quality = o.choice("quality", Quality::IDS, Quality::from_id)?.unwrap_or_default();
        s.sampling = o.choice("sampling", Sampling::IDS, Sampling::from_id)?.unwrap_or_default();
        s.frame_blend = o.choice("frameBlend", FrameBlend::IDS, FrameBlend::from_id)?.unwrap_or_default();
        Ok(s)
    }
}

named! {
    /// What of a matte layer shows its layer through.
    #[derive(Default)]
    pub enum MatteKind {
        #[default]
        Alpha = "alpha",
        AlphaInverted = "alphaInverted",
        Luma = "luma",
        LumaInverted = "lumaInverted",
    }
}

/// Another layer of the composition whose alpha or brightness says where this one shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matte {
    pub layer: u64,
    pub kind: MatteKind,
}

/// A layer of a composition.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Its id in the project (0 until it is given one).
    pub id: u64,
    pub name: String,
    pub source: Source,
    pub label: Label,
    pub comment: String,
    /// The composition time the layer's own time starts from.
    pub start: Tick,
    /// When it is first and last seen, in composition time (the out point is not in it).
    pub in_point: Tick,
    pub out_point: Tick,
    /// How fast its own time goes, as a percentage of its length: 200 lasts twice as long
    /// (plays at half speed), below zero plays backwards.
    pub stretch: f64,
    pub switches: Switches,
    pub blend: BlendMode,
    /// Draws only where the layers under it are.
    pub preserve_transparency: bool,
    pub matte: Option<Matte>,
    /// The layer it moves with.
    pub parent: Option<u64>,
    pub markers: Vec<Marker>,
    pub auto_orient: AutoOrient,
    /// What a composition or footage layer shows is timed by its `timeRemap` property.
    pub time_remap: bool,
    /// Its position is three properties, x, y and z, each animated on its own.
    pub separate_position: bool,
    /// The picture an environment light with no source of its own lights the 3D scene by (the
    /// layer itself is not drawn).
    pub environment: bool,
    /// Shown as the sky all round the 3D scene: its picture as a panorama, seen through the
    /// camera as if endlessly far.
    pub environment_background: bool,
    pub props: Group,
}

impl Layer {
    /// A layer showing `source`, seen from 0 for `duration`, its properties as defined.
    pub fn new(id: u64, name: &str, source: Source, duration: Tick) -> Layer {
        let props = Group::new(source.props_def());
        Layer {
            id,
            name: name.to_string(),
            source,
            label: Label::None,
            comment: String::new(),
            start: Tick::ZERO,
            in_point: Tick::ZERO,
            out_point: duration,
            stretch: 100.0,
            switches: Switches::default(),
            blend: BlendMode::Normal,
            preserve_transparency: false,
            matte: None,
            parent: None,
            markers: Vec::new(),
            auto_orient: AutoOrient::Off,
            time_remap: false,
            separate_position: false,
            environment: false,
            environment_background: false,
            props,
        }
    }

    /// Whether it is seen at composition time `t`.
    pub fn is_active_at(&self, t: Tick) -> bool {
        t >= self.in_point && t < self.out_point
    }

    /// Its own time at composition time `t`.
    pub fn layer_time(&self, t: Tick) -> Tick {
        let d = t - self.start;
        if self.stretch == 100.0 { d } else { Tick((d.0 as f64 * 100.0 / self.stretch).round() as i64) }
    }

    /// The composition time at its own time `t`.
    pub fn comp_time(&self, t: Tick) -> Tick {
        if self.stretch == 100.0 { self.start + t } else { self.start + Tick((t.0 as f64 * self.stretch / 100.0).round() as i64) }
    }

    /// The time of what it shows at composition time `t`, as [`Layer::source_time`] gives it, a
    /// composition it shows keeping its protected stretches at their own speed while the layer
    /// is stretched (Responsive Design).
    pub fn shown_time(&self, project: &crate::Project, t: Tick) -> Tick {
        if !self.time_remap
            && self.stretch != 100.0
            && let Source::Comp(id) = self.source
            && let Some(nested) = project.comp(id)
            && let Some(r) = nested.responsive_time(self.stretch / 100.0, (t - self.start).seconds())
        {
            return r;
        }
        self.source_time(t)
    }

    /// The time of what it shows (a composition or footage), at composition time `t`.
    pub fn source_time(&self, t: Tick) -> Tick {
        let lt = self.layer_time(t);
        match self.props.prop("timeRemap") {
            Ok(p) if self.time_remap => Tick::from_seconds(p.value_at(lt).as_f64()),
            _ => lt,
        }
    }

    /// Its position at its own time `t`, from position or, separated, from x, y and z.
    pub fn position_at(&self, t: Tick) -> [f64; 3] {
        let tr = self.transform();
        let get = |p: &str| tr.prop(p).map(|p| p.value_at(t)).ok();
        if self.separate_position {
            let n = |p: &str| get(p).map_or(0.0, |v| v.as_f64());
            [n("positionX"), n("positionY"), n("positionZ")]
        } else {
            get("position").map_or([0.0; 3], |v| v.as_vec3())
        }
    }

    pub fn is_3d(&self) -> bool {
        self.switches.three_d || matches!(self.source, Source::Camera | Source::Light(_) | Source::Model(_) | Source::Primitive(_))
    }

    /// Every id in it given anew by `ids` (its own, its parent's and matte's, the instances in
    /// its lists, layers its properties name), each old id becoming one new one; and the items it
    /// shows renamed by `items`.
    pub fn remap(&mut self, ids: &mut dyn FnMut(u64) -> u64, items: &dyn Fn(u64) -> u64) {
        self.id = ids(self.id);
        self.parent = self.parent.map(&mut *ids);
        if let Some(m) = &mut self.matte {
            m.layer = ids(m.layer);
        }
        match &mut self.source {
            Source::Comp(id) | Source::Footage(id) | Source::Model(id) => *id = items(*id),
            _ => {}
        }
        fn walk(g: &mut Group, ids: &mut dyn FnMut(u64) -> u64) {
            if g.id != 0 {
                g.id = ids(g.id);
            }
            for c in &mut g.children {
                match c {
                    // the essential properties go by the ids of the controls they stand for
                    crate::Node::Group(sub) if sub.def.id == crate::props::ESSENTIALS_ID => {}
                    crate::Node::Group(sub) => walk(sub, ids),
                    crate::Node::Prop(p) => {
                        let fix = |v: &mut veymelo_keyframe::Value, ids: &mut dyn FnMut(u64) -> u64| {
                            if let veymelo_keyframe::Value::Layer(Some(id)) = v {
                                *id = ids(*id);
                            }
                        };
                        fix(&mut p.value, ids);
                        for k in &mut p.keys {
                            fix(&mut k.value, ids);
                        }
                    }
                }
            }
        }
        walk(&mut self.props, ids);
    }

    pub fn transform(&self) -> &Group {
        self.props.group("transform").expect("every layer has a transform")
    }

    pub fn to_json(&self) -> Json {
        let mut o = Map::new();
        if self.id != 0 {
            o.insert("id", self.id as f64);
        }
        o.insert("type", self.source.id());
        o.insert("name", self.name.as_str());
        match &self.source {
            Source::Solid { color, width, height } => {
                o.insert("color", veymelo_keyframe::Value::Color(*color).to_json(veymelo_keyframe::Kind::Color));
                o.insert("width", *width as f64);
                o.insert("height", *height as f64);
            }
            Source::Footage(id) | Source::Comp(id) | Source::Model(id) => {
                o.insert("item", *id as f64);
            }
            Source::Light(kind) => {
                o.insert("light", kind.id());
            }
            Source::Primitive(kind) => {
                o.insert("shape", kind.id());
            }
            Source::Null | Source::Text | Source::Shape | Source::Camera => {}
        }
        if self.label != Label::None {
            o.insert("label", self.label.id());
        }
        if !self.comment.is_empty() {
            o.insert("comment", self.comment.as_str());
        }
        o.insert("in", self.in_point.seconds());
        o.insert("out", self.out_point.seconds());
        if self.start != Tick::ZERO {
            o.insert("start", self.start.seconds());
        }
        if self.stretch != 100.0 {
            o.insert("stretch", self.stretch);
        }
        if let Some(s) = self.switches.to_json() {
            o.insert("switches", s);
        }
        if self.blend != BlendMode::Normal {
            o.insert("blend", self.blend.id());
        }
        if self.preserve_transparency {
            o.insert("preserveTransparency", true);
        }
        if let Some(m) = self.matte {
            o.insert("matte", Json::object([("layer", Json::Number(m.layer as f64)), ("kind", Json::from(m.kind.id()))]));
        }
        if let Some(p) = self.parent {
            o.insert("parent", p as f64);
        }
        if !self.markers.is_empty() {
            o.insert("markers", Json::array(self.markers.iter().map(Marker::to_json)));
        }
        if self.auto_orient != AutoOrient::Off {
            o.insert("autoOrient", self.auto_orient.id());
        }
        if self.time_remap {
            o.insert("timeRemap", true);
        }
        if self.separate_position {
            o.insert("separatePosition", true);
        }
        if self.environment {
            o.insert("environment", true);
        }
        if self.environment_background {
            o.insert("environmentBackground", true);
        }
        if let Some(p) = self.props.to_json() {
            o.insert("props", p);
        }
        Json::Object(o)
    }

    /// A layer from JSON; `duration` is the composition's, where a layer missing its out point ends.
    pub fn from_json(j: &Json, duration: Tick, catalog: &Catalog, at: &str) -> Result<Layer, LoadError> {
        let o = Obj::new(j, at, "a layer")?;
        let ty = o.need_text("type")?;
        let mut known = vec!["id", "type", "name", "label", "comment", "in", "out", "start", "stretch", "switches", "blend", "preserveTransparency", "matte", "parent", "markers", "autoOrient", "separatePosition", "environment", "environmentBackground", "props"];
        if matches!(ty, "comp" | "footage") {
            known.push("timeRemap");
        }
        let source = match ty {
            "solid" => {
                known.extend(["color", "width", "height"]);
                o.only(&known)?;
                let color = match o.get("color") {
                    None => Rgba::BLACK,
                    Some(c) => veymelo_keyframe::Value::from_json(veymelo_keyframe::Kind::Color, c).map_err(|e| LoadError { at: o.at("color"), message: e })?.as_color(),
                };
                Source::Solid { color, width: o.need_whole("width", 1, 30_000)? as u32, height: o.need_whole("height", 1, 30_000)? as u32 }
            }
            "footage" | "comp" | "model" => {
                known.push("item");
                o.only(&known)?;
                let Some(item) = o.id("item")? else { return fail(at, format!("a {ty} layer needs \"item\", the id of the {ty} it shows")) };
                match ty {
                    "comp" => Source::Comp(item),
                    "model" => Source::Model(item),
                    _ => Source::Footage(item),
                }
            }
            "light" => {
                known.push("light");
                o.only(&known)?;
                Source::Light(o.choice("light", LightKind::IDS, LightKind::from_id)?.unwrap_or_default())
            }
            "primitive" => {
                known.push("shape");
                o.only(&known)?;
                Source::Primitive(o.choice("shape", Primitive::IDS, Primitive::from_id)?.unwrap_or_default())
            }
            "null" | "text" | "shape" | "camera" => {
                o.only(&known)?;
                match ty {
                    "null" => Source::Null,
                    "text" => Source::Text,
                    "shape" => Source::Shape,
                    _ => Source::Camera,
                }
            }
            _ => return fail(&o.at("type"), format!("there is no layer type \"{ty}\" (there is {})", Source::IDS.join(", "))),
        };
        let name = o.text("name")?.unwrap_or(ty);
        let mut layer = Layer::new(o.id("id")?.unwrap_or(0), name, source, duration);
        layer.props = Group::from_json(layer.source.props_def(), o.get("props"), catalog, &o.at("props"))?;
        layer.label = o.choice("label", Label::IDS, Label::from_id)?.unwrap_or_default();
        layer.comment = o.text("comment")?.unwrap_or("").into();
        layer.in_point = o.time_or("in", Tick::ZERO)?;
        layer.out_point = o.time_or("out", duration)?;
        if layer.out_point <= layer.in_point {
            return fail(at, format!("a layer ends after it starts: in {} s, out {} s", layer.in_point.seconds(), layer.out_point.seconds()));
        }
        layer.start = o.time_or("start", Tick::ZERO)?;
        layer.stretch = o.num_or("stretch", 100.0)?;
        if layer.stretch == 0.0 || layer.stretch.abs() > 1e6 {
            return fail(&o.at("stretch"), "stretch is a percentage, not 0 (100 is the layer's own speed, below zero plays it backwards)");
        }
        if let Some(s) = o.get("switches") {
            layer.switches = Switches::from_json(s, &o.at("switches"))?;
        }
        layer.blend = o.choice("blend", &BlendMode::ALL.map(BlendMode::id), BlendMode::from_id)?.unwrap_or_default();
        layer.preserve_transparency = o.flag("preserveTransparency", false)?;
        if let Some(m) = o.get("matte") {
            let at = o.at("matte");
            let m = Obj::new(m, &at, "a matte")?;
            m.only(&["layer", "kind"])?;
            let Some(layer_id) = m.id("layer")? else { return fail(&at, "a matte needs \"layer\", the id of the layer it uses") };
            layer.matte = Some(Matte { layer: layer_id, kind: m.choice("kind", MatteKind::IDS, MatteKind::from_id)?.unwrap_or_default() });
        }
        layer.parent = o.id("parent")?;
        for (i, m) in o.list("markers")?.iter().enumerate() {
            layer.markers.push(Marker::from_json(m, &format!("{}[{i}]", o.at("markers")))?);
        }
        layer.markers.sort_by_key(|m| m.time);
        layer.auto_orient = o.choice("autoOrient", AutoOrient::IDS, AutoOrient::from_id)?.unwrap_or_default();
        layer.time_remap = o.flag("timeRemap", false)?;
        layer.separate_position = o.flag("separatePosition", false)?;
        layer.environment = o.flag("environment", false)?;
        layer.environment_background = o.flag("environmentBackground", false)?;
        Ok(layer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veymelo_keyframe::Value;

    fn solid() -> Layer {
        Layer::new(4, "Red Solid", Source::Solid { color: Rgba::from_hex("#ff0000").unwrap(), width: 1920, height: 1080 }, Tick::from_seconds(10.0))
    }

    #[test]
    fn a_plain_layer_writes_what_it_is() {
        assert_eq!(solid().to_json().to_json(), r##"{"id":4,"type":"solid","name":"Red Solid","color":"#ff0000","width":1920,"height":1080,"in":0,"out":10}"##);
        let back = Layer::from_json(&solid().to_json(), Tick::from_seconds(10.0), &Catalog::base(), "layers[0]").unwrap();
        assert_eq!(back, solid());
    }

    #[test]
    fn everything_about_a_layer_reads_back() {
        let mut l = solid();
        l.label = Label::SeaFoam;
        l.comment = "the sky".into();
        l.start = Tick::from_seconds(-1.0);
        l.in_point = Tick::from_seconds(0.5);
        l.stretch = -50.0;
        l.switches.three_d = true;
        l.switches.video = false;
        l.switches.quality = Quality::Draft;
        l.blend = BlendMode::ColorDodge;
        l.preserve_transparency = true;
        l.matte = Some(Matte { layer: 9, kind: MatteKind::LumaInverted });
        l.parent = Some(2);
        l.markers.push(Marker { time: Tick::from_seconds(2.0), duration: Tick::SECOND, comment: "beat".into(), label: Label::Red, ..Marker::default() });
        l.auto_orient = AutoOrient::AlongPath;
        l.props.prop_mut("transform/rotation").unwrap().set_value(Value::Number(45.0)).unwrap();
        let j = l.to_json();
        assert!(j.to_json().contains(r#""switches":{"video":false,"threeD":true,"quality":"draft"}"#), "{}", j.to_json());
        assert_eq!(Layer::from_json(&j, Tick::from_seconds(10.0), &Catalog::base(), "").unwrap(), l);
    }

    #[test]
    fn times_follow_the_stretch() {
        let mut l = solid();
        l.start = Tick::from_seconds(2.0);
        assert_eq!(l.layer_time(Tick::from_seconds(3.0)), Tick::SECOND);
        l.stretch = 200.0;
        assert_eq!(l.layer_time(Tick::from_seconds(4.0)), Tick::SECOND);
        assert_eq!(l.comp_time(Tick::SECOND), Tick::from_seconds(4.0));
        l.stretch = -100.0;
        assert_eq!(l.layer_time(Tick::from_seconds(3.0)), Tick::from_seconds(-1.0));
        assert!(l.is_active_at(Tick::ZERO) && !l.is_active_at(Tick::from_seconds(10.0)));
    }

    #[test]
    fn layer_mistakes() {
        let read = |text: &str| Layer::from_json(&Json::parse(text).unwrap(), Tick::from_seconds(10.0), &Catalog::base(), "layers[3]").unwrap_err().to_string();
        assert_eq!(read(r#"{"type": "sprite"}"#), "layers[3].type: there is no layer type \"sprite\" (there is solid, null, text, shape, footage, comp, model, primitive, camera, light)");
        assert_eq!(read(r#"{"type": "solid", "width": 100}"#), "layers[3]: \"height\" is missing");
        assert_eq!(read(r##"{"type": "null", "color": "#fff"}"##).split(" (").next().unwrap(), "layers[3]: there is no \"color\" here");
        assert_eq!(read(r#"{"type": "null", "in": 5, "out": 2}"#), "layers[3]: a layer ends after it starts: in 5 s, out 2 s");
        assert_eq!(read(r#"{"type": "comp"}"#), "layers[3]: a comp layer needs \"item\", the id of the comp it shows");
        assert_eq!(read(r#"{"type": "null", "blend": "plus"}"#).split(", \"dissolve\"").next().unwrap(), "layers[3].blend: expected one of \"normal\"");
        assert_eq!(read(r#"{"type": "null", "switches": {"3d": true}}"#).split(" (").next().unwrap(), "layers[3].switches: there is no \"3d\" here");
        assert_eq!(read(r#"{"type": "null", "markers": [{"comment": "x"}]}"#), "layers[3].markers[0]: a marker needs \"t\", its time in seconds");
    }

    #[test]
    fn labels_by_name() {
        assert_eq!(Label::ALL.len(), 17);
        assert_eq!(Label::from_id("seaFoam"), Some(Label::SeaFoam));
        assert_eq!(Label::Red.color().to_hex(), "#b53838");
    }
}
