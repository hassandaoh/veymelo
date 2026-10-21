// What the GPU's picture programs share: pixels read, the picture between its pixels, and the
// blend modes, each as the processor works them (crates/raster, crates/color).

#include <metal_stdlib>
using namespace metal;

// Pixel (x, y) of a picture `w` × `h` (nothing outside it).
inline float4 pixel(const device float4 *src, uint w, uint h, int x, int y) {
    if (x < 0 || y < 0 || x >= int(w) || y >= int(h)) {
        return float4(0.0);
    }
    return src[uint(y) * w + uint(x)];
}

// The picture between its pixels, `t` past pixel (x0, y0) toward the next across and down, as
// the processor mixes them (across first, then down).
inline float4 between(const device float4 *src, uint w, uint h, int x0, int y0, float2 t) {
    float4 a = pixel(src, w, h, x0, y0), b = pixel(src, w, h, x0 + 1, y0);
    float4 c = pixel(src, w, h, x0, y0 + 1), d = pixel(src, w, h, x0 + 1, y0 + 1);
    float4 top = a + (b - a) * t.x;
    float4 bottom = c + (d - c) * t.x;
    return top + (bottom - top) * t.y;
}

// The picture at (x, y) (pixel units, a pixel's middle at its half), between its pixels.
inline float4 bilinear(const device float4 *src, uint w, uint h, float x, float y) {
    float fx = x - 0.5, fy = y - 0.5;
    float x0 = floor(fx), y0 = floor(fy);
    return between(src, w, h, int(x0), int(y0), float2(fx - x0, fy - y0));
}

// A pixel's colour and alpha, not premultiplied (as the effects' unpremul).
inline float4 unpremul(float4 p) {
    return p.w > 1e-7 ? float4(p.xyz / p.w, p.w) : float4(0.0);
}

// The blend modes, numbered as veymelo_color::BlendMode declares them.
constant constexpr uint NORMAL = 0, DISSOLVE = 1, DANCING_DISSOLVE = 2, DARKEN = 3, MULTIPLY = 4, COLOR_BURN = 5,
    CLASSIC_COLOR_BURN = 6, LINEAR_BURN = 7, DARKER_COLOR = 8, ADD = 9, LIGHTEN = 10, SCREEN = 11, COLOR_DODGE = 12,
    CLASSIC_COLOR_DODGE = 13, LINEAR_DODGE = 14, LIGHTER_COLOR = 15, OVERLAY = 16, SOFT_LIGHT = 17, HARD_LIGHT = 18,
    LINEAR_LIGHT = 19, VIVID_LIGHT = 20, PIN_LIGHT = 21, HARD_MIX = 22, DIFFERENCE = 23, CLASSIC_DIFFERENCE = 24,
    EXCLUSION = 25, SUBTRACT = 26, DIVIDE = 27, HUE = 28, SATURATION = 29, COLOR = 30, LUMINOSITY = 31,
    STENCIL_ALPHA = 32, STENCIL_LUMA = 33, SILHOUETTE_ALPHA = 34, SILHOUETTE_LUMA = 35, ALPHA_ADD = 36,
    LUMINESCENT_PREMUL = 37;

inline float luminance(float3 c) {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
}

inline bool keeps_over_range(uint m) {
    return !(m == COLOR_DODGE || m == CLASSIC_COLOR_DODGE || m == SOFT_LIGHT || m == HARD_LIGHT || m == LINEAR_LIGHT ||
             m == VIVID_LIGHT || m == PIN_LIGHT || m == HARD_MIX || m == HUE || m == SATURATION || m == COLOR);
}

inline float color_burn(float b, float s) {
    if (b >= 1.0) {
        return 1.0;
    }
    if (s <= 0.0) {
        return 0.0;
    }
    return 1.0 - min((1.0 - b) / s, 1.0);
}

inline float color_dodge(float b, float s) {
    if (b <= 0.0) {
        return 0.0;
    }
    if (s >= 1.0) {
        return 1.0;
    }
    return min(b / (1.0 - s), 1.0);
}

inline float screen1(float b, float s) {
    return b + s - b * s;
}

inline float hard_light(float b, float s) {
    return s <= 0.5 ? b * (2.0 * s) : screen1(b, 2.0 * s - 1.0);
}

inline float soft_light(float b, float s) {
    if (s <= 0.5) {
        return b - (1.0 - 2.0 * s) * b * (1.0 - b);
    }
    float d = b <= 0.25 ? ((16.0 * b - 12.0) * b + 4.0) * b : sqrt(b);
    return b + (2.0 * s - 1.0) * (d - b);
}

inline float channel(uint m, float b, float s) {
    switch (m) {
        case DARKEN: return min(b, s);
        case MULTIPLY: return b * s;
        case COLOR_BURN: return color_burn(b, s);
        case CLASSIC_COLOR_BURN: return s <= 0.0 ? 0.0 : 1.0 - (1.0 - b) / s;
        case LINEAR_BURN: return max(b + s - 1.0, 0.0);
        case ADD: case LINEAR_DODGE: return b + s;
        case LIGHTEN: return max(b, s);
        case SCREEN: return screen1(b, s);
        case COLOR_DODGE: return color_dodge(b, s);
        case CLASSIC_COLOR_DODGE: return s >= 1.0 ? 1.0 : min(b / (1.0 - s), 1.0);
        case OVERLAY: return hard_light(s, b);
        case SOFT_LIGHT: return soft_light(b, s);
        case HARD_LIGHT: return hard_light(b, s);
        case LINEAR_LIGHT: return clamp(b + 2.0 * s - 1.0, 0.0, 1.0);
        case VIVID_LIGHT: return s <= 0.5 ? color_burn(b, 2.0 * s) : color_dodge(b, 2.0 * s - 1.0);
        case PIN_LIGHT: return s <= 0.5 ? min(b, 2.0 * s) : max(b, 2.0 * s - 1.0);
        case HARD_MIX: return b + s >= 1.0 ? 1.0 : 0.0;
        case DIFFERENCE: case CLASSIC_DIFFERENCE: return abs(b - s);
        case EXCLUSION: return b + s - 2.0 * b * s;
        case SUBTRACT: return max(b - s, 0.0);
        case DIVIDE: return s <= 0.0 ? (b > 0.0 ? 1.0 : 0.0) : b / s;
        default: return s;
    }
}

inline float lum(float3 c) {
    return 0.3 * c.x + 0.59 * c.y + 0.11 * c.z;
}

inline float3 clip_colour(float3 c) {
    float l = lum(c);
    float lo = min(min(c.x, c.y), c.z), hi = max(max(c.x, c.y), c.z);
    float3 o = c;
    if (lo < 0.0) {
        o = l + (o - l) * l / max(l - lo, 1e-9);
    }
    if (hi > 1.0) {
        o = l + (o - l) * (1.0 - l) / max(hi - l, 1e-9);
    }
    return o;
}

inline float3 with_lum(float3 c, float l) {
    float d = l - lum(c);
    return clip_colour(c + d);
}

inline float sat(float3 c) {
    return max(max(c.x, c.y), c.z) - min(min(c.x, c.y), c.z);
}

inline float3 with_sat(float3 c, float s) {
    float lo = min(min(c.x, c.y), c.z), hi = max(max(c.x, c.y), c.z);
    if (hi - lo <= 1e-9) {
        return float3(0.0);
    }
    return (c - lo) * s / (hi - lo);
}

inline float3 formula(uint m, float3 b, float3 s) {
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

// Premultiplied `src` (its opacity in it) mixed onto premultiplied `dst` by mode `m`; `noise` (0
// to 1, a different one for each pixel) decides Dissolve.
inline float4 blend(uint m, float4 dst, float4 src, float noise) {
    float sa = src.w, da = dst.w;
    switch (m) {
        case NORMAL: {
            float k = 1.0 - sa;
            return float4(src.x + dst.x * k, src.y + dst.y * k, src.z + dst.z * k, sa + da * k);
        }
        case DISSOLVE: case DANCING_DISSOLVE:
            if (sa <= 0.0 || noise >= sa) {
                return dst;
            }
            return float4(src.x / sa, src.y / sa, src.z / sa, 1.0);
        case STENCIL_ALPHA: return dst * sa;
        case SILHOUETTE_ALPHA: return dst * (1.0 - sa);
        case STENCIL_LUMA: return dst * luminance(src.xyz);
        case SILHOUETTE_LUMA: return dst * (1.0 - luminance(src.xyz));
        case ALPHA_ADD: {
            float k = 1.0 - sa;
            return float4(src.x + dst.x * k, src.y + dst.y * k, src.z + dst.z * k, min(sa + da, 1.0));
        }
        case LUMINESCENT_PREMUL: {
            float k = 1.0 - sa;
            return float4(src.x + dst.x * k, src.y + dst.y * k, src.z + dst.z * k, sa + da - sa * da);
        }
        default: break;
    }
    if (sa <= 0.0) {
        return dst;
    }
    if (da <= 0.0) {
        return src;
    }
    float3 cs = float3(src.x / sa, src.y / sa, src.z / sa);
    float3 cb = float3(dst.x / da, dst.y / da, dst.z / da);
    if (!keeps_over_range(m)) {
        cs = clamp(cs, 0.0, 1.0);
        cb = clamp(cb, 0.0, 1.0);
    }
    float4 out;
    if (m == CLASSIC_COLOR_DODGE || m == CLASSIC_DIFFERENCE || m == CLASSIC_COLOR_BURN) {
        // the source fades toward the colour that changes nothing, then works at full strength
        float neutral = m == CLASSIC_COLOR_BURN ? 1.0 : 0.0;
        float3 b = formula(m, cb, neutral + sa * (cs - neutral));
        out.xyz = (1.0 - da) * src.xyz + da * b;
    } else {
        float3 b = formula(m, cb, cs);
        out.xyz = (1.0 - da) * src.xyz + (1.0 - sa) * dst.xyz + sa * da * b;
    }
    out.w = sa + da - sa * da;
    return out;
}
