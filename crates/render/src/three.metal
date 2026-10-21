// A run of 3D layers put together on the GPU: what `Scene::pixel` in three.rs does, one thread a
// canvas pixel. Every step follows the processor's, in the same order, so the two draw the same
// pictures (to within the last bits of a float).
//
// Each plane's perspective is given relative to a canvas point near it, so its numbers stay
// small and single precision keeps what double precision has there: at canvas point (x, y), with
// x' = x - cx0 and y' = y - cy0, the layer point is (u0 + du, v0 + dv) where
//     w = g x' + h y' + 1,   du = (Au x' + Bu y') / w,   dv = (Av x' + Bv y') / w,
// and its depth from the camera is z0 + d0 du + d1 dv.
//
// Built after light.metal (the lights, and an environment's) and shadows.metal (the pictures,
// and the shadows).

struct Plane {
    float4 m;        // Au, Bu, Av, Bv
    float4 w;        // g, h, cx0, cy0
    float4 at;       // u0, v0, depth there (from the scene's middle), depth there (from the eye)
    float4 dz;       // d0, d1
    float4 tex;      // the picture's pixel at (u0, v0): x, y; its pixels a unit
    int4 reach;      // the canvas pixels it may cover: x0, y0, x1, y1
    uint4 img;       // the picture: first texel, width, height
    float4 origin;   // the world point of (0, 0); opacity
    float4 across;
    float4 down;
    float4 normal;
    float4 mat0;     // ambient, diffuse, specular, shininess
    float4 mat1;     // metal, transmission, shine exponent
    uint4 flags;     // casts shadows (0 off, 1 on, 2 only), accepts them (0, 1, 2 only), takes light, blend mode
    uint4 more;      // seed, matte's first value (NONE: no matte), only over what is under, lit already
    uint4 order;     // its place in the stack
    float4 mirror;   // how much it mirrors, how sharply, how much less face on
    uint4 map;       // the points the light bouncing onto it is gathered at (Caster's)
    float4 cell;
};

// A model layer as drawn (lit already): its colours and its depths (from the scene's middle,
// endless where it is not), a row of each `width` long.
struct Shot {
    int4 reach;      // the canvas pixels it covers: x0, y0, x1, y1
    uint4 at;        // its first colour, its first depth
    float4 how;      // opacity
    uint4 more;      // seed, matte's first value (NONE: no matte), only over what is under, blend mode
    uint4 order;     // its place in the stack
};

struct Params {
    uint4 a;         // canvas width, first column, first row, columns
    uint4 b;         // planes, lights, flat layers casting shadows, shots
    float4 eye;      // xyz; the environment's turn
    uint4 c;         // the environment's levels it shines with (0: none); how many pixels the list of those left to the processor holds
    float4 d;        // the environment's occlusion; how far round a point the ambient occlusion looks (0: none occludes); how much light bounces
};

struct World {
    const device Plane *planes;
    const device float4 *texels;
    constant Light *lights;
    Shadows shadows;
    const device float *mattes;
    constant Params *params;
    const device Shot *shots;
    const device float *depths;
    const device float4 *shot_px;
    Env env;
    const device Face *faces;
    const device float4 *bounce;   // the light bouncing onto each point it was gathered at
};

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
static bool under(const device Plane &p, float x, float y, thread float &du, thread float &dv, thread float &z) {
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

static float4 texel(thread const World &s, const device Plane &p, float du, float dv) {
    return bilinear(s.texels, p.img, p.tex.x + du * p.tex.z, p.tex.y + dv * p.tex.z);
}

// with more samples where it is seen smaller than its pixels
static float4 sample(thread const World &s, const device Plane &p, float du, float dv, float cx, float cy) {
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

// A plane's texel (premultiplied) at its point (u, v), lit by the lights and the environment.
static float4 light(thread const World &s, uint i, float u, float v, float4 tx) {
    const device Plane &p = s.planes[i];
    uint nl = s.params->b.y;
    bool env = s.env.count != 0;
    if ((nl == 0 && !env) || p.flags.z == 0) return tx;
    float a = tx.w;
    float3 rgb = tx.xyz / a;
    float3 at = p.origin.xyz + p.across.xyz * u + p.down.xyz * v;
    float3 viewer = normalize(s.params->eye.xyz - at);
    float3 n = p.normal.xyz;
    if (dot(n, viewer) < 0.0) n = -n;
    // how open the sky round it is (on the side it is seen from)
    float looks = s.params->d.y;
    float open = (looks > 0.0 && p.flags.y != 0) ? open_sky(s.shadows, i, at, n, looks) : 1.0;
    float3 ambient = float3(0.0), diffuse = float3(0.0), shine = float3(0.0);
    float shadow = 0.0;
    float exponent = p.mat1.z;
    for (uint li = 0; li < nl; li++) {
        constant Light &l = s.lights[li];
        if (l.kind.x == AMBIENT) {
            ambient += l.color.xyz * (1.0 - l.more.x * (1.0 - open));
            continue;
        }
        float k = reach(l, at);
        if (k <= 0.0) continue;
        float far;
        float3 to = toward(l, at, far);
        float facing = dot(n, to);
        float lambert = facing > 0.0 ? facing : -facing * p.mat1.y;
        if (lambert <= 0.0) continue;
        float3 through = (l.kind.z != 0 && p.flags.y != 0) ? unshadowed(s.shadows, l, i, at) : float3(1.0);
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
    float3 lit = nl == 0 ? float3(0.0) : rgb * (p.mat0.x * ambient + p.mat0.y * diffuse) + p.mat0.z * shine;
    // the environment's light: its diffuse and shine as its options say (50 % as it is), not a
    // metal, as dull as its shine is broad
    if (env) lit += env_light(s.env, rgb, p.normal.xyz, viewer, 0.0, 1.0 - p.mat0.w, p.mat0.y * 2.0, p.mat0.z * 2.0, 1.0 - s.params->d.x * (1.0 - open));
    // the light bouncing onto it from the surfaces round it
    if (s.params->d.z > 0.0 && p.map.w != 0) {
        lit += rgb * (p.mat0.y * 2.0 * s.params->d.z) * grid_at(s.bounce, p.map, p.cell, u, v);
    }
    // what it mirrors, mixed in
    if (p.mirror.x > 0.0) {
        float r = p.mirror.x * (1.0 - p.mirror.z * clamp(dot(n, viewer), 0.0, 1.0));
        if (r > 0.0) {
            Seen by = { s.faces, s.lights, nl, s.env, s.bounce, s.params->d.z };
            float3 seen = mirrored(s.shadows, by, i, at, n, viewer, p.mirror.y, lit);
            lit = lit * (1.0 - r) + seen * r;
        }
    }
    if (p.flags.y == 2) {
        return float4(0.0, 0.0, 0.0, a * shadow);
    }
    return float4(lit * a, a);
}

// ---- a pixel -------------------------------------------------------------------------------
//
// A pixel's pieces are the planes there (in front, not clear), then the model layers there
// (numbered after the planes'), kept in a short list with their depths. They are laid from the
// farthest to the nearest by picking, again and again, the farthest of those not laid yet (at the
// same depth, the lower in the stack first, then the one gathered first, as the processor's stable
// sort has it). A pixel with more pieces than the list holds is left to the processor.

#define MOST_AT 16

struct Pieces {
    uint id[MOST_AT];
    float z[MOST_AT];
    uint n;
    bool over;       // more than it holds
};

static void add(thread Pieces &out, uint id, float z) {
    if (out.n == MOST_AT) {
        out.over = true;
        return;
    }
    out.id[out.n] = id;
    out.z[out.n] = z;
    out.n++;
}

// where in a model layer's colours and depths canvas pixel (x, y) is
static uint shot_at(const device Shot &q, uint x, uint y) {
    return (y - uint(q.reach.y)) * uint(q.reach.z - q.reach.x) + (x - uint(q.reach.x));
}

// the pieces at canvas point (cx, cy) of pixel (x, y)
static void gather(thread const World &s, uint x, uint y, float cx, float cy, thread Pieces &out) {
    out.n = 0;
    out.over = false;
    uint planes = s.params->b.x;
    int xi = int(x), yi = int(y);
    for (uint i = 0; i < planes; i++) {
        const device Plane &p = s.planes[i];
        if (xi < p.reach.x || xi >= p.reach.z || yi < p.reach.y || yi >= p.reach.w || p.flags.x == 2) continue;
        float du, dv, z;
        if (!under(p, cx, cy, du, dv, z)) continue;
        if (sample(s, p, du, dv, cx, cy).w <= 1e-5) continue;
        add(out, i, z);
    }
    // a model layer's piece is the same all over the pixel
    for (uint k = 0; k < s.params->b.w; k++) {
        const device Shot &q = s.shots[k];
        if (xi < q.reach.x || xi >= q.reach.z || yi < q.reach.y || yi >= q.reach.w) continue;
        uint at = shot_at(q, x, y);
        float z = s.depths[q.at.y + at];
        if (!isfinite(z) || !(s.shot_px[q.at.x + at].w > 1e-5)) continue;
        add(out, planes + k, z);
    }
}

// how much a plane's depth changes across the pixel
static float change(const device Plane &p, float z, float cx, float cy) {
    float c = 0.0, du, dv, z1;
    if (under(p, cx + 1.0, cy, du, dv, z1)) c += fabs(z1 - z);
    if (under(p, cx, cy + 1.0, du, dv, z1)) c += fabs(z1 - z);
    return c;
}

// whether the pieces' depths cross within the pixel (a model's edges are smooth already: only
// planes change across it)
static bool crossing(thread const World &s, thread const Pieces &ps, float cx, float cy) {
    uint planes = s.params->b.x;
    for (uint a = 0; a < ps.n; a++) {
        float ca = ps.id[a] < planes ? change(s.planes[ps.id[a]], ps.z[a], cx, cy) : 0.0;
        for (uint b = a + 1; b < ps.n; b++) {
            float cb = ps.id[b] < planes ? change(s.planes[ps.id[b]], ps.z[b], cx, cy) : 0.0;
            if (fabs(ps.z[a] - ps.z[b]) < (ca + cb) * 0.75) return true;
        }
    }
    return false;
}

// piece i's place in the stack
static uint order_of(thread const World &s, uint i) {
    uint planes = s.params->b.x;
    return i < planes ? s.planes[i].order.x : s.shots[i - planes].order.x;
}

// whether piece (z, i) is laid before (zb, j): the farther first, then the lower in the stack,
// then the one gathered first
static bool before(thread const World &s, float z, uint i, float zb, uint j) {
    if (z != zb) return z > zb;
    uint oi = order_of(s, i), oj = order_of(s, j);
    if (oi != oj) return oi < oj;
    return i < j;
}

// the pieces at (cx, cy) over `below`
static float4 lay(thread const World &s, thread const Pieces &ps, float4 below, uint x, uint y, float cx, float cy) {
    float4 out = below;
    uint width = s.params->a.x;
    uint planes = s.params->b.x;
    // the last laid (none yet: before everything)
    uint last = NONE;
    for (uint step = 0; step < ps.n; step++) {
        uint pick = NONE;
        for (uint k = 0; k < ps.n; k++) {
            if (last != NONE && !before(s, ps.z[last], ps.id[last], ps.z[k], ps.id[k])) continue;
            if (pick == NONE || before(s, ps.z[k], ps.id[k], ps.z[pick], ps.id[pick])) pick = k;
        }
        if (pick == NONE) break;
        last = pick;
        uint id = ps.id[pick];
        if (id >= planes) {
            // a model layer: lit already
            const device Shot &q = s.shots[id - planes];
            float k = q.how.x;
            if (q.more.y != NONE) k *= s.mattes[q.more.y + y * width + x];
            if (q.more.z != 0) k *= below.w;
            if (k <= 0.0) continue;
            out = blend(q.more.w, out, s.shot_px[q.at.x + shot_at(q, x, y)] * k, noise(x, y, q.more.x));
            continue;
        }
        const device Plane &p = s.planes[id];
        float k = p.origin.w;
        if (p.more.y != NONE) k *= s.mattes[p.more.y + y * width + x];
        if (p.more.z != 0) k *= below.w;
        if (k <= 0.0) continue;
        float du, dv, z;
        under(p, cx, cy, du, dv, z);
        float4 tx = sample(s, p, du, dv, cx, cy);
        float4 color = p.more.w != 0 ? tx : light(s, id, p.at.x + du, p.at.y + dv, tx);
        out = blend(p.flags.w, out, color * k, noise(x, y, p.more.x));
    }
    return out;
}

kernel void scene(device float4 *canvas [[buffer(0)]],
                  const device Plane *planes [[buffer(1)]],
                  const device float4 *texels [[buffer(2)]],
                  constant Light *lights [[buffer(3)]],
                  const device Caster *casters [[buffer(4)]],
                  const device float *mattes [[buffer(5)]],
                  constant Params &params [[buffer(6)]],
                  const device Shot *shots [[buffer(7)]],
                  const device float *depths [[buffer(8)]],
                  const device float4 *env_px [[buffer(9)]],
                  const device uint4 *env_levels [[buffer(10)]],
                  device atomic_uint *left [[buffer(11)]],
                  device uint *left_at [[buffer(12)]],
                  const device float4 *shadow_px [[buffer(13)]],
                  const device float4 *shot_px [[buffer(14)]],
                  const device Face *faces [[buffer(15)]],
                  const device float4 *bounce [[buffer(16)]],
#if RAYS
                  primitive_acceleration_structure models [[buffer(17)]],
                  primitive_acceleration_structure everything [[buffer(18)]],
#endif
                  uint2 at [[thread_position_in_grid]]) {
#if RAYS
    Shadows shadows = { casters, params.b.z, shadow_px, models, everything };
#else
    Shadows shadows = { casters, params.b.z, shadow_px };
#endif
    World s = { planes, texels, lights, shadows, mattes, &params, shots, depths, shot_px, { env_px, env_levels, params.c.x, params.eye.w }, faces, bounce };
    if (at.x >= params.a.w) return;
    uint x = params.a.y + at.x;
    uint y = params.a.z + at.y;
    uint width = params.a.x;
    // the canvas buffer holds its rows from the first one drawn
    uint i = at.y * width + x;
    float4 below = canvas[i];
    float cx = float(x) + 0.5;
    float cy = float(y) + 0.5;
    Pieces ps;
    gather(s, x, y, cx, cy, ps);
    bool over = ps.over;
    float4 out = below;
    if (!over && ps.n > 1 && crossing(s, ps, cx, cy)) {
        // a crossing goes through this pixel: four looks, put together
        float4 sum = float4(0.0);
        for (uint k = 0; k < 4 && !over; k++) {
            float sx = cx + (k & 1 ? 0.25 : -0.25);
            float sy = cy + (k & 2 ? 0.25 : -0.25);
            Pieces sub;
            gather(s, x, y, sx, sy, sub);
            over = sub.over;
            sum += lay(s, sub, below, x, y, sx, sy) * 0.25;
        }
        out = sum;
    } else if (!over && ps.n > 0) {
        out = lay(s, ps, below, x, y, cx, cy);
    }
    if (over) {
        // too many pieces here: left to the processor
        uint k = atomic_fetch_add_explicit(left, 1u, memory_order_relaxed);
        if (k < params.c.y) left_at[k] = y * width + x;
        return;
    }
    canvas[i] = out;
}
