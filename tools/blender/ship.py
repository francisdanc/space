"""Build the ship model in Blender and export it as assets/models/ship.glb.

Run:  blender --background --python tools/blender/ship.py -- <parts.json> <out.glb> [preview_dir]

The game exports its collision parts (space --dump-ship-parts tools/blender/ship_parts.json), so
walls, floor, ramp, seat etc. here match exactly what the player walks on. Details are added
only where they can't get in the way: outside the cabin, or as thin trim.

Ship-local axes (what the game uses): +X right, +Y up, -Z forward.
Blender axes: Z up. The glTF exporter maps Blender (x, y, z) -> glTF (x, z, -y), so a ship-local
point (x, y, z) goes to Blender (x, -z, y).
"""

import json
import math
import os
import sys

import bmesh
import bpy
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
PARTS_JSON = argv[0] if len(argv) > 0 else "tools/blender/ship_parts.json"
OUT_GLB = argv[1] if len(argv) > 1 else "assets/models/ship.glb"
PREVIEW_DIR = argv[2] if len(argv) > 2 else None


def L(x, y, z):
    """Ship-local point -> Blender coordinates."""
    return Vector((x, -z, y))


def S(sx, sy, sz):
    """Ship-local size -> Blender dimensions."""
    return Vector((sx, sz, sy))


# --------------------------------------------------------------------------- scene / materials

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene


def material(name, color, metallic=0.0, roughness=0.5, emission=None, strength=0.0, alpha=1.0):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = m.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Metallic"].default_value = metallic
    bsdf.inputs["Roughness"].default_value = roughness
    if emission is not None:
        bsdf.inputs["Emission Color"].default_value = (*emission, 1.0)
        bsdf.inputs["Emission Strength"].default_value = strength
    if alpha < 1.0:
        bsdf.inputs["Alpha"].default_value = alpha
        if hasattr(m, "surface_render_method"):
            m.surface_render_method = "BLENDED"
        if hasattr(m, "blend_method"):
            m.blend_method = "BLEND"
    m.diffuse_color = (*color, alpha)  # used by the Workbench preview
    return m


TEX_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "cache", "ship_textures"))
# Size (m) one texture repeat covers, per textured material; used for the box UV projection.
TILE = {"hull": 4.0, "accent": 2.0, "interior": 3.0, "hull_dark": 2.0, "floor": 1.5, "metal": 1.5,
        "nozzle": 1.5, "seat": 0.8}


def textured(name, tex, color):
    """PBR material from cache/ship_textures/<tex>_* (tools/textures/build_ship_textures.py)."""
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    bsdf = nt.nodes.get("Principled BSDF")

    def image(kind, non_color):
        node = nt.nodes.new("ShaderNodeTexImage")
        node.image = bpy.data.images.load(os.path.join(TEX_DIR, f"{tex}_{kind}.png"))
        if non_color:
            node.image.colorspace_settings.name = "Non-Color"
        return node

    nt.links.new(image("albedo", False).outputs["Color"], bsdf.inputs["Base Color"])
    nt.links.new(image("rough", True).outputs["Color"], bsdf.inputs["Roughness"])
    nt.links.new(image("metal", True).outputs["Color"], bsdf.inputs["Metallic"])
    nmap = nt.nodes.new("ShaderNodeNormalMap")
    nt.links.new(image("normal", True).outputs["Color"], nmap.inputs["Color"])
    nt.links.new(nmap.outputs["Normal"], bsdf.inputs["Normal"])
    m.diffuse_color = (*color, 1.0)
    m["tex"] = tex
    return m


MAT = {
    "hull": textured("Hull", "hull", (0.72, 0.74, 0.77)),
    "hull_dark": textured("HullDark", "hull_dark", (0.10, 0.11, 0.13)),
    "accent": textured("Accent", "accent", (0.85, 0.32, 0.05)),
    "interior": textured("Interior", "interior", (0.42, 0.44, 0.47)),
    "floor": textured("Floor", "floor", (0.16, 0.17, 0.18)),
    "seat": textured("Seat", "seat", (0.30, 0.12, 0.06)),
    "screen": material("Screen", (0.02, 0.05, 0.07), emission=(0.15, 0.7, 0.95), strength=4.0),
    "button_g": material("ButtonGreen", (0.02, 0.1, 0.02), emission=(0.2, 1.0, 0.3), strength=4.0),
    "button_r": material("ButtonRed", (0.1, 0.02, 0.02), emission=(1.0, 0.15, 0.1), strength=4.0),
    "glass": material("Glass", (0.25, 0.4, 0.5), metallic=0.0, roughness=0.05, alpha=0.18),
    "metal": textured("Metal", "metal", (0.45, 0.46, 0.48)),
    "nozzle": textured("Nozzle", "nozzle", (0.06, 0.06, 0.07)),
    "glow": material("EngineGlow", (0.1, 0.2, 0.4), emission=(0.35, 0.65, 1.0), strength=12.0),
    "light": material("CabinLight", (0.9, 0.85, 0.75), emission=(1.0, 0.92, 0.8), strength=6.0),
    "nav_red": material("NavRed", (0.5, 0.0, 0.0), emission=(1.0, 0.05, 0.02), strength=10.0),
    "nav_green": material("NavGreen", (0.0, 0.5, 0.0), emission=(0.05, 1.0, 0.1), strength=10.0),
    "nav_white": material("NavWhite", (0.8, 0.8, 0.8), emission=(1.0, 1.0, 1.0), strength=10.0),
}

ship = bpy.data.objects.new("Ship", None)
scene.collection.objects.link(ship)


def finish(obj, mat, bevel=0.0):
    obj.data.materials.append(MAT[mat])
    obj.parent = ship
    if bevel > 0.0:
        mod = obj.modifiers.new("Bevel", "BEVEL")
        mod.width = bevel
        mod.segments = 2
        mod.limit_method = "ANGLE"
    for poly in obj.data.polygons:
        poly.use_smooth = False
    return obj


def box(name, center, size, mat, tilt=0.0, bevel=0.03):
    """Axis-aligned (in ship space) box; `tilt` rotates about the ship's X axis."""
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=L(*center))
    obj = bpy.context.active_object
    obj.name = name
    obj.scale = S(*size)
    obj.rotation_euler = (tilt, 0.0, 0.0)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    return finish(obj, mat, bevel)


def cylinder_z(name, center, radius, length, mat, verts=24, bevel=0.02):
    """Cylinder along the ship's Z axis (front-back)."""
    bpy.ops.mesh.primitive_cylinder_add(vertices=verts, radius=radius, depth=length, location=L(*center),
                                        rotation=(math.pi / 2, 0, 0))
    obj = bpy.context.active_object
    obj.name = name
    return finish(obj, mat, bevel)


def cylinder_y(name, center, radius, length, mat, verts=16, bevel=0.0):
    """Cylinder along the ship's Y axis (vertical)."""
    bpy.ops.mesh.primitive_cylinder_add(vertices=verts, radius=radius, depth=length, location=L(*center))
    obj = bpy.context.active_object
    obj.name = name
    return finish(obj, mat, bevel)


def cone_z(name, center, r_front, r_back, length, mat, verts=24):
    """Truncated cone along ship Z; r_front at -Z (towards the nose), r_back at +Z."""
    bpy.ops.mesh.primitive_cone_add(vertices=verts, radius1=r_back, radius2=r_front, depth=length,
                                    location=L(*center), rotation=(math.pi / 2, 0, 0))
    obj = bpy.context.active_object
    obj.name = name
    return finish(obj, mat, 0.0)


def hull_from_sections(name, sections, mat, bevel=0.04):
    """Loft between rectangular cross-sections [(z, x_half, y_bottom, y_top), ...] (ship space)."""
    mesh = bpy.data.meshes.new(name)
    obj = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(obj)
    bm = bmesh.new()
    rings = []
    for z, xh, yb, yt in sections:
        rings.append([bm.verts.new(L(x, y, z)) for (x, y) in ((-xh, yb), (xh, yb), (xh, yt), (-xh, yt))])
    for a, b in zip(rings, rings[1:]):
        for i in range(4):
            j = (i + 1) % 4
            bm.faces.new((a[i], a[j], b[j], b[i]))
    bm.faces.new(rings[0])
    bm.faces.new(list(reversed(rings[-1])))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    return finish(obj, mat, bevel)


def wing(name, side, mat):
    """Swept, tapered wing inside the wing collider box (x 4.0..7.2, z -0.5..4.5, y -0.75..-0.45)."""
    s = side
    root_x, tip_x = 3.3 * s, 7.2 * s
    # (x, z_front, z_back, thickness)
    root = (root_x, -0.5, 4.5, 0.3)
    tip = (tip_x, 2.6, 4.4, 0.12)
    yc = -0.6
    mesh = bpy.data.meshes.new(name)
    obj = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(obj)
    bm = bmesh.new()
    def ring(x, zf, zb, t):
        return [bm.verts.new(L(x, yc + dy, z)) for (z, dy) in ((zf, -t / 2), (zb, -t / 2), (zb, t / 2), (zf, t / 2))]
    a, b = ring(*root), ring(*tip)
    for i in range(4):
        j = (i + 1) % 4
        bm.faces.new((a[i], a[j], b[j], b[i]))
    bm.faces.new(a)
    bm.faces.new(list(reversed(b)))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    return finish(obj, mat, 0.03)


# --------------------------------------------------------------------------- parts from the game

with open(PARTS_JSON) as f:
    parts = json.load(f)

PART_MAT = {
    "floor": "floor", "roof": "hull", "wall": "hull", "front_lower": "hull", "window_frame": "hull_dark",
    "dash": "hull_dark", "screen": "screen", "seat": "seat", "seat_back": "seat", "rear_wall": "hull",
    "door_top": "hull", "ramp": "floor", "glass": "glass",
}
# Parts rebuilt below with nicer shapes (their colliders stay as boxes in the game). The cabin
# walls, floor, roof and front/rear walls are replaced by one bevelled hull shell whose inner
# faces sit exactly on the cabin interior.
REPLACED = {"nose", "engine", "wing", "leg", "pad", "stripe", "floor", "roof", "wall", "front_lower",
            "window_frame", "rear_wall", "door_top"}

# Cabin interior (must match the game: x ±3.1, y -1.5..1.5, z -8.0..8.0).
CABIN = (3.1, -1.5, 1.5, -8.0, 8.0)
# Window and doorway openings.
WINDOW = (2.6, -0.4, 1.1)     # x half-width, y bottom, y top (front wall)
DOOR = (1.1, -1.5, 0.6)       # x half-width, y bottom, y top (rear wall)
# Outer hull: half-width, bottom, top, chamfers, length.
HX, HB, HT, CT, CB, HZ = 3.35, -1.75, 1.75, 0.45, 0.3, 8.25


def octagon(z, hx=HX, yb=HB, yt=HT, ct=CT, cb=CB):
    return [(-hx + cb, yb), (hx - cb, yb), (hx, yb + cb), (hx, yt - ct), (hx - ct, yt), (-hx + ct, yt),
            (-hx, yt - ct), (-hx, yb + cb)]


def loft(name, sections, mat, bevel=0.0):
    """Closed loft between polygon cross-sections [(z, [(x, y), ...]), ...] in ship space."""
    mesh = bpy.data.meshes.new(name)
    obj = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(obj)
    bm = bmesh.new()
    rings = [[bm.verts.new(L(x, y, z)) for (x, y) in pts] for z, pts in sections]
    n = len(rings[0])
    for a, b in zip(rings, rings[1:]):
        for i in range(n):
            j = (i + 1) % n
            bm.faces.new((a[i], a[j], b[j], b[i]))
    bm.faces.new(rings[0])
    bm.faces.new(list(reversed(rings[-1])))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    return finish(obj, mat, bevel)


def cut(obj, cutter):
    """Boolean-subtract `cutter` from `obj`; the cut faces take the cutter's material."""
    mod = obj.modifiers.new("Cut", "BOOLEAN")
    mod.operation = "DIFFERENCE"
    mod.object = cutter
    if hasattr(mod, "solver"):
        mod.solver = "EXACT"
    if hasattr(mod, "material_mode"):
        mod.material_mode = "TRANSFER"
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.modifier_apply(modifier=mod.name)
    bpy.data.objects.remove(cutter, do_unlink=True)


def cutter_box(center, size, mat):
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=L(*center))
    c = bpy.context.active_object
    c.scale = S(*size)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    c.data.materials.append(MAT[mat])
    return c


shell = loft("Hull", [(-HZ, octagon(-HZ)), (HZ, octagon(HZ))], "hull")
cx, yb, yt, z0, z1 = CABIN
cut(shell, cutter_box((0.0, (yb + yt) / 2, (z0 + z1) / 2), (2 * cx, yt - yb, z1 - z0), "interior"))
cut(shell, cutter_box((0.0, (WINDOW[1] + WINDOW[2]) / 2, -8.2), (2 * WINDOW[0], WINDOW[2] - WINDOW[1], 0.8),
                      "hull_dark"))
cut(shell, cutter_box((0.0, (DOOR[1] + DOOR[2]) / 2, 8.2), (2 * DOOR[0], DOOR[2] - DOOR[1], 0.8), "hull_dark"))
# The cabin floor is a darker metal than the walls.
box("FloorPlate", (0.0, -1.505, 0.0), (6.18, 0.01, 15.98), "floor", bevel=0.0)

for p in parts:
    if p["name"] in REPLACED:
        continue
    bevel = 0.0 if p["name"] in ("glass", "screen") else 0.03
    box(p["name"], p["center"], p["size"], PART_MAT[p["name"]], tilt=p["tilt"], bevel=bevel)

# --------------------------------------------------------------------------- exterior details

# Tapered nose below the window (the game keeps a box collider for it).
hull_from_sections("Nose", [
    (-8.25, 3.3, -1.7, -0.42),
    (-9.4, 2.6, -1.62, -0.55),
    (-10.6, 1.3, -1.35, -0.85),
], "hull")
box("NoseStripe", (0.0, -0.5, -8.9), (4.6, 0.04, 1.2), "accent", bevel=0.0)

# Roof: accent stripes, a dorsal spine and an antenna.
for x in (-1.6, 1.6):
    box("RoofStripe", (x, 1.765, 1.5), (0.5, 0.03, 9.0), "accent", bevel=0.0)
box("Spine", (0.0, 1.88, 3.0), (0.6, 0.3, 8.0), "hull_dark", bevel=0.06)
box("Fin", (0.0, 2.4, 6.2), (0.12, 0.8, 2.2), "hull_dark", bevel=0.03)
cylinder_y("Antenna", (0.0, 2.3, -2.0), 0.03, 1.1, "metal")

# Side trim: accent stripe and dark panel bands along the hull.
for s in (-1.0, 1.0):
    box("SideStripe", (3.36 * s, 0.6, 0.0), (0.03, 0.18, 15.6), "accent", bevel=0.0)
    for z in (-6.0, -2.0, 2.0):
        box("SidePanel", (3.36 * s, -0.6, z), (0.03, 0.9, 3.4), "hull_dark", bevel=0.0)

# Engine nacelles (inside the engine collider boxes: x ±3.3..4.5, y -0.6..1.2, z 3.25..7.75).
for s in (-1.0, 1.0):
    cylinder_z("Nacelle", (3.9 * s, 0.3, 5.3), 0.6, 4.1, "hull", bevel=0.04)
    cone_z("Intake", (3.9 * s, 0.3, 3.05), 0.42, 0.6, 0.4, "hull_dark")
    box("NacelleStripe", (3.9 * s, 0.92, 5.3), (0.25, 0.04, 3.6), "accent", bevel=0.0)
    cone_z("Nozzle", (3.9 * s, 0.3, 7.75), 0.55, 0.45, 0.6, "nozzle")
    cylinder_z("Glow", (3.9 * s, 0.3, 8.03), 0.36, 0.04, "glow", bevel=0.0)
    wing("Wing", s, "hull")
    box("WingStripe", (5.4 * s, -0.44, 3.6), (2.6, 0.02, 0.35), "accent", bevel=0.0)
    # Navigation lights on the wing tips: red on the left, green on the right.
    box("NavLight", (7.15 * s, -0.6, 3.5), (0.14, 0.14, 0.4), "nav_red" if s < 0 else "nav_green", bevel=0.0)

# Landing legs and pads (inside their colliders).
for s in (-1.0, 1.0):
    for z in (-5.5, 5.5):
        cylinder_y("Leg", (2.6 * s, -2.3, z), 0.14, 1.4, "metal")
        box("LegMount", (2.6 * s, -1.75, z), (0.5, 0.15, 0.5), "hull_dark", bevel=0.02)
        cylinder_y("Pad", (2.6 * s, -2.95, z), 0.5, 0.1, "hull_dark", verts=24)

# Tail light and ramp edge stripes.
box("TailLight", (0.0, 1.45, 8.27), (0.6, 0.08, 0.04), "nav_white", bevel=0.0)
ramp = next(p for p in parts if p["name"] == "ramp")
for x in (-1.0, 1.0):
    c = ramp["center"]
    box("RampEdge", (x, c[1] + 0.08 * math.cos(ramp["tilt"]), c[2] + 0.08 * math.sin(ramp["tilt"])),
        (0.12, 0.02, ramp["size"][2] - 0.1), "accent", tilt=ramp["tilt"], bevel=0.0)
# Door frame.
for x in (-1.14, 1.14):
    box("DoorFrame", (x, -0.45, 8.27), (0.08, 2.1, 0.04), "accent", bevel=0.0)
box("DoorFrame", (0.0, 0.64, 8.27), (2.36, 0.08, 0.04), "accent", bevel=0.0)
# Window frame trim.
box("WindowFrame", (0.0, 1.14, -8.27), (5.3, 0.08, 0.04), "hull_dark", bevel=0.0)
for x in (-2.64, 2.64):
    box("WindowFrame", (x, 0.35, -8.27), (0.08, 1.58, 0.04), "hull_dark", bevel=0.0)

# --------------------------------------------------------------------------- interior details

# Wall lining (thin, so it doesn't change the walkable space noticeably).
for s in (-1.0, 1.0):
    # Wall ribs every 2 m.
    for z in range(-6, 7, 2):
        box("Rib", (3.06 * s, 0.0, float(z)), (0.06, 2.95, 0.15), "hull_dark", bevel=0.0)
for z in (-3.5, 3.0):
    box("CeilingLight", (0.0, 1.475, z), (0.5, 0.02, 2.4), "light", bevel=0.0)
# Floor walkway strip.
box("Walkway", (0.0, -1.495, 0.5), (1.6, 0.01, 14.0), "hull_dark", bevel=0.0)
# Dashboard buttons and side screens.
for i, x in enumerate((-1.6, -1.35, 1.35, 1.6)):
    box("Button", (x, -0.39, -7.4), (0.12, 0.02, 0.12), "button_g" if i % 2 == 0 else "button_r", bevel=0.0)
for s in (-1.0, 1.0):
    box("SideScreen", (1.7 * s, -0.2, -7.85), (0.7, 0.4, 0.02), "screen", bevel=0.0)
# Armrests on the pilot seat.
for s in (-1.0, 1.0):
    box("Armrest", (0.45 * s, -0.65, -6.0), (0.1, 0.1, 0.7), "seat", bevel=0.02)

# --------------------------------------------------------------------------- UVs

# World-scale box projection, so every textured surface gets the same texel density whatever
# the object's size. Done per face from the object-space position (objects are unrotated except
# the tilted ramp parts, whose local axes follow the ramp).
for obj in [o for o in scene.objects if o.type == "MESH"]:
    me = obj.data
    if not me.uv_layers:
        me.uv_layers.new(name="UVMap")
    uv = me.uv_layers.active.data
    for poly in me.polygons:
        mat = me.materials[poly.material_index] if me.materials else None
        tex = mat.get("tex") if mat else None
        size = TILE.get(tex, 2.0)
        n = poly.normal
        ax = max(range(3), key=lambda i: abs(n[i]))
        for li in poly.loop_indices:
            co = me.vertices[me.loops[li].vertex_index].co
            u, v = [(co[1], co[2]), (co[0], co[2]), (co[0], co[1])][ax]
            uv[li].uv = (u / size, v / size)

# --------------------------------------------------------------------------- export

os.makedirs(os.path.dirname(os.path.abspath(OUT_GLB)), exist_ok=True)
bpy.ops.object.select_all(action="SELECT")
bpy.ops.export_scene.gltf(filepath=os.path.abspath(OUT_GLB), export_format="GLB", export_apply=True,
                          export_yup=True)
print(f"exported {OUT_GLB}")

# --------------------------------------------------------------------------- preview renders

if PREVIEW_DIR:
    os.makedirs(PREVIEW_DIR, exist_ok=True)
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.display.shading.light = "STUDIO"
    scene.display.shading.color_type = "MATERIAL"
    scene.display.shading.show_cavity = True
    scene.render.resolution_x, scene.render.resolution_y = 1200, 700
    world = bpy.data.worlds.new("World")
    scene.world = world
    cam_data = bpy.data.cameras.new("Cam")
    cam = bpy.data.objects.new("Cam", cam_data)
    scene.collection.objects.link(cam)
    scene.camera = cam
    for name, eye in (("rear", L(-16, 9, 22)), ("front", L(14, 6, -22)), ("side", L(-26, 2, 0))):
        cam.location = eye
        direction = L(0, -0.5, 0) - eye
        cam.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()
        scene.render.filepath = os.path.join(os.path.abspath(PREVIEW_DIR), f"ship_{name}.png")
        bpy.ops.render.render(write_still=True)
    print("previews written")
