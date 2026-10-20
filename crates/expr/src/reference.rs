//! What expressions can say, for whoever writes them: each thing by category, how it is
//! written, what it gives, and an example that works (the tests run every one).

/// One thing an expression can say: how it is written, what it gives, an example.
pub struct Entry {
    pub write: &'static str,
    pub about: &'static str,
    pub example: &'static str,
}

const fn e(write: &'static str, about: &'static str, example: &'static str) -> Entry {
    Entry { write, about, example }
}

/// Everything, by category.
pub fn reference() -> Vec<(&'static str, Vec<Entry>)> {
    vec![
        (
            "Global",
            vec![
                e("time", "the time now, in seconds of the composition", "time * 30"),
                e("value", "this property's own value (what its keys give)", "value"),
                e("thisComp", "this composition", "thisComp.width"),
                e("thisLayer", "this layer", "thisLayer.index"),
                e("thisProperty", "this property", "thisProperty.numKeys"),
                e("comp(name)", "another composition, by name", "comp(\"Main\").width"),
                e("index", "this layer's number, from 1 at the top", "index"),
                e("posterizeTime(perSecond)", "the expression worked out only so many times a second", "posterizeTime(4); time"),
            ],
        ),
        (
            "Time",
            vec![
                e("timeToFrames(t = time, perSecond?)", "seconds as frames", "timeToFrames(2)"),
                e("framesToTime(frames, perSecond?)", "frames as seconds", "framesToTime(60)"),
                e("timeToTimecode(t = time, perSecond?)", "seconds as a timecode text, 0:00:02:00", "timeToTimecode(2).length"),
            ],
        ),
        (
            "Vector math",
            vec![
                e("add(a, b)", "numbers or lists added", "add([1, 2], [3, 4])"),
                e("sub(a, b)", "one taken from the other", "sub([5, 5], [1, 2])"),
                e("mul(a, k)", "a list times a number", "mul([1, 2], 3)"),
                e("div(a, k)", "a list divided by a number", "div([4, 8], 2)"),
                e("clamp(v, least, most)", "kept between two values", "clamp(150, 0, 100)"),
                e("dot(a, b)", "the dot product", "dot([1, 0], [0.5, 1])"),
                e("cross(a, b)", "the cross product of two 3D lists", "cross([1, 0, 0], [0, 1, 0])"),
                e("normalize(v)", "the same direction, one long", "normalize([3, 4])"),
                e("length(a, b?)", "how long a list is, or how far apart two points are", "length([0, 0], [3, 4])"),
                e("lookAt(from, to)", "the turn [x, y, z] that faces one point toward another", "lookAt([0, 0, 0], [100, 0, 0])"),
            ],
        ),
        (
            "Random",
            vec![
                e("seedRandom(seed, timeless = false)", "starts the random numbers from a seed; timeless keeps them through time", "seedRandom(3, true); random()"),
                e("random(least?, most?)", "a random number (or list), new each frame", "random(10, 20)"),
                e("gaussRandom(least?, most?)", "random, mostly near the middle", "gaussRandom(0, 10)"),
                e("noise(v)", "smooth noise from -1 to 1 for a number or point", "noise(time)"),
            ],
        ),
        (
            "Interpolation",
            vec![
                e("linear(t, tLeast, tMost, a, b)", "a to b as t goes across, at an even pace (or linear(t, a, b) from 0 to 1)", "linear(time, 0, 2, 0, 100)"),
                e("ease(t, tLeast, tMost, a, b)", "the same, slow at both ends", "ease(time, 0, 2, 0, 100)"),
                e("easeIn(t, tLeast, tMost, a, b)", "slow at the start", "easeIn(time, 0, 2, 0, 100)"),
                e("easeOut(t, tLeast, tMost, a, b)", "slow at the end", "easeOut(time, 0, 2, 0, 100)"),
            ],
        ),
        (
            "Colour",
            vec![
                e("rgbToHsl(rgba)", "red, green, blue, alpha (0 to 1) as hue, saturation, lightness, alpha", "rgbToHsl([1, 0, 0, 1])"),
                e("hslToRgb(hsla)", "back again", "hslToRgb([0.5, 1, 0.5, 1])"),
                e("hexToRgb(\"#rrggbb\")", "a hex colour as [r, g, b, a] from 0 to 1", "hexToRgb(\"#ff8000\")"),
            ],
        ),
        (
            "Angles",
            vec![
                e("degreesToRadians(d)", "degrees as radians", "degreesToRadians(180)"),
                e("radiansToDegrees(r)", "radians as degrees", "radiansToDegrees(Math.PI)"),
            ],
        ),
        (
            "Property",
            vec![
                e("valueAtTime(t)", "the value at another time", "valueAtTime(time - 0.5)"),
                e("velocity", "how fast the value changes now, per second", "velocity"),
                e("velocityAtTime(t)", "how fast at another time", "velocityAtTime(1)"),
                e("speed", "how fast, as one number", "speed"),
                e("speedAtTime(t)", "how fast at another time, as one number", "speedAtTime(1)"),
                e("wiggle(perSecond, amount, octaves = 1, multiplier = 0.5, t = time)", "the value shaken at random, smoothly", "wiggle(2, 30)"),
                e("temporalWiggle(perSecond, amount, octaves?, multiplier?, t?)", "the value taken from a time shaken at random", "temporalWiggle(2, 0.2)"),
                e("smooth(width = 0.2, samples = 5, t = time)", "the value averaged over a little time around it", "smooth(0.2, 5)"),
                e("loopOut(\"cycle\" | \"pingpong\" | \"offset\" | \"continue\", keys = 0)", "the keys played again after the last", "loopOut(\"pingpong\")"),
                e("loopIn(type, keys = 0)", "the keys played again before the first", "loopIn(\"cycle\")"),
                e("loopOutDuration(type, seconds = 0)", "loopOut by a length of time", "loopOutDuration(\"cycle\", 1)"),
                e("loopInDuration(type, seconds = 0)", "loopIn by a length of time", "loopInDuration(\"cycle\", 1)"),
                e("numKeys", "how many keys it has", "numKeys"),
                e("key(n)", "its key n (from 1): .time, .value, .index", "key(1).time"),
                e("nearestKey(t)", "the key nearest a time", "nearestKey(time).index"),
                e("name", "its name", "thisProperty.name.length"),
                e("propertyGroup(n = 1)", "the group so many above it (1: the one it is in; past the top, its layer)", "thisProperty.propertyGroup(1).name.length"),
                e("propertyIndex", "its place in the group it is in, from 1", "thisProperty.propertyIndex"),
            ],
        ),
        (
            "Layer",
            vec![
                e("name, index", "its name, its number from the top", "thisLayer.name.length + thisLayer.index"),
                e("inPoint, outPoint, startTime", "when it starts showing, stops, and where its own time starts (seconds)", "thisLayer.outPoint - thisLayer.inPoint"),
                e("enabled, active", "whether it is switched on, and showing now", "thisLayer.active ? 1 : 0"),
                e("width, height", "the size of what it shows", "thisLayer.width"),
                e("hasParent, parent", "whether it has a parent, and the parent", "thisLayer.hasParent ? 1 : 0"),
                e("transform.position, anchorPoint, scale, rotation, opacity…", "its transform, also named alone (position)", "thisComp.layer(\"A\").transform.position[0]"),
                e("effect(name or n)(param)", "an effect's setting, by name or number", "thisComp.layer(\"B\").effect(\"Blur\")(\"Blurriness\")"),
                e("mask(name or n).maskPath, maskFeather, maskOpacity, maskExpansion", "a mask's properties", "thisComp.layer(\"B\").mask(1).maskOpacity"),
                e("content(name or n)", "a shape layer's shape, and a group's shapes in turn", "thisComp.layer(\"S\").content(\"Group 1\").content(\"Rectangle 1\").size[0]"),
                e("text.sourceText", "a text layer's words", "thisComp.layer(\"T\").text.sourceText.length"),
                e("marker", "its markers (see Markers)", "thisLayer.marker.numKeys"),
                e("sourceRectAtTime(t = time)", "the box of what it shows (words, shapes too), in its own units: .left, .top, .width, .height", "thisComp.layer(\"B\").sourceRectAtTime().width"),
                e("sampleImage(point, radius = [0.5, 0.5], afterEffects = true, t = time)", "the colour [r, g, b, a] round a point of it, from 0 to 1", "thisComp.layer(\"B\").sampleImage([50, 50], [4, 4])[0]"),
                e("source", "the composition a composition layer shows", "1"),
                e("toComp(point), fromComp(point)", "a point of the layer in the composition's pixels, and back", "thisComp.layer(\"B\").toComp([0, 0])[0]"),
                e("toWorld(point), fromWorld(point)", "the same in 3D space", "thisComp.layer(\"B\").toWorld([0, 0, 0])[0]"),
                e("toWorldVec(v), fromWorldVec(v), toCompVec(v), fromCompVec(v)", "a direction (not a point) the same ways", "thisComp.layer(\"B\").toWorldVec([1, 0, 0])[0]"),
            ],
        ),
        (
            "Composition",
            vec![
                e("name, width, height", "its name and size", "thisComp.width / thisComp.height"),
                e("duration, frameDuration", "how long it is, and one frame (seconds)", "thisComp.duration / thisComp.frameDuration"),
                e("numLayers", "how many layers it has", "thisComp.numLayers"),
                e("layer(name or n)", "one of its layers", "thisComp.layer(1).index"),
                e("layer(other, places)", "the layer so many places under another (above, below zero)", "thisComp.layer(thisLayer, -1).index"),
                e("pixelAspect, displayStartTime, bgColor", "its pixel shape, where its time is shown to start, its background", "thisComp.bgColor[3]"),
                e("activeCamera", "the camera it is seen through", "thisComp.activeCamera.index"),
                e("marker", "its markers (see Markers)", "thisComp.marker.numKeys"),
            ],
        ),
        (
            "Property group",
            vec![
                e("name, numProperties, enabled", "a group's name, how much it holds, whether it is on", "thisComp.layer(\"B\").mask(1).numProperties"),
                e("property(name or n)", "what a group holds", "thisComp.layer(\"B\").effect(1).property(1)"),
                e("group(name)", "a group can also be called with what it holds: transform(\"position\")", "thisComp.layer(\"A\")(\"transform\")(\"position\")[1]"),
            ],
        ),
        (
            "Markers",
            vec![
                e("marker.numKeys", "how many markers", "thisComp.marker.numKeys"),
                e("marker.key(n or comment)", "a marker: .time, .duration, .comment, .chapter, .url, .index", "thisComp.marker.key(1).time"),
                e("marker.nearestKey(t)", "the marker nearest a time", "thisComp.marker.nearestKey(time).index"),
            ],
        ),
        (
            "Path",
            vec![
                e("createPath(points, inTangents?, outTangents?, closed = true)", "a path made of points", "createPath([[0, 0], [100, 0], [100, 100]]).points().length"),
                e("points(t?), inTangents(t?), outTangents(t?)", "a path's points and their handles", "createPath([[0, 0], [100, 0]], [], [], false).points()[1][0]"),
                e("isClosed()", "whether the path is closed", "createPath([[0, 0], [100, 0], [100, 100]]).isClosed() ? 1 : 0"),
                e("pointOnPath(along = 0.5, t?)", "a point along the path, from 0 to 1", "createPath([[0, 0], [100, 0]], [], [], false).pointOnPath(0.5)[0]"),
                e("tangentOnPath(along, t?), normalOnPath(along, t?)", "its direction there, and the direction across", "createPath([[0, 0], [100, 0]], [], [], false).tangentOnPath(0.5)[0]"),
            ],
        ),
        (
            "JavaScript",
            vec![
                e("Math.sin, cos, tan, atan2, sqrt, pow, abs, min, max, floor, round, PI…", "JavaScript's Math", "Math.max(1, Math.sqrt(16))"),
                e("parseFloat, parseInt, isNaN, isFinite, String, Number", "numbers from text and back", "parseFloat(\"2.5\") + Number(\"1\")"),
                e("x.toFixed(digits)", "a number as text with so many decimals", "(2.345).toFixed(1).length"),
                e("if/else, for, while, var/let/const, function, ? :", "the usual JavaScript", "var n = 0; for (var i = 0; i < 3; i++) { n += i; } n"),
            ],
        ),
    ]
}
