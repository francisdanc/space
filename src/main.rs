mod atmosphere;
mod body;
mod camera;
mod debug;
mod economy;
mod erosion;
mod factory;
mod flora;
mod forage;
mod graphics;
mod hydro;
mod impostor;
mod interact;
mod items;
mod life;
mod mining;
mod moon;
mod nav;
mod noise;
mod physics;
mod planet;
mod plants;
mod player;
mod save;
mod ship;
mod sky;
mod surface_map;
mod terrain;
mod textures;
mod world;

use avian3d::prelude::PhysicsSet;
use bevy::prelude::*;
use bevy::transform::TransformSystem;

use camera::CameraWorld;

fn main() {
    // `space --dump-ship-parts file.json` writes the ship's parts for the Blender script.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--dump-ship-parts") {
        let path = args.get(i + 1).map(String::as_str).unwrap_or("ship_parts.json");
        ship::dump_parts(path).expect("failed to write ship parts");
        println!("wrote {path}");
        return;
    }
    // The planet: terrain (with the erosion simulation, cached after the first run) and life.
    let mut terrain = terrain::Terrain::with_erosion(planet::SEED);
    // Haven Outpost: the planet's base, on a levelled pad.
    let haven = economy::found_haven(&mut terrain);
    let terrain = std::sync::Arc::new(terrain);
    let bio = match std::env::var("SPACE_BIOSPHERE").ok().as_deref() {
        Some("barren") => life::Biosphere::barren(),
        Some(s) if s.starts_with("alien") => {
            life::Biosphere::alien(s.trim_start_matches("alien").trim_start_matches(':').parse().unwrap_or(42))
        }
        _ => life::Biosphere::earthlike(planet::SEED),
    };
    println!("{}", bio.summary());
    // A saved game starts where it was left (the rest is applied once the world exists).
    let saved = save::read();
    let (start, mode) = match &saved {
        Some(s) => (bevy::math::DVec3::from(s.player_pos) + crate::world::up_at(bevy::math::DVec3::from(s.player_pos)) * 2.0, player::ControlMode::Walk),
        None => debug::start_position(&terrain, &bio),
    };
    let clearing = debug::start_clearing(&terrain, start);
    let base_clearing = flora::Clearing { dir: haven, radius: 95.0 };
    // The start glade is only for test starts: the world must be the same in every session.
    let clearings = if saved.is_none() && std::env::var("SPACE_START").is_ok() { vec![clearing, base_clearing] } else { vec![base_clearing] };
    let flora = flora::FloraContext::new(terrain, bio, planet::SEED, clearings);
    let ship_spawn = debug::ship_spawn(&flora.terrain);
    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            // Absolute path so the game finds its models however it is launched.
            file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets").to_string(),
            ..default()
        }).set(WindowPlugin {
            primary_window: Some(Window {
                title: "Space".into(),
                resolution: (1600.0_f32, 900.0_f32).into(),
                present_mode: if std::env::var("SPACE_NOVSYNC").is_ok() {
                    bevy::window::PresentMode::AutoNoVsync
                } else {
                    bevy::window::PresentMode::AutoVsync
                },
                ..default()
            }),
            ..default()
        }))
        .insert_resource(CameraWorld { pos: start, speed_mult: 1.0, altitude: 0.0, speed: 0.0 })
        .insert_resource(mode)
        .insert_resource(ship::ShipSpawn(ship_spawn))
        .insert_resource(save::PendingLoad(saved))
        .insert_resource(planet::PlanetInit(std::sync::Arc::new(flora)))
        // Merged plant meshes are large (several MB per near chunk); spread their GPU uploads over
        // frames instead of stalling one frame when many chunks finish together.
        .insert_resource(bevy::render::render_asset::RenderAssetBytesPerFrame::new(
            std::env::var("SPACE_UPLOAD_MB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(8) << 20,
        ))
        .init_resource::<ship::Piloting>()
        .init_resource::<ship::CanBoard>()
        .add_plugins(physics::SpacePhysicsPlugin)
        .add_plugins(atmosphere::AtmospherePlugin)
        .add_plugins(graphics::GraphicsPlugin)
        .add_plugins(body::BodyPlugin)
        .add_plugins(interact::InteractPlugin)
        .add_plugins(forage::ForagePlugin)
        .add_plugins(mining::MiningPlugin)
        .add_plugins(factory::FactoryPlugin)
        .add_plugins(economy::EconomyPlugin)
        .add_plugins(save::SavePlugin)
        .add_plugins(nav::NavPlugin)
        .add_plugins(moon::MoonPlugin)
        .add_plugins(planet::PlanetPlugin)
        .add_systems(Startup, (camera::spawn_camera, sky::setup_sky, player::spawn_player))
        .add_systems(PostStartup, ship::spawn_ship)
        .add_systems(
            Update,
            (
                camera::grab_cursor,
                player::toggle_mode,
                ship::board_or_exit,
                ship::interior_transfer,
                camera::fly,
                player::ground_check,
                player::player_control,
                ship::ship_control,
                ship::ship_ground_safety,
                planet::update_lod,
                sky::update_hud,
            )
                .chain(),
        )
        // After physics has moved things: place the camera, then everything relative to it.
        .add_systems(
            PostUpdate,
            (player::sync_player_camera, ship::sync_ship_camera, body::place_body, planet::apply_floating_origin, physics::apply_body_origin, atmosphere::update_atmosphere, graphics::update_terrain_material)
                .chain()
                .after(PhysicsSet::Sync)
                .before(TransformSystem::TransformPropagate),
        )
        .add_plugins(debug::DebugPlugin)
        .run();
}
