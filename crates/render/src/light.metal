// What the GPU's 3D kernels share: the lights, and how much of each reaches a point. Each step
// follows the processor's (`scene::Light`), in the same order.

#include <metal_stdlib>
using namespace metal;

#define NEAR 1.0
#define NONE 0xffffffffu

struct Light {
    float4 at;       // xyz, half the cone (radians)
    float4 dir;      // xyz, cone feather
    float4 color;    // rgb, radius
    float4 fall;     // falloff distance, shadow darkness, shadow diffusion
    uint4 kind;      // 0 parallel, 1 spot, 2 point, 3 ambient; falloff; casts shadows
    float4 more;     // an ambient light's occlusion (0 to 1), how far round a point it looks
};

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

// ---- an environment's light (scene::Environment) -------------------------------------------
//
// Its panoramas one after another: the light falling on a surface facing each way first, then
// the panorama halved again and again (what rougher surfaces shine with).

struct Env {
    const device float4 *px;
    const device uint4 *levels;  // each: its first texel, width, height (the falling light's first)
    uint count;                  // how many it shines with (0: no environment)
    float rotation;
};

// where world direction `d` falls on a panorama turned `rot` radians about the vertical
static float2 panorama_at(float3 d, float rot) {
    float u = 0.5 + (atan2(d.x, d.z) + rot) / 6.283185307179586;
    return float2(u - floor(u), acos(clamp(-d.y, -1.0, 1.0)) / 3.141592653589793);
}

// panorama `level`'s colour at (u, v), between its pixels, round past its left and right edges
static float3 pano_at(thread const Env &e, uint level, float2 at) {
    uint4 l = e.levels[level];
    int w = int(l.y), h = int(l.z);
    float x = at.x * float(w) - 0.5;
    float y = clamp(at.y * float(h) - 0.5, 0.0, float(h - 1));
    float x0 = floor(x), y0 = floor(y);
    float fx = x - x0, fy = y - y0;
    int i0 = int(x0), j0 = int(y0);
    int ia = i0 % w, ib = (i0 + 1) % w;
    ia += ia < 0 ? w : 0;
    ib += ib < 0 ? w : 0;
    int ja = min(j0, h - 1), jb = min(j0 + 1, h - 1);
    float3 a = e.px[l.x + uint(ja * w + ia)].xyz, b = e.px[l.x + uint(ja * w + ib)].xyz;
    float3 c = e.px[l.x + uint(jb * w + ia)].xyz, d = e.px[l.x + uint(jb * w + ib)].xyz;
    float3 top = a + (b - a) * fx;
    return top + (c + (d - c) * fx - top) * fy;
}

// what a surface `rough` (0 a mirror, 1 matte) shines with along direction `d`
static float3 env_shine(thread const Env &e, float3 d, float rough) {
    float level = clamp(rough, 0.0, 1.0) * float(e.count - 1);
    uint l0 = uint(floor(level));
    float f = level - floor(level);
    uint l1 = min(l0 + 1, e.count - 1);
    float2 at = panorama_at(d, e.rotation);
    float3 sa = pano_at(e, 1 + l0, at), sb = pano_at(e, 1 + l1, at);
    return sa + (sb - sa) * f;
}

// the light it gives a surface of colour `rgb` facing `n` (toward `viewer`), `metal` and `rough`,
// taking `diffuse` of the light falling on it and `specular` of its shine, its nooks `occlusion` dark
static float3 env_light(thread const Env &e, float3 rgb, float3 n, float3 viewer, float metal, float rough, float diffuse, float specular, float occlusion) {
    n = dot(n, viewer) < 0.0 ? -n : n;
    float nv = clamp(dot(n, viewer), 1e-4, 1.0);
    float3 reflected = n * (2.0 * nv) - viewer;
    float3 fall = pano_at(e, 0, panorama_at(n, e.rotation));
    float3 shine = env_shine(e, reflected, rough);
    // the share of light a surface shines back, by angle and roughness (Karis's fit of the split-sum)
    float r = rough;
    float c0 = 1.0 - r, c1 = r * -0.0275 + 0.0425, c2 = r * -0.572 + 1.04, c3 = r * 0.022 - 0.04;
    float a004 = min(c0 * c0, exp2(-9.28 * nv)) * c0 + c1;
    float scale = -1.04 * a004 + c2, bias = 1.04 * a004 + c3;
    float3 back = (0.04 + (rgb - 0.04) * metal) * scale + bias;
    return (fall * rgb * (1.0 - back) * (1.0 - metal) * diffuse + shine * back * specular) * occlusion;
}
