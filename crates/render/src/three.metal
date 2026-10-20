// A run of 3D layers put together on the GPU: what `Scene::pixel` in three.rs does, one thread a
// canvas pixel. Every step follows the processor's, in the same order, so the two draw the same
// pictures (to within the last bits of a float).
//
// Each plane's perspective is given relative to a canvas point near it, so its numbers stay
// small and single precision keeps what double precision has there: at canvas point (x, y), with
// x' = x - cx0 and y' = y - cy0, the layer point is (u0 + du, v0 + dv) where
//     w = g x' + h y' + 1,   du = (Au x' + Bu y') / w,   dv = (Av x' + Bv y') / w,
// and its depth from the camera is z0 + d0 du + d1 dv.

#include <metal_stdlib>
using namespace metal;

#define NEAR 1.0
#define NONE 0xffffffffu

struct Plane {
    float4 m;        // Au, Bu, Av, Bv
    float4 w;        // g, h, cx0, cy0
    float4 at;       // u0, v0, depth there (from the scene's middle), depth there (from the eye)
    float4 dz;       // d0, d1
    float4 tex;      // the picture's pixel at (u0, v0): x, y; its pixels a unit
    float4 own;      // its own picture (for shadows): origin x, y; its pixels a unit
    int4 reach;      // the canvas pixels it may cover: x0, y0, x1, y1
    uint4 img;       // the picture: first texel, width, height
    uint4 ownimg;    // its own picture: first texel, width, height
    float4 origin;   // the world point of (0, 0); opacity
    float4 across;
    float4 down;
    float4 normal;
    float4 unworld[4];
    float4 mat0;     // ambient, diffuse, specular, shininess
    float4 mat1;     // metal, transmission, shine exponent
    uint4 flags;     // casts shadows (0 off, 1 on, 2 only), accepts them (0, 1, 2 only), takes light, blend mode
    uint4 more;      // seed, matte's first value (NONE: no matte), only over what is under, lit already
    uint4 order;     // its place in the stack
};

struct Light {
    float4 at;       // xyz, half the cone (radians)
    float4 dir;      // xyz, cone feather
    float4 color;    // rgb, radius
    float4 fall;     // falloff distance, shadow darkness, shadow diffusion
    uint4 kind;      // 0 parallel, 1 spot, 2 point, 3 ambient; falloff; casts shadows
};

struct Params {
    uint4 a;         // canvas width, first column, first row, columns
    uint4 b;         // planes, lights, casters
    float4 eye;
};

struct World {
    constant Plane *planes;
    const device float4 *texels;
    constant Light *lights;
    constant uint *casters;
    const device float *mattes;
    constant Params *params;
};

// ---- pictures ----------------------------------------------------------------------------

static float4 fetch(const device float4 *t, uint4 img, int x, int y) {
    if (x < 0 || y < 0 || x >= int(img.y) || y >= int(img.z)) {
        return float4(0.0);
    }
    return t[img.x + uint(y) * img.y + uint(x)];
}

// between its pixels (centres at .5), clear outside
static float4 bilinear(const device float4 *t, uint4 img, float x, float y) {
    float fx = x - 0.5;
    float fy = y - 0.5;
    // far outside: nothing (and no number too big to be a whole one)
    if (!(fx > -2.0 && fy > -2.0 && fx < float(img.y) + 2.0 && fy < float(img.z) + 2.0)) {
        return float4(0.0);
    }
    float x0 = floor(fx);
    float y0 = floor(fy);
    float tx = fx - x0;
    float ty = fy - y0;
    int ix = int(x0);
    int iy = int(y0);
    float4 a = fetch(t, img, ix, iy);
    float4 b = fetch(t, img, ix + 1, iy);
    float4 c = fetch(t, img, ix, iy + 1);
    float4 d = fetch(t, img, ix + 1, iy + 1);
    float4 top = a + (b - a) * tx;
    float4 bottom = c + (d - c) * tx;
    return top + (bottom - top) * ty;
}

static float noise(uint x, uint y, uint seed) {
    uint h = (x * 0x8DA6B343u) ^ (y * 0xD8163841u) ^ (seed * 0xCB1AB31Fu);
    h ^= h >> 15;
    h *= 0x2C1B3C6Du;
    h ^= h >> 12;
    h *= 0x297A2D39u;
    h ^= h >> 15;
    return float(h >> 8) / float(1u << 24);
}

// ---- blend modes (veymelo_color::blend) --------------------------------------------------

enum Mode : uint {
    NORMAL = 0, DISSOLVE = 1, DANCING_DISSOLVE = 2, DARKEN = 3, MULTIPLY = 4, COLOR_BURN = 5,
    CLASSIC_COLOR_BURN = 6, LINEAR_BURN = 7, DARKER_COLOR = 8, ADD = 9, LIGHTEN = 10, SCREEN = 11,
    COLOR_DODGE = 12, CLASSIC_COLOR_DODGE = 13, LINEAR_DODGE = 14, LIGHTER_COLOR = 15, OVERLAY = 16,
    SOFT_LIGHT = 17, HARD_LIGHT = 18, LINEAR_LIGHT = 19, VIVID_LIGHT = 20, PIN_LIGHT = 21, HARD_MIX = 22,
    DIFFERENCE = 23, CLASSIC_DIFFERENCE = 24, EXCLUSION = 25, SUBTRACT = 26, DIVIDE = 27, HUE = 28,
    SATURATION = 29, COLOR = 30, LUMINOSITY = 31, STENCIL_ALPHA = 32, STENCIL_LUMA = 33,
    SILHOUETTE_ALPHA = 34, SILHOUETTE_LUMA = 35, ALPHA_ADD = 36, LUMINESCENT_PREMUL = 37
};

static float luminance(float3 c) {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
}

static bool keeps_over_range(uint m) {
    return !(m == COLOR_DODGE || m == CLASSIC_COLOR_DODGE || m == SOFT_LIGHT || m == HARD_LIGHT || m == LINEAR_LIGHT ||
             m == VIVID_LIGHT || m == PIN_LIGHT || m == HARD_MIX || m == HUE || m == SATURATION || m == COLOR);
}

static float color_burn(float b, float s) {
    if (b >= 1.0) return 1.0;
    if (s <= 0.0) return 0.0;
    return 1.0 - min((1.0 - b) / s, 1.0);
}

static float color_dodge(float b, float s) {
    if (b <= 0.0) return 0.0;
    if (s >= 1.0) return 1.0;
    return min(b / (1.0 - s), 1.0);
}

static float screen(float b, float s) {
    return b + s - b * s;
}

static float hard_light(float b, float s) {
    return s <= 0.5 ? b * (2.0 * s) : screen(b, 2.0 * s - 1.0);
}

static float soft_light(float b, float s) {
    if (s <= 0.5) {
        return b - (1.0 - 2.0 * s) * b * (1.0 - b);
    }
    float d = b <= 0.25 ? ((16.0 * b - 12.0) * b + 4.0) * b : sqrt(b);
    return b + (2.0 * s - 1.0) * (d - b);
}

static float channel(uint m, float b, float s) {
    switch (m) {
        case DARKEN: return min(b, s);
        case MULTIPLY: return b * s;
        case COLOR_BURN: return color_burn(b, s);
        case CLASSIC_COLOR_BURN: return s <= 0.0 ? 0.0 : 1.0 - (1.0 - b) / s;
        case LINEAR_BURN: return max(b + s - 1.0, 0.0);
        case ADD:
        case LINEAR_DODGE: return b + s;
        case LIGHTEN: return max(b, s);
        case SCREEN: return screen(b, s);
        case COLOR_DODGE: return color_dodge(b, s);
        case CLASSIC_COLOR_DODGE: return s >= 1.0 ? 1.0 : min(b / (1.0 - s), 1.0);
        case OVERLAY: return hard_light(s, b);
        case SOFT_LIGHT: return soft_light(b, s);
        case HARD_LIGHT: return hard_light(b, s);
        case LINEAR_LIGHT: return clamp(b + 2.0 * s - 1.0, 0.0, 1.0);
        case VIVID_LIGHT: return s <= 0.5 ? color_burn(b, 2.0 * s) : color_dodge(b, 2.0 * s - 1.0);
        case PIN_LIGHT: return s <= 0.5 ? min(b, 2.0 * s) : max(b, 2.0 * s - 1.0);
        case HARD_MIX: return b + s >= 1.0 ? 1.0 : 0.0;
        case DIFFERENCE:
        case CLASSIC_DIFFERENCE: return fabs(b - s);
        case EXCLUSION: return b + s - 2.0 * b * s;
        case SUBTRACT: return max(b - s, 0.0);
        case DIVIDE: return s <= 0.0 ? (b > 0.0 ? 1.0 : 0.0) : b / s;
        default: return s;
    }
}

static float lum(float3 c) {
    return 0.3 * c.x + 0.59 * c.y + 0.11 * c.z;
}

static float3 clip_color(float3 c) {
    float l = lum(c);
    float lo = min(min(c.x, c.y), c.z);
    float hi = max(max(c.x, c.y), c.z);
    float3 o = c;
    if (lo < 0.0) {
        o = l + (o - l) * l / max(l - lo, 1e-9);
    }
    if (hi > 1.0) {
        o = l + (o - l) * (1.0 - l) / max(hi - l, 1e-9);
    }
    return o;
}

static float3 with_lum(float3 c, float l) {
    return clip_color(c + (l - lum(c)));
}

static float sat(float3 c) {
    return max(max(c.x, c.y), c.z) - min(min(c.x, c.y), c.z);
}

static float3 with_sat(float3 c, float s) {
    float lo = min(min(c.x, c.y), c.z);
    float hi = max(max(c.x, c.y), c.z);
    if (hi - lo <= 1e-9) return float3(0.0);
    return (c - lo) * s / (hi - lo);
}

static float3 formula(uint m, float3 b, float3 s) {
    switch (m) {
        case HUE: return with_lum(with_sat(s, sat(b)), lum(b));
        case SATURATION: return with_lum(with_sat(b, sat(s)), lum(b));
        case COLOR: return with_lum(s, lum(b));
        case LUMINOSITY: return with_lum(b, lum(s));
        case DARKER_COLOR: return luminance(s) < luminance(b) ? s : b;
        case LIGHTER_COLOR: return luminance(s) > luminance(b) ? s : b;
        default: return float3(channel(m, b.x, s.x), channel(m, b.y, s.y), channel(m, b.z, s.z));
    }
}

static float4 blend(uint m, float4 dst, float4 src, float n) {
    float sa = src.w;
    float da = dst.w;
    switch (m) {
        case NORMAL: {
            float k = 1.0 - sa;
            return float4(src.xyz + dst.xyz * k, sa + da * k);
        }
        case DISSOLVE:
        case DANCING_DISSOLVE:
            if (sa <= 0.0 || n >= sa) return dst;
            return float4(src.xyz / sa, 1.0);
        case STENCIL_ALPHA: return dst * sa;
        case SILHOUETTE_ALPHA: return dst * (1.0 - sa);
        case STENCIL_LUMA: return dst * luminance(src.xyz);
        case SILHOUETTE_LUMA: return dst * (1.0 - luminance(src.xyz));
        case ALPHA_ADD: {
            float k = 1.0 - sa;
            return float4(src.xyz + dst.xyz * k, min(sa + da, 1.0));
        }
        case LUMINESCENT_PREMUL: {
            float k = 1.0 - sa;
            return float4(src.xyz + dst.xyz * k, sa + da - sa * da);
        }
        default: break;
    }
    if (sa <= 0.0) return dst;
    if (da <= 0.0) return src;
    float3 cs = src.xyz / sa;
    float3 cb = dst.xyz / da;
    if (!keeps_over_range(m)) {
        cs = clamp(cs, 0.0, 1.0);
        cb = clamp(cb, 0.0, 1.0);
    }
    float4 out;
    float neutral = -1.0;
    if (m == CLASSIC_COLOR_DODGE || m == CLASSIC_DIFFERENCE) neutral = 0.0;
    if (m == CLASSIC_COLOR_BURN) neutral = 1.0;
    if (neutral >= 0.0) {
        float3 b = formula(m, cb, neutral + sa * (cs - neutral));
        out.xyz = (1.0 - da) * src.xyz + da * b;
    } else {
        float3 b = formula(m, cb, cs);
        out.xyz = (1.0 - da) * src.xyz + (1.0 - sa) * dst.xyz + sa * da * b;
    }
    out.w = sa + da - sa * da;
    return out;
}

// ---- where a plane is ----------------------------------------------------------------------

// The layer point (relative to the plane's (u0, v0)) and depth under canvas point (x, y), if it
// is in front of the camera.
static bool under(constant Plane &p, float x, float y, thread float &du, thread float &dv, thread float &z) {
    float xr = x - p.w.z;
    float yr = y - p.w.w;
    float w = p.w.x * xr + p.w.y * yr + 1.0;
    if (fabs(w) < 1e-30) return false;
    float k = 1.0 / w;
    du = (p.m.x * xr + p.m.y * yr) * k;
    dv = (p.m.z * xr + p.m.w * yr) * k;
    float far = p.at.w + p.dz.x * du + p.dz.y * dv;
    if (!(far >= NEAR)) return false;
    z = p.at.z + p.dz.x * du + p.dz.y * dv;
    return true;
}

static float4 texel(thread const World &s, constant Plane &p, float du, float dv) {
    return bilinear(s.texels, p.img, p.tex.x + du * p.tex.z, p.tex.y + dv * p.tex.z);
}

// with more samples where it is seen smaller than its pixels
static float4 sample(thread const World &s, constant Plane &p, float du, float dv, float cx, float cy) {
    float step = 0.0;
    float a, b, z;
    if (under(p, cx + 1.0, cy, a, b, z)) step = max(step, length(float2(a - du, b - dv)));
    if (under(p, cx, cy + 1.0, a, b, z)) step = max(step, length(float2(a - du, b - dv)));
    step *= p.tex.z;
    if (step <= 1.25) {
        return texel(s, p, du, dv);
    }
    uint n = clamp(uint(ceil(step)), 2u, 4u);
    float4 sum = float4(0.0);
    for (uint j = 0; j < n; j++) {
        for (uint i = 0; i < n; i++) {
            float sx = cx - 0.5 + (float(i) + 0.5) / float(n);
            float sy = cy - 0.5 + (float(j) + 0.5) / float(n);
            float su, sv, sz;
            if (under(p, sx, sy, su, sv, sz)) {
                sum += texel(s, p, su, sv);
            }
        }
    }
    return sum / float(n * n);
}

// ---- light ---------------------------------------------------------------------------------

enum Kind : uint { PARALLEL = 0, SPOT = 1, POINT = 2, AMBIENT = 3 };

static float3 toward(constant Light &l, float3 p, thread float &far) {
    if (l.kind.x == PARALLEL) {
        far = INFINITY;
        return -l.dir.xyz;
    }
    float3 d = l.at.xyz - p;
    float len = length(d);
    if (len < 1e-9) {
        far = 0.0;
        return float3(0.0, 0.0, -1.0);
    }
    far = len;
    return d / len;
}

static float smooth_step(float t) {
    return t * t * (3.0 - 2.0 * t);
}

static float reach(constant Light &l, float3 p) {
    float dist;
    float3 to = toward(l, p, dist);
    float k = 1.0;
    if (l.kind.y == 1 && isfinite(dist)) {
        float radius = l.color.w, fd = l.fall.x;
        k = dist <= radius ? 1.0 : (fd <= 0.0 ? 0.0 : 1.0 - smooth_step(clamp((dist - radius) / fd, 0.0, 1.0)));
    } else if (l.kind.y == 2 && isfinite(dist)) {
        float r = max(l.color.w, 1.0);
        k = dist <= r ? 1.0 : (r / dist) * (r / dist);
    }
    if (l.kind.x == SPOT) {
        float a = acos(clamp(dot(-to, l.dir.xyz), -1.0, 1.0));
        float cone = l.at.w;
        float inner = cone * (1.0 - l.dir.w);
        k *= a <= inner ? 1.0 : (a >= cone ? 0.0 : 1.0 - smooth_step((a - inner) / max(cone - inner, 1e-9)));
    }
    return k;
}

static float3 apply(constant float4 *m, float3 p) {
    float x = dot(m[0].xyz, p) + m[0].w;
    float y = dot(m[1].xyz, p) + m[1].w;
    float z = dot(m[2].xyz, p) + m[2].w;
    float w = dot(m[3].xyz, p) + m[3].w;
    if (w != 1.0 && w != 0.0) return float3(x / w, y / w, z / w);
    return float3(x, y, z);
}

// how much of the light gets through to q from one of its points (`dir`, `len` away)
static float3 pass_from(thread const World &s, uint own, float3 q, float3 dir, float len) {
    float3 pass = float3(1.0);
    uint n = s.params->b.z;
    for (uint k = 0; k < n; k++) {
        uint c = s.casters[k];
        if (c == own) continue;
        constant Plane &p = s.planes[c];
        float facing = dot(dir, p.normal.xyz);
        if (fabs(facing) < 1e-12) continue;
        float t = dot(p.origin.xyz - q, p.normal.xyz) / facing;
        if (t <= 1e-6 || t >= len - 1e-6) continue;
        float3 hit = apply(p.unworld, q + dir * t);
        float4 tx = bilinear(s.texels, p.ownimg, (hit.x - p.own.x) * p.own.z, (hit.y - p.own.y) * p.own.z);
        float a = clamp(tx.w, 0.0, 1.0);
        if (a <= 0.0) continue;
        float tr = p.mat1.y;
        pass *= 1.0 - a + a * tr * (tx.xyz / a);
    }
    return pass;
}

static uint whole(float v) {
    // as the processor's `as usize` then `as u32`: below zero (or not a number) is 0
    return v > 0.0 ? (v < 4294967296.0 ? uint(v) : 0xffffffffu) : 0u;
}

static float3 unshadowed(thread const World &s, uint own, uint li, float3 q) {
    constant Light &l = s.lights[li];
    bool others = false;
    for (uint k = 0; k < s.params->b.z; k++) {
        if (s.casters[k] != own) {
            others = true;
            break;
        }
    }
    if (!others) return float3(1.0);
    float far;
    float3 to = toward(l, q, far);
    float dark = l.fall.y;
    float diffusion = l.fall.z;
    if (diffusion <= 0.0) {
        float3 p = pass_from(s, own, q, to, far);
        return 1.0 - dark * (1.0 - p);
    }
    float3 side = fabs(to.x) < 0.9 ? float3(1.0, 0.0, 0.0) : float3(0.0, 1.0, 0.0);
    float3 a = normalize(cross(to, side));
    float3 b = cross(to, a);
    bool parallel = l.kind.x == PARALLEL;
    float3 centre = parallel ? q + to * 1000.0 : l.at.xyz;
    float turn = noise(whole(q.x * 7.0), whole(q.y * 7.0), as_type<uint>(int(q.z))) * 6.283185307179586;
    float3 few[4];
    for (uint k = 0; k < 4; k++) {
        float r = diffusion * sqrt((float(k) + 0.5) / 4.0);
        float ang = float(k) * 2.399963 + turn;
        float3 d = centre + a * (r * cos(ang)) + b * (r * sin(ang)) - q;
        float len = length(d);
        few[k] = pass_from(s, own, q, d / max(len, 1e-9), parallel ? INFINITY : len);
    }
    bool agree = true;
    for (uint k = 1; k < 4; k++) {
        float3 e = fabs(few[k] - few[0]);
        if (!(e.x < 1e-3 && e.y < 1e-3 && e.z < 1e-3)) agree = false;
    }
    float3 sum = float3(0.0);
    float n = 4.0;
    if (agree) {
        for (uint k = 0; k < 4; k++) sum += few[k];
    } else {
        n = 48.0;
        for (uint k = 0; k < 48; k++) {
            float r = diffusion * sqrt((float(k) + 0.5) / 48.0);
            float ang = float(k) * 2.399963 + turn;
            float3 d = centre + a * (r * cos(ang)) + b * (r * sin(ang)) - q;
            float len = length(d);
            sum += pass_from(s, own, q, d / max(len, 1e-9), parallel ? INFINITY : len);
        }
    }
    return 1.0 - dark * (1.0 - sum / n);
}

// A plane's texel (premultiplied) at its point (u, v), lit by the lights.
static float4 light(thread const World &s, uint i, float u, float v, float4 tx) {
    constant Plane &p = s.planes[i];
    uint nl = s.params->b.y;
    if (nl == 0 || p.flags.z == 0) return tx;
    float a = tx.w;
    float3 rgb = tx.xyz / a;
    float3 at = p.origin.xyz + p.across.xyz * u + p.down.xyz * v;
    float3 viewer = normalize(s.params->eye.xyz - at);
    float3 n = p.normal.xyz;
    if (dot(n, viewer) < 0.0) n = -n;
    float3 ambient = float3(0.0), diffuse = float3(0.0), shine = float3(0.0);
    float shadow = 0.0;
    float exponent = p.mat1.z;
    for (uint li = 0; li < nl; li++) {
        constant Light &l = s.lights[li];
        if (l.kind.x == AMBIENT) {
            ambient += l.color.xyz;
            continue;
        }
        float k = reach(l, at);
        if (k <= 0.0) continue;
        float far;
        float3 to = toward(l, at, far);
        float facing = dot(n, to);
        float lambert = facing > 0.0 ? facing : -facing * p.mat1.y;
        if (lambert <= 0.0) continue;
        float3 through = (l.kind.z != 0 && p.flags.y != 0) ? unshadowed(s, i, li, at) : float3(1.0);
        shadow = max(shadow, 1.0 - (through.x + through.y + through.z) / 3.0);
        float kd = lambert * k;
        diffuse += l.color.xyz * kd * through;
        if (facing > 0.0 && p.mat0.z > 0.0) {
            float3 half_way = normalize(to + viewer);
            float sh = pow(max(dot(n, half_way), 0.0), exponent) * k;
            float3 tint = 1.0 + (rgb - 1.0) * p.mat1.x;
            shine += l.color.xyz * sh * through * tint;
        }
    }
    if (p.flags.y == 2) {
        return float4(0.0, 0.0, 0.0, a * shadow);
    }
    float3 lit = rgb * (p.mat0.x * ambient + p.mat0.y * diffuse) + p.mat0.z * shine;
    return float4(lit * a, a);
}

// ---- a pixel -------------------------------------------------------------------------------
//
// A pixel's pieces are the planes there (in front, not clear) — which ones are kept as bits, so
// a thread holds no list. They are laid from the farthest to the nearest by picking, again and
// again, the farthest of those not laid yet (at the same depth, the lower in the stack first,
// as the processor's stable sort has it).

// which planes have a piece at canvas point (cx, cy) of pixel (x, y); how many
static uint gather(thread const World &s, uint x, uint y, float cx, float cy, thread uint2 &bits) {
    uint n = 0;
    bits = uint2(0);
    uint planes = s.params->b.x;
    for (uint i = 0; i < planes; i++) {
        constant Plane &p = s.planes[i];
        int xi = int(x), yi = int(y);
        if (xi < p.reach.x || xi >= p.reach.z || yi < p.reach.y || yi >= p.reach.w || p.flags.x == 2) continue;
        float du, dv, z;
        if (!under(p, cx, cy, du, dv, z)) continue;
        if (sample(s, p, du, dv, cx, cy).w <= 1e-5) continue;
        bits[i >> 5] |= 1u << (i & 31);
        n++;
    }
    return n;
}

static bool has(uint2 bits, uint i) {
    return (bits[i >> 5] >> (i & 31)) & 1u;
}

// how much a plane's depth changes across the pixel
static float change(constant Plane &p, float z, float cx, float cy) {
    float c = 0.0, du, dv, z1;
    if (under(p, cx + 1.0, cy, du, dv, z1)) c += fabs(z1 - z);
    if (under(p, cx, cy + 1.0, du, dv, z1)) c += fabs(z1 - z);
    return c;
}

// whether the pieces' depths cross within the pixel
static bool crossing(thread const World &s, uint2 bits, float cx, float cy) {
    uint planes = s.params->b.x;
    for (uint a = 0; a < planes; a++) {
        if (!has(bits, a)) continue;
        constant Plane &pa = s.planes[a];
        float du, dv, za;
        under(pa, cx, cy, du, dv, za);
        float ca = change(pa, za, cx, cy);
        for (uint b = a + 1; b < planes; b++) {
            if (!has(bits, b)) continue;
            constant Plane &pb = s.planes[b];
            float zb;
            under(pb, cx, cy, du, dv, zb);
            if (fabs(za - zb) < (ca + change(pb, zb, cx, cy)) * 0.75) return true;
        }
    }
    return false;
}

// whether piece (z, plane i) is laid before (zb, plane j): the farther first, then the lower in
// the stack, then the earlier plane
static bool before(thread const World &s, float z, uint i, float zb, uint j) {
    if (z != zb) return z > zb;
    uint oi = s.planes[i].order.x, oj = s.planes[j].order.x;
    if (oi != oj) return oi < oj;
    return i < j;
}

// the pieces at (cx, cy) over `below`
static float4 lay(thread const World &s, uint2 bits, uint n, float4 below, uint x, uint y, float cx, float cy) {
    float4 out = below;
    uint width = s.params->a.x;
    uint planes = s.params->b.x;
    // the last laid (none yet: before everything)
    float last_z = INFINITY;
    uint last = NONE;
    for (uint step = 0; step < n; step++) {
        uint pick = NONE;
        float pick_z = 0.0, pick_du = 0.0, pick_dv = 0.0;
        for (uint i = 0; i < planes; i++) {
            if (!has(bits, i)) continue;
            float du, dv, z;
            under(s.planes[i], cx, cy, du, dv, z);
            if (last != NONE && !before(s, last_z, last, z, i)) continue;
            if (pick == NONE || before(s, z, i, pick_z, pick)) {
                pick = i;
                pick_z = z;
                pick_du = du;
                pick_dv = dv;
            }
        }
        if (pick == NONE) break;
        last = pick;
        last_z = pick_z;
        constant Plane &p = s.planes[pick];
        float k = p.origin.w;
        if (p.more.y != NONE) k *= s.mattes[p.more.y + y * width + x];
        if (p.more.z != 0) k *= below.w;
        if (k <= 0.0) continue;
        float4 tx = sample(s, p, pick_du, pick_dv, cx, cy);
        float4 color = p.more.w != 0 ? tx : light(s, pick, p.at.x + pick_du, p.at.y + pick_dv, tx);
        out = blend(p.flags.w, out, color * k, noise(x, y, p.more.x));
    }
    return out;
}

kernel void scene(device float4 *canvas [[buffer(0)]],
                  constant Plane *planes [[buffer(1)]],
                  const device float4 *texels [[buffer(2)]],
                  constant Light *lights [[buffer(3)]],
                  constant uint *casters [[buffer(4)]],
                  const device float *mattes [[buffer(5)]],
                  constant Params &params [[buffer(6)]],
                  uint2 at [[thread_position_in_grid]]) {
    World s = { planes, texels, lights, casters, mattes, &params };
    if (at.x >= params.a.w) return;
    uint x = params.a.y + at.x;
    uint y = params.a.z + at.y;
    uint width = params.a.x;
    // the canvas buffer holds its rows from the first one drawn
    uint i = at.y * width + x;
    float4 below = canvas[i];
    float cx = float(x) + 0.5;
    float cy = float(y) + 0.5;
    uint2 bits;
    uint n = gather(s, x, y, cx, cy, bits);
    if (n == 0) return;
    if (n > 1 && crossing(s, bits, cx, cy)) {
        // a crossing goes through this pixel: four looks, put together
        float4 sum = float4(0.0);
        for (uint k = 0; k < 4; k++) {
            float sx = cx + (k & 1 ? 0.25 : -0.25);
            float sy = cy + (k & 2 ? 0.25 : -0.25);
            uint2 sub;
            uint m = gather(s, x, y, sx, sy, sub);
            sum += lay(s, sub, m, below, x, y, sx, sy) * 0.25;
        }
        canvas[i] = sum;
        return;
    }
    canvas[i] = lay(s, bits, n, below, x, y, cx, cy);
}
