//! First-person player: a physics capsule that walks on the planet with gravity toward the
//! planet centre. "Up" is always away from the planet, so the player can walk all the way round.
//!
//! The body (`Player`) owns the yaw; the camera adds pitch on top. Later the spacesuit/body mesh
//! hangs off the same entity.
//!
//! Inside a ship ([`InShip`]) the player's physics runs in the ship's interior proxy, where "up"
//! is the ship's +Y and gravity is artificial (see ship.rs).

use avian3d::prelude::*;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::camera::{CameraWorld, FlyCam};
use crate::physics::TerrainCollision;
use crate::planet::Planet;

/// Surface gravity in m/s².
pub const GRAVITY: f64 = 9.81;
const CAPSULE_RADIUS: f64 = 0.35;
const CAPSULE_LENGTH: f64 = 1.1;
/// Distance from the capsule centre down to the feet.
pub const HALF_HEIGHT: f64 = CAPSULE_RADIUS + CAPSULE_LENGTH * 0.5;
/// Eye height above the capsule centre (eyes ~1.65 m above the feet).
const EYE_OFFSET: f64 = 0.75;
/// How far in front of the body's centre line the eyes are (m).
const EYE_FORWARD: f32 = 0.15;
const WALK_SPEED: f64 = 4.5;
const RUN_SPEED: f64 = 9.0;
const JUMP_SPEED: f64 = 5.0;
/// How far below the feet we look for ground to snap onto.
const SNAP_DISTANCE: f64 = 0.6;
/// Steepest walkable slope (cosine of ~55°).
const MAX_SLOPE_COS: f64 = 0.57;
/// Highest ledge the player steps up onto automatically.
const MAX_STEP: f64 = 0.45;
/// Radius of the sphere cast used to find the ground (smaller than the capsule, so walls
/// beside the player aren't mistaken for ground).
const GROUND_PROBE_RADIUS: f64 = 0.22;

#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControlMode {
    /// On foot, first person.
    Walk,
    /// Free-flying debug camera.
    Fly,
    /// Flying a ship.
    Pilot,
}

#[derive(Component)]
pub struct Player {
    /// Facing direction, kept tangent to the planet surface.
    pub heading: DVec3,
    /// Look up/down angle of the camera, in radians.
    pub pitch: f32,
    pub grounded: bool,
    /// Surface normal under the player (valid while grounded).
    pub ground_normal: DVec3,
    /// Distance from the capsule centre down to the ground, if within snapping range.
    pub ground_dist: Option<f64>,
    /// Was on the ground last frame (allows snapping down over crests and bumps).
    pub was_grounded: bool,
    /// Seconds since the last jump; ground snapping is off for a moment after jumping.
    pub since_jump: f32,
    /// Direction the player is trying to move (frame space), used to look for steps.
    pub move_dir: DVec3,
    /// Height of a walkable step just ahead of the feet, if there is one.
    pub step_ahead: Option<f64>,
    /// Camera behind the player instead of at the eyes (V toggles).
    pub third_person: bool,
    pub spawn: DVec3,
    /// Fraction of the body under water (0..1).
    pub submerged: f32,
    /// Swimming rather than walking.
    pub swimming: bool,
}

/// The player is inside a ship: `Position` is in that ship's interior proxy (ship-local axes,
/// offset by `proxy`), and `Player::heading` is in ship-local axes too.
#[derive(Component, Clone, Copy)]
pub struct InShip {
    pub ship: Entity,
    pub proxy: DVec3,
}

/// "Up" for the player in whichever frame they are in.
fn frame_up(pos: DVec3, in_ship: Option<&InShip>) -> DVec3 {
    if in_ship.is_some() { DVec3::Y } else { crate::world::up_at(pos) }
}

/// Set by the test hooks to hold "walk forward" without a keyboard.
#[derive(Resource)]
pub struct AutoWalk {
    /// Hold Shift (run) as well.
    pub run: bool,
}

/// Rotation whose local Y is `up` and local -Z is `forward`.
pub fn basis(forward: DVec3, up: DVec3) -> DQuat {
    let right = forward.cross(up).normalize();
    let forward = up.cross(right);
    DQuat::from_mat3(&DMat3::from_cols(right, up, -forward))
}

pub fn tangent(v: DVec3, up: DVec3) -> DVec3 {
    (v - up * v.dot(up)).try_normalize().unwrap_or_else(|| up.any_orthonormal_vector())
}

/// Capsule centre position for standing on the surface below `p` (a position, or for the
/// planet a direction from its centre), on whichever body is nearest.
pub fn standing_position(planet: &Planet, p: DVec3) -> DVec3 {
    let s = crate::world::surface_point(&planet.terrain, p);
    s + crate::world::up_at(s) * (HALF_HEIGHT + 0.05)
}

pub fn spawn_player(mut commands: Commands, cam: Res<CameraWorld>, planet: Res<Planet>) {
    let pos = standing_position(&planet, cam.pos);
    let up = crate::world::up_at(pos);
    let heading = tangent(DVec3::Y, up);
    commands.spawn((
        Player {
            heading,
            pitch: -0.1,
            grounded: false,
            ground_normal: up,
            ground_dist: None,
            was_grounded: false,
            since_jump: 1.0,
            move_dir: DVec3::ZERO,
            step_ahead: None,
            third_person: false,
            spawn: pos,
            submerged: 0.0,
            swimming: false,
        },
        RigidBody::Dynamic,
        Collider::capsule(CAPSULE_RADIUS, CAPSULE_LENGTH),
        LockedAxes::ROTATION_LOCKED,
        Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
        Restitution::ZERO.with_combine_rule(CoefficientCombine::Min),
        Mass(80.0),
        Position(pos),
        Rotation(basis(heading, up)),
        LinearVelocity::default(),
        Transform::default(),
        TerrainCollision::default(),
        crate::items::Inventory::new(24),
    ));
}

/// Tab switches between walking and the free camera.
pub fn toggle_mode(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<ControlMode>,
    planet: Res<Planet>,
    cam: Res<CameraWorld>,
    cam_t: Query<&Transform, With<FlyCam>>,
    mut q: Query<(Entity, &mut Player, &mut Position, &mut LinearVelocity)>,
) {
    if !keys.just_pressed(KeyCode::Tab) {
        return;
    }
    *mode = match *mode {
        ControlMode::Walk => ControlMode::Fly,
        ControlMode::Pilot => ControlMode::Pilot,
        ControlMode::Fly => {
            // Drop the player onto the ground below the free camera, facing the same way.
            if let (Ok((e, mut p, mut pos, mut vel)), Ok(t)) = (q.single_mut(), cam_t.single()) {
                commands.entity(e).remove::<InShip>();
                pos.0 = standing_position(&planet, cam.pos);
                vel.0 = DVec3::ZERO;
                let up = crate::world::up_at(pos.0);
                p.heading = tangent(t.forward().as_dvec3(), up);
                p.pitch = 0.0;
            }
            ControlMode::Walk
        }
    };
}

/// Ground check: ray from the capsule centre straight down. Runs before [`player_control`].
pub fn ground_check(spatial: SpatialQuery, mut q: Query<(Entity, &Position, &mut Player, Option<&InShip>)>) {
    let Ok((entity, pos, mut p, in_ship)) = q.single_mut() else { return };
    let up = frame_up(pos.0, in_ship);
    let down = Dir3::new(-up.as_vec3()).unwrap_or(Dir3::NEG_Y);
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    // Cast a small sphere (not a thin ray) down from the capsule centre, so standing on the
    // edge of a log, boulder or ledge still counts as ground. Look further than "touching" so
    // we can also snap down onto ground that drops away.
    let probe = Collider::sphere(GROUND_PROBE_RADIUS);
    let config = ShapeCastConfig::from_max_distance(HALF_HEIGHT - GROUND_PROBE_RADIUS + SNAP_DISTANCE);
    let hit = spatial.cast_shape(&probe, pos.0, DQuat::IDENTITY, down, &config, &filter);
    // Distance from the capsule centre down to the ground under it.
    p.ground_dist = hit.as_ref().map(|h| h.distance + GROUND_PROBE_RADIUS);
    p.ground_normal = hit.as_ref().map(|h| h.normal1).unwrap_or(up);
    // Too steep to stand on counts as not grounded (you slide off).
    let walkable = p.ground_normal.dot(up) > MAX_SLOPE_COS;
    p.grounded = walkable && p.ground_dist.is_some_and(|d| d <= HALF_HEIGHT + 0.12);

    // Step detection: probe down from just ahead of the feet, from step height.
    p.step_ahead = None;
    if p.move_dir != DVec3::ZERO {
        let feet = pos.0 - up * HALF_HEIGHT;
        let origin = feet + p.move_dir * (CAPSULE_RADIUS + 0.12) + up * MAX_STEP;
        if let Some(h) = spatial.cast_ray(origin, down, MAX_STEP + 0.05, true, &filter) {
            let step = MAX_STEP - h.distance;
            // A probe that starts inside something has hit a wall (e.g. a tree trunk), not a step.
            if h.distance > 0.02 && step > 0.03 && h.normal.dot(up) > MAX_SLOPE_COS {
                p.step_ahead = Some(step);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn player_control(
    time: Res<Time>,
    mode: Res<ControlMode>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    windows: Query<&Window, With<PrimaryWindow>>,
    autowalk: Option<Res<AutoWalk>>,
    planet: Res<Planet>,
    survival: Option<Res<crate::forage::Survival>>,
    mut q: Query<(&mut Player, &mut Position, &mut Rotation, &mut LinearVelocity, Option<&InShip>)>,
) {
    if *mode == ControlMode::Pilot {
        return;
    }
    let Ok((mut p, mut pos, mut rot, mut vel, in_ship)) = q.single_mut() else { return };
    let dt = time.delta_secs_f64();
    let up = frame_up(pos.0, in_ship);
    let walking = *mode == ControlMode::Walk;

    // Mouse look: yaw turns the body, pitch only the camera.
    let locked = windows
        .single()
        .map(|w| w.cursor_options.grab_mode != CursorGrabMode::None)
        .unwrap_or(false);
    if walking && keys.just_pressed(KeyCode::KeyV) {
        p.third_person = !p.third_person;
    }
    if walking && locked {
        let d = motion.delta;
        p.heading = DQuat::from_axis_angle(up, -d.x as f64 * 0.002) * p.heading;
        p.pitch = (p.pitch - d.y * 0.002).clamp(-1.55, 1.55);
    }
    // Keep the heading tangent as we move around the curved surface.
    p.heading = tangent(p.heading, up);
    rot.0 = basis(p.heading, up);

    let mut fwd = 0.0;
    let mut side = 0.0;
    if walking {
        if keys.pressed(KeyCode::KeyW) || autowalk.is_some() {
            fwd += 1.0;
        }
        if keys.pressed(KeyCode::KeyS) {
            fwd -= 1.0;
        }
        if keys.pressed(KeyCode::KeyD) {
            side += 1.0;
        }
        if keys.pressed(KeyCode::KeyA) {
            side -= 1.0;
        }
    }
    let right = p.heading.cross(up);
    // Starving: too weak to run, and slower.
    let starving = survival.as_ref().is_some_and(|s| s.starving());
    let running = (keys.pressed(KeyCode::ShiftLeft) || autowalk.as_ref().is_some_and(|a| a.run)) && !starving;
    let speed = if running { RUN_SPEED } else if starving { WALK_SPEED * 0.75 } else { WALK_SPEED };
    let target = (p.heading * fwd + right * side).normalize_or_zero() * speed;

    let g_mag = if in_ship.is_some() { GRAVITY } else { crate::world::gravity(pos.0).length() };
    p.since_jump += dt as f32;

    // Ground snapping: if we were on the ground last frame and it is still just below us (we
    // ran over a crest or down a bump), stick to it instead of launching into the air.
    let mut on_ground = p.grounded;
    if !on_ground && p.was_grounded && p.since_jump > 0.3 && vel.0.dot(up) < 2.0 {
        if let Some(dist) = p.ground_dist {
            if p.ground_normal.dot(up) > MAX_SLOPE_COS {
                pos.0 -= up * (dist - HALF_HEIGHT - 0.02);
                on_ground = true;
            }
        }
    }

    p.move_dir = target.normalize_or_zero();
    // Step up onto a low ledge (e.g. the end of a ramp) when it is blocking us.
    if on_ground && target.length() > 0.1 {
        let moving = (vel.0 - up * vel.0.dot(up)).length();
        if let (Some(step), true) = (p.step_ahead, moving < 0.5 * target.length()) {
            pos.0 += up * (step + 0.02);
        }
    }

    // Water: how much of the body is below the sea surface.
    let sub = if in_ship.is_some() { 0.0 } else { crate::physics::submerged(&planet.terrain, pos.0, 2.0 * HALF_HEIGHT) };
    p.submerged = sub as f32;
    // Float once the water is chest-deep, unless standing on the bottom in fairly shallow water.
    let swimming = sub > 0.72 && !(on_ground && sub < 0.85);
    p.swimming = swimming;
    let target = if on_ground && !swimming { target * (1.0 - 0.55 * sub) } else { target };

    if swimming {
        // Swim where you look (pitch down to dive), Space to rise, C or Ctrl to sink.
        let pitch = p.pitch as f64;
        let look = p.heading * pitch.cos() + up * pitch.sin();
        let mut dir = look * fwd + right * side;
        if walking && keys.pressed(KeyCode::Space) {
            dir += up;
        }
        if walking && (keys.pressed(KeyCode::KeyC) || keys.pressed(KeyCode::ControlLeft)) {
            dir -= up;
        }
        let swim_speed = if running { 3.2 } else { 1.8 };
        let target = dir.normalize_or_zero() * swim_speed;
        // Buoyancy: neutral with the head just out of the water (80% under), gently up when
        // fully submerged. Water drag pulls the velocity towards the swim stroke.
        let buoyancy = g_mag * (sub / 0.8 - 1.0);
        vel.0 = vel.0.lerp(target, 1.0 - (-2.5 * dt).exp()) + up * buoyancy * dt;
        on_ground = false;
    } else if on_ground {
        // Walk along the surface: steer velocity within the ground plane and drop any motion
        // away from it, so bumps don't bounce us. A small push keeps the feet planted.
        let n = p.ground_normal;
        let along = |v: DVec3| v - n * v.dot(n);
        let target = along(target).normalize_or_zero() * target.length();
        let v = along(vel.0).lerp(target, 1.0 - (-12.0 * dt).exp());
        vel.0 = v - n * (g_mag * dt).max(0.5);
        if walking && keys.just_pressed(KeyCode::Space) {
            vel.0 = v + up * JUMP_SPEED;
            p.since_jump = 0.0;
            on_ground = false;
        }
    } else {
        // In the air: a little air control, gravity straight down.
        let v_up = up * vel.0.dot(up);
        let v_h = (vel.0 - v_up).lerp(target, 1.0 - (-1.5 * dt).exp());
        vel.0 = v_h + v_up - up * g_mag * dt;
    }
    p.was_grounded = on_ground;

    // Safety net: never end up under the terrain (e.g. before the collision patch is built).
    if in_ship.is_some() {
        return;
    }
    let ground = crate::world::ground_point(&planet.terrain, pos.0);
    if (pos.0 - ground).dot(up) < HALF_HEIGHT - 0.3 {
        pos.0 = standing_position(&planet, pos.0);
        let inward = vel.0.dot(up).min(0.0);
        vel.0 -= up * inward;
    }
}

/// After physics: put the camera at the player's eyes (walk mode only).
pub fn sync_player_camera(
    mode: Res<ControlMode>,
    mut cam: ResMut<CameraWorld>,
    planet: Res<Planet>,
    q: Query<(&Player, &Position, &Rotation, &LinearVelocity, Option<&InShip>)>,
    ships: Query<(&Position, &Rotation, &LinearVelocity), Without<Player>>,
    mut cam_t: Query<&mut Transform, (With<FlyCam>, Without<Player>)>,
) {
    if *mode != ControlMode::Walk {
        return;
    }
    let (Ok((p, pos, rot, vel, in_ship)), Ok(mut t)) = (q.single(), cam_t.single_mut()) else { return };
    let look = Quat::from_rotation_x(p.pitch);
    if let Some(Ok((spos, srot, svel))) = in_ship.map(|s| ships.get(s.ship)) {
        // Inside a ship: map the proxy-space body onto the real ship.
        let local = pos.0 - in_ship.unwrap().proxy;
        cam.pos = spos.0 + srot.0 * (local + DVec3::Y * EYE_OFFSET);
        cam.speed = (svel.0 + srot.0 * vel.0).length();
        t.rotation = (srot.0 * rot.0).as_quat() * look;
    } else {
        let up = crate::world::up_at(pos.0);
        cam.pos = pos.0 + up * EYE_OFFSET;
        cam.speed = vel.0.length();
        t.rotation = rot.0.as_quat() * look;
    }
    if p.third_person {
        // Pull the camera back behind and slightly above the eyes, looking the same way.
        cam.pos += (t.rotation * Vec3::new(0.0, 0.35, 3.2)).as_dvec3();
    } else {
        // Eyes sit at the front of the helmet, so looking down shows the chest and legs.
        let body_forward = t.rotation * Quat::from_rotation_x(-p.pitch) * Vec3::NEG_Z;
        cam.pos += (body_forward * EYE_FORWARD).as_dvec3();
    }
    cam.altitude = crate::world::altitude(&planet.terrain, cam.pos);
}
