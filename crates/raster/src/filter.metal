// Filters over whole pictures on the GPU: passes along every row of a premultiplied RGBA
// picture, as filter.rs makes them on the processor (its columns are its rows once it is
// turned). A group of threads makes a stretch of a row together: the pixels it reads are fetched
// once into memory the group shares, and each thread reads its own from there.

// (after common.metal)

// A pass: the picture's rows (`w` long, `h` of them), whether nothing lies past their ends
// (else the end pixels go on), a bell's weights, and a box's width with where its ends lie
// from each pixel and what they take of the pixels they cut (worked out once on the processor).
struct Pass {
    uint w;
    uint h;
    uint clear;
    uint taps;
    float width;
    int first;
    int last;
    float first_part;
    float last_part;
};

// The pixels a group makes along a row together, and the most a pass reaches either side of a
// pixel for the group's stretch to be fetched whole (past that, each thread fetches its own). A
// bell's thread makes one pixel; a box's makes RUN side by side, its box slid along.
constant constexpr int STRETCH = 256;
constant constexpr int REACH = 128;
constant constexpr int RUN = 8;
constant constexpr int BOX_STRETCH = 64 * RUN;
constant constexpr int BELL_RUN = 4;

// Pixel `i` of row `row` (what lies past the ends as the pass says).
inline float4 at(const device float4 *src, constant Pass &p, int i, uint row) {
    if (p.clear && (i < 0 || i >= int(p.w))) {
        return float4(0.0);
    }
    return src[row * p.w + uint(clamp(i, 0, int(p.w) - 1))];
}

// The stretch a group makes, `before` pixels before it and `after` after it, fetched into
// `tile` (its first is pixel `start` - `before`).
inline void fetch(threadgroup float4 *tile, const device float4 *src, constant Pass &p, uint row, int start, int stretch, int before, int after, uint t, uint n) {
    int len = stretch + before + after;
    for (int k = int(t); k < len; k += int(n)) {
        tile[k] = at(src, p, start - before + k, row);
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
}

// The row blurred by a Gaussian: its weights, the bell's middle at the pixel (past the ends the
// stretch fetched holds nothing, or the end pixels, as the pass says). A thread makes BELL_RUN
// pixels side by side, each pixel it reads weighed into all of them: the weights come with
// BELL_RUN - 1 noughts either side, so each pixel's own come in order and the rest add nothing.
kernel void bell_rows(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant float *weights [[buffer(2)]], constant Pass &p [[buffer(3)]], uint2 g [[thread_position_in_grid]], uint2 t [[thread_position_in_threadgroup]], uint2 n [[threads_per_threadgroup]], uint2 group [[threadgroup_position_in_grid]]) {
    threadgroup float4 tile[STRETCH + 2 * REACH];
    int r = int(p.taps) / 2;
    int start = int(group.x) * STRETCH;
    fetch(tile, src, p, g.y, start, STRETCH, r, r, t.x, n.x);
    int first = int(g.x) * BELL_RUN;
    int from = first - start;
    float4 o[BELL_RUN];
    for (int j = 0; j < BELL_RUN; j++) {
        o[j] = float4(0.0);
    }
    for (int k = 0; k < int(p.taps) + BELL_RUN - 1; k++) {
        float4 v = tile[from + k];
        for (int j = 0; j < BELL_RUN; j++) {
            o[j] += v * weights[k - j + BELL_RUN - 1];
        }
    }
    for (int j = 0; j < BELL_RUN && first + j < int(p.w); j++) {
        out[g.y * p.w + uint(first + j)] = o[j];
    }
}

// The row blurred by a box `width` pixels wide round each pixel's middle: the pixels it covers
// whole, and the parts of those at its ends. A thread makes RUN pixels side by side: the whole
// ones added up for the first, then for each next the one coming in added and the one going out
// taken away.
kernel void box_rows(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant Pass &p [[buffer(2)]], uint2 g [[thread_position_in_grid]], uint2 t [[thread_position_in_threadgroup]], uint2 n [[threads_per_threadgroup]], uint2 group [[threadgroup_position_in_grid]]) {
    threadgroup float4 tile[BOX_STRETCH + 2 * REACH];
    int start = int(group.x) * BOX_STRETCH;
    int before = -p.first;
    fetch(tile, src, p, g.y, start, BOX_STRETCH, before, p.last, t.x, n.x);
    int first = int(g.x) * RUN;
    int i = first - start + before;
    float4 whole = float4(0.0);
    for (int j = i + p.first + 1; j < i + p.last; j++) {
        whole += tile[j];
    }
    for (int k = 0; k < RUN && first + k < int(p.w); k++, i++) {
        if (k > 0) {
            whole += tile[i + p.last - 1] - tile[i + p.first];
        }
        out[g.y * p.w + uint(first + k)] = (tile[i + p.first] * p.first_part + whole + tile[i + p.last] * p.last_part) / p.width;
    }
}

// As box_rows, for a box too wide to fetch round a stretch: each thread reads its own.
kernel void wide_box_rows(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant Pass &p [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    int i = int(g.x);
    float4 v = at(src, p, i + p.first, g.y) * p.first_part + at(src, p, i + p.last, g.y) * p.last_part;
    for (int j = i + p.first + 1; j < i + p.last; j++) {
        v += at(src, p, j, g.y);
    }
    out[g.y * p.w + g.x] = v / p.width;
}

// The picture turned on its diagonal (rows become columns): `w` × `h` in, `h` × `w` out, a
// square of it at a time through shared memory, so both reading and writing go along rows.
kernel void turn(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant uint2 &size [[buffer(2)]], uint2 t [[thread_position_in_threadgroup]], uint2 group [[threadgroup_position_in_grid]]) {
    threadgroup float4 square[32][33];
    uint2 base = group * 32;
    for (uint k = t.y; k < 32; k += 8) {
        uint2 at = uint2(base.x + t.x, base.y + k);
        if (at.x < size.x && at.y < size.y) {
            square[k][t.x] = src[at.y * size.x + at.x];
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint k = t.y; k < 32; k += 8) {
        uint2 to = uint2(base.y + t.x, base.x + k);
        if (to.x < size.y && to.y < size.x) {
            out[to.y * size.y + to.x] = square[t.x][k];
        }
    }
}

struct Along {
    uint w;
    uint h;
    uint n;
};

// Each pixel the mean of the picture along a line through it: its samples, each a number of
// whole pixels from it and the fraction past them (the same for every pixel).
kernel void along_blur(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], const device float4 *steps [[buffer(2)]], constant Along &p [[buffer(3)]], uint2 g [[thread_position_in_grid]]) {
    float4 acc = float4(0.0);
    for (uint i = 0; i < p.n; i++) {
        float4 s = steps[i];
        acc += between(src, p.w, p.h, int(g.x) + int(s.x), int(g.y) + int(s.y), s.zw);
    }
    out[g.y * p.w + g.x] = acc / float(p.n);
}

struct Round {
    uint w;
    uint h;
    int cx;
    int cy;
    float fx;
    float fy;
    float amount;
    uint zoom;
};

// Each pixel the mean of the picture round a point (turned either way by half the amount, in
// degrees) or along the line to it (scaled by half the amount either way): as many samples as
// the farthest moves pixels. Where a sample lies is worked out from its pixel (how far it moves),
// so it is as near the processor's as can be.
kernel void round_blur(device float4 *out [[buffer(0)]], const device float4 *src [[buffer(1)]], constant Round &p [[buffer(2)]], uint2 g [[thread_position_in_grid]]) {
    // from the point to the pixel's middle
    float vx = float(int(g.x) - p.cx) + (0.5 - p.fx);
    float vy = float(int(g.y) - p.cy) + (0.5 - p.fy);
    float reach = precise::sqrt(vx * vx + vy * vy);
    float travel = p.zoom ? reach * abs(p.amount) : reach * abs(p.amount) * (M_PI_F / 180.0);
    uint n = uint(clamp(ceil(travel) + 1.0, 2.0, 512.0));
    float4 acc = float4(0.0);
    for (uint i = 0; i < n; i++) {
        float t = float(i) / float(n - 1) - 0.5;
        float ox, oy;
        if (p.zoom) {
            float k = t * p.amount;
            ox = vx * k;
            oy = vy * k;
        } else {
            float a = t * p.amount * (M_PI_F / 180.0);
            float s = precise::sin(a);
            float h = precise::sin(a * 0.5);
            // turned: cos less one, without losing it to rounding
            float c1 = -2.0 * h * h;
            ox = vx * c1 - vy * s;
            oy = vx * s + vy * c1;
        }
        float flx = floor(ox), fly = floor(oy);
        acc += between(src, p.w, p.h, int(g.x) + int(flx), int(g.y) + int(fly), float2(ox - flx, oy - fly));
    }
    out[g.y * p.w + g.x] = acc / float(n);
}
