// The smooth noise of noise.rs on the GPU (after common.metal): its places held as two floats
// that add up to the processor's double (common.metal's df), so the noise lands where the
// processor's does however fine its layers get; the rest worked in floats.

// splitmix64's mix (noise::hash), and several seeds as one (noise::mix). Worked on the number's
// two 32-bit halves (x: low, y: high), as the GPU multiplies 64-bit numbers slowly.
inline uint2 mul64(uint2 a, uint2 b) {
    return uint2(a.x * b.x, mulhi(a.x, b.x) + a.x * b.y + a.y * b.x);
}

inline uint2 xor_shift(uint2 a, uint k) {
    return a ^ uint2((a.x >> k) | (a.y << (32 - k)), a.y >> k);
}

inline uint2 halves(ulong v) {
    return uint2(uint(v), uint(v >> 32));
}

inline ulong whole(uint2 h) {
    return (ulong(h.y) << 32) | ulong(h.x);
}

inline ulong hash64(ulong v) {
    uint2 x = halves(v);
    // + 0x9E3779B97F4A7C15, with the carry
    uint lo = x.x + 0x7F4A7C15u;
    x = uint2(lo, x.y + 0x9E3779B9u + (lo < x.x ? 1u : 0u));
    x = mul64(xor_shift(x, 30), uint2(0x1CE4E5B9u, 0xBF58476Du));
    x = mul64(xor_shift(x, 27), uint2(0x133111EBu, 0x94D049BBu));
    return whole(xor_shift(x, 31));
}

inline ulong mix2(ulong a, ulong b) {
    ulong h = 0x2545F4914F6CDD1Dul;
    h = hash64(h ^ hash64(a));
    return hash64(h ^ hash64(b));
}

// h % 12 from its two 32-bit halves (the GPU divides 64-bit numbers slowly): 2³² is 4 past a
// multiple of 12.
inline uint mod12(ulong h) {
    uint hi = uint(h >> 32), lo = uint(h);
    return ((hi % 12) * 4 + lo % 12) % 12;
}

inline float slope(ulong h, float x, float y, float z) {
    switch (mod12(h)) {
        case 0: return x + y;
        case 1: return -x + y;
        case 2: return x - y;
        case 3: return -x - y;
        case 4: return x + z;
        case 5: return -x + z;
        case 6: return x - z;
        case 7: return -x - z;
        case 8: return y + z;
        case 9: return -y + z;
        case 10: return y - z;
        default: return -y - z;
    }
}

inline float fade(float t) {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

// What z (the evolution) is, worked out once: its whole part, how far past it, that faded, and
// the period it repeats in (0: none).
struct Depth {
    long iz;
    long period;
    float w;
    float c;
};

inline long wrapped(long k, long period) {
    if (period > 0) {
        long r = k % period;
        return r < 0 ? r + period : r;
    }
    return k;
}

// noise::smooth from the lattice cell (ix, iy, iz) and how far into it (u, v, w; w faded: c):
// about -1 to 1. Each corner's hash is noise::mix of the seed and its whole x, y and z; the
// corners share most of that mixing (two x's, two y's, two z's), so each shared part is mixed
// once.
inline float smooth_cell(long ix, long iy, long iz, float u, float v, float w, float c, ulong seed, long period) {
    float a = fade(u), b = fade(v);
    ulong h1 = hash64(0x2545F4914F6CDD1Dul ^ hash64(seed));
    ulong hx[2] = {hash64(ulong(ix)), hash64(ulong(ix + 1))};
    ulong hy[2] = {hash64(ulong(iy)), hash64(ulong(iy + 1))};
    ulong hz[2] = {hash64(ulong(wrapped(iz, period))), hash64(ulong(wrapped(iz + 1, period)))};
    float k[2][2][2];
    for (int dx = 0; dx < 2; dx++) {
        ulong h2 = hash64(h1 ^ hx[dx]);
        for (int dy = 0; dy < 2; dy++) {
            ulong h3 = hash64(h2 ^ hy[dy]);
            for (int dz = 0; dz < 2; dz++) {
                k[dx][dy][dz] = slope(hash64(h3 ^ hz[dz]), u - float(dx), v - float(dy), w - float(dz));
            }
        }
    }
    float x00 = k[0][0][0] + (k[1][0][0] - k[0][0][0]) * a;
    float x10 = k[0][1][0] + (k[1][1][0] - k[0][1][0]) * a;
    float x01 = k[0][0][1] + (k[1][0][1] - k[0][0][1]) * a;
    float x11 = k[0][1][1] + (k[1][1][1] - k[0][1][1]) * a;
    float y0 = x00 + (x10 - x00) * b;
    float y1 = x01 + (x11 - x01) * b;
    return clamp((y0 + (y1 - y0) * c) * 0.97, -1.0, 1.0);
}

// noise::smooth at (x, y) and a depth worked out once: about -1 to 1.
inline float smooth_noise(df x, df y, Depth z, ulong seed) {
    long ix, iy;
    float u, v;
    df_floor(x, ix, u);
    df_floor(y, iy, v);
    return smooth_cell(ix, iy, z.iz, u, v, z.w, z.c, seed, z.period);
}

// noise::smooth at (x, y, z), each held exactly.
inline float smooth3(df x, df y, df z, ulong seed) {
    long ix, iy, iz;
    float u, v, w;
    df_floor(x, ix, u);
    df_floor(y, iy, v);
    df_floor(z, iz, w);
    return smooth_cell(ix, iy, iz, u, v, w, fade(w), seed, 0);
}

// The most octaves, and noise::fractal's settings worked out once: how many octaves, the last
// one's part, each octave's strength fall, what their strengths add up to, the turn between
// octaves, and each octave's shift (o × 17.3 across, o × 31.7 down: high and low parts).
constant constexpr uint MOST_OCTAVES = 24;

struct Octaves {
    uint n;
    float part;
    float falloff;
    float total;
    df turn_c;
    df turn_s;
    float4 shift[MOST_OCTAVES];
};

// noise::fractal at (x, y): octaves of smooth noise, each twice as fine, turned against the last.
inline float fractal_noise(df x, df y, Depth z, ulong seed, constant Octaves &f) {
    float sum = 0.0, strength = 1.0, fine = 1.0;
    df px = x, py = y;
    for (uint o = 0; o < f.n; o++) {
        float weight = (o + 1 == f.n && f.part > 0.0) ? f.part : 1.0;
        float4 shift = f.shift[o];
        df sx = df_add(df_scale(px, fine), df{shift.x, shift.y});
        df sy = df_add(df_scale(py, fine), df{shift.z, shift.w});
        sum += smooth_noise(sx, sy, z, mix2(seed, ulong(o))) * strength * weight;
        strength *= f.falloff;
        fine *= 2.0;
        df nx = df_sub(df_mul(px, f.turn_c), df_mul(py, f.turn_s));
        py = df_add(df_mul(px, f.turn_s), df_mul(py, f.turn_c));
        px = nx;
    }
    return sum / max(f.total, 1e-9);
}
