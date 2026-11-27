//! Sun, starfield, a cheap atmosphere approximation (sky colour + haze) and the HUD.

use bevy::asset::RenderAssetUsages;
use bevy::pbr::{CascadeShadowConfigBuilder, NotShadowCaster};
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};

use crate::camera::CameraWorld;
use crate::planet::Planet;
use crate::player::{ControlMode, Player};
use crate::ship::{CanBoard, Piloting, Ship};

/// Direction from the planet towards the sun.
pub fn sun_dir() -> Vec3 {
    Vec3::new(1.0, 0.4, 0.3).normalize()
}

/// Sky objects are drawn this far from the camera (effectively at infinity).
const SKY_DIST: f32 = 1.0e10;

#[derive(Component)]
pub struct Stars;

#[derive(Component)]
pub struct Hud;

pub fn setup_sky(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let sun = sun_dir();
    commands.spawn((
        DirectionalLight { illuminance: 4_000.0, shadows_enabled: !crate::debug::hidden("shadows"), ..default() },
        Transform::IDENTITY.looking_to(-sun, Vec3::Y),
        // Shadows near the viewer only (forest canopy, ship, body); beyond that the baked
        // occlusion in the vertex colours carries the shading.
        CascadeShadowConfigBuilder {
            num_cascades: 2,
            minimum_distance: 0.1,
            first_cascade_far_bound: 40.0,
            maximum_distance: std::env::var("SPACE_SHADOW_DIST").ok().and_then(|v| v.parse().ok()).unwrap_or(250.0),
            overlap_proportion: 0.2,
        }
        .build(),
    ));
    // 1024 texels per cascade: the near cascade spans 40 m (4 cm texels), plenty for leaf
    // shadows, and it fills four times fewer texels than Bevy's default 2048.
    let size = std::env::var("SPACE_SHADOW_RES").ok().and_then(|v| v.parse().ok()).unwrap_or(1024);
    commands.insert_resource(bevy::pbr::DirectionalLightShadowMap { size });
    commands.insert_resource(AmbientLight { color: Color::WHITE, brightness: 120.0, ..default() });
    commands.insert_resource(ClearColor(Color::BLACK));

    // Sun disc (angular radius ~0.27 degrees, like ours).
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(SKY_DIST * 0.0047).mesh().uv(32, 16))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.96, 0.88),
            unlit: true,
            fog_enabled: false,
            ..default()
        })),
        Transform::from_translation(sun * SKY_DIST),
        NotShadowCaster,
    ));

    // Starfield: small camera-facing quads on a huge sphere centred on the camera.
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let mut seed: u32 = 0x1234_5678;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    let dist = SKY_DIST * 2.0;
    for _ in 0..6000 {
        let z = rnd() * 2.0 - 1.0;
        let a = rnd() * std::f32::consts::TAU;
        let r = (1.0 - z * z).sqrt();
        let d = Vec3::new(r * a.cos(), z, r * a.sin());
        let mag = rnd().powf(6.0);
        let size = dist * (0.0006 + 0.0016 * mag);
        let tint = rnd();
        let c = Vec3::new(0.8 + 0.2 * tint, 0.85, 1.0 - 0.2 * tint) * (0.25 + 2.5 * mag);
        let right = d.any_orthonormal_vector();
        let up = d.cross(right);
        let base = positions.len() as u32;
        for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.push((d * dist + (right * sx + up * sy) * size).to_array());
            colors.push([c.x, c.y, c.z, 1.0]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_indices(Indices::U32(indices));
    commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            fog_enabled: false,
            cull_mode: None,
            ..default()
        })),
        Transform::IDENTITY,
        NotShadowCaster,
        Stars,
    ));

    commands.spawn((
        Text::new(""),
        TextFont { font_size: 15.0, ..default() },
        TextColor(Color::srgb(0.85, 0.9, 1.0)),
        Node { position_type: PositionType::Absolute, top: Val::Px(10.0), left: Val::Px(12.0), ..default() },
        Hud,
    ));

    // Small dot in the centre of the screen.
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|p| {
            p.spawn((
                Node { width: Val::Px(4.0), height: Val::Px(4.0), ..default() },
                BorderRadius::MAX,
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.75)),
            ));
        });
}

fn fmt_dist(m: f64) -> String {
    if m.abs() >= 10_000.0 { format!("{:.1} km", m / 1000.0) } else { format!("{m:.1} m") }
}

pub fn update_hud(
    cam: Res<CameraWorld>,
    planet: Res<Planet>,
    time: Res<Time>,
    mode: Res<ControlMode>,
    player: Query<&Player>,
    can_board: Res<CanBoard>,
    piloting: Res<Piloting>,
    ships: Query<(&Ship, &avian3d::prelude::Position, &avian3d::prelude::LinearVelocity)>,
    mut q: Query<&mut Text, With<Hud>>,
    mut fps: Local<f32>,
) {
    let Ok(mut text) = q.single_mut() else { return };
    let inst = 1.0 / time.delta_secs().max(1e-4);
    *fps = if *fps == 0.0 { inst } else { *fps * 0.95 + inst * 0.05 };
    let s = planet.stats;
    let (mode_line, controls) = match *mode {
        ControlMode::Walk => {
            let (grounded, swimming) = player.single().map(|p| (p.grounded, p.swimming)).unwrap_or((false, false));
            (
                if swimming {
                    "Swimming (Space up, C down, look down to dive)".to_string()
                } else {
                    format!("On foot ({})", if grounded { "grounded" } else { "airborne" })
                },
                if swimming {
                    "WASD swim (towards where you look)   Shift faster   Space up   C down\nV third person   Tab: free camera"
                } else if can_board.0 {
                    "WASD walk   Shift run   Space jump   V third person   Tab: free camera\nE use   R eat   I inventory   B build   N nav target   F5 save   F9 load\n\n>> F: take the pilot seat <<"
                } else {
                    "WASD walk   Shift run   Space jump   V third person   Tab: free camera\nE use   R eat   I inventory   B build   N nav target   F5 save   F9 load"
                },
            )
        }
        ControlMode::Pilot => {
            let mut line = "Piloting".to_string();
            if let Some(Ok((s, pos, vel))) = piloting.0.map(|e| ships.get(e)) {
                let alt = crate::world::altitude_raw(pos.0);
                let flight = if alt > crate::ship::ASSIST_CEILING {
                    "space flight (Newtonian)"
                } else if s.assist {
                    "flight assist ON"
                } else {
                    "flight assist OFF"
                };
                let vertical = vel.0.dot(pos.0.normalize());
                let orbit = match crate::physics::orbit_extremes(pos.0, vel.0) {
                    Some((ap, pe)) => {
                        let status = if pe > crate::atmosphere::ATMOSPHERE_HEIGHT { "  ORBIT" } else { "" };
                        format!("Apoapsis {}   Periapsis {}{status}", fmt_dist(ap), fmt_dist(pe))
                    }
                    None => "Escape trajectory".to_string(),
                };
                line = format!("Piloting ({flight})
Vertical  {}/s
{orbit}", fmt_dist(vertical));
            }
            (
                line,
                "Mouse pitch/yaw   Q/E roll   WASD thrust   Space/C up/down\nShift boost   B brake   X flight assist   V chase/cockpit view\nN nav target   T autopilot to target   F: leave the pilot seat",
            )
        }
        ControlMode::Fly => (
            "Free camera".to_string(),
            "WASD move   Space/C up/down   Q/E roll\nShift: boost   Scroll: speed\nTab: walk (drops you on the ground below)",
        ),
    };
    text.0 = format!(
        "{mode_line}\nAltitude  {}\nSpeed     {}/s  (x{:.2})\nChunks    {} shown, {} cached, {} building, LOD {}\nPlants    {:.2} M triangles\nFPS       {:.0}\n\n\
         Click: capture mouse   Esc: release\n{controls}",
        fmt_dist(cam.altitude),
        fmt_dist(cam.speed),
        cam.speed_mult,
        s.shown,
        s.cached,
        s.pending,
        s.max_level,
        s.flora_tris as f64 / 1e6,
        *fps,
    );
}
