use super::*;
use veymelo_color::Rgba;
use veymelo_keyframe::Keyframe;
use veymelo_project::{Expression, Marker, Source};
use veymelo_time::FrameRate;

const TEN: Tick = Tick(10 * veymelo_time::TICKS_PER_SECOND);

fn secs(s: f64) -> Tick {
    Tick::from_seconds(s)
}

/// A 400 × 300 composition at 30 frames a second with a null "A" at (100, 50) above a 100 × 100
/// solid "B" at (200, 150).
fn scene() -> (Project, u64) {
    let mut p = Project::new();
    let id = p.add_comp("Main", Comp::new(400, 300, FrameRate::FPS_30, TEN));
    let mut a = Layer::new(p.alloc(), "A", Source::Null, TEN);
    a.props.prop_mut("transform/position").unwrap().set_value(Value::Vec3([100.0, 50.0, 0.0])).unwrap();
    let mut b = Layer::new(p.alloc(), "B", Source::Solid { color: Rgba::WHITE, width: 100, height: 100 }, TEN);
    b.props.prop_mut("transform/position").unwrap().set_value(Value::Vec3([200.0, 150.0, 0.0])).unwrap();
    p.comp_mut(id).unwrap().layers.extend([a, b]);
    (p, id)
}

fn layer_mut<'p>(p: &'p mut Project, comp: u64, name: &str) -> &'p mut Layer {
    p.comp_mut(comp).unwrap().layers.iter_mut().find(|l| l.name == name).unwrap()
}

fn set(p: &mut Project, comp: u64, layer: &str, path: &str, text: &str) {
    layer_mut(p, comp, layer).props.prop_mut(path).unwrap().expression = Some(Expression { text: text.into(), enabled: true });
}

fn keys(p: &mut Project, comp: u64, layer: &str, path: &str, k: &[(f64, f64)]) {
    let keys = k.iter().map(|(t, v)| Keyframe::new(secs(*t), Value::Number(*v))).collect();
    layer_mut(p, comp, layer).props.prop_mut(path).unwrap().set_keys(keys).unwrap();
}

/// The value at `t` and the notes.
fn get(p: &Project, comp: u64, layer: &str, path: &str, t: f64) -> (Value, Vec<String>) {
    let e = Eval::new(p);
    let c = p.comp(comp).unwrap();
    let l = c.layers.iter().find(|l| l.name == layer).unwrap();
    let v = e.value(l.props.prop(path).unwrap(), e.place(c, l), secs(t));
    (v, e.notes())
}

fn num(p: &Project, comp: u64, layer: &str, path: &str, t: f64) -> f64 {
    let (v, notes) = get(p, comp, layer, path, t);
    assert!(notes.is_empty(), "{notes:?}");
    v.as_f64()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// The value of an expression on B's rotation (or of `path`) at `t`.
fn run(text: &str, t: f64) -> f64 {
    let (mut p, c) = scene();
    set(&mut p, c, "B", "transform/rotation", text);
    num(&p, c, "B", "transform/rotation", t)
}

#[test]
fn numbers_arrays_and_time() {
    let (mut p, c) = scene();
    set(&mut p, c, "B", "transform/position", "value + [10, 20]");
    assert_eq!(get(&p, c, "B", "transform/position", 0.0).0, Value::Vec3([210.0, 170.0, 0.0]));
    set(&mut p, c, "B", "transform/position", "[value[0] * 2, 5]");
    assert_eq!(get(&p, c, "B", "transform/position", 0.0).0, Value::Vec3([400.0, 5.0, 0.0]), "a point of two keeps its depth");
    assert_eq!(run("time * 90", 1.5), 135.0);
    assert_eq!(run("var a = 2; function sq(x) { return x * x; } sq(a) + 1", 0.0), 5.0);
    assert_eq!(run("var a = []; for (var i = 0; i < 3; i++) a.push(i * 2); a[2] + a.length", 0.0), 7.0);
    assert_eq!(run("[1, 2, 3].map(function (x) { return x * 10; }).reduce(function (s, x) { return s + x; }, 0)", 0.0), 60.0);
    assert_eq!(run("index * 10 + thisComp.numLayers", 0.0), 22.0);
    assert_eq!(run("thisComp.width / 4 + thisComp.frameDuration * 30", 0.0), 101.0);
    assert_eq!(run("Math.round(-2.5) + Math.max(1, 7, 3) + Math.PI * 0", 0.0), 5.0);
    assert_eq!(run("clamp(250, 0, 100) + clamp([-5, 5], 0, 1)[1]", 0.0), 101.0);
    assert_eq!(run("length([0, 0], [3, 4]) + dot([1, 2], [3, 4])", 0.0), 16.0);
    assert_eq!(run("timeToFrames(1.5) + framesToTime(15)", 0.0), 45.5);
}

#[test]
fn eases() {
    assert_eq!(run("linear(time, 0, 2, 0, 100)", 1.0), 50.0);
    assert_eq!(run("linear(time, 0, 2, 0, 100)", 3.0), 100.0, "held at the end");
    assert_eq!(run("ease(time, 0, 2, 0, 100)", 1.0), 50.0);
    assert!(near(run("easeIn(0.25, 0, 100)", 0.0), 100.0 * 0.0625 * 1.75));
    assert!(near(run("easeOut(0.25, 0, 100)", 0.0), 100.0 * (1.0 - 0.5625 * 1.25)));
    assert_eq!(run("linear(time, 1, 0, 0, 100)", 0.25), 75.0, "a range running backward");
    assert_eq!(run("linear(0.5, [0, 0], [10, 20])[1]", 0.0), 10.0);
}

#[test]
fn other_layers_and_their_expressions() {
    let (mut p, c) = scene();
    set(&mut p, c, "A", "transform/position", "value + [0, 100]");
    set(&mut p, c, "B", "transform/position", "thisComp.layer(\"A\").transform.position + [5, 0]");
    assert_eq!(get(&p, c, "B", "transform/position", 0.0).0, Value::Vec3([105.0, 150.0, 0.0]), "B sees A with its expression");
    set(&mut p, c, "B", "transform/position", "thisComp.layer(1).position");
    assert_eq!(get(&p, c, "B", "transform/position", 0.0).0, Value::Vec3([100.0, 150.0, 0.0]));
    set(&mut p, c, "B", "transform/rotation", "thisComp.layer(thisLayer, -1).name.length + thisLayer.width");
    assert_eq!(num(&p, c, "B", "transform/rotation", 0.0), 101.0);
    // where a point of a layer is in the composition (B's anchor is at its top left)
    set(&mut p, c, "B", "transform/rotation", "");
    set(&mut p, c, "B", "transform/position", "");
    set(&mut p, c, "B", "transform/scale", "[200, 200]");
    set(&mut p, c, "A", "transform/rotation", "thisComp.layer(\"B\").toComp([10, 10])[0]");
    assert_eq!(num(&p, c, "A", "transform/rotation", 0.0), 220.0);
    set(&mut p, c, "A", "transform/rotation", "thisComp.layer(\"B\").fromComp([220, 170])[1]");
    assert_eq!(num(&p, c, "A", "transform/rotation", 0.0), 10.0);
    // a layer's own place, from an expression its place depends on, uses its keys' value there
    set(&mut p, c, "B", "transform/rotation", "toComp([0, 0])[0]");
    let (v, notes) = get(&p, c, "B", "transform/rotation", 0.0);
    assert_eq!((v, notes.len()), (Value::Number(200.0), 1));
    // effects and masks are found by name; the lists are empty here
    set(&mut p, c, "B", "transform/rotation", "effect(\"Blur\")(\"Amount\")");
    let (v, notes) = get(&p, c, "B", "transform/rotation", 0.0);
    assert_eq!(v, Value::Number(0.0));
    assert!(notes[0].contains("Effects has no Blur"), "{notes:?}");
}

#[test]
fn loops() {
    let (mut p, c) = scene();
    keys(&mut p, c, "B", "transform/rotation", &[(0.0, 0.0), (1.0, 90.0)]);
    let at = |p: &mut Project, text: &str, t: f64| {
        set(p, c, "B", "transform/rotation", text);
        num(p, c, "B", "transform/rotation", t)
    };
    assert_eq!(at(&mut p, "loopOut()", 0.5), 45.0, "inside the keys it is their value");
    assert!(near(at(&mut p, "loopOut(\"cycle\")", 1.25), 22.5));
    assert!(near(at(&mut p, "loopOut(\"pingpong\")", 1.25), 67.5));
    assert!(near(at(&mut p, "loopOut(\"pingpong\")", 2.25), 22.5));
    assert!(near(at(&mut p, "loopOut(\"offset\")", 2.25), 202.5));
    assert!(near(at(&mut p, "loopOut(\"continue\")", 1.5), 135.0));
    keys(&mut p, c, "B", "transform/rotation", &[(1.0, 0.0), (2.0, 90.0), (3.0, 0.0)]);
    assert!(near(at(&mut p, "loopIn(\"cycle\")", 0.25), 67.5), "as at 2.25");
    assert!(near(at(&mut p, "loopIn(\"cycle\", 1)", 0.25), 22.5), "only the first stretch of keys");
    assert!(near(at(&mut p, "loopIn(\"pingpong\", 1)", 0.75), 22.5));
    assert!(near(at(&mut p, "loopIn(\"offset\", 1)", 0.5), -45.0));
    assert!(near(at(&mut p, "loopIn(\"continue\")", 0.0), -90.0));
    assert!(near(at(&mut p, "loopOutDuration(\"cycle\", 1)", 3.5), 45.0));
    set(&mut p, c, "B", "transform/rotation", "loopOut(\"bounce\")");
    assert!(get(&p, c, "B", "transform/rotation", 4.0).1[0].contains("\"bounce\" is not a way to loop"));
}

#[test]
fn keys_markers_velocity() {
    let (mut p, c) = scene();
    keys(&mut p, c, "B", "transform/rotation", &[(0.0, 0.0), (2.0, 60.0)]);
    set(&mut p, c, "A", "transform/rotation", "var r = thisComp.layer(\"B\").rotation; r.key(2).time * 100 + r.key(2).value + r.numKeys");
    assert_eq!(num(&p, c, "A", "transform/rotation", 0.0), 262.0);
    set(&mut p, c, "A", "transform/rotation", "thisComp.layer(\"B\").transform.rotation.velocity");
    assert!(near(num(&p, c, "A", "transform/rotation", 1.0), 30.0));
    set(&mut p, c, "A", "transform/rotation", "var r = thisComp.layer(\"B\").transform.rotation; r.valueAtTime(1.5) + r.nearestKey(0.4).index");
    assert!(near(num(&p, c, "A", "transform/rotation", 0.0), 46.0));
    set(&mut p, c, "A", "transform/rotation", "nearestKey(0.4).index");
    assert!(get(&p, c, "A", "transform/rotation", 0.0).1[0].ends_with("rotation has no keys"));
    p.comp_mut(c).unwrap().markers.push(Marker { time: secs(2.5), comment: "go".into(), ..Marker::default() });
    set(&mut p, c, "A", "transform/rotation", "thisComp.marker.key(\"go\").time + thisComp.marker.nearestKey(9).index + thisComp.marker.numKeys");
    assert_eq!(num(&p, c, "A", "transform/rotation", 0.0), 4.5);
    // a layer's markers move with it
    let b = layer_mut(&mut p, c, "B");
    b.start = secs(1.0);
    b.markers.push(Marker { time: secs(0.5), ..Marker::default() });
    set(&mut p, c, "A", "transform/rotation", "thisComp.layer(\"B\").marker.key(1).time");
    assert_eq!(num(&p, c, "A", "transform/rotation", 0.0), 1.5);
}

#[test]
fn wiggle_and_random_are_the_same_every_time() {
    let (mut p, c) = scene();
    set(&mut p, c, "B", "transform/position", "wiggle(3, 40)");
    set(&mut p, c, "A", "transform/position", "wiggle(3, 40)");
    let at = |p: &Project, l: &str, t: f64| get(p, c, l, "transform/position", t).0.as_vec3();
    let mut moved = 0.0f64;
    for f in 0..90 {
        let w = at(&p, "B", f as f64 / 30.0);
        assert!((w[0] - 200.0).abs() <= 40.0 && (w[1] - 150.0).abs() <= 40.0 && w[2] == 0.0, "a 2D layer does not wiggle in depth: {w:?}");
        moved = moved.max((w[0] - 200.0).abs());
    }
    assert!(moved > 10.0, "it moves");
    assert_eq!(at(&p, "B", 1.0), at(&p.clone(), "B", 1.0), "a copy wiggles the same");
    assert_ne!(at(&p, "B", 1.0)[0] - 200.0, at(&p, "A", 1.0)[0] - 100.0, "each property its own way");
    assert_ne!(at(&p, "B", 1.0)[0] - 200.0, at(&p, "B", 1.0)[1] - 150.0, "each part its own way");
    // random: new each frame, unless seeded to hold
    let r = |text: &str, t: f64| run(text, t);
    assert_eq!(r("random()", 1.0), r("random()", 1.0));
    assert_ne!(r("random()", 1.0), r("random()", 1.0 + 1.0 / 30.0));
    assert_ne!(r("random() - random()", 1.0), 0.0, "each draw is new");
    assert_eq!(r("seedRandom(5, true); random(10, 20)", 0.0), r("seedRandom(5, true); random(10, 20)", 3.0));
    assert_ne!(r("seedRandom(5, true); random()", 0.0), r("seedRandom(6, true); random()", 0.0));
    for f in 0..50 {
        let v = r("random(10, 20)", f as f64 / 30.0);
        assert!((10.0..20.0).contains(&v));
    }
    let n = r("noise(time)", 0.3);
    assert!((-1.0..=1.0).contains(&n) && (n - r("noise(time)", 0.3001)).abs() < 0.01);
}

#[test]
fn values_that_do_not_fit() {
    let (mut p, c) = scene();
    set(&mut p, c, "B", "transform/opacity", "150");
    assert_eq!(num(&p, c, "B", "transform/opacity", 0.0), 100.0, "held at its limit");
    let fails = |text: &str, path: &str| {
        let (mut p, c) = scene();
        set(&mut p, c, "B", path, text);
        let (v, notes) = get(&p, c, "B", path, 0.0);
        assert_eq!(notes.len(), 1, "{notes:?}");
        (v, notes[0].clone())
    };
    let (v, why) = fails("[1, 2]", "transform/rotation");
    assert_eq!(v, Value::Number(0.0), "its keys' value instead");
    assert_eq!(why, "the expression on transform/rotation of \"B\" failed: it gives an array of 2, and a number is wanted");
    assert!(fails("wigle(2, 3)", "transform/rotation").1.ends_with("there is nothing called \"wigle\""));
    assert!(fails("1 +", "transform/rotation").1.contains("is not written right at character"));
    assert!(fails("while (true) {}", "transform/rotation").1.contains("takes too long"));
    assert!(fails("5", "transform/position").1.contains("a number, and"));
    // two expressions that lead to each other stop, each with its keys' value
    let (mut p, c) = scene();
    set(&mut p, c, "A", "transform/rotation", "thisComp.layer(\"B\").rotation + 1");
    set(&mut p, c, "B", "transform/rotation", "thisComp.layer(\"A\").rotation + 1");
    let (v, notes) = get(&p, c, "A", "transform/rotation", 0.0);
    assert_eq!(v, Value::Number(2.0), "B saw A's keys' value (0), A saw B (1)");
    assert!(notes[0].contains("leads back to itself"), "{notes:?}");
    // its own property is its keys' value: no loop
    let (mut p, c) = scene();
    set(&mut p, c, "B", "transform/rotation", "thisProperty.value + thisLayer.rotation + 3");
    assert_eq!(num(&p, c, "B", "transform/rotation", 0.0), 3.0);
}

#[test]
fn words() {
    assert_eq!(run("\"abc\".toUpperCase().length + \"a-b-c\".split(\"-\").length", 0.0), 6.0);
    let (mut p, c) = scene();
    let mut t = Layer::new(p.alloc(), "Title", Source::Text, TEN);
    let doc = TextDoc { font: "Tahoma".into(), size: 40.0, ..TextDoc::of("x") };
    t.props.prop_mut("text/sourceText").unwrap().set_value(Value::Doc(Box::new(doc.clone()))).unwrap();
    p.comp_mut(c).unwrap().layers.push(t);
    set(&mut p, c, "Title", "text/sourceText", "\"Frame \" + timeToFrames() + \" / \" + (time).toFixed(2) + \" \" + timeToTimecode()");
    let (v, notes) = get(&p, c, "Title", "text/sourceText", 1.5);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(v, Value::Doc(Box::new(TextDoc { text: "Frame 45 / 1.50 0:00:01:15".into(), ..doc })), "the new words keep their look");
    set(&mut p, c, "Title", "text/sourceText", "value.toUpperCase() + thisComp.layer(\"B\").name");
    assert_eq!(get(&p, c, "Title", "text/sourceText", 0.0).0.doc().unwrap().text, "XB");
}

#[test]
fn time_held_and_switched_off() {
    let (mut p, c) = scene();
    keys(&mut p, c, "B", "transform/rotation", &[(0.0, 0.0), (1.0, 100.0)]);
    set(&mut p, c, "B", "transform/rotation", "posterizeTime(4); value");
    assert_eq!(num(&p, c, "B", "transform/rotation", 0.9), 75.0);
    set(&mut p, c, "B", "transform/rotation", "posterizeTime(2); time");
    assert_eq!(num(&p, c, "B", "transform/rotation", 0.9), 0.5);
    layer_mut(&mut p, c, "B").props.prop_mut("transform/rotation").unwrap().expression.as_mut().unwrap().enabled = false;
    assert_eq!(num(&p, c, "B", "transform/rotation", 0.5), 50.0, "a switched off expression does nothing");
}

#[test]
fn matrix_parents_and_time_remap() {
    let (mut p, c) = scene();
    let a_id = p.comp(c).unwrap().layers[0].id;
    set(&mut p, c, "A", "transform/rotation", "90");
    layer_mut(&mut p, c, "B").parent = Some(a_id);
    let e = Eval::new(&p);
    let comp = p.comp(c).unwrap();
    let m = e.matrix(e.place(comp, &comp.layers[1]), Tick::ZERO);
    let q = m.apply(vec2(0.0, 0.0));
    // B at (200, 150) under A turned a quarter about (100, 50)
    assert!(near(q.x, 100.0 - 150.0) && near(q.y, 50.0 + 200.0), "{q:?}");
    assert!(e.notes().is_empty());
}

#[test]
fn three_d_points_through_the_camera() {
    let (mut p, c) = scene();
    // B made 3D: through the standard camera its z = 0 plane is as in 2D
    layer_mut(&mut p, c, "B").switches.three_d = true;
    set(&mut p, c, "A", "transform/rotation", "thisComp.layer(\"B\").toComp([10, 10])[0]");
    assert!(near(num(&p, c, "A", "transform/rotation", 0.0), 210.0));
    // pushed back as far again as the camera is: half as far from the middle
    let zoom = veymelo_project::scene::default_zoom(400.0);
    layer_mut(&mut p, c, "B").props.prop_mut("transform/position").unwrap().set_value(Value::Vec3([200.0, 150.0, zoom])).unwrap();
    assert!(near(num(&p, c, "A", "transform/rotation", 0.0), 200.0 + 10.0 / 2.0), "{}", num(&p, c, "A", "transform/rotation", 0.0));
    // in the world, and back
    set(&mut p, c, "A", "transform/rotation", "thisComp.layer(\"B\").toWorld([10, 10])[2]");
    assert!(near(num(&p, c, "A", "transform/rotation", 0.0), zoom));
    set(&mut p, c, "A", "transform/rotation", "var b = thisComp.layer(\"B\"); b.fromComp(b.toComp([30, 40]))[1]");
    assert!(near(num(&p, c, "A", "transform/rotation", 0.0), 40.0));
    // the active camera, when there is one
    set(&mut p, c, "A", "transform/rotation", "thisComp.activeCamera.toWorld([0, 0, 0])[2]");
    let (_, notes) = get(&p, c, "A", "transform/rotation", 0.0);
    assert!(notes[0].contains("no camera"), "{notes:?}");
    let mut cam = Layer::new(p.alloc(), "Camera", Source::Camera, TEN);
    cam.props.prop_mut("transform/position").unwrap().set_value(Value::Vec3([200.0, 150.0, -800.0])).unwrap();
    p.comp_mut(c).unwrap().layers.insert(0, cam);
    assert!(near(num(&p, c, "A", "transform/rotation", 0.0), -800.0));
}

/// A made-up effect "Blur" with one setting, for the reference's examples.
static BLUR: veymelo_project::GroupDef = veymelo_project::GroupDef {
    id: "blur",
    name: "Blur",
    body: veymelo_project::Body::Fixed(&[veymelo_project::DefNode::Prop(veymelo_project::PropDef::new("blurriness", "Blurriness", veymelo_keyframe::Kind::Number, Value::Number(8.0)))]),
    doc: "a made-up blur",
};

/// Pictures as a stand-in for drawing: every layer a box at (1, 2), 3 × 4, red all over.
struct Drawn;

impl Pictures for Drawn {
    fn bounds(&self, _: &Project, _: u64, _: u64, _: Tick) -> Option<[f64; 4]> {
        Some([1.0, 2.0, 3.0, 4.0])
    }
    fn sample(&self, _: &Project, _: u64, _: u64, _: [f64; 2], _: [f64; 2], _: bool, _: Tick) -> Option<[f64; 4]> {
        Some([1.0, 0.0, 0.0, 1.0])
    }
}

/// Every example of the reference works: on a number or on a point.
#[test]
fn every_example_in_the_reference_works() {
    use veymelo_project::{Catalog, Group, Node, shapes};
    show_pictures(&Drawn);
    let (mut p, c) = scene();
    let catalog = Catalog::base();
    // B: an effect, a mask, keys; S: a group holding a rectangle; T: words; a camera; markers
    let (fx, mask) = (p.alloc(), p.alloc());
    {
        let b = layer_mut(&mut p, c, "B");
        b.props.group_mut("effects").unwrap().children.push(Node::Group(Group::instance(&BLUR, fx, "Blur")));
        b.props.group_mut("masks").unwrap().children.push(Node::Group(Group::instance(catalog.find("masks", "mask").unwrap(), mask, "Mask 1")));
        b.markers.push(Marker { time: secs(1.0), comment: "hi".into(), ..Marker::default() });
    }
    keys(&mut p, c, "B", "transform/rotation", &[(0.0, 0.0), (2.0, 90.0)]);
    let (group, rect, s_id, t_id, cam_id) = (p.alloc(), p.alloc(), p.alloc(), p.alloc(), p.alloc());
    let mut s = Layer::new(s_id, "S", Source::Shape, TEN);
    let mut g = Group::instance(&shapes::GROUP, group, "Group 1");
    g.group_mut("contents").unwrap().children.push(Node::Group(Group::instance(&shapes::RECT, rect, "Rectangle 1")));
    s.props.group_mut("contents").unwrap().children.push(Node::Group(g));
    let mut t = Layer::new(t_id, "T", Source::Text, TEN);
    t.props.prop_mut("text/sourceText").unwrap().set_value(Value::Doc(Box::new(veymelo_keyframe::TextDoc::of("Hello")))).unwrap();
    let cam = Layer::new(cam_id, "Cam", Source::Camera, TEN);
    let comp = p.comp_mut(c).unwrap();
    comp.layers.extend([s, t, cam]);
    comp.markers.push(Marker { time: secs(2.0), comment: "two".into(), ..Marker::default() });
    let mut wrong = Vec::new();
    let bogus = reference::Entry { write: "nothing", about: "", example: "thisComp.noSuchThing" };
    let all = reference::reference().into_iter().chain([("Check", vec![bogus])]);
    let mut caught = false;
    for (category, entries) in all {
        for entry in entries {
            let mut tried = Vec::new();
            for path in ["transform/rotation", "transform/position"] {
                let mut q = p.clone();
                set(&mut q, c, "B", path, entry.example);
                let e = Eval::new(&q);
                let comp = q.comp(c).unwrap();
                let l = comp.layers.iter().find(|l| l.name == "B").unwrap();
                match e.work(l.props.prop(path).unwrap(), e.place(comp, l), secs(1.0)) {
                    Ok(_) => {
                        tried.clear();
                        break;
                    }
                    Err((_, why)) => tried.push(why),
                }
            }
            if category == "Check" {
                caught = !tried.is_empty();
            } else if !tried.is_empty() {
                wrong.push(format!("{category} / {}: {} -> {}", entry.write, entry.example, tried.join(" | ")));
            }
        }
    }
    assert!(caught, "a name nothing has fails");
    assert!(wrong.is_empty(), "{} examples fail:\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn boxes_colours_and_groups_above() {
    show_pictures(&Drawn);
    assert!(near(run("thisComp.layer(\"B\").sourceRectAtTime().height", 0.0), 4.0));
    assert!(near(run("thisComp.layer(\"B\").sampleImage([0, 0])[0]", 0.0), 1.0));
    // B's rotation: in Transform, its place there; two up is the layer
    assert!(near(run("thisProperty.propertyGroup(1).name == \"Transform\" ? 1 : 0", 0.0), 1.0));
    assert!(near(run("thisProperty.propertyGroup(2).index", 0.0), 2.0));
    assert!(near(run("thisProperty.propertyGroup(9).name == \"B\" ? 1 : 0", 0.0), 1.0));
    let (p, c) = scene();
    let rotation = p.comp(c).unwrap().layers[1].props.group("transform").unwrap().children.iter().position(|n| n.id() == "rotation").unwrap() + 1;
    assert!(near(run("thisProperty.propertyIndex", 0.0), rotation as f64));
}

/// What expressions give, kept to hold them while their code is written anew: each expression
/// worked out on keyed properties every frame for four seconds. Run by hand:
///   VEYMELO_GOLDENS=/tmp/g cargo test -p veymelo-expr --release expr_goldens -- --ignored
/// The first run writes `/tmp/g/expr.txt`, later runs compare with it.
#[test]
#[ignore]
fn expr_goldens() {
    let Some(dir) = std::env::var_os("VEYMELO_GOLDENS").map(std::path::PathBuf::from) else { return };
    let loops = ["cycle", "pingpong", "offset", "continue"];
    let mut exprs: Vec<String> = vec![];
    for kind in loops {
        for n in [0, 1, 2] {
            exprs.push(format!("loopOut(\"{kind}\", {n})"));
            exprs.push(format!("loopIn(\"{kind}\", {n})"));
        }
        for d in [0.0, 0.5, 1.5] {
            exprs.push(format!("loopOutDuration(\"{kind}\", {d})"));
            exprs.push(format!("loopInDuration(\"{kind}\", {d})"));
        }
    }
    for w in ["wiggle(2, 30)", "wiggle(5, 10, 3, 0.7)", "wiggle(1.5, 50, 1, 0.5, time * 2)", "wiggle(0.7, 80, 4, 0.3)", "temporalWiggle(2, 0.3)", "smooth(0.5, 7)", "smooth()"] {
        exprs.push(w.into());
    }
    for r in ["seedRandom(3, true); random()", "seedRandom(5); random(10, 20)", "seedRandom(2, true); gaussRandom()", "seedRandom(9); gaussRandom(5, 15)", "gaussRandom([0, 0], [1, 1])[1]", "noise(time * 2) * 100", "noise([time, 3]) * 100", "random([10, 20])[0]"] {
        exprs.push(r.into());
    }
    for t in ["timeToFrames()", "timeToFrames(time, 24)", "timeToFrames(time + 0.01, 25, true)", "framesToTime(37)", "posterizeTime(3); time", "posterizeTime(7); value", "posterizeTime(0.5); time * 10"] {
        exprs.push(t.into());
    }
    for e in ["ease(time, 0.5, 2.5, 10, 90)", "easeIn(time, 0, 3, -20, 40)", "easeOut(time, 1, 2, 0, 100)", "linear(time, 1, 3, 5, 25)", "ease(time, 0, 4)", "easeIn(time, 2, 1)", "ease(time, 0, 2, [0, 10], [100, 30])[1]", "linear(time * 0.7)"] {
        exprs.push(e.into());
    }
    let mut lines = vec![];
    for keyset in [vec![(0.5, 0.0), (1.5, 90.0)], vec![(1.0, 10.0), (1.6, 40.0), (2.4, -30.0), (3.0, 5.0)]] {
        for text in &exprs {
            let (mut p, c) = scene();
            keys(&mut p, c, "B", "transform/rotation", &keyset);
            set(&mut p, c, "B", "transform/rotation", text);
            let vals: Vec<String> = (0..=120).map(|k| format!("{:.12}", get(&p, c, "B", "transform/rotation", k as f64 / 30.0).0.as_f64())).collect();
            lines.push(format!("{:?}\t{text}\t{}", keyset, vals.join(" ")));
        }
    }
    let file = dir.join("expr.txt");
    if !file.exists() || std::env::var_os("VEYMELO_GOLDENS_WRITE").is_some() {
        std::fs::write(&file, lines.join("\n")).unwrap();
        eprintln!("{} expressions written", lines.len());
        return;
    }
    let before = std::fs::read_to_string(&file).unwrap();
    let mut changed = vec![];
    for (was, now) in before.lines().zip(&lines) {
        let (a, b): (Vec<&str>, Vec<&str>) = (was.split('\t').collect(), now.split('\t').collect());
        let most = a[2].split(' ').zip(b[2].split(' ')).map(|(x, y)| (x.parse::<f64>().unwrap() - y.parse::<f64>().unwrap()).abs()).fold(0.0, f64::max);
        if a[1] != b[1] || most > 1e-9 {
            changed.push(format!("{} {}: most {most}", a[0], a[1]));
        }
    }
    eprintln!("{} expressions, {} changed", lines.len(), changed.len());
    for c in &changed {
        eprintln!("  {c}");
    }
    assert!(changed.is_empty());
}

#[test]
fn gauss_random_keeps_nine_in_ten_within_its_range() {
    let (mut p, c) = scene();
    set(&mut p, c, "B", "transform/rotation", "seedRandom(4, true); var n = 0; for (var i = 0; i < 20000; i++) { var g = gaussRandom(); if (g >= 0 && g <= 1) n++; } n / 20000");
    let share = num(&p, c, "B", "transform/rotation", 0.0);
    assert!((share - 0.9).abs() < 0.007, "{share}");
}
