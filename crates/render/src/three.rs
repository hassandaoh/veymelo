//! 3D layers drawn together: each a flat picture standing in the composition's world, seen
//! through the camera, lit by the lights, throwing shadows on the others.
//!
//! A run of 3D layers one after another in the stack is drawn as one scene (a 2D layer between
//! them starts another). Each pixel gathers what of every layer lies under it — found back
//! through each layer's perspective, so its picture is exact at any angle — lights it, and lays
//! the pieces from the farthest to the nearest, each by its layer's blend mode and opacity.
//! Layers that pass through each other are so cut where they cross (a pixel the crossing goes
//! through is looked at four times, so the line is smooth), and see-through layers show what
//! is behind them in depth, not in stack order.
//!
//! Shadows are found by looking from each point toward each light past the layers that cast
//! them (a see-through layer lets some light through, coloured by it by its light
//! transmission); a light's shadow diffusion makes it a disc instead of a point, so its shadows
//! soften the farther they fall.

use std::borrow::Cow;

use veymelo_color::{BlendMode, blend};
use veymelo_effects::Buf;
use veymelo_geom::{Mat3, Mat4, Vec3, vec2, vec3};
use veymelo_project::{Comp, Layer, LightKind};
use veymelo_raster::{Image, Px, Sampling, noise};
use veymelo_time::Tick;

use crate::scene::{self, Camera, Focus, Light, Material, NEAR};
use crate::{Renderer, Take};

mod caustics;

/// A layer standing in the scene.
struct Plane<'d> {
    /// Its place in the stack (higher: above).
    order: usize,
    /// Its picture as drawn (lit already, and blurred, when the camera blurs).
    buf: Cow<'d, Buf>,
    /// Its picture as it is, for the shadows it casts.
    own: &'d Buf,
    /// Whether `buf` is lit already.
    lit: bool,
    /// From the canvas's pixels back to the layer's own units (perspective), and on.
    back: Mat3,
    onto: Mat3,
    /// Its depth from the camera at layer point (u, v): `d[0]·u + d[1]·v + d[2]`.
    depth: [f64; 3],
    /// The world point of layer point (u, v) is `origin + u·across + v·down`.
    origin: Vec3,
    across: Vec3,
    down: Vec3,
    normal: Vec3,
    /// From the world back to the layer's units.
    unworld: Mat4,
    /// The canvas pixels it may cover: x0, y0, x1, y1.
    reach: [i64; 4],
    material: Material,
    opacity: f32,
    blend: BlendMode,
    seed: u32,
    /// How much of it shows at each canvas pixel by its track matte (as the matte is seen).
    matte: Option<std::sync::Arc<Vec<f32>>>,
    /// Drawn only where what is under the run is.
    keep_under: bool,
    /// The points the light bouncing onto it is gathered at (none: none bounces onto it).
    grid: Option<crate::bounce::Grid>,
    /// The cells the light glass bends onto it is gathered in (none: none is).
    caustic: Option<crate::bounce::Grid>,
}

impl Plane<'_> {
    /// The layer's picture as drawn at its point (u, v).
    fn texel(&self, u: f64, v: f64) -> Px {
        let b = &self.buf;
        b.img.sample(Sampling::Bilinear, (u - b.origin.x) * b.scale, (v - b.origin.y) * b.scale)
    }

    /// The layer's own picture (unlit, sharp) at (u, v), for the shadow it casts.
    fn shadow_texel(&self, u: f64, v: f64) -> Px {
        let b = &self.own;
        b.img.sample(Sampling::Bilinear, (u - b.origin.x) * b.scale, (v - b.origin.y) * b.scale)
    }

    /// Layer point and depth under canvas point (x, y), if it is in front of the camera.
    fn under(&self, x: f64, y: f64) -> Option<(f64, f64, f64)> {
        let m = &self.back.0;
        let w = m[2][0] * x + m[2][1] * y + m[2][2];
        if w.abs() < 1e-12 {
            return None;
        }
        let u = (m[0][0] * x + m[0][1] * y + m[0][2]) / w;
        let v = (m[1][0] * x + m[1][1] * y + m[1][2]) / w;
        let z = self.depth[0] * u + self.depth[1] * v + self.depth[2];
        (z >= NEAR).then_some((u, v, z))
    }
}

/// A layer taking part in a run: one of the composition's, or one inside a collapsed 3D
/// composition layer (inside another, and so on), seen in the outer scene through `chain`
/// (the composition layers it is inside, outermost first, each with its composition).
pub(crate) struct Member<'a> {
    pub layer: &'a Layer,
    pub comp: &'a Comp,
    pub chain: Vec<(&'a Comp, &'a Layer)>,
}

/// A layer of the run with its picture drawn, and its track matte.
struct Drawn<'a> {
    order: usize,
    layer: &'a Layer,
    comp: &'a Comp,
    chain: Vec<(&'a Comp, &'a Layer)>,
    buf: Buf,
    matte: Option<std::sync::Arc<Vec<f32>>>,
    /// Where its picture stands in the layer's own units (a character of per-character 3D
    /// text; else as it is).
    local: Mat4,
    /// A 3D model layer's model (its picture then is empty), or the solid an extruded text or
    /// shape layer makes (in its own space: true).
    model: Option<(std::sync::Arc<veymelo_model::Model>, bool)>,
}

/// A 3D model layer as drawn, with how it lays over what is behind it.
struct ModelShot {
    order: usize,
    shot: crate::meshes::Shot,
    opacity: f32,
    blend: BlendMode,
    seed: u32,
    matte: Option<std::sync::Arc<Vec<f32>>>,
    keep_under: bool,
}

/// A piece of a run's layer at a canvas pixel, for the render passes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    /// Its depth from the camera.
    pub z: f32,
    /// How much of the pixel it covers (its alpha, opacity and matte).
    pub a: f32,
    /// The run's member it is of.
    pub member: usize,
    /// Its normal in the camera's space, facing the camera.
    pub normal: [f32; 3],
    /// Where on the layer it is (0 to 1 across and down), or a model's texture point.
    pub uv: [f32; 2],
    /// A model's material there, if it has one.
    pub material: Option<usize>,
}

/// A piece of a layer at a pixel: its depth, its layer, its colour (premultiplied, lit).
#[derive(Clone, Copy)]
struct Piece {
    z: f64,
    plane: usize,
    color: Px,
}

impl<'a> Renderer<'a> {
    /// How many canvas pixels a 3D layer's unit is where it is seen biggest (the detail it is
    /// drawn at), held to four times its own size.
    pub(crate) fn detail_3d(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3) -> f64 {
        let cam = scene::camera(&self.eval, comp, t);
        let world = self.eval.world(self.at(comp, layer), t, Some(cam.eye));
        let r = self.bounds(comp, layer);
        let k = view.apply_vec(vec2(1.0, 0.0)).length();
        let unit = world.apply_vec(vec3(1.0, 0.0, 0.0)).length().max(world.apply_vec(vec3(0.0, 1.0, 0.0)).length());
        let most = r
            .corners()
            .iter()
            .chain([r.center()].iter())
            .map(|c| cam.view.apply(world.apply(vec3(c.x, c.y, 0.0))).z)
            .map(|z| if z < NEAR { f64::INFINITY } else { k * cam.zoom * unit / z })
            .fold(0.0, f64::max);
        // out of focus a layer needs less
        most.clamp(1.0 / 64.0, if cam.focus.is_some() { 1.5 } else { 4.0 })
    }

    /// The part of a 3D layer that can be seen on a canvas `size` pixels big, in its own units
    /// (its rectangle `r` cut to that part, grown a little), and the detail that part is seen
    /// with: an empty part when none of it shows. Found by laying a grid over the layer and
    /// seeing which of its cells the camera's picture reaches, so a layer far bigger than the
    /// picture is drawn only where it is seen, and only as finely as it is seen there.
    ///
    /// A layer whose unseen parts can still change what shows (blurred along its motion,
    /// changed by effects or styles, made solid) is not cut: it is only left out when it, and
    /// a wide margin round it, is nowhere in the picture (none comes back otherwise: drawn
    /// whole). None either for a layer casting a shadow by a light that throws them (its part
    /// out of the picture casts its shadow into it), in a scene with an environment light, or
    /// for one seen through collapsed compositions.
    pub(crate) fn seen_3d(&self, comp: &Comp, layer: &Layer, t: Tick, view: Mat3, size: (usize, usize), r: veymelo_geom::Rect) -> Option<(veymelo_geom::Rect, f64)> {
        if self.fixed.is_some() || r.is_empty() || size.0 == 0 || size.1 == 0 {
            return None;
        }
        let at = self.at(comp, layer);
        let shadowing = scene::lights(&self.eval, comp, t, self.linear).iter().any(|l| l.shadows) && scene::Material::of(&self.eval, at, t).casts != 0;
        if shadowing || comp.layers.iter().any(|l| l.environment && l.is_active_at(t)) {
            return None;
        }
        let solid = matches!(layer.source, veymelo_project::Source::Text | veymelo_project::Source::Shape) && layer.props.prop("geometryOptions/extrusionDepth").is_ok_and(|p| self.eval.value(p, at, t).as_f64() > 0.0);
        let whole = self.blurs(comp, layer) || self.has_effects(layer) || crate::styles::list(layer).is_some() || solid;
        let cam = scene::camera(&self.eval, comp, t);
        let world = self.eval.world(at, t, Some(cam.eye));
        let (w, h) = (comp.width as f64, comp.height as f64);
        let lens = Mat3([[cam.zoom, 0.0, w / 2.0], [0.0, cam.zoom, h / 2.0], [0.0, 0.0, 1.0]]);
        let c = (cam.view * world).0;
        let through = Mat3([[c[0][0], c[0][1], c[0][3]], [c[1][0], c[1][1], c[1][3]], [c[2][0], c[2][1], c[2][3]]]);
        let onto = view * lens * through;
        let depth = |u: f64, v: f64| c[2][0] * u + c[2][1] * v + c[2][3];
        let k = view.apply_vec(vec2(1.0, 0.0)).length();
        let unit = world.apply_vec(vec3(1.0, 0.0, 0.0)).length().max(world.apply_vec(vec3(0.0, 1.0, 0.0)).length());
        // the canvas, a few pixels wider all round (a line's edge still reaches in)
        let screen = veymelo_geom::Rect::new(-8.0, -8.0, size.0 as f64 + 8.0, size.1 as f64 + 8.0);
        // what is looked for: the layer itself, or it and a wide margin for what its effects spread
        let looked = if whole { r.inflate(r.width().max(r.height()) * 0.5 + 400.0) } else { r };
        const N: usize = 32;
        let (gw, gh) = (looked.width() / N as f64, looked.height() / N as f64);
        let corner = |i: usize, j: usize| -> (f64, f64) { (looked.x0 + gw * i as f64, looked.y0 + gh * j as f64) };
        // each grid point: where it falls on the canvas and how far it is, or none behind the camera
        let mut points = Vec::with_capacity((N + 1) * (N + 1));
        for j in 0..=N {
            for i in 0..=N {
                let (u, v) = corner(i, j);
                let z = depth(u, v);
                points.push((z >= NEAR).then(|| (onto.apply(vec2(u, v)), z)));
            }
        }
        let (mut part, mut most) = (veymelo_geom::Rect::EMPTY, 0.0f64);
        for j in 0..N {
            for i in 0..N {
                let four = [points[j * (N + 1) + i], points[j * (N + 1) + i + 1], points[(j + 1) * (N + 1) + i], points[(j + 1) * (N + 1) + i + 1]];
                // a cell partly behind the camera may reach anywhere: kept
                let seen = if four.iter().any(|p| p.is_none()) {
                    true
                } else {
                    let mut b = veymelo_geom::Rect::EMPTY;
                    for p in four.iter().flatten() {
                        b.include(p.0);
                    }
                    !b.intersect(&screen).is_empty()
                };
                if seen {
                    let (u0, v0) = corner(i, j);
                    part = part.union(&veymelo_geom::Rect::new(u0, v0, u0 + gw, v0 + gh));
                    for p in four.iter() {
                        most = most.max(match p {
                            Some((_, z)) => k * cam.zoom * unit / z,
                            None => f64::INFINITY,
                        });
                    }
                }
            }
        }
        if part.is_empty() {
            return Some((veymelo_geom::Rect::EMPTY, 0.0));
        }
        if whole {
            return None;
        }
        let detail = most.clamp(1.0 / 64.0, if cam.focus.is_some() { 1.5 } else { 4.0 });
        // grown by a few canvas pixels, in its own units
        Some((part.inflate(4.0 / detail).intersect(&r), detail))
    }

    /// Through the collapsed composition layers `chain` (outermost first) at time `t`: the
    /// transform from the innermost composition's world to the outer one's, the innermost
    /// composition's time, and the layers' opacities multiplied; none where one is not showing
    /// its composition then. `eye` (the outer world's) turns those that face the camera.
    fn through(&self, chain: &[(&Comp, &Layer)], t: Tick, eye: Option<Vec3>) -> Option<(Mat4, Tick, f64)> {
        let (mut m, mut time, mut alpha) = (Mat4::IDENTITY, t, 1.0);
        for (c, l) in chain {
            let at = self.at(c, l);
            let e = eye.and_then(|e| m.inverse().map(|mi| mi.apply(e)));
            m = m * self.eval.world(at, time, e);
            alpha *= self.eval.opacity(at, time);
            time = self.eval.source_time(at, time);
            let veymelo_project::Source::Comp(id) = l.source else { return None };
            let inner = self.project.comp(id)?;
            if time < Tick::ZERO || time >= inner.duration {
                return None;
            }
        }
        Some((m, time, alpha))
    }

    /// The detail a layer inside collapsed compositions is drawn at: where its plane (placed in
    /// the outer world by `outer`) is seen biggest through the outer composition's camera.
    #[allow(clippy::too_many_arguments)]
    fn detail_in(&self, comp: &Comp, own: &Comp, layer: &Layer, outer: &Mat4, when: Tick, t: Tick, view: Mat3) -> f64 {
        let cam = scene::camera(&self.eval, comp, t);
        let world = *outer * self.eval.world(self.at(own, layer), when, outer.inverse().map(|m| m.apply(cam.eye)));
        let r = self.bounds(own, layer);
        let k = view.apply_vec(vec2(1.0, 0.0)).length();
        let unit = world.apply_vec(vec3(1.0, 0.0, 0.0)).length().max(world.apply_vec(vec3(0.0, 1.0, 0.0)).length());
        r.corners()
            .iter()
            .chain([r.center()].iter())
            .map(|c| cam.view.apply(world.apply(vec3(c.x, c.y, 0.0))).z)
            .map(|z| if z < NEAR { f64::INFINITY } else { k * cam.zoom * unit / z })
            .fold(0.0, f64::max)
            .clamp(1.0 / 64.0, if cam.focus.is_some() { 1.5 } else { 4.0 })
    }

    /// The layers a run takes from `layer`: itself, or — a collapsed 3D composition layer with
    /// nothing done to it, all of whose shown layers are 3D — those inside it (and inside
    /// those, and so on), seen through it.
    pub(crate) fn members<'c>(&self, comp: &'c Comp, layer: &'c Layer, t: Tick, chain: &[(&'c Comp, &'c Layer)], out: &mut Vec<Member<'c>>)
    where
        'a: 'c,
    {
        let has = |list: &str| layer.props.group(list).is_ok_and(|g| !g.children.is_empty());
        let inner = match layer.source {
            veymelo_project::Source::Comp(id) if layer.switches.collapse && layer.matte.is_none() && !layer.preserve_transparency && !has("masks") && !has("effects") && crate::styles::list(layer).is_none() => self.project.comp(id),
            _ => None,
        };
        let Some(inner) = inner else {
            out.push(Member { layer, comp, chain: chain.to_vec() });
            return;
        };
        let time = self.eval.source_time(self.at(comp, layer), t);
        let shown: Vec<&Layer> = inner.layers.iter().rev().filter(|l| l.is_active_at(time) && l.switches.video).collect();
        let drawing = |l: &Layer| !matches!(l.source, veymelo_project::Source::Camera | veymelo_project::Source::Light(_) | veymelo_project::Source::Null);
        if shown.iter().any(|l| drawing(l) && !scene::in_3d(l)) {
            self.note(format!("\"{}\" is a collapsed 3D composition with 2D layers in it: drawn flat, as one layer", layer.name));
            out.push(Member { layer, comp, chain: chain.to_vec() });
            return;
        }
        let mut deeper = chain.to_vec();
        deeper.push((comp, layer));
        for l in shown.into_iter().filter(|l| drawing(l)) {
            self.members(inner, l, time, &deeper, out);
        }
    }

    /// A run of 3D layers (bottom first) drawn onto `canvas`, `view` taking the composition's
    /// pixels to the canvas's, their opacity times `fade`. Where they are motion blurred, the
    /// scene is put together at moments across the shutter (each layer drawn once) and those
    /// pictures are averaged. With `record`, nothing is drawn: each canvas pixel's pieces are
    /// kept there instead, nearest first, at the frame's time (for the render passes).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_3d(&self, comp: &Comp, members: &[Member], t: Tick, view: Mat3, canvas: &mut Image, fade: f64, depth: usize, record: Option<&mut Vec<Vec<Hit>>>) {
        // the environment lighting the scene; the layers shown as the sky all round, behind the rest
        let env = self.environment(comp, t, depth);
        let mut skies = false;
        if record.is_none() {
            for m in members.iter().filter(|m| m.chain.is_empty() && m.layer.environment_background) {
                self.sky(comp, m.layer, t, view, canvas, fade, depth);
                skies = true;
            }
        }
        // the sky settled with the rest, however the run ends (a canvas is settled once each
        // layer, or run of them, is drawn)
        self.run_drawn(comp, members, t, view, canvas, fade, depth, record, env);
        if skies {
            self.settle(canvas);
        }
    }

    /// The run's layers drawn, as `run_3d` says, over the sky drawn already.
    #[allow(clippy::too_many_arguments)]
    fn run_drawn(&self, comp: &Comp, members: &[Member], t: Tick, view: Mat3, canvas: &mut Image, fade: f64, depth: usize, record: Option<&mut Vec<Vec<Hit>>>, env: Option<scene::Environment>) {
        // each layer's picture, at the frame's time, and its track matte as it is seen
        let mut drawn: Vec<Drawn> = Vec::new();
        for (order, m) in members.iter().enumerate() {
            let (layer, own) = (m.layer, m.comp);
            // a picture lighting the scene, or the sky drawn already
            if layer.environment || layer.environment_background {
                continue;
            }
            // per-character 3D text: each character a plane of its own
            if m.chain.is_empty() && layer.source == veymelo_project::Source::Text && layer.props.prop("text/perChar3d").is_ok_and(|p| self.eval.value(p, self.at(comp, layer), t).as_bool()) {
                let detail = self.detail_3d(comp, layer, t, view);
                for (img, origin, scale, local) in self.text_chars(comp, layer, t, detail) {
                    drawn.push(Drawn { order, layer, comp: own, chain: Vec::new(), buf: Buf { img, origin, scale }, matte: None, local, model: None });
                }
                continue;
            }
            // a 3D primitive: its solid made at the frame's time, in the layer's own space
            if let veymelo_project::Source::Primitive(kind) = layer.source {
                let when = if m.chain.is_empty() { t } else { self.through(&m.chain, t, None).map_or(t, |x| x.1) };
                let model = crate::meshes::primitive(&self.eval, self.at(own, layer), when, kind);
                let matte = if m.chain.is_empty() {
                    layer.matte.and_then(|mt| comp.layer(mt.layer).filter(|ml| ml.id != layer.id).map(|ml| (ml, mt.kind))).map(|(ml, kind)| {
                        let cut = self.alone(comp, ml, t, view, canvas.width, canvas.height, depth);
                        std::sync::Arc::new(cut.px.iter().map(|q| crate::matte_of(*q, kind)).collect::<Vec<f32>>())
                    })
                } else {
                    None
                };
                let empty = Buf { img: Image::new(0, 0), origin: vec2(0.0, 0.0), scale: 1.0 };
                drawn.push(Drawn { order, layer, comp: own, chain: m.chain.clone(), buf: empty, matte, local: Mat4::IDENTITY, model: Some((model, true)) });
                continue;
            }
            if let veymelo_project::Source::Model(item) = layer.source {
                let path = match self.project.item(item).map(|i| &i.kind) {
                    Some(veymelo_project::ItemKind::Footage(f)) => f.path.clone(),
                    _ => continue,
                };
                match veymelo_media::model(&path) {
                    Ok(model) => {
                        for n in &model.notes {
                            self.note(format!("\"{}\": {n}", layer.name));
                        }
                        let matte = if m.chain.is_empty() {
                            layer.matte.and_then(|mt| comp.layer(mt.layer).filter(|ml| ml.id != layer.id).map(|ml| (ml, mt.kind))).map(|(ml, kind)| {
                                let cut = self.alone(comp, ml, t, view, canvas.width, canvas.height, depth);
                                std::sync::Arc::new(cut.px.iter().map(|q| crate::matte_of(*q, kind)).collect::<Vec<f32>>())
                            })
                        } else {
                            None
                        };
                        let empty = Buf { img: Image::new(0, 0), origin: vec2(0.0, 0.0), scale: 1.0 };
                        drawn.push(Drawn { order, layer, comp: own, chain: m.chain.clone(), buf: empty, matte, local: Mat4::IDENTITY, model: Some((model, false)) });
                    }
                    Err(e) => self.note(format!("\"{}\" cannot show its model: {e}", layer.name)),
                }
                continue;
            }
            let buf = if m.chain.is_empty() {
                self.source(comp, layer, t, view, canvas, depth, Take::All)
            } else {
                // inside a collapsed composition: drawn at the detail the outer scene sees it with
                let Some((outer, when, _)) = self.through(&m.chain, t, None) else { continue };
                let detail = self.detail_in(comp, own, layer, &outer, when, t, view);
                let helper = crate::Renderer { fixed: Some(detail), ..crate::Renderer::new(self.project, self.options) };
                let buf = helper.source(own, layer, when, view, canvas, depth + m.chain.len(), Take::All);
                for n in helper.notes.into_inner() {
                    self.note(n);
                }
                buf
            };
            let Some(mut buf) = buf else { continue };
            if let Some((st, _)) = self.styled(own, layer, t, &buf) {
                buf = st.flat();
            }
            let matte = if m.chain.is_empty() {
                layer.matte.and_then(|mt| comp.layer(mt.layer).filter(|ml| ml.id != layer.id).map(|ml| (ml, mt.kind))).map(|(ml, kind)| {
                    let cut = self.alone(comp, ml, t, view, canvas.width, canvas.height, depth);
                    std::sync::Arc::new(cut.px.iter().map(|q| crate::matte_of(*q, kind)).collect::<Vec<f32>>())
                })
            } else {
                if layer.matte.is_some() {
                    self.note(format!("\"{}\" is inside a collapsed 3D composition; its track matte is not used there", layer.name));
                }
                None
            };
            // an extruded text or shape layer: made solid
            let solid = match layer.source {
                veymelo_project::Source::Text | veymelo_project::Source::Shape => {
                    let at = self.at(own, layer);
                    let read = |path: &str, d: f64| layer.props.prop(path).map_or(d, |p| self.eval.value(p, at, t).as_f64());
                    let depth = read("geometryOptions/extrusionDepth", 0.0);
                    (depth > 0.0)
                        .then(|| {
                            let g = crate::extrude::Geometry {
                                style: layer.props.prop("geometryOptions/bevelStyle").map_or(0, |p| self.eval.value(p, at, t).as_choice()),
                                bevel: read("geometryOptions/bevelDepth", 2.0),
                                hole_bevel: read("geometryOptions/holeBevelDepth", 100.0) / 100.0,
                                depth,
                            };
                            crate::extrude::solid(&buf, &g, self.linear)
                        })
                        .flatten()
                }
                _ => None,
            };
            let model = solid.map(|m| (std::sync::Arc::new(m), true));
            drawn.push(Drawn { order, layer, comp: own, chain: m.chain.clone(), buf, matte, local: Mat4::IDENTITY, model });
        }
        if drawn.is_empty() {
            return;
        }
        let blurred = drawn.iter().any(|d| self.blurs(d.comp, d.layer) || d.chain.iter().any(|(c, l)| self.blurs(c, l)));
        if !blurred || record.is_some() {
            self.scene_at(comp, &drawn, t, t, view, canvas, fade, env.as_ref(), record);
            return;
        }
        let mb = &comp.motion_blur;
        let frame = comp.frame_rate.frame_duration().seconds();
        let n = mb.samples.clamp(2, 64) as usize;
        let under = canvas.clone();
        let mut sum = vec![[0f32; 4]; canvas.px.len()];
        for i in 0..n {
            let open = (mb.phase + mb.angle * i as f64 / (n - 1) as f64) / 360.0 * frame;
            let mut img = under.clone();
            self.scene_at(comp, &drawn, t, t + Tick::from_seconds(open), view, &mut img, fade, env.as_ref(), None);
            for (s, p) in sum.iter_mut().zip(&img.px) {
                (0..4).for_each(|k| s[k] += p[k]);
            }
        }
        for (p, s) in canvas.px.iter_mut().zip(&sum) {
            *p = s.map(|v| v / n as f32);
        }
        self.settle(canvas);
    }

    /// The environment lighting composition `comp` at `t`: its light's picture (its source layer's,
    /// else the composition's environment layer's) made light.
    fn environment(&self, comp: &Comp, t: Tick, depth: usize) -> Option<scene::Environment> {
        let scene::EnvLight { source, tint, rotation, occlusion, occlusion_distance } = scene::environment_light(&self.eval, comp, t, self.linear)?;
        let layer = match source {
            Some(id) => comp.layer(id),
            None => comp.layers.iter().find(|l| l.environment && l.is_active_at(t)),
        }?;
        let pano = self.panorama(comp, layer, t, depth)?;
        // as much light as there is picture (premultiplied), times the light's colour
        let px = pano.px.iter().map(|p| [p[0] * tint[0], p[1] * tint[1], p[2] * tint[2]]).collect();
        scene::Environment::new(pano.width, pano.height, px, rotation).map(|mut e| {
            (e.occlusion, e.occlusion_distance) = (occlusion, occlusion_distance);
            e
        })
    }

    /// A layer's picture as a panorama: drawn whole at its own size (none bigger than 4096
    /// across), just its own rectangle (what it shows, from its corner).
    fn panorama(&self, comp: &Comp, layer: &veymelo_project::Layer, t: Tick, depth: usize) -> Option<Image> {
        let (w, h) = crate::source_size(self.project, layer)?;
        if w < 1.0 || h < 1.0 {
            return None;
        }
        let scale = (4096.0 / w).min(1.0);
        let helper = crate::Renderer { fixed: Some(scale), ..crate::Renderer::new(self.project, self.options) };
        let buf = helper.source(comp, layer, t, Mat3::default(), &Image::new(1, 1), depth + 1, Take::All);
        for n in helper.notes.into_inner() {
            self.note(n);
        }
        let buf = buf?;
        let (pw, ph) = ((w * scale).round().max(1.0) as usize, (h * scale).round().max(1.0) as usize);
        let mut out = Image::new(pw, ph);
        // the buffer's pixels from its origin, at its own scale
        let to_buf = Mat3::scale(vec2(buf.scale / scale, buf.scale / scale)) * Mat3::translate(vec2(-buf.origin.x * scale, -buf.origin.y * scale));
        let from = to_buf.inverse()?;
        veymelo_raster::place(&mut out, &buf.img, &from, Sampling::Bilinear, veymelo_color::BlendMode::Normal, 1.0, 0);
        Some(out)
    }

    /// An environment background layer drawn onto `canvas`: its picture as a panorama all round,
    /// seen through the camera as if endlessly far (turned as the environment light is).
    #[allow(clippy::too_many_arguments)]
    fn sky(&self, comp: &Comp, layer: &veymelo_project::Layer, t: Tick, view: Mat3, canvas: &mut Image, fade: f64, depth: usize) {
        let opacity = (self.eval.opacity(self.at(comp, layer), t) * fade) as f32;
        let Some(unview) = view.inverse() else { return };
        if opacity <= 0.0 {
            return;
        }
        let Some(img) = self.panorama(comp, layer, t, depth) else { return };
        let (iw, ih) = (img.width as f64, img.height as f64);
        let rotation = scene::environment_light(&self.eval, comp, t, self.linear).map_or(0.0, |e| e.rotation);
        let cam = scene::camera(&self.eval, comp, t);
        let Some(world) = cam.view.inverse() else { return };
        let (w, h) = (comp.width as f64, comp.height as f64);
        let width = canvas.width;
        veymelo_raster::par::rows(&mut canvas.px, width, |y, row| {
            for (x, d) in row.iter_mut().enumerate() {
                let at = unview.apply(vec2(x as f64 + 0.5, y as f64 + 0.5));
                let dir = world.apply_vec(vec3(at.x - w / 2.0, at.y - h / 2.0, cam.zoom)).normalize();
                let (u, v) = scene::panorama_at(dir, rotation);
                let s = img.sample(veymelo_raster::Sampling::Bilinear, (u * iw).clamp(0.5, iw - 0.5), (v * ih).clamp(0.5, ih - 0.5));
                let k = 1.0 - s[3] * opacity;
                *d = std::array::from_fn(|c| s[c] * opacity + d[c] * k);
            }
        });
    }

    /// The run's layers (their pictures drawn) put together as the scene is at `moment` (those
    /// motion blurred; the rest as it is at `t`) and drawn onto `canvas`.
    #[allow(clippy::too_many_arguments)]
    fn scene_at(&self, comp: &Comp, drawn: &[Drawn], t: Tick, moment: Tick, view: Mat3, canvas: &mut Image, fade: f64, env: Option<&scene::Environment>, record: Option<&mut Vec<Vec<Hit>>>) {
        let (cam_now, cam_then) = (scene::camera(&self.eval, comp, t), scene::camera(&self.eval, comp, moment));
        let lights = scene::lights(&self.eval, comp, moment, self.linear);
        let (w, h) = (comp.width as f64, comp.height as f64);
        let mut planes = Vec::new();
        let mut models: Vec<(&Drawn, Tick, Mat4, f64)> = Vec::new();
        for d in drawn {
            let Drawn { order, layer, comp: own, chain, buf, matte, local, model } = d;
            let moving = self.blurs(own, layer) || chain.iter().any(|(c, l)| self.blurs(c, l));
            let (when, cam) = if moving { (moment, &cam_then) } else { (t, &cam_now) };
            // through the collapsed compositions it is in: their transforms, times and opacities
            let Some((outer, inner_when, outer_alpha)) = self.through(chain, when, Some(cam.eye)) else { continue };
            let at = self.at(own, layer);
            let alpha = self.eval.opacity(at, inner_when) * outer_alpha * fade;
            if alpha <= 0.0 {
                continue;
            }
            if model.is_some() {
                let eye = outer.inverse().map(|m| m.apply(cam.eye));
                models.push((d, inner_when, outer * self.eval.world(at, inner_when, eye), alpha));
                continue;
            }
            // the camera's picture: from its space to the composition's pixels, then the canvas's
            let lens = Mat3([[cam.zoom, 0.0, w / 2.0], [0.0, cam.zoom, h / 2.0], [0.0, 0.0, 1.0]]);
            let eye = outer.inverse().map(|m| m.apply(cam.eye));
            let world = outer * self.eval.world(at, inner_when, eye) * *local;
            let seen = cam.view * world;
            let c = &seen.0;
            let through = Mat3([[c[0][0], c[0][1], c[0][3]], [c[1][0], c[1][1], c[1][3]], [c[2][0], c[2][1], c[2][3]]]);
            let onto = view * lens * through;
            let Some(back) = onto.inverse() else { continue };
            let depth_of = [c[2][0], c[2][1], c[2][3]];
            let origin = world.apply(vec3(0.0, 0.0, 0.0));
            let (across, down) = (world.apply_vec(vec3(1.0, 0.0, 0.0)), world.apply_vec(vec3(0.0, 1.0, 0.0)));
            let normal = across.cross(down);
            if normal.length() < 1e-12 {
                continue;
            }
            // the canvas pixels its picture can cover
            let (bw, bh) = (buf.img.width as f64 / buf.scale, buf.img.height as f64 / buf.scale);
            let corners = [(0.0, 0.0), (bw, 0.0), (bw, bh), (0.0, bh)].map(|(dx, dy)| (buf.origin.x + dx, buf.origin.y + dy));
            let mut reach = [i64::MAX, i64::MAX, i64::MIN, i64::MIN];
            let mut behind = false;
            for (u, v) in corners {
                if depth_of[0] * u + depth_of[1] * v + depth_of[2] < NEAR {
                    behind = true;
                    continue;
                }
                let p = onto.apply(vec2(u, v));
                reach = [reach[0].min(p.x.floor() as i64 - 1), reach[1].min(p.y.floor() as i64 - 1), reach[2].max(p.x.ceil() as i64 + 1), reach[3].max(p.y.ceil() as i64 + 1)];
            }
            if behind {
                reach = [0, 0, canvas.width as i64, canvas.height as i64];
            }
            let reach = [reach[0].max(0), reach[1].max(0), reach[2].min(canvas.width as i64), reach[3].min(canvas.height as i64)];
            planes.push(Plane {
                order: *order,
                own: buf,
                lit: false,
                buf: Cow::Borrowed(buf),
                back,
                onto,
                depth: depth_of,
                origin,
                across,
                down,
                normal: normal.normalize(),
                unworld: world.inverse().unwrap_or(Mat4::IDENTITY),
                reach,
                material: scene::Material::of(&self.eval, at, inner_when),
                opacity: alpha as f32,
                blend: layer.blend,
                seed: self.seed(comp, layer, t),
                matte: matte.clone(),
                keep_under: layer.preserve_transparency,
                grid: None,
                caustic: None,
            });
        }
        if planes.is_empty() && models.is_empty() {
            return;
        }
        let cam = cam_then;
        let casters: Vec<usize> = (0..planes.len()).filter(|&i| planes[i].material.casts != 0).collect();
        // each model posed and placed; those that cast shadows, as trees of their triangles
        let posed: Vec<(scene::Material, veymelo_model::Pose, Mat4)> = models
            .iter()
            .map(|(d, when, world, _)| {
                let at = self.at(d.comp, d.layer);
                let (model, own_space) = d.model.as_ref().expect("a model");
                let (pose, to_layer) = if *own_space { (model.pose(None, 0.0), Mat4::IDENTITY) } else { crate::meshes::posed(&self.eval, at, model, *when) };
                (scene::Material::of(&self.eval, at, *when), pose, *world * to_layer)
            })
            .collect();
        let casting_models: Vec<usize> = (0..models.len()).filter(|&i| posed[i].0.casts != 0 && !lights.is_empty()).collect();
        let mut casting = Casting { models: casting_models.iter().map(|&i| (&*models[i].0.model.as_ref().expect("a model").0, &posed[i].1, posed[i].2)).collect(), trees: std::sync::OnceLock::new(), glass: Vec::new(), caustics: false };
        // every model (seen, or casting a shadow alone), for what hides the sky from a point and
        // what a mirror sees (each of its triangles as a reflection sees it, one after another)
        let open_reach = scene::occlusion_reach(&lights, env);
        let mirrors = planes.iter().map(|p| &p.material).chain(posed.iter().map(|(m, ..)| m)).any(|m| m.reflection > 0.0) || posed.iter().any(|(m, ..)| m.transparency > 0.0);
        // light bouncing (where there is light to bounce)
        let bounce = if comp.bounce.amount > 0.0 && (!lights.is_empty() || env.is_some()) { comp.bounce.amount as f32 } else { 0.0 };
        let everything = Casting { models: if open_reach.is_some() || mirrors || bounce > 0.0 { models.iter().zip(&posed).map(|((d, ..), (_, pose, place))| (&*d.model.as_ref().expect("a model").0, pose, *place)).collect() } else { Vec::new() }, trees: std::sync::OnceLock::new(), glass: Vec::new(), caustics: false };
        // the points the bouncing light is gathered at: a grid over each flat layer that takes
        // light (on the side the camera sees), then every model's vertices
        let mut probes: Vec<crate::bounce::Probe> = Vec::new();
        let mut model_probes: Vec<(usize, usize)> = Vec::new();
        if bounce > 0.0 {
            for (i, p) in planes.iter_mut().enumerate() {
                if !p.material.lit || p.material.casts == 2 {
                    continue;
                }
                let (w, h) = (p.own.img.width as f64 / p.own.scale, p.own.img.height as f64 / p.own.scale);
                let grid = crate::bounce::Grid::over(p.own.origin.x, p.own.origin.y, w, h, probes.len());
                let n = if p.normal.dot(cam.eye - p.origin) < 0.0 { -p.normal } else { p.normal };
                probes.extend(grid.points().map(|(u, v)| crate::bounce::Probe { at: p.origin + p.across * u + p.down * v, n, own: i }));
                p.grid = Some(grid);
            }
            for ((d, ..), (_, pose, place)) in models.iter().zip(&posed) {
                let first = probes.len();
                probes.extend(crate::meshes::probes(&d.model.as_ref().expect("a model").0, pose, place).into_iter().map(|(at, n)| crate::bounce::Probe { at, n, own: usize::MAX }));
                model_probes.push((first, probes.len() - first));
            }
        }
        let mut faces: Vec<crate::meshes::Face> = Vec::new();
        let mut face_first = Vec::new();
        if mirrors || bounce > 0.0 {
            for (k, ((d, ..), (m, pose, place))) in models.iter().zip(&posed).enumerate() {
                face_first.push(faces.len());
                faces.extend(crate::meshes::faces(&d.model.as_ref().expect("a model").0, pose, place, m, self.linear, model_probes.get(k).map(|p| p.0)));
            }
        }
        // each casting model's glass (a see-through model has its faces, mirrors or not)
        casting.glass = casting_models.iter().map(|&i| (posed[i].0.transparency > 0.0 && i < face_first.len()).then(|| Glass { clear: posed[i].0.transparency, rolloff: posed[i].0.clear_rolloff, ior: posed[i].0.ior, first: face_first[i] })).collect();
        // the light glass bends, landing where it is bent to (its cells after the bounce's
        // points in the scene's light)
        let mut bent: Vec<[f32; 3]> = Vec::new();
        if comp.caustics > 0.0 && casting.glass.iter().any(Option::is_some) && lights.iter().any(|l| l.shadows) {
            casting.caustics = true;
            let none: Vec<[f32; 3]> = Vec::new();
            let found = Scene { planes: &planes, shots: &[], casting: &casting, everything: &everything, reach: open_reach, faces: &faces, face_first: &face_first, light: &none, bounce, width: canvas.width, lights: &lights, casters: &casters, cam: &cam, env, shadows: None }.caustics(comp.caustics as f32);
            for c in found {
                planes[c.plane].caustic = Some(crate::bounce::Grid { first: probes.len() + bent.len(), ..c.grid });
                bent.extend(c.light);
            }
        }
        // for the GPU: whether any casting model is glass, then each casting triangle's face (a
        // triangle of a model not glass: none)
        let glass_faces: Vec<u32> = if casting.glass.iter().any(Option::is_some) {
            let mut v = vec![if casting.caustics { 2u32 } else { 1 }];
            for (ci, &i) in casting_models.iter().enumerate() {
                let triangles = crate::meshes::corners(casting.models[ci].0, casting.models[ci].1, &casting.models[ci].2).len() / 3;
                match casting.glass[ci] {
                    Some(g) => v.extend((0..triangles).map(|j| (g.first + j) as u32)),
                    None => v.extend(std::iter::repeat_n(crate::gpu::NONE, triangles)),
                }
                let _ = i;
            }
            v
        } else {
            vec![0]
        };
        // what casts shadows, as the GPU reads it (none: the processor draws what they fall on)
        let gpu_shadows = if self.options.gpu && !crate::gpu::given_up() {
            let shadowing = lights.iter().any(|l| l.shadows) && (!casters.is_empty() || !casting.is_empty());
            let f = |v: f64| v as f32;
            let list: Vec<(crate::gpu::Caster, &Image)> = planes
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let u = &p.unworld.0;
                    let caster = crate::gpu::Caster {
                        origin: [f(p.origin.x), f(p.origin.y), f(p.origin.z), p.material.transmission],
                        normal: [f(p.normal.x), f(p.normal.y), f(p.normal.z), if p.material.casts != 0 { 1.0 } else { 0.0 }],
                        unworld: [0, 1, 2, 3].map(|r| u[r].map(f)),
                        own: [f(p.own.origin.x), f(p.own.origin.y), f(p.own.scale), 0.0],
                        img: [0, 0, 0, i as u32],
                        mat: [p.material.ambient, p.material.diffuse, p.material.lit as u32 as f32, if p.material.casts == 2 { 0.0 } else { 1.0 }],
                        map: crate::gpu::Caster::grid(p.grid).0,
                        cell: crate::gpu::Caster::grid(p.grid).1,
                        mirror: [p.material.reflection, p.material.rolloff, 0.0, 0.0],
                        caustic: crate::gpu::Caster::grid(p.caustic).0,
                        bent: crate::gpu::Caster::grid(p.caustic).1,
                    };
                    (caster, &p.own.img)
                })
                .collect();
            crate::gpu::Shadows::new(&list, &if shadowing { casting.corners() } else { Vec::new() }, &everything.corners(), &faces, &glass_faces).unwrap_or_else(|e| {
                crate::gpu::failed(&e);
                None
            })
        } else {
            None
        };
        // the light bouncing onto each point, bounce after bounce: on the GPU when it can, else
        // by the processor
        let on_gpu = gpu_shadows.as_ref().filter(|_| !probes.is_empty() && !crate::gpu::given_up()).and_then(|sh| {
            gather_on_gpu(sh, &probes, &lights, env, bounce, comp.bounce.bounces)
                .map_err(|e| crate::gpu::failed(&e))
                .ok()
        });
        let mut light: Vec<[f32; 3]> = match on_gpu {
            Some(light) => light,
            None if probes.is_empty() => Vec::new(),
            None => {
                let none: Vec<[f32; 3]> = Vec::new();
                let gather = Scene { planes: &planes, shots: &[], casting: &casting, everything: &everything, reach: open_reach, faces: &faces, face_first: &face_first, light: &none, bounce, width: canvas.width, lights: &lights, casters: &casters, cam: &cam, env, shadows: None };
                let mut e = vec![[0f32; 3]; probes.len()];
                for _ in 0..comp.bounce.bounces {
                    let before = e.clone();
                    veymelo_raster::par::chunks(&mut e, 16, |k, part| {
                        for (i, out) in part.iter_mut().enumerate() {
                            *out = gather.gather_light(&probes[k * 16 + i], &before);
                        }
                    });
                }
                e
            }
        };
        crate::bounce::smooth(&mut light, &planes.iter().filter_map(|p| p.grid).collect::<Vec<_>>());
        if !bent.is_empty() {
            light.resize(probes.len(), [0.0; 3]);
            light.extend(bent);
        }
        // the model layers, drawn with the shadows falling on them
        let shots: Vec<ModelShot> = {
            let lit = Scene { planes: &planes, shots: &[], casting: &casting, everything: &everything, reach: open_reach, faces: &faces, face_first: &face_first, light: &light, bounce, width: canvas.width, lights: &lights, casters: &casters, cam: &cam, env, shadows: None };
            let open = |q: Vec3, n: Vec3| lit.open(usize::MAX, q, n);
            let mirror = |q: Vec3, n: Vec3, viewer: Vec3, sharpness: f32, none: [f32; 3]| lit.mirrored(usize::MAX, q, n, viewer, sharpness, none);
            let through = |q: Vec3, n: Vec3, viewer: Vec3, ior: f32| lit.seen_through(q, n, viewer, ior);
            let standard = crate::meshes::standard_lights(&cam);
            let unshadowed = |l: usize, q: Vec3| if l < lights.len() { lit.unshadowed(usize::MAX, l, q) } else { [1.0; 3] };
            let lens = Mat3([[cam.zoom, 0.0, w / 2.0], [0.0, cam.zoom, h / 2.0], [0.0, 0.0, 1.0]]);
            let mut out = Vec::new();
            for (k, ((d, _, _, alpha), (material, pose, place))) in models.iter().zip(&posed).enumerate() {
                let (material, place) = (*material, *place);
                let layer = d.layer;
                let model = &d.model.as_ref().expect("a model").0;
                // its shadow alone
                if material.casts == 2 {
                    continue;
                }
                let look = crate::meshes::Look {
                    model,
                    pose,
                    place,
                    cam: &cam,
                    onto: view * lens,
                    width: canvas.width,
                    height: canvas.height,
                    material,
                    lights: if lights.is_empty() && env.is_none() { &standard } else { &lights },
                    unshadowed: &unshadowed,
                    linear: self.linear,
                    env,
                    shadowed: material.accepts != 0 && lights.iter().any(|l| l.shadows) && (!casters.is_empty() || !casting.is_empty()),
                    shadows: gpu_shadows.as_ref(),
                    open: open_reach.map(|_| &open as &(dyn Fn(Vec3, Vec3) -> f32 + Sync)),
                    reach: open_reach,
                    mirror: mirrors.then_some(&mirror as crate::meshes::Mirror),
                    bounce: model_probes.get(k).filter(|_| bounce > 0.0).map(|&(first, count)| (&light[first..first + count], bounce)),
                    through: (material.transparency > 0.0).then_some(&through as crate::meshes::Through),
                    scene_light: (&light, bounce),
                    gpu: self.options.gpu,
                };
                if let Some(mut shot) = crate::meshes::shoot(&look) {
                    // out of focus: blurred by its depth
                    if let Some(focus) = &cam.focus {
                        shot = crate::meshes::defocus(shot, focus, view.apply_vec(vec2(1.0, 0.0)).length(), canvas.width, canvas.height);
                    }
                    out.push(ModelShot { order: d.order, shot, opacity: *alpha as f32, blend: layer.blend, seed: self.seed(comp, layer, t), matte: d.matte.clone(), keep_under: layer.preserve_transparency });
                }
            }
            out
        };
        if let Some(record) = record {
            let bounds: Vec<(usize, veymelo_geom::Rect)> = drawn.iter().map(|d| (d.order, self.bounds(d.comp, d.layer))).collect();
            let scene = Scene { planes: &planes, shots: &shots, casting: &casting, everything: &everything, reach: open_reach, faces: &faces, face_first: &face_first, light: &light, bounce, width: canvas.width, lights: &lights, casters: &casters, cam: &cam, env, shadows: gpu_shadows.as_ref() };
            veymelo_raster::par::rows(record, canvas.width, |y, row| {
                for (x, hits) in row.iter_mut().enumerate() {
                    hits.extend(scene.hits(x, y, &bounds));
                    hits.sort_by(|a, b| a.z.total_cmp(&b.z));
                }
            });
            return;
        }
        if let Some(focus) = &cam.focus {
            // blurred out of focus: each lit in its own picture first (so its light and shadows
            // blur with it), then blurred by its depth
            let lit: Vec<Buf> = {
                let scene = Scene { planes: &planes, shots: &shots, casting: &casting, everything: &everything, reach: open_reach, faces: &faces, face_first: &face_first, light: &light, bounce, width: canvas.width, lights: &lights, casters: &casters, cam: &cam, env, shadows: gpu_shadows.as_ref() };
                // as far past the canvas as anything can blur in from
                let margin = (focus.aperture * focus.level * 2.0).min(800.0);
                (0..planes.len()).map(|i| scene.lit_picture(i, canvas.width as f64, canvas.height as f64, margin)).collect()
            };
            for (p, b) in planes.iter_mut().zip(lit) {
                let size = cam.zoom * p.across.length().max(p.down.length());
                p.buf = Cow::Owned(defocus(b, focus, p.depth, size));
                p.lit = true;
            }
        }
        let reach = planes.iter().map(|p| p.reach).chain(shots.iter().map(|s| s.shot.reach())).fold([i64::MAX, i64::MAX, i64::MIN, i64::MIN], |r, p| [r[0].min(p[0]), r[1].min(p[1]), r[2].max(p[2]), r[3].max(p[3])]);
        let width = canvas.width;
        let (y0, y1) = (reach[1].max(0) as usize, (reach[3].max(0) as usize).min(canvas.height));
        if y0 >= y1 {
            return;
        }
        let scene = Scene { planes: &planes, shots: &shots, casting: &casting, everything: &everything, reach: open_reach, faces: &faces, face_first: &face_first, light: &light, bounce, width: canvas.width, lights: &lights, casters: &casters, cam: &cam, env, shadows: gpu_shadows.as_ref() };
        let (x0, x1) = (reach[0].max(0) as usize, (reach[2].max(0) as usize).min(width));
        let on_gpu = self.options.gpu
            && !crate::gpu::given_up()
            && match scene.on_gpu(canvas, [x0, y0, x1, y1]) {
                Ok(drawn) => drawn,
                Err(e) => {
                    crate::gpu::failed(&e);
                    false
                }
            };
        if !on_gpu {
            let rows = &mut canvas.px[y0 * width..y1 * width];
            veymelo_raster::par::rows(rows, width, |row, line| {
                let y = y0 + row;
                for (x, px) in line.iter_mut().enumerate().take(x1).skip(x0) {
                    *px = scene.pixel(x, y, *px);
                }
            });
        }
        if let Some(s) = gpu_shadows {
            s.keep();
        }
        self.settle(canvas);
    }
}

/// How a layer's piece lays over what is behind it: its place in the stack, opacity, blend
/// mode, seed, matte, and whether only over what is under the run.
struct Laid<'s> {
    order: usize,
    opacity: f32,
    blend: BlendMode,
    seed: u32,
    matte: Option<&'s std::sync::Arc<Vec<f32>>>,
    keep_under: bool,
}

/// How many points of a diffuse light's disc a point in the soft edge of its shadow looks at.
const SOFT_POINTS: usize = 128;

/// How many surfaces deep a reflection goes (a mirror in a mirror in a mirror).
const MIRROR_DEPTH: usize = 3;

/// How many times a way through glass is reflected whole inside it, at most.
const GLASS_TURNS: usize = 4;

/// A surface a reflection meets: how far along the ray, how it looks (lit as a mirror sees it),
/// which way it faces (toward where the ray came from), how much it mirrors and how much less
/// face on, and the flat layer it is (`usize::MAX`: a model's).
struct Met {
    d: f64,
    lit: [f32; 3],
    n: Vec3,
    mirror: (f32, f32),
    own: usize,
}

/// The light bouncing onto each of `probes` (in order), gathered on the GPU `bounces` times
/// over, `amount` of it taken each time: what `Scene::gather_light` gathers.
fn gather_on_gpu(sh: &crate::gpu::Shadows, probes: &[crate::bounce::Probe], lights: &[Light], env: Option<&scene::Environment>, amount: f32, bounces: u32) -> Result<Vec<[f32; 3]>, String> {
    use crate::gpu;
    use veymelo_os::gpu::{Arg, Buffer, bytes_of, run};
    let f = |v: f64| v as f32;
    let list: Vec<gpu::Probe> = probes.iter().map(|p| gpu::Probe { at: [f(p.at.x), f(p.at.y), f(p.at.z), f32::from_bits(if p.own == usize::MAX { u32::MAX } else { p.own as u32 })], n: [f(p.n.x), f(p.n.y), f(p.n.z), 0.0] }).collect();
    let lights: Vec<gpu::Light> = lights.iter().map(gpu::Light::of).collect();
    let (env_px, env_levels, turn, shining) = gpu::environment(env)?;
    let params = gpu::BounceParams { a: [list.len() as u32, lights.len() as u32, sh.count, shining], b: [amount, turn, 0.0, 0.0] };
    let (list, lights) = (Buffer::of(&list)?, gpu::take_with(gpu::LIGHTS, &lights)?);
    // what the bounce before gathered (none, the first time), and what this one does
    let (mut before, mut out) = (Buffer::new(probes.len() * 16)?, Buffer::new(probes.len() * 16)?);
    for _ in 0..bounces {
        let mut args = vec![Arg::Write(&mut out), Arg::Read(&list), Arg::Read(&before), Arg::Read(&sh.casters), Arg::Read(&sh.texels), Arg::Read(&sh.faces), Arg::Read(&lights), Arg::Read(&env_px), Arg::Read(&env_levels), Arg::Bytes(bytes_of(&params))];
        let (kernel, traced) = sh.kernel(gpu::GATHER, gpu::GATHER_RAYS)?;
        args.extend(traced);
        run(&kernel, &args, [probes.len(), 1])?;
        drop(args);
        std::mem::swap(&mut before, &mut out);
    }
    let light = before.slice::<[f32; 4]>()[..probes.len()].iter().map(|c| [c[0], c[1], c[2]]).collect();
    gpu::keep(gpu::LIGHTS, lights);
    gpu::keep(gpu::ENV, env_px);
    gpu::keep(gpu::ENV_LEVELS, env_levels);
    Ok(light)
}

/// The models casting shadows, posed and placed; their triangles arranged in trees for the
/// processor's rays when it first needs them.
struct Casting<'m> {
    models: Vec<(&'m veymelo_model::Model, &'m veymelo_model::Pose, Mat4)>,
    trees: std::sync::OnceLock<Vec<crate::meshes::Caster>>,
    /// Each model's glass, if it is see-through: its shadow lets light through, tinted.
    glass: Vec<Option<Glass>>,
    /// Whether the light glass bends lands where it is bent to (caustics): then glass lets no
    /// light straight through to a flat layer.
    caustics: bool,
}

/// A see-through model casting a shadow: how much it is seen through and how much less face
/// on, how much light bends going into it, and where its faces start.
#[derive(Clone, Copy)]
struct Glass {
    clear: f32,
    rolloff: f32,
    ior: f32,
    first: usize,
}

/// The most glass surfaces a shadow's ray is followed through, and how far past one the next is
/// looked for.
const GLASS_CROSSINGS: usize = 16;
const GLASS_STEP: f64 = 1e-3;

/// A glass face's way (its corners' between, at the second and third corners' weights `u`, `v`)
/// and colour there.
fn glass_at(f: &crate::meshes::Face, u: f64, v: f64) -> (Vec3, [f32; 3]) {
    let w = [1.0 - u - v, u, v];
    let n = vec3((0..3).map(|j| f.normals[j][0] as f64 * w[j]).sum(), (0..3).map(|j| f.normals[j][1] as f64 * w[j]).sum(), (0..3).map(|j| f.normals[j][2] as f64 * w[j]).sum());
    let n = if n.length() > 1e-12 { n.normalize() } else { vec3(0.0, 0.0, 1.0) };
    (n, std::array::from_fn(|c| (0..3).map(|j| f.colours[j][c] * w[j] as f32).sum()))
}

/// How much light a glass surface lets through crossing it along `dir` (where it faces `n`): half
/// its clearness and tint (going in and coming out again, the light is let through as much and
/// tinted as much as what is seen through the glass), less clear the more it is crossed slant if
/// it rolls off.
fn through_glass(clear: f32, rolloff: f32, n: Vec3, dir: Vec3, tint: f32) -> f32 {
    let facing = n.dot(dir).abs().clamp(0.0, 1.0) as f32;
    (clear * (1.0 - rolloff * (1.0 - facing)) * tint.max(0.0)).sqrt()
}

impl Casting<'_> {
    fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    fn trees(&self) -> &[crate::meshes::Caster] {
        self.trees.get_or_init(|| self.models.iter().map(|(m, pose, place)| crate::meshes::Caster::new(m, pose, place)).collect())
    }

    /// Their triangles in the world, three corners each.
    fn corners(&self) -> Vec<[f32; 3]> {
        self.models.iter().flat_map(|(m, pose, place)| crate::meshes::corners(m, pose, place)).collect()
    }
}

/// What a run's pixels are worked out from.
struct Scene<'s> {
    planes: &'s [Plane<'s>],
    /// The model layers as drawn (their pieces numbered after the planes').
    shots: &'s [ModelShot],
    /// The models that cast shadows.
    casting: &'s Casting<'s>,
    /// Every model, and how far round a point the ambient occlusion looks for them (none: no
    /// light occludes).
    everything: &'s Casting<'s>,
    reach: Option<f64>,
    /// Every model's triangles as a reflection sees them, and where each model's start.
    faces: &'s [crate::meshes::Face],
    face_first: &'s [usize],
    /// The light bouncing onto each point it is gathered at, and how much of it surfaces take
    /// (0: none bounces).
    light: &'s [[f32; 3]],
    bounce: f32,
    /// The canvas's width (for the mattes).
    width: usize,
    lights: &'s [Light],
    casters: &'s [usize],
    cam: &'s Camera,
    /// The environment lighting it, if one does.
    env: Option<&'s scene::Environment>,
    /// What casts shadows, as the GPU reads it (none: the GPU does not draw it).
    shadows: Option<&'s crate::gpu::Shadows>,
}

impl Scene<'_> {
    /// The pixels in `area` (x0, y0, x1, y1) of `canvas` drawn on the GPU, as `pixel` draws
    /// them (a pixel with more pieces than the GPU keeps at one is drawn by the processor after),
    /// if it can draw this scene (one whose shadows it can find): whether it did.
    fn on_gpu(&self, canvas: &mut Image, area: [usize; 4]) -> Result<bool, String> {
        use crate::gpu;
        use veymelo_os::gpu::{Arg, bytes_of, run};
        let [x0, y0, x1, y1] = area;
        let Some(shadows) = self.shadows else { return Ok(false) };
        if self.planes.len() + self.shots.len() >= u32::MAX as usize {
            return Ok(false);
        }
        if x0 >= x1 || y0 >= y1 {
            return Ok(true);
        }
        let f = |v: f64| v as f32;
        let v4 = |v: Vec3, w: f64| [f(v.x), f(v.y), f(v.z), f(w)];
        use gpu::PIECE;
        // the planes' pictures: as they are (in `shadows` already), or lit and blurred out of
        // focus (copied here, one after another)
        let mut copied = None;
        let first: Vec<u32> = if self.planes.iter().any(|p| p.lit) {
            let total: usize = self.planes.iter().map(|p| p.buf.img.px.len()).sum();
            if total > u32::MAX as usize || total * 16 > 1 << 31 {
                return Ok(false);
            }
            let mut texels = gpu::take(gpu::TEXELS, total * 16)?;
            let mut first = Vec::with_capacity(self.planes.len());
            let mut out = &mut texels.slice_mut::<[f32; 4]>()[..total];
            let mut at = 0;
            for img in self.planes.iter().map(|p| &p.buf.img) {
                let (to, rest) = out.split_at_mut(img.px.len());
                veymelo_raster::par::chunks(to, PIECE, |k, c| c.copy_from_slice(&img.px[k * PIECE..k * PIECE + c.len()]));
                out = rest;
                first.push(at as u32);
                at += img.px.len();
            }
            copied = Some(texels);
            first
        } else {
            shadows.at.clone()
        };
        let texels = copied.as_ref().unwrap_or(&shadows.texels);
        // the model layers' colours (their depths apart)
        let colours: usize = self.shots.iter().map(|s| s.shot.px.len()).sum();
        if colours * 16 > 1 << 31 {
            return Ok(false);
        }
        let mut shot_px = gpu::take(gpu::SHOT_PX, colours * 16)?;
        {
            let mut out = &mut shot_px.slice_mut::<[f32; 4]>()[..colours];
            for s in self.shots {
                let (to, rest) = out.split_at_mut(s.shot.px.len());
                veymelo_raster::par::chunks(to, PIECE, |k, c| c.iter_mut().zip(&s.shot.px[k * PIECE..]).for_each(|(o, p)| *o = p.1));
                out = rest;
            }
        }
        // the track mattes, each once
        let mut mattes: Vec<f32> = Vec::new();
        let mut matte_at: Vec<(*const Vec<f32>, u32)> = Vec::new();
        let mut matte_of = |matte: Option<&std::sync::Arc<Vec<f32>>>| match matte {
            None => gpu::NONE,
            Some(mt) => {
                let key = std::sync::Arc::as_ptr(mt);
                match matte_at.iter().find(|(k, _)| *k == key) {
                    Some((_, o)) => *o,
                    None => {
                        let o = mattes.len() as u32;
                        mattes.extend_from_slice(mt);
                        matte_at.push((key, o));
                        o
                    }
                }
            }
        };
        let mut planes = Vec::with_capacity(self.planes.len());
        let mut depths = Vec::with_capacity(self.planes.len());
        for p in self.planes {
            // the perspective from a canvas point near the plane (whole, so x - cx0 is exact)
            let cx0 = ((p.reach[0] + p.reach[2]) / 2).clamp(0, canvas.width as i64) as f64;
            let cy0 = ((p.reach[1] + p.reach[3]) / 2).clamp(0, canvas.height as i64) as f64;
            let m = &p.back.0;
            let cw = m[2][0] * cx0 + m[2][1] * cy0 + m[2][2];
            if !cw.is_finite() || cw.abs() < 1e-300 {
                return Ok(false);
            }
            // u = (a x' + b y' + u0) / w and v = (d x' + e y' + v0) / w, w = g x' + h y' + 1
            let (a, b, u0) = (m[0][0] / cw, m[0][1] / cw, (m[0][0] * cx0 + m[0][1] * cy0 + m[0][2]) / cw);
            let (d, e, v0) = (m[1][0] / cw, m[1][1] / cw, (m[1][0] * cx0 + m[1][1] * cy0 + m[1][2]) / cw);
            let (g, h) = (m[2][0] / cw, m[2][1] / cw);
            let z0 = p.depth[0] * u0 + p.depth[1] * v0 + p.depth[2];
            depths.push(z0);
            let img = first[planes.len()];
            planes.push(gpu::Plane {
                m: [f(a - u0 * g), f(b - u0 * h), f(d - v0 * g), f(e - v0 * h)],
                w: [f(g), f(h), f(cx0), f(cy0)],
                at: [f(u0), f(v0), 0.0, f(z0)],
                dz: [f(p.depth[0]), f(p.depth[1]), 0.0, 0.0],
                tex: [f((u0 - p.buf.origin.x) * p.buf.scale), f((v0 - p.buf.origin.y) * p.buf.scale), f(p.buf.scale), 0.0],
                reach: p.reach.map(|v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32),
                img: [img as u32, p.buf.img.width as u32, p.buf.img.height as u32, 0],
                origin: v4(p.origin, p.opacity as f64),
                across: v4(p.across, 0.0),
                down: v4(p.down, 0.0),
                normal: v4(p.normal, 0.0),
                mat0: [p.material.ambient, p.material.diffuse, p.material.specular, p.material.shininess],
                mat1: [p.material.metal, p.material.transmission, f(p.material.exponent()), 0.0],
                flags: [p.material.casts, p.material.accepts, p.material.lit as u32, p.blend as u32],
                more: [p.seed, matte_of(p.matte.as_ref()), p.keep_under as u32, p.lit as u32],
                order: [p.order as u32, 0, 0, 0],
                mirror: [p.material.reflection, p.material.sharpness, p.material.rolloff, 0.0],
                map: gpu::Caster::grid(p.grid).0,
                cell: gpu::Caster::grid(p.grid).1,
                caustic: gpu::Caster::grid(p.caustic).0,
                bent: gpu::Caster::grid(p.caustic).1,
            });
        }
        // depths from the middle of the scene, so they keep their precision
        let middle = depths.iter().sum::<f64>() / depths.len().max(1) as f64;
        for (p, z0) in planes.iter_mut().zip(&depths) {
            p.at[2] = f(z0 - middle);
        }
        // the model layers: where their colours are, and their depths (from the middle too)
        let mut zs = gpu::take(gpu::DEPTHS, colours * 4)?;
        let mut shots = Vec::with_capacity(self.shots.len());
        {
            let mut out = &mut zs.slice_mut::<f32>()[..colours];
            let (mut colour, mut depth) = (0, 0);
            for s in self.shots {
                let q = &s.shot;
                let (to, rest) = out.split_at_mut(q.px.len());
                veymelo_raster::par::chunks(to, PIECE, |k, c| c.iter_mut().zip(&q.px[k * PIECE..]).for_each(|(o, p)| *o = f(p.0 - middle)));
                out = rest;
                shots.push(gpu::Shot {
                    reach: [q.x0, q.y0, q.x0 + q.w, q.y0 + q.h].map(|v| v.min(i32::MAX as usize) as i32),
                    at: [colour as u32, depth as u32, 0, 0],
                    how: [s.opacity, 0.0, 0.0, 0.0],
                    more: [s.seed, matte_of(s.matte.as_ref()), s.keep_under as u32, s.blend as u32],
                    order: [s.order as u32, 0, 0, 0],
                });
                colour += q.px.len();
                depth += q.px.len();
            }
        }
        let lights: Vec<gpu::Light> = self.lights.iter().map(gpu::Light::of).collect();
        let width = canvas.width;
        let (env, env_levels, turn, shining) = gpu::environment(self.env)?;
        let params = gpu::Params {
            a: [width as u32, x0 as u32, y0 as u32, (x1 - x0) as u32],
            b: [planes.len() as u32, lights.len() as u32, shadows.count, shots.len() as u32],
            eye: [f(self.cam.eye.x), f(self.cam.eye.y), f(self.cam.eye.z), turn],
            c: [shining, gpu::MOST_LEFT as u32, 0, 0],
            d: [self.env.map_or(0.0, |e| e.occlusion), self.reach.map_or(0.0, f), self.bounce, 0.0],
        };
        let (planes, lights, mattes, shots) = (gpu::take_with(gpu::PLANES, &planes)?, gpu::take_with(gpu::LIGHTS, &lights)?, gpu::take_with(gpu::MATTES, &mattes)?, gpu::take_with(gpu::SHOTS, &shots)?);
        let mut rows = gpu::take_with(gpu::ROWS, &canvas.px[y0 * width..y1 * width])?;
        // how many pixels it leaves to the processor (a new buffer is all zero), and which
        let mut count = veymelo_os::gpu::Buffer::new(4)?;
        let mut left_at = gpu::take(gpu::LEFT, 4 * gpu::MOST_LEFT)?;
        // the light bouncing onto each point
        let bounce = gpu::take_with(gpu::BOUNCE, &self.light.iter().map(|c| [c[0], c[1], c[2], 0.0]).collect::<Vec<[f32; 4]>>())?;
        let mut args = vec![
            Arg::Write(&mut rows),
            Arg::Read(&planes),
            Arg::Read(texels),
            Arg::Read(&lights),
            Arg::Read(&shadows.casters),
            Arg::Read(&mattes),
            Arg::Bytes(bytes_of(&params)),
            Arg::Read(&shots),
            Arg::Read(&zs),
            Arg::Read(&env),
            Arg::Read(&env_levels),
            Arg::Write(&mut count),
            Arg::Write(&mut left_at),
            Arg::Read(&shadows.texels),
            Arg::Read(&shot_px),
            Arg::Read(&shadows.faces),
            Arg::Read(&bounce),
        ];
        let (kernel, traced) = shadows.kernel(gpu::SCENE, gpu::SCENE_RAYS)?;
        args.extend(traced);
        run(&kernel, &args, [x1 - x0, y1 - y0])?;
        drop(args);
        let n = count.slice::<u32>()[0] as usize;
        if n > gpu::MOST_LEFT {
            return Ok(false);
        }
        canvas.px[y0 * width..y1 * width].copy_from_slice(&rows.slice::<[f32; 4]>()[..(y1 - y0) * width]);
        for &i in &left_at.slice::<u32>()[..n] {
            let i = i as usize;
            canvas.px[i] = self.pixel(i % width, i / width, canvas.px[i]);
        }
        if let Some(texels) = copied {
            gpu::keep(gpu::TEXELS, texels);
        }
        for (slot, b) in [(gpu::SHOT_PX, shot_px), (gpu::ROWS, rows), (gpu::PLANES, planes), (gpu::LIGHTS, lights), (gpu::MATTES, mattes), (gpu::SHOTS, shots), (gpu::DEPTHS, zs), (gpu::ENV, env), (gpu::ENV_LEVELS, env_levels), (gpu::LEFT, left_at), (gpu::BOUNCE, bounce)] {
            gpu::keep(slot, b);
        }
        gpu::drew_scene();
        Ok(true)
    }

    /// Canvas pixel (x, y), over `under` (what is drawn below the run).
    fn pixel(&self, x: usize, y: usize, under: Px) -> Px {
        let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
        let pieces = self.gather(x, y, cx, cy);
        if pieces.is_empty() {
            return under;
        }
        if pieces.len() > 1 && self.crossing(&pieces, cx, cy) {
            // a crossing goes through this pixel: four looks, put together
            let mut sum = [0f32; 4];
            for (dx, dy) in [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)] {
                let p = self.lay(&self.gather(x, y, cx + dx, cy + dy), under, x, y);
                (0..4).for_each(|k| sum[k] += p[k] * 0.25);
            }
            return sum;
        }
        self.lay(&pieces, under, x, y)
    }

    /// The pieces at canvas point (cx, cy), lit.
    fn gather(&self, x: usize, y: usize, cx: f64, cy: f64) -> Vec<Piece> {
        let mut out = Vec::new();
        for (i, p) in self.planes.iter().enumerate() {
            let (xi, yi) = (x as i64, y as i64);
            if xi < p.reach[0] || xi >= p.reach[2] || yi < p.reach[1] || yi >= p.reach[3] || p.material.casts == 2 {
                continue;
            }
            let Some((u, v, z)) = p.under(cx, cy) else { continue };
            let texel = self.sample(p, u, v, cx, cy);
            if texel[3] <= 1e-5 {
                continue;
            }
            let color = if p.lit { texel } else { self.light(i, u, v, texel) };
            out.push(Piece { z, plane: i, color });
        }
        for (k, s) in self.shots.iter().enumerate() {
            if let Some((z, color)) = s.shot.at(x, y) {
                out.push(Piece { z, plane: self.planes.len() + k, color });
            }
        }
        out
    }

    /// The pieces at canvas pixel (x, y) for the render passes (`bounds`: each member's
    /// rectangle in its own units, for where on it a piece is).
    fn hits(&self, x: usize, y: usize, bounds: &[(usize, veymelo_geom::Rect)]) -> Vec<Hit> {
        let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
        let facing = |n: Vec3| {
            let n = self.cam.view.apply_vec(n);
            let n = if n.length() > 1e-12 { n.normalize() } else { vec3(0.0, 0.0, -1.0) };
            let n = if n.z > 0.0 { n * -1.0 } else { n };
            [n.x as f32, n.y as f32, n.z as f32]
        };
        let mut out = Vec::new();
        for p in self.planes {
            let (xi, yi) = (x as i64, y as i64);
            if xi < p.reach[0] || xi >= p.reach[2] || yi < p.reach[1] || yi >= p.reach[3] || p.material.casts == 2 {
                continue;
            }
            let Some((u, v, z)) = p.under(cx, cy) else { continue };
            let a = p.texel(u, v)[3] * p.opacity * p.matte.as_ref().map_or(1.0, |m| m[y * self.width + x]);
            if a <= 1e-4 {
                continue;
            }
            let uv = bounds.iter().find(|(o, _)| *o == p.order).map_or([0.0; 2], |(_, r)| [((u - r.x0) / r.width().max(1e-9)) as f32, ((v - r.y0) / r.height().max(1e-9)) as f32]);
            out.push(Hit { z: z as f32, a: a.min(1.0), member: p.order, normal: facing(p.normal), uv, material: None });
        }
        for s in self.shots {
            let Some((z, color)) = s.shot.at(x, y) else { continue };
            let a = color[3] * s.opacity * s.matte.as_ref().map_or(1.0, |m| m[y * self.width + x]);
            if a <= 1e-4 {
                continue;
            }
            let f = s.shot.facts_at(x, y);
            let n = vec3(f.normal[0] as f64, f.normal[1] as f64, f.normal[2] as f64);
            out.push(Hit { z: z as f32, a: a.min(1.0), member: s.order, normal: facing(n), uv: f.uv, material: (f.material != u32::MAX).then_some(f.material as usize) });
        }
        out
    }

    /// How piece source `i` (a plane, or a model after them) lays over what is behind it.
    fn how(&self, i: usize) -> Laid<'_> {
        match self.planes.get(i) {
            Some(p) => Laid { order: p.order, opacity: p.opacity, blend: p.blend, seed: p.seed, matte: p.matte.as_ref(), keep_under: p.keep_under },
            None => {
                let s = &self.shots[i - self.planes.len()];
                Laid { order: s.order, opacity: s.opacity, blend: s.blend, seed: s.seed, matte: s.matte.as_ref(), keep_under: s.keep_under }
            }
        }
    }

    /// A plane's picture at (u, v), with more samples where it is seen smaller than its pixels.
    fn sample(&self, p: &Plane, u: f64, v: f64, cx: f64, cy: f64) -> Px {
        let step = [p.under(cx + 1.0, cy), p.under(cx, cy + 1.0)].iter().flatten().map(|(a, b, _)| (a - u).hypot(b - v)).fold(0.0, f64::max) * p.buf.scale;
        // at its own size or larger, one sample (a hair over a pixel a pixel, from rounding, is
        // still its own size)
        if step <= 1.25 {
            return p.texel(u, v);
        }
        let n = (step.ceil() as usize).clamp(2, 4);
        let mut sum = [0f32; 4];
        let mut count = 0;
        for j in 0..n {
            for i in 0..n {
                let (sx, sy) = (cx - 0.5 + (i as f64 + 0.5) / n as f64, cy - 0.5 + (j as f64 + 0.5) / n as f64);
                if let Some((su, sv, _)) = p.under(sx, sy) {
                    let t = p.texel(su, sv);
                    (0..4).for_each(|k| sum[k] += t[k]);
                }
                count += 1;
            }
        }
        sum.map(|s| s / count as f32)
    }

    /// Whether the pieces' depths cross within the pixel (their difference is less than they
    /// change across it).
    fn crossing(&self, pieces: &[Piece], cx: f64, cy: f64) -> bool {
        // a model's edges are smooth already: only planes are looked at again
        let change: Vec<f64> = pieces
            .iter()
            .map(|q| match self.planes.get(q.plane) {
                Some(p) => [p.under(cx + 1.0, cy), p.under(cx, cy + 1.0)].iter().flatten().map(|(_, _, z)| (z - q.z).abs()).sum::<f64>(),
                None => 0.0,
            })
            .collect();
        (0..pieces.len()).any(|a| (a + 1..pieces.len()).any(|b| (pieces[a].z - pieces[b].z).abs() < (change[a] + change[b]) * 0.75))
    }

    /// The pieces laid over `under` from the farthest to the nearest (at the same depth, as in
    /// the stack), each by its layer's mode and opacity.
    fn lay(&self, pieces: &[Piece], under: Px, x: usize, y: usize) -> Px {
        let mut order: Vec<&Piece> = pieces.iter().collect();
        order.sort_by(|a, b| b.z.total_cmp(&a.z).then(self.how(a.plane).order.cmp(&self.how(b.plane).order)));
        let mut out = under;
        let width = self.width;
        for q in order {
            let laid = self.how(q.plane);
            let mut k = laid.opacity;
            if let Some(m) = laid.matte {
                k *= m[y * width + x];
            }
            if laid.keep_under {
                k *= under[3];
            }
            if k <= 0.0 {
                continue;
            }
            let src = q.color.map(|v| v * k);
            out = blend(laid.blend, out, src, noise(x, y, laid.seed));
        }
        out
    }

    /// Plane `i`'s own picture lit by the lights, pixel by pixel — those that can be seen on a
    /// `w` × `h` canvas, `margin` pixels past it allowed for the blur (the rest cannot show).
    fn lit_picture(&self, i: usize, w: f64, h: f64, margin: f64) -> Buf {
        let p = &self.planes[i];
        let mut out = p.own.clone();
        if (self.lights.is_empty() && self.env.is_none()) || !p.material.lit {
            return out;
        }
        let (width, origin, scale) = (out.img.width, out.origin, out.scale);
        let m = &p.onto.0;
        veymelo_raster::par::rows(&mut out.img.px, width, |y, row| {
            for (x, px) in row.iter_mut().enumerate() {
                if px[3] <= 1e-5 {
                    continue;
                }
                let (u, v) = (origin.x + (x as f64 + 0.5) / scale, origin.y + (y as f64 + 0.5) / scale);
                let z = p.depth[0] * u + p.depth[1] * v + p.depth[2];
                let q = m[2][0] * u + m[2][1] * v + m[2][2];
                let (sx, sy) = ((m[0][0] * u + m[0][1] * v + m[0][2]) / q, (m[1][0] * u + m[1][1] * v + m[1][2]) / q);
                if z < NEAR || sx < -margin || sy < -margin || sx > w + margin || sy > h + margin {
                    continue;
                }
                *px = self.light(i, u, v, *px);
            }
        });
        out
    }

    /// A plane's texel (premultiplied) at its point (u, v), lit by the lights.
    fn light(&self, i: usize, u: f64, v: f64, texel: Px) -> Px {
        let p = &self.planes[i];
        if (self.lights.is_empty() && self.env.is_none()) || !p.material.lit {
            return texel;
        }
        let a = texel[3];
        let rgb = [texel[0] / a, texel[1] / a, texel[2] / a];
        let at = p.origin + p.across * u + p.down * v;
        let viewer = (self.cam.eye - at).normalize();
        // how open the sky round it is (on the side it is seen from)
        let open = if self.reach.is_some() && p.material.accepts != 0 { self.open(i, at, if p.normal.dot(viewer) < 0.0 { -p.normal } else { p.normal }) } else { 1.0 };
        let (mut lit, shadow) = if self.lights.is_empty() { ([0.0; 3], 0.0) } else { scene::shade(rgb, at, p.normal, viewer, &p.material, self.lights, &|l, q| self.unshadowed(i, l, q), open) };
        // the environment's light on it: its diffuse and shine as their options say (50 % as it
        // is), not a metal, its roughness as dull as its shine is broad
        if let Some(env) = self.env {
            let m = &p.material;
            let add = env.light(rgb, p.normal, viewer, 0.0, 1.0 - m.shininess, m.diffuse * 2.0, m.specular * 2.0, 1.0 - env.occlusion * (1.0 - open));
            (0..3).for_each(|k| lit[k] += add[k]);
        }
        // the light bouncing onto it from the surfaces round it
        if let Some(g) = p.grid.filter(|_| self.bounce > 0.0) {
            let e = g.at(self.light, u, v);
            let k = p.material.diffuse * 2.0 * self.bounce;
            (0..3).for_each(|c| lit[c] += rgb[c] * k * e[c]);
        }
        // the light glass bends onto it
        if let Some(g) = p.caustic {
            let e = g.at(self.light, u, v);
            (0..3).for_each(|c| lit[c] += rgb[c] * p.material.diffuse * e[c]);
        }
        // what it mirrors, mixed in
        if p.material.reflection > 0.0 {
            let facing = if p.normal.dot(viewer) < 0.0 { -p.normal } else { p.normal };
            let r = p.material.mirrors(facing, viewer);
            if r > 0.0 {
                let seen = self.mirrored(i, at, facing, viewer, p.material.sharpness, lit);
                lit = std::array::from_fn(|k| lit[k] * (1.0 - r) + seen[k] * r);
            }
        }
        if p.material.accepts == 2 {
            // only the shadows on it
            return [0.0, 0.0, 0.0, a * shadow];
        }
        [lit[0] * a, lit[1] * a, lit[2] * a, a]
    }

    /// What a surface at world point `q` (on plane `own`, or none) facing `n` mirrors, seen from
    /// `viewer`, as sharply as `sharpness`: along each way it looks (`scene::mirror_rays`), the
    /// nearest model or flat layer (where it is more there than not) as a mirror sees it, else
    /// the environment's light, else `none`; put together.
    fn mirrored(&self, own: usize, q: Vec3, n: Vec3, viewer: Vec3, sharpness: f32, none: [f32; 3]) -> [f32; 3] {
        let rays = scene::mirror_rays(q, n, viewer, sharpness);
        let mut sum = [0f32; 3];
        for dir in &rays {
            let dir = *dir;
            let seen = if dir.dot(n) <= 0.0 { none } else { self.seen_along(own, q, dir, sharpness).unwrap_or(none) };
            (0..3).for_each(|k| sum[k] += seen[k]);
        }
        sum.map(|v| v / rays.len() as f32)
    }

    /// What the ray from world point `q` (on plane `own`, or none) along `dir` sees, as a mirror
    /// `sharpness` sharp sees it: the surface it meets, and — where that mirrors too — what it
    /// mirrors, from mirror to mirror up to `MIRROR_DEPTH` deep; past the last surface met, the
    /// environment's light; none where the first meets nothing and there is no environment.
    fn seen_along(&self, own: usize, q: Vec3, dir: Vec3, sharpness: f32) -> Option<[f32; 3]> {
        // each surface met: how it looks, and how much it mirrors
        let mut chain: Vec<([f32; 3], f32)> = Vec::with_capacity(MIRROR_DEPTH);
        let mut beyond = None;
        let (mut own, mut q, mut dir) = (own, q, dir);
        for level in 0..MIRROR_DEPTH {
            let Some(m) = self.meet(own, q, dir) else {
                beyond = self.env.map(|e| e.shine(dir, if level == 0 { 1.0 - sharpness } else { 0.0 }));
                break;
            };
            let r = m.mirror.0 * (1.0 - m.mirror.1 * m.n.dot(-dir).clamp(0.0, 1.0) as f32);
            chain.push((m.lit, r));
            if r <= 0.0 {
                break;
            }
            (own, q, dir) = (m.own, q + dir * m.d, dir - m.n * (2.0 * dir.dot(m.n)));
        }
        // put together from the last met: each its own look and as much of what is beyond it as
        // it mirrors (nothing beyond: its own look alone)
        for (lit, r) in chain.into_iter().rev() {
            beyond = Some(match beyond {
                Some(b) if r > 0.0 => std::array::from_fn(|k| lit[k] * (1.0 - r) + b[k] * r),
                _ => lit,
            });
        }
        beyond
    }

    /// What is seen through a model at world point `q` on its surface facing `n`, from `viewer`:
    /// the way in bent by index of refraction `ior`, on through it to where it goes out (bent
    /// again; reflected whole inside where it cannot, a few times at most), and what that way
    /// meets past it (`seen_along`); none where it meets nothing and there is no environment.
    fn seen_through(&self, q: Vec3, n: Vec3, viewer: Vec3, ior: f32) -> Option<[f32; 3]> {
        let ior = ior as f64;
        let mut dir = scene::refract(-viewer, n, 1.0 / ior)?;
        let mut at = q;
        for _ in 0..GLASS_TURNS {
            // where it goes out: the nearest of the model's (any model's) surfaces inside
            let mut out: Option<(f64, Vec3)> = None;
            for (k, tree) in self.everything.trees().iter().enumerate() {
                let far = out.map_or(f64::INFINITY, |o| o.0);
                if let Some((d, which, u, v)) = tree.nearest(at, dir, 0.5, far) {
                    let f = &self.faces[self.face_first[k] + which as usize];
                    let w = [1.0 - u - v, u, v];
                    let nn = vec3((0..3).map(|j| f.normals[j][0] as f64 * w[j]).sum(), (0..3).map(|j| f.normals[j][1] as f64 * w[j]).sum(), (0..3).map(|j| f.normals[j][2] as f64 * w[j]).sum());
                    out = Some((d, if nn.length() > 1e-12 { nn.normalize() } else { -dir }));
                }
            }
            let Some((d, n_out)) = out else { break };
            at += dir * d;
            // the surface faces out; going out, the way meets it from inside
            let inward = if n_out.dot(dir) > 0.0 { -n_out } else { n_out };
            match scene::refract(dir, inward, ior) {
                Some(leaving) => return self.seen_along(usize::MAX, at, leaving, 1.0),
                None => dir = dir - inward * (2.0 * dir.dot(inward)),
            }
        }
        self.seen_along(usize::MAX, at, dir, 1.0)
    }

    /// The nearest surface the ray from world point `q` (on plane `own`, or none) along `dir`
    /// meets — a model, or a flat layer where it is more there than not — as a mirror sees it.
    fn meet(&self, own: usize, q: Vec3, dir: Vec3) -> Option<Met> {
        // the nearest model (a pixel off the surface, so it does not see itself where it is)
        let mut best: Option<Met> = None;
        for (k, tree) in self.everything.trees().iter().enumerate() {
            let far = best.as_ref().map_or(f64::INFINITY, |b| b.d);
            if let Some((d, which, u, v)) = tree.nearest(q, dir, 1.0, far) {
                let f = &self.faces[self.face_first[k] + which as usize];
                let w = [1.0 - u - v, u, v];
                let mix = |c: &[[f32; 4]; 3], i: usize| (0..3).map(|j| c[j][i] as f64 * w[j]).sum::<f64>();
                let nn = vec3(mix(&f.normals, 0), mix(&f.normals, 1), mix(&f.normals, 2));
                let nn = if nn.length() > 1e-12 { nn.normalize() } else { -dir };
                let nn = if nn.dot(dir) > 0.0 { -nn } else { nn };
                let rgb = [0, 1, 2].map(|i| mix(&f.colours, i) as f32);
                let fell: [f32; 3] = std::array::from_fn(|c| (0..3).map(|j| self.light.get(f.probes[j] as usize).map_or(0.0, |p| p[c] * w[j] as f32)).sum::<f32>() * self.bounce);
                let lit = scene::seen_in_mirror(rgb, q + dir * d, nn, f.normals[0][3], f.normals[1][3], f.normals[2][3] > 0.5, self.lights, self.env, fell);
                best = Some(Met { d, lit: std::array::from_fn(|i| lit[i] + f.glow[i]), n: nn, mirror: (f.colours[0][3], f.colours[1][3]), own: usize::MAX });
            }
        }
        // the nearest flat layer nearer still
        for (k, p) in self.planes.iter().enumerate() {
            let facing = dir.dot(p.normal);
            if k == own || p.material.casts == 2 || facing.abs() < 1e-12 {
                continue;
            }
            let s = (p.origin - q).dot(p.normal) / facing;
            if s <= 1e-6 || s >= best.as_ref().map_or(f64::INFINITY, |b| b.d) {
                continue;
            }
            let hit = p.unworld.apply(q + dir * s);
            let t = p.shadow_texel(hit.x, hit.y);
            if t[3] < 0.5 {
                continue;
            }
            let rgb = [t[0] / t[3], t[1] / t[3], t[2] / t[3]];
            let nn = if facing > 0.0 { -p.normal } else { p.normal };
            let mut fell = p.grid.filter(|_| self.bounce > 0.0).map_or([0.0; 3], |g| g.at(self.light, hit.x, hit.y).map(|v| v * self.bounce));
            if let Some(g) = p.caustic {
                let e = g.at(self.light, hit.x, hit.y);
                (0..3).for_each(|c| fell[c] += e[c] * 0.5);
            }
            let lit = scene::seen_in_mirror(rgb, q + dir * s, nn, p.material.ambient, p.material.diffuse, p.material.lit, self.lights, self.env, fell);
            best = Some(Met { d: s, lit, n: nn, mirror: (p.material.reflection, p.material.rolloff), own: k });
        }
        best
    }

    /// How much light gets through to world point `q` (on plane `own`) from a point of a light
    /// `len` away along `dir`, per colour: none past a model in the way (a pixel off the surface,
    /// so one does not shade itself where it is), as much as each flat layer casting shadows in
    /// the way lets through.
    fn passes(&self, own: usize, q: Vec3, dir: Vec3, len: f64) -> [f32; 3] {
        let mut pass = [1f32; 3];
        for (k, tree) in self.casting.trees().iter().enumerate() {
            match self.casting.glass.get(k).copied().flatten() {
                None => {
                    if tree.blocks(q, dir, 1.0, len - 1.0) {
                        return [0.0; 3];
                    }
                }
                // through glass, surface after surface: each lets through its part, tinted (none
                // to a flat layer when the light it bends lands where it is bent to)
                Some(g) => {
                    let mut near = 1.0;
                    for _ in 0..GLASS_CROSSINGS {
                        let Some((d, which, u, v)) = tree.nearest(q, dir, near, len - 1.0) else { break };
                        if self.casting.caustics && own != usize::MAX {
                            return [0.0; 3];
                        }
                        let (n, rgb) = glass_at(&self.faces[g.first + which as usize], u, v);
                        for c in 0..3 {
                            pass[c] *= through_glass(g.clear, g.rolloff, n, dir, rgb[c]);
                        }
                        near = d + GLASS_STEP;
                    }
                }
            }
        }
        for &c in self.casters.iter().filter(|&&c| c != own) {
            let p = &self.planes[c];
            let facing = dir.dot(p.normal);
            if facing.abs() < 1e-12 {
                continue;
            }
            let s = (p.origin - q).dot(p.normal) / facing;
            if s <= 1e-6 || s >= len - 1e-6 {
                continue;
            }
            let hit = p.unworld.apply(q + dir * s);
            let t = p.shadow_texel(hit.x, hit.y);
            let a = t[3].clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let tr = p.material.transmission;
            for k in 0..3 {
                pass[k] *= 1.0 - a + a * tr * (t[k] / a);
            }
        }
        pass
    }

    /// The light coming back to `probe` from all round (the mean along its rays, `bounce::rays`),
    /// `before` what the bounce before gathered.
    fn gather_light(&self, probe: &crate::bounce::Probe, before: &[[f32; 3]]) -> [f32; 3] {
        let mut sum = [0f32; 3];
        for dir in crate::bounce::rays(probe.at, probe.n) {
            let c = self.bounced_from(probe.own, probe.at, dir, before);
            (0..3).for_each(|k| sum[k] += c[k]);
        }
        sum.map(|v| v / crate::bounce::RAYS as f32)
    }

    /// The light the ray from world point `q` (on plane `own`, or none) along `dir` brings back:
    /// the nearest model's or flat layer's (where it is more there than not), lit simply
    /// (`lit_simply`), else the environment's sky, else none.
    fn bounced_from(&self, own: usize, q: Vec3, dir: Vec3, before: &[[f32; 3]]) -> [f32; 3] {
        let mut best: Option<(f64, [f32; 3])> = None;
        for (k, tree) in self.everything.trees().iter().enumerate() {
            let far = best.map_or(f64::INFINITY, |b| b.0);
            if let Some((d, which, u, v)) = tree.nearest(q, dir, 1.0, far) {
                let f = &self.faces[self.face_first[k] + which as usize];
                let w = [1.0 - u - v, u, v];
                let mix = |c: &[[f32; 4]; 3], i: usize| (0..3).map(|j| c[j][i] as f64 * w[j]).sum::<f64>();
                let nn = vec3(mix(&f.normals, 0), mix(&f.normals, 1), mix(&f.normals, 2));
                let nn = if nn.length() > 1e-12 { nn.normalize() } else { -dir };
                let nn = if nn.dot(dir) > 0.0 { -nn } else { nn };
                let rgb = [0, 1, 2].map(|i| mix(&f.colours, i) as f32);
                let e: [f32; 3] = std::array::from_fn(|c| (0..3).map(|j| before.get(f.probes[j] as usize).map_or(0.0, |p| p[c] * w[j] as f32)).sum());
                let lit = self.lit_simply(rgb, q + dir * d, nn, (f.normals[0][3], f.normals[1][3], f.normals[2][3] > 0.5), usize::MAX, e);
                best = Some((d, std::array::from_fn(|i| lit[i] + f.glow[i])));
            }
        }
        for (k, p) in self.planes.iter().enumerate() {
            let facing = dir.dot(p.normal);
            if k == own || p.material.casts == 2 || facing.abs() < 1e-12 {
                continue;
            }
            let s = (p.origin - q).dot(p.normal) / facing;
            if s <= 1e-6 || s >= best.map_or(f64::INFINITY, |b| b.0) {
                continue;
            }
            let hit = p.unworld.apply(q + dir * s);
            let t = p.shadow_texel(hit.x, hit.y);
            if t[3] < 0.5 {
                continue;
            }
            let rgb = [t[0] / t[3], t[1] / t[3], t[2] / t[3]];
            let nn = if facing > 0.0 { -p.normal } else { p.normal };
            let e = p.grid.map_or([0.0; 3], |g| g.at(before, hit.x, hit.y));
            best = Some((s, self.lit_simply(rgb, q + dir * s, nn, (p.material.ambient, p.material.diffuse, p.material.lit), k, e)));
        }
        best.map(|b| b.1).unwrap_or_else(|| self.env.map_or([0.0; 3], |e| e.shine(dir, 1.0)))
    }

    /// A surface the bouncing light meets, lit simply: colour `rgb` (straight) at `at` facing `n`
    /// (toward where the ray came from), taking `ambient` of the ambient lights, `diffuse` of the
    /// light falling on it (its shadows looked for along one ray to each light's middle), of the
    /// light the environment lets fall on it, and of what the bounce before brought it
    /// (`before`); as it is when it takes no light (`lit`).
    fn lit_simply(&self, rgb: [f32; 3], at: Vec3, n: Vec3, (ambient, diffuse, lit): (f32, f32, bool), own: usize, before: [f32; 3]) -> [f32; 3] {
        if !lit {
            return rgb;
        }
        let (mut amb, mut dif) = ([0f32; 3], [0f32; 3]);
        for l in self.lights {
            if l.kind == LightKind::Ambient {
                (0..3).for_each(|c| amb[c] += l.color[c]);
                continue;
            }
            let k = l.reach(at);
            let (to, far) = l.toward(at);
            let facing = n.dot(to);
            if k <= 0.0 || facing <= 0.0 {
                continue;
            }
            let through = if l.shadows && (self.casters.iter().any(|&c| c != own) || !self.casting.is_empty()) {
                let p = self.passes(own, at, to, far);
                p.map(|v| 1.0 - l.darkness as f32 * (1.0 - v))
            } else {
                [1.0; 3]
            };
            (0..3).for_each(|c| dif[c] += l.color[c] * (facing * k) as f32 * through[c]);
        }
        let fall = self.env.map_or([0.0; 3], |e| e.falling(n));
        std::array::from_fn(|c| rgb[c] * (ambient * amb[c] + diffuse * dif[c] + diffuse * 2.0 * (fall[c] + self.bounce * before[c])))
    }

    /// How much of the sky round world point `q` (on plane `own`, or none, facing `n`) is open,
    /// as far as the ambient occlusion looks: the share of the rays from it (`scene::open_rays`)
    /// that meet no model, each as much as the flat layers it passes let it through.
    fn open(&self, own: usize, q: Vec3, n: Vec3) -> f32 {
        let Some(reach) = self.reach else { return 1.0 };
        let mut sum = 0.0f32;
        for dir in scene::open_rays(q, n) {
            // a pixel off the surface, so it does not hide itself where it is
            if self.everything.trees().iter().any(|c| c.blocks(q, dir, 1.0, reach)) {
                continue;
            }
            let mut pass = 1.0f32;
            for (k, p) in self.planes.iter().enumerate() {
                let facing = dir.dot(p.normal);
                if k == own || facing.abs() < 1e-12 {
                    continue;
                }
                let s = (p.origin - q).dot(p.normal) / facing;
                if s <= 1e-6 || s >= reach {
                    continue;
                }
                let hit = p.unworld.apply(q + dir * s);
                pass *= 1.0 - p.shadow_texel(hit.x, hit.y)[3].clamp(0.0, 1.0);
            }
            sum += pass;
        }
        sum / scene::OPEN_RAYS as f32
    }

    /// How much of light `l` reaches world point `q` on plane `own` past the layers that cast
    /// shadows (per colour, 0 to 1), its darkness applied; a diffuse light looked at from many
    /// points of its disc.
    fn unshadowed(&self, own: usize, l: usize, q: Vec3) -> [f32; 3] {
        let light = &self.lights[l];
        if !self.casters.iter().any(|&c| c != own) && self.casting.is_empty() {
            return [1.0; 3];
        }
        let (to, far) = light.toward(q);
        let pass_from = |dir: Vec3, len: f64| self.passes(own, q, dir, len);
        let dark = light.darkness as f32;
        let finish = |sum: [f32; 3], n: f32| sum.map(|s| 1.0 - dark * (1.0 - s / n));
        if light.diffusion <= 0.0 {
            return finish(pass_from(to, far), 1.0);
        }
        // a diffuse light is a disc across the way to it (a parallel light's as if it were 1000
        // pixels away), looked at from points spread evenly over it (a sunflower), the pattern
        // turned a little from point to point
        let side = if to.x.abs() < 0.9 { vec3(1.0, 0.0, 0.0) } else { vec3(0.0, 1.0, 0.0) };
        let a = to.cross(side).normalize();
        let b = to.cross(a);
        let parallel = light.kind == LightKind::Parallel;
        let centre = if parallel { q + to * 1000.0 } else { light.at };
        let turn = noise((q.x * 7.0) as usize, (q.y * 7.0) as usize, (q.z as i64) as u32) as f64 * std::f64::consts::TAU;
        let from = |k: usize, n: usize| -> [f32; 3] {
            let r = light.diffusion * ((k as f64 + 0.5) / n as f64).sqrt();
            let ang = k as f64 * 2.399_963 + turn;
            let d = centre + a * (r * ang.cos()) + b * (r * ang.sin()) - q;
            let len = d.length();
            pass_from(d / len.max(1e-9), if parallel { f64::INFINITY } else { len })
        };
        // its middle and eight points round its rim first: where they agree (wholly lit or wholly
        // shadowed) that is it (a shadow's edge coming over the disc crosses its rim); in the
        // soft edge between, many points over it
        let rim = |k: usize| -> [f32; 3] {
            let r = if k == 8 { 0.0 } else { light.diffusion };
            let ang = k as f64 * std::f64::consts::TAU / 8.0 + turn;
            let d = centre + a * (r * ang.cos()) + b * (r * ang.sin()) - q;
            let len = d.length();
            pass_from(d / len.max(1e-9), if parallel { f64::INFINITY } else { len })
        };
        let few: Vec<[f32; 3]> = (0..9).map(rim).collect();
        let agree = few.iter().all(|p| (0..3).all(|c| (p[c] - few[0][c]).abs() < 1e-3));
        let (n, samples): (usize, Vec<[f32; 3]>) = if agree { (1, vec![few[0]]) } else { (SOFT_POINTS, (0..SOFT_POINTS).map(|k| from(k, SOFT_POINTS)).collect()) };
        let mut sum = [0f32; 3];
        for p in &samples {
            (0..3).for_each(|k| sum[k] += p[k]);
        }
        finish(sum, n as f32)
    }
}

/// A layer's picture blurred as the camera sees it out of focus: each point as much as its depth
/// is from the focus distance (`depth` gives the depth at layer point (u, v)), `size` the
/// composition pixels a layer unit is at depth 1. Bright parts are first made brighter, as
/// light out of focus is; a fast rectangle iris blurs into squares, the others into rounds.
fn defocus(mut buf: Buf, focus: &Focus, depth: [f64; 3], size: f64) -> Buf {
    // how wide (the picture's pixels) a point at layer point (u, v) spreads
    let scale = buf.scale;
    let spread = move |u: f64, v: f64| {
        let z = depth[0] * u + depth[1] * v + depth[2];
        if z <= NEAR { 0.0 } else { focus.spread(z) * z / size.max(1e-9) * scale }
    };
    let (w, h) = (buf.img.width as f64 / buf.scale, buf.img.height as f64 / buf.scale);
    let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(dx, dy)| spread(buf.origin.x + dx, buf.origin.y + dy));
    let most = corners.iter().fold(0.0f64, |a, &b| a.max(b)).min(400.0);
    if most < 0.5 {
        return buf;
    }
    // room for it to spread past the edges
    buf.pad((most / 2.0).ceil() as usize + 2);
    if focus.gain > 0.0 {
        for p in buf.img.px.iter_mut() {
            let a = p[3];
            if a <= 0.0 {
                continue;
            }
            let l = (0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]) / a;
            let over = ((l as f64 - focus.threshold) / (1.0 - focus.threshold).max(1e-3)).clamp(0.0, 1.0) as f32;
            if over > 0.0 {
                let boost = 1.0 + focus.gain as f32 * 4.0 * over;
                let grey = l * a;
                for c in p.iter_mut().take(3) {
                    // the brightened part a little toward its own colour or toward white
                    *c = (grey + (*c - grey) * (1.0 - focus.saturation as f32 * 0.5)) * boost;
                }
            }
        }
    }
    // a few blurs from none to the most, each pixel between the two nearest its own spread
    let levels = ((most / 2.0).ceil() as usize + 1).clamp(2, 8);
    let blurs: Vec<Image> = (0..levels)
        .map(|k| {
            let d = most * k as f64 / (levels - 1) as f64;
            if d < 0.5 {
                buf.img.clone()
            } else if focus.sides == 0 {
                veymelo_raster::filter::boxes(&buf.img, d, d * focus.aspect.min(4.0), 1, veymelo_raster::filter::Edge::Clear)
            } else {
                // three boxes a third as wide each: round
                veymelo_raster::filter::boxes(&buf.img, d / 3f64.sqrt(), d / 3f64.sqrt() * focus.aspect.min(4.0), 3, veymelo_raster::filter::Edge::Clear)
            }
        })
        .collect();
    let (width, origin) = (buf.img.width, buf.origin);
    for (i, p) in buf.img.px.iter_mut().enumerate() {
        let (u, v) = (origin.x + ((i % width) as f64 + 0.5) / scale, origin.y + ((i / width) as f64 + 0.5) / scale);
        let f = (spread(u, v).min(most) / most * (levels - 1) as f64).clamp(0.0, (levels - 1) as f64);
        let k = (f.floor() as usize).min(levels - 2);
        let t = (f - k as f64) as f32;
        let (a, b) = (blurs[k].px[i], blurs[k + 1].px[i]);
        *p = std::array::from_fn(|c| a[c] + (b[c] - a[c]) * t);
    }
    buf
}
