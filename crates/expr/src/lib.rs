//! Expressions: a property worked out by a little program instead of (or on top of) its keys.
//!
//! The language is our own, written the way compositing tools' expressions are, so whoever knows
//! those can write these: `wiggle(2, 30)`, `loopOut("cycle")`, `value + [0, Math.sin(time * 3) * 20]`,
//! `thisComp.layer("Null 1").transform.position`, `linear(time, 0, 1, 0, 100)`. Arrays add and
//! scale part by part. The program's last value (or what it returns) is the property's value.
//!
//! [`Eval`] works out properties with their expressions, and where layers are (`matrix`), for
//! the renderer and for commands. An expression that fails, or that leads back to itself, leaves
//! its property as its keys make it, and says why in the notes.

mod host;
pub mod interp;
pub mod noise;
pub mod reference;
pub mod syntax;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use veymelo_geom::{Mat3, Mat4, Vec3, vec2, vec3};
use veymelo_keyframe::{Kind, TextDoc, Value};
use veymelo_project::{Applies, Comp, Layer, Project, Property};
use veymelo_time::Tick;

use interp::{Fail, Obj, Val, show};
use syntax::{Stmt, SyntaxError};

/// A program read from its text, or what is wrong with how it is written.
type Parsed = Result<Rc<Vec<Stmt>>, SyntaxError>;

/// Works out properties with their expressions, for one project.
pub struct Eval<'a> {
    pub project: &'a Project,
    notes: RefCell<Vec<String>>,
    /// The properties being worked out now (an expression that leads back to one of them stops).
    busy: RefCell<Vec<usize>>,
    parsed: RefCell<HashMap<String, Parsed>>,
    /// Names an expression may read besides the usual ones while one is worked out (a text
    /// selector's `textIndex`, `textTotal`, `selectorValue`).
    pub(crate) vars: RefCell<Vec<(String, Vec<f64>)>>,
    /// A composition seen other than through its own camera (the project unchanged).
    pub orbit: Option<Orbit>,
}

/// A way to see composition `comp` other than through its own camera, as a preview is looked
/// round: its camera (or the standard one) turned round the middle of what it sees, `turn`
/// degrees about the vertical and `tilt` up and down, and `dolly` times as far from it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub comp: u64,
    pub turn: f64,
    pub tilt: f64,
    pub dolly: f64,
}

/// What expressions may ask of the pictures, from whatever draws them: the box of what a layer
/// shows, and the colour of its pixels.
pub trait Pictures: Sync {
    /// The box of what layer `layer` (of composition `comp`) shows at composition time `t`, in
    /// its own units: [left, top, width, height].
    fn bounds(&self, project: &Project, comp: u64, layer: u64, t: Tick) -> Option<[f64; 4]>;
    /// The colour of layer `layer` round `point` of its own (an ellipse `radius` across and
    /// down), with its effects (`after`) or before them, at composition time `t`: [r, g, b, a]
    /// from 0 to 1, not premultiplied.
    #[allow(clippy::too_many_arguments)]
    fn sample(&self, project: &Project, comp: u64, layer: u64, point: [f64; 2], radius: [f64; 2], after: bool, t: Tick) -> Option<[f64; 4]>;
}

static PICTURES: OnceLock<&'static dyn Pictures> = OnceLock::new();

/// Let expressions see the pictures through `p` (what draws does, once).
pub fn show_pictures(p: &'static dyn Pictures) {
    let _ = PICTURES.set(p);
}

pub(crate) fn pictures() -> Option<&'static dyn Pictures> {
    PICTURES.get().copied()
}

/// Where a property is: its layer and that layer's composition.
#[derive(Clone, Copy)]
pub struct Place<'a> {
    pub layer: &'a Layer,
    pub comp: &'a Comp,
    pub comp_id: u64,
}

/// How deep expressions may lead into each other's properties.
const MAX_DEPTH: usize = 32;

impl<'a> Eval<'a> {
    pub fn new(project: &'a Project) -> Eval<'a> {
        Eval { project, notes: RefCell::new(Vec::new()), busy: RefCell::new(Vec::new()), parsed: RefCell::new(HashMap::new()), vars: RefCell::new(Vec::new()), orbit: None }
    }

    /// What went wrong in expressions so far, each once.
    pub fn notes(&self) -> Vec<String> {
        self.notes.borrow().clone()
    }

    fn note(&self, text: String) {
        let mut n = self.notes.borrow_mut();
        if !n.contains(&text) {
            n.push(text);
        }
    }

    /// The place of the layer with `id`, in whichever composition it is.
    pub fn place_of(&self, id: u64) -> Option<Place<'a>> {
        let (comp_id, i) = self.project.find_layer(id)?;
        let comp = self.project.comp(comp_id)?;
        Some(Place { layer: &comp.layers[i], comp, comp_id })
    }

    /// The place of `layer` in `comp` (a composition of this project).
    pub fn place(&self, comp: &'a Comp, layer: &'a Layer) -> Place<'a> {
        let comp_id = self.project.items.iter().find(|i| i.as_comp().is_some_and(|c| std::ptr::eq(c, comp))).map_or(0, |i| i.id);
        Place { layer, comp, comp_id }
    }

    /// A property's value at composition time `t`, its expression applied. When the expression
    /// fails, the value its keys give, and a note why.
    pub fn value(&self, prop: &'a Property, at: Place<'a>, t: Tick) -> Value {
        match self.work(prop, at, t) {
            Ok(v) => v,
            Err((base, why)) => {
                self.note(why);
                base
            }
        }
    }

    /// As [`Eval::value`], the expression reading `vars` too (a number, or a list of them).
    pub fn value_with(&self, prop: &'a Property, at: Place<'a>, t: Tick, vars: &[(&str, Vec<f64>)]) -> Value {
        let n = vars.len();
        self.vars.borrow_mut().extend(vars.iter().map(|(k, v)| (k.to_string(), v.clone())));
        let out = self.value(prop, at, t);
        let mut v = self.vars.borrow_mut();
        let keep = v.len().saturating_sub(n);
        v.truncate(keep);
        out
    }

    /// As [`Eval::work`], the expression reading `vars` too.
    pub fn work_with(&self, prop: &'a Property, at: Place<'a>, t: Tick, vars: &[(&str, Vec<f64>)]) -> Result<Value, (Value, String)> {
        let n = vars.len();
        self.vars.borrow_mut().extend(vars.iter().map(|(k, v)| (k.to_string(), v.clone())));
        let out = self.work(prop, at, t);
        let mut v = self.vars.borrow_mut();
        let keep = v.len().saturating_sub(n);
        v.truncate(keep);
        out
    }

    /// A property's value at composition time `t` with its expression, or what is wrong with
    /// the expression and the value its keys give.
    pub fn work(&self, prop: &'a Property, at: Place<'a>, t: Tick) -> Result<Value, (Value, String)> {
        let base = prop.value_at(at.layer.layer_time(t));
        let Some(e) = prop.expression.as_ref().filter(|e| e.enabled && !e.text.trim().is_empty()) else { return Ok(base) };
        let name = || format!("the expression on {} of \"{}\"", path_of(at.layer, prop), at.layer.name);
        let key = prop as *const Property as usize;
        if self.busy.borrow().contains(&key) {
            return Err((base, format!("{} leads back to itself; its keys' value is used", name())));
        }
        if self.busy.borrow().len() >= MAX_DEPTH {
            return Err((base, format!("{} leads through too many other expressions", name())));
        }
        let program = self.parsed.borrow_mut().entry(e.text.clone()).or_insert_with(|| syntax::parse(&e.text).map(Rc::new)).clone();
        let program = match program {
            Ok(p) => p,
            Err(err) => return Err((base, format!("{} is not written right at character {}: {}", name(), err.at, err.message))),
        };
        self.busy.borrow_mut().push(key);
        let got = host::run(self, prop, at, t, &program).and_then(|v| from_val(&v, prop.def.kind, &base));
        self.busy.borrow_mut().pop();
        let v = got.map_err(|why| (base.clone(), format!("{} failed: {why}", name())))?;
        match prop.def.check(&v) {
            Ok(()) => Ok(v),
            Err(why) => {
                // a number past a property's limits is held at them (an opacity of 130 is 100)
                let held = v.with_components(|k| {
                    let x = v.component(k);
                    if x.is_nan() { x } else { x.clamp(prop.def.min, prop.def.max) }
                });
                if prop.def.check(&held).is_ok() { Ok(held) } else { Err((base, format!("{} gives what it cannot hold: {why}", name()))) }
            }
        }
    }

    /// The value of the property at `path` of a layer, at composition time `t`.
    pub fn prop(&self, at: Place<'a>, path: &str, t: Tick) -> Option<Value> {
        let p = at.layer.props.prop(path).ok()?;
        Some(self.value(p, at, t))
    }

    /// A layer's position at composition time `t` (from its x, y and z when separate).
    pub fn position(&self, at: Place<'a>, t: Tick) -> [f64; 3] {
        if at.layer.separate_position {
            let n = |p: &str| self.prop(at, p, t).map_or(0.0, |v| v.as_f64());
            [n("transform/positionX"), n("transform/positionY"), n("transform/positionZ")]
        } else {
            self.prop(at, "transform/position", t).map_or([0.0; 3], |v| v.as_vec3())
        }
    }

    /// A layer's own transform at `t`: from its pixels to its parent's (or the composition's).
    pub fn local_matrix(&self, at: Place<'a>, t: Tick) -> Mat3 {
        let v3 = |p: &str, d: f64| self.prop(at, p, t).map_or([d; 3], |v| v.as_vec3());
        let (anchor, scale) = (v3("transform/anchor", 0.0), v3("transform/scale", 100.0));
        let p = self.position(at, t);
        let rotation = self.prop(at, "transform/rotation", t).map_or(0.0, |v| v.as_f64());
        Mat3::layer_2d(vec2(anchor[0], anchor[1]), vec2(p[0], p[1]), vec2(scale[0], scale[1]), rotation)
    }

    /// Where a layer's own pixels go in its composition at `t`: its transform, then its
    /// parent's, and so on up.
    pub fn matrix(&self, at: Place<'a>, t: Tick) -> Mat3 {
        let mut m = self.local_matrix(at, t);
        let mut up = at.layer.parent;
        // a ring of parents cannot be in a project, but stop anyway after every layer was seen
        for _ in 0..at.comp.layers.len() {
            let Some(p) = up.and_then(|id| at.comp.layer(id)) else { break };
            m = self.local_matrix(Place { layer: p, ..at }, t) * m;
            up = p.parent;
        }
        m
    }

    /// A layer's own 3D transform at `t`, from its space to its parent's (or the composition's
    /// world): a 3D layer by its anchor, scale, rotations and orientation (or the way it is
    /// auto-oriented: along its path, or toward `eye` when it faces the camera); a camera or
    /// light by its position and the way it faces (aimed at its point of interest when it is
    /// two-node); a 2D layer by its flat transform.
    pub fn local_3d(&self, at: Place<'a>, t: Tick, eye: Option<Vec3>) -> Mat4 {
        let layer = at.layer;
        let v3 = |p: &str, d: f64| self.prop(at, p, t).map_or([d; 3], |v| v.as_vec3());
        let n = |p: &str| self.prop(at, p, t).map_or(0.0, |v| v.as_f64());
        let scene = matches!(layer.source, veymelo_project::Source::Camera | veymelo_project::Source::Light(_));
        if !layer.is_3d() {
            return Mat4::from_mat3(&self.local_matrix(at, t));
        }
        let p = self.position(at, t);
        let pos = vec3(p[0], p[1], p[2]);
        let turns = Mat4::orientation(Vec3::from(v3("transform/orientation", 0.0))) * Mat4::rotate_z(n("transform/rotation")) * Mat4::rotate_y(n("transform/rotationY")) * Mat4::rotate_x(n("transform/rotationX"));
        if scene {
            let aim = match layer.auto_orient {
                veymelo_project::AutoOrient::TowardsPointOfInterest => facing(Vec3::from(v3("transform/pointOfInterest", 0.0)) - pos),
                _ => Mat4::IDENTITY,
            };
            return Mat4::translate(pos) * aim * turns;
        }
        let a = v3("transform/anchor", 0.0);
        let sc = v3("transform/scale", 100.0);
        // auto-orient takes the place of the orientation
        let orient = match layer.auto_orient {
            veymelo_project::AutoOrient::AlongPath => self.heading(at, t).map(along),
            veymelo_project::AutoOrient::TowardsCamera => eye.map(|e| {
                // the eye in the parent's space
                let back = self.parent_world(at, t, None).inverse().unwrap_or(Mat4::IDENTITY);
                facing(pos - back.apply(e))
            }),
            _ => None,
        };
        let rotations = Mat4::rotate_z(n("transform/rotation")) * Mat4::rotate_y(n("transform/rotationY")) * Mat4::rotate_x(n("transform/rotationX"));
        let orientation = orient.unwrap_or_else(|| Mat4::orientation(Vec3::from(v3("transform/orientation", 0.0))));
        Mat4::translate(pos) * orientation * rotations * Mat4::scale(Vec3::from(sc) / 100.0) * Mat4::translate(-Vec3::from(a))
    }

    /// Where a layer's own space goes in the composition's 3D world at `t`: its transform, then
    /// its parent's, and so on up.
    pub fn world(&self, at: Place<'a>, t: Tick, eye: Option<Vec3>) -> Mat4 {
        self.parent_world(at, t, eye) * self.local_3d(at, t, eye)
    }

    /// The camera a composition is seen through at `t`: from the world to its own space, the
    /// eye, its zoom, and its layer (none: the standard one, a 50 mm lens in front of the
    /// composition's middle, showing the z = 0 plane at its own size).
    pub fn camera(&self, comp: &'a Comp, t: Tick) -> (Mat4, Vec3, f64, Option<&'a Layer>) {
        let (view, eye, zoom, layer) = self.own_camera(comp, t);
        let Some(o) = self.orbit.filter(|o| self.project.comp(o.comp).is_some_and(|c| std::ptr::eq(c, comp)) && (o.turn != 0.0 || o.tilt != 0.0 || o.dolly != 1.0)) else { return (view, eye, zoom, layer) };
        let Some(back) = view.inverse() else { return (view, eye, zoom, layer) };
        // round the middle of what it sees (as far off as its zoom), about the world's vertical
        // and its own across
        let ahead = back.apply_vec(vec3(0.0, 0.0, 1.0)).normalize();
        let across = back.apply_vec(vec3(1.0, 0.0, 0.0)).normalize();
        let pivot = eye + ahead * zoom;
        let turn = (veymelo_geom::Quat::from_axis_deg(vec3(0.0, 1.0, 0.0), o.turn) * veymelo_geom::Quat::from_axis_deg(across, -o.tilt)).to_mat4();
        let at = pivot + turn.apply_vec(eye - pivot) * o.dolly.max(1e-3);
        let mut moved = turn * back;
        (moved.0[0][3], moved.0[1][3], moved.0[2][3]) = (at.x, at.y, at.z);
        (moved.inverse().unwrap_or(view), at, zoom, layer)
    }

    /// The composition's own camera at `t` (its top camera layer that is on and there, or the
    /// standard one): from the world to its space, where it is, its zoom, and its layer.
    fn own_camera(&self, comp: &'a Comp, t: Tick) -> (Mat4, Vec3, f64, Option<&'a Layer>) {
        let (w, h) = (comp.width as f64, comp.height as f64);
        let Some(layer) = comp.layers.iter().find(|l| l.source == veymelo_project::Source::Camera && l.switches.video && l.is_active_at(t)) else {
            let zoom = veymelo_project::scene::default_zoom(w);
            let eye = vec3(w / 2.0, h / 2.0, -zoom);
            return (Mat4::translate(-eye), eye, zoom, None);
        };
        let at = self.place(comp, layer);
        let world = self.world(at, t, None);
        let eye = world.apply(vec3(0.0, 0.0, 0.0));
        let zoom = self.prop(at, "cameraOptions/zoom", t).map_or(veymelo_project::scene::default_zoom(w), |v| v.as_f64()).max(1e-3);
        (world.inverse().unwrap_or(Mat4::translate(-eye)), eye, zoom, Some(layer))
    }

    /// The world transform of a layer's parent (none: as it is).
    pub fn parent_world(&self, at: Place<'a>, t: Tick, eye: Option<Vec3>) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        let mut up = at.layer.parent;
        for _ in 0..at.comp.layers.len() {
            let Some(p) = up.and_then(|id| at.comp.layer(id)) else { break };
            m = self.local_3d(Place { layer: p, ..at }, t, eye) * m;
            up = p.parent;
        }
        m
    }

    /// Which way a layer's position is moving at `t` (in its parent's space), or the way it last
    /// moved (or will first move) where it stands still.
    fn heading(&self, at: Place<'a>, t: Tick) -> Option<Vec3> {
        let pos = |t: Tick| Vec3::from(self.position(at, t));
        let dt = Tick::from_seconds(0.005);
        let dir = |d: Vec3| (d.length() > 1e-9 && d.length().is_finite()).then(|| d.normalize());
        if let Some(d) = dir(pos(t + dt) - pos(t - dt)) {
            return Some(d);
        }
        // standing: the last move before (from each earlier key, the latest first), or the first
        // move after
        let tr = at.layer.props.group("transform").ok()?;
        let mut keys: Vec<Tick> = ["position", "positionX", "positionY", "positionZ"].iter().filter_map(|m| tr.prop(m).ok()).flat_map(|p| p.keys.iter().map(|k| at.layer.comp_time(k.time))).collect();
        keys.sort();
        keys.dedup();
        keys.iter().rev().filter(|k| **k <= t).find_map(|&k| dir(pos(k) - pos(k - dt))).or_else(|| keys.iter().filter(|k| **k > t).find_map(|&k| dir(pos(k + dt) - pos(k))))
    }

    /// A layer's opacity at `t`, 0 to 1.
    pub fn opacity(&self, at: Place<'a>, t: Tick) -> f64 {
        self.prop(at, "transform/opacity", t).map_or(100.0, |v| v.as_f64()) / 100.0
    }

    /// The time of what a layer shows (a composition, footage) at composition time `t`: its
    /// time remap's value when that is on (else its own time, a composition's protected
    /// stretches kept at their speed).
    pub fn source_time(&self, at: Place<'a>, t: Tick) -> Tick {
        match at.layer.props.prop("timeRemap") {
            Ok(p) if at.layer.time_remap => Tick::from_seconds(self.value(p, at, t).as_f64()),
            _ => at.layer.shown_time(self.project, t),
        }
    }
}

/// Where a property is among its layer's, as commands write it (`transform/position`).
pub fn path_of(layer: &Layer, prop: &Property) -> String {
    let mut found = None;
    layer.props.walk(&mut |path, p| {
        if found.is_none() && std::ptr::eq(p, prop) {
            found = Some(path.to_string());
        }
    });
    found.unwrap_or_else(|| prop.def.id.to_string())
}

/// A property's value as the language sees it.
fn to_val<'a>(eval: &Eval<'a>, v: &Value) -> Val<'a> {
    match v {
        Value::Number(n) => Val::Num(*n),
        Value::Vec2(a) => Val::nums(a),
        Value::Vec3(a) => Val::nums(a),
        Value::Color(c) => Val::nums(&[c.r, c.g, c.b, c.a]),
        Value::Bool(b) => Val::Bool(*b),
        // menus count from 1, as in compositing tools
        Value::Choice(i) => Val::Num(*i as f64 + 1.0),
        Value::Text(s) => Val::Str(s.as_str().into()),
        Value::Doc(d) => Val::Str(d.text.as_str().into()),
        Value::Layer(Some(id)) => eval.place_of(*id).map_or(Val::Undef, |p| Val::Obj(Obj::Layer(p.layer, p.comp, p.comp_id))),
        Value::Path(p) => Val::Obj(Obj::Path(Rc::new(p.clone()))),
        Value::Layer(None) | Value::Gradient(_) => Val::Undef,
    }
}

/// A property's value of `layer` as the language sees it: on a 2D layer a point has two parts
/// (x and y), as compositing tools show it.
fn prop_val<'a>(eval: &Eval<'a>, prop: &Property, layer: &Layer, v: &Value) -> Val<'a> {
    match v {
        Value::Vec3(a) if !layer.is_3d() && prop.def.applies != Applies::ThreeD => Val::nums(&a[..2]),
        _ => to_val(eval, v),
    }
}

/// What a program gave, as a value of the property's kind (`base` is the value its keys give).
fn from_val(v: &Val, kind: Kind, base: &Value) -> Result<Value, Fail> {
    let nums: Option<Vec<f64>> = match v {
        Val::Num(n) => Some(vec![*n]),
        Val::Bool(b) => Some(vec![if *b { 1.0 } else { 0.0 }]),
        Val::Arr(items) => items.iter().map(|x| if let Val::Num(n) = x { Some(*n) } else { None }).collect(),
        _ => None,
    };
    let what = || match v {
        Val::Arr(items) => format!("an array of {}", items.len()),
        other => other.kind().to_string(),
    };
    let wrong = || format!("it gives {}, and {} is wanted", what(), kind.shape());
    // a number for one, an array of at least `n` for more
    let parts = |n: usize| nums.as_ref().filter(|p| if n == 1 { p.len() == 1 } else { matches!(v, Val::Arr(_)) && p.len() >= n }).ok_or_else(wrong);
    Ok(match kind {
        Kind::Number => Value::Number(parts(1)?[0]),
        Kind::Vec2 => {
            let n = parts(2)?;
            Value::Vec2([n[0], n[1]])
        }
        Kind::Vec3 => {
            // a point of two keeps its depth
            let n = parts(2)?;
            Value::Vec3([n[0], n[1], n.get(2).copied().unwrap_or(base.as_vec3()[2])])
        }
        Kind::Color => {
            let n = parts(3)?;
            Value::Color(veymelo_color::Rgba { r: n[0], g: n[1], b: n[2], a: n.get(3).copied().unwrap_or(1.0) })
        }
        Kind::Bool => Value::Bool(v.truthy()),
        Kind::Choice(options) => {
            let n = parts(1)?[0].round();
            if !(n >= 1.0 && n <= options.len() as f64) {
                return Err(format!("it gives {}; this menu goes from 1 to {} ({})", show(v), options.len(), options.join(", ")));
            }
            Value::Choice(n as u32 - 1)
        }
        Kind::Text => Value::Text(show(v)),
        // new words keep the look of the words they replace
        Kind::Doc => match base.doc() {
            Some(d) => {
                let d = TextDoc { text: show(v), ..d };
                if d == TextDoc::of(&d.text) { Value::Text(d.text) } else { Value::Doc(Box::new(d)) }
            }
            None => Value::Text(show(v)),
        },
        Kind::Layer => match v {
            Val::Obj(Obj::Layer(l, ..)) => Value::Layer(Some(l.id)),
            Val::Undef => Value::Layer(None),
            _ => return Err(wrong()),
        },
        Kind::Path => match v {
            Val::Obj(Obj::Path(p)) => Value::Path((**p).clone()),
            _ => return Err(format!("it gives {}, and a path is wanted (createPath makes one)", what())),
        },
        Kind::Gradient => return Err(format!("an expression cannot give {} yet", kind.shape())),
    })
}

#[cfg(test)]
mod tests;

/// A turn whose z points along `forward` and whose y is as near down (+y) as can be.
pub fn facing(forward: Vec3) -> Mat4 {
    let f = if forward.length() < 1e-12 { vec3(0.0, 0.0, 1.0) } else { forward.normalize() };
    let mut d = vec3(0.0, 1.0, 0.0) - f * f.y;
    if d.length() < 1e-9 {
        d = vec3(0.0, 0.0, -1.0) - f * f.z;
    }
    let d = d.normalize();
    let r = d.cross(f).normalize();
    Mat4([[r.x, d.x, f.x, 0.0], [r.y, d.y, f.y, 0.0], [r.z, d.z, f.z, 0.0], [0.0, 0.0, 0.0, 1.0]])
}

/// A turn whose x points along `tangent` (a 3D layer oriented along its path).
fn along(tangent: Vec3) -> Mat4 {
    let x = tangent.normalize();
    let mut y = vec3(0.0, 1.0, 0.0) - x * x.y;
    if y.length() < 1e-9 {
        y = vec3(0.0, 0.0, 1.0) - x * x.z;
    }
    let y = y.normalize();
    let z = x.cross(y);
    Mat4([[x.x, y.x, z.x, 0.0], [x.y, y.y, z.y, 0.0], [x.z, y.z, z.z, 0.0], [0.0, 0.0, 0.0, 1.0]])
}
