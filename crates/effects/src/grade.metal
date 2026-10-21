// Colour work on the GPU (after common.metal), as grade.rs does it.

// Each pixel's brightness (not premultiplied), opaque.
kernel void brightness_map(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant uint &w [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    float l = luminance(unpremul(src[g.y * w + g.x]).xyz);
    out[g.y * w + g.x] = float4(l, l, l, 1.0);
}

struct ShadowHighlight {
    uint w;
    float s_amt;
    float h_amt;
    float s_tw;
    float h_tw;
    float cc;
    float mc;
    float keep;
};

// Shadow/Highlight: each pixel lifted by how dark its surroundings are and lowered by how
// bright, its colour kept as strong as the correction asks.
kernel void shadow_highlight(device float4 *out [[buffer(0)]], const device float4 *around_h [[buffer(1)]], const device float4 *src [[buffer(2)]], const device float4 *around_s [[buffer(3)]], constant ShadowHighlight &p [[buffer(4)]], uint2 g [[thread_position_in_grid]]) {
    uint i = g.y * p.w + g.x;
    float4 px = src[i];
    float4 c = unpremul(px);
    if (c.w <= 0.0) {
        out[i] = px;
        return;
    }
    float ws = clamp(1.0 - around_s[i].x / p.s_tw, 0.0, 1.0);
    ws = ws * ws;
    float wh = clamp((around_h[i].x - (1.0 - p.h_tw)) / p.h_tw, 0.0, 1.0);
    wh = wh * wh;
    float gain = (1.0 + p.s_amt * ws * 2.0) * (1.0 - p.h_amt * wh * 0.5);
    float l = luminance(c.xyz);
    float nl = l * gain;
    float sat = precise::pow(max(gain, 1e-3), p.cc);
    float3 o = nl + (c.xyz - l) * sat;
    if (p.mc != 0.0) {
        float shift = (0.5 + (nl - 0.5) * (1.0 + p.mc)) - nl;
        o = o + shift;
    }
    o = max(o, 0.0);
    o = o + (c.xyz - o) * p.keep;
    out[i] = float4(o * c.w, c.w);
}
