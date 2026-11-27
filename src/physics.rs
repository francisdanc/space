//! Physics setup (Avian, f64) and terrain collision.
//!
//! Physics runs in true f64 world coordinates (planet centre at the origin), so it never needs
//! re-centring. Avian's own Transform syncing is turned off; [`apply_body_origin`] places bodies
//! relative to the camera instead, like everything else (floating origin).
//!
//! Terrain collision is separate from the visible LOD mesh: every entity marked
//! [`TerrainCollision`] gets its own small heightfield patch, rebuilt on a worker thread as it
//! moves. That keeps collision available wherever the player *or a ship* is, whatever the LOD.

use std::sync::Arc;

use avian3d::prelude::*;
use avian3d::sync::SyncConfig;
use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

use crate::camera::CameraWorld;
use crate::planet::Planet;
use crate::flora::{FloraCollision, FloraContext, colliders_near};
use crate::terrain::RADIUS;

/// Side length of a collision patch in metres.
const PATCH_SIZE: f64 = 128.0;
/// Samples per patch side (1 m spacing).
const PATCH_RES: usize = 129;
/// Rebuild the patch when its owner is this far from the patch centre.
const REBUILD_DIST: f64 = 32.0;

pub struct SpacePhysicsPlugin;

impl Plugin for SpacePhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::new(PostUpdate))
            .insert_resource(Gravity::ZERO)
            .insert_resource(SyncConfig {
                position_to_transform: false,
                transform_to_position: false,
                transform_to_collider_scale: false,
            })
            .insert_resource(PrepareConfig { position_to_transform: false, transform_to_position: false })
            .add_systems(Update, (apply_planet_gravity, update_terrain_patches, water_forces));
    }
}

/// Gravitational acceleration at a point, falling off with the inverse square of the distance.
pub fn gravity_at(pos: DVec3) -> f64 {
    let ratio = RADIUS / pos.length().max(1.0);
    crate::player::GRAVITY * ratio * ratio
}

/// Altitudes (above the nearest body's radius) of the highest and lowest points of the orbit
/// through `pos` with velocity `vel` around that body. `None` for an escape trajectory.
pub fn orbit_extremes(pos: DVec3, vel: DVec3) -> Option<(f64, f64)> {
    let body = crate::world::nearest(pos);
    let (mu, radius) = (body.mu(), body.radius());
    let pos = pos - body.center();
    let r = pos.length();
    let energy = vel.length_squared() * 0.5 - mu / r;
    if energy >= 0.0 {
        return None;
    }
    let a = -mu / (2.0 * energy);
    let h = pos.cross(vel).length();
    let e = (1.0 + 2.0 * energy * h * h / (mu * mu)).max(0.0).sqrt();
    Some((a * (1.0 + e) - radius, a * (1.0 - e) - radius))
}

/// Bodies pulled toward the planet centre (the player handles its own gravity).
#[derive(Component)]
pub struct PlanetGravity;

fn apply_planet_gravity(time: Res<Time>, mut q: Query<(&Position, &mut LinearVelocity), With<PlanetGravity>>) {
    let dt = time.delta_secs_f64();
    for (pos, mut vel) in &mut q {
        vel.0 += crate::world::gravity(pos.0) * dt;
    }
}

/// Add to any physics body that should collide with the planet surface.
#[derive(Component, Default)]
pub struct TerrainCollision {
    patch_center: Option<DVec3>,
    /// The ground heightfield and one static body per plant collider.
    patch: Vec<Entity>,
    task: Option<Task<PatchData>>,
}

struct PatchData {
    center: DVec3,
    rotation: DQuat,
    collider: Collider,
    /// Trunks, boulders and logs, each a separate body so the broad phase can skip them.
    plants: Vec<(DVec3, DQuat, Collider)>,
}

/// Local frame of the patch: Y points away from the planet, X/Z span the tangent plane.
/// A body that floats: buoyancy and drag in proportion to how much of it is under the sea
/// surface (the sphere at RADIUS).
#[derive(Component, Clone, Copy)]
pub struct Floats {
    /// Height of the floating hull, centred on the body's origin.
    pub height: f64,
    /// Buoyancy when fully submerged, in g (2 = floats half under).
    pub lift: f64,
}

/// Fraction (0..1) of a body of height `height`, centred at `pos`, that is under water (the
/// sea, or a river or lake).
pub fn submerged(terrain: &crate::terrain::Terrain, pos: DVec3, height: f64) -> f64 {
    if crate::world::nearest(pos) == crate::world::Body::Moon {
        return 0.0;
    }
    let d = pos.normalize();
    let level = match terrain.water(d) {
        Some(w) => w.level.max(0.0),
        None => 0.0,
    };
    let depth = RADIUS + level - pos.length();
    ((depth + height * 0.5) / height).clamp(0.0, 1.0)
}

/// Buoyancy, drag and a righting torque for floating bodies.
fn water_forces(
    time: Res<Time>,
    planet: Res<Planet>,
    mut q: Query<(&Floats, &Position, &Rotation, &mut LinearVelocity, &mut AngularVelocity)>,
) {
    let dt = time.delta_secs_f64();
    for (f, pos, rot, mut vel, mut ang) in &mut q {
        let s = submerged(&planet.terrain, pos.0, f.height);
        if s <= 0.0 {
            continue;
        }
        let up = crate::world::up_at(pos.0);
        vel.0 += up * crate::world::gravity(pos.0).length() * f.lift * s * dt;
        // Water drag: strong vertically (damps bobbing), weaker sideways (it can drift).
        let v_up = up * vel.0.dot(up);
        let v_side = vel.0 - v_up;
        vel.0 = v_up * (-1.6 * s * dt).exp() + v_side * (-0.5 * s * dt).exp();
        // Settle level on the water.
        let body_up = rot.0 * DVec3::Y;
        ang.0 = ang.0 * (-2.0 * s * dt).exp() + body_up.cross(up) * (1.5 * s * dt);
    }
}

fn patch_frame(around: DVec3) -> (DVec3, DVec3, DVec3) {
    let n = crate::world::up_at(around);
    let e1 = n.any_orthonormal_vector();
    let e2 = e1.cross(n);
    (n, e1, e2)
}

fn build_patch(ctx: &FloraContext, around: DVec3) -> PatchData {
    let t = &*ctx.terrain;
    let body = crate::world::nearest(around);
    let (bc, br) = (body.center(), body.radius());
    let (n, e1, e2) = patch_frame(around);
    // The solid ground: the sea floor under water (water is not solid; see water_forces).
    let ground = |d: DVec3| crate::world::ground(t, body, d);
    let dir0 = (around - bc).normalize();
    let center = bc + dir0 * (br + ground(dir0));
    let step = PATCH_SIZE / (PATCH_RES - 1) as f64;
    let half = PATCH_SIZE * 0.5;
    // heights[i][j]: i runs along local X, j along local Z (verified by a unit test below).
    let heights: Vec<Vec<f64>> = (0..PATCH_RES)
        .map(|i| {
            (0..PATCH_RES)
                .map(|j| {
                    let x = -half + i as f64 * step;
                    let z = -half + j as f64 * step;
                    let d = (center + e1 * x + e2 * z - bc).normalize();
                    let p = bc + d * (br + ground(d));
                    (p - center).dot(n)
                })
                .collect()
        })
        .collect();
    let rotation = DQuat::from_mat3(&DMat3::from_cols(e1, n, e2));
    let ground = Collider::heightfield(heights, DVec3::new(PATCH_SIZE, 1.0, PATCH_SIZE));
    // Tree trunks, boulders and logs in the patch (same plants as are drawn).
    let mut plants = Vec::new();
    let plants_here = if body == crate::world::Body::Planet { colliders_near(ctx, center, PATCH_SIZE * 0.5) } else { Vec::new() };
    for pc in plants_here {
        let s = pc.scale as f64;
        let up = pc.rot * DVec3::Y;
        let (offset, rot, shape) = match pc.shape {
            FloraCollision::Trunk { radius, height } => {
                let h = height as f64 * s;
                (up * (h * 0.5), pc.rot, Collider::cylinder(radius as f64 * s, h))
            }
            FloraCollision::Sphere { radius, y } => (up * (y as f64 * s), pc.rot, Collider::sphere(radius as f64 * s)),
            FloraCollision::Log { radius, length } => (
                up * (radius as f64 * s),
                pc.rot * DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2),
                Collider::capsule(radius as f64 * s, length as f64 * s),
            ),
        };
        plants.push((pc.pos + offset, rot, shape));
    }
    PatchData { center, rotation, collider: ground, plants }
}

fn update_terrain_patches(
    mut commands: Commands,
    planet: Res<Planet>,
    mut owners: Query<(&Position, &mut TerrainCollision), Without<crate::player::InShip>>,
    mut prof: ResMut<crate::debug::Prof>,
) {
    let _guard = crate::debug::ProfGuard(&mut prof, "terrain_patches", std::time::Instant::now());
    let pool = AsyncComputeTaskPool::get();
    for (pos, mut tc) in &mut owners {
        if let Some(task) = &mut tc.task {
            if let Some(data) = block_on(future::poll_once(task)) {
                tc.task = None;
                for old in tc.patch.drain(..) {
                    commands.entity(old).despawn();
                }
                tc.patch_center = Some(data.center);
                tc.patch.push(
                    commands
                        .spawn((
                            RigidBody::Static,
                            data.collider,
                            Position(data.center),
                            Rotation(data.rotation),
                            Transform::default(),
                        ))
                        .id(),
                );
                for (pos, rot, collider) in data.plants {
                    tc.patch.push(
                        commands.spawn((RigidBody::Static, collider, Position(pos), Rotation(rot), Transform::default())).id(),
                    );
                }
            }
        }
        let far = tc.patch_center.is_none_or(|c| (pos.0 - c).length() > REBUILD_DIST);
        // Only bother when near the ground.
        let altitude = crate::world::altitude(&planet.terrain, pos.0);
        if far && tc.task.is_none() && altitude < PATCH_SIZE * 0.5 {
            let ctx: Arc<FloraContext> = planet.flora.clone();
            let around = pos.0;
            tc.task = Some(pool.spawn(async move { build_patch(&ctx, around) }));
        }
    }
}

/// Floating origin for physics bodies: render them relative to the camera.
pub fn apply_body_origin(
    cam: Res<CameraWorld>,
    mut q: Query<(&Position, &Rotation, &mut Transform), With<RigidBody>>,
) {
    for (pos, rot, mut t) in &mut q {
        t.translation = (pos.0 - cam.pos).as_vec3();
        t.rotation = rot.0.as_quat();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::Terrain;

    /// Checks the heightfield layout: a ray cast straight down onto the collider must hit the
    /// height our own function says is at that (x, z).
    #[test]
    fn heightfield_layout_matches_terrain() {
        let ctx = FloraContext::new(Arc::new(Terrain::new(crate::planet::SEED)), crate::life::Biosphere::barren(), 1, Vec::new());
        let t = &*ctx.terrain;
        let around = DVec3::new(0.3, 0.8, -0.5).normalize() * RADIUS;
        let patch = build_patch(&ctx, around);
        let (n, e1, e2) = patch_frame(around);
        let mut worst: f64 = 0.0;
        for (x, z) in [(0.0, 0.0), (40.0, -20.0), (-55.5, 33.3), (10.0, 60.0), (-60.0, -60.0)] {
            let d = (patch.center + e1 * x + e2 * z).normalize();
            let expected = (d * (RADIUS + t.height(d)) - patch.center).dot(n);
            let origin = patch.center + e1 * x + e2 * z + n * 500.0;
            let (dist, _) = patch
                .collider
                .cast_ray(patch.center, patch.rotation, origin, -n, 1000.0, true)
                .expect("ray should hit the patch");
            let got = 500.0 - dist;
            worst = worst.max((got - expected).abs());
        }
        // 1 m grid linear interpolation of the true surface: allow a few cm.
        assert!(worst < 0.25, "heightfield mismatch {worst} m");
    }
}
