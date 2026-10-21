// Generated light and patterns on the GPU (after common.metal and noise.metal), as generate.rs
// makes them.

constant constexpr uint RAY_STEPS = 48;

inline float smooth_step(float e0, float e1, float x) {
    float t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

inline float luma_px(float4 p) {
    return 0.2126 * p.x + 0.7152 * p.y + 0.0722 * p.z;
}

// The picture at `t` of the way from the pixel toward the centre (`v`: the pixel less the
// centre).
inline float4 toward(const device float4 *src, uint w, uint h, uint2 g, float vx, float vy, float t) {
    float ox = -vx * t, oy = -vy * t;
    float flx = floor(ox), fly = floor(oy);
    return between(src, w, h, int(g.x) + int(flx), int(g.y) + int(fly), float2(ox - flx, oy - fly));
}

struct Burst {
    uint w;
    uint h;
    uint mode;
    uint set;
    df cx;
    df cy;
    float intensity;
    float r;
    float g;
    float b;
    float steps[RAY_STEPS];
};

// Light Burst: along the line toward the centre, the brightest (or a fading or even mean) of the
// picture.
kernel void light_burst(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant Burst &p [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    float vx = df_float(df_sub(df_of(float(g.x) + 0.5), p.cx)), vy = df_float(df_sub(df_of(float(g.y) + 0.5), p.cy));
    float4 acc = float4(0.0), best = float4(0.0);
    float total = 0.0, best_l = -1.0;
    for (uint i = 0; i < RAY_STEPS; i++) {
        float4 v = toward(src, p.w, p.h, g, vx, vy, p.steps[i]);
        if (p.mode == 0) {
            float l = luma_px(v) + v.w * 1e-3;
            if (l > best_l) {
                best_l = l;
                best = v;
            }
        } else if (p.mode == 1) {
            float wt = 1.0 - float(i) / float(RAY_STEPS);
            acc += v * wt;
            total += wt;
        } else {
            acc += v;
            total += 1.0;
        }
    }
    float4 o = p.mode == 0 ? best : acc / max(total, 1e-6);
    if (p.set) {
        float4 u = unpremul(o);
        float l = luminance(u.xyz) * u.w;
        o = float4(p.r * l, p.g * l, p.b * l, u.w);
    }
    out[g.y * p.w + g.x] = float4(o.x * p.intensity, o.y * p.intensity, o.z * p.intensity, clamp(o.w * p.intensity, 0.0, 1.0));
}

struct Rays {
    uint w;
    uint h;
    uint square;
    uint from_source;
    uint mode;
    uint pad;
    df cx;
    df cy;
    float rad;
    float rad2;
    float gain;
    float r;
    float g;
    float b;
    float steps[RAY_STEPS];
};

// Light Rays: the light along the line toward the centre, from the part of the picture near the
// centre, put on the picture by the mode.
kernel void light_rays(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant Rays &p [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    float vx = df_float(df_sub(df_of(float(g.x) + 0.5), p.cx)), vy = df_float(df_sub(df_of(float(g.y) + 0.5), p.cy));
    float4 acc = float4(0.0);
    for (uint i = 0; i < RAY_STEPS; i++) {
        float s = 1.0 - p.steps[i];
        float dx = abs(vx * s), dy = abs(vy * s);
        float d = p.square ? max(dx, dy) : sqrt(dx * dx + dy * dy);
        float m = 1.0 - smooth_step(p.rad, p.rad2, d);
        if (m > 0.0) {
            acc += toward(src, p.w, p.h, g, vx, vy, p.steps[i]) * m;
        }
    }
    float4 ray = acc / float(RAY_STEPS) * p.gain;
    if (!p.from_source) {
        float4 u = unpremul(ray);
        float l = luminance(u.xyz) * ray.w;
        ray = float4(p.r * l, p.g * l, p.b * l, ray.w);
    }
    float la = min(ray.w, 1.0);
    float4 q = src[g.y * p.w + g.x];
    float4 o;
    switch (p.mode) {
        case 1: o = float4(max(q.xyz, ray.xyz), max(q.w, la)); break;
        case 2: o = float4(q.xyz + ray.xyz - q.xyz * ray.xyz, q.w + la - q.w * la); break;
        case 3: o = float4(ray.xyz, la); break;
        default: {
            float cover = clamp(la, 0.0, 1.0);
            o = float4(q.xyz + ray.xyz, clamp(q.w + cover * (1.0 - q.w), 0.0, 1.0));
        }
    }
    out[g.y * p.w + g.x] = o;
}

struct Cells {
    uint w;
    uint h;
    uint shape;
    uint subs;
    uint reach;
    uint overflow;
    uint invert;
    uint tiled;
    long nx;
    long ny;
    df ax;
    df ay;
    df step;
    float contrast;
    float disperse;
    float evo;
    float pad;
    ulong seed;
};

// noise::unit (itself a hash) of noise::mix(tx, ty, seed, k), the first three mixed once (h3).
inline float unit4(ulong h3, ulong k) {
    return float(hash64(hash64(h3 ^ hash64(k))) >> 11) * 1.1102230246251565e-16;
}

// Cell Pattern: at each sample, the nearest and second nearest cell points (each wandering from
// its cell's middle) and the nearest one's tone, shaped by the pattern.
kernel void cell_pattern(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant Cells &p [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    float total = 0.0;
    for (uint sy = 0; sy < p.subs; sy++) {
        for (uint sx = 0; sx < p.subs; sx++) {
            float ox = (float(sx) + 0.5) / float(p.subs), oy = (float(sy) + 0.5) / float(p.subs);
            df u = df_add(p.ax, df_mul(df_add(df_of(float(g.x)), df_of(ox)), p.step));
            df v = df_add(p.ay, df_mul(df_add(df_of(float(g.y)), df_of(oy)), p.step));
            long cx, cy;
            float fu, fv;
            df_floor(u, cx, fu);
            df_floor(v, cy, fv);
            float f1 = INFINITY, f2 = INFINITY, tone = 0.0;
            int reach = int(p.reach);
            for (int j = -reach; j <= reach; j++) {
                for (int i = -reach; i <= reach; i++) {
                    long gx = cx + i, gy = cy + j;
                    long tx = gx, ty = gy;
                    if (p.tiled) {
                        tx = gx % p.nx;
                        tx = tx < 0 ? tx + p.nx : tx;
                        ty = gy % p.ny;
                        ty = ty < 0 ? ty + p.ny : ty;
                    }
                    ulong h3 = hash64(0x2545F4914F6CDD1Dul ^ hash64(ulong(tx)));
                    h3 = hash64(h3 ^ hash64(ulong(ty)));
                    h3 = hash64(h3 ^ hash64(p.seed));
                    float ang = 2.0 * M_PI_F * (unit4(h3, 2) + p.evo);
                    // the point less the sample, from the sample's cell
                    float px = float(i) + 0.5 + (unit4(h3, 0) - 0.5) * p.disperse + 0.15 * p.disperse * precise::cos(ang) - fu;
                    float py = float(j) + 0.5 + (unit4(h3, 1) - 0.5) * p.disperse + 0.15 * p.disperse * precise::sin(ang) - fv;
                    float d = sqrt(px * px + py * py);
                    if (d < f1) {
                        f2 = f1;
                        f1 = d;
                        tone = unit4(h3, 3);
                    } else if (d < f2) {
                        f2 = d;
                    }
                }
            }
            float e = f2 - f1;
            switch (p.shape) {
                case 0: total += clamp(1.0 - f1 * 1.3, 0.0, 1.0); break;
                case 1: total += tone * 0.8 + 0.2 * clamp(1.0 - f1, 0.0, 1.0); break;
                case 2: total += clamp(e * 2.0, 0.0, 1.0); break;
                case 3: total += tone * smooth_step(0.0, 0.06, e); break;
                case 4: total += tone; break;
                case 5: total += clamp(1.0 - f1 * f1 * 2.0, 0.0, 1.0); break;
                case 6: total += tone * clamp(e * 3.0, 0.0, 1.0); break;
                default: total += 1.0 - clamp(abs(e - 0.15) * 5.0, 0.0, 1.0); break;
            }
        }
    }
    float v = (total / float(p.subs * p.subs) - 0.5) * p.contrast + 0.5;
    if (p.overflow == 1) {
        v = 0.5 + 0.5 * precise::tanh((v - 0.5) * 2.0);
    } else if (p.overflow == 2) {
        float t = precise::fmod(v, 2.0);
        t = t < 0.0 ? t + 2.0 : t;
        v = t > 1.0 ? 2.0 - t : t;
    } else {
        v = clamp(v, 0.0, 1.0);
    }
    if (p.invert) {
        v = 1.0 - v;
    }
    out[g.y * p.w + g.x] = float4(v, v, v, 1.0);
}
