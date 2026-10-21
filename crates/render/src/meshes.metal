// A 3D model filled on the GPU: what `meshes::fill` does on the processor, one thread a canvas
// pixel of the part the model reaches. Its triangles are sorted (on the processor) into tiles of
// 32 × 8 pixels, each tile's in the model's order; a pixel looks at its tile's at four samples (two
// across, two down), keeping the nearest at each, then colours each triangle it kept once, at the
// pixel's middle, and says what the surface nearest there is. Every step follows the processor's,
// in the same order, so the two draw the same pictures (to within the last bits of a float).
//
// Built after light.metal (the lights, and an environment's) and shadows.metal (the shadows
// falling on it).

#define TILE_W 32
#define TILE_H 8

struct Vert {
    float4 screen;   // on the canvas, from the shot's corner: x, y; 1 / its depth
    float4 world;
    float4 normal;
    float4 tangent;  // xyz, which way its bitangent goes
    float4 uv;       // the first texture point, the second
    float4 color;
    float4 bounce;   // the light bouncing onto it
};

struct Tri {
    uint4 v;         // its corners, its material
    float4 reach;    // the part of the canvas it reaches (from the shot's corner): x0, y0, x1, y1
};

struct Mat {
    float4 base;     // linear rgba
    float4 emissive; // linear rgb; the normal map's scale
    float4 pbr;      // metallic, roughness, occlusion strength, alpha cutoff
    uint4 maps;      // its pictures (NONE: none): colour, metal-roughness, normal, occlusion
    uint4 more;      // the picture of the light it gives off; which texture points each reads (a
                     // bit each, in that order, the light's last); alpha (0 opaque, 1 mask, 2 see-through); unlit
};

struct Pic {
    uint4 a;         // first texel, width, height; read between its pixels (bit 0), wrap across (bits 1-2) and down (bits 3-4)
};

struct MeshParams {
    uint4 a;         // columns, rows, tiles across, lights
    uint4 b;         // colours worked in linear light, takes light, the environment's levels it shines with (0: none), takes shadows
    float4 eye;      // xyz; the environment's turn
    float4 layer;    // the layer's ambient, diffuse, specular, light transmission
    uint4 c;         // flat layers (rays meet them all; some cast shadows)
    float4 d;        // the environment's occlusion; how far round a point the ambient occlusion looks (0: none occludes); how much light bounces onto it; how much bounces in the scene
    float4 e;        // how much the layer mirrors, how sharply, how much less face on
    float4 f;        // how much it is seen through, how much less edge on, its index of refraction
};

struct Mesh {
    const device Vert *verts;
    const device Tri *tris;
    constant Mat *mats;
    constant Pic *pics;
    const device float4 *texels;
    constant Light *lights;
    constant MeshParams *params;
    Env env;
    Shadows shadows;
    const device Face *faces;
    const device float4 *light;    // the light bouncing onto each point it was gathered at
};

// ---- pictures ------------------------------------------------------------------------------

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

static float4 texel(thread const Mesh &s, Pic p, float x, float y) {
    // (held to what a whole number holds: far past the edges is as far as can be)
    int ix = int(clamp(x, -1e9, 1e9));
    int iy = int(clamp(y, -1e9, 1e9));
    int w = int(p.a.y), h = int(p.a.z);
    return s.texels[p.a.x + fold(iy, h, (p.a.w >> 3) & 3u) * uint(w) + fold(ix, w, (p.a.w >> 1) & 3u)];
}

// picture `which` at its texture point set `set` of `uv`: none, false
static bool picture(thread const Mesh &s, uint which, uint set, float4 uv, thread float4 &out) {
    if (which == NONE) return false;
    Pic p = s.pics[which];
    if (p.a.y == 0 || p.a.z == 0) {
        out = float4(1.0);
        return true;
    }
    float2 at = set == 0 ? uv.xy : uv.zw;
    float x = at.x * float(p.a.y) - 0.5;
    float y = at.y * float(p.a.z) - 0.5;
    if ((p.a.w & 1u) == 0) {
        out = texel(s, p, round(x), round(y));
        return true;
    }
    float x0 = floor(x), y0 = floor(y);
    float tx = x - x0, ty = y - y0;
    float4 a = texel(s, p, x0, y0), b = texel(s, p, x0 + 1.0, y0);
    float4 c = texel(s, p, x0, y0 + 1.0), d = texel(s, p, x0 + 1.0, y0 + 1.0);
    float4 top = a + (b - a) * tx;
    float4 bottom = c + (d - c) * tx;
    out = top + (bottom - top) * ty;
    return true;
}

// ---- colours -------------------------------------------------------------------------------

static float srgb_to_linear(float v) {
    return v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4);
}

static float linear_to_srgb(float v) {
    return v <= 0.0031308 ? v * 12.92 : 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

// a linear colour in the working space
static float3 to_work(thread const Mesh &s, float3 c) {
    if (s.params->b.x != 0) return c;
    return float3(linear_to_srgb(max(c.x, 0.0)), linear_to_srgb(max(c.y, 0.0)), linear_to_srgb(max(c.z, 0.0)));
}

// a picture's (sRGB) colour in the working space
static float3 pic_to_work(thread const Mesh &s, float3 c) {
    if (s.params->b.x == 0) return c;
    return float3(srgb_to_linear(max(c.x, 0.0)), srgb_to_linear(max(c.y, 0.0)), srgb_to_linear(max(c.z, 0.0)));
}

// ---- a triangle ----------------------------------------------------------------------------

static float edge(float2 a, float2 b, float x, float y) {
    return (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x);
}

static float area_of(float2 a, float2 b, float2 c) {
    return (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
}

// a triangle's weights at canvas point (x, y), held inside it, as perspective has them
static float3 inside(Vert a, Vert b, Vert c, float x, float y) {
    float2 s0 = a.screen.xy, s1 = b.screen.xy, s2 = c.screen.xy;
    float area = area_of(s0, s1, s2);
    float3 raw = max(float3(edge(s1, s2, x, y) / area, edge(s2, s0, x, y) / area, edge(s0, s1, x, y) / area), 0.0);
    float3 sb = raw / max(raw.x + raw.y + raw.z, 1e-12);
    float3 iz = float3(a.screen.z, b.screen.z, c.screen.z);
    float z = sb.x * iz.x + sb.y * iz.y + sb.z * iz.z;
    return sb * iz / z;
}

// what a point of triangle `t` (at weights `w` of its corners) looks like, lit: premultiplied
static float4 shade(thread const Mesh &s, Tri t, float3 w) {
    Vert a = s.verts[t.v.x], b = s.verts[t.v.y], c = s.verts[t.v.z];
    float3 world = a.world.xyz * w.x + b.world.xyz * w.y + c.world.xyz * w.z;
    float3 n = a.normal.xyz * w.x + b.normal.xyz * w.y + c.normal.xyz * w.z;
    n = length(n) > 1e-12 ? normalize(n) : float3(0.0, 0.0, -1.0);
    float4 uv = a.uv * w.x + b.uv * w.y + c.uv * w.z;
    float4 vcolor = a.color * w.x + b.color * w.y + c.color * w.z;
    Mat m = s.mats[t.v.w];
    uint sets = m.more.y;
    float3 factor = to_work(s, m.base.xyz * vcolor.xyz);
    float4 tex = float4(1.0);
    picture(s, m.maps.x, sets & 1u, uv, tex);
    float3 rgb = factor * pic_to_work(s, tex.xyz);
    float alpha = m.more.z == 0 ? 1.0 : clamp(m.base.w * vcolor.w * tex.w, 0.0, 1.0);
    float3 glow = float3(1.0);
    float4 g;
    if (picture(s, m.more.x, (sets >> 4) & 1u, uv, g)) glow = pic_to_work(s, g.xyz);
    float3 emissive = to_work(s, m.emissive.xyz) * glow;
    if (m.more.w != 0) return float4(rgb * alpha, alpha);
    // the normal map turns the normal within the surface
    float4 nt;
    if (picture(s, m.maps.z, (sets >> 2) & 1u, uv, nt)) {
        float4 tan = a.tangent * w.x + b.tangent * w.y + c.tangent * w.z;
        float3 tv = normalize(tan.xyz - n * dot(tan.xyz, n));
        float3 bv = cross(n, tv) * (tan.w < 0.0 ? -1.0 : 1.0);
        float k = m.emissive.w;
        float3 turned = tv * ((nt.x * 2.0 - 1.0) * k) + bv * ((nt.y * 2.0 - 1.0) * k) + n * (nt.z * 2.0 - 1.0);
        if (length(turned) > 1e-9) n = normalize(turned);
    }
    float4 mr = float4(1.0);
    picture(s, m.maps.y, (sets >> 1) & 1u, uv, mr);
    float metal = clamp(m.pbr.x * mr.z, 0.0, 1.0);
    float rough = clamp(m.pbr.y * mr.y, 0.02, 1.0);
    float occlusion = 1.0;
    float4 o;
    if (picture(s, m.maps.w, (sets >> 3) & 1u, uv, o)) occlusion = 1.0 + m.pbr.z * (o.x - 1.0);
    // the metal-roughness material as the layer's light-taking one (scene::shade, no shadows),
    // and the environment's light as its metal and roughness take it
    float3 lit = rgb;
    uint nl = s.params->a.w;
    bool env = s.env.count != 0;
    float3 viewer = normalize(s.params->eye.xyz - world);
    // how open the sky round it is (on the side it is seen from)
    float looks = s.params->d.y;
    float open = (looks > 0.0 && s.params->b.y != 0 && s.params->b.w != 0) ? open_sky(s.shadows, NONE, world, dot(n, viewer) < 0.0 ? -n : n, looks) : 1.0;
    if (env && nl == 0 && s.params->b.y != 0) lit = float3(0.0);
    if (nl != 0 && s.params->b.y != 0) {
        float4 layer = s.params->layer;
        float ambient_k = layer.x * occlusion;
        float diffuse_k = layer.y * (1.0 - 0.9 * metal);
        float specular_k = layer.z * (1.0 - rough) * (1.0 - rough) * 2.0;
        float exponent = max(pow(256.0, clamp(1.0 - rough, 0.0, 1.0)), 1.0);
        float3 facing_n = dot(n, viewer) < 0.0 ? -n : n;
        float3 ambient = float3(0.0), diffuse = float3(0.0), shine = float3(0.0);
        for (uint li = 0; li < nl; li++) {
            constant Light &l = s.lights[li];
            if (l.kind.x == AMBIENT) {
                ambient += l.color.xyz * (1.0 - l.more.x * (1.0 - open));
                continue;
            }
            float k = reach(l, world);
            if (k <= 0.0) continue;
            float far;
            float3 to = toward(l, world, far);
            float facing = dot(facing_n, to);
            float lambert = facing > 0.0 ? facing : -facing * layer.w;
            if (lambert <= 0.0) continue;
            float3 through = (l.kind.z != 0 && s.params->b.w != 0) ? unshadowed(s.shadows, l, NONE, world) : float3(1.0);
            diffuse += l.color.xyz * (lambert * k) * through;
            if (facing > 0.0 && specular_k > 0.0) {
                float3 half_way = normalize(to + viewer);
                float sh = pow(max(dot(facing_n, half_way), 0.0), exponent) * k;
                shine += l.color.xyz * sh * through * (1.0 + (rgb - 1.0) * metal);
            }
        }
        lit = rgb * (ambient_k * ambient + diffuse_k * diffuse) + specular_k * shine;
    }
    if (env && s.params->b.y != 0) {
        float4 layer = s.params->layer;
        lit += env_light(s.env, rgb, n, viewer, metal, rough, layer.y * 2.0, layer.z * 2.0, occlusion * (1.0 - s.params->d.x * (1.0 - open)));
    }
    // the light bouncing onto it from the surfaces round it
    if (s.params->d.z > 0.0 && s.params->b.y != 0) {
        float3 e = a.bounce.xyz * w.x + b.bounce.xyz * w.y + c.bounce.xyz * w.z;
        lit += rgb * (s.params->layer.y * (1.0 - 0.9 * metal) * 2.0 * s.params->d.z) * e;
    }
    float3 col = lit + emissive;
    // what it mirrors, mixed in
    float4 mirror = s.params->e;
    if (mirror.x > 0.0) {
        float3 facing = dot(n, viewer) < 0.0 ? -n : n;
        float r = mirror.x * (1.0 - mirror.z * clamp(dot(facing, viewer), 0.0, 1.0));
        if (r > 0.0) {
            Seen by = { s.faces, s.lights, nl, s.env, s.light, s.params->d.w };
            float3 seen = mirrored(s.shadows, by, NONE, world, facing, viewer, mirror.y, col);
            col = col * (1.0 - r) + seen * r;
        }
    }
    // seen through, as glass: what is behind it bent and tinted by it (nothing met past it: what
    // is under it shows through)
    float4 glass = s.params->f;
    if (glass.x > 0.0) {
        float3 facing = dot(n, viewer) < 0.0 ? -n : n;
        float t = glass.x * (1.0 - glass.y * (1.0 - clamp(dot(facing, viewer), 0.0, 1.0)));
        if (t > 0.0) {
            Seen by = { s.faces, s.lights, nl, s.env, s.light, s.params->d.w };
            float3 seen;
            if (seen_through(s.shadows, by, world, facing, viewer, glass.z, seen)) {
                col = col * (1.0 - t) + seen * rgb * t;
            } else {
                alpha *= 1.0 - t;
            }
        }
    }
    return float4(col * alpha, alpha);
}

// ---- a pixel -------------------------------------------------------------------------------

kernel void mesh(device float4 *color [[buffer(0)]],
                 device float4 *nearest [[buffer(1)]],
                 device float4 *surface [[buffer(2)]],
                 const device Vert *verts [[buffer(3)]],
                 const device Tri *tris [[buffer(4)]],
                 const device uint *starts [[buffer(5)]],
                 const device uint *listed [[buffer(6)]],
                 constant Mat *mats [[buffer(7)]],
                 constant Pic *pics [[buffer(8)]],
                 const device float4 *texels [[buffer(9)]],
                 constant Light *lights [[buffer(10)]],
                 constant MeshParams &params [[buffer(11)]],
                 const device float4 *env_px [[buffer(12)]],
                 const device uint4 *env_levels [[buffer(13)]],
                 const device Caster *casters [[buffer(14)]],
                 const device float4 *shadow_px [[buffer(15)]],
                 const device Face *faces [[buffer(16)]],
                 const device float4 *light [[buffer(17)]],
#if RAYS
                 primitive_acceleration_structure models [[buffer(18)]],
                 primitive_acceleration_structure everything [[buffer(19)]],
#endif
                 uint2 at [[thread_position_in_grid]]) {
#if RAYS
    Shadows shadows = { casters, params.c.x, shadow_px, models, everything };
#else
    Shadows shadows = { casters, params.c.x, shadow_px };
#endif
    Mesh s = { verts, tris, mats, pics, texels, lights, &params, { env_px, env_levels, params.b.z, params.eye.w }, shadows, faces, light };
    uint w = params.a.x;
    if (at.x >= w || at.y >= params.a.y) return;
    float x = float(at.x), y = float(at.y);
    uint tile = (at.y / TILE_H) * params.a.z + at.x / TILE_W;
    // the nearest triangle at each of the four samples, and its depth
    float depth[4] = { INFINITY, INFINITY, INFINITY, INFINITY };
    uint who[4] = { NONE, NONE, NONE, NONE };
    for (uint k = starts[tile]; k < starts[tile + 1]; k++) {
        uint id = listed[k];
        Tri t = tris[id];
        if (x + 0.75 < t.reach.x || x + 0.25 > t.reach.z || y + 0.75 < t.reach.y || y + 0.25 > t.reach.w) continue;
        Vert a = verts[t.v.x], b = verts[t.v.y], c = verts[t.v.z];
        float2 s0 = a.screen.xy, s1 = b.screen.xy, s2 = c.screen.xy;
        float area = area_of(s0, s1, s2);
        if (fabs(area) < 1e-12) continue;
        Mat m = mats[t.v.w];
        for (uint n = 0; n < 4; n++) {
            float py = y + (float(n >> 1) + 0.5) / 2.0;
            float px = x + (float(n & 1u) + 0.5) / 2.0;
            float w0 = edge(s1, s2, px, py) / area, w1 = edge(s2, s0, px, py) / area, w2 = edge(s0, s1, px, py) / area;
            if (w0 < 0.0 || w1 < 0.0 || w2 < 0.0) continue;
            float iz = w0 * a.screen.z + w1 * b.screen.z + w2 * c.screen.z;
            if (iz <= 0.0) continue;
            float z = 1.0 / iz;
            if (z >= depth[n]) continue;
            if (m.more.z == 1) {
                // a mask: shown where its alpha is at least the cutoff
                float3 bw = float3(w0 * a.screen.z, w1 * b.screen.z, w2 * c.screen.z) / iz;
                float4 uv = a.uv * bw.x + b.uv * bw.y + c.uv * bw.z;
                float4 p = float4(1.0);
                picture(s, m.maps.x, m.more.y & 1u, uv, p);
                if (m.base.w * p.w < m.pbr.w) continue;
            }
            depth[n] = z;
            who[n] = id;
        }
    }
    // the four samples, each triangle's colour worked out once
    float4 sum = float4(0.0);
    float near = INFINITY;
    uint closest = NONE;
    uint done_id[4] = { NONE, NONE, NONE, NONE };
    float4 done[4];
    float cx = x + 0.5, cy = y + 0.5;
    for (uint n = 0; n < 4; n++) {
        uint id = who[n];
        if (id == NONE) continue;
        if (depth[n] < near) {
            near = depth[n];
            closest = id;
        }
        float4 col = float4(0.0);
        bool found = false;
        for (uint k = 0; k < 4; k++) {
            if (done_id[k] == id) {
                col = done[k];
                found = true;
                break;
            }
        }
        if (!found) {
            Tri t = tris[id];
            col = shade(s, t, inside(verts[t.v.x], verts[t.v.y], verts[t.v.z], cx, cy));
            done_id[n] = id;
            done[n] = col;
        }
        sum += col * 0.25;
    }
    uint i = at.y * w + at.x;
    color[i] = sum;
    if (closest == NONE) {
        nearest[i] = float4(INFINITY, 0.0, 0.0, -1.0);
        surface[i] = float4(0.0, 0.0, as_type<float>(NONE), 0.0);
        return;
    }
    // what the surface nearest there is
    Tri t = tris[closest];
    Vert a = verts[t.v.x], b = verts[t.v.y], c = verts[t.v.z];
    float3 bw = inside(a, b, c, cx, cy);
    float3 n = a.normal.xyz * bw.x + b.normal.xyz * bw.y + c.normal.xyz * bw.z;
    n = length(n) > 1e-12 ? normalize(n) : float3(0.0, 0.0, -1.0);
    float2 uv = a.uv.xy * bw.x + b.uv.xy * bw.y + c.uv.xy * bw.z;
    nearest[i] = float4(near, n);
    surface[i] = float4(uv, as_type<float>(t.v.w), 0.0);
}
