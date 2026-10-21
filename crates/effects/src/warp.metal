// Turbulent Displace on the GPU (after common.metal and noise.metal), as warp.rs makes it: each
// pixel takes the picture from where the noise pushes it (a whole number of pixels and the part
// of one past them, so it lands where the processor's does).

struct Turbulent {
    uint w;
    uint h;
    uint kind;
    uint pin;
    uint locked;
    uint pad;
    df off_x;
    df off_y;
    df inv_size;
    df lx;
    df ly;
    float lw;
    float lh;
    float edge;
    float amount;
    df e;
    ulong seed;
    Depth z;
};

kernel void turbulent_displace(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant Octaves &oct [[buffer(2)]], constant Turbulent &p [[buffer(3)]], uint2 g [[thread_position_in_grid]]) {
    // where the pixel's middle is in the noise
    df u = df_mul(df_sub(df_of(float(g.x) + 0.5), p.off_x), p.inv_size);
    df v = df_mul(df_sub(df_of(float(g.y) + 0.5), p.off_y), p.inv_size);
    #define N(a, b, k) fractal_noise(a, b, p.z, p.seed ^ (k), oct)
    float dx, dy;
    switch (p.kind) {
        case 1: case 2: case 4: case 5: {
            // along the slope of the noise (out of its hollows) or round it
            df e = p.e;
            float gx = (N(df_add(u, e), v, 0ul) - N(df_sub(u, e), v, 0ul)) / 0.1;
            float gy = (N(u, df_add(v, e), 0ul) - N(u, df_sub(v, e), 0ul)) / 0.1;
            float k = p.amount * 0.125;
            if (p.kind == 1 || p.kind == 4) {
                dx = gx * k;
                dy = gy * k;
            } else {
                dx = -gy * k;
                dy = gx * k;
            }
            break;
        }
        case 6: dx = 0.0; dy = N(u, v, 17ul) * p.amount * 0.5; break;
        case 7: dx = N(u, v, 0ul) * p.amount * 0.5; dy = 0.0; break;
        case 8: dx = N(u, v, 0ul) * p.amount * 0.5; dy = dx; break;
        default: dx = N(u, v, 0ul) * p.amount * 0.5; dy = N(u, v, 17ul) * p.amount * 0.5; break;
    }
    #undef N
    // how far the pixel's middle is in from the layer's outermost pixels' middles
    float fx = df_float(df_sub(df_of(float(g.x)), p.lx));
    float fy = df_float(df_sub(df_of(float(g.y)), p.ly));
    float l = clamp(fx / p.edge, 0.0, 1.0), r = clamp((p.lw - 1.0 - fx) / p.edge, 0.0, 1.0);
    float t = clamp(fy / p.edge, 0.0, 1.0), bt = clamp((p.lh - 1.0 - fy) / p.edge, 0.0, 1.0);
    float free = 1.0;
    switch (p.pin) {
        case 1: free = min(min(l, r), min(t, bt)); break;
        case 2: free = min(t, bt); break;
        case 3: free = min(l, r); break;
        case 4: free = l; break;
        case 5: free = t; break;
        case 6: free = r; break;
        case 7: free = bt; break;
        default: break;
    }
    float ox = dx * free, oy = dy * free;
    if (p.locked) {
        ox = clamp(ox, -fx, p.lw - 1.0 - fx);
        oy = clamp(oy, -fy, p.lh - 1.0 - fy);
    }
    float flx = floor(ox), fly = floor(oy);
    out[g.y * p.w + g.x] = between(src, p.w, p.h, int(g.x) + int(flx), int(g.y) + int(fly), float2(ox - flx, oy - fly));
}
