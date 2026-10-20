//! Deflate (RFC 1951), and in a zlib wrapper (RFC 1950): repeats found with hash chains,
//! each block written with Huffman codes made for it, with the fixed codes, or as it is,
//! whichever is shortest.

/// Lengths of repeats: (code, extra bits, first length) for codes 257 to 285.
pub(crate) const LENGTHS: [(u16, u8, u16); 29] = [
    (257, 0, 3),
    (258, 0, 4),
    (259, 0, 5),
    (260, 0, 6),
    (261, 0, 7),
    (262, 0, 8),
    (263, 0, 9),
    (264, 0, 10),
    (265, 1, 11),
    (266, 1, 13),
    (267, 1, 15),
    (268, 1, 17),
    (269, 2, 19),
    (270, 2, 23),
    (271, 2, 27),
    (272, 2, 31),
    (273, 3, 35),
    (274, 3, 43),
    (275, 3, 51),
    (276, 3, 59),
    (277, 4, 67),
    (278, 4, 83),
    (279, 4, 99),
    (280, 4, 115),
    (281, 5, 131),
    (282, 5, 163),
    (283, 5, 195),
    (284, 5, 227),
    (285, 0, 258),
];

/// Distances of repeats: (extra bits, first distance) for codes 0 to 29.
pub(crate) const DISTANCES: [(u8, u16); 30] = [
    (0, 1),
    (0, 2),
    (0, 3),
    (0, 4),
    (1, 5),
    (1, 7),
    (2, 9),
    (2, 13),
    (3, 17),
    (3, 25),
    (4, 33),
    (4, 49),
    (5, 65),
    (5, 97),
    (6, 129),
    (6, 193),
    (7, 257),
    (7, 385),
    (8, 513),
    (8, 769),
    (9, 1025),
    (9, 1537),
    (10, 2049),
    (10, 3073),
    (11, 4097),
    (11, 6145),
    (12, 8193),
    (12, 12289),
    (13, 16385),
    (13, 24577),
];

/// The order code lengths of the code-length code are written in.
pub(crate) const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

const WINDOW: usize = 32 * 1024;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
/// How many earlier places with the same three bytes are tried.
const CHAIN: usize = 96;
const HASH_BITS: u32 = 15;

/// A byte, or a repeat of `len` bytes from `dist` back.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Literal(u8),
    Repeat { len: u16, dist: u16 },
}

fn length_code(len: u16) -> (usize, u8, u16) {
    let i = LENGTHS.iter().rposition(|l| l.2 <= len).expect("len >= 3");
    let (code, bits, base) = LENGTHS[i];
    (code as usize, bits, len - base)
}

fn distance_code(dist: u16) -> (usize, u8, u16) {
    let i = DISTANCES.iter().rposition(|d| d.1 <= dist).expect("dist >= 1");
    let (bits, base) = DISTANCES[i];
    (i, bits, dist - base)
}

/// The bytes as literals and repeats.
fn tokens(data: &[u8]) -> Vec<Token> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let hash = |i: usize| ((u32::from(data[i]) << 16 | u32::from(data[i + 1]) << 8 | u32::from(data[i + 2])).wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize;
    let mut head = vec![usize::MAX; 1 << HASH_BITS];
    let mut prev = vec![usize::MAX; WINDOW];
    let insert = |i: usize, head: &mut Vec<usize>, prev: &mut Vec<usize>| {
        if i + MIN_MATCH <= data.len() {
            let h = hash(i);
            prev[i % WINDOW] = head[h];
            head[h] = i;
        }
    };
    // the longest repeat starting at `i`, among the places with the same first three bytes
    let longest = |i: usize, head: &Vec<usize>, prev: &Vec<usize>| -> (usize, usize) {
        if i + MIN_MATCH > data.len() {
            return (0, 0);
        }
        let most = (data.len() - i).min(MAX_MATCH);
        let (mut best, mut best_dist) = (0, 0);
        let mut at = head[hash(i)];
        for _ in 0..CHAIN {
            if at == usize::MAX || at >= i || i - at > WINDOW {
                break;
            }
            if data[at + best.min(most - 1)] == data[i + best.min(most - 1)] {
                let n = data[at..].iter().zip(&data[i..i + most]).take_while(|(a, b)| a == b).count();
                if n > best {
                    best = n;
                    best_dist = i - at;
                    if n == most {
                        break;
                    }
                }
            }
            let next = prev[at % WINDOW];
            if next == usize::MAX || next >= at {
                break;
            }
            at = next;
        }
        if best >= MIN_MATCH { (best, best_dist) } else { (0, 0) }
    };
    let mut i = 0;
    while i < data.len() {
        let (len, dist) = longest(i, &head, &prev);
        insert(i, &mut head, &mut prev);
        if len == 0 {
            out.push(Token::Literal(data[i]));
            i += 1;
            continue;
        }
        // a longer repeat one byte later is worth a literal first
        let (next_len, _) = if len < 32 { longest(i + 1, &head, &prev) } else { (0, 0) };
        if next_len > len + 1 {
            out.push(Token::Literal(data[i]));
            i += 1;
            continue;
        }
        out.push(Token::Repeat { len: len as u16, dist: dist as u16 });
        for k in i + 1..i + len {
            insert(k, &mut head, &mut prev);
        }
        i += len;
    }
    out
}

/// Code lengths of at most `limit` bits for symbols seen `freq` times (0 for unseen ones).
fn code_lengths(freq: &[u32], limit: u8) -> Vec<u8> {
    let mut freq: Vec<u32> = freq.to_vec();
    // a code needs two symbols to be whole: unseen ones get a count until two are seen
    for i in 0..freq.len() {
        if freq.iter().filter(|f| **f > 0).count() >= 2 {
            break;
        }
        if freq[i] == 0 {
            freq[i] = 1;
        }
    }
    loop {
        let lengths = huffman(&freq);
        if lengths.iter().all(|l| *l <= limit) {
            return lengths;
        }
        // too deep: even the counts out and try again
        for f in freq.iter_mut().filter(|f| **f > 0) {
            *f = (*f >> 1).max(1);
        }
    }
}

/// Huffman code lengths for the counts (0 for symbols never seen).
fn huffman(freq: &[u32]) -> Vec<u8> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    // nodes: leaves first, then joins; parent of each node
    let mut parent: Vec<usize> = Vec::new();
    let mut heap = BinaryHeap::new();
    let mut leaf_of = vec![usize::MAX; freq.len()];
    for (s, &f) in freq.iter().enumerate() {
        if f > 0 {
            leaf_of[s] = parent.len();
            heap.push(Reverse((u64::from(f), parent.len())));
            parent.push(usize::MAX);
        }
    }
    while heap.len() > 1 {
        let Reverse((fa, a)) = heap.pop().expect("two left");
        let Reverse((fb, b)) = heap.pop().expect("two left");
        let n = parent.len();
        parent.push(usize::MAX);
        parent[a] = n;
        parent[b] = n;
        heap.push(Reverse((fa + fb, n)));
    }
    let depth = |mut n: usize| {
        let mut d = 0u8;
        while parent[n] != usize::MAX {
            n = parent[n];
            d = d.saturating_add(1);
        }
        d
    };
    leaf_of.iter().map(|&n| if n == usize::MAX { 0 } else { depth(n).max(1) }).collect()
}

/// Canonical codes for code lengths (RFC 1951 3.2.2).
fn codes(lengths: &[u8]) -> Vec<u16> {
    let mut count = [0u16; 16];
    for &l in lengths {
        count[l as usize] += 1;
    }
    count[0] = 0;
    let mut next = [0u16; 16];
    let mut code = 0u16;
    for bits in 1..16 {
        code = (code + count[bits - 1]) << 1;
        next[bits] = code;
    }
    lengths
        .iter()
        .map(|&l| {
            if l == 0 {
                0
            } else {
                let c = next[l as usize];
                next[l as usize] += 1;
                c
            }
        })
        .collect()
}

/// Bits, first bit lowest.
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn new() -> Bits {
        Bits { out: Vec::new(), acc: 0, n: 0 }
    }

    fn put(&mut self, value: u32, bits: u8) {
        self.acc |= u64::from(value) << self.n;
        self.n += u32::from(bits);
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// A Huffman code, its first bit the highest.
    fn code(&mut self, code: u16, len: u8) {
        let reversed = (code.reverse_bits() >> (16 - len as u32)) as u32;
        self.put(reversed, len);
    }

    fn align(&mut self) {
        if self.n > 0 {
            self.put(0, (8 - self.n) as u8);
        }
    }

    /// How many bits have been written.
    fn len(&self) -> usize {
        self.out.len() * 8 + self.n as usize
    }
}

/// The code lengths of both codes, run-length coded (RFC 1951 3.2.7): (symbol, extra value).
fn run_lengths(lengths: &[u8]) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lengths.len() {
        let l = lengths[i];
        let run = lengths[i..].iter().take_while(|&&x| x == l).count();
        if l == 0 && run >= 3 {
            let n = run.min(138);
            out.push(if n >= 11 { (18, (n - 11) as u8) } else { (17, (n - 3) as u8) });
            i += n;
        } else if l != 0 && run >= 4 {
            out.push((l, 0));
            let n = (run - 1).min(6);
            out.push((16, (n - 3) as u8));
            i += 1 + n;
        } else {
            out.push((l, 0));
            i += 1;
        }
    }
    out
}

/// The Huffman codes a block is written with.
struct Codes {
    lit_len: Vec<u8>,
    lit_code: Vec<u16>,
    dist_len: Vec<u8>,
    dist_code: Vec<u16>,
}

impl Codes {
    fn fixed() -> Codes {
        let lit_len: Vec<u8> = (0..288).map(|s| match s {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        }).collect();
        let dist_len = vec![5u8; 30];
        Codes { lit_code: codes(&lit_len), dist_code: codes(&dist_len), lit_len, dist_len }
    }

    fn made_for(tokens: &[Token]) -> Codes {
        let (mut lit, mut dist) = (vec![0u32; 286], vec![0u32; 30]);
        for t in tokens {
            match *t {
                Token::Literal(b) => lit[b as usize] += 1,
                Token::Repeat { len, dist: d } => {
                    lit[length_code(len).0] += 1;
                    dist[distance_code(d).0] += 1;
                }
            }
        }
        lit[256] = 1;
        let (lit_len, dist_len) = (code_lengths(&lit, 15), code_lengths(&dist, 15));
        Codes { lit_code: codes(&lit_len), dist_code: codes(&dist_len), lit_len, dist_len }
    }

    /// Write the tokens and the end of the block.
    fn write(&self, tokens: &[Token], bits: &mut Bits) {
        for t in tokens {
            match *t {
                Token::Literal(b) => bits.code(self.lit_code[b as usize], self.lit_len[b as usize]),
                Token::Repeat { len, dist } => {
                    let (c, extra, v) = length_code(len);
                    bits.code(self.lit_code[c], self.lit_len[c]);
                    bits.put(u32::from(v), extra);
                    let (c, extra, v) = distance_code(dist);
                    bits.code(self.dist_code[c], self.dist_len[c]);
                    bits.put(u32::from(v), extra);
                }
            }
        }
        bits.code(self.lit_code[256], self.lit_len[256]);
    }

    /// Write how the codes are made (for a block with its own codes).
    fn write_header(&self, bits: &mut Bits) {
        let hlit = 257 + self.lit_len[257..].iter().rposition(|&l| l > 0).map_or(0, |i| i + 1);
        let hdist = 1 + self.dist_len.iter().rposition(|&l| l > 0).unwrap_or(0);
        let mut all = self.lit_len[..hlit].to_vec();
        all.extend_from_slice(&self.dist_len[..hdist]);
        let runs = run_lengths(&all);
        let mut freq = [0u32; 19];
        for (s, _) in &runs {
            freq[*s as usize] += 1;
        }
        let cl_len = code_lengths(&freq, 7);
        let cl_code = codes(&cl_len);
        let hclen = (4..=19).rev().find(|&n| cl_len[ORDER[n - 1]] > 0).unwrap_or(4);
        bits.put((hlit - 257) as u32, 5);
        bits.put((hdist - 1) as u32, 5);
        bits.put((hclen - 4) as u32, 4);
        for &s in &ORDER[..hclen] {
            bits.put(u32::from(cl_len[s]), 3);
        }
        for (s, extra) in runs {
            bits.code(cl_code[s as usize], cl_len[s as usize]);
            match s {
                16 => bits.put(u32::from(extra), 2),
                17 => bits.put(u32::from(extra), 3),
                18 => bits.put(u32::from(extra), 7),
                _ => {}
            }
        }
    }
}

/// `data`, deflated.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let all = tokens(data);
    let mut bits = Bits::new();
    if all.is_empty() {
        // one empty block with the fixed codes
        bits.put(1, 1);
        bits.put(1, 2);
        Codes::fixed().write(&[], &mut bits);
        bits.align();
        return bits.out;
    }
    const BLOCK: usize = 32 * 1024;
    let fixed = Codes::fixed();
    let mut start_byte = 0;
    let blocks: Vec<&[Token]> = all.chunks(BLOCK).collect();
    for (i, block) in blocks.iter().enumerate() {
        let last = i + 1 == blocks.len();
        let bytes: usize = block.iter().map(|t| if let Token::Repeat { len, .. } = t { *len as usize } else { 1 }).sum();
        let raw = &data[start_byte..start_byte + bytes];
        start_byte += bytes;
        // try both codes and keep the shorter
        let own = Codes::made_for(block);
        let mut with_own = Bits::new();
        own.write_header(&mut with_own);
        own.write(block, &mut with_own);
        let mut with_fixed = Bits::new();
        fixed.write(block, &mut with_fixed);
        let stored_bits = raw.len().div_ceil(65_535) * 40 + raw.len() * 8 + 8;
        let best = with_own.len().min(with_fixed.len());
        if stored_bits < best {
            let pieces: Vec<&[u8]> = raw.chunks(65_535).collect();
            for (k, piece) in pieces.iter().enumerate() {
                bits.put(u32::from(last && k + 1 == pieces.len()), 1);
                bits.put(0, 2);
                bits.align();
                let n = piece.len() as u16;
                bits.put(u32::from(n), 16);
                bits.put(u32::from(!n), 16);
                bits.out.extend_from_slice(piece);
            }
            continue;
        }
        bits.put(u32::from(last), 1);
        let (kind, body) = if with_own.len() <= with_fixed.len() { (2, &own) } else { (1, &fixed) };
        bits.put(kind, 2);
        if kind == 2 {
            body.write_header(&mut bits);
        }
        body.write(block, &mut bits);
    }
    bits.align();
    bits.out
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65_521;
        b %= 65_521;
    }
    b << 16 | a
}

/// `data` in a zlib stream.
pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0xDA];
    out.extend(deflate(data));
    out.extend(adler32(data).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inflate;

    fn noise(n: usize, seed: u32) -> Vec<u8> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    #[test]
    fn everything_comes_back() {
        let mut samples: Vec<Vec<u8>> = vec![vec![], vec![7], b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_vec(), b"the cat sat on the mat; the cat sat on the hat".to_vec()];
        samples.push(noise(100_000, 1));
        samples.push((0..200_000u32).map(|i| (i / 300 % 7) as u8 * 30).collect());
        let mut mixed = noise(5000, 9);
        mixed.extend(std::iter::repeat_n(0u8, 70_000));
        mixed.extend(noise(70_000, 3));
        samples.push(mixed);
        for s in &samples {
            let d = deflate(s);
            assert_eq!(inflate(&d).as_deref(), Ok(&s[..]), "{} bytes", s.len());
        }
    }

    #[test]
    fn repeats_get_small_and_noise_stays_its_size() {
        let flat = vec![200u8; 1_000_000];
        assert!(deflate(&flat).len() < 3000, "{}", deflate(&flat).len());
        let n = noise(50_000, 5);
        assert!(deflate(&n).len() < 50_000 + 100, "{}", deflate(&n).len());
        let text = "Veymelo makes videos. ".repeat(2000);
        assert!(deflate(text.as_bytes()).len() < 500);
    }

    #[test]
    fn adler_and_zlib() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        let z = zlib(b"hello");
        assert_eq!(&z[..2], &[0x78, 0xDA]);
        assert_eq!((u16::from(z[0]) << 8 | u16::from(z[1])) % 31, 0, "the header checks out");
        assert_eq!(inflate(&z[2..z.len() - 4]).unwrap(), b"hello");
    }

    #[test]
    fn code_lengths_stay_in_their_limit() {
        // counts like Fibonacci make the deepest trees
        let mut f = vec![1u32, 1];
        while f.len() < 40 {
            let n = f[f.len() - 1] + f[f.len() - 2];
            f.push(n);
        }
        let l = code_lengths(&f, 15);
        assert!(l.iter().all(|&x| (1..=15).contains(&x)));
        let kraft: f64 = l.iter().map(|&x| 0.5f64.powi(x as i32)).sum();
        assert!(kraft <= 1.0 + 1e-12);
        assert_eq!(code_lengths(&[0, 5, 0], 15).iter().filter(|&&x| x > 0).count(), 2, "a lone symbol gets a partner");
    }
}
