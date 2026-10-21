//! Files locked by the standard security handler with no password to open them (only what may be
//! done with them limited): each string and stream unlocked with RC4 or AES (128 bits, a key for
//! each object; 256 bits, one key), its key made from the empty password as the handler's
//! revisions 2 to 6 say. MD5, RC4, AES and SHA-2 are our own, checked against their standards'
//! examples.

use crate::object::{Dict, Obj};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Method {
    None,
    Rc4,
    Aes,
}

pub struct Crypt {
    key: Vec<u8>,
    strings: Method,
    streams: Method,
    /// Revision 5 or 6: one key for every object.
    whole: bool,
}

/// The padding the handler fills a password out with.
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

impl Crypt {
    pub fn new(enc: &Dict, id: &[u8]) -> Result<Crypt, String> {
        let name = enc.get("Filter").and_then(Obj::as_name).unwrap_or("");
        if name != "Standard" {
            return Err(format!("it is locked by the {name} security handler, which is not read"));
        }
        let int = |k: &str, d: i64| enc.get(k).and_then(Obj::as_i64).unwrap_or(d);
        let bytes = |k: &str| enc.get(k).and_then(Obj::as_str).map(<[u8]>::to_vec).unwrap_or_default();
        let (v, r) = (int("V", 0), int("R", 2));
        let method_of = |cf: &str| -> Method {
            if v < 4 {
                return Method::Rc4;
            }
            if cf == "Identity" {
                return Method::None;
            }
            let filters = enc.get("CF").and_then(Obj::as_dict);
            match filters.and_then(|f| f.get(cf)).and_then(Obj::as_dict).and_then(|d| d.get("CFM")).and_then(Obj::as_name) {
                Some("V2") => Method::Rc4,
                Some("AESV2" | "AESV3") => Method::Aes,
                _ => Method::None,
            }
        };
        let strings = method_of(enc.get("StrF").and_then(Obj::as_name).unwrap_or("Identity"));
        let streams = method_of(enc.get("StmF").and_then(Obj::as_name).unwrap_or("Identity"));
        let (o, u) = (bytes("O"), bytes("U"));
        if r >= 5 {
            // the key: from the empty password and the user salt, unlocking UE
            let hash = |input: &[u8], salt: &[u8]| if r == 5 { sha256(&[input, salt].concat()).to_vec() } else { hash_r6(input, salt, &[]) };
            if u.len() < 48 {
                return Err("its encryption dictionary is cut off".into());
            }
            if hash(&[], &u[32..40])[..32] != u[..32] {
                return Err("it is locked with a password".into());
            }
            let inter = hash(&[], &u[40..48]);
            let ue = bytes("UE");
            let mut key = ue.get(..32).ok_or("its encryption dictionary has no UE")?.to_vec();
            let aes = Aes::new(&inter[..32]);
            let mut prev = [0u8; 16];
            for block in key.chunks_mut(16) {
                let c: [u8; 16] = block.try_into().unwrap_or([0; 16]);
                let mut b = c;
                aes.decrypt(&mut b);
                for k in 0..16 {
                    block[k] = b[k] ^ prev[k];
                }
                prev = c;
            }
            return Ok(Crypt { key, strings, streams, whole: true });
        }
        // revisions 2 to 4: MD5 of the padded empty password, O, P, the file's ID
        let n = if r == 2 { 5 } else { (int("Length", 40) / 8).clamp(5, 16) as usize };
        let p = int("P", 0) as i32;
        let mut input = PAD.to_vec();
        input.extend(o.get(..32).unwrap_or(&o));
        input.extend(p.to_le_bytes());
        input.extend(id);
        if r >= 4 && enc.get("EncryptMetadata").and_then(Obj::as_bool) == Some(false) {
            input.extend([0xFF; 4]);
        }
        let mut key = md5(&input).to_vec();
        if r >= 3 {
            for _ in 0..50 {
                key = md5(&key[..n]).to_vec();
            }
        }
        key.truncate(n);
        // the user value checks the key
        let check = if r == 2 {
            rc4(&key, &PAD) == u.get(..32).unwrap_or(&[])
        } else {
            let mut x = rc4(&key, &md5(&[&PAD[..], id].concat()));
            for i in 1..=19u8 {
                let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
                x = rc4(&k, &x);
            }
            u.len() >= 16 && x[..16] == u[..16]
        };
        if !check {
            return Err("it is locked with a password".into());
        }
        Ok(Crypt { key, strings, streams, whole: false })
    }

    fn key_for(&self, id: (u32, u16), aes: bool) -> Vec<u8> {
        if self.whole {
            return self.key.clone();
        }
        let mut input = self.key.clone();
        input.extend(&id.0.to_le_bytes()[..3]);
        input.extend(id.1.to_le_bytes());
        if aes {
            input.extend(b"sAlT");
        }
        md5(&input)[..(self.key.len() + 5).min(16)].to_vec()
    }

    fn unlock(&self, data: &[u8], id: (u32, u16), method: Method) -> Vec<u8> {
        match method {
            Method::None => data.to_vec(),
            Method::Rc4 => rc4(&self.key_for(id, false), data),
            Method::Aes => {
                if data.len() < 32 {
                    return Vec::new();
                }
                let aes = Aes::new(&self.key_for(id, true));
                let mut prev: [u8; 16] = data[..16].try_into().unwrap_or([0; 16]);
                let mut out = Vec::with_capacity(data.len() - 16);
                for block in data[16..].chunks(16) {
                    let c: [u8; 16] = match block.try_into() {
                        Ok(c) => c,
                        Err(_) => break,
                    };
                    let mut b = c;
                    aes.decrypt(&mut b);
                    out.extend((0..16).map(|k| b[k] ^ prev[k]));
                    prev = c;
                }
                // PKCS #7 padding
                if let Some(&pad) = out.last()
                    && (1..=16).contains(&pad)
                {
                    out.truncate(out.len() - pad as usize);
                }
                out
            }
        }
    }

    /// A stream's bytes unlocked.
    pub fn stream(&self, data: &[u8], id: (u32, u16)) -> Vec<u8> {
        self.unlock(data, id, self.streams)
    }

    /// An object's strings unlocked (all through its arrays and dictionaries).
    pub fn strings(&self, o: Obj, id: (u32, u16)) -> Obj {
        match o {
            Obj::Str(s) => Obj::Str(self.unlock(&s, id, self.strings)),
            Obj::Array(a) => Obj::Array(a.into_iter().map(|x| self.strings(x, id)).collect()),
            Obj::Dict(d) => Obj::Dict(Dict(d.0.into_iter().map(|(k, v)| (k, self.strings(v, id))).collect())),
            other => other,
        }
    }
}

/// Revision 6's hash (ISO 32000-2, 7.6.4.3.4): SHA-256, then rounds of AES-128 and SHA-2.
fn hash_r6(password: &[u8], salt: &[u8], udata: &[u8]) -> Vec<u8> {
    let mut k = sha256(&[password, salt, udata].concat()).to_vec();
    let mut i = 0usize;
    loop {
        let one = [password, &k, udata].concat();
        let k1: Vec<u8> = one.iter().copied().cycle().take(one.len() * 64).collect();
        let aes = Aes::new(&k[..16]);
        let mut prev: [u8; 16] = k[16..32].try_into().unwrap_or([0; 16]);
        let mut e = Vec::with_capacity(k1.len());
        for block in k1.chunks(16) {
            let mut b: [u8; 16] = block.try_into().unwrap_or([0; 16]);
            for j in 0..16 {
                b[j] ^= prev[j];
            }
            aes.encrypt(&mut b);
            e.extend(b);
            prev = b;
        }
        let sum: u32 = e[..16].iter().map(|&b| b as u32).sum();
        k = match sum % 3 {
            0 => sha256(&e).to_vec(),
            1 => sha512(&e, true).to_vec(),
            _ => sha512(&e, false).to_vec(),
        };
        if i >= 63 && (*e.last().unwrap_or(&0) as usize) <= i - 31 {
            break;
        }
        i += 1;
    }
    k.truncate(32);
    k
}

pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len().max(1)]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|&b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21];
    let k: [u32; 64] = std::array::from_fn(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32);
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend(bits.to_le_bytes());
    let mut h = [0x6745_2301u32, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476];
    for chunk in msg.chunks(64) {
        let m: [u32; 16] = std::array::from_fn(|i| u32::from_le_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]));
        let [mut a, mut b, mut c, mut d] = h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        h = [h[0].wrapping_add(a), h[1].wrapping_add(b), h[2].wrapping_add(c), h[3].wrapping_add(d)];
    }
    let mut out = [0u8; 16];
    for (i, v) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// The first 32 bits of the fractional parts of the cube roots of the first 64 primes, and the
/// square roots of the first 8 (SHA-256's constants).
const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend(bits.to_be_bytes());
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K256[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

const K512: [u64; 80] = [
    0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc, 0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118, 0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2, 0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694, 0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65, 0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5, 0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4, 0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70, 0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df, 0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b, 0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30, 0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8, 0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8, 0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3, 0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec, 0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b, 0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178, 0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b, 0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c, 0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
];

/// SHA-512, or (`short`) SHA-384.
pub fn sha512(data: &[u8], short: bool) -> Vec<u8> {
    let mut msg = data.to_vec();
    let bits = (data.len() as u128).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 128 != 112 {
        msg.push(0);
    }
    msg.extend(bits.to_be_bytes());
    let mut h: [u64; 8] = if short {
        [0xcbbb9d5dc1059ed8, 0x629a292a367cd507, 0x9159015a3070dd17, 0x152fecd8f70e5939, 0x67332667ffc00b31, 0x8eb44a8768581511, 0xdb0c2e0d64f98fa7, 0x47b5481dbefa4fa4]
    } else {
        [0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179]
    };
    for chunk in msg.chunks(128) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            w[i] = u64::from_be_bytes(chunk[8 * i..8 * i + 8].try_into().unwrap_or([0; 8]));
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..80 {
            let s1 = v[4].rotate_right(14) ^ v[4].rotate_right(18) ^ v[4].rotate_right(41);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K512[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(28) ^ v[0].rotate_right(34) ^ v[0].rotate_right(39);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out: Vec<u8> = h.iter().flat_map(|v| v.to_be_bytes()).collect();
    if short {
        out.truncate(48);
    }
    out
}

/// AES's substitution box, worked out from the field's inverses (FIPS-197 5.1.1).
fn sbox() -> &'static ([u8; 256], [u8; 256]) {
    static BOX: std::sync::OnceLock<([u8; 256], [u8; 256])> = std::sync::OnceLock::new();
    BOX.get_or_init(|| {
        let mut s = [0u8; 256];
        let mut inv = [0u8; 256];
        let (mut p, mut q) = (1u8, 1u8);
        loop {
            // p times 3, q divided by 3
            p ^= (p << 1) ^ if p & 0x80 != 0 { 0x1B } else { 0 };
            q ^= q << 1;
            q ^= q << 2;
            q ^= q << 4;
            if q & 0x80 != 0 {
                q ^= 0x09;
            }
            let x = q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4) ^ 0x63;
            s[p as usize] = x;
            if p == 1 {
                break;
            }
        }
        s[0] = 0x63;
        for (i, &v) in s.iter().enumerate() {
            inv[v as usize] = i as u8;
        }
        (s, inv)
    })
}

fn xtime(b: u8) -> u8 {
    (b << 1) ^ if b & 0x80 != 0 { 0x1B } else { 0 }
}

fn mul(mut a: u8, mut b: u8) -> u8 {
    let mut r = 0;
    while b != 0 {
        if b & 1 != 0 {
            r ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    r
}

/// AES with a 128- or 256-bit key: one block at a time.
pub struct Aes {
    keys: Vec<[u8; 16]>,
}

impl Aes {
    pub fn new(key: &[u8]) -> Aes {
        let (s, _) = sbox();
        let nk = key.len() / 4;
        let rounds = nk + 6;
        let mut w: Vec<[u8; 4]> = key.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        let mut rcon = 1u8;
        for i in nk..4 * (rounds + 1) {
            let mut t = w[i - 1];
            if i % nk == 0 {
                t = [s[t[1] as usize] ^ rcon, s[t[2] as usize], s[t[3] as usize], s[t[0] as usize]];
                rcon = xtime(rcon);
            } else if nk > 6 && i % nk == 4 {
                t = t.map(|b| s[b as usize]);
            }
            w.push(std::array::from_fn(|k| w[i - nk][k] ^ t[k]));
        }
        Aes { keys: w.chunks(4).map(|c| std::array::from_fn(|k| c[k / 4][k % 4])).collect() }
    }

    pub fn encrypt(&self, b: &mut [u8; 16]) {
        let (s, _) = sbox();
        let n = self.keys.len() - 1;
        b.iter_mut().zip(&self.keys[0]).for_each(|(v, k)| *v ^= k);
        for round in 1..=n {
            // substitute, shift rows (the state is column by column)
            let t = *b;
            for c in 0..4 {
                for r in 0..4 {
                    b[4 * c + r] = s[t[4 * ((c + r) % 4) + r] as usize];
                }
            }
            if round != n {
                for c in 0..4 {
                    let col = [b[4 * c], b[4 * c + 1], b[4 * c + 2], b[4 * c + 3]];
                    for r in 0..4 {
                        b[4 * c + r] = xtime(col[r]) ^ (xtime(col[(r + 1) % 4]) ^ col[(r + 1) % 4]) ^ col[(r + 2) % 4] ^ col[(r + 3) % 4];
                    }
                }
            }
            b.iter_mut().zip(&self.keys[round]).for_each(|(v, k)| *v ^= k);
        }
    }

    pub fn decrypt(&self, b: &mut [u8; 16]) {
        let (_, inv) = sbox();
        let n = self.keys.len() - 1;
        b.iter_mut().zip(&self.keys[n]).for_each(|(v, k)| *v ^= k);
        for round in (0..n).rev() {
            let t = *b;
            for c in 0..4 {
                for r in 0..4 {
                    b[4 * ((c + r) % 4) + r] = inv[t[4 * c + r] as usize];
                }
            }
            b.iter_mut().zip(&self.keys[round]).for_each(|(v, k)| *v ^= k);
            if round != 0 {
                for c in 0..4 {
                    let col = [b[4 * c], b[4 * c + 1], b[4 * c + 2], b[4 * c + 3]];
                    for r in 0..4 {
                        b[4 * c + r] = mul(col[r], 14) ^ mul(col[(r + 1) % 4], 11) ^ mul(col[(r + 2) % 4], 13) ^ mul(col[(r + 3) % 4], 9);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn the_ciphers_and_hashes_give_their_standards_answers() {
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sha512(b"abc", false)), "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f");
        assert_eq!(hex(&sha512(b"abc", true)), "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7");
        assert_eq!(hex(&rc4(b"Key", b"Plaintext")), "bbf316e8d940af0ad3");
        let mut b: [u8; 16] = std::array::from_fn(|i| (i as u8) * 0x11);
        let plain = b;
        let k128: Vec<u8> = (0..16).collect();
        let aes = Aes::new(&k128);
        aes.encrypt(&mut b);
        assert_eq!(hex(&b), "69c4e0d86a7b0430d8cdb78070b4c55a");
        aes.decrypt(&mut b);
        assert_eq!(b, plain);
        let k256: Vec<u8> = (0..32).collect();
        let aes = Aes::new(&k256);
        aes.encrypt(&mut b);
        assert_eq!(hex(&b), "8ea2b7ca516745bfeafc49904b496089");
        aes.decrypt(&mut b);
        assert_eq!(b, plain);
    }
}
