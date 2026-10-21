// VR Fractal Noise on the GPU (after common.metal and noise.metal), as sphere.rs makes it: fractal
// noise at each pixel's direction on the sphere. The directions' sines and cosines (each column's
// longitude and each row's latitude, for each eye) and each layer's size, place and evolution are
// worked out on the processor.

struct VrLayer {
    ulong seed;
    df fine;
    df at_k[3];
    df at_next[3];
    float t;
    float before;
    float inv_den;
    float strength;
    float part;
    float pad;
};

constant constexpr uint MOST_LAYERS = 20;

struct Vr {
    uint w;
    uint h;
    uint eyes;
    uint kind;
    uint mode;
    uint invert;
    uint count;
    uint pad;
    uint4 eye[2];
    uint4 at;
    float contrast;
    float brightness;
    float opacity;
    float weights;
    df m[9];
    VrLayer layer[MOST_LAYERS];
};

kernel void vr_fractal(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], const device float4 *trig [[buffer(2)]], constant Vr &p [[buffer(3)]], uint2 g [[thread_position_in_grid]]) {
    // the eye the pixel is in, and where in it
    uint e = (p.eyes > 1 && (g.x >= p.eye[1].x && g.y >= p.eye[1].y)) ? 1 : 0;
    uint x = g.x - p.eye[e].x, y = g.y - p.eye[e].y;
    float4 lon = trig[(e == 0 ? p.at.x : p.at.z) + x];
    float4 lat = trig[(e == 0 ? p.at.y : p.at.w) + y];
    df sl = df{lon.x, lon.y}, cl = df{lon.z, lon.w}, sa = df{lat.x, lat.y}, ca = df{lat.z, lat.w};
    df d0 = df_mul(ca, sl), d1 = sa, d2 = df_mul(ca, cl);
    // turned as the view is
    df t[3];
    for (int i = 0; i < 3; i++) {
        t[i] = df_add(df_add(df_mul(p.m[i * 3], d0), df_mul(p.m[i * 3 + 1], d1)), df_mul(p.m[i * 3 + 2], d2));
    }
    float sum = 0.0, most = 0.0;
    for (uint i = 0; i < p.count; i++) {
        VrLayer l = p.layer[i];
        df q0 = df_mul(t[0], l.fine), q1 = df_mul(t[1], l.fine), q2 = df_mul(t[2], l.fine);
        // between two unrelated fields as the evolution goes on, kept as strong in between
        float f1 = smooth3(df_add(q0, l.at_k[0]), df_add(q1, l.at_k[1]), df_add(q2, l.at_k[2]), l.seed);
        float f2 = smooth3(df_add(q0, l.at_next[0]), df_add(q1, l.at_next[1]), df_add(q2, l.at_next[2]), l.seed);
        float n = (f1 * l.before + f2 * l.t) * l.inv_den;
        float s = clamp(n, -1.0, 1.0);
        float v;
        switch (p.kind) {
            case 1: { float k = 1.0 - abs(s); v = k * k; break; }
            case 3: { float k = clamp(abs(s) / 0.12, 0.0, 1.0); v = 1.0 - k * k; break; }
            default: v = s * 0.5 + 0.5; break;
        }
        most = max(most, v * l.strength);
        sum += v * l.strength * l.part;
    }
    float v = p.kind == 2 ? most : sum / max(p.weights, 1e-9);
    v = (v - 0.5) * p.contrast + 0.5 + p.brightness;
    if (p.invert) {
        v = 1.0 - v;
    }
    v = clamp(v, 0.0, 1.0);
    // alone it fills the layer's shape; blended it covers all of the layer
    float4 px = src[g.y * p.w + g.x];
    float a = p.opacity * (p.mode == 0 ? px.w : 1.0);
    float4 grey = float4(v * a, v * a, v * a, a);
    out[g.y * p.w + g.x] = p.mode == 0 ? grey : blend(p.mode - 1, px, grey, 0.5);
}
