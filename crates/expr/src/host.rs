//! One expression running: what its names mean (`time`, `value`, `thisLayer`…), what the
//! objects of the project have, and the built-in functions.

use std::cell::Cell;
use std::rc::Rc;

use veymelo_geom::vec2;
use veymelo_project::{Comp, Group, Layer, Marker, Node, Property, Source};
use veymelo_time::Tick;

use crate::interp::{Fail, Fun, Host, Obj, Run, Val, child, show};
use crate::noise::{self, mix, of_text, unit};
use crate::syntax::Stmt;
use crate::{Eval, Place, path_of, prop_val};

/// Run `program` as the expression of `prop`, at composition time `t`.
pub(crate) fn run<'a>(eval: &Eval<'a>, prop: &'a Property, at: Place<'a>, t: Tick, program: &[Stmt]) -> Result<Val<'a>, Fail> {
    let ctx = Ctx { eval, prop, at, time: Cell::new(t), seed: Cell::new(0), timeless: Cell::new(false), draws: Cell::new(0) };
    let run = Run::new(&ctx);
    let v = run.program(program)?;
    ctx.resolve(v)
}

/// The functions that can be called by name alone.
const BUILTINS: &[&str] = &[
    "clamp", "linear", "ease", "easeIn", "easeOut", "random", "gaussRandom", "seedRandom", "noise", "wiggle", "temporalWiggle", "smooth", "loopOut", "loopIn",
    "loopOutDuration", "loopInDuration", "valueAtTime", "velocityAtTime", "speedAtTime", "nearestKey", "key", "add", "sub", "mul", "div", "dot", "cross",
    "normalize", "length", "lookAt", "degreesToRadians", "radiansToDegrees", "timeToFrames", "framesToTime", "timeToTimecode", "rgbToHsl", "hslToRgb",
    "hexToRgb", "posterizeTime", "comp", "parseFloat", "parseInt", "isNaN", "isFinite", "String", "Number", "createPath",
];

/// What a path (a path property's value, or one createPath made) answers; on a property each
/// takes a time last (its value then).
const PATH_METHODS: &[&str] = &["points", "inTangents", "outTangents", "isClosed", "pointOnPath", "tangentOnPath", "normalOnPath"];

const MATH: &[&str] = &[
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "sqrt", "cbrt", "pow", "abs", "sign", "floor", "ceil", "round", "trunc",
    "min", "max", "exp", "log", "log2", "log10", "hypot", "random",
];

/// The methods a property has (the rest it lends to its value: `position.toString()`).
const PROP_METHODS: &[&str] =
    &["valueAtTime", "velocityAtTime", "speedAtTime", "key", "nearestKey", "wiggle", "temporalWiggle", "smooth", "loopOut", "loopIn", "loopOutDuration", "loopInDuration"];

/// One expression running.
struct Ctx<'e, 'a> {
    eval: &'e Eval<'a>,
    prop: &'a Property,
    at: Place<'a>,
    /// Composition time (posterizeTime can change it).
    time: Cell<Tick>,
    /// seedRandom's seed, whether it holds through time, and how many numbers were drawn.
    seed: Cell<u64>,
    timeless: Cell<bool>,
    draws: Cell<u64>,
}

/// Every number of a value.
fn parts(v: &Val) -> Option<Vec<f64>> {
    match v {
        Val::Num(n) => Some(vec![*n]),
        Val::Bool(b) => Some(vec![if *b { 1.0 } else { 0.0 }]),
        Val::Arr(items) => items.iter().map(|x| if let Val::Num(n) = x { Some(*n) } else { None }).collect(),
        _ => None,
    }
}

/// Numbers shaped like `like`: one number for a number, else an array.
fn shaped<'a>(like: &Val, v: Vec<f64>) -> Val<'a> {
    if matches!(like, Val::Num(_) | Val::Bool(_)) && v.len() == 1 { Val::Num(v[0]) } else { Val::nums(&v) }
}

/// The time `n` frames are, or how many frames a time is.
fn frames(t: f64, fps: f64, duration: bool) -> f64 {
    let f = t * fps;
    // floating point can leave a whole number of frames a hair short
    if duration && f != 0.0 {
        if f < 0.0 { (f + 1e-6).floor() } else { (f - 1e-6).ceil() }
    } else {
        (f + 1e-6).floor()
    }
}

/// A node of a layer's properties as a value.
fn node_val<'a>(n: &'a Node, at: Place<'a>) -> Val<'a> {
    match n {
        Node::Prop(p) => Val::Obj(Obj::Prop(p, at.layer, at.comp, at.comp_id)),
        Node::Group(g) => Val::Obj(Obj::Group(g, at.layer, at.comp, at.comp_id)),
    }
}

/// The groups from `root` (left out) down to the one holding `target` (a group or a property),
/// outermost first; none if it is not in there.
fn groups_to(root: &Group, target: *const ()) -> Option<Vec<&Group>> {
    for n in &root.children {
        match n {
            Node::Prop(p) if std::ptr::eq((p as *const Property).cast(), target) => return Some(Vec::new()),
            Node::Group(g) if std::ptr::eq((g as *const Group).cast(), target) => return Some(Vec::new()),
            Node::Group(g) => {
                if let Some(mut inner) = groups_to(g, target) {
                    inner.insert(0, g);
                    return Some(inner);
                }
            }
            _ => {}
        }
    }
    None
}

/// Where `target` is in the group holding it, from 1.
fn index_in(root: &Group, target: *const ()) -> Option<usize> {
    let chain = groups_to(root, target)?;
    let holder = chain.last().copied().unwrap_or(root);
    holder.children.iter().position(|n| match n {
        Node::Prop(p) => std::ptr::eq((p as *const Property).cast(), target),
        Node::Group(g) => std::ptr::eq((g as *const Group).cast(), target),
    }).map(|i| i + 1)
}

/// What a group holds, by name or by number (from 1).
fn group_child<'a>(g: &'a Group, at: Place<'a>, key: &Val<'a>) -> Result<Val<'a>, Fail> {
    let found = match key {
        Val::Str(name) => child(g, name),
        Val::Num(k) => (*k as usize).checked_sub(1).and_then(|i| g.children.get(i)),
        _ => None,
    };
    found.map(|n| node_val(n, at)).ok_or_else(|| {
        let have: Vec<&str> = g.children.iter().map(Node::name).collect();
        format!("{} has no {} (it has {})", g.name(), show(key), if have.is_empty() { "nothing".into() } else { have.join(", ") })
    })
}

impl<'e, 'a> Ctx<'e, 'a> {
    fn secs(&self) -> f64 {
        self.time.get().seconds()
    }

    fn fps(&self) -> f64 {
        1.0 / self.at.comp.frame_rate.frame_duration().seconds()
    }

    /// A seed of a property's own, the same however the project is copied or saved.
    fn seed_of(&self, layer: &Layer, prop: &Property) -> u64 {
        mix(&[layer.id, of_text(&path_of(layer, prop))])
    }

    /// The next random number from 0 to 1: new at every frame (unless seedRandom said
    /// timeless), new at every draw, its own for each property.
    fn draw(&self) -> f64 {
        let n = self.draws.get();
        self.draws.set(n + 1);
        let when = if self.timeless.get() { 0 } else { self.time.get().0 as u64 };
        unit(mix(&[self.seed_of(self.at.layer, self.prop), self.seed.get(), when, n]))
    }

    /// Most numbers fall from 0 to 1 (nine in ten), more near the middle.
    /// A draw from a bell round 0.5, nine in ten of them within 0 to 1: two plain draws made one
    /// normal one (Box and Muller's way), spread so that 0.5 is as far as the 95th percentile.
    fn draw_gauss(&self) -> f64 {
        // the normal distribution's 95th percentile, in standard deviations
        const Z95: f64 = 1.644_853_626_951_472_2;
        let (u, v) = (self.draw().max(1e-12), self.draw());
        0.5 + (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos() * (0.5 / Z95)
    }

    fn index_of(&self, layer: &Layer, comp: &Comp) -> f64 {
        comp.index_of(layer.id).map_or(0.0, |i| (i + 1) as f64)
    }

    /// A property's value at composition time `t`: this expression's own property as its keys
    /// make it (so it cannot lead back to itself), any other with its expression.
    fn read(&self, p: &'a Property, at: Place<'a>, t: Tick) -> Val<'a> {
        if std::ptr::eq(p, self.prop) {
            return prop_val(self.eval, p, at.layer, &p.value_at(at.layer.layer_time(t)));
        }
        // a separated position reads as its x, y and z
        if at.layer.separate_position && at.layer.props.prop("transform/position").is_ok_and(|q| std::ptr::eq(q, p)) {
            return prop_val(self.eval, p, at.layer, &veymelo_keyframe::Value::Vec3(self.eval.position(at, t)));
        }
        prop_val(self.eval, p, at.layer, &self.eval.value(p, at, t))
    }

    /// How fast a property changes at `t`, per second.
    fn velocity(&self, p: &'a Property, at: Place<'a>, t: Tick) -> Val<'a> {
        let h = 1e-3;
        let (a, b) = (self.read(p, at, t - Tick::from_seconds(h)), self.read(p, at, t + Tick::from_seconds(h)));
        let (pa, pb) = (parts(&a).unwrap_or_default(), parts(&b).unwrap_or_default());
        shaped(&a, pa.iter().zip(&pb).map(|(x, y)| (y - x) / (2.0 * h)).collect())
    }

    fn speed(&self, p: &'a Property, at: Place<'a>, t: Tick) -> f64 {
        parts(&self.velocity(p, at, t)).map_or(0.0, |d| d.iter().map(|x| x * x).sum::<f64>().sqrt())
    }

    /// loopOut and loopIn: the keys' motion repeated after the last key (or before the first),
    /// over the last (first) `keys` keys, or `duration` seconds, or all of them.
    #[allow(clippy::too_many_arguments)]
    fn looped(&self, run: &Run<'a, '_>, p: &'a Property, at: Place<'a>, kind: &str, keys: usize, duration: Option<f64>, after: bool) -> Result<Val<'a>, Fail> {
        let t = at.layer.layer_time(self.time.get());
        let own = |t: Tick| prop_val(self.eval, p, at.layer, &p.value_at(t));
        let k = &p.keys;
        let (Some(first), Some(last)) = (k.first().map(|k| k.time), k.last().map(|k| k.time)) else { return Ok(own(t)) };
        if (after && t <= last) || (!after && t >= first) {
            return Ok(own(t));
        }
        let (a, b) = match (duration.filter(|d| *d > 0.0), after) {
            (Some(d), true) => (last - Tick::from_seconds(d), last),
            (Some(d), false) => (first, first + Tick::from_seconds(d)),
            (None, true) => (if keys > 0 && keys < k.len() { k[k.len() - 1 - keys].time } else { first }, last),
            (None, false) => (first, if keys > 0 && keys < k.len() { k[keys].time } else { last }),
        };
        let kind = kind.to_ascii_lowercase();
        if kind == "continue" {
            // on at the speed it had at its end, measured inside the keys
            let h = Tick::from_seconds(1e-3);
            let (edge, inside) = if after { (b, b - h) } else { (a, a + h) };
            let speed = run.binary("/", run.binary("-", own(edge), own(inside))?, Val::Num(if after { h.seconds() } else { -h.seconds() }))?;
            return run.binary("+", own(edge), run.binary("*", speed, Val::Num((t - edge).seconds()))?);
        }
        let period = (b - a).0;
        if period <= 0 {
            return Ok(own(t));
        }
        // how many whole loops from the start of the stretch, and how far into this one
        let from = (t - a).0;
        let (cycles, phase) = (from.div_euclid(period), Tick(from.rem_euclid(period)));
        Ok(match kind.as_str() {
            "pingpong" => own(if cycles.rem_euclid(2) == 1 { b - phase } else { a + phase }),
            "offset" => {
                let change = run.binary("-", own(b), own(a))?;
                run.binary("+", own(a + phase), run.binary("*", change, Val::Num(cycles as f64))?)?
            }
            "cycle" => own(a + phase),
            other => return Err(format!("\"{other}\" is not a way to loop (cycle, pingpong, offset or continue)")),
        })
    }

    fn loop_call(&self, run: &Run<'a, '_>, p: &'a Property, at: Place<'a>, name: &str, args: &[Val<'a>]) -> Result<Val<'a>, Fail> {
        let kind = match args.first().map(|v| self.resolve(v.clone())).transpose()? {
            Some(Val::Str(s)) => s.to_string(),
            None | Some(Val::Undef) => "cycle".into(),
            Some(other) => return Err(format!("{name} takes a way to loop, like \"cycle\", not {}", other.kind())),
        };
        let n = args.get(1).map_or(Ok(0.0), |v| run.number(v.clone()))?;
        let by_time = name.ends_with("Duration");
        self.looped(run, p, at, &kind, if by_time { 0 } else { n.max(0.0) as usize }, by_time.then_some(n), name.starts_with("loopOut"))
    }

    /// wiggle(freq, amount, octaves = 1, ampMult = 0.5, t = time): the value shaken.
    fn wiggle(&self, run: &Run<'a, '_>, p: &'a Property, at: Place<'a>, args: &[Val<'a>], temporal: bool) -> Result<Val<'a>, Fail> {
        let n = |k: usize, d: f64| args.get(k).map_or(Ok(d), |v| run.number(v.clone()));
        let (freq, amp, octaves, amp_mult) = (n(0, 1.0)?, n(1, 0.0)?, n(2, 1.0)?, n(3, 0.5)?);
        let t = match args.get(4) {
            Some(v) => Tick::from_seconds(run.number(v.clone())?),
            None => self.time.get(),
        };
        let seed = self.seed_of(at.layer, p);
        if temporal {
            // shaken in time: the value from a little before or after
            let off = noise::wiggle(seed, 1, freq, amp, octaves, amp_mult, t.seconds())[0];
            return Ok(self.read(p, at, t + Tick::from_seconds(off)));
        }
        let base = self.read(p, at, t);
        let Some(v) = parts(&base) else { return Err(format!("{} cannot wiggle", base.kind())) };
        let off = noise::wiggle(seed, v.len(), freq, amp, octaves, amp_mult, t.seconds());
        Ok(shaped(&base, v.iter().zip(&off).map(|(x, d)| x + d).collect()))
    }

    /// smooth(width = 0.2, samples = 5, t = time): the value averaged over `width` seconds.
    fn smooth(&self, run: &Run<'a, '_>, p: &'a Property, at: Place<'a>, args: &[Val<'a>]) -> Result<Val<'a>, Fail> {
        let n = |k: usize, d: f64| args.get(k).map_or(Ok(d), |v| run.number(v.clone()));
        let (width, samples, t) = (n(0, 0.2)?, n(1, 5.0)?.round().clamp(1.0, 1000.0) as usize, n(2, self.secs())?);
        if samples == 1 {
            return Ok(self.read(p, at, Tick::from_seconds(t)));
        }
        let mut sum: Option<Val<'a>> = None;
        for i in 0..samples {
            let v = self.read(p, at, Tick::from_seconds(t - width / 2.0 + width * i as f64 / (samples - 1) as f64));
            sum = Some(match sum {
                None => v,
                Some(s) => run.binary("+", s, v)?,
            });
        }
        run.binary("/", sum.unwrap_or(Val::Num(0.0)), Val::Num(samples as f64))
    }

    /// A property's own methods.
    fn prop_method(&self, run: &Run<'a, '_>, p: &'a Property, at: Place<'a>, name: &str, args: &[Val<'a>]) -> Result<Val<'a>, Fail> {
        let num = |k: usize, d: f64| args.get(k).map_or(Ok(d), |v| run.number(v.clone()));
        let (l, c, cid) = (at.layer, at.comp, at.comp_id);
        match name {
            "valueAtTime" => Ok(self.read(p, at, Tick::from_seconds(num(0, self.secs())?))),
            "velocityAtTime" => Ok(self.velocity(p, at, Tick::from_seconds(num(0, self.secs())?))),
            "speedAtTime" => Ok(Val::Num(self.speed(p, at, Tick::from_seconds(num(0, self.secs())?)))),
            "key" => {
                let k = num(0, 1.0)?.floor();
                if !(k >= 1.0 && k <= p.keys.len() as f64) {
                    return Err(format!("{} has {} keys, so there is no key({k})", p.def.id, p.keys.len()));
                }
                Ok(Val::Obj(Obj::Key(p, l, c, cid, k as usize - 1)))
            }
            "nearestKey" => {
                let t = l.layer_time(Tick::from_seconds(num(0, self.secs())?));
                let k = p.keys.iter().enumerate().min_by_key(|(_, key)| (key.time - t).abs()).map(|(k, _)| k).ok_or_else(|| format!("{} has no keys", p.def.id))?;
                Ok(Val::Obj(Obj::Key(p, l, c, cid, k)))
            }
            "wiggle" | "temporalWiggle" => self.wiggle(run, p, at, args, name == "temporalWiggle"),
            "smooth" => self.smooth(run, p, at, args),
            _ => self.loop_call(run, p, at, name, args),
        }
    }

    fn layer_member(&self, l: &'a Layer, c: &'a Comp, cid: u64, name: &str) -> Result<Val<'a>, Fail> {
        let at = Place { layer: l, comp: c, comp_id: cid };
        let secs = |t: Tick| Val::Num(t.seconds());
        Ok(match name {
            "name" => Val::Str(l.name.as_str().into()),
            "index" => Val::Num(self.index_of(l, c)),
            "inPoint" => secs(l.in_point),
            "outPoint" => secs(l.out_point),
            "startTime" => secs(l.start),
            "stretch" => Val::Num(l.stretch),
            "enabled" => Val::Bool(l.switches.video),
            "active" => Val::Bool(l.switches.video && l.is_active_at(self.time.get())),
            "hasParent" => Val::Bool(l.parent.is_some()),
            "parent" => match l.parent.and_then(|p| c.layer(p)) {
                Some(p) => Val::Obj(Obj::Layer(p, c, cid)),
                None => return Err(format!("\"{}\" has no parent (hasParent says whether a layer has one)", l.name)),
            },
            "source" => match l.source {
                Source::Comp(id) => match self.eval.project.comp(id) {
                    Some(inner) => Val::Obj(Obj::Comp(inner, id)),
                    None => Val::Undef,
                },
                _ => return Err(format!("\"{}\" shows no composition", l.name)),
            },
            "width" | "height" => {
                let size = match &l.source {
                    Source::Solid { width, height, .. } => (*width as f64, *height as f64),
                    Source::Comp(id) => self.eval.project.comp(*id).map_or((0.0, 0.0), |c| (c.width as f64, c.height as f64)),
                    _ => (c.width as f64, c.height as f64),
                };
                Val::Num(if name == "width" { size.0 } else { size.1 })
            }
            "marker" => Val::Obj(Obj::Markers(&l.markers, Some(l))),
            "effect" | "mask" | "content" | "toComp" | "fromComp" | "toWorld" | "fromWorld" | "toCompVec" | "fromCompVec" | "toWorldVec" | "fromWorldVec" | "sourceRectAtTime" | "sampleImage" => Val::Fun(Rc::new(Fun::Method(Val::Obj(Obj::Layer(l, c, cid)), name.into()))),
            _ => {
                // the layer's own groups and properties, then its transform's and its text's
                let found = child(&l.props, name).or_else(|| ["transform", "text"].iter().find_map(|g| l.props.group(g).ok().and_then(|g| child(g, name))));
                match found {
                    Some(n) => node_val(n, at),
                    None => return Err(format!("a layer has no \"{name}\"")),
                }
            }
        })
    }

    fn layer_method(&self, run: &Run<'a, '_>, l: &'a Layer, c: &'a Comp, cid: u64, name: &str, args: &[Val<'a>]) -> Result<Val<'a>, Fail> {
        let at = Place { layer: l, comp: c, comp_id: cid };
        let arg = |k: usize| args.get(k).cloned().map_or(Ok(Val::Undef), |v| self.resolve(v));
        match name {
            "effect" | "mask" | "content" => {
                let list = match name {
                    "effect" => "effects",
                    "mask" => "masks",
                    _ => "contents",
                };
                let g = l.props.group(list).map_err(|_| format!("\"{}\" has no {list}", l.name))?;
                group_child(g, at, &arg(0)?)
            }
            // the box of what it shows, in its own units
            "sourceRectAtTime" => {
                let t = match args.first() {
                    Some(v) => Tick::from_seconds(run.number(self.resolve(v.clone())?)?),
                    None => self.time.get(),
                };
                let r = crate::pictures().and_then(|p| p.bounds(self.eval.project, cid, l.id, t)).unwrap_or_else(|| {
                    let (w, h) = match &l.source {
                        Source::Solid { width, height, .. } => (*width as f64, *height as f64),
                        Source::Comp(id) => self.eval.project.comp(*id).map_or((0.0, 0.0), |c| (c.width as f64, c.height as f64)),
                        _ => (0.0, 0.0),
                    };
                    [0.0, 0.0, w, h]
                });
                Ok(Val::Obj(Obj::Rect(r)))
            }
            // the colour round a point of it
            "sampleImage" => {
                let p = parts(&arg(0)?).filter(|p| p.len() >= 2).ok_or("sampleImage takes a point of the layer, like [100, 50]")?;
                let radius = match arg(1)? {
                    Val::Undef => [0.5, 0.5],
                    v => parts(&v).filter(|r| r.len() >= 2).map(|r| [r[0], r[1]]).ok_or("sampleImage's radius is [across, down]")?,
                };
                let after = match arg(2)? {
                    Val::Undef => true,
                    v => v.truthy(),
                };
                let t = match args.get(3) {
                    Some(v) => Tick::from_seconds(run.number(self.resolve(v.clone())?)?),
                    None => self.time.get(),
                };
                let pictures = crate::pictures().ok_or("sampleImage reads pixels, which only drawing has")?;
                let c = pictures.sample(self.eval.project, cid, l.id, [p[0], p[1]], radius, after, t).ok_or_else(|| format!("\"{}\" has no pixels there to sample", l.name))?;
                Ok(Val::nums(&c))
            }
            // a direction, not a point: where it takes the point less where it takes nothing
            "toCompVec" | "fromCompVec" | "toWorldVec" | "fromWorldVec" => {
                let base = name.trim_end_matches("Vec");
                let v = arg(0)?;
                let mut rest: Vec<Val<'a>> = args.iter().skip(1).cloned().collect();
                let mut with = |p: Val<'a>| -> Result<Vec<f64>, Fail> {
                    rest.insert(0, p);
                    let out = self.layer_method(run, l, c, cid, base, &rest);
                    rest.remove(0);
                    parts(&out?).ok_or_else(|| format!("{name} gives no point"))
                };
                let (a, o) = (with(v)?, with(Val::nums(&[0.0, 0.0, 0.0]))?);
                Ok(Val::nums(&a.iter().zip(&o).map(|(x, y)| x - y).collect::<Vec<_>>()))
            }
            "toComp" | "toWorld" | "fromComp" | "fromWorld" => {
                let p = parts(&arg(0)?).filter(|p| p.len() >= 2).ok_or_else(|| format!("{name} takes a point, like [100, 50]"))?;
                let t = Tick::from_seconds(args.get(1).map_or(Ok(self.secs()), |v| run.number(v.clone()))?);
                let z = p.get(2).copied().unwrap_or(0.0);
                if l.is_3d() || name.ends_with("World") {
                    let (view, eye, zoom, _) = self.eval.camera(c, t);
                    let world = self.eval.world(at, t, Some(eye));
                    let (w, h) = (c.width as f64, c.height as f64);
                    let q = match name {
                        "toWorld" => world.apply(veymelo_geom::vec3(p[0], p[1], z)),
                        "fromWorld" => world.inverse().ok_or_else(|| format!("\"{}\" is scaled to nothing", l.name))?.apply(veymelo_geom::vec3(p[0], p[1], z)),
                        // through the camera to the composition's pixels
                        "toComp" => {
                            let cam = view.apply(world.apply(veymelo_geom::vec3(p[0], p[1], z)));
                            if cam.z <= 1e-9 {
                                return Err(format!("that point of \"{}\" is behind the camera", l.name));
                            }
                            veymelo_geom::vec3(zoom * cam.x / cam.z + w / 2.0, zoom * cam.y / cam.z + h / 2.0, cam.z)
                        }
                        // the composition pixel's point on the layer's plane
                        _ => {
                            let back = view.inverse().unwrap_or(veymelo_geom::Mat4::IDENTITY);
                            let ray = back.apply_vec(veymelo_geom::vec3((p[0] - w / 2.0) / zoom, (p[1] - h / 2.0) / zoom, 1.0));
                            let (o, n) = (world.apply(veymelo_geom::vec3(0.0, 0.0, 0.0)), world.apply_vec(veymelo_geom::vec3(1.0, 0.0, 0.0)).cross(world.apply_vec(veymelo_geom::vec3(0.0, 1.0, 0.0))));
                            let facing = ray.dot(n);
                            if facing.abs() < 1e-12 {
                                return Err(format!("\"{}\" is seen edge-on there", l.name));
                            }
                            let hit = eye + ray * ((o - eye).dot(n) / facing);
                            world.inverse().unwrap_or(veymelo_geom::Mat4::IDENTITY).apply(hit)
                        }
                    };
                    return Ok(Val::nums(&[q.x, q.y, q.z]));
                }
                let m = self.eval.matrix(at, t);
                let m = if name.starts_with("from") { m.inverse().ok_or_else(|| format!("\"{}\" is scaled to nothing", l.name))? } else { m };
                let q = m.apply(vec2(p[0], p[1]));
                Ok(Val::nums(&[q.x, q.y, z]))
            }
            _ => Err(format!("a layer cannot {name}")),
        }
    }

    fn comp_layer(&self, c: &'a Comp, cid: u64, args: &[Val<'a>]) -> Result<Val<'a>, Fail> {
        let what = args.first().cloned().map_or(Ok(Val::Undef), |v| self.resolve(v))?;
        let found = match &what {
            Val::Num(k) => (*k as usize).checked_sub(1).and_then(|i| c.layers.get(i)),
            Val::Str(n) => c.layers.iter().find(|l| *l.name == **n),
            // a layer and how many places above (below zero) or under it
            Val::Obj(Obj::Layer(l, ..)) => {
                let off = match args.get(1) {
                    Some(v) => self.resolve(v.clone())?,
                    None => Val::Num(0.0),
                };
                let Val::Num(off) = off else { return Err("layer(other, n) takes a number of places".into()) };
                c.index_of(l.id).and_then(|i| usize::try_from(i as i64 + off as i64).ok()).and_then(|i| c.layers.get(i))
            }
            _ => None,
        };
        found.map(|l| Val::Obj(Obj::Layer(l, c, cid))).ok_or_else(|| {
            let names: Vec<&str> = c.layers.iter().map(|l| l.name.as_str()).collect();
            format!("this composition has no layer {} (it has {} layers: {})", show(&what), c.layers.len(), names.join(", "))
        })
    }

    /// When a marker is, in composition time.
    fn marker_time(&self, m: &Marker, l: Option<&Layer>) -> Tick {
        l.map_or(m.time, |l| l.comp_time(m.time))
    }

    fn builtin(&self, run: &Run<'a, '_>, name: &str, args: Vec<Val<'a>>) -> Result<Val<'a>, Fail> {
        let arg = |k: usize| args.get(k).cloned().map_or(Ok(Val::Undef), |v| self.resolve(v));
        let num = |k: usize, d: f64| -> Result<f64, Fail> {
            match arg(k)? {
                Val::Undef => Ok(d),
                v => run.number(v),
            }
        };
        let vec = |k: usize, what: &str| parts(&arg(k)?).ok_or_else(|| format!("{name} takes {what}"));
        match name {
            n if PROP_METHODS.contains(&n) => self.prop_method(run, self.prop, self.at, n, &args),
            // createPath(points, inTangents, outTangents, closed): the tangents none, the path closed, unless given
            "createPath" => {
                let a: Vec<Val<'a>> = (0..4).map(arg).collect::<Result<_, Fail>>()?;
                let points = pairs(run, a.first(), None)?;
                let ins = pairs(run, a.get(1), Some(points.len()))?;
                let outs = pairs(run, a.get(2), Some(points.len()))?;
                let closed = !matches!(a.get(3), Some(v) if !matches!(v, Val::Undef) && !v.truthy());
                Ok(Val::Obj(Obj::Path(Rc::new(veymelo_keyframe::ShapePath { points, in_tangents: ins, out_tangents: outs, closed, feather: veymelo_keyframe::Feathers::NONE }))))
            }
            n if PATH_METHODS.contains(&n) && self.prop.def.kind == veymelo_keyframe::Kind::Path => {
                let me = Val::Obj(Obj::Prop(self.prop, self.at.layer, self.at.comp, self.at.comp_id));
                self.call_method(run, me, n, args)
            }
            "posterizeTime" => {
                let fps = num(0, 0.0)?;
                if fps > 0.0 {
                    self.time.set(Tick::from_seconds((self.secs() * fps + 1e-6).floor() / fps));
                }
                Ok(Val::Undef)
            }
            "seedRandom" => {
                self.seed.set(num(0, 0.0)?.to_bits());
                self.timeless.set(arg(1)?.truthy());
                self.draws.set(0);
                Ok(Val::Undef)
            }
            "random" | "gaussRandom" => {
                let draw = || if name == "random" { self.draw() } else { self.draw_gauss() };
                match (arg(0)?, arg(1)?) {
                    (Val::Undef, _) => Ok(Val::Num(draw())),
                    (hi, Val::Undef) => {
                        let h = parts(&hi).ok_or("random takes numbers")?;
                        Ok(shaped(&hi, h.iter().map(|x| x * draw()).collect()))
                    }
                    (lo, hi) => {
                        let (l, h) = (parts(&lo).ok_or("random takes numbers")?, parts(&hi).ok_or("random takes numbers")?);
                        let n = l.len().max(h.len());
                        let pick = |v: &[f64], k: usize| if v.len() == 1 { v[0] } else { v.get(k).copied().unwrap_or(0.0) };
                        let like = if matches!(lo, Val::Arr(_)) { &lo } else { &hi };
                        Ok(shaped(like, (0..n).map(|k| pick(&l, k) + (pick(&h, k) - pick(&l, k)) * draw()).collect()))
                    }
                }
            }
            "noise" => {
                let p = vec(0, "a number or a point")?;
                Ok(Val::Num(noise::noise3([0, 1, 2].map(|k| p.get(k).copied().unwrap_or(0.0)))))
            }
            "clamp" => {
                let v = arg(0)?;
                let (pv, lo, hi) = (vec(0, "numbers")?, vec(1, "numbers")?, vec(2, "numbers")?);
                let pick = |p: &[f64], k: usize| p.get(k).or(p.first()).copied().unwrap_or(0.0);
                Ok(shaped(&v, pv.iter().enumerate().map(|(k, x)| x.max(pick(&lo, k).min(pick(&hi, k))).min(pick(&lo, k).max(pick(&hi, k)))).collect()))
            }
            "linear" | "ease" | "easeIn" | "easeOut" => {
                // (t, tMin, tMax, a, b), or (t, a, b) with t from 0 to 1
                let t = num(0, 0.0)?;
                let (t0, t1, a, b) = if args.len() > 3 { (num(1, 0.0)?, num(2, 1.0)?, arg(3)?, arg(4)?) } else { (0.0, 1.0, arg(1)?, arg(2)?) };
                let u = if t1 == t0 {
                    if t >= t1 { 1.0 } else { 0.0 }
                } else {
                    ((t - t0) / (t1 - t0)).clamp(0.0, 1.0)
                };
                // cubics from 0 to 1: ease leaves and arrives at rest; easeIn leaves at rest and
                // arrives at the speed of a straight line, easeOut the other way round
                let k = match name {
                    "ease" => u * u * (3.0 - 2.0 * u),
                    "easeIn" => u * u * (2.0 - u),
                    "easeOut" => 1.0 - (1.0 - u) * (1.0 - u) * (1.0 + u),
                    _ => u,
                };
                run.binary("+", a.clone(), run.binary("*", run.binary("-", b, a)?, Val::Num(k))?)
            }
            "add" | "sub" | "mul" | "div" => {
                let op = match name {
                    "add" => "+",
                    "sub" => "-",
                    "mul" => "*",
                    _ => "/",
                };
                run.binary(op, arg(0)?, arg(1)?)
            }
            "dot" => {
                let (a, b) = (vec(0, "two arrays")?, vec(1, "two arrays")?);
                Ok(Val::Num(a.iter().zip(&b).map(|(x, y)| x * y).sum()))
            }
            "cross" => {
                let (a, b) = (vec(0, "two arrays")?, vec(1, "two arrays")?);
                let g = |v: &[f64], k: usize| v.get(k).copied().unwrap_or(0.0);
                Ok(Val::nums(&[g(&a, 1) * g(&b, 2) - g(&a, 2) * g(&b, 1), g(&a, 2) * g(&b, 0) - g(&a, 0) * g(&b, 2), g(&a, 0) * g(&b, 1) - g(&a, 1) * g(&b, 0)]))
            }
            "length" => {
                let a = vec(0, "a point, or two")?;
                let d: Vec<f64> = match arg(1)? {
                    Val::Undef => a,
                    _ => {
                        let b = vec(1, "two points")?;
                        (0..a.len().max(b.len())).map(|k| a.get(k).copied().unwrap_or(0.0) - b.get(k).copied().unwrap_or(0.0)).collect()
                    }
                };
                Ok(Val::Num(d.iter().map(|x| x * x).sum::<f64>().sqrt()))
            }
            "normalize" => {
                let a = vec(0, "an array")?;
                let l = a.iter().map(|x| x * x).sum::<f64>().sqrt();
                Ok(Val::nums(&a.iter().map(|x| if l > 0.0 { x / l } else { 0.0 }).collect::<Vec<_>>()))
            }
            "lookAt" => {
                // the orientation that turns a layer at `from` to face `at`
                let (from, to) = (vec(0, "two points")?, vec(1, "two points")?);
                let d: Vec<f64> = (0..3).map(|k| to.get(k).copied().unwrap_or(0.0) - from.get(k).copied().unwrap_or(0.0)).collect();
                let l = d.iter().map(|x| x * x).sum::<f64>().sqrt();
                if l == 0.0 {
                    return Ok(Val::nums(&[0.0; 3]));
                }
                Ok(Val::nums(&[-(d[1] / l).clamp(-1.0, 1.0).asin().to_degrees(), d[0].atan2(d[2]).to_degrees(), 0.0]))
            }
            "degreesToRadians" => Ok(Val::Num(num(0, 0.0)?.to_radians())),
            "radiansToDegrees" => Ok(Val::Num(num(0, 0.0)?.to_degrees())),
            "timeToFrames" => Ok(Val::Num(frames(num(0, self.secs() + self.at.comp.start.seconds())?, num(1, self.fps())?, arg(2)?.truthy()))),
            "framesToTime" => Ok(Val::Num(num(0, 0.0)? / num(1, self.fps())?)),
            "timeToTimecode" => {
                let (t, base) = (num(0, self.secs() + self.at.comp.start.seconds())?, num(1, 30.0)?.round().max(1.0));
                let f = (t.abs() * base + 1e-6).floor() as i64;
                let b = base as i64;
                let (ff, s) = (f % b, f / b);
                Ok(Val::Str(format!("{}{}:{:02}:{:02}:{:02}", if t < 0.0 { "-" } else { "" }, s / 3600, s / 60 % 60, s % 60, ff).into()))
            }
            "rgbToHsl" => {
                let c = vec(0, "a colour, [r, g, b, a]")?;
                let g = |k: usize, d: f64| c.get(k).copied().unwrap_or(d);
                let (h, s, l) = veymelo_color::rgb_to_hsl(g(0, 0.0), g(1, 0.0), g(2, 0.0));
                Ok(Val::nums(&[h / 360.0, s, l, g(3, 1.0)]))
            }
            "hslToRgb" => {
                let c = vec(0, "a colour, [h, s, l, a]")?;
                let g = |k: usize, d: f64| c.get(k).copied().unwrap_or(d);
                let (r, gr, b) = veymelo_color::hsl_to_rgb(g(0, 0.0) * 360.0, g(1, 0.0), g(2, 0.0));
                Ok(Val::nums(&[r, gr, b, g(3, 1.0)]))
            }
            "hexToRgb" => match arg(0)? {
                Val::Str(s) => {
                    let c = veymelo_color::Rgba::from_hex(&s).ok_or_else(|| format!("\"{s}\" is not a colour like \"#ff8800\""))?;
                    Ok(Val::nums(&[c.r, c.g, c.b, c.a]))
                }
                _ => Err("hexToRgb takes a string like \"#ff8800\"".into()),
            },
            "comp" => match arg(0)? {
                Val::Str(n) => {
                    let item = self.eval.project.items.iter().find(|i| *i.name == *n && i.as_comp().is_some()).ok_or_else(|| format!("there is no composition \"{n}\""))?;
                    Ok(item.as_comp().map_or(Val::Undef, |c| Val::Obj(Obj::Comp(c, item.id))))
                }
                _ => Err("comp takes a composition's name".into()),
            },
            "parseFloat" | "parseInt" => {
                let s = show(&arg(0)?);
                let t = s.trim();
                let mut dot = name == "parseInt";
                let end = t
                    .char_indices()
                    .take_while(|(k, c)| {
                        let ok = c.is_ascii_digit() || (*k == 0 && (*c == '-' || *c == '+')) || (*c == '.' && !dot);
                        dot |= *c == '.';
                        ok
                    })
                    .count();
                Ok(Val::Num(t[..end].parse::<f64>().map_or(f64::NAN, |v| if name == "parseInt" { v.trunc() } else { v })))
            }
            "isNaN" => Ok(Val::Bool(num(0, f64::NAN).map_or(true, f64::is_nan))),
            "isFinite" => Ok(Val::Bool(num(0, f64::NAN).is_ok_and(f64::is_finite))),
            "String" => Ok(Val::Str(show(&arg(0)?).into())),
            "Number" => Ok(Val::Num(match arg(0)? {
                Val::Str(s) => s.trim().parse::<f64>().unwrap_or(if s.trim().is_empty() { 0.0 } else { f64::NAN }),
                v => run.number(v).unwrap_or(f64::NAN),
            })),
            _ => Err(format!("{name} cannot be called this way")),
        }
    }

    fn math(&self, run: &Run<'a, '_>, name: &str, args: &[Val<'a>]) -> Result<Val<'a>, Fail> {
        let all = args.iter().map(|a| run.number(a.clone())).collect::<Result<Vec<f64>, Fail>>()?;
        let x = all.first().copied().unwrap_or(f64::NAN);
        let y = all.get(1).copied().unwrap_or(f64::NAN);
        Ok(Val::Num(match name {
            "sin" => x.sin(),
            "cos" => x.cos(),
            "tan" => x.tan(),
            "asin" => x.asin(),
            "acos" => x.acos(),
            "atan" => x.atan(),
            "atan2" => x.atan2(y),
            "sinh" => x.sinh(),
            "cosh" => x.cosh(),
            "tanh" => x.tanh(),
            "sqrt" => x.sqrt(),
            "cbrt" => x.cbrt(),
            "pow" => x.powf(y),
            "abs" => x.abs(),
            "sign" => {
                if x == 0.0 || x.is_nan() {
                    x
                } else {
                    x.signum()
                }
            }
            "floor" => x.floor(),
            "ceil" => x.ceil(),
            // halves go up, as in JavaScript (-2.5 is -2)
            "round" => (x + 0.5).floor(),
            "trunc" => x.trunc(),
            "exp" => x.exp(),
            "log" => x.ln(),
            "log2" => x.log2(),
            "log10" => x.log10(),
            "hypot" => all.iter().map(|v| v * v).sum::<f64>().sqrt(),
            "min" => all.iter().copied().fold(f64::INFINITY, f64::min),
            "max" => all.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            "random" => self.draw(),
            _ => return Err(format!("Math has no {name}")),
        }))
    }

    fn string_method(&self, run: &Run<'a, '_>, s: &str, name: &str, args: &[Val<'a>]) -> Result<Val<'a>, Fail> {
        let text = |k: usize| -> Result<String, Fail> { Ok(args.get(k).map(|v| self.resolve(v.clone())).transpose()?.map_or(String::new(), |v| show(&v))) };
        let num = |k: usize, d: f64| -> Result<f64, Fail> { args.get(k).map_or(Ok(d), |v| run.number(v.clone())) };
        let chars: Vec<char> = s.chars().collect();
        let n = chars.len() as f64;
        // a place in the string; below zero counts from its end
        let place = |v: f64| (if v < 0.0 { (n + v).max(0.0) } else { v.min(n) }) as usize;
        let count = |b: Option<usize>| b.map_or(-1.0, |b| s[..b].chars().count() as f64);
        let str = |v: String| Val::Str(v.into());
        Ok(match name {
            "toUpperCase" => str(s.to_uppercase()),
            "toLowerCase" => str(s.to_lowercase()),
            "trim" => str(s.trim().into()),
            "indexOf" => Val::Num(count(s.find(&text(0)?))),
            "lastIndexOf" => Val::Num(count(s.rfind(&text(0)?))),
            "includes" => Val::Bool(s.contains(&text(0)?)),
            "startsWith" => Val::Bool(s.starts_with(&text(0)?)),
            "endsWith" => Val::Bool(s.ends_with(&text(0)?)),
            "replace" => str(s.replacen(&text(0)?, &text(1)?, 1)),
            "replaceAll" => str(s.replace(&text(0)?, &text(1)?)),
            "split" => match args.first() {
                None => Val::arr(vec![Val::Str(s.into())]),
                Some(_) => {
                    let sep = text(0)?;
                    if sep.is_empty() { Val::arr(chars.iter().map(|c| str(c.to_string())).collect()) } else { Val::arr(s.split(sep.as_str()).map(|p| str(p.into())).collect()) }
                }
            },
            "charAt" => str(chars.get(num(0, 0.0)?.max(0.0) as usize).map_or(String::new(), |c| c.to_string())),
            "charCodeAt" => Val::Num(chars.get(num(0, 0.0)?.max(0.0) as usize).map_or(f64::NAN, |c| *c as u32 as f64)),
            "slice" | "substring" => {
                let (mut a, mut b) = (num(0, 0.0)?, num(1, n)?);
                if name == "substring" {
                    (a, b) = (a.max(0.0), b.max(0.0));
                    if a > b {
                        (a, b) = (b, a);
                    }
                }
                let (a, b) = (place(a), place(b));
                str(if a < b { chars[a..b].iter().collect() } else { String::new() })
            }
            "substr" => {
                let a = place(num(0, 0.0)?);
                let b = (a as f64 + num(1, n)?.max(0.0)).min(n) as usize;
                str(chars[a..b.max(a)].iter().collect())
            }
            "padStart" | "padEnd" => {
                let (want, fill) = (num(0, 0.0)?.max(0.0) as usize, args.get(1).map_or(Ok(" ".to_string()), |_| text(1))?);
                let short = want.saturating_sub(chars.len());
                let pad: String = fill.chars().cycle().take(if fill.is_empty() { 0 } else { short }).collect();
                str(if name == "padStart" { pad + s } else { s.to_string() + &pad })
            }
            "repeat" => str(s.repeat(num(0, 0.0)?.clamp(0.0, 10_000.0) as usize)),
            "toString" | "valueOf" => str(s.into()),
            _ => return Err(format!("a string cannot {name}")),
        })
    }

    fn array_method(&self, run: &Run<'a, '_>, items: &Rc<Vec<Val<'a>>>, name: &str, args: &[Val<'a>]) -> Result<Val<'a>, Fail> {
        let arg = |k: usize| args.get(k).cloned().map_or(Ok(Val::Undef), |v| self.resolve(v));
        let num = |k: usize, d: f64| -> Result<f64, Fail> { args.get(k).map_or(Ok(d), |v| run.number(v.clone())) };
        let n = items.len() as f64;
        let place = |v: f64| (if v < 0.0 { (n + v).max(0.0) } else { v.min(n) }) as usize;
        let f = || match arg(0) {
            Ok(f @ Val::Fun(_)) => Ok(f),
            _ => Err(format!("{name} takes a function, like {name}(function (x) {{ return x * 2; }})")),
        };
        Ok(match name {
            "join" => {
                let sep = match arg(0)? {
                    Val::Undef => ",".to_string(),
                    v => show(&v),
                };
                Val::Str(items.iter().map(show).collect::<Vec<_>>().join(&sep).into())
            }
            "slice" => {
                let (a, b) = (place(num(0, 0.0)?), place(num(1, n)?));
                Val::arr(if a < b { items[a..b].to_vec() } else { Vec::new() })
            }
            "concat" => {
                let mut out = (**items).clone();
                for k in 0..args.len() {
                    match arg(k)? {
                        Val::Arr(more) => out.extend(more.iter().cloned()),
                        v => out.push(v),
                    }
                }
                Val::arr(out)
            }
            "indexOf" | "includes" => {
                let want = arg(0)?;
                let k = items.iter().position(|x| matches!(run.binary("==", x.clone(), want.clone()), Ok(Val::Bool(true))));
                if name == "includes" { Val::Bool(k.is_some()) } else { Val::Num(k.map_or(-1.0, |k| k as f64)) }
            }
            "reverse" => Val::arr(items.iter().rev().cloned().collect()),
            "map" => {
                let f = f()?;
                Val::arr(items.iter().enumerate().map(|(k, x)| run.call(f.clone(), vec![x.clone(), Val::Num(k as f64)])).collect::<Result<_, _>>()?)
            }
            "filter" => {
                let f = f()?;
                let mut out = Vec::new();
                for (k, x) in items.iter().enumerate() {
                    if self.resolve(run.call(f.clone(), vec![x.clone(), Val::Num(k as f64)])?)?.truthy() {
                        out.push(x.clone());
                    }
                }
                Val::arr(out)
            }
            "reduce" => {
                let f = f()?;
                let mut it = items.iter().enumerate();
                let mut acc = match args.get(1) {
                    Some(v) => self.resolve(v.clone())?,
                    None => it.next().map(|(_, x)| x.clone()).ok_or("reduce of an empty array needs a start")?,
                };
                for (k, x) in it {
                    acc = run.call(f.clone(), vec![acc, x.clone(), Val::Num(k as f64)])?;
                }
                acc
            }
            "toString" => Val::Str(show(&Val::Arr(items.clone())).into()),
            _ => return Err(format!("an array cannot {name}")),
        })
    }
}

impl<'e, 'a> Host<'a> for Ctx<'e, 'a> {
    fn resolve(&self, v: Val<'a>) -> Result<Val<'a>, Fail> {
        Ok(match v {
            Val::Obj(Obj::Prop(p, l, c, cid)) => self.read(p, Place { layer: l, comp: c, comp_id: cid }, self.time.get()),
            Val::Obj(Obj::Key(p, l, _, _, k)) => p.keys.get(k).map_or(Val::Undef, |key| prop_val(self.eval, p, l, &key.value)),
            other => other,
        })
    }

    fn member(&self, of: &Val<'a>, name: &str) -> Result<Val<'a>, Fail> {
        let method = || Val::Fun(Rc::new(Fun::Method(of.clone(), name.into())));
        match of {
            Val::Obj(Obj::Comp(c, id)) => Ok(match name {
                "width" => Val::Num(c.width as f64),
                "height" => Val::Num(c.height as f64),
                "duration" => Val::Num(c.duration.seconds()),
                "frameDuration" => Val::Num(c.frame_rate.frame_duration().seconds()),
                "numLayers" => Val::Num(c.layers.len() as f64),
                "pixelAspect" => Val::Num(c.pixel_aspect),
                "displayStartTime" => Val::Num(c.start.seconds()),
                "bgColor" => Val::nums(&[c.background.r, c.background.g, c.background.b, c.background.a]),
                "name" => Val::Str(self.eval.project.item(*id).map_or(String::new(), |i| i.name.clone()).into()),
                "marker" => Val::Obj(Obj::Markers(&c.markers, None)),
                "activeCamera" => match self.eval.camera(c, self.time.get()).3 {
                    Some(l) => Val::Obj(Obj::Layer(l, c, *id)),
                    None => return Err("the composition has no camera (it is seen through the standard one)".into()),
                },
                "layer" => method(),
                _ => return Err(format!("a composition has no \"{name}\"")),
            }),
            Val::Obj(Obj::Layer(l, c, cid)) => self.layer_member(l, c, *cid, name),
            Val::Obj(Obj::Group(g, l, c, cid)) => match name {
                "name" => Ok(Val::Str(g.name().into())),
                "numProperties" => Ok(Val::Num(g.children.len() as f64)),
                "enabled" => Ok(Val::Bool(g.enabled)),
                "property" | "content" | "propertyGroup" => Ok(method()),
                "propertyIndex" => Ok(Val::Num(index_in(&l.props, (*g as *const Group).cast()).unwrap_or(0) as f64)),
                _ => group_child(g, Place { layer: l, comp: c, comp_id: *cid }, &Val::Str(name.into())),
            },
            Val::Obj(Obj::Prop(p, l, c, cid)) => {
                let at = Place { layer: l, comp: c, comp_id: *cid };
                Ok(match name {
                    "value" => self.read(p, at, self.time.get()),
                    "name" => Val::Str(p.def.name.into()),
                    "numKeys" => Val::Num(p.keys.len() as f64),
                    "velocity" => self.velocity(p, at, self.time.get()),
                    "speed" => Val::Num(self.speed(p, at, self.time.get())),
                    "expression" => Val::Str(p.expression.as_ref().map_or("", |e| e.text.as_str()).into()),
                    "propertyGroup" => method(),
                    "propertyIndex" => Val::Num(index_in(&l.props, (*p as *const Property).cast()).unwrap_or(0) as f64),
                    n if PROP_METHODS.contains(&n) => method(),
                    // the rest belongs to its value: position[0], text.sourceText.length
                    _ => return self.member(&self.read(p, at, self.time.get()), name),
                })
            }
            Val::Obj(Obj::Key(p, l, _, _, k)) => {
                let key = p.keys.get(*k).ok_or("there is no such key")?;
                Ok(match name {
                    "time" => Val::Num(l.comp_time(key.time).seconds()),
                    "value" => prop_val(self.eval, p, l, &key.value),
                    "index" => Val::Num(*k as f64 + 1.0),
                    _ => return Err(format!("a key has time, value and index, not \"{name}\"")),
                })
            }
            Val::Obj(Obj::Markers(list, _)) => match name {
                "numKeys" => Ok(Val::Num(list.len() as f64)),
                "key" | "nearestKey" => Ok(method()),
                _ => Err(format!("markers have numKeys, key(n) and nearestKey(t), not \"{name}\"")),
            },
            Val::Obj(Obj::Marker(m, l, k)) => Ok(match name {
                "time" => Val::Num(self.marker_time(m, *l).seconds()),
                "duration" => Val::Num(m.duration.seconds()),
                "comment" => Val::Str(m.comment.as_str().into()),
                "chapter" => Val::Str(m.chapter.as_str().into()),
                "url" => Val::Str(m.url.as_str().into()),
                "index" => Val::Num(*k as f64 + 1.0),
                _ => return Err(format!("a marker has time, duration, comment, chapter, url and index, not \"{name}\"")),
            }),
            Val::Obj(Obj::Rect(r)) => match name {
                "left" => Ok(Val::Num(r[0])),
                "top" => Ok(Val::Num(r[1])),
                "width" => Ok(Val::Num(r[2])),
                "height" => Ok(Val::Num(r[3])),
                _ => Err(format!("a box has left, top, width and height, not \"{name}\"")),
            },
            Val::Obj(Obj::Math) => match name {
                "PI" => Ok(Val::Num(std::f64::consts::PI)),
                "E" => Ok(Val::Num(std::f64::consts::E)),
                "LN2" => Ok(Val::Num(std::f64::consts::LN_2)),
                "LN10" => Ok(Val::Num(std::f64::consts::LN_10)),
                "LOG2E" => Ok(Val::Num(std::f64::consts::LOG2_E)),
                "LOG10E" => Ok(Val::Num(std::f64::consts::LOG10_E)),
                "SQRT2" => Ok(Val::Num(std::f64::consts::SQRT_2)),
                "SQRT1_2" => Ok(Val::Num(std::f64::consts::FRAC_1_SQRT_2)),
                n if MATH.contains(&n) => Ok(method()),
                _ => Err(format!("Math has no \"{name}\"")),
            },
            Val::Arr(items) if name == "length" => Ok(Val::Num(items.len() as f64)),
            Val::Str(s) if name == "length" => Ok(Val::Num(s.chars().count() as f64)),
            Val::Num(_) | Val::Str(_) | Val::Arr(_) => Ok(method()),
            other => Err(format!("{} has no \"{name}\"", other.kind())),
        }
    }

    fn global(&self, name: &str) -> Option<Val<'a>> {
        let at = self.at;
        if let Some((_, v)) = self.eval.vars.borrow().iter().rev().find(|(k, _)| k == name) {
            return Some(if v.len() == 1 { Val::Num(v[0]) } else { Val::Arr(std::rc::Rc::new(v.iter().map(|x| Val::Num(*x)).collect())) });
        }
        Some(match name {
            "time" => Val::Num(self.secs()),
            "value" => prop_val(self.eval, self.prop, at.layer, &self.prop.value_at(at.layer.layer_time(self.time.get()))),
            "thisComp" => Val::Obj(Obj::Comp(at.comp, at.comp_id)),
            "thisLayer" => Val::Obj(Obj::Layer(at.layer, at.comp, at.comp_id)),
            "thisProperty" => Val::Obj(Obj::Prop(self.prop, at.layer, at.comp, at.comp_id)),
            "index" => Val::Num(self.index_of(at.layer, at.comp)),
            "numKeys" => Val::Num(self.prop.keys.len() as f64),
            "velocity" => self.velocity(self.prop, at, self.time.get()),
            "speed" => Val::Num(self.speed(self.prop, at, self.time.get())),
            "Math" => Val::Obj(Obj::Math),
            "undefined" => Val::Undef,
            "NaN" => Val::Num(f64::NAN),
            "Infinity" => Val::Num(f64::INFINITY),
            n => match BUILTINS.iter().chain(PATH_METHODS.iter().filter(|_| self.prop.def.kind == veymelo_keyframe::Kind::Path)).find(|b| **b == n) {
                Some(b) => Val::Fun(Rc::new(Fun::Builtin(b))),
                // what belongs to the layer can be named alone: position, transform, effect(…)
                None => return self.layer_member(at.layer, at.comp, at.comp_id, n).ok(),
            },
        })
    }

    fn call_builtin(&self, run: &Run<'a, '_>, name: &str, args: Vec<Val<'a>>) -> Result<Val<'a>, Fail> {
        self.builtin(run, name, args)
    }

    fn call_method(&self, run: &Run<'a, '_>, on: Val<'a>, name: &str, args: Vec<Val<'a>>) -> Result<Val<'a>, Fail> {
        let arg = |k: usize| args.get(k).cloned().map_or(Ok(Val::Undef), |v| self.resolve(v));
        match &on {
            Val::Obj(Obj::Comp(c, cid)) if name == "layer" => self.comp_layer(c, *cid, &args),
            Val::Obj(Obj::Layer(l, c, cid)) => self.layer_method(run, l, c, *cid, name, &args),
            Val::Obj(Obj::Group(g, l, c, cid)) if name == "property" => group_child(g, Place { layer: l, comp: c, comp_id: *cid }, &arg(0)?),
            // the group so many above it (1: the one it is in); past the top, its layer
            Val::Obj(Obj::Group(_, l, c, cid)) | Val::Obj(Obj::Prop(_, l, c, cid)) if name == "propertyGroup" => {
                let n = match arg(0)? {
                    Val::Undef => 1.0,
                    v => run.number(v)?,
                };
                if n < 1.0 || n.fract() != 0.0 {
                    return Err("propertyGroup takes how many groups up, from 1".into());
                }
                let target: *const () = match &on {
                    Val::Obj(Obj::Group(g, ..)) => (*g as *const Group).cast(),
                    Val::Obj(Obj::Prop(p, ..)) => (*p as *const Property).cast(),
                    _ => unreachable!("matched"),
                };
                let chain = groups_to(&l.props, target).unwrap_or_default();
                Ok(match chain.len().checked_sub(n as usize) {
                    Some(k) if k < chain.len() => Val::Obj(Obj::Group(chain[k], l, c, *cid)),
                    _ => Val::Obj(Obj::Layer(l, c, *cid)),
                })
            }
            // a shape group's own shapes: content("Group 1").content("Rectangle 1")
            Val::Obj(Obj::Group(g, l, c, cid)) if name == "content" => {
                let list = g.group("contents").map_err(|_| format!("{} holds no shapes", g.name()))?;
                group_child(list, Place { layer: l, comp: c, comp_id: *cid }, &arg(0)?)
            }
            Val::Obj(Obj::Prop(p, l, c, cid)) if PATH_METHODS.contains(&name) && p.def.kind == veymelo_keyframe::Kind::Path => {
                let at = Place { layer: l, comp: c, comp_id: *cid };
                // the time comes last: after the place along the path for the …OnPath ones
                let k = if name.ends_with("OnPath") { 1 } else { 0 };
                let t = match args.get(k) {
                    Some(v) => run.number(self.resolve(v.clone())?)?,
                    None => self.secs(),
                };
                // its own path is read as its keys give it (before this expression), as `value` is
                let v = if std::ptr::eq(*p, self.prop) { p.value_at(l.layer_time(Tick::from_seconds(t))) } else { veymelo_keyframe::Value::Number(0.0) };
                let read = if let veymelo_keyframe::Value::Path(own) = v { Val::Obj(Obj::Path(Rc::new(own))) } else { self.read(p, at, Tick::from_seconds(t)) };
                match read {
                    Val::Obj(Obj::Path(path)) => path_method(run, &path, name, &args[..args.len().min(k)]),
                    _ => Err(format!("{} has no path", p.def.id)),
                }
            }
            Val::Obj(Obj::Path(path)) => path_method(run, path, name, &args),
            Val::Obj(Obj::Prop(p, l, c, cid)) => {
                let at = Place { layer: l, comp: c, comp_id: *cid };
                if PROP_METHODS.contains(&name) {
                    self.prop_method(run, p, at, name, &args)
                } else {
                    let v = self.read(p, at, self.time.get());
                    self.call_method(run, v, name, args)
                }
            }
            Val::Obj(Obj::Markers(list, l)) => {
                let (list, l): (&'a [Marker], Option<&'a Layer>) = (list, *l);
                let k = match name {
                    "key" => match arg(0)? {
                        Val::Num(n) => (n as usize).checked_sub(1).filter(|k| *k < list.len()),
                        // by its comment
                        Val::Str(s) => list.iter().position(|m| *m.comment == *s),
                        _ => None,
                    }
                    .ok_or_else(|| format!("there is no marker {} ({} markers)", show(&arg(0).unwrap_or(Val::Undef)), list.len()))?,
                    "nearestKey" => {
                        let t = Tick::from_seconds(match arg(0)? {
                            Val::Undef => self.secs(),
                            v => run.number(v)?,
                        });
                        list.iter().enumerate().min_by_key(|(_, m)| (self.marker_time(m, l) - t).abs()).map(|(k, _)| k).ok_or("there are no markers")?
                    }
                    _ => return Err(format!("markers cannot {name}")),
                };
                Ok(Val::Obj(Obj::Marker(&list[k], l, k)))
            }
            Val::Obj(Obj::Math) => self.math(run, name, &args),
            Val::Num(n) => match name {
                "toFixed" => {
                    let digits = run.number(arg(0)?).unwrap_or(0.0).clamp(0.0, 100.0) as usize;
                    Ok(Val::Str(format!("{n:.digits$}").into()))
                }
                "toString" | "valueOf" => Ok(Val::Str(show(&on).into())),
                _ => Err(format!("a number cannot {name}")),
            },
            Val::Str(s) => self.string_method(run, s, name, &args),
            Val::Arr(items) => self.array_method(run, items, name, &args),
            other => Err(format!("{} cannot {name}", other.kind())),
        }
    }
}

/// A path's answer to `name`.
fn path_method<'a>(run: &Run<'a, '_>, p: &veymelo_keyframe::ShapePath, name: &str, args: &[Val<'a>]) -> Result<Val<'a>, Fail> {
    let pts = |v: &[[f64; 2]]| Val::arr(v.iter().map(|q| Val::nums(q)).collect());
    let place = || -> Result<f64, Fail> {
        match args.first() {
            Some(v) => run.number(v.clone()),
            None => Ok(0.5),
        }
    };
    let along = |f: f64| veymelo_path::bezier::along(p, f).ok_or_else(|| "the path has no points".to_string());
    Ok(match name {
        "points" => pts(&p.points),
        "inTangents" => pts(&p.in_tangents),
        "outTangents" => pts(&p.out_tangents),
        "isClosed" => Val::Bool(p.closed),
        "pointOnPath" => Val::nums(&along(place()?)?.0),
        "tangentOnPath" => Val::nums(&along(place()?)?.1),
        // a quarter turn from the direction, to its right on screen (y down)
        "normalOnPath" => {
            let d = along(place()?)?.1;
            Val::nums(&[-d[1], d[0]])
        }
        _ => return Err(format!("a path cannot {name} (it can {})", PATH_METHODS.join(", "))),
    })
}

/// createPath's points, as [x, y] pairs.
fn pairs<'a>(run: &Run<'a, '_>, v: Option<&Val<'a>>, n: Option<usize>) -> Result<Vec<[f64; 2]>, Fail> {
    let list: Vec<[f64; 2]> = match v {
        None | Some(Val::Undef) => Vec::new(),
        Some(Val::Arr(items)) => items
            .iter()
            .map(|q| match q {
                Val::Arr(xy) if xy.len() >= 2 => Ok([run.number(xy[0].clone())?, run.number(xy[1].clone())?]),
                other => Err(format!("a point is [x, y], not {}", other.kind())),
            })
            .collect::<Result<_, Fail>>()?,
        Some(other) => return Err(format!("points are an array of [x, y], not {}", other.kind())),
    };
    Ok(match n {
        Some(n) if list.is_empty() => vec![[0.0; 2]; n],
        Some(n) if list.len() != n => return Err(format!("{} tangents for {n} points", list.len())),
        _ => list,
    })
}
