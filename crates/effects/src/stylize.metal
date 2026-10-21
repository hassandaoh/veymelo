// Roughen Edges on the GPU (after common.metal and noise.metal), as stylize.rs makes it: the
// alpha softened over the border (by the raster's blur), then each pixel's edge worn by the noise.

// The alpha in every channel.
kernel void alpha_of(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant uint &w [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    out[g.y * w + g.x] = float4(src[g.y * w + g.x].w);
}

struct Roughen {
    uint w;
    uint h;
    uint coloured;
    uint pad;
    df off_x;
    df off_y;
    df kx;
    df ky;
    float influence;
    float strong;
    float sharp;
    float edge_r;
    float edge_g;
    float edge_b;
    ulong seed;
    Depth z;
};

inline float smooth_step(float e0, float e1, float x) {
    float t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

kernel void roughen(device float4 *out [[buffer(0)]], const device float4 *depth [[buffer(1)]], constant Octaves &oct [[buffer(2)]], const device float4 *src [[buffer(3)]], constant Roughen &p [[buffer(4)]], uint2 g [[thread_position_in_grid]]) {
    uint i = g.y * p.w + g.x;
    float4 px = src[i];
    if (px.w <= 0.0) {
        out[i] = px;
        return;
    }
    float dv = depth[i].x;
    df u = df_mul(df_sub(df_of(float(g.x) + 0.5), p.off_x), p.kx);
    df v = df_mul(df_sub(df_of(float(g.y) + 0.5), p.off_y), p.ky);
    float n = fractal_noise(u, v, p.z, p.seed, oct) * 0.5;
    float s = dv - 0.5 + n * p.influence * p.strong * 0.5;
    float m = clamp(s * p.sharp * 4.0 + 0.5, 0.0, 1.0);
    float4 c = unpremul(px);
    float3 col = c.xyz;
    if (p.coloured) {
        float t = 1.0 - smooth_step(0.5, 0.9, dv + n * p.influence * 0.5);
        col = col + (float3(p.edge_r, p.edge_g, p.edge_b) - col) * t;
    }
    float a = c.w * m;
    out[i] = float4(col * a, a);
}
