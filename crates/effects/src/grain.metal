// Fractal and Turbulent Noise on the GPU (after common.metal and noise.metal), as grain.rs makes
// them: layers of value noise, each layer's place, turn, size, seed and strength worked out on
// the processor.

// The grid's number at a whole point, from 0 to 1, from its x, y and z each times its own
// constant (worked out once for the points of a patch that share them) and the seed.
inline float lattice(ulong x, ulong y, ulong zs) {
    return float(uint(hash64(x ^ y ^ zs) >> 40)) / 16777216.0;
}

inline float catmull(float p0, float p1, float p2, float p3, float t) {
    return 0.5 * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t);
}

constant constexpr float SOFT_GAIN = (181.0 / 231.0) / 0.55;

inline long wrap_z(long k, long period) {
    if (period > 0) {
        long r = k % period;
        return r < 0 ? r + period : r;
    }
    return k;
}

// One slice of value noise at whole z `kz`: the patch of grid points round (ix, iy) it needs.
inline float slice(long ix, long iy, long kz, float tx, float ty, ulong seed, uint kind) {
    ulong zs = (ulong(kz) * 0x165667B19E3779F9ul) ^ seed;
    ulong xs[4], ys[4];
    for (int k = 0; k < 4; k++) {
        xs[k] = ulong(ix + k - 1) * 0x9E3779B97F4A7C15ul;
        ys[k] = ulong(iy + k - 1) * 0xC2B2AE3D27D4EB4Ful;
    }
    #define C(dx, dy) lattice(xs[(dx) + 1], ys[(dy) + 1], zs)
    switch (kind) {
        case 0: return C(0, 0);
        case 1: {
            float a = C(0, 0) + (C(1, 0) - C(0, 0)) * tx;
            float b = C(0, 1) + (C(1, 1) - C(0, 1)) * tx;
            return a + (b - a) * ty;
        }
        case 2: {
            float sx = 1.0 - tx, sy = 1.0 - ty;
            float wx[3] = {0.5 * sx * sx, 0.5 + tx * sx, 0.5 * tx * tx};
            float wy[3] = {0.5 * sy * sy, 0.5 + ty * sy, 0.5 * ty * ty};
            float out = 0.0;
            for (int dy = -1; dy <= 1; dy++) {
                float row = wx[0] * C(-1, dy) + wx[1] * C(0, dy) + wx[2] * C(1, dy);
                out = dy == -1 ? wy[0] * row : out + wy[dy + 1] * row;
            }
            return out;
        }
        default: {
            float rows[4];
            for (int dy = -1; dy <= 2; dy++) {
                rows[dy + 1] = catmull(C(-1, dy), C(0, dy), C(1, dy), C(2, dy), tx);
            }
            return catmull(rows[0], rows[1], rows[2], rows[3], ty);
        }
    }
    #undef C
}

// Value noise at (x, y, z), about 0 to 1 (grain::value).
inline float value_noise(float x, float y, float z, ulong seed, uint kind, long period) {
    float h = kind == 2 ? 0.5 : 0.0;
    float x0 = floor(x + h), y0 = floor(y + h), z0 = floor(z);
    float tx = x + h - x0, ty = y + h - y0, tz = z - z0;
    float fz = tz * tz * tz * (tz * (tz * 6.0 - 15.0) + 10.0);
    long ix = long(x0), iy = long(y0), iz = long(z0);
    float a = slice(ix, iy, wrap_z(iz, period), tx, ty, seed, kind);
    float b = slice(ix, iy, wrap_z(iz + 1, period), tx, ty, seed, kind);
    float n = a + (b - a) * fz;
    return kind == 2 ? 0.5 + (n - 0.5) * SOFT_GAIN : n;
}

inline float shape_noise(uint kind, float n) {
    float s = n * 2.0 - 1.0;
    switch (kind) {
        case 1: return 1.0 - s * s;
        case 2: return 1.0 - abs(s);
        case 3: { float t = 1.0 - abs(s); return t * t; }
        case 8: { float t = clamp(abs(s) / 0.12, 0.0, 1.0); return 1.0 - t * t; }
        default: return n;
    }
}

inline float overflow_noise(float v, uint how) {
    switch (how) {
        case 1:
            if (v > 0.8) {
                return 0.8 + 0.2 * precise::tanh((v - 0.8) / 0.2);
            }
            if (v < 0.2) {
                return 0.2 - 0.2 * precise::tanh((0.2 - v) / 0.2);
            }
            return v;
        case 2: {
            float t = precise::fmod(v, 2.0);
            t = t < 0.0 ? t + 2.0 : t;
            return t > 1.0 ? 2.0 - t : t;
        }
        case 3: return v;
        default: return clamp(v, 0.0, 1.0);
    }
}

// A layer of the noise as the processor works it out once (grain.rs Layer).
struct Layer {
    ulong seed;
    float org_x;
    float org_y;
    float sub_s;
    float sub_c;
    float fine;
    float ez;
    float ez_a;
    float ez_c;
    float strength;
    float w;
    float push;
    float pad;
};

struct Noise {
    uint w;
    uint h;
    uint kind;
    uint noise;
    uint overflow;
    uint invert;
    uint mode;
    uint layers;
    long period;
    uint sub_turn;
    float contrast;
    float brightness;
    float opacity;
    float total;
    float turn_s;
    float turn_c;
    float size_x;
    float size_y;
    float pad;
};

kernel void fractal_noise(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], const device Layer *layers [[buffer(2)]], constant Noise &p [[buffer(3)]], uint2 g [[thread_position_in_grid]]) {
    float x = float(g.x) + 0.5, y = float(g.y) + 0.5;
    float sum = 0.0, best = 0.0;
    for (uint o = 0; o < p.layers; o++) {
        Layer l = layers[o];
        float dx = x - l.org_x, dy = y - l.org_y;
        float u = (dx * p.turn_c + dy * p.turn_s) / p.size_x;
        float v = (-dx * p.turn_s + dy * p.turn_c) / p.size_y;
        if (p.sub_turn) {
            float uu = u * l.sub_c + v * l.sub_s;
            v = -u * l.sub_s + v * l.sub_c;
            u = uu;
        }
        u = u * l.fine;
        v = v * l.fine;
        if (p.kind >= 4 && p.kind <= 6) {
            // dynamic: each layer pushed about by a coarser field of its own
            float a = value_noise(u * 0.5, v * 0.5, l.ez_a, l.seed ^ 0x55ul, p.noise, p.period) - 0.5;
            float c = value_noise(u * 0.5, v * 0.5, l.ez_c, l.seed ^ 0xaaul, p.noise, p.period) - 0.5;
            if (p.kind == 5) {
                float s = precise::sin(a * 3.0), co = precise::cos(a * 3.0);
                float uu = u * co - v * s;
                v = u * s + v * co;
                u = uu;
            } else {
                u += a * 1.5 * l.push;
                v += c * 1.5 * l.push;
            }
        }
        float s = shape_noise(p.kind, value_noise(u, v, l.ez, l.seed, p.noise, p.period));
        if (p.kind == 7) {
            best = max(best, s * l.strength);
        }
        sum += s * l.strength * l.w;
    }
    float raw = p.kind == 7 ? best : sum / max(p.total, 1e-6);
    float grey = overflow_noise((raw - 0.5) * p.contrast + 0.5 + p.brightness, p.overflow);
    if (p.invert) {
        grey = 1.0 - grey;
    }
    // put on the layer's pixel by the mode (none: the noise alone), kept inside its alpha
    float4 px = src[g.y * p.w + g.x];
    float4 c = unpremul(px);
    float4 o;
    if (c.w <= 0.0) {
        o = px;
    } else {
        float4 n = float4(grey * p.opacity, grey * p.opacity, grey * p.opacity, p.opacity);
        o = (p.mode == 0 ? n : blend(p.mode - 1, float4(c.xyz, 1.0), n, 0.5)) * c.w;
    }
    out[g.y * p.w + g.x] = o;
}

// Curl Noise (after noise.metal): the noise field over the pixels, the flow round its contours,
// and the picture carried along the flow.

struct Curl {
    uint w;
    uint h;
    uint steps;
    uint wrap;
    df off_x;
    df off_y;
    df turn_c;
    df turn_s;
    df inv_size;
    float size;
    float dt;
    ulong seed;
    Depth z;
};

// The noise at each pixel's middle (in the first channel).
kernel void curl_field(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant Octaves &oct [[buffer(2)]], constant Curl &p [[buffer(3)]], uint2 g [[thread_position_in_grid]]) {
    df dx = df_sub(df_of(float(g.x) + 0.5), p.off_x);
    df dy = df_sub(df_of(float(g.y) + 0.5), p.off_y);
    df u = df_mul(df_add(df_mul(dx, p.turn_c), df_mul(dy, p.turn_s)), p.inv_size);
    df v = df_mul(df_sub(df_mul(dy, p.turn_c), df_mul(dx, p.turn_s)), p.inv_size);
    out[g.y * p.w + g.x] = float4(fractal_noise(u, v, p.z, p.seed, oct), 0.0, 0.0, 0.0);
}

// The flow round the field's contours (its curl): about a pixel a unit of the amount.
kernel void curl_flow(device float4 *out [[buffer(0)]], const device float4 *field [[buffer(1)]], constant Curl &p [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    int x = int(g.x), y = int(g.y);
    #define AT(i, j) field[uint(clamp(j, 0, int(p.h) - 1)) * p.w + uint(clamp(i, 0, int(p.w) - 1))].x
    float gx = (AT(x + 1, y) - AT(x - 1, y)) * 0.5;
    float gy = (AT(x, y + 1) - AT(x, y - 1)) * 0.5;
    #undef AT
    out[g.y * p.w + g.x] = float4(gy * p.size * 0.5, -gx * p.size * 0.5, 0.0, 0.0);
}

// The flow seen: across in red, down in green.
kernel void curl_view(device float4 *out [[buffer(0)]], const device float4 *flow [[buffer(1)]], constant Curl &p [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    float4 v = flow[g.y * p.w + g.x];
    out[g.y * p.w + g.x] = float4(clamp(0.5 + v.x * 0.5, 0.0, 1.0), clamp(0.5 + v.y * 0.5, 0.0, 1.0), 0.5, 1.0);
}

// Each pixel followed back along the flow, and the picture taken from where it ends.
kernel void curl_carry(device float4 *out [[buffer(0)]], const device float4 *flow [[buffer(1)]], const device float4 *src [[buffer(2)]], constant Curl &p [[buffer(3)]], uint2 g [[thread_position_in_grid]]) {
    float wf = float(p.w), hf = float(p.h);
    float sx = float(g.x) + 0.5, sy = float(g.y) + 0.5;
    for (uint k = 0; k < p.steps; k++) {
        float4 v = bilinear(flow, p.w, p.h, clamp(sx, 0.5, wf - 0.5), clamp(sy, 0.5, hf - 0.5));
        sx -= v.x * p.dt;
        sy -= v.y * p.dt;
    }
    if (p.wrap) {
        sx = precise::fmod(sx, wf);
        sx = sx < 0.0 ? sx + wf : sx;
        sy = precise::fmod(sy, hf);
        sy = sy < 0.0 ? sy + hf : sy;
    }
    out[g.y * p.w + g.x] = bilinear(src, p.w, p.h, clamp(sx, 0.5, wf - 0.5), clamp(sy, 0.5, hf - 0.5));
}
