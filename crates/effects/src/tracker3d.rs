//! The 3D Camera Tracker: footage's camera worked out (effect.analyze follows its features and
//! solves where the camera was on each frame and where the followed points are in the world,
//! see `veymelo_track::scene3d`), kept in its Solution setting; camera.fromTracker then makes a
//! 3D camera that moves as the footage's did, and puts things on its points. The footage itself
//! is shown as it is, its solved points drawn over it when asked.

use std::sync::{Arc, Mutex};

use veymelo_geom::vec2;
use veymelo_keyframe::{Kind, Value};
use veymelo_project::{PropDef, Unit};
use veymelo_track::scene3d::{Solution, from_text};

use crate::defs::{choice, effect, flag, num, pct};
use crate::{Buf, Ctx, Spec};

const CATEGORY: &str = "perspective";

/// The shot types: the angle of view found, or given.
pub const SHOT_TYPES: &[&str] = &["fixedAngleOfView", "specifyAngleOfView"];

/// How the shot is solved: whichever fits, a camera moving through the scene, or one only turning.
pub const SOLVE_METHODS: &[&str] = &["autoDetect", "typical", "tripodPan"];

effect!(
    TRACKER_DEF,
    "camera3dTracker",
    "3D Camera Tracker",
    "footage's camera worked out: effect.analyze follows its features and solves where the camera was on each frame and where the points are; camera.fromTracker makes a 3D camera that moves as it did, and things standing on its points",
    [
        choice("shotType", "Shot Type", SHOT_TYPES, 0).doc("fixedAngleOfView: the lens's angle of view found (it does not zoom); specifyAngleOfView: given in Horizontal Angle of View"),
        num("horizontalAngleOfView", "Horizontal Angle of View", 40.0).range(1.0, 179.0).unit(Unit::Degrees).doc("how wide the lens sees across, when given"),
        choice("solveMethod", "Solve Method", SOLVE_METHODS, 0).doc("autoDetect: whichever fits; typical: the camera moves through the scene; tripodPan: it only turns"),
        flag("renderTrackPoints", "Render Track Points", false).doc("draw the solved points where they are seen on each frame"),
        pct("trackPointSize", "Track Point Size", 100.0, 10.0, 1000.0),
        PropDef::new("solution", "Solution", Kind::Text, Value::Text(String::new())).setting().doc("where the camera was on each frame and where the points are, as effect.analyze worked it out"),
    ]
);
pub const TRACKER: Spec = Spec::new(&TRACKER_DEF, CATEGORY, run);

pub const ALL: [Spec; 1] = [TRACKER];

/// A solution, its frames' first time and the time between them.
pub type Solved = (Solution, f64, f64);

/// The solution an effect keeps (and its frames' first time and the time between them), read
/// once for as long as it stays the same.
pub fn solution(text: &str) -> Option<Arc<Solved>> {
    static READ: Mutex<Option<(u64, Arc<Solved>)>> = Mutex::new(None);
    if text.is_empty() {
        return None;
    }
    let key = text.bytes().fold(0xCBF2_9CE4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01B3));
    let mut kept = READ.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, s)) = kept.as_ref()
        && *k == key
    {
        return Some(s.clone());
    }
    let s = Arc::new(from_text(text)?);
    *kept = Some((key, s.clone()));
    Some(s)
}

fn run(ctx: &Ctx, mut buf: Buf) -> Buf {
    let text = ctx.value_in(ctx.effect, "solution").map(|v| v.as_text().to_string()).unwrap_or_default();
    let Some(s) = solution(&text) else {
        ctx.note("3D Camera Tracker has not solved the camera yet: effect.analyze does".into());
        return buf;
    };
    if !ctx.flag("renderTrackPoints") {
        return buf;
    }
    let (sol, start, step) = (&s.0, s.1, s.2);
    let k = ((ctx.time.seconds() - start) / step.max(1e-9)).round();
    if k < 0.0 || k as usize >= sol.cameras.len() {
        return buf;
    }
    let k = k as usize;
    let r = 3.0 * ctx.num("trackPointSize", 100.0) / 100.0 * buf.scale;
    // nearer points warmer
    let depths: Vec<f64> = sol.points.iter().flatten().filter_map(|x| Some(sol.cameras[k].as_ref()?.to_camera(*x)[2])).filter(|z| *z > 0.0).collect();
    let (near, far) = depths.iter().fold((f64::INFINITY, 0.0f64), |(a, b), z| (a.min(*z), b.max(*z)));
    let (w, h) = (buf.img.width as i64, buf.img.height as i64);
    for x in sol.points.iter().flatten() {
        let (Some(at), Some(cam)) = (sol.seen(k, *x), sol.cameras[k].as_ref()) else { continue };
        let f = ((cam.to_camera(*x)[2] - near) / (far - near).max(1e-9)).clamp(0.0, 1.0) as f32;
        let colour = [1.0 - 0.8 * f, 0.6 * (1.0 - (2.0 * f - 1.0).abs()) + 0.2, 0.2 + 0.8 * f];
        let p = buf.pixel(vec2(at[0], at[1]));
        for y in (p.y - r - 1.0).floor() as i64..=(p.y + r + 1.0).ceil() as i64 {
            for xx in (p.x - r - 1.0).floor() as i64..=(p.x + r + 1.0).ceil() as i64 {
                if xx < 0 || y < 0 || xx >= w || y >= h {
                    continue;
                }
                let d = ((xx as f64 + 0.5 - p.x).powi(2) + (y as f64 + 0.5 - p.y).powi(2)).sqrt();
                let a = (r + 0.5 - d).clamp(0.0, 1.0) as f32;
                let q = &mut buf.img.px[(y * w + xx) as usize];
                *q = [colour[0] * a + q[0] * (1.0 - a), colour[1] * a + q[1] * (1.0 - a), colour[2] * a + q[2] * (1.0 - a), a + q[3] * (1.0 - a)];
            }
        }
    }
    buf
}
