//! Flora: plant species, where they grow, and turning them into geometry and colliders.
//!
//! ## Placement
//! Each species belongs to a [`Layer`] (canopy trees, understory, ground plants, small detail).
//! Every layer has its own global grid on each cube face; each grid cell gets at most one
//! plant, chosen by the species' densities at that spot (so species compete for space). The
//! choice depends only on the cell and the LOD-independent [`Ecology`], never on which terrain
//! chunk is asking, so a tree is the same tree at every level of detail and in the collision.
//!
//! ## Rendering
//! A terrain chunk renders the instances of the layers allowed at its level (big trees from
//! far away, ferns and grass only close up), as one merged mesh, using the tier of detail that
//! suits its level. Plants sit on the chunk's own surface so they never float above it.

use std::sync::Arc;

use bevy::math::{DQuat, DVec3, Quat, Vec3};

use crate::life::Biosphere;
use crate::plants::{Part, Template, orient};
use crate::terrain::{ChunkKey, Ecology, RADIUS, Terrain, face_dir, face_uv_of, smoothstep};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Layer {
    /// Big trees: visible from kilometres away.
    Canopy,
    /// Small trees, bushes, boulders, logs.
    Understory,
    /// Ferns, shrubs, mushrooms.
    Ground,
    /// Grass and flowers, only right around the viewer.
    Detail,
}

pub const LAYERS: [Layer; 4] = [Layer::Canopy, Layer::Understory, Layer::Ground, Layer::Detail];

impl Layer {
    /// Grid resolution: 2^k cells along a cube-face edge (cell size ~10 m / 5 m / 2.4 m / 1.2 m).
    pub fn grid_level(self) -> u8 {
        match self {
            Layer::Canopy => 20,
            Layer::Understory => 21,
            Layer::Ground => 22,
            Layer::Detail => 23,
        }
    }

    /// Coarsest terrain chunk level that draws this layer.
    pub fn min_chunk_level(self) -> u8 {
        match self {
            // Distant trees are 2-triangle impostors, so they can reach ~10 km.
            Layer::Canopy => 12,
            Layer::Understory => 16,
            Layer::Ground => 17,
            Layer::Detail => 17,
        }
    }

    fn salt(self) -> u64 {
        self as u64 + 1
    }
}

/// Where a species grows: trapezoid preferences (min, optimum low, optimum high, max).
#[derive(Clone, Copy, Debug)]
pub struct Envelope {
    pub temperature: (f32, f32, f32, f32),
    pub moisture: (f32, f32, f32, f32),
    /// Fades out above this height (m).
    pub max_height: f32,
    /// Extra growth near rivers (multiplied by wetness).
    pub river_bonus: f32,
}

fn trapezoid(x: f32, (a, b, c, d): (f32, f32, f32, f32)) -> f32 {
    if x <= a || x >= d {
        0.0
    } else if x < b {
        (x - a) / (b - a)
    } else if x <= c {
        1.0
    } else {
        (d - x) / (d - c)
    }
}

impl Envelope {
    pub fn eval(&self, e: &Ecology) -> f32 {
        let alt = 1.0 - smoothstep((self.max_height * 0.7) as f64, self.max_height as f64, e.height as f64) as f32;
        trapezoid(e.temperature, self.temperature) * trapezoid(e.moisture, self.moisture) * alt * (1.0 + self.river_bonus * e.wetness)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FloraCollision {
    /// Vertical cylinder from the base.
    Trunk { radius: f32, height: f32 },
    /// Sphere centred `y` above the base.
    Sphere { radius: f32, y: f32 },
    /// Horizontal capsule along the plant's local X axis.
    Log { radius: f32, length: f32 },
}

/// How a species colours the ground where it grows (seen through gaps, and from far away
/// once the plants themselves are no longer drawn).
#[derive(Clone, Copy, Debug)]
pub struct GroundEffect {
    /// Terrain texture layer these plants leave on the ground (see `ground`).
    pub layer: usize,
    /// How strongly they cover the ground, 0..1.
    pub cover: f32,
    /// Trees: their canopy stands in for them on distant terrain.
    pub canopy: bool,
}

/// Terrain texture layers (order of textures::TERRAIN_LAYERS).
pub mod ground {
    pub const GRASS: usize = 0;
    pub const LITTER: usize = 1;
    pub const MOSS: usize = 2;
    pub const DIRT: usize = 3;
    pub const SAND: usize = 4;
    pub const ROCK: usize = 5;
    pub const SNOW: usize = 6;
    pub const LAND_LAYERS: usize = 7;
}

pub struct FloraSpecies {
    pub name: String,
    pub layer: Layer,
    pub envelope: Envelope,
    /// Chance that a grid cell holds this species where the envelope is 1.
    pub density: f32,
    pub scale: (f32, f32),
    /// Minimum cos(slope) to grow on (1 = only flat ground).
    pub min_slope_cos: f32,
    /// 0 = grows straight up, 1 = perpendicular to the slope.
    pub align: f32,
    pub collision: Option<FloraCollision>,
    pub ground: GroundEffect,
    /// Meshes for tiers 0..2 (close, medium, far).
    pub tiers: [Option<Arc<Template>>; 3],
    /// What the player can gather from it, if anything.
    pub forage: Option<Forage>,
    /// Timber: the log item and how many logs a full-size tree gives, and its stump.
    pub timber: Option<Timber>,
}

#[derive(Clone)]
pub struct Timber {
    pub log: crate::items::ItemId,
    pub logs: u32,
    pub stump: Option<Arc<Template>>,
}

/// Something to gather from a plant (berries, fruit, mushrooms, herbs).
#[derive(Clone)]
pub struct Forage {
    pub item: crate::items::ItemId,
    /// Range of how many you get.
    pub amount: (u32, u32),
    /// Prompt, e.g. "Pick berries".
    pub verb: &'static str,
    /// Seconds until it can be harvested again.
    pub regrow: f64,
    /// Rare extra finds: (item, chance per harvest).
    pub bonus: Vec<(crate::items::ItemId, f32)>,
    /// How far from the plant's base you can reach it (m, before scale).
    pub reach: f32,
    /// Height of the part you pick, above the base (m, before scale).
    pub height: f32,
    /// Meshes once picked (a bush without berries); `None`: the plant is gone until it regrows.
    pub bare: Option<[Option<Arc<Template>>; 3]>,
}

impl FloraSpecies {
    /// Make the species a source of timber.
    pub fn timber(mut self, log: crate::items::ItemId, logs: u32, stump: Template) -> Self {
        self.timber = Some(Timber { log, logs, stump: Some(Arc::new(stump)) });
        self
    }

    /// Make the species foragable.
    pub fn forage(mut self, f: Forage) -> Self {
        self.forage = Some(f);
        self
    }

    pub fn density_at(&self, e: &Ecology) -> f32 {
        self.density * self.envelope.eval(e)
    }

    fn tier(&self, want: usize) -> Option<&Arc<Template>> {
        // Prefer the requested tier, then coarser ones, then finer ones.
        (want..3).chain((0..want).rev()).find_map(|t| self.tiers[t].as_ref())
    }
}

/// One plant.
#[derive(Clone, Copy, Debug)]
pub struct Instance {
    pub species: usize,
    pub u: f64,
    pub v: f64,
    pub dir: DVec3,
    pub yaw: f32,
    pub scale: f32,
    /// 0..1 brightness variation.
    pub tint: f32,
    /// 0..1, used to keep only the most prominent plants far away.
    pub rank: f32,
    /// Unique id of the placement cell (layer, face, i, j): identifies this plant for state
    /// kept outside the procedural world, like "picked".
    pub cell: u64,
}

#[inline]
fn hash(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[inline]
fn unit(h: u64, k: u32) -> f32 {
    ((hash(h.wrapping_add((k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))) >> 40) as f32) / (1u64 << 24) as f32
}

/// A circle on the planet where nothing grows (landing sites, later settlements).
#[derive(Clone, Copy, Debug)]
pub struct Clearing {
    pub dir: DVec3,
    pub radius: f64,
}

/// Everything plant placement needs, shared with the worker threads.
pub struct FloraContext {
    pub terrain: Arc<Terrain>,
    pub bio: Biosphere,
    pub seed: u64,
    pub clearings: Vec<Clearing>,
    /// Species indices per layer.
    by_layer: [Vec<usize>; 4],
    /// Plants that have been picked (cell ids), so chunks are meshed with their bare form.
    pub harvested: Arc<std::sync::RwLock<std::collections::HashSet<u64>>>,
    /// Impostor slot and bounds per species (big plants only); see impostor.rs.
    pub impostors: Vec<Option<crate::impostor::ImpostorInfo>>,
}

impl FloraContext {
    pub fn new(terrain: Arc<Terrain>, bio: Biosphere, seed: u64, clearings: Vec<Clearing>) -> Self {
        let mut by_layer: [Vec<usize>; 4] = Default::default();
        for (i, s) in bio.flora.iter().enumerate() {
            by_layer[LAYERS.iter().position(|l| *l == s.layer).unwrap()].push(i);
        }
        let impostors = crate::impostor::layout(&bio);
        Self { terrain, bio, seed, clearings, by_layer, impostors, harvested: Default::default() }
    }


    fn layer_species(&self, layer: Layer) -> &[usize] {
        &self.by_layer[LAYERS.iter().position(|l| *l == layer).unwrap()]
    }

    /// The plant (if any) in grid cell (i, j) of `layer` on `face`. Deterministic.
    pub fn cell_instance(&self, layer: Layer, face: u8, i: u64, j: u64) -> Option<Instance> {
        let candidates = self.layer_species(layer);
        if candidates.is_empty() {
            return None;
        }
        let h = hash(self.seed ^ layer.salt().wrapping_mul(0x1000_0000_01B3) ^ ((face as u64) << 58) ^ (i << 29) ^ j);
        let cell = 2.0 / (1u64 << layer.grid_level()) as f64;
        let u = -1.0 + (i as f64 + 0.1 + 0.8 * unit(h, 0) as f64) * cell;
        let v = -1.0 + (j as f64 + 0.1 + 0.8 * unit(h, 1) as f64) * cell;
        let dir = face_dir(face, u, v);
        let eco = self.terrain.ecology(dir);
        if eco.height < 1.0 {
            return None;
        }
        let mut total = 0.0;
        let mut dens = [0.0f32; 16];
        for (k, &s) in candidates.iter().enumerate().take(16) {
            dens[k] = self.bio.flora[s].density_at(&eco);
            total += dens[k];
        }
        let roll = unit(h, 2);
        if roll >= total.min(1.0) {
            return None;
        }
        // Pick a species in proportion to its density.
        let mut pick = unit(h, 3) * total;
        let mut chosen = candidates[0];
        for (k, &s) in candidates.iter().enumerate().take(16) {
            if pick < dens[k] {
                chosen = s;
                break;
            }
            pick -= dens[k];
        }
        if self.clearings.iter().any(|c| c.dir.angle_between(dir) * RADIUS < c.radius) {
            return None;
        }
        let sp = &self.bio.flora[chosen];
        let size = unit(h, 4);
        Some(Instance {
            species: chosen,
            u,
            v,
            dir,
            yaw: unit(h, 5) * std::f32::consts::TAU,
            scale: sp.scale.0 + (sp.scale.1 - sp.scale.0) * size,
            tint: unit(h, 6),
            rank: size,
            cell: (layer as u64) << 62 | (face as u64) << 59 | i << 29 | j,
        })
    }

    /// All plants of `layer` inside a terrain chunk.
    pub fn chunk_instances(&self, layer: Layer, key: ChunkKey) -> impl Iterator<Item = Instance> + '_ {
        let shift = layer.grid_level() - key.level;
        let (x0, y0) = ((key.x as u64) << shift, (key.y as u64) << shift);
        let n = 1u64 << shift;
        (0..n * n).filter_map(move |k| self.cell_instance(layer, key.face, x0 + k % n, y0 + k / n))
    }

    /// Ground texture weights the vegetation growing here leaves (litter under conifers, moss
    /// under ferns, grass in meadows...), and how much of it is tree canopy (shown on distant
    /// terrain where the trees themselves are no longer drawn).
    pub fn ground_cover(&self, eco: &Ecology) -> ([f32; ground::LAND_LAYERS], f32) {
        let mut w = [0.0f32; ground::LAND_LAYERS];
        let mut canopy = 0.0f32;
        for s in &self.bio.flora {
            let d = s.density_at(eco) * s.ground.cover;
            if d <= 0.0 {
                continue;
            }
            w[s.ground.layer] += d;
            if s.ground.canopy {
                canopy += d;
            }
        }
        (w, canopy.min(1.0))
    }
}

/// Merged geometry, relative to the chunk centre.
#[derive(Default)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    /// x = texture-array layer, y = reserved (wind).
    pub uvs_b: Vec<[f32; 2]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub instances: usize,
}

/// A chunk's plants, split by material (textured bark/rock vs alpha-tested foliage cards) and
/// by size: big ones (trees, bushes, rocks) cast shadows, small ones (ferns, grass, flowers)
/// are too small for their shadows to be worth drawing.
#[derive(Default)]
pub struct FloraMesh {
    pub big_solid: MeshData,
    pub big_cards: MeshData,
    /// Crown fill cards of big plants: drawn, but no shadows.
    pub big_fill: MeshData,
    pub small_solid: MeshData,
    pub small_cards: MeshData,
    /// One camera-facing quad per distant big plant (impostor.wgsl).
    pub impostors: MeshData,
    /// Triangles per layer (canopy, understory, ground, detail), for profiling.
    pub tris_by_layer: [usize; 4],
}

impl MeshData {
    /// An impostor quad: the tree's bounding-sphere centre at every corner, the corner in uv,
    /// (radius, slot + yaw / TAU) in uv_b and the brightness tint in the colour.
    fn impostor(&mut self, center: Vec3, up: Vec3, radius: f32, width: f32, slot: u32, yaw: f32, tint: f32) {
        let base = self.positions.len() as u32;
        let k = 0.85 + 0.3 * tint;
        let yaw_frac = (yaw.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU).min(0.999);
        for corner in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            self.positions.push(center.to_array());
            self.normals.push(up.to_array());
            self.uvs.push(corner);
            self.uvs_b.push([radius, slot as f32 + yaw_frac]);
            self.colors.push([k, width, 0.0, 1.0]);
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        self.instances += 1;
    }

    pub(crate) fn append(&mut self, t: &Part, offset: Vec3, rot: Quat, scale: f32, tint: f32) {
        let base = self.positions.len() as u32;
        let k = 0.85 + 0.3 * tint;
        for i in 0..t.pos.len() {
            self.positions.push((offset + rot * (t.pos[i] * scale)).to_array());
            self.normals.push((rot * t.nrm[i]).to_array());
            let tan = rot * t.tan[i].truncate();
            self.tangents.push([tan.x, tan.y, tan.z, t.tan[i].w]);
            self.uvs.push(t.uv[i].to_array());
            self.uvs_b.push(t.uv2[i].to_array());
            let c = t.col[i];
            self.colors.push([c[0] * k, c[1] * k, c[2] * k, 1.0]);
        }
        self.indices.extend(t.idx.iter().map(|i| i + base));
        self.instances += 1;
    }
}

/// The chunk's sampled surface, for setting plants on it: a (g × g) grid with a one-cell
/// border around the chunk's own vertices.
pub struct SurfaceGrid<'a> {
    pub key: ChunkKey,
    pub g: usize,
    pub heights: &'a [f64],
    pub normals: &'a [DVec3],
    /// Inland water level per grid point (-inf where there is none).
    pub water: &'a [f64],
}

impl SurfaceGrid<'_> {
    /// Bilinear height and normal at face coordinates (u, v) inside the chunk.
    fn at(&self, u: f64, v: f64) -> (f64, DVec3) {
        let (u0, v0) = self.key.uv0();
        let step = self.key.size() / (self.g - 3) as f64;
        let x = ((u - u0) / step + 1.0).clamp(0.0, (self.g - 1) as f64 - 1e-6);
        let y = ((v - v0) / step + 1.0).clamp(0.0, (self.g - 1) as f64 - 1e-6);
        let (ix, iy) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = (x - ix as f64, y - iy as f64);
        let i = |a: usize, b: usize| b * self.g + a;
        let (a, b, c, d) = (i(ix, iy), i(ix + 1, iy), i(ix, iy + 1), i(ix + 1, iy + 1));
        let lerp2 = |p: f64, q: f64, r: f64, s: f64| (p + (q - p) * fx) + ((r + (s - r) * fx) - (p + (q - p) * fx)) * fy;
        let h = lerp2(self.heights[a], self.heights[b], self.heights[c], self.heights[d]);
        let n = (self.normals[a].lerp(self.normals[b], fx)).lerp(self.normals[c].lerp(self.normals[d], fx), fy);
        (h, n.normalize_or(DVec3::Y))
    }

    /// Highest inland water level around (u, v), or -inf.
    fn water_at(&self, u: f64, v: f64) -> f64 {
        let (u0, v0) = self.key.uv0();
        let step = self.key.size() / (self.g - 3) as f64;
        let x = ((u - u0) / step + 1.0).clamp(0.0, (self.g - 1) as f64 - 1e-6) as usize;
        let y = ((v - v0) / step + 1.0).clamp(0.0, (self.g - 1) as f64 - 1e-6) as usize;
        let i = |a: usize, b: usize| b * self.g + a;
        self.water[i(x, y)].max(self.water[i(x + 1, y)]).max(self.water[i(x, y + 1)]).max(self.water[i(x + 1, y + 1)])
    }
}

/// Detail tier for a chunk level: 0 close (within ~230 m), 1 medium (~230-450 m), then
/// impostors for big plants (IMPOSTOR_TIER; mesh tier 2 for species without one).
fn tier_for_level(level: u8) -> usize {
    if level >= 18 {
        0
    } else if level >= 17 {
        1
    } else {
        IMPOSTOR_TIER
    }
}

const IMPOSTOR_TIER: usize = 3;

/// Far away, only the most prominent plants (highest rank) are drawn, thinning with distance.
fn min_rank(layer: Layer, level: u8) -> f32 {
    match (layer, level) {
        (Layer::Canopy, 12) => 0.94,
        (Layer::Canopy, 13) => 0.86,
        (Layer::Canopy, 14) => 0.68,
        (Layer::Canopy, 15) => 0.42,
        (Layer::Understory, 16) => 0.5,
        // A sparse ring of small plants beyond the full-density zone softens its edge.
        (Layer::Ground, 17) => 0.75,
        (Layer::Detail, 17) => 0.8,
        _ => 0.0,
    }
}

/// Build the merged plant mesh for a terrain chunk.
pub fn build_chunk_flora(ctx: &FloraContext, surface: &SurfaceGrid, center: DVec3) -> FloraMesh {
    let key = surface.key;
    let mut out = FloraMesh::default();
    let tier = tier_for_level(key.level);
    let harvested = ctx.harvested.read().unwrap();
    for layer in LAYERS {
        if key.level < layer.min_chunk_level() || ctx.layer_species(layer).is_empty() {
            continue;
        }
        let min_rank = min_rank(layer, key.level);
        for inst in ctx.chunk_instances(layer, key) {
            if inst.rank < min_rank {
                continue;
            }
            let sp = &ctx.bio.flora[inst.species];
            let (h, n) = surface.at(inst.u, inst.v);
            // Not under water (sea, rivers, lakes), not on cliffs.
            if h < 0.3 || h < surface.water_at(inst.u, inst.v) + 0.3 || n.dot(inst.dir) < sp.min_slope_cos as f64 {
                continue;
            }
            // Small plants always use their full shape (their coarse shapes read as blobs).
            let big = matches!(layer, Layer::Canopy | Layer::Understory);
            let mut want = if big { tier } else { 0 };
            // Small trees and bushes are only ~100 px tall beyond 230 m: impostors already.
            if want == 1 && ctx.impostors[inst.species].is_some_and(|i| i.radius * inst.scale < 16.0) {
                want = IMPOSTOR_TIER;
            }
            // Sink the base a little so roots don't show on slopes.
            let base = inst.dir * (RADIUS + h - 0.15 * inst.scale as f64) - center;
            let up = inst.dir.lerp(n, sp.align as f64).normalize().as_vec3();
            if want == IMPOSTOR_TIER && !(gone_early(&harvested, &inst) && sp.timber.is_some()) {
                if let Some(info) = ctx.impostors[inst.species] {
                    let c = base.as_vec3() + up * (info.center_y * inst.scale);
                    out.impostors.impostor(c, up, info.radius * inst.scale, info.width, info.slot, inst.yaw, inst.tint);
                    out.tris_by_layer[layer as usize] += 2;
                    continue;
                }
            }
            // Picked plants: their bare form (or nothing) until they regrow; felled trees: a stump.
            let gone = harvested.contains(&inst.cell);
            let t = if gone && sp.timber.is_some() {
                if want == IMPOSTOR_TIER { None } else { sp.timber.as_ref().and_then(|t| t.stump.as_ref()) }
            } else {
                match sp.forage.as_ref().filter(|_| gone) {
                    Some(f) => match &f.bare {
                        Some(bare) => (want.min(2)..3).chain((0..want.min(2)).rev()).find_map(|k| bare[k].as_ref()),
                        None => None,
                    },
                    None => sp.tier(want.min(2)),
                }
            };
            let Some(t) = t else { continue };
            let rot = orient(up, inst.yaw);
            let (solid, cards) = if big { (&mut out.big_solid, &mut out.big_cards) } else { (&mut out.small_solid, &mut out.small_cards) };
            solid.append(&t.solid, base.as_vec3(), rot, inst.scale, inst.tint);
            cards.append(&t.cards, base.as_vec3(), rot, inst.scale, inst.tint);
            if big {
                out.big_fill.append(&t.fill, base.as_vec3(), rot, inst.scale, inst.tint);
            } else {
                cards.append(&t.fill, base.as_vec3(), rot, inst.scale, inst.tint);
            }
            out.tris_by_layer[layer as usize] += t.solid.tris() + t.cards.tris() + t.fill.tris();
        }
    }
    out
}

fn gone_early(harvested: &std::collections::HashSet<u64>, inst: &Instance) -> bool {
    harvested.contains(&inst.cell)
}

/// A standing timber tree near a point.
pub struct TreeAt {
    pub cell: u64,
    pub species: usize,
    pub base: DVec3,
    pub scale: f32,
}

/// Standing (not felled) timber trees within `radius` m of `center`.
pub fn trees_near(ctx: &FloraContext, center: DVec3, radius: f64) -> Vec<TreeAt> {
    let mut out = Vec::new();
    let up = center.normalize();
    let (face, _, _) = crate::terrain::dir_to_face_uv(up);
    let e1 = up.any_orthonormal_vector();
    let e2 = up.cross(e1);
    let corners = [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)].map(|(a, b)| face_uv_of(face, (center + (e1 * a + e2 * b) * radius).normalize()));
    let (umin, umax) = (corners.iter().map(|c| c.0).fold(f64::MAX, f64::min), corners.iter().map(|c| c.0).fold(f64::MIN, f64::max));
    let (vmin, vmax) = (corners.iter().map(|c| c.1).fold(f64::MAX, f64::min), corners.iter().map(|c| c.1).fold(f64::MIN, f64::max));
    let harvested = ctx.harvested.read().unwrap();
    for layer in [Layer::Canopy, Layer::Understory] {
        let cells = (1u64 << layer.grid_level()) as f64;
        let to_cell = |t: f64| (((t.clamp(-1.0, 1.0) + 1.0) * 0.5 * cells).floor() as i64).clamp(0, cells as i64 - 1) as u64;
        for j in to_cell(vmin)..=to_cell(vmax) {
            for i in to_cell(umin)..=to_cell(umax) {
                let Some(inst) = ctx.cell_instance(layer, face, i, j) else { continue };
                if ctx.bio.flora[inst.species].timber.is_none() || harvested.contains(&inst.cell) {
                    continue;
                }
                let h = ctx.terrain.height(inst.dir);
                if h < 0.3 || ctx.terrain.water(inst.dir).is_some_and(|w| h < w.level + 0.3) {
                    continue;
                }
                let base = inst.dir * (RADIUS + h);
                if (base - center).length() <= radius {
                    out.push(TreeAt { cell: inst.cell, species: inst.species, base, scale: inst.scale });
                }
            }
        }
    }
    out
}

/// A plant you can gather from, near the player.
pub struct Forageable {
    pub cell: u64,
    pub species: usize,
    /// Base of the plant on the ground.
    pub base: DVec3,
    /// The part you pick (e.g. the middle of a bush).
    pub point: DVec3,
    pub reach: f64,
    pub picked: bool,
}

/// Foragable plants within `half` metres (square) of `center`.
pub fn forageables_near(ctx: &FloraContext, center: DVec3, half: f64) -> Vec<Forageable> {
    let mut out = Vec::new();
    let up = center.normalize();
    let (face, _, _) = crate::terrain::dir_to_face_uv(up);
    let e1 = up.any_orthonormal_vector();
    let e2 = up.cross(e1);
    let corners = [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
        .map(|(a, b)| face_uv_of(face, (center + (e1 * a + e2 * b) * half).normalize()));
    let (umin, umax) = (corners.iter().map(|c| c.0).fold(f64::MAX, f64::min), corners.iter().map(|c| c.0).fold(f64::MIN, f64::max));
    let (vmin, vmax) = (corners.iter().map(|c| c.1).fold(f64::MAX, f64::min), corners.iter().map(|c| c.1).fold(f64::MIN, f64::max));
    let harvested = ctx.harvested.read().unwrap();
    for layer in LAYERS {
        if !ctx.layer_species(layer).iter().any(|&s| ctx.bio.flora[s].forage.is_some()) {
            continue;
        }
        let cells = (1u64 << layer.grid_level()) as f64;
        let to_cell = |t: f64| (((t.clamp(-1.0, 1.0) + 1.0) * 0.5 * cells).floor() as i64).clamp(0, cells as i64 - 1) as u64;
        for j in to_cell(vmin)..=to_cell(vmax) {
            for i in to_cell(umin)..=to_cell(umax) {
                let Some(inst) = ctx.cell_instance(layer, face, i, j) else { continue };
                let sp = &ctx.bio.flora[inst.species];
                let Some(f) = &sp.forage else { continue };
                let h = ctx.terrain.height(inst.dir);
                if h < 0.3 || ctx.terrain.water(inst.dir).is_some_and(|w| h < w.level + 0.3) {
                    continue;
                }
                let base = inst.dir * (RADIUS + h - 0.15 * inst.scale as f64);
                let s = inst.scale as f64;
                out.push(Forageable {
                    cell: inst.cell,
                    species: inst.species,
                    base,
                    point: base + inst.dir * (f.height as f64 * s),
                    reach: f.reach as f64 * s.max(0.7),
                    picked: harvested.contains(&inst.cell),
                });
            }
        }
    }
    out
}

/// A collision shape for a plant, in world space.
pub struct PlantCollider {
    pub pos: DVec3,
    pub rot: DQuat,
    pub shape: FloraCollision,
    pub scale: f32,
}

/// Colliders of the plants within `half` metres (square) of `center` (a point on the
/// surface), with bases on the exact terrain height.
pub fn colliders_near(ctx: &FloraContext, center: DVec3, half: f64) -> Vec<PlantCollider> {
    let mut out = Vec::new();
    let up = center.normalize();
    let (face, _, _) = crate::terrain::dir_to_face_uv(up);
    let e1 = up.any_orthonormal_vector();
    let e2 = up.cross(e1);
    let corners = [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
        .map(|(a, b)| face_uv_of(face, (center + (e1 * a + e2 * b) * half).normalize()));
    let (umin, umax) = (corners.iter().map(|c| c.0).fold(f64::MAX, f64::min), corners.iter().map(|c| c.0).fold(f64::MIN, f64::max));
    let (vmin, vmax) = (corners.iter().map(|c| c.1).fold(f64::MAX, f64::min), corners.iter().map(|c| c.1).fold(f64::MIN, f64::max));
    for layer in [Layer::Canopy, Layer::Understory] {
        if !ctx.layer_species(layer).iter().any(|&s| ctx.bio.flora[s].collision.is_some()) {
            continue;
        }
        let cells = (1u64 << layer.grid_level()) as f64;
        let to_cell = |t: f64| (((t.clamp(-1.0, 1.0) + 1.0) * 0.5 * cells).floor() as i64).clamp(0, cells as i64 - 1) as u64;
        for j in to_cell(vmin)..=to_cell(vmax) {
            for i in to_cell(umin)..=to_cell(umax) {
                let Some(inst) = ctx.cell_instance(layer, face, i, j) else { continue };
                let sp = &ctx.bio.flora[inst.species];
                let Some(shape) = sp.collision else { continue };
                if sp.timber.is_some() && ctx.harvested.read().unwrap().contains(&inst.cell) {
                    continue;
                }
                let h = ctx.terrain.height(inst.dir);
                if h < 0.3 || ctx.terrain.water(inst.dir).is_some_and(|w| h < w.level + 0.3) {
                    continue;
                }
                let pos = inst.dir * (RADIUS + h - 0.15 * inst.scale as f64);
                let rot = orient(inst.dir.as_vec3(), inst.yaw).as_dquat();
                out.push(PlantCollider { pos, rot, shape, scale: inst.scale });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> FloraContext {
        let terrain = Arc::new(Terrain::new(1337));
        FloraContext::new(terrain, Biosphere::earthlike(1337), 1337, Vec::new())
    }

    #[test]
    fn placement_is_the_same_at_every_level_of_detail() {
        // A chunk's plants must be exactly its four children's plants.
        let ctx = ctx();
        let parent = ChunkKey { face: 4, level: 16, x: 41_000, y: 23_000 };
        for layer in [Layer::Canopy, Layer::Understory] {
            let mut a: Vec<(i64, i64)> =
                ctx.chunk_instances(layer, parent).map(|i| ((i.u * 1e9) as i64, (i.v * 1e9) as i64)).collect();
            let mut b: Vec<(i64, i64)> = parent
                .children()
                .iter()
                .flat_map(|c| ctx.chunk_instances(layer, *c).collect::<Vec<_>>())
                .map(|i| ((i.u * 1e9) as i64, (i.v * 1e9) as i64))
                .collect();
            a.sort();
            b.sort();
            assert_eq!(a, b, "{layer:?} differs between levels");
        }
    }

    #[test]
    fn forests_exist_and_trees_are_spaced() {
        // Find a dense giant-conifer forest and check its tree spacing.
        let ctx = ctx();
        let giant = ctx.bio.flora.iter().position(|s| s.name.contains("giant")).unwrap();
        let mut best = (0.0, DVec3::ZERO);
        for k in 0..20_000 {
            let y = 1.0 - 2.0 * (k as f64 + 0.5) / 20_000.0;
            let r = (1.0 - y * y).sqrt();
            let d = DVec3::new(r * (k as f64 * 2.4).cos(), y, r * (k as f64 * 2.4).sin());
            let e = ctx.terrain.ecology(d);
            let dens = ctx.bio.flora[giant].density_at(&e);
            if e.height > 1.0 && dens > best.0 {
                best = (dens, d);
            }
        }
        println!("densest giant-conifer spot: {:.2}", best.0);
        assert!(best.0 > 0.3, "no giant conifer forest on the planet");
        let (face, u, v) = crate::terrain::dir_to_face_uv(best.1);
        let key = ChunkKey { face, level: 15, x: ((u + 1.0) * 0.5 * 32768.0) as u32, y: ((v + 1.0) * 0.5 * 32768.0) as u32 };
        let trees: Vec<_> = ctx.chunk_instances(Layer::Canopy, key).collect();
        println!("{} canopy trees in a 305 m chunk", trees.len());
        assert!(trees.len() > 100);
    }
}

#[cfg(test)]
mod biome_stats {
    use super::*;
    use crate::life::Biosphere;

    /// Share of the land where each canopy species dominates (or no trees). For tuning the
    /// climate envelopes: `cargo test --release biome_shares -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn biome_shares() {
        let terrain = Arc::new(Terrain::with_erosion(crate::planet::SEED));
        let bio = Biosphere::earthlike(crate::planet::SEED);
        let n = 20_000;
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        let (mut land, mut tmin, mut tmax, mut mmin, mut mmax) = (0, f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for k in 0..n {
            let y = 1.0 - 2.0 * (k as f64 + 0.5) / n as f64;
            let r = (1.0 - y * y).sqrt();
            let d = DVec3::new(r * (k as f64 * 2.399_963).cos(), y, r * (k as f64 * 2.399_963).sin());
            let e = terrain.ecology(d);
            if e.height < 1.0 {
                continue;
            }
            land += 1;
            tmin = tmin.min(e.temperature);
            tmax = tmax.max(e.temperature);
            mmin = mmin.min(e.moisture);
            mmax = mmax.max(e.moisture);
            let best = bio
                .flora
                .iter()
                .filter(|s| matches!(s.layer, Layer::Canopy) || s.name == "cactus" || s.name == "grass" || s.name == "tundra shrub")
                .map(|s| (s.envelope.eval(&e), s.name.clone()))
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .unwrap();
            let name = if best.0 < 0.2 { "bare".to_string() } else { best.1 };
            *counts.entry(name).or_default() += 1;
        }
        println!("land {land} samples, temperature {tmin:.0}..{tmax:.0} C, moisture {mmin:.2}..{mmax:.2}");
        for (k, v) in counts {
            println!("{k:>16}: {:5.1}%", 100.0 * v as f64 / land as f64);
        }
    }
}
