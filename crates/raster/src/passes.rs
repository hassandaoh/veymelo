//! A layer's render passes: channels beyond its colour that a 3D render writes beside it —
//! depth, object and material numbers, surface normals, texture coordinates, coverage, and
//! Cryptomatte's names-by-pixel — kept by name (`Z`, `ObjectID`, `diffuse.R`, …).
//!
//! They come with footage that carries them (multi-part OpenEXR) and from our own 3D layers;
//! the 3D Channel effects read them.
//!
//! - Each channel is `width` × `height` numbers, row by row, over the layer from `origin` (layer
//!   pixels), `scale` of its pixels to a layer pixel.
//! - Depth is the distance from the camera in layer pixels; [`FAR`] where nothing is.
//! - Object and material numbers are whole numbers, 0 where nothing is.
//! - Cryptomatte follows its published specification (Psyop, 2015): a kind of name (say
//!   `CryptoObject`) is kept in `CryptoObject00.R/G/B/A`, `CryptoObject01.R/…` as pairs of an ID
//!   and how much of the pixel it covers, the most covering first; an ID is the 32-bit
//!   MurmurHash3 of the name read as a float (its exponent kept off all zeros and all ones), and
//!   the manifest gives the names for the IDs.

/// The depth where nothing was rendered.
pub const FAR: f32 = 1.0e10;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Passes {
    pub width: usize,
    pub height: usize,
    /// The layer point its first pixel's corner is at.
    pub origin: [f64; 2],
    /// Its pixels to a layer pixel.
    pub scale: f64,
    /// Its channels by name.
    pub channels: Vec<(String, Vec<f32>)>,
    /// Cryptomatte manifests: a kind of name, and its names with their IDs.
    pub manifests: Vec<(String, Vec<(String, u32)>)>,
}

impl Passes {
    pub fn new(width: usize, height: usize, scale: f64) -> Passes {
        Passes { width, height, origin: [0.0; 2], scale, channels: Vec::new(), manifests: Vec::new() }
    }

    /// A channel by its name (any case).
    pub fn get(&self, name: &str) -> Option<&[f32]> {
        self.channels.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_slice())
    }

    /// The first of `names` there is, by its whole name, or else as the last part of one (`Z`
    /// finds `depth.Z`).
    pub fn find(&self, names: &[&str]) -> Option<&[f32]> {
        names.iter().find_map(|n| self.get(n)).or_else(|| {
            names.iter().find_map(|n| {
                let end = format!(".{}", n.to_ascii_lowercase());
                self.channels.iter().find(|(c, _)| c.to_ascii_lowercase().ends_with(&end)).map(|(_, v)| v.as_slice())
            })
        })
    }

    pub fn names(&self) -> Vec<&str> {
        self.channels.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// The depth (Blender's `Depth.Z` before any other `Z`, a normal's say).
    pub fn depth(&self) -> Option<&[f32]> {
        self.find(&["Depth.Z", "Z", "depth", "zDepth", "Z-Depth"])
    }

    /// Object numbers (Blender's `IndexOB`, and its `Object Index` since Blender 5).
    pub fn object_id(&self) -> Option<&[f32]> {
        self.find(&["ObjectID", "object_id", "objectId", "id", "IndexOB.X", "Object Index.X"])
    }

    pub fn material_id(&self) -> Option<&[f32]> {
        self.find(&["MaterialID", "material_id", "materialId", "IndexMA.X", "Material Index.X"])
    }

    pub fn coverage(&self) -> Option<&[f32]> {
        self.find(&["Coverage"])
    }

    /// The pixel at layer point (x, y), if it is over the passes.
    pub fn index_at(&self, x: f64, y: f64) -> Option<usize> {
        let (px, py) = (((x - self.origin[0]) * self.scale).floor(), ((y - self.origin[1]) * self.scale).floor());
        (px >= 0.0 && py >= 0.0 && px < self.width as f64 && py < self.height as f64).then(|| py as usize * self.width + px as usize)
    }

    /// The kinds of Cryptomatte names there are (those with a `00.R` channel).
    pub fn crypto_kinds(&self) -> Vec<String> {
        let mut v: Vec<String> = self.channels.iter().filter_map(|(n, _)| n.strip_suffix("00.R").or_else(|| n.strip_suffix("00.r")).map(str::to_string)).collect();
        v.sort();
        v.dedup();
        v
    }

    /// A Cryptomatte kind's (ID, coverage) channel pairs, most covering first: `00.R/G`, `00.B/A`,
    /// `01.R/G`, … up to the first missing.
    pub fn crypto_ranks(&self, kind: &str) -> Vec<(&[f32], &[f32])> {
        let mut out = Vec::new();
        for k in 0.. {
            let get = |c: &str| self.get(&format!("{kind}{k:02}.{c}"));
            let (Some(r), Some(g)) = (get("R"), get("G")) else { break };
            out.push((r, g));
            if let (Some(b), Some(a)) = (get("B"), get("A")) {
                out.push((b, a));
            }
        }
        out
    }

    /// The name a kind's manifest gives an ID.
    pub fn crypto_name(&self, kind: &str, id: u32) -> Option<&str> {
        self.manifests.iter().find(|(k, _)| k == kind)?.1.iter().find(|(_, h)| *h == id).map(|(n, _)| n.as_str())
    }
}

/// MurmurHash3, 32-bit, for x86 (Austin Appleby's public-domain hash).
pub fn murmur3(data: &[u8], seed: u32) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    let mix = |k: u32| k.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
    let (blocks, tail) = data.as_chunks::<4>();
    let mut h = seed;
    for b in blocks {
        h ^= mix(u32::from_le_bytes(*b));
        h = h.rotate_left(13).wrapping_mul(5).wrapping_add(0xe654_6b64);
    }
    if !tail.is_empty() {
        h ^= mix(tail.iter().enumerate().fold(0u32, |k, (i, b)| k ^ (*b as u32) << (8 * i)));
    }
    h ^= data.len() as u32;
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

/// A name's Cryptomatte ID: its hash with the exponent kept off all zeros and all ones (so the
/// float it is read as is an ordinary number).
pub fn crypto_id(name: &str) -> u32 {
    let h = murmur3(name.as_bytes(), 0);
    let e = (h >> 23) & 0xff;
    if e == 0 || e == 255 { h ^ (1 << 23) } else { h }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn murmur3_gives_the_published_values() {
        assert_eq!(murmur3(b"", 0), 0);
        assert_eq!(murmur3(b"", 1), 0x514e_28b7);
        assert_eq!(murmur3(b"", 0xffff_ffff), 0x81f1_6f39);
        assert_eq!(murmur3(&[0, 0, 0, 0], 0), 0x2362_f9de);
        assert_eq!(murmur3(b"a", 0x9747_b28c), 0x7fa0_9ea6);
        assert_eq!(murmur3(b"aaaa", 0x9747_b28c), 0x5a97_808a);
        assert_eq!(murmur3(b"Hello, world!", 0x9747_b28c), 0x2488_4cba);
        assert_eq!(murmur3(b"The quick brown fox jumps over the lazy dog", 0x9747_b28c), 0x2fa8_26cd);
    }

    #[test]
    fn crypto_ids_are_ordinary_floats() {
        for n in ["bunny", "set", "ground", "a", "", "flowers1"] {
            let f = f32::from_bits(crypto_id(n));
            assert!(f.is_finite() && (f == 0.0 || f.is_normal()), "{n}: {f}");
        }
    }

    #[test]
    fn finding_channels() {
        let mut p = Passes::new(2, 1, 1.0);
        p.channels.push(("depth.Z".into(), vec![5.0, 6.0]));
        p.channels.push(("CryptoMaterial00.r".into(), vec![1.0, 2.0]));
        p.channels.push(("CryptoMaterial00.g".into(), vec![0.5, 1.0]));
        p.channels.push(("CryptoMaterial00.b".into(), vec![3.0, 4.0]));
        p.channels.push(("CryptoMaterial00.a".into(), vec![0.25, 0.0]));
        p.channels.push(("CryptoMaterial01.R".into(), vec![7.0, 8.0]));
        p.channels.push(("CryptoMaterial01.G".into(), vec![0.1, 0.0]));
        assert_eq!(p.depth(), Some(&[5.0, 6.0][..]));
        // a depth layer's Z before a normal's
        let mut q = Passes::new(1, 1, 1.0);
        q.channels.push(("ViewLayer.Normal.Z".into(), vec![1.0]));
        q.channels.push(("ViewLayer.Depth.Z".into(), vec![7.0]));
        q.channels.push(("ViewLayer.Object Index.X".into(), vec![2.0]));
        assert_eq!((q.depth(), q.object_id()), (Some(&[7.0][..]), Some(&[2.0][..])));
        assert_eq!(p.crypto_kinds(), vec!["CryptoMaterial".to_string()]);
        let ranks: Vec<f32> = p.crypto_ranks("CryptoMaterial").iter().map(|(id, _)| id[1]).collect();
        assert_eq!(ranks, vec![2.0, 4.0, 8.0]);
        assert_eq!(p.index_at(1.5, 0.2), Some(1));
        assert_eq!(p.index_at(2.5, 0.2), None);
        p.origin = [10.0, 0.0];
        assert_eq!(p.index_at(11.5, 0.2), Some(1));
    }
}
