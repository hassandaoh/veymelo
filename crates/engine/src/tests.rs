//! The session driven the way an agent drives it: commands by id with JSON parameters.

use super::*;
use veymelo_keyframe::Value;

fn run(s: &mut Session, id: &str, params: &str) -> Json {
    let p = if params.is_empty() { Json::Null } else { Json::parse(params).unwrap_or_else(|e| panic!("{params}: {e}")) };
    s.run(id, &p).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn fails(s: &mut Session, id: &str, params: &str) -> String {
    let p = if params.is_empty() { Json::Null } else { Json::parse(params).unwrap() };
    match s.run(id, &p) {
        Ok(v) => panic!("{id} {params} should fail, gave {}", v.to_json()),
        Err(e) => e.message,
    }
}

fn id(j: &Json, key: &str) -> u64 {
    j.get(key).and_then(Json::as_u64).unwrap_or_else(|| panic!("no {key} in {}", j.to_json()))
}

/// A session with a vertical composition holding one red solid.
fn started() -> (Session, u64, u64) {
    let mut s = Session::new();
    let c = run(&mut s, "comp.new", r#"{"name": "Main", "width": 1080, "height": 1920, "fps": 30, "duration": 5}"#);
    let l = run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ff0000", "width": 400, "height": 200}"##);
    (s, id(&c, "comp"), id(&l, "layer"))
}

#[test]
fn a_composition_and_a_layer() {
    let (s, c, l) = started();
    assert_eq!(s.view.comp, Some(c));
    let layer = s.project().layer(l).unwrap();
    assert_eq!(layer.name, "Solid 1");
    assert_eq!(layer.out_point, Tick::from_seconds(5.0));
    assert_eq!(layer.props.prop("transform/position").unwrap().value, Value::Vec3([540.0, 960.0, 0.0]), "centred in the composition");
    assert_eq!(layer.props.prop("transform/anchor").unwrap().value, Value::Vec3([200.0, 100.0, 0.0]), "turning about its own centre");
    assert_eq!(s.history().list().len(), 3);
    assert!(s.modified());
}

#[test]
fn properties_take_values_and_keys() {
    let (mut s, _, _) = started();
    let out = run(&mut s, "prop.set", r#"{"layer": "Solid 1", "path": "transform/opacity", "value": 50}"#);
    assert_eq!(out.get("value"), Some(&Json::Number(50.0)));
    assert_eq!(run(&mut s, "prop.get", r##"{"layer": "#1", "path": "transform/opacity"}"##).get("value"), Some(&Json::Number(50.0)));
    assert_eq!(
        fails(&mut s, "prop.set", r#"{"layer": "Solid 1", "path": "transform/opacity", "value": 500}"#),
        "opacity goes from 0 to 100%, not 500"
    );
    assert_eq!(
        fails(&mut s, "prop.set", r#"{"layer": "Solid 1", "path": "transform/positon", "value": [0, 0]}"#),
        "transform has no \"positon\" (it has anchor, position, positionX, positionY, positionZ, scale, orientation, rotationX, rotationY, rotation, opacity)"
    );
    // keys, one at a time
    run(&mut s, "prop.set", r#"{"layer": "Solid 1", "path": "transform/position", "value": [0, 0], "time": 0}"#);
    let out = run(&mut s, "prop.set", r#"{"layer": "Solid 1", "path": "transform/position", "value": [100, 200], "time": 2}"#);
    assert_eq!(out.get("keys").unwrap().to_json(), r#"[{"t":0,"v":[0,0,0]},{"t":2,"v":[100,200,0]}]"#);
    let mid = run(&mut s, "prop.get", r#"{"layer": "Solid 1", "path": "transform/position", "time": 1}"#);
    assert_eq!(mid.get("value").unwrap().to_json(), "[50,100,0]");
    assert!(fails(&mut s, "prop.set", r#"{"layer": "Solid 1", "path": "transform/position", "value": [1, 1]}"#).starts_with("transform/position has keys; give `time`"));
    // all keys at once, eased
    run(&mut s, "prop.keys", r#"{"layer": "Solid 1", "path": "transform/rotation", "keys": [{"t": 0, "v": 0, "out": "bezier"}, {"t": 1, "v": 90, "in": "bezier"}]}"#);
    let r = run(&mut s, "prop.get", r#"{"layer": "Solid 1", "path": "transform/rotation", "time": 0.5}"#);
    assert!((r.get("value").unwrap().as_f64().unwrap() - 45.0).abs() < 1e-9);
    let r = run(&mut s, "prop.get", r#"{"layer": "Solid 1", "path": "transform/rotation", "time": 0.1}"#);
    assert!(r.get("value").unwrap().as_f64().unwrap() < 9.0 * 0.5, "starts slowly");
    // no keys: still at the value it had at the view's time
    run(&mut s, "view.set", r#"{"time": 1}"#);
    run(&mut s, "prop.keys", r#"{"layer": "Solid 1", "path": "transform/rotation", "keys": []}"#);
    assert_eq!(s.project().layer(2).unwrap().props.prop("transform/rotation").unwrap().value, Value::Number(90.0));
    let changed = run(&mut s, "prop.list", r#"{"layer": "Solid 1", "changed": true}"#);
    let paths: Vec<&str> = changed.as_array().unwrap().iter().map(|p| p.get("path").unwrap().as_str().unwrap()).collect();
    assert_eq!(paths, ["transform/anchor", "transform/position", "transform/rotation", "transform/opacity"]);
    let o = run(&mut s, "prop.set", r#"{"layer": "Solid 1", "path": "transform/orientation", "value": [0, 45, 0]}"#);
    assert!(o.get("note").unwrap().as_str().unwrap().contains("only acts on 3D layers"));
}

#[test]
fn undo_redo_and_branches() {
    let (mut s, _, l) = started();
    let opacity = |s: &Session| s.project().layer(l).unwrap().props.prop("transform/opacity").unwrap().value.as_f64();
    run(&mut s, "prop.set", r#"{"layer": 2, "path": "transform/opacity", "value": 10}"#);
    run(&mut s, "prop.set", r#"{"layer": 2, "path": "transform/opacity", "value": 20}"#);
    let twenty = s.history().current();
    assert_eq!(run(&mut s, "edit.undo", "").get("undid").unwrap().as_str(), Some("Set transform/opacity of Solid 1"));
    assert_eq!(opacity(&s), 10.0);
    run(&mut s, "prop.set", r#"{"layer": 2, "path": "transform/opacity", "value": 30}"#);
    assert_eq!(opacity(&s), 30.0);
    // the 20 is still there, on a branch
    run(&mut s, "history.goto", &format!(r#"{{"id": {twenty}}}"#));
    assert_eq!(opacity(&s), 20.0);
    while s.history().can_undo() {
        run(&mut s, "edit.undo", "");
    }
    assert!(s.project().items.is_empty());
    assert_eq!(s.view.comp, None, "the composition went with the undo");
    assert_eq!(fails(&mut s, "edit.undo", ""), "there is nothing to undo");
    // setting a value to what it is makes no step
    while s.history().can_redo() {
        run(&mut s, "edit.redo", "");
    }
    let steps = s.history().len();
    run(&mut s, "prop.set", r#"{"layer": 2, "path": "transform/opacity", "value": 20}"#);
    assert_eq!(s.history().len(), steps);
    let list = run(&mut s, "history.list", "");
    assert!(list.as_array().unwrap().iter().any(|e| e.get("current") == Some(&Json::Bool(true))));
}

#[test]
fn batches_are_one_step_and_all_or_nothing() {
    let (mut s, _, _) = started();
    let before = s.history().len();
    let out = run(
        &mut s,
        "batch",
        r##"{"label": "Build the title", "steps": [
            {"command": "layer.add", "params": {"type": "null", "name": "Rig"}},
            {"command": "layer.add", "params": {"type": "solid", "color": "#00ff00", "name": "Card"}},
            {"command": "layer.set", "params": {"layer": "$2.layer", "parent": "$1.layer"}},
            {"command": "prop.get", "params": {"layer": "$2.layer", "path": "transform/opacity"}}
        ]}"##,
    );
    let results = out.get("results").unwrap().as_array().unwrap();
    assert_eq!(results.len(), 4);
    let (rig, card) = (id(&results[0], "layer"), id(&results[1], "layer"));
    assert_eq!(s.project().layer(card).unwrap().parent, Some(rig));
    assert_eq!(s.history().len(), before + 1);
    assert_eq!(s.history().label(), "Build the title");
    // a step that fails takes the others back with it
    let layers = s.project().comp(s.view.comp.unwrap()).unwrap().layers.len();
    let e = fails(&mut s, "batch", r#"{"steps": [{"command": "layer.add", "params": {"type": "null"}}, {"command": "layer.set", "params": {"layer": "$1.layer", "blend": "glow"}}]}"#);
    assert!(e.starts_with("step 2 (layer.set): layer.set: `blend` is one of \"normal\""), "{e}");
    assert!(e.ends_with("; nothing was changed"), "{e}");
    assert_eq!(s.project().comp(s.view.comp.unwrap()).unwrap().layers.len(), layers);
    assert_eq!(fails(&mut s, "batch", r#"{"steps": [{"command": "layer.set", "params": {"layer": "$4.layer"}}]}"#), "step 1 (layer.set): \"$4.layer\": there is no step 4 before this one; nothing was changed");
    assert_eq!(fails(&mut s, "batch", r#"{"steps": [{"command": "edit.undo"}]}"#), "step 1 (edit.undo): edit.undo cannot run inside a batch; nothing was changed");
    assert_eq!(fails(&mut s, "batch", r#"{"steps": [{"command": "layer.add", "parms": {}}]}"#), "step 1 (layer.add): a step has no `parms` (it has command, params); nothing was changed");
}

#[test]
fn mistakes_name_what_there_is() {
    let (mut s, _, _) = started();
    assert_eq!(fails(&mut s, "layer.ad", ""), "there is no command `layer.ad`. Did you mean `layer.add`? (`command.list` lists them all.)");
    assert_eq!(fails(&mut s, "layer.add", r##"{"type": "solid", "colour": "#fff"}"##), "layer.add has no parameter `colour`. Did you mean `color`? (It takes only comp, type, name, color, width, height, item, lens, twoNode, light, shape, text, at, in, out.)");
    assert_eq!(fails(&mut s, "layer.add", r#"{"type": "sprite"}"#), "layer.add: `type` is one of \"solid\", \"null\", \"adjustment\", \"shape\", \"text\", \"comp\", \"footage\", \"camera\", \"light\", \"primitive\", got \"sprite\"");
    assert_eq!(fails(&mut s, "prop.get", r#"{"layer": "Sun", "path": "transform/opacity"}"#), "there is no layer \"Sun\" (it has #1 \"Solid 1\" (2))");
    assert_eq!(fails(&mut s, "comp.info", r#"{"comp": "Intro"}"#), "there is no composition \"Intro\" (there is \"Main\" (1))");
    assert_eq!(fails(&mut s, "layer.add", r##"{"type": "null", "color": "#fff"}"##), "`color` is for solid layers, not a null layer");
    assert_eq!(fails(&mut Session::new(), "layer.add", r#"{"type": "null"}"#), "there is no composition yet (comp.new makes one)");
}

#[test]
fn parents_mattes_and_nested_compositions() {
    let (mut s, main, solid) = started();
    let null = id(&run(&mut s, "layer.add", r#"{"type": "null"}"#), "layer");
    run(&mut s, "layer.set", &format!(r#"{{"layer": {solid}, "parent": {null}, "matte": {{"layer": {null}, "kind": "luma"}}}}"#));
    assert_eq!(fails(&mut s, "layer.set", &format!(r#"{{"layer": {null}, "parent": {solid}}}"#)), format!("that would break the project (items[0].layers[0].parent: layer {null} would be its own parent); nothing was changed"));
    assert_eq!(fails(&mut s, "layer.remove", &format!(r#"{{"layer": {null}}}"#)), "it is the parent of \"Solid 1\"; give them another parent (layer.set parent) first");
    run(&mut s, "layer.set", &format!(r#"{{"layer": {solid}, "parent": null}}"#));
    run(&mut s, "layer.remove", &format!(r#"{{"layer": {null}}}"#));
    assert_eq!(s.project().layer(solid).unwrap().matte, None, "its matte went with it");
    // one composition inside another, but not inside itself
    let inner = id(&run(&mut s, "comp.new", r#"{"name": "Logo", "width": 500, "height": 500, "duration": 2}"#), "comp");
    run(&mut s, "view.set", &format!(r#"{{"comp": {main}}}"#));
    let nested = run(&mut s, "layer.add", r#"{"type": "comp", "item": "Logo", "at": 2}"#);
    assert_eq!(nested.get("index"), Some(&Json::Number(2.0)));
    let layer = s.project().layer(id(&nested, "layer")).unwrap();
    assert_eq!((layer.name.as_str(), layer.out_point), ("Logo", Tick::from_seconds(2.0)));
    assert_eq!(fails(&mut s, "layer.add", &format!(r#"{{"comp": {inner}, "type": "comp", "item": {inner}}}"#)), "a composition cannot show itself");
    assert!(fails(&mut s, "layer.add", &format!(r#"{{"comp": {inner}, "type": "comp", "item": {main}}}"#)).contains("has itself inside"));
    assert_eq!(fails(&mut s, "comp.remove", r#"{"comp": "Logo"}"#), "layers show this composition: \"Logo\" in \"Main\" (remove them first)");
    run(&mut s, "layer.move", r#"{"layer": "Logo", "to": 1}"#);
    assert_eq!(s.project().comp(main).unwrap().layers[0].name, "Logo");
    run(&mut s, "layer.set", r#"{"layer": "Logo", "switches": {"threeD": true, "quality": "draft"}, "blend": "screen", "label": "blue"}"#);
    run(&mut s, "layer.set", r#"{"layer": "Logo", "switches": {"video": false}}"#);
    let sw = &s.project().comp(main).unwrap().layers[0].switches;
    assert!(sw.three_d && !sw.video && sw.quality == veymelo_project::Quality::Draft, "switches change one at a time");
    assert!(fails(&mut s, "layer.set", r#"{"layer": "Logo", "switches": {"3d": true}}"#).contains("there is no \"3d\" here"));
}

#[test]
fn a_project_opens_on_the_composition_it_was_saved_showing() {
    let dir = std::env::temp_dir().join(format!("veymelo-engine-shown-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shown.veymelo");
    let (mut s, _, _) = started();
    let made = run(&mut s, "comp.new", r#"{"name": "Second"}"#);
    let second = made.get("comp").and_then(Json::as_u64).unwrap();
    run(&mut s, "project.save", &format!(r#"{{"path": "{}"}}"#, path.display()));
    let saved = s.project().clone();
    run(&mut s, "project.new", "");
    run(&mut s, "project.open", &format!(r#"{{"path": "{}"}}"#, path.display()));
    assert_eq!(s.view.comp, Some(second), "it opens where the work was");
    assert_eq!(*s.project(), saved, "which composition was shown is not part of the project");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_and_opening() {
    let dir = std::env::temp_dir().join(format!("veymelo-engine-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("one.veymelo");
    let (mut s, _, _) = started();
    run(&mut s, "prop.set", r#"{"layer": 2, "path": "transform/scale", "value": [50, 50]}"#);
    assert!(fails(&mut s, "project.save", "").starts_with("the project has no file yet"));
    assert!(fails(&mut s, "project.new", "").starts_with("the project has changes that are not saved"));
    run(&mut s, "project.save", &format!(r#"{{"path": "{}"}}"#, path.display()));
    assert!(!s.modified());
    let saved = s.project().clone();
    run(&mut s, "project.new", "");
    assert!(s.project().items.is_empty());
    run(&mut s, "project.open", &format!(r#"{{"path": "{}"}}"#, path.display()));
    assert_eq!(*s.project(), saved);
    assert_eq!(s.view.comp, Some(1));
    assert_eq!(s.history().len(), 1, "an opened project's history starts over");
    let info = run(&mut s, "project.info", "");
    assert_eq!(info.get("modified"), Some(&Json::Bool(false)));
    run(&mut s, "comp.set", r#"{"duration": 8}"#);
    assert!(s.modified());
    run(&mut s, "edit.undo", "");
    assert!(!s.modified(), "back at the saved state");
    let doc = run(&mut s, "doc.inspect", r#"{"layer": "Solid 1"}"#);
    assert_eq!(doc.get("props").unwrap().to_json(), r#"{"transform":{"anchor":[200,100,0],"position":[540,960,0],"scale":[50,50,0]}}"#);
    assert!(fails(&mut s, "project.open", r#"{"path": "/nowhere/x.veymelo"}"#).starts_with("cannot read /nowhere/x.veymelo"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compositions_change() {
    let (mut s, c, _) = started();
    run(&mut s, "comp.set", r##"{"name": "Story", "fps": 29.97, "background": "#102030", "duration": 4}"##);
    let info = run(&mut s, "comp.info", r#"{"comp": "Story"}"#);
    assert_eq!(info.get("fps"), Some(&Json::from("30000/1001")));
    assert_eq!(info.get("frames"), Some(&Json::Number(120.0)), "4 s at 29.97 is 119.88 frames: the last one counts");
    assert_eq!(info.get("background"), Some(&Json::from("#102030")));
    assert_eq!(s.project().comp(c).unwrap().work_area.duration, Tick::from_seconds(4.0), "the work area follows a whole-composition one");
    run(&mut s, "comp.set", r#"{"workArea": [1, 2]}"#);
    assert_eq!(fails(&mut s, "comp.set", r#"{"workArea": [3, 9]}"#), "`workArea` is [start, end] in seconds, from 0 to 4 s");
    assert_eq!(fails(&mut s, "comp.new", r#"{"width": 0}"#), "`width` is from 1 to 30000 pixels, not 0");
    assert_eq!(run(&mut s, "comp.new", "").get("name"), Some(&Json::from("Comp 1")));
    run(&mut s, "comp.remove", r#"{"comp": "Comp 1"}"#);
    assert_eq!(s.view.comp, Some(c));
}

#[test]
fn every_command_explains_itself() {
    let mut ids: Vec<&str> = commands().iter().map(|c| c.id).collect();
    ids.dedup();
    assert_eq!(ids.len(), commands().len(), "each id once");
    for c in commands() {
        assert!(c.summary.ends_with('.') && !c.label.is_empty() && !c.returns.is_empty(), "{}", c.id);
        for p in c.params {
            assert!(!p.doc.is_empty(), "{} {}", c.id, p.name);
        }
        let mut names: Vec<&str> = c.params.iter().map(|p| p.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), c.params.len(), "{} has each parameter once", c.id);
    }
    let mut s = Session::new();
    let list = run(&mut s, "command.list", r#"{"search": "undo"}"#);
    assert!(list.as_array().unwrap().iter().any(|c| c.get("id") == Some(&Json::from("edit.undo"))));
    let d = run(&mut s, "command.describe", r#"{"command": "layer.add"}"#);
    assert_eq!(d.get("params").unwrap().get("required").unwrap().to_json(), r#"["type"]"#);
    assert_eq!(d.get("changes"), Some(&Json::Bool(true)));
}

#[test]
fn every_change_is_counted() {
    let mut s = Session::new();
    let r0 = s.revision();
    run(&mut s, "comp.new", "");
    let r1 = s.revision();
    assert!(r1 > r0);
    run(&mut s, "comp.info", "");
    assert_eq!(s.revision(), r1, "looking changes nothing");
    run(&mut s, "view.set", r#"{"time": 2}"#);
    assert!(s.revision() > r1, "the view counts too");
    assert_eq!(s.history().len(), 2, "but is not an undo step");
}

#[test]
fn frames_are_drawn() {
    let (mut s, _, _) = started();
    let dir = std::env::temp_dir().join(format!("veymelo-render-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.png");
    let out = run(&mut s, "render.frame", &format!(r#"{{"scale": 0.5, "path": "{}"}}"#, path.display()));
    assert_eq!((out.get("width"), out.get("height")), (Some(&Json::Number(540.0)), Some(&Json::Number(960.0))));
    assert_eq!(out.get("notes").unwrap().to_json(), "[]");
    assert_eq!(&std::fs::read(&path).unwrap()[1..4], b"PNG");
    let r = s.render(None, None, &render::Options::default()).unwrap();
    assert_eq!(r.image.color_at(540, 960).to_hex(), "#ff0000");
    assert_eq!(r.image.color_at(0, 0).to_hex(), "#000000");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn parenting_keeps_a_layer_where_it_is() {
    let (mut s, _, solid) = started();
    let rig = id(&run(&mut s, "layer.add", r#"{"type": "null"}"#), "layer");
    run(&mut s, "prop.set", r#"{"layer": "Null 1", "path": "transform/position", "value": [300, 400]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Null 1", "path": "transform/rotation", "value": 30}"#);
    run(&mut s, "prop.set", r#"{"layer": "Null 1", "path": "transform/scale", "value": [200, 200]}"#);
    let before = s.render(None, None, &render::Options::default()).unwrap().image;
    run(&mut s, "layer.set", &format!(r#"{{"layer": {solid}, "parent": {rig}}}"#));
    let after = s.render(None, None, &render::Options::default()).unwrap().image;
    let differ = before.px.iter().zip(&after.px).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| (x - y).abs() > 2.0 / 255.0)).count();
    assert!(differ < 20, "it stays where it was ({differ} pixels moved)");
    let scale = s.project().layer(solid).unwrap().props.prop("transform/scale").unwrap().value.as_vec3();
    assert!((scale[0] - 50.0).abs() < 1e-9, "its scale makes up for its parent's: {scale:?}");
    // the parent moving now moves it
    run(&mut s, "prop.set", r#"{"layer": "Null 1", "path": "transform/position", "value": [500, 400]}"#);
    let moved = s.render(None, None, &render::Options::default()).unwrap().image;
    assert_ne!(moved, after);
    // let go, with keep off: it keeps its values and jumps
    run(&mut s, "layer.set", &format!(r#"{{"layer": {solid}, "parent": null, "keep": false}}"#));
    assert_eq!(s.project().layer(solid).unwrap().props.prop("transform/scale").unwrap().value.as_vec3()[0], 50.0);
    assert_eq!(fails(&mut s, "layer.set", &format!(r#"{{"layer": {solid}, "keep": true}}"#)), "`keep` goes with `parent`");
}

#[test]
fn a_matte_hides_itself_and_adjustment_layers_are_layers() {
    let (mut s, _, solid) = started();
    let shape = run(&mut s, "layer.add", r#"{"type": "solid", "name": "Shape", "width": 100, "height": 100}"#);
    let out = run(&mut s, "layer.set", &format!(r#"{{"layer": {solid}, "matte": {{"layer": "Shape", "kind": "luma"}}}}"#));
    assert_eq!(out.get("notes").unwrap().at(0).unwrap().as_str(), Some("\"Shape\" is used as a matte now, so its own video was switched off"));
    assert!(!s.project().layer(id(&shape, "layer")).unwrap().switches.video);
    let adj = run(&mut s, "layer.add", r#"{"type": "adjustment"}"#);
    let l = s.project().layer(id(&adj, "layer")).unwrap();
    assert_eq!((l.name.as_str(), l.switches.adjustment), ("Adjustment Layer 1", true));
    assert_eq!(fails(&mut s, "layer.add", r##"{"type": "adjustment", "color": "#fff"}"##), "`color` is for solid layers, not a adjustment layer");
}

#[test]
fn duplicate_split_and_precompose() {
    let (mut s, main, solid) = started();
    run(&mut s, "prop.set", &format!(r#"{{"layer": {solid}, "path": "transform/opacity", "value": 40}}"#));
    let copy = run(&mut s, "layer.duplicate", &format!(r#"{{"layer": {solid}}}"#));
    assert_eq!((copy.get("name").unwrap().as_str(), copy.get("index")), (Some("Solid 2"), Some(&Json::Number(1.0))));
    let c = s.project().layer(id(&copy, "layer")).unwrap();
    assert_eq!(c.props.prop("transform/opacity").unwrap().value, Value::Number(40.0), "everything about it comes along");
    // cut in two at 2 s
    let part = run(&mut s, "layer.split", &format!(r#"{{"layer": {solid}, "time": 2}}"#));
    assert_eq!(s.project().layer(solid).unwrap().out_point, Tick::from_seconds(2.0));
    assert_eq!(s.project().layer(id(&part, "layer")).unwrap().in_point, Tick::from_seconds(2.0));
    assert!(fails(&mut s, "layer.split", &format!(r#"{{"layer": {solid}, "time": 3}}"#)).contains("cut it in between"));
    // two layers into a composition of their own: the picture stays the same
    let null = id(&run(&mut s, "layer.add", r#"{"type": "null", "at": 4}"#), "layer");
    let before = s.render(None, Some(Tick::SECOND), &render::Options::default()).unwrap().image;
    let pre = run(&mut s, "layer.precompose", &format!(r#"{{"layers": [{solid}, "Solid 2"], "name": "Cards"}}"#));
    let cards = id(&pre, "comp");
    assert_eq!(s.project().comp(cards).unwrap().layers.len(), 2);
    let names: Vec<String> = s.project().comp(main).unwrap().layers.iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, ["Cards", "Solid 3", "Null 1"], "the new layer is where the top moved one was");
    let after = s.render(None, Some(Tick::SECOND), &render::Options::default()).unwrap().image;
    assert_eq!(before.color_at(540, 960), after.color_at(540, 960), "the picture is the same");
    run(&mut s, "edit.undo", "");
    // a parent across the cut is let go, and said
    run(&mut s, "layer.set", &format!(r#"{{"layer": {solid}, "parent": {null}}}"#));
    let pre = run(&mut s, "layer.precompose", &format!(r#"{{"layers": [{solid}]}}"#));
    assert_eq!(pre.get("notes").unwrap().at(0).unwrap().as_str(), Some("\"Solid 1\" lost its parent \"Null 1\""));
    assert_eq!(pre.get("name").unwrap().as_str(), Some("Pre-comp 1"));
    run(&mut s, "edit.undo", "");
    run(&mut s, "edit.undo", "");
    assert_eq!(s.project().comp(main).unwrap().layers.len(), 4, "undone, the layers are back as they were");
}

#[test]
fn project_and_shutter_settings() {
    let (mut s, c, _) = started();
    let out = run(&mut s, "project.set", r#"{"bitDepth": "32", "linearBlending": true}"#);
    assert_eq!(out.get("bitDepth"), Some(&Json::Number(32.0)));
    assert!(s.project().settings.linear_blending);
    assert_eq!(fails(&mut s, "project.set", r#"{"bitDepth": "12"}"#), "project.set: `bitDepth` is one of \"8\", \"16\", \"32\", got \"12\"");
    run(&mut s, "comp.set", r#"{"shutterAngle": 360, "shutterPhase": 0, "samples": 32, "motionBlur": false}"#);
    let mb = &s.project().comp(c).unwrap().motion_blur;
    assert_eq!((mb.angle, mb.phase, mb.samples), (360.0, 0.0, 32));
    assert!(!s.project().comp(c).unwrap().enable_motion_blur);
    assert_eq!(fails(&mut s, "comp.set", r#"{"shutterAngle": 900}"#), "`shutterAngle` is 0 to 720 degrees, not 900");
    let doc = run(&mut s, "doc.inspect", "");
    assert!(doc.to_json().contains(r#""settings":{"bitDepth":32,"linearBlending":true}"#), "{}", doc.to_json());
}

#[test]
fn keys_one_at_a_time() {
    let (mut s, _, _) = started();
    let path = r#""layer": "Solid 1", "path": "transform/opacity""#;
    assert!(fails(&mut s, "key.set", &format!(r#"{{{path}, "time": 1}}"#)).starts_with("there is no key at 1 s (it has no keys); give `value` to make one"));
    run(&mut s, "key.set", &format!(r#"{{{path}, "time": 0, "value": 0}}"#));
    let k = run(&mut s, "key.set", &format!(r#"{{{path}, "time": 2, "value": 100, "in": "bezier", "easeIn": {{"speed": 0, "influence": 0.75}}}}"#));
    assert_eq!(k.get("index"), Some(&Json::Number(1.0)));
    assert_eq!(k.get("easeIn").unwrap().to_json(), r#"{"speed":0,"influence":0.75}"#);
    // found within half a frame of the time given
    run(&mut s, "key.set", &format!(r#"{{{path}, "time": 2.01, "label": "red"}}"#));
    let list = run(&mut s, "key.list", &format!("{{{path}}}"));
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list.at(1).unwrap().get("label"), Some(&Json::Number(1.0)));
    // the keyframe assistant
    run(&mut s, "key.ease", &format!(r#"{{{path}, "ease": "easy"}}"#));
    let mid = |s: &mut Session, t: f64| run(s, "prop.get", &format!(r#"{{{path}, "time": {t}}}"#)).get("value").unwrap().as_f64().unwrap();
    assert!((mid(&mut s, 1.0) - 50.0).abs() < 1e-6 && mid(&mut s, 0.2) < 5.0, "eased");
    run(&mut s, "key.ease", &format!(r#"{{{path}, "ease": "hold", "times": [0]}}"#));
    assert_eq!(mid(&mut s, 1.9), 0.0, "held until the next key");
    run(&mut s, "key.ease", &format!(r#"{{{path}, "ease": "linear"}}"#));
    assert!((mid(&mut s, 0.5) - 25.0).abs() < 1e-6);
    // moved, reversed, removed
    run(&mut s, "key.move", &format!(r#"{{{path}, "time": 2, "to": 4}}"#));
    assert!((mid(&mut s, 2.0) - 50.0).abs() < 1e-6);
    run(&mut s, "key.move", &format!(r#"{{{path}, "by": -1, "times": [4]}}"#));
    assert!((mid(&mut s, 1.5) - 50.0).abs() < 1e-6);
    assert_eq!(fails(&mut s, "key.move", &format!(r#"{{{path}, "time": 3, "to": 0}}"#)), "that would put two keys at one moment");
    run(&mut s, "key.reverse", &format!("{{{path}}}"));
    assert_eq!(mid(&mut s, 0.0), 100.0, "what happened last happens first");
    assert!(fails(&mut s, "key.remove", &format!(r#"{{{path}, "times": [9]}}"#)).starts_with("there is no key at 9 s (there are keys at 0, 3 s)"));
    run(&mut s, "view.set", r#"{"time": 1.5}"#);
    run(&mut s, "key.remove", &format!("{{{path}}}"));
    let p = s.project().layer(2).unwrap().props.prop("transform/opacity").unwrap().clone();
    assert!(!p.is_animated() && (p.value.as_f64() - 50.0).abs() < 1e-6, "no keys left: the value at the view's time stays");
}

#[test]
fn points_move_on_curved_paths() {
    let (mut s, _, _) = started();
    run(&mut s, "prop.keys", r#"{"layer": "Solid 1", "path": "transform/position", "keys": [{"t": 0, "v": [100, 1000]}, {"t": 1, "v": [540, 400]}, {"t": 2, "v": [980, 1000]}]}"#);
    let at = |s: &mut Session, t: f64| run(s, "prop.get", &format!(r#"{{"layer": "Solid 1", "path": "transform/position", "time": {t}}}"#)).get("value").unwrap().as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>();
    let half = at(&mut s, 0.5);
    let straight_y = 700.0;
    assert!(half[1] < straight_y - 5.0, "the path bows up through the middle key: {half:?}");
    // tangents set by hand: a sharp corner at the middle key
    run(&mut s, "key.set", r#"{"layer": "Solid 1", "path": "transform/position", "time": 1, "spatialIn": [0, 0], "spatialOut": [0, 0]}"#);
    let half = at(&mut s, 0.5);
    assert!((half[1] - straight_y).abs() < 1e-6 && (half[0] - 320.0).abs() < 1e-6, "straight lines now: {half:?}");
    // a roving middle key moves to where one speed puts it
    run(&mut s, "key.set", r#"{"layer": "Solid 1", "path": "transform/position", "time": 1, "roving": true}"#);
    let keys = run(&mut s, "key.list", r#"{"layer": "Solid 1", "path": "transform/position"}"#);
    assert!((keys.at(1).unwrap().get("t").unwrap().as_f64().unwrap() - 1.0).abs() < 1e-6, "both sides are as long, so it stays at 1 s");
}

#[test]
fn remapped_frozen_reversed_and_stretched_time() {
    let mut s = Session::new();
    // an inner composition whose solid goes from left to right over 2 s
    run(&mut s, "comp.new", r#"{"name": "Inner", "width": 200, "height": 100, "duration": 2}"#);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 20, "height": 100}"##);
    run(&mut s, "prop.keys", r#"{"layer": "Solid 1", "path": "transform/position", "keys": [{"t": 0, "v": [10, 50]}, {"t": 2, "v": [190, 50]}]}"#);
    let outer = id(&run(&mut s, "comp.new", r#"{"name": "Outer", "width": 200, "height": 100, "duration": 2}"#), "comp");
    run(&mut s, "layer.add", r#"{"type": "comp", "item": "Inner"}"#);
    let x_at = |s: &mut Session, t: f64| {
        let img = s.render(None, Some(Tick::from_seconds(t)), &render::Options::default()).unwrap().image;
        (0..200).find(|&x| img.color_at(x, 50).r > 0.5).map(|x| x as f64 + 10.0).unwrap_or(-1.0)
    };
    assert!((x_at(&mut s, 1.0) - 100.0).abs() <= 1.0);
    // time remapping on: the same at first, then played backwards
    let tr = run(&mut s, "layer.timeRemap", r#"{"layer": "Inner", "on": true}"#);
    assert_eq!(tr.get("keys").unwrap().to_json(), r#"[{"t":0,"v":0},{"t":2,"v":2}]"#);
    assert!((x_at(&mut s, 0.5) - 55.0).abs() <= 1.0);
    run(&mut s, "prop.keys", r#"{"layer": "Inner", "path": "timeRemap", "keys": [{"t": 0, "v": 2}, {"t": 2, "v": 0}]}"#);
    assert!((x_at(&mut s, 0.5) - 145.0).abs() <= 1.0, "backwards: at 0.5 s it shows 1.5 s");
    // a freeze frame holds one moment
    run(&mut s, "layer.freeze", r#"{"layer": "Inner", "time": 0.5}"#);
    assert!((x_at(&mut s, 1.7) - 145.0).abs() <= 1.0);
    run(&mut s, "layer.timeRemap", r#"{"layer": "Inner", "on": false}"#);
    assert!((x_at(&mut s, 0.5) - 55.0).abs() <= 1.0, "off again: its own time");
    // reversed, it stays where it is and plays backwards
    run(&mut s, "layer.reverse", r#"{"layer": "Inner"}"#);
    let l = s.project().comp(outer).unwrap().layers[0].clone();
    assert_eq!((l.in_point, l.out_point, l.stretch), (Tick::ZERO, Tick::from_seconds(2.0), -100.0));
    assert!((x_at(&mut s, 0.5) - 145.0).abs() <= 1.5, "{}", x_at(&mut s, 0.5));
    run(&mut s, "layer.reverse", r#"{"layer": "Inner"}"#);
    // stretched to half speed, it lasts twice as long from its in point
    let st = run(&mut s, "layer.stretch", r#"{"layer": "Inner", "stretch": 200}"#);
    assert_eq!((st.get("in"), st.get("out")), (Some(&Json::Number(0.0)), Some(&Json::Number(4.0))));
    assert!((x_at(&mut s, 1.0) - 55.0).abs() <= 1.0, "at 1 s it shows 0.5 s");
    assert!(fails(&mut s, "layer.timeRemap", r#"{"comp": "Inner", "layer": "Solid 1", "on": true}"#).starts_with("\"Solid 1\" is a solid layer"));
}

#[test]
fn markers_on_compositions_and_layers() {
    let (mut s, _, solid) = started();
    run(&mut s, "marker.add", r#"{"time": 1, "comment": "beat", "label": "red"}"#);
    run(&mut s, "marker.add", r#"{"time": 3, "duration": 0.5, "chapter": "Two"}"#);
    assert_eq!(fails(&mut s, "marker.add", r#"{"time": 1.01}"#), "there is a marker at that time already");
    let list = run(&mut s, "marker.list", "");
    assert_eq!(list.to_json(), r#"[{"index":0,"t":1,"comment":"beat","label":"red","time":1},{"index":1,"t":3,"duration":0.5,"chapter":"Two","time":3}]"#);
    // a layer's marker moves with the layer
    run(&mut s, "marker.add", &format!(r#"{{"layer": {solid}, "time": 2, "comment": "hit"}}"#));
    run(&mut s, "layer.set", &format!(r#"{{"layer": {solid}, "start": 1}}"#));
    let lm = run(&mut s, "marker.list", &format!(r#"{{"layer": {solid}}}"#));
    assert_eq!((lm.at(0).unwrap().get("t"), lm.at(0).unwrap().get("time")), (Some(&Json::Number(2.0)), Some(&Json::Number(3.0))));
    run(&mut s, "marker.remove", r#"{"time": 1}"#);
    assert_eq!(fails(&mut s, "marker.remove", r#"{"time": 2}"#), "there is no marker at 2 s (there are markers at 3 s)");
    assert_eq!(run(&mut s, "marker.list", "").as_array().unwrap().len(), 1);
}

#[test]
fn position_in_separate_dimensions() {
    let (mut s, _, solid) = started();
    run(&mut s, "prop.keys", r#"{"layer": "Solid 1", "path": "transform/position", "keys": [{"t": 0, "v": [100, 200], "out": "bezier", "easeOut": {"speed": 0, "influence": 0.5}}, {"t": 2, "v": [500, 600], "in": "bezier", "easeIn": {"speed": 0, "influence": 0.5}}]}"#);
    let before = run(&mut s, "prop.get", r#"{"layer": "Solid 1", "path": "transform/position", "time": 0.5}"#).get("value").unwrap().clone();
    run(&mut s, "layer.set", &format!(r#"{{"layer": {solid}, "separatePosition": true}}"#));
    let l = s.project().layer(solid).unwrap();
    assert_eq!(l.position_at(Tick::from_seconds(0.5)).map(|v| (v * 1e6).round() / 1e6), before.as_array().unwrap().iter().map(|v| (v.as_f64().unwrap() * 1e6).round() / 1e6).collect::<Vec<_>>()[..].try_into().unwrap_or([0.0; 3]), "the motion is the same");
    assert!(fails(&mut s, "prop.set", r#"{"layer": "Solid 1", "path": "transform/position", "value": [0, 0]}"#).contains("use transform/positionX and transform/positionY"));
    // now x and y move on their own
    run(&mut s, "key.ease", r#"{"layer": "Solid 1", "path": "transform/positionY", "ease": "linear"}"#);
    let y = run(&mut s, "prop.get", r#"{"layer": "Solid 1", "path": "transform/positionY", "time": 0.5}"#).get("value").unwrap().as_f64().unwrap();
    assert!((y - 300.0).abs() < 1e-6, "{y}");
    let out = run(&mut s, "layer.set", &format!(r#"{{"layer": {solid}, "separatePosition": false}}"#));
    assert!(out.get("notes").is_some());
    assert!(s.project().layer(solid).unwrap().props.prop("transform/position").unwrap().is_animated());
}

#[test]
fn shape_layers_built_by_commands() {
    let (mut s, _, _) = started();
    let sl = id(&run(&mut s, "layer.add", r#"{"type": "shape"}"#), "layer");
    assert_eq!(s.project().layer(sl).unwrap().name, "Shape Layer 1");
    let g = run(&mut s, "shape.add", r#"{"layer": "Shape Layer 1", "type": "group"}"#);
    assert_eq!(g.get("path").unwrap().as_str(), Some("contents/Group 1"));
    let r = run(&mut s, "shape.add", r##"{"layer": "Shape Layer 1", "type": "rect", "into": "Group 1", "props": {"size": [300, 120], "roundness": 20}}"##);
    assert_eq!(r.get("path").unwrap().as_str(), Some("contents/Group 1/contents/Rectangle 1"));
    run(&mut s, "shape.add", r##"{"layer": "Shape Layer 1", "type": "fill", "into": "Group 1", "props": {"color": "#00ff00"}}"##);
    run(&mut s, "shape.add", r##"{"layer": "Shape Layer 1", "type": "stroke", "into": "Group 1", "at": 2, "props": {"width": 8, "color": "#ffffff"}}"##);
    // the values can be animated like any property
    run(&mut s, "prop.keys", r#"{"layer": "Shape Layer 1", "path": "contents/Group 1/contents/Rectangle 1/size", "keys": [{"t": 0, "v": [0, 120]}, {"t": 1, "v": [300, 120]}]}"#);
    let img = s.render(None, Some(Tick::SECOND), &render::Options::default()).unwrap().image;
    assert_eq!(img.color_at(540, 960).to_hex(), "#00ff00", "filled in the middle, over the red solid");
    assert_eq!(img.color_at(540 - 150, 960).to_hex(), "#ffffff", "the stroke at its left edge");
    let names: Vec<String> = s.project().layer(sl).unwrap().props.group("contents/Group 1/contents").unwrap().children.iter().map(|n| n.name().to_string()).collect();
    assert_eq!(names, ["Rectangle 1", "Stroke 1", "Fill 1"], "the stroke went in second");
    // mistakes
    assert!(fails(&mut s, "shape.add", r#"{"layer": "Solid 1", "type": "rect"}"#).starts_with("\"Solid 1\" is a solid layer; shape items go in shape layers"));
    assert_eq!(fails(&mut s, "shape.add", r#"{"layer": "Shape Layer 1", "type": "rect", "props": {"sise": [1, 1]}}"#), "`props`: rect has no \"sise\" (it has direction, size, position, roundness)");
    assert!(fails(&mut s, "shape.add", r#"{"layer": "Shape Layer 1", "type": "fill", "into": "Group 9"}"#).contains("has no \"Group 9\""));
    run(&mut s, "shape.remove", r#"{"layer": "Shape Layer 1", "item": "Group 1/contents/Stroke 1"}"#);
    assert_eq!(s.project().layer(sl).unwrap().props.group("contents/Group 1/contents").unwrap().children.len(), 2);
}

#[test]
fn masks_by_command() {
    let (mut s, _, solid) = started();
    let m = run(&mut s, "mask.add", &format!(r#"{{"layer": {solid}, "ellipse": [0, 0, 400, 200], "feather": 10}}"#));
    assert_eq!(m.get("path").unwrap().as_str(), Some("masks/Mask 1"));
    let mask = s.project().layer(solid).unwrap().props.group("masks/Mask 1").unwrap().clone();
    assert_eq!(mask.prop("feather").unwrap().value, Value::Vec2([10.0, 10.0]));
    let img = s.render(None, None, &render::Options::default()).unwrap().image;
    assert_eq!(img.color_at(540, 960).to_hex(), "#ff0000", "inside the ellipse");
    assert_eq!(img.color_at(345, 865).to_hex(), "#000000", "its corner is cut away");
    // the default mask is the whole layer; subtracting it leaves nothing
    run(&mut s, "mask.add", &format!(r#"{{"layer": {solid}, "mode": "subtract"}}"#));
    let img = s.render(None, None, &render::Options::default()).unwrap().image;
    assert_eq!(img.color_at(540, 960).to_hex(), "#000000");
    run(&mut s, "mask.remove", &format!(r#"{{"layer": {solid}, "mask": "Mask 2"}}"#));
    assert_eq!(fails(&mut s, "mask.add", &format!(r#"{{"layer": {solid}, "rect": [0, 0, 1], "ellipse": [0, 0, 1, 1]}}"#)), "give one of `rect`, `ellipse` or `path`");
    run(&mut s, "layer.add", r#"{"type": "shape"}"#);
    assert_eq!(fails(&mut s, "mask.add", r#"{"layer": "Shape Layer 1"}"#), "this layer has no size of its own: give `rect`, `ellipse` or `path`");
}

#[cfg(target_os = "macos")]
#[test]
fn text_layers_built_by_commands() {
    let (mut s, _, _) = started();
    let t = run(&mut s, "layer.add", r##"{"type": "text", "text": {"text": "สวัสดี Sunday", "font": "Tahoma", "size": 96, "fill": "#ffffff"}}"##);
    let tid = id(&t, "layer");
    assert_eq!(t.get("name").unwrap().as_str(), Some("สวัสดี Sunday"), "named by its words");
    run(&mut s, "text.set", &format!(r##"{{"layer": {tid}, "fill": "#00ff00", "justify": "center"}}"##));
    let doc = s.project().layer(tid).unwrap().props.prop("text/sourceText").unwrap().value.doc().unwrap();
    assert_eq!((doc.text.as_str(), doc.size, doc.justify), ("สวัสดี Sunday", 96.0, veymelo_keyframe::Justify::Center), "the rest stays as it was");
    let img = s.render(None, None, &render::Options::default()).unwrap().image;
    let green = (0..img.px.len()).filter(|&i| { let p = img.px[i]; p[1] > 0.9 && p[0] < 0.1 }).count();
    assert!(green > 500, "green letters centred on the middle: {green}");
    // an animator that drops the letters in, one after another
    let an = run(&mut s, "text.animator.add", &format!(r#"{{"layer": {tid}, "props": {{"position": [0, -200], "opacity": 0}}, "selectorProps": {{"shape": "rampDown"}}}}"#));
    let sel = an.get("selector").unwrap().as_str().unwrap().to_string();
    run(&mut s, "prop.keys", &format!(r#"{{"layer": {tid}, "path": "{sel}/offset", "keys": [{{"t": 0, "v": -100}}, {{"t": 1, "v": 100}}]}}"#));
    assert_eq!(s.project().layer(tid).unwrap().props.group("text/animators/Animator 1/selectors").unwrap().children.len(), 1);
    run(&mut s, "text.selector.add", &format!(r#"{{"layer": {tid}, "animator": "Animator 1", "type": "wiggly"}}"#));
    assert!(fails(&mut s, "text.set", r#"{"layer": "Solid 1", "text": "x"}"#).contains("not a text layer"));
    assert!(fails(&mut s, "text.set", &format!(r#"{{"layer": {tid}, "justify": "middle"}}"#)).starts_with("text.set: `justify` is one of"));
}

#[test]
fn expressions_by_command() {
    let (mut s, _, _) = started();
    let out = run(&mut s, "prop.expression", r#"{"layer": "Solid 1", "path": "transform/rotation", "text": "time * 90"}"#);
    assert_eq!(out.to_json(), r#"{"path":"transform/rotation","expression":"time * 90","value":0}"#);
    let out = run(&mut s, "prop.evaluate", r#"{"layer": "Solid 1", "path": "transform/rotation", "times": [0, 1, 2.5]}"#);
    assert_eq!(out.get("values").unwrap().to_json(), r#"[{"t":0,"value":0},{"t":1,"value":90},{"t":2.5,"value":225}]"#);
    let got = run(&mut s, "prop.get", r#"{"layer": "Solid 1", "path": "transform/rotation", "time": 1}"#);
    assert_eq!((got.get("value"), got.get("keyed")), (Some(&Json::Number(90.0)), Some(&Json::Number(0.0))), "its keys' value is shown beside it");
    // trying an expression changes nothing
    let tried = run(&mut s, "prop.evaluate", r#"{"layer": "Solid 1", "path": "transform/position", "text": "value + [10, 0]"}"#);
    assert_eq!(tried.get("values").unwrap().to_json(), r#"[{"t":0,"value":[550,960,0]}]"#);
    assert!(s.project().layer(2).is_none_or(|l| l.props.prop("transform/position").unwrap().expression.is_none()));
    // mistakes: written wrong is refused, failing is said
    assert_eq!(
        fails(&mut s, "prop.expression", r#"{"layer": "Solid 1", "path": "transform/rotation", "text": "wiggle(2,"}"#),
        "`text` is not written right at character 9: the expression ends too soon"
    );
    let out = run(&mut s, "prop.expression", r#"{"layer": "Solid 1", "path": "transform/opacity", "text": "wigle(2, 30)"}"#);
    assert_eq!(out.get("value"), Some(&Json::Number(100.0)));
    assert_eq!(out.get("error").and_then(Json::as_str), Some("the expression on transform/opacity of \"Solid 1\" failed: there is nothing called \"wigle\""));
    let notes = s.render(None, None, &render::Options::default()).unwrap().notes;
    assert_eq!(notes, ["the expression on transform/opacity of \"Solid 1\" failed: there is nothing called \"wigle\""]);
    let off = run(&mut s, "prop.expression", r#"{"layer": "Solid 1", "path": "transform/opacity", "enabled": false}"#);
    assert_eq!(off.get("enabled"), Some(&Json::Bool(false)));
    assert!(s.render(None, None, &render::Options::default()).unwrap().notes.is_empty(), "switched off, it is not run");
    run(&mut s, "prop.expression", r#"{"layer": "Solid 1", "path": "transform/opacity", "text": ""}"#);
    assert!(fails(&mut s, "prop.expression", r#"{"layer": "Solid 1", "path": "transform/opacity", "enabled": true}"#).contains("has no expression to switch"));
    // the picture follows the expression
    run(&mut s, "prop.expression", r#"{"layer": "Solid 1", "path": "transform/position", "text": "[300, 200]"}"#);
    run(&mut s, "prop.expression", r#"{"layer": "Solid 1", "path": "transform/rotation", "text": ""}"#);
    let img = s.render(None, None, &render::Options::default()).unwrap().image;
    assert_eq!((img.color_at(300, 200).to_hex(), img.color_at(540, 960).to_hex()), ("#ff0000".into(), "#000000".into()));
    run(&mut s, "edit.undo", "");
    run(&mut s, "edit.undo", "");
    let img = s.render(None, None, &render::Options::default()).unwrap().image;
    assert_eq!(img.color_at(540, 960).to_hex(), "#ff0000", "undone, it is back in the middle");
    assert!(fails(&mut s, "prop.expression", r#"{"layer": "Solid 1", "path": "transform/rotation"}"#).contains("give `text`"));
}

#[test]
fn puppet_pins_by_command() {
    let (mut s, _, _) = started();
    let types = run(&mut s, "effect.types", r#"{"category": "distort"}"#);
    assert_eq!(types.as_array().unwrap()[0].get("type").and_then(Json::as_str), Some("puppet"));
    // the solid is 400 × 200, its middle at the composition's (540, 960)
    let first = run(&mut s, "puppet.pin", r#"{"layer": "Solid 1", "at": [20, 100]}"#);
    assert_eq!(first.get("path").and_then(Json::as_str), Some("effects/Puppet/meshes/Mesh 1/pins/Puppet Pin 1"));
    let second = run(&mut s, "puppet.pin", r#"{"layer": "Solid 1", "at": [380, 100]}"#);
    assert_eq!(second.get("path").and_then(Json::as_str), Some("effects/Puppet/meshes/Mesh 1/pins/Puppet Pin 2"), "the same mesh covers both");
    run(&mut s, "prop.keys", r#"{"layer": "Solid 1", "path": "effects/Puppet/meshes/Mesh 1/pins/Puppet Pin 2/position", "keys": [{"t": 0, "v": [380, 100]}, {"t": 1, "v": [380, 300]}]}"#);
    let info = run(&mut s, "puppet.info", r#"{"layer": "Solid 1", "time": 1}"#);
    let mesh = &info.get("meshes").unwrap().as_array().unwrap()[0];
    assert!(mesh.get("triangles").and_then(Json::as_f64).unwrap() > 100.0, "{}", info.to_json());
    assert_eq!(mesh.get("bounds").unwrap().to_json(), "[-3,-3,403,203]", "grown 3 past the solid");
    assert_eq!(mesh.get("pins").unwrap().as_array().unwrap()[1].get("position").unwrap().to_json(), "[380,300]");
    let frame = |s: &mut Session, t: f64| {
        run(s, "view.set", &format!(r#"{{"time": {t}}}"#));
        s.render(None, None, &render::Options::default()).unwrap()
    };
    let still = frame(&mut s, 0.0);
    assert!(still.notes.is_empty(), "{:?}", still.notes);
    assert_eq!((still.image.color_at(360, 960).to_hex(), still.image.color_at(720, 960).to_hex()), ("#ff0000".into(), "#ff0000".into()));
    let bent = frame(&mut s, 1.0);
    assert_eq!(bent.image.color_at(360, 960).to_hex(), "#ff0000", "the pinned end stays");
    assert_eq!(bent.image.color_at(720, 1160).to_hex(), "#ff0000", "the moved end went down 200");
    assert_eq!(bent.image.color_at(735, 900).to_hex(), "#000000", "where it was is empty");
    // a bend pin in the middle, a starch pin, mistakes
    let bend = run(&mut s, "puppet.pin", r#"{"layer": "Solid 1", "at": [200, 100], "kind": "bend", "props": {"rotation": 20}}"#);
    assert_eq!(bend.get("pin").and_then(Json::as_str), Some("Puppet Pin 3"));
    let starch = run(&mut s, "puppet.pin", r#"{"layer": "Solid 1", "at": [100, 100], "kind": "starch"}"#);
    assert_eq!(starch.get("pin").and_then(Json::as_str), Some("Starch 1"));
    assert!(fails(&mut s, "effect.add", r#"{"layer": "Solid 1", "type": "puppett"}"#).contains("did you mean \"puppet\""));
    run(&mut s, "layer.add", r#"{"type": "null"}"#);
    assert!(fails(&mut s, "puppet.pin", r#"{"layer": "Null 1", "at": [0, 0]}"#).contains("shows nothing"), "a null has effects, but no pixels to pin");
    // a project with a puppet is saved and read back
    let text = s.project().to_text();
    assert_eq!(&project::Project::from_text(&text, &catalog()).unwrap(), s.project());
    run(&mut s, "item.remove", r#"{"layer": "Solid 1", "path": "effects/Puppet/meshes/Mesh 1/pins/Starch 1"}"#);
    assert!(fails(&mut s, "item.remove", r#"{"layer": "Solid 1", "path": "transform/position"}"#).contains("not a list"));
    run(&mut s, "effect.remove", r#"{"layer": "Solid 1", "effect": "Puppet"}"#);
    assert_eq!(frame(&mut s, 1.0).image.color_at(720, 1160).to_hex(), "#000000", "without it the solid is flat again");
}

#[test]
fn effects_blur_layers_and_what_is_under_adjustment_layers() {
    let (mut s, _, _) = started();
    let frame = |s: &mut Session| s.render(None, None, &render::Options::default()).unwrap();
    // the red solid is 400 × 200 in the middle: its left edge at x 340
    assert_eq!(frame(&mut s).image.color_at(335, 960).to_hex(), "#000000");
    run(&mut s, "effect.add", r#"{"layer": "Solid 1", "type": "gaussianBlur", "props": {"blurriness": 20}}"#);
    let soft = frame(&mut s);
    assert!(soft.notes.is_empty(), "{:?}", soft.notes);
    assert!(soft.image.color_at(335, 960).r > 0.1, "the blur spreads past the edge");
    assert_eq!(soft.image.color_at(540, 960).to_hex(), "#ff0000", "the middle stays");
    // a radial blur's centre starts in the middle of the layer
    let radial = run(&mut s, "effect.add", r#"{"layer": "Solid 1", "type": "radialBlur"}"#);
    let path = format!("{}/center", radial.get("path").and_then(Json::as_str).unwrap());
    assert_eq!(run(&mut s, "prop.get", &format!(r#"{{"layer": "Solid 1", "path": "{path}"}}"#)).get("value").unwrap().to_json(), "[200,100]");
    run(&mut s, "effect.remove", r#"{"layer": "Solid 1", "effect": "Radial Blur"}"#);
    run(&mut s, "effect.remove", r#"{"layer": "Solid 1", "effect": "Gaussian Blur"}"#);
    // an adjustment layer blurs what is under it, where it is
    let adj = run(&mut s, "layer.add", r#"{"type": "adjustment"}"#);
    let aid = id(&adj, "layer");
    run(&mut s, "effect.add", &format!(r#"{{"layer": {aid}, "type": "gaussianBlur", "props": {{"blurriness": 20}}}}"#));
    let under = frame(&mut s);
    assert!(under.image.color_at(335, 960).r > 0.1 && under.image.color_at(745, 960).r > 0.1, "both edges softened: {:?}", under.notes);
    // with a mask over its left half only the left edge is softened
    run(&mut s, "mask.add", &format!(r#"{{"layer": {aid}, "rect": [0, 0, 540, 1920]}}"#));
    let half = frame(&mut s);
    assert!(half.image.color_at(335, 960).r > 0.1);
    assert_eq!(half.image.color_at(745, 960).to_hex(), "#000000");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {aid}, "path": "transform/opacity", "value": 0}}"#));
    assert_eq!(frame(&mut s).image.color_at(335, 960).to_hex(), "#000000", "at no opacity it changes nothing");
    // an expression control drives a property
    run(&mut s, "effect.add", r#"{"layer": "Solid 1", "type": "sliderControl", "props": {"slider": 40}}"#);
    run(&mut s, "prop.expression", r#"{"layer": "Solid 1", "path": "transform/opacity", "text": "effect(\"Slider Control\")(\"Slider\")"}"#);
    assert_eq!(run(&mut s, "prop.get", r#"{"layer": "Solid 1", "path": "transform/opacity"}"#).get("value"), Some(&Json::Number(40.0)));
    assert_eq!(frame(&mut s).image.color_at(540, 960).to_hex(), "#660000");
}

#[test]
fn liquify_strokes_by_command() {
    let (mut s, _, _) = started();
    // the solid is 400 × 200, its middle at the composition's (540, 960)
    let first = run(&mut s, "liquify.stroke", r#"{"layer": "Solid 1", "points": [[100, 100], [300, 100]], "size": 120, "pressure": 100}"#);
    assert_eq!(first.get("effect").and_then(Json::as_str), Some("effects/Liquify"), "added");
    assert_eq!(first.get("stroke").and_then(Json::as_str), Some("warp 120 100 70 0 0 100,100 300,100"));
    let second = run(&mut s, "liquify.stroke", r#"{"layer": "Solid 1", "points": [[200, 100]], "tool": "bloat"}"#);
    assert_eq!(second.get("strokes").and_then(Json::as_f64), Some(2.0));
    assert_eq!(second.get("stroke").and_then(Json::as_str), Some("bloat 64 50 70 0 0 200,100"), "the rest from the effect's brush");
    let mesh = run(&mut s, "prop.get", r#"{"layer": "Solid 1", "path": "effects/Liquify/distortionMesh"}"#);
    assert_eq!(mesh.get("value").and_then(Json::as_str).unwrap().lines().count(), 2);
    // drawn: the red solid's left edge was dragged right
    let img = s.render(None, None, &render::Options::default()).unwrap().image;
    assert_eq!(img.color_at(345, 960).to_hex(), "#ff0000", "the edge left of the stroke stays");
    // with keys, the stroke goes into the key at the time
    run(&mut s, "prop.keys", r#"{"layer": "Solid 1", "path": "effects/Liquify/distortionMesh", "keys": [{"t": 0, "v": ""}, {"t": 1, "v": ""}]}"#);
    run(&mut s, "liquify.stroke", r#"{"layer": "Solid 1", "points": [[50, 50]], "time": 1}"#);
    let at = |s: &mut Session, t: f64| run(s, "prop.get", &format!(r#"{{"layer": "Solid 1", "path": "effects/Liquify/distortionMesh", "time": {t}}}"#)).get("value").and_then(Json::as_str).unwrap().to_string();
    assert_eq!((at(&mut s, 0.0), at(&mut s, 1.0).lines().count()), (String::new(), 1));
    assert!(fails(&mut s, "liquify.stroke", r#"{"layer": "Solid 1", "points": []}"#).contains("at least one point"));
    assert!(fails(&mut s, "liquify.stroke", r#"{"layer": "Solid 1", "points": [[1, 1]], "effect": "Nope"}"#).contains("no Liquify"));
}

/// A small composition on clear: a white square in the middle, styled.
fn styled() -> (Session, u64) {
    let mut s = Session::new();
    run(&mut s, "comp.new", r##"{"name": "S", "width": 100, "height": 100, "fps": 30, "duration": 1, "background": "#000000"}"##);
    let l = run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 40, "height": 40}"##);
    (s, id(&l, "layer"))
}

fn picture(s: &Session) -> render::Rendered {
    s.render(None, None, &render::Options { background: false, ..render::Options::default() }).unwrap()
}

fn px(r: &render::Rendered, x: usize, y: usize) -> [f32; 4] {
    r.image.px[y * r.image.width + x]
}

#[test]
fn layer_styles_are_added_in_their_order() {
    let (mut s, l) = styled();
    run(&mut s, "style.add", r##"{"layer": "#1", "type": "stroke", "props": {"size": 4}}"##);
    run(&mut s, "style.add", r##"{"layer": "#1", "type": "Drop Shadow"}"##);
    let list = s.project().layer(l).unwrap().props.group("layerStyles").unwrap();
    let order: Vec<&str> = list.children.iter().map(|c| c.id()).collect();
    assert_eq!(order, ["blendingOptions", "dropShadow", "stroke"]);
    // again: switched on, given the settings, not doubled
    run(&mut s, "style.add", r##"{"layer": "#1", "type": "stroke", "props": {"size": 9}}"##);
    let list = s.project().layer(l).unwrap().props.group("layerStyles").unwrap();
    assert_eq!(list.children.len(), 3);
    assert_eq!(list.prop("stroke/size").unwrap().value, Value::Number(9.0));
    assert!(fails(&mut s, "style.add", r##"{"layer": "#1", "type": "glowy"}"##).contains("there is no layer style"));
    run(&mut s, "style.remove", r##"{"layer": "#1", "style": "dropShadow"}"##);
    assert_eq!(s.project().layer(l).unwrap().props.group("layerStyles").unwrap().children.len(), 2);
    run(&mut s, "style.remove", r##"{"layer": "#1", "all": true}"##);
    assert!(s.project().layer(l).unwrap().props.group("layerStyles").unwrap().children.is_empty());
    let types = run(&mut s, "style.types", "");
    assert_eq!(types.as_array().map(|a| a.len()), Some(10));
}

#[test]
fn the_light_is_one_for_the_composition() {
    let (mut s, a) = styled();
    run(&mut s, "style.add", r##"{"layer": "#1", "type": "dropShadow"}"##);
    run(&mut s, "prop.set", r##"{"layer": "#1", "path": "layerStyles/blendingOptions/globalLightAngle", "value": 45}"##);
    let b = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ff0000", "width": 10, "height": 10}"##), "layer");
    run(&mut s, "style.add", r##"{"layer": "#1", "type": "bevelEmboss"}"##);
    let angle = |s: &Session, l: u64| s.project().layer(l).unwrap().props.prop("layerStyles/blendingOptions/globalLightAngle").unwrap().value.clone();
    assert_eq!(angle(&s, b), Value::Number(45.0), "a layer given styles takes the composition's light");
    run(&mut s, "prop.set", r##"{"layer": "#1", "path": "layerStyles/blendingOptions/globalLightAngle", "value": 200}"##);
    assert_eq!(angle(&s, a), Value::Number(200.0), "changing one changes them all");
}

#[test]
fn styles_are_drawn() {
    let (mut s, _) = styled();
    let plain = picture(&s);
    assert_eq!(px(&plain, 75, 75)[3], 0.0);
    // a drop shadow down to the right (the light from the upper left), not to the upper left
    run(&mut s, "style.add", r##"{"layer": "#1", "type": "dropShadow", "props": {"distance": 10, "size": 0, "opacity": 100}}"##);
    let shadow = picture(&s);
    assert!(px(&shadow, 72, 72)[3] > 0.9 && px(&shadow, 72, 72)[0] < 0.01, "black under the lower right: {:?}", px(&shadow, 72, 72));
    assert_eq!(px(&shadow, 27, 27)[3], 0.0);
    assert_eq!(px(&shadow, 50, 50), [1.0, 1.0, 1.0, 1.0], "the layer on top");
    run(&mut s, "style.remove", r##"{"layer": "#1", "all": true}"##);
    // a stroke outside: 5 pixels more all round; a colour overlay: red inside only
    run(&mut s, "style.add", r##"{"layer": "#1", "type": "stroke", "props": {"size": 5, "color": "#0000ff"}}"##);
    run(&mut s, "style.add", r##"{"layer": "#1", "type": "colorOverlay", "props": {"color": "#ff0000"}}"##);
    let ring = picture(&s);
    assert_eq!(px(&ring, 50, 50), [1.0, 0.0, 0.0, 1.0]);
    let edge = px(&ring, 50, 27);
    assert!(edge[2] > 0.99 && edge[3] > 0.99, "the stroke: {edge:?}");
    assert_eq!(px(&ring, 50, 22)[3], 0.0, "no farther");
    // no fill: the styles alone
    run(&mut s, "prop.set", r##"{"layer": "#1", "path": "layerStyles/blendingOptions/advancedBlending/fillOpacity", "value": 0}"##);
    run(&mut s, "style.remove", r##"{"layer": "#1", "style": "colorOverlay"}"##);
    let hollow = picture(&s);
    assert_eq!(px(&hollow, 50, 50)[3], 0.0);
    assert!(px(&hollow, 50, 27)[3] > 0.99);
}

#[test]
fn blending_channels_and_knockout() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r##"{"name": "S", "width": 60, "height": 60, "fps": 30, "duration": 1, "background": "#000000"}"##);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#0000ff", "name": "Under"}"##);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 20, "height": 20, "name": "Top"}"##);
    run(&mut s, "style.add", r#"{"layer": "Top", "type": "colorOverlay", "props": {"opacity": 0}}"#);
    run(&mut s, "prop.set", r#"{"layer": "Top", "path": "layerStyles/blendingOptions/advancedBlending/green", "value": false}"#);
    let p = px(&picture(&s), 30, 30);
    assert!(p[0] > 0.99 && p[1] < 0.01 && p[2] > 0.99, "white drawn but not into green: {p:?}");
    run(&mut s, "prop.set", r#"{"layer": "Top", "path": "layerStyles/blendingOptions/advancedBlending/green", "value": true}"#);
    run(&mut s, "prop.set", r#"{"layer": "Top", "path": "layerStyles/blendingOptions/advancedBlending/fillOpacity", "value": 0}"#);
    run(&mut s, "prop.set", r#"{"layer": "Top", "path": "layerStyles/blendingOptions/advancedBlending/knockout", "value": "deep"}"#);
    let p = px(&picture(&s), 30, 30);
    assert!(p[3] < 0.01, "a hole through what is under it: {p:?}");
    assert!(px(&picture(&s), 5, 5)[2] > 0.99, "the rest as it was");
}

/// A composition with a white 3D square in the middle (40 × 40) on clear.
fn scene_3d() -> Session {
    let mut s = Session::new();
    run(&mut s, "comp.new", r##"{"name": "S", "width": 100, "height": 100, "fps": 30, "duration": 1, "background": "#000000"}"##);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 40, "height": 40, "name": "Card"}"##);
    run(&mut s, "layer.set", r#"{"layer": "Card", "switches": {"threeD": true}}"#);
    s
}

/// How many pixels are mostly covered.
fn covered(r: &render::Rendered) -> usize {
    r.image.px.iter().filter(|p| p[3] > 0.5).count()
}

#[test]
fn a_3d_layer_facing_the_camera_is_as_in_2d() {
    let mut s = scene_3d();
    let flat3d = picture(&s);
    run(&mut s, "layer.set", r#"{"layer": "Card", "switches": {"threeD": false}}"#);
    let flat2d = picture(&s);
    let most = flat3d.image.px.iter().zip(&flat2d.image.px).map(|(a, b)| (0..4).map(|k| (a[k] - b[k]).abs()).fold(0f32, f32::max)).fold(0f32, f32::max);
    assert!(most < 0.02, "the standard camera shows the z = 0 plane at its own size: {most}");
    // a camera added by default sees the same
    run(&mut s, "layer.set", r#"{"layer": "Card", "switches": {"threeD": true}}"#);
    run(&mut s, "layer.add", r#"{"type": "camera"}"#);
    let through = picture(&s);
    let most = through.image.px.iter().zip(&flat2d.image.px).map(|(a, b)| (0..4).map(|k| (a[k] - b[k]).abs()).fold(0f32, f32::max)).fold(0f32, f32::max);
    assert!(most < 0.02, "{most}");
}

#[test]
fn farther_is_smaller_and_edge_on_is_nothing() {
    let mut s = scene_3d();
    let near = covered(&picture(&s));
    let zoom = veymelo_project::scene::default_zoom(100.0);
    run(&mut s, "prop.set", &format!(r#"{{"layer": "Card", "path": "transform/position", "value": [50, 50, {zoom}]}}"#));
    let far = covered(&picture(&s));
    assert!((far as f64 - near as f64 / 4.0).abs() < near as f64 * 0.05, "twice as far, half as wide: {near} {far}");
    run(&mut s, "prop.set", r#"{"layer": "Card", "path": "transform/position", "value": [50, 50, 0]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Card", "path": "transform/rotationY", "value": 90}"#);
    assert!(covered(&picture(&s)) < 5, "edge-on");
    run(&mut s, "prop.set", r#"{"layer": "Card", "path": "transform/rotationY", "value": 60}"#);
    let turned = covered(&picture(&s));
    assert!(turned < near * 6 / 10 && turned > near * 4 / 10, "turned 60°: about half as wide: {turned} of {near}");
}

#[test]
fn depth_not_stack_order_says_what_is_in_front() {
    let mut s = scene_3d();
    // a red card on top of the stack but behind the white one
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ff0000", "width": 40, "height": 40, "name": "Back"}"##);
    run(&mut s, "layer.set", r#"{"layer": "Back", "switches": {"threeD": true}}"#);
    run(&mut s, "prop.set", r#"{"layer": "Back", "path": "transform/position", "value": [60, 60, 50]}"#);
    let p = px(&picture(&s), 50, 50);
    assert!(p[1] > 0.9, "the white card in front where they overlap: {p:?}");
    // turned through each other: each in front on one side of where they cross
    run(&mut s, "prop.set", r#"{"layer": "Back", "path": "transform/position", "value": [50, 50, 0]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Back", "path": "transform/rotationY", "value": 40}"#);
    let r = picture(&s);
    let (left, right) = (px(&r, 38, 50), px(&r, 62, 50));
    assert!((left[1] > 0.9) != (right[1] > 0.9), "the crossing parts them: {left:?} {right:?}");
}

#[test]
fn lights_light_and_shadows_fall() {
    let mut s = scene_3d();
    let unlit = px(&picture(&s), 50, 50);
    assert_eq!(unlit, [1.0, 1.0, 1.0, 1.0], "no lights: as it is");
    // a light straight in front: the card takes its diffuse half (and no shine)
    run(&mut s, "layer.add", r#"{"type": "light", "light": "parallel"}"#);
    run(&mut s, "prop.set", r#"{"layer": "Parallel Light 1", "path": "transform/position", "value": [50, 50, -500]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Parallel Light 1", "path": "transform/pointOfInterest", "value": [50, 50, 0]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Card", "path": "materialOptions/specularIntensity", "value": 0}"#);
    let lit = px(&picture(&s), 50, 50);
    assert!((lit[0] - 0.5).abs() < 0.02, "{lit:?}");
    // a card between the light and this one casts a shadow on it
    run(&mut s, "prop.set", r#"{"layer": "Parallel Light 1", "path": "lightOptions/castsShadows", "value": true}"#);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#808080", "width": 10, "height": 10, "name": "Caster"}"##);
    run(&mut s, "layer.set", r#"{"layer": "Caster", "switches": {"threeD": true}}"#);
    run(&mut s, "prop.set", r#"{"layer": "Caster", "path": "transform/position", "value": [40, 50, -20]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Caster", "path": "materialOptions/castsShadows", "value": "only"}"#);
    let r = picture(&s);
    assert!(px(&r, 40, 50)[0] < 0.05, "in the shadow: {:?}", px(&r, 40, 50));
    assert!((px(&r, 60, 50)[0] - 0.5).abs() < 0.02, "out of it");
}

#[test]
fn the_gpu_draws_a_lit_scene_as_the_processor_does() {
    if !veymelo_os::gpu::available() {
        return;
    }
    let mut s = Session::new();
    run(&mut s, "comp.new", r##"{"name": "S", "width": 160, "height": 120, "fps": 30, "duration": 1, "background": "#000000"}"##);
    // a floor, a tinted see-through card standing on it, a card turned through it, and one
    // screened over the rest and cut by a matte
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#d0c8b0", "width": 300, "height": 300, "name": "Floor"}"##);
    run(&mut s, "layer.set", r#"{"layer": "Floor", "switches": {"threeD": true}}"#);
    run(&mut s, "prop.set", r#"{"layer": "Floor", "path": "transform/position", "value": [80, 90, 0]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Floor", "path": "transform/rotationX", "value": -80}"#);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#3070ff", "width": 50, "height": 60, "name": "Glass"}"##);
    run(&mut s, "layer.set", r#"{"layer": "Glass", "switches": {"threeD": true}}"#);
    run(&mut s, "prop.set", r#"{"layer": "Glass", "path": "transform/position", "value": [60, 60, -20]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Glass", "path": "transform/opacity", "value": 70}"#);
    run(&mut s, "prop.set", r#"{"layer": "Glass", "path": "materialOptions/castsShadows", "value": "on"}"#);
    run(&mut s, "prop.set", r#"{"layer": "Glass", "path": "materialOptions/lightTransmission", "value": 40}"#);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ff5020", "width": 60, "height": 40, "name": "Turned"}"##);
    run(&mut s, "layer.set", r#"{"layer": "Turned", "switches": {"threeD": true}}"#);
    run(&mut s, "prop.set", r#"{"layer": "Turned", "path": "transform/position", "value": [90, 60, -10]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Turned", "path": "transform/rotationY", "value": 50}"#);
    run(&mut s, "prop.set", r#"{"layer": "Turned", "path": "materialOptions/castsShadows", "value": "on"}"#);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 70, "height": 120, "name": "Matte"}"##);
    run(&mut s, "prop.set", r#"{"layer": "Matte", "path": "transform/position", "value": [125, 60]}"#);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#20c060", "width": 50, "height": 50, "name": "Screened"}"##);
    run(&mut s, "layer.set", r#"{"layer": "Screened", "switches": {"threeD": true}, "blend": "screen", "matte": {"layer": "Matte"}}"#);
    run(&mut s, "prop.set", r#"{"layer": "Screened", "path": "transform/position", "value": [120, 55, -30]}"#);
    // a soft spot light throwing shadows, a parallel one, and some ambient light
    run(&mut s, "layer.add", r#"{"type": "light", "light": "spot", "name": "Spot"}"#);
    run(&mut s, "prop.set", r#"{"layer": "Spot", "path": "transform/position", "value": [20, -40, -160]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Spot", "path": "transform/pointOfInterest", "value": [80, 70, 0]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Spot", "path": "lightOptions/castsShadows", "value": true}"#);
    run(&mut s, "prop.set", r#"{"layer": "Spot", "path": "lightOptions/shadowDiffusion", "value": 12}"#);
    run(&mut s, "prop.set", r#"{"layer": "Spot", "path": "lightOptions/falloff", "value": "smooth"}"#);
    run(&mut s, "layer.add", r#"{"type": "light", "light": "parallel", "name": "Sun"}"#);
    run(&mut s, "prop.set", r#"{"layer": "Sun", "path": "transform/position", "value": [200, -100, -200]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Sun", "path": "lightOptions/intensity", "value": 40}"#);
    run(&mut s, "layer.add", r#"{"type": "light", "light": "ambient", "name": "Fill"}"#);
    run(&mut s, "prop.set", r#"{"layer": "Fill", "path": "lightOptions/intensity", "value": 20}"#);
    run(&mut s, "layer.add", r#"{"type": "camera"}"#);
    run(&mut s, "prop.set", r#"{"layer": "Camera 1", "path": "transform/position", "value": [40, 20, -220]}"#);
    let draw = |gpu: bool| s.render(None, None, &render::Options { background: false, gpu, ..render::Options::default() }).unwrap();
    let before = render::scenes_drawn();
    let (cpu, gpu) = (draw(false), draw(true));
    assert!(render::scenes_drawn() > before, "the GPU drew it");
    assert!(cpu.notes.is_empty(), "{:?}", cpu.notes);
    let lit = cpu.image.px.iter().filter(|p| p[3] > 0.0).count();
    assert!(lit > 10_000, "the scene fills the picture: {lit}");
    // as the processor draws it, to within a level of eight bits (the edges of soft shadows
    // and of crossings may round the other way)
    let mut off = 0;
    for (p, q) in cpu.image.px.iter().zip(&gpu.image.px) {
        let most = (0..4).map(|k| (p[k] - q[k]).abs()).fold(0f32, f32::max);
        assert!(most < 4.0 / 255.0, "{p:?} {q:?}");
        if most > 0.5 / 255.0 {
            off += 1;
        }
    }
    let worst = cpu.image.px.iter().zip(&gpu.image.px).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).fold(0f32, f32::max)).fold(0f32, f32::max);
    eprintln!("{off} of {} pixels half a level or more apart, the most {:.2} levels", cpu.image.px.len(), worst * 255.0);
    assert!(off * 200 < cpu.image.px.len(), "{off} of {} pixels a level or more apart", cpu.image.px.len());
}

#[test]
fn out_of_focus_blurs() {
    let mut s = scene_3d();
    run(&mut s, "layer.add", r#"{"type": "camera"}"#);
    let sharp = picture(&s);
    run(&mut s, "prop.set", r#"{"layer": "Camera 1", "path": "cameraOptions/depthOfField", "value": true}"#);
    run(&mut s, "prop.set", r#"{"layer": "Camera 1", "path": "cameraOptions/focusDistance", "value": 60}"#);
    run(&mut s, "prop.set", r#"{"layer": "Camera 1", "path": "cameraOptions/aperture", "value": 20}"#);
    let soft = picture(&s);
    let edge = |r: &render::Rendered| px(r, 30, 50)[3] - px(r, 29, 50)[3];
    assert!(edge(&soft) < edge(&sharp) * 0.5, "the edge spread: {} {}", edge(&soft), edge(&sharp));
}

#[test]
fn moving_3d_layers_blur() {
    let mut s = scene_3d();
    run(&mut s, "prop.keys", r#"{"layer": "Card", "path": "transform/position", "keys": [{"t": 0, "v": [0, 50, 0]}, {"t": 1, "v": [600, 50, 0]}]}"#);
    run(&mut s, "view.set", r#"{"time": 0.1}"#);
    let sharp = picture(&s);
    run(&mut s, "layer.set", r#"{"layer": "Card", "switches": {"motionBlur": true}}"#);
    let blurred = picture(&s);
    let part = |r: &render::Rendered| r.image.px.iter().filter(|p| p[3] > 0.02 && p[3] < 0.98).count();
    assert!(part(&blurred) > part(&sharp) * 4, "its edges smeared along the way it moves: {} {}", part(&blurred), part(&sharp));
}

#[test]
fn effects_see_through_the_composition_camera() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r##"{"name": "S", "width": 80, "height": 60, "fps": 30, "duration": 1, "background": "#000000"}"##);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#3080ff", "name": "Cards"}"##);
    let plain = picture(&s);
    run(&mut s, "effect.add", r#"{"layer": "Cards", "type": "cardDance", "props": {"cameraSystem": "compCamera", "lightType": "firstCompLight"}}"#);
    let at_rest = picture(&s);
    let most = |a: &render::Rendered, b: &render::Rendered| a.image.px.iter().zip(&b.image.px).map(|(p, q)| (0..3).map(|k| (p[k] - q[k]).abs()).fold(0f32, f32::max)).fold(0f32, f32::max);
    assert!(most(&at_rest, &plain) < 0.03, "through the standard camera, unlit: as it is ({})", most(&at_rest, &plain));
    run(&mut s, "layer.add", r#"{"type": "camera"}"#);
    run(&mut s, "prop.set", r#"{"layer": "Camera 1", "path": "transform/position", "value": [10, 30, -120]}"#);
    let turned = picture(&s);
    assert!(most(&turned, &plain) > 0.3, "the camera moved: seen from the side");
}

#[test]
fn a_3d_layer_cut_by_its_track_matte() {
    let mut s = scene_3d();
    // a matte covering the left half
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 50, "height": 100, "name": "Matte"}"##);
    run(&mut s, "prop.set", r#"{"layer": "Matte", "path": "transform/position", "value": [25, 50]}"#);
    run(&mut s, "layer.set", r#"{"layer": "Card", "matte": {"layer": "Matte"}}"#);
    let r = picture(&s);
    assert!(px(&r, 40, 50)[3] > 0.99 && px(&r, 60, 50)[3] < 0.01, "{:?} {:?}", px(&r, 40, 50), px(&r, 60, 50));
}

#[test]
fn a_collapsed_composition_blurs_as_it_moves() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r##"{"name": "Inner", "width": 100, "height": 100, "fps": 30, "duration": 1}"##);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 30, "height": 30}"##);
    run(&mut s, "comp.new", r##"{"name": "Outer", "width": 200, "height": 100, "fps": 30, "duration": 1, "background": "#000000"}"##);
    run(&mut s, "layer.add", r#"{"type": "comp", "item": "Inner"}"#);
    run(&mut s, "layer.set", r#"{"layer": "Inner", "switches": {"collapse": true}}"#);
    run(&mut s, "prop.keys", r#"{"layer": "Inner", "path": "transform/position", "keys": [{"t": 0, "v": [0, 50, 0]}, {"t": 1, "v": [900, 50, 0]}]}"#);
    run(&mut s, "view.set", r#"{"time": 0.1}"#);
    let sharp = picture(&s);
    run(&mut s, "layer.set", r#"{"layer": "Inner", "switches": {"motionBlur": true}}"#);
    let blurred = picture(&s);
    let part = |r: &render::Rendered| r.image.px.iter().filter(|p| p[3] > 0.02 && p[3] < 0.98).count();
    assert!(part(&blurred) > part(&sharp) * 4 && blurred.notes.is_empty(), "{} {} {:?}", part(&blurred), part(&sharp), blurred.notes);
}

#[test]
fn a_collapsed_3d_composition_joins_the_scene() {
    let mut s = Session::new();
    // inside: a red 3D card turned 40°
    run(&mut s, "comp.new", r##"{"name": "Inner", "width": 100, "height": 100, "fps": 30, "duration": 1}"##);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ff0000", "width": 40, "height": 40, "name": "Red"}"##);
    run(&mut s, "layer.set", r#"{"layer": "Red", "switches": {"threeD": true}}"#);
    run(&mut s, "prop.set", r#"{"layer": "Red", "path": "transform/rotationY", "value": 40}"#);
    // outside: a white card straight on, and the inner composition collapsed and 3D on it
    run(&mut s, "comp.new", r##"{"name": "Outer", "width": 100, "height": 100, "fps": 30, "duration": 1, "background": "#000000"}"##);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 40, "height": 40, "name": "White"}"##);
    run(&mut s, "layer.set", r#"{"layer": "White", "switches": {"threeD": true}}"#);
    run(&mut s, "layer.add", r#"{"type": "comp", "item": "Inner"}"#);
    run(&mut s, "layer.set", r#"{"layer": "Inner", "switches": {"threeD": true, "collapse": true}}"#);
    let r = picture(&s);
    let (left, right) = (px(&r, 38, 50), px(&r, 62, 50));
    assert!((left[1] > 0.9) != (right[1] > 0.9), "the inner card crosses the outer one: {left:?} {right:?}");
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    // and as a flat layer (not collapsed) it is all in front, as one plane
    run(&mut s, "layer.set", r#"{"layer": "Inner", "switches": {"collapse": false}}"#);
    let flat = picture(&s);
    assert!(px(&flat, 38, 50)[1] < 0.1 && px(&flat, 62, 50)[1] < 0.1, "the inner composition's picture on top: {:?} {:?}", px(&flat, 38, 50), px(&flat, 62, 50));
}

#[test]
fn the_render_queue_makes_files() {
    let (mut s, _, _) = started();
    let dir = std::env::temp_dir().join(format!("veymelo-queue-{}", std::process::id()));
    let gif = dir.join("main.gif").to_string_lossy().into_owned();
    let out = run(&mut s, "queue.add", &format!(r#"{{"path": "{gif}", "format": "gif", "settings": {{"resolution": 0.1, "span": "custom", "start": 0, "end": 0.2}}, "output": {{"gifLoop": false}}}}"#));
    let item = id(&out, "item");
    assert!(fails(&mut s, "queue.add", r#"{"path": "x.png", "settings": {"resolution": 3}}"#).contains("resolution"));
    let list = run(&mut s, "queue.list", "");
    assert_eq!(list.as_array().map(|a| a.len()), Some(1));
    let done = run(&mut s, "queue.render", "");
    let made = done.get("made").and_then(Json::as_array).unwrap();
    assert_eq!(made.len(), 1, "{}", done.to_json());
    assert_eq!(made[0].get("frames").and_then(Json::as_f64), Some(6.0));
    assert!(std::fs::read(&gif).unwrap().starts_with(b"GIF89a"));
    assert_eq!(s.project().queue[0].status, veymelo_project::queue::Status::Done);
    // done: nothing more to make, unless again
    assert!(fails(&mut s, "queue.render", "").contains("nothing"));
    run(&mut s, "queue.set", &format!(r#"{{"item": {item}, "output": {{"resize": [20, 30]}}}}"#));
    assert_eq!(s.project().queue[0].status, veymelo_project::queue::Status::Queued, "changed: to make again");
    // it is kept in the project
    let text = s.project().to_text();
    assert!(text.contains("renderQueue") && text.contains("main.gif"));
    run(&mut s, "queue.remove", &format!(r#"{{"item": {item}}}"#));
    assert!(s.project().queue.is_empty());
    // at once, without the queue
    let png = dir.join("now_##.png").to_string_lossy().into_owned();
    let now = run(&mut s, "render.export", &format!(r#"{{"path": "{png}", "format": "pngSequence", "settings": {{"resolution": 0.05, "span": "custom", "start": 1, "end": 1.1}}}}"#));
    assert_eq!(now.get("frames").and_then(Json::as_f64), Some(3.0));
}

#[test]
fn lottie_out_and_back_draws_the_same() {
    let (mut s, c, _) = started();
    run(&mut s, "comp.set", r#"{"width": 270, "height": 480}"#);
    run(&mut s, "prop.keys", r#"{"layer": "Solid 1", "path": "transform/position", "keys": [{"t": 0, "v": [40, 60, 0]}, {"t": 2, "v": [230, 400, 0]}]}"#);
    run(&mut s, "prop.set", r#"{"layer": "Solid 1", "path": "transform/scale", "value": [30, 30, 100]}"#);
    run(&mut s, "layer.add", r#"{"type": "shape"}"#);
    run(&mut s, "shape.add", r#"{"layer": "Shape Layer 1", "type": "group"}"#);
    run(&mut s, "shape.add", r##"{"layer": "Shape Layer 1", "type": "ellipse", "into": "Group 1", "props": {"size": [120, 80], "position": [135, 240]}}"##);
    run(&mut s, "shape.add", r##"{"layer": "Shape Layer 1", "type": "fill", "into": "Group 1", "props": {"color": "#22aaff"}}"##);
    run(&mut s, "shape.add", r##"{"layer": "Shape Layer 1", "type": "stroke", "into": "Group 1", "props": {"width": 6, "color": "#ffffff"}}"##);
    run(&mut s, "prop.keys", r#"{"layer": "Shape Layer 1", "path": "transform/rotation", "keys": [{"t": 0, "v": 0}, {"t": 2, "v": 90}]}"#);
    let dir = std::env::temp_dir().join(format!("veymelo-lottie-{}", std::process::id()));
    let path = dir.join("scene.lottie").to_string_lossy().into_owned();
    let out = run(&mut s, "lottie.export", &format!(r#"{{"path": "{path}"}}"#));
    assert_eq!(out.get("layers").and_then(Json::as_f64), Some(2.0));
    assert!(out.get("warnings").and_then(Json::as_array).is_some_and(|w| w.is_empty()), "{}", out.to_json());
    assert!(std::fs::read(&path).unwrap().starts_with(b"PK"));
    assert!(fails(&mut s, "lottie.export", r#"{"path": "scene.mp4"}"#).contains(".json or .lottie"));
    let back = run(&mut s, "lottie.import", &format!(r#"{{"path": "{path}"}}"#));
    let b = id(&back, "comp");
    assert_eq!(s.view.comp, Some(b), "the view goes to it");
    // the same pictures (over the same background)
    assert_eq!(s.project().comp(b).unwrap().background, s.project().comp(c).unwrap().background);
    for t in [0.0, 0.7, 1.3, 2.0] {
        let opts = render::Options { background: true, ..render::Options::default() };
        let (x, y) = (s.render(Some(&Json::Number(c as f64)), Some(Tick::from_seconds(t)), &opts).unwrap(), s.render(Some(&Json::Number(b as f64)), Some(Tick::from_seconds(t)), &opts).unwrap());
        let most = x.image.px.iter().zip(&y.image.px).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max);
        // eases go out to a millionth, so an edge pixel may round one step the other way
        let off = x.image.px.iter().zip(&y.image.px).filter(|(p, q)| (0..4).any(|k| (p[k] - q[k]).abs() > 1e-3)).count();
        assert!(most <= 1.0 / 255.0 + 1e-6 && off <= 3, "at {t} s {off} pixels differ, by up to {most}");
    }
    run(&mut s, "edit.undo", "");
    assert!(s.project().comp(b).is_none(), "the import is one undo step");
    assert!(fails(&mut s, "lottie.import", r#"{"path": "/no/such/file.json"}"#).contains("/no/such/file.json"));
}

#[cfg(target_os = "macos")]
#[test]
fn footage_made_by_us_comes_back_the_same() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r##"{"name": "Moving", "width": 64, "height": 36, "fps": 10, "duration": 1, "background": "#102030"}"##);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ff8000", "width": 10, "height": 10}"##);
    run(&mut s, "prop.keys", r##"{"layer": "#1", "path": "transform/position", "keys": [{"t": 0, "v": [5, 18, 0]}, {"t": 0.9, "v": [59, 18, 0]}]}"##);
    let dir = std::env::temp_dir().join(format!("veymelo-footage-{}", std::process::id()));
    let mov = dir.join("moving.mov").to_string_lossy().into_owned();
    run(&mut s, "render.export", &format!(r#"{{"path": "{mov}", "format": "prores", "output": {{"prores": "4444"}}, "settings": {{"span": "comp"}}}}"#));
    let pngs = dir.join("frames_##.png").to_string_lossy().into_owned();
    run(&mut s, "render.export", &format!(r#"{{"path": "{pngs}", "format": "pngSequence", "settings": {{"span": "comp"}}}}"#));
    // in as footage
    let video = run(&mut s, "footage.import", &format!(r#"{{"path": "{mov}"}}"#));
    assert_eq!((video.get("kind").and_then(Json::as_str), video.get("width").and_then(Json::as_f64), video.get("frames").and_then(Json::as_f64)), (Some("video"), Some(64.0), Some(10.0)), "{}", video.to_json());
    let first = dir.join("frames_03.png").to_string_lossy().into_owned();
    let seq = run(&mut s, "footage.import", &format!(r#"{{"path": "{first}", "sequence": true}}"#));
    assert_eq!((seq.get("kind").and_then(Json::as_str), seq.get("start").and_then(Json::as_f64), seq.get("frames").and_then(Json::as_f64)), (Some("sequence"), Some(0.0), Some(10.0)), "{}", seq.to_json());
    run(&mut s, "footage.set", &format!(r#"{{"item": {}, "fps": 10}}"#, id(&seq, "item")));
    // drawn in a composition of their own: the same pictures
    run(&mut s, "comp.new", r#"{"name": "Back", "width": 64, "height": 36, "fps": 10, "duration": 1}"#);
    let picture_at = |s: &Session, comp: &str, t: f64| s.render(Some(&Json::from(comp)), Some(Tick::from_seconds(t)), &render::Options::default()).unwrap();
    for (item, tolerance) in [(id(&seq, "item"), 1.5 / 255.0), (id(&video, "item"), 6.0 / 255.0)] {
        run(&mut s, "layer.add", &format!(r#"{{"comp": "Back", "type": "footage", "item": {item}}}"#));
        for t in [0.0, 0.3, 0.8] {
            let (a, b) = (picture_at(&s, "Moving", t), picture_at(&s, "Back", t));
            let worst = a.image.px.iter().zip(&b.image.px).map(|(p, q)| (0..3).map(|k| (p[k] - q[k]).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max);
            assert!(worst <= tolerance, "item {item} at {t} s differs by {worst}");
        }
        run(&mut s, "layer.remove", r##"{"comp": "Back", "layer": "#1"}"##);
    }
    // conformed to 5 a second: twice as long, its frames twice as long each
    let slow = run(&mut s, "footage.set", &format!(r#"{{"item": {}, "fps": 5}}"#, id(&video, "item")));
    assert_eq!((slow.get("duration").and_then(Json::as_f64), slow.get("madeAt").and_then(Json::as_str)), (Some(2.0), None), "{}", slow.to_json());
    assert_eq!(slow.get("madeAt").map(|v| v.to_json()), Some("10".into()));
    run(&mut s, "comp.set", r#"{"comp": "Back", "duration": 2}"#);
    run(&mut s, "layer.add", &format!(r#"{{"comp": "Back", "type": "footage", "item": {}}}"#, id(&video, "item")));
    let at = |s: &Session, comp: &str, t: f64, x: usize| picture_at(s, comp, t).image.px[18 * 64 + x];
    // at 0.7 s it shows its frame 3 (0.3 s of the original)
    let (shown, original) = (at(&s, "Back", 0.7, 31), at(&s, "Moving", 0.3, 31));
    assert!((shown[0] - original[0]).abs() < 0.03, "{shown:?} {original:?}");
    // frames mixed where the composition falls between two of them
    run(&mut s, "layer.set", r##"{"comp": "Back", "layer": "#1", "switches": {"frameBlend": "frameMix"}}"##);
    let (a, b, mixed) = (at(&s, "Moving", 0.3, 26), at(&s, "Moving", 0.4, 26), at(&s, "Back", 0.7, 26));
    assert!((mixed[0] - (a[0] + b[0]) / 2.0).abs() < 0.03, "{a:?} {b:?} {mixed:?}");
    let info = run(&mut s, "footage.info", "");
    assert_eq!(info.get("footage").and_then(Json::as_array).map(Vec::len), Some(2));
    // a file that went: missing, and drawn as nothing with a note
    std::fs::rename(&mov, dir.join("gone.mov")).unwrap();
    veymelo_media::forget(None);
    let missing = run(&mut s, "footage.info", &format!(r#"{{"item": {}}}"#, id(&video, "item")));
    assert_eq!(missing.get("missing").and_then(Json::as_bool), Some(true));
    let r = picture_at(&s, "Back", 0.5);
    assert!(r.notes.iter().any(|n| n.contains("moving.mov")), "{:?}", r.notes);
    run(&mut s, "footage.replace", &format!(r#"{{"item": {}, "path": "{}"}}"#, id(&video, "item"), dir.join("gone.mov").display()));
    assert!(picture_at(&s, "Back", 0.5).notes.is_empty());
    assert!(fails(&mut s, "footage.import", r#"{"path": "notes.txt"}"#).contains("not a kind of file"));
}

#[test]
fn sound_measured_and_seen() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r#"{"name": "Beep", "width": 64, "height": 64, "fps": 30, "duration": 4}"#);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#000000", "width": 8, "height": 8}"##);
    let quiet = run(&mut s, "audio.levels", "");
    assert_eq!((quiet.get("heard").and_then(Json::as_bool), quiet.get("integrated")), (Some(false), None));
    // a 1 kHz tone at −20 dBFS (a tenth): −20 LUFS
    run(&mut s, "effect.add", r##"{"layer": "#1", "type": "tone", "props": {"frequency1": 1000, "frequency2": 0, "frequency3": 0, "frequency4": 0, "frequency5": 0, "level": 10}}"##);
    let l = run(&mut s, "audio.levels", r#"{"start": 0.5, "end": 3.5}"#);
    assert_eq!(l.get("heard").and_then(Json::as_bool), Some(true));
    let lufs = l.get("integrated").and_then(Json::as_f64).unwrap();
    assert!((lufs + 20.0).abs() < 0.1, "{}", l.to_json());
    assert_eq!(l.get("peak").map(|p| p.to_json()), Some("[-20,-20]".into()));
    assert_eq!(l.get("clips").and_then(Json::as_bool), Some(false));
    let png = std::env::temp_dir().join(format!("veymelo-wave-{}.png", std::process::id())).to_string_lossy().into_owned();
    let w = run(&mut s, "audio.waveform", &format!(r#"{{"path": "{png}", "width": 300, "height": 100}}"#));
    assert_eq!(w.get("width").and_then(Json::as_f64), Some(300.0));
    assert!(std::fs::read(&png).unwrap().starts_with(b"\x89PNG"));
    let (pic, about) = s.hear(&Json::parse(r#"{"width": 200}"#).unwrap()).unwrap();
    assert!(pic.starts_with(b"\x89PNG") && about.get("integrated").is_some());
    assert!(fails(&mut s, "audio.levels", r#"{"start": 2, "end": 1}"#).contains("ends after it starts"));
}

#[test]
fn sound_into_keys() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r#"{"name": "Beat", "width": 64, "height": 64, "fps": 10, "duration": 2}"#);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#000000", "width": 8, "height": 8}"##);
    assert!(fails(&mut s, "audio.toKeys", "").contains("nothing in the composition is heard"));
    // a tone that comes in at one second
    run(&mut s, "effect.add", r##"{"layer": "#1", "type": "tone", "props": {"frequency1": 500, "frequency2": 0, "frequency3": 0, "frequency4": 0, "frequency5": 0, "level": 50}}"##);
    run(&mut s, "layer.set", r##"{"layer": "#1", "in": 1}"##);
    let out = run(&mut s, "audio.toKeys", "");
    assert_eq!(out.get("keys").and_then(Json::as_f64), Some(20.0));
    let mut slider = |t: f64| run(&mut s, "prop.evaluate", &format!(r#"{{"layer": "Audio Amplitude", "path": "effects/Both Channels/slider", "times": [{t}]}}"#)).get("values").unwrap().at(0).unwrap().get("value").unwrap().as_f64().unwrap();
    assert!(slider(0.5).abs() < 1e-6, "silent at first");
    let loud = slider(1.5);
    assert!((loud - 50.0 / 2f64.sqrt()).abs() < 0.5, "a sine at half scale: {loud} (its square mean, 35.4)");
    // an expression follows it
    run(&mut s, "prop.expression", r##"{"layer": "#2", "path": "transform/opacity", "text": "thisComp.layer(\"Audio Amplitude\").effect(\"Both Channels\")(\"Slider\") * 2"}"##);
    let o = run(&mut s, "prop.evaluate", r##"{"layer": "#2", "path": "transform/opacity", "times": [1.5]}"##);
    assert!((o.get("values").unwrap().at(0).unwrap().get("value").unwrap().as_f64().unwrap() - 70.7).abs() < 1.0);
}

#[cfg(target_os = "macos")]
#[test]
fn an_animated_gif_as_footage() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r##"{"name": "Blink", "width": 32, "height": 32, "fps": 10, "duration": 1, "background": "#000000"}"##);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 32, "height": 32, "in": 0.5}"##);
    let gif = std::env::temp_dir().join(format!("veymelo-blink-{}.gif", std::process::id())).to_string_lossy().into_owned();
    run(&mut s, "render.export", &format!(r#"{{"path": "{gif}", "format": "gif", "settings": {{"span": "comp"}}}}"#));
    let f = run(&mut s, "footage.import", &format!(r#"{{"path": "{gif}"}}"#));
    assert_eq!((f.get("kind").and_then(Json::as_str), f.get("frames").and_then(Json::as_f64), f.get("duration").and_then(Json::as_f64)), (Some("video"), Some(10.0), Some(1.0)), "{}", f.to_json());
    run(&mut s, "comp.new", r#"{"name": "Back", "width": 32, "height": 32, "fps": 10, "duration": 1}"#);
    run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {}}}"#, id(&f, "item")));
    let grey = |s: &Session, t: f64| s.render(Some(&Json::from("Back")), Some(Tick::from_seconds(t)), &render::Options::default()).unwrap().image.px[16 * 32 + 16][0];
    assert!(grey(&s, 0.2) < 0.05 && grey(&s, 0.7) > 0.95, "dark, then white: {} {}", grey(&s, 0.2), grey(&s, 0.7));
}

#[test]
fn project_items_kept_in_order() {
    let (mut s, main, solid) = started();
    let folder = id(&run(&mut s, "project.folder.new", r#"{"name": "Shots"}"#), "item");
    let inner = id(&run(&mut s, "project.folder.new", &format!(r#"{{"name": "Old", "folder": {folder}}}"#)), "item");
    let moved = run(&mut s, "project.item.set", r#"{"item": "Main", "folder": "Shots", "label": "blue", "comment": "the cut"}"#);
    assert_eq!((moved.get("folder").and_then(Json::as_u64), moved.get("label").and_then(Json::as_str)), (Some(folder), Some("blue")));
    assert!(fails(&mut s, "project.item.set", &format!(r#"{{"item": {folder}, "folder": {inner}}}"#)).contains("inside itself"));
    // a copy: its own ids, its links kept
    run(&mut s, "layer.add", r#"{"type": "null"}"#);
    run(&mut s, "layer.set", &format!(r#"{{"layer": {solid}, "parent": "Null 1"}}"#));
    let copy = run(&mut s, "project.item.duplicate", r#"{"item": "Main"}"#);
    assert_eq!(copy.get("name").and_then(Json::as_str), Some("Main 2"));
    let (a, b) = (s.project().comp(main).unwrap().clone(), s.project().comp(id(&copy, "item")).unwrap().clone());
    assert!(a.layers.iter().zip(&b.layers).all(|(x, y)| x.id != y.id && x.name == y.name));
    let child = b.layers.iter().find(|l| l.name == "Solid 1").unwrap();
    assert_eq!(child.parent.and_then(|p| b.layer(p)).map(|l| l.name.as_str()), Some("Null 1"), "parented to its own copy of the null");
    assert_eq!(s.project().item(id(&copy, "item")).unwrap().folder, Some(folder));
    // the folder taken out: what was in it goes up
    run(&mut s, "project.item.remove", &format!(r#"{{"item": {folder}}}"#));
    assert_eq!(s.project().item(main).unwrap().folder, None);
    assert_eq!(s.project().item(inner).unwrap().folder, None);
}

#[cfg(target_os = "macos")]
#[test]
fn a_project_gathered_moves_whole() {
    let dir = std::env::temp_dir().join(format!("veymelo-collect-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pic = dir.join("logo.tif").to_string_lossy().into_owned();
    let px: Vec<u8> = (0..16).flat_map(|_| [255u8, 0, 0, 255]).collect();
    veymelo_os::write_picture(&pic, 4, 4, &px, "public.tiff", true, 1.0).unwrap();
    let (mut s, _, _) = started();
    let logo = run(&mut s, "footage.import", &format!(r#"{{"path": "{pic}"}}"#));
    run(&mut s, "footage.import", &format!(r#"{{"path": "{pic}", "name": "spare"}}"#));
    run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {}}}"#, id(&logo, "item")));
    // footage a layer shows is kept unless forced
    assert!(fails(&mut s, "project.item.remove", &format!(r#"{{"item": {}}}"#, id(&logo, "item"))).contains("force"));
    let unused = run(&mut s, "project.removeUnused", "");
    assert_eq!(unused.get("removed").map(|r| r.to_json()), Some(r#"["spare"]"#.into()));
    // gathered, then the folder moved: the copy still finds its footage
    let out = dir.join("gathered");
    let got = run(&mut s, "project.collect", &format!(r#"{{"folder": "{}"}}"#, out.display()));
    assert_eq!(got.get("files").and_then(Json::as_array).map(Vec::len), Some(1));
    let text = std::fs::read_to_string(out.join("project.veymelo")).unwrap();
    assert!(text.contains(r#""path": "footage/logo.tif""#), "kept relative to the project");
    let moved = dir.join("moved");
    std::fs::rename(&out, &moved).unwrap();
    let mut t = Session::open(&moved.join("project.veymelo")).unwrap();
    let info = run(&mut t, "footage.info", r#"{"item": "logo.tif"}"#);
    assert_eq!(info.get("missing"), None, "{}", info.to_json());
    assert!(info.get("path").and_then(Json::as_str).unwrap().starts_with(moved.to_str().unwrap()));
    // and brought into another project
    let mut u = Session::new();
    let back = run(&mut u, "project.import", &format!(r#"{{"path": "{}"}}"#, moved.join("project.veymelo").display()));
    assert_eq!(back.get("items").and_then(Json::as_array).map(Vec::len), Some(2), "{}", back.to_json());
    let comp = u.project().items.iter().find(|i| i.as_comp().is_some()).unwrap();
    assert_eq!(comp.folder, Some(id(&back, "folder")));
    assert!(u.project().problems().is_empty(), "{:?}", u.project().problems());
    run(&mut t, "footage.reload", "");
}

#[test]
fn keyframe_assistants() {
    let (mut s, _, l) = started();
    let path = format!(r#""layer": {l}, "path": "transform/rotation""#);
    run(&mut s, "prop.keys", &format!(r#"{{{path}, "keys": [{{"t": 0, "v": 0}}, {{"t": 2, "v": 100}}]}}"#));
    // the Wiggler: a key every sixth frame, each within 10 of the line, the same for the same seed
    let w = run(&mut s, "key.wiggle", &format!(r#"{{{path}, "frequency": 5, "magnitude": 10, "seed": 3}}"#));
    assert_eq!((w.get("before").and_then(Json::as_f64), w.get("after").and_then(Json::as_f64)), (Some(2.0), Some(11.0)));
    let keys = w.get("keys").and_then(Json::as_array).unwrap().clone();
    for k in &keys {
        let (t, v) = (k.get("time").and_then(Json::as_f64).unwrap(), k.get("v").and_then(Json::as_f64).unwrap());
        assert!((v - t * 50.0).abs() <= 10.0 + 1e-9, "{t} {v}");
    }
    run(&mut s, "edit.undo", "");
    let again = run(&mut s, "key.wiggle", &format!(r#"{{{path}, "frequency": 5, "magnitude": 10, "seed": 3}}"#));
    assert_eq!(again.get("keys"), Some(&Json::Array(keys)), "the same seed, the same keys");
    // the Smoother: a key every frame of a wave, made a handful
    let wave: Vec<String> = (0..=60).map(|f| format!(r#"{{"t": {}, "v": {}}}"#, f as f64 / 30.0, (f as f64 / 60.0 * std::f64::consts::TAU).sin() * 50.0)).collect();
    run(&mut s, "prop.keys", &format!(r#"{{{path}, "keys": [{}]}}"#, wave.join(",")));
    let sm = run(&mut s, "key.smooth", &format!(r#"{{{path}, "tolerance": 1}}"#));
    let after = sm.get("after").and_then(Json::as_f64).unwrap();
    assert!(after <= 16.0, "{after} keys of 61");
    let at = |s: &mut Session, t: f64| run(s, "prop.evaluate", &format!(r#"{{{path}, "times": [{t}]}}"#)).get("values").unwrap().at(0).unwrap().get("value").unwrap().as_f64().unwrap();
    for f in [7, 22, 41, 53] {
        let want = (f as f64 / 60.0 * std::f64::consts::TAU).sin() * 50.0;
        assert!((at(&mut s, f as f64 / 30.0) - want).abs() <= 1.01, "frame {f}");
    }
    // keys scaled in time about the first, and in value
    run(&mut s, "prop.keys", &format!(r#"{{{path}, "keys": [{{"t": 0.5, "v": 10}}, {{"t": 1, "v": 20}}]}}"#));
    run(&mut s, "key.transform", &format!(r#"{{{path}, "timeScale": 2, "valueScale": 3}}"#));
    let k = run(&mut s, "key.list", &format!(r#"{{{path}}}"#));
    let times: Vec<f64> = k.as_array().unwrap().iter().map(|k| k.get("time").and_then(Json::as_f64).unwrap()).collect();
    assert_eq!(times, [0.5, 1.5]);
    assert!((at(&mut s, 1.5) - 60.0).abs() < 1e-9);
    // Exponential Scale: the same factor each frame (100 to 400 over two frames: 200 between)
    let scale = format!(r#""layer": {l}, "path": "transform/scale""#);
    run(&mut s, "prop.keys", &format!(r#"{{{scale}, "keys": [{{"t": 0, "v": [100, 100, 100]}}, {{"t": 0.0666667, "v": [400, 400, 100]}}]}}"#));
    run(&mut s, "key.exponentialScale", &format!(r#"{{"layer": {l}}}"#));
    let mid = run(&mut s, "prop.evaluate", &format!(r#"{{{scale}, "times": [0.0333333]}}"#));
    assert!((mid.get("values").unwrap().at(0).unwrap().get("value").unwrap().at(0).unwrap().as_f64().unwrap() - 200.0).abs() < 0.5, "{}", mid.to_json());
    // Motion Sketch: a drawn line, a key a frame (no smoothing), then smoothed to two
    let pts: Vec<String> = (0..=10).map(|i| format!("[{}, {}]", 100 + i * 10, 300)).collect();
    let n = run(&mut s, "motion.sketch", &format!(r#"{{"layer": {l}, "points": [{}], "start": 1, "smoothing": 0}}"#, pts.join(",")));
    assert_eq!(n.get("keys").and_then(Json::as_f64), Some(11.0));
    run(&mut s, "edit.undo", "");
    let n = run(&mut s, "motion.sketch", &format!(r#"{{"layer": {l}, "points": [{}], "start": 1}}"#, pts.join(",")));
    assert_eq!(n.get("keys").and_then(Json::as_f64), Some(2.0), "a straight line needs two");
}

#[test]
fn layers_one_after_another() {
    let (mut s, _, _) = started();
    for _ in 0..2 {
        run(&mut s, "layer.add", r##"{"type": "solid", "color": "#00ff00", "out": 2}"##);
    }
    run(&mut s, "layer.set", r##"{"layer": "#3", "out": 2}"##);
    let seq = run(&mut s, "layer.sequence", r##"{"layers": ["#3", "#2", "#1"], "overlap": 0.5, "transition": "crossDissolve"}"##);
    let ins: Vec<f64> = seq.get("layers").unwrap().as_array().unwrap().iter().map(|l| l.get("in").and_then(Json::as_f64).unwrap()).collect();
    assert_eq!(ins, [0.0, 1.5, 3.0]);
    let mid = run(&mut s, "prop.evaluate", r##"{"layer": "#2", "path": "transform/opacity", "times": [1.75, 2.5, 3.25]}"##);
    let v: Vec<f64> = mid.get("values").unwrap().as_array().unwrap().iter().map(|x| x.get("value").and_then(Json::as_f64).unwrap()).collect();
    assert_eq!(v, [50.0, 100.0, 50.0], "fading in, held, fading out");
    let st = run(&mut s, "layer.stagger", r##"{"layers": ["#1", "#2", "#3"], "step": 0.2}"##);
    assert_eq!(st.get("layers").unwrap().at(2).unwrap().get("offset").and_then(Json::as_f64), Some(0.4));
    assert!(fails(&mut s, "layer.stagger", r##"{"layers": ["#1"], "step": 1}"##).contains("two layers"));
}

#[test]
fn editing_in_time_and_space() {
    let (mut s, main, solid) = started();
    // an inner composition 4 s long, shown from 1 s to 3 s
    let inner = id(&run(&mut s, "comp.new", r#"{"name": "Inner", "width": 100, "height": 100, "fps": 30, "duration": 4}"#), "comp");
    run(&mut s, "view.set", &format!(r#"{{"comp": {main}}}"#));
    let l = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "comp", "item": {inner}, "in": 1, "out": 3}}"#)), "layer");
    // slipped: content later by 0.5 s, the layer where it was; as far as its content reaches
    let sl = run(&mut s, "layer.slip", &format!(r#"{{"layer": {l}, "by": 0.5}}"#));
    assert_eq!(sl.get("moved").and_then(Json::as_f64), Some(0.5));
    let far = run(&mut s, "layer.slip", &format!(r#"{{"layer": {l}, "by": 10}}"#));
    assert_eq!(far.get("moved").and_then(Json::as_f64), Some(0.5), "its content starts at its in point at most");
    let shown = s.project().comp(main).unwrap().layer(l).unwrap().clone();
    assert_eq!((shown.in_point, shown.out_point, shown.start), (Tick::from_seconds(1.0), Tick::from_seconds(3.0), Tick::from_seconds(1.0)));
    // stretched so its in point is at 0, the out staying: 3 s for 2 s of content
    let st = run(&mut s, "layer.stretchTo", &format!(r#"{{"layer": {l}, "in": 0}}"#));
    assert!((st.get("stretch").and_then(Json::as_f64).unwrap() - 150.0).abs() < 1e-9);
    let stretched = s.project().comp(main).unwrap().layer(l).unwrap().clone();
    assert_eq!(stretched.source_time(Tick::from_seconds(3.0)), shown.source_time(Tick::from_seconds(3.0)), "the out point shows what it did");
    // held on its last frame to the end
    run(&mut s, "edit.undo", "");
    let fr = run(&mut s, "layer.freezeLast", &format!(r#"{{"layer": {l}}}"#));
    assert_eq!(fr.get("out").and_then(Json::as_f64), Some(5.0));
    let held = s.project().comp(main).unwrap().layer(l).unwrap().clone();
    assert_eq!(held.source_time(Tick::from_seconds(4.5)), held.source_time(Tick::from_seconds(3.0) - Tick::from_seconds(1.0 / 30.0)));
    // the work area extracted: what came after moves up
    run(&mut s, "comp.set", r#"{"workArea": [2, 2.5]}"#);
    let ex = run(&mut s, "comp.extractWorkArea", "");
    assert_eq!(ex.get("cut").and_then(Json::as_f64), Some(2.0), "both layers cross it");
    let c = s.project().comp(main).unwrap();
    assert_eq!(c.layers.len(), 4);
    assert!(c.layers.iter().filter(|x| x.name == "Solid 1").any(|x| x.in_point == Tick::from_seconds(2.0) && x.out_point == Tick::from_seconds(4.5)), "the solid's second part moved up");
    assert!(s.project().problems().is_empty(), "{:?}", s.project().problems());
    // trimmed to a work area
    run(&mut s, "edit.undo", "");
    run(&mut s, "comp.set", r#"{"workArea": [1, 3]}"#);
    run(&mut s, "comp.trimToWorkArea", "");
    let c = s.project().comp(main).unwrap();
    assert_eq!((c.duration, c.layer(solid).unwrap().out_point), (Tick::from_seconds(2.0), Tick::from_seconds(2.0)));
    // cropped to a region: the solid stays where it was in the picture
    let before = s.project().comp(main).unwrap().layer(solid).unwrap().props.prop("transform/position").unwrap().value.clone();
    run(&mut s, "comp.crop", r#"{"rect": [340, 860, 400, 200]}"#);
    let c = s.project().comp(main).unwrap();
    assert_eq!((c.width, c.height), (400, 200));
    assert_eq!(c.layer(solid).unwrap().props.prop("transform/position").unwrap().value, Value::Vec3([before.as_vec3()[0] - 340.0, before.as_vec3()[1] - 860.0, 0.0]));
    // to a layer's bounds
    run(&mut s, "edit.undo", "");
    let fit = run(&mut s, "comp.crop", &format!(r#"{{"layer": {solid}, "time": 0.5}}"#));
    assert_eq!((fit.get("width").and_then(Json::as_f64), fit.get("height").and_then(Json::as_f64)), (Some(400.0), Some(200.0)));
}

#[test]
fn paths_in_expressions() {
    let (mut s, _, l) = started();
    run(&mut s, "mask.add", &format!(r#"{{"layer": {l}, "rect": [0, 0, 200, 100]}}"#));
    let p = |s: &mut Session, layer: u64, path: &str, t: f64| run(s, "prop.evaluate", &format!(r#"{{"layer": {layer}, "path": "{path}", "times": [{t}]}}"#)).get("values").unwrap().at(0).unwrap().get("value").unwrap().clone();
    // a path made by an expression
    run(&mut s, "prop.expression", &format!(r#"{{"layer": {l}, "path": "masks/#1/path", "text": "createPath([[0,0],[100,0],[100,50]], [], [], false)"}}"#));
    let made = p(&mut s, l, "masks/#1/path", 0.0);
    assert_eq!(made.get("points").map(|v| v.to_json()), Some("[[0,0],[100,0],[100,50]]".into()), "{}", made.to_json());
    assert!(!made.get("closed").and_then(Json::as_bool).unwrap_or(false), "{}", made.to_json());
    // read from another layer: its points, and a point along it (by length)
    let n = id(&run(&mut s, "layer.add", r#"{"type": "null"}"#), "layer");
    run(&mut s, "prop.expression", &format!(r#"{{"layer": {n}, "path": "transform/rotation", "text": "thisComp.layer(\"Solid 1\").mask(\"Mask 1\").path.points().length"}}"#));
    assert_eq!(p(&mut s, n, "transform/rotation", 0.0).as_f64(), Some(3.0));
    run(&mut s, "prop.expression", &format!(r#"{{"layer": {n}, "path": "transform/position", "text": "thisComp.layer(\"Solid 1\").mask(\"Mask 1\").path.pointOnPath(0.75)"}}"#));
    let at = p(&mut s, n, "transform/position", 0.0);
    assert_eq!(at.to_json(), "[100,12.5,0]", "three quarters of 150 along: 12.5 down the second side");
    run(&mut s, "prop.expression", &format!(r#"{{"layer": {n}, "path": "transform/rotation", "text": "var d = thisComp.layer(\"Solid 1\").mask(\"Mask 1\").path.tangentOnPath(0.25); radiansToDegrees(Math.atan2(d[1], d[0]))"}}"#));
    assert!(p(&mut s, n, "transform/rotation", 0.0).as_f64().unwrap().abs() < 1e-6, "along the first side, going right");
}

#[test]
fn path_commands() {
    let (mut s, _, l) = started();
    run(&mut s, "mask.add", &format!(r#"{{"layer": {l}, "rect": [0, 0, 200, 100]}}"#));
    let mp = format!(r#""layer": {l}, "path": "masks/#1/path""#);
    let shape = |s: &Session| match s.project().layer(l).unwrap().props.prop("masks/#1/path").unwrap().value.clone() {
        Value::Path(p) => p,
        _ => unreachable!(),
    };
    // a vertex in the middle of the top side, then taken out again
    run(&mut s, "path.vertex", &format!(r#"{{{mp}, "op": "insert", "index": 0}}"#));
    assert_eq!(shape(&s).points.len(), 5);
    assert_eq!(shape(&s).points[1], [100.0, 0.0]);
    run(&mut s, "path.vertex", &format!(r#"{{{mp}, "op": "set", "index": 1, "point": [100, -30]}}"#));
    assert_eq!(shape(&s).points[1], [100.0, -30.0]);
    run(&mut s, "path.vertex", &format!(r#"{{{mp}, "op": "smooth", "index": 1}}"#));
    assert!(shape(&s).out_tangents[1][0] > 0.0, "smooth: a tangent along the side");
    run(&mut s, "path.vertex", &format!(r#"{{{mp}, "op": "remove", "index": 1}}"#));
    run(&mut s, "path.vertex", &format!(r#"{{{mp}, "op": "first", "index": 2}}"#));
    assert_eq!(shape(&s).points[0], [200.0, 100.0]);
    run(&mut s, "path.vertex", &format!(r#"{{{mp}, "op": "open"}}"#));
    assert!(!shape(&s).closed);
    run(&mut s, "path.vertex", &format!(r#"{{{mp}, "op": "close"}}"#));
    // smart interpolation between a rectangle and a wider rectangle with more vertices
    let rect = r#"{"closed": true, "points": [[0,0],[200,0],[200,100],[0,100]]}"#;
    let wide = r#"{"closed": true, "points": [[-50,0],[50,-20],[250,0],[250,100],[100,120],[-50,100]]}"#;
    run(&mut s, "prop.keys", &format!(r#"{{{mp}, "keys": [{{"t": 0, "v": {rect}}}, {{"t": 1, "v": {wide}}}]}}"#));
    let mi = run(&mut s, "mask.interpolate", &format!(r#"{{{mp}, "rate": 10, "spacing": 0}}"#));
    assert_eq!(mi.get("keys").and_then(Json::as_f64), Some(9.0));
    let keys = s.project().layer(l).unwrap().props.prop("masks/#1/path").unwrap().keys.clone();
    assert_eq!(keys.len(), 11);
    assert!(keys.iter().all(|k| matches!(&k.value, Value::Path(p) if p.points.len() == 6)), "every key as many vertices");
    // nulls on the path's points, following them
    run(&mut s, "prop.keys", &format!(r#"{{{mp}, "keys": []}}"#));
    run(&mut s, "prop.set", &format!(r#"{{{mp}, "value": {rect}}}"#));
    let made = run(&mut s, "path.nulls", &format!(r#"{{{mp}, "mode": "nullsFollowPoints"}}"#));
    let first = made.get("nulls").unwrap().at(0).unwrap().as_u64().unwrap();
    let at = |s: &mut Session, layer: u64, path: &str| run(s, "prop.evaluate", &format!(r#"{{"layer": {layer}, "path": "{path}", "times": [0]}}"#)).get("values").unwrap().at(0).unwrap().get("value").unwrap().clone();
    // the solid is centred (540, 960) with its anchor at its middle (200, 100): vertex 0 is at (340, 860)
    assert_eq!(at(&mut s, first, "transform/position").to_json(), "[340,860,0]");
    // the path following nulls: move one, its vertex goes with it
    run(&mut s, "edit.undo", "");
    let made = run(&mut s, "path.nulls", &format!(r#"{{{mp}, "mode": "pointsFollowNulls"}}"#));
    let second = made.get("nulls").unwrap().at(1).unwrap().as_u64().unwrap();
    run(&mut s, "prop.set", &format!(r#"{{"layer": {second}, "path": "transform/position", "value": [600, 860, 0]}}"#));
    let path = at(&mut s, l, "masks/#1/path");
    assert_eq!(path.get("points").unwrap().at(1).unwrap().to_json(), "[260,0]", "{}", path.to_json());
    // a null travelling along it
    run(&mut s, "edit.undo", "");
    run(&mut s, "edit.undo", "");
    let tr = run(&mut s, "path.nulls", &format!(r#"{{{mp}, "mode": "trace"}}"#));
    let t = tr.get("nulls").unwrap().at(0).unwrap().as_u64().unwrap();
    let half = run(&mut s, "prop.evaluate", &format!(r#"{{"layer": {t}, "path": "transform/position", "times": [2.5]}}"#)).get("values").unwrap().at(0).unwrap().get("value").unwrap().to_json();
    assert_eq!(half, "[540,960,0]", "half way round the rectangle: the opposite corner");
}

#[test]
fn shapes_converted_grouped_and_traced() {
    let (mut s, _, solid) = started();
    let sl = id(&run(&mut s, "layer.add", r#"{"type": "shape"}"#), "layer");
    run(&mut s, "shape.add", r##"{"layer": "Shape Layer 1", "type": "rect", "props": {"size": [300, 120], "position": [0, 0], "roundness": 20}}"##);
    run(&mut s, "shape.add", r##"{"layer": "Shape Layer 1", "type": "fill", "props": {"color": "#ff0000"}}"##);
    let before = picture(&s);
    let conv = run(&mut s, "shape.toPath", r#"{"layer": "Shape Layer 1", "item": "Rectangle 1"}"#);
    assert_eq!(conv.get("points").and_then(Json::as_f64), Some(8.0), "four corners, rounded: eight vertices");
    let after = picture(&s);
    let worst = before.image.px.iter().zip(&after.image.px).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max);
    assert!(worst < 0.02, "the same outline drawn: {worst}");
    run(&mut s, "shape.group", r#"{"layer": "Shape Layer 1", "items": ["Path 1", "Fill 1"]}"#);
    let contents = |s: &Session| s.project().layer(sl).unwrap().props.group("contents").unwrap().children.iter().map(|c| c.name().to_string()).collect::<Vec<_>>();
    assert_eq!(contents(&s), ["Group 1"]);
    run(&mut s, "shape.ungroup", r#"{"layer": "Shape Layer 1", "item": "Group 1"}"#);
    assert_eq!(contents(&s), ["Path 1", "Fill 1"]);
    // the solid's own outline traced onto it
    let traced = run(&mut s, "layer.autoTrace", &format!(r#"{{"layer": {solid}, "time": 0.5, "cornerRoundness": 0}}"#));
    assert_eq!(traced.get("masks").and_then(Json::as_array).map(Vec::len), Some(1));
    let mask = s.project().layer(solid).unwrap().props.group("masks").unwrap().children.last().unwrap().as_group().unwrap().clone();
    let Value::Path(p) = mask.prop("path").unwrap().value.clone() else { unreachable!() };
    let (xs, ys): (Vec<f64>, Vec<f64>) = p.points.iter().map(|q| (q[0], q[1])).unzip();
    let (x0, x1) = (xs.iter().copied().fold(f64::MAX, f64::min), xs.iter().copied().fold(f64::MIN, f64::max));
    let (y0, y1) = (ys.iter().copied().fold(f64::MAX, f64::min), ys.iter().copied().fold(f64::MIN, f64::max));
    assert!(x0.abs() < 1.5 && (x1 - 400.0).abs() < 1.5 && y0.abs() < 1.5 && (y1 - 200.0).abs() < 1.5, "the solid's edges, in its own pixels: {x0} {x1} {y0} {y1}");
}

#[test]
fn layers_arranged_and_effects_handled() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r#"{"name": "Board", "width": 400, "height": 200, "fps": 10, "duration": 2}"#);
    let ids: Vec<u64> = [(40, 30, 60), (20, 20, 200), (60, 40, 330)]
        .iter()
        .map(|(w, h, x)| {
            let l = id(&run(&mut s, "layer.add", &format!(r##"{{"type": "solid", "color": "#ffffff", "width": {w}, "height": {h}}}"##)), "layer");
            run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "transform/position", "value": [{x}, {}, 0]}}"#, 50 + w));
            l
        })
        .collect();
    let edges = |s: &Session| -> Vec<[f64; 4]> {
        ids.iter()
            .map(|id| {
                let l = s.project().layer(*id).unwrap();
                let p = l.props.prop("transform/position").unwrap().value.as_vec3();
                let veymelo_project::Source::Solid { width, height, .. } = l.source else { unreachable!() };
                [p[0] - width as f64 / 2.0, p[1] - height as f64 / 2.0, p[0] + width as f64 / 2.0, p[1] + height as f64 / 2.0]
            })
            .collect()
    };
    let list = format!("[{}, {}, {}]", ids[0], ids[1], ids[2]);
    run(&mut s, "layer.align", &format!(r#"{{"layers": {list}, "edge": "top"}}"#));
    assert!(edges(&s).iter().all(|e| e[1].abs() < 1e-9), "{:?}", edges(&s));
    run(&mut s, "layer.align", &format!(r#"{{"layers": {list}, "edge": "right", "to": "layers"}}"#));
    assert!(edges(&s).iter().all(|e| (e[2] - 360.0).abs() < 1e-9), "{:?}", edges(&s));
    run(&mut s, "edit.undo", "");
    run(&mut s, "layer.distribute", &format!(r#"{{"layers": {list}, "by": "hgaps"}}"#));
    let e = edges(&s);
    let mut sorted = e.clone();
    sorted.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert!(((sorted[1][0] - sorted[0][2]) - (sorted[2][0] - sorted[1][2])).abs() < 1e-6, "equal gaps: {sorted:?}");
    run(&mut s, "layer.distribute", &format!(r#"{{"layers": {list}, "by": "hcenter"}}"#));
    let e = edges(&s);
    let mut c: Vec<f64> = e.iter().map(|b| (b[0] + b[2]) / 2.0).collect();
    c.sort_by(f64::total_cmp);
    assert!((c[1] - (c[0] + c[2]) / 2.0).abs() < 1e-6);
    // nudged with its position animated: a key where it was moved
    run(&mut s, "prop.keys", &format!(r#"{{"layer": {}, "path": "transform/position", "keys": [{{"t": 0, "v": [100, 100, 0]}}, {{"t": 2, "v": [300, 100, 0]}}]}}"#, ids[0]));
    run(&mut s, "layer.nudge", &format!(r#"{{"layers": [{}], "y": 10, "time": 1}}"#, ids[0]));
    let keys = s.project().layer(ids[0]).unwrap().props.prop("transform/position").unwrap().keys.clone();
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[1].value, Value::Vec3([200.0, 110.0, 0.0]));
    // effects: copied here and to another layer, reset, switched off, all taken off
    let (a, b) = (ids[0], ids[1]);
    run(&mut s, "effect.add", &format!(r#"{{"layer": {a}, "type": "gaussianBlur", "props": {{"blurriness": 12}}}}"#));
    let d = run(&mut s, "effect.duplicate", &format!(r#"{{"layer": {a}, "effect": "Gaussian Blur"}}"#));
    assert_eq!(d.get("name").and_then(Json::as_str), Some("Gaussian Blur 2"));
    run(&mut s, "effect.duplicate", &format!(r#"{{"layer": {a}, "effect": "Gaussian Blur", "toLayer": {b}}}"#));
    assert_eq!(s.project().layer(b).unwrap().props.prop("effects/Gaussian Blur/blurriness").unwrap().value, Value::Number(12.0));
    run(&mut s, "effect.reset", &format!(r#"{{"layer": {a}, "effect": "Gaussian Blur 2"}}"#));
    assert_eq!(s.project().layer(a).unwrap().props.prop("effects/Gaussian Blur 2/blurriness").unwrap().value, Value::Number(0.0));
    run(&mut s, "effect.set", &format!(r#"{{"layer": {a}, "effect": "Gaussian Blur", "enabled": false, "name": "Soft"}}"#));
    assert!(!s.project().layer(a).unwrap().props.group("effects/Soft").unwrap().enabled);
    assert!(s.project().problems().is_empty(), "{:?}", s.project().problems());
    run(&mut s, "effect.removeAll", &format!(r#"{{"layer": {a}}}"#));
    assert!(s.project().layer(a).unwrap().props.group("effects").unwrap().children.is_empty());
}

#[test]
fn animation_presets_go_on_and_come_back() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r#"{"name": "Card", "width": 400, "height": 300, "fps": 10, "duration": 4}"#);
    let solid = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 100, "height": 60}"##), "layer");
    let words = id(&run(&mut s, "layer.add", r##"{"type": "text", "text": {"text": "Hello there", "font": "Helvetica", "size": 40, "fill": "#ffffff"}}"##), "layer");
    let all = run(&mut s, "anim.presets", r#"{"folder": "/nowhere/at/all"}"#);
    let ids: Vec<&str> = all.as_array().unwrap().iter().filter_map(|p| p.get("id").and_then(Json::as_str)).collect();
    for want in ["fadeIn", "slideInLeft", "popIn", "blurIn", "typewriter", "wiggle"] {
        assert!(ids.contains(&want), "{ids:?}");
    }
    let text_ones = run(&mut s, "anim.presets", r#"{"category": "text", "folder": "/nowhere"}"#);
    assert!(text_ones.as_array().unwrap().iter().all(|p| p.get("category").and_then(Json::as_str) == Some("text")));
    let prop = |s: &Session, l: u64, path: &str| s.project().layer(l).unwrap().props.prop(path).unwrap().clone();
    // a fade in from its in point to its own opacity, eased
    run(&mut s, "prop.set", &format!(r#"{{"layer": {solid}, "path": "transform/opacity", "value": 80}}"#));
    let r = run(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{solid}], "preset": "Fade In"}}"#));
    assert_eq!(r.get("preset").and_then(Json::as_str), Some("Fade In"));
    let o = prop(&s, solid, "transform/opacity");
    assert_eq!(o.keys.len(), 2);
    assert_eq!((o.keys[0].value.clone(), o.keys[1].value.clone()), (Value::Number(0.0), Value::Number(80.0)));
    assert_eq!(o.keys[1].time, veymelo_time::Tick::from_seconds(0.5));
    assert_eq!(o.keys[0].interp_out, veymelo_keyframe::Interp::Bezier);
    // slid in from just past the left edge: its right edge at 0
    let r = run(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{solid}], "preset": "slideInLeft", "time": 1, "duration": 1}}"#));
    assert_eq!(r.get("applied").and_then(|a| a.at(0)).and_then(|a| a.get("from")).and_then(Json::as_f64), Some(1.0));
    let p = prop(&s, solid, "transform/position");
    assert_eq!(p.keys.len(), 2);
    assert_eq!(p.keys[0].value, Value::Vec3([-50.0, 150.0, 0.0]));
    assert_eq!(p.keys[1].value, Value::Vec3([200.0, 150.0, 0.0]));
    assert_eq!(p.keys[1].time, veymelo_time::Tick::from_seconds(2.0));
    assert!(!p.keys[0].spatial_auto, "a straight slide");
    assert!(s.render(None, None, &render::Options::default()).is_ok());
    // a fade out ends at its out point, stretched
    run(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{solid}], "preset": "fadeOut", "duration": 1}}"#));
    let o = prop(&s, solid, "transform/opacity");
    assert_eq!(o.keys.len(), 4);
    assert_eq!(o.keys[2].time, veymelo_time::Tick::from_seconds(3.0));
    assert_eq!(o.keys[3].value, Value::Number(0.0));
    // a blur in: an effect of its own with its keys
    run(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{solid}], "preset": "blurIn"}}"#));
    let b = prop(&s, solid, "effects/Blur In/blurriness");
    assert_eq!((b.keys[0].value.clone(), b.keys[1].value.clone()), (Value::Number(40.0), Value::Number(0.0)));
    // text presets only on text
    let no = fails(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{solid}], "preset": "typewriter"}}"#));
    assert!(no.contains("text layers"), "{no}");
    run(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{words}], "preset": "typewriter"}}"#));
    let st = prop(&s, words, "text/animators/Typewriter/selectors/Range Selector 1/start");
    assert_eq!(st.keys.len(), 2);
    assert_eq!(prop(&s, words, "text/animators/Typewriter/opacity").value, Value::Number(0.0));
    // loops are expressions that run
    for (preset, path) in [("pulse", "transform/scale"), ("float", "transform/position"), ("spin", "transform/rotation"), ("blink", "transform/opacity"), ("wiggle", "transform/position")] {
        let l = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#00ff00", "width": 20, "height": 20}"##), "layer");
        run(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{l}], "preset": "{preset}"}}"#));
        for t in [0.0, 0.3, 1.1] {
            let g = run(&mut s, "prop.get", &format!(r#"{{"layer": {l}, "path": "{path}", "time": {t}}}"#));
            assert!(g.get("error").is_none(), "{preset}: {}", g.to_json());
        }
        let g = run(&mut s, "prop.get", &format!(r#"{{"layer": {l}, "path": "{path}", "time": 0.25}}"#));
        let v = g.get("value").unwrap();
        match preset {
            "pulse" => assert!((v.at(0).and_then(Json::as_f64).unwrap() - 106.0).abs() < 1e-6, "{}", g.to_json()),
            "spin" => assert!((v.as_f64().unwrap() - 22.5).abs() < 1e-6, "{}", g.to_json()),
            "blink" => assert_eq!(v.as_f64(), Some(0.0), "{}", g.to_json()),
            "float" => assert!((v.at(1).and_then(Json::as_f64).unwrap() - (150.0 + 12.0 * 0.5f64.sin())).abs() < 1e-6, "{}", g.to_json()),
            _ => {}
        }
    }
    // every preset that comes with the program goes on a text layer
    for p in all.as_array().unwrap() {
        let pid = p.get("id").and_then(Json::as_str).unwrap();
        let l = id(&run(&mut s, "layer.add", r##"{"type": "text", "text": {"text": "Go", "font": "Helvetica", "size": 60, "fill": "#ffffff"}}"##), "layer");
        run(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{l}], "preset": "{pid}"}}"#));
        assert!(s.render(None, Some(veymelo_time::Tick::from_seconds(0.5)), &render::Options::default()).is_ok(), "{pid}");
    }
    assert!(s.project().problems().is_empty(), "{:?}", s.project().problems());
    // kept as a file, then put on another layer at another time
    let dir = std::env::temp_dir().join(format!("veymelo-presets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let folder = dir.display().to_string();
    let saved = run(&mut s, "anim.savePreset", &format!(r#"{{"layer": {solid}, "name": "Come and Go", "folder": "{folder}", "category": "mine"}}"#));
    assert!((saved.get("length").and_then(Json::as_f64).unwrap() - 4.0).abs() < 1e-9, "{}", saved.to_json());
    let changes: Vec<&str> = saved.get("changes").and_then(Json::as_array).unwrap().iter().filter_map(Json::as_str).collect();
    assert!(changes.contains(&"transform/opacity") && changes.contains(&"transform/position") && changes.contains(&"effect gaussianBlur"), "{changes:?}");
    let listed = run(&mut s, "anim.presets", &format!(r#"{{"category": "mine", "folder": "{folder}"}}"#));
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let other = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ff00ff", "width": 100, "height": 60}"##), "layer");
    run(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{other}], "preset": "come and go", "folder": "{folder}", "time": 0, "duration": 2}}"#));
    let (a, b) = (prop(&s, solid, "transform/position"), prop(&s, other, "transform/position"));
    assert_eq!(b.keys.len(), a.keys.len());
    assert_eq!(b.keys[0].value, a.keys[0].value);
    // halved: the slide that went from 1 s to 2 s goes from 0.5 s to 1 s
    assert_eq!((b.keys[0].time, b.keys[1].time), (veymelo_time::Tick::from_seconds(0.5), veymelo_time::Tick::from_seconds(1.0)));
    assert_eq!(prop(&s, other, "effects/Blur In/blurriness").keys.len(), 2);
    // a layer moving in separate dimensions takes a slide in X and Y
    let sep = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#0000ff", "width": 100, "height": 60}"##), "layer");
    run(&mut s, "layer.set", &format!(r#"{{"layer": {sep}, "separatePosition": true}}"#));
    run(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{sep}], "preset": "slideInTop"}}"#));
    assert_eq!(prop(&s, sep, "transform/positionY").keys[0].value, Value::Number(-30.0));
    assert_eq!(prop(&s, sep, "transform/positionX").keys[0].value, Value::Number(200.0));
    let no = fails(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{sep}], "preset": "nope"}}"#));
    assert!(no.contains("no preset"), "{no}");
    // a preset given whole, its points and lengths measured on the layer (100 × 60)
    let inline = r#"{"name": "Here", "effects": [{"type": "radialWipe", "props": {"wipeCenter": {"ofLayer": [0.25, 0.75]}, "feather": {"ofLayer": 0.1}}}]}"#;
    run(&mut s, "anim.applyPreset", &format!(r#"{{"layers": [{sep}], "preset": {inline}}}"#));
    assert_eq!(prop(&s, sep, "effects/Radial Wipe/wipeCenter").value, Value::Vec2([25.0, 45.0]));
    let feather = prop(&s, sep, "effects/Radial Wipe/feather").value.as_f64();
    assert!((feather - 0.1 * 100f64.hypot(60.0)).abs() < 1e-9, "{feather}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pre_rendered_and_proxied() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r##"{"name": "Slow", "width": 64, "height": 36, "fps": 10, "duration": 2, "background": "#000000"}"##);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ff8000", "width": 12, "height": 12}"##);
    run(&mut s, "prop.keys", r##"{"layer": "#1", "path": "transform/position", "keys": [{"t": 0, "v": [6, 18, 0]}, {"t": 1.9, "v": [58, 18, 0]}]}"##);
    run(&mut s, "comp.new", r##"{"name": "Main", "width": 64, "height": 36, "fps": 10, "duration": 2, "background": "#203040"}"##);
    run(&mut s, "layer.add", r#"{"type": "comp", "item": "Slow"}"#);
    let picture = |s: &Session, t: f64, proxies: veymelo_project::queue::ProxyUse| s.render(Some(&Json::from("Main")), Some(Tick::from_seconds(t)), &render::Options { proxies, ..render::Options::default() }).unwrap().image;
    let worst = |a: &veymelo_raster::Image, b: &veymelo_raster::Image| a.px.iter().zip(&b.px).map(|(p, q)| (0..3).map(|k| (p[k] - q[k]).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max);
    let before: Vec<_> = [0.0, 0.7, 1.5].iter().map(|t| picture(&s, *t, veymelo_project::queue::ProxyUse::None)).collect();
    let dir = std::env::temp_dir().join(format!("veymelo-prerender-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // a proxy at half size: drawn while on, fitted to the composition's size
    let half = dir.join("slow-proxy.mov").to_string_lossy().into_owned();
    let made = run(&mut s, "project.proxy.create", &format!(r#"{{"item": "Slow", "path": "{half}"}}"#));
    assert_eq!(made.get("proxy").and_then(|p| p.get("size")).map(Json::to_json), Some("[32,18]".into()), "{}", made.to_json());
    let slow = s.project().items.iter().find(|i| i.name == "Slow").unwrap().id;
    assert!(s.project().item(slow).unwrap().proxy.as_ref().unwrap().on);
    for (k, t) in [0.0, 0.7, 1.5].iter().enumerate() {
        let shown = picture(&s, *t, veymelo_project::queue::ProxyUse::Current);
        let d = worst(&shown, &before[k]);
        assert!(d > 0.0 && d < 0.75, "the proxy at {t} s: softer, in the same place ({d})");
        assert_eq!(worst(&picture(&s, *t, veymelo_project::queue::ProxyUse::None), &before[k]), 0.0);
    }
    run(&mut s, "project.proxy.set", r#"{"item": "Slow", "on": false}"#);
    assert_eq!(worst(&picture(&s, 0.7, veymelo_project::queue::ProxyUse::Current), &before[1]), 0.0, "off: itself");
    // saved beside the project: written relative, read back whole
    let file = dir.join("p.veymelo");
    run(&mut s, "project.save", &format!(r#"{{"path": "{}"}}"#, file.display()));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"path\": \"slow-proxy.mov\"") || text.contains("\"path\":\"slow-proxy.mov\""), "{text}");
    let back = Session::open(&file).unwrap();
    assert_eq!(back.project().item(slow).unwrap().proxy.as_ref().unwrap().footage.path, half);
    run(&mut s, "project.proxy.set", r#"{"item": "Slow", "path": null}"#);
    assert!(s.project().item(slow).unwrap().proxy.is_none());
    // pre-rendered: the file takes the composition's place and draws the same
    let mov = dir.join("slow.mov").to_string_lossy().into_owned();
    let r = run(&mut s, "comp.preRender", &format!(r#"{{"comp": "Slow", "path": "{mov}"}}"#));
    assert_eq!(r.get("replaced").and_then(Json::as_f64), Some(1.0), "{}", r.to_json());
    let main = s.project().items.iter().find(|i| i.name == "Main").unwrap().id;
    let footage = id(&r, "footage");
    assert_eq!(s.project().comp(main).unwrap().layers[0].source, veymelo_project::Source::Footage(footage));
    for (k, t) in [0.0, 0.7, 1.5].iter().enumerate() {
        let d = worst(&picture(&s, *t, veymelo_project::queue::ProxyUse::None), &before[k]);
        assert!(d < 8.0 / 255.0, "the pre-render at {t} s differs by {d}");
    }
    run(&mut s, "edit.undo", "");
    // only the work area: the file lines up with where it came from
    run(&mut s, "comp.set", r#"{"comp": "Slow", "workArea": [0.5, 1.0]}"#);
    let part = dir.join("part.mov").to_string_lossy().into_owned();
    run(&mut s, "comp.preRender", &format!(r#"{{"comp": "Slow", "path": "{part}", "span": "workArea"}}"#));
    let l = &s.project().comp(main).unwrap().layers[0];
    assert_eq!(l.start, Tick::from_seconds(0.5));
    let d = worst(&picture(&s, 0.7, veymelo_project::queue::ProxyUse::None), &before[1]);
    assert!(d < 8.0 / 255.0, "the work area's pre-render at 0.7 s differs by {d}");
    run(&mut s, "edit.undo", "");
    // through the queue, as its proxy
    let q = run(&mut s, "queue.add", &format!(r#"{{"comp": "Slow", "path": "{}", "format": "prores", "output": {{"prores": "4444"}}, "settings": {{"span": "comp", "resolution": 0.5}}, "after": "setProxy"}}"#, dir.join("queued.mov").display()));
    let done = run(&mut s, "queue.render", &format!(r#"{{"item": {}}}"#, id(&q, "item")));
    assert!(done.get("made").and_then(|m| m.at(0)).and_then(|m| m.get("proxy")).is_some(), "{}", done.to_json());
    assert!(s.project().item(slow).unwrap().proxy.is_some());
    let stored = s.project().queue[0].to_json();
    assert_eq!(stored.get("after").and_then(Json::as_str), Some("setProxy"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scene_edits_found_and_cut() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r#"{"name": "Shots", "width": 64, "height": 36, "fps": 10, "duration": 3}"#);
    // three shots a second each: a colour with something moving across it
    for (k, (bg, fg)) in [("#1030a0", "#ffffff"), ("#e08020", "#201008"), ("#30a050", "#f0e040")].iter().enumerate() {
        let (from, to) = (k as f64, k as f64 + 1.0);
        let b = id(&run(&mut s, "layer.add", &format!(r##"{{"type": "solid", "color": "{bg}", "in": {from}, "out": {to}}}"##)), "layer");
        let f = id(&run(&mut s, "layer.add", &format!(r##"{{"type": "solid", "color": "{fg}", "width": 10, "height": 10, "in": {from}, "out": {to}}}"##)), "layer");
        run(&mut s, "prop.keys", &format!(r#"{{"layer": {f}, "path": "transform/position", "keys": [{{"t": {from}, "v": [8, 18, 0]}}, {{"t": {}, "v": [56, 18, 0]}}]}}"#, to - 0.1));
        let _ = b;
    }
    let dir = std::env::temp_dir().join(format!("veymelo-scenes-{}", std::process::id()));
    let mov = dir.join("shots.mov").to_string_lossy().into_owned();
    run(&mut s, "render.export", &format!(r#"{{"path": "{mov}", "format": "prores", "settings": {{"span": "comp"}}}}"#));
    let item = id(&run(&mut s, "footage.import", &format!(r#"{{"path": "{mov}"}}"#)), "item");
    run(&mut s, "comp.new", r#"{"name": "Edit", "width": 64, "height": 36, "fps": 10, "duration": 3}"#);
    let l = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {item}}}"#)), "layer");
    let r = run(&mut s, "layer.sceneEdits", &format!(r#"{{"layer": {l}}}"#));
    let times: Vec<f64> = r.get("cuts").and_then(Json::as_array).unwrap().iter().filter_map(|c| c.get("time").and_then(Json::as_f64)).collect();
    assert_eq!(times, vec![1.0, 2.0], "{}", r.to_json());
    let marks: Vec<f64> = s.project().layer(l).unwrap().markers.iter().map(|m| m.time.seconds()).collect();
    assert_eq!(marks, vec![1.0, 2.0]);
    run(&mut s, "edit.undo", "");
    // a layer a shot, the later ones above
    let r = run(&mut s, "layer.sceneEdits", &format!(r#"{{"layer": {l}, "make": "split"}}"#));
    let pieces: Vec<u64> = r.get("layers").and_then(Json::as_array).unwrap().iter().filter_map(Json::as_u64).collect();
    assert_eq!(pieces.len(), 3);
    assert_eq!(pieces[0], l);
    let spans: Vec<(f64, f64)> = pieces.iter().map(|p| {
        let x = s.project().layer(*p).unwrap();
        (x.in_point.seconds(), x.out_point.seconds())
    }).collect();
    assert_eq!(spans, vec![(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)]);
    let edit = s.project().items.iter().find(|i| i.name == "Edit").unwrap().id;
    let order: Vec<u64> = s.project().comp(edit).unwrap().layers.iter().map(|x| x.id).collect();
    assert_eq!(order, vec![pieces[2], pieces[1], pieces[0]]);
    run(&mut s, "edit.undo", "");
    // each shot in a composition of its own, trimmed to it
    let r = run(&mut s, "layer.sceneEdits", &format!(r#"{{"layer": {l}, "make": "splitPrecompose"}}"#));
    assert_eq!(r.get("comps").and_then(Json::as_array).map(Vec::len), Some(3));
    let nests: Vec<u64> = r.get("layers").and_then(Json::as_array).unwrap().iter().filter_map(Json::as_u64).collect();
    let second = s.project().layer(nests[1]).unwrap();
    assert!(matches!(second.source, veymelo_project::Source::Comp(_)));
    assert_eq!((second.in_point.seconds(), second.out_point.seconds()), (1.0, 2.0));
    assert!(s.project().items.iter().any(|i| i.name == "shots.mov shot 2"), "{:?}", s.project().items.iter().map(|i| &i.name).collect::<Vec<_>>());
    assert!(s.project().problems().is_empty(), "{:?}", s.project().problems());
    // drawn as before: the shot in the middle at 1.5 s
    let picture = s.render(Some(&Json::from("Edit")), Some(Tick::from_seconds(1.5)), &render::Options::default()).unwrap().image;
    let p = picture.px[2 * 64 + 2];
    assert!(p[0] > 0.8 && p[2] < 0.25, "orange behind: {p:?}");
    let no = fails(&mut s, "layer.sceneEdits", &format!(r#"{{"layer": {}}}"#, nests[0]));
    assert!(no.contains("footage"), "{no}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_poster_frame_stands_for_the_composition() {
    let (mut s, c, _) = started();
    run(&mut s, "comp.set", r#"{"posterTime": 2.5}"#);
    assert_eq!(run(&mut s, "comp.info", "").get("posterTime").and_then(Json::as_f64), Some(2.5));
    let text = s.project().to_text();
    assert!(text.contains("\"posterTime\": 2.5"), "{text}");
    let back = veymelo_project::Project::from_text(&text, &s.catalog).unwrap();
    assert_eq!(back.comp(c).unwrap().poster, Tick::from_seconds(2.5));
    assert!(fails(&mut s, "comp.set", r#"{"posterTime": 9}"#).contains("posterTime"));
    // a composition cut shorter than its poster frame starts it again
    run(&mut s, "comp.set", r#"{"duration": 2}"#);
    assert_eq!(s.project().comp(c).unwrap().poster, Tick::ZERO);
}

#[test]
fn essential_properties_set_per_layer() {
    let mut s = Session::new();
    let dir = std::env::temp_dir().join(format!("veymelo-essential-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // two pictures to swap between
    run(&mut s, "comp.new", r##"{"name": "Pics", "width": 200, "height": 100, "fps": 10, "duration": 1, "background": "#00ff00"}"##);
    let green = dir.join("green.png").to_string_lossy().into_owned();
    run(&mut s, "render.frame", &format!(r#"{{"path": "{green}"}}"#));
    run(&mut s, "comp.set", r##"{"background": "#ffff00"}"##);
    let yellow = dir.join("yellow.png").to_string_lossy().into_owned();
    run(&mut s, "render.frame", &format!(r#"{{"path": "{yellow}"}}"#));
    let pic_g = id(&run(&mut s, "footage.import", &format!(r#"{{"path": "{green}"}}"#)), "item");
    let pic_y = id(&run(&mut s, "footage.import", &format!(r#"{{"path": "{yellow}"}}"#)), "item");
    // the card: a filled background, a stripe, a title, a picture
    run(&mut s, "comp.new", r#"{"name": "Card", "width": 200, "height": 100, "fps": 10, "duration": 2}"#);
    let bg = id(&run(&mut s, "layer.add", r##"{"type": "solid", "name": "BG", "color": "#ffffff"}"##), "layer");
    run(&mut s, "effect.add", &format!(r##"{{"layer": {bg}, "type": "fill", "props": {{"color": "#ff0000"}}}}"##));
    let stripe = id(&run(&mut s, "layer.add", r##"{"type": "solid", "name": "Stripe", "color": "#ffffff", "width": 200, "height": 10}"##), "layer");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {stripe}, "path": "transform/position", "value": [100, 95, 0]}}"#));
    run(&mut s, "effect.add", &format!(r##"{{"layer": {stripe}, "type": "fill", "props": {{"color": "#000000"}}}}"##));
    let pic = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {pic_g}, "name": "Pic"}}"#)), "layer");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {pic}, "path": "transform/scale", "value": [20, 20, 100]}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {pic}, "path": "transform/position", "value": [170, 20, 0]}}"#));
    let title = id(&run(&mut s, "layer.add", r##"{"type": "text", "name": "Title", "text": {"text": "Hi", "font": "Helvetica", "size": 30, "fill": "#ffffff"}}"##), "layer");
    let color = run(&mut s, "essential.add", &format!(r#"{{"layer": {bg}, "path": "effects/Fill/color", "name": "Colour"}}"#));
    let c_color = id(&color, "control");
    let c_op = id(&run(&mut s, "essential.add", &format!(r#"{{"layer": {title}, "path": "transform/opacity"}}"#)), "control");
    let c_font = id(&run(&mut s, "essential.add", &format!(r#"{{"layer": {title}, "path": "text/sourceText", "as": "font", "name": "Title Font"}}"#)), "control");
    let c_media = id(&run(&mut s, "essential.add", &format!(r#"{{"layer": {pic}, "path": "transform/opacity", "name": "Pic Opacity"}}"#)), "control");
    let _ = c_media;
    let c_pic = id(&run(&mut s, "essential.addMedia", &format!(r#"{{"layer": {pic}, "name": "Picture"}}"#)), "control");
    run(&mut s, "essential.link", &format!(r#"{{"control": {c_color}, "layer": {stripe}, "path": "effects/Fill/color"}}"#));
    let card = s.project().items.iter().find(|i| i.name == "Card").unwrap().id;
    // linked: the stripe takes the colour in the card itself
    assert_eq!(s.project().comp(card).unwrap().layer(stripe).unwrap().props.prop("effects/Fill/color").unwrap().value, s.project().comp(card).unwrap().layer(bg).unwrap().props.prop("effects/Fill/color").unwrap().value);
    let listed = run(&mut s, "essential.list", r#"{"comp": "Card"}"#);
    assert_eq!(listed.get("controls").and_then(Json::as_array).map(Vec::len), Some(5), "{}", listed.to_json());
    assert!(fails(&mut s, "essential.add", &format!(r#"{{"comp": "Card", "layer": {bg}, "path": "effects/Fill/color"}}"#)).contains("already"));
    // the main composition shows the card twice
    run(&mut s, "comp.new", r##"{"name": "Main", "width": 400, "height": 100, "fps": 10, "duration": 2, "background": "#000000"}"##);
    let a = id(&run(&mut s, "layer.add", r#"{"type": "comp", "item": "Card", "name": "A"}"#), "layer");
    let b = id(&run(&mut s, "layer.add", r#"{"type": "comp", "item": "Card", "name": "B"}"#), "layer");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {a}, "path": "transform/position", "value": [100, 50, 0]}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {b}, "path": "transform/position", "value": [300, 50, 0]}}"#));
    let inst = run(&mut s, "essential.instance", &format!(r#"{{"layer": {a}}}"#));
    assert_eq!(inst.as_array().unwrap().len(), 5, "{}", inst.to_json());
    assert!(inst.as_array().unwrap().iter().all(|p| p.get("overridden").and_then(Json::as_bool) == Some(false)));
    let px = |s: &Session, x: usize, y: usize, t: f64| {
        let img = s.render(Some(&Json::from("Main")), Some(Tick::from_seconds(t)), &render::Options::default()).unwrap().image;
        img.px[y * img.width + x]
    };
    let near = |p: [f32; 4], q: [f32; 3]| (0..3).all(|k| (p[k] - q[k]).abs() < 0.02);
    assert!(near(px(&s, 20, 50, 0.0), [1.0, 0.0, 0.0]) && near(px(&s, 220, 50, 0.0), [1.0, 0.0, 0.0]));
    // A blue, through the property path; B keeps the card's red
    run(&mut s, "prop.set", &format!(r##"{{"layer": {a}, "path": "essentials/Colour", "value": "#0000ff"}}"##));
    assert!(near(px(&s, 20, 50, 0.0), [0.0, 0.0, 1.0]), "{:?}", px(&s, 20, 50, 0.0));
    assert!(near(px(&s, 20, 96, 0.0), [0.0, 0.0, 1.0]), "the linked stripe too");
    assert!(near(px(&s, 220, 50, 0.0), [1.0, 0.0, 0.0]));
    let inst = run(&mut s, "essential.instance", &format!(r#"{{"layer": {a}}}"#));
    let colour = inst.as_array().unwrap().iter().find(|p| p.get("control").and_then(Json::as_u64) == Some(c_color)).unwrap();
    assert_eq!(colour.get("overridden").and_then(Json::as_bool), Some(true));
    // keys of its own: B's colour animated, then a mirror sets the same control
    run(&mut s, "prop.keys", &format!(r##"{{"layer": {b}, "path": "essentials/@{c_color}", "keys": [{{"t": 0, "v": "#ff0000"}}, {{"t": 1, "v": "#00ff00"}}]}}"##));
    assert!(near(px(&s, 220, 50, 1.0), [0.0, 1.0, 0.0]), "{:?}", px(&s, 220, 50, 1.0));
    let mirror = id(&run(&mut s, "essential.addMirror", &format!(r#"{{"comp": "Card", "control": {c_color}, "name": "Accent"}}"#)), "control");
    run(&mut s, "essential.set", &format!(r##"{{"layer": {b}, "control": "Accent", "value": "#ff00ff"}}"##));
    assert!(near(px(&s, 220, 50, 1.0), [1.0, 0.0, 1.0]), "the mirror sets its control: {:?}", px(&s, 220, 50, 1.0));
    let _ = mirror;
    // the title's font only: bigger on A, its words still the card's
    run(&mut s, "essential.set", &format!(r#"{{"layer": {a}, "control": {c_font}, "value": {{"text": "ignored", "font": "Helvetica", "size": 60}}}}"#));
    let render_a = s.render(Some(&Json::from("Main")), Some(Tick::ZERO), &render::Options::default()).unwrap().image;
    let lit = |img: &veymelo_raster::Image, x0: usize, x1: usize| (0..100).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|(x, y)| img.px[y * img.width + x][1] > 0.9 && img.px[y * img.width + x][0] > 0.9 && img.px[y * img.width + x][2] > 0.9).count();
    assert!(lit(&render_a, 0, 200) > lit(&render_a, 200, 400) * 2, "A's title bigger: {} vs {}", lit(&render_a, 0, 200), lit(&render_a, 200, 400));
    // B's picture swapped
    run(&mut s, "essential.set", &format!(r#"{{"layer": {b}, "control": {c_pic}, "item": {pic_y}}}"#));
    assert!(near(px(&s, 370, 20, 0.0), [1.0, 1.0, 0.0]) && near(px(&s, 170, 20, 0.0), [0.0, 1.0, 0.0]), "{:?} {:?}", px(&s, 370, 20, 0.0), px(&s, 170, 20, 0.0));
    // kept through a save and back
    let file = dir.join("e.veymelo");
    run(&mut s, "project.save", &format!(r#"{{"path": "{}"}}"#, file.display()));
    let back = Session::open(&file).unwrap();
    let pick = |s: &Session| s.render(Some(&Json::from("Main")), Some(Tick::from_seconds(1.0)), &render::Options::default()).unwrap().image;
    assert_eq!(pick(&back).px, pick(&s).px, "the same after opening it again");
    // the layer goes back to the card's colour; B's opacity becomes the card's own
    run(&mut s, "essential.revert", &format!(r#"{{"layer": {a}, "control": {c_color}}}"#));
    assert!(near(px(&s, 20, 50, 0.0), [1.0, 0.0, 0.0]));
    run(&mut s, "essential.set", &format!(r#"{{"layer": {b}, "control": {c_op}, "value": 40}}"#));
    run(&mut s, "essential.push", &format!(r#"{{"layer": {b}, "control": {c_op}}}"#));
    assert_eq!(s.project().comp(card).unwrap().layer(title).unwrap().props.prop("transform/opacity").unwrap().value, Value::Number(40.0));
    let inst_a = run(&mut s, "essential.instance", &format!(r#"{{"layer": {a}}}"#));
    let op_a = inst_a.as_array().unwrap().iter().find(|p| p.get("control").and_then(Json::as_u64) == Some(c_op)).unwrap();
    assert_eq!(op_a.get("value").and_then(Json::as_f64), Some(40.0), "A follows the card's new value");
    // a duplicated layer keeps what it set
    let dup = id(&run(&mut s, "layer.duplicate", &format!(r#"{{"layer": {b}}}"#)), "layer");
    let dup_inst = run(&mut s, "essential.instance", &format!(r#"{{"layer": {dup}}}"#));
    assert!(dup_inst.as_array().unwrap().iter().any(|p| p.get("control").and_then(Json::as_u64) == Some(c_pic) && p.get("overridden").and_then(Json::as_bool) == Some(true)));
    assert!(s.project().problems().is_empty(), "{:?}", s.project().problems());
    // a template: the card and what it uses, brought in again
    let tpl = dir.join("card.json").to_string_lossy().into_owned();
    run(&mut s, "essential.setName", r#"{"comp": "Card", "name": "Lower Third"}"#);
    let ex = run(&mut s, "essential.exportTemplate", &format!(r#"{{"comp": "Card", "path": "{tpl}"}}"#));
    assert_eq!(ex.get("items").and_then(Json::as_f64), Some(2.0), "the card and the picture it shows: {}", ex.to_json());
    let info = run(&mut s, "essential.templateInfo", &format!(r#"{{"path": "{tpl}"}}"#));
    assert_eq!(info.get("name").and_then(Json::as_str), Some("Lower Third"));
    let im = run(&mut s, "essential.importTemplate", &format!(r#"{{"path": "{tpl}", "addToComp": "Main"}}"#));
    let new_layer = id(&im, "layer");
    run(&mut s, "essential.set", &format!(r##"{{"layer": {new_layer}, "control": "Colour", "value": "#00ffff"}}"##));
    assert!(s.project().problems().is_empty(), "{:?}", s.project().problems());
    // a control taken away goes from every layer
    run(&mut s, "essential.remove", &format!(r#"{{"comp": "Card", "control": {c_color}}}"#));
    let inst = run(&mut s, "essential.instance", &format!(r#"{{"layer": {b}}}"#));
    assert!(inst.as_array().unwrap().iter().all(|p| p.get("control").and_then(Json::as_u64) != Some(c_color) && p.get("name").and_then(Json::as_str) != Some("Accent")), "{}", inst.to_json());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn essential_properties_reach_the_sound() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r#"{"name": "Beep", "width": 64, "height": 64, "fps": 30, "duration": 2}"#);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#000000", "width": 8, "height": 8}"##);
    run(&mut s, "effect.add", r##"{"layer": "#1", "type": "tone", "props": {"frequency1": 1000, "frequency2": 0, "frequency3": 0, "frequency4": 0, "frequency5": 0, "level": 10}}"##);
    run(&mut s, "essential.add", r##"{"layer": "#1", "path": "effects/Tone/level", "name": "Loudness"}"##);
    run(&mut s, "comp.new", r#"{"name": "Mix", "width": 64, "height": 64, "fps": 30, "duration": 2}"#);
    let l = id(&run(&mut s, "layer.add", r#"{"type": "comp", "item": "Beep"}"#), "layer");
    let peak = |s: &mut Session| run(s, "audio.levels", r#"{"comp": "Mix", "start": 0.2, "end": 1.8}"#).get("peak").and_then(|p| p.at(0)).and_then(Json::as_f64);
    assert_eq!(peak(&mut s), Some(-20.0));
    run(&mut s, "essential.set", &format!(r#"{{"layer": {l}, "control": "Loudness", "value": 50}}"#));
    let louder = peak(&mut s).unwrap();
    assert!((louder - 20.0 * 0.5f64.log10()).abs() < 0.1, "half scale: {louder}");
}

#[test]
fn text_styled_in_ranges_and_edited() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r#"{"name": "Words", "width": 800, "height": 300, "fps": 10, "duration": 2}"#);
    let l = id(&run(&mut s, "layer.add", r##"{"type": "text", "text": {"text": "Hello world", "font": "Helvetica", "size": 40, "fill": "#ffffff"}}"##), "layer");
    let doc = |s: &Session| s.project().layer(l).unwrap().props.prop("text/sourceText").unwrap().value.doc().unwrap();
    // "world" bold and orange; the paragraph centred
    run(&mut s, "text.set", &format!(r##"{{"layer": {l}, "range": [6, 11], "weight": 700, "fill": "#ff8800", "justify": "center"}}"##));
    let d = doc(&s);
    assert_eq!(d.style_at(7).weight, 700.0);
    assert_eq!(d.style_at(1).weight, 400.0);
    assert_eq!(d.para_at(0).justify, veymelo_keyframe::Justify::Center);
    // typed inside "world": bold too; a new paragraph takes the setting
    run(&mut s, "text.insert", &format!(r#"{{"layer": {l}, "text": "XX", "at": 8}}"#));
    assert_eq!(doc(&s).text, "Hello woXXrld");
    assert_eq!(doc(&s).style_at(9).weight, 700.0);
    run(&mut s, "text.insert", &format!(r#"{{"layer": {l}, "text": "\nnext", "at": 13}}"#));
    assert_eq!(doc(&s).para_at(1).justify, veymelo_keyframe::Justify::Center);
    run(&mut s, "text.delete", &format!(r#"{{"layer": {l}, "range": [8, 10]}}"#));
    assert_eq!(doc(&s).text, "Hello world\nnext");
    // the whole of it: every run changes, and what is typed later takes it
    run(&mut s, "text.set", &format!(r#"{{"layer": {l}, "size": 60, "underline": true, "features": ["onum"], "direction": "rtl"}}"#));
    let d = doc(&s);
    assert!(d.styles().iter().all(|st| st.size == 60.0 && st.underline && st.features == vec!["onum".to_string()]));
    assert_eq!(d.para_at(1).direction, veymelo_keyframe::Direction::Rtl);
    assert!(fails(&mut s, "text.set", &format!(r#"{{"layer": {l}, "kerning": "loose"}}"#)).contains("kerning"));
    // fonts and what they have
    let fonts = run(&mut s, "text.fonts", r#"{"query": "helvet"}"#);
    assert!(fonts.get("families").and_then(Json::as_array).unwrap().iter().any(|f| f.get("family").and_then(Json::as_str) == Some("Helvetica")));
    let features = run(&mut s, "text.fontFeatures", r#"{"font": "Georgia"}"#);
    assert!(features.get("features").and_then(Json::as_array).is_some_and(|f| !f.is_empty()), "{}", features.to_json());
    // an expression selector: every other letter picked
    run(&mut s, "text.animator.add", &format!(r#"{{"layer": {l}, "props": {{"opacity": 0}}, "selector": "expression"}}"#));
    run(&mut s, "prop.expression", &format!(r#"{{"layer": {l}, "path": "text/animators/Animator 1/selectors/Expression Selector 1/amount", "text": "textIndex % 2 == 0 ? 100 : 0"}}"#));
    let r = s.render(Some(&Json::from("Words")), Some(Tick::ZERO), &render::Options::default()).unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    run(&mut s, "text.removeAllAnimators", &format!(r#"{{"layer": {l}}}"#));
    assert!(s.project().layer(l).unwrap().props.group("text/animators").unwrap().children.is_empty());
}

#[test]
fn characters_stand_in_depth() {
    let mut s = Session::new();
    run(&mut s, "comp.new", r#"{"name": "Depth", "width": 600, "height": 300, "fps": 10, "duration": 2}"#);
    run(&mut s, "layer.add", r#"{"type": "camera"}"#);
    let l = id(&run(&mut s, "layer.add", r##"{"type": "text", "text": {"text": "DEPTH", "font": "Helvetica", "weight": 700, "size": 100, "fill": "#ffffff", "justify": "center"}}"##), "layer");
    run(&mut s, "layer.set", &format!(r#"{{"layer": {l}, "switches": {{"threeD": true}}}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "transform/position", "value": [300, 190, 0]}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "text/perChar3d", "value": true}}"#));
    let ink = |s: &Session| s.render(Some(&Json::from("Depth")), Some(Tick::ZERO), &render::Options { background: false, ..render::Options::default() }).unwrap().image.px.iter().map(|p| p[3]).sum::<f32>();
    let flat = ink(&s);
    assert!(flat > 1000.0, "{flat}");
    // every character turned edge-on to the camera: next to nothing seen
    run(&mut s, "text.animator.add", &format!(r#"{{"layer": {l}, "props": {{"rotationY": 90}}}}"#));
    let edge = ink(&s);
    // (those off to the side are seen a little from the side, through the lens)
    assert!(edge < flat * 0.2, "edge-on {edge} against {flat}");
    // turned half way and pushed back: smaller still seen
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "text/animators/Animator 1/rotationY", "value": 0}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "text/animators/Animator 1/position", "value": [0, 0, 600]}}"#));
    let far = ink(&s);
    assert!(far < flat * 0.9 && far > flat * 0.2, "farther away is smaller: {far} against {flat}");
}

#[test]
fn properties_follow_go_back_bake_and_start_keys() {
    let (mut s, _, l) = started();
    let ctrl = id(&run(&mut s, "layer.add", r#"{"type": "null", "name": "Ctrl"}"#), "layer");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {ctrl}, "path": "transform/position", "value": [100, 200, 0]}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {ctrl}, "path": "transform/rotation", "value": 30}}"#));
    // a pick whip: the same kind, one number of a list, one number to a list
    let linked = run(&mut s, "prop.link", &format!(r#"{{"layer": {l}, "path": "transform/position", "toLayer": {ctrl}, "toPath": "transform/position"}}"#));
    assert_eq!(linked.get("expression").and_then(Json::as_str), Some("thisComp.layer(\"Ctrl\").transform.position"));
    assert_eq!(linked.get("value").and_then(Json::as_array).map(|v| v[0].as_f64()), Some(Some(100.0)));
    let one = run(&mut s, "prop.link", &format!(r#"{{"layer": {l}, "path": "transform/rotation", "toLayer": {ctrl}, "toPath": "transform/position", "dimension": 1}}"#));
    assert_eq!(one.get("expression").and_then(Json::as_str), Some("thisComp.layer(\"Ctrl\").transform.position[1]"));
    assert_eq!(one.get("value").and_then(Json::as_f64), Some(200.0));
    let spread = run(&mut s, "prop.link", &format!(r#"{{"layer": {l}, "path": "transform/scale", "toLayer": {ctrl}, "toPath": "transform/rotation"}}"#));
    assert!(spread.get("expression").and_then(Json::as_str).unwrap().starts_with("temp = thisComp.layer(\"Ctrl\").transform.rotation;"), "{}", spread.to_json());
    assert_eq!(spread.get("value").and_then(Json::as_array).map(|v| (v[0].as_f64(), v[1].as_f64())), Some((Some(30.0), Some(30.0))));
    // an effect's setting, a mask's, a shape inside a group
    run(&mut s, "effect.add", &format!(r#"{{"layer": {l}, "type": "gaussianBlur", "props": {{"blurriness": 12}}}}"#));
    let fx = run(&mut s, "prop.link", &format!(r#"{{"layer": {ctrl}, "path": "transform/opacity", "toLayer": {l}, "toPath": "effects/Gaussian Blur/blurriness"}}"#));
    assert_eq!(fx.get("expression").and_then(Json::as_str), Some("thisComp.layer(\"Solid 1\").effect(\"Gaussian Blur\")(\"Blurriness\")"));
    assert_eq!(fx.get("value").and_then(Json::as_f64), Some(12.0));
    run(&mut s, "mask.add", &format!(r#"{{"layer": {l}, "ellipse": [0, 0, 100, 100]}}"#));
    let mask = run(&mut s, "prop.link", &format!(r#"{{"layer": {ctrl}, "path": "transform/opacity", "toLayer": {l}, "toPath": "masks/Mask 1/opacity"}}"#));
    assert_eq!(mask.get("expression").and_then(Json::as_str), Some("thisComp.layer(\"Solid 1\").mask(\"Mask 1\").maskOpacity"));
    let shape = id(&run(&mut s, "layer.add", r#"{"type": "shape", "name": "S"}"#), "layer");
    run(&mut s, "shape.add", &format!(r#"{{"layer": {shape}, "type": "group"}}"#));
    run(&mut s, "shape.add", &format!(r#"{{"layer": {shape}, "type": "rect", "into": "Group 1", "props": {{"size": [80, 40]}}}}"#));
    let size = run(&mut s, "prop.link", &format!(r#"{{"layer": {ctrl}, "path": "transform/rotation", "toLayer": {shape}, "toPath": "contents/Group 1/contents/Rectangle 1/size"}}"#));
    assert_eq!(size.get("expression").and_then(Json::as_str), Some("thisComp.layer(\"S\").content(\"Group 1\").content(\"Rectangle 1\").size[0]"));
    assert_eq!(size.get("value").and_then(Json::as_f64), Some(80.0));
    assert!(fails(&mut s, "prop.link", &format!(r#"{{"layer": {l}, "path": "transform/opacity", "toPath": "transform/opacity"}}"#)).contains("itself"));
    assert!(fails(&mut s, "prop.link", &format!(r#"{{"layer": {l}, "path": "transform/opacity", "toLayer": {ctrl}, "toPath": "transform/position", "dimension": 3}}"#)).contains("dimension"));
    // an expression baked into keys, one a frame; the expression kept, switched off
    run(&mut s, "prop.expression", &format!(r#"{{"layer": {ctrl}, "path": "transform/orientation", "text": "[time * 30, 0, 0]"}}"#));
    run(&mut s, "prop.expression", &format!(r#"{{"layer": {ctrl}, "path": "transform/rotation", "text": "time * 30"}}"#));
    let baked = run(&mut s, "prop.bake", &format!(r#"{{"layer": {ctrl}, "path": "transform/rotation"}}"#));
    assert_eq!(baked.get("keys").and_then(Json::as_f64), Some(150.0), "five seconds at 30 a second");
    let rot = run(&mut s, "prop.get", &format!(r#"{{"layer": {ctrl}, "path": "transform/rotation", "time": 1}}"#));
    assert_eq!(rot.get("value").and_then(Json::as_f64), Some(30.0));
    assert_eq!(rot.get("expressionEnabled").and_then(Json::as_bool), Some(false));
    let every = run(&mut s, "prop.bake", &format!(r#"{{"layer": {ctrl}, "path": "transform/orientation", "from": 1, "to": 2, "step": 5}}"#));
    assert_eq!(every.get("keys").and_then(Json::as_f64), Some(7.0), "frames 30 to 60 every 5");
    assert!(fails(&mut s, "prop.bake", &format!(r#"{{"layer": {ctrl}, "path": "transform/rotation"}}"#)).contains("no expression"));
    // back as it starts: the anchor in the solid's middle, the position in the composition's
    run(&mut s, "prop.expression", &format!(r#"{{"layer": {l}, "path": "transform/scale", "text": ""}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "transform/scale", "value": [50, 50, 100]}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "transform/anchor", "value": [0, 0, 0]}}"#));
    let back = run(&mut s, "prop.reset", &format!(r#"{{"layer": {l}, "path": "transform/scale"}}"#));
    assert_eq!(back.get("reset").and_then(Json::as_array).map(Vec::len), Some(1));
    let get = |s: &mut Session, layer: u64, path: &str| run(s, "prop.get", &format!(r#"{{"layer": {layer}, "path": "{path}"}}"#)).get("value").cloned().unwrap();
    assert_eq!(get(&mut s, l, "transform/scale").to_json(), "[100,100,100]");
    run(&mut s, "prop.reset", &format!(r#"{{"layer": {l}, "path": "transform"}}"#));
    assert_eq!(get(&mut s, l, "transform/anchor").to_json(), "[200,100,0]");
    assert_eq!(get(&mut s, l, "transform/position").to_json(), "[540,960,0]", "the link went with the reset");
    run(&mut s, "prop.reset", &format!(r#"{{"layer": {l}, "path": "effects/Gaussian Blur/blurriness"}}"#));
    assert_ne!(get(&mut s, l, "effects/Gaussian Blur/blurriness").as_f64(), Some(12.0));
    // keyed: the start value as a key at the view's time
    run(&mut s, "prop.keys", &format!(r#"{{"layer": {l}, "path": "transform/opacity", "keys": [{{"t": 0, "v": 20}}, {{"t": 2, "v": 40}}]}}"#));
    run(&mut s, "view.set", r#"{"time": 1}"#);
    run(&mut s, "prop.reset", &format!(r#"{{"layer": {l}, "path": "transform/opacity"}}"#));
    let op = run(&mut s, "prop.get", &format!(r#"{{"layer": {l}, "path": "transform/opacity", "time": 1}}"#));
    assert_eq!((op.get("value").and_then(Json::as_f64), op.get("keys").and_then(Json::as_array).map(Vec::len)), (Some(100.0), Some(3)));
    // the stopwatch
    let on = run(&mut s, "prop.animate", &format!(r#"{{"layer": {l}, "path": "transform/rotation", "on": true}}"#));
    assert_eq!(on.get("keys").and_then(Json::as_array).map(Vec::len), Some(1));
    let off = run(&mut s, "prop.animate", &format!(r#"{{"layer": {l}, "path": "transform/opacity", "on": false}}"#));
    assert!(off.get("keys").is_none());
    assert_eq!(off.get("value").and_then(Json::as_f64), Some(100.0));
    // what fails, and what can be said
    run(&mut s, "prop.expression", &format!(r#"{{"layer": {ctrl}, "path": "transform/scale", "text": "thisComp.layer(\"Nope\").transform.scale"}}"#));
    let errors = run(&mut s, "expr.errors", "");
    assert_eq!(errors.get("count").and_then(Json::as_f64), Some(1.0), "{}", errors.to_json());
    let e = &errors.get("errors").and_then(Json::as_array).unwrap()[0];
    assert_eq!((e.get("name").and_then(Json::as_str), e.get("path").and_then(Json::as_str)), (Some("Ctrl"), Some("transform/scale")));
    assert!(e.get("message").and_then(Json::as_str).unwrap().contains("Nope"));
    let reference = run(&mut s, "expr.reference", "");
    assert!(reference.as_array().map_or(0, Vec::len) >= 12);
    let vector = run(&mut s, "expr.reference", r#"{"category": "vector math"}"#);
    assert_eq!(vector.as_array().map(Vec::len), Some(1));
    assert!(fails(&mut s, "expr.reference", r#"{"category": "Nothing"}"#).contains("Global"));
}

#[test]
fn groups_copied_moved_renamed_and_switched() {
    let (mut s, _, l) = started();
    run(&mut s, "mask.add", &format!(r#"{{"layer": {l}, "ellipse": [0, 0, 100, 100]}}"#));
    run(&mut s, "mask.add", &format!(r#"{{"layer": {l}, "rect": [0, 0, 50, 50]}}"#));
    let names = |s: &mut Session, list: &str| -> Vec<String> {
        let layer = s.project().comp(s.view.comp.unwrap()).unwrap().layer(l).unwrap().clone();
        layer.props.group(list).unwrap().children.iter().map(|n| n.name().to_string()).collect()
    };
    let copy = run(&mut s, "group.duplicate", &format!(r#"{{"layer": {l}, "path": "masks/Mask 1"}}"#));
    assert_eq!(copy.get("name").and_then(Json::as_str), Some("Mask 3"));
    assert_eq!(names(&mut s, "masks"), ["Mask 1", "Mask 3", "Mask 2"], "just below the one copied");
    run(&mut s, "group.rename", &format!(r#"{{"layer": {l}, "path": "masks/Mask 3", "name": "Hole"}}"#));
    assert!(fails(&mut s, "group.rename", &format!(r#"{{"layer": {l}, "path": "masks/Hole", "name": "Mask 1"}}"#)).contains("already"));
    run(&mut s, "group.enable", &format!(r#"{{"layer": {l}, "path": "masks/Hole", "on": false}}"#));
    let off = s.project().comp(s.view.comp.unwrap()).unwrap().layer(l).unwrap().props.group("masks/Hole").unwrap().enabled;
    assert!(!off);
    let moved = run(&mut s, "group.move", &format!(r#"{{"layer": {l}, "path": "masks/Hole", "to": 1}}"#));
    assert_eq!(moved.get("list").map(Json::to_json).as_deref(), Some(r#"["Hole","Mask 1","Mask 2"]"#));
    run(&mut s, "group.remove", &format!(r#"{{"layer": {l}, "path": "masks/Hole"}}"#));
    assert_eq!(names(&mut s, "masks"), ["Mask 1", "Mask 2"]);
    // only what is in a list
    assert!(fails(&mut s, "group.remove", &format!(r#"{{"layer": {l}, "path": "transform"}}"#)).contains("list"));
    assert!(fails(&mut s, "group.remove", &format!(r#"{{"layer": {l}, "path": "masks"}}"#)).contains("list"));
    // a shape item into another group's shapes; not into a list of another kind
    let shape = id(&run(&mut s, "layer.add", r#"{"type": "shape"}"#), "layer");
    for (ty, into) in [("group", ""), ("group", ""), ("rect", "Group 1"), ("fill", "Group 1")] {
        let into = if into.is_empty() { String::new() } else { format!(r#", "into": "{into}""#) };
        run(&mut s, "shape.add", &format!(r#"{{"layer": {shape}, "type": "{ty}"{into}}}"#));
    }
    let into = run(&mut s, "group.move", &format!(r#"{{"layer": {shape}, "path": "contents/Group 1/contents/Rectangle 1", "into": "contents/Group 2/contents"}}"#));
    assert_eq!(into.get("path").and_then(Json::as_str), Some("contents/Group 2/contents/Rectangle 1"));
    assert!(fails(&mut s, "group.move", &format!(r#"{{"layer": {shape}, "path": "contents/Group 1", "into": "contents/Group 1/contents"}}"#)).contains("itself"));
    assert!(fails(&mut s, "group.move", &format!(r#"{{"layer": {shape}, "path": "contents/Group 1", "into": "masks"}}"#)).contains("holds"));
    // an animator copied to another text layer
    let a = id(&run(&mut s, "layer.add", r#"{"type": "text", "text": {"text": "A"}}"#), "layer");
    let b = id(&run(&mut s, "layer.add", r#"{"type": "text", "text": {"text": "B"}}"#), "layer");
    run(&mut s, "text.animator.add", &format!(r#"{{"layer": {a}, "props": {{"opacity": 0}}}}"#));
    let there = run(&mut s, "group.duplicate", &format!(r#"{{"layer": {a}, "path": "text/animators/Animator 1", "toLayer": {b}}}"#));
    assert_eq!(there.get("path").and_then(Json::as_str), Some("text/animators/Animator 1"));
    let copied = s.project().comp(s.view.comp.unwrap()).unwrap().layer(b).unwrap().props.prop("text/animators/Animator 1/opacity").map(|p| p.value.clone());
    assert_eq!(copied.ok(), Some(Value::Number(0.0)));
}

#[test]
fn markers_edited_converted_and_from_nested_compositions() {
    let (mut s, main, l) = started();
    run(&mut s, "marker.add", r#"{"time": 1, "comment": "beat"}"#);
    let set = run(&mut s, "marker.set", r#"{"time": 1, "to": 2, "duration": 0.5, "label": "red", "frameTarget": "_blank", "cuePoint": {"name": "cue", "navigation": true, "params": [["scene", "2"]]}}"#);
    assert_eq!(set.get("time").and_then(Json::as_f64), Some(2.0));
    let listed = run(&mut s, "marker.list", "");
    let m = &listed.as_array().unwrap()[0];
    assert_eq!(m.get("comment").and_then(Json::as_str), Some("beat"), "what is not given stays");
    assert_eq!(m.get("cuePoint").map(Json::to_json).as_deref(), Some(r#"{"name":"cue","navigation":true,"params":[["scene","2"]]}"#));
    // the project keeps it
    let again = veymelo_project::Project::from_json(&s.project().to_json(), &s.catalog).unwrap();
    assert_eq!(again.comp(main).unwrap().markers, s.project().comp(main).unwrap().markers);
    run(&mut s, "marker.set", r#"{"time": 2, "cuePoint": null}"#);
    assert!(run(&mut s, "marker.list", "").as_array().unwrap()[0].get("cuePoint").is_none());
    assert!(fails(&mut s, "marker.set", r#"{"time": 3, "comment": "x"}"#).contains("markers at 2"));
    // to a layer that starts later, and back
    run(&mut s, "layer.set", &format!(r#"{{"layer": {l}, "start": 1}}"#));
    run(&mut s, "marker.convert", &format!(r#"{{"time": 2, "toLayer": {l}}}"#));
    assert!(run(&mut s, "marker.list", "").as_array().unwrap().is_empty());
    let on_layer = run(&mut s, "marker.list", &format!(r#"{{"layer": {l}}}"#));
    assert_eq!(on_layer.as_array().unwrap()[0].get("t").and_then(Json::as_f64), Some(1.0), "in the layer's own time");
    run(&mut s, "marker.convert", &format!(r#"{{"time": 2, "layer": {l}}}"#));
    assert_eq!(run(&mut s, "marker.list", "").as_array().unwrap()[0].get("time").and_then(Json::as_f64), Some(2.0));
    // the markers of a composition shown later, and stretched
    run(&mut s, "comp.new", r#"{"name": "Inner", "width": 100, "height": 100, "fps": 30, "duration": 4}"#);
    run(&mut s, "marker.add", r#"{"time": 0.5, "comment": "a"}"#);
    run(&mut s, "marker.add", r#"{"time": 1.5, "comment": "b"}"#);
    run(&mut s, "view.set", r#"{"comp": "Main"}"#);
    let nest = id(&run(&mut s, "layer.add", r#"{"type": "comp", "item": "Inner"}"#), "layer");
    run(&mut s, "layer.set", &format!(r#"{{"layer": {nest}, "start": 2}}"#));
    let nested = run(&mut s, "marker.nested", &format!(r#"{{"layer": {nest}}}"#));
    let times: Vec<f64> = nested.as_array().unwrap().iter().map(|m| m.get("time").and_then(Json::as_f64).unwrap()).collect();
    assert_eq!(times, [2.5, 3.5]);
    // twice as fast, its in point (0, its own time -2) held: its own time 0 comes at 1
    run(&mut s, "layer.stretch", &format!(r#"{{"layer": {nest}, "stretch": 50}}"#));
    let fast = run(&mut s, "marker.nested", &format!(r#"{{"layer": {nest}}}"#));
    let times: Vec<f64> = fast.as_array().unwrap().iter().map(|m| m.get("time").and_then(Json::as_f64).unwrap()).collect();
    assert_eq!(times, [1.25, 1.75]);
    let added = run(&mut s, "marker.fromSource", &format!(r#"{{"layers": [{nest}]}}"#));
    assert_eq!(added.get("added").and_then(Json::as_f64), Some(2.0));
    assert_eq!(run(&mut s, "marker.fromSource", &format!(r#"{{"layers": [{nest}]}}"#)).get("added").and_then(Json::as_f64), Some(0.0), "not doubled");
    let own = run(&mut s, "marker.list", &format!(r#"{{"layer": {nest}}}"#));
    let times: Vec<f64> = own.as_array().unwrap().iter().map(|m| m.get("time").and_then(Json::as_f64).unwrap()).collect();
    assert_eq!(times, [1.25, 1.75]);
    assert!(fails(&mut s, "marker.nested", &format!(r#"{{"layer": {l}}}"#)).contains("does not show a composition"));
}

#[test]
fn keys_copied_to_other_properties_and_layers() {
    let (mut s, _, l) = started();
    let other = id(&run(&mut s, "layer.add", r#"{"type": "null"}"#), "layer");
    run(&mut s, "prop.keys", &format!(r#"{{"layer": {l}, "path": "transform/opacity", "keys": [{{"t": 0, "v": 0}}, {{"t": 1, "v": 100, "in": "hold"}}]}}"#));
    let copied = run(&mut s, "key.copy", &format!(r#"{{"layer": {l}, "path": "transform/opacity", "toLayer": {other}, "at": 2}}"#));
    let keys = copied.get("keys").and_then(Json::as_array).unwrap();
    assert_eq!(keys.iter().map(|k| k.get("time").and_then(Json::as_f64).unwrap()).collect::<Vec<_>>(), [2.0, 3.0]);
    assert_eq!(keys[1].get("in").and_then(Json::as_str), Some("hold"), "everything about a key goes with it");
    // to another property of the same kind; one key only
    let one = run(&mut s, "key.copy", &format!(r#"{{"layer": {l}, "path": "transform/opacity", "times": [1], "toPath": "transform/rotation"}}"#));
    assert_eq!(one.get("keys").and_then(Json::as_array).map(Vec::len), Some(1));
    assert!(fails(&mut s, "key.copy", &format!(r#"{{"layer": {l}, "path": "transform/opacity", "toPath": "transform/position"}}"#)).contains("one kind"));
    assert!(fails(&mut s, "key.copy", &format!(r#"{{"layer": {l}, "path": "transform/opacity"}}"#)).contains("where they are"));
    // keys already at those moments give way
    let over = run(&mut s, "key.copy", &format!(r#"{{"layer": {l}, "path": "transform/opacity", "toLayer": {other}, "at": 3}}"#));
    assert_eq!(over.get("keys").and_then(Json::as_array).map(Vec::len), Some(3), "2, 3 (replaced) and 4");
}

#[test]
fn an_effect_colour_picked_from_what_it_sees() {
    let (mut s, _, l) = started();
    // a green fill, then a second fill: the second sees green, the first the red solid
    run(&mut s, "effect.add", &format!(r##"{{"layer": {l}, "type": "fill", "props": {{"color": "#00ff00"}}}}"##));
    run(&mut s, "effect.add", &format!(r##"{{"layer": {l}, "type": "fill", "name": "Fill 2", "props": {{"color": "#0000ff"}}}}"##));
    let second = run(&mut s, "effect.pickColor", &format!(r#"{{"layer": {l}, "effect": "Fill 2", "param": "color", "point": [200, 100], "average": true}}"#));
    assert_eq!(second.get("color").map(Json::to_json).as_deref(), Some(r##""#00ff00""##), "{}", second.to_json());
    let first = run(&mut s, "effect.pickColor", &format!(r#"{{"layer": {l}, "effect": "Fill", "param": "color", "point": [10, 10]}}"#));
    assert_eq!(first.get("color").map(Json::to_json).as_deref(), Some(r##""#ff0000""##));
    assert!(fails(&mut s, "effect.pickColor", &format!(r#"{{"layer": {l}, "effect": "Fill", "param": "color", "point": [900, 10]}}"#)).contains("nothing to pick"));
    assert!(fails(&mut s, "effect.pickColor", &format!(r#"{{"layer": {l}, "effect": "Fill", "param": "opacity", "point": [10, 10]}}"#)).contains("not a colour"));
}

/// A 200 × 200 composition on black holding a white solid as big, and the solid.
fn masked() -> (Session, u64) {
    let mut s = Session::new();
    run(&mut s, "comp.new", r##"{"name": "M", "width": 200, "height": 200, "fps": 30, "duration": 2, "background": "#000000"}"##);
    let l = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 200, "height": 200}"##), "layer");
    run(&mut s, "mask.add", &format!(r#"{{"layer": {l}, "rect": [50, 50, 100, 100]}}"#));
    (s, l)
}

fn alpha(s: &Session, x: usize, y: usize) -> f32 {
    px(&picture(s), x, y)[3]
}

#[test]
fn masks_softened_rounded_blurred_and_reshaped() {
    let (mut s, l) = masked();
    // a 20 px feather: smooth, then in a straight ramp (5.5 px out of the edge at x 50)
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "masks/Mask 1/feather", "value": [20, 20]}}"#));
    let smooth = alpha(&s, 44, 100);
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "masks/Mask 1/featherFalloff", "value": "linear"}}"#));
    let straight = alpha(&s, 44, 100);
    assert!(smooth < 0.12, "{smooth}");
    assert!((straight - 0.225).abs() < 0.05, "{straight}");
    // softening points instead: one point 20 px outward softens the whole outline
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "masks/Mask 1/feather", "value": [0, 0]}}"#));
    let added = run(&mut s, "mask.feather", &format!(r#"{{"layer": {l}, "mask": "Mask 1", "op": "add", "piece": 0, "width": 20}}"#));
    assert_eq!(added.get("points").and_then(Json::as_array).map(Vec::len), Some(1));
    let out = alpha(&s, 100, 44);
    assert!((out - 0.725).abs() < 0.06, "outward: {out}");
    run(&mut s, "mask.feather", &format!(r#"{{"layer": {l}, "mask": "Mask 1", "op": "set", "index": 0, "width": -20}}"#));
    let inward = alpha(&s, 100, 55);
    assert!((inward - 0.275).abs() < 0.06, "inward: {inward}");
    assert!(alpha(&s, 100, 48) < 0.01, "sharp outside");
    // a point put in on that piece keeps the softening where it was
    run(&mut s, "path.vertex", &format!(r#"{{"layer": {l}, "path": "masks/Mask 1/path", "op": "insert", "index": 0}}"#));
    let listed = run(&mut s, "mask.feather", &format!(r#"{{"layer": {l}, "mask": "Mask 1", "op": "list"}}"#));
    assert_eq!(listed.get("points").map(Json::to_json).as_deref(), Some(r#"[{"piece":1,"at":0,"width":-20}]"#));
    // the project keeps them
    let again = veymelo_project::Project::from_json(&s.project().to_json(), &s.catalog).unwrap();
    assert_eq!(again.layer(l).unwrap().props.prop("masks/Mask 1/path").unwrap().value, s.project().layer(l).unwrap().props.prop("masks/Mask 1/path").unwrap().value);
    run(&mut s, "mask.feather", &format!(r#"{{"layer": {l}, "mask": "Mask 1", "op": "remove", "index": 0}}"#));
    assert!(fails(&mut s, "mask.feather", &format!(r#"{{"layer": {l}, "mask": "Mask 1", "op": "remove", "index": 0}}"#)).contains("no softening points"));
    assert!(fails(&mut s, "mask.feather", &format!(r#"{{"layer": {l}, "mask": "Mask 1", "op": "add", "piece": 9, "width": 5}}"#)).contains("no piece 9"));
    // RotoBezier: curves through the square's points, its sides bowing out
    run(&mut s, "mask.reset", &format!(r#"{{"layer": {l}, "mask": "Mask 1"}}"#));
    run(&mut s, "mask.shape", &format!(r#"{{"layer": {l}, "mask": "Mask 1", "rect": [50, 50, 100, 100]}}"#));
    assert!(alpha(&s, 100, 42) < 0.01);
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "masks/Mask 1/rotoBezier", "value": true}}"#));
    assert!(alpha(&s, 100, 42) > 0.99, "the top bows up to about 37.5");
    assert!(alpha(&s, 100, 35) < 0.01);
    // free transform: half as big about its middle
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "masks/Mask 1/rotoBezier", "value": false}}"#));
    let moved = run(&mut s, "path.transform", &format!(r#"{{"layer": {l}, "path": "masks/Mask 1/path", "scale": 50, "offset": [10, 0]}}"#));
    assert_eq!(moved.get("points").map(Json::to_json).as_deref(), Some("[[85,75],[135,75],[135,125],[85,125]]"));
    assert!(alpha(&s, 80, 100) < 0.01 && alpha(&s, 130, 100) > 0.99);
    // the mask as it starts: as big as the layer
    run(&mut s, "mask.reset", &format!(r#"{{"layer": {l}, "mask": "Mask 1"}}"#));
    assert!(alpha(&s, 2, 2) > 0.99);
    // moving fast with motion blur: a soft edge where it would be sharp
    // its right side from -50 to 250 in a second: at 100 half way, 10 px a frame
    run(&mut s, "prop.keys", &format!(r#"{{"layer": {l}, "path": "masks/Mask 1/path", "keys": [{{"t": 0, "v": {{"points": [[-100, 50], [-50, 50], [-50, 150], [-100, 150]], "closed": true}}}}, {{"t": 1, "v": {{"points": [[-100, 50], [250, 50], [250, 150], [-100, 150]], "closed": true}}}}]}}"#));
    run(&mut s, "view.set", r#"{"time": 0.5}"#);
    let edge = |s: &Session| (90..110).map(|x| alpha(s, x, 100)).filter(|a| *a > 0.05 && *a < 0.95).count();
    assert!(edge(&s) <= 1, "sharp without the switch");
    run(&mut s, "layer.set", &format!(r#"{{"layer": {l}, "switches": {{"motionBlur": true}}}}"#));
    assert!(edge(&s) >= 3, "blurred over the shutter: {}", edge(&s));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "masks/Mask 1/motionBlur", "value": "off"}}"#));
    assert!(edge(&s) <= 1, "off for this mask");
}

#[test]
fn layers_fitted_centred_flipped_and_their_anchors_centred() {
    let (mut s, _, l) = started();
    let t = id(&run(&mut s, "layer.add", r#"{"type": "text", "text": {"text": "Hello", "font": "Helvetica", "size": 100}}"#), "layer");
    let get = |s: &mut Session, layer: u64, path: &str| run(s, "prop.get", &format!(r#"{{"layer": {layer}, "path": "{path}"}}"#)).get("value").cloned().unwrap().to_json();
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "transform/position", "value": [100, 100, 0]}}"#));
    run(&mut s, "layer.transform", &format!(r#"{{"layers": [{l}], "op": "center"}}"#));
    assert_eq!(get(&mut s, l, "transform/position"), "[540,960,0]");
    // the 400 × 200 solid filling 1080 × 1920, across only, and down only
    run(&mut s, "layer.transform", &format!(r#"{{"layers": [{l}], "op": "fit"}}"#));
    assert_eq!(get(&mut s, l, "transform/scale"), "[270,960,100]");
    run(&mut s, "layer.transform", &format!(r#"{{"layers": [{l}], "op": "fitWidth"}}"#));
    assert_eq!(get(&mut s, l, "transform/scale"), "[270,270,100]");
    run(&mut s, "layer.transform", &format!(r#"{{"layers": [{l}], "op": "fitHeight"}}"#));
    assert_eq!(get(&mut s, l, "transform/scale"), "[960,960,100]");
    run(&mut s, "layer.transform", &format!(r#"{{"layers": [{l}], "op": "flipH"}}"#));
    assert_eq!(get(&mut s, l, "transform/scale"), "[-960,960,100]");
    run(&mut s, "layer.transform", &format!(r#"{{"layers": [{l}], "op": "reset"}}"#));
    assert_eq!(get(&mut s, l, "transform/scale"), "[100,100,100]");
    // keyed: the change as a key at the view's time
    run(&mut s, "prop.keys", &format!(r#"{{"layer": {l}, "path": "transform/scale", "keys": [{{"t": 0, "v": [100, 100, 100]}}, {{"t": 2, "v": [50, 50, 100]}}]}}"#));
    run(&mut s, "view.set", r#"{"time": 1}"#);
    run(&mut s, "layer.transform", &format!(r#"{{"layers": [{l}], "op": "flipV"}}"#));
    let keys = run(&mut s, "key.list", &format!(r#"{{"layer": {l}, "path": "transform/scale"}}"#));
    assert_eq!(keys.as_array().map(Vec::len), Some(3));
    // the text's anchor in the middle of its letters, the letters staying where they are
    let before = picture(&s);
    let centred = run(&mut s, "layer.centerAnchor", &format!(r#"{{"layers": [{t}]}}"#));
    let anchor = centred.get("layers").and_then(Json::as_array).unwrap()[0].get("anchor").and_then(Json::as_array).unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>();
    assert!(anchor[0] > 100.0 && anchor[1] < 0.0, "the middle of the letters, above the baseline: {anchor:?}");
    let after = picture(&s);
    let ink = |r: &render::Rendered| r.image.px.iter().enumerate().filter(|(_, p)| p[3] > 0.5).map(|(i, _)| i % r.image.width).sum::<usize>();
    assert_eq!(ink(&before), ink(&after), "nothing moved");
    let text_fit = run(&mut s, "layer.transform", &format!(r#"{{"layers": [{t}], "op": "fitWidth"}}"#));
    assert!(text_fit.get("notes").is_none(), "{}", text_fit.to_json());
}

#[test]
fn every_marker_taken_away() {
    let (mut s, _, _) = started();
    run(&mut s, "marker.add", r#"{"time": 1}"#);
    run(&mut s, "marker.add", r#"{"time": 2}"#);
    assert_eq!(run(&mut s, "marker.remove", r#"{"all": true}"#).get("removed").and_then(Json::as_f64), Some(2.0));
    assert!(fails(&mut s, "marker.remove", "").contains("all"));
}

#[test]
fn expressions_read_boxes_and_colours_from_the_pictures() {
    let (mut s, _, l) = started();
    let t = id(&run(&mut s, "layer.add", r#"{"type": "text", "text": {"text": "Wide words", "font": "Helvetica", "size": 100}}"#), "layer");
    // a box behind the words: as wide as they are
    let w = run(&mut s, "prop.evaluate", &format!(r#"{{"layer": {l}, "path": "transform/rotation", "text": "thisComp.layer(\"Wide words\").sourceRectAtTime().width"}}"#));
    let width = w.get("values").and_then(Json::as_array).unwrap()[0].get("value").and_then(Json::as_f64).unwrap();
    assert!(width > 300.0 && width < 600.0, "about the words' width at 100 px: {width}");
    let top = run(&mut s, "prop.evaluate", &format!(r#"{{"layer": {l}, "path": "transform/rotation", "text": "thisComp.layer(\"Wide words\").sourceRectAtTime().top"}}"#));
    assert!(top.get("values").and_then(Json::as_array).unwrap()[0].get("value").and_then(Json::as_f64).unwrap() < -50.0, "above the baseline");
    // the red solid's colour in its middle, and outside it nothing
    let red = run(&mut s, "prop.evaluate", &format!(r#"{{"layer": {t}, "path": "transform/rotation", "text": "thisComp.layer(\"Solid 1\").sampleImage([200, 100], [3, 3])[0] * 100 + thisComp.layer(\"Solid 1\").sampleImage([200, 100])[1]"}}"#));
    assert_eq!(red.get("values").and_then(Json::as_array).unwrap()[0].get("value").and_then(Json::as_f64), Some(100.0));
    let none = run(&mut s, "prop.evaluate", &format!(r#"{{"layer": {t}, "path": "transform/rotation", "text": "thisComp.layer(\"Solid 1\").sampleImage([900, 100])[3]"}}"#));
    assert_eq!(none.get("values").and_then(Json::as_array).unwrap()[0].get("value").and_then(Json::as_f64), Some(0.0));
}

#[test]
fn footage_found_again_stood_in_for_and_edited_in() {
    let mut s = Session::new();
    let dir = std::env::temp_dir().join(format!("veymelo-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).unwrap();
    // two pictures in one folder, imported, then the folder moved
    run(&mut s, "comp.new", r##"{"name": "Pics", "width": 64, "height": 32, "fps": 30, "duration": 2, "background": "#ff0000"}"##);
    for name in ["one", "two"] {
        let p = dir.join("a").join(format!("{name}.png"));
        run(&mut s, "render.frame", &format!(r#"{{"path": "{}"}}"#, p.display()));
    }
    let one = id(&run(&mut s, "footage.import", &format!(r#"{{"path": "{}"}}"#, dir.join("a/one.png").display())), "item");
    let many = run(&mut s, "footage.import", &format!(r#"{{"paths": ["{}", "{}"]}}"#, dir.join("a/two.png").display(), dir.join("a/none.png").display()));
    assert_eq!((many.get("items").and_then(Json::as_array).map(Vec::len), many.get("failed").and_then(Json::as_array).map(Vec::len)), (Some(1), Some(1)));
    std::fs::rename(dir.join("a"), dir.join("b")).unwrap();
    assert_eq!(run(&mut s, "footage.missing", "").as_array().map(Vec::len), Some(2));
    let back = run(&mut s, "footage.relink", &format!(r#"{{"item": {one}, "path": "{}"}}"#, dir.join("b/one.png").display()));
    assert_eq!(back.get("relinked").and_then(Json::as_array).map(Vec::len), Some(2), "the one beside it too: {}", back.to_json());
    assert_eq!(back.get("stillMissing").and_then(Json::as_f64), Some(0.0));
    // moved deeper: found by looking through a folder; a file of the same name but another size is not it
    std::fs::create_dir_all(dir.join("c/deep")).unwrap();
    std::fs::rename(dir.join("b"), dir.join("c/deep/b")).unwrap();
    // a third, gone, with only a picture of another size by its name to be found
    std::fs::create_dir_all(dir.join("x")).unwrap();
    run(&mut s, "render.frame", &format!(r#"{{"path": "{}"}}"#, dir.join("x/three.png").display()));
    let three = id(&run(&mut s, "footage.import", &format!(r#"{{"path": "{}"}}"#, dir.join("x/three.png").display())), "item");
    std::fs::remove_dir_all(dir.join("x")).unwrap();
    run(&mut s, "comp.set", r#"{"width": 40}"#);
    std::fs::create_dir_all(dir.join("c/other")).unwrap();
    run(&mut s, "render.frame", &format!(r#"{{"path": "{}"}}"#, dir.join("c/other/three.png").display()));
    let found = run(&mut s, "footage.find", &format!(r#"{{"folder": "{}"}}"#, dir.join("c").display()));
    assert_eq!(found.get("relinked").and_then(Json::as_array).map(Vec::len), Some(2), "{}", found.to_json());
    assert!(found.get("differs").and_then(Json::as_array).is_some_and(|d| d.iter().any(|x| x.get("why").and_then(Json::as_str).is_some_and(|w| w.contains("40 × 32")))), "{}", found.to_json());
    assert_eq!(run(&mut s, "footage.missing", "").as_array().map(Vec::len), Some(1), "three is not found");
    run(&mut s, "project.item.remove", &format!(r#"{{"item": {three}}}"#));
    // a placeholder: colour bars of its size
    run(&mut s, "comp.new", r#"{"name": "Edit", "width": 320, "height": 240, "fps": 30, "duration": 6}"#);
    let ph = id(&run(&mut s, "footage.placeholder", r#"{"name": "Shot 3", "width": 320, "height": 240, "duration": 2}"#), "item");
    run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {ph}}}"#));
    let bars = picture(&s);
    assert_eq!(px(&bars, 10, 10)[..3].iter().map(|v| (v * 100.0).round() as i32).collect::<Vec<_>>(), [75, 75, 75], "white at three quarters");
    assert_eq!(px(&bars, 310, 10)[..3].iter().map(|v| (v * 100.0).round() as i32).collect::<Vec<_>>(), [0, 0, 75], "blue");
    // a file put in a placeholder's place, then a placeholder back, then solids
    run(&mut s, "footage.replace", &format!(r#"{{"item": {ph}, "path": "{}"}}"#, dir.join("c/deep/b/one.png").display()));
    assert_eq!(run(&mut s, "footage.info", &format!(r#"{{"item": {ph}}}"#)).get("kind").and_then(Json::as_str), Some("image"));
    run(&mut s, "footage.replace", &format!(r#"{{"item": {ph}, "placeholder": true}}"#));
    assert_eq!(run(&mut s, "footage.info", &format!(r#"{{"item": {ph}}}"#)).get("kind").and_then(Json::as_str), Some("placeholder"));
    let solids = run(&mut s, "footage.replace", &format!(r##"{{"item": {ph}, "solid": "#00ff00"}}"##));
    assert_eq!(solids.get("layers").and_then(Json::as_f64), Some(1.0));
    assert_eq!(px(&picture(&s), 10, 10)[1], 1.0, "green");
    // taken the way other footage is
    run(&mut s, "footage.set", &format!(r#"{{"item": {one}, "loops": 3, "alpha": "premultiplied"}}"#));
    let two = s.project().items.iter().find(|i| i.name == "two.png").unwrap().id;
    let like = run(&mut s, "footage.interpretLike", &format!(r#"{{"item": {two}, "from": {one}}}"#));
    assert_eq!((like.get("loops").and_then(Json::as_f64), like.get("alpha").and_then(Json::as_str)), (Some(3.0), Some("premultiplied")));
    // the same file twice made one
    let again = id(&run(&mut s, "footage.import", &format!(r#"{{"path": "{}", "name": "one again"}}"#, dir.join("c/deep/b/one.png").display())), "item");
    run(&mut s, "footage.set", &format!(r#"{{"item": {again}, "loops": 3, "alpha": "premultiplied"}}"#));
    let shown = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {again}}}"#)), "layer");
    let merged = run(&mut s, "project.consolidate", "");
    assert_eq!(merged.get("merged").and_then(Json::as_array).map(Vec::len), Some(1), "{}", merged.to_json());
    assert_eq!(s.project().layer(shown).unwrap().source, veymelo_project::Source::Footage(one));
    let used = run(&mut s, "project.item.usage", &format!(r#"{{"item": {one}}}"#));
    assert_eq!(used.get("layers").and_then(Json::as_array).map(Vec::len), Some(1));
    // edits: two shots one after another, a one-second shot put in between, pushing the second
    run(&mut s, "comp.new", r#"{"name": "Cut", "width": 64, "height": 32, "fps": 30, "duration": 4}"#);
    let a1 = id(&run(&mut s, "layer.add", r##"{"type": "solid", "name": "A", "color": "#ff0000", "out": 2}"##), "layer");
    let b1 = id(&run(&mut s, "layer.add", r##"{"type": "solid", "name": "B", "color": "#0000ff", "in": 2, "out": 4}"##), "layer");
    let ins = run(&mut s, "layer.insertFootage", &format!(r#"{{"item": {two}, "at": 2, "from": 0, "to": 1, "ripple": true}}"#));
    assert_eq!((ins.get("in").and_then(Json::as_f64), ins.get("out").and_then(Json::as_f64)), (Some(2.0), Some(3.0)));
    let times = |s: &Session, id: u64| { let l = s.project().layer(id).unwrap(); (l.in_point.seconds(), l.out_point.seconds()) };
    assert_eq!(times(&s, a1), (0.0, 2.0));
    assert_eq!(times(&s, b1), (3.0, 5.0), "pushed later by its length");
    // one across the moment is cut there; over the top leaves the rest where it is
    let ins2 = run(&mut s, "layer.insertFootage", &format!(r#"{{"item": {two}, "at": 1, "to": 0.5, "ripple": true}}"#));
    assert!(ins2.get("pushed").and_then(Json::as_array).is_some_and(|p| p.len() >= 3), "{}", ins2.to_json());
    assert_eq!(times(&s, a1), (0.0, 1.0), "cut where it went in");
    run(&mut s, "layer.insertFootage", &format!(r#"{{"item": {two}, "at": 0, "to": 0.5}}"#));
    assert_eq!(times(&s, a1), (0.0, 1.0));
    // only some compositions kept
    let reduced = run(&mut s, "project.reduce", r#"{"comps": ["Cut"]}"#);
    let names: Vec<&str> = reduced.get("removed").and_then(Json::as_array).unwrap().iter().filter_map(Json::as_str).collect();
    assert!(names.contains(&"Pics") && names.contains(&"Edit") && names.contains(&"Shot 3") && !names.contains(&"two.png"), "{names:?}");
    // saved, saved again to the next numbered name, and a copy
    let show = dir.join("Show.veymelo");
    run(&mut s, "project.save", &format!(r#"{{"path": "{}"}}"#, show.display()));
    let next = run(&mut s, "project.save", r#"{"increment": true}"#);
    assert!(next.get("path").and_then(Json::as_str).unwrap().ends_with("Show 2.veymelo"));
    let copy = dir.join("Elsewhere.veymelo");
    run(&mut s, "project.save", &format!(r#"{{"copy": "{}"}}"#, copy.display()));
    assert!(copy.exists() && s.path.as_ref().unwrap().ends_with("Show 2.veymelo"));
    // a file that changed since it was read
    let pic = dir.join("c/deep/b/two.png");
    run(&mut s, "view.set", r#"{"comp": "Cut", "time": 2.5}"#);
    picture(&s);
    std::thread::sleep(std::time::Duration::from_millis(20));
    run(&mut s, "comp.set", r#"{"comp": "Cut", "width": 50}"#);
    run(&mut s, "render.frame", &format!(r#"{{"comp": "Cut", "path": "{}"}}"#, pic.display()));
    assert_eq!(run(&mut s, "footage.info", &format!(r#"{{"item": {two}}}"#)).get("changed").and_then(Json::as_bool), Some(true));
    let reloaded = run(&mut s, "footage.reload", r#"{"changed": true}"#);
    assert_eq!(reloaded.get("reloaded").and_then(Json::as_array).map(Vec::len), Some(1), "{}", reloaded.to_json());
    assert_eq!(run(&mut s, "footage.info", &format!(r#"{{"item": {two}}}"#)).get("width").and_then(Json::as_f64), Some(50.0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn renders_with_more_outputs_templates_and_layered_frames() {
    let mut s = Session::new();
    let dir = std::env::temp_dir().join(format!("veymelo-renders-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let folder = dir.display().to_string();
    // a preset of our own, and one that comes with the program
    let c = id(&run(&mut s, "comp.new", r##"{"name": "Small", "preset": "Square 1080 30", "width": 64, "height": 64, "duration": 0.2, "background": "#0000ff"}"##), "comp");
    let made = s.project().comp(c).unwrap();
    assert_eq!((made.width, made.height, made.duration.seconds()), (64, 64, 0.2), "what is given over the preset");
    run(&mut s, "queue.saveTemplate", &format!(r#"{{"kind": "comp", "name": "Tiny", "folder": "{folder}"}}"#));
    let tiny = id(&run(&mut s, "comp.new", &format!(r#"{{"name": "From Tiny", "preset": "Tiny", "folder": "{folder}"}}"#)), "comp");
    assert_eq!(s.project().comp(tiny).unwrap().width, 64);
    run(&mut s, "view.set", r#"{"comp": "Small"}"#);
    run(&mut s, "layer.add", r##"{"type": "solid", "name": "Red", "color": "#ff0000", "width": 32, "height": 32}"##);
    // one item, two files: pictures that keep see-through, and pictures on the background
    let item = id(&run(&mut s, "queue.add", &format!(r#"{{"path": "{folder}/clear_##.png", "outputTemplate": "PNG Sequence with Alpha", "settingsTemplate": "Best Settings"}}"#)), "item");
    let outs = run(&mut s, "queue.output", &format!(r#"{{"item": {item}, "op": "add", "path": "{folder}/flat_##.png", "format": "pngSequence"}}"#));
    assert_eq!(outs.get("outputs").and_then(Json::as_array).map(Vec::len), Some(2));
    let rendered = run(&mut s, "queue.render", &format!(r#"{{"item": {item}}}"#));
    let files = rendered.get("made").and_then(Json::as_array).unwrap()[0].get("files").and_then(Json::as_array).unwrap().len();
    assert_eq!(files, 12, "six frames, each in two files: {}", rendered.to_json());
    let corner = |path: &str| {
        let p = veymelo_os::read_picture(path, 0).unwrap();
        p.px[0]
    };
    assert!(corner(&format!("{folder}/clear_00.png"))[3] < 0.01, "clear");
    assert!(corner(&format!("{folder}/flat_00.png"))[2] > 0.9, "blue background");
    // a template of its own, a copy of the item, moved first
    run(&mut s, "queue.output", &format!(r#"{{"item": {item}, "op": "set", "index": 0, "output": {{"quality": 55}}}}"#));
    run(&mut s, "queue.saveTemplate", &format!(r#"{{"kind": "output", "name": "Mine", "item": {item}, "folder": "{folder}"}}"#));
    let listed = run(&mut s, "queue.templates", &format!(r#"{{"folder": "{folder}"}}"#));
    assert!(listed.get("output").and_then(Json::as_array).unwrap().iter().any(|t| t.get("name").and_then(Json::as_str) == Some("Mine") && t.get("quality").and_then(Json::as_f64) == Some(55.0)));
    let copy = id(&run(&mut s, "queue.duplicate", &format!(r#"{{"item": {item}}}"#)), "item");
    let list = run(&mut s, "queue.list", "");
    assert!(list.as_array().unwrap()[1].get("output").and_then(|o| o.get("path")).and_then(Json::as_str).unwrap().ends_with("clear_## 2.png"));
    let order = run(&mut s, "queue.move", &format!(r#"{{"item": {copy}, "to": 1}}"#));
    assert_eq!(order.get("order").map(Json::to_json), Some(format!("[{copy},{item}]")));
    run(&mut s, "queue.output", &format!(r#"{{"item": {copy}, "op": "remove", "index": 1}}"#));
    assert!(fails(&mut s, "queue.output", &format!(r#"{{"item": {copy}, "op": "remove", "index": 0}}"#)).contains("stays"));
    run(&mut s, "queue.deleteTemplate", &format!(r#"{{"kind": "output", "name": "Mine", "folder": "{folder}"}}"#));
    assert!(fails(&mut s, "queue.deleteTemplate", &format!(r#"{{"kind": "output", "name": "H.264", "folder": "{folder}"}}"#)).contains("stays"));
    // frames as EXR (read back by the system), with layers, and as layered Photoshop files
    let exr = format!("{folder}/frame.exr");
    run(&mut s, "render.frame", &format!(r#"{{"path": "{exr}"}}"#));
    let back = veymelo_os::read_picture(&exr, 0).unwrap();
    assert_eq!((back.width, back.height), (64, 64));
    let mid = back.px[32 * 64 + 32];
    assert!(mid[0] > 0.9 && mid[2] < 0.05, "red in the middle: {mid:?}");
    let layered = run(&mut s, "render.frame", &format!(r#"{{"path": "{folder}/layers.exr", "layers": true, "half": true}}"#));
    assert_eq!(layered.get("layers").and_then(Json::as_f64), Some(1.0));
    let bytes = std::fs::read(format!("{folder}/layers.exr")).unwrap();
    assert!(bytes.windows(6).any(|w| w == b"Red.R\0"), "the layer's channels");
    assert!(veymelo_os::read_picture(&format!("{folder}/layers.exr"), 0).is_ok(), "readable as a picture");
    let psd = run(&mut s, "render.frame", &format!(r#"{{"path": "{folder}/layers.psd"}}"#));
    assert_eq!(psd.get("layers").and_then(Json::as_f64), Some(1.0));
    let file = std::fs::read(format!("{folder}/layers.psd")).unwrap();
    assert_eq!(&file[..4], b"8BPS");
    assert!(file.windows(4).any(|w| w == b"luni"));
    let flat = veymelo_os::read_picture(&format!("{folder}/layers.psd"), 0).unwrap();
    assert_eq!((flat.width, flat.height), (64, 64));
    assert!(flat.px[32 * 64 + 32][0] > 0.9, "the merged picture");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn protected_stretches_keep_their_speed() {
    let mut s = Session::new();
    // a 10 s composition with a 2 s intro and a 2 s outro kept, shown at 200 %
    run(&mut s, "comp.new", r#"{"name": "Inner", "width": 10, "height": 10, "fps": 10, "duration": 10}"#);
    run(&mut s, "view.set", r#"{"time": 2}"#);
    run(&mut s, "comp.responsiveTime", r#"{"op": "intro"}"#);
    run(&mut s, "view.set", r#"{"time": 8}"#);
    let outro = run(&mut s, "comp.responsiveTime", r#"{"op": "outro"}"#);
    assert_eq!((outro.get("start").and_then(Json::as_f64), outro.get("end").and_then(Json::as_f64)), (Some(8.0), Some(10.0)));
    run(&mut s, "comp.new", r#"{"name": "Outer", "width": 10, "height": 10, "fps": 10, "duration": 20}"#);
    let l = id(&run(&mut s, "layer.add", r#"{"type": "comp", "item": "Inner"}"#), "layer");
    run(&mut s, "layer.stretch", &format!(r#"{{"layer": {l}, "stretch": 200}}"#));
    let shown = |s: &mut Session, t: f64| {
        let v = run(s, "prop.evaluate", &format!(r#"{{"layer": {l}, "path": "transform/rotation", "text": "0", "times": [{t}]}}"#));
        let _ = v;
        let p = s.project();
        let c = p.comp(s.view.comp.unwrap()).unwrap();
        let layer = c.layer(l).unwrap();
        layer.shown_time(p, Tick::from_seconds(t)).seconds()
    };
    assert!((shown(&mut s, 1.0) - 1.0).abs() < 1e-6, "the intro at its own speed");
    // the middle 6 s take the whole stretch: 20 - 4 = 16 s for them
    assert!((shown(&mut s, 2.0 + 8.0) - 5.0).abs() < 1e-6, "half way through the middle");
    assert!((shown(&mut s, 19.0) - 9.0).abs() < 1e-6, "the outro at its own speed");
    // a freeze holds what is shown
    run(&mut s, "layer.freeze", &format!(r#"{{"layer": {l}, "time": 19}}"#));
    let held = s.project().layer(l).unwrap().props.prop("timeRemap").unwrap().keys[0].value.as_f64();
    assert!((held - 9.0).abs() < 1e-6, "{held}");
}

#[test]
fn timelines_go_out_to_editors_and_come_back() {
    let mut s = Session::new();
    let dir = std::env::temp_dir().join(format!("veymelo-timeline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let folder = dir.display().to_string();
    // a two-second clip of our own, and a picture
    run(&mut s, "comp.new", r##"{"name": "Src", "width": 64, "height": 36, "fps": 25, "duration": 2, "background": "#00ff00"}"##);
    run(&mut s, "render.export", &format!(r#"{{"path": "{folder}/clip.mov", "format": "prores"}}"#));
    run(&mut s, "render.frame", &format!(r#"{{"path": "{folder}/still.png"}}"#));
    let clip = id(&run(&mut s, "footage.import", &format!(r#"{{"path": "{folder}/clip.mov"}}"#)), "item");
    let still = id(&run(&mut s, "footage.import", &format!(r#"{{"path": "{folder}/still.png"}}"#)), "item");
    // an edit: the clip from 0.4 s for a second, then from 0 s for a second, the picture above both
    run(&mut s, "comp.new", r#"{"name": "Edit", "width": 64, "height": 36, "fps": 25, "duration": 3}"#);
    let a = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {clip}, "name": "A"}}"#)), "layer");
    let b = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {clip}, "name": "B"}}"#)), "layer");
    let p = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {still}, "name": "P"}}"#)), "layer");
    run(&mut s, "layer.add", r##"{"type": "solid", "name": "Not a file", "color": "#ff0000"}"##);
    let set = |s: &mut Session, l: u64, start: f64, inn: f64, out: f64| {
        run(s, "layer.set", &format!(r#"{{"layer": {l}, "start": {start}, "in": {inn}, "out": {out}}}"#));
    };
    set(&mut s, a, -0.4, 0.0, 1.0);
    set(&mut s, b, 1.0, 1.0, 2.0);
    set(&mut s, p, 0.0, 0.4, 1.4);
    for ending in ["otio", "xml", "fcpxml", "edl"] {
        let file = format!("{folder}/edit.{ending}");
        let out = run(&mut s, "timeline.export", &format!(r#"{{"comp": "Edit", "path": "{file}"}}"#));
        let notes = out.get("notes").and_then(Json::as_array).unwrap();
        assert!(notes.iter().any(|n| n.as_str().unwrap().contains("not footage")), "{ending}: {}", out.to_json());
        assert_eq!(out.get("tracks").and_then(Json::as_f64), Some(2.0), "two clips one after another on one track, the picture above (the clip has no sound): {}", out.to_json());
        let back = run(&mut s, "timeline.import", &format!(r#"{{"path": "{file}", "edlFps": 25}}"#));
        let cid = id(&back, "comp");
        let comp = s.project().comp(cid).unwrap();
        let timing: Vec<(String, f64, f64, f64, bool, bool)> = comp.layers.iter().rev().map(|l| (l.name.clone(), l.start.seconds(), l.in_point.seconds(), l.out_point.seconds(), l.switches.video, l.switches.audio)).collect();
        let pictures: Vec<_> = timing.iter().filter(|t| t.4).map(|t| (t.0.as_str(), (t.1 * 100.0).round() / 100.0, t.2, t.3)).collect();
        let want = if ending == "edl" { vec![("A", -0.4, 0.0, 1.0), ("B", 1.0, 1.0, 2.0)] } else { vec![("A", -0.4, 0.0, 1.0), ("B", 1.0, 1.0, 2.0), ("P", 0.0, 0.4, 1.4)] };
        let names: Vec<_> = pictures.iter().map(|p| (p.0.split(' ').next().unwrap(), p.1, p.2, p.3)).collect();
        assert_eq!(names, want, "{ending}: {timing:?}");
        assert!(back.get("missing").and_then(Json::as_array).unwrap().is_empty());
    }
    // a file gone: a placeholder stands in, found again later
    std::fs::rename(dir.join("clip.mov"), dir.join("moved.mov")).unwrap();
    let back = run(&mut s, "timeline.import", &format!(r#"{{"path": "{folder}/edit.otio"}}"#));
    assert_eq!(back.get("missing").and_then(Json::as_array).map(Vec::len), Some(1));
    let ph = s.project().items.iter().rev().find(|i| i.name == "clip.mov").unwrap();
    assert!(matches!(&ph.kind, veymelo_project::ItemKind::Footage(f) if f.kind == veymelo_project::FootageKind::Placeholder && f.width == 64));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A 400 × 240 composition with a red 3D square in the middle and a two-node camera.
fn scene() -> (Session, u64, u64, u64) {
    let mut s = Session::new();
    let c = id(&run(&mut s, "comp.new", r#"{"name": "Main", "width": 400, "height": 240, "fps": 25, "duration": 2}"#), "comp");
    let sq = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ff0000", "width": 80, "height": 80}"##), "layer");
    run(&mut s, "layer.set", &format!(r#"{{"layer": {sq}, "switches": {{"threeD": true}}}}"#));
    let cam = id(&run(&mut s, "layer.add", r#"{"type": "camera"}"#), "layer");
    (s, c, sq, cam)
}

/// What an expression gives on a layer's property now.
fn worked_out(s: &mut Session, comp: u64, layer: u64, path: &str, text: &str) -> Vec<f64> {
    let text = if text.is_empty() { String::new() } else { format!(r#", "text": {}"#, Json::from(text).to_json()) };
    let r = run(s, "prop.evaluate", &format!(r#"{{"comp": {comp}, "layer": {layer}, "path": "{path}"{text}}}"#));
    let v = r.get("values").and_then(Json::as_array).and_then(|v| v[0].get("value")).cloned().unwrap_or_else(|| panic!("{}", r.to_json()));
    match v {
        Json::Array(a) => a.iter().map(|x| x.as_f64().unwrap()).collect(),
        n => vec![n.as_f64().unwrap_or_else(|| panic!("{}", r.to_json()))],
    }
}

/// What `thisComp.layer(<layer>).<member>` gives, read on another layer of its composition (an
/// expression tried on one of its own properties would stand in for what is there).
fn member_of(s: &mut Session, comp: u64, layer: u64, member: &str) -> Vec<f64> {
    let cp = s.project().comp(comp).unwrap();
    let name = cp.layer(layer).unwrap().name.clone();
    let other = cp.layers.iter().find(|x| x.id != layer && !matches!(x.source, veymelo_project::Source::Camera | veymelo_project::Source::Light(_))).unwrap().id;
    worked_out(s, comp, other, "transform/position", &format!("thisComp.layer({}).{member}", Json::from(name).to_json()))
}

fn near(a: &[f64], b: &[f64], tol: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

/// The box of the pixels mostly of one channel (0 red, 1 green), and their middle across.
fn ink(r: &render::Rendered, channel: usize) -> Option<([usize; 4], f64)> {
    let (w, img) = (r.image.width, &r.image);
    let mut b = [usize::MAX, usize::MAX, 0, 0];
    let (mut sum, mut n) = (0.0, 0.0);
    for (i, p) in img.px.iter().enumerate() {
        if p[3] > 0.5 && p[channel] > 0.5 && p[(channel + 1) % 3] < 0.3 {
            let (x, y) = (i % w, i / w);
            b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            sum += x as f64;
            n += 1.0;
        }
    }
    (n > 0.0).then(|| (b, sum / n))
}

#[test]
fn cameras_orbited_panned_dollied_and_aimed() {
    let (mut s, c, sq, cam) = scene();
    let get = |s: &mut Session, layer: u64, path: &str| -> Vec<f64> {
        let v = run(s, "prop.get", &format!(r#"{{"layer": {layer}, "path": "{path}"}}"#)).get("value").cloned().unwrap();
        v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap()).collect()).unwrap_or_else(|| vec![v.as_f64().unwrap()])
    };
    let zoom = get(&mut s, cam, "cameraOptions/zoom")[0];
    // a quarter turn to its right: beside the square, still looking at it
    let r = run(&mut s, "camera.orbit", r#"{"yaw": 90}"#);
    assert!(near(&get(&mut s, cam, "transform/position"), &[200.0 + zoom, 120.0, 0.0], 1e-6), "{}", r.to_json());
    run(&mut s, "edit.undo", "");
    // up 30°, and no farther than nearly straight above
    run(&mut s, "camera.orbit", r#"{"pitch": 30}"#);
    let (sn, cs) = 30f64.to_radians().sin_cos();
    assert!(near(&get(&mut s, cam, "transform/position"), &[200.0, 120.0 - zoom * sn, -zoom * cs], 1e-6));
    run(&mut s, "camera.orbit", r#"{"pitch": 200}"#);
    let p = get(&mut s, cam, "transform/position");
    assert!(((120.0 - p[1]) / zoom - 89f64.to_radians().sin()).abs() < 1e-9, "kept short of above: {p:?}");
    run(&mut s, "edit.undo", "");
    run(&mut s, "edit.undo", "");
    // tracked right 100 pixels of its picture: the square slides 100 pixels left
    let before = ink(&picture(&s), 0).unwrap().1;
    run(&mut s, "camera.pan", r#"{"x": 100}"#);
    assert!(near(&get(&mut s, cam, "transform/pointOfInterest"), &[300.0, 120.0, 0.0], 1e-6));
    let after = ink(&picture(&s), 0).unwrap().1;
    assert!((before - after - 100.0).abs() < 1.0, "{before} → {after}");
    run(&mut s, "edit.undo", "");
    // dollied: both forward, or the camera alone nearing the point
    run(&mut s, "camera.dolly", r#"{"by": 100}"#);
    assert!(near(&get(&mut s, cam, "transform/position"), &[200.0, 120.0, 100.0 - zoom], 1e-6));
    assert!(near(&get(&mut s, cam, "transform/pointOfInterest"), &[200.0, 120.0, 100.0], 1e-6));
    run(&mut s, "edit.undo", "");
    run(&mut s, "camera.dolly", &format!(r#"{{"by": {}, "still": true}}"#, zoom * 2.0));
    assert!(near(&get(&mut s, cam, "transform/position"), &[200.0, 120.0, -1.0], 1e-6), "a pixel short");
    run(&mut s, "edit.undo", "");
    // animated: a key at the time
    run(&mut s, "prop.keys", &format!(r#"{{"layer": {cam}, "path": "transform/position", "keys": [{{"t": 0, "v": [200, 120, {}]}}, {{"t": 1, "v": [200, 120, {}]}}]}}"#, -zoom, -zoom - 200.0));
    run(&mut s, "camera.dolly", r#"{"by": 50, "time": 0.5}"#);
    assert_eq!(run(&mut s, "key.list", &format!(r#"{{"layer": {cam}, "path": "transform/position"}}"#)).as_array().map(Vec::len), Some(3));
    run(&mut s, "edit.undo", "");
    run(&mut s, "edit.undo", "");
    // a one-node camera turns as it swings, to keep looking at the point
    run(&mut s, "layer.set", &format!(r#"{{"layer": {cam}, "autoOrient": "off"}}"#));
    run(&mut s, "camera.orbit", r#"{"yaw": 90}"#);
    assert!(near(&get(&mut s, cam, "transform/orientation"), &[0.0, -90.0, 0.0], 1e-6), "{:?}", get(&mut s, cam, "transform/orientation"));
    let ahead: Vec<f64> = member_of(&mut s, c, cam, "toWorld([0, 0, 0])").iter().zip(member_of(&mut s, c, cam, "toWorldVec([0, 0, 1])")).map(|(p, f)| p + f * zoom).collect();
    assert!(near(&ahead, &[200.0, 120.0, 0.0], 1e-6), "still looking at the square's middle: {ahead:?}");
    run(&mut s, "edit.undo", "");
    run(&mut s, "edit.undo", "");

    // two squares far apart: turned to the middle between them, near enough to fill the picture
    let other = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ff0000", "width": 80, "height": 80}"##), "layer");
    run(&mut s, "layer.set", &format!(r#"{{"layer": {other}, "switches": {{"threeD": true}}}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {sq}, "path": "transform/position", "value": [-200, 120, 300]}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {other}, "path": "transform/position", "value": [200, 120, 300]}}"#));
    let r = run(&mut s, "camera.lookAt", "");
    assert!(near(&get(&mut s, cam, "transform/pointOfInterest"), &[0.0, 120.0, 300.0], 1e-6), "{}", r.to_json());
    let (b, _) = ink(&picture(&s), 0).unwrap();
    let (w, h) = (400.0, 240.0);
    let edges = [(b[0] as f64, w), (w - 1.0 - b[2] as f64, w), (b[1] as f64, h), (h - 1.0 - b[3] as f64, h)];
    assert!(edges.iter().all(|(edge, room)| *edge >= 0.05 * room - 1.5), "inside the margin: {b:?}");
    assert!(edges.iter().any(|(edge, room)| (*edge - 0.05 * room).abs() < 2.0), "touching it on one side: {b:?}");
    // facing the same way, moved sideways to them (and not nearer or farther)
    run(&mut s, "edit.undo", "");
    let from = get(&mut s, cam, "transform/position");
    run(&mut s, "camera.lookAt", &format!(r#"{{"layers": [{other}], "keepDirection": true, "fit": false}}"#));
    assert!(near(&get(&mut s, cam, "transform/position"), &[200.0, 120.0, from[2]], 1e-6));
    assert!(near(&get(&mut s, cam, "transform/pointOfInterest"), &[200.0, 120.0, 300.0], 1e-6));
    // a 2D layer is not seen by the camera
    run(&mut s, "layer.set", &format!(r#"{{"layer": {other}, "switches": {{"threeD": false}}}}"#));
    assert!(fails(&mut s, "camera.lookAt", &format!(r#"{{"layers": [{other}]}}"#)).contains("2D"));
    // a camera following an expression is not moved under it
    run(&mut s, "prop.expression", &format!(r#"{{"layer": {cam}, "path": "transform/position", "text": "[200, 120, -900]"}}"#));
    assert!(fails(&mut s, "camera.pan", r#"{"x": 10}"#).contains("expression"));
}

#[test]
fn cameras_focused_hung_on_nulls_and_carrying_lights() {
    let (mut s, c, sq, cam) = scene();
    let get = |s: &mut Session, layer: u64, path: &str| -> Json { run(s, "prop.get", &format!(r#"{{"layer": {layer}, "path": "{path}"}}"#)).get("value").cloned().unwrap() };
    let zoom = get(&mut s, cam, "cameraOptions/zoom").as_f64().unwrap();
    run(&mut s, "prop.set", &format!(r#"{{"layer": {sq}, "path": "transform/position", "value": [260, 120, 500]}}"#));
    // sharp at the square, along the way the camera looks (not straight at it)
    let r = run(&mut s, "camera.focus", &format!(r#"{{"to": {sq}}}"#));
    assert!((get(&mut s, cam, "cameraOptions/focusDistance").as_f64().unwrap() - (zoom + 500.0)).abs() < 1e-6, "{}", r.to_json());
    assert!(r.get("notes").is_some(), "depth of field is off: {}", r.to_json());
    // kept sharp as it moves
    run(&mut s, "camera.focus", &format!(r#"{{"to": {sq}, "link": true}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {sq}, "path": "transform/position", "value": [260, 120, 800]}}"#));
    let follows = worked_out(&mut s, c, cam, "cameraOptions/focusDistance", "");
    assert!((follows[0] - (zoom + 800.0)).abs() < 1e-6, "{follows:?}");
    run(&mut s, "camera.focus", r#"{"to": "pointOfInterest", "link": true}"#);
    run(&mut s, "camera.dolly", r#"{"by": -100, "still": true}"#);
    assert!((worked_out(&mut s, c, cam, "cameraOptions/focusDistance", "")[0] - (zoom + 100.0)).abs() < 1e-6);
    assert!(fails(&mut s, "camera.focus", &format!(r#"{{"to": {cam}}}"#)).contains("itself"));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {sq}, "path": "transform/position", "value": [200, 120, -3000]}}"#));
    assert!(fails(&mut s, "camera.focus", &format!(r#"{{"to": {sq}}}"#)).contains("behind"));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {sq}, "path": "transform/position", "value": [200, 120, 0]}}"#));

    // hung on a null at the point it looks at: where it was, and the null swings it round
    let eye = |s: &mut Session| member_of(s, c, cam, "toWorld([0, 0, 0])");
    let before = (eye(&mut s), picture(&s));
    let r = run(&mut s, "camera.orbitNull", "");
    let null = id(&r, "null");
    assert!(near(&eye(&mut s), &before.0, 1e-6), "{:?} {:?}", eye(&mut s), before.0);
    assert_eq!(picture(&s).image.px, before.1.image.px, "the same picture");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {null}, "path": "transform/rotationY", "value": -90}}"#));
    let d = zoom + 100.0;
    assert!(near(&eye(&mut s), &[200.0 + d, 120.0, 0.0], 1e-6), "a quarter round, to the right: {:?}", eye(&mut s));
    assert!(fails(&mut s, "camera.orbitNull", "").contains("already"));
    run(&mut s, "edit.undo", "");
    run(&mut s, "edit.undo", "");

    // a light riding on the camera: at its eye, shining its way; let go where it is
    let light = id(&run(&mut s, "layer.add", r#"{"type": "light", "light": "spot"}"#), "layer");
    run(&mut s, "light.followCamera", &format!(r#"{{"lights": [{light}], "camera": {cam}}}"#));
    let seen = |s: &mut Session, l: u64| (member_of(s, c, l, "toWorld([0, 0, 0])"), member_of(s, c, l, "toWorldVec([0, 0, 1])"));
    let (at, way) = seen(&mut s, light);
    let (cam_at, cam_way) = seen(&mut s, cam);
    assert!(near(&at, &cam_at, 1e-6) && near(&way, &cam_way, 1e-9), "{at:?} {cam_at:?}");
    run(&mut s, "camera.orbit", r#"{"yaw": 40, "pitch": 20}"#);
    let (at, way) = seen(&mut s, light);
    let (cam_at, cam_way) = seen(&mut s, cam);
    assert!(near(&at, &cam_at, 1e-6) && near(&way, &cam_way, 1e-9), "it goes along");
    run(&mut s, "light.followCamera", &format!(r#"{{"lights": [{light}], "on": false}}"#));
    assert_eq!(s.project().comp(c).unwrap().layer(light).unwrap().parent, None);
    let (after, after_way) = seen(&mut s, light);
    assert!(near(&after, &at, 1e-6) && near(&after_way, &way, 1e-9), "{after:?} {at:?} {after_way:?} {way:?}");
    assert!(fails(&mut s, "light.followCamera", &format!(r#"{{"lights": [{sq}]}}"#)).contains("not a light"));
}

#[test]
fn a_stereo_rig_sees_with_two_eyes() {
    let (mut s, c, far, cam) = scene();
    run(&mut s, "comp.set", r##"{"background": "#ffffff"}"##);
    let near_sq = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#00ff00", "width": 60, "height": 60}"##), "layer");
    run(&mut s, "layer.set", &format!(r#"{{"layer": {near_sq}, "switches": {{"threeD": true}}}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {far}, "path": "transform/position", "value": [200, 80, 0]}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {near_sq}, "path": "transform/position", "value": [200, 170, -300]}}"#));
    let r = run(&mut s, "camera.stereoRig", r#"{"sceneDepth": 10}"#);
    assert_eq!(id(&r, "master"), cam, "the camera it is seen through");
    let (left, right, out) = (id(&r, "leftComp"), id(&r, "rightComp"), id(&r, "stereoComp"));
    let (lc, rc) = (id(&r, "leftCamera"), id(&r, "rightCamera"));
    let names: Vec<String> = [left, right, out].iter().map(|i| s.project().item(*i).unwrap().name.clone()).collect();
    assert_eq!(names, ["Main Left Eye", "Main Right Eye", "Main Stereo 3D"]);
    // the eyes 10 % of the width apart, either side of the master
    let world = |s: &mut Session, comp: u64, l: u64| member_of(s, comp, l, "toWorld([0, 0, 0])");
    let master = world(&mut s, c, cam);
    assert!(near(&world(&mut s, left, lc), &[master[0] - 20.0, master[1], master[2]], 1e-6), "{:?} {master:?}", world(&mut s, left, lc));
    assert!(near(&world(&mut s, right, rc), &[master[0] + 20.0, master[1], master[2]], 1e-6));
    // what each eye sees: the nearer square shifts more between them than the farther one
    let draw = |s: &Session, comp: u64| s.render(Some(&Json::Number(comp as f64)), None, &render::Options::default()).unwrap();
    let apart = |s: &Session| {
        let (l, r) = (draw(s, left), draw(s, right));
        let x = |img: &render::Rendered, ch| ink(img, ch).unwrap_or_else(|| panic!("no colour {ch}")).1;
        (x(&l, 0) - x(&r, 0), x(&l, 1) - x(&r, 1))
    };
    let (red, green) = apart(&s);
    assert!((red - 40.0).abs() < 1.0, "the far square as far apart as the eyes: {red}");
    assert!(green > red + 5.0, "the near one more: {green} {red}");
    // turned in to meet at the point of interest: the far square, there, does not shift
    let controls = id(&r, "controls");
    run(&mut s, "prop.set", &format!(r#"{{"comp": {c}, "layer": {controls}, "path": "effects/Stereo 3D Controls/convergence", "value": true}}"#));
    let (red, green) = apart(&s);
    assert!(red.abs() < 1.0 && green > 5.0, "{red} {green}");
    // the eyes farther apart, by the controls
    run(&mut s, "prop.set", &format!(r#"{{"comp": {c}, "layer": {controls}, "path": "effects/Stereo 3D Controls/sceneDepth", "value": 20}}"#));
    let (_, wider) = apart(&s);
    assert!(wider > green * 1.8, "{wider} {green}");
    // the master rolled: the eyes roll with it
    run(&mut s, "prop.set", &format!(r#"{{"comp": {c}, "layer": {cam}, "path": "transform/rotation", "value": 20}}"#));
    let roll = worked_out(&mut s, left, lc, "transform/rotation", "")[0];
    assert!((roll - 20.0).abs() < 1e-6, "{roll}");
    // both put together for red and blue glasses
    let both = draw(&s, out);
    let (alone_l, alone_r) = (draw(&s, left), draw(&s, right));
    assert!(both.image.px != alone_l.image.px && both.image.px != alone_r.image.px);
    if let Ok(dir) = std::env::var("VEYMELO_LOOK") {
        run(&mut s, "prop.set", &format!(r#"{{"comp": {c}, "layer": {cam}, "path": "transform/rotation", "value": 0}}"#));
        for (comp, name) in [(left, "left"), (right, "right"), (out, "stereo")] {
            run(&mut s, "render.frame", &format!(r#"{{"comp": {comp}, "path": "{dir}/stereo-{name}.png"}}"#));
        }
    }
}

#[test]
fn strokes_painted_cloned_erased_and_written_on() {
    let (mut s, _, l) = started();
    // the solid's pixel (x, y) is the composition's (340 + x, 860 + y)
    let seen = |s: &Session, x: usize, y: usize| px(&picture(s), 340 + x, 860 + y);
    let r = run(&mut s, "paint.stroke", &format!(r##"{{"layer": {l}, "points": [[50, 100], [350, 100]], "color": "#ffffff", "diameter": 20}}"##));
    assert_eq!(r.get("stroke").and_then(Json::as_str), Some("effects/Paint/strokes/Brush 1"));
    assert_eq!(seen(&s, 200, 100), [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(seen(&s, 200, 130), [1.0, 0.0, 0.0, 1.0], "the solid elsewhere");
    // the next stroke goes into the same effect; an eraser rubs both out across it
    let r = run(&mut s, "paint.stroke", &format!(r##"{{"layer": {l}, "points": [[200, 20], [200, 180]], "color": "#0000ff", "tip": "hard 10"}}"##));
    assert_eq!(r.get("stroke").and_then(Json::as_str), Some("effects/Paint/strokes/Brush 2"));
    assert_eq!(seen(&s, 200, 50), [0.0, 0.0, 1.0, 1.0]);
    run(&mut s, "paint.stroke", &format!(r#"{{"layer": {l}, "kind": "eraser", "points": [[100, 0], [100, 200]], "diameter": 30}}"#));
    assert_eq!(seen(&s, 100, 100)[3], 0.0);
    assert_eq!(seen(&s, 100, 30)[3], 0.0, "the solid too");
    run(&mut s, "edit.undo", "");
    // only the stroke before: the solid shows again under the blue
    run(&mut s, "paint.stroke", &format!(r#"{{"layer": {l}, "kind": "eraser", "eraseMode": "lastStroke", "points": [[150, 50], [250, 50]], "diameter": 30}}"#));
    assert_eq!(seen(&s, 200, 50), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(seen(&s, 200, 100), [0.0, 0.0, 1.0, 1.0], "the blue elsewhere stays");
    // a clone copies the white stroke down
    run(&mut s, "paint.stroke", &format!(r#"{{"layer": {l}, "kind": "clone", "points": [[300, 160], [320, 160]], "clonePosition": [300, 100], "diameter": 16}}"#));
    assert_eq!(seen(&s, 310, 160), [1.0, 1.0, 1.0, 1.0]);
    // written on over a second from 1 s: none at first, half way through at 1.5 s
    run(&mut s, "paint.stroke", &format!(r##"{{"layer": {l}, "points": [[0, 190], [400, 190]], "color": "#00ff00", "time": 1, "duration": "writeOn", "drawTime": 1}}"##));
    run(&mut s, "view.set", r#"{"time": 0.5}"#);
    assert_eq!(seen(&s, 100, 190), [1.0, 0.0, 0.0, 1.0], "not yet");
    run(&mut s, "view.set", r#"{"time": 1.5}"#);
    assert_eq!(seen(&s, 100, 190), [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(seen(&s, 300, 190), [1.0, 0.0, 0.0, 1.0], "not that far yet");
    // a single frame
    run(&mut s, "paint.stroke", &format!(r##"{{"layer": {l}, "points": [[20, 20], [60, 20]], "color": "#000000", "duration": "singleFrame"}}"##));
    assert_eq!(seen(&s, 40, 20), [0.0, 0.0, 0.0, 1.0]);
    run(&mut s, "view.set", r#"{"time": 1.6}"#);
    assert_eq!(seen(&s, 40, 20), [1.0, 0.0, 0.0, 1.0], "gone the next frame");
    // another effect after Paint: the next stroke starts a Paint of its own after it
    run(&mut s, "effect.add", &format!(r#"{{"layer": {l}, "type": "invert"}}"#));
    let r = run(&mut s, "paint.stroke", &format!(r#"{{"layer": {l}, "points": [[10, 10], [20, 10]]}}"#));
    assert_eq!(r.get("stroke").and_then(Json::as_str), Some("effects/Paint 2/strokes/Brush 1"));
    // taken away like any group
    run(&mut s, "group.remove", &format!(r#"{{"layer": {l}, "path": "effects/Paint/strokes/Brush 2"}}"#));
    assert!(fails(&mut s, "prop.get", &format!(r#"{{"layer": {l}, "path": "effects/Paint/strokes/Brush 2/options/color"}}"#)).contains("Brush 2"));
    // mistakes
    assert!(fails(&mut s, "paint.stroke", &format!(r#"{{"layer": {l}, "kind": "clone", "points": [[0, 0]]}}"#)).contains("clonePosition"));
    assert!(fails(&mut s, "paint.stroke", &format!(r##"{{"layer": {l}, "kind": "eraser", "color": "#fff", "points": [[0, 0]]}}"##)).contains("brush"));
    assert!(fails(&mut s, "paint.stroke", &format!(r#"{{"layer": {l}, "points": [[0]]}}"#)).contains("point 1"));
    assert!(fails(&mut s, "paint.stroke", &format!(r#"{{"layer": {l}, "tip": "huge", "points": [[0, 0]]}}"#)).contains("paint.tips"));
    assert!(run(&mut s, "paint.tips", "").as_array().is_some_and(|t| t.len() > 10));
}

#[test]
fn puppet_pins_recorded_and_trailing() {
    let (mut s, c, l) = started();
    for at in ["[20, 100]", "[200, 100]", "[380, 100]"] {
        run(&mut s, "puppet.pin", &format!(r#"{{"layer": {l}, "at": {at}}}"#));
    }
    let pos = |s: &mut Session, pin: &str, t: f64| -> Vec<f64> {
        let r = run(s, "prop.evaluate", &format!(r#"{{"comp": {c}, "layer": {l}, "path": "effects/Puppet/meshes/Mesh 1/pins/{pin}/position", "times": [{t}]}}"#));
        r.get("values").and_then(Json::as_array).unwrap()[0].get("value").and_then(Json::as_array).unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
    };
    // the right end dragged up and back over a second, a key a frame; the middle dragged along
    let r = run(&mut s, "puppet.record", &format!(r#"{{"layer": {l}, "pin": "Puppet Pin 3", "pins": ["Puppet Pin 2"], "points": [[0, 380, 100], [0.5, 380, 40], [1, 380, 100]], "smoothing": 0}}"#));
    assert_eq!(r.get("keys").and_then(Json::as_f64), Some(31.0), "{}", r.to_json());
    assert!(near(&pos(&mut s, "Puppet Pin 3", 0.5), &[380.0, 40.0], 1e-6));
    assert!(near(&pos(&mut s, "Puppet Pin 2", 0.5), &[200.0, 40.0], 1e-6), "as far as it, from where it was");
    assert!(near(&pos(&mut s, "Puppet Pin 1", 0.5), &[20.0, 100.0], 1e-6), "the left one stays");
    // the solid bends: its right end up at half a second
    run(&mut s, "view.set", r#"{"time": 0.5}"#);
    assert_eq!(picture(&s).image.color_at(720, 900).to_hex(), "#ff0000");
    // smoothed, played half as fast from 1 s: fewer keys, near the drag
    run(&mut s, "edit.undo", "");
    let r = run(&mut s, "puppet.record", &format!(r#"{{"layer": {l}, "pin": "effects/Puppet/meshes/Mesh 1/pins/Puppet Pin 3", "points": [[0, 380, 100], [0.5, 380, 40], [1, 380, 100]], "start": 1, "speed": 200}}"#));
    assert!(r.get("keys").and_then(Json::as_f64).unwrap() < 10.0, "{}", r.to_json());
    assert_eq!(r.get("end").and_then(Json::as_f64), Some(3.0));
    assert!(near(&pos(&mut s, "Puppet Pin 3", 2.0), &[380.0, 40.0], 1.0));
    // the middle and left trail the right end: the nearest by a tenth of a second, the next by two
    let r = run(&mut s, "puppet.follow", &format!(r#"{{"layer": {l}, "leader": "Puppet Pin 3", "pins": ["Puppet Pin 1", "Puppet Pin 2"]}}"#));
    let delays: Vec<(String, f64)> = r.get("pins").and_then(Json::as_array).unwrap().iter().map(|p| (p.get("pin").and_then(Json::as_str).unwrap().rsplit('/').next().unwrap().to_string(), p.get("delay").and_then(Json::as_f64).unwrap())).collect();
    assert_eq!(delays, [("Puppet Pin 2".to_string(), 0.1), ("Puppet Pin 1".to_string(), 0.2)]);
    let lead = pos(&mut s, "Puppet Pin 3", 1.9);
    assert!(near(&pos(&mut s, "Puppet Pin 2", 2.0), &[200.0, lead[1]], 1e-6), "where the leader was a tenth before");
    let lead = pos(&mut s, "Puppet Pin 3", 1.8);
    assert!(near(&pos(&mut s, "Puppet Pin 1", 2.0), &[20.0, lead[1]], 1e-6));
    // mistakes
    assert!(fails(&mut s, "puppet.record", &format!(r#"{{"layer": {l}, "pin": "Pin 9", "points": [[0, 1, 2]]}}"#)).contains("Puppet Pin 1"));
    assert!(fails(&mut s, "puppet.record", &format!(r#"{{"layer": {l}, "pin": "Puppet Pin 1", "points": [[1, 2]]}}"#)).contains("[t, x, y]"));
    run(&mut s, "puppet.pin", &format!(r#"{{"layer": {l}, "at": [100, 100], "kind": "bend"}}"#));
    assert!(fails(&mut s, "puppet.follow", &format!(r#"{{"layer": {l}, "leader": "Puppet Pin 4", "pins": ["Puppet Pin 1"]}}"#)).contains("bend pin"));
    assert!(fails(&mut s, "puppet.follow", &format!(r#"{{"layer": {l}, "leader": "Puppet Pin 3", "pins": []}}"#)).contains("pins"));
}

/// A 400 × 300 shot ("Shot", a second at 25 frames a second): a still noisy ground, and a noisy
/// patch `size` across moving from (120, 150) to (280, 190), turning `turn` degrees; shown by a
/// layer of composition "Main" the same size, which is returned with the shot's layer.
fn shot(size: f64, turn: f64) -> (Session, u64, u64) {
    let mut s = Session::new();
    run(&mut s, "comp.new", r#"{"name": "Shot", "width": 400, "height": 300, "fps": 25, "duration": 1.2}"#);
    let ground = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#808080", "name": "Ground"}"##), "layer");
    run(&mut s, "effect.add", &format!(r#"{{"layer": {ground}, "type": "fractalNoise", "props": {{"scale": 30, "contrast": 150, "randomSeed": 3}}}}"#));
    let patch = id(&run(&mut s, "layer.add", &format!(r##"{{"type": "solid", "color": "#808080", "name": "Patch", "width": {size}, "height": {size}}}"##)), "layer");
    run(&mut s, "effect.add", &format!(r#"{{"layer": {patch}, "type": "fractalNoise", "props": {{"scale": 12, "contrast": 300, "randomSeed": 9}}}}"#));
    run(&mut s, "prop.keys", &format!(r#"{{"layer": {patch}, "path": "transform/position", "keys": [{{"t": 0, "v": [120, 150, 0]}}, {{"t": 1, "v": [280, 190, 0]}}]}}"#));
    if turn != 0.0 {
        run(&mut s, "prop.keys", &format!(r#"{{"layer": {patch}, "path": "transform/rotation", "keys": [{{"t": 0, "v": 0}}, {{"t": 1, "v": {turn}}}]}}"#));
    }
    let main = id(&run(&mut s, "comp.new", r#"{"name": "Main", "width": 400, "height": 300, "fps": 25, "duration": 1.2}"#), "comp");
    let layer = id(&run(&mut s, "layer.add", r#"{"type": "comp", "item": "Shot"}"#), "layer");
    (s, main, layer)
}

#[test]
fn motion_tracked_applied_and_stabilized() {
    let (mut s, c, l) = shot(60.0, 0.0);
    let value = |s: &mut Session, layer: u64, path: &str, t: f64| -> Vec<f64> {
        let r = run(s, "prop.evaluate", &format!(r#"{{"comp": {c}, "layer": {layer}, "path": "{path}", "times": [{t}]}}"#));
        let v = r.get("values").and_then(Json::as_array).unwrap()[0].get("value").cloned().unwrap();
        v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap()).collect()).unwrap_or_else(|| vec![v.as_f64().unwrap()])
    };
    let r = run(&mut s, "track.new", &format!(r#"{{"layer": {l}, "points": [{{"center": [120, 150], "featureSize": [40, 40], "searchSize": [90, 90]}}]}}"#));
    assert_eq!(r.get("tracker").and_then(Json::as_str), Some("motionTrackers/Tracker 1"));
    let r = run(&mut s, "track.analyze", &format!(r#"{{"layer": {l}, "from": 0, "to": 1}}"#));
    assert_eq!(r.get("frames").and_then(Json::as_f64), Some(26.0), "{}", r.to_json());
    assert!(r.get("lowest").and_then(Json::as_f64).unwrap() > 90.0, "{}", r.to_json());
    let centre = "motionTrackers/Tracker 1/points/Track Point 1/featureCenter";
    for (t, want) in [(0.4, [184.0, 166.0]), (1.0, [280.0, 190.0])] {
        let got = value(&mut s, l, centre, t);
        assert!(near(&got, &want, 0.3), "at {t}: {got:?} {want:?}");
    }
    // a null moved with it
    let null = id(&run(&mut s, "layer.add", r#"{"type": "null", "name": "Follower"}"#), "layer");
    let r = run(&mut s, "track.apply", &format!(r#"{{"layer": {l}, "target": {null}}}"#));
    assert_eq!(r.get("keys").and_then(Json::as_f64), Some(26.0));
    assert!(near(&value(&mut s, null, "transform/position", 0.4)[..2], &[184.0, 166.0], 0.3));
    // only across: down stays where it was
    run(&mut s, "edit.undo", "");
    run(&mut s, "track.apply", &format!(r#"{{"layer": {l}, "target": {null}, "dims": "x"}}"#));
    let p = value(&mut s, null, "transform/position", 1.0);
    assert!((p[0] - 280.0).abs() < 0.3 && p[1] == 150.0, "{p:?}");
    // a point of another layer instead of its position
    let light = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#000000", "name": "Light"}"##), "layer");
    run(&mut s, "effect.add", &format!(r#"{{"layer": {light}, "type": "fractalNoise"}}"#));
    run(&mut s, "track.apply", &format!(r#"{{"layer": {l}, "target": {light}, "targetProperty": "effects/Fractal Noise/offsetTurbulence"}}"#));
    assert!(near(&value(&mut s, light, "effects/Fractal Noise/offsetTurbulence", 1.0), &[280.0, 190.0], 0.3));
    // stabilized: the shot moved so the patch holds still where it started
    run(&mut s, "track.new", &format!(r#"{{"layer": {l}, "kind": "stabilize", "points": [{{"center": [120, 150], "featureSize": [40, 40], "searchSize": [90, 90]}}]}}"#));
    run(&mut s, "track.analyze", &format!(r#"{{"layer": {l}, "tracker": "Tracker 2", "from": 0, "to": 1}}"#));
    run(&mut s, "track.apply", &format!(r#"{{"layer": {l}, "tracker": "Tracker 2"}}"#));
    let held = |s: &Session, t: f64| {
        let (p, tick) = (s.project(), veymelo_time::Tick::from_seconds(t));
        let comp = p.comp(c).unwrap();
        let layer = comp.layer(l).unwrap();
        let f = layer.props.prop("motionTrackers/Tracker 2/points/Track Point 1/featureCenter").unwrap().value_at(layer.layer_time(tick)).as_vec2();
        let e = veymelo_expr::Eval::new(p);
        let q = e.matrix(e.place(comp, layer), tick).apply(veymelo_geom::vec2(f[0], f[1]));
        vec![q.x, q.y]
    };
    for t in [0.3, 0.8] {
        assert!(near(&held(&s, t), &[120.0, 150.0], 1e-6), "at {t}: {:?}", held(&s, t));
    }
    // mistakes
    assert!(fails(&mut s, "track.apply", &format!(r#"{{"layer": {l}, "tracker": "Tracker 1", "target": {l}}}"#)).contains("own track"));
    run(&mut s, "track.new", &format!(r#"{{"layer": {l}, "name": "Fresh"}}"#));
    assert!(fails(&mut s, "track.apply", &format!(r#"{{"layer": {l}, "tracker": "Fresh", "target": {null}}}"#)).contains("track.analyze"));
    assert!(fails(&mut s, "track.new", &format!(r#"{{"layer": {null}}}"#)).contains("nothing moving"));
    assert!(fails(&mut s, "track.new", &format!(r#"{{"layer": {l}, "kind": "perspective", "points": [{{"center": [1, 1]}}]}}"#)).contains("4 track points"));
}

#[test]
fn tracks_turning_and_corner_pins() {
    // a big patch turning 30° as it moves: two points on it give the turn
    let (mut s, c, l) = shot(120.0, 30.0);
    let value = |s: &mut Session, layer: u64, path: &str, t: f64| -> Vec<f64> {
        let r = run(s, "prop.evaluate", &format!(r#"{{"comp": {c}, "layer": {layer}, "path": "{path}", "times": [{t}]}}"#));
        let v = r.get("values").and_then(Json::as_array).unwrap()[0].get("value").cloned().unwrap();
        v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap()).collect()).unwrap_or_else(|| vec![v.as_f64().unwrap()])
    };
    let pts = r#"[{"center": [90, 150], "featureSize": [30, 30], "searchSize": [70, 70]}, {"center": [150, 150], "featureSize": [30, 30], "searchSize": [70, 70]}]"#;
    run(&mut s, "track.new", &format!(r#"{{"layer": {l}, "rotation": true, "scale": true, "points": {pts}}}"#));
    let r = run(&mut s, "track.analyze", &format!(r#"{{"layer": {l}, "from": 0, "to": 1}}"#));
    assert!(r.get("lowest").and_then(Json::as_f64).unwrap() > 85.0, "{}", r.to_json());
    let null = id(&run(&mut s, "layer.add", r#"{"type": "null"}"#), "layer");
    run(&mut s, "track.apply", &format!(r#"{{"layer": {l}, "target": {null}}}"#));
    let turned = value(&mut s, null, "transform/rotation", 1.0)[0];
    let scaled = value(&mut s, null, "transform/scale", 1.0);
    assert!((turned - 30.0).abs() < 0.5, "{turned}");
    assert!((scaled[0] - 100.0).abs() < 1.0, "not grown: {scaled:?}");
    // the first point went round the patch's middle with it
    let p = value(&mut s, null, "transform/position", 1.0);
    let (sn, cs) = 30f64.to_radians().sin_cos();
    assert!(near(&p[..2], &[280.0 - 30.0 * cs, 190.0 - 30.0 * sn], 0.4), "{p:?}");
    // four points on the patch pin another layer's corners
    run(&mut s, "track.new", &format!(r#"{{"layer": {l}, "kind": "perspective", "points": [{}]}}"#, [[95.0, 125.0], [145.0, 125.0], [95.0, 175.0], [145.0, 175.0]].iter().map(|q| format!(r#"{{"center": [{}, {}], "featureSize": [24, 24], "searchSize": [60, 60]}}"#, q[0], q[1])).collect::<Vec<_>>().join(", ")));
    let r = run(&mut s, "track.analyze", &format!(r#"{{"layer": {l}, "tracker": "Tracker 2", "from": 0, "to": 1}}"#));
    assert!(r.get("lowest").and_then(Json::as_f64).unwrap() > 85.0, "{}", r.to_json());
    let sign = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "name": "Sign"}"##), "layer");
    let r = run(&mut s, "track.apply", &format!(r#"{{"layer": {l}, "tracker": "Tracker 2", "target": {sign}}}"#));
    assert!(r.get("paths").unwrap().to_json().contains("effects/Corner Pin/upperLeft"), "{}", r.to_json());
    // the sign fills the composition, so its pixels are the composition's: its upper left pinned
    // to the first point as the patch turned it about its middle
    let ul = value(&mut s, sign, "effects/Corner Pin/upperLeft", 1.0);
    let d = [-25.0, -25.0];
    let want = [280.0 + d[0] * cs - d[1] * sn, 190.0 + d[0] * sn + d[1] * cs];
    assert!(near(&ul, &want, 0.5), "{ul:?} {want:?}");
    // the tracker remembers its target: applied again, the sign gets a second Corner Pin
    let again = run(&mut s, "track.apply", &format!(r#"{{"layer": {l}, "tracker": "Tracker 2"}}"#));
    assert_eq!(again.get("target").and_then(Json::as_u64), Some(sign));
    assert!(again.get("paths").unwrap().to_json().contains("Corner Pin 2"));
}

/// The mean difference of two pictures inside the box from (x0, y0) to (x1, y1).
fn differ(a: &render::Rendered, b: &render::Rendered, [x0, y0, x1, y1]: [usize; 4]) -> f64 {
    let mut sum = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            let (p, q) = (px(a, x, y), px(b, x, y));
            sum += (0..3).map(|c| (p[c] - q[c]).abs() as f64).sum::<f64>() / 3.0;
        }
    }
    sum / ((x1 - x0) * (y1 - y0)) as f64
}

#[test]
fn shaky_footage_held_still_and_rolling_shutter_taken_out() {
    let mut s = Session::new();
    // a big noisy ground shaking about inside a 320 × 240 shot, drifting right
    run(&mut s, "comp.new", r#"{"name": "Shaky", "width": 320, "height": 240, "fps": 25, "duration": 0.6}"#);
    let ground = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#808080", "width": 640, "height": 480}"##), "layer");
    run(&mut s, "effect.add", &format!(r#"{{"layer": {ground}, "type": "fractalNoise", "props": {{"scale": 25, "contrast": 220, "randomSeed": 4}}}}"#));
    let keys: Vec<String> = (0..15).map(|k| format!(r#"{{"t": {}, "v": [{}, {}, 0]}}"#, k as f64 / 25.0, 160.0 + 2.0 * k as f64 + 6.0 * (k as f64 * 2.3).sin(), 120.0 + 5.0 * (k as f64 * 1.9).cos())).collect();
    run(&mut s, "prop.keys", &format!(r#"{{"layer": {ground}, "path": "transform/position", "keys": [{}]}}"#, keys.join(",")));
    let main = id(&run(&mut s, "comp.new", r#"{"name": "Main", "width": 320, "height": 240, "fps": 25, "duration": 0.6}"#), "comp");
    let shot = id(&run(&mut s, "layer.add", r#"{"type": "comp", "item": "Shaky"}"#), "layer");
    run(&mut s, "effect.add", &format!(r#"{{"layer": {shot}, "type": "warpStabilizer", "props": {{"result": "noMotion", "framing": "stabilizeOnly"}}}}"#));
    let still = picture(&s);
    assert!(still.notes.iter().any(|n| n.contains("effect.analyze")), "{:?}", still.notes);
    let r = run(&mut s, "effect.analyze", &format!(r#"{{"layer": {shot}}}"#));
    assert_eq!(r.get("frames").and_then(Json::as_f64), Some(15.0), "{}", r.to_json());
    assert!(r.get("agreed").and_then(Json::as_f64).unwrap() > 90.0, "{}", r.to_json());
    let frame = |s: &Session, t: f64| s.render(Some(&Json::Number(main as f64)), Some(veymelo_time::Tick::from_seconds(t)), &render::Options::default()).unwrap();
    let middle = [80, 60, 240, 180];
    let first = frame(&s, 0.0);
    // held still: later frames look like the first where they overlap
    for t in [0.2, 0.44] {
        let held = differ(&first, &frame(&s, t), middle);
        assert!(held < 0.01, "at {t}: {held}");
    }
    run(&mut s, "effect.set", &format!(r#"{{"layer": {shot}, "effect": "Warp Stabilizer", "enabled": false}}"#));
    let shaking = differ(&first, &frame(&s, 0.2), middle);
    assert!(shaking > 0.05, "{shaking}");
    run(&mut s, "effect.set", &format!(r#"{{"layer": {shot}, "effect": "Warp Stabilizer", "enabled": true}}"#));
    // smoothed rather than held: the drift stays, and cropped and scaled the edges never show
    run(&mut s, "prop.set", &format!(r#"{{"layer": {shot}, "path": "effects/Warp Stabilizer/result", "value": "smoothMotion"}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {shot}, "path": "effects/Warp Stabilizer/framing", "value": "stabilizeCropAutoScale"}}"#));
    for t in [0.0, 0.2, 0.56] {
        let f = frame(&s, t);
        assert!(f.image.px.iter().all(|p| p[3] > 0.99), "at {t} every pixel is filled");
    }

    // a camera reading its picture a row at a time while the ground slides right 8 pixels a
    // frame: the rows slanted, the lower further on
    let rolling = id(&run(&mut s, "comp.new", r#"{"name": "Rolling", "width": 320, "height": 240, "fps": 25, "duration": 0.6}"#), "comp");
    let wide = id(&run(&mut s, "layer.add", &format!(r##"{{"comp": {rolling}, "type": "solid", "color": "#808080", "width": 800, "height": 240}}"##)), "layer");
    run(&mut s, "effect.add", &format!(r#"{{"comp": {rolling}, "layer": {wide}, "type": "fractalNoise", "props": {{"scale": 25, "contrast": 220, "randomSeed": 6}}}}"#));
    run(&mut s, "prop.keys", &format!(r#"{{"comp": {rolling}, "layer": {wide}, "path": "transform/position", "keys": [{{"t": 0, "v": [400, 120, 0]}}, {{"t": 0.56, "v": [512, 120, 0]}}]}}"#));
    let straight = id(&run(&mut s, "comp.new", r#"{"name": "Straight", "width": 320, "height": 240, "fps": 25, "duration": 0.6}"#), "comp");
    let plain = id(&run(&mut s, "layer.add", &format!(r#"{{"comp": {straight}, "type": "comp", "item": "Rolling"}}"#)), "layer");
    let _ = plain;
    // the slant: half a frame's reading time, 8 pixels a frame: 4 across the height
    run(&mut s, "effect.add", &format!(r#"{{"comp": {rolling}, "layer": {wide}, "type": "cornerPin", "props": {{"upperLeft": [-2, 0], "upperRight": [798, 0], "lowerLeft": [2, 240], "lowerRight": [802, 240]}}}}"#));
    let fixed = id(&run(&mut s, "comp.new", r#"{"name": "Fixed", "width": 320, "height": 240, "fps": 25, "duration": 0.6}"#), "comp");
    let slanted = id(&run(&mut s, "layer.add", &format!(r#"{{"comp": {fixed}, "type": "comp", "item": "Rolling"}}"#)), "layer");
    run(&mut s, "effect.add", &format!(r#"{{"comp": {fixed}, "layer": {slanted}, "type": "rollingShutterRepair"}}"#));
    run(&mut s, "effect.analyze", &format!(r#"{{"comp": {fixed}, "layer": {slanted}}}"#));
    let draw = |s: &Session, c: u64| s.render(Some(&Json::Number(c as f64)), Some(veymelo_time::Tick::from_seconds(0.24)), &render::Options::default()).unwrap();
    // against the ground drawn without the slant (the same comp with its Corner Pin off)
    let repaired = draw(&s, fixed);
    run(&mut s, "effect.set", &format!(r#"{{"comp": {fixed}, "layer": {slanted}, "effect": "Rolling Shutter Repair", "enabled": false}}"#));
    let slant = draw(&s, fixed);
    run(&mut s, "effect.set", &format!(r#"{{"comp": {rolling}, "layer": {wide}, "effect": "Corner Pin", "enabled": false}}"#));
    let truth = draw(&s, straight);
    let box_ = [40, 20, 280, 220];
    let (after, before) = (differ(&repaired, &truth, box_), differ(&slant, &truth, box_));
    assert!(after < 0.4 * before, "repaired {after}, slanted {before}");
}

#[test]
fn a_mask_follows_what_is_inside_it() {
    let (mut s, c, l) = shot(120.0, 30.0);
    run(&mut s, "mask.add", &format!(r#"{{"layer": {l}, "rect": [90, 120, 60, 60]}}"#));
    let r = run(&mut s, "mask.track", &format!(r##"{{"layer": {l}, "mask": "#1", "from": 0, "to": 1}}"##));
    assert_eq!(r.get("frames").and_then(Json::as_f64), Some(26.0), "{}", r.to_json());
    let shape = |s: &mut Session, t: f64| {
        let r = run(s, "prop.evaluate", &format!(r#"{{"comp": {c}, "layer": {l}, "path": "masks/Mask 1/path", "times": [{t}]}}"#));
        let v = r.get("values").and_then(Json::as_array).unwrap()[0].get("value").cloned().unwrap();
        v.get("points").and_then(Json::as_array).unwrap().iter().map(|p| p.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect::<Vec<_>>()).collect::<Vec<_>>()
    };
    // the mask's corners went round the patch's middle with it, as the patch moved and turned
    let (sn, cs) = 30f64.to_radians().sin_cos();
    let corners = shape(&mut s, 1.0);
    for (got, d) in corners.iter().zip([[-30.0, -30.0], [30.0, -30.0], [30.0, 30.0], [-30.0, 30.0]]) {
        let want = [280.0 + d[0] * cs - d[1] * sn, 190.0 + d[0] * sn + d[1] * cs];
        assert!(near(got, &want, 1.0), "{got:?} {want:?}");
    }
    // moved only: its corners keep their square
    run(&mut s, "edit.undo", "");
    run(&mut s, "mask.track", &format!(r##"{{"layer": {l}, "mask": "Mask 1", "method": "position", "from": 0, "to": 0.4}}"##));
    let moved = shape(&mut s, 0.4);
    assert!((moved[1][0] - moved[0][0] - 60.0).abs() < 1e-6 && (moved[1][1] - moved[0][1]).abs() < 1e-6, "{moved:?}");
    assert!(fails(&mut s, "mask.track", &format!(r#"{{"layer": {l}, "mask": "Nope"}}"#)).contains("Mask 1"));
}

/// Bytes as base64.
fn base64_of(bytes: &[u8]) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            out.push(if k <= c.len() { ABC[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// A glTF file of a red cube a unit wide, spinning a quarter turn about its up over a second,
/// with a camera five units in front of it and a light above.
fn cube_file(name: &str) -> String {
    let mut pos = Vec::new();
    let mut nor = Vec::new();
    let mut idx: Vec<u16> = Vec::new();
    for (n, u, v) in [([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]), ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]), ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])] {
        for s in [1.0f32, -1.0] {
            let base = (pos.len() / 3) as u16;
            let n: [f32; 3] = n.map(|x: f32| x * s);
            let (u, v): ([f32; 3], [f32; 3]) = if s > 0.0 { (u, v) } else { (v, u) };
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                for k in 0..3 {
                    pos.push(0.5 * n[k] + 0.5 * a * u[k] + 0.5 * b * v[k]);
                    nor.push(n[k]);
                }
            }
            idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    let mut bin = Vec::new();
    pos.iter().for_each(|v| bin.extend(v.to_le_bytes()));
    nor.iter().for_each(|v| bin.extend(v.to_le_bytes()));
    idx.iter().for_each(|v| bin.extend(v.to_le_bytes()));
    let s = std::f32::consts::FRAC_1_SQRT_2;
    for v in [0.0f32, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, s, 0.0, s] {
        bin.extend(v.to_le_bytes());
    }
    let (p, n, i) = (pos.len() * 4, nor.len() * 4, idx.len() * 2);
    let json = format!(
        r#"{{"asset": {{"version": "2.0"}}, "scene": 0, "scenes": [{{"nodes": [0, 1, 2]}}],
        "nodes": [{{"name": "Cube", "mesh": 0}}, {{"name": "Eye", "camera": 0, "translation": [0, 0, 5]}},
                  {{"name": "Sun", "translation": [0, 3, 0], "rotation": [-{s}, 0, 0, {s}], "extensions": {{"KHR_lights_punctual": {{"light": 0}}}}}}],
        "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0, "NORMAL": 1}}, "indices": 2, "material": 0}}]}}],
        "materials": [{{"name": "Red", "pbrMetallicRoughness": {{"baseColorFactor": [0.8, 0.05, 0.05, 1], "metallicFactor": 0, "roughnessFactor": 0.6}}}}],
        "cameras": [{{"type": "perspective", "perspective": {{"yfov": 0.6, "znear": 0.1}}}}],
        "extensions": {{"KHR_lights_punctual": {{"lights": [{{"type": "directional", "color": [1, 1, 1], "intensity": 2}}]}}}},
        "animations": [{{"name": "Spin", "samplers": [{{"input": 3, "output": 4}}], "channels": [{{"sampler": 0, "target": {{"node": 0, "path": "rotation"}}}}]}}],
        "accessors": [
            {{"bufferView": 0, "componentType": 5126, "count": 24, "type": "VEC3", "min": [-0.5, -0.5, -0.5], "max": [0.5, 0.5, 0.5]}},
            {{"bufferView": 1, "componentType": 5126, "count": 24, "type": "VEC3"}},
            {{"bufferView": 2, "componentType": 5123, "count": 36, "type": "SCALAR"}},
            {{"bufferView": 3, "componentType": 5126, "count": 2, "type": "SCALAR"}},
            {{"bufferView": 4, "componentType": 5126, "count": 2, "type": "VEC4"}}],
        "bufferViews": [{{"buffer": 0, "byteLength": {p}}}, {{"buffer": 0, "byteOffset": {p}, "byteLength": {n}}}, {{"buffer": 0, "byteOffset": {pn}, "byteLength": {i}}},
                        {{"buffer": 0, "byteOffset": {pni}, "byteLength": 8}}, {{"buffer": 0, "byteOffset": {pni8}, "byteLength": 32}}],
        "buffers": [{{"byteLength": {len}, "uri": "data:application/octet-stream;base64,{data}"}}]}}"#,
        pn = p + n,
        pni = p + n + i,
        pni8 = p + n + i + 8,
        len = bin.len(),
        data = base64_of(&bin),
    );
    let dir = std::env::temp_dir().join(format!("veymelo-engine-models-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, json).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_3d_model_imported_lit_animated_and_its_camera_and_light_made_layers() {
    let mut s = Session::new();
    let c = id(&run(&mut s, "comp.new", r#"{"name": "Main", "width": 400, "height": 300, "fps": 25, "duration": 2}"#), "comp");
    let file = cube_file("cube.gltf");
    let item = run(&mut s, "footage.import", &format!(r#"{{"path": {}}}"#, Json::from(file.as_str()).to_json()));
    assert_eq!(item.get("kind").and_then(Json::as_str), Some("model"), "{}", item.to_json());
    assert_eq!(item.get("duration").and_then(Json::as_f64), Some(1.0), "the length of its animation");
    let info = run(&mut s, "model.info", &format!(r#"{{"item": {}}}"#, id(&item, "item")));
    assert_eq!(info.get("triangles").and_then(Json::as_f64), Some(12.0), "{}", info.to_json());
    assert_eq!(info.get("size").unwrap().to_json(), "[1,1,1]");
    assert_eq!(info.get("animations").unwrap().to_json(), r#"[{"index":1,"name":"Spin","length":1}]"#);
    let l = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {}}}"#, id(&item, "item"))), "layer");
    let layer = s.project().comp(c).unwrap().layer(l).unwrap();
    assert!(matches!(layer.source, veymelo_project::Source::Model(_)) && layer.is_3d());
    assert_eq!(layer.props.prop("model/unitScale").unwrap().value.as_f64(), 150.0, "a unit as half the height");
    // its front face square in the middle, red and lit; the corners empty
    let frame = |s: &Session, t: f64| s.render(Some(&Json::Number(c as f64)), Some(veymelo_time::Tick::from_seconds(t)), &render::Options { background: false, ..render::Options::default() }).unwrap();
    let f = frame(&s, 0.0);
    assert!(f.notes.is_empty(), "{:?}", f.notes);
    let mid = px(&f, 200, 150);
    assert!(mid[3] > 0.99 && mid[0] > 0.3 && mid[0] > 2.0 * mid[1], "{mid:?}");
    assert_eq!(px(&f, 10, 10)[3], 0.0);
    let red_across = |f: &render::Rendered| (0..400).filter(|x| px(f, *x, 150)[3] > 0.5).count();
    let square = red_across(&f);
    // its front face half a unit nearer than its middle, so seen bigger: 150 × zoom / (zoom − 75)
    let zoom = 400.0 * 50.0 / 36.0;
    assert!((square as f64 - 150.0 * zoom / (zoom - 75.0)).abs() < 2.0, "{square}");
    // turned: three faces seen, each lit differently
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "transform/rotationY", "value": 35}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "transform/rotationX", "value": -25}}"#));
    let f = frame(&s, 0.0);
    if let Ok(dir) = std::env::var("VEYMELO_LOOK") {
        std::fs::write(format!("{dir}/cube-turned.png"), render::to_png(&f.image)).unwrap();
    }
    // tipped forward it shows its front, its right and the face underneath
    let (front, right, under) = (px(&f, 140, 110), px(&f, 270, 160), px(&f, 200, 220));
    assert!(front[3] > 0.99 && right[3] > 0.99 && under[3] > 0.99, "{front:?} {right:?} {under:?}");
    let shade = |p: [f32; 4]| p[0];
    assert!(shade(front) > shade(right) + 0.1 && (shade(right) - shade(under)).abs() > 0.02, "faces lit differently: {front:?} {right:?} {under:?}");
    run(&mut s, "edit.undo", "");
    run(&mut s, "edit.undo", "");
    // a flat layer in front hides it; behind, it is hidden
    let wall = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#0000ff", "width": 200, "height": 300}"##), "layer");
    run(&mut s, "layer.set", &format!(r#"{{"layer": {wall}, "switches": {{"threeD": true}}}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {wall}, "path": "transform/position", "value": [100, 150, -200]}}"#));
    let f = frame(&s, 0.0);
    assert!(px(&f, 170, 150)[2] > 0.9 && px(&f, 170, 150)[0] < 0.1, "the wall in front: {:?}", px(&f, 170, 150));
    assert!(px(&f, 230, 150)[0] > 0.3, "the cube beside it");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {wall}, "path": "transform/position", "value": [100, 150, 300]}}"#));
    let f = frame(&s, 0.0);
    let p = px(&f, 170, 150);
    assert!(p[0] > 0.3 && (p[2] - p[1]).abs() < 0.02, "the cube's red (no blue of the wall) in front of it: {p:?}");
    run(&mut s, "layer.remove", &format!(r#"{{"layer": {wall}}}"#));
    // its animation: half way through the spin it is turned an eighth, its corner toward us, wider
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "model/animation", "value": 1}}"#));
    let turned = red_across(&frame(&s, 0.5));
    // its side corners at its middle's depth: as wide as its diagonal, 150√2
    assert!((turned as f64 - 150.0 * std::f64::consts::SQRT_2).abs() < 3.0, "{turned} {square}");
    assert_eq!(red_across(&frame(&s, 1.0)), red_across(&frame(&s, 0.0)), "a quarter turn of a cube looks as it started (looping)");
    // its camera and light made layers where they are
    let r = run(&mut s, "camera.fromModel", &format!(r#"{{"layer": {l}, "time": 0}}"#));
    let cam = r.get("layers").and_then(Json::as_array).unwrap()[0].as_u64().unwrap();
    let get = |s: &Session, layer: u64, path: &str| s.project().comp(c).unwrap().layer(layer).unwrap().props.prop(path).unwrap().value.as_vec3();
    // five units in front of the cube's middle (at the composition's middle): 750 pixels nearer
    let eye = get(&s, cam, "transform/position");
    assert!(near(&eye, &[200.0, 150.0, -750.0], 1e-6), "{eye:?}");
    let zoom = s.project().comp(c).unwrap().layer(cam).unwrap().props.prop("cameraOptions/zoom").unwrap().value.as_f64();
    assert!((zoom - 150.0 / 0.3f64.tan()).abs() < 1e-6, "{zoom}");
    let r = run(&mut s, "light.fromModel", &format!(r#"{{"layer": {l}}}"#));
    let sun = r.get("layers").and_then(Json::as_array).unwrap()[0].as_u64().unwrap();
    assert!(near(&get(&s, sun, "transform/position"), &[200.0, 150.0 - 450.0, 0.0], 1e-6), "three units up: 450 pixels");
    let poi = get(&s, sun, "transform/pointOfInterest");
    assert!(poi[1] > 0.0 && (poi[0] - 200.0).abs() < 1e-6, "shining down: {poi:?}");
    // the project with its model kept and read back
    let text = s.project().to_text();
    let back = project::Project::from_text(&text, &catalog()).unwrap();
    assert_eq!(&back, s.project());
}

/// A glTF file `name` from its JSON (where `BUFFER` stands for its buffer) and buffer bytes.
fn gltf_file(name: &str, json: &str, bin: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("veymelo-engine-models-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let buffer = format!(r#"{{"byteLength": {}, "uri": "data:application/octet-stream;base64,{}"}}"#, bin.len(), base64_of(bin));
    std::fs::write(&path, json.replace("BUFFER", &buffer)).unwrap();
    path.to_string_lossy().into_owned()
}

fn floats_le(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[test]
fn skinned_and_morphed_models_drawn_bent() {
    // a strip two units tall standing on its root joint; a second joint at its middle turns a
    // quarter (toward the left) over a second, bending its top half over
    let pos = floats_le(&[-0.1, 0.0, 0.0, 0.1, 0.0, 0.0, -0.1, 1.0, 0.0, 0.1, 1.0, 0.0, -0.1, 2.0, 0.0, 0.1, 2.0, 0.0]);
    let nor = floats_le(&[0.0, 0.0, 1.0].repeat(6));
    let joints: Vec<u8> = [0u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0].to_vec();
    let weights = floats_le(&[1.0, 0.0, 0.0, 0.0].repeat(6));
    let idx: Vec<u8> = [0u16, 1, 3, 0, 3, 2, 2, 3, 5, 2, 5, 4].iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut ibm = Vec::new();
    for m in [[1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 1.0]] {
        ibm.extend(floats_le(&m));
    }
    let s = std::f32::consts::FRAC_1_SQRT_2;
    let times = floats_le(&[0.0, 1.0]);
    let turns = floats_le(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, s, s]);
    let parts = [pos, nor, joints, weights, idx, ibm, times, turns];
    let mut bin = Vec::new();
    let mut views = Vec::new();
    for p in &parts {
        views.push(format!(r#"{{"buffer": 0, "byteOffset": {}, "byteLength": {}}}"#, bin.len(), p.len()));
        bin.extend(p);
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
    }
    let json = format!(
        r#"{{"asset": {{"version": "2.0"}}, "scenes": [{{"nodes": [0, 1]}}],
        "nodes": [{{"name": "Strip", "mesh": 0, "skin": 0}}, {{"name": "Root", "children": [2]}}, {{"name": "Middle", "translation": [0, 1, 0]}}],
        "skins": [{{"joints": [1, 2], "inverseBindMatrices": 5}}],
        "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0, "NORMAL": 1, "JOINTS_0": 2, "WEIGHTS_0": 3}}, "indices": 4}}]}}],
        "animations": [{{"samplers": [{{"input": 6, "output": 7}}], "channels": [{{"sampler": 0, "target": {{"node": 2, "path": "rotation"}}}}]}}],
        "accessors": [
            {{"bufferView": 0, "componentType": 5126, "count": 6, "type": "VEC3", "min": [-0.1, 0, 0], "max": [0.1, 2, 0]}},
            {{"bufferView": 1, "componentType": 5126, "count": 6, "type": "VEC3"}},
            {{"bufferView": 2, "componentType": 5121, "count": 6, "type": "VEC4"}},
            {{"bufferView": 3, "componentType": 5126, "count": 6, "type": "VEC4"}},
            {{"bufferView": 4, "componentType": 5123, "count": 12, "type": "SCALAR"}},
            {{"bufferView": 5, "componentType": 5126, "count": 2, "type": "MAT4"}},
            {{"bufferView": 6, "componentType": 5126, "count": 2, "type": "SCALAR"}},
            {{"bufferView": 7, "componentType": 5126, "count": 2, "type": "VEC4"}}],
        "bufferViews": [{}], "buffers": [BUFFER]}}"#,
        views.join(", ")
    );
    let file = gltf_file("strip.gltf", &json, &bin);
    let mut s = Session::new();
    let c = id(&run(&mut s, "comp.new", r#"{"name": "Main", "width": 400, "height": 400, "fps": 25, "duration": 2}"#), "comp");
    let item = id(&run(&mut s, "footage.import", &format!(r#"{{"path": {}}}"#, Json::from(file.as_str()).to_json())), "item");
    let l = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {item}}}"#)), "layer");
    // a unit 100 pixels: the strip from 100 above the middle to 100 below it, standing
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "model/unitScale", "value": 100}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "model/animation", "value": 1}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "model/loop", "value": false}}"#));
    let frame = |s: &Session, t: f64| s.render(Some(&Json::Number(c as f64)), Some(veymelo_time::Tick::from_seconds(t)), &render::Options { background: false, ..render::Options::default() }).unwrap();
    // the model's middle (a unit up its strip) at the composition's middle; a pixel there is
    // seen a little bigger than a unit's hundredth, the strip standing in front of z = 0
    let still = frame(&s, 0.0);
    assert!(px(&still, 200, 150)[3] > 0.5, "standing: its top half above the middle");
    assert_eq!(px(&still, 140, 200)[3], 0.0, "nothing to the left");
    let bent = frame(&s, 1.0);
    assert_eq!(px(&bent, 200, 140)[3], 0.0, "bent over: nothing above any more");
    assert!(px(&bent, 140, 200)[3] > 0.5, "its top half lies to the left of the middle");
    assert!(px(&bent, 200, 250)[3] > 0.5, "the bottom half still stands");

    // a triangle whose morph target pulls its top corner a unit to the right, fully weighted
    let mut bin = floats_le(&[-0.5, -0.5, 0.0, 0.5, -0.5, 0.0, 0.0, 0.5, 0.0]);
    bin.extend(floats_le(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    let json = r#"{"asset": {"version": "2.0"}, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0, "weights": [1]}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "targets": [{"POSITION": 1}]}], "weights": [0]}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-0.5, -0.5, 0], "max": [0.5, 0.5, 0]},
                      {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"}],
        "bufferViews": [{"buffer": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 36}], "buffers": [BUFFER]}"#;
    let file = gltf_file("morph.gltf", json, &bin);
    let item = id(&run(&mut s, "footage.import", &format!(r#"{{"path": {}}}"#, Json::from(file.as_str()).to_json())), "item");
    run(&mut s, "layer.remove", &format!(r#"{{"layer": {l}}}"#));
    let t = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {item}}}"#)), "layer");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {t}, "path": "model/unitScale", "value": 100}}"#));
    // the triangle's middle at the composition's: its top corner a unit up-right of where it was
    let f = frame(&s, 0.0);
    assert!(px(&f, 290, 165)[3] > 0.5, "the corner pulled right");
    // a quarter unit up, the triangle once lay across the middle; pulled over, it lies right of it
    assert_eq!(px(&f, 200, 175)[3], 0.0, "not where it was");
}

#[test]
fn models_cast_shadows_and_blur_out_of_focus() {
    let mut s = Session::new();
    let c = id(&run(&mut s, "comp.new", r#"{"name": "Main", "width": 400, "height": 300, "fps": 25, "duration": 2}"#), "comp");
    let item = id(&run(&mut s, "footage.import", &format!(r#"{{"path": {}}}"#, Json::from(cube_file("shade.gltf").as_str()).to_json())), "item");
    // a floor under the cube, a light straight above it
    let floor = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#c0c0c0", "width": 1200, "height": 1200}"##), "layer");
    run(&mut s, "layer.set", &format!(r#"{{"layer": {floor}, "switches": {{"threeD": true}}}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {floor}, "path": "transform/rotationX", "value": 90}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {floor}, "path": "transform/position", "value": [200, 230, 0]}}"#));
    let cube = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {item}}}"#)), "layer");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {cube}, "path": "model/unitScale", "value": 100}}"#));
    let lamp = id(&run(&mut s, "layer.add", r#"{"type": "light", "light": "point"}"#), "layer");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {lamp}, "path": "transform/position", "value": [200, -400, 0]}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {lamp}, "path": "lightOptions/castsShadows", "value": true}}"#));
    let cam = id(&run(&mut s, "layer.add", r#"{"type": "camera"}"#), "layer");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {cam}, "path": "transform/position", "value": [200, -150, -600]}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {cam}, "path": "transform/pointOfInterest", "value": [200, 180, 0]}}"#));
    let frame = |s: &Session| s.render(Some(&Json::Number(c as f64)), None, &render::Options { background: false, ..render::Options::default() }).unwrap();
    // the floor beside the cube's foot, where its shadow spreads wider than it
    let lit = frame(&s);
    let spot = (150, 205);
    run(&mut s, "prop.set", &format!(r#"{{"layer": {cube}, "path": "materialOptions/castsShadows", "value": "on"}}"#));
    let shaded = frame(&s);
    let (a, b) = (px(&lit, spot.0, spot.1), px(&shaded, spot.0, spot.1));
    assert!(b[0] < a[0] * 0.8, "the cube's shadow on the floor: {a:?} → {b:?}");
    // its shadow alone: the cube itself not drawn, its shadow still there
    run(&mut s, "prop.set", &format!(r#"{{"layer": {cube}, "path": "materialOptions/castsShadows", "value": "only"}}"#));
    let only = frame(&s);
    assert!((px(&only, spot.0, spot.1)[0] - b[0]).abs() < 0.02, "the shadow stays");
    assert!(px(&only, 200, 150)[0] > 0.6 && (px(&only, 200, 150)[0] - px(&only, 200, 150)[2]).abs() < 0.05, "no red cube, the grey floor behind: {:?}", px(&only, 200, 150));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {cube}, "path": "materialOptions/castsShadows", "value": "off"}}"#));
    run(&mut s, "layer.remove", &format!(r#"{{"layer": {floor}}}"#));
    // out of focus: its edges soften
    let sharp = frame(&s);
    run(&mut s, "prop.set", &format!(r#"{{"layer": {cam}, "path": "cameraOptions/depthOfField", "value": true}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {cam}, "path": "cameraOptions/focusDistance", "value": 2000}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {cam}, "path": "cameraOptions/aperture", "value": 60}}"#));
    let soft = frame(&s);
    let partly = |f: &render::Rendered| (0..400).filter(|x| (0.05..0.95).contains(&px(f, *x, 150)[3])).count();
    assert!(partly(&soft) > partly(&sharp) + 10, "{} {}", partly(&soft), partly(&sharp));
}

#[test]
fn text_and_shapes_made_solid() {
    let mut s = Session::new();
    let c = id(&run(&mut s, "comp.new", r#"{"name": "Main", "width": 400, "height": 300, "fps": 25, "duration": 2}"#), "comp");
    // a bar 40 wide and 160 tall, standing turned most of the way round
    let bar = id(&run(&mut s, "layer.add", r#"{"type": "shape"}"#), "layer");
    run(&mut s, "shape.add", &format!(r#"{{"layer": {bar}, "type": "rect", "props": {{"size": [40, 160]}}}}"#));
    run(&mut s, "shape.add", &format!(r##"{{"layer": {bar}, "type": "fill", "props": {{"color": "#ff0000"}}}}"##));
    run(&mut s, "layer.set", &format!(r#"{{"layer": {bar}, "switches": {{"threeD": true}}}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {bar}, "path": "transform/position", "value": [200, 150, 0]}}"#));
    run(&mut s, "prop.set", &format!(r#"{{"layer": {bar}, "path": "transform/rotationY", "value": 75}}"#));
    let frame = |s: &Session| s.render(Some(&Json::Number(c as f64)), None, &render::Options { background: false, ..render::Options::default() }).unwrap();
    let across = |f: &render::Rendered| (0..400).filter(|x| px(f, *x, 150)[3] > 0.5).count();
    let flat = frame(&s);
    // made solid 120 deep, its side shows: wider, and lit (shaded, not its flat red)
    run(&mut s, "prop.set", &format!(r#"{{"layer": {bar}, "path": "geometryOptions/extrusionDepth", "value": 120}}"#));
    let solid = frame(&s);
    assert!(across(&solid) > across(&flat) + 60, "{} {}", across(&solid), across(&flat));
    // its side coloured as its edge, lit
    let side = (0..400).map(|x| px(&solid, x, 150)).filter(|p| p[3] > 0.99).map(|p| p[0]).fold(0.0f32, f32::max);
    assert!(side > 0.3, "{side}");
    // bevelled in all three styles
    for style in ["angular", "concave", "convex"] {
        run(&mut s, "prop.set", &format!(r#"{{"layer": {bar}, "path": "geometryOptions/bevelStyle", "value": "{style}"}}"#));
        run(&mut s, "prop.set", &format!(r#"{{"layer": {bar}, "path": "geometryOptions/bevelDepth", "value": 8}}"#));
        let b = frame(&s);
        // the bevel takes off the corner nearest the camera: a little narrower, no more
        assert!(b.notes.is_empty() && (across(&b) as f64 - across(&solid) as f64).abs() < 14.0, "{style}: {} {}", across(&b), across(&solid));
    }
    // a text layer has the same settings
    let t = id(&run(&mut s, "layer.add", r#"{"type": "text", "text": "O"}"#), "layer");
    assert!(run(&mut s, "prop.get", &format!(r#"{{"layer": {t}, "path": "geometryOptions/holeBevelDepth"}}"#)).get("value").and_then(Json::as_f64) == Some(100.0));
}

#[test]
fn primitives_drawn_as_solids() {
    let mut s = Session::new();
    let c = id(&run(&mut s, "comp.new", r#"{"name": "Main", "width": 400, "height": 300, "fps": 25, "duration": 2}"#), "comp");
    let ball = id(&run(&mut s, "layer.newPrimitive", r##"{"shape": "sphere", "radius": 80, "baseColor": "#ff0000", "position": [200, 150, 0]}"##), "layer");
    let frame = |s: &Session| s.render(Some(&Json::Number(c as f64)), None, &render::Options { background: false, ..render::Options::default() }).unwrap();
    let across = |f: &render::Rendered| (0..400).filter(|x| px(f, *x, 150)[3] > 0.5).count();
    let f = frame(&s);
    // a red ball 160 across in the middle (a little more: its front is nearer the camera)
    let mid = px(&f, 200, 150);
    assert!(mid[3] > 0.99 && mid[0] > mid[1] + 0.5 && (mid[1] - mid[2]).abs() < 0.01, "{mid:?}");
    assert!((158..172).contains(&across(&f)), "{}", across(&f));
    assert_eq!(px(&f, 10, 10)[3], 0.0);
    // bigger; then blue
    run(&mut s, "prop.set", &format!(r#"{{"layer": {ball}, "path": "geometryOptions/radius", "value": 120}}"#));
    assert!(across(&frame(&s)) > 230, "{}", across(&frame(&s)));
    run(&mut s, "material.set", &format!(r##"{{"layers": [{ball}], "baseColor": "#0000ff", "roughness": 20}}"##));
    let blue = px(&frame(&s), 200, 150);
    assert!(blue[2] > blue[0] + 0.3, "{blue:?}");
    // a cube beside it given the ball's material, then its own again
    let cube = id(&run(&mut s, "layer.newPrimitive", r#"{"shape": "cube", "width": 60, "height": 60, "depth": 60, "position": [40, 40, 0]}"#), "layer");
    let grey = px(&frame(&s), 40, 40);
    assert!(grey[3] > 0.99 && (grey[0] - grey[2]).abs() < 0.02, "{grey:?}");
    run(&mut s, "material.duplicateAssign", &format!(r#"{{"layers": [{ball}, {cube}]}}"#));
    let taken = px(&frame(&s), 40, 40);
    assert!(taken[2] > taken[0] + 0.1, "the ball's blue: {taken:?}");
    run(&mut s, "material.reset", &format!(r#"{{"layers": [{cube}]}}"#));
    assert!((px(&frame(&s), 40, 40)[0] - grey[0]).abs() < 0.01, "grey again");
    // what a shape has not
    assert!(fails(&mut s, "layer.newPrimitive", r#"{"shape": "cube", "tubeRadius": 5}"#).contains("a cube has no tubeRadius"));
    assert!(fails(&mut s, "material.set", &format!(r#"{{"layers": [{cube}]}}"#)).contains("none of the layers"));
    // every solid draws
    for (k, shape) in ["plane", "torus", "cone", "cylinder"].iter().enumerate() {
        let l = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "primitive", "shape": "{shape}"}}"#)), "layer");
        run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "transform/position", "value": [{}, 260, 0]}}"#, 60 + 90 * k));
        run(&mut s, "prop.set", &format!(r#"{{"layer": {l}, "path": "transform/scale", "value": [15, 15, 15]}}"#));
    }
    let all = frame(&s);
    for k in 0..4 {
        assert!(px(&all, 60 + 90 * k, 260)[3] > 0.5, "solid {k} drawn");
    }
    // kept in the project file and read back the same
    let text = s.project().to_text();
    assert!(text.contains(r#""type": "primitive""#) && text.contains(r#""shape": "torus""#), "{text}");
    assert_eq!(&veymelo_project::Project::from_text(&text, &s.catalog).unwrap(), s.project());
}

#[test]
fn an_environment_lights_the_scene() {
    let mut s = Session::new();
    // a panorama: blue on its left half, red on its right
    let pano = id(&run(&mut s, "comp.new", r#"{"name": "Pano", "width": 200, "height": 100, "fps": 25, "duration": 2}"#), "comp");
    let blue = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#0000ff", "width": 100, "height": 100}"##), "layer");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {blue}, "path": "transform/position", "value": [50, 50, 0]}}"#));
    let red = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ff0000", "width": 100, "height": 100}"##), "layer");
    run(&mut s, "prop.set", &format!(r#"{{"layer": {red}, "path": "transform/position", "value": [150, 50, 0]}}"#));
    let c = id(&run(&mut s, "comp.new", r#"{"name": "Main", "width": 400, "height": 300, "fps": 25, "duration": 2}"#), "comp");
    let ball = id(&run(&mut s, "layer.newPrimitive", r##"{"shape": "sphere", "radius": 100, "baseColor": "#ffffff", "roughness": 100, "position": [200, 150, 0]}"##), "layer");
    let env = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "comp", "item": {pano}}}"#)), "layer");
    run(&mut s, "layer.environment", &format!(r#"{{"layers": [{env}]}}"#));
    let light = id(&run(&mut s, "layer.add", r#"{"type": "light", "light": "environment"}"#), "layer");
    let frame = |s: &Session| s.render(Some(&Json::Number(c as f64)), None, &render::Options { background: false, ..render::Options::default() }).unwrap();
    let f = frame(&s);
    // the ball's right side faces the panorama's red, its left its blue; the panorama itself unseen
    let (right, left) = (px(&f, 280, 150), px(&f, 120, 150));
    assert!(right[0] > right[2] + 0.2 && left[2] > left[0] + 0.2, "right {right:?}, left {left:?}");
    assert_eq!(px(&f, 5, 5)[3], 0.0, "the environment layer is not drawn");
    // turned half round, the sides swap
    run(&mut s, "prop.set", &format!(r#"{{"layer": {light}, "path": "lightOptions/rotation", "value": 180}}"#));
    let turned = frame(&s);
    let (right, left) = (px(&turned, 280, 150), px(&turned, 120, 150));
    assert!(right[2] > right[0] + 0.2 && left[0] > left[2] + 0.2, "right {right:?}, left {left:?}");
    // a mirror shows the panorama itself where it faces; brighter where the matte ball only took
    // light from all round
    run(&mut s, "material.set", &format!(r#"{{"layers": [{ball}], "metallic": 100, "roughness": 0}}"#));
    let mirror = frame(&s);
    assert!(px(&mirror, 280, 150)[2] > 0.5, "{:?}", px(&mirror, 280, 150));
    // as the sky behind
    let sky = id(&run(&mut s, "light.environmentBackground", ""), "layer");
    let with_sky = frame(&s);
    let corner = px(&with_sky, 5, 150);
    assert!(corner[3] > 0.99 && (corner[0] > 0.9 || corner[2] > 0.9), "{corner:?}");
    assert_eq!(s.project().comp(c).unwrap().layers.last().map(|l| l.id), Some(sky), "at the bottom");
    // only pictures can be the environment; saved and read back
    assert!(fails(&mut s, "layer.environment", &format!(r#"{{"layers": [{ball}]}}"#)).contains("cannot be the environment"));
    let text = s.project().to_text();
    assert!(text.contains(r#""environment": true"#) && text.contains(r#""environmentBackground": true"#), "{text}");
    assert_eq!(&veymelo_project::Project::from_text(&text, &s.catalog).unwrap(), s.project());
}

#[test]
fn vector_pictures_drawn_sharp_and_made_shapes() {
    let dir = std::env::temp_dir().join(format!("veymelo-svg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("logo.svg");
    std::fs::write(
        &file,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 100 50">
  <defs><linearGradient id="g"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs>
  <rect x="5" y="5" width="40" height="40" fill="#20a050" stroke="#000" stroke-width="2"/>
  <g opacity="0.5"><circle cx="75" cy="25" r="20" fill="url(#g)"/></g>
  <text x="0" y="10">hi</text>
</svg>"##,
    )
    .unwrap();
    let mut s = Session::new();
    let c = id(&run(&mut s, "comp.new", r#"{"name": "Main", "width": 400, "height": 200, "fps": 25, "duration": 2}"#), "comp");
    let item = id(&run(&mut s, "footage.import", &format!(r#"{{"path": {}}}"#, Json::from(file.to_string_lossy().as_ref()).to_json())), "item");
    let pic = id(&run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {item}}}"#)), "layer");
    // four times its own size: still sharp (the square's edge crosses a pixel or two)
    run(&mut s, "prop.set", &format!(r#"{{"layer": {pic}, "path": "transform/scale", "value": [400, 400, 100]}}"#));
    let frame = |s: &Session| s.render(Some(&Json::Number(c as f64)), None, &render::Options { background: false, ..render::Options::default() }).unwrap();
    let vector = frame(&s);
    let soft = (0..200).filter(|x| (0.05..0.95).contains(&px(&vector, *x, 100)[3])).count();
    assert!(soft <= 4, "{soft} soft pixels along a row");
    let green = px(&vector, 100, 100);
    assert!(green[1] > 0.5 && green[0] < 0.2 && green[3] > 0.99, "{green:?}");
    // made shapes: the same picture from the shape layer, the picture's own layer off
    let made = run(&mut s, "layer.shapesFromVector", &format!(r#"{{"layer": {pic}}}"#));
    assert_eq!((made.get("name").and_then(Json::as_str), made.get("shapes").and_then(Json::as_u64)), (Some("logo.svg Outlines"), Some(2)));
    assert_eq!(made.get("skipped").unwrap().to_json(), r#"["text"]"#);
    let shapes = frame(&s);
    for (x, y) in [(100, 100), (24, 24), (300, 100), (250, 100), (360, 100), (200, 30)] {
        let (a, b) = (px(&vector, x, y), px(&shapes, x, y));
        assert!((0..4).all(|k| (a[k] - b[k]).abs() < 0.06), "at ({x}, {y}): picture {a:?}, shapes {b:?}");
    }
    let layer = s.project().comp(c).unwrap().layers.iter().find(|l| l.name == "logo.svg Outlines").unwrap().clone();
    assert!(layer.props.prop("contents/g/contents/circle/contents/Gradient Fill 1/colors").is_ok(), "the gradient kept as a gradient");
    assert!(!s.project().comp(c).unwrap().layer(pic).unwrap().switches.video);
    assert!(fails(&mut s, "layer.shapesFromVector", &format!(r#"{{"layer": {}}}"#, layer.id)).contains("does not show a vector picture"));
}

#[test]
fn photoshop_layers_made_a_composition() {
    use veymelo_psd::write::{layer as psd_layer, write as psd_write};
    let dir = std::env::temp_dir().join(format!("veymelo-psd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plane = |n: usize, v: u8| vec![v; n];
    // bottom first: a red background; a group (pass through) holding a half-seen green square and
    // a hidden blue one; a top layer that multiplies, yellow over the left half
    let mut back = psd_layer("Background", [0, 0, 40, 60]);
    back.channels = vec![(0, 1, plane(2400, 255)), (1, 1, plane(2400, 0)), (2, 1, plane(2400, 0))];
    let mut end = psd_layer("</Layer group>", [0, 0, 0, 0]);
    (end.section, end.hidden) = (Some(3), true);
    let mut blue = psd_layer("Blue", [0, 0, 10, 10]);
    blue.hidden = true;
    blue.channels = vec![(0, 0, plane(100, 0)), (1, 0, plane(100, 0)), (2, 0, plane(100, 255))];
    let mut green = psd_layer("Green", [10, 20, 30, 40]);
    green.opacity = 128;
    green.channels = vec![(-1, 2, plane(400, 255)), (0, 2, plane(400, 0)), (1, 2, plane(400, 255)), (2, 2, plane(400, 0))];
    let mut folder = psd_layer("Shapes", [0, 0, 0, 0]);
    (folder.section, folder.blend) = (Some(1), b"pass");
    let mut top = psd_layer("Tint", [0, 0, 40, 30]);
    top.blend = b"mul ";
    top.channels = vec![(0, 3, plane(1200, 255)), (1, 3, plane(1200, 255)), (2, 3, plane(1200, 0))];
    let file = dir.join("poster.psd");
    std::fs::write(&file, psd_write(60, 40, 8, &[back, end, blue, green, folder, top])).unwrap();
    let mut s = Session::new();
    let path = Json::from(file.to_string_lossy().as_ref()).to_json();
    for how in ["composition", "compositionLayerSizes"] {
        let made = run(&mut s, "footage.import", &format!(r#"{{"path": {path}, "as": "{how}", "name": "Poster {how}"}}"#));
        assert_eq!(made.get("layers").and_then(Json::as_u64), Some(4), "{}", made.to_json());
        let c = id(&made, "item");
        let comp = s.project().comp(c).unwrap();
        let names: Vec<&str> = comp.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Tint", "Shapes", "Background"]);
        assert_eq!((comp.layers[0].blend, comp.layers[1].switches.collapse), (veymelo_color::BlendMode::Multiply, true));
        let f = s.render(Some(&Json::Number(c as f64)), None, &render::Options { background: false, ..render::Options::default() }).unwrap();
        let at = |x: usize, y: usize| px(&f, x, y);
        // red alone on the right; under the yellow it multiplies, red still
        assert_eq!(at(50, 5), [1.0, 0.0, 0.0, 1.0]);
        // the green square half over the red (left of it yellow multiplies: red times yellow)
        let g = at(35, 20);
        assert!((g[0] - 0.5).abs() < 0.02 && (g[1] - 0.5).abs() < 0.02 && g[2] < 0.01, "{how}: {g:?}");
        let under_tint = at(25, 20);
        assert!((under_tint[1] - 0.5).abs() < 0.02 && (under_tint[0] - 0.5).abs() < 0.02, "{how}: green and red under the yellow multiplied: {under_tint:?}");
        // the hidden blue nowhere
        assert_eq!(at(5, 5), [1.0, 0.0, 0.0, 1.0]);
    }
    // saved and read back with the layer each footage shows
    let text = s.project().to_text();
    assert!(text.contains(r#""name": "Green""#) && text.contains(r#""cropped": true"#), "{text}");
    assert_eq!(&veymelo_project::Project::from_text(&text, &s.catalog).unwrap(), s.project());
    assert!(fails(&mut s, "footage.import", &format!(r#"{{"path": {}, "as": "composition"}}"#, Json::from("/tmp/x.png").to_json())).contains("only a Photoshop document"));
}

/// A render in an OpenEXR file (Blender's, a part a pass: depth, normals, object numbers,
/// Cryptomatte): the footage shows its beauty pass, its colours taken as premultiplied, and the
/// 3D Channel effects read its passes.
#[test]
fn render_passes_come_with_openexr_footage() {
    let file = format!("{}/../exr/fixtures/render-zip.exr", env!("CARGO_MANIFEST_DIR"));
    let mut s = Session::new();
    run(&mut s, "comp.new", r#"{"name": "Main", "width": 48, "height": 32, "fps": 24, "duration": 1}"#);
    let item = id(&run(&mut s, "footage.import", &format!(r#"{{"path": "{file}"}}"#)), "item");
    let info = run(&mut s, "footage.info", &format!(r#"{{"item": {item}}}"#));
    assert_eq!((info.get("width").and_then(Json::as_u64), info.get("alpha").and_then(Json::as_str)), (Some(48), Some("premultiplied")), "{}", info.to_json());
    run(&mut s, "layer.add", &format!(r#"{{"type": "footage", "item": {item}}}"#));
    // where the box (object 2), the ball (3, "bunny" in the manifest) and the floor (1) are
    let passes = veymelo_media::passes(&file).unwrap();
    assert!(passes.manifests.iter().any(|(k, m)| k == "ViewLayer.CryptoObject" && m.iter().any(|(n, _)| n == "bunny")), "{:?}", passes.manifests);
    let ids = passes.object_id().unwrap();
    let inside = |k: f32| (49..48 * 31).find(|&i| [i, i - 1, i + 1, i - 48, i + 48].iter().all(|j| ids[*j] == k)).unwrap();
    let (boxed, ball, floor) = (inside(2.0), inside(3.0), inside(1.0));
    let at = |r: &render::Rendered, i: usize| px(r, i % 48, i / 48);
    let plain = picture(&s);
    let (b, f) = (at(&plain, ball), at(&plain, floor));
    assert!(b[3] > 0.99 && b[2] > b[0] && f[3] > 0.99, "the beauty pass: {b:?} {f:?}");
    // the box alone, by its number
    run(&mut s, "effect.add", r##"{"layer": "#1", "type": "idMatte", "props": {"idSelection": 2}}"##);
    let r = picture(&s);
    assert!(at(&r, boxed)[3] > 0.99 && at(&r, ball)[3] == 0.0 && at(&r, floor)[3] == 0.0);
    assert_eq!(at(&r, boxed), at(&plain, boxed));
    // the ball alone, by its name
    run(&mut s, "effect.removeAll", r##"{"layer": "#1"}"##);
    run(&mut s, "effect.add", r##"{"layer": "#1", "type": "cryptomatte", "props": {"selection": "bunny", "matteOnly": true}}"##);
    let r = picture(&s);
    assert!(at(&r, ball)[3] > 0.99 && at(&r, boxed)[3] == 0.0 && at(&r, floor)[3] == 0.0, "{:?}", at(&r, ball));
    // the depth: the floor's near edge nearer than the ball, the sky far
    run(&mut s, "effect.removeAll", r##"{"layer": "#1"}"##);
    run(&mut s, "effect.add", r##"{"layer": "#1", "type": "channelExtract3d", "props": {"blackPoint": 4, "whitePoint": 12}}"##);
    let r = picture(&s);
    let depth = passes.depth().unwrap();
    assert!(depth[ball] > 5.0 && depth[ball] < 12.0 && depth[0] > 1e9, "{} {}", depth[ball], depth[0]);
    let grey = at(&r, ball)[0];
    let want = veymelo_color::linear_to_srgb(((depth[ball] - 4.0) / 8.0) as f64) as f32;
    assert!((grey - want).abs() < 0.02 || (grey - (depth[ball] - 4.0) / 8.0).abs() < 0.02, "{grey} {}", depth[ball]);
    assert!(r.notes.is_empty(), "{:?}", r.notes);
}

/// A composition of our own 3D layers shown in another: the 3D Channel effects on it read its
/// passes as a render would write them — depth from its camera, layer numbers, layer names for
/// Cryptomatte (a primitive's included), normals — and a 2D layer above lies on its plane.
#[test]
fn a_3d_composition_gives_its_passes() {
    let mut s = Session::new();
    let scene = id(&run(&mut s, "comp.new", r##"{"name": "Scene", "width": 200, "height": 100, "fps": 25, "duration": 1}"##), "comp");
    // a far wall, a nearer card on its left, a ball on its right, a 2D label at the top
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#336699", "width": 200, "height": 100, "name": "Wall"}"##);
    run(&mut s, "layer.set", r#"{"layer": "Wall", "switches": {"threeD": true}}"#);
    run(&mut s, "prop.set", r#"{"layer": "Wall", "path": "transform/position", "value": [100, 50, 300]}"#);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffcc00", "width": 40, "height": 40, "name": "Card"}"##);
    run(&mut s, "layer.set", r#"{"layer": "Card", "switches": {"threeD": true}}"#);
    run(&mut s, "prop.set", r#"{"layer": "Card", "path": "transform/position", "value": [50, 50, -50]}"#);
    run(&mut s, "layer.newPrimitive", r##"{"shape": "sphere", "radius": 20, "position": [150, 50, 0], "name": "Ball"}"##);
    run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 200, "height": 10, "name": "Label"}"##);
    run(&mut s, "prop.set", r#"{"layer": "Label", "path": "transform/position", "value": [100, 5]}"#);
    let main = id(&run(&mut s, "comp.new", r##"{"name": "Main", "width": 200, "height": 100, "fps": 25, "duration": 1}"##), "comp");
    run(&mut s, "layer.add", &format!(r#"{{"comp": {main}, "type": "comp", "item": {scene}}}"#));
    let frame = |s: &Session| s.render(Some(&Json::Number(main as f64)), None, &render::Options { background: false, ..render::Options::default() }).unwrap();
    let plain = frame(&s);
    // the wall far back, seen smaller (from 15 to 185 across, 25 to 75 down)
    let (card, ball, wall, label) = ((50, 50), (150, 50), (100, 30), (100, 4));
    let at = |r: &render::Rendered, (x, y): (usize, usize)| px(r, x, y);
    assert!(at(&plain, card)[3] > 0.99 && at(&plain, ball)[3] > 0.99 && at(&plain, wall)[3] > 0.99 && at(&plain, label)[3] > 0.99);
    assert_eq!(at(&plain, (100, 90))[3], 0.0);
    // the card alone, by its name; then the ball, a primitive
    for (name, keep, gone) in [("Card", card, ball), ("Ball", ball, card)] {
        run(&mut s, "effect.removeAll", &format!(r##"{{"comp": {main}, "layer": "#1"}}"##));
        run(&mut s, "effect.add", &format!(r##"{{"comp": {main}, "layer": "#1", "type": "cryptomatte", "props": {{"selection": "{name}", "display": "matte"}}}}"##));
        let r = frame(&s);
        assert!(at(&r, keep)[3] > 0.99 && at(&r, gone)[3] == 0.0 && at(&r, wall)[3] == 0.0, "{name}: {:?} {:?}", at(&r, keep), at(&r, gone));
        assert!(r.notes.is_empty(), "{:?}", r.notes);
    }
    // the wall by its number (4: the bottom of four)
    run(&mut s, "effect.removeAll", &format!(r##"{{"comp": {main}, "layer": "#1"}}"##));
    run(&mut s, "effect.add", &format!(r##"{{"comp": {main}, "layer": "#1", "type": "idMatte", "props": {{"idSelection": 4}}}}"##));
    let r = frame(&s);
    assert!(at(&r, wall)[3] > 0.99 && at(&r, card)[3] == 0.0 && at(&r, label)[3] == 0.0);
    // depth: the card nearer than the ball, the ball than the wall; the label on the plane
    run(&mut s, "effect.removeAll", &format!(r##"{{"comp": {main}, "layer": "#1"}}"##));
    run(&mut s, "effect.add", &format!(r##"{{"comp": {main}, "layer": "#1", "type": "channelExtract3d", "props": {{"blackPoint": 0, "whitePoint": 2000}}}}"##));
    let r = frame(&s);
    let d = |p| at(&r, p)[0];
    assert!(d(card) < d(ball) && d(ball) < d(wall) && d(wall) < 1.0, "{} {} {}", d(card), d(ball), d(wall));
    // normals: the card faces the camera; the ball's left edge faces left
    run(&mut s, "effect.removeAll", &format!(r##"{{"comp": {main}, "layer": "#1"}}"##));
    run(&mut s, "effect.add", &format!(r##"{{"comp": {main}, "layer": "#1", "type": "channelExtract3d", "props": {{"channel": "surfaceNormals", "whitePoint": 1}}}}"##));
    let r = frame(&s);
    let n = at(&r, card);
    assert!((n[0] - n[1]).abs() < 0.02 && n[2] < 0.05, "facing: {n:?}");
    let edge = at(&r, (132, 50));
    assert!(edge[0] < n[0] - 0.1, "{edge:?}");
}

/// How long frames of a project take to draw, played on from each time asked frame by frame for
/// two seconds (run by hand, under a profiler if wanted: VEYMELO_PROJECT=/path/to.veymelo
/// VEYMELO_TIMES=5,40 VEYMELO_GPU=0|1 cargo test --release frames_take -- --ignored --nocapture).
#[test]
#[ignore]
fn frames_take() {
    let Some(file) = std::env::var_os("VEYMELO_PROJECT") else { return };
    let s = Session::open(std::path::Path::new(&file)).unwrap();
    let times: Vec<f64> = std::env::var("VEYMELO_TIMES").unwrap_or_else(|_| "5".into()).split(',').filter_map(|t| t.parse().ok()).collect();
    let gpu = std::env::var("VEYMELO_GPU").map_or(veymelo_os::gpu::available(), |v| v != "0");
    let options = veymelo_render::Options { gpu, ..veymelo_render::Options::default() };
    let fps = s.view.comp.and_then(|c| s.project().comp(c)).map_or(30.0, |c| c.frame_rate.num as f64 / c.frame_rate.den as f64);
    for t in times {
        s.render(None, Some(veymelo_time::Tick::from_seconds(t)), &options).unwrap();
        let frames = (fps * 2.0) as usize;
        let started = std::time::Instant::now();
        for k in 1..=frames {
            std::hint::black_box(s.render(None, Some(veymelo_time::Tick::from_seconds(t + k as f64 / fps)), &options).unwrap());
        }
        eprintln!("{t} s on: {:.1} ms a frame", started.elapsed().as_secs_f64() * 1000.0 / frames as f64);
    }
}
