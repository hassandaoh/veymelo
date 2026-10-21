// Blurs of the effects on the GPU (after common.metal), as blur.rs makes them.

struct Fast {
    uint w;
    uint h;
    uint mode;
    uint pad;
    df cx;
    df cy;
    df s;
    float sf;
};

// Radial Fast Blur: each pixel the mean (or the brightest, or the darkest) of the picture along
// the line from it toward the centre, as many samples as the farthest moves pixels (counted from
// the centre and the pixel held exactly, so it is the processor's count).
kernel void radial_fast(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant Fast &p [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    df dvx = df_sub(df_of(float(g.x) + 0.5), p.cx);
    df dvy = df_sub(df_of(float(g.y) + 0.5), p.cy);
    long far = df_ceil(df_mul(df_sqrt(df_add(df_mul(dvx, dvx), df_mul(dvy, dvy))), p.s));
    uint n = uint(clamp(far + 1, 2l, 256l));
    float vx = df_float(dvx), vy = df_float(dvy);
    float4 acc = p.mode == 1 ? float4(-FLT_MAX) : p.mode == 2 ? float4(FLT_MAX) : float4(0.0);
    for (uint i = 0; i < n; i++) {
        // toward the centre: the pixel's offset from it shrunk by s·i/(n−1)
        float t = p.sf * float(i) / float(n - 1);
        float ox = -vx * t, oy = -vy * t;
        float flx = floor(ox), fly = floor(oy);
        float4 q = between(src, p.w, p.h, int(g.x) + int(flx), int(g.y) + int(fly), float2(ox - flx, oy - fly));
        acc = p.mode == 1 ? max(acc, q) : p.mode == 2 ? min(acc, q) : acc + q / float(n);
    }
    float a = clamp(acc.w, 0.0, 1.0);
    // the brightest or darkest of each channel may come from different pixels: kept within the alpha
    out[g.y * p.w + g.x] = p.mode == 0 ? float4(max(acc.xyz, 0.0), a) : float4(clamp(acc.xyz, 0.0, a), a);
}

struct Zoom {
    uint w;
    uint h;
    uint kind;
    uint pad;
    df cx;
    df cy;
    df k;
    float zoom;
    float arc;
};

// Zoom Blur: each pixel the weighed mean of the picture toward the centre (evenly, fading along
// the streak, both ways about the pixel) or turned round it (evenly, fading to the arc's ends),
// as many samples as the streak is long (counted from the centre and the pixel held exactly).
kernel void zoom_blur(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant Zoom &p [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    uint i0 = g.y * p.w + g.x;
    df dvx = df_sub(df_of(float(g.x) + 0.5), p.cx);
    df dvy = df_sub(df_of(float(g.y) + 0.5), p.cy);
    long far = df_ceil(df_mul(df_sqrt(df_add(df_mul(dvx, dvx), df_mul(dvy, dvy))), p.k));
    uint n = uint(clamp(far, 1l, 256l));
    if (n <= 1) {
        out[i0] = src[i0];
        return;
    }
    float dx = df_float(dvx), dy = df_float(dvy);
    float4 acc = float4(0.0);
    float sum = 0.0;
    for (uint i = 0; i < n; i++) {
        float t = float(i) / float(n - 1);
        float ox, oy, w = 1.0;
        if (p.kind <= 1) {
            ox = -dx * (p.zoom * t);
            oy = -dy * (p.zoom * t);
            if (p.kind == 1) {
                w = (1.0 - t) + 0.05;
            }
        } else if (p.kind == 2) {
            ox = dx * (p.zoom * (t - 0.5));
            oy = dy * (p.zoom * (t - 0.5));
        } else {
            float a = p.arc * (t - 0.5);
            float s = precise::sin(a), h = precise::sin(a * 0.5);
            float c1 = -2.0 * h * h;
            ox = dx * c1 - dy * s;
            oy = dx * s + dy * c1;
            if (p.kind == 4) {
                w = (1.0 - abs(2.0 * t - 1.0)) + 0.05;
            }
        }
        float flx = floor(ox), fly = floor(oy);
        acc += between(src, p.w, p.h, int(g.x) + int(flx), int(g.y) + int(fly), float2(ox - flx, oy - fly)) * w;
        sum += w;
    }
    out[i0] = acc / sum;
}

struct Keeping {
    uint w;
    uint h;
    uint n;
    uint hard;
    uint grey;
    float range;
    float like2;
    float pad;
};

// A blur that keeps edges (Bilateral and Smart Blur): each pixel the mean of its neighbours
// (each a step across and down, and how near it is, worked out on the processor), each weighed
// by how near it is and how like the pixel it is (or, hard: whether its brightness is within the
// range). Grey: the result's brightness alone.
kernel void keeping_edges(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], const device float4 *near [[buffer(2)]], constant Keeping &p [[buffer(3)]], uint2 g [[thread_position_in_grid]]) {
    uint i0 = g.y * p.w + g.x;
    float4 c = src[i0];
    float cl = luminance(c.xyz);
    float4 acc = float4(0.0);
    float total = 0.0;
    for (uint k = 0; k < p.n; k++) {
        float4 s = near[k];
        int x = clamp(int(g.x) + int(s.x), 0, int(p.w) - 1), y = clamp(int(g.y) + int(s.y), 0, int(p.h) - 1);
        float4 q = src[uint(y) * p.w + uint(x)];
        float wk;
        if (p.hard) {
            wk = abs(luminance(q.xyz) - cl) <= p.range ? 1.0 : 0.0;
        } else {
            float4 d = q - c;
            float diff = d.x * d.x + d.y * d.y + d.z * d.z + d.w * d.w;
            wk = precise::exp(s.z - diff / p.like2);
        }
        if (wk > 0.0) {
            acc += q * wk;
            total += wk;
        }
    }
    float4 o = total > 0.0 ? acc / total : c;
    if (p.grey) {
        float4 u = unpremul(o);
        float l = luminance(u.xyz);
        o = float4(l * u.w, l * u.w, l * u.w, u.w);
    }
    out[i0] = o;
}

struct SmartEdges {
    uint w;
    uint h;
    uint mode;
    float limit;
};

// Smart Blur's edges: where the blur's brightness changes strongly (Sobel), white on black, or
// white over the blur.
kernel void smart_edges(device float4 *out [[buffer(0)]], const device float4 *blurred [[buffer(1)]], constant SmartEdges &p [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    int x = int(g.x), y = int(g.y);
    #define L(i, j) luminance(blurred[uint(clamp(j, 0, int(p.h) - 1)) * p.w + uint(clamp(i, 0, int(p.w) - 1))].xyz)
    float gx = L(x + 1, y - 1) + 2.0 * L(x + 1, y) + L(x + 1, y + 1) - L(x - 1, y - 1) - 2.0 * L(x - 1, y) - L(x - 1, y + 1);
    float gy = L(x - 1, y + 1) + 2.0 * L(x, y + 1) + L(x + 1, y + 1) - L(x - 1, y - 1) - 2.0 * L(x, y - 1) - L(x + 1, y - 1);
    #undef L
    bool on = sqrt(gx * gx + gy * gy) * 0.25 > p.limit;
    float4 q = blurred[g.y * p.w + g.x];
    float v = p.mode == 1 ? (on ? 1.0 : 0.0) : 1.0;
    out[g.y * p.w + g.x] = (p.mode == 1 || on) ? float4(v * q.w, v * q.w, v * q.w, q.w) : q;
}
