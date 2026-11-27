//! Navigation: on-screen markers for destinations, a selected target with flight guidance, and
//! a travel autopilot.
//!
//! - Markers for both bases, the moon and your factory sites, with distances; the selected
//!   target is highlighted and stays on screen (pinned to the edge when behind you).
//! - N cycles the target. The guidance line shows distance, closing speed, time to arrival,
//!   and when to start braking with full thrust.
//! - T (piloting) engages the travel autopilot: climb away from the surface, cruise (routing
//!   around a planet in the way), brake, and settle into a hover above the destination; T or
//!   any flight key hands control back.

use avian3d::prelude::*;
use bevy::math::DVec3;
use bevy::prelude::*;

use crate::camera::{CameraWorld, FlyCam};
use crate::economy::BASE_NAMES;
use crate::planet::Planet;
use crate::player::ControlMode;
use crate::ship::{Piloting, Ship};
use crate::world::{Body, MOON_CENTER, MOON_RADIUS};

/// Thrust available to the autopilot's travel drive (m/s²), and the braking the guidance assumes
/// for manual flight (full boost in space).
const TRAVEL_ACCEL: f64 = 1500.0;
const MANUAL_ACCEL: f64 = 600.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dest {
    Base(usize),
    Moon,
    Site(usize),
}

/// Selected destination and the autopilot's route.
#[derive(Resource)]
pub struct Nav {
    pub target: usize,
    pub route: Vec<DVec3>,
    pub engaged: Option<Entity>,
    pub status: String,
}

impl Default for Nav {
    fn default() -> Self {
        Self { target: 1, route: Vec::new(), engaged: None, status: String::new() }
    }
}

/// All destinations: (what, name, position).
pub fn destinations(planet: &Planet, factory: &crate::factory::Factory) -> Vec<(Dest, String, DVec3)> {
    let t = &planet.terrain;
    let mut v = vec![
        (Dest::Base(0), BASE_NAMES[0].to_string(), crate::economy::landing_spot(t, 0)),
        (Dest::Base(1), BASE_NAMES[1].to_string(), crate::economy::landing_spot(t, 1)),
        (Dest::Moon, "Selene (moon)".to_string(), MOON_CENTER),
    ];
    for (i, s) in factory.sites.iter().enumerate() {
        v.push((Dest::Site(i), format!("Factory site {}", i + 1), DVec3::from(s.origin)));
    }
    v
}

/// Where the autopilot should stop: hovering above the destination.
fn arrival_point(dest: Dest, pos: DVec3) -> DVec3 {
    let up = crate::world::up_at(pos);
    match dest {
        Dest::Moon => MOON_CENTER + (pos - MOON_CENTER).normalize() * (MOON_RADIUS + 60_000.0),
        _ => pos + up * 120.0,
    }
}

/// Route from `from` to `to`: climb to a safe altitude, cruise (via a detour if a body's
/// sphere is in the way), drop to above the destination, then descend.
fn plan(from: DVec3, to: DVec3, dest: Dest) -> Vec<DVec3> {
    let mut route = Vec::new();
    let b0 = crate::world::nearest(from);
    let climb = b0.center() + (from - b0.center()).normalize() * (b0.radius() + 80_000.0);
    if (from - b0.center()).length() < b0.radius() + 60_000.0 {
        route.push(climb);
    }
    let b1 = crate::world::nearest(to);
    let approach = if dest == Dest::Moon { to } else { b1.center() + (to - b1.center()).normalize() * (b1.radius() + 60_000.0) };
    let start = *route.last().unwrap_or(&from);
    // Detour around any body the straight line would pass through.
    for b in [Body::Planet, Body::Moon] {
        let (a, c) = (start, approach);
        let ab = c - a;
        let t = ((b.center() - a).dot(ab) / ab.length_squared().max(1.0)).clamp(0.0, 1.0);
        let closest = a + ab * t;
        let clear = b.radius() + 100_000.0;
        if (closest - b.center()).length() < clear && t > 0.0 && t < 1.0 {
            let out = (closest - b.center()).try_normalize().unwrap_or_else(|| ab.any_orthonormal_vector());
            route.push(b.center() + out * (b.radius() * 1.6));
        }
    }
    if dest != Dest::Moon {
        route.push(approach);
    }
    route.push(to);
    route
}

#[derive(Component)]
struct Marker(usize);
#[derive(Component)]
struct GuidanceText;

pub struct NavPlugin;

impl Plugin for NavPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Nav>()
            .add_systems(Startup, setup)
            .add_systems(Update, (keys, autopilot).chain().after(crate::ship::ship_control).before(crate::ship::ship_ground_safety))
            .add_systems(PostUpdate, (markers, guidance).after(bevy::transform::TransformSystem::TransformPropagate));
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        GuidanceText,
        Text::new(""),
        TextFont { font_size: 17.0, ..default() },
        TextColor(Color::srgb(0.55, 0.9, 1.0)),
        TextLayout::new_with_justify(JustifyText::Center),
        Node { position_type: PositionType::Absolute, top: Val::Px(8.0), width: Val::Percent(100.0), justify_content: JustifyContent::Center, ..default() },
    ));
}

#[allow(clippy::too_many_arguments)]
fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<ControlMode>,
    piloting: Res<Piloting>,
    planet: Res<Planet>,
    factory: Res<crate::factory::Factory>,
    mut nav: ResMut<Nav>,
    mut ships: Query<(&mut Ship, &Position)>,
    mut toasts: ResMut<crate::forage::Toasts>,
) {
    let dests = destinations(&planet, &factory);
    if keys.just_pressed(KeyCode::KeyN) && !crate::factory::building() {
        nav.target = (nav.target + 1) % dests.len();
        toasts.push(format!("Nav target: {}", dests[nav.target].1), Color::srgb(0.55, 0.9, 1.0));
    }
    nav.target = nav.target.min(dests.len() - 1);
    let Some(entity) = piloting.0.filter(|_| *mode == ControlMode::Pilot) else {
        return;
    };
    let Ok((mut ship, pos)) = ships.get_mut(entity) else { return };
    let manual = [KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyA, KeyCode::KeyD, KeyCode::Space, KeyCode::KeyC, KeyCode::KeyB].iter().any(|k| keys.just_pressed(*k));
    if ship.autopilot && (keys.just_pressed(KeyCode::KeyT) || manual) {
        ship.autopilot = false;
        ship.assist = true;
        nav.engaged = None;
        toasts.push("Autopilot off", Color::srgb(0.55, 0.9, 1.0));
    } else if !ship.autopilot && keys.just_pressed(KeyCode::KeyT) {
        let name = engage(&mut nav, &dests, entity, &mut ship, pos.0);
        toasts.push(format!("Autopilot: travelling to {name}"), Color::srgb(0.55, 0.9, 1.0));
    }
}

/// Plan a route to the selected target and hand the ship to the autopilot. Returns the name.
pub fn engage(nav: &mut Nav, dests: &[(Dest, String, DVec3)], entity: Entity, ship: &mut Ship, from: DVec3) -> String {
    let (dest, name, at) = dests[nav.target].clone();
    nav.route = plan(from, arrival_point(dest, at), dest);
    nav.engaged = Some(entity);
    ship.autopilot = true;
    ship.engines_on = true;
    name
}

/// Fly the route: chase each waypoint at the fastest speed that can still stop at the last one,
/// with gravity cancelled, turning the ship to face where it is going.
fn autopilot(time: Res<Time>, planet: Res<Planet>, mut nav: ResMut<Nav>, mut ships: Query<(&mut Ship, &Position, &Rotation, &mut LinearVelocity, &mut AngularVelocity)>) {
    let Some(entity) = nav.engaged else {
        nav.status.clear();
        return;
    };
    let Ok((mut ship, pos, rot, mut vel, mut ang)) = ships.get_mut(entity) else {
        nav.engaged = None;
        return;
    };
    if !ship.autopilot {
        nav.engaged = None;
        return;
    }
    let dt = time.delta_secs_f64().max(1e-4);
    // Drop waypoints we have reached.
    while nav.route.len() > 1 && (nav.route[0] - pos.0).length() < (vel.0.length() * 2.0).max(3_000.0) {
        nav.route.remove(0);
    }
    let Some(&wp) = nav.route.first() else {
        nav.engaged = None;
        return;
    };
    let last = nav.route.len() == 1;
    let to = wp - pos.0;
    let dist = to.length();
    let alt = crate::world::altitude(&planet.terrain, pos.0);
    // Speed limits near the ground, so climbs and descents are gentle.
    let vmax = if alt < 5_000.0 { 150.0 + alt * 0.1 } else if alt < 60_000.0 { 800.0 + alt * 0.05 } else { 250_000.0 };
    let remaining: f64 = dist + nav.route.windows(2).map(|w| (w[1] - w[0]).length()).sum::<f64>();
    // As fast as allowed, but always able to slow down for the turn at this waypoint: the
    // speed kept through the corner shrinks with how sharp the turn is (and is zero at the end).
    let a = TRAVEL_ACCEL * 0.8;
    let corner = match nav.route.get(1) {
        Some(&next) => {
            let cos = to.normalize_or_zero().dot((next - wp).normalize_or_zero()).max(0.0);
            (2.0 * a * (remaining - dist)).sqrt().min(250_000.0) * cos * cos
        }
        None => 0.0,
    };
    let mut speed = vmax.min((2.0 * a * dist + corner * corner).sqrt());
    if last {
        speed = speed.min(dist * 0.6 + 1.0);
    }
    let want = to.normalize_or_zero() * speed;
    let dv = (want - vel.0).clamp_length_max(TRAVEL_ACCEL * dt);
    vel.0 += dv - crate::world::gravity(pos.0) * dt;
    // Face the direction of travel (level with the local horizon when slow).
    let up = crate::world::up_at(pos.0);
    let fwd_dir = if vel.0.length() > 30.0 { vel.0.normalize() } else { crate::player::tangent(rot.0 * DVec3::NEG_Z, up) };
    let fwd_dir = if fwd_dir.cross(up).length() < 0.05 { crate::player::tangent(rot.0 * DVec3::NEG_Z, up) } else { fwd_dir };
    let target_rot = crate::player::basis(fwd_dir, up);
    let err = target_rot * rot.0.inverse();
    let (axis, angle) = err.to_axis_angle();
    let angle = if angle > std::f64::consts::PI { angle - std::f64::consts::TAU } else { angle };
    ang.0 = axis * angle * 2.5;
    // Arrived: hover there and hand back.
    if last && dist < 40.0 && vel.0.length() < 8.0 {
        ship.autopilot = false;
        ship.assist = true;
        ship.engines_on = true;
        nav.engaged = None;
        nav.route.clear();
        nav.status = "Arrived".into();
        return;
    }
    let eta = remaining / vel.0.length().max(1.0);
    let phase = if alt < 60_000.0 && nav.route.len() > 1 { "climbing" } else if last { "arriving" } else { "cruising" };
    nav.status = format!("AUTOPILOT {phase}  {}  ETA {}", fmt_dist(remaining), fmt_time(eta));
}

pub fn fmt_dist(m: f64) -> String {
    if m < 1000.0 { format!("{m:.0} m") } else if m < 100_000.0 { format!("{:.1} km", m / 1000.0) } else { format!("{:.0} km", m / 1000.0) }
}

fn fmt_time(s: f64) -> String {
    if s > 3600.0 * 10.0 { "--".into() } else if s >= 60.0 { format!("{}m {:02}s", (s / 60.0) as u64, (s % 60.0) as u64) } else { format!("{s:.0}s") }
}

/// On-screen markers: projected destinations (pinned to the edge when off screen).
#[allow(clippy::too_many_arguments)]
fn markers(
    mut commands: Commands,
    cam: Res<CameraWorld>,
    planet: Res<Planet>,
    factory: Res<crate::factory::Factory>,
    nav: Res<Nav>,
    camera: Query<(&Camera, &GlobalTransform), With<FlyCam>>,
    mut existing: Query<(Entity, &Marker, &mut Node, &mut Text, &mut TextColor)>,
) {
    let Ok((camera, cam_t)) = camera.single() else { return };
    let Some(size) = camera.logical_viewport_size() else { return };
    let dests = destinations(&planet, &factory);
    let mut seen = vec![false; dests.len()];
    // Screen spots already labelled: unselected markers that would overlap one are hidden.
    let mut placed: Vec<Vec2> = Vec::new();
    for (e, m, mut node, mut text, mut color) in &mut existing {
        let Some((_, name, at)) = dests.get(m.0) else {
            commands.entity(e).despawn();
            continue;
        };
        seen[m.0] = true;
        let rel = *at - cam.pos;
        let dist = rel.length();
        let selected = m.0 == nav.target;
        // Hide markers for places right next to you (except the selected one).
        if dist < 60.0 && !selected {
            node.display = Display::None;
            continue;
        }
        node.display = Display::Flex;
        let dir = (rel / dist).as_vec3();
        let ahead = cam_t.forward().dot(dir) > 0.0;
        let p = camera.world_to_viewport(cam_t, dir * 1000.0).ok().filter(|_| ahead);
        let (x, y, edge) = match p {
            Some(v) if v.x > 0.0 && v.y > 0.0 && v.x < size.x && v.y < size.y => (v.x, v.y, false),
            _ if selected => {
                // Pin to the screen edge in the target's direction.
                let local = cam_t.compute_transform().rotation.inverse() * dir;
                let d2 = Vec2::new(local.x, -local.y).normalize_or(Vec2::X);
                let c = size * 0.5;
                let k = ((c.x - 40.0) / d2.x.abs().max(1e-3)).min((c.y - 30.0) / d2.y.abs().max(1e-3));
                let p = c + d2 * k;
                (p.x, p.y, true)
            }
            _ => {
                node.display = Display::None;
                continue;
            }
        };
        // Keep the whole label (three lines) on screen.
        let (x, y) = (x.clamp(60.0, size.x - 60.0), y.clamp(60.0, size.y - 58.0));
        if !selected && placed.iter().any(|q| (q.x - x).abs() < 110.0 && (q.y - y).abs() < 50.0) {
            node.display = Display::None;
            continue;
        }
        placed.push(Vec2::new(x, y));
        node.left = Val::Px(x - 60.0);
        node.top = Val::Px(y - 10.0);
        let arrow = if edge { "> " } else { "<> " };
        let s = format!("{arrow}{name}\n{}", fmt_dist(dist));
        if text.0 != s {
            text.0 = s;
        }
        color.0 = if selected { Color::srgb(1.0, 0.85, 0.3) } else { Color::srgba(0.55, 0.9, 1.0, 0.75) };
    }
    for (i, s) in seen.iter().enumerate() {
        if !s {
            commands.spawn((
                Marker(i),
                Text::new(""),
                TextFont { font_size: 14.0, ..default() },
                TextColor(Color::WHITE),
                TextLayout::new_with_justify(JustifyText::Center),
                Node { position_type: PositionType::Absolute, width: Val::Px(120.0), ..default() },
            ));
        }
    }
}

/// Guidance for the selected target: distance, closing speed, ETA, and when to brake.
#[allow(clippy::too_many_arguments)]
fn guidance(
    cam: Res<CameraWorld>,
    mode: Res<ControlMode>,
    piloting: Res<Piloting>,
    planet: Res<Planet>,
    factory: Res<crate::factory::Factory>,
    nav: Res<Nav>,
    ships: Query<&LinearVelocity, With<Ship>>,
    mut text: Query<&mut Text, With<GuidanceText>>,
) {
    let Ok(mut t) = text.single_mut() else { return };
    let dests = destinations(&planet, &factory);
    let Some((_, name, at)) = dests.get(nav.target) else { return };
    let rel = *at - cam.pos;
    let dist = rel.length();
    let mut s = format!("NAV  {name}  {}", fmt_dist(dist));
    let piloted = piloting.0.filter(|_| *mode == ControlMode::Pilot).and_then(|e| ships.get(e).ok());
    if let Some(vel) = piloted {
        let closing = vel.0.dot(rel / dist.max(1.0));
        s += &format!("   closing {}/s", fmt_dist(closing.abs()).replace(' ', ""));
        if closing > 1.0 {
            s += &format!("   ETA {}", fmt_time(dist / closing));
            let stopping = closing * closing / (2.0 * MANUAL_ACCEL);
            if stopping > dist * 0.9 && dist > 2_000.0 {
                s += "   BRAKE NOW (B)";
            }
        }
        if nav.engaged.is_some() {
            s = format!("{s}\n{}   (T or a flight key to take over)", nav.status);
        } else if dist > 1_000.0 {
            s += "\nT: autopilot   N: next target";
        }
    } else if *mode == ControlMode::Walk {
        s += "   (N: next target)";
    }
    if t.0 != s {
        t.0 = s;
    }
}
