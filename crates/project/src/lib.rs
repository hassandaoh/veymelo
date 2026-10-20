//! The project: its items (folders, compositions, footage), the compositions' layers and each
//! layer's properties, and the `.veymelo` file they are kept in.
//!
//! A `.veymelo` file is JSON. It holds only what differs from the definitions in code (an
//! untouched opacity is not written), times are in seconds, colours `#rrggbb`, and everything
//! that has an id may be written without one: reading gives it one. Reading is strict: an
//! unknown key, a wrong kind of value or a reference to nothing is an error that says where.

mod comp;
mod defs;
pub mod essential;
mod layer;
mod props;
mod read;
pub mod queue;
pub mod scene;
pub mod shapes;
pub mod styles;
pub mod text;
pub mod tracking;

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use veymelo_json::{Map, Value as Json};
use veymelo_time::{FrameRate, Tick};

pub use comp::{Comp, MAX_SECONDS, MAX_SIZE, MotionBlur, fps_from_json, fps_json};
pub use defs::{AUDIO, Applies, Body, Catalog, DefNode, EFFECTS, GroupDef, MASK, MASK_MODES, MASKS, NULL_LAYER, PropDef, SHAPE_LAYER, TEXT_LAYER, TIMED_LAYER, TRANSFORM, Unit, VISUAL_LAYER};
pub use layer::{AutoOrient, CuePoint, FrameBlend, Label, Layer, LightKind, Marker, Matte, MatteKind, Primitive, Quality, Sampling, Source, Switches};
pub use props::{Expression, Group, Node, Own, Property};
pub use read::LoadError;
pub use veymelo_keyframe as keyframe;
pub use veymelo_keyframe::{Keyframe, Kind, Value};

use read::{Obj, fail, named};

/// What the first key of a project file says it is.
pub const FORMAT: &str = "veymelo-studio";
/// The version of the file format this program writes (and the newest it reads).
pub const VERSION: u64 = 1;

named! {
    #[derive(Default)]
    pub enum FootageKind {
        #[default]
        Video = "video",
        Image = "image",
        /// Numbered pictures played as frames.
        Sequence = "sequence",
        Audio = "audio",
        /// Standing in for footage there is not yet (its size, rate and length given): drawn as
        /// colour bars.
        Placeholder = "placeholder",
        /// A 3D model (glTF or OBJ), shown by 3D model layers.
        Model = "model",
    }
}

named! {
    /// How footage's colours are joined with its alpha.
    #[derive(Default)]
    pub enum AlphaMode {
        #[default]
        Straight = "straight",
        Premultiplied = "premultiplied",
        /// Every pixel solid.
        Ignore = "ignore",
    }
}

/// A picture, a video, a picture sequence or a sound, kept in a file outside the project.
#[derive(Clone, Debug, PartialEq)]
pub struct Footage {
    pub path: String,
    pub kind: FootageKind,
    /// Its size in pixels (0 for sound).
    pub width: u32,
    pub height: u32,
    pub pixel_aspect: f64,
    pub frame_rate: FrameRate,
    /// How long it plays (0 for a still picture, which lasts as long as its layer).
    pub duration: Tick,
    pub has_audio: bool,
    pub alpha: AlphaMode,
    /// How many times it plays.
    pub loops: u32,
    /// A picture sequence's first number (its path has a `#` for each digit of the number,
    /// or one `#` for numbers not padded).
    pub start: u32,
    /// The rate its frames were made at, when it plays at another (`frame_rate`): conformed.
    pub native_rate: Option<FrameRate>,
    /// One layer of a layered file (a Photoshop document), instead of the whole picture.
    pub layer: Option<SourceLayer>,
}

/// A layer of a layered picture file that footage shows: its place among the file's layers (from
/// the bottom, as the file keeps them), its name, and whether the footage is just the layer's own
/// rectangle (else the whole document's, the layer where it is in it).
#[derive(Clone, Debug, PartialEq)]
pub struct SourceLayer {
    pub index: u32,
    pub name: String,
    pub cropped: bool,
}

impl Footage {
    pub fn new(path: &str, kind: FootageKind) -> Footage {
        Footage { path: path.to_string(), kind, width: 0, height: 0, pixel_aspect: 1.0, frame_rate: FrameRate::FPS_30, duration: Tick::ZERO, has_audio: false, alpha: AlphaMode::Straight, loops: 1, start: 0, native_rate: None, layer: None }
    }

    /// The file of frame `n` of a picture sequence (its path for anything else).
    pub fn frame_path(&self, n: u64) -> String {
        if self.kind != FootageKind::Sequence {
            return self.path.clone();
        }
        let Some(first) = self.path.find('#') else { return self.path.clone() };
        let digits = self.path[first..].chars().take_while(|c| *c == '#').count();
        let number = format!("{:0digits$}", self.start as u64 + n);
        format!("{}{number}{}", &self.path[..first], &self.path[first + digits..])
    }
}

/// What an item is.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Folder,
    /// Shared, so a copy of the project for undo costs little; changed through
    /// [`Project::comp_mut`].
    Comp(Arc<Comp>),
    Footage(Footage),
}

/// An entry of the project: a folder, a composition or footage.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: u64,
    pub name: String,
    pub label: layer::Label,
    pub comment: String,
    /// The folder it is in (none: the top).
    pub folder: Option<u64>,
    pub kind: ItemKind,
    /// A file drawn in its place while it is switched on (a smaller or rendered stand-in).
    pub proxy: Option<Proxy>,
}

/// A stand-in for a composition or footage: a file drawn in its place, fitted to its size.
#[derive(Clone, Debug, PartialEq)]
pub struct Proxy {
    pub footage: Footage,
    /// Used where proxies that are switched on are used (the view, renders that say so).
    pub on: bool,
}

impl Item {
    /// The files it draws: its footage and its proxy's.
    pub fn files_mut(&mut self) -> impl Iterator<Item = &mut Footage> {
        let own = match &mut self.kind {
            ItemKind::Footage(f) => Some(f),
            _ => None,
        };
        own.into_iter().chain(self.proxy.as_mut().map(|p| &mut p.footage))
    }

    pub fn type_id(&self) -> &'static str {
        match self.kind {
            ItemKind::Folder => "folder",
            ItemKind::Comp(_) => "comp",
            ItemKind::Footage(_) => "footage",
        }
    }

    pub fn as_comp(&self) -> Option<&Comp> {
        if let ItemKind::Comp(c) = &self.kind { Some(c) } else { None }
    }

    /// As it is written in a project file.
    pub fn to_json(&self) -> Json {
        item_json(self)
    }
}

named! {
    #[derive(Default)]
    pub enum TimeDisplay {
        #[default]
        Timecode = "timecode",
        Frames = "frames",
    }
}

/// Settings for the whole project.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Bits for each colour channel while working: 8, 16 or 32 (floating point).
    pub bit_depth: u8,
    /// Mix colours in linear light instead of as stored.
    pub linear_blending: bool,
    /// Samples a second of sound.
    pub audio_rate: u32,
    pub time_display: TimeDisplay,
    /// The number of the first frame when counting in frames (0 or 1).
    pub frame_start: u8,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { bit_depth: 8, linear_blending: false, audio_rate: 48_000, time_display: TimeDisplay::Timecode, frame_start: 0 }
    }
}

/// A whole project.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub settings: Settings,
    /// In the order they are listed.
    pub items: Vec<Item>,
    /// Compositions waiting to be made into files, in order.
    pub queue: Vec<queue::RenderItem>,
    /// The next id to give out; items, layers and instances (masks, effects…) share them.
    pub next_id: u64,
    /// The composition shown when the project was last saved (it opens on it again).
    pub shown: Option<u64>,
}

impl Default for Project {
    fn default() -> Project {
        Project::new()
    }
}

impl Project {
    pub fn new() -> Project {
        Project { settings: Settings::default(), items: Vec::new(), queue: Vec::new(), next_id: 1, shown: None }
    }

    /// A new id.
    pub fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn item(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn item_mut(&mut self, id: u64) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    pub fn comp(&self, id: u64) -> Option<&Comp> {
        self.item(id)?.as_comp()
    }

    /// A composition to change (a copy of it, if a copy of the project shares it).
    pub fn comp_mut(&mut self, id: u64) -> Option<&mut Comp> {
        match &mut self.item_mut(id)?.kind {
            ItemKind::Comp(c) => Some(Arc::make_mut(c)),
            _ => None,
        }
    }

    /// Add an item, giving it an id. Returns the id.
    pub fn add(&mut self, name: &str, folder: Option<u64>, kind: ItemKind) -> u64 {
        let id = self.alloc();
        self.items.push(Item { id, name: name.to_string(), label: layer::Label::None, comment: String::new(), folder, kind, proxy: None });
        id
    }

    pub fn add_comp(&mut self, name: &str, comp: Comp) -> u64 {
        self.add(name, None, ItemKind::Comp(Arc::new(comp)))
    }

    /// A copy of composition `id`, its layers and their instances given new ids (links between
    /// its own layers kept), added beside it as `name`. Its id.
    pub fn duplicate_comp(&mut self, id: u64, name: &str) -> Option<u64> {
        let item = self.item(id)?.clone();
        let mut comp = item.as_comp()?.clone();
        let mut map = std::collections::HashMap::new();
        let mut next = self.next_id;
        let mut fresh = |old: u64| {
            *map.entry(old).or_insert_with(|| {
                next += 1;
                next - 1
            })
        };
        // layers first, so links between them find their new ids
        for l in &comp.layers {
            fresh(l.id);
        }
        for l in &mut comp.layers {
            l.remap(&mut fresh, &|i| i);
        }
        comp.essentials.remap(&mut fresh);
        self.next_id = next;
        let new = self.add(name, item.folder, ItemKind::Comp(Arc::new(comp)));
        if let Some(it) = self.item_mut(new) {
            (it.label, it.comment) = (item.label, item.comment);
        }
        Some(new)
    }

    /// Everything of another project added to this one, every id given anew, under `folder`.
    /// The new items' ids, in the other's order.
    pub fn merge(&mut self, other: Project, folder: Option<u64>) -> Vec<u64> {
        let mut map = std::collections::HashMap::new();
        let mut next = self.next_id;
        let mut fresh = |old: u64| {
            *map.entry(old).or_insert_with(|| {
                next += 1;
                next - 1
            })
        };
        let item_ids: Vec<(u64, u64)> = other.items.iter().map(|i| (i.id, fresh(i.id))).collect();
        let item_of = |old: u64| item_ids.iter().find(|(o, _)| *o == old).map_or(old, |(_, n)| *n);
        let mut added = Vec::new();
        for mut item in other.items {
            item.id = item_of(item.id);
            item.folder = item.folder.map(item_of).or(folder);
            if let ItemKind::Comp(c) = &mut item.kind {
                let c = Arc::make_mut(c);
                for l in &c.layers {
                    fresh(l.id);
                }
                for l in &mut c.layers {
                    l.remap(&mut fresh, &item_of);
                    essential::remap_instance(l, &mut fresh);
                }
                c.essentials.remap(&mut fresh);
            }
            added.push(item.id);
            self.items.push(item);
        }
        self.next_id = next.max(self.next_id);
        added
    }

    /// The composition a layer is in, and where in its stack.
    pub fn find_layer(&self, id: u64) -> Option<(u64, usize)> {
        self.items.iter().find_map(|i| Some((i.id, i.as_comp()?.index_of(id)?)))
    }

    pub fn layer(&self, id: u64) -> Option<&Layer> {
        let (comp, i) = self.find_layer(id)?;
        self.comp(comp)?.layers.get(i)
    }

    /// The project as JSON.
    pub fn to_json(&self) -> Json {
        let mut o = Map::new();
        o.insert("format", FORMAT);
        o.insert("version", VERSION as f64);
        let s = &self.settings;
        let d = Settings::default();
        let mut set = Map::new();
        if s.bit_depth != d.bit_depth {
            set.insert("bitDepth", s.bit_depth as f64);
        }
        if s.linear_blending != d.linear_blending {
            set.insert("linearBlending", s.linear_blending);
        }
        if s.audio_rate != d.audio_rate {
            set.insert("audioRate", s.audio_rate as f64);
        }
        if s.time_display != d.time_display {
            set.insert("timeDisplay", s.time_display.id());
        }
        if s.frame_start != d.frame_start {
            set.insert("frameStart", s.frame_start as f64);
        }
        if !set.is_empty() {
            o.insert("settings", set);
        }
        o.insert("nextId", self.next_id as f64);
        if let Some(c) = self.shown.filter(|c| self.comp(*c).is_some()) {
            o.insert("shown", c as f64);
        }
        o.insert("items", Json::array(self.items.iter().map(item_json)));
        if !self.queue.is_empty() {
            o.insert("renderQueue", Json::array(self.queue.iter().map(|q| q.to_json())));
        }
        Json::Object(o)
    }

    /// The project as the text of a `.veymelo` file.
    pub fn to_text(&self) -> String {
        self.to_json().to_pretty()
    }

    /// The project a `.veymelo` file's JSON holds.
    pub fn from_json(j: &Json, catalog: &Catalog) -> Result<Project, LoadError> {
        let o = Obj::new(j, "", "a project")?;
        if o.text("format").ok().flatten() != Some(FORMAT) {
            return fail("", format!("this is not a Veymelo project (it should start with \"format\": \"{FORMAT}\")"));
        }
        let version = o.need_whole("version", 1, u32::MAX as u64)?;
        if version > VERSION {
            return fail("version", format!("this project was made by a newer Veymelo (file version {version}; this one reads up to {VERSION})"));
        }
        o.only(&["format", "version", "settings", "nextId", "shown", "items", "renderQueue"])?;
        let mut p = Project::new();
        if let Some(s) = o.get("settings") {
            let s = Obj::new(s, "settings", "the settings")?;
            s.only(&["bitDepth", "linearBlending", "audioRate", "timeDisplay", "frameStart"])?;
            let d = Settings::default();
            p.settings = Settings {
                bit_depth: match s.whole("bitDepth", 8, 32)? {
                    None => d.bit_depth,
                    Some(b @ (8 | 16 | 32)) => b as u8,
                    Some(_) => return fail("settings.bitDepth", "the bit depth is 8, 16 or 32"),
                },
                linear_blending: s.flag("linearBlending", d.linear_blending)?,
                audio_rate: s.whole("audioRate", 8_000, 192_000)?.map_or(d.audio_rate, |v| v as u32),
                time_display: s.choice("timeDisplay", TimeDisplay::IDS, TimeDisplay::from_id)?.unwrap_or_default(),
                frame_start: s.whole("frameStart", 0, 1)?.map_or(d.frame_start, |v| v as u8),
            };
        }
        for (i, item) in o.list("items")?.iter().enumerate() {
            p.items.push(item_from_json(item, catalog, &format!("items[{i}]"))?);
        }
        if o.get("renderQueue").is_some() {
            for (i, q) in o.list("renderQueue")?.iter().enumerate() {
                p.queue.push(queue::RenderItem::from_json(q, &format!("renderQueue[{i}]"))?);
            }
        }
        p.next_id = o.id("nextId")?.unwrap_or(1);
        p.give_ids()?;
        // (a composition no longer there is not opened on)
        p.shown = o.id("shown")?.filter(|c| p.comp(*c).is_some());
        essential::settle(&mut p);
        if let Some(e) = p.problems().into_iter().next() {
            return Err(e);
        }
        Ok(p)
    }

    /// The project in the text of a `.veymelo` file.
    pub fn from_text(text: &str, catalog: &Catalog) -> Result<Project, LoadError> {
        let j = Json::parse(text).map_err(|e| LoadError { at: String::new(), message: format!("not JSON: {e}") })?;
        Project::from_json(&j, catalog)
    }

    /// Read the project in the file at `path`. Footage paths written relative to the file's
    /// folder are made whole.
    pub fn open(path: &Path, catalog: &Catalog) -> Result<Project, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut p = Project::from_text(&text, catalog).map_err(|e| format!("{}: {e}", path.display()))?;
        let dir = std::path::absolute(path).ok().and_then(|p| p.parent().map(Path::to_path_buf));
        if let Some(dir) = dir {
            for f in p.items.iter_mut().flat_map(Item::files_mut) {
                if !f.path.is_empty() && Path::new(&f.path).is_relative() {
                    f.path = dir.join(&f.path).to_string_lossy().into_owned();
                }
            }
        }
        Ok(p)
    }

    /// Write the project to `path`, whole or not at all (a new file is written, then put in
    /// place of the old one). Footage in the file's folder (or below it) is written relative to
    /// it, so the folder can be moved whole.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".saving");
        let tmp = std::path::PathBuf::from(tmp);
        let dir = std::path::absolute(path).ok().and_then(|p| p.parent().map(Path::to_path_buf));
        let text = match dir {
            Some(dir) if self.items.iter().any(|i| matches!(&i.kind, ItemKind::Footage(f) if Path::new(&f.path).starts_with(&dir)) || i.proxy.as_ref().is_some_and(|p| Path::new(&p.footage.path).starts_with(&dir))) => {
                let mut copy = self.clone();
                for f in copy.items.iter_mut().flat_map(Item::files_mut) {
                    if let Ok(rest) = Path::new(&f.path).strip_prefix(&dir) {
                        f.path = rest.to_string_lossy().into_owned();
                    }
                }
                copy.to_text()
            }
            _ => self.to_text(),
        };
        std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("cannot put {} in place: {e}", path.display()))
    }

    /// Every id must be used once; what has none (0) gets one.
    fn give_ids(&mut self) -> Result<(), LoadError> {
        let mut seen = HashSet::new();
        let mut twice = None;
        let mut note = |id: u64, at: String| {
            if id != 0 && !seen.insert(id) && twice.is_none() {
                twice = Some(LoadError { at, message: format!("id {id} is used more than once") });
            }
        };
        for (i, item) in self.items.iter().enumerate() {
            note(item.id, format!("items[{i}]"));
            if let ItemKind::Comp(c) = &item.kind {
                for (j, l) in c.layers.iter().enumerate() {
                    note(l.id, format!("items[{i}].layers[{j}]"));
                    l.props.instances(&mut |g| note(g.id, format!("items[{i}].layers[{j}].props")));
                }
                for k in c.essentials.flat() {
                    note(k.id, format!("items[{i}].essentials"));
                }
            }
        }
        for (i, q) in self.queue.iter().enumerate() {
            note(q.id, format!("renderQueue[{i}]"));
        }
        if let Some(e) = twice {
            return Err(e);
        }
        let mut next = self.next_id.max(seen.iter().max().map_or(1, |m| m + 1));
        let mut give = |id: &mut u64| {
            if *id == 0 {
                *id = next;
                next += 1;
            }
        };
        for item in &mut self.items {
            give(&mut item.id);
            if let ItemKind::Comp(c) = &mut item.kind {
                let c = Arc::make_mut(c);
                for l in &mut c.layers {
                    give(&mut l.id);
                    l.props.instances_mut(&mut |g| give(&mut g.id));
                }
                c.essentials.give_ids(&mut give);
            }
        }
        for q in &mut self.queue {
            give(&mut q.id);
        }
        self.next_id = next;
        Ok(())
    }

    /// What does not fit together: references to nothing, folders or compositions inside
    /// themselves, parents in a ring. Empty when all is well.
    pub fn problems(&self) -> Vec<LoadError> {
        let mut out = Vec::new();
        let mut bad = |at: String, message: String| out.push(LoadError { at, message });
        for (i, item) in self.items.iter().enumerate() {
            let at = format!("items[{i}]");
            if let Some(f) = item.folder {
                match self.item(f) {
                    Some(Item { kind: ItemKind::Folder, .. }) => {
                        // a folder may not be inside itself
                        let mut f = Some(f);
                        for _ in 0..=self.items.len() {
                            if f == Some(item.id) {
                                bad(at.clone(), format!("folder {} is inside itself", item.id));
                                break;
                            }
                            f = f.and_then(|f| self.item(f)).and_then(|i| i.folder);
                        }
                    }
                    _ => bad(format!("{at}.folder"), format!("there is no folder {f}")),
                }
            }
            let ItemKind::Comp(c) = &item.kind else { continue };
            if self.comp_inside(item.id, item.id, &mut HashSet::new()) {
                bad(at.clone(), format!("composition {} has itself inside", item.id));
            }
            for (j, l) in c.layers.iter().enumerate() {
                let at = format!("{at}.layers[{j}]");
                match l.source {
                    layer::Source::Comp(id) if self.comp(id).is_none() => bad(format!("{at}.item"), format!("there is no composition {id}")),
                    layer::Source::Footage(id) if !matches!(self.item(id), Some(Item { kind: ItemKind::Footage(_), .. })) => bad(format!("{at}.item"), format!("there is no footage {id}")),
                    layer::Source::Model(id) if !matches!(self.item(id), Some(Item { kind: ItemKind::Footage(f), .. }) if f.kind == FootageKind::Model) => bad(format!("{at}.item"), format!("there is no 3D model {id}")),
                    _ => {}
                }
                if let Some(p) = l.parent {
                    if c.layer(p).is_none() {
                        bad(format!("{at}.parent"), format!("there is no layer {p} in this composition"));
                    } else {
                        let mut up = Some(p);
                        for _ in 0..=c.layers.len() {
                            if up == Some(l.id) {
                                bad(format!("{at}.parent"), format!("layer {} would be its own parent", l.id));
                                break;
                            }
                            up = up.and_then(|u| c.layer(u)).and_then(|u| u.parent);
                        }
                    }
                }
                if let Some(m) = l.matte {
                    if m.layer == l.id {
                        bad(format!("{at}.matte"), "a layer cannot be its own matte".into());
                    } else if c.layer(m.layer).is_none() {
                        bad(format!("{at}.matte"), format!("there is no layer {} in this composition", m.layer));
                    }
                }
            }
        }
        for (i, q) in self.queue.iter().enumerate() {
            if self.comp(q.comp).is_none() {
                bad(format!("renderQueue[{i}].comp"), format!("there is no composition {}", q.comp));
            }
        }
        out
    }

    /// Whether composition `id` is `target` or has it inside (through its layers), at any depth.
    fn comp_inside(&self, id: u64, target: u64, seen: &mut HashSet<u64>) -> bool {
        let Some(c) = self.comp(id) else { return false };
        for l in &c.layers {
            if let layer::Source::Comp(inner) = l.source {
                if inner == target {
                    return true;
                }
                if seen.insert(inner) && self.comp_inside(inner, target, seen) {
                    return true;
                }
            }
        }
        false
    }
}

fn item_json(item: &Item) -> Json {
    let mut o = Map::new();
    o.insert("id", item.id as f64);
    o.insert("type", item.type_id());
    o.insert("name", item.name.as_str());
    if item.label != layer::Label::None {
        o.insert("label", item.label.id());
    }
    if !item.comment.is_empty() {
        o.insert("comment", item.comment.as_str());
    }
    if let Some(f) = item.folder {
        o.insert("folder", f as f64);
    }
    match &item.kind {
        ItemKind::Folder => {}
        ItemKind::Comp(c) => c.write(&mut o),
        ItemKind::Footage(f) => footage_write(f, &mut o),
    }
    if let Some(p) = &item.proxy {
        let mut pj = Map::new();
        footage_write(&p.footage, &mut pj);
        pj.insert("on", p.on);
        o.insert("proxy", pj);
    }
    Json::Object(o)
}

fn footage_write(f: &Footage, o: &mut Map) {
    let d = Footage::new("", f.kind);
    o.insert("path", f.path.as_str());
    o.insert("kind", f.kind.id());
    if f.width != 0 || f.height != 0 {
        o.insert("width", f.width as f64);
        o.insert("height", f.height as f64);
    }
    if f.pixel_aspect != d.pixel_aspect {
        o.insert("pixelAspect", f.pixel_aspect);
    }
    if matches!(f.kind, FootageKind::Video | FootageKind::Sequence) {
        o.insert("fps", fps_json(f.frame_rate));
    }
    if f.duration != Tick::ZERO {
        o.insert("duration", f.duration.seconds());
    }
    if f.has_audio {
        o.insert("hasAudio", true);
    }
    if f.alpha != d.alpha {
        o.insert("alpha", f.alpha.id());
    }
    if f.loops != 1 {
        o.insert("loops", f.loops as f64);
    }
    if f.start != 0 {
        o.insert("start", f.start as f64);
    }
    if let Some(r) = f.native_rate {
        o.insert("nativeFps", fps_json(r));
    }
    if let Some(l) = &f.layer {
        let mut m = Map::new();
        m.insert("index", l.index as f64);
        m.insert("name", l.name.as_str());
        if l.cropped {
            m.insert("cropped", true);
        }
        o.insert("layer", Json::Object(m));
    }
}

const FOOTAGE_FIELDS: [&str; 13] = ["path", "kind", "width", "height", "pixelAspect", "fps", "duration", "hasAudio", "alpha", "loops", "start", "nativeFps", "layer"];

fn footage_read(o: &Obj) -> Result<Footage, LoadError> {
    let kind = o.choice("kind", FootageKind::IDS, FootageKind::from_id)?.unwrap_or_default();
    let mut f = Footage::new(o.need_text("path")?, kind);
    f.width = o.whole("width", 0, MAX_SIZE as u64)?.unwrap_or(0) as u32;
    f.height = o.whole("height", 0, MAX_SIZE as u64)?.unwrap_or(0) as u32;
    f.pixel_aspect = o.num_or("pixelAspect", 1.0)?;
    if let Some(r) = o.get("fps") {
        f.frame_rate = fps_from_json(r).ok_or_else(|| LoadError { at: o.at("fps"), message: "expected frames a second (30, 29.97, \"30000/1001\")".into() })?;
    }
    f.duration = o.time_or("duration", Tick::ZERO)?;
    f.has_audio = o.flag("hasAudio", false)?;
    f.alpha = o.choice("alpha", AlphaMode::IDS, AlphaMode::from_id)?.unwrap_or_default();
    f.loops = o.whole("loops", 1, 10_000)?.unwrap_or(1) as u32;
    f.start = o.whole("start", 0, u32::MAX as u64)?.unwrap_or(0) as u32;
    if let Some(r) = o.get("nativeFps") {
        f.native_rate = Some(fps_from_json(r).ok_or_else(|| LoadError { at: o.at("nativeFps"), message: "expected frames a second (30, 29.97, \"30000/1001\")".into() })?);
    }
    if let Some(j) = o.get("layer") {
        let at = o.at("layer");
        let l = Obj::new(j, &at, "a layer of the file")?;
        l.only(&["index", "name", "cropped"])?;
        f.layer = Some(SourceLayer { index: l.whole("index", 0, u32::MAX as u64)?.unwrap_or(0) as u32, name: l.text("name")?.unwrap_or("").to_string(), cropped: l.flag("cropped", false)? });
    }
    Ok(f)
}

fn item_from_json(j: &Json, catalog: &Catalog, at: &str) -> Result<Item, LoadError> {
    let o = Obj::new(j, at, "an item")?;
    let ty = o.need_text("type")?;
    let mut known = vec!["id", "type", "name", "label", "comment", "folder"];
    let proxy = match o.get("proxy") {
        Some(j) => {
            let at = o.at("proxy");
            let p = Obj::new(j, &at, "a proxy")?;
            let mut fields = FOOTAGE_FIELDS.to_vec();
            fields.push("on");
            p.only(&fields)?;
            Some(Proxy { footage: footage_read(&p)?, on: p.flag("on", true)? })
        }
        None => None,
    };
    let kind = match ty {
        "folder" => {
            o.only(&known)?;
            ItemKind::Folder
        }
        "comp" => {
            known.extend(Comp::FIELDS);
            known.extend(["markers", "proxy"]);
            o.only(&known)?;
            ItemKind::Comp(Arc::new(Comp::read(&o, catalog)?))
        }
        "footage" => {
            known.extend(FOOTAGE_FIELDS);
            known.push("proxy");
            o.only(&known)?;
            ItemKind::Footage(footage_read(&o)?)
        }
        _ => return fail(&o.at("type"), format!("there is no item type \"{ty}\" (there is folder, comp, footage)")),
    };
    Ok(Item {
        id: o.id("id")?.unwrap_or(0),
        name: o.text("name")?.unwrap_or(ty).to_string(),
        label: o.choice("label", layer::Label::IDS, layer::Label::from_id)?.unwrap_or_default(),
        comment: o.text("comment")?.unwrap_or("").to_string(),
        folder: o.id("folder")?,
        kind,
        proxy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use veymelo_color::Rgba;

    fn sample() -> Project {
        let mut p = Project::new();
        let folder = p.add("Solids", None, ItemKind::Folder);
        let clip = p.add("clip.mov", Some(folder), ItemKind::Footage(Footage { width: 1280, height: 720, duration: Tick::from_seconds(4.0), has_audio: true, ..Footage::new("media/clip.mov", FootageKind::Video) }));
        let inner = p.add_comp("Title", Comp::new(800, 200, FrameRate::FPS_29_97, Tick::from_seconds(3.0)));
        let main = p.add_comp("Main", Comp::new(1920, 1080, FrameRate::FPS_30, Tick::from_seconds(10.0)));
        let ids: Vec<u64> = (0..4).map(|_| p.alloc()).collect();
        let c = p.comp_mut(main).unwrap();
        c.background = Rgba::from_hex("#202020").unwrap();
        let mut bg = Layer::new(ids[0], "Background", Source::Solid { color: Rgba::WHITE, width: 1920, height: 1080 }, c.duration);
        bg.props.prop_mut("transform/position").unwrap().set_value(Value::Vec3([960.0, 540.0, 0.0])).unwrap();
        let mut title = Layer::new(ids[1], "Title", Source::Comp(inner), Tick::from_seconds(3.0));
        title.parent = Some(ids[2]);
        let null = Layer::new(ids[2], "Mover", Source::Null, c.duration);
        let mut video = Layer::new(ids[3], "Clip", Source::Footage(clip), Tick::from_seconds(4.0));
        video.matte = Some(Matte { layer: ids[0], kind: MatteKind::Luma });
        c.layers = vec![title, null, video, bg];
        p
    }

    #[test]
    fn a_project_reads_back_the_same() {
        let p = sample();
        let text = p.to_text();
        let back = Project::from_text(&text, &Catalog::base()).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.to_text(), text, "written the same way every time");
        assert!(text.starts_with("{\n  \"format\": \"veymelo-studio\",\n  \"version\": 1,"));
        assert!(text.contains("\"fps\": \"30000/1001\""));
        assert_eq!(p.find_layer(p.items[3].as_comp().unwrap().layers[2].id), Some((p.items[3].id, 2)));
    }

    #[test]
    fn a_project_written_by_hand() {
        // no ids anywhere: reading gives them out, after any written ones
        let text = r##"{
          "format": "veymelo-studio", "version": 1,
          "items": [
            {"type": "comp", "name": "Main", "width": 1080, "height": 1920, "fps": 30, "duration": 5,
             "layers": [
               {"type": "solid", "name": "Card", "color": "#ffcc00", "width": 600, "height": 600,
                "props": {"transform": {"position": [540, 960], "opacity": {"keys": [{"t": 0, "v": 0}, {"t": 0.5, "v": 100}]}},
                          "masks": [{"type": "mask"}]}},
               {"id": 40, "type": "null", "in": 1}
             ]}
          ]
        }"##;
        let p = Project::from_text(text, &Catalog::base()).unwrap();
        let c = p.items[0].as_comp().unwrap();
        assert_eq!((p.items[0].id, c.layers[0].id, c.layers[0].props.group("masks/#1").unwrap().id, c.layers[1].id), (41, 42, 43, 40));
        assert_eq!(p.next_id, 44);
        assert_eq!(c.layers[1].out_point, Tick::from_seconds(5.0), "a layer lasts to the end of its composition");
        let opacity = c.layers[0].props.prop("transform/opacity").unwrap();
        assert_eq!(opacity.value_at(Tick::from_seconds(0.25)), Value::Number(50.0));
    }

    #[test]
    fn broken_projects_say_what_is_wrong() {
        let read = |items: &str| Project::from_text(&format!(r#"{{"format": "veymelo-studio", "version": 1, "items": [{items}]}}"#), &Catalog::base()).unwrap_err().to_string();
        let comp = |id: u64, layers: &str| format!(r#"{{"id": {id}, "type": "comp", "name": "C", "width": 100, "height": 100, "fps": 25, "duration": 2, "layers": [{layers}]}}"#);
        assert_eq!(read(&comp(1, r#"{"id": 2, "type": "null", "parent": 3}"#)), "items[0].layers[0].parent: there is no layer 3 in this composition");
        assert_eq!(read(&comp(1, r#"{"id": 2, "type": "null", "parent": 3}, {"id": 3, "type": "null", "parent": 2}"#)), "items[0].layers[0].parent: layer 2 would be its own parent");
        assert_eq!(read(&comp(1, r#"{"id": 2, "type": "null", "matte": {"layer": 2}}"#)), "items[0].layers[0].matte: a layer cannot be its own matte");
        assert_eq!(read(&comp(1, r#"{"id": 2, "type": "comp", "item": 1}"#)), "items[0]: composition 1 has itself inside");
        assert_eq!(read(&format!("{}, {}", comp(1, r#"{"type": "comp", "item": 5}"#), comp(5, r#"{"type": "comp", "item": 1}"#))), "items[0]: composition 1 has itself inside");
        assert_eq!(read(&comp(1, r#"{"type": "footage", "item": 1}"#)), "items[0].layers[0].item: there is no footage 1");
        assert_eq!(read(r#"{"id": 1, "type": "folder", "name": "A", "folder": 1}"#), "items[0]: folder 1 is inside itself");
        assert_eq!(read(r#"{"id": 1, "type": "folder", "name": "A"}, {"id": 1, "type": "folder", "name": "B"}"#), "items[1]: id 1 is used more than once");
        assert_eq!(read(r#"{"type": "comp", "width": 100, "height": 100, "duration": 2}"#), "items[0]: \"fps\" is missing (30, 29.97, \"30000/1001\"…)");
        assert_eq!(read(r#"{"type": "comp", "width": 100, "height": 100, "fps": 0, "duration": 2}"#), "items[0].fps: expected frames a second (30, 29.97, \"30000/1001\"), got 0");
        assert_eq!(read(r#"{"type": "comp", "width": 40000, "height": 100, "fps": 30, "duration": 2}"#), "items[0].width: expected a whole number from 1 to 30000, got 40000");
        assert_eq!(read(r#"{"type": "video"}"#), "items[0].type: there is no item type \"video\" (there is folder, comp, footage)");
        let other = Project::from_text(r#"{"format": "veymelo-studio", "version": 9, "items": []}"#, &Catalog::base()).unwrap_err();
        assert_eq!(other.to_string(), "version: this project was made by a newer Veymelo (file version 9; this one reads up to 1)");
        assert_eq!(Project::from_text("{\"name\": 1}", &Catalog::base()).unwrap_err().message, "this is not a Veymelo project (it should start with \"format\": \"veymelo-studio\")");
        assert!(Project::from_text("{oops", &Catalog::base()).unwrap_err().message.starts_with("not JSON: "));
    }

    #[test]
    fn copies_share_compositions_until_changed() {
        let mut p = sample();
        let copy = p.clone();
        let main = p.items[3].id;
        let shared = |a: &Project, b: &Project| match (&a.items[3].kind, &b.items[3].kind) {
            (ItemKind::Comp(x), ItemKind::Comp(y)) => Arc::ptr_eq(x, y),
            _ => false,
        };
        assert!(shared(&p, &copy));
        p.comp_mut(main).unwrap().hide_shy = true;
        assert!(!shared(&p, &copy));
        assert!(!copy.comp(main).unwrap().hide_shy);
    }

    #[test]
    fn saving_puts_the_whole_file_in_place() {
        let dir = std::env::temp_dir().join(format!("veymelo-project-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.veymelo");
        let p = sample();
        p.save(&path).unwrap();
        // footage written relative to the file is read back whole, from the file's folder
        let mut whole = p.clone();
        for it in &mut whole.items {
            if let ItemKind::Footage(f) = &mut it.kind {
                f.path = std::path::absolute(&dir).unwrap().join(&f.path).to_string_lossy().into_owned();
            }
        }
        let back = Project::open(&path, &Catalog::base()).unwrap();
        assert_eq!(back, whole);
        // saved again, still relative
        back.save(&path).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"path\": \"media/clip.mov\""));
        assert!(!dir.join("a.veymelo.saving").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
