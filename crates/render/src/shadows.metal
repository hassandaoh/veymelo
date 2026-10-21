// Shadows, as the processor finds them (`Scene::unshadowed` in three.rs): from a point toward
// each light, past the flat layers that cast them (a see-through layer lets some light through,
// coloured by it by its light transmission) and the models (traced through the GPU's arrangement
// of their triangles, when RAYS is defined); a light's shadow diffusion makes it a disc, looked at
// from many points of it. And the pictures the kernels read.
//
// Built after light.metal, before the kernels' own.

#if RAYS
#include <metal_raytracing>
using namespace metal::raytracing;
#endif

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

// ---- shadows -------------------------------------------------------------------------------

// A flat layer, as rays meet it.
struct Caster {
    float4 origin;   // the world point of its (0, 0); its light transmission
    float4 normal;   // xyz; whether it casts shadows (1) or not (0)
    float4 unworld[4];
    float4 own;      // its picture: origin x, y; its pixels a unit
    uint4 img;       // its picture: first texel, width, height; which plane of the scene it is
    float4 mat;      // how it takes light: ambient, diffuse, takes light; seen (0: it casts its shadow alone)
    uint4 map;       // the points the light bouncing onto it is gathered at: the first, across, down; whether it has them
    float4 cell;     // where they are: the grid's corner (its own units), a cell across and down
    float4 mirror;   // how much it mirrors, how much less face on
    uint4 caustic;   // the cells the light glass bends onto it is gathered in, as `map`
    float4 bent;     // and where they are, as `cell`
};

// The light bouncing onto a flat layer at its point (u, v), between the points of its grid
// (`map`, `cell`) in `light` (bounce::Grid::at).
static float3 grid_at(const device float4 *light, uint4 map, float4 cell, float u, float v) {
    float x = clamp((u - cell.x) / cell.z - 0.5, 0.0, float(map.y - 1));
    float y = clamp((v - cell.y) / cell.w - 0.5, 0.0, float(map.z - 1));
    uint x0 = uint(floor(x)), y0 = uint(floor(y));
    uint x1 = min(x0 + 1, map.y - 1), y1 = min(y0 + 1, map.z - 1);
    float fx = x - floor(x), fy = y - floor(y);
    float3 a = light[map.x + y0 * map.y + x0].xyz, b = light[map.x + y0 * map.y + x1].xyz;
    float3 c = light[map.x + y1 * map.y + x0].xyz, d = light[map.x + y1 * map.y + x1].xyz;
    float3 top = a + (b - a) * fx;
    return top + (c + (d - c) * fx - top) * fy;
}

struct Face {
    float4 normals[3];  // its corners' (w: the layer's ambient, diffuse, whether it takes light)
    float4 colours[3];  // its corners' (w: how much the layer mirrors, how much less face on)
    float4 glow;
    uint4 probes;       // the points its corners' bouncing light was gathered at, its picture (NONE: none)
    float4 uvs[2];      // its corners' points on its picture (u, v each); whether colours are worked in linear light
};

// A picture as the GPU reads it, among others one after another.
struct Pic {
    uint4 a;         // first texel, width, height; read between its pixels (bit 0), wrap across (bits 1-2) and down (bits 3-4)
};

// a picture's pixel `v` along `n` of them, past its edges as it repeats (0 repeat, 1 clamp, 2 mirror)
static uint fold(int v, int n, uint wrap) {
    if (wrap == 1) return uint(clamp(v, 0, n - 1));
    if (wrap == 2) {
        int m = v % (2 * n);
        if (m < 0) m += 2 * n;
        return uint(m < n ? m : 2 * n - 1 - m);
    }
    int m = v % n;
    if (m < 0) m += n;
    return uint(m);
}

static float4 pic_texel(const device float4 *texels, Pic p, float x, float y) {
    // (held to what a whole number holds: far past the edges is as far as can be)
    int ix = int(clamp(x, -1e9, 1e9));
    int iy = int(clamp(y, -1e9, 1e9));
    int w = int(p.a.y), h = int(p.a.z);
    return texels[p.a.x + fold(iy, h, (p.a.w >> 3) & 3u) * uint(w) + fold(ix, w, (p.a.w >> 1) & 3u)];
}

// picture `p` at its point `at` (0 to 1 across and down; Texture::at): white with no pixels
static float4 pic_at(const device float4 *texels, Pic p, float2 at) {
    if (p.a.y == 0 || p.a.z == 0) return float4(1.0);
    float x = at.x * float(p.a.y) - 0.5;
    float y = at.y * float(p.a.z) - 0.5;
    if ((p.a.w & 1u) == 0) return pic_texel(texels, p, round(x), round(y));
    float x0 = floor(x), y0 = floor(y);
    float tx = x - x0, ty = y - y0;
    float4 a = pic_texel(texels, p, x0, y0), b = pic_texel(texels, p, x0 + 1.0, y0);
    float4 c = pic_texel(texels, p, x0, y0 + 1.0), d = pic_texel(texels, p, x0 + 1.0, y0 + 1.0);
    float4 top = a + (b - a) * tx;
    float4 bottom = c + (d - c) * tx;
    return top + (bottom - top) * ty;
}

static float srgb_to_linear(float v) {
    return v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4);
}

// What casts shadows: the flat layers (and their pictures), and with rays the models casting
// them and all of them; each model triangle's face (shadows go through glass), and for each
// casting triangle its face if its model is glass (the first: whether any is).
struct Shadows {
    const device Caster *casters;
    uint count;
    const device float4 *texels;
#if RAYS
    primitive_acceleration_structure models;      // those casting shadows
    primitive_acceleration_structure everything;  // all of them (for the ambient occlusion)
    const device Face *faces;
    const device uint *glass;
    const device Pic *pics;          // the faces' pictures
    const device float4 *pic_texels;
#endif
};

// a face's colour at its corners' weights `w`: between its corners', times its picture there
static float3 face_rgb(thread const Shadows &sh, Face f, float3 w) {
    float3 rgb = f.colours[0].xyz * w.x + f.colours[1].xyz * w.y + f.colours[2].xyz * w.z;
#if RAYS
    if (f.probes.w != NONE) {
        float2 at = float2(f.uvs[0].x * w.x + f.uvs[0].z * w.y + f.uvs[1].x * w.z, f.uvs[0].y * w.x + f.uvs[0].w * w.y + f.uvs[1].y * w.z);
        float3 c = pic_at(sh.pic_texels, sh.pics[f.probes.w], at).xyz;
        if (f.uvs[1].z > 0.5) c = float3(srgb_to_linear(max(c.x, 0.0)), srgb_to_linear(max(c.y, 0.0)), srgb_to_linear(max(c.z, 0.0)));
        rgb *= c;
    }
#endif
    return rgb;
}

// The most glass surfaces a shadow's ray is followed through, and how far past one the next is
// looked for (three.rs).
constant constexpr uint GLASS_CROSSINGS = 16;
constant constexpr float GLASS_STEP = 1e-3;

static float3 apply(const device float4 *m, float3 p) {
    float x = dot(m[0].xyz, p) + m[0].w;
    float y = dot(m[1].xyz, p) + m[1].w;
    float z = dot(m[2].xyz, p) + m[2].w;
    float w = dot(m[3].xyz, p) + m[3].w;
    if (w != 1.0 && w != 0.0) return float3(x / w, y / w, z / w);
    return float3(x, y, z);
}

// how much of the light gets through to q (on plane `own`, or NONE) from one of its points
// (`dir`, `len` away)
static float3 pass_from(thread const Shadows &sh, uint own, float3 q, float3 dir, float len) {
    float3 pass = float3(1.0);
#if RAYS
    // a model in the way blocks it (a pixel off the surface, so one does not shade itself where
    // it is); glass lets its part through, tinted, surface after surface (three.rs's passes)
    if (sh.glass[0] == 0) {
        ray r(q, dir, 1.0, len - 1.0);
        intersector<> it;
        it.accept_any_intersection(true);
        if (it.intersect(r, sh.models).type != intersection_type::none) return float3(0.0);
    } else {
        float near = 1.0;
        for (uint k = 0; k < GLASS_CROSSINGS; k++) {
            ray r(q, dir, near, len - 1.0);
            intersector<triangle_data> it;
            intersection_result<triangle_data> h = it.intersect(r, sh.models);
            if (h.type != intersection_type::triangle) break;
            uint fi = sh.glass[1 + h.primitive_id];
            if (fi == NONE) return float3(0.0);
            // none straight through to a flat layer when the light glass bends lands where it
            // is bent to
            if (sh.glass[0] == 2 && own != NONE) return float3(0.0);
            const device Face &f = sh.faces[fi];
            float2 b = h.triangle_barycentric_coord;
            float3 wt = float3(1.0 - b.x - b.y, b.x, b.y);
            float3 n = f.normals[0].xyz * wt.x + f.normals[1].xyz * wt.y + f.normals[2].xyz * wt.z;
            n = length(n) > 1e-12 ? normalize(n) : float3(0.0, 0.0, 1.0);
            float3 rgb = face_rgb(sh, f, wt);
            float facing = clamp(fabs(dot(n, dir)), 0.0, 1.0);
            float clear = f.colours[2].w * (1.0 - f.glow.w * (1.0 - facing));
            pass *= sqrt(clear * max(rgb, 0.0));
            near = h.distance + GLASS_STEP;
        }
    }
#endif
    for (uint k = 0; k < sh.count; k++) {
        const device Caster &p = sh.casters[k];
        if (p.img.w == own || p.normal.w == 0.0) continue;
        float facing = dot(dir, p.normal.xyz);
        if (fabs(facing) < 1e-12) continue;
        float t = dot(p.origin.xyz - q, p.normal.xyz) / facing;
        if (t <= 1e-6 || t >= len - 1e-6) continue;
        float3 hit = apply(p.unworld, q + dir * t);
        float4 tx = bilinear(sh.texels, p.img, (hit.x - p.own.x) * p.own.z, (hit.y - p.own.y) * p.own.z);
        float a = clamp(tx.w, 0.0, 1.0);
        if (a <= 0.0) continue;
        pass *= 1.0 - a + a * p.origin.w * (tx.xyz / a);
    }
    return pass;
}

static uint whole(float v) {
    // as the processor's `as usize` then `as u32`: below zero (or not a number) is 0
    return v > 0.0 ? (v < 4294967296.0 ? uint(v) : 0xffffffffu) : 0u;
}

// how many points of a diffuse light's disc a point in the soft edge of its shadow looks at
#define SOFT_POINTS 128

// how much of light `l` reaches world point q (on plane `own`, or NONE) past what casts shadows,
// per colour, its darkness applied
static float3 unshadowed(thread const Shadows &sh, constant Light &l, uint own, float3 q) {
#if !RAYS
    bool others = false;
    for (uint k = 0; k < sh.count; k++) {
        if (sh.casters[k].img.w != own && sh.casters[k].normal.w != 0.0) {
            others = true;
            break;
        }
    }
    if (!others) return float3(1.0);
#endif
    float far;
    float3 to = toward(l, q, far);
    float dark = l.fall.y;
    float diffusion = l.fall.z;
    if (diffusion <= 0.0) {
        float3 p = pass_from(sh, own, q, to, far);
        return 1.0 - dark * (1.0 - p);
    }
    // a diffuse light is a disc across the way to it (a parallel light's as if it were 1000
    // pixels away), looked at from points spread evenly over it (a sunflower), the pattern turned
    // a little from point to point
    float3 side = fabs(to.x) < 0.9 ? float3(1.0, 0.0, 0.0) : float3(0.0, 1.0, 0.0);
    float3 a = normalize(cross(to, side));
    float3 b = cross(to, a);
    bool parallel = l.kind.x == PARALLEL;
    float3 centre = parallel ? q + to * 1000.0 : l.at.xyz;
    float turn = noise(whole(q.x * 7.0), whole(q.y * 7.0), as_type<uint>(int(q.z))) * 6.283185307179586;
    // a few points first: where they agree (wholly lit or wholly shadowed) that is it; in the
    // soft edge between, many more
    // its middle and eight points round its rim first: where they agree (wholly lit or wholly
    // shadowed) that is it (a shadow's edge coming over the disc crosses its rim); in the soft
    // edge between, many points over it
    float3 first = float3(0.0);
    bool agree = true;
    for (uint k = 0; k < 9; k++) {
        float r = k == 8 ? 0.0 : diffusion;
        float ang = float(k) * 6.283185307179586 / 8.0 + turn;
        float3 d = centre + a * (r * cos(ang)) + b * (r * sin(ang)) - q;
        float len = length(d);
        float3 p = pass_from(sh, own, q, d / max(len, 1e-9), parallel ? INFINITY : len);
        if (k == 0) {
            first = p;
        } else {
            float3 e = fabs(p - first);
            if (!(e.x < 1e-3 && e.y < 1e-3 && e.z < 1e-3)) {
                agree = false;
                break;
            }
        }
    }
    float3 sum = first;
    float n = 1.0;
    if (!agree) {
        sum = float3(0.0);
        n = float(SOFT_POINTS);
        for (uint k = 0; k < SOFT_POINTS; k++) {
            float r = diffusion * sqrt((float(k) + 0.5) / float(SOFT_POINTS));
            float ang = float(k) * 2.399963 + turn;
            float3 d = centre + a * (r * cos(ang)) + b * (r * sin(ang)) - q;
            float len = length(d);
            sum += pass_from(sh, own, q, d / max(len, 1e-9), parallel ? INFINITY : len);
        }
    }
    return 1.0 - dark * (1.0 - sum / n);
}

// ---- ambient occlusion ---------------------------------------------------------------------

#define OPEN_RAYS 32
#define OPEN_SEED 0x9E3779B9u

// how much of the sky round world point q (on plane `own`, or NONE, facing n) is open, as far as
// `reach`: the share of rays spread over the half sphere it faces (more toward its top, as light
// falls: a sunflower on the disc beneath, lifted; the pattern turned a little from point to
// point) that meet no model, each as much as the flat layers it passes let it through
static float open_sky(thread const Shadows &sh, uint own, float3 q, float3 n, float reach) {
    float3 side = fabs(n.x) < 0.9 ? float3(1.0, 0.0, 0.0) : float3(0.0, 1.0, 0.0);
    float3 a = normalize(cross(n, side));
    float3 b = cross(n, a);
    float turn = noise(whole(q.x * 7.0), whole(q.y * 7.0), as_type<uint>(int(q.z)) ^ OPEN_SEED) * 6.283185307179586;
    float sum = 0.0;
    for (uint k = 0; k < OPEN_RAYS; k++) {
        float r = sqrt((float(k) + 0.5) / float(OPEN_RAYS));
        float ang = float(k) * 2.399963 + turn;
        float3 dir = a * (r * cos(ang)) + b * (r * sin(ang)) + n * sqrt(max(1.0 - r * r, 0.0));
#if RAYS
        // a pixel off the surface, so it does not hide itself where it is
        ray rr(q, dir, 1.0, reach);
        intersector<> it;
        it.accept_any_intersection(true);
        if (it.intersect(rr, sh.everything).type != intersection_type::none) continue;
#endif
        float pass = 1.0;
        for (uint c = 0; c < sh.count; c++) {
            const device Caster &p = sh.casters[c];
            float facing = dot(dir, p.normal.xyz);
            if (p.img.w == own || fabs(facing) < 1e-12) continue;
            float t = dot(p.origin.xyz - q, p.normal.xyz) / facing;
            if (t <= 1e-6 || t >= reach) continue;
            float3 hit = apply(p.unworld, q + dir * t);
            pass *= 1.0 - clamp(bilinear(sh.texels, p.img, (hit.x - p.own.x) * p.own.z, (hit.y - p.own.y) * p.own.z).w, 0.0, 1.0);
        }
        sum += pass;
    }
    return sum / float(OPEN_RAYS);
}

// ---- reflections ---------------------------------------------------------------------------

#define MIRROR_RAYS 16
#define MIRROR_CONE 0.5
#define MIRROR_SEED 0x85EBCA6Bu

// A model's triangle as a reflection sees it (meshes::Face).

// What a mirror sees by: the lights (as many as there are), the environment, and the light
// bouncing onto each point it was gathered at, as much of it as is taken (0: none bounces).
struct Seen {
    const device Face *faces;
    constant Light *lights;
    uint count;
    Env env;
    const device float4 *light;
    float amount;
};

// a surface a reflection meets, lit simply: colour `rgb` facing n (toward where it is seen from)
// at p, taking `ambient` of the ambient lights and `diffuse` of the light falling on it (no
// shadows, no shine), and the light the environment lets fall on it; as it is when it takes no light
static float3 seen_in_mirror(thread const Seen &by, float3 rgb, float3 p, float3 n, float ambient, float diffuse, bool lit, float3 fell) {
    if (!lit || (by.count == 0 && by.env.count == 0)) return rgb;
    float3 amb = float3(0.0), dif = float3(0.0);
    for (uint li = 0; li < by.count; li++) {
        constant Light &l = by.lights[li];
        if (l.kind.x == AMBIENT) {
            amb += l.color.xyz;
            continue;
        }
        float k = reach(l, p);
        float far;
        float3 to = toward(l, p, far);
        float facing = dot(n, to);
        if (k <= 0.0 || facing <= 0.0) continue;
        dif += l.color.xyz * (facing * k);
    }
    float3 fall = by.env.count != 0 ? pano_at(by.env, 0, panorama_at(n, by.env.rotation)) : float3(0.0);
    return rgb * (ambient * amb + diffuse * dif + diffuse * 2.0 * (fall + fell));
}

#define MIRROR_DEPTH 3

// A surface a reflection meets: how far along the ray, how it looks, which way it faces (toward
// where the ray came from), how much it mirrors and how much less face on, the flat layer it is.
struct Met {
    bool found;
    float d;
    float3 lit;
    float3 n;
    float2 mirror;
    uint own;
};

// the nearest surface the ray from q (on plane `own`, or NONE) along `dir` meets, as a mirror sees it
static Met meet(thread const Shadows &sh, thread const Seen &by, uint own, float3 q, float3 dir) {
    Met m = { false, INFINITY, float3(0.0), float3(0.0), float2(0.0), NONE };
#if RAYS
    // the nearest model (a pixel off the surface, so it does not see itself where it is)
    ray r(q, dir, 1.0, INFINITY);
    intersector<triangle_data> it;
    intersection_result<triangle_data> h = it.intersect(r, sh.everything);
    if (h.type == intersection_type::triangle) {
        Face f = by.faces[h.primitive_id];
        float2 b = h.triangle_barycentric_coord;
        float3 w = float3(1.0 - b.x - b.y, b.x, b.y);
        float3 nn = f.normals[0].xyz * w.x + f.normals[1].xyz * w.y + f.normals[2].xyz * w.z;
        nn = length(nn) > 1e-12 ? normalize(nn) : -dir;
        if (dot(nn, dir) > 0.0) nn = -nn;
        float3 rgb = face_rgb(sh, f, w);
        float3 fell = float3(0.0);
        if (by.amount > 0.0) {
            for (uint j = 0; j < 3; j++) {
                if (f.probes[j] != NONE) fell += by.light[f.probes[j]].xyz * w[j];
            }
        }
        m = { true, h.distance, seen_in_mirror(by, rgb, q + dir * h.distance, nn, f.normals[0].w, f.normals[1].w, f.normals[2].w > 0.5, fell * by.amount) + f.glow.xyz, nn, float2(f.colours[0].w, f.colours[1].w), NONE };
    }
#endif
    // the nearest flat layer nearer still (where it is more there than not)
    for (uint c = 0; c < sh.count; c++) {
        const device Caster &p = sh.casters[c];
        float facing = dot(dir, p.normal.xyz);
        if (p.img.w == own || p.mat.w == 0.0 || fabs(facing) < 1e-12) continue;
        float s = dot(p.origin.xyz - q, p.normal.xyz) / facing;
        if (s <= 1e-6 || s >= m.d) continue;
        float3 hit = apply(p.unworld, q + dir * s);
        float4 t = bilinear(sh.texels, p.img, (hit.x - p.own.x) * p.own.z, (hit.y - p.own.y) * p.own.z);
        if (t.w < 0.5) continue;
        float3 nn = facing > 0.0 ? -p.normal.xyz : p.normal.xyz;
        float3 fell = (by.amount > 0.0 && p.map.w != 0) ? grid_at(by.light, p.map, p.cell, hit.x, hit.y) * by.amount : float3(0.0);
        if (p.caustic.w != 0) fell += grid_at(by.light, p.caustic, p.bent, hit.x, hit.y) * 0.5;
        m = { true, s, seen_in_mirror(by, t.xyz / t.w, q + dir * s, nn, p.mat.x, p.mat.y, p.mat.z > 0.5, fell), nn, p.mirror.xy, p.img.w };
    }
    return m;
}

// what the ray from q (on plane `own`, or NONE) along `dir` sees, as a mirror `sharpness` sharp
// sees it: the surface it meets and, where that mirrors too, what it mirrors, from mirror to
// mirror up to MIRROR_DEPTH deep; past the last met, the environment's light; false where the
// first meets nothing and there is no environment
static bool seen_along(thread const Shadows &sh, thread const Seen &by, uint own, float3 q, float3 dir, float sharpness, thread float3 &out) {
    float3 lits[MIRROR_DEPTH];
    float rs[MIRROR_DEPTH];
    uint count = 0;
    bool far_side = false;
    float3 beyond = float3(0.0);
    for (uint level = 0; level < MIRROR_DEPTH; level++) {
        Met m = meet(sh, by, own, q, dir);
        if (!m.found) {
            if (by.env.count != 0) {
                beyond = env_shine(by.env, dir, level == 0 ? 1.0 - sharpness : 0.0);
                far_side = true;
            }
            break;
        }
        float r = m.mirror.x * (1.0 - m.mirror.y * clamp(dot(m.n, -dir), 0.0, 1.0));
        lits[count] = m.lit;
        rs[count] = r;
        count++;
        if (r <= 0.0) break;
        q = q + dir * m.d;
        dir = dir - m.n * (2.0 * dot(dir, m.n));
        own = m.own;
    }
    // put together from the last met: each its own look and as much of what is beyond it as it
    // mirrors (nothing beyond: its own look alone)
    for (int i = int(count) - 1; i >= 0; i--) {
        beyond = (far_side && rs[i] > 0.0) ? lits[i] * (1.0 - rs[i]) + beyond * rs[i] : lits[i];
        far_side = true;
    }
    out = beyond;
    return far_side;
}

// what a surface at q (on plane `own`, or NONE) facing n mirrors, seen from `viewer`, as sharply
// as `sharpness` (1: one way; less: many ways in a cone round it, wider the less, the pattern
// turned a little from point to point), `none` where it meets nothing; put together
static float3 mirrored(thread const Shadows &sh, thread const Seen &by, uint own, float3 q, float3 n, float3 viewer, float sharpness, float3 none) {
    float3 r = n * (2.0 * dot(n, viewer)) - viewer;
    float3 seen;
    if (sharpness >= 1.0) {
        return (dot(r, n) > 0.0 && seen_along(sh, by, own, q, r, sharpness, seen)) ? seen : none;
    }
    float3 side = fabs(r.x) < 0.9 ? float3(1.0, 0.0, 0.0) : float3(0.0, 1.0, 0.0);
    float3 a = normalize(cross(r, side));
    float3 b = cross(r, a);
    float wide = tan((1.0 - sharpness) * MIRROR_CONE);
    float turn = noise(whole(q.x * 7.0), whole(q.y * 7.0), as_type<uint>(int(q.z)) ^ MIRROR_SEED) * 6.283185307179586;
    float3 sum = float3(0.0);
    for (uint k = 0; k < MIRROR_RAYS; k++) {
        float s = wide * sqrt((float(k) + 0.5) / float(MIRROR_RAYS));
        float ang = float(k) * 2.399963 + turn;
        float3 dir = normalize(r + a * (s * cos(ang)) + b * (s * sin(ang)));
        sum += (dot(dir, n) > 0.0 && seen_along(sh, by, own, q, dir, sharpness, seen)) ? seen : none;
    }
    return sum / float(MIRROR_RAYS);
}

// ---- light bouncing (bounce.rs) ------------------------------------------------------------

#define BOUNCE_RAYS 128
#define BOUNCE_SEED 0x27D4EB2Fu

// A point light is gathered at: where it is (w: the flat layer it is on, NONE: a model's) and
// which way it faces.
struct Probe {
    float4 at;
    float4 n;
};

struct BounceParams {
    uint4 a;         // points, lights, flat layers, the environment's levels it shines with
    float4 b;        // how much bounces, the environment's turn
};

// A surface the bouncing light meets, lit simply: colour `rgb` at `at` facing n (toward where the
// ray came from), taking `ambient` of the ambient lights, `diffuse` of the light falling on it
// (its shadows looked for along one ray to each light's middle), of the light the environment
// lets fall on it, and of what the bounce before brought it; as it is when it takes no light.
static float3 lit_simply(thread const Shadows &sh, thread const Seen &by, float amount, float3 rgb, float3 at, float3 n, float ambient, float diffuse, bool lit, uint own, float3 before) {
    if (!lit) return rgb;
    float3 amb = float3(0.0), dif = float3(0.0);
    for (uint li = 0; li < by.count; li++) {
        constant Light &l = by.lights[li];
        if (l.kind.x == AMBIENT) {
            amb += l.color.xyz;
            continue;
        }
        float k = reach(l, at);
        float far;
        float3 to = toward(l, at, far);
        float facing = dot(n, to);
        if (k <= 0.0 || facing <= 0.0) continue;
        float3 through = l.kind.z != 0 ? 1.0 - l.fall.y * (1.0 - pass_from(sh, own, at, to, far)) : float3(1.0);
        dif += l.color.xyz * (facing * k) * through;
    }
    float3 fall = by.env.count != 0 ? pano_at(by.env, 0, panorama_at(n, by.env.rotation)) : float3(0.0);
    return rgb * (ambient * amb + diffuse * dif + diffuse * 2.0 * (fall + amount * before));
}

// the light the ray from q (on plane `own`, or NONE) along `dir` brings back: the nearest model's
// or flat layer's (where it is more there than not), lit simply, else the environment's sky
static float3 bounced_from(thread const Shadows &sh, thread const Seen &by, float amount, uint own, float3 q, float3 dir, const device float4 *before) {
    float best = INFINITY;
    float3 c = float3(0.0);
    bool found = false;
#if RAYS
    ray r(q, dir, 1.0, INFINITY);
    intersector<triangle_data> it;
    intersection_result<triangle_data> h = it.intersect(r, sh.everything);
    if (h.type == intersection_type::triangle) {
        Face f = by.faces[h.primitive_id];
        float2 b = h.triangle_barycentric_coord;
        float3 w = float3(1.0 - b.x - b.y, b.x, b.y);
        float3 nn = f.normals[0].xyz * w.x + f.normals[1].xyz * w.y + f.normals[2].xyz * w.z;
        nn = length(nn) > 1e-12 ? normalize(nn) : -dir;
        if (dot(nn, dir) > 0.0) nn = -nn;
        float3 rgb = face_rgb(sh, f, w);
        float3 e = float3(0.0);
        for (uint j = 0; j < 3; j++) {
            if (f.probes[j] != NONE) e += before[f.probes[j]].xyz * w[j];
        }
        c = lit_simply(sh, by, amount, rgb, q + dir * h.distance, nn, f.normals[0].w, f.normals[1].w, f.normals[2].w > 0.5, NONE, e) + f.glow.xyz;
        best = h.distance;
        found = true;
    }
#endif
    for (uint k = 0; k < sh.count; k++) {
        const device Caster &p = sh.casters[k];
        float facing = dot(dir, p.normal.xyz);
        if (p.img.w == own || p.mat.w == 0.0 || fabs(facing) < 1e-12) continue;
        float s = dot(p.origin.xyz - q, p.normal.xyz) / facing;
        if (s <= 1e-6 || s >= best) continue;
        float3 hit = apply(p.unworld, q + dir * s);
        float4 t = bilinear(sh.texels, p.img, (hit.x - p.own.x) * p.own.z, (hit.y - p.own.y) * p.own.z);
        if (t.w < 0.5) continue;
        float3 nn = facing > 0.0 ? -p.normal.xyz : p.normal.xyz;
        float3 e = p.map.w != 0 ? grid_at(before, p.map, p.cell, hit.x, hit.y) : float3(0.0);
        c = lit_simply(sh, by, amount, t.xyz / t.w, q + dir * s, nn, p.mat.x, p.mat.y, p.mat.z > 0.5, p.img.w, e);
        best = s;
        found = true;
    }
    if (!found) c = by.env.count != 0 ? env_shine(by.env, dir, 1.0) : float3(0.0);
    return c;
}

// The light coming back to each point from all round: the mean along rays spread over the half
// sphere it faces (bounce::rays), `before` what the bounce before gathered.
kernel void gather(device float4 *out [[buffer(0)]],
                   const device Probe *probes [[buffer(1)]],
                   const device float4 *before [[buffer(2)]],
                   const device Caster *casters [[buffer(3)]],
                   const device float4 *shadow_px [[buffer(4)]],
                   const device Face *faces [[buffer(5)]],
                   constant Light *lights [[buffer(6)]],
                   const device float4 *env_px [[buffer(7)]],
                   const device uint4 *env_levels [[buffer(8)]],
                   constant BounceParams &params [[buffer(9)]],
#if RAYS
                   primitive_acceleration_structure models [[buffer(10)]],
                   primitive_acceleration_structure everything [[buffer(11)]],
                   const device uint *glass [[buffer(12)]],
                   const device Pic *pics [[buffer(13)]],
                   const device float4 *pic_texels [[buffer(14)]],
#endif
                   uint2 at [[thread_position_in_grid]]) {
    if (at.x >= params.a.x) return;
#if RAYS
    Shadows sh = { casters, params.a.z, shadow_px, models, everything, faces, glass, pics, pic_texels };
#else
    Shadows sh = { casters, params.a.z, shadow_px };
#endif
    Seen by = { faces, lights, params.a.y, { env_px, env_levels, params.a.w, params.b.y }, before, 0.0 };
    Probe p = probes[at.x];
    float3 q = p.at.xyz, n = p.n.xyz;
    uint own = as_type<uint>(p.at.w);
    float3 side = fabs(n.x) < 0.9 ? float3(1.0, 0.0, 0.0) : float3(0.0, 1.0, 0.0);
    float3 a = normalize(cross(n, side));
    float3 b = cross(n, a);
    float turn = noise(whole(q.x * 7.0), whole(q.y * 7.0), as_type<uint>(int(q.z)) ^ BOUNCE_SEED) * 6.283185307179586;
    float3 sum = float3(0.0);
    for (uint k = 0; k < BOUNCE_RAYS; k++) {
        float r = sqrt((float(k) + 0.5) / float(BOUNCE_RAYS));
        float ang = float(k) * 2.399963 + turn;
        float3 dir = a * (r * cos(ang)) + b * (r * sin(ang)) + n * sqrt(max(1.0 - r * r, 0.0));
        sum += bounced_from(sh, by, params.b.x, own, q, dir, before);
    }
    out[at.x] = float4(sum / float(BOUNCE_RAYS), 0.0);
}

// ---- glass ---------------------------------------------------------------------------------

#define GLASS_TURNS 4

// `d` bent going through a surface facing `n` (against it) into a stuff `eta` times as slow:
// false where it cannot go through (scene::refract)
static bool bend(float3 d, float3 n, float eta, thread float3 &out) {
    float c = dot(n, d);
    float k = 1.0 - eta * eta * (1.0 - c * c);
    if (k < 0.0) return false;
    out = normalize(d * eta - n * (eta * c + sqrt(k)));
    return true;
}

// what is seen through a model at q on its surface facing n, from `viewer`: the way in bent by
// index of refraction `ior`, on through it to where it goes out (bent again; reflected whole
// inside where it cannot, a few times at most), and what that way meets past it
static bool seen_through(thread const Shadows &sh, thread const Seen &by, float3 q, float3 n, float3 viewer, float ior, thread float3 &out) {
    float3 dir;
    if (!bend(-viewer, n, 1.0 / ior, dir)) return false;
    float3 at = q;
#if RAYS
    for (uint turn = 0; turn < GLASS_TURNS; turn++) {
        // where it goes out: the nearest surface inside
        ray r(at, dir, 0.5, INFINITY);
        intersector<triangle_data> it;
        intersection_result<triangle_data> h = it.intersect(r, sh.everything);
        if (h.type != intersection_type::triangle) break;
        Face f = by.faces[h.primitive_id];
        float2 b = h.triangle_barycentric_coord;
        float3 w = float3(1.0 - b.x - b.y, b.x, b.y);
        float3 nn = f.normals[0].xyz * w.x + f.normals[1].xyz * w.y + f.normals[2].xyz * w.z;
        nn = length(nn) > 1e-12 ? normalize(nn) : -dir;
        at = at + dir * h.distance;
        float3 inward = dot(nn, dir) > 0.0 ? -nn : nn;
        float3 leaving;
        if (bend(dir, inward, ior, leaving)) return seen_along(sh, by, NONE, at, leaving, 1.0, out);
        dir = dir - inward * (2.0 * dot(dir, inward));
    }
#endif
    return seen_along(sh, by, NONE, at, dir, 1.0, out);
}
