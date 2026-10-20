use super::*;

const SAMPLE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200" height="100" viewBox="0 0 400 200">
  <style>
    .warm { fill: #ff8000 }
    #big { stroke: navy; stroke-width: 10 }
    rect { fill-opacity: 1 }
  </style>
  <defs>
    <linearGradient id="fade" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="red"/>
      <stop offset="100%" stop-color="blue"/>
    </linearGradient>
    <linearGradient id="again" xlink:href="#fade" gradientUnits="userSpaceOnUse" x1="0" x2="400"/>
    <symbol id="dot" viewBox="0 0 10 10"><circle cx="5" cy="5" r="5" fill="lime"/></symbol>
  </defs>
  <rect id="big" class="warm" x="20" y="20" width="160" height="160" rx="20"/>
  <g transform="translate(200 0)" opacity="0.5">
    <rect x="20" y="20" width="160" height="80" fill="url(#fade)"/>
    <rect x="20" y="20" width="160" height="80" fill="black" style="fill-opacity: 0"/>
  </g>
  <rect x="220" y="120" width="160" height="60" style="fill:url(#again)"/>
  <use xlink:href="#dot" x="380" y="0" width="20" height="20"/>
  <text x="0" y="0">words</text>
  <path d="M0 200 L 10 190 Z" fill="none" stroke="currentColor" color="#00f"/>
</svg>"##;

#[test]
fn a_document_read() {
    let doc = parse(SAMPLE.as_bytes()).unwrap();
    assert_eq!((doc.width, doc.height), (200.0, 100.0));
    assert_eq!(doc.skipped, vec!["text".to_string()]);
    let shapes = doc.shapes();
    assert_eq!(shapes.len(), 6);
    // the style sheet's class and id rules, and a round rectangle kept a rectangle
    let (_, _, big) = shapes[0];
    assert_eq!(big.name, "big");
    assert_eq!(big.fill, Some(Paint::Color([1.0, 128.0 / 255.0, 0.0])));
    assert!(matches!(big.geom, Geom::Rect { rx, .. } if rx == 20.0));
    let stroke = big.stroke.as_ref().unwrap();
    assert_eq!((stroke.width, &stroke.paint), (10.0, &Paint::Color([0.0, 0.0, 128.0 / 255.0])));
    // the viewBox halves everything: the big square's corner at (10, 10) in the picture
    let (m, _, _) = shapes[0];
    let corner = m.apply(vec2(20.0, 20.0));
    assert!((corner.x - 10.0).abs() < 1e-9 && (corner.y - 10.0).abs() < 1e-9);
    // the group's opacity reaches its shapes; a gradient inheriting its stops by href
    assert_eq!(shapes[1].1, 0.5);
    let Some(Paint::Gradient(g)) = &shapes[3].2.fill else { panic!("a gradient") };
    assert!(!g.box_units && g.stops.len() == 2 && matches!(g.kind, GradientKind::Linear { x2, .. } if x2 == 400.0));
    // a symbol used at its place and size: a circle 20 across at (380, 0)
    let (m, _, dot) = shapes[4];
    assert_eq!(dot.fill, Some(Paint::Color([0.0, 1.0, 0.0])));
    let centre = m.apply(vec2(5.0, 5.0));
    assert!((centre.x - 195.0).abs() < 1e-9 && (centre.y - 5.0).abs() < 1e-9, "{centre:?}");
    // currentColor for a stroke
    assert_eq!(shapes[5].2.stroke.as_ref().map(|s| s.paint.clone()), Some(Paint::Color([0.0, 0.0, 1.0])));
    assert!(parse(b"<html/>").unwrap_err().contains("not an SVG"));
    assert!(looks_like(SAMPLE.as_bytes()) && !looks_like(b"\x89PNG"));
}

#[test]
fn a_document_drawn() {
    let doc = parse(SAMPLE.as_bytes()).unwrap();
    let color = |c: [f64; 4]| -> [f32; 4] { [(c[0] * c[3]) as f32, (c[1] * c[3]) as f32, (c[2] * c[3]) as f32, c[3] as f32] };
    // twice the picture's size
    let img = draw(&doc, Mat3::scale(vec2(2.0, 2.0)), 400, 200, &color);
    let at = |x: usize, y: usize| img.px[y * 400 + x];
    // the orange square's middle, its navy edge, outside it
    assert_eq!(at(100, 100), [1.0, 128.0 / 255.0, 0.0, 1.0]);
    let edge = at(100, 21);
    assert!(edge[2] > 0.4 && edge[0] < 0.1, "{edge:?}");
    assert_eq!(at(5, 100)[3], 0.0);
    // the fading group: the gradient at half (red on the left, blue on the right)
    let (left, right) = (at(222, 40), at(378, 40));
    assert!((left[3] - 0.5).abs() < 0.02 && left[0] > left[2] && right[2] > right[0], "{left:?} {right:?}");
    // the gradient over the whole width in user units: purple-ish in the middle of its rectangle
    let mid = at(300, 150);
    assert!(mid[0] > 0.2 && mid[2] > 0.2 && mid[3] > 0.99, "{mid:?}");
    // the used symbol's green dot
    assert_eq!(at(390, 10), [0.0, 1.0, 0.0, 1.0]);
}

#[test]
fn radial_gradients_and_their_spread() {
    let svg = r##"<svg width="100" height="100"><defs>
        <radialGradient id="r" r="0.25" spreadMethod="repeat"><stop offset="0" stop-color="white"/><stop offset="1" stop-color="black"/></radialGradient>
        </defs><rect width="100" height="100" fill="url(#r)"/></svg>"##;
    let doc = parse(svg.as_bytes()).unwrap();
    let img = draw(&doc, Mat3::IDENTITY, 100, 100, &|c| [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32]);
    let at = |x: usize, y: usize| img.px[y * 100 + x][0];
    // white at the middle, darker out to a quarter away, white again just past it
    assert!(at(50, 50) > 0.95 && at(50, 73) < 0.15 && at(50, 77) > 0.85, "{} {} {}", at(50, 50), at(50, 73), at(50, 77));
}

