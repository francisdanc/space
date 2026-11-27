"""Build the astronaut (player body) in Blender and export assets/models/astronaut.glb.

Run:  blender --background --python tools/blender/astronaut.py -- <out.glb> [preview_dir]

A low-poly spacesuit made of rigid pieces, each parented to a bone of a simple skeleton, with
three looping animations: Idle, Walk and Run (exported as glTF animations with those names).

Axes: Blender Z up; the character faces Blender +Y, which the glTF exporter turns into -Z, the
game's "forward". The feet are at the origin. Total height ~1.85 m (eyes at ~1.65 m).
Pieces named Helmet* / Visor are hidden by the game in first-person view.
"""

import math
import os
import sys

import bpy
from mathutils import Matrix, Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
OUT_GLB = argv[0] if len(argv) > 0 else "assets/models/astronaut.glb"
PREVIEW_DIR = argv[1] if len(argv) > 1 else None

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.render.fps = 24


def material(name, color, metallic=0.0, roughness=0.6, emission=None, strength=0.0):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = m.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Metallic"].default_value = metallic
    bsdf.inputs["Roughness"].default_value = roughness
    if emission is not None:
        bsdf.inputs["Emission Color"].default_value = (*emission, 1.0)
        bsdf.inputs["Emission Strength"].default_value = strength
    m.diffuse_color = (*color, 1.0)
    return m


MAT = {
    "suit": material("Suit", (0.86, 0.86, 0.84), roughness=0.75),
    "joint": material("Joint", (0.22, 0.23, 0.25), roughness=0.8),
    "accent": material("SuitAccent", (0.85, 0.32, 0.05), roughness=0.6),
    "visor": material("Visor", (0.55, 0.38, 0.08), metallic=1.0, roughness=0.08),
    "metal": material("SuitMetal", (0.5, 0.52, 0.55), metallic=0.8, roughness=0.35),
    "light": material("SuitLight", (0.05, 0.1, 0.1), emission=(0.2, 0.9, 1.0), strength=6.0),
}

# --------------------------------------------------------------------------- skeleton

arm_data = bpy.data.armatures.new("Skeleton")
arm = bpy.data.objects.new("Astronaut", arm_data)
scene.collection.objects.link(arm)
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="EDIT")

BONES = {
    # name: (head, tail, parent)
    "Hips": ((0, 0, 0.95), (0, 0, 1.05), None),
    "Spine": ((0, 0, 1.05), (0, 0, 1.42), "Hips"),
    "Head": ((0, 0, 1.44), (0, 0, 1.76), "Spine"),
}
for side, x in (("L", 1.0), ("R", -1.0)):
    BONES[f"UpperArm.{side}"] = ((0.24 * x, 0, 1.39), (0.27 * x, 0, 1.11), "Spine")
    BONES[f"LowerArm.{side}"] = ((0.27 * x, 0, 1.11), (0.28 * x, 0.02, 0.86), f"UpperArm.{side}")
    BONES[f"UpperLeg.{side}"] = ((0.11 * x, 0, 0.93), (0.11 * x, 0, 0.52), "Hips")
    BONES[f"LowerLeg.{side}"] = ((0.11 * x, 0, 0.52), (0.11 * x, 0, 0.11), f"UpperLeg.{side}")

for name, (head, tail, parent) in BONES.items():
    b = arm_data.edit_bones.new(name)
    b.head, b.tail = Vector(head), Vector(tail)
    b.roll = 0.0
    if parent:
        b.parent = arm_data.edit_bones[parent]
        b.use_connect = False
bpy.ops.object.mode_set(mode="OBJECT")


def attach(obj, bone_name, mat):
    """Parent a mesh to a bone, keeping its current world placement."""
    obj.data.materials.append(MAT[mat])
    bone = arm_data.bones[bone_name]
    world = obj.matrix_world.copy()
    obj.parent = arm
    obj.parent_type = "BONE"
    obj.parent_bone = bone_name
    tail = arm.matrix_world @ bone.matrix_local @ Matrix.Translation((0, bone.length, 0))
    obj.matrix_parent_inverse = tail.inverted()
    obj.matrix_world = world
    for p in obj.data.polygons:
        p.use_smooth = True
    return obj


def new_obj(name):
    obj = bpy.context.active_object
    obj.name = name
    return obj


def sphere(name, loc, r, bone, mat, scale=(1, 1, 1), segs=20):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=segs, ring_count=segs // 2, radius=r, location=loc, scale=scale)
    return attach(new_obj(name), bone, mat)


def cyl(name, a, b, r, bone, mat, verts=14):
    a, b = Vector(a), Vector(b)
    d = b - a
    bpy.ops.mesh.primitive_cylinder_add(vertices=verts, radius=r, depth=d.length, location=(a + b) / 2)
    obj = new_obj(name)
    obj.rotation_mode = "QUATERNION"
    obj.rotation_quaternion = d.to_track_quat("Z", "Y")
    return attach(obj, bone, mat)


def rbox(name, loc, size, bone, mat, bevel=0.03):
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=loc, scale=size)
    obj = new_obj(name)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    mod = obj.modifiers.new("Bevel", "BEVEL")
    mod.width = bevel
    mod.segments = 3
    return attach(obj, bone, mat)


def torus(name, loc, r, minor, bone, mat):
    bpy.ops.mesh.primitive_torus_add(location=loc, major_radius=r, minor_radius=minor, major_segments=24,
                                     minor_segments=8)
    return attach(new_obj(name), bone, mat)


# --------------------------------------------------------------------------- suit pieces

# Head / helmet (hidden in first person).
sphere("Helmet", (0, 0, 1.61), 0.165, "Head", "suit")
sphere("Visor", (0, 0.088, 1.615), 0.142, "Head", "visor", scale=(1.0, 0.7, 0.8))
sphere("HelmetLamp", (0.12, 0.08, 1.7), 0.025, "Head", "light")
torus("HelmetRing", (0, 0, 1.45), 0.135, 0.035, "Spine", "metal")

# Torso, chest panel, backpack.
rbox("Torso", (0, 0, 1.23), (0.40, 0.25, 0.40), "Spine", "suit", bevel=0.06)
rbox("ChestPanel", (0, 0.13, 1.28), (0.20, 0.03, 0.12), "Spine", "joint", bevel=0.01)
for i, x in enumerate((-0.05, 0.0, 0.05)):
    sphere("ChestLight", (x, 0.148, 1.28), 0.012, "Spine", "light" if i != 1 else "accent", segs=8)
rbox("ChestStripe", (0, 0.127, 1.37), (0.36, 0.01, 0.03), "Spine", "accent", bevel=0.0)
rbox("Backpack", (0, -0.22, 1.23), (0.36, 0.18, 0.48), "Spine", "suit", bevel=0.05)
rbox("BackpackPanel", (0, -0.315, 1.25), (0.24, 0.02, 0.26), "Spine", "joint", bevel=0.01)
cyl("Hose", (0.15, -0.14, 1.36), (0.12, 0.08, 1.45), 0.02, "Spine", "joint")

# Hips and belt.
rbox("Belt", (0, 0, 0.99), (0.36, 0.24, 0.13), "Hips", "joint", bevel=0.03)
rbox("Buckle", (0, 0.125, 0.99), (0.09, 0.02, 0.07), "Hips", "metal", bevel=0.01)

for side, x in (("L", 1.0), ("R", -1.0)):
    # Arms.
    sphere("Shoulder", (0.24 * x, 0, 1.39), 0.085, f"UpperArm.{side}", "suit")
    cyl("UpperArm", (0.24 * x, 0, 1.37), (0.27 * x, 0, 1.13), 0.065, f"UpperArm.{side}", "suit")
    torus("ArmBand", (0.255 * x, 0, 1.25), 0.066, 0.014, f"UpperArm.{side}", "accent")
    sphere("Elbow", (0.27 * x, 0, 1.11), 0.062, f"LowerArm.{side}", "joint")
    cyl("Forearm", (0.27 * x, 0, 1.10), (0.28 * x, 0.02, 0.90), 0.06, f"LowerArm.{side}", "suit")
    rbox("Glove", (0.28 * x, 0.025, 0.83), (0.08, 0.1, 0.13), f"LowerArm.{side}", "joint", bevel=0.03)
    # Legs.
    cyl("Thigh", (0.11 * x, 0, 0.93), (0.11 * x, 0, 0.54), 0.085, f"UpperLeg.{side}", "suit")
    sphere("Knee", (0.11 * x, 0.02, 0.52), 0.075, f"LowerLeg.{side}", "joint")
    cyl("Shin", (0.11 * x, 0, 0.51), (0.11 * x, 0, 0.14), 0.075, f"LowerLeg.{side}", "suit")
    rbox("ShinStripe", (0.11 * x, 0.075, 0.33), (0.06, 0.01, 0.2), f"LowerLeg.{side}", "accent", bevel=0.0)
    rbox("Boot", (0.11 * x, 0.04, 0.06), (0.14, 0.27, 0.12), f"LowerLeg.{side}", "joint", bevel=0.03)

# --------------------------------------------------------------------------- animations

for pb in arm.pose.bones:
    pb.rotation_mode = "XYZ"
arm.animation_data_create()


def pose(frame, rot):
    """Set bone rotations (about each bone's X axis unless a tuple) and key every bone."""
    for pb in arm.pose.bones:
        r = rot.get(pb.name, 0.0)
        pb.rotation_euler = r if isinstance(r, tuple) else (r, 0.0, 0.0)
        pb.keyframe_insert("rotation_euler", frame=frame)


# For bones pointing down, a positive rotation about X swings them backwards (towards -Y);
# KNEE/ELBOW signs bend the joints the natural way.
KNEE = -1.0
ELBOW = 1.0


def gait(swing, knee, arm_swing, lean):
    """One stride: left leg forward then back. Returns {phase: rotations}."""
    def at(t):
        s = math.sin(2 * math.pi * t)
        # Knees bend most while the leg swings forward (lifting the foot).
        lk = knee * max(0.0, math.sin(2 * math.pi * t + 0.6)) + 0.1
        rk = knee * max(0.0, -math.sin(2 * math.pi * t + 0.6)) + 0.1
        return {
            "UpperLeg.L": -swing * s, "UpperLeg.R": swing * s,
            "LowerLeg.L": KNEE * lk, "LowerLeg.R": KNEE * rk,
            "UpperArm.L": arm_swing * s, "UpperArm.R": -arm_swing * s,
            "LowerArm.L": ELBOW * -0.35, "LowerArm.R": ELBOW * -0.35,
            "Spine": -lean,
        }
    return at


CLIPS = []
walk = gait(0.42, 0.7, 0.3, 0.03)
run = gait(0.75, 1.2, 0.6, 0.15)
CLIPS.append(("Walk", 24, [(f, walk(f / 24)) for f in range(0, 25, 3)]))
CLIPS.append(("Run", 16, [(f, run(f / 16)) for f in range(0, 17, 2)]))
idle = lambda t: {
    "Spine": 0.02 * math.sin(2 * math.pi * t),
    "Head": -0.02 * math.sin(2 * math.pi * t),
    "UpperArm.L": 0.04 * math.sin(2 * math.pi * t), "UpperArm.R": 0.04 * math.sin(2 * math.pi * t),
    "LowerArm.L": ELBOW * -0.2, "LowerArm.R": ELBOW * -0.2,
}
CLIPS.append(("Idle", 48, [(f, idle(f / 48)) for f in range(0, 49, 8)]))

for name, length, keys in CLIPS:
    act = bpy.data.actions.new(name)
    act.use_fake_user = True
    arm.animation_data.action = act
    for frame, rot in keys:
        pose(frame, rot)
    track = arm.animation_data.nla_tracks.new()
    track.name = name
    strip = track.strips.new(name, 0, act)
    if hasattr(strip, "action_slot") and hasattr(act, "slots") and len(act.slots) > 0:
        strip.action_slot = act.slots[0]
    track.mute = False
    arm.animation_data.action = None

# --------------------------------------------------------------------------- export

os.makedirs(os.path.dirname(os.path.abspath(OUT_GLB)), exist_ok=True)
bpy.ops.object.select_all(action="SELECT")
bpy.ops.export_scene.gltf(filepath=os.path.abspath(OUT_GLB), export_format="GLB", export_apply=True,
                          export_yup=True, export_animations=True, export_animation_mode="NLA_TRACKS")
print(f"exported {OUT_GLB}")

# --------------------------------------------------------------------------- previews

if PREVIEW_DIR:
    out = os.path.abspath(PREVIEW_DIR)
    os.makedirs(out, exist_ok=True)
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.display.shading.light = "STUDIO"
    scene.display.shading.color_type = "MATERIAL"
    scene.render.resolution_x, scene.render.resolution_y = 600, 700
    scene.world = bpy.data.worlds.new("World")
    cam = bpy.data.objects.new("Cam", bpy.data.cameras.new("Cam"))
    scene.collection.objects.link(cam)
    scene.camera = cam

    def shoot(fname, eye, clip=None, frame=0):
        for t in arm.animation_data.nla_tracks:
            t.mute = clip is not None and t.name != clip
        scene.frame_set(frame)
        cam.location = Vector(eye)
        cam.rotation_euler = (Vector((0, 0, 0.95)) - Vector(eye)).to_track_quat("-Z", "Y").to_euler()
        scene.render.filepath = os.path.join(out, fname)
        bpy.ops.render.render(write_still=True)

    shoot("astro_front.png", (1.6, 3.4, 1.4), clip="Idle", frame=0)
    shoot("astro_walk.png", (4.0, 0.0, 1.1), clip="Walk", frame=6)
    shoot("astro_run.png", (4.0, 0.0, 1.1), clip="Run", frame=4)
    print("previews written")
