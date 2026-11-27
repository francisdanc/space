//! Procedural plant (and rock) meshes. Every form is generated from parameters, so each planet
//! can have its own species, alien ones included. Each form has up to three levels of detail:
//! tier 0 (close), tier 1 (medium) and tier 2 (far). Meshes are in metres, Y up, base at the
//! origin.
//!
//! A template has two parts:
//! - `solid`: trunks, branches, rocks, logs. Textured from the bark/rock texture array (the
//!   layer index travels in `uv2.x`), with tiling UVs; the vertex colour tints it and carries
//!   baked ambient occlusion.
//! - `cards`: alpha-tested foliage cards (needles, leaves, fern fronds, grass) from the foliage
//!   atlas. Card normals are bent outward from the crown so foliage shades softly like a
//!   volume rather than as flat planes.

use std::f32::consts::{PI, TAU};

use bevy::math::{Quat, Vec2, Vec3, Vec4};

use crate::textures::{card as cell, flora_layer as tex};

pub type Rgb = [f32; 3];

/// Geometry with everything the flora materials need.
#[derive(Default, Clone)]
pub struct Part {
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    pub tan: Vec<Vec4>,
    pub uv: Vec<Vec2>,
    /// x = texture-array layer (solid parts), y = reserved for wind sway.
    pub uv2: Vec<Vec2>,
    pub col: Vec<[f32; 4]>,
    pub idx: Vec<u32>,
}

impl Part {
    fn vert(&mut self, p: Vec3, n: Vec3, t: Vec3, uv: Vec2, layer: f32, c: [f32; 4]) -> u32 {
        self.pos.push(p);
        self.nrm.push(n.normalize_or(Vec3::Y));
        self.tan.push(t.normalize_or(Vec3::X).extend(1.0));
        self.uv.push(uv);
        self.uv2.push(Vec2::new(layer, 0.0));
        self.col.push(c);
        (self.pos.len() - 1) as u32
    }

    pub fn tris(&self) -> usize {
        self.idx.len() / 3
    }
}

#[derive(Default, Clone)]
pub struct Template {
    pub solid: Part,
    pub cards: Part,
    /// Extra foliage cards that fill the crown out but cast no shadows (the main sprays in
    /// `cards` carry the shadow shapes, so the shadow pass draws far fewer cards).
    pub fill: Part,
    /// While set, `card` writes into `fill`.
    fill_mode: bool,
}

impl Template {
    #[cfg(test)]
    pub fn tris(&self) -> usize {
        self.solid.tris() + self.cards.tris() + self.fill.tris()
    }
}

/// Deterministic little RNG for shape variation.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x2545_F491_4F6C_DD1D)
    }
    pub fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
}

fn shade(c: Rgb, k: f32) -> [f32; 4] {
    [c[0] * k, c[1] * k, c[2] * k, 1.0]
}

pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

const WHITE: Rgb = [1.0, 1.0, 1.0];

/// Two unit vectors perpendicular to `axis` (and to each other).
fn frame(axis: Vec3) -> (Vec3, Vec3) {
    let a = if axis.y.abs() < 0.95 { Vec3::Y.cross(axis).normalize() } else { axis.any_orthonormal_vector() };
    (a, axis.cross(a))
}

// --------------------------------------------------------------------------- building blocks

impl Template {
    /// Bark-textured tapered tube from `a` (radius ra) to `b` (radius rb). `tile` is the
    /// texture size in metres.
    #[allow(clippy::too_many_arguments)]
    fn tube(&mut self, a: Vec3, b: Vec3, ra: f32, rb: f32, segs: u32, layer: f32, tile: f32, tint: Rgb, ao_base: f32, cap: bool) {
        let m = &mut self.solid;
        let axis = (b - a).normalize_or(Vec3::Y);
        let len = (b - a).length();
        let (u, v) = frame(axis);
        let slope = (ra - rb) / len.max(1e-3);
        let wraps = ((TAU * ra.max(rb)) / tile).round().max(1.0);
        let base = m.pos.len() as u32;
        for i in 0..=segs {
            let t = i as f32 / segs as f32 * TAU;
            let dir = u * t.cos() + v * t.sin();
            let n = (dir + axis * slope).normalize();
            let tangent = -u * t.sin() + v * t.cos();
            let us = i as f32 / segs as f32 * wraps;
            m.vert(a + dir * ra, n, tangent, Vec2::new(us, 0.0), layer, shade(tint, ao_base));
            m.vert(b + dir * rb, n, tangent, Vec2::new(us, -len / tile), layer, shade(tint, 1.0));
        }
        for i in 0..segs {
            let (a0, b0, a1, b1) = (base + 2 * i, base + 2 * i + 1, base + 2 * i + 2, base + 2 * i + 3);
            m.idx.extend_from_slice(&[a0, a1, b1, a0, b1, b0]);
        }
        if cap && rb > 0.01 {
            let c = m.vert(b, axis, u, Vec2::new(0.5, 0.5), tex::PLAIN, shade(mix(tint, [0.5, 0.35, 0.2], 0.6), 0.7));
            for i in 0..segs {
                m.idx.extend_from_slice(&[base + 2 * i + 1, base + 2 * i + 3, c]);
            }
        }
    }

    /// Ellipsoid with spherical UVs (rocks), or plain vertex-coloured (alien bulbs etc.).
    #[allow(clippy::too_many_arguments)]
    fn blob(&mut self, c: Vec3, radii: Vec3, segs: u32, rings: u32, layer: f32, tile: f32, top: Rgb, bottom: Rgb, lumpy: f32, rng: &mut Rng) {
        let m = &mut self.solid;
        let base = m.pos.len() as u32;
        let wraps = ((TAU * radii.x.max(radii.z)) / tile).round().max(1.0);
        let vwraps = ((PI * radii.y) / tile).round().max(1.0);
        for r in 0..=rings {
            let phi = r as f32 / rings as f32 * PI;
            for s in 0..=segs {
                let th = s as f32 / segs as f32 * TAU;
                let n = Vec3::new(phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin());
                let k = if r == 0 || r == rings || s == segs { 1.0 } else { 1.0 + lumpy * (rng.next() - 0.5) * 2.0 };
                let p = c + n * radii * k;
                let ao = 0.45 + 0.55 * (n.y * 0.5 + 0.5);
                let col = mix(bottom, top, n.y * 0.5 + 0.5);
                let tangent = Vec3::new(-th.sin(), 0.0, th.cos());
                let uv = Vec2::new(s as f32 / segs as f32 * wraps, r as f32 / rings as f32 * vwraps);
                m.vert(p, n, tangent, uv, layer, shade(col, ao));
            }
        }
        let row = segs + 1;
        for r in 0..rings {
            for s in 0..segs {
                let a = base + r * row + s;
                let b = a + 1;
                let c2 = a + row;
                let d = c2 + 1;
                m.idx.extend_from_slice(&[a, b, d, a, d, c2]);
            }
        }
    }

    /// Cone, textured from `layer` (tiling every `tile` metres) or plain-coloured.
    #[allow(clippy::too_many_arguments)]
    fn cone(&mut self, base: Vec3, apex: Vec3, r: f32, segs: u32, layer: f32, tile: f32, c_rim: Rgb, c_tip: Rgb) {
        let m = &mut self.solid;
        let axis = (apex - base).normalize_or(Vec3::Y);
        let (u, v) = frame(axis);
        let h = (apex - base).length();
        let slant = (h * h + r * r).sqrt();
        let wraps = ((TAU * r) / tile).round().max(1.0);
        let first = m.pos.len() as u32;
        for i in 0..=segs {
            let t = i as f32 / segs as f32 * TAU;
            let dir = u * t.cos() + v * t.sin();
            let n = (dir * h + axis * r).normalize();
            let tangent = -u * t.sin() + v * t.cos();
            let us = i as f32 / segs as f32 * wraps;
            m.vert(base + dir * r, n, tangent, Vec2::new(us, 0.0), layer, shade(c_rim, 0.8));
            m.vert(apex, n, tangent, Vec2::new(us, -slant / tile), layer, shade(c_tip, 1.0));
        }
        for i in 0..segs {
            let a = first + 2 * i;
            m.idx.extend_from_slice(&[a, a + 2, a + 1]);
        }
    }

    /// A foliage card: a quad growing from `base` along `up` (length `len`), `width` wide,
    /// drooping `bend` metres at the tip. `facing` is the card's normal before bending;
    /// `crown` is the point the shading normals are bent away from.
    #[allow(clippy::too_many_arguments)]
    fn card(&mut self, cell_index: u32, base: Vec3, up: Vec3, facing: Vec3, len: f32, width: f32, bend: f32, tint: Rgb, ao: f32, crown: Option<Vec3>) {
        let m = if self.fill_mode { &mut self.fill } else { &mut self.cards };
        let up = up.normalize();
        let right = facing.cross(up).normalize_or(up.any_orthonormal_vector()) * (width * 0.5);
        let face = up.cross(right).normalize();
        // Atlas cell (2x2), inset a little so mipmaps don't bleed between cards.
        let (cx, cy) = ((cell_index % 2) as f32 * 0.5, (cell_index / 2) as f32 * 0.5);
        let inset = 1.0 / 128.0;
        let uv = |u: f32, v: f32| Vec2::new(cx + inset + u * (0.5 - 2.0 * inset), cy + inset + v * (0.5 - 2.0 * inset));
        let rows = if bend.abs() > 0.01 { 2 } else { 1 };
        let start = m.pos.len() as u32;
        for r in 0..=rows {
            let t = r as f32 / rows as f32;
            let centre = base + up * (len * t) - Vec3::Y * (bend * t * t);
            for (side, u) in [(-1.0, 0.0), (1.0, 1.0)] {
                let p = centre + right * side;
                let n = match crown {
                    // Soft volumetric shading: mostly "away from the crown centre".
                    Some(c) => (face * 0.35 + (p - c).normalize_or(face) * 0.65).normalize(),
                    None => face,
                };
                let k = ao * (0.75 + 0.25 * t);
                m.vert(p, n, right, uv(u, 1.0 - t), 0.0, shade(tint, k));
            }
        }
        for r in 0..rows {
            let a = start + 2 * r;
            m.idx.extend_from_slice(&[a, a + 1, a + 3, a, a + 3, a + 2]);
        }
    }

    /// Plain-coloured flat strip along a spine (alien leaves, ribbon grass).
    fn ribbon(&mut self, spine: &[Vec3], side: Vec3, width: impl Fn(f32) -> f32, c_base: Rgb, c_tip: Rgb, zigzag: f32) {
        let m = &mut self.solid;
        let base = m.pos.len() as u32;
        let n = spine.len();
        for (i, p) in spine.iter().enumerate() {
            let t = i as f32 / (n - 1) as f32;
            let tangent = if i + 1 < n { spine[i + 1] - *p } else { *p - spine[i - 1] };
            let normal = tangent.cross(side).normalize_or(Vec3::Y);
            let notch = if i % 2 == 0 { 1.0 } else { 1.0 - zigzag };
            let w = width(t) * notch;
            let col = shade(mix(c_base, c_tip, t), 0.6 + 0.4 * t);
            m.vert(*p - side * w, normal, side, Vec2::ZERO, tex::PLAIN, col);
            m.vert(*p + side * w, normal, side, Vec2::ZERO, tex::PLAIN, col);
        }
        for i in 0..(n as u32 - 1) {
            let (a, b, c, d) = (base + 2 * i, base + 2 * i + 1, base + 2 * i + 2, base + 2 * i + 3);
            m.idx.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
}

/// Points along an arc that leaves `start` in direction `dir`, rising and drooping under gravity.
fn arc(start: Vec3, dir: Vec3, len: f32, rise: f32, droop: f32, steps: usize) -> Vec<Vec3> {
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            start + dir * (len * t) + Vec3::Y * (rise * t - droop * t * t)
        })
        .collect()
}



/// How a trunk is shaped (see `Template::trunk`).
#[derive(Clone, Copy)]
pub struct TrunkStyle {
    /// Extra radius at the ground (fraction of the base radius), fading over `flare_height` m.
    pub flare: f32,
    pub flare_height: f32,
    /// Buttress lobes at the base (0 = none) and how far they stick out (fraction of radius).
    pub lobes: u32,
    pub lobe_depth: f32,
    /// Depth of the vertical bark furrows (fraction of radius).
    pub furrows: f32,
    /// Sideways bend of the centreline (fraction of height).
    pub lean: f32,
    /// Moss creeping up one side from the ground (m).
    pub moss: f32,
    /// Above this fraction of the height the bark is in the crown's shade (1 = nowhere).
    pub shade_above: f32,
}

/// Deterministic pseudo-random value in -1..1 from an integer pair.
fn hash11(a: u32, b: u32) -> f32 {
    let mut x = (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x = (x ^ (x >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    ((x >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
}

impl Template {
    /// A tree trunk: a gently curving centreline with a natural taper (quick near the ground,
    /// slow above), a root flare spreading into buttress lobes, and vertical bark furrows that
    /// wander up the trunk, all as real geometry. Furrows and the base are darkened (baked
    /// occlusion) and moss tints one side near the ground. Returns the centreline at height
    /// fraction t (for attaching branches).
    #[allow(clippy::too_many_arguments)]
    pub fn trunk(&mut self, height: f32, r0: f32, r1: f32, style: TrunkStyle, rings: u32, segs: u32, layer: f32, tile: f32, tint: Rgb, seed: u64) -> impl Fn(f32) -> Vec3 + use<> {
        let s = seed as u32;
        // Centreline: two slow bends in random directions.
        let (a1, a2) = (hash11(s, 1) * PI, hash11(s, 2) * PI);
        let lean = style.lean * height;
        let center = move |t: f32| {
            let b1 = (t * PI * 0.9).sin() * lean;
            let b2 = (t * PI * 2.1 + 1.3).sin() * lean * 0.4;
            Vec3::new(a1.cos() * b1 + a2.cos() * b2, t * height, a1.sin() * b1 + a2.sin() * b2)
        };
        // Furrow pattern: a sum of angular harmonics whose phases drift with height, so ridges
        // run roughly vertically but wander and merge like real bark.
        let harmonics: Vec<(f32, f32, f32, f32)> = (0..5)
            .map(|k| {
                let n = [5.0, 7.0, 9.0, 12.0, 15.0][k];
                (n, 0.6 + 0.4 * hash11(s, 10 + k as u32).abs(), hash11(s, 20 + k as u32) * PI, hash11(s, 30 + k as u32) * 0.25)
            })
            .collect();
        let lobe_phase = hash11(s, 40) * PI;
        let moss_dir = hash11(s, 41) * PI;
        let radius = |t: f32, th: f32| -> (f32, f32) {
            let y = t * height;
            // Taper: most of the thinning happens low down.
            let base = r1 + (r0 - r1) * (1.0 - t).powf(1.6);
            let flare = 1.0 + style.flare * (-y / style.flare_height.max(0.1)).exp();
            let mut lobe = 0.0;
            if style.lobes > 0 {
                let c = ((th + lobe_phase) * style.lobes as f32).cos().max(0.0);
                lobe = c.powf(2.5) * style.lobe_depth * (-y / (style.flare_height * 0.8).max(0.1)).exp();
            }
            let mut f = 0.0;
            let mut norm = 0.0;
            for &(n, amp, ph, drift) in &harmonics {
                f += amp * (n * th + ph + drift * y).cos();
                norm += amp;
            }
            let f = f / norm;
            // Sharpen into narrow ridges between broad furrows.
            let ridge = -(1.0 - f.abs()).powf(2.0) + 0.5;
            let r = base * (flare + lobe) * (1.0 + style.furrows * ridge);
            (r, ridge)
        };

        let m = &mut self.solid;
        let start = m.pos.len() as u32;
        let row = segs + 1;
        let mid_r = 0.5 * (r0 + r1);
        let wraps = ((TAU * mid_r) / tile).round().max(1.0);
        let moss_col: Rgb = [0.32, 0.42, 0.18];
        let mut grid: Vec<(Vec3, f32, f32, f32)> = Vec::with_capacity(((rings + 1) * row) as usize);
        for i in 0..=rings {
            // Rings crowd towards the base, where the shape changes fastest.
            let t = (i as f32 / rings as f32).powf(1.35);
            let c = center(t);
            for j in 0..=segs {
                let th = j as f32 / segs as f32 * TAU;
                let (r, ridge) = radius(t, th);
                let p = c + Vec3::new(th.cos(), 0.0, th.sin()) * r;
                grid.push((p, t, th, ridge));
            }
        }
        // Normals from the surface itself (neighbouring rings and columns).
        let idx = |i: u32, j: u32| (i.min(rings) * row + j % segs) as usize;
        for i in 0..=rings {
            for j in 0..=segs {
                let (p, t, th, ridge) = grid[(i * row + j) as usize];
                let pu = grid[idx(i, j + 1)].0 - grid[idx(i, j + segs - 1)].0;
                let pv = grid[idx((i + 1).min(rings), j)].0 - grid[idx(i.saturating_sub(1), j)].0;
                let out = Vec3::new(th.cos(), 0.0, th.sin());
                let mut n = pv.cross(pu).normalize_or(out);
                if n.dot(out) < 0.0 {
                    n = -n;
                }
                let tangent = pu.normalize_or(Vec3::new(-th.sin(), 0.0, th.cos()));
                let y = t * height;
                // Baked light: furrows and the base in shadow, brighter up the trunk.
                let crown_shade = 1.0 - 0.6 * ((t - style.shade_above) / 0.08).clamp(0.0, 1.0);
                let ao = (0.72 + 0.28 * (y / (height * 0.3)).min(1.0)) * (0.72 + 0.28 * (ridge + 0.5).clamp(0.0, 1.0)) * crown_shade;
                let moss = if style.moss > 0.0 {
                    let side = ((th - moss_dir).cos() * 0.5 + 0.5).powf(2.0);
                    (1.0 - (y / style.moss).clamp(0.0, 1.0)) * side
                } else {
                    0.0
                };
                let col = mix(tint, moss_col, moss * 0.8);
                // Bark fibres run long: the texture is stretched up the trunk.
                let uv = Vec2::new(j as f32 / segs as f32 * wraps, -y / (tile * 1.6));
                m.vert(p, n, tangent, uv, layer, shade(col, ao));
            }
        }
        for i in 0..rings {
            for j in 0..segs {
                let a = start + i * row + j;
                let b = a + 1;
                let c = a + row;
                let d = c + 1;
                m.idx.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        center
    }

    /// A limb or branch: a tapered tube along a gentle curve from `a` towards `b`, sagging by
    /// `sag` m in the middle.
    #[allow(clippy::too_many_arguments)]
    pub fn limb(&mut self, a: Vec3, b: Vec3, ra: f32, rb: f32, sag: f32, rings: u32, segs: u32, layer: f32, tile: f32, tint: Rgb, ao: f32) {
        let pts: Vec<Vec3> = (0..=rings)
            .map(|i| {
                let t = i as f32 / rings as f32;
                a + (b - a) * t + Vec3::Y * (4.0 * t * (1.0 - t) * sag)
            })
            .collect();
        for i in 0..rings as usize {
            let t0 = i as f32 / rings as f32;
            let t1 = (i + 1) as f32 / rings as f32;
            let r = |t: f32| ra + (rb - ra) * t;
            // Overlap the pieces slightly so the joints don't show gaps.
            let dir = (pts[i + 1] - pts[i]).normalize_or(Vec3::Y);
            self.tube(pts[i] - dir * r(t0) * 0.3, pts[i + 1], r(t0), r(t1), segs, layer, tile, tint, ao * (0.85 + 0.15 * t0), false);
        }
    }
}

/// A conifer branch: a bark twig drooping out from the trunk, clothed in needle sprays.
#[allow(clippy::too_many_arguments)]
fn conifer_branch(m: &mut Template, origin: Vec3, dir: Vec3, len: f32, droop: f32, bark: f32, needle_tint: Rgb, crown: Vec3, ao: f32, tube: bool, rng: &mut Rng) {
    let end = origin + dir * len - Vec3::Y * droop;
    if tube {
        // The twig stops short of the tip so the sprays hide its end.
        m.tube(origin, origin + (end - origin) * 0.72, (0.04 * len).clamp(0.05, 0.3), 0.03, 4, bark, 1.5, [0.55, 0.45, 0.4], 0.6, false);
    }
    let along = (end - origin).normalize();
    let side = along.cross(Vec3::Y).normalize_or(Vec3::X);
    // Many overlapping sprays: a main spray along the branch plus side branchlets fanning out
    // left and right, each tilted and sized a little differently, lighter towards the tip.
    let sprays = if tube { 6 } else { 3 };
    for k in 0..sprays {
        let t = (k as f32 + 0.4) / sprays as f32;
        let base = origin + (end - origin) * (t * 0.85);
        let l = len * (0.5 - 0.2 * t) * rng.range(0.85, 1.15);
        let w = l * 1.2;
        let tip = [needle_tint[0] * (0.85 + 0.4 * t), needle_tint[1] * (0.85 + 0.35 * t), needle_tint[2] * (0.85 + 0.25 * t)];
        let tint = mix(needle_tint, tip, rng.next());
        let k_ao = ao * (0.8 + 0.2 * t);
        let up_face = (Vec3::Y + side * rng.range(-0.4, 0.4) + along * rng.range(-0.2, 0.2)).normalize();
        m.card(cell::NEEDLES, base, along, up_face, l, w, l * 0.3, tint, k_ao, Some(crown));
        // Side branchlets (fill: no shadows).
        m.fill_mode = true;
        for sgn in [-1.0f32, 1.0] {
            if !tube && rng.next() < 0.6 {
                continue;
            }
            let dir = (along + side * sgn * rng.range(0.5, 1.0) - Vec3::Y * rng.range(0.0, 0.3)).normalize();
            let face = (Vec3::Y + dir.cross(Vec3::Y).normalize_or(side) * rng.range(-0.5, 0.5)).normalize();
            let ls = l * rng.range(0.55, 0.8);
            m.card(cell::NEEDLES, base, dir, face, ls, ls * 1.1, 0.0, tint, k_ao * 0.95, Some(crown));
        }
        // A crossed spray every other step for volume seen edge-on.
        if k % 2 == 0 {
            let cross = (side + Vec3::Y * rng.range(0.2, 0.6)).normalize();
            m.card(cell::NEEDLES, base, along, cross, l * 0.8, w * 0.7, 0.0, tint, k_ao * 0.9, Some(crown));
        }
        m.fill_mode = false;
    }
}

/// The crown's tip: sprays fanning up and out from the top of the trunk, so the top reads
/// as foliage rather than a bare pole.
fn leader(m: &mut Template, h: f32, tint: Rgb, count: u32, rng: &mut Rng) {
    let light = [tint[0] * 1.15, tint[1] * 1.15, tint[2] * 1.05];
    for k in 0..count {
        let a = k as f32 / count as f32 * TAU + rng.range(-0.3, 0.3);
        let out = Vec3::new(a.cos(), 0.0, a.sin());
        let y = h * rng.range(0.8, 0.9);
        let dir = (Vec3::Y * rng.range(0.6, 1.6) + out).normalize();
        let l = rng.range(4.0, 7.0);
        m.card(cell::NEEDLES, Vec3::Y * y, dir, (out + Vec3::Y * 0.3).normalize(), l, l * 0.9, l * 0.15, light, 1.0, Some(Vec3::Y * (h * 0.8)));
    }
}

/// Short needle sprays around the trunk inside the crown, so it reads as dense foliage
/// rather than a bare pole between the branches.
#[allow(clippy::too_many_arguments)]
fn inner_sprays(m: &mut Template, y: f32, count: u32, r0: f32, len: f32, tint: Rgb, ao: f32, rng: &mut Rng) {
    let dark = [tint[0] * 0.75, tint[1] * 0.8, tint[2] * 0.75];
    m.fill_mode = true;
    for _ in 0..count {
        let a = rng.next() * TAU;
        let dir = Vec3::new(a.cos(), rng.range(-0.4, 0.3), a.sin()).normalize();
        let base = Vec3::Y * (y + rng.range(-1.5, 1.5)) + dir * r0;
        let face = (Vec3::Y + dir.cross(Vec3::Y) * rng.range(-0.6, 0.6)).normalize();
        let l = len * rng.range(0.7, 1.1);
        m.card(cell::NEEDLES, base, dir, face, l, l * 1.2, 0.0, dark, ao, Some(Vec3::Y * (y + 2.0)));
    }
    m.fill_mode = false;
}

// --------------------------------------------------------------------------- Earth-like forms

/// Giant forest conifer (redwood-like): massive straight trunk with buttress roots, short
/// drooping branches only in the upper half, a narrow crown. ~70 m tall.
pub fn giant_conifer(needle_tint: Rgb, seed: u64) -> [Option<Template>; 3] {
    let h = 70.0;
    let crown_base = h * 0.42;
    let crown_r = |y: f32| {
        let t = ((y - crown_base) / (h - crown_base)).clamp(0.0, 1.0);
        8.0 * (1.0 - t).powf(0.85) + 2.2
    };
    let bark = tex::BARK_REDWOOD;
    let bark_tint = [1.25, 0.88, 0.72];
    let mut tiers: [Option<Template>; 3] = Default::default();
    for (tier, slot) in tiers.iter_mut().enumerate() {
        let mut m = Template::default();
        let mut rng = Rng::new(seed);
        let segs = [14, 8, 4][tier];
        let trunk_h = h * 0.84;
        let style = TrunkStyle {
            flare: 0.9,
            flare_height: 5.0,
            lobes: 5,
            lobe_depth: 0.45,
            furrows: [0.16, 0.08, 0.0][tier],
            lean: 0.008,
            moss: 3.0,
            shade_above: crown_base / trunk_h,
        };
        let (rings, tsegs) = [(22, 32), (8, 12), (4, 6)][tier];
        let _ = segs;
        let axis = m.trunk(trunk_h, 1.9, 0.35, style, rings, tsegs, bark, 2.5, bark_tint, seed);
        let at = |y: f32| axis((y / trunk_h).clamp(0.0, 1.0));
        match tier {
            0 => {
                let mut angle = rng.next() * TAU;
                let whorls = 18;
                for l in 0..whorls {
                    let y0 = crown_base + (h - crown_base - 3.0) * l as f32 / whorls as f32;
                    let ao = 0.5 + 0.5 * (l as f32 / whorls as f32);
                    for _ in 0..5 {
                        angle += 2.399;
                        let y = y0 + rng.range(-1.0, 1.0);
                        let dir = Vec3::new(angle.cos(), rng.range(-0.3, 0.05), angle.sin()).normalize();
                        let r = crown_r(y) * rng.range(0.7, 1.1);
                        conifer_branch(&mut m, at(y) + dir * 0.4, dir, r, 0.12 * r, bark, needle_tint, Vec3::Y * (y + 2.0), ao, true, &mut rng);
                    }
                    inner_sprays(&mut m, y0, 5, 1.0, crown_r(y0) * 0.45, needle_tint, ao * 0.8, &mut rng);
                }
                leader(&mut m, h, needle_tint, 10, &mut rng);
            }
            1 => {
                let mut angle = rng.next() * TAU;
                for l in 0..12 {
                    let y = crown_base + (h - crown_base - 3.0) * l as f32 / 12.0;
                    for _ in 0..3 {
                        angle += 2.399;
                        let dir = Vec3::new(angle.cos(), -0.15, angle.sin()).normalize();
                        let r = crown_r(y) * 1.1;
                        conifer_branch(&mut m, at(y), dir, r, 0.12 * r, bark, needle_tint, Vec3::Y * (y + 2.0), 0.6 + 0.4 * l as f32 / 12.0, false, &mut rng);
                    }
                    inner_sprays(&mut m, y, 3, 1.0, crown_r(y) * 0.5, needle_tint, 0.6, &mut rng);
                }
                leader(&mut m, h, needle_tint, 5, &mut rng);
            }
            _ => {
                for a in [0.0f32, 1.57] {
                    let f = Vec3::new(a.cos(), 0.0, a.sin());
                    m.card(cell::NEEDLES, Vec3::Y * crown_base, Vec3::Y, f, h - crown_base + 3.0, crown_r(crown_base) * 2.6, 0.0, needle_tint, 0.8, None);
                }
            }
        }
        *slot = Some(m);
    }
    tiers
}

/// Spruce/fir/pine: whorls of drooping needle-clad branches all the way down. ~18 m;
/// `slender` narrows it.
pub fn conifer(bark: f32, needle_tint: Rgb, slender: f32, seed: u64) -> [Option<Template>; 3] {
    let h = 18.0;
    let width = 4.2 * (1.0 - slender * 0.5);
    let mut tiers: [Option<Template>; 3] = Default::default();
    for (tier, slot) in tiers.iter_mut().enumerate() {
        let mut m = Template::default();
        let mut rng = Rng::new(seed);
        let trunk_h = h * 0.88;
        let style = TrunkStyle {
            flare: 0.55,
            flare_height: 1.0,
            lobes: [4, 0, 0][tier],
            lobe_depth: 0.2,
            furrows: [0.07, 0.0, 0.0][tier],
            lean: 0.012,
            moss: 1.2,
            shade_above: 0.2,
        };
        let (rings, tsegs) = [(14, 18), (6, 8), (3, 4)][tier];
        let axis = m.trunk(trunk_h, 0.38, 0.06, style, rings, tsegs, bark, 1.5, WHITE, seed);
        let at = |y: f32| axis((y / trunk_h).clamp(0.0, 1.0));
        if tier == 0 {
            // Stubs of dead lower branches, below the living whorls.
            for k in 0..7 {
                let y = h * (0.04 + 0.08 * k as f32 / 7.0) + rng.range(0.0, 0.3);
                let a = rng.next() * TAU;
                let dir = Vec3::new(a.cos(), rng.range(-0.25, 0.1), a.sin()).normalize();
                let len = rng.range(0.25, 0.8);
                m.tube(at(y), at(y) + dir * len, 0.035, 0.012, 4, bark, 1.5, [0.75, 0.7, 0.65], 0.6, false);
            }
        }
        match tier {
            0 | 1 => {
                let whorls = [14, 8][tier];
                let per = [4, 3][tier];
                let mut angle = rng.next() * TAU;
                for l in 0..whorls {
                    let t = l as f32 / whorls as f32;
                    let y = h * (0.12 + 0.83 * t);
                    for _ in 0..per {
                        angle += 2.399;
                        let dir = Vec3::new(angle.cos(), -0.2, angle.sin()).normalize();
                        let r = (width * (1.0 - t * 0.9) + 0.4) * rng.range(0.85, 1.1);
                        conifer_branch(&mut m, at(y), dir, r, 0.2 * r, bark, needle_tint, Vec3::Y * (y + 1.0), 0.55 + 0.45 * t, tier == 0, &mut rng);
                    }
                }
                leader(&mut m, h * 1.05, needle_tint, [6, 3][tier], &mut rng);
            }
            _ => {
                for a in [0.0f32, 1.57] {
                    let f = Vec3::new(a.cos(), 0.0, a.sin());
                    m.card(cell::NEEDLES, Vec3::Y * (h * 0.08), Vec3::Y, f, h, width * 2.6, 0.0, needle_tint, 0.8, None);
                }
            }
        }
        *slot = Some(m);
    }
    tiers
}

/// Broadleaf tree (oak-like): trunk, spreading limbs and a crown of leafy twigs. ~16 m.
pub fn broadleaf(leaf_tint: Rgb, seed: u64) -> [Option<Template>; 3] {
    let h = 16.0;
    let crown = Vec3::Y * (h * 0.66);
    let bark = tex::BARK_BROADLEAF;
    let mut tiers: [Option<Template>; 3] = Default::default();
    for (tier, slot) in tiers.iter_mut().enumerate() {
        let mut m = Template::default();
        let mut rng = Rng::new(seed);
        let style = TrunkStyle {
            flare: 0.6,
            flare_height: 1.2,
            lobes: [4, 0, 0][tier],
            lobe_depth: 0.25,
            furrows: [0.1, 0.0, 0.0][tier],
            lean: 0.03,
            moss: 2.0,
            shade_above: 0.9,
        };
        let (rings, tsegs) = [(12, 20), (5, 8), (3, 5)][tier];
        let axis = m.trunk(h * 0.4, 0.65, 0.42, style, rings, tsegs, bark, 1.5, WHITE, seed);
        let fork = axis(0.97);
        let limbs = [5, 3, 0][tier];
        let mut ends = Vec::new();
        for k in 0..limbs {
            let a = k as f32 / limbs as f32 * TAU + rng.range(-0.3, 0.3);
            let end = crown + Vec3::new(a.cos() * rng.range(2.5, 4.0), rng.range(-0.5, 3.0), a.sin() * rng.range(2.5, 4.0));
            // Limbs rise steeply from the fork, then arch outwards.
            let (lr, ls) = [(5, 8), (3, 5), (2, 4)][tier];
            m.limb(fork, end, 0.34, 0.08, 0.6, lr, ls, bark, 1.5, WHITE, 0.8);
            if tier == 0 {
                // A couple of side branches off each limb.
                for b in 0..2 {
                    let t = 0.45 + 0.25 * b as f32;
                    let from = fork + (end - fork) * t + Vec3::Y * (4.0 * t * (1.0 - t) * 0.6);
                    let side = Vec3::new(-(a + 1.0 * (b as f32 - 0.5)).sin(), rng.range(0.2, 0.7), (a + 1.0 * (b as f32 - 0.5)).cos());
                    m.limb(from, from + side.normalize() * rng.range(1.2, 2.2), 0.1, 0.03, 0.2, 2, 4, bark, 1.5, WHITE, 0.85);
                }
            }
            ends.push(end);
        }
        let cards = [170, 55, 8][tier];
        let size = [2.6, 4.2, 7.5][tier];
        for k in 0..cards {
            // Leafy twigs over the crown's surface (an ellipsoid), facing outward.
            let z = rng.range(-0.5, 1.0);
            let a = rng.next() * TAU;
            let rxy = (1.0 - z * z).max(0.0).sqrt();
            let outward = Vec3::new(rxy * a.cos(), z, rxy * a.sin());
            // Throughout the crown, denser towards the outside where the light is.
            let depth = rng.range(0.35, 1.0).sqrt();
            // The outer shell casts the shadow; the inside is fill.
            m.fill_mode = depth < 0.8;
            let p = crown + outward * Vec3::new(4.6, 3.6, 4.6) * depth;
            let from = if ends.is_empty() { crown } else { ends[k % ends.len()] };
            let up = (p - from).normalize_or(outward) * 0.5 + outward * 0.5;
            let ao = 0.5 + 0.5 * (outward.y * 0.5 + 0.5);
            let tint = mix(leaf_tint, [leaf_tint[0] * 1.15, leaf_tint[1] * 1.1, leaf_tint[2] * 0.9], rng.next());
            m.card(cell::LEAVES, p - up * size * 0.4, up, outward, size, size * 0.8, size * 0.15, tint, ao, Some(crown));
        }
        m.fill_mode = false;
        *slot = Some(m);
    }
    tiers
}

/// Small fruit tree (wild apple): a broadleaf tree scaled down by its species, with fruit
/// hanging in the lower half of the crown where it can be reached.
pub fn fruit_tree(leaf_tint: Rgb, fruit: Option<Rgb>, seed: u64) -> [Option<Template>; 3] {
    let mut tiers = broadleaf(leaf_tint, seed);
    if let Some(c) = fruit {
        let h = 16.0;
        let crown = Vec3::Y * (h * 0.66);
        for (tier, slot) in tiers.iter_mut().enumerate() {
            let Some(m) = slot else { continue };
            let mut rng = Rng::new(seed ^ 0xA991E);
            let n = [36, 14, 0][tier];
            for _ in 0..n {
                let a = rng.next() * TAU;
                let z = rng.range(-0.75, 0.2);
                let rxy = (1.0 - z * z).max(0.0).sqrt();
                let out = Vec3::new(rxy * a.cos(), z, rxy * a.sin());
                let p = crown + out * Vec3::new(4.6, 3.6, 4.6) * rng.range(0.85, 1.0);
                // Apples ~9 cm across once the species scale (~0.4) is applied.
                m.blob(p, Vec3::splat(0.22), 6, 4, tex::PLAIN, 1.0, c, mix(c, [0.25, 0.3, 0.0], 0.5), 0.05, &mut rng);
            }
        }
    }
    tiers
}

/// Leafy bush, optionally with berries/flowers. ~1.6 m.
pub fn bush(leaf_tint: Rgb, accent: Option<Rgb>, seed: u64) -> [Option<Template>; 3] {
    let mut tiers: [Option<Template>; 3] = Default::default();
    for (tier, slot) in tiers.iter_mut().enumerate().take(2) {
        let mut m = Template::default();
        let mut rng = Rng::new(seed);
        let centre = Vec3::Y * 0.75;
        let cards = [14, 5][tier];
        for _ in 0..cards {
            let a = rng.next() * TAU;
            let z = rng.range(-0.1, 1.0);
            let rxy = (1.0 - z * z).max(0.0).sqrt();
            let out = Vec3::new(rxy * a.cos(), z, rxy * a.sin());
            let up = (out + Vec3::Y * 0.5).normalize();
            m.card(cell::LEAVES, Vec3::Y * 0.1 + out * 0.2, up, out, rng.range(1.0, 1.4), 1.0, 0.15, leaf_tint, 0.6 + 0.4 * z.max(0.0), Some(centre));
        }
        if let (0, Some(c)) = (tier, accent) {
            // Clusters of round berries (or flower heads) among the leaves.
            let dark = mix(c, [0.0, 0.0, 0.0], 0.45);
            for _ in 0..9 {
                let a = rng.next() * TAU;
                let p = Vec3::new(a.cos() * 0.68, rng.range(0.55, 1.3), a.sin() * 0.68);
                for k in 0..3 {
                    let o = Vec3::new(rng.range(-0.06, 0.06), -0.04 * k as f32, rng.range(-0.06, 0.06));
                    m.blob(p + o, Vec3::splat(0.045), 6, 4, tex::PLAIN, 1.0, c, dark, 0.0, &mut rng);
                }
            }
        }
        *slot = Some(m);
    }
    tiers
}

/// Sword fern: arching frond cards radiating from the crown. ~1.2 m.
pub fn fern(tint: Rgb, fronds: u32, seed: u64) -> [Option<Template>; 3] {
    let mut m = Template::default();
    let mut rng = Rng::new(seed);
    for f in 0..fronds {
        let a = f as f32 / fronds as f32 * TAU + rng.range(-0.25, 0.25);
        let out = Vec3::new(a.cos(), 0.0, a.sin());
        let up = (out + Vec3::Y * rng.range(0.9, 1.6)).normalize();
        let face = (Vec3::Y - out * 0.3).normalize();
        let len = rng.range(1.0, 1.45);
        m.card(cell::FERN, Vec3::Y * 0.03, up, face, len, len * 0.42, len * 0.45, tint, 1.0, Some(Vec3::Y * 0.2));
    }
    [Some(m), None, None]
}

/// Grass clump: three crossed cards of blades. ~0.5 m.
pub fn grass(tint: Rgb, seed: u64) -> [Option<Template>; 3] {
    let mut m = Template::default();
    let mut rng = Rng::new(seed);
    let a0 = rng.next() * PI;
    for k in 0..3 {
        let a = a0 + k as f32 * PI / 3.0;
        let face = Vec3::new(a.cos(), 0.0, a.sin());
        m.card(cell::GRASS, Vec3::ZERO, Vec3::Y, face, 0.55, 0.75, 0.0, tint, 1.0, Some(Vec3::Y * -0.3));
    }
    [Some(m), None, None]
}

/// Blade-ribbon grass in any colour (alien worlds, where the green card would look wrong).
pub fn grass_ribbons(color: Rgb, seed: u64) -> [Option<Template>; 3] {
    let mut m = Template::default();
    let mut rng = Rng::new(seed);
    for _ in 0..6 {
        let a = rng.next() * TAU;
        let dir = Vec3::new(a.cos(), 0.0, a.sin());
        let start = dir * rng.range(0.0, 0.12);
        let spine = arc(start, dir, rng.range(0.15, 0.3), rng.range(0.35, 0.6), 0.15, 2);
        let side = Vec3::Y.cross(dir).normalize();
        m.ribbon(&spine, side, |t| 0.03 * (1.0 - t), mix(color, [0.0; 3], 0.4), mix(color, [0.6, 0.6, 0.3], 0.3), 0.0);
    }
    [Some(m), None, None]
}

/// Small flower: stem and coloured head. ~0.4 m.
pub fn flower(stem: Rgb, petal: Rgb, seed: u64) -> [Option<Template>; 3] {
    let mut m = Template::default();
    let mut rng = Rng::new(seed);
    let top = Vec3::new(rng.range(-0.05, 0.05), rng.range(0.3, 0.45), rng.range(-0.05, 0.05));
    m.tube(Vec3::ZERO, top, 0.012, 0.01, 3, tex::PLAIN, 1.0, stem, 0.7, false);
    m.blob(top, Vec3::new(0.07, 0.035, 0.07), 5, 2, tex::PLAIN, 1.0, petal, mix(petal, [0.0; 3], 0.3), 0.3, &mut rng);
    [Some(m), None, None]
}

/// Boulder, photo-textured (mossy or bare rock). ~1 m radius.
pub fn boulder(mossy: bool, tint: Rgb, seed: u64) -> [Option<Template>; 3] {
    let mut tiers: [Option<Template>; 3] = Default::default();
    let layer = if mossy { tex::ROCK_MOSSY } else { tex::ROCK_GREY };
    for (tier, slot) in tiers.iter_mut().enumerate().take(2) {
        let mut m = Template::default();
        let mut rng = Rng::new(seed);
        let (s, r) = [(10, 6), (6, 4)][tier];
        m.blob(Vec3::Y * 0.35, Vec3::new(1.0, 0.75, 0.9), s, r, layer, 1.6, tint, mix(tint, [0.0; 3], 0.4), 0.25, &mut rng);
        *slot = Some(m);
    }
    tiers
}

/// An ore outcrop: a weathered rock with veins of ore or clusters of crystals breaking out of
/// it, coloured by the ore. ~1.5 m across before scaling.
pub fn ore_outcrop(ore: Rgb, rock: Rgb, crystals: bool, seed: u64) -> Template {
    let mut m = Template::default();
    let mut rng = Rng::new(seed);
    m.blob(Vec3::Y * 0.3, Vec3::new(0.95, 0.7, 0.85), 12, 7, tex::ROCK_GREY, 1.4, rock, mix(rock, [0.0; 3], 0.45), 0.3, &mut rng);
    // A couple of smaller rocks around it.
    for _ in 0..3 {
        let a = rng.next() * TAU;
        let p = Vec3::new(a.cos() * 0.95, 0.1, a.sin() * 0.95);
        let r = rng.range(0.25, 0.4);
        m.blob(p, Vec3::new(r, r * 0.7, r), 8, 5, tex::ROCK_GREY, 1.4, rock, mix(rock, [0.0; 3], 0.45), 0.3, &mut rng);
    }
    let dark = mix(ore, [0.0; 3], 0.5);
    let n = if crystals { 9 } else { 14 };
    for _ in 0..n {
        let a = rng.next() * TAU;
        let up = rng.range(0.15, 0.95);
        let out = Vec3::new(a.cos() * (1.0 - up * up).sqrt(), up, a.sin() * (1.0 - up * up).sqrt());
        let p = Vec3::Y * 0.3 + out * Vec3::new(0.9, 0.68, 0.8);
        if crystals {
            // Tall crystal prisms.
            let h = rng.range(0.25, 0.6);
            m.blob(p + Vec3::Y * h * 0.5, Vec3::new(0.08, h, 0.08), 6, 4, tex::PLAIN, 1.0, ore, dark, 0.0, &mut rng);
        } else {
            // Ore nodules in the rock.
            let r = rng.range(0.1, 0.2);
            m.blob(p, Vec3::new(r, r * 0.8, r), 6, 4, tex::PLAIN, 1.0, ore, dark, 0.15, &mut rng);
        }
    }
    m
}

/// A felled tree's stump: a short bark cylinder with a pale cut top. `r` is the trunk radius.
pub fn stump(r: f32, bark: f32, bark_tint: Rgb) -> Template {
    let mut m = Template::default();
    let h = (r * 0.9).clamp(0.35, 1.6);
    m.tube(Vec3::new(0.0, -0.3, 0.0), Vec3::new(0.0, h, 0.0), r * 1.15, r, 10, bark, 1.0, bark_tint, 0.8, false);
    // Cut face: light heartwood (bark texture brightened) slightly above the rim.
    m.tube(Vec3::new(0.0, h - 0.02, 0.0), Vec3::new(0.0, h + 0.01, 0.0), r * 0.95, r * 0.95, 10, bark, 0.3, [2.2, 1.9, 1.5], 1.0, true);
    m
}

/// Fallen log lying along X, bark-textured. ~8 m long.
pub fn log() -> [Option<Template>; 3] {
    let mut tiers: [Option<Template>; 3] = Default::default();
    for (tier, slot) in tiers.iter_mut().enumerate().take(2) {
        let mut m = Template::default();
        let segs = [10, 6][tier];
        m.tube(Vec3::new(-4.0, 0.55, 0.0), Vec3::new(4.0, 0.5, 0.0), 0.6, 0.5, segs, tex::BARK_REDWOOD, 2.0, [0.75, 0.7, 0.6], 1.0, true);
        *slot = Some(m);
    }
    tiers
}

/// Columnar cactus with arms. ~3.5 m.
pub fn cactus(color: Rgb, fruit: Option<Rgb>, seed: u64) -> [Option<Template>; 3] {
    let mut t0 = Template::default();
    let mut rng = Rng::new(seed);
    let light = mix(color, [0.5, 0.6, 0.3], 0.2);
    t0.tube(Vec3::ZERO, Vec3::Y * 3.5, 0.32, 0.26, 10, tex::PLAIN, 1.0, color, 0.7, true);
    let mut tips = vec![Vec3::Y * 3.5];
    for k in 0..2 {
        let a = rng.next() * TAU + k as f32 * PI;
        let dir = Vec3::new(a.cos(), 0.0, a.sin());
        let y = rng.range(1.2, 2.2);
        let elbow = Vec3::Y * y + dir * 0.8;
        let top = elbow + Vec3::Y * rng.range(0.8, 1.4);
        t0.tube(Vec3::Y * y, elbow, 0.18, 0.18, 6, tex::PLAIN, 1.0, color, 0.8, false);
        t0.tube(elbow, top, 0.18, 0.15, 6, tex::PLAIN, 1.0, light, 0.9, true);
        tips.push(top);
    }
    let mut t1 = Template::default();
    t1.tube(Vec3::ZERO, Vec3::Y * 3.5, 0.32, 0.26, 5, tex::PLAIN, 1.0, light, 0.7, true);
    if let Some(c) = fruit {
        // Fruit crowning the stem and arm tips.
        let mut frng = Rng::new(seed ^ 0xF00D);
        for tip in &tips {
            for _ in 0..3 {
                let a = frng.next() * TAU;
                let p = *tip + Vec3::new(a.cos() * 0.12, 0.05, a.sin() * 0.12);
                t0.blob(p, Vec3::new(0.08, 0.1, 0.08), 6, 4, tex::PLAIN, 1.0, c, mix(c, [0.2, 0.0, 0.0], 0.4), 0.05, &mut frng);
            }
        }
        t1.blob(Vec3::Y * 3.55, Vec3::splat(0.14), 5, 3, tex::PLAIN, 1.0, c, c, 0.0, &mut frng);
    }
    [Some(t0), Some(t1), None]
}

/// Dry shrub: a tangle of thin twigs with sparse leaves. ~1 m.
pub fn dry_shrub(twig: Rgb, leaf: Rgb, seed: u64) -> [Option<Template>; 3] {
    let mut t0 = Template::default();
    let mut rng = Rng::new(seed);
    for _ in 0..7 {
        let a = rng.next() * TAU;
        let end = Vec3::new(a.cos() * rng.range(0.4, 0.8), rng.range(0.5, 1.1), a.sin() * rng.range(0.4, 0.8));
        t0.tube(Vec3::ZERO, end, 0.03, 0.01, 3, tex::BARK_SPRUCE, 0.5, twig, 0.7, false);
        let out = end.normalize();
        t0.card(cell::LEAVES, end * 0.6, out, Vec3::Y, 0.5, 0.35, 0.05, leaf, 0.9, Some(Vec3::Y * 0.4));
    }
    let mut t1 = Template::default();
    for k in 0..3 {
        let a = k as f32 * 2.1;
        t1.card(cell::LEAVES, Vec3::ZERO, Vec3::Y, Vec3::new(a.cos(), 0.0, a.sin()), 1.0, 1.0, 0.0, leaf, 0.8, None);
    }
    [Some(t0), Some(t1), None]
}

/// Small forest-floor mushrooms. ~0.25 m.
pub fn mushroom(stem: Rgb, cap: Rgb, seed: u64) -> [Option<Template>; 3] {
    let mut m = Template::default();
    let mut rng = Rng::new(seed);
    for _ in 0..3 {
        let p = Vec3::new(rng.range(-0.2, 0.2), 0.0, rng.range(-0.2, 0.2));
        let h = rng.range(0.1, 0.25);
        m.tube(p, p + Vec3::Y * h, 0.025, 0.02, 4, tex::PLAIN, 1.0, stem, 0.7, false);
        m.blob(p + Vec3::Y * h, Vec3::new(0.08, 0.04, 0.08), 6, 3, tex::PLAIN, 1.0, cap, mix(cap, [0.0; 3], 0.5), 0.1, &mut rng);
    }
    [Some(m), None, None]
}

// --------------------------------------------------------------------------- alien forms

/// Giant mushroom tree: a curving stalk under a wide cap dotted with spots. ~14 m.
pub fn alien_mushroom_tree(stalk: Rgb, cap: Rgb, spots: Rgb, seed: u64) -> [Option<Template>; 3] {
    let mut tiers: [Option<Template>; 3] = Default::default();
    for (tier, slot) in tiers.iter_mut().enumerate() {
        let mut m = Template::default();
        let mut rng = Rng::new(seed);
        let segs = [10, 6, 4][tier];
        let bend = Vec3::new(rng.range(-1.5, 1.5), 0.0, rng.range(-1.5, 1.5));
        let mid = Vec3::Y * 7.0 + bend * 0.5;
        let top = Vec3::Y * 13.0 + bend;
        m.tube(Vec3::ZERO, mid, 1.1, 0.8, segs, tex::BARK_BROADLEAF, 2.0, stalk, 0.6, false);
        m.tube(mid, top, 0.8, 0.7, segs, tex::BARK_BROADLEAF, 2.0, stalk, 0.9, false);
        let (s, r) = [(14, 6), (8, 3), (6, 2)][tier];
        m.blob(top + Vec3::Y * 0.8, Vec3::new(6.5, 2.2, 6.5), s, r, tex::PLAIN, 1.0, cap, mix(cap, stalk, 0.6), 0.08, &mut rng);
        if tier == 0 {
            for _ in 0..10 {
                let a = rng.next() * TAU;
                let d = rng.range(1.5, 5.0);
                let y = 2.2 * (1.0 - (d / 6.5).powi(2)).max(0.0).sqrt();
                m.blob(top + Vec3::new(a.cos() * d, 0.8 + y, a.sin() * d), Vec3::new(0.6, 0.25, 0.6), 5, 2, tex::PLAIN, 1.0, spots, spots, 0.0, &mut rng);
            }
        }
        *slot = Some(m);
    }
    tiers
}

/// Crystal spire: stacked, twisting, tapering hexagonal segments. ~20 m.
pub fn alien_spire(base: Rgb, tip: Rgb, twist: f32, seed: u64) -> [Option<Template>; 3] {
    let mut tiers: [Option<Template>; 3] = Default::default();
    for (tier, slot) in tiers.iter_mut().enumerate() {
        let mut m = Template::default();
        let mut rng = Rng::new(seed);
        let segments = [7, 3, 1][tier];
        let h = 20.0;
        for k in 0..segments {
            let t0 = k as f32 / segments as f32;
            let t1 = (k + 1) as f32 / segments as f32;
            let off = |t: f32| Vec3::new((t * twist * 3.0).sin(), 0.0, (t * twist * 3.0).cos()) * (t * 1.5);
            let a = Vec3::Y * (h * t0) + off(t0);
            let b = Vec3::Y * (h * t1) + off(t1);
            m.tube(a, b, 1.4 * (1.0 - t0 * 0.85), 1.4 * (1.0 - t1 * 0.85), 6, tex::PLAIN, 1.0, mix(base, tip, t0), if k == 0 { 0.6 } else { 0.95 }, k + 1 == segments);
        }
        if tier == 0 {
            for _ in 0..5 {
                let a = rng.next() * TAU;
                let y = rng.range(0.0, 6.0);
                let dir = Vec3::new(a.cos(), rng.range(0.6, 1.2), a.sin()).normalize();
                let p = Vec3::Y * y + Vec3::new(a.cos(), 0.0, a.sin());
                m.cone(p, p + dir * rng.range(2.0, 4.0), 0.35, 5, tex::PLAIN, 1.0, base, tip);
            }
        }
        *slot = Some(m);
    }
    tiers
}

/// Coral-like tendril plant: branching tubes ending in bulbs. ~5 m.
pub fn alien_tendril(color: Rgb, tip: Rgb, branches: u32, seed: u64) -> [Option<Template>; 3] {
    #[allow(clippy::too_many_arguments)]
    fn grow(m: &mut Template, from: Vec3, dir: Vec3, len: f32, r: f32, depth: u32, branches: u32, color: Rgb, tip: Rgb, rng: &mut Rng) {
        let end = from + dir * len;
        m.tube(from, end, r, r * 0.7, 6, tex::PLAIN, 1.0, color, 0.8, false);
        if depth == 0 {
            m.blob(end, Vec3::splat(r * 2.2), 6, 4, tex::PLAIN, 1.0, tip, color, 0.1, rng);
            return;
        }
        for _ in 0..branches {
            let a = rng.next() * TAU;
            let nd = (dir + Vec3::new(a.cos(), 0.2, a.sin()) * 0.7).normalize();
            grow(m, end, nd, len * 0.7, r * 0.7, depth - 1, branches, color, tip, rng);
        }
    }
    let mut tiers: [Option<Template>; 3] = Default::default();
    for (tier, slot) in tiers.iter_mut().enumerate() {
        let mut m = Template::default();
        let mut rng = Rng::new(seed);
        match tier {
            0 => grow(&mut m, Vec3::ZERO, Vec3::Y, 2.0, 0.25, 2, branches, color, tip, &mut rng),
            1 => grow(&mut m, Vec3::ZERO, Vec3::Y, 2.0, 0.25, 1, branches, color, tip, &mut rng),
            _ => m.blob(Vec3::Y * 2.5, Vec3::new(2.0, 2.5, 2.0), 5, 3, tex::PLAIN, 1.0, tip, color, 0.2, &mut rng),
        }
        *slot = Some(m);
    }
    tiers
}

/// Cluster of inflated bulbs on short stalks. ~1.5 m.
pub fn alien_bulbs(color: Rgb, glow: Rgb, seed: u64) -> [Option<Template>; 3] {
    let mut t0 = Template::default();
    let mut rng = Rng::new(seed);
    for _ in 0..5 {
        let a = rng.next() * TAU;
        let p = Vec3::new(a.cos() * rng.range(0.1, 0.6), rng.range(0.4, 1.3), a.sin() * rng.range(0.1, 0.6));
        t0.tube(Vec3::ZERO, p, 0.05, 0.04, 4, tex::PLAIN, 1.0, color, 0.7, false);
        let r = rng.range(0.18, 0.35);
        t0.blob(p, Vec3::new(r, r * 1.2, r), 8, 5, tex::PLAIN, 1.0, glow, color, 0.05, &mut rng);
    }
    let mut t1 = Template::default();
    t1.blob(Vec3::Y * 0.8, Vec3::new(0.6, 0.8, 0.6), 6, 4, tex::PLAIN, 1.0, glow, color, 0.1, &mut rng);
    [Some(t0), Some(t1), None]
}

/// Fan plant: broad round paddles radiating from the base. ~1.3 m.
pub fn alien_fan(color: Rgb, edge: Rgb, seed: u64) -> [Option<Template>; 3] {
    let mut m = Template::default();
    let mut rng = Rng::new(seed);
    let paddles = 6;
    for f in 0..paddles {
        let a = f as f32 / paddles as f32 * TAU + rng.range(-0.3, 0.3);
        let dir = Vec3::new(a.cos(), 0.0, a.sin());
        let spine = arc(Vec3::Y * 0.05, dir, rng.range(0.5, 0.9), rng.range(0.8, 1.2), 0.3, 6);
        let side = Vec3::Y.cross(dir).normalize();
        m.ribbon(&spine, side, |t| 0.35 * (t * PI * 0.9 + 0.1).sin().max(0.05), color, edge, 0.0);
    }
    [Some(m), None, None]
}

/// Rotation of a template: its Y axis to `up`, turned `yaw` radians about it.
pub fn orient(up: Vec3, yaw: f32) -> Quat {
    Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(yaw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_are_valid_and_within_budget() {
        let g = [0.9, 1.0, 0.85];
        let b = [0.3, 0.2, 0.1];
        let all: Vec<(&str, [Option<Template>; 3], [usize; 3])> = vec![
            ("giant conifer", giant_conifer(g, 1), [8500, 1300, 120]),
            ("conifer", conifer(tex::BARK_SPRUCE, g, 0.3, 2), [4500, 700, 60]),
            ("broadleaf", broadleaf(g, 3), [2000, 450, 80]),
            ("bush", bush(g, Some([0.8, 0.1, 0.1]), 4), [1400, 20, 0]),
            ("fern", fern(g, 7, 5), [60, 0, 0]),
            ("grass", grass(g, 6), [6, 0, 0]),
            ("flower", flower(g, [1.0, 0.8, 0.1], 7), [40, 0, 0]),
            ("boulder", boulder(true, g, 8), [130, 60, 0]),
            ("log", log(), [40, 30, 0]),
            ("cactus", cactus(g, Some([0.8, 0.1, 0.3]), 10), [600, 80, 0]),
            ("fruit tree", fruit_tree(g, Some([0.6, 0.1, 0.05]), 18), [3500, 1200, 80]),
            ("dry shrub", dry_shrub(b, g, 11), [80, 10, 0]),
            ("mushroom", mushroom(b, [0.8, 0.2, 0.1], 12), [140, 0, 0]),
            ("alien mushroom tree", alien_mushroom_tree(b, g, b, 13), [500, 140, 60]),
            ("alien spire", alien_spire(b, g, 1.0, 14), [250, 60, 20]),
            ("alien tendril", alien_tendril(b, g, 3, 15), [800, 200, 40]),
            ("alien bulbs", alien_bulbs(b, g, 16), [500, 60, 0]),
            ("alien fan", alien_fan(b, g, 17), [80, 0, 0]),
        ];
        for (name, tiers, budget) in all {
            for (i, t) in tiers.iter().enumerate() {
                let Some(t) = t else { continue };
                for p in [&t.solid, &t.cards, &t.fill] {
                    assert!(p.idx.iter().all(|&k| (k as usize) < p.pos.len()), "{name} tier {i} bad index");
                    assert!(p.pos.iter().all(|v| v.is_finite()), "{name} tier {i} non-finite vertex");
                    assert!(p.uv.iter().all(|v| v.is_finite()) && p.tan.iter().all(|v| v.is_finite()));
                    let n = p.pos.len();
                    assert!(p.nrm.len() == n && p.tan.len() == n && p.uv.len() == n && p.uv2.len() == n && p.col.len() == n);
                }
                assert!(t.tris() > 0, "{name} tier {i} is empty");
                assert!(t.tris() <= budget[i], "{name} tier {i}: {} triangles > budget {}", t.tris(), budget[i]);
            }
        }
    }
}
