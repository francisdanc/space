"""Download CC0 PBR materials from ambientCG and pack them into the game's texture arrays.

Run:  python tools/textures/build_textures.py

All source materials are CC0 (public domain) from https://ambientcg.com. Downloads are cached in
cache/textures_src; outputs go to assets/textures:

  terrain_albedo.png   N layers stacked vertically (sRGB colour)
  terrain_normal.png   N layers (OpenGL-convention normal maps, linear)
  terrain_arh.png      N layers: R = ambient occlusion, G = roughness, B = height (for blending)
  flora_albedo.png / flora_normal.png / flora_arh.png   bark and rock layers for plants
                       (leaf, needle, fern and grass cards are added by tools/blender/foliage.py)

Layer order must match TERRAIN_LAYERS / FLORA_LAYERS in src/textures.rs.
"""

import io
import os
import sys
import urllib.request
import zipfile

import numpy as np
from PIL import Image, ImageFilter, ImageOps

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
SRC = os.path.join(ROOT, "cache", "textures_src")
OUT = os.path.join(ROOT, "assets", "textures")
SIZE = 1024
UA = {"User-Agent": "Mozilla/5.0 (texture-build)"}

# (name, ambientCG asset id). None = flat placeholder layer (filled procedurally in the shader).
TERRAIN = [
    ("grass", "Grass004"),
    ("forest_litter", "Ground076"),
    ("moss", "Ground037"),
    ("dirt", "Ground078"),
    ("sand", "Ground054"),
    ("rock", "Rock030"),
    ("snow", "Snow006"),
    ("water", None),
]
FLORA = [
    ("bark_redwood", "Bark014"),
    ("bark_spruce", "Bark004"),
    ("bark_broadleaf", "Bark002"),
    ("rock_mossy", "Rock063"),
    ("rock_grey", "Rock051"),
    ("plain", "plain"),
    ("needle_mass", "mass:needles"),
    ("leaf_mass", "mass:leaves"),
]
# Foliage cards, packed 2x2 into one atlas (they are drawn with alpha cut-out).
CARD_ATLAS = ["needles", "leaves", "fern", "grass"]
CARDS = os.path.join(ROOT, "cache", "foliage")


def fetch(asset):
    """Return {map_name: PIL image} for an ambientCG asset (2K JPG), downloading once."""
    os.makedirs(SRC, exist_ok=True)
    path = os.path.join(SRC, f"{asset}_2K-JPG.zip")
    if not os.path.exists(path):
        url = f"https://ambientcg.com/get?file={asset}_2K-JPG.zip"
        print("downloading", url)
        data = urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=120).read()
        with open(path, "wb") as f:
            f.write(data)
    maps = {}
    with zipfile.ZipFile(path) as z:
        for n in z.namelist():
            for key in ("Color", "NormalGL", "Roughness", "AmbientOcclusion", "Displacement"):
                if n.endswith(f"_{key}.jpg") or n.endswith(f"_{key}.png"):
                    maps[key] = Image.open(io.BytesIO(z.read(n)))
    return maps


def card(name):
    """A Blender-rendered foliage card: bleed colours into the transparent area (no dark
    fringes when mipmapped), and derive normal/height from the alpha so leaves look rounded."""
    im = Image.open(os.path.join(CARDS, f"{name}.png")).convert("RGBA").resize((SIZE, SIZE), Image.LANCZOS)
    a = np.asarray(im, dtype=np.float32) / 255.0
    rgb, alpha = a[..., :3], a[..., 3]
    filled = rgb * alpha[..., None]
    weight = alpha.copy()
    out = np.where(alpha[..., None] > 0.5, rgb, 0.0)
    done = alpha > 0.5
    for radius in (2, 4, 8, 16, 32, 64, 128):
        blur = lambda x: np.asarray(Image.fromarray((x * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(radius)), dtype=np.float32) / 255.0
        wb = blur(weight)
        cb = np.stack([blur(filled[..., k]) for k in range(3)], -1)
        fill = (~done) & (wb > 0.002)
        out[fill] = cb[fill] / wb[fill][:, None]
        done |= fill
    albedo = Image.fromarray(np.dstack([out, alpha]).clip(0, 1).__mul__(255).astype(np.uint8), "RGBA")
    height = np.asarray(Image.fromarray((alpha * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(5)), dtype=np.float32) / 255.0
    gy, gx = np.gradient(height)
    n = np.dstack([-gx * 6.0, gy * 6.0, np.ones_like(gx)])
    n /= np.linalg.norm(n, axis=-1, keepdims=True)
    normal = Image.fromarray(((n * 0.5 + 0.5) * 255).astype(np.uint8), "RGB")
    ao = (0.6 + 0.4 * height) * 255
    arh = Image.fromarray(np.dstack([ao, np.full_like(ao, 150.0), height * 255]).astype(np.uint8), "RGB")
    return albedo, normal, arh


def mass(name, count=260, scale=0.33, seed=7):
    """A seamless tiling texture of dense foliage: the card scattered many times (random
    rotation/size, wrapping at the edges) over a dark base. Used on solid inner crowns."""
    import random
    rng = random.Random(seed)
    albedo, normal, _ = card(name)
    base_col = tuple(int(c * 0.45) for c in albedo.convert("RGB").resize((1, 1), Image.BOX).getpixel((0, 0)))
    out_a = Image.new("RGB", (SIZE, SIZE), base_col)
    out_n = Image.new("RGB", (SIZE, SIZE), (128, 128, 255))
    for _ in range(count):
        size = int(SIZE * scale * rng.uniform(0.7, 1.2))
        ang = rng.uniform(0, 360)
        a = albedo.resize((size, size), Image.LANCZOS).rotate(ang, Image.BILINEAR, expand=True)
        nm = normal.resize((size, size), Image.LANCZOS).rotate(ang, Image.BILINEAR, expand=True)
        shade = rng.uniform(0.6, 1.1)
        a = Image.merge("RGBA", [c.point(lambda v, k=shade: min(255, int(v * k))) for c in a.split()[:3]] + [a.split()[3]])
        x, y = rng.randrange(SIZE), rng.randrange(SIZE)
        for dx in (-SIZE, 0, SIZE):
            for dy in (-SIZE, 0, SIZE):
                out_a.paste(a, (x + dx - a.width // 2, y + dy - a.height // 2), a)
                out_n.paste(nm, (x + dx - a.width // 2, y + dy - a.height // 2), a)
    arh = Image.merge("RGB", (Image.new("L", (SIZE, SIZE), 200), Image.new("L", (SIZE, SIZE), 170), Image.new("L", (SIZE, SIZE), 128)))
    return out_a, out_n, arh


def layer(asset):
    if isinstance(asset, str) and asset.startswith("mass:"):
        return mass(asset[5:])
    if isinstance(asset, str) and asset.startswith("card:"):
        return card(asset[5:])
    if asset == "plain":
        flat = lambda c, mode="RGB": Image.new(mode, (SIZE, SIZE), c)
        return flat((255, 255, 255, 255), "RGBA"), flat((128, 128, 255)), flat((255, 160, 128))
    if asset is None:
        flat = lambda c: Image.new("RGB", (SIZE, SIZE), c)
        return flat((60, 90, 110)), flat((128, 128, 255)), flat((255, 15, 128))
    m = fetch(asset)
    size = (SIZE, SIZE)
    color = m["Color"].convert("RGB").resize(size, Image.LANCZOS)
    normal = m["NormalGL"].convert("RGB").resize(size, Image.LANCZOS)
    rough = m["Roughness"].convert("L").resize(size, Image.LANCZOS)
    ao = m["AmbientOcclusion"].convert("L").resize(size, Image.LANCZOS) if "AmbientOcclusion" in m else Image.new("L", size, 255)
    height = m["Displacement"].convert("L").resize(size, Image.LANCZOS) if "Displacement" in m else Image.new("L", size, 128)
    height = ImageOps.autocontrast(height, cutoff=1)
    return color, normal, Image.merge("RGB", (ao, rough, height))


def build(prefix, entries, rgba_albedo=False):
    layers = [layer(a) for _, a in entries]
    for k, suffix in enumerate(("albedo", "normal", "arh")):
        mode = "RGBA" if (k == 0 and rgba_albedo) else "RGB"
        sheet = Image.new(mode, (SIZE, SIZE * len(layers)))
        for i, maps in enumerate(layers):
            sheet.paste(maps[k].convert(mode), (0, SIZE * i))
        out = os.path.join(OUT, f"{prefix}_{suffix}.png")
        sheet.save(out, optimize=False)
        print("wrote", out, sheet.size)


if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    build("terrain", TERRAIN)
    build("flora", FLORA)
    maps = [card(n) for n in CARD_ATLAS]
    for k, suffix in enumerate(("albedo", "normal", "arh")):
        mode = "RGBA" if k == 0 else "RGB"
        atlas = Image.new(mode, (SIZE * 2, SIZE * 2))
        for i, m in enumerate(maps):
            atlas.paste(m[k].convert(mode), ((i % 2) * SIZE, (i // 2) * SIZE))
        out = os.path.join(OUT, f"foliage_{suffix}.png")
        atlas.save(out)
        print("wrote", out, atlas.size)
    print("CC0 materials from ambientCG:", ", ".join(a for _, a in TERRAIN + FLORA if a and ":" not in a and a != "plain"))
