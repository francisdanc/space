//! Quadtree LOD for the planet: decides which chunks are needed, generates them on worker
//! threads, and swaps the visible set only once every needed chunk is ready (no holes).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

use crate::camera::CameraWorld;
use crate::flora::{FloraContext, MeshData};
use crate::graphics::{FloraSolidMaterial, FoliageMaterial, ImpostorMaterial, TerrainMaterial, flora_solid_material, foliage_material, impostor_material, terrain_material};
use crate::textures::Textures;
use crate::terrain::{ChunkKey, ChunkMesh, MAX_LEVEL, RADIUS, Terrain, build_chunk};

/// World seed: change it to get a different planet.
pub const SEED: u64 = 1337;
/// Split a chunk when the camera is closer than this many chunk-widths.
pub(crate) const SPLIT_FACTOR: f64 = 3.0;
/// Keep at most this many chunks (shown + cached) alive.
const MAX_CACHE: usize = 1200;

/// A position in the true (f64) world. The floating-origin system turns it into a Transform.
#[derive(Component)]
pub struct WorldPos(pub DVec3);

enum Slot {
    Pending(Task<PreparedChunk>),
    Ready(Entity),
}

/// A chunk's meshes, assembled on the worker thread (including the bounds, which need a pass
/// over every vertex), so the main thread only has to hand them to the renderer.
struct PreparedChunk {
    center: DVec3,
    terrain: Mesh,
    /// (mesh, bounds, is cards, casts shadows)
    parts: Vec<(Mesh, bevy::render::primitives::Aabb, bool, bool)>,
    impostors: Option<(Mesh, bevy::render::primitives::Aabb)>,
    water: Option<(Mesh, bevy::render::primitives::Aabb)>,
    flora_tris: [usize; 4],
}

fn prepare_chunk(key: ChunkKey, data: ChunkMesh) -> PreparedChunk {
    let ChunkMesh { center, positions, normals, colors, uv0, uv1, indices, flora, water } = data;
    let water = (!water.positions.is_empty()).then(|| {
        let aabb = bevy::render::primitives::Aabb::enclosing(water.positions.iter().map(|p| Vec3::from(*p))).unwrap_or_default();
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, water.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, water.normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, water.uvs)
            .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, water.tangents);
        (mesh, aabb)
    });
    let terrain = terrain_mesh(positions, normals, colors, uv0, uv1, indices);
    let flora_tris = flora.tris_by_layer;
    let all = [
        (flora.big_solid, false, !crate::debug::hidden("solid_shadow")),
        // Leaf cards cast shadows only in the nearest ring; further out the trunks carry the
        // shadow shapes, and fill cards never cast any.
        (flora.big_cards, true, key.level >= 18 && !crate::debug::hidden("card_shadow")),
        (flora.big_fill, true, false),
        (flora.small_solid, false, false),
        (flora.small_cards, true, false),
    ];
    let mut parts = Vec::new();
    for (i, (part, is_cards, casts_shadow)) in all.into_iter().enumerate() {
        let name = ["big_solid", "big_cards", "big_fill", "small_solid", "small_cards"][i];
        if part.indices.is_empty() || crate::debug::hidden(name) || crate::debug::hidden("flora") {
            continue;
        }
        let (mut mesh, aabb) = flora_mesh(part);
        if is_cards {
            // Cards don't read the texture-layer channel, and their shader rebuilds the tangent
            // frame from derivatives: 48 instead of 72 bytes per vertex to upload.
            mesh.remove_attribute(Mesh::ATTRIBUTE_UV_1);
            mesh.remove_attribute(Mesh::ATTRIBUTE_TANGENT);
        }
        parts.push((mesh, aabb, is_cards, casts_shadow));
    }
    let show_impostors = std::env::var("SPACE_NO_IMP").is_err() && !crate::debug::hidden("impostors") && !crate::debug::hidden("flora");
    let impostors = (!flora.impostors.indices.is_empty() && show_impostors).then(|| impostor_mesh(flora.impostors));
    PreparedChunk { center, terrain, parts, impostors, water, flora_tris }
}

/// Main-thread time per frame for handing finished chunks to the renderer; the rest wait.
const FINISH_BUDGET_MS: f64 = 3.0;

struct Entry {
    slot: Slot,
    /// The previous version of this chunk, kept on screen while it is being rebuilt.
    replacing: Option<Entity>,
    last_wanted: u64,
    /// Plant triangles in this chunk per layer (for the HUD and profiling).
    flora_tris: [usize; 4],
}

#[derive(Default, Clone, Copy)]
pub struct LodStats {
    pub shown: usize,
    pub cached: usize,
    pub pending: usize,
    pub max_level: u8,
    pub settled: bool,
    /// Plant triangles in the visible chunks.
    pub flora_tris: usize,
    /// Same, per layer (canopy, understory, ground, detail).
    pub flora_layers: [usize; 4],
}

/// The planet's terrain and life, built in `main` before the app starts (the erosion
/// simulation may take a while on first run).
#[derive(Resource, Clone)]
pub struct PlanetInit(pub Arc<FloraContext>);

#[derive(Resource)]
pub struct Planet {
    pub terrain: Arc<Terrain>,
    pub flora: Arc<FloraContext>,
    flora_material: Handle<FoliageMaterial>,
    pub flora_solid_material: Handle<FloraSolidMaterial>,
    centers: CenterCache,
    chunks: HashMap<ChunkKey, Entry>,
    shown: HashSet<ChunkKey>,
    pub terrain_material: Handle<TerrainMaterial>,
    pub impostor_material: Handle<ImpostorMaterial>,
    frame: u64,
    pub stats: LodStats,
}

pub struct PlanetPlugin;

impl Plugin for PlanetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, (crate::textures::load_textures, setup_planet).chain());
    }
}

fn setup_planet(
    mut commands: Commands,
    init: Res<PlanetInit>,
    textures: Res<Textures>,
    mut terrain_materials: ResMut<Assets<TerrainMaterial>>,
    mut solid_materials: ResMut<Assets<FloraSolidMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut impostor_materials: ResMut<Assets<ImpostorMaterial>>,
    mut foliage_materials: ResMut<Assets<FoliageMaterial>>,
) {
    let bio = &init.0.bio;
    let v = |c: [f32; 3]| Vec3::from(c);
    let surface = crate::surface_map::bake(&init.0, &mut images);
    let terrain_material =
        terrain_materials.add(terrain_material(&textures, v(bio.canopy_color), bio.layer_tints.map(v), surface));
    let flora_material = foliage_materials.add(foliage_material(&textures));
    let impostor_material = impostor_materials.add(impostor_material(crate::impostor::bake(&init.0, &mut images)));
    let flora_solid_material = solid_materials.add(flora_solid_material(&textures));
    commands.insert_resource(Planet {
        terrain: init.0.terrain.clone(),
        flora: init.0.clone(),
        flora_material,
        flora_solid_material,
        centers: CenterCache::default(),
        chunks: HashMap::new(),
        shown: HashSet::new(),
        terrain_material,
        impostor_material,
        frame: 0,
        stats: LodStats::default(),
    });
}

/// Surface points at chunk centres, cached: the full terrain height is too expensive to
/// evaluate for thousands of quadtree nodes every frame.
#[derive(Default)]
struct CenterCache(HashMap<ChunkKey, DVec3>);

impl CenterCache {
    fn get(&mut self, t: &Terrain, key: ChunkKey) -> DVec3 {
        if self.0.len() > 200_000 {
            self.0.clear();
        }
        *self.0.entry(key).or_insert_with(|| {
            let d = key.center_dir();
            d * (RADIUS + t.surface_height(d))
        })
    }
}

fn collect(t: &Terrain, cache: &mut CenterCache, key: ChunkKey, cam: DVec3, out: &mut Vec<ChunkKey>) {
    let split = key.level < MAX_LEVEL && (cam - cache.get(t, key)).length() < key.world_size() * SPLIT_FACTOR;
    if split {
        for c in key.children() {
            collect(t, cache, c, cam, out);
        }
    } else {
        out.push(key);
    }
}

fn make_mesh(positions: Vec<[f32; 3]>, normals: Vec<[f32; 3]>, colors: Vec<[f32; 4]>, indices: Vec<u32>, usage: RenderAssetUsages) -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, usage)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_indices(Indices::U32(indices))
}

/// Terrain chunk mesh: texture-layer weights travel in the colour and both UV channels.
pub(crate) fn terrain_mesh(positions: Vec<[f32; 3]>, normals: Vec<[f32; 3]>, colors: Vec<[f32; 4]>, uv0: Vec<[f32; 2]>, uv1: Vec<[f32; 2]>, indices: Vec<u32>) -> Mesh {
    make_mesh(positions, normals, colors, indices, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv0)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, uv1)
}

/// Plant mesh for a chunk: GPU-only (no CPU copy kept), with its bounds computed up front so
/// frustum culling works without the CPU copy.
pub(crate) fn flora_mesh(f: MeshData) -> (Mesh, bevy::render::primitives::Aabb) {
    let aabb = bevy::render::primitives::Aabb::enclosing(f.positions.iter().map(|p| Vec3::from(*p))).unwrap_or_default();
    let mesh = make_mesh(f.positions, f.normals, f.colors, f.indices, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, f.tangents)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, f.uvs)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, f.uvs_b);
    (mesh, aabb)
}

/// Impostor quads: every corner sits at its tree's centre (the shader expands them), so the
/// bounds are grown by the largest radius.
fn impostor_mesh(f: MeshData) -> (Mesh, bevy::render::primitives::Aabb) {
    let r = f.uvs_b.iter().map(|b| b[0]).fold(0.0f32, f32::max) * 1.3;
    let (lo, hi) = f.positions.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| {
        (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p)))
    });
    let aabb = bevy::render::primitives::Aabb::from_min_max(lo - Vec3::splat(r), hi + Vec3::splat(r));
    let mesh = make_mesh(f.positions, f.normals, f.colors, f.indices, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, f.uvs)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, f.uvs_b);
    (mesh, aabb)
}

impl Planet {
    /// Rebuild the near chunks containing `dir` (e.g. after a plant there was picked or has
    /// regrown). The old meshes stay on screen until the new ones are ready.
    pub fn rebuild_at(&mut self, dir: DVec3) {
        let (face, u, v) = crate::terrain::dir_to_face_uv(dir.normalize());
        let pool = AsyncComputeTaskPool::get();
        for level in 15..=crate::terrain::MAX_LEVEL {
            let n = 1u64 << level;
            let to = |t: f64| (((t + 1.0) * 0.5 * n as f64).floor() as u64).min(n - 1) as u32;
            let key = ChunkKey { face, level, x: to(u), y: to(v) };
            if let Some(entry) = self.chunks.get_mut(&key) {
                if let Slot::Ready(old) = entry.slot {
                    let ctx = self.flora.clone();
                    entry.slot = Slot::Pending(pool.spawn(async move { prepare_chunk(key, build_chunk(&ctx, key)) }));
                    entry.replacing = Some(old);
                }
            }
        }
    }
}

pub fn update_lod(
    mut commands: Commands,
    mut planet: ResMut<Planet>,
    cam: Res<CameraWorld>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut prof: ResMut<crate::debug::Prof>,
    water_material: Res<crate::atmosphere::WaterHandle>,
) {
    let t0 = std::time::Instant::now();
    let _guard = crate::debug::ProfGuard(&mut prof, "update_lod", t0);
    let Planet { terrain, flora, flora_material, flora_solid_material, centers, chunks, shown, terrain_material, impostor_material, frame, stats } = &mut *planet;
    *frame += 1;
    let frame = *frame;

    let mut desired = Vec::new();
    for f in 0..6 {
        collect(terrain, centers, ChunkKey::root(f), cam.pos, &mut desired);
    }

    let t_collect = std::time::Instant::now();
    let pool = AsyncComputeTaskPool::get();
    for &k in &desired {
        let e = chunks.entry(k).or_insert_with(|| {
            let ctx = flora.clone();
            Entry { slot: Slot::Pending(pool.spawn(async move { prepare_chunk(k, build_chunk(&ctx, k)) })), replacing: None, last_wanted: 0, flora_tris: [0; 4] }
        });
        e.last_wanted = frame;
    }

    let collect_ms = t_collect.duration_since(t0).as_secs_f64() * 1000.0;
    let t_finish = std::time::Instant::now();
    // Collect finished chunk meshes, within a per-frame time budget.
    for (key, entry) in chunks.iter_mut() {
        if t_finish.elapsed().as_secs_f64() * 1000.0 > FINISH_BUDGET_MS {
            break;
        }
        if let Slot::Pending(task) = &mut entry.slot {
            if let Some(data) = block_on(future::poll_once(task)) {
                let PreparedChunk { center, terrain, parts, impostors, water, flora_tris } = data;
                entry.flora_tris = flora_tris;
                let mut ent = commands.spawn((
                    Mesh3d(meshes.add(terrain)),
                    MeshMaterial3d(terrain_material.clone()),
                    Transform::from_translation((center - cam.pos).as_vec3()),
                    Visibility::Hidden,
                    WorldPos(center),
                ));
                for (mesh, aabb, is_cards, casts_shadow) in parts {
                    let fmesh = meshes.add(mesh);
                    let (cards_mat, solid_mat) = (flora_material.clone(), flora_solid_material.clone());
                    ent.with_children(|c| {
                        let mut e = c.spawn((Mesh3d(fmesh), aabb, Transform::IDENTITY));
                        if is_cards {
                            e.insert(MeshMaterial3d(cards_mat));
                        } else {
                            e.insert(MeshMaterial3d(solid_mat));
                        }
                        if !casts_shadow {
                            e.insert(bevy::pbr::NotShadowCaster);
                        }
                    });
                }
                if let Some((wmesh, aabb)) = water {
                    let wmesh = meshes.add(wmesh);
                    let mat = water_material.0.clone();
                    ent.with_children(|c| {
                        c.spawn((Mesh3d(wmesh), aabb, Transform::IDENTITY, MeshMaterial3d(mat), bevy::pbr::NotShadowCaster));
                    });
                }
                if let Some((imesh, aabb)) = impostors {
                    let imesh = meshes.add(imesh);
                    let mat = impostor_material.clone();
                    ent.with_children(|c| {
                        c.spawn((Mesh3d(imesh), aabb, Transform::IDENTITY, MeshMaterial3d(mat), bevy::pbr::NotShadowCaster));
                    });
                }
                // A rebuilt chunk takes its predecessor's place on screen straight away.
                let replaced = entry.replacing.take();
                if replaced.is_some() && shown.contains(key) {
                    ent.insert(Visibility::Visible);
                }
                entry.slot = Slot::Ready(ent.id());
                if let Some(old) = replaced {
                    commands.entity(old).despawn();
                }
            }
        }
    }

    let finish_ms = t_finish.elapsed().as_secs_f64() * 1000.0;
    if collect_ms > 10.0 || finish_ms > 10.0 {
        debug!("update_lod spike: quadtree {collect_ms:.1} ms, finished chunks {finish_ms:.1} ms");
        if std::env::var("SPACE_PROFILE").is_ok() {
            info!("PROFILE spike: quadtree {collect_ms:.1} ms, finished chunks {finish_ms:.1} ms");
        }
    }
    // Swap the visible set only when every desired chunk is ready.
    let all_ready = desired.iter().all(|k| matches!(chunks[k].slot, Slot::Ready(_)));
    if all_ready {
        let new: HashSet<ChunkKey> = desired.iter().copied().collect();
        if new != *shown {
            for k in shown.difference(&new) {
                if let Some(Entry { slot: Slot::Ready(e), .. }) = chunks.get(k) {
                    commands.entity(*e).insert(Visibility::Hidden);
                }
            }
            for k in new.difference(shown) {
                if let Some(Entry { slot: Slot::Ready(e), .. }) = chunks.get(k) {
                    commands.entity(*e).insert(Visibility::Visible);
                }
            }
            *shown = new;
        }
    }

    // Evict the least recently wanted hidden chunks.
    if chunks.len() > MAX_CACHE {
        let mut cands: Vec<(u64, ChunkKey)> = chunks
            .iter()
            .filter(|(k, e)| !shown.contains(*k) && e.last_wanted != frame && matches!(e.slot, Slot::Ready(_)))
            .map(|(k, e)| (e.last_wanted, *k))
            .collect();
        cands.sort_unstable_by_key(|c| c.0);
        let excess = chunks.len() - MAX_CACHE;
        for (_, k) in cands.into_iter().take(excess) {
            if let Some(Entry { slot: Slot::Ready(e), .. }) = chunks.remove(&k) {
                commands.entity(e).despawn();
            }
        }
    }

    let pending = chunks.values().filter(|e| matches!(e.slot, Slot::Pending(_))).count();
    *stats = LodStats {
        shown: shown.len(),
        cached: chunks.len(),
        pending,
        max_level: shown.iter().map(|k| k.level).max().unwrap_or(0),
        flora_tris: shown.iter().filter_map(|k| chunks.get(k)).map(|e| e.flora_tris.iter().sum::<usize>()).sum(),
        flora_layers: shown.iter().filter_map(|k| chunks.get(k)).fold([0; 4], |mut a, e| {
            for i in 0..4 {
                a[i] += e.flora_tris[i];
            }
            a
        }),
        settled: all_ready && pending == 0,
    };
}

/// Floating origin: the camera always sits at (0,0,0); everything else is placed relative to it.
pub fn apply_floating_origin(cam: Res<CameraWorld>, mut q: Query<(&WorldPos, &mut Transform)>, mut prof: ResMut<crate::debug::Prof>) {
    let _guard = crate::debug::ProfGuard(&mut prof, "floating_origin", std::time::Instant::now());
    for (wp, mut t) in &mut q {
        t.translation = (wp.0 - cam.pos).as_vec3();
    }
}
