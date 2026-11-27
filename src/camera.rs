//! Free-flight camera with f64 world position, altitude-scaled speed and ground collision.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::camera::PerspectiveProjection;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::planet::Planet;
use crate::player::ControlMode;

#[derive(Resource)]
pub struct CameraWorld {
    /// True camera position in metres, planet centre at the origin.
    pub pos: DVec3,
    pub speed_mult: f64,
    pub altitude: f64,
    pub speed: f64,
}

#[derive(Component)]
pub struct FlyCam;

/// Minimum height of the camera above the ground, in metres.
const MIN_CLEARANCE: f64 = 1.8;

pub fn spawn_camera(mut commands: Commands, cam: Res<CameraWorld>) {
    let down = -crate::world::up_at(cam.pos).as_vec3();
    // Look at the planet when far away, otherwise look along the horizon.
    let far = crate::world::altitude_raw(cam.pos) > crate::world::nearest(cam.pos).radius() * 0.5;
    let (fwd, up) = if far {
        (down, Vec3::Y)
    } else {
        let up = -down;
        let tangent = up.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        ((tangent - up * 0.08).normalize(), up)
    };
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 70f32.to_radians(),
            near: 0.1,
            far: 1.0e12,
            ..default()
        }),
        Transform::IDENTITY.looking_to(fwd, up),
        // The atmosphere shader reads scene depth to stop its rays at terrain and ships.
        bevy::core_pipeline::prepass::DepthPrepass,
        crate::graphics::camera_effects(),
        FlyCam,
    ))
    .insert_if(crate::graphics::ssao().unwrap_or_default(), || !crate::debug::hidden("ssao"));
}

pub fn grab_cursor(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let Ok(mut w) = windows.single_mut() else { return };
    if mouse.just_pressed(MouseButton::Left) {
        w.cursor_options.grab_mode = CursorGrabMode::Locked;
        w.cursor_options.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        w.cursor_options.grab_mode = CursorGrabMode::None;
        w.cursor_options.visible = true;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn fly(
    time: Res<Time>,
    mode: Res<ControlMode>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    windows: Query<&Window, With<PrimaryWindow>>,
    planet: Res<Planet>,
    mut cam: ResMut<CameraWorld>,
    mut q: Query<&mut Transform, With<FlyCam>>,
) {
    if *mode != ControlMode::Fly {
        return;
    }
    let Ok(mut t) = q.single_mut() else { return };
    let dt = time.delta_secs();
    let locked = windows
        .single()
        .map(|w| w.cursor_options.grab_mode != CursorGrabMode::None)
        .unwrap_or(false);

    if locked {
        let d = motion.delta;
        t.rotate_local_y(-d.x * 0.002);
        t.rotate_local_x(-d.y * 0.002);
    }
    let mut roll = 0.0;
    if keys.pressed(KeyCode::KeyQ) {
        roll += 1.0;
    }
    if keys.pressed(KeyCode::KeyE) {
        roll -= 1.0;
    }
    t.rotate_local_z(roll * 1.5 * dt);

    if scroll.delta.y != 0.0 {
        cam.speed_mult = (cam.speed_mult * 1.25f64.powf(scroll.delta.y as f64)).clamp(0.01, 100.0);
    }

    let terrain = &planet.terrain;
    let alt = crate::world::altitude(terrain, cam.pos);

    let mut dir = Vec3::ZERO;
    for (k, v) in [
        (KeyCode::KeyW, Vec3::NEG_Z),
        (KeyCode::KeyS, Vec3::Z),
        (KeyCode::KeyA, Vec3::NEG_X),
        (KeyCode::KeyD, Vec3::X),
        (KeyCode::Space, Vec3::Y),
        (KeyCode::KeyC, Vec3::NEG_Y),
    ] {
        if keys.pressed(k) {
            dir += v;
        }
    }
    let mut speed = alt.max(2.0) * 0.8 * cam.speed_mult;
    if keys.pressed(KeyCode::ShiftLeft) {
        speed *= 8.0;
    }
    if dir != Vec3::ZERO {
        let world_dir = (t.rotation * dir.normalize()).as_dvec3();
        cam.pos += world_dir * speed * dt as f64;
    }
    cam.speed = if dir != Vec3::ZERO { speed } else { 0.0 };

    // Ground collision (the free camera may dive under water, not into the ground).
    let up = crate::world::up_at(cam.pos);
    let ground = crate::world::ground_point(terrain, cam.pos);
    if (cam.pos - ground).dot(up) < MIN_CLEARANCE {
        cam.pos = ground + up * MIN_CLEARANCE;
    }
    cam.altitude = crate::world::altitude(terrain, cam.pos);

    // Near the planet, gently roll the camera so the horizon stays level.
    if cam.altitude < 300_000.0 && roll == 0.0 {
        let up = up.as_vec3();
        let fwd = *t.forward();
        let proj = up - fwd * up.dot(fwd);
        if proj.length() > 0.1 {
            let proj = proj.normalize();
            let cur_up = *t.up();
            let angle = cur_up.cross(proj).dot(fwd).atan2(cur_up.dot(proj));
            let k = 1.0 - (-3.0 * dt).exp();
            t.rotate(Quat::from_axis_angle(fwd, angle * k));
        }
    }
}
