//! Colours as CSS writes them: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()` and `rgba()`
//! (numbers or percentages, commas or spaces and a slash), `hsl()` and `hsla()`, the named
//! colours of CSS Color Level 3 (and `rebeccapurple`), and `transparent`. Straight RGBA from 0
//! to 1.

/// The named colours, in order.
const NAMED: [(&str, u32); 148] = [
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

/// A number, or a percentage of `full`.
fn amount(s: &str, full: f64) -> Option<f64> {
    let s = s.trim();
    match s.strip_suffix('%') {
        Some(p) => p.trim().parse::<f64>().ok().map(|v| v / 100.0 * full),
        None => s.parse::<f64>().ok(),
    }
}

/// An angle in degrees (`deg`, `rad`, `grad`, `turn` or a bare number).
fn angle(s: &str) -> Option<f64> {
    let s = s.trim();
    for (unit, k) in [("deg", 1.0), ("grad", 0.9), ("rad", 180.0 / std::f64::consts::PI), ("turn", 360.0)] {
        if let Some(v) = s.strip_suffix(unit) {
            return v.trim().parse::<f64>().ok().map(|v| v * k);
        }
    }
    s.parse().ok()
}

/// The colour `s` writes.
pub fn parse_color(s: &str) -> Option<[f64; 4]> {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix('#') {
        let digit = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok().map(|v| v as f64 / 15.0);
        let pair = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok().map(|v| v as f64 / 255.0);
        return match hex.len() {
            3 => Some([digit(0)?, digit(1)?, digit(2)?, 1.0]),
            4 => Some([digit(0)?, digit(1)?, digit(2)?, digit(3)?]),
            6 => Some([pair(0)?, pair(2)?, pair(4)?, 1.0]),
            8 => Some([pair(0)?, pair(2)?, pair(4)?, pair(6)?]),
            _ => None,
        };
    }
    if lower == "transparent" {
        return Some([0.0; 4]);
    }
    if let Some(open) = lower.find('(') {
        let name = lower[..open].trim();
        let inner = lower[open + 1..].trim_end().strip_suffix(')')?;
        // the alpha after a slash, or as a fourth number
        let (main, slash) = match inner.split_once('/') {
            Some((m, a)) => (m, Some(a)),
            None => (inner, None),
        };
        let parts: Vec<&str> = main.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
        let alpha = match (slash, parts.get(3).copied()) {
            (Some(a), _) | (None, Some(a)) => amount(a, 1.0)?.clamp(0.0, 1.0),
            (None, None) => 1.0,
        };
        if parts.len() < 3 {
            return None;
        }
        return match name {
            "rgb" | "rgba" => Some([amount(parts[0], 255.0)? / 255.0, amount(parts[1], 255.0)? / 255.0, amount(parts[2], 255.0)? / 255.0, alpha].map(|v| v.clamp(0.0, 1.0))),
            "hsl" | "hsla" => {
                let h = angle(parts[0])?.rem_euclid(360.0) / 360.0;
                let (sat, l) = (amount(parts[1], 1.0)?.clamp(0.0, 1.0), amount(parts[2], 1.0)?.clamp(0.0, 1.0));
                let q = if l < 0.5 { l * (1.0 + sat) } else { l + sat - l * sat };
                let p = 2.0 * l - q;
                let hue = |t: f64| {
                    let t = t.rem_euclid(1.0);
                    if t < 1.0 / 6.0 {
                        p + (q - p) * 6.0 * t
                    } else if t < 0.5 {
                        q
                    } else if t < 2.0 / 3.0 {
                        p + (q - p) * (2.0 / 3.0 - t) * 6.0
                    } else {
                        p
                    }
                };
                Some([hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0), alpha])
            }
            _ => None,
        };
    }
    let k = NAMED.binary_search_by(|(n, _)| (*n).cmp(lower.as_str())).ok()?;
    let v = NAMED[k].1;
    Some([(v >> 16) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0, 1.0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_every_way_css_writes_them() {
        assert_eq!(parse_color("#f00"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse_color("#00FF0080").map(|c| (c[1], (c[3] * 255.0).round())), Some((1.0, 128.0)));
        assert_eq!(parse_color("rgb(255, 0, 0)"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse_color("rgba(0,0,255,0.5)"), Some([0.0, 0.0, 1.0, 0.5]));
        assert_eq!(parse_color("rgb(100% 0% 0% / 25%)"), Some([1.0, 0.0, 0.0, 0.25]));
        let g = parse_color("hsl(120, 100%, 50%)").unwrap();
        assert!((g[0]).abs() < 1e-9 && (g[1] - 1.0).abs() < 1e-9 && g[2].abs() < 1e-9);
        assert_eq!(parse_color("RebeccaPurple"), Some([0x66 as f64 / 255.0, 0x33 as f64 / 255.0, 0x99 as f64 / 255.0, 1.0]));
        assert_eq!(parse_color("yellowgreen").map(|c| c[2]), Some(0x32 as f64 / 255.0));
        assert_eq!(parse_color("transparent"), Some([0.0; 4]));
        assert_eq!(parse_color("notacolour"), None);
    }
}
