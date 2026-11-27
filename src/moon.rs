//! The moon's surface: a cube-sphere quadtree of terrain chunks around the moon's centre, like
//! the planet's (planet.rs) but simpler: no plants, no water, no biomes. Grey regolith on
//! level ground, dark mare basalt in the lowlands, rock on crater walls.

use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

use crate::camera::CameraWorld;
use crate::graphics::TerrainMaterial;
use crate::planet::{SPLIT_FACTOR, WorldPos, terrain_mesh};
use crate::terrain::{ChunkKey, N, RADIUS, face_dir, smoothstep};
use crate::world::{MOON_CENTER, MOON_RADIUS, moon};

/// Finest level: ~1.7 m between vertices on this smaller body.
const MAX_LEVEL: u8 = 17;

enum Slot {
    Pending(Task<MoonChunk>),
    Ready(Entity),
}

struct MoonChunk {
    center: DVec3,
    mesh: Mesh,
}

#[derive(Resource)]
pub struct Moon {
    chunks: HashMap<ChunkKey, (Slot, u64)>,
    shown: HashSet<ChunkKey>,
    centers: HashMap<ChunkKey, DVec3>,
    pub material: Handle<TerrainMaterial>,
    frame: u64,
}

pub struct MoonPlugin;

impl Plugin for MoonPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, setup).add_systems(Update, update_lod.after(crate::planet::update_lod));
    }
}

fn setup(mut commands: Commands, textures: Res<crate::textures::Textures>, mut images: ResMut<Assets<Image>>, mut materials: ResMut<Assets<TerrainMaterial>>) {
    // Regolith (sand layer), mare (dirt layer) and rock, tinted to greys.
    let mut tints = [Vec3::ONE; 8];
    tints[3] = Vec3::new(0.3, 0.37, 0.56); // mare basalt
    tints[4] = Vec3::new(0.68, 0.79, 1.12); // regolith
    tints[5] = Vec3::new(0.8, 0.8, 0.86); // rock
    let surface = crate::surface_map::constant(&mut images, [0, 0, 0, 40], [200, 15, 0, 0]);
    let material = materials.add(crate::graphics::terrain_material(&textures, Vec3::ZERO, tints, surface));
    commands.insert_resource(Moon { chunks: HashMap::new(), shown: HashSet::new(), centers: HashMap::new(), material, frame: 0 });
}

fn world_size(key: ChunkKey) -> f64 {
    key.world_size() * MOON_RADIUS / RADIUS
}

fn center_of(centers: &mut HashMap<ChunkKey, DVec3>, key: ChunkKey) -> DVec3 {
    if centers.len() > 100_000 {
        centers.clear();
    }
    *centers.entry(key).or_insert_with(|| {
        let d = key.center_dir();
        d * (MOON_RADIUS + moon().height(d))
    })
}

fn collect(centers: &mut HashMap<ChunkKey, DVec3>, key: ChunkKey, cam: DVec3, out: &mut Vec<ChunkKey>) {
    let split = key.level < MAX_LEVEL && (cam - center_of(centers, key)).length() < world_size(key) * SPLIT_FACTOR;
    if split {
        for c in key.children() {
            collect(centers, c, cam, out);
        }
    } else {
        out.push(key);
    }
}

/// Texture-layer weights for a moon vertex: (colors, uv0, uv1) as the terrain shader expects.
fn weights(d: DVec3, slope: f64) -> ([f32; 4], [f32; 2], [f32; 2]) {
    let rock = smoothstep(0.93, 0.8, slope) as f32;
    let mare = (moon().mare(d) as f32) * (1.0 - rock);
    let regolith = (1.0 - rock - mare).max(0.0);
    ([0.0, 0.0, 0.0, mare], [regolith, rock], [0.0, 0.0])
}

fn build(key: ChunkKey) -> MoonChunk {
    let m = moon();
    let (u0, v0) = key.uv0();
    let step = key.size() / N as f64;
    let g = N + 3;
    let mut pts = Vec::with_capacity(g * g);
    let mut dirs = Vec::with_capacity(g * g);
    for j in 0..g {
        for i in 0..g {
            let d = face_dir(key.face, u0 + (i as f64 - 1.0) * step, v0 + (j as f64 - 1.0) * step);
            pts.push(d * (MOON_RADIUS + m.height(d)));
            dirs.push(d);
        }
    }
    let center = key.center_dir() * MOON_RADIUS;
    let vn = N + 1;
    let (mut positions, mut normals, mut colors, mut uv0, mut uv1) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for j in 0..vn {
        for i in 0..vn {
            let gi = (j + 1) * g + (i + 1);
            let n = (pts[gi + 1] - pts[gi - 1]).cross(pts[gi + g] - pts[gi - g]).try_normalize().unwrap_or(dirs[gi]);
            positions.push((pts[gi] - center).as_vec3().to_array());
            normals.push(n.as_vec3().to_array());
            let (c, a, b) = weights(dirs[gi], n.dot(dirs[gi]));
            colors.push(c);
            uv0.push(a);
            uv1.push(b);
        }
    }
    let mut indices = Vec::new();
    for j in 0..N {
        for i in 0..N {
            let a = (j * vn + i) as u32;
            let (b, c, d) = (a + 1, a + vn as u32, a + vn as u32 + 1);
            indices.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    // Skirts hide cracks between detail levels.
    let mut perim = Vec::new();
    (0..N).for_each(|i| perim.push((i, 0)));
    (0..N).for_each(|j| perim.push((N, j)));
    (1..=N).rev().for_each(|i| perim.push((i, N)));
    (1..=N).rev().for_each(|j| perim.push((0, j)));
    let depth = world_size(key) / N as f64 * 0.5 + 20.0;
    let start = positions.len() as u32;
    for &(i, j) in &perim {
        let vi = j * vn + i;
        let gi = (j + 1) * g + (i + 1);
        positions.push((pts[gi] - dirs[gi] * depth - center).as_vec3().to_array());
        normals.push(normals[vi]);
        colors.push(colors[vi]);
        uv0.push(uv0[vi]);
        uv1.push(uv1[vi]);
    }
    let pl = perim.len();
    for k in 0..pl {
        let (ai, aj) = perim[k];
        let (bi, bj) = perim[(k + 1) % pl];
        let (a, b) = ((aj * vn + ai) as u32, (bj * vn + bi) as u32);
        let (sa, sb) = (start + k as u32, start + ((k + 1) % pl) as u32);
        indices.extend_from_slice(&[a, b, sb, a, sb, sa]);
    }
    let mut mesh = terrain_mesh(positions, normals, colors, uv0, uv1, indices);
    mesh.asset_usage = RenderAssetUsages::RENDER_WORLD;
    MoonChunk { center: MOON_CENTER + center, mesh }
}

fn update_lod(mut commands: Commands, mut moon_res: Option<ResMut<Moon>>, cam: Res<CameraWorld>, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<TerrainMaterial>>) {
    let Some(m) = moon_res.as_deref_mut() else { return };
    m.frame += 1;
    let frame = m.frame;
    let cam_rel = cam.pos - MOON_CENTER;
    let mut desired = Vec::new();
    for f in 0..6 {
        collect(&mut m.centers, ChunkKey::root(f), cam_rel, &mut desired);
    }
    let pool = AsyncComputeTaskPool::get();
    for &k in &desired {
        m.chunks.entry(k).or_insert_with(|| (Slot::Pending(pool.spawn(async move { build(k) })), 0)).1 = frame;
    }
    let start = std::time::Instant::now();
    for (slot, _) in m.chunks.values_mut() {
        if start.elapsed().as_secs_f64() > 0.002 {
            break;
        }
        if let Slot::Pending(task) = slot {
            if let Some(c) = block_on(future::poll_once(task)) {
                let e = commands
                    .spawn((
                        Mesh3d(meshes.add(c.mesh)),
                        MeshMaterial3d(m.material.clone()),
                        Transform::from_translation((c.center - cam.pos).as_vec3()),
                        Visibility::Hidden,
                        WorldPos(c.center),
                    ))
                    .id();
                *slot = Slot::Ready(e);
            }
        }
    }
    if desired.iter().all(|k| matches!(m.chunks[k].0, Slot::Ready(_))) {
        let new: HashSet<ChunkKey> = desired.iter().copied().collect();
        if new != m.shown {
            for k in m.shown.difference(&new) {
                if let Some((Slot::Ready(e), _)) = m.chunks.get(k) {
                    commands.entity(*e).insert(Visibility::Hidden);
                }
            }
            for k in new.difference(&m.shown) {
                if let Some((Slot::Ready(e), _)) = m.chunks.get(k) {
                    commands.entity(*e).insert(Visibility::Visible);
                }
            }
            m.shown = new;
        }
    }
    // Evict old hidden chunks.
    if m.chunks.len() > 900 {
        let shown = &m.shown;
        let mut old: Vec<(u64, ChunkKey)> = m
            .chunks
            .iter()
            .filter(|(k, (s, w))| !shown.contains(*k) && *w != frame && matches!(s, Slot::Ready(_)))
            .map(|(k, (_, w))| (*w, *k))
            .collect();
        old.sort_unstable_by_key(|o| o.0);
        let excess = m.chunks.len() - 900;
        for (_, k) in old.into_iter().take(excess) {
            if let Some((Slot::Ready(e), _)) = m.chunks.remove(&k) {
                commands.entity(e).despawn();
            }
        }
    }
    // Texture coordinates and the far-distance surface lookup follow the camera.
    if let Some(mat) = mats.get_mut(&m.material) {
        let wrap = |v: f64| v.rem_euclid(4096.0);
        mat.extension.params.cam_offset = DVec3::new(wrap(cam.pos.x), wrap(cam.pos.y), wrap(cam.pos.z)).as_vec3().extend(0.0);
        mat.extension.params.planet_center = (MOON_CENTER - cam.pos).as_vec3().extend(0.0);
    }
}
