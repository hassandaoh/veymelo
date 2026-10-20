# Fixture EXR files written by OpenEXR itself (through Blender's OpenImageIO), and what
# OpenEXR reads back from them (index.json: each part's channels, hashed), for our own
# decoder's tests. The renders (render.py) go in the folder first, to be read back too.
#   Blender -b --factory-startup --python render.py -- <out dir>
#   Blender -b --factory-startup --python openexr.py -- <out dir>
import sys, os, math, json
import OpenImageIO as oiio
import numpy as np

out = sys.argv[sys.argv.index("--") + 1]
os.makedirs(out, exist_ok=True)

# a 34 × 25 data window inside a 40 × 30 display window
X0, Y0, W, H, DW, DH = 3, 2, 34, 25, 40, 30
NAMES = ["A", "B", "G", "R", "Y", "Z", "id"]
FORMATS = [oiio.HALF] * 5 + [oiio.FLOAT, oiio.UINT32]


def value(c, x, y):
    s = math.sin(x * 0.21 + c) * 0.5 + math.cos(y * 0.13 - c * 0.7) * 0.4 + 0.6
    grain = ((x * 7 + y * 13 + c * 29) % 17) / 170.0
    return (s + grain) * (1.0 + c * 0.5)


def pixels(w, h, scale=1):
    a = np.zeros((h, w, len(NAMES)), dtype=np.float32)
    for y in range(h):
        for x in range(w):
            for c in range(len(NAMES)):
                a[y, x, c] = value(c, x * scale, y * scale)
            a[y, x, 0] = min(a[y, x, 0], 1.0)
            # a bright highlight and a negative value, for the lossy ones
            if x == 20 and y == 10:
                a[y, x, 3] = 37.5
            if x == 5 and y == 20:
                a[y, x, 2] = -0.25
            # whole numbers: OpenImageIO scales floats to them by 2³² − 1
            a[y, x, 6] = ((x // 4 + y // 4) % 3 + 1) / 4294967295.0
    return a


def spec(compression, tiles=None, w=W, h=H):
    s = oiio.ImageSpec(w, h, len(NAMES), oiio.FLOAT)
    s.x, s.y = X0, Y0
    s.full_x, s.full_y, s.full_width, s.full_height = 0, 0, DW, DH
    s.channelnames = tuple(NAMES)
    s.channelformats = tuple(FORMATS)
    s.alpha_channel = 0
    s.attribute("compression", compression)
    s.attribute("Comment", "fixture")
    if tiles:
        s.tile_width, s.tile_height = tiles
    return s


def write(name, compression, tiles=None, mip=False, order=None):
    path = os.path.join(out, name)
    o = oiio.ImageOutput.create(path)
    s = spec(compression, tiles)
    if order:
        s.attribute("openexr:lineOrder", order)
    if mip:
        s.attribute("textureformat", "Plain Texture")
    if not o.open(path, s):
        print("open failed", name, o.geterror())
        return
    if not o.write_image(pixels(W, H)):
        print("write failed", name, o.geterror())
    if mip:
        w, h, scale = W, H, 1
        while w > 1 or h > 1:
            w, h, scale = max(1, w // 2), max(1, h // 2), scale * 2
            ls = spec(compression, tiles, w, h)
            ls.attribute("textureformat", "Plain Texture")
            if not o.open(path, ls, "AppendMIPLevel"):
                print("mip failed", o.geterror())
                break
            o.write_image(pixels(w, h, scale))
    o.close()


for c in ["none", "rle", "zips", "zip", "piz", "pxr24", "b44", "b44a", "dwaa", "dwab"]:
    write(f"lines-{c}.exr", c)
write("tiles-piz.exr", "piz", (16, 8))
write("tiles-zip-mip.exr", "zip", (16, 8), mip=True)
write("tiles-b44a.exr", "b44a", (16, 8))
write("tiles-dwaa.exr", "dwaa", (16, 8))
write("lines-zip-down.exr", "zip", order="decreasingY")

# two parts: the picture and a depth with a Cryptomatte manifest
path = os.path.join(out, "parts.exr")
o = oiio.ImageOutput.create(path)
a = oiio.ImageSpec(W, H, 4, oiio.HALF)
a.x, a.y, a.full_x, a.full_y, a.full_width, a.full_height = X0, Y0, 0, 0, DW, DH
a.channelnames = ("R", "G", "B", "A")
a.attribute("oiio:subimagename", "beauty")
a.attribute("compression", "zip")
b = oiio.ImageSpec(W, H, 1, oiio.FLOAT)
b.x, b.y, b.full_x, b.full_y, b.full_width, b.full_height = X0, Y0, 0, 0, DW, DH
b.channelnames = ("depth.Z",)
b.attribute("oiio:subimagename", "depth")
b.attribute("compression", "piz")
b.attribute("cryptomatte/abc1234/name", "CryptoObject")
b.attribute("cryptomatte/abc1234/manifest", '{"bunny":"13851a76","floor":"0a1b2c3d"}')
o.open(path, (a, b))
px = pixels(W, H)
o.write_image(np.ascontiguousarray(px[:, :, [3, 2, 1, 0]]))
o.open(path, b, "AppendSubimage")
o.write_image(np.ascontiguousarray(px[:, :, 5:6]))
o.close()


# what OpenEXR reads back: every part's channels over its data window, as floats
def dump(path):
    i = oiio.ImageInput.open(path)
    if not i:
        print("cannot read", path, oiio.geterror())
        return None
    parts = []
    sub = 0
    while i.seek_subimage(sub, 0):
        s = i.spec()
        data = i.read_image(sub, 0, 0, s.nchannels, oiio.FLOAT)
        parts.append({"name": s.getattribute("oiio:subimagename") or "", "channels": list(s.channelnames), "x": s.x, "y": s.y, "width": s.width, "height": s.height, "compression": s.getattribute("compression"), "data": data})
        sub += 1
    i.close()
    return parts


def levels(path):
    i = oiio.ImageInput.open(path)
    n = 0
    while i.seek_subimage(0, n):
        n += 1
    i.close()
    return n


def fnv(b):
    """FNV-1a, 64-bit, of a channel's floats (as the tests hash them)."""
    h = 0xcbf29ce484222325
    for x in b:
        h = ((h ^ x) * 0x100000001b3) & 0xffffffffffffffff
    return "%016x" % h


if __name__ == "__main__":
    index = {}
    for f in sorted(os.listdir(out)):
        if not f.endswith(".exr"):
            continue
        parts = dump(os.path.join(out, f))
        if parts is None:
            continue
        for p in parts:
            d = np.asarray(p["data"], dtype="<f4")
            p["hashes"] = [fnv(np.ascontiguousarray(d[:, :, c]).tobytes()) for c in range(d.shape[2])]
        index[f] = {"levels": levels(os.path.join(out, f)), "parts": [{k: v for k, v in p.items() if k != "data"} for p in parts]}
        print(f, [(p["name"], p["compression"], p["x"], p["y"], p["width"], p["height"], p["channels"]) for p in parts])
    with open(os.path.join(out, "index.json"), "w") as j:
        json.dump(index, j, indent=1)
