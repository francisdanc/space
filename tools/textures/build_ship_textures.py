"""Build the ship's PBR texture sets from CC0 ambientCG scans (downloads cached in cache/textures_src).

Run:  python tools/textures/build_ship_textures.py

Writes cache/ship_textures/<material>_{albedo,normal,rough,metal}.png (1024 px), which
tools/blender/ship.py wires into its materials; the glTF export embeds them in ship.glb.

Painted materials (hull, accent, interior) are panel plating recoloured to a paint colour, with
paint chipped off along the scratches of a worn-paint scan so bare metal shows through.
"""

import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(__file__))
from build_textures import fetch  # noqa: E402  (same download cache)

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "cache", "ship_textures")
SIZE = 1024


def load(asset, key, mode="RGB"):
    maps = fetch(asset)
    if key not in maps:
        return None
    return np.asarray(maps[key].convert(mode).resize((SIZE, SIZE), Image.LANCZOS), dtype=np.float32) / 255.0


def srgb_to_lin(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def lin_to_srgb(c):
    c = np.clip(c, 0, 1)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * c ** (1 / 2.4) - 0.055)


def save(name, kind, arr):
    os.makedirs(OUT, exist_ok=True)
    arr = np.clip(arr, 0, 1)
    mode = "L" if arr.ndim == 2 else "RGB"
    Image.fromarray((arr * 255 + 0.5).astype(np.uint8), mode).save(os.path.join(OUT, f"{name}_{kind}.png"))


def blend_normals(a, b, w):
    """Mix two GL normal maps by weight w (0 = a)."""
    na, nb = a * 2 - 1, b * 2 - 1
    n = na * (1 - w[..., None]) + nb * w[..., None]
    n /= np.linalg.norm(n, axis=-1, keepdims=True) + 1e-6
    return n * 0.5 + 0.5


def painted(name, paint_srgb, chip_amount, rough_paint):
    """Panel plating painted `paint_srgb`, with chips of bare metal."""
    plate_c = load("MetalPlates004", "Color")
    plate_n = load("MetalPlates004", "NormalGL")
    plate_r = load("MetalPlates004", "Roughness", "L")
    bare_c = load("Metal032", "Color")
    bare_n = load("Metal032", "NormalGL")
    bare_r = load("Metal032", "Roughness", "L")
    worn = load("PaintedMetal004", "Color")
    worn_n = load("PaintedMetal004", "NormalGL")
    # Chip mask: where the red paint of the worn scan is gone (low saturation).
    sat = worn.max(-1) - worn.min(-1)
    chip = np.clip((0.45 - sat) / 0.25, 0, 1)
    chip = np.clip((chip - (1 - chip_amount)) / max(chip_amount, 1e-3), 0, 1)

    # Paint keeps the plating's light/dark variation (panel seams, grime) around the paint colour.
    lum = srgb_to_lin(plate_c).mean(-1)
    detail = lum / lum.mean()
    paint = srgb_to_lin(np.array(paint_srgb, dtype=np.float32))[None, None, :] * (0.55 + 0.45 * detail[..., None])
    # Subtle streaky grime from the worn scan's luminance.
    grime = srgb_to_lin(worn).mean(-1)
    paint *= (0.88 + 0.12 * grime / grime.mean())[..., None]
    bare = srgb_to_lin(bare_c) * 0.9
    albedo = paint * (1 - chip[..., None]) + bare * chip[..., None]
    save(name, "albedo", lin_to_srgb(albedo))
    n = blend_normals(plate_n, worn_n, np.full(chip.shape, 0.35))
    n = blend_normals(n, bare_n, chip)
    save(name, "normal", n)
    save(name, "rough", np.clip(rough_paint + (plate_r - plate_r.mean()) * 0.5, 0.05, 1) * (1 - chip) + bare_r * chip)
    save(name, "metal", chip)


def scanned(name, asset, tint=None, rough_scale=1.0, metal=None, gain=1.0):
    """A scan used as-is, optionally re-tinted (multiply in linear space)."""
    c = srgb_to_lin(load(asset, "Color")) * gain
    if tint is not None:
        # Keep the scan's light/dark detail, replace its colour.
        lum = c.mean(-1, keepdims=True)
        c = srgb_to_lin(np.array(tint, dtype=np.float32))[None, None, :] * (0.4 + 0.6 * lum / lum.mean())
    ao = load(asset, "AmbientOcclusion", "L")
    if ao is not None:
        c *= (0.35 + 0.65 * ao)[..., None]
    save(name, "albedo", lin_to_srgb(c))
    save(name, "normal", load(asset, "NormalGL"))
    save(name, "rough", np.clip(load(asset, "Roughness", "L") * rough_scale, 0.04, 1))
    m = load(asset, "Metalness", "L")
    if metal is not None or m is None:
        m = np.full((SIZE, SIZE), metal if metal is not None else 0.0, dtype=np.float32)
    save(name, "metal", m)


if __name__ == "__main__":
    painted("hull", (0.80, 0.81, 0.82), chip_amount=0.07, rough_paint=0.45)
    painted("accent", (0.86, 0.36, 0.07), chip_amount=0.15, rough_paint=0.5)
    painted("interior", (0.52, 0.54, 0.56), chip_amount=0.08, rough_paint=0.6)
    scanned("hull_dark", "MetalPlates006", gain=0.8)
    scanned("floor", "DiamondPlate008C", gain=0.6)
    scanned("metal", "Metal032")
    scanned("nozzle", "Metal032", gain=0.15, rough_scale=1.3)
    scanned("seat", "Leather026", tint=(0.32, 0.15, 0.08), metal=0.0)
    print("ship textures written to", OUT)
