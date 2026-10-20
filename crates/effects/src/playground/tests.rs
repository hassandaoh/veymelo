use super::*;
use crate::testing::{instance, n, run_when};
use veymelo_keyframe::Value;

fn go(values: &[(&str, Value)], t: f64, masks: &[crate::MaskShape]) -> Buf {
    let img = Image::filled(100, 80, [0.2, 0.2, 0.2, 1.0]);
    let mut g = instance(&PLAYGROUND, &[]);
    PLAYGROUND.start(&mut g, veymelo_geom::Rect::from_size(100.0, 80.0));
    for (path, v) in values {
        g.prop_mut(path).unwrap_or_else(|e| panic!("{path}: {e}")).set_value(v.clone()).unwrap();
    }
    run_when(&PLAYGROUND, &g, img, Tick::from_seconds(t), masks, None)
}

/// How much is drawn, and its middle.
fn mass(img: &Image) -> (f32, f32, f32) {
    let (mut m, mut sx, mut sy) = (0f32, 0f32, 0f32);
    for (i, p) in img.px.iter().enumerate() {
        m += p[3];
        sx += p[3] * (i % img.width) as f32;
        sy += p[3] * (i / img.width) as f32;
    }
    (m, sx / m.max(1e-9), sy / m.max(1e-9))
}

#[test]
fn the_cannon_fires_and_gravity_pulls() {
    let start = go(&[], 0.0, &[]);
    assert!(start.img.px.iter().all(|p| p[3] == 0.0), "nothing yet, the layer replaced");
    let a = go(&[], 0.5, &[]);
    assert_eq!(a.img.px, go(&[], 0.5, &[]).img.px, "the same each time");
    let (m, x, y) = mass(&a.img);
    assert!(m > 5.0 && (x - 50.0).abs() < 8.0 && y < 72.0, "shot up from the cannon: {m} {x} {y}");
    let red = a.img.px.iter().filter(|p| p[0] > 0.5 && p[1] < 0.1).count();
    assert!(red > 5, "red dots");
    // no gravity: higher (before any leave the top)
    let (_, _, y_free) = mass(&go(&[("gravityForce", n(0.0))], 0.3, &[]).img);
    let (_, _, y_heavy) = mass(&go(&[], 0.3, &[]).img);
    assert!(y_free < y_heavy, "{y_free} {y_heavy}");
    // pointed right
    let (_, x_right, _) = mass(&go(&[("cannonDirection", n(90.0)), ("gravityForce", n(0.0))], 0.3, &[]).img);
    assert!(x_right > 55.0, "{x_right}");
}

#[test]
fn the_grid_fires_every_frame() {
    let v = [("cannonParticlesPerSecond", n(0.0)), ("gridParticlesAcross", n(4.0)), ("gridParticlesDown", n(3.0)), ("gridWidth", n(60.0)), ("gridHeight", n(40.0)), ("gravityForce", n(200.0))];
    let out = go(&v, 0.5, &[]);
    let (m, _, y) = mass(&out.img);
    assert!(m > 10.0 && y > 40.0, "falling columns below the grid: {m} {y}");
    // still, the grid holds its points
    let still = go(&[v[0].clone(), v[1].clone(), v[2].clone(), v[3].clone(), v[4].clone(), ("gravityForce", n(0.0))], 0.5, &[]);
    let (_, sx, sy) = mass(&still.img);
    assert!((sx - 50.0).abs() < 1.0 && (sy - 40.0).abs() < 1.0, "{sx} {sy}");
}

#[test]
fn walls_keep_them_in() {
    let box_ = crate::MaskShape { name: "Mask 1".into(), points: vec![[20.0, 20.0], [80.0, 20.0], [80.0, 70.0], [20.0, 70.0]], closed: true, inverted: false };
    let v = [("wallBoundary", n(1.0)), ("cannonPosition", Value::Vec2([50.0, 50.0])), ("cannonVelocity", n(200.0)), ("cannonDirectionRandomSpread", n(360.0)), ("gravityForce", n(0.0))];
    let out = go(&v, 1.5, &[box_]);
    let outside: f32 = out.img.px.iter().enumerate().filter(|(i, _)| { let (x, y) = (i % 100, i / 100); !(17..84).contains(&x) || !(17..74).contains(&y) }).map(|(_, p)| p[3]).sum();
    let (m, _, _) = mass(&out.img);
    assert!(m > 20.0 && outside < m * 0.02, "{outside} of {m} outside the wall");
}

#[test]
fn letters_and_mappers() {
    if crate::words::font_for("", 400.0).is_none() {
        return;
    }
    let dots = go(&[("cannonParticlesPerSecond", n(20.0))], 0.6, &[]);
    let text = go(&[("cannonParticlesPerSecond", n(20.0)), ("cannonText", Value::Text("AB".into())), ("fontSize", n(14.0))], 0.6, &[]);
    assert!(mass(&text.img).0 > mass(&dots.img).0 * 2.0, "letters cover more than dots");
    // repel spreads them out
    let spread = |img: &Image| {
        let (m, x, y) = mass(img);
        img.px.iter().enumerate().map(|(i, p)| p[3] * (((i % 100) as f32 - x).powi(2) + ((i / 100) as f32 - y).powi(2))).sum::<f32>() / m.max(1e-9)
    };
    let tight = go(&[("cannonDirectionRandomSpread", n(0.0)), ("cannonVelocityRandomSpread", n(0.0)), ("gravityForce", n(0.0))], 0.4, &[]);
    let pushed = go(&[("cannonDirectionRandomSpread", n(0.0)), ("cannonVelocityRandomSpread", n(0.0)), ("gravityForce", n(0.0)), ("repelForce", n(20.0)), ("repelForceRadius", n(15.0))], 0.4, &[]);
    assert!(spread(&pushed.img) > spread(&tight.img), "pushed apart");
    // bursting: smaller pieces, more of them
    let burst = go(&[("cannonParticleRadius", n(5.0)), ("particleExploderRadiusOfNewParticles", n(1.5))], 0.4, &[]);
    assert!(burst.img.px.iter().filter(|p| p[3] > 0.0).count() != dots.img.px.iter().filter(|p| p[3] > 0.0).count());
}

#[test]
fn affects_picks_by_age() {
    let mut a = Affects::default();
    let mut q = Particle::new([0.0; 2], [0.0; 2], [1.0; 4], 1.0, CANNON, 0);
    q.age = 2.0;
    a.age = 1.0;
    assert_eq!(a.weight(&q), 1.0, "older than one second");
    a.age = -1.0;
    assert_eq!(a.weight(&q), 0.0, "not younger than one second");
    a.age = 2.0;
    a.feather = 1.0;
    assert!((a.weight(&q) - 0.5).abs() < 1e-6, "on the line, half");
    a = Affects { from: 2, ..Affects::default() };
    assert_eq!(a.weight(&q), 0.0, "grid only");
}
