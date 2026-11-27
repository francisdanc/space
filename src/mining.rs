//! Ore deposits and mining.
//!
//! Deposits are part of the procedural world: every cell of a coarse grid on each body
//! (~150 m on the planet, ~105 m on the moon) may hold one, with the ore chosen by geology:
//! iron and copper anywhere, quartz in dry lands and on beaches, titanium high up, platinum on
//! the highest steep peaks; on the moon titanium, iron, helium-3 (richest in the maria),
//! platinum, and void crystals deep in craters. How much has been taken from each deposit is
//! world state (`Mining::mined`, saved), so deposits shrink as they are mined and vanish when
//! empty. Hold E to mine by hand; drills (factory.rs) mine deposits automatically.

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;

use crate::forage::Toasts;
use crate::interact::{Focus, FocusKind, Interact, View, aim};
use crate::items::{Inventory, ItemId, Rarity, item};
use crate::planet::{Planet, WorldPos};
use crate::player::Player;
use crate::terrain::{Terrain, dir_to_face_uv, face_dir};
use crate::world::{Body, MOON_RADIUS, moon};

/// Grid level of the deposit cells on each body.
fn grid_level(b: Body) -> u32 {
    match b {
        Body::Planet => 16,
        Body::Moon => 14,
    }
}

/// Deposits are spawned (drawn, collidable) within this distance of the camera.
const SHOW_RADIUS: f64 = 400.0;

#[derive(Clone, Copy, Debug)]
pub struct Deposit {
    pub cell: u64,
    pub body: Body,
    pub ore: ItemId,
    /// Units in the deposit when untouched.
    pub amount: u32,
    /// Base of the outcrop on the ground.
    pub base: DVec3,
}

fn hash(x: u64, k: u64) -> f64 {
    let mut h = x ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = (h ^ (h >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 29)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (h >> 11) as f64 / (1u64 << 53) as f64
}

fn cell_id(b: Body, face: u8, i: u64, j: u64) -> u64 {
    (matches!(b, Body::Moon) as u64) << 63 | (face as u64) << 58 | i << 29 | j
}

/// Pick from weighted choices with a 0..1 roll.
fn pick(weights: &[(ItemId, f64)], roll: f64) -> ItemId {
    let total: f64 = weights.iter().map(|w| w.1.max(0.0)).sum();
    let mut r = roll * total;
    for &(id, w) in weights {
        if r < w.max(0.0) {
            return id;
        }
        r -= w.max(0.0);
    }
    weights[0].0
}

/// Units in a fresh deposit by rarity.
fn amount_for(ore: ItemId, roll: f64) -> u32 {
    let (lo, hi) = match ore.def().rarity {
        Rarity::Common => (80.0, 200.0),
        Rarity::Uncommon => (50.0, 120.0),
        Rarity::Rare => (25.0, 60.0),
        Rarity::Exotic => (10.0, 25.0),
        Rarity::Legendary => (3.0, 8.0),
    };
    (lo + (hi - lo) * roll) as u32
}

/// The deposit in a grid cell, if any.
pub fn deposit_at(t: &Terrain, b: Body, face: u8, i: u64, j: u64) -> Option<Deposit> {
    let cell = cell_id(b, face, i, j);
    let seed = crate::planet::SEED;
    if hash(cell ^ seed, 1) > 0.2 {
        return None;
    }
    let n = (1u64 << grid_level(b)) as f64;
    let u = -1.0 + (i as f64 + 0.2 + 0.6 * hash(cell, 2)) * 2.0 / n;
    let v = -1.0 + (j as f64 + 0.2 + 0.6 * hash(cell, 3)) * 2.0 / n;
    let dir = face_dir(face, u, v);
    let r = b.radius();
    let h = crate::world::ground(t, b, dir);
    // Slope from the ground 10 m away.
    let side = dir.any_orthonormal_vector();
    let d2 = (dir + side * (10.0 / r)).normalize();
    let slope = (crate::world::ground(t, b, d2) - h).abs() / 10.0;
    let weights: Vec<(ItemId, f64)> = match b {
        Body::Planet => {
            if h < 2.0 || t.water_level(dir).is_some() || t.on_pad(dir, 60.0) {
                return None;
            }
            let eco = t.ecology(dir);
            let dry = (0.35 - eco.moisture as f64).max(0.0) * 3.0;
            let high = crate::terrain::smoothstep(1200.0, 2500.0, h);
            let peak = crate::terrain::smoothstep(2300.0, 3500.0, h) * (0.3 + slope.min(1.0));
            vec![
                (item::IRON_ORE, 40.0),
                (item::COPPER_ORE, 28.0),
                (item::QUARTZ, 10.0 + 25.0 * dry + 12.0 * (1.0 - crate::terrain::smoothstep(2.0, 15.0, h))),
                (item::TITANIUM_ORE, 3.0 + 22.0 * high),
                (item::PLATINUM_ORE, 0.6 + 9.0 * peak),
            ]
        }
        Body::Moon => {
            if (dir - crate::world::moon_station_dir()).length() * MOON_RADIUS < 250.0 {
                return None;
            }
            let mare = moon().mare(dir);
            let deep = crate::terrain::smoothstep(-1500.0, -4000.0, h);
            vec![
                (item::TITANIUM_ORE, 30.0),
                (item::IRON_ORE, 25.0),
                (item::HELIUM3, 18.0 + 30.0 * mare),
                (item::PLATINUM_ORE, 6.0),
                (item::VOID_CRYSTAL, 0.8 + 8.0 * deep),
            ]
        }
    };
    let ore = pick(&weights, hash(cell, 4));
    let base = b.center() + dir * (r + h - 0.2);
    Some(Deposit { cell, body: b, ore, amount: amount_for(ore, hash(cell, 5)), base })
}

/// All deposits within `radius` m of `pos` (on the nearest body).
pub fn deposits_near(t: &Terrain, pos: DVec3, radius: f64) -> Vec<Deposit> {
    let b = crate::world::nearest(pos);
    let level = grid_level(b);
    let n = 1u64 << level;
    let dir = (pos - b.center()).normalize();
    let (face, u, v) = dir_to_face_uv(dir);
    let cell_m = b.radius() * std::f64::consts::FRAC_PI_2 / n as f64;
    let span = (radius / cell_m).ceil() as i64 + 1;
    let to = |t: f64| (((t + 1.0) * 0.5 * n as f64).floor() as i64).clamp(0, n as i64 - 1);
    let (ci, cj) = (to(u), to(v));
    let mut out = Vec::new();
    for j in (cj - span).max(0)..=(cj + span).min(n as i64 - 1) {
        for i in (ci - span).max(0)..=(ci + span).min(n as i64 - 1) {
            if let Some(d) = deposit_at(t, b, face, i as u64, j as u64) {
                if (d.base - pos).length() < radius {
                    out.push(d);
                }
            }
        }
    }
    out
}

/// Ore colour and whether it grows as crystals.
fn look(ore: ItemId) -> ([f32; 3], bool) {
    match ore {
        item::IRON_ORE => ([0.45, 0.12, 0.05], false),
        item::COPPER_ORE => ([0.12, 0.45, 0.35], false),
        item::QUARTZ => ([0.95, 0.88, 0.92], true),
        item::TITANIUM_ORE => ([0.55, 0.62, 0.75], false),
        item::PLATINUM_ORE => ([0.9, 0.9, 0.85], false),
        item::HELIUM3 => ([0.95, 0.85, 0.35], true),
        item::VOID_CRYSTAL => ([0.55, 0.15, 0.95], true),
        _ => ([0.5, 0.5, 0.5], false),
    }
}

/// World state: units taken from each deposit (by cell), and what is spawned.
#[derive(Resource, Default)]
pub struct Mining {
    pub mined: HashMap<u64, u32>,
    /// (visual, collider, deposit)
    spawned: HashMap<u64, (Entity, Entity, Deposit)>,
    meshes: HashMap<ItemId, Handle<Mesh>>,
    scanned_at: Option<DVec3>,
    progress: f32,
    target: Option<u64>,
}

impl Mining {
    pub fn remaining(&self, d: &Deposit) -> u32 {
        d.amount.saturating_sub(self.mined.get(&d.cell).copied().unwrap_or(0))
    }

    /// Take one unit from a deposit (drills and hand mining). False when it is empty.
    pub fn take(&mut self, d: &Deposit) -> bool {
        if self.remaining(d) == 0 {
            return false;
        }
        *self.mined.entry(d.cell).or_insert(0) += 1;
        true
    }

    /// Forget spawned deposits (after loading a save) so they respawn with the right sizes.
    pub fn respawn(&mut self, commands: &mut Commands) {
        for (_, (e, c, _)) in self.spawned.drain() {
            commands.entity(e).despawn();
            commands.entity(c).despawn();
        }
        self.scanned_at = None;
    }
}

#[derive(Component)]
pub struct DepositMarker(pub u64);

pub struct MiningPlugin;

impl Plugin for MiningPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mining>()
            .add_systems(Update, (spawn_deposits, offer).chain().in_set(crate::interact::InteractSet::Offer))
            .add_systems(Update, mine.in_set(crate::interact::InteractSet::Act));
    }
}

fn outcrop_scale(remaining: u32, amount: u32) -> f32 {
    0.55 + 0.9 * (remaining as f32 / amount.max(1) as f32).sqrt() * (amount as f32 / 120.0).clamp(0.6, 1.6)
}

fn spawn_deposits(
    mut commands: Commands,
    cam: Res<crate::camera::CameraWorld>,
    planet: Res<Planet>,
    mut mining: ResMut<Mining>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if mining.scanned_at.is_some_and(|p| (p - cam.pos).length() < 40.0) {
        return;
    }
    mining.scanned_at = Some(cam.pos);
    debug!("deposit scan at {:?}", crate::world::nearest(cam.pos));
    let near = if crate::world::altitude(&planet.terrain, cam.pos) < SHOW_RADIUS { deposits_near(&planet.terrain, cam.pos, SHOW_RADIUS) } else { Vec::new() };
    let keep: std::collections::HashSet<u64> = near.iter().map(|d| d.cell).collect();
    let gone: Vec<u64> = mining.spawned.keys().filter(|c| !keep.contains(c)).copied().collect();
    for c in gone {
        if let Some((e, col, _)) = mining.spawned.remove(&c) {
            commands.entity(e).despawn();
            commands.entity(col).despawn();
        }
    }
    for d in near {
        let remaining = mining.remaining(&d);
        if remaining == 0 || mining.spawned.contains_key(&d.cell) {
            continue;
        }
        let mesh = mining
            .meshes
            .entry(d.ore)
            .or_insert_with(|| {
                let (color, crystals) = look(d.ore);
                let rock = if d.body == Body::Moon { [0.75, 0.75, 0.78] } else { [0.85, 0.8, 0.75] };
                let t = crate::plants::ore_outcrop(color, rock, crystals, d.ore.0 as u64 * 77 + 5);
                let mut md = crate::flora::MeshData::default();
                md.append(&t.solid, Vec3::ZERO, Quat::IDENTITY, 1.0, 0.5);
                meshes.add(crate::planet::flora_mesh(md).0)
            })
            .clone();
        let up = crate::world::up_at(d.base);
        let rot = DQuat::from_rotation_arc(DVec3::Y, up) * DQuat::from_rotation_y(hash(d.cell, 9) * 6.28);
        let s = outcrop_scale(remaining, d.amount);
        let e = commands
            .spawn((
                DepositMarker(d.cell),
                Mesh3d(mesh),
                MeshMaterial3d(planet.flora_solid_material.clone()),
                Transform { rotation: rot.as_quat(), scale: Vec3::splat(s), ..default() },
                WorldPos(d.base),
            ))
            .id();
        let col = commands
            .spawn((RigidBody::Static, Collider::sphere(0.75 * s as f64), Position(d.base + up * (0.3 * s as f64)), Rotation(rot), Transform::default()))
            .id();
        mining.spawned.insert(d.cell, (e, col, d));
    }
    let _ = MOON_RADIUS;
}

/// How hard an ore is: 1 = hand tools and the basic drill, 2 = titanium drill, 3 = plasma drill.
pub fn ore_tier(id: ItemId) -> u8 {
    use crate::items::item::*;
    match id {
        TITANIUM_ORE | HELIUM3 => 2,
        PLATINUM_ORE | VOID_CRYSTAL => 3,
        _ => 1,
    }
}

/// The drill an ore needs, by name.
pub fn tier_tool(tier: u8) -> &'static str {
    ["hand tools", "hand tools", "a Titanium drill", "a Plasma drill"][tier.min(3) as usize]
}

fn offer(view: Res<View>, mut mining: ResMut<Mining>, mut focus: ResMut<Focus>) {
    mining.target = None;
    let mut best: Option<(f64, u64, String)> = None;
    for (cell, (_, _, d)) in &mining.spawned {
        let up = crate::world::up_at(d.base);
        let s = outcrop_scale(mining.remaining(d), d.amount) as f64;
        let Some(score) = aim(&view, d.base + up * (0.5 * s), 3.2 + s) else { continue };
        if best.as_ref().is_none_or(|b| score < b.0) {
            let def = d.ore.def();
            let tier = ore_tier(d.ore);
            let prompt = if tier > 1 {
                format!("{} [{}]  ({} left): too hard to mine by hand, needs {}", def.name, def.rarity.name(), mining.remaining(d), tier_tool(tier))
            } else {
                format!("Hold E: mine {} [{}]  ({} left)", def.name, def.rarity.name(), mining.remaining(d))
            };
            best = Some((score, *cell, prompt));
        }
    }
    if let Some((score, cell, prompt)) = best {
        mining.target = Some(cell);
        focus.offer(FocusKind::Mine, score, prompt);
    }
}

#[allow(clippy::too_many_arguments)]
fn mine(
    mut commands: Commands,
    time: Res<Time>,
    interact: Res<Interact>,
    focus: Res<Focus>,
    mut mining: ResMut<Mining>,
    mut player: Query<&mut Inventory, With<Player>>,
    mut toasts: ResMut<Toasts>,
    mut transforms: Query<&mut Transform, With<DepositMarker>>,
) {
    let target = mining.target.filter(|_| interact.held && focus.is(FocusKind::Mine));
    let Some(cell) = target else {
        mining.progress = 0.0;
        return;
    };
    let Some(&(entity, collider, d)) = mining.spawned.get(&cell) else { return };
    let Ok(mut inv) = player.single_mut() else { return };
    if ore_tier(d.ore) > 1 {
        if interact.pressed {
            toasts.push(format!("{} needs {}", d.ore.def().name, tier_tool(ore_tier(d.ore))), Color::srgb(1.0, 0.45, 0.3));
        }
        return;
    }
    // Rarer ores take longer to cut out.
    let per_unit = 0.5 * (1.0 + 0.6 * d.ore.def().rarity as u8 as f32);
    mining.progress += time.delta_secs();
    if mining.progress < per_unit {
        return;
    }
    mining.progress = 0.0;
    if inv.add(d.ore, 1) == 0 {
        toasts.push("Inventory full", Color::srgb(1.0, 0.4, 0.3));
        return;
    }
    mining.take(&d);
    let def = d.ore.def();
    toasts.push_merged(def.name, 1, def.rarity.color());
    let remaining = mining.remaining(&d);
    if remaining == 0 {
        commands.entity(entity).despawn();
        commands.entity(collider).despawn();
        mining.spawned.remove(&cell);
        toasts.push(format!("{} deposit exhausted", def.name), Color::srgb(0.8, 0.8, 0.8));
    } else if let Ok(mut t) = transforms.get_mut(entity) {
        t.scale = Vec3::splat(outcrop_scale(remaining, d.amount));
    }
}
