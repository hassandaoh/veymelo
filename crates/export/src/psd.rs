//! Photoshop files with layers, written our own way from Adobe's Photoshop file format: 8-bit
//! RGB, each layer its own pixels (straight colour and its transparency, cut to where it has
//! any) with its name, blend mode and opacity, and the whole picture as the merged image.

/// One layer: its name, where its pixels are (left, top, width, height in the picture), those
/// pixels row by row (straight colour and alpha, 0 to 1), its blend mode's Photoshop key and
/// its opacity (0 to 1), and whether it is seen.
pub struct Layer {
    pub name: String,
    pub rect: [i32; 4],
    pub pixels: Vec<[f32; 4]>,
    pub blend: [u8; 4],
    pub opacity: f64,
    pub visible: bool,
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A layer's name as Photoshop's short name has it: at most 255 bytes of plain letters (others
/// a "?"; the whole name goes in as Unicode too), padded so its length byte and it fill fours.
fn short_name(name: &str) -> Vec<u8> {
    let plain: Vec<u8> = name.chars().map(|c| if c.is_ascii() && !c.is_ascii_control() { c as u8 } else { b'?' }).take(255).collect();
    let mut out = vec![plain.len() as u8];
    out.extend(plain);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out
}

/// The name in Unicode, as the `luni` additional layer information.
fn unicode_name(name: &str) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let mut data = (units.len() as u32).to_be_bytes().to_vec();
    for u in &units {
        data.extend(u.to_be_bytes());
    }
    if !data.len().is_multiple_of(2) {
        data.push(0);
    }
    let mut out = b"8BIMluni".to_vec();
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(data);
    out
}

/// The bytes of a Photoshop file `width` × `height`: `layers` from the bottom up, and `merged`,
/// the whole picture (straight colour, row by row), as what readers without layers show.
pub fn encode(width: usize, height: usize, layers: &[Layer], merged: &[[f32; 4]]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(b"8BPS");
    out.extend(1u16.to_be_bytes());
    out.extend([0u8; 6]);
    out.extend(3u16.to_be_bytes());
    out.extend((height as u32).to_be_bytes());
    out.extend((width as u32).to_be_bytes());
    out.extend(8u16.to_be_bytes());
    out.extend(3u16.to_be_bytes());
    // no colour table, no image resources
    out.extend(0u32.to_be_bytes());
    out.extend(0u32.to_be_bytes());
    // the layers: their records, then each one's channels
    let mut records = Vec::new();
    let mut planes = Vec::new();
    records.extend((layers.len() as i16).to_be_bytes());
    for l in layers {
        let [x, y, w, h] = l.rect;
        let (w, h) = (w.max(0) as usize, h.max(0) as usize);
        records.extend(y.to_be_bytes());
        records.extend(x.to_be_bytes());
        records.extend((y + h as i32).to_be_bytes());
        records.extend((x + w as i32).to_be_bytes());
        records.extend(4u16.to_be_bytes());
        // transparency, red, green, blue: each two bytes saying it is not compressed, and its bytes
        for (id, k) in [(-1i16, 3usize), (0, 0), (1, 1), (2, 2)] {
            records.extend(id.to_be_bytes());
            records.extend((2 + (w * h) as u32).to_be_bytes());
            planes.extend(0u16.to_be_bytes());
            planes.extend(l.pixels.iter().take(w * h).map(|p| byte(p[k])));
        }
        records.extend(b"8BIM");
        records.extend(l.blend);
        records.push((l.opacity.clamp(0.0, 1.0) * 255.0).round() as u8);
        records.push(0);
        // bit 1 hides it; bit 3 says bit 4 means something
        records.push(if l.visible { 0x08 } else { 0x0a });
        records.push(0);
        let mut extra = Vec::new();
        extra.extend(0u32.to_be_bytes());
        extra.extend(0u32.to_be_bytes());
        extra.extend(short_name(&l.name));
        extra.extend(unicode_name(&l.name));
        records.extend((extra.len() as u32).to_be_bytes());
        records.extend(extra);
    }
    let mut info = records;
    info.extend(planes);
    if !info.len().is_multiple_of(2) {
        info.push(0);
    }
    let mut section = (info.len() as u32).to_be_bytes().to_vec();
    section.extend(info);
    // no global layer mask
    section.extend(0u32.to_be_bytes());
    out.extend((section.len() as u32).to_be_bytes());
    out.extend(section);
    // the merged picture: not compressed, red, green, blue
    out.extend(0u16.to_be_bytes());
    for k in 0..3 {
        out.extend(merged.iter().map(|p| byte(p[k])));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_fill_fours() {
        assert_eq!(short_name("ab"), vec![2, b'a', b'b', 0]);
        assert_eq!(short_name("abc").len(), 4);
        assert_eq!(short_name("abcd").len(), 8);
        assert_eq!(short_name("ชื่อ")[1], b'?');
    }
}
