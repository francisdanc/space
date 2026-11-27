"""Render foliage "cards" (alpha-tested textures for leaves, needles, fern fronds and grass).

Run:  blender --background --python tools/blender/foliage.py -- <out_dir>

Each card is built as real geometry in Blender (needles, leaves, leaflets, blades), lit flat
(Workbench, material colours, no shading) and rendered from above with a transparent
background, giving a clean albedo + alpha image. tools/textures/build_textures.py then adds
normal/roughness maps and packs the cards into the flora texture array.

Cards (1024x1024, the branch base at the bottom centre, growing up the image):
  needles.png  conifer branch spray       leaves.png  broadleaf twig with leaves
  fern.png     sword-fern frond           grass.png   clump of grass blades (side view)
"""

import math
import os
import random
import sys

import bmesh
import bpy
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
OUT = os.path.abspath(argv[0] if argv else "cache/foliage")
os.makedirs(OUT, exist_ok=True)
CARD = 0.5  # metres covered by a card


_mats = {}


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    _mats.clear()
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.display.shading.light = "FLAT"
    scene.display.shading.color_type = "MATERIAL"
    scene.render.film_transparent = True
    scene.render.resolution_x = scene.render.resolution_y = 1024
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.view_settings.view_transform = "Standard"
    scene.render.filter_size = 0.6
    cam = bpy.data.objects.new("Cam", bpy.data.cameras.new("Cam"))
    cam.data.type = "ORTHO"
    cam.data.ortho_scale = CARD
    cam.location = (0, CARD / 2, 5)
    scene.collection.objects.link(cam)
    scene.camera = cam
    return scene


def mat(rgb):
    key = tuple(round(c, 3) for c in rgb)
    if key not in _mats:
        m = bpy.data.materials.new(str(key))
        m.diffuse_color = (*rgb, 1.0)
        _mats[key] = m
    return _mats[key]


def mesh_obj(name, verts, faces, rgb):
    me = bpy.data.meshes.new(name)
    me.from_pydata(verts, [], faces)
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    ob.data.materials.append(mat(rgb))
    return ob


def strip(name, points, widths, rgb, z=0.0):
    """Flat tapered strip through 2D points (x, y) with half-widths."""
    verts, faces = [], []
    for i, (p, w) in enumerate(zip(points, widths)):
        q = points[min(i + 1, len(points) - 1)]
        r = points[max(i - 1, 0)]
        d = Vector((q[0] - r[0], q[1] - r[1])).normalized()
        n = Vector((-d.y, d.x))
        verts += [(p[0] - n.x * w, p[1] - n.y * w, z), (p[0] + n.x * w, p[1] + n.y * w, z)]
        if i:
            a = 2 * (i - 1)
            faces.append((a, a + 1, a + 3, a + 2))
    return mesh_obj(name, verts, faces, rgb)


def polygon(name, pts, rgb, z=0.0):
    return mesh_obj(name, [(x, y, z) for x, y in pts], [tuple(range(len(pts)))], rgb)


def leaf_shape(cx, cy, angle, length, width, serrate=0.0, segs=10):
    """Points of a pointed-oval leaf from its base (cx, cy) along `angle`."""
    pts = []
    for side in (1, -1):
        rng = range(segs + 1) if side == 1 else range(segs, -1, -1)
        for i in rng:
            t = i / segs
            w = width * math.sin(math.pi * t) ** 0.8 * (1 - 0.2 * t)
            if serrate and 0 < i < segs and i % 2:
                w *= 1 - serrate
            x, y = t * length, side * w
            ca, sa = math.cos(angle), math.sin(angle)
            pts.append((cx + x * ca - y * sa, cy + x * sa + y * ca))
    return pts


def render(scene, name):
    scene.render.filepath = os.path.join(OUT, f"{name}.png")
    bpy.ops.render.render(write_still=True)
    print("rendered", name)


def jitter(rgb, k, rng):
    return tuple(max(0.0, min(1.0, c * (1 + rng.uniform(-k, k)))) for c in rgb)


# --------------------------------------------------------------------------- conifer spray
def needles():
    scene = reset()
    rng = random.Random(1)
    twig = (0.22, 0.12, 0.06)
    greens = [(0.05, 0.13, 0.05), (0.06, 0.16, 0.06), (0.08, 0.19, 0.07), (0.04, 0.11, 0.045)]

    def branch(x0, y0, angle, length, depth):
        pts = [(x0 + math.cos(angle) * length * t, y0 + math.sin(angle) * length * t) for t in (0, 0.5, 1)]
        strip("twig", pts, [0.004 * (1 - 0.5 * t) for t in (0, 0.5, 1)], twig, z=0.01)
        steps = int(length / 0.0042)
        for i in range(steps):
            t = i / steps
            bx = x0 + math.cos(angle) * length * t
            by = y0 + math.sin(angle) * length * t
            for side in (1, -1):
                a = angle + side * rng.uniform(0.7, 1.1) + rng.uniform(-0.15, 0.15)
                ln = rng.uniform(0.026, 0.04) * (1 - 0.4 * t)
                c = jitter(greens[rng.randrange(4)], 0.15, rng)
                tip = (bx + math.cos(a) * ln, by + math.sin(a) * ln)
                strip("needle", [(bx, by), tip], [0.0026, 0.0008], c, z=0.02 + rng.random() * 0.01)
        if depth:
            for i in range(1, 8):
                t = i / 8
                for side in (1, -1):
                    if rng.random() < 0.95:
                        branch(x0 + math.cos(angle) * length * t, y0 + math.sin(angle) * length * t,
                               angle + side * rng.uniform(0.6, 0.9), length * (0.45 - 0.25 * t), depth - 1)

    branch(0.0, 0.01, math.pi / 2, 0.47, 1)
    render(scene, "needles")


# --------------------------------------------------------------------------- broadleaf twig
def leaves():
    scene = reset()
    rng = random.Random(2)
    twig = (0.25, 0.17, 0.1)
    strip("twig", [(0, 0.0), (0.01, 0.2), (-0.01, 0.46)], [0.006, 0.004, 0.002], twig, z=0.0)
    for k in range(16):
        t = 0.08 + 0.85 * k / 16
        y = t * 0.46
        side = 1 if k % 2 else -1
        a = math.pi / 2 + side * rng.uniform(0.6, 1.2)
        ln = rng.uniform(0.09, 0.13)
        c = jitter((0.12, 0.25, 0.06), 0.25, rng)
        pts = leaf_shape(0, y, a, ln, ln * 0.32, serrate=0.12, segs=12)
        polygon("leaf", pts, c, z=0.02 + k * 0.001)
        # Midrib.
        strip("vein", [(0, y), (math.cos(a) * ln * 0.9, y + math.sin(a) * ln * 0.9)], [0.0015, 0.0006],
              tuple(min(1, x * 1.35) for x in c), z=0.03 + k * 0.001)
    polygon("tipleaf", leaf_shape(0, 0.44, math.pi / 2, 0.05, 0.02, segs=8), (0.13, 0.27, 0.07), z=0.05)
    render(scene, "leaves")


# --------------------------------------------------------------------------- sword fern frond
def fern():
    scene = reset()
    rng = random.Random(3)
    rachis = [(math.sin(t * 2.0) * 0.01, 0.005 + t * 0.48) for t in [i / 20 for i in range(21)]]
    strip("rachis", rachis, [0.004 * (1 - i / 22) for i in range(21)], (0.12, 0.2, 0.05), z=0.01)
    pairs = 26
    for k in range(pairs):
        t = (k + 1) / (pairs + 1)
        x, y = rachis[int(t * 20)]
        size = 0.095 * math.sin(math.pi * (0.15 + 0.85 * t)) * (1.15 - t * 0.6)
        for side in (1, -1):
            a = math.pi / 2 - side * (1.15 - 0.3 * t)
            c = jitter((0.07, 0.2, 0.05), 0.2, rng)
            polygon("pinna", leaf_shape(x, y, a, size, size * 0.22, serrate=0.25, segs=10), c, z=0.02 + k * 0.0005)
    render(scene, "fern")


# --------------------------------------------------------------------------- grass clump
def grass():
    scene = reset()
    rng = random.Random(4)
    for k in range(70):
        x0 = rng.gauss(0, 0.06)
        h = rng.uniform(0.25, 0.47)
        lean = rng.uniform(-0.12, 0.12) + x0 * 0.8
        pts = [(x0 + lean * (t ** 1.6), h * t) for t in [i / 6 for i in range(7)]]
        w = rng.uniform(0.004, 0.007)
        base = (0.05, 0.12, 0.03)
        top = jitter((0.2, 0.33, 0.08), 0.25, rng)
        # Two-tone blade: darker lower half, lighter upper half.
        strip("blade_lo", pts[:4], [w, w * 0.9, w * 0.8, w * 0.7], base, z=rng.random() * 0.1)
        strip("blade_hi", pts[3:], [w * 0.7, w * 0.5, w * 0.3, 0.0003], top, z=0.11 + rng.random() * 0.1)
    render(scene, "grass")


needles()
leaves()
fern()
grass()
print("done")
