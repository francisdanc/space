//! Ships: a dynamic rigid body with planet gravity, flight assist, landing legs, a walkable
//! interior with a rear ramp, and a pilot seat. Mesh and colliders come from one list of parts.
//!
//! ## Walking inside a moving ship
//! Each ship has an *interior proxy*: a static copy of its colliders parked at a fixed spot in
//! physics space (near the planet centre, where nothing else exists). When the player walks in
//! through the door, their physics body moves into the proxy, in ship-local coordinates, with
//! artificial gravity toward the ship's floor. The proxy never moves, so the ship can fly, turn
//! and accelerate without throwing the player around. The camera maps the player back onto the
//! real ship every frame. Walking out of the door converts back to world space, keeping the
//! ship's velocity.

use avian3d::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::math::{DQuat, DVec2, DVec3};
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::camera::{CameraWorld, FlyCam};
use crate::physics::{PlanetGravity, TerrainCollision};
use crate::planet::Planet;
use crate::player::{ControlMode, HALF_HEIGHT, InShip, Player, basis, tangent};

/// Ship model built by tools/blender/ship.py.
const SHIP_MODEL: &str = "models/ship.glb";

/// Distance from the ship's origin down to the bottom of the landing pads.
pub const LEG_HEIGHT: f64 = 3.0;
/// Top of the cabin floor, in ship-local Y.
const FLOOR_Y: f64 = -1.5;
/// Centre of the pilot seat (ship-local, -Z is forward).
const SEAT: DVec3 = DVec3::new(0.0, -1.15, -6.0);
/// Pilot's eye position while seated.
const PILOT_EYE: DVec3 = DVec3::new(0.0, 0.1, -6.05);
/// Where the player stands after getting up from the seat (capsule centre).
const SEAT_EXIT: DVec3 = DVec3::new(0.0, FLOOR_Y + HALF_HEIGHT + 0.05, -4.6);
/// You can sit down when this close to the seat.
const SEAT_RANGE: f64 = 1.8;
/// Above this altitude flight assist stops hovering/braking and the ship flies Newtonian.
pub const ASSIST_CEILING: f64 = 60_000.0;
/// Main thrust acceleration without assist (m/s²).
const THRUST: f64 = 60.0;
/// Thrust multiplier with Shift: in the atmosphere (assist speeds) and in space (raw thrust).
const BOOST_ASSIST: f64 = 5.0;
const BOOST_SPACE: f64 = 10.0;
/// Chase camera offset behind and above the ship.
const CHASE_OFFSET: DVec3 = DVec3::new(0.0, 6.0, 26.0);
/// Interior proxies are parked here (plus a per-ship offset); nothing else exists near the
/// planet centre.
const PROXY_BASE: DVec3 = DVec3::ZERO;

#[derive(Component)]
pub struct Ship {
    /// Flight assist: hover in place, hold velocity targets, cancel drift.
    pub assist: bool,
    pub cockpit_view: bool,
    /// Engines hold the ship in the air when nobody is at the controls.
    pub engines_on: bool,
    /// Where this ship's interior proxy sits in physics space.
    pub proxy: DVec3,
    /// The travel autopilot is flying (nav.rs); manual controls stand down.
    pub autopilot: bool,
}

/// Static copy of a ship's colliders used for walking inside it.
#[derive(Component)]
pub struct InteriorProxy;

/// Which ship the player is flying, if any.
#[derive(Resource, Default)]
pub struct Piloting(pub Option<Entity>);

/// Shown in the HUD when the player is next to the pilot seat.
#[derive(Resource, Default)]
pub struct CanBoard(pub bool);

/// Scripted input for automated tests (see debug.rs).
#[derive(Resource, Default, Clone, Copy)]
pub struct ShipInput {
    pub forward: f64,
    pub side: f64,
    pub vertical: f64,
    pub toggle_board: bool,
    /// Also apply this input to a ship nobody is piloting (to test walking in a moving ship).
    pub autopilot: bool,
    pub boost: bool,
    pub brake: bool,
}

struct Part {
    /// What this piece is; the Blender model script uses it to pick materials and details.
    name: &'static str,
    center: DVec3,
    size: DVec3,
    /// Rotation about the ship's X axis, in radians (used for the ramp).
    tilt: f64,
    color: [f32; 3],
    visible: bool,
    collide: bool,
}

fn part(name: &'static str, center: [f64; 3], size: [f64; 3], color: [f32; 3]) -> Part {
    Part { name, center: DVec3::from(center), size: DVec3::from(size), tilt: 0.0, color, visible: true, collide: true }
}

fn parts() -> Vec<Part> {
    let hull = [0.36, 0.38, 0.41];
    let dark = [0.08, 0.08, 0.09];
    let floor = [0.16, 0.17, 0.18];
    let seat = [0.25, 0.1, 0.05];
    let mut v = vec![
        // Cabin shell: interior is x ±3.1, y -1.5..1.5, z -8.0..8.0.
        part("floor", [0.0, -1.6, 0.0], [6.6, 0.2, 16.4], floor),
        part("roof", [0.0, 1.6, 0.0], [6.6, 0.2, 16.4], hull),
        part("wall", [3.2, 0.0, 0.0], [0.2, 3.0, 16.4], hull),
        part("wall", [-3.2, 0.0, 0.0], [0.2, 3.0, 16.4], hull),
        // Front wall with a window.
        part("front_lower", [0.0, -0.95, -8.1], [6.6, 1.1, 0.2], hull),
        part("window_frame", [0.0, 1.3, -8.1], [6.6, 0.4, 0.2], dark),
        part("window_frame", [2.85, 0.35, -8.1], [0.5, 1.5, 0.2], dark),
        part("window_frame", [-2.85, 0.35, -8.1], [0.5, 1.5, 0.2], dark),
        Part { visible: false, ..part("glass", [0.0, 0.35, -8.1], [5.2, 1.5, 0.06], dark) },
        part("nose", [0.0, -0.95, -9.0], [5.0, 1.1, 1.6], hull),
        // Cockpit: dashboard, screen, pilot seat.
        part("dash", [0.0, -0.65, -7.5], [4.2, 0.5, 1.0], dark),
        Part { collide: false, ..part("screen", [0.0, -0.39, -7.5], [2.0, 0.02, 0.6], [0.1, 0.45, 0.6]) },
        part("seat", [0.0, -1.15, -6.0], [0.8, 0.7, 0.8], seat),
        part("seat_back", [0.0, -0.4, -5.6], [0.8, 1.2, 0.1], seat),
        // Rear wall with a 2.2 m wide, 2.1 m tall doorway.
        part("rear_wall", [2.15, 0.0, 8.1], [2.1, 3.0, 0.2], hull),
        part("rear_wall", [-2.15, 0.0, 8.1], [2.1, 3.0, 0.2], hull),
        part("door_top", [0.0, 1.05, 8.1], [2.2, 0.9, 0.2], hull),
        // Roof stripe.
        Part { collide: false, ..part("stripe", [0.0, 1.72, 1.5], [6.65, 0.05, 8.0], [0.8, 0.28, 0.04]) },
    ];
    // Ramp from the doorway down to pad level.
    let len: f64 = 3.6;
    let thick = 0.15;
    let a = ((LEG_HEIGHT + FLOOR_Y) / len).asin();
    let dir = DVec3::new(0.0, -a.sin(), a.cos());
    let n = DVec3::new(0.0, a.cos(), a.sin());
    let start = DVec3::new(0.0, FLOOR_Y, 8.15);
    v.push(Part {
        center: start + dir * (len * 0.5) - n * (thick * 0.5),
        size: DVec3::new(2.2, thick, len),
        tilt: a,
        ..part("ramp", [0.0; 3], [1.0; 3], floor)
    });
    for s in [-1.0, 1.0] {
        v.push(part("engine", [3.9 * s, 0.3, 5.5], [1.2, 1.8, 4.5], dark));
        v.push(part("wing", [5.6 * s, -0.6, 2.0], [3.2, 0.3, 5.0], hull));
        for z in [-5.5, 5.5] {
            v.push(part("leg", [2.6 * s, -2.35, z], [0.4, 1.3, 0.4], dark));
            v.push(part("pad", [2.6 * s, -2.95, z], [1.1, 0.1, 1.1], dark));
        }
    }
    v
}

/// Write the parts list as JSON (for the Blender model script, tools/blender/ship.py).
pub fn dump_parts(path: &str) -> std::io::Result<()> {
    let items: Vec<String> = parts()
        .iter()
        .map(|p| {
            format!(
                "{{\"name\":\"{}\",\"center\":[{},{},{}],\"size\":[{},{},{}],\"tilt\":{},\"visible\":{},\"collide\":{}}}",
                p.name, p.center.x, p.center.y, p.center.z, p.size.x, p.size.y, p.size.z, p.tilt, p.visible, p.collide
            )
        })
        .collect();
    std::fs::write(path, format!("[\n{}\n]\n", items.join(",\n")))
}

fn ship_mesh(parts: &[Part]) -> Mesh {
    let mut pos = Vec::new();
    let mut nor = Vec::new();
    let mut col = Vec::new();
    let mut idx = Vec::new();
    for p in parts.iter().filter(|p| p.visible) {
        let rot = Quat::from_rotation_x(p.tilt as f32);
        let center = p.center.as_vec3();
        let h = p.size.as_vec3() * 0.5;
        let c = [p.color[0], p.color[1], p.color[2], 1.0];
        // Each face: normal and two in-plane axes with (u × v) = normal (CCW from outside).
        for (n, u, v) in [
            (Vec3::X, Vec3::Y, Vec3::Z),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::Z, Vec3::X),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::Y, Vec3::X),
        ] {
            let base = pos.len() as u32;
            let fc = n * h;
            let (du, dv) = (u * h, v * h);
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                pos.push((center + rot * (fc + du * a + dv * b)).to_array());
                nor.push((rot * n).to_array());
                col.push(c);
            }
            idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nor)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
        .with_inserted_indices(Indices::U32(idx))
}

fn ship_collider(parts: &[Part]) -> Collider {
    Collider::compound(
        parts
            .iter()
            .filter(|p| p.collide)
            .map(|p| {
                (
                    Position(p.center),
                    Rotation(DQuat::from_rotation_x(p.tilt)),
                    Collider::cuboid(p.size.x, p.size.y, p.size.z),
                )
            })
            .collect(),
    )
}

/// Is a point (ship-local) inside the cabin? `margin` > 0 makes the box bigger.
fn in_cabin(p: DVec3, margin: f64) -> bool {
    p.x.abs() < 3.1 + margin && p.y > -1.5 - margin && p.y < 1.5 + margin && p.z > -8.0 - margin && p.z < 8.05 + margin
}

/// Where the first ship is parked (a base's landing pad), if not in front of the player.
#[derive(Resource)]
pub struct ShipSpawn(pub Option<DVec3>);

/// Spawn a ship parked in front of the player, rear ramp facing them.
pub fn spawn_ship(
    mut commands: Commands,
    planet: Res<Planet>,
    player: Query<(&Player, &Position)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<AssetServer>,
    spawn_at: Option<Res<ShipSpawn>>,
    mut count: Local<u32>,
) {
    let Ok((p, ppos)) = player.single() else { return };
    // On a base's pad when starting there, else 26 m ahead of the player.
    let spot = spawn_at.as_ref().and_then(|s| s.0).unwrap_or(ppos.0 + p.heading * 26.0);
    let dir = crate::world::up_at(spot);
    let pos = crate::world::surface_point(&planet.terrain, spot) + dir * (LEG_HEIGHT + 0.3);
    let forward = tangent(p.heading, dir);
    let proxy = PROXY_BASE + DVec3::new(0.0, 0.0, 200.0 * *count as f64);
    *count += 1;

    let parts = parts();
    let collider = ship_collider(&parts);
    commands.spawn((
        InteriorProxy,
        RigidBody::Static,
        collider.clone(),
        Position(proxy),
        Rotation::default(),
        Transform::default(),
    ));
    commands
        .spawn((
            Ship { assist: true, cockpit_view: true, engines_on: false, proxy, autopilot: false },
            (RigidBody::Dynamic, collider, ColliderDensity(60.0), Friction::new(0.9), SweptCcd::default()),
            Position(pos),
            Rotation(basis(forward, dir)),
            LinearVelocity::default(),
            AngularVelocity::default(),
            Transform::default(),
            Visibility::default(),
            PlanetGravity,
            TerrainCollision::default(),
            // The cabin is hollow: the ship floats with about half its hull under water.
            crate::physics::Floats { height: 3.5, lift: 2.2 },
        ))
        .with_children(|c| {
            // The Blender model (tools/blender/ship.py) if it has been built, else plain boxes.
            if crate::game_dir().join("assets").join(SHIP_MODEL).exists() {
                c.spawn(SceneRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(SHIP_MODEL))));
            } else {
                c.spawn((
                    Mesh3d(meshes.add(ship_mesh(&parts))),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: Color::WHITE,
                        perceptual_roughness: 0.6,
                        metallic: 0.05,
                        ..default()
                    })),
                ));
            }
            // Cabin lights.
            for z in [-3.5, 3.0] {
                c.spawn((
                    PointLight {
                        intensity: 120_000.0,
                        range: 14.0,
                        color: Color::srgb(1.0, 0.93, 0.82),
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_xyz(0.0, 1.2, z),
                ));
            }
        });
}

/// Ship-local velocity of a point attached to the ship.
fn point_velocity(ship_vel: DVec3, ship_ang: DVec3, ship_pos: DVec3, p: DVec3) -> DVec3 {
    ship_vel + ship_ang.cross(p - ship_pos)
}

/// Move the player between world space and a ship's interior proxy as they cross the doorway.
pub fn interior_transfer(
    mut commands: Commands,
    mode: Res<ControlMode>,
    ships: Query<(Entity, &Ship, &Position, &Rotation, &LinearVelocity, &AngularVelocity), Without<Player>>,
    mut player: Query<(Entity, &mut Player, &mut Position, &mut LinearVelocity, Option<&InShip>), Without<Ship>>,
) {
    if *mode == ControlMode::Pilot {
        return;
    }
    let Ok((pe, mut p, mut pos, mut vel, in_ship)) = player.single_mut() else { return };
    match in_ship {
        None => {
            for (se, ship, spos, srot, svel, sang) in &ships {
                let local = srot.0.inverse() * (pos.0 - spos.0);
                if in_cabin(local, 0.0) {
                    let rel = vel.0 - point_velocity(svel.0, sang.0, spos.0, pos.0);
                    pos.0 = ship.proxy + local;
                    vel.0 = srot.0.inverse() * rel;
                    p.heading = srot.0.inverse() * p.heading;
                    commands.entity(pe).insert(InShip { ship: se, proxy: ship.proxy });
                    break;
                }
            }
        }
        Some(inside) => {
            let Ok((_, _, spos, srot, svel, sang)) = ships.get(inside.ship) else {
                commands.entity(pe).remove::<InShip>();
                return;
            };
            let local = pos.0 - inside.proxy;
            if !in_cabin(local, 0.3) {
                let world = spos.0 + srot.0 * local;
                vel.0 = point_velocity(svel.0, sang.0, spos.0, world) + srot.0 * vel.0;
                pos.0 = world;
                p.heading = srot.0 * p.heading;
                commands.entity(pe).remove::<InShip>();
            }
        }
    }
}

/// F at the pilot seat sits down / gets up.
#[allow(clippy::too_many_arguments)]
pub fn board_or_exit(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    script: Option<Res<ShipInput>>,
    mut mode: ResMut<ControlMode>,
    mut piloting: ResMut<Piloting>,
    mut can_board: ResMut<CanBoard>,
    planet: Res<Planet>,
    mut ships: Query<(&mut Ship, &Position), Without<Player>>,
    mut player: Query<(Entity, &mut Player, &mut Position, &mut LinearVelocity, Option<&InShip>), Without<Ship>>,
) {
    let Ok((pe, mut p, mut ppos, mut pvel, in_ship)) = player.single_mut() else { return };
    let pressed = keys.just_pressed(KeyCode::KeyF) || script.is_some_and(|s| s.toggle_board);

    match *mode {
        ControlMode::Walk => {
            let near_seat = in_ship.is_some_and(|s| (ppos.0 - s.proxy - SEAT).length() < SEAT_RANGE);
            can_board.0 = near_seat;
            if near_seat && pressed {
                let ship = in_ship.unwrap().ship;
                if let Ok((mut s, _)) = ships.get_mut(ship) {
                    s.engines_on = true;
                }
                piloting.0 = Some(ship);
                commands.entity(pe).insert((RigidBodyDisabled, ColliderDisabled));
                *mode = ControlMode::Pilot;
            }
        }
        ControlMode::Pilot => {
            can_board.0 = false;
            if !pressed {
                return;
            }
            let Some(ship) = piloting.0.take() else { return };
            if let Ok((mut s, spos)) = ships.get_mut(ship) {
                // Stay hovering if airborne, power down if landed.
                let clearance = crate::world::altitude(&planet.terrain, spos.0) - LEG_HEIGHT;
                s.engines_on = clearance > 1.0;
                ppos.0 = s.proxy + SEAT_EXIT;
            }
            pvel.0 = DVec3::ZERO;
            // Stand up facing the back of the cabin, towards the door.
            p.heading = DVec3::Z;
            p.pitch = 0.0;
            commands.entity(pe).remove::<(RigidBodyDisabled, ColliderDisabled)>();
            *mode = ControlMode::Walk;
        }
        ControlMode::Fly => can_board.0 = false,
    }
}

/// Flight controls for the piloted ship; engines-on ships without a pilot hold their position.
/// Mouse pitches/yaws, Q/E rolls, WASD + Space/C thrust, Shift boosts, B brakes, X toggles
/// flight assist, V toggles cockpit/chase view.
#[allow(clippy::too_many_arguments)]
pub fn ship_control(
    time: Res<Time>,
    mode: Res<ControlMode>,
    piloting: Res<Piloting>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    windows: Query<&Window, With<PrimaryWindow>>,
    script: Option<Res<ShipInput>>,
    planet: Res<Planet>,
    mut ships: Query<(Entity, &mut Ship, &Position, &Rotation, &mut LinearVelocity, &mut AngularVelocity)>,
) {
    let dt = time.delta_secs_f64().max(1e-4);
    let s = script.map(|s| *s).unwrap_or_default();
    for (entity, mut ship, pos, rot, mut vel, mut ang) in &mut ships {
        let piloted = *mode == ControlMode::Pilot && piloting.0 == Some(entity);
        if ship.autopilot {
            continue;
        }
        if !piloted && !ship.engines_on {
            // Parking brake: a powered-down ship resting on the ground stays put instead of
            // creeping down slopes.
            let clearance = crate::world::altitude(&planet.terrain, pos.0) - LEG_HEIGHT;
            if clearance < 0.6 && vel.0.length() < 1.5 && ang.0.length() < 0.3 {
                let k = (-8.0 * dt).exp();
                vel.0 *= k;
                ang.0 *= k;
            }
            continue;
        }
        let mut local_rate = DVec3::ZERO;
        let mut input = DVec3::ZERO;
        let mut boosting = false;
        let mut braking = false;
        if piloted {
            if keys.just_pressed(KeyCode::KeyX) {
                ship.assist = !ship.assist;
            }
            if keys.just_pressed(KeyCode::KeyV) {
                ship.cockpit_view = !ship.cockpit_view;
            }
            let locked = windows
                .single()
                .map(|w| w.cursor_options.grab_mode != CursorGrabMode::None)
                .unwrap_or(false);
            let m = if locked { motion.delta.as_dvec2() } else { DVec2::ZERO };
            let key = |k: KeyCode| if keys.pressed(k) { 1.0 } else { 0.0 };
            let max_rate = 2.5;
            local_rate = DVec3::new(
                (-m.y * 0.0025 / dt).clamp(-max_rate, max_rate),
                (-m.x * 0.0025 / dt).clamp(-max_rate, max_rate),
                (key(KeyCode::KeyQ) - key(KeyCode::KeyE)) * 1.8,
            );
            input = DVec3::new(
                key(KeyCode::KeyD) - key(KeyCode::KeyA),
                key(KeyCode::Space) - key(KeyCode::KeyC),
                key(KeyCode::KeyS) - key(KeyCode::KeyW),
            );
            boosting = keys.pressed(KeyCode::ShiftLeft);
            braking = keys.pressed(KeyCode::KeyB);
        }
        if piloted || s.autopilot {
            input += DVec3::new(s.side, s.vertical, -s.forward);
            boosting |= s.boost;
            braking |= s.brake;
        }
        let input = input.clamp(DVec3::NEG_ONE, DVec3::ONE);
        ang.0 = ang.0.lerp(rot.0 * local_rate, 1.0 - (-15.0 * dt).exp());

        let up = crate::world::up_at(pos.0);
        let altitude = crate::world::altitude(&planet.terrain, pos.0);
        // Above the assist ceiling the ship flies purely Newtonian, so gravity bends its path
        // into an orbit instead of being cancelled out.
        let in_space = altitude > ASSIST_CEILING;
        let space_accel = THRUST * if boosting { BOOST_SPACE } else { 1.0 };
        if braking && (in_space || !ship.assist) {
            // B: burn against the current motion until stopped (then gravity takes over, and
            // flight assist catches the ship again below the assist ceiling).
            let brake = THRUST * BOOST_SPACE;
            let stop = (-vel.0).clamp_length_max(brake * dt);
            vel.0 += stop;
            continue;
        }
        let input = if braking { DVec3::ZERO } else { input };
        if (ship.assist || !piloted) && !in_space {
            let boost = if boosting { BOOST_ASSIST } else { 1.0 };
            // Velocity targets that scale with altitude: careful near the ground, fast up high.
            let max_speed = (altitude * 0.5).clamp(40.0, 2_500.0) * boost;
            let mut target = rot.0 * input * max_speed;
            // Limit descent speed near the ground so landings are gentle (~3 m/s at touchdown).
            let max_descent = (altitude - LEG_HEIGHT).max(0.0) * 0.5 + 3.0;
            let down = -target.dot(up);
            if down > max_descent {
                target += up * (down - max_descent);
            }
            let accel = (max_speed * 0.6).max(25.0);
            let dv = (target - vel.0).clamp_length_max(accel * dt);
            // Also cancel gravity, so letting go of the controls means hovering in place.
            vel.0 += dv - crate::world::gravity(pos.0) * dt;
        } else if piloted || s.autopilot {
            vel.0 += rot.0 * input * space_accel * dt;
        }
    }
}

/// Keep ships above the terrain even if a fast impact tunnels through the collision patch.
pub fn ship_ground_safety(planet: Res<Planet>, mut ships: Query<(&mut Position, &mut LinearVelocity), With<Ship>>) {
    for (mut pos, mut vel) in &mut ships {
        let up = crate::world::up_at(pos.0);
        let ground = crate::world::ground_point(&planet.terrain, pos.0);
        if (pos.0 - ground).dot(up) < 0.5 {
            pos.0 = ground + up * LEG_HEIGHT;
            let inward = vel.0.dot(up).min(0.0);
            vel.0 -= up * inward;
        }
    }
}

/// After physics: pilot sits in the seat, camera follows the ship.
pub fn sync_ship_camera(
    mode: Res<ControlMode>,
    piloting: Res<Piloting>,
    planet: Res<Planet>,
    mut cam: ResMut<CameraWorld>,
    ships: Query<(&Ship, &Position, &Rotation, &LinearVelocity), Without<Player>>,
    mut player: Query<&mut Position, (With<Player>, Without<Ship>)>,
    mut cam_t: Query<&mut Transform, With<FlyCam>>,
) {
    if *mode != ControlMode::Pilot {
        return;
    }
    let Some(Ok((ship, pos, rot, vel))) = piloting.0.map(|e| ships.get(e)) else { return };
    if let Ok(mut ppos) = player.single_mut() {
        ppos.0 = ship.proxy + SEAT + DVec3::new(0.0, 0.5, 0.0);
    }
    let offset = if ship.cockpit_view { PILOT_EYE } else { CHASE_OFFSET };
    cam.pos = pos.0 + rot.0 * offset;
    cam.altitude = crate::world::altitude(&planet.terrain, cam.pos);
    cam.speed = vel.0.length();
    if let Ok(mut t) = cam_t.single_mut() {
        // Chase view looks slightly down at the ship.
        let tilt = if ship.cockpit_view { DQuat::IDENTITY } else { DQuat::from_rotation_x(-0.18) };
        t.rotation = (rot.0 * tilt).as_quat();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cabin_geometry_is_consistent() {
        // Standing on the floor just inside the door counts as inside; the seat is inside.
        assert!(in_cabin(DVec3::new(0.0, FLOOR_Y + HALF_HEIGHT, 7.9), 0.0));
        assert!(in_cabin(SEAT_EXIT, 0.0));
        assert!((SEAT_EXIT - SEAT).length() < SEAT_RANGE);
        // The bottom end of the ramp is at pad level.
        let ramp = parts().into_iter().find(|p| p.tilt != 0.0).unwrap();
        let end = ramp.center + DQuat::from_rotation_x(ramp.tilt) * DVec3::new(0.0, ramp.size.y * 0.5, ramp.size.z * 0.5);
        assert!((end.y + LEG_HEIGHT).abs() < 0.05, "ramp end at y={}", end.y);
    }
}
