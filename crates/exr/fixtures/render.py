# A small render with its passes (depth, normals, object and material numbers, Cryptomatte),
# written by Blender as a multi-layer EXR.
#   Blender -b --factory-startup --python render.py -- <out dir>
import sys, os
import bpy

out = sys.argv[sys.argv.index("--") + 1]
os.makedirs(out, exist_ok=True)
scene = bpy.context.scene
for o in list(bpy.data.objects):
    bpy.data.objects.remove(o)


def material(name, color):
    m = bpy.data.materials.new(name)
    m.pass_index = len(bpy.data.materials)
    m.use_nodes = True
    m.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (*color, 1)
    return m


bpy.ops.mesh.primitive_plane_add(size=8, location=(0, 0, 0))
floor = bpy.context.object
floor.name = "floor"
floor.data.materials.append(material("ground", (0.6, 0.6, 0.55)))
floor.pass_index = 1
bpy.ops.mesh.primitive_cube_add(size=1.6, location=(-1.1, 0, 0.8))
box = bpy.context.object
box.name = "box"
box.data.materials.append(material("red", (0.8, 0.1, 0.05)))
box.pass_index = 2
bpy.ops.mesh.primitive_uv_sphere_add(radius=0.9, location=(1.2, 0.3, 0.9))
ball = bpy.context.object
ball.name = "bunny"
ball.data.materials.append(material("blue", (0.1, 0.2, 0.9)))
ball.pass_index = 3
bpy.ops.object.camera_add(location=(0, -7, 3.2), rotation=(1.2, 0, 0))
scene.camera = bpy.context.object
bpy.ops.object.light_add(type="SUN", location=(2, -2, 5), rotation=(0.6, 0.3, 0))
bpy.context.object.data.energy = 3

scene.render.engine = "CYCLES"
scene.cycles.device = "CPU"
scene.cycles.samples = 16
scene.cycles.use_denoising = False
scene.render.resolution_x, scene.render.resolution_y, scene.render.resolution_percentage = 48, 32, 100
layer = scene.view_layers[0]
layer.use_pass_z = True
layer.use_pass_normal = True
layer.use_pass_object_index = True
layer.use_pass_material_index = True
layer.use_pass_cryptomatte_object = True
layer.use_pass_cryptomatte_material = True
layer.pass_cryptomatte_depth = 2
scene.render.film_transparent = True
s = scene.render.image_settings
s.media_type = "MULTI_LAYER_IMAGE"
s.file_format = "OPEN_EXR_MULTILAYER"
for codec, depth in [("ZIP", "32"), ("DWAA", "16")]:
    s.exr_codec = codec
    s.color_depth = depth
    scene.render.filepath = os.path.join(out, f"render-{codec.lower()}.exr")
    bpy.ops.render.render(write_still=True)
