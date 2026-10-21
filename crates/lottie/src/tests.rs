use super::*;
use veymelo_color::Rgba;
use veymelo_keyframe::{Ease, Gradient, Interp, Keyframe, ShapePath, TextDoc, Value};
use veymelo_project::shapes as defs;
use veymelo_project::{Catalog, Comp, Footage, FootageKind, Group, ItemKind, Layer, LightKind, Marker, Matte, MatteKind, Node, Project, Property, Source};
use veymelo_time::{FrameRate, Tick};

fn t(s: f64) -> Tick {
    Tick::from_seconds(s)
}

fn set(g: &mut Group, path: &str, v: Value) {
    g.prop_mut(path).unwrap_or_else(|e| panic!("{e}")).set_value(v).unwrap_or_else(|e| panic!("{path}: {e}"));
}

fn keys(g: &mut Group, path: &str, keys: Vec<Keyframe>) {
    g.prop_mut(path).unwrap_or_else(|e| panic!("{e}")).set_keys(keys).unwrap_or_else(|e| panic!("{path}: {e}"));
}

fn eased(time: f64, v: Value, out: Ease, inn: Ease) -> Keyframe {
    let mut k = Keyframe::new(t(time), v);
    (k.interp_in, k.interp_out, k.ease_in, k.ease_out) = (Interp::Bezier, Interp::Bezier, vec![inn], vec![out]);
    k
}

fn item(p: &mut Project, def: &'static veymelo_project::GroupDef, name: &str) -> Group {
    Group::instance(def, p.alloc(), name)
}

fn push(g: &mut Group, list: &str, child: Group) {
    g.group_mut(list).unwrap().children.push(Node::Group(child));
}

/// A composition with something of everything Lottie holds, and a light it does not.
fn project() -> (Project, u64) {
    let mut p = Project::new();
    let rate = FrameRate::new(25, 1).unwrap();
    // a composition shown inside
    let mut inner = Comp::new(80, 60, rate, t(4.0));
    let mut sq = Layer::new(p.alloc(), "Square", Source::Solid { color: Rgba::from_hex("#3366cc").unwrap(), width: 40, height: 40 }, t(4.0));
    set(&mut sq.props, "transform/position", Value::Vec3([40.0, 30.0, 0.0]));
    inner.layers.push(sq);
    let inner_id = p.add_comp("Inner", inner);

    let mut comp = Comp::new(200, 100, rate, t(4.0));
    comp.markers.push(Marker { time: t(1.0), comment: "beat".into(), ..Marker::default() });

    // a shape layer
    let mut shapes = Layer::new(p.alloc(), "Shapes", Source::Shape, t(4.0));
    let mut group = item(&mut p, &defs::GROUP, "Box");
    let mut rect = item(&mut p, &defs::RECT, "Rect");
    keys(&mut rect, "size", vec![eased(0.0, Value::Vec2([40.0, 20.0]), Ease::EASY, Ease::EASY), eased(2.0, Value::Vec2([80.0, 40.0]), Ease::EASY, Ease { speed: 10.0, influence: 0.5 })]);
    set(&mut rect, "roundness", Value::Number(4.0));
    let mut fill = item(&mut p, &defs::FILL, "Fill");
    set(&mut fill, "color", Value::Color(Rgba::from_hex("#ff0000").unwrap()));
    let mut stroke = item(&mut p, &defs::STROKE, "Stroke");
    set(&mut stroke, "width", Value::Number(3.0));
    set(&mut stroke, "cap", Value::Choice(1));
    set(&mut stroke, "dashes/dash", Value::Number(5.0));
    set(&mut stroke, "dashes/gap", Value::Number(2.0));
    for child in [rect, stroke, fill] {
        push(&mut group, "contents", child);
    }
    set(&mut group, "transform/position", Value::Vec2([10.0, 5.0]));
    set(&mut group, "transform/rotation", Value::Number(15.0));
    let mut star = item(&mut p, &defs::STAR, "Star");
    set(&mut star, "points", Value::Number(6.0));
    set(&mut star, "direction", Value::Choice(1));
    let mut path = item(&mut p, &defs::PATH, "Tri");
    let tri = ShapePath { points: vec![[0.0, 0.0], [30.0, 0.0], [15.0, 25.0]], in_tangents: vec![[0.0; 2], [0.0, -5.0], [3.0, 0.0]], out_tangents: vec![[5.0, 0.0], [0.0; 2], [0.0; 2]], closed: true, feather: veymelo_keyframe::Feathers::NONE };
    let tri2 = ShapePath { points: vec![[0.0, 0.0], [40.0, 0.0], [20.0, 35.0]], ..tri.clone() };
    keys(&mut path, "path", vec![Keyframe::new(t(0.0), Value::Path(tri)), Keyframe::new(t(1.0), Value::Path(tri2)).hold(), Keyframe::new(t(2.0), Value::Path(ShapePath::polygon(&[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]], true)))]);
    let mut grad = item(&mut p, &defs::GRADIENT_FILL, "Gradient");
    set(&mut grad, "type", Value::Choice(1));
    set(&mut grad, "colors", Value::Gradient(Gradient { colors: vec![(0.0, [1.0, 0.0, 0.0]), (0.5, [0.0, 1.0, 0.0]), (1.0, [0.0, 0.0, 1.0])], opacities: vec![(0.0, 1.0), (1.0, 0.25)] }));
    let mut trim = item(&mut p, &defs::TRIM, "Trim");
    keys(&mut trim, "end", vec![Keyframe::new(t(0.0), Value::Number(0.0)), Keyframe::new(t(3.0), Value::Number(100.0))]);
    let mut rep = item(&mut p, &defs::REPEATER, "Repeat");
    set(&mut rep, "copies", Value::Number(4.0));
    set(&mut rep, "composite", Value::Choice(1));
    set(&mut rep, "transform/endOpacity", Value::Number(20.0));
    for child in [group, star, path, grad, trim, rep] {
        push(&mut shapes.props, "contents", child);
    }
    keys(
        &mut shapes.props,
        "transform/position",
        vec![
            eased(0.0, Value::Vec3([20.0, 20.0, 0.0]), Ease::EASY, Ease::EASY),
            eased(2.0, Value::Vec3([150.0, 70.0, 0.0]), Ease { speed: 30.0, influence: 0.4 }, Ease::EASY),
            Keyframe::new(t(3.0), Value::Vec3([100.0, 20.0, 0.0])),
        ],
    );
    keys(&mut shapes.props, "transform/opacity", vec![Keyframe::new(t(0.0), Value::Number(100.0)).hold(), Keyframe::new(t(1.0), Value::Number(40.0)).hold(), Keyframe::new(t(2.0), Value::Number(90.0))]);

    // a null it moves with
    let mut null = Layer::new(p.alloc(), "Handle", Source::Null, t(4.0));
    set(&mut null.props, "transform/rotation", Value::Number(-10.0));
    shapes.parent = Some(null.id);

    // words, with an animator
    let mut words = Layer::new(p.alloc(), "Words", Source::Text, t(4.0));
    let doc = TextDoc { text: "Hello\nworld".into(), size: 40.0, tracking: 25.0, justify: veymelo_keyframe::Justify::Center, ..TextDoc::default() };
    set(&mut words.props, "text/sourceText", Value::Doc(Box::new(doc)));
    let mut anim = item(&mut p, &veymelo_project::text::ANIMATOR, "Fade In");
    set(&mut anim, "opacity", Value::Number(0.0));
    let mut range = item(&mut p, &veymelo_project::text::RANGE, "Range Selector 1");
    keys(&mut range, "start", vec![Keyframe::new(t(0.0), Value::Number(0.0)), Keyframe::new(t(2.0), Value::Number(100.0))]);
    set(&mut range, "basedOn", Value::Choice(2));
    push(&mut anim, "selectors", range);
    push(&mut words.props, "text/animators", anim);

    // a composition layer with a matte above it
    let mut matte = Layer::new(p.alloc(), "Matte", Source::Solid { color: Rgba::WHITE, width: 50, height: 50 }, t(4.0));
    matte.switches.video = false;
    let mut shown = Layer::new(p.alloc(), "Inner", Source::Comp(inner_id), t(4.0));
    shown.matte = Some(Matte { layer: matte.id, kind: MatteKind::Luma });
    shown.time_remap = true;
    keys(&mut shown.props, "timeRemap", vec![Keyframe::new(t(0.0), Value::Number(0.0)), Keyframe::new(t(4.0), Value::Number(2.0))]);
    (shown.start, shown.in_point, shown.out_point, shown.stretch) = (t(0.4), t(0.8), t(3.6), 200.0);

    // a solid with a mask and effects
    let mut masked = Layer::new(p.alloc(), "Masked", Source::Solid { color: Rgba::from_hex("#00ff88").unwrap(), width: 100, height: 100 }, t(4.0));
    masked.blend = veymelo_color::BlendMode::Screen;
    let mut mask = item(&mut p, &veymelo_project::MASK, "Hole");
    set(&mut mask, "path", Value::Path(ShapePath::polygon(&[[10.0, 10.0], [90.0, 10.0], [90.0, 90.0], [10.0, 90.0]], true)));
    set(&mut mask, "mode", Value::Choice(2));
    set(&mut mask, "inverted", Value::Bool(true));
    set(&mut mask, "opacity", Value::Number(50.0));
    push(&mut masked.props, "masks", mask);
    let mut blur = item(&mut p, veymelo_effects::find("gaussianBlur").unwrap().def, "Blur");
    keys(&mut blur, "blurriness", vec![Keyframe::new(t(0.0), Value::Number(0.0)), eased(1.0, Value::Number(12.0), Ease::EASY, Ease::EASY)]);
    set(&mut blur, "dimensions", Value::Choice(1));
    let mut slider = item(&mut p, veymelo_effects::find("sliderControl").unwrap().def, "Amount");
    set(&mut slider, "slider", Value::Number(3.0));
    let mut shadow = item(&mut p, veymelo_effects::find("dropShadow").unwrap().def, "Shadow");
    set(&mut shadow, "opacity", Value::Number(40.0));
    let mut chooser = item(&mut p, veymelo_effects::find("layerControl").unwrap().def, "Which");
    set(&mut chooser, "layer", Value::Layer(Some(null.id)));
    for fx in [blur, slider, shadow, chooser] {
        push(&mut masked.props, "effects", fx);
    }

    // a camera, and a light Lottie has not
    let mut camera = Layer::new(p.alloc(), "Camera", Source::Camera, t(4.0));
    set(&mut camera.props, "transform/position", Value::Vec3([100.0, 50.0, -400.0]));
    set(&mut camera.props, "cameraOptions/zoom", Value::Number(400.0));
    let light = Layer::new(p.alloc(), "Light", Source::Light(LightKind::Point), t(4.0));

    comp.layers = vec![camera, words, shapes, null, matte, shown, masked, light];
    let id = p.add_comp("Main", comp);
    (p, id)
}

fn out(p: &Project, comp: u64, options: &Options) -> Exported {
    export(p, comp, options, &|_| None).unwrap()
}

fn back(e: &Exported) -> (Project, Imported) {
    let mut p = Project::new();
    let text = e.json.to_json();
    let imported = import(&mut p, text.as_bytes(), "Back", None, &mut |_| None).unwrap();
    (p, imported)
}

/// Every property below `g`, with a path that finds it by place (`contents/#1/size`).
fn by_place<'a>(g: &'a Group, prefix: &str, out: &mut Vec<(String, &'a Property)>) {
    for (i, c) in g.children.iter().enumerate() {
        match c {
            Node::Prop(p) => out.push((format!("{prefix}{}", p.def.id), p)),
            Node::Group(sub) => {
                let seg = if g.is_list() { format!("#{}", i + 1) } else { sub.def.id.to_string() };
                by_place(sub, &format!("{prefix}{seg}/"), out);
            }
        }
    }
}

fn close(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Path(x), Value::Path(y)) => {
            let near = |p: &[[f64; 2]], q: &[[f64; 2]]| p.len() == q.len() && p.iter().zip(q).all(|(u, v)| (u[0] - v[0]).abs() < 1e-4 && (u[1] - v[1]).abs() < 1e-4);
            x.closed == y.closed && near(&x.points, &y.points) && near(&x.in_tangents, &y.in_tangents) && near(&x.out_tangents, &y.out_tangents)
        }
        (Value::Gradient(x), Value::Gradient(y)) => (0..=10).all(|i| x.at(i as f64 / 10.0).iter().zip(y.at(i as f64 / 10.0)).all(|(u, v)| (u - v).abs() < 1e-6)),
        _ if a.dims() > 0 && a.dims() == b.dims() => (0..a.dims()).all(|d| (a.component(d) - b.component(d)).abs() < 1e-4 * (1.0 + a.component(d).abs())),
        _ => a == b,
    }
}

#[test]
fn eases_as_lottie_curves() {
    let mut cx = values::Ctx { rate: FrameRate::new(25, 1).unwrap(), expressions: false, warnings: Vec::new() };
    let mut g = Group::new(&veymelo_project::TRANSFORM);
    // easy ease: stopped at both keys, a third of the way each
    keys(&mut g, "opacity", vec![eased(0.0, Value::Number(0.0), Ease::EASY, Ease::EASY), eased(1.0, Value::Number(100.0), Ease::EASY, Ease::EASY)]);
    let j = values::prop_json(&mut cx, g.prop("opacity").unwrap(), &values::same, "o");
    let k0 = &j.get("k").unwrap().as_array().unwrap()[0];
    assert_eq!(k0.get("o").unwrap().to_json(), r#"{"x":[0.333333],"y":[0]}"#);
    assert_eq!(k0.get("i").unwrap().to_json(), r#"{"x":[0.666667],"y":[1]}"#);
    assert_eq!(j.get("k").unwrap().as_array().unwrap()[1].to_json(), r#"{"t":25,"s":[100]}"#, "the last key has its time and value");
    // leaving at 50 a second for half the segment, of 100 over 2 seconds: y = 50 · 0.5 · 2 / 100
    keys(&mut g, "opacity", vec![eased(0.0, Value::Number(0.0), Ease { speed: 50.0, influence: 0.5 }, Ease::EASY), eased(2.0, Value::Number(100.0), Ease::EASY, Ease { speed: 25.0, influence: 0.2 })]);
    let j = values::prop_json(&mut cx, g.prop("opacity").unwrap(), &values::same, "o");
    let k0 = &j.get("k").unwrap().as_array().unwrap()[0];
    assert_eq!(k0.get("o").unwrap().to_json(), r#"{"x":[0.5],"y":[0.5]}"#);
    assert_eq!(k0.get("i").unwrap().to_json(), r#"{"x":[0.8],"y":[0.9]}"#, "1 − 25 · 0.2 · 2 / 100");
    // linear is the straight line; a hold says h
    keys(&mut g, "opacity", vec![Keyframe::new(t(0.0), Value::Number(0.0)), Keyframe::new(t(1.0), Value::Number(10.0)).hold(), Keyframe::new(t(2.0), Value::Number(0.0))]);
    let j = values::prop_json(&mut cx, g.prop("opacity").unwrap(), &values::same, "o");
    let ks = j.get("k").unwrap().as_array().unwrap();
    assert_eq!(ks[0].get("o").unwrap().to_json(), r#"{"x":[0.333333],"y":[0.333333]}"#);
    assert_eq!(ks[1].to_json(), r#"{"t":25,"s":[10],"h":1}"#);
    // a position's curve: tangents beside each key, its ease along the path's length
    keys(&mut g, "position", vec![Keyframe::new(t(0.0), Value::Vec3([0.0, 0.0, 0.0])), Keyframe::new(t(1.0), Value::Vec3([30.0, 40.0, 0.0])), Keyframe::new(t(2.0), Value::Vec3([60.0, 0.0, 0.0]))]);
    let j = values::prop_json(&mut cx, g.prop("position").unwrap(), &values::same, "p");
    let k1 = &j.get("k").unwrap().as_array().unwrap()[1];
    assert_eq!(k1.get("to").unwrap().to_json(), "[10,0,0]", "a sixth of the way from the key before to the key after");
    assert_eq!(k1.get("o").unwrap().get("x").unwrap().to_json(), "[0.333333]", "one curve for the point");
}

#[test]
fn a_composition_goes_out_and_comes_back() {
    let (p, id) = project();
    let e = out(&p, id, &Options::default());
    let j = &e.json;
    assert_eq!((j.get("fr").unwrap().as_f64(), j.get("op").unwrap().as_f64(), j.get("w").unwrap().as_f64()), (Some(25.0), Some(100.0), Some(200.0)));
    let layers = j.get("layers").unwrap().as_array().unwrap();
    let types: Vec<f64> = layers.iter().map(|l| l.get("ty").unwrap().as_f64().unwrap()).collect();
    assert_eq!(types, [13.0, 5.0, 4.0, 3.0, 1.0, 0.0, 1.0, 3.0], "camera, text, shape, null, solid, precomp, solid, the light as a null");
    assert_eq!(layers[2].get("parent").unwrap().as_f64(), Some(4.0), "parents by their ind");
    assert_eq!((layers[5].get("tt").unwrap().as_f64(), layers[5].get("tp").unwrap().as_f64(), layers[4].get("td").unwrap().as_f64()), (Some(3.0), Some(5.0), Some(1.0)));
    assert_eq!((layers[5].get("sr").unwrap().as_f64(), layers[5].get("st").unwrap().as_f64(), layers[5].get("ip").unwrap().as_f64()), (Some(2.0), Some(10.0), Some(20.0)));
    assert_eq!(layers[7].get("hd").unwrap().as_bool(), Some(true));
    assert_eq!(layers[6].get("bm").unwrap().as_f64(), Some(2.0), "screen");
    assert_eq!(j.get("assets").unwrap().as_array().unwrap().len(), 1, "the inner composition");
    let fx = layers[6].get("ef").unwrap().as_array().unwrap();
    assert_eq!(fx.iter().map(|f| f.get("ty").unwrap().as_f64().unwrap()).collect::<Vec<_>>(), [29.0, 5.0, 25.0, 5.0]);
    assert_eq!(fx[2].get("ef").unwrap().as_array().unwrap()[1].get("v").unwrap().get("k").unwrap().as_f64(), Some(102.0), "drop shadow opacity out of 255");
    assert_eq!(fx[3].get("ef").unwrap().as_array().unwrap()[0].get("v").unwrap().get("k").unwrap().as_f64(), Some(4.0), "a layer by its ind");
    assert!(e.warnings.iter().any(|w| w.starts_with("Light: Lottie has no lights")), "{:?}", e.warnings);

    let (q, imported) = back(&e);
    let c = q.comp(imported.comp).unwrap();
    let a = p.comp(id).unwrap();
    assert_eq!((c.width, c.height, c.frame_rate, c.duration), (a.width, a.height, a.frame_rate, a.duration));
    assert_eq!(c.markers[0].comment, "beat");
    assert_eq!(c.layers.len(), a.layers.len());
    let times = [0.0, 0.37, 0.8, 1.0, 1.52, 2.0, 2.44, 2.9, 3.4];
    let mut compared = 0;
    for (la, lb) in a.layers.iter().zip(&c.layers) {
        assert_eq!(la.name, lb.name);
        if matches!(la.source, Source::Light(_)) {
            assert_eq!(lb.source, Source::Null);
            continue;
        }
        assert_eq!(la.source.id(), lb.source.id(), "{}", la.name);
        assert_eq!((la.start, la.in_point, la.out_point, la.stretch, la.blend), (lb.start, lb.in_point, lb.out_point, lb.stretch, lb.blend), "{}", la.name);
        assert_eq!(la.parent.and_then(|p| a.index_of(p)), lb.parent.and_then(|p| c.index_of(p)), "{} parent", la.name);
        assert_eq!(la.matte.map(|m| (a.index_of(m.layer), m.kind)), lb.matte.map(|m| (c.index_of(m.layer), m.kind)));
        let (mut pa, mut pb) = (Vec::new(), Vec::new());
        by_place(&la.props, "", &mut pa);
        by_place(&lb.props, "", &mut pb);
        for (path, prop) in &pa {
            if path.starts_with("materialOptions") || path.contains("/@") {
                continue;
            }
            let Some((_, other)) = pb.iter().find(|(q, _)| q == path) else { panic!("{}: {path} did not come back", la.name) };
            for s in times {
                let (u, v) = (prop.value_at(t(s)), other.value_at(t(s)));
                let (u, v) = match (&u, &v) {
                    (Value::Layer(x), Value::Layer(y)) => (Value::Number(x.and_then(|x| a.index_of(x)).map_or(-1.0, |i| i as f64)), Value::Number(y.and_then(|y| c.index_of(y)).map_or(-1.0, |i| i as f64))),
                    _ => (u, v),
                };
                assert!(close(&u, &v), "{} {path} at {s} s: {u:?} went out, {v:?} came back", la.name);
            }
            compared += 1;
        }
    }
    assert!(compared > 150, "{compared} properties compared");
    let inner = q.comp(match c.layers[5].source {
        Source::Comp(i) => i,
        _ => unreachable!(),
    });
    assert_eq!(inner.map(|c| (c.width, c.layers.len())), Some((80, 1)));
    // and it reads as a project again
    let text = q.to_text();
    assert!(Project::from_text(&text, &{
        let mut cat = Catalog::base();
        veymelo_effects::register(&mut cat);
        cat
    })
    .is_ok());
}

#[test]
fn what_lottie_cannot_hold_is_said() {
    let (mut p, id) = project();
    let c = p.comp_mut(id).unwrap();
    let l = &mut c.layers[2];
    let mut wig = Group::instance(&defs::WIGGLE, 900, "Wiggle");
    set(&mut wig, "size", Value::Number(3.0));
    push(&mut l.props, "contents", wig);
    l.switches.motion_blur = true;
    let style = Group::instance(veymelo_project::styles::find("dropShadow").unwrap(), 901, "Drop Shadow");
    push(&mut l.props, veymelo_project::styles::LIST, style);
    let mut glow = Group::instance(veymelo_effects::find("glow").unwrap().def, 902, "Glow");
    glow.enabled = true;
    push(&mut c.layers[6].props, "effects", glow);
    c.layers[6].switches.adjustment = true;
    let e = out(&p, id, &Options::default());
    for said in ["Shapes › Wiggle: Lottie has no wiggle", "Shapes: its layer styles go along", "Shapes: Lottie has no motion blur", "Masked › Glow: Lottie players do not draw Glow", "Masked: Lottie has no adjustment layers"] {
        assert!(e.warnings.iter().any(|w| w.starts_with(said)), "{said} in {:#?}", e.warnings);
    }
}

#[test]
fn a_file_as_other_tools_write_it() {
    // keys with `e` (the old way of saying where a segment ends), a separate position, a
    // group's transform last, colours 0 to 1, a track matte without `tp`
    let lottie = r##"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":300,"h":200,"nm":"Old","ddd":0,"assets":[],
      "layers":[
        {"ddd":0,"ind":1,"ty":4,"nm":"Matte","td":1,"ks":{"o":{"a":0,"k":100},"p":{"a":0,"k":[150,100,0]},"a":{"a":0,"k":[0,0,0]},"s":{"a":0,"k":[100,100,100]},"r":{"a":0,"k":0}},
         "shapes":[{"ty":"el","d":1,"s":{"a":0,"k":[120,120]},"p":{"a":0,"k":[0,0]},"nm":"Ellipse Path 1"},{"ty":"fl","c":{"a":0,"k":[1,1,1,1]},"o":{"a":0,"k":100},"r":1,"nm":"Fill 1"}],
         "ip":0,"op":60,"st":0,"bm":0},
        {"ddd":0,"ind":2,"ty":4,"nm":"Dot","tt":1,"ks":{"o":{"a":0,"k":100},"r":{"a":1,"k":[{"i":{"x":[0.833],"y":[0.833]},"o":{"x":[0.167],"y":[0.167]},"t":0,"s":[0],"e":[90]},{"t":30}]},
           "p":{"s":true,"x":{"a":1,"k":[{"i":{"x":[0.58],"y":[1]},"o":{"x":[0.42],"y":[0]},"t":0,"s":[50]},{"t":60,"s":[250]}]},"y":{"a":0,"k":100}},
           "a":{"a":0,"k":[0,0,0]},"s":{"a":0,"k":[100,100,100]}},
         "shapes":[{"ty":"gr","it":[{"ty":"rc","d":1,"s":{"a":0,"k":[40,40]},"p":{"a":0,"k":[0,0]},"r":{"a":0,"k":6}},
             {"ty":"st","c":{"a":0,"k":[0,0.5,1,1]},"o":{"a":0,"k":100},"w":{"a":0,"k":4},"lc":2,"lj":2,"ml":4},
             {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":45},"o":{"a":0,"k":100},"sk":{"a":0,"k":0},"sa":{"a":0,"k":0}}],"nm":"Box"}],
         "ip":0,"op":60,"st":0,"bm":0}],
      "markers":[]}"##;
    let mut p = Project::new();
    let im = import(&mut p, lottie.as_bytes(), "x", None, &mut |_| None).unwrap();
    assert!(im.warnings.is_empty(), "{:?}", im.warnings);
    let c = p.comp(im.comp).unwrap();
    assert_eq!((c.width, c.frame_rate.as_f64(), c.duration), (300, 30.0, t(2.0)));
    let (matte, dot) = (&c.layers[0], &c.layers[1]);
    assert!(!matte.switches.video, "a matte layer is not seen itself");
    assert_eq!(dot.matte, Some(Matte { layer: matte.id, kind: MatteKind::Alpha }), "the layer above, without tp");
    // the old `e`: 0 to 90 over a second, linear (its curve is the straight line)
    let r = dot.props.prop("transform/rotation").unwrap();
    assert!((r.value_at(t(0.5)).as_f64() - 45.0).abs() < 1e-9);
    assert!(dot.separate_position);
    let x = dot.props.prop("transform/positionX").unwrap();
    assert!((x.value_at(t(1.0)).as_f64() - 150.0).abs() < 1e-6, "ease in and out meet in the middle");
    assert!(x.value_at(t(0.25)).as_f64() < 50.0 + 200.0 * 0.125, "slow at first");
    let rect = dot.props.prop("contents/#1/contents/#1/roundness").unwrap();
    assert_eq!(rect.value, Value::Number(6.0));
    let stroke = dot.props.group("contents/#1/contents/#2").unwrap();
    assert_eq!((stroke.prop("cap").unwrap().value.as_choice(), stroke.prop("join").unwrap().value.as_choice()), (1, 1));
    assert_eq!(stroke.prop("color").unwrap().value, Value::Color(veymelo_color::rgba(0.0, 0.5, 1.0, 1.0)));
    assert_eq!(dot.props.prop("contents/#1/transform/rotation").unwrap().value, Value::Number(45.0));
}

#[test]
fn a_dotlottie_archive_both_ways() {
    let (p, id) = project();
    let e = out(&p, id, &Options::default());
    let packed = dotlottie(&e, "main clip", &[]);
    let files = veymelo_zip::unzip(&packed).unwrap();
    assert_eq!(files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["manifest.json", "animations/main_clip.json"]);
    let (json, _) = document(&packed).unwrap();
    assert_eq!(json, e.json);
    let mut q = Project::new();
    assert!(import(&mut q, &packed, "Packed", None, &mut |_| None).is_ok());
    assert!(document(b"PK\x03\x04 not really").is_err());
    assert!(document(b"{\"no\": ").is_err());
}

#[test]
fn pictures_inside_or_beside() {
    let mut p = Project::new();
    let mut f = Footage::new("/pictures/cat.png", FootageKind::Image);
    (f.width, f.height) = (64, 48);
    let pic = p.add("Cat", None, ItemKind::Footage(f));
    let mut comp = Comp::new(100, 100, FrameRate::new(30, 1).unwrap(), t(1.0));
    comp.layers.push(Layer::new(p.alloc(), "Cat", Source::Footage(pic), t(1.0)));
    let id = p.add_comp("Main", comp);
    let bytes = b"\x89PNG pretend".to_vec();
    let inside = export(&p, id, &Options::default(), &|path| (path == "/pictures/cat.png").then(|| bytes.clone())).unwrap();
    let asset = &inside.json.get("assets").unwrap().as_array().unwrap()[0];
    assert!(asset.get("p").unwrap().as_str().unwrap().starts_with("data:image/png;base64,"));
    let mut kept = Vec::new();
    let mut q = Project::new();
    let im = import(&mut q, inside.json.to_json().as_bytes(), "Back", None, &mut |pic| {
        kept.push(pic.bytes.clone());
        Some(format!("/kept/{}.{}", pic.id, pic.ext))
    })
    .unwrap();
    assert_eq!(kept, vec![bytes.clone()]);
    let layer = &q.comp(im.comp).unwrap().layers[0];
    let Source::Footage(item) = layer.source else { panic!("a picture layer") };
    let Some(ItemKind::Footage(f)) = q.item(item).map(|i| &i.kind) else { panic!() };
    assert_eq!((f.path.as_str(), f.width, f.height), ("/kept/image_1.png", 64, 48));
    // beside it: named in the document, and packed into an archive's images
    let beside = export(&p, id, &Options { embed_images: false, ..Options::default() }, &|_| None).unwrap();
    assert_eq!(beside.images, vec![("img_1.png".to_string(), "/pictures/cat.png".to_string())]);
    let packed = dotlottie(&beside, "a", &[("img_1.png".into(), bytes.clone())]);
    let mut kept = Vec::new();
    let mut r = Project::new();
    import(&mut r, &packed, "Packed", None, &mut |pic| {
        kept.push(pic.bytes.clone());
        Some("/kept/x.png".into())
    })
    .unwrap();
    assert_eq!(kept, vec![bytes], "found in the archive's images");
}

#[test]
fn text_as_letters() {
    let mut p = Project::new();
    let mut comp = Comp::new(400, 200, FrameRate::new(30, 1).unwrap(), t(1.0));
    let mut l = Layer::new(p.alloc(), "Title", Source::Text, t(1.0));
    let doc = TextDoc { text: "Ho".into(), font: "Helvetica".into(), size: 100.0, stroke: Some(Rgba::BLACK), stroke_width: 2.0, ..TextDoc::default() };
    set(&mut l.props, "text/sourceText", Value::Doc(Box::new(doc)));
    comp.layers.push(l);
    let id = p.add_comp("Main", comp);
    let as_text = out(&p, id, &Options::default());
    let fonts = as_text.json.get("fonts").unwrap().get("list").unwrap().as_array().unwrap();
    assert_eq!(fonts[0].get("fName").unwrap().as_str(), Some("Helvetica-Regular"));
    let e = out(&p, id, &Options { text_as_shapes: true, ..Options::default() });
    let layer = &e.json.get("layers").unwrap().as_array().unwrap()[0];
    assert_eq!(layer.get("ty").unwrap().as_f64(), Some(4.0));
    let items = layer.get("shapes").unwrap().as_array().unwrap();
    if veymelo_text::resolve("Helvetica", 400.0, false).is_none() {
        return;
    }
    let names: Vec<&str> = items.iter().filter_map(|i| i.get("nm")?.as_str()).collect();
    assert_eq!(names, ["H", "o", "Fill", "Stroke"]);
    // the letters' curves are curves: some point has a tangent
    let curvy = items[1].get("it").unwrap().as_array().unwrap().iter().filter_map(|s| s.get("ks")).any(|k| k.get("k").unwrap().get("o").unwrap().as_array().unwrap().iter().any(|o| o.to_json() != "[0,0]"));
    assert!(curvy, "an o is round");
    let h = items[0].get("it").unwrap().as_array().unwrap()[0].get("ks").unwrap().get("k").unwrap();
    assert!(h.get("v").unwrap().as_array().unwrap().iter().all(|v| v.at(1).unwrap().as_f64().unwrap() <= 0.5), "letters sit on the baseline, above it");
}

/// Lottie files made elsewhere, named one a line in the file `VEYMELO_LOTTIE_SAMPLES`: each read,
/// written out again and read back.
#[test]
#[ignore = "needs Lottie files of your own: VEYMELO_LOTTIE_SAMPLES=list.txt cargo test -p veymelo-lottie -- --ignored"]
fn files_made_elsewhere() {
    let Ok(list) = std::env::var("VEYMELO_LOTTIE_SAMPLES") else { return };
    for path in std::fs::read_to_string(list).unwrap().lines().filter(|l| !l.trim().is_empty()) {
        let bytes = std::fs::read(path).unwrap();
        let mut p = Project::new();
        let im = import(&mut p, &bytes, "Sample", std::path::Path::new(path).parent(), &mut |pic| Some(format!("/tmp/{}.{}", pic.id, pic.ext))).unwrap_or_else(|e| panic!("{path}: {e}"));
        let c = p.comp(im.comp).unwrap();
        let e = export(&p, im.comp, &Options::default(), &|_| None).unwrap();
        let (q, again) = back(&e);
        let d = q.comp(again.comp).unwrap();
        assert_eq!(c.layers.len(), d.layers.len(), "{path}");
        println!("{path}\n  {} × {}, {} layers, {} s; in: {:?}\n  out: {:?}", c.width, c.height, c.layers.len(), c.duration.seconds(), im.warnings, e.warnings);
    }
}

#[test]
fn sound_layers_go_out_and_come_back() {
    let mut p = Project::new();
    let mut f = Footage::new("/sounds/beat.wav", FootageKind::Audio);
    (f.has_audio, f.duration) = (true, t(2.0));
    let item = p.add("beat.wav", None, ItemKind::Footage(f));
    let mut c = Comp::new(100, 100, FrameRate::FPS_30, t(2.0));
    let mut l = Layer::new(p.alloc(), "Beat", Source::Footage(item), t(2.0));
    l.props.prop_mut("audio/levels").unwrap().set_value(Value::Vec2([-6.0, -6.0])).unwrap();
    c.layers.push(l);
    let id = p.add_comp("Main", c);
    let bytes = b"RIFF....WAVEfmt ".to_vec();
    let e = export(&p, id, &Options::default(), &|path| (path == "/sounds/beat.wav").then(|| bytes.clone())).unwrap();
    let layer = &e.json.get("layers").unwrap().as_array().unwrap()[0];
    assert_eq!(layer.get("ty").and_then(veymelo_json::Value::as_f64), Some(6.0));
    let lv = layer.get("au").and_then(|a| a.get("lv")).and_then(|l| l.get("k")).unwrap().to_json();
    assert_eq!(lv, "[50.119,50.119]", "-6 dB as a %");
    let asset = &e.json.get("assets").unwrap().as_array().unwrap()[0];
    assert!(asset.get("p").and_then(veymelo_json::Value::as_str).unwrap().starts_with("data:audio/wav;base64,"));
    assert!(e.warnings.is_empty(), "{:?}", e.warnings);
    // and back: the sound kept as a file, its layer as loud as it was
    let mut q = Project::new();
    let mut kept = Vec::new();
    let text = e.json.to_json();
    let imported = import(&mut q, text.as_bytes(), "Back", None, &mut |s: &Picture| {
        kept.push((s.ext.clone(), s.bytes.clone()));
        Some(format!("/kept/{}.{}", s.name, s.ext))
    })
    .unwrap();
    assert_eq!(kept, vec![("wav".to_string(), bytes)]);
    let comp = q.comp(imported.comp).unwrap();
    let Source::Footage(back) = comp.layers[0].source else { panic!("a footage layer") };
    let ItemKind::Footage(f) = &q.item(back).unwrap().kind else { panic!("footage") };
    assert_eq!(f.kind, FootageKind::Audio);
    let levels = comp.layers[0].props.prop("audio/levels").unwrap().value.as_vec2();
    assert!((levels[0] + 6.0).abs() < 0.01, "{levels:?}");
}

#[test]
fn layer_styles_go_out_and_come_back() {
    use veymelo_project::styles::{self, BLENDING_OPTIONS, LIST};
    let (mut p, id) = project();
    let c = p.comp_mut(id).unwrap();
    let l = &mut c.layers[2];
    let mut bo = Group::instance(&BLENDING_OPTIONS, 950, "Blending Options");
    set(&mut bo, "globalLightAngle", Value::Number(45.0));
    set(&mut bo, "globalLightAltitude", Value::Number(40.0));
    push(&mut l.props, LIST, bo);
    let mut shadow = Group::instance(styles::find("dropShadow").unwrap(), 951, "Drop Shadow");
    set(&mut shadow, "useGlobalLight", Value::Bool(false));
    set(&mut shadow, "angle", Value::Number(200.0));
    set(&mut shadow, "distance", Value::Number(12.0));
    set(&mut shadow, "color", Value::Color(Rgba::from_hex("#102030").unwrap()));
    set(&mut shadow, "blendMode", Value::Choice(8));
    keys(&mut shadow, "opacity", vec![eased(0.0, Value::Number(20.0), Ease::EASY, Ease::EASY), eased(2.0, Value::Number(90.0), Ease::EASY, Ease::EASY)]);
    push(&mut l.props, LIST, shadow);
    let mut bevel = Group::instance(styles::find("bevelEmboss").unwrap(), 952, "Bevel and Emboss");
    set(&mut bevel, "style", Value::Choice(3));
    set(&mut bevel, "size", Value::Number(9.0));
    push(&mut l.props, LIST, bevel);
    let mut grad = Group::instance(styles::find("gradientOverlay").unwrap(), 953, "Gradient Overlay");
    set(&mut grad, "colors", Value::Gradient(veymelo_keyframe::Gradient { colors: vec![(0.0, [1.0, 0.0, 0.0]), (1.0, [0.0, 0.0, 1.0])], opacities: vec![(0.0, 1.0), (1.0, 0.5)] }));
    set(&mut grad, "style", Value::Choice(1));
    set(&mut grad, "offset", Value::Vec2([3.0, -4.0]));
    push(&mut l.props, LIST, grad);
    let mut off = Group::instance(styles::find("stroke").unwrap(), 954, "Stroke");
    off.enabled = false;
    push(&mut l.props, LIST, off);
    let e = out(&p, id, &Options::default());
    let text = e.json.to_json();
    assert!(text.contains(r#""sy":"#), "{text}");
    assert!(e.warnings.iter().any(|w| w.contains("Stroke: switched off")), "{:#?}", e.warnings);
    let (back_p, imported) = back(&e);
    let comp = back_p.comp(imported.comp).unwrap();
    let l = comp.layers.iter().find(|l| l.name == "Shapes").unwrap();
    let list = l.props.group(LIST).unwrap();
    let ids: Vec<&str> = list.children.iter().filter_map(Node::as_group).map(|g| g.def.id).collect();
    assert_eq!(ids, ["blendingOptions", "dropShadow", "bevelEmboss", "gradientOverlay"]);
    let style = |id: &str| list.children.iter().filter_map(Node::as_group).find(|g| g.def.id == id).unwrap();
    let ds = style("dropShadow");
    assert_eq!(ds.prop("angle").unwrap().value, Value::Number(200.0));
    assert_eq!(ds.prop("distance").unwrap().value, Value::Number(12.0));
    assert_eq!(ds.prop("blendMode").unwrap().value, Value::Choice(8));
    assert!(close(&ds.prop("color").unwrap().value, &Value::Color(Rgba::from_hex("#102030").unwrap())));
    assert_eq!(ds.prop("opacity").unwrap().keys.len(), 2);
    // the bevel followed the light: it comes back following it, the light as it was
    let bv = style("bevelEmboss");
    assert_eq!((bv.prop("style").unwrap().value.clone(), bv.prop("useGlobalLight").unwrap().value.clone()), (Value::Choice(3), Value::Bool(true)));
    let bo = style("blendingOptions");
    assert_eq!((bo.prop("globalLightAngle").unwrap().value.clone(), bo.prop("globalLightAltitude").unwrap().value.clone()), (Value::Number(45.0), Value::Number(40.0)));
    let go = style("gradientOverlay");
    assert_eq!(go.prop("offset").unwrap().value, Value::Vec2([3.0, -4.0]));
    assert!(close(&go.prop("colors").unwrap().value, &Value::Gradient(veymelo_keyframe::Gradient { colors: vec![(0.0, [1.0, 0.0, 0.0]), (1.0, [0.0, 0.0, 1.0])], opacities: vec![(0.0, 1.0), (1.0, 0.5)] })));
}
