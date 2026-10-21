use super::*;

/// Numbers that look random in −1 to 1, the same each time.
fn shake(i: usize) -> f64 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x % 2_000_001) as f64 / 1_000_000.0 - 1.0
}

/// Points in a box in front of the cameras, seen by `poses` through a lens of focal `f` on
/// 1280 × 720 frames: the runs of frames each is seen on (at most 15 to 34 long), off by `noise`
/// pixels at most.
fn seen(points: &[V3], poses: &[Pose], f: f64, noise: f64) -> Vec<Track> {
    let c = [640.0, 360.0];
    let mut tracks = Vec::new();
    for (i, x) in points.iter().enumerate() {
        let mut run: Option<Track> = None;
        for (k, p) in poses.iter().enumerate() {
            let q = p.to_camera(*x);
            let at = [c[0] + f * q[0] / q[2] + noise * shake(i * 1000 + k), c[1] + f * q[1] / q[2] + noise * shake(i * 1000 + k + 500)];
            let inside = q[2] > 0.1 && at[0] > 0.0 && at[0] < 1280.0 && at[1] > 0.0 && at[1] < 720.0;
            match (&mut run, inside) {
                (Some(t), true) if t.at.len() < 15 + i % 20 => t.at.push(at),
                (_, true) => {
                    if let Some(t) = run.take().filter(|t| t.at.len() >= 3) {
                        tracks.push(t);
                    }
                    run = Some(Track { first: k, at: vec![at] });
                }
                (_, false) => {
                    if let Some(t) = run.take().filter(|t| t.at.len() >= 3) {
                        tracks.push(t);
                    }
                }
            }
        }
        if let Some(t) = run.filter(|t| t.at.len() >= 3) {
            tracks.push(t);
        }
    }
    tracks
}

fn scene(n: usize) -> Vec<V3> {
    (0..n).map(|i| [4.0 * shake(3 * i), 1.0 + 2.0 * shake(3 * i + 1), 10.0 + 4.0 * shake(3 * i + 2)]).collect()
}

/// A camera dollying sideways and a little forward, turning to keep the scene in view.
fn dolly(frames: usize) -> Vec<Pose> {
    (0..frames)
        .map(|k| {
            let s = k as f64 / frames as f64;
            let center = [-2.0 + 4.0 * s, -0.3 * s, 1.5 * s];
            let r = mul(&rodrigues([0.02 * (k as f64 * 0.3).sin(), -0.25 + 0.5 * s, 0.03 * s]), &I3);
            Pose { r, t: mv(&r, &center).map(|v| -v) }
        })
        .collect()
}

/// The similarity (scale, turn, move) taking points `a` nearest onto `b` (Umeyama's).
fn align(a: &[V3], b: &[V3]) -> (f64, M3, V3) {
    let n = a.len() as f64;
    let ma = a.iter().fold([0.0; 3], |s, p| add(s, *p)).map(|v| v / n);
    let mb = b.iter().fold([0.0; 3], |s, p| add(s, *p)).map(|v| v / n);
    let mut cov = [[0.0; 3]; 3];
    let mut var = 0.0;
    for (p, q) in a.iter().zip(b) {
        let (da, db) = (sub(*p, ma), sub(*q, mb));
        var += dot(da, da) / n;
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += db[i] * da[j] / n;
            }
        }
    }
    let (u, d, v) = svd3(&cov);
    let mut sgn = [1.0, 1.0, 1.0];
    if det(&u) * det(&v) < 0.0 {
        sgn[2] = -1.0;
    }
    let s_m: M3 = [[sgn[0], 0.0, 0.0], [0.0, sgn[1], 0.0], [0.0, 0.0, sgn[2]]];
    let r = mul(&mul(&u, &s_m), &tr(&v));
    let scale = (d[0] * sgn[0] + d[1] * sgn[1] + d[2] * sgn[2]) / var;
    let t = sub(mb, mv(&r, &ma).map(|x| x * scale));
    (scale, r, t)
}

fn path_error(sol: &Solution, truth: &[Pose]) -> f64 {
    let pairs: Vec<(V3, V3)> = sol.cameras.iter().zip(truth).filter_map(|(c, t)| Some((c.as_ref()?.center(), t.center()))).collect();
    let (a, b): (Vec<V3>, Vec<V3>) = pairs.into_iter().unzip();
    let (s, r, t) = align(&a, &b);
    let worst = a.iter().zip(&b).map(|(p, q)| norm(sub(add(mv(&r, p).map(|x| x * s), t), *q))).fold(0.0, f64::max);
    worst / norm(sub(b[0], b[b.len() - 1]))
}

#[test]
fn the_parts_of_a_matrix_put_back_together() {
    let a: M3 = [[2.0, -1.0, 0.5], [0.3, 1.7, -2.0], [1.1, 0.2, 0.9]];
    let (u, s, v) = svd3(&a);
    let d: M3 = [[s[0], 0.0, 0.0], [0.0, s[1], 0.0], [0.0, 0.0, s[2]]];
    let back = mul(&mul(&u, &d), &tr(&v));
    for i in 0..3 {
        for j in 0..3 {
            assert!((back[i][j] - a[i][j]).abs() < 1e-9, "{back:?}");
        }
    }
    assert!(s[0] >= s[1] && s[1] >= s[2]);
    let r = rodrigues([0.3, -0.2, 0.5]);
    let q = from_quat(quat(&r));
    assert!((0..3).all(|i| (0..3).all(|j| (q[i][j] - r[i][j]).abs() < 1e-12)));
}

#[test]
fn a_moving_camera_is_found_with_its_lens_given() {
    let truth = dolly(40);
    let f = 640.0 / (25f64.to_radians()).tan();
    let tracks = seen(&scene(300), &truth, f, 0.3);
    let sol = solve(&tracks, 40, [0.0, 0.0, 1280.0, 720.0], &Settings { angle: Some(50.0), how: None }).expect("solved");
    assert_eq!(sol.how, Solve::Typical);
    assert!(sol.cameras.iter().all(Option::is_some), "every frame placed");
    assert!(sol.error < 0.5, "{}", sol.error);
    let off = path_error(&sol, &truth);
    assert!(off < 0.01, "the path off by {off} of its length");
    // kept as text and read back
    let (back, start, step) = from_text(&to_text(&sol, 1.0, 0.04)).unwrap();
    assert_eq!((start, step), (1.0, 0.04));
    assert!((back.error - sol.error).abs() < 1e-5 && back.cameras.len() == 40);
    assert!((mean_error(&back, &tracks) - sol.error).abs() < 0.05 || back.points.len() != sol.points.len());
}

#[test]
fn a_lens_not_given_is_found() {
    let truth = dolly(40);
    let f = 640.0 / (25f64.to_radians()).tan();
    let tracks = seen(&scene(300), &truth, f, 0.3);
    let sol = solve(&tracks, 40, [0.0, 0.0, 1280.0, 720.0], &Settings { angle: None, how: None }).expect("solved");
    assert!((sol.focal / f - 1.0).abs() < 0.05, "focal {} for {f}", sol.focal);
    assert!(sol.error < 0.5, "{}", sol.error);
}

#[test]
fn a_camera_that_only_turns_is_a_tripod_pan() {
    let truth: Vec<Pose> = (0..30).map(|k| Pose { r: rodrigues([0.01 * (k as f64 * 0.4).sin(), 0.012 * k as f64, 0.0]), t: [0.0; 3] }).collect();
    let f = 640.0 / (30f64.to_radians()).tan();
    let tracks = seen(&scene(300), &truth, f, 0.2);
    let sol = solve(&tracks, 30, [0.0, 0.0, 1280.0, 720.0], &Settings { angle: Some(60.0), how: None }).expect("solved");
    assert_eq!(sol.how, Solve::Tripod);
    assert!(sol.error < 0.5, "{}", sol.error);
    // each frame's turn from the first as the camera's was
    for k in [10, 29] {
        let (got, want) = (mul(&sol.cameras[k].unwrap().r, &tr(&sol.cameras[0].unwrap().r)), mul(&truth[k].r, &tr(&truth[0].r)));
        let d: f64 = (0..3).flat_map(|i| (0..3).map(move |j| (i, j))).map(|(i, j)| (got[i][j] - want[i][j]).abs()).fold(0.0, f64::max);
        assert!(d < 2e-3, "frame {k}: {d}");
    }
}
