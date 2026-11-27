//! Saving and loading.
//!
//! The procedural world is a pure function of the seed, so a save only holds what the player
//! changed: where they are, what they carry, hunger, credits, the markets, picked plants (with
//! their regrow timers), how much was mined from each deposit, the factories, and the ship.
//!
//! - F5 saves, F9 loads, and the game autosaves every minute.
//! - On start the save is loaded (the world streams in around the saved position).
//! - Test runs (SPACE_START set) never load or write the save; SPACE_NEW_GAME=1 ignores it.

use std::path::PathBuf;

use avian3d::prelude::*;
use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::economy::Economy;
use crate::factory::Factory;
use crate::forage::{Regrowth, Survival, Toasts};
use crate::items::{Inventory, ItemId};
use crate::mining::Mining;
use crate::planet::Planet;
use crate::player::{ControlMode, InShip, Player};
use crate::ship::{Piloting, Ship};

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct SaveGame {
    pub version: u32,
    pub player_pos: [f64; 3],
    pub heading: [f64; 3],
    pub pitch: f32,
    pub food: f32,
    pub inventory: Vec<Option<(ItemId, u32)>>,
    pub economy: Economy,
    /// (plant cell, where, seconds until it regrows)
    pub harvested: Vec<(u64, [f64; 3], f64)>,
    pub mined: Vec<(u64, u32)>,
    pub factory: Factory,
    pub ship: Option<ShipSave>,
}

#[derive(Serialize, Deserialize)]
pub struct ShipSave {
    pub pos: [f64; 3],
    pub rot: [f64; 4],
    pub vel: [f64; 3],
    pub engines_on: bool,
}

pub fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("saves").join("save.json")
}

/// Saving and loading are off in test runs.
pub fn enabled() -> bool {
    std::env::var("SPACE_START").is_err() && std::env::var("SPACE_NEW_GAME").is_err()
}

/// Read the save from disk (at startup, before the world is built).
pub fn read() -> Option<SaveGame> {
    if !enabled() {
        return None;
    }
    let text = std::fs::read_to_string(path()).ok()?;
    match serde_json::from_str::<SaveGame>(&text) {
        Ok(s) if s.version == VERSION => Some(s),
        Ok(_) => None,
        Err(e) => {
            eprintln!("could not read the save ({e}); starting a new game");
            None
        }
    }
}

/// A save waiting to be applied once the player, ship and world exist.
#[derive(Resource)]
pub struct PendingLoad(pub Option<SaveGame>);

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, apply_pending.after(crate::ship::spawn_ship)).add_systems(Last, (hotkeys, autosave));
    }
}

/// Everything to save, from plain data.
#[allow(clippy::too_many_arguments)]
fn build_save(
    now: f64,
    p: &Player,
    pos: DVec3,
    inv: &Inventory,
    aboard: bool,
    survival: &Survival,
    economy: &Economy,
    regrowth: &Regrowth,
    mining: &Mining,
    factory: &Factory,
    ship: Option<(&Ship, DVec3, DQuat, DVec3)>,
) -> SaveGame {
    // Aboard a ship the body is in the cabin's physics proxy: save a spot by the ramp instead.
    let player_pos = match (aboard, ship) {
        (true, Some((_, spos, srot, _))) => spos + srot * DVec3::new(0.0, -1.5, 14.0),
        _ => pos,
    };
    SaveGame {
        version: VERSION,
        player_pos: player_pos.to_array(),
        heading: p.heading.to_array(),
        pitch: p.pitch,
        food: survival.food,
        inventory: inv.slots.clone(),
        economy: serde_json::from_value(serde_json::to_value(economy).unwrap()).unwrap(),
        harvested: regrowth.0.iter().map(|(c, at, t)| (*c, at.to_array(), (t - now).max(0.0))).collect(),
        mined: mining.mined.iter().map(|(c, n)| (*c, *n)).collect(),
        factory: serde_json::from_value(serde_json::to_value(factory).unwrap()).unwrap(),
        ship: ship.map(|(s, p, r, v)| ShipSave { pos: p.to_array(), rot: [r.x, r.y, r.z, r.w], vel: v.to_array(), engines_on: s.engines_on }),
    }
}

fn write(save: &SaveGame) -> std::io::Result<()> {
    let p = path();
    std::fs::create_dir_all(p.parent().unwrap())?;
    // Write to a temporary file first so a crash mid-write never corrupts the save.
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(save).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, p)
}

#[allow(clippy::too_many_arguments)]
fn apply(
    save: SaveGame,
    commands: &mut Commands,
    time: &Time,
    planet: &mut Planet,
    player: &mut (Mut<Player>, Mut<Position>, Mut<Inventory>, Option<&InShip>),
    survival: &mut Survival,
    economy: &mut Economy,
    regrowth: &mut Regrowth,
    mining: &mut Mining,
    factory: &mut Factory,
    ship: Option<(Mut<Ship>, Mut<Position>, Mut<Rotation>, Mut<LinearVelocity>)>,
) {
    let (p, pos, inv, _) = player;
    pos.0 = crate::player::standing_position(planet, DVec3::from(save.player_pos));
    p.heading = DVec3::from(save.heading);
    p.pitch = save.pitch;
    survival.food = save.food;
    inv.slots = save.inventory;
    inv.slots.resize(24, None);
    *economy = save.economy;
    // Picked plants: re-mesh both the previously and the newly picked ones.
    let now = time.elapsed_secs_f64();
    for (_, at, _) in regrowth.0.drain(..) {
        planet.rebuild_at(at);
    }
    {
        let mut set = planet.flora.harvested.write().unwrap();
        set.clear();
        for (cell, _, _) in &save.harvested {
            set.insert(*cell);
        }
    }
    for (cell, at, left) in save.harvested {
        let at = DVec3::from(at);
        regrowth.0.push((cell, at, now + left));
        planet.rebuild_at(at);
    }
    mining.mined = save.mined.into_iter().collect();
    mining.respawn(commands);
    *factory = save.factory;
    factory.reindex(commands);
    if let (Some(s), Some((mut ship, mut spos, mut srot, mut svel))) = (save.ship, ship) {
        spos.0 = DVec3::from(s.pos);
        srot.0 = DQuat::from_array(s.rot);
        svel.0 = DVec3::from(s.vel);
        ship.engines_on = s.engines_on;
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn apply_pending(
    mut commands: Commands,
    time: Res<Time>,
    pending: Option<ResMut<PendingLoad>>,
    mut planet: ResMut<Planet>,
    mut player: Query<(&mut Player, &mut Position, &mut Inventory, Option<&InShip>), Without<Ship>>,
    mut survival: ResMut<Survival>,
    mut economy: ResMut<Economy>,
    mut regrowth: ResMut<Regrowth>,
    mut mining: ResMut<Mining>,
    mut factory: ResMut<Factory>,
    mut ships: Query<(&mut Ship, &mut Position, &mut Rotation, &mut LinearVelocity), Without<Player>>,
) {
    let Some(save) = pending.and_then(|mut p| p.0.take()) else { return };
    let Ok(mut pl) = player.single_mut() else { return };
    apply(save, &mut commands, &time, &mut planet, &mut pl, &mut survival, &mut economy, &mut regrowth, &mut mining, &mut factory, ships.iter_mut().next());
    info!("save loaded");
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    time: Res<Time>,
    mut mode: ResMut<ControlMode>,
    mut piloting: ResMut<Piloting>,
    mut planet: ResMut<Planet>,
    mut player: Query<(&mut Player, &mut Position, &mut Inventory, Option<&InShip>), Without<Ship>>,
    mut survival: ResMut<Survival>,
    mut economy: ResMut<Economy>,
    mut regrowth: ResMut<Regrowth>,
    mut mining: ResMut<Mining>,
    mut factory: ResMut<Factory>,
    mut ships: Query<(&mut Ship, &mut Position, &mut Rotation, &mut LinearVelocity), Without<Player>>,
    mut toasts: ResMut<Toasts>,
    entities: Query<Entity, With<Player>>,
) {
    if !enabled() {
        return;
    }
    let Ok(mut pl) = player.single_mut() else { return };
    if keys.just_pressed(KeyCode::F5) {
        let aboard = pl.3.is_some() || *mode == ControlMode::Pilot;
        let ship = ships.iter().next().map(|(s, p, r, v)| (s, p.0, r.0, v.0));
        let save = build_save(time.elapsed_secs_f64(), &pl.0, pl.1.0, &pl.2, aboard, &survival, &economy, &regrowth, &mining, &factory, ship);
        match write(&save) {
            Ok(()) => toasts.push("Game saved", Color::srgb(0.6, 0.85, 1.0)),
            Err(e) => toasts.push(format!("Save failed: {e}"), Color::srgb(1.0, 0.4, 0.3)),
        }
    }
    if keys.just_pressed(KeyCode::F9) {
        let Some(save) = read() else {
            toasts.push("No save to load", Color::srgb(1.0, 0.45, 0.3));
            return;
        };
        // Step out of the ship and onto your feet first.
        if let Ok(e) = entities.single() {
            commands.entity(e).remove::<InShip>().remove::<(RigidBodyDisabled, ColliderDisabled)>();
        }
        piloting.0 = None;
        *mode = ControlMode::Walk;
        apply(save, &mut commands, &time, &mut planet, &mut pl, &mut survival, &mut economy, &mut regrowth, &mut mining, &mut factory, ships.iter_mut().next());
        toasts.push("Game loaded", Color::srgb(0.6, 0.85, 1.0));
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn autosave(
    time: Res<Time>,
    mode: Res<ControlMode>,
    player: Query<(&Player, &Position, &Inventory, Option<&InShip>), Without<Ship>>,
    survival: Res<Survival>,
    economy: Res<Economy>,
    regrowth: Res<Regrowth>,
    mining: Res<Mining>,
    factory: Res<Factory>,
    ships: Query<(&Ship, &Position, &Rotation, &LinearVelocity), Without<Player>>,
    mut next: Local<f64>,
) {
    if !enabled() {
        return;
    }
    let now = time.elapsed_secs_f64();
    if *next == 0.0 {
        *next = now + 60.0;
    }
    if now < *next {
        return;
    }
    *next = now + 60.0;
    let Ok((p, pos, inv, in_ship)) = player.single() else { return };
    let ship = ships.iter().next().map(|(s, p, r, v)| (s, p.0, r.0, v.0));
    let aboard = in_ship.is_some() || *mode == ControlMode::Pilot;
    let save = build_save(now, p, pos.0, inv, aboard, &survival, &economy, &regrowth, &mining, &factory, ship);
    if let Err(e) = write(&save) {
        warn!("autosave failed: {e}");
    } else {
        info!("autosaved");
    }
}
