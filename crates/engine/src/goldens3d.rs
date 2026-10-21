//! What 3D scenes look like, kept to hold them while the 3D renderer moves to the GPU: scenes of
//! each kind (flat layers lit and shadowed, primitives in their materials, models casting shadows,
//! extruded text, an environment light, depth of field, motion blur) drawn at a few times. Run by
//! hand:
//!
//!   VEYMELO_GOLDENS=/tmp/g cargo test -p veymelo-engine --release goldens_3d -- --ignored
//!
//! The first run writes each frame (`/tmp/g/<scene>-<t>.bin`, raw float pixels) and a PNG of it to
//! look at; later runs compare with them and write each one that changed, before, after and the
//! difference, to `/tmp/g/changed/`. `VEYMELO_GOLDENS_WRITE=1` writes anew; `VEYMELO_GOLDENS_ONLY`
//! runs the scenes whose name holds it; `VEYMELO_GOLDENS_SCALE=4` draws them that much bigger
//! (kept apart, in `/tmp/g/x4/`); `VEYMELO_GOLDENS_CPU=1` draws without the GPU (its frames kept
//! apart too, in `cpu/`), and `VEYMELO_GOLDENS_VS_CPU=1` holds the GPU's to those.

use super::*;
use std::path::{Path, PathBuf};

fn run(s: &mut Session, id: &str, params: &str) -> Json {
    let p = if params.is_empty() { Json::Null } else { Json::parse(params).unwrap_or_else(|e| panic!("{params}: {e}")) };
    s.run(id, &p).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn id(j: &Json, key: &str) -> u64 {
    j.get(key).and_then(Json::as_u64).unwrap_or_else(|| panic!("no {key} in {}", j.to_json()))
}

fn set(s: &mut Session, layer: u64, path: &str, value: &str) {
    run(s, "prop.set", &format!(r#"{{"layer": {layer}, "path": "{path}", "value": {value}}}"#));
}

fn three_d(s: &mut Session, layer: u64) {
    run(s, "layer.set", &format!(r#"{{"layer": {layer}, "switches": {{"threeD": true}}}}"#));
}

/// A composition 480 × 270 at 30 frames a second, with a camera turned a little and a floor.
fn stage(name: &str) -> (Session, u64, u64) {
    let mut s = Session::new();
    let c = id(&run(&mut s, "comp.new", &format!(r##"{{"name": "{name}", "width": 480, "height": 270, "fps": 30, "duration": 2, "background": "#f4f4f6"}}"##)), "comp");
    let floor = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#9c9890", "width": 1400, "height": 1400, "name": "Floor"}"##), "layer");
    three_d(&mut s, floor);
    set(&mut s, floor, "transform/position", "[240, 230, 300]");
    set(&mut s, floor, "transform/rotationX", "90");
    let cam = id(&run(&mut s, "layer.add", r#"{"type": "camera", "lens": 35}"#), "layer");
    set(&mut s, cam, "transform/position", "[90, -40, -520]");
    set(&mut s, cam, "transform/pointOfInterest", "[240, 150, 120]");
    (s, c, cam)
}

fn primitive(s: &mut Session, shape: &str, at: [f64; 3], color: &str, metallic: f64, roughness: f64) -> u64 {
    let size = match shape {
        "cube" => r#""width": 90, "height": 90, "depth": 90"#,
        "sphere" => r#""radius": 50"#,
        "torus" => r#""radius": 45, "tubeRadius": 16"#,
        _ => r#""radius": 45, "height": 100"#,
    };
    id(&run(s, "layer.newPrimitive", &format!(r#"{{"shape": "{shape}", "position": [{}, {}, {}], "baseColor": "{color}", "metallic": {metallic}, "roughness": {roughness}, {size}}}"#, at[0], at[1], at[2])), "layer")
}

fn light(s: &mut Session, kind: &str, at: [f64; 3], shadows: bool) -> u64 {
    let l = id(&run(s, "layer.add", &format!(r#"{{"type": "light", "light": "{kind}"}}"#)), "layer");
    set(s, l, "transform/position", &format!("[{}, {}, {}]", at[0], at[1], at[2]));
    set(s, l, "transform/pointOfInterest", "[240, 170, 120]");
    set(s, l, "lightOptions/intensity", if kind == "ambient" { "30" } else { "70" });
    if shadows {
        set(s, l, "lightOptions/castsShadows", "true");
        set(s, l, "lightOptions/shadowDiffusion", "12");
    }
    l
}

/// Each scene: its name, its session and composition, and the times to draw.
fn scenes() -> Vec<(&'static str, Session, u64, Vec<f64>)> {
    let mut out = vec![];
    // flat layers lit by a spot and a point, throwing soft shadows on the floor and each other
    {
        let (mut s, c, _) = stage("flat");
        for (k, (color, at)) in [("#e4572e", [170.0, 150.0, 90.0]), ("#2e86ab", [300.0, 160.0, 160.0])].into_iter().enumerate() {
            let l = id(&run(&mut s, "layer.add", &format!(r##"{{"type": "solid", "color": "{color}", "width": 120, "height": 160}}"##)), "layer");
            three_d(&mut s, l);
            set(&mut s, l, "transform/position", &format!("[{}, {}, {}]", at[0], at[1], at[2]));
            set(&mut s, l, "transform/rotationY", &format!("{}", 25.0 - 40.0 * k as f64));
            set(&mut s, l, "materialOptions/castsShadows", "\"on\"");
        }
        let t = id(&run(&mut s, "layer.add", r#"{"type": "text", "text": "Veymelo"}"#), "layer");
        three_d(&mut s, t);
        set(&mut s, t, "transform/position", "[240, 70, 60]");
        set(&mut s, t, "materialOptions/castsShadows", "\"on\"");
        light(&mut s, "spot", [60.0, -160.0, -160.0], true);
        light(&mut s, "point", [420.0, -60.0, 0.0], true);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("flat", s, c, vec![0.0, 1.0]));
    }
    // primitives in their materials, lit by a parallel light and an ambient one (no shadows)
    {
        let (mut s, c, _) = stage("primitives");
        primitive(&mut s, "cube", [140.0, 170.0, 120.0], "#d1495b", 0.0, 0.5);
        primitive(&mut s, "sphere", [250.0, 160.0, 140.0], "#edae49", 1.0, 0.25);
        primitive(&mut s, "torus", [360.0, 160.0, 120.0], "#00798c", 0.2, 0.8);
        primitive(&mut s, "cone", [200.0, 170.0, 280.0], "#30638e", 0.0, 0.3);
        primitive(&mut s, "cylinder", [320.0, 170.0, 300.0], "#8a8a8a", 0.6, 0.4);
        light(&mut s, "parallel", [40.0, -200.0, -240.0], false);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("primitives", s, c, vec![0.0]));
    }
    // the same with no lights of its own (the standard key and fill light it)
    {
        let (mut s, c, _) = stage("unlit");
        primitive(&mut s, "sphere", [200.0, 160.0, 140.0], "#edae49", 0.0, 0.5);
        primitive(&mut s, "cube", [310.0, 170.0, 160.0], "#d1495b", 0.0, 0.5);
        out.push(("unlit", s, c, vec![0.0]));
    }
    // models throwing shadows on the floor and on each other, by a light with a size
    {
        let (mut s, c, _) = stage("shadows");
        let cube = primitive(&mut s, "cube", [180.0, 160.0, 140.0], "#d1495b", 0.0, 0.5);
        set(&mut s, cube, "transform/rotationY", "30");
        let sphere = primitive(&mut s, "sphere", [300.0, 150.0, 120.0], "#edae49", 0.0, 0.4);
        for m in [cube, sphere] {
            set(&mut s, m, "materialOptions/castsShadows", "\"on\"");
        }
        let plane = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#6c9a8b", "width": 140, "height": 140}"##), "layer");
        three_d(&mut s, plane);
        set(&mut s, plane, "transform/position", "[250, 120, 260]");
        set(&mut s, plane, "materialOptions/castsShadows", "\"on\"");
        light(&mut s, "point", [120.0, -180.0, -60.0], true);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("shadows", s, c, vec![0.0]));
    }
    // text and a shape made solid, bevelled
    {
        let (mut s, c, _) = stage("extruded");
        let t = id(&run(&mut s, "layer.add", r#"{"type": "text", "text": "AI"}"#), "layer");
        three_d(&mut s, t);
        set(&mut s, t, "transform/position", "[240, 175, 140]");
        set(&mut s, t, "transform/rotationY", "-25");
        set(&mut s, t, "transform/scale", "[150, 150, 150]");
        set(&mut s, t, "geometryOptions/extrusionDepth", "30");
        set(&mut s, t, "geometryOptions/bevelStyle", "\"convex\"");
        set(&mut s, t, "geometryOptions/bevelDepth", "4");
        light(&mut s, "spot", [80.0, -160.0, -200.0], true);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("extruded", s, c, vec![0.0]));
    }
    // an environment light from a picture (a sky made of solids), shining on metal and rough
    {
        let (mut s, c, _) = stage("environment");
        let sky = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#87b5e8", "width": 512, "height": 256, "name": "Sky"}"##), "layer");
        let sun = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#fff2c4", "width": 512, "height": 256, "name": "Sun"}"##), "layer");
        let pc = id(&run(&mut s, "layer.precompose", &format!(r#"{{"layers": [{sky}, {sun}], "name": "Panorama"}}"#)), "comp");
        let _ = pc;
        primitive(&mut s, "sphere", [180.0, 160.0, 140.0], "#cccccc", 1.0, 0.15);
        primitive(&mut s, "sphere", [310.0, 160.0, 140.0], "#cc6644", 0.0, 0.8);
        let env = id(&run(&mut s, "layer.add", r#"{"type": "light", "light": "environment"}"#), "layer");
        let pano = s.project().comp(c).and_then(|cp| cp.layers.iter().find(|l| l.name == "Panorama").map(|l| l.id)).expect("the panorama");
        set(&mut s, env, "lightOptions/source", &format!("{pano}"));
        run(&mut s, "layer.set", &format!(r#"{{"layer": {pano}, "switches": {{"video": false}}}}"#));
        out.push(("environment", s, c, vec![0.0]));
    }
    // depth of field: near and far out of focus
    {
        let (mut s, c, cam) = stage("focus");
        primitive(&mut s, "cube", [120.0, 170.0, -40.0], "#d1495b", 0.0, 0.5);
        primitive(&mut s, "sphere", [250.0, 160.0, 140.0], "#edae49", 0.0, 0.4);
        primitive(&mut s, "torus", [380.0, 160.0, 420.0], "#00798c", 0.0, 0.6);
        let l = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#30638e", "width": 100, "height": 140}"##), "layer");
        three_d(&mut s, l);
        set(&mut s, l, "transform/position", "[330, 140, 60]");
        set(&mut s, cam, "cameraOptions/depthOfField", "true");
        set(&mut s, cam, "cameraOptions/focusDistance", "700");
        set(&mut s, cam, "cameraOptions/aperture", "60");
        light(&mut s, "parallel", [40.0, -200.0, -240.0], false);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("focus", s, c, vec![0.0]));
    }
    // bokeh: small bright lights far out of focus open into the iris's shape (a hexagon, turned,
    // its rim brighter), a sphere in focus
    {
        let (mut s, c, cam) = stage("bokeh");
        run(&mut s, "comp.set", &format!(r##"{{"comp": {c}, "background": "#10141c"}}"##));
        primitive(&mut s, "sphere", [240.0, 170.0, 140.0], "#edae49", 0.0, 0.4);
        for k in 0..7 {
            let l = id(&run(&mut s, "layer.add", &format!(r##"{{"type": "solid", "color": "{}", "width": 40, "height": 40}}"##, ["#ffffff", "#ffd27a", "#9ad1ff"][k % 3])), "layer");
            three_d(&mut s, l);
            set(&mut s, l, "transform/position", &format!("[{}, {}, 900]", -60.0 + 100.0 * k as f64, 120.0 + 30.0 * (k % 3) as f64));
            set(&mut s, l, "materialOptions/acceptsLights", "false");
        }
        set(&mut s, cam, "cameraOptions/depthOfField", "true");
        set(&mut s, cam, "cameraOptions/focusDistance", "640");
        set(&mut s, cam, "cameraOptions/aperture", "90");
        set(&mut s, cam, "cameraOptions/irisShape", "\"hexagon\"");
        set(&mut s, cam, "cameraOptions/irisRotation", "15");
        set(&mut s, cam, "cameraOptions/irisDiffractionFringe", "40");
        set(&mut s, cam, "cameraOptions/highlightGain", "100");
        set(&mut s, cam, "cameraOptions/highlightThreshold", "150");
        light(&mut s, "parallel", [40.0, -200.0, -240.0], false);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("bokeh", s, c, vec![0.0]));
    }
    // ambient occlusion: models standing on the floor and in a corner, a card leaning on one,
    // the ambient light kept out of the nooks
    {
        let (mut s, c, _) = stage("occlusion");
        primitive(&mut s, "cube", [160.0, 185.0, 160.0], "#d1495b", 0.0, 0.5);
        primitive(&mut s, "sphere", [270.0, 180.0, 150.0], "#edae49", 0.0, 0.5);
        primitive(&mut s, "torus", [350.0, 214.0, 190.0], "#00798c", 0.0, 0.6);
        let wall = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#c9c4ba", "width": 600, "height": 300, "name": "Wall"}"##), "layer");
        three_d(&mut s, wall);
        set(&mut s, wall, "transform/position", "[240, 80, 230]");
        let card = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#30638e", "width": 70, "height": 100}"##), "layer");
        three_d(&mut s, card);
        set(&mut s, card, "transform/position", "[205, 180, 120]");
        set(&mut s, card, "transform/rotationX", "-20");
        light(&mut s, "parallel", [40.0, -200.0, -240.0], false);
        let amb = light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        set(&mut s, amb, "lightOptions/intensity", "60");
        set(&mut s, amb, "lightOptions/ambientOcclusion", "100");
        set(&mut s, amb, "lightOptions/occlusionDistance", "80");
        out.push(("occlusion", s, c, vec![0.0]));
    }
    // reflections: a mirror floor, a glossy wall behind, a sphere mirroring a little, and a word
    {
        let (mut s, c, _) = stage("mirror");
        let floor = s.project().comp(c).and_then(|cp| cp.layers.iter().find(|l| l.name == "Floor").map(|l| l.id)).expect("the floor");
        set(&mut s, floor, "materialOptions/reflectionIntensity", "70");
        set(&mut s, floor, "materialOptions/reflectionRolloff", "30");
        primitive(&mut s, "cube", [150.0, 185.0, 170.0], "#d1495b", 0.0, 0.5);
        let ball = primitive(&mut s, "sphere", [270.0, 180.0, 150.0], "#edae49", 0.0, 0.5);
        set(&mut s, ball, "materialOptions/reflectionIntensity", "40");
        primitive(&mut s, "torus", [360.0, 214.0, 190.0], "#00798c", 0.0, 0.6);
        let wall = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#30638e", "width": 600, "height": 300, "name": "Wall"}"##), "layer");
        three_d(&mut s, wall);
        set(&mut s, wall, "transform/position", "[240, 80, 260]");
        set(&mut s, wall, "materialOptions/reflectionIntensity", "50");
        set(&mut s, wall, "materialOptions/reflectionSharpness", "60");
        let t = id(&run(&mut s, "layer.add", r#"{"type": "text", "text": "Veymelo"}"#), "layer");
        three_d(&mut s, t);
        set(&mut s, t, "transform/position", "[250, 120, 200]");
        light(&mut s, "parallel", [40.0, -200.0, -240.0], false);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("mirror", s, c, vec![0.0]));
    }
    // light bouncing: a box with a red wall and a green one, white blocks and a ball in it, lit
    // from above (the walls tint the floor and the blocks by them, the shadows fill)
    {
        let mut s = Session::new();
        let c = id(&run(&mut s, "comp.new", r##"{"name": "bounce", "width": 480, "height": 270, "fps": 30, "duration": 2, "background": "#101010"}"##), "comp");
        run(&mut s, "comp.set", r#"{"lightBounce": 100, "bounces": 2}"#);
        let wall = |s: &mut Session, name: &str, color: &str, at: [f64; 3], rot: (&str, f64)| {
            let l = id(&run(s, "layer.add", &format!(r##"{{"type": "solid", "color": "{color}", "width": 400, "height": 400, "name": "{name}"}}"##)), "layer");
            three_d(s, l);
            set(s, l, "transform/position", &format!("[{}, {}, {}]", at[0], at[1], at[2]));
            if rot.1 != 0.0 {
                set(s, l, &format!("transform/{}", rot.0), &format!("{}", rot.1));
            }
            l
        };
        wall(&mut s, "Floor", "#d8d4cc", [240.0, 335.0, 200.0], ("rotationX", 90.0));
        wall(&mut s, "Ceiling", "#d8d4cc", [240.0, -65.0, 200.0], ("rotationX", 90.0));
        wall(&mut s, "Back", "#d8d4cc", [240.0, 135.0, 400.0], ("rotationX", 0.0));
        wall(&mut s, "Left", "#c0281e", [40.0, 135.0, 200.0], ("rotationY", 90.0));
        wall(&mut s, "Right", "#2a9a3a", [440.0, 135.0, 200.0], ("rotationY", 90.0));
        let tall = id(&run(&mut s, "layer.newPrimitive", r##"{"shape": "cube", "position": [170, 255, 260], "baseColor": "#e0ddd6", "metallic": 0, "roughness": 0.6, "width": 100, "height": 160, "depth": 100}"##), "layer");
        set(&mut s, tall, "transform/rotationY", "20");
        let short = id(&run(&mut s, "layer.newPrimitive", r##"{"shape": "cube", "position": [320, 290, 170], "baseColor": "#e0ddd6", "metallic": 0, "roughness": 0.6, "width": 90, "height": 90, "depth": 90}"##), "layer");
        set(&mut s, short, "transform/rotationY", "-18");
        primitive(&mut s, "sphere", [300.0, 205.0, 170.0], "#e0ddd6", 0.0, 0.4);
        for l in [tall, short] {
            set(&mut s, l, "materialOptions/castsShadows", "\"on\"");
        }
        let cam = id(&run(&mut s, "layer.add", r#"{"type": "camera", "lens": 35}"#), "layer");
        set(&mut s, cam, "transform/position", "[240, 150, -470]");
        set(&mut s, cam, "transform/pointOfInterest", "[240, 150, 200]");
        let key = light(&mut s, "point", [240.0, -40.0, 200.0], true);
        set(&mut s, key, "lightOptions/intensity", "90");
        let amb = light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        set(&mut s, amb, "lightOptions/intensity", "8");
        out.push(("bounce", s, c, vec![0.0]));
    }
    // glass: a clear ball and a blue box in front of stripes and words, bending what is behind
    // them (and seen through to the paper where nothing is behind)
    {
        let (mut s, c, _) = stage("glass");
        for k in 0..8 {
            let color = ["#e4572e", "#f3a712", "#29335c", "#669bbc"][k % 4];
            let l = id(&run(&mut s, "layer.add", &format!(r##"{{"type": "solid", "color": "{color}", "width": 40, "height": 300}}"##)), "layer");
            three_d(&mut s, l);
            set(&mut s, l, "transform/position", &format!("[{}, 110, 320]", 100.0 + 40.0 * k as f64));
        }
        let t = id(&run(&mut s, "layer.add", r#"{"type": "text", "text": "GLASS"}"#), "layer");
        three_d(&mut s, t);
        set(&mut s, t, "transform/position", "[240, 120, 300]");
        set(&mut s, t, "transform/scale", "[160, 160, 160]");
        let ball = primitive(&mut s, "sphere", [200.0, 170.0, 160.0], "#ffffff", 0.0, 0.1);
        set(&mut s, ball, "materialOptions/transparency", "90");
        set(&mut s, ball, "materialOptions/indexOfRefraction", "1.5");
        set(&mut s, ball, "materialOptions/reflectionIntensity", "15");
        let box_ = primitive(&mut s, "cube", [320.0, 185.0, 170.0], "#9fd3ff", 0.0, 0.1);
        set(&mut s, box_, "transform/rotationY", "30");
        set(&mut s, box_, "materialOptions/transparency", "85");
        set(&mut s, box_, "materialOptions/transparencyRolloff", "30");
        set(&mut s, box_, "materialOptions/indexOfRefraction", "1.45");
        light(&mut s, "parallel", [40.0, -200.0, -240.0], false);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("glass", s, c, vec![0.0]));
    }
    // glass casting shadows: an orange glass ball and a blue glass box let the light through,
    // tinted (the box less clear slant on), beside a solid cone's shadow
    {
        let (mut s, c, _) = stage("glass-shadows");
        let ball = primitive(&mut s, "sphere", [170.0, 170.0, 150.0], "#ff9a3c", 0.0, 0.1);
        set(&mut s, ball, "materialOptions/transparency", "85");
        set(&mut s, ball, "materialOptions/indexOfRefraction", "1.5");
        let box_ = primitive(&mut s, "cube", [290.0, 185.0, 130.0], "#5aa9ff", 0.0, 0.1);
        set(&mut s, box_, "transform/rotationY", "35");
        set(&mut s, box_, "materialOptions/transparency", "80");
        set(&mut s, box_, "materialOptions/transparencyRolloff", "40");
        let cone = primitive(&mut s, "cone", [380.0, 180.0, 160.0], "#3b3b3b", 0.0, 0.6);
        for m in [ball, box_, cone] {
            set(&mut s, m, "materialOptions/castsShadows", "\"on\"");
        }
        light(&mut s, "point", [150.0, -220.0, -40.0], true);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("glass-shadows", s, c, vec![0.0]));
    }
    // a picture on a model seen in a mirror and through glass, read where each is seen: a disc
    // with a round gradient made solid, over a mirror floor, beside a glass ball
    {
        let (mut s, c, _) = stage("mirror-pictures");
        let floor = s.project().comp(c).and_then(|cp| cp.layers.iter().find(|l| l.name == "Floor").map(|l| l.id)).expect("the floor");
        set(&mut s, floor, "materialOptions/reflectionIntensity", "70");
        let disc = id(&run(&mut s, "layer.add", r#"{"type": "shape"}"#), "layer");
        run(&mut s, "shape.add", &format!(r#"{{"layer": {disc}, "type": "ellipse", "props": {{"size": [150, 150]}}}}"#));
        run(&mut s, "shape.add", &format!(r##"{{"layer": {disc}, "type": "gradientFill", "props": {{"type": "radial", "start": [0, 0], "end": [75, 0], "colors": ["#ffe066", "#e63946", "#1d3557", "#2a9d8f"]}}}}"##));
        three_d(&mut s, disc);
        set(&mut s, disc, "transform/position", "[200, 140, 160]");
        set(&mut s, disc, "transform/rotationY", "-20");
        set(&mut s, disc, "geometryOptions/extrusionDepth", "16");
        let ball = primitive(&mut s, "sphere", [330.0, 175.0, 110.0], "#ffffff", 0.0, 0.05);
        set(&mut s, ball, "materialOptions/transparency", "90");
        set(&mut s, ball, "materialOptions/indexOfRefraction", "1.3");
        light(&mut s, "parallel", [40.0, -200.0, -240.0], false);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("mirror-pictures", s, c, vec![0.0]));
    }
    // what mirrors show: a cube not seen in them, a ball seen only in them, a card seen in them
    // and a torus not, over a mirror floor
    {
        let (mut s, c, _) = stage("reflections");
        let floor = s.project().comp(c).and_then(|cp| cp.layers.iter().find(|l| l.name == "Floor").map(|l| l.id)).expect("the floor");
        set(&mut s, floor, "materialOptions/reflectionIntensity", "80");
        let cube = primitive(&mut s, "cube", [150.0, 185.0, 170.0], "#d1495b", 0.0, 0.5);
        set(&mut s, cube, "materialOptions/appearsInReflections", "\"off\"");
        let ball = primitive(&mut s, "sphere", [250.0, 180.0, 150.0], "#edae49", 0.0, 0.5);
        set(&mut s, ball, "materialOptions/appearsInReflections", "\"only\"");
        let card = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#30638e", "width": 70, "height": 90}"##), "layer");
        three_d(&mut s, card);
        set(&mut s, card, "transform/position", "[340, 175, 180]");
        let ring = primitive(&mut s, "torus", [420.0, 200.0, 140.0], "#00798c", 0.0, 0.6);
        set(&mut s, ring, "materialOptions/appearsInReflections", "\"off\"");
        light(&mut s, "parallel", [40.0, -200.0, -240.0], false);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("reflections", s, c, vec![0.0]));
    }
    // see-through models: their own far sides seen through their near ones, nearest last — a
    // see-through cube round a solid ball, a see-through torus, a see-through ball by a cone
    {
        let (mut s, c, _) = stage("see-through");
        let cube = primitive(&mut s, "cube", [150.0, 175.0, 160.0], "#5aa9ff", 0.0, 0.4);
        set(&mut s, cube, "transform/rotationY", "30");
        set(&mut s, cube, "transform/rotationX", "20");
        set(&mut s, cube, "materialOptions/baseColor", "[0.35, 0.66, 1.0, 0.35]");
        primitive(&mut s, "sphere", [150.0, 175.0, 160.0], "#edae49", 0.0, 0.4);
        set(&mut s, cube, "transform/scale", "[130, 130, 130]");
        let ring = primitive(&mut s, "torus", [290.0, 170.0, 140.0], "#d1495b", 0.0, 0.4);
        set(&mut s, ring, "transform/rotationX", "60");
        set(&mut s, ring, "materialOptions/baseColor", "[0.82, 0.29, 0.36, 0.5]");
        let ball = primitive(&mut s, "sphere", [380.0, 180.0, 150.0], "#2a9d8f", 0.0, 0.3);
        set(&mut s, ball, "materialOptions/baseColor", "[0.16, 0.62, 0.56, 0.4]");
        primitive(&mut s, "cone", [410.0, 180.0, 230.0], "#3b3b3b", 0.0, 0.6);
        light(&mut s, "parallel", [40.0, -200.0, -240.0], false);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("see-through", s, c, vec![0.0]));
    }
    // caustics: a clear glass ball focusing a point light into a bright spot on the floor, a
    // tinted glass cube throwing its coloured light bent, beside an opaque cone's shadow
    {
        let (mut s, c, _) = stage("caustics");
        run(&mut s, "comp.set", &format!(r#"{{"comp": {c}, "caustics": 100}}"#));
        let ball = primitive(&mut s, "sphere", [175.0, 150.0, 150.0], "#f2f6ff", 0.0, 0.05);
        set(&mut s, ball, "materialOptions/transparency", "95");
        set(&mut s, ball, "materialOptions/indexOfRefraction", "1.5");
        let box_ = primitive(&mut s, "cube", [300.0, 185.0, 130.0], "#7cc4ff", 0.0, 0.05);
        set(&mut s, box_, "transform/rotationY", "35");
        set(&mut s, box_, "transform/rotationX", "20");
        set(&mut s, box_, "materialOptions/transparency", "85");
        set(&mut s, box_, "materialOptions/indexOfRefraction", "1.4");
        let cone = primitive(&mut s, "cone", [390.0, 180.0, 170.0], "#3b3b3b", 0.0, 0.6);
        for m in [ball, box_, cone] {
            set(&mut s, m, "materialOptions/castsShadows", "\"on\"");
        }
        let l = light(&mut s, "point", [200.0, -260.0, 60.0], true);
        set(&mut s, l, "lightOptions/shadowDiffusion", "4");
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("caustics", s, c, vec![0.0]));
    }
    // many layers: 120 cards in rows (more than the GPU once took), and a stack of 24 see-through
    // ones on one spot (more at a pixel than it keeps: those pixels are drawn by the processor)
    {
        let (mut s, c, _) = stage("many");
        for k in 0..120 {
            let (i, j) = (k % 15, k / 15);
            let color = ["#d1495b", "#edae49", "#00798c", "#30638e"][k % 4];
            let l = id(&run(&mut s, "layer.add", &format!(r##"{{"type": "solid", "color": "{color}", "width": 16, "height": 22}}"##)), "layer");
            three_d(&mut s, l);
            set(&mut s, l, "transform/position", &format!("[{}, {}, {}]", 60.0 + 26.0 * i as f64, 120.0 + 12.0 * j as f64, 40.0 + 30.0 * j as f64));
            set(&mut s, l, "transform/rotationY", &format!("{}", (k * 37 % 90) as f64 - 45.0));
        }
        for k in 0..24 {
            let l = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#ffffff", "width": 60, "height": 60}"##), "layer");
            three_d(&mut s, l);
            set(&mut s, l, "transform/position", &format!("[380, 60, {}]", -20.0 + 4.0 * k as f64));
            set(&mut s, l, "transform/opacity", "12");
        }
        light(&mut s, "parallel", [40.0, -200.0, -240.0], false);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        out.push(("many", s, c, vec![0.0]));
    }
    // a big scene: 400 cards in rows casting soft shadows on the floor and each other by a spot
    // light, among them models casting too, ambient occlusion round all
    {
        let (mut s, c, _) = stage("crowd");
        for k in 0..400 {
            let (i, j) = (k % 25, k / 25);
            let color = ["#d1495b", "#edae49", "#00798c", "#30638e", "#f4f1de"][k % 5];
            let l = id(&run(&mut s, "layer.add", &format!(r##"{{"type": "solid", "color": "{color}", "width": 12, "height": 18}}"##)), "layer");
            three_d(&mut s, l);
            set(&mut s, l, "transform/position", &format!("[{}, {}, {}]", 30.0 + 18.0 * i as f64, 200.0 - 6.0 * (k % 3) as f64, 20.0 + 26.0 * j as f64));
            set(&mut s, l, "transform/rotationY", &format!("{}", (k * 53 % 120) as f64 - 60.0));
            set(&mut s, l, "materialOptions/castsShadows", "\"on\"");
        }
        for (k, shape) in ["cube", "sphere", "torus"].iter().enumerate() {
            let m = primitive(&mut s, shape, [140.0 + 110.0 * k as f64, 190.0, 160.0], "#9a8c98", 0.0, 0.5);
            set(&mut s, m, "materialOptions/castsShadows", "\"on\"");
        }
        let key = light(&mut s, "spot", [120.0, -220.0, -60.0], true);
        set(&mut s, key, "lightOptions/coneAngle", "120");
        let amb = light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        set(&mut s, amb, "lightOptions/ambientOcclusion", "100");
        set(&mut s, amb, "lightOptions/occlusionDistance", "40");
        out.push(("crowd", s, c, vec![0.0]));
    }
    // motion blur: a cube flying and turning, a plane sliding
    {
        let (mut s, c, _) = stage("motion");
        let cube = primitive(&mut s, "cube", [140.0, 160.0, 140.0], "#d1495b", 0.0, 0.5);
        run(&mut s, "key.set", &format!(r#"{{"layer": {cube}, "path": "transform/position", "time": 0, "value": [100, 160, 140]}}"#));
        run(&mut s, "key.set", &format!(r#"{{"layer": {cube}, "path": "transform/position", "time": 1, "value": [380, 140, 140]}}"#));
        run(&mut s, "key.set", &format!(r#"{{"layer": {cube}, "path": "transform/rotationY", "time": 0, "value": 0}}"#));
        run(&mut s, "key.set", &format!(r#"{{"layer": {cube}, "path": "transform/rotationY", "time": 1, "value": 360}}"#));
        run(&mut s, "layer.set", &format!(r#"{{"layer": {cube}, "switches": {{"motionBlur": true}}}}"#));
        light(&mut s, "parallel", [40.0, -200.0, -240.0], false);
        light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        run(&mut s, "comp.set", &format!(r#"{{"comp": {c}, "motionBlur": true}}"#));
        out.push(("motion", s, c, vec![0.5]));
    }
    out
}

fn write_frame(img: &veymelo_raster::Image) -> Vec<u8> {
    let mut out = vec![];
    out.extend_from_slice(&(img.width as u32).to_le_bytes());
    out.extend_from_slice(&(img.height as u32).to_le_bytes());
    for p in &img.px {
        for c in p {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    out
}

fn read_frame(b: &[u8]) -> veymelo_raster::Image {
    let w = u32::from_le_bytes(b[0..4].try_into().unwrap()) as usize;
    let h = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let mut img = veymelo_raster::Image::new(w, h);
    for (i, p) in img.px.iter_mut().enumerate() {
        for (c, v) in p.iter_mut().enumerate() {
            let at = 8 + (i * 4 + c) * 4;
            *v = f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        }
    }
    img
}

/// The largest difference of a channel, the mean, and how many pixels differ by more than 2/255.
fn apart(a: &veymelo_raster::Image, b: &veymelo_raster::Image) -> Option<(f32, f32, usize)> {
    if a.width != b.width || a.height != b.height {
        return None;
    }
    let (mut most, mut sum, mut many) = (0f32, 0f64, 0usize);
    for (p, q) in a.px.iter().zip(&b.px) {
        let d = (0..4).map(|c| (p[c] - q[c]).abs()).fold(0f32, f32::max);
        most = most.max(d);
        sum += d as f64;
        if d > 2.0 / 255.0 {
            many += 1;
        }
    }
    Some((most, (sum / a.px.len().max(1) as f64) as f32, many))
}

fn side_by_side(a: &veymelo_raster::Image, b: &veymelo_raster::Image) -> veymelo_raster::Image {
    let (w, h) = (a.width, a.height);
    let mut out = veymelo_raster::Image::new(w * 3 + 16, h);
    for y in 0..h {
        for x in 0..w {
            let (p, q) = (a.px[y * w + x], b.px[y * w + x]);
            out.px[y * out.width + x] = p;
            out.px[y * out.width + w + 8 + x] = q;
            let d = |c: usize| ((p[c] - q[c]).abs() * 10.0).min(1.0);
            out.px[y * out.width + 2 * (w + 8) + x] = [d(0).max(d(3)), d(1).max(d(3)), d(2).max(d(3)), 1.0];
        }
    }
    out
}

fn file(dir: &Path, scene: &str, t: f64) -> PathBuf {
    dir.join(format!("{scene}-{t}.bin"))
}

#[test]
#[ignore]
fn goldens_3d() {
    let Some(dir) = std::env::var_os("VEYMELO_GOLDENS").map(PathBuf::from) else { return };
    let scale: f64 = std::env::var("VEYMELO_GOLDENS_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let dir = if scale == 1.0 { dir } else { dir.join(format!("x{scale}")) };
    let mut options = render::Options { scale, ..render::Options::default() };
    let cpu = std::env::var_os("VEYMELO_GOLDENS_CPU").is_some();
    if cpu {
        options.gpu = false;
    }
    let dir = if cpu || std::env::var_os("VEYMELO_GOLDENS_VS_CPU").is_some() { dir.join("cpu") } else { dir };
    std::fs::create_dir_all(dir.join("changed")).unwrap();
    let only = std::env::var("VEYMELO_GOLDENS_ONLY").ok();
    let rewrite = std::env::var_os("VEYMELO_GOLDENS_WRITE").is_some();
    let (mut written, mut same, mut changed) = (0, 0, vec![]);
    for (name, s, c, times) in scenes() {
        if only.as_deref().is_some_and(|w| !name.contains(w)) {
            continue;
        }
        for t in times {
            let started = std::time::Instant::now();
            let r = s.render(Some(&Json::Number(c as f64)), Some(veymelo_time::Tick::from_seconds(t)), &options).unwrap_or_else(|e| panic!("{name} at {t}: {e:?}"));
            eprintln!("{name} at {t}: {:.0} ms{}", started.elapsed().as_secs_f64() * 1000.0, if r.notes.is_empty() { String::new() } else { format!(" ({:?})", r.notes) });
            let path = file(&dir, name, t);
            if rewrite || !path.exists() {
                std::fs::write(&path, write_frame(&r.image)).unwrap();
                std::fs::write(path.with_extension("png"), render::to_png(&r.image)).unwrap();
                written += 1;
                continue;
            }
            let before = read_frame(&std::fs::read(&path).unwrap());
            match apart(&before, &r.image) {
                Some((most, _, _)) if most <= 1e-5 => same += 1,
                d => {
                    let what = d.map_or("another size".to_string(), |(most, mean, many)| format!("most {most:.4}, mean {mean:.5}, {many} pixels past 2/255"));
                    changed.push(format!("{name} at {t}: {what}"));
                    std::fs::write(dir.join("changed").join(format!("{name}-{t}.png")), render::to_png(&side_by_side(&before, &r.image))).unwrap();
                }
            }
        }
    }
    eprintln!("{written} written, {same} the same, {} changed", changed.len());
    for c in &changed {
        eprintln!("  {c}");
    }
    assert!(changed.is_empty(), "{} frames changed (see {})", changed.len(), dir.join("changed").display());
}

/// A scene with all the 3D renderer does (soft shadows from models and words, ambient
/// occlusion, a glossy floor, a chrome ball, bevelled words), drawn at 1080p on the GPU and by
/// the processor into `VEYMELO_SHOWCASE` (a folder), each timed:
///
///   VEYMELO_SHOWCASE=/tmp/s cargo test -p veymelo-engine --release showcase_3d -- --ignored
#[test]
#[ignore]
fn showcase_3d() {
    let Some(out) = std::env::var_os("VEYMELO_SHOWCASE").map(PathBuf::from) else { return };
    let mut s = Session::new();
    let c = id(&run(&mut s, "comp.new", r##"{"name": "Showcase", "width": 480, "height": 270, "fps": 30, "duration": 2, "background": "#e9e7e2"}"##), "comp");
    let floor = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#3a3d44", "width": 3000, "height": 3000, "name": "Floor"}"##), "layer");
    three_d(&mut s, floor);
    set(&mut s, floor, "transform/position", "[240, 230, 600]");
    set(&mut s, floor, "transform/rotationX", "90");
    set(&mut s, floor, "materialOptions/reflectionIntensity", "45");
    set(&mut s, floor, "materialOptions/reflectionSharpness", "88");
    set(&mut s, floor, "materialOptions/reflectionRolloff", "50");
    let wall = id(&run(&mut s, "layer.add", r##"{"type": "solid", "color": "#d8d2c6", "width": 3000, "height": 900, "name": "Wall"}"##), "layer");
    three_d(&mut s, wall);
    set(&mut s, wall, "transform/position", "[240, -220, 420]");
    let cam = id(&run(&mut s, "layer.add", r#"{"type": "camera", "lens": 35}"#), "layer");
    set(&mut s, cam, "transform/position", "[110, 40, -360]");
    set(&mut s, cam, "transform/pointOfInterest", "[250, 170, 180]");
    let shiny = |s: &mut Session, l: u64| {
        set(s, l, "materialOptions/castsShadows", "\"on\"");
    };
    let cube = primitive(&mut s, "cube", [150.0, 185.0, 210.0], "#c8323c", 0.0, 0.45);
    set(&mut s, cube, "transform/rotationY", "28");
    shiny(&mut s, cube);
    let ball = primitive(&mut s, "sphere", [265.0, 180.0, 170.0], "#d9d9de", 1.0, 0.12);
    set(&mut s, ball, "materialOptions/reflectionIntensity", "75");
    shiny(&mut s, ball);
    let ring = primitive(&mut s, "torus", [370.0, 182.0, 240.0], "#0f8b8d", 0.0, 0.35);
    set(&mut s, ring, "transform/rotationX", "70");
    shiny(&mut s, ring);
    let cone = primitive(&mut s, "cone", [330.0, 180.0, 120.0], "#e8a33d", 0.3, 0.4);
    shiny(&mut s, cone);
    let t = id(&run(&mut s, "layer.add", r#"{"type": "text", "text": "Veymelo"}"#), "layer");
    three_d(&mut s, t);
    set(&mut s, t, "transform/position", "[250, 95, 330]");
    set(&mut s, t, "transform/scale", "[120, 120, 120]");
    set(&mut s, t, "geometryOptions/extrusionDepth", "18");
    set(&mut s, t, "geometryOptions/bevelStyle", "\"convex\"");
    set(&mut s, t, "geometryOptions/bevelDepth", "3");
    set(&mut s, t, "materialOptions/castsShadows", "\"on\"");
    let key = light(&mut s, "spot", [60.0, -180.0, -60.0], true);
    set(&mut s, key, "lightOptions/coneAngle", "120");
    set(&mut s, key, "lightOptions/shadowDiffusion", "18");
    set(&mut s, key, "lightOptions/intensity", "85");
    let fill = light(&mut s, "parallel", [420.0, -100.0, -200.0], false);
    set(&mut s, fill, "lightOptions/intensity", "25");
    let amb = light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
    set(&mut s, amb, "lightOptions/intensity", "45");
    set(&mut s, amb, "lightOptions/ambientOcclusion", "100");
    set(&mut s, amb, "lightOptions/occlusionDistance", "70");
    for (name, gpu) in [("gpu", true), ("cpu", false)] {
        let started = std::time::Instant::now();
        let r = s.render(Some(&Json::Number(c as f64)), Some(veymelo_time::Tick::from_seconds(0.0)), &render::Options { scale: 4.0, gpu: gpu && render::Options::default().gpu, ..render::Options::default() }).unwrap();
        eprintln!("{name}: {:.0} ms {:?}", started.elapsed().as_secs_f64() * 1000.0, r.notes);
        std::fs::write(out.join(format!("showcase-{name}.png")), render::to_png(&r.image)).unwrap();
    }
    // a room lit by light bouncing round it, a glass ball and a chrome one in it
    {
        let mut s = Session::new();
        let c = id(&run(&mut s, "comp.new", r##"{"name": "Room", "width": 480, "height": 270, "fps": 30, "duration": 2, "background": "#101010"}"##), "comp");
        run(&mut s, "comp.set", r#"{"lightBounce": 100, "bounces": 2}"#);
        let wall = |s: &mut Session, color: &str, size: (f64, f64), at: [f64; 3], rot: (&str, f64)| {
            let l = id(&run(s, "layer.add", &format!(r##"{{"type": "solid", "color": "{color}", "width": {}, "height": {}}}"##, size.0, size.1)), "layer");
            three_d(s, l);
            set(s, l, "transform/position", &format!("[{}, {}, {}]", at[0], at[1], at[2]));
            if rot.1 != 0.0 {
                set(s, l, &format!("transform/{}", rot.0), &format!("{}", rot.1));
            }
            l
        };
        let floor = wall(&mut s, "#cdb894", (700.0, 600.0), [240.0, 300.0, 250.0], ("rotationX", 90.0));
        set(&mut s, floor, "materialOptions/reflectionIntensity", "25");
        set(&mut s, floor, "materialOptions/reflectionSharpness", "80");
        set(&mut s, floor, "materialOptions/reflectionRolloff", "60");
        wall(&mut s, "#ece8e0", (700.0, 600.0), [240.0, -60.0, 250.0], ("rotationX", 90.0));
        wall(&mut s, "#ece8e0", (700.0, 400.0), [240.0, 120.0, 520.0], ("rotationX", 0.0));
        wall(&mut s, "#2f6db5", (600.0, 400.0), [-90.0, 120.0, 250.0], ("rotationY", 90.0));
        wall(&mut s, "#e2752d", (600.0, 400.0), [570.0, 120.0, 250.0], ("rotationY", 90.0));
        let glass = primitive(&mut s, "sphere", [170.0, 245.0, 260.0], "#ffffff", 0.0, 0.05);
        set(&mut s, glass, "materialOptions/transparency", "92");
        set(&mut s, glass, "materialOptions/indexOfRefraction", "1.5");
        set(&mut s, glass, "materialOptions/reflectionIntensity", "12");
        let chrome = primitive(&mut s, "sphere", [320.0, 245.0, 330.0], "#e8e8ee", 1.0, 0.05);
        set(&mut s, chrome, "materialOptions/reflectionIntensity", "80");
        let block = id(&run(&mut s, "layer.newPrimitive", r##"{"shape": "cube", "position": [420, 230, 430], "baseColor": "#f2efe8", "metallic": 0, "roughness": 0.6, "width": 80, "height": 140, "depth": 80}"##), "layer");
        set(&mut s, block, "transform/rotationY", "25");
        for l in [chrome, block] {
            set(&mut s, l, "materialOptions/castsShadows", "\"on\"");
        }
        let cam = id(&run(&mut s, "layer.add", r#"{"type": "camera", "lens": 35}"#), "layer");
        set(&mut s, cam, "transform/position", "[240, 120, -330]");
        set(&mut s, cam, "transform/pointOfInterest", "[260, 200, 300]");
        let key = light(&mut s, "point", [260.0, -30.0, 230.0], true);
        set(&mut s, key, "lightOptions/intensity", "95");
        set(&mut s, key, "lightOptions/shadowDiffusion", "25");
        let amb = light(&mut s, "ambient", [0.0, 0.0, 0.0], false);
        set(&mut s, amb, "lightOptions/intensity", "10");
        set(&mut s, amb, "lightOptions/ambientOcclusion", "100");
        set(&mut s, amb, "lightOptions/occlusionDistance", "60");
        for (name, gpu) in [("gpu", true), ("cpu", false)] {
            let started = std::time::Instant::now();
            let r = s.render(Some(&Json::Number(c as f64)), Some(veymelo_time::Tick::from_seconds(0.0)), &render::Options { scale: 4.0, gpu: gpu && render::Options::default().gpu, ..render::Options::default() }).unwrap();
            eprintln!("room {name}: {:.0} ms {:?}", started.elapsed().as_secs_f64() * 1000.0, r.notes);
            std::fs::write(out.join(format!("showcase-room-{name}.png")), render::to_png(&r.image)).unwrap();
        }
    }
    // looked round, as the preview is
    let orbit = Some(veymelo_expr::Orbit { comp: c, turn: 35.0, tilt: 12.0, dolly: 1.15 });
    let r = s.render(Some(&Json::Number(c as f64)), Some(veymelo_time::Tick::from_seconds(0.0)), &render::Options { scale: 4.0, orbit, ..render::Options::default() }).unwrap();
    std::fs::write(out.join("showcase-orbit.png"), render::to_png(&r.image)).unwrap();
}
