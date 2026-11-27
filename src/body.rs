//! The player's visible body: the astronaut model from tools/blender/astronaut.py, placed at the
//! player's feet every frame (in world or ship-interior space) and animated from their speed.
//! In first-person view the helmet is hidden so it doesn't block the camera.

use std::time::Duration;

use avian3d::prelude::*;
use bevy::gltf::Gltf;
use bevy::prelude::*;

use crate::camera::CameraWorld;
use crate::player::{ControlMode, HALF_HEIGHT, InShip, Player};

const MODEL: &str = "models/astronaut.glb";
/// Animation clips in the model, in the order of `BodyAnimations::nodes`.
const CLIPS: [&str; 3] = ["Idle", "Walk", "Run"];
/// Ground speed (m/s) at which the Walk and Run clips play at normal speed.
const WALK_CLIP_SPEED: f32 = 4.5;
const RUN_CLIP_SPEED: f32 = 9.0;

pub struct BodyPlugin;

impl Plugin for BodyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_body)
            .add_systems(Update, (build_animation_graph, attach_animation_graph, animate_body).chain());
    }
}

#[derive(Component)]
pub struct Body;

#[derive(Resource)]
struct BodyAnimations {
    gltf: Handle<Gltf>,
    graph: Option<Handle<AnimationGraph>>,
    nodes: Vec<AnimationNodeIndex>,
}

/// Which clip is playing (index into `CLIPS`).
#[derive(Component)]
struct CurrentClip(usize);

fn spawn_body(mut commands: Commands, assets: Res<AssetServer>) {
    let model_file = crate::game_dir().join("assets").join(MODEL);
    if !model_file.exists() {
        warn!("{} not found; run tools/blender/astronaut.py to build the player body", model_file.display());
        return;
    }
    commands.insert_resource(BodyAnimations { gltf: assets.load(MODEL), graph: None, nodes: Vec::new() });
    commands.spawn((
        Body,
        SceneRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(MODEL))),
        Transform::default(),
        Visibility::Hidden,
    ));
}

fn build_animation_graph(
    anims: Option<ResMut<BodyAnimations>>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let Some(mut anims) = anims else { return };
    if anims.graph.is_some() {
        return;
    }
    let Some(gltf) = gltfs.get(&anims.gltf) else { return };
    let clips: Option<Vec<_>> = CLIPS.iter().map(|n| gltf.named_animations.get(*n).cloned()).collect();
    let Some(clips) = clips else {
        warn!("astronaut model is missing one of the animations {CLIPS:?}");
        anims.graph = Some(Handle::default());
        return;
    };
    let (graph, nodes) = AnimationGraph::from_clips(clips);
    anims.graph = Some(graphs.add(graph));
    anims.nodes = nodes;
}

/// Once the model's AnimationPlayer exists and the graph is built, hook them up and start idling.
fn attach_animation_graph(
    mut commands: Commands,
    anims: Option<Res<BodyAnimations>>,
    mut players: Query<(Entity, &mut AnimationPlayer), Without<AnimationGraphHandle>>,
) {
    let Some(anims) = anims else { return };
    let (Some(graph), false) = (&anims.graph, anims.nodes.is_empty()) else { return };
    for (entity, mut player) in &mut players {
        let mut transitions = AnimationTransitions::new();
        transitions.play(&mut player, anims.nodes[0], Duration::ZERO).repeat();
        commands.entity(entity).insert((AnimationGraphHandle(graph.clone()), transitions, CurrentClip(0)));
    }
}

/// Pick Idle / Walk / Run from the player's speed over the ground and set the playback rate so
/// the feet keep pace.
fn animate_body(
    anims: Option<Res<BodyAnimations>>,
    player: Query<(&Player, &LinearVelocity, &Position, Option<&InShip>)>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions, &mut CurrentClip)>,
) {
    let Some(anims) = anims else { return };
    let Ok((p, vel, pos, in_ship)) = player.single() else { return };
    let up = if in_ship.is_some() { bevy::math::DVec3::Y } else { crate::world::up_at(pos.0) };
    let ground_speed = (vel.0 - up * vel.0.dot(up)).length() as f32;
    let want = if !p.grounded || ground_speed < 0.4 {
        0
    } else if ground_speed < 6.5 {
        1
    } else {
        2
    };
    for (mut player, mut transitions, mut current) in &mut players {
        if current.0 != want {
            transitions.play(&mut player, anims.nodes[want], Duration::from_millis(250)).repeat();
            current.0 = want;
        }
        let rate = match want {
            1 => (ground_speed / WALK_CLIP_SPEED).clamp(0.4, 1.6),
            2 => (ground_speed / RUN_CLIP_SPEED).clamp(0.6, 1.4),
            _ => 1.0,
        };
        if let Some(active) = player.animation_mut(anims.nodes[want]) {
            active.set_speed(rate);
        }
    }
}

/// After the camera is placed: put the body at the player's feet, relative to the camera.
/// Hidden while piloting; helmet hidden in first person.
#[allow(clippy::type_complexity)]
pub fn place_body(
    mode: Res<ControlMode>,
    cam: Res<CameraWorld>,
    player: Query<(&Player, &Position, &Rotation, Option<&InShip>)>,
    ships: Query<(&Position, &Rotation), Without<Player>>,
    mut body: Query<(&mut Transform, &mut Visibility), With<Body>>,
    mut parts: Query<(&Name, &mut Visibility), Without<Body>>,
) {
    let Ok((mut t, mut vis)) = body.single_mut() else { return };
    let Ok((p, pos, rot, in_ship)) = player.single() else { return };
    if *mode == ControlMode::Pilot {
        *vis = Visibility::Hidden;
        return;
    }
    let (feet, rotation) = match in_ship.map(|s| (s, ships.get(s.ship))) {
        Some((s, Ok((spos, srot)))) => {
            let local_feet = pos.0 - s.proxy - bevy::math::DVec3::Y * HALF_HEIGHT;
            (spos.0 + srot.0 * local_feet, srot.0 * rot.0)
        }
        _ => (pos.0 - crate::world::up_at(pos.0) * HALF_HEIGHT, rot.0),
    };
    t.translation = (feet - cam.pos).as_vec3();
    t.rotation = rotation.as_quat();
    *vis = Visibility::Inherited;

    let first_person = *mode == ControlMode::Walk && !p.third_person;
    for (name, mut v) in &mut parts {
        let n = name.as_str();
        if n.starts_with("Helmet") || n.starts_with("Visor") {
            *v = if first_person { Visibility::Hidden } else { Visibility::Inherited };
        }
    }
}
