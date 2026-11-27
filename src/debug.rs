//! Test hooks driven by environment variables, so the game can be checked without a human:
//!   SPACE_START       = orbit (default) | high | low | ground | mountain | coast | sea | fly-coast | river | lake
//!                     (river/lake: free camera beside a sunlit river 60-250 m wide, or a lakeshore;
//!                     SPACE_FLY_ALT sets the distance)
//!   SPACE_SCREENSHOT  = path.png   take a screenshot once the terrain has finished loading, then exit
//!   SPACE_FLY_ALT     = metres      altitude for the fly-<...> starts (default 120)
//!   SPACE_NO_IMP      (set)         don't draw tree impostors (to compare)
//!   SPACE_START       = (unset) haven | selene | moon | moon-low | moon-orbit   (game starts)
//!   SPACE_MINE / SPACE_FACTORY / SPACE_LOGGING=1|stump / SPACE_TRADE=<base> / SPACE_SAVETEST=1|2 /
//!   SPACE_AUTOPILOT=<target index>   scripted feature tests
//!   (Any SPACE_START value disables loading and writing the save.)
//!   SPACE_FORAGE      = name        stand by the nearest foragable plant (matching name), pick it
//!                                   at 8 s, eat at 9 s, open the inventory at 9.5 s
//!   SPACE_CLOSEUP     = metres      stand that far from the nearest tree, facing it
//!   SPACE_PROFILE     (set)         log GPU time per render pass, CPU time of heavy systems and
//!                                   frame-time percentiles when the screenshot is taken
//!   SPACE_HIDE        = a,b,...     hide parts to measure their cost: flora, big_solid, big_cards,
//!                                   big_fill, small_solid, small_cards, impostors, ssao, shadows
//!   SPACE_SHADOW_RES / SPACE_SHADOW_DIST / SPACE_UPLOAD_MB   override shadow and upload settings
//!   SPACE_IMPOSTOR_DUMP = path.png  save the first species' impostor atlas

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::planet::Planet;
use crate::life::Biosphere;
use crate::player::{AutoWalk, ControlMode, Player};
use crate::ship::ShipInput;
use crate::sky::sun_dir;
use crate::terrain::{RADIUS, Terrain};

/// Find a sunlit spot whose height lies in `range`.
fn find_spot(t: &Terrain, range: std::ops::Range<f64>) -> DVec3 {
    find_spot_lit(t, range, 0.55)
}

/// Find a spot whose height lies in `range`, with the sun at elevation ~asin(`light`).
fn find_spot_lit(t: &Terrain, range: std::ops::Range<f64>, target_light: f64) -> DVec3 {
    let sun = sun_dir().as_dvec3();
    let n = 20_000;
    let mut best = sun;
    let mut best_light = -1.0;
    for i in 0..n {
        let y = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
        let r = (1.0 - y * y).sqrt();
        let phi = i as f64 * 2.399_963;
        let d = DVec3::new(r * phi.cos(), y, r * phi.sin());
        let light = d.dot(sun);
        // Prefer mid-morning light: low enough sun to show relief.
        let score = 1.0 - (light - target_light).abs();
        if light > target_light * 0.5 && score > best_light && range.contains(&t.height(d)) {
            best = d;
            best_light = score;
        }
    }
    best
}

/// Where the game starts and in which control mode. Default: standing on the ground.
/// A sunlit spot where the given plant species grow most densely (any canopy species if
/// `names` is empty).
pub fn find_spot_species(t: &Terrain, bio: &Biosphere, names: &[&str]) -> Option<DVec3> {
    let sun = sun_dir().as_dvec3();
    let wanted: Vec<usize> = bio
        .flora
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            if names.is_empty() { s.layer == crate::flora::Layer::Canopy } else { names.iter().any(|n| s.name.contains(n)) }
        })
        .map(|(i, _)| i)
        .collect();
    if wanted.is_empty() {
        return None;
    }
    let n = 40_000;
    let mut candidates: Vec<(f64, DVec3)> = Vec::new();
    for i in 0..n {
        let y = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
        let r = (1.0 - y * y).sqrt();
        let phi = i as f64 * 2.399_963;
        let d = DVec3::new(r * phi.cos(), y, r * phi.sin());
        let light = d.dot(sun);
        if !(0.3..0.85).contains(&light) || t.water_level(d).is_some() {
            continue;
        }
        let e = t.ecology(d);
        if e.height < 30.0 {
            continue;
        }
        let dens: f32 = wanted.iter().map(|&k| bio.flora[k].density_at(&e)).sum();
        candidates.push((dens as f64 - 0.3 * (light - 0.55).abs(), d));
    }
    // Among the best spots, prefer flat ground (somewhere a ship can park).
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    candidates
        .iter()
        .take(150)
        .map(|&(score, d)| {
            let e1 = d.any_orthonormal_vector();
            let e2 = d.cross(e1);
            let h0 = t.height(d);
            let step = 30.0 / RADIUS;
            let relief: f64 = [e1, -e1, e2, -e2].iter().map(|o| (t.height((d + *o * step).normalize()) - h0).abs()).sum();
            // ~2 degree slopes cost 0.15, ~6 degrees 0.5: strongly prefer flat glades.
            (score - relief / 25.0, d)
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, d)| d)
}

/// Where the ship starts: on the landing pad when starting at a base.
pub fn ship_spawn(t: &Terrain) -> Option<DVec3> {
    match std::env::var("SPACE_START").unwrap_or_default().as_str() {
        "" | "haven" => Some(crate::economy::landing_spot(t, 0)),
        "selene" => Some(crate::economy::landing_spot(t, 1)),
        _ => None,
    }
}

/// A sunlit spot on a river (`lake` false: at least `min_width` m wide) or a lake.
fn find_water(t: &Terrain, lake: bool) -> Option<DVec3> {
    let sun = sun_dir().as_dvec3();
    let n = 400_000;
    (0..n)
        .filter_map(|i| {
            let y = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
            let r = (1.0 - y * y).sqrt();
            let phi = i as f64 * 2.399_963;
            let d = DVec3::new(r * phi.cos(), y, r * phi.sin());
            let light = d.dot(sun);
            if light < 0.45 {
                return None;
            }
            let w = t.water(d)?;
            let is_lake = w.flow == DVec3::ZERO;
            let width_ok = if lake {
                // Near a shore: dry ground within 1.2 km in some direction.
                let e1 = crate::player::tangent(DVec3::Y, d);
                let e2 = d.cross(e1);
                [e1, -e1, e2, -e2].iter().any(|&o| t.water_level((d + o * (1200.0 / RADIUS)).normalize()).is_none())
            } else {
                (60.0..250.0).contains(&w.width)
            };
            (is_lake == lake && width_ok && w.level > 30.0 && t.height(d) < w.level - 1.0).then_some((light, d))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, d)| d)
}

/// Where the free camera should look at start (set by the river/lake starts).
#[derive(Resource)]
pub struct AimAt(pub DVec3);

/// Where the game starts and in which control mode. Default: standing in a glade in the
/// planet's densest forest.
pub fn start_position(t: &Terrain, bio: &Biosphere) -> (DVec3, ControlMode) {
    let mode = std::env::var("SPACE_START").unwrap_or_default();
    if mode.is_empty() || mode == "haven" {
        // A new game: standing by Haven Outpost's landing pad.
        let p = crate::economy::landing_spot(t, 0);
        let up = crate::world::up_at(p);
        // Behind the pad, looking across it (and the ship) towards the base.
        return (p + up * 30.0 - crate::player::tangent(DVec3::Y, up) * 24.0, ControlMode::Walk);
    }
    if mode == "selene" {
        let p = crate::economy::landing_spot(t, 1);
        return (p + crate::world::up_at(p) * 30.0, ControlMode::Walk);
    }
    if let Some(rest) = mode.strip_prefix("moon") {
        // The moon: standing at the station site, or flying above it.
        let d = crate::world::moon_station_dir();
        let ground = crate::world::MOON_CENTER + d * (crate::world::MOON_RADIUS + crate::world::moon().height(d));
        return match rest {
            "-low" => (ground + d * 5_000.0, ControlMode::Fly),
            "-orbit" => (crate::world::MOON_CENTER + d * crate::world::MOON_RADIUS * 3.2, ControlMode::Fly),
            _ => (ground + d * 30.0, ControlMode::Walk),
        };
    }
    if mode == "river" || mode == "lake" {
        // Hover beside and above the water, looking at it (see aim_fly_cam).
        let d = find_water(t, mode == "lake").expect("no sunlit water found");
        let level = t.water(d).map_or(0.0, |w| w.level);
        let target = d * (RADIUS + level);
        let up = d;
        let side = crate::player::tangent(DVec3::Y, up);
        let dist = std::env::var("SPACE_FLY_ALT").ok().and_then(|v| v.parse().ok()).unwrap_or(250.0);
        let cam = target + side * dist + up * (dist * 0.5);
        let _ = AIM.set(target);
        let e = t.ecology(d);
        let bank = (d + side.normalize() * (60.0 / RADIUS)).normalize();
        let eb = t.ecology(bank);
        for sp in &bio.flora {
            let dens = sp.density_at(&eb);
            if dens > 0.01 {
                println!("  {}: density {:.2}", sp.name, dens);
            }
        }
        println!("  bank eco: height {:.0}, wetness {:.2}, coast {:.2}, terrain h {:.1}", eb.height, eb.wetness, eb.coast, t.height(bank));
        println!("water start: temp {:.1} C, moisture {:.2} (bank 60 m: {:.2}), riparian {:.2}", e.temperature, e.moisture, eb.moisture, t.hydro.as_ref().map_or(0.0, |h| h.riparian(bank)));
        return (cam, ControlMode::Fly);
    }
    if mode == "leo" {
        // Over a sunlit spot near the terminator; the ship is moved into orbit after spawning.
        let d = find_spot(t, 0.0..9000.0);
        return (d * (RADIUS + LEO_ALTITUDE), ControlMode::Pilot);
    }
    if mode == "sunset" {
        let d = find_spot_lit(t, 100.0..2000.0, 0.04);
        return (d * (RADIUS + t.surface_height(d) + 30.0), ControlMode::Walk);
    }
    if mode == "orbit" {
        return (DVec3::new(0.6, 0.3, 0.8).normalize() * RADIUS * 3.0, ControlMode::Fly);
    }
    // Forests: SPACE_START=forest (default) or species:<name>, optionally fly-<...> for the
    // free camera 120 m up.
    let (fly, what) = match mode.strip_prefix("fly-") {
        Some(rest) => (true, rest.to_string()),
        None => (false, mode.clone()),
    };
    if what.is_empty() || what == "forest" || what.starts_with("species:") {
        let names: Vec<&str> = what.strip_prefix("species:").map(|n| vec![n]).unwrap_or_default();
        let names = if what.is_empty() || what == "forest" { Vec::new() } else { names };
        if let Some(d) = find_spot_species(t, bio, &names) {
            let fly_alt = std::env::var("SPACE_FLY_ALT").ok().and_then(|v| v.parse().ok()).unwrap_or(120.0);
            let alt = if fly { fly_alt } else { 30.0 };
            let control = if fly { ControlMode::Fly } else { ControlMode::Walk };
            return (d * (RADIUS + t.surface_height(d) + alt), control);
        }
    }
    let (range, alt) = match what.as_str() {
        "high" => (200.0..1500.0, 400_000.0),
        "low" => (200.0..1500.0, 15_000.0),
        "mountain" => (3000.0..9000.0, 1_500.0),
        // A beach: the player faces the sea (see DebugPlugin), the ship is parked inland.
        "coast" => (0.3..2.0, 30.0),
        // Open water 8-40 m deep: the player spawns floating, the ship lands on the water.
        "sea" => (-40.0..-8.0, 30.0),
        _ => (100.0..800.0, 30.0),
    };
    let d = find_spot(t, range);
    let alt = if fly { std::env::var("SPACE_FLY_ALT").ok().and_then(|v| v.parse().ok()).unwrap_or(alt) } else { alt };
    let control = if fly || matches!(what.as_str(), "high" | "low") { ControlMode::Fly } else { ControlMode::Walk };
    (d * (RADIUS + t.surface_height(d) + alt), control)
}

/// The glade kept clear of plants around the starting point and the parked ship.
pub fn start_clearing(t: &Terrain, start: DVec3) -> crate::flora::Clearing {
    let up = start.normalize();
    let heading = crate::player::tangent(DVec3::Y, up);
    // The ship parks 26 m ahead of the player (see ship::spawn_ship).
    let _ = t;
    crate::flora::Clearing { dir: (up * RADIUS + heading * 13.0).normalize(), radius: 45.0 }
}

/// Altitude of the test orbit.
const LEO_ALTITUDE: f64 = 400_000.0;

/// SPACE_START=leo: put the ship in a circular orbit with the player at the controls.
#[allow(clippy::type_complexity)]
fn place_in_orbit(
    mut commands: Commands,
    mut piloting: ResMut<crate::ship::Piloting>,
    mut ships: Query<
        (Entity, &crate::ship::Ship, &mut avian3d::prelude::Position, &mut avian3d::prelude::Rotation, &mut avian3d::prelude::LinearVelocity),
        Without<Player>,
    >,
    mut player: Query<(Entity, &mut avian3d::prelude::Position), With<Player>>,
) {
    let (Ok((se, ship, mut pos, mut rot, mut vel)), Ok((pe, mut ppos))) = (ships.single_mut(), player.single_mut()) else {
        return;
    };
    let up = pos.0.normalize();
    let r = RADIUS + LEO_ALTITUDE;
    let mu = crate::player::GRAVITY * RADIUS * RADIUS;
    let east = crate::player::tangent(DVec3::Y.cross(up), up);
    pos.0 = up * r;
    vel.0 = east * (mu / r).sqrt();
    // Nose down a little so the planet's horizon is in view from the cockpit.
    rot.0 = crate::player::basis(east, up) * bevy::math::DQuat::from_rotation_x(-0.4);
    ppos.0 = ship.proxy;
    commands.entity(pe).insert((
        crate::player::InShip { ship: se, proxy: ship.proxy },
        avian3d::prelude::RigidBodyDisabled,
        avian3d::prelude::ColliderDisabled,
    ));
    piloting.0 = Some(se);
}

/// How bumpy walking is: fraction of frames in the air and the highest the feet got.
#[derive(Resource, Default)]
struct WalkStats {
    frames: u32,
    airborne: u32,
    max_feet: f64,
}

fn walk_stats(
    time: Res<Time>,
    planet: Res<Planet>,
    mut stats: ResMut<WalkStats>,
    player: Query<(&Player, &avian3d::prelude::Position), Without<crate::player::InShip>>,
) {
    // Skip the first seconds while terrain and collision load.
    if time.elapsed_secs() < 3.0 {
        return;
    }
    let Ok((p, pos)) = player.single() else { return };
    let up = pos.0.normalize();
    let feet = pos.0.length() - crate::player::HALF_HEIGHT - RADIUS - planet.terrain.surface_height(up);
    stats.frames += 1;
    if !p.grounded {
        stats.airborne += 1;
    }
    stats.max_feet = stats.max_feet.max(feet);
}

/// Free camera (fly-coast): look out to sea, a little downwards.
fn aim_fly_cam_at_sea(cam: Res<crate::camera::CameraWorld>, planet: Res<Planet>, mut q: Query<&mut Transform, With<crate::camera::FlyCam>>) {
    let up = cam.pos.normalize();
    let e1 = crate::player::tangent(DVec3::Y, up);
    let e2 = up.cross(e1);
    let best = (0..32)
        .map(|i| {
            let a = i as f64 / 32.0 * std::f64::consts::TAU;
            let dir = e1 * a.cos() + e2 * a.sin();
            let h: f64 = [300.0, 800.0, 2000.0].iter().map(|r| planet.terrain.height((cam.pos + dir * *r).normalize())).sum();
            (h, dir)
        })
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
        .unwrap()
        .1;
    let pitch = std::env::var("SPACE_PITCH").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(-0.2);
    let fwd = (best * pitch.cos() + up * pitch.sin()).as_vec3();
    if let Ok(mut t) = q.single_mut() {
        *t = Transform::IDENTITY.looking_to(fwd, up.as_vec3());
    }
}

/// Turn the player towards (SEA = true) or away from the lowest ground ~300 m around.
fn face_sea<const SEA: bool>(mut q: Query<(&mut Player, &avian3d::prelude::Position)>, planet: Res<Planet>) {
    let Ok((mut p, pos)) = q.single_mut() else { return };
    let up = pos.0.normalize();
    let e1 = crate::player::tangent(DVec3::Y, up);
    let e2 = up.cross(e1);
    let best = (0..32)
        .map(|i| {
            let a = i as f64 / 32.0 * std::f64::consts::TAU;
            let dir = e1 * a.cos() + e2 * a.sin();
            let h: f64 = [150.0, 300.0, 600.0].iter().map(|r| planet.terrain.height((pos.0 + dir * *r).normalize())).sum();
            (h, dir)
        })
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
        .unwrap()
        .1;
    p.heading = if SEA { best } else { -best };
}

static AIM: std::sync::OnceLock<DVec3> = std::sync::OnceLock::new();

/// Free camera: look at the start target (river/lake starts).
fn aim_fly_cam(cam: Res<crate::camera::CameraWorld>, mut q: Query<&mut Transform, With<crate::camera::FlyCam>>) {
    let Some(target) = AIM.get() else { return };
    let up = cam.pos.normalize().as_vec3();
    let fwd = (*target - cam.pos).normalize().as_vec3();
    if let Ok(mut t) = q.single_mut() {
        *t = Transform::IDENTITY.looking_to(fwd, up);
    }
}

/// SPACE_CLOSEUP=metres: stand that far from the nearest big tree, facing its trunk.
fn closeup(mut q: Query<(&mut Player, &mut avian3d::prelude::Position)>, planet: Res<Planet>) {
    let Some(dist) = std::env::var("SPACE_CLOSEUP").ok().and_then(|v| v.parse::<f64>().ok()) else { return };
    let Ok((mut p, mut pos)) = q.single_mut() else { return };
    let trees = crate::flora::colliders_near(&planet.flora, pos.0, 60.0);
    let Some(tree) = trees
        .iter()
        .filter(|c| matches!(c.shape, crate::flora::FloraCollision::Trunk { .. }))
        .min_by(|a, b| (a.pos - pos.0).length().total_cmp(&(b.pos - pos.0).length()))
    else {
        return;
    };
    let up = tree.pos.normalize();
    let away = crate::player::tangent(pos.0 - tree.pos, up);
    let spot = crate::player::standing_position(&planet, (tree.pos + away * dist).normalize());
    pos.0 = spot;
    p.heading = crate::player::tangent(tree.pos - spot, up);
}

/// SPACE_FORAGE=1: stand next to the nearest foragable plant, facing it.
fn forage_setup(mut q: Query<(&mut Player, &mut avian3d::prelude::Position)>, planet: Res<Planet>) {
    if std::env::var("SPACE_FORAGE").is_err() {
        return;
    }
    let Ok((mut p, mut pos)) = q.single_mut() else { return };
    let plants = crate::flora::forageables_near(&planet.flora, pos.0, 80.0);
    // SPACE_FORAGE=<part of a species name> picks that species, if one is near.
    let want = std::env::var("SPACE_FORAGE").unwrap_or_default();
    let matching: Vec<_> = plants.iter().filter(|f| planet.flora.bio.flora[f.species].name.contains(want.as_str())).collect();
    let pool: Vec<_> = if matching.is_empty() { plants.iter().collect() } else { matching };
    let Some(f) = pool.into_iter().min_by(|a, b| (a.base - pos.0).length().total_cmp(&(b.base - pos.0).length())) else {
        warn!("no foragable plant nearby");
        return;
    };
    let up = f.base.normalize();
    let away = crate::player::tangent(pos.0 - f.base, up);
    let spot = crate::player::standing_position(&planet, (f.base + away * 1.7).normalize());
    pos.0 = spot;
    let to = f.point - (spot + up * 0.75);
    p.heading = crate::player::tangent(to, up);
    p.pitch = (to.normalize().dot(up)).asin() as f32;
    info!("forage test: {} at {:.1} m", planet.flora.bio.flora[f.species].name, (f.base - spot).length());
}

/// SPACE_FORAGE=1: pick at 8 s, eat at 9 s, open the inventory at 9.5 s.
fn forage_script(time: Res<Time>, mut input: ResMut<crate::forage::ForageInput>, mut open: ResMut<crate::forage::InventoryOpen>, target: Res<crate::forage::ForageTarget>, inv: Query<&crate::items::Inventory>, mut step: Local<u8>) {
    let t = time.elapsed_secs();
    if *step == 0 && t > 8.0 {
        info!("forage test: target {}", if target.0.is_some() { "in sight" } else { "NONE" });
        input.harvest = true;
        *step = 1;
    } else if *step == 1 && t > 9.0 {
        input.eat = true;
        *step = 2;
    } else if *step == 2 && t > 9.5 {
        open.0 = true;
        if let Ok(inv) = inv.single() {
            for (id, n) in inv.slots.iter().flatten() {
                info!("forage test: carrying {n} x {} ({})", id.def().name, id.def().rarity.name());
            }
        }
        *step = 3;
    }
}

/// SPACE_MINE=1: stand by the nearest ore deposit, hold E from 6 s.
fn mine_script(
    time: Res<Time>,
    planet: Res<Planet>,
    mut q: Query<(&mut Player, &mut avian3d::prelude::Position, &crate::items::Inventory)>,
    mut interact: ResMut<crate::interact::Interact>,
    mut step: Local<u8>,
) {
    let t = time.elapsed_secs();
    let Ok((mut p, mut pos, inv)) = q.single_mut() else { return };
    if *step == 0 && t > 5.0 {
        let deps = crate::mining::deposits_near(&planet.terrain, pos.0, 300.0);
        let Some(d) = deps.iter().min_by(|a, b| (a.base - pos.0).length().total_cmp(&(b.base - pos.0).length())) else {
            warn!("mine test: no deposit within 300 m");
            *step = 9;
            return;
        };
        let up = crate::world::up_at(d.base);
        let away = crate::player::tangent(pos.0 - d.base, up);
        let spot = crate::player::standing_position(&planet, d.base + away * 2.6 + up * 2.0);
        pos.0 = spot;
        let to = d.base + up * 0.5 - (spot + up * 0.75);
        p.heading = crate::player::tangent(to, up);
        p.pitch = to.normalize().dot(up).asin() as f32;
        info!("mine test: {} deposit ({} units) at {:.0} m", d.ore.def().name, d.amount, (d.base - spot).length());
        *step = 1;
    } else if *step == 1 && t > 7.0 {
        interact.force_hold = true;
        *step = 2;
    } else if *step == 2 && t > 12.0 {
        interact.force_hold = false;
        for (id, n) in inv.slots.iter().flatten() {
            info!("mine test: carrying {n} x {}", id.def().name);
        }
        *step = 3;
    }
}

/// SPACE_FACTORY=1: lay out drill > conveyor > conveyor > smelter > conveyor > storage on the
/// nearest deposit at 6 s, then report what reached the storage.
fn factory_script(
    time: Res<Time>,
    planet: Res<Planet>,
    mining: Res<crate::mining::Mining>,
    mut factory: ResMut<crate::factory::Factory>,
    mut q: Query<(&mut Player, &mut avian3d::prelude::Position)>,
    mut step: Local<u8>,
) {
    use crate::factory::{Module, ModuleKind, Site};
    let t = time.elapsed_secs();
    let Ok((mut p, mut pos)) = q.single_mut() else { return };
    if *step == 0 && t > 6.0 {
        let deps = crate::mining::deposits_near(&planet.terrain, pos.0, 300.0);
        let Some(d) = deps.iter().filter(|d| crate::factory::recipe(crate::factory::ModuleKind::Smelter, d.ore).is_some()).min_by(|a, b| (a.base - pos.0).length().total_cmp(&(b.base - pos.0).length())) else {
            warn!("factory test: no smeltable deposit within 300 m");
            *step = 9;
            return;
        };
        let up = crate::world::up_at(d.base);
        let e2 = crate::player::tangent(DVec3::Y, up);
        let e1 = e2.cross(up);
        let site = factory.add_site(Site { body: crate::world::nearest(d.base), origin: d.base.to_array(), e1: e1.to_array(), e2: e2.to_array() });
        let m = |kind, z: i32, deposit| Module { kind, site, cell: [0, z], facing: 0, stock: Vec::new(), output: Vec::new(), progress: 0.0, carried: None, deposit, felled: 0 };
        factory.insert(m(ModuleKind::Drill, 0, Some(d.cell)));
        factory.insert(m(ModuleKind::Conveyor, 1, None));
        factory.insert(m(ModuleKind::Conveyor, 2, None));
        factory.insert(m(ModuleKind::Smelter, 3, None));
        factory.insert(m(ModuleKind::Conveyor, 4, None));
        factory.insert(m(ModuleKind::Storage, 5, None));
        // Watch from the side.
        let mid = d.base + e2 * 5.0;
        let spot = crate::player::standing_position(&planet, mid + e1 * 9.0 + up * 3.0);
        pos.0 = spot;
        let to = mid + up * 1.0 - (spot + up * 0.75);
        p.heading = crate::player::tangent(to, up);
        p.pitch = to.normalize().dot(up).asin() as f32;
        info!("factory test: drill on {} ({} left)", d.ore.def().name, mining.remaining(d));
        *step = 1;
    } else if *step == 1 && t > 40.0 {
        for m in factory.modules.iter().flatten() {
            info!("factory test: {:?} stock {:?} output {:?} carried {:?}", m.kind, m.stock.iter().map(|s| (s.0.def().name, s.1)).collect::<Vec<_>>(), m.output.iter().map(|s| (s.0.def().name, s.1)).collect::<Vec<_>>(), m.carried.map(|c| c.0.def().name));
        }
        *step = 2;
    }
}

/// SPACE_LOGGING=1: logging station > conveyor > sawmill > conveyor > storage at the start; report
/// at 70 s (the screenshot shows the line and the stumps).
fn logging_script(
    time: Res<Time>,
    planet: Res<Planet>,
    mut factory: ResMut<crate::factory::Factory>,
    mut q: Query<(&mut Player, &mut avian3d::prelude::Position)>,
    regrowth: Res<crate::forage::Regrowth>,
    mut step: Local<u8>,
) {
    use crate::factory::{Module, ModuleKind, Site};
    let t = time.elapsed_secs();
    let Ok((mut p, mut pos)) = q.single_mut() else { return };
    if *step == 1 && t > 60.0 && std::env::var("SPACE_LOGGING").as_deref() == Ok("stump") {
        // Look at the first stump from 7 m.
        if let Some(&(_, at, _)) = regrowth.0.first() {
            let up = crate::world::up_at(at);
            let side = crate::player::tangent(at - pos.0, up);
            let spot = crate::player::standing_position(&planet, at - side * 7.0 + up * 3.0);
            pos.0 = spot;
            let to = at + up * 0.8 - (spot + up * 0.75);
            p.heading = crate::player::tangent(to, up);
            p.pitch = to.normalize().dot(up).asin() as f32;
        }
    }
    if *step == 0 && t > 6.0 {
        let up = crate::world::up_at(pos.0);
        let e2 = crate::player::tangent(DVec3::Y, up);
        let e1 = e2.cross(up);
        // Back out of the start glade (45 m) into the trees.
        let origin = crate::world::ground_point(&planet.terrain, pos.0 - e2 * 70.0);
        let trees = crate::flora::trees_near(&planet.flora, origin, crate::factory::LOGGING_RADIUS);
        info!("logging test: {} trees in reach", trees.len());
        let site = factory.add_site(Site { body: crate::world::nearest(origin), origin: origin.to_array(), e1: e1.to_array(), e2: e2.to_array() });
        let m = |kind, z: i32| Module { kind, site, cell: [0, z], facing: 0, stock: Vec::new(), output: Vec::new(), progress: 0.0, carried: None, deposit: None, felled: 0 };
        factory.insert(m(ModuleKind::LoggingStation, 0));
        factory.insert(m(ModuleKind::Conveyor, 1));
        factory.insert(m(ModuleKind::Sawmill, 2));
        factory.insert(m(ModuleKind::Conveyor, 3));
        factory.insert(m(ModuleKind::Storage, 4));
        let mid = origin + e2 * 4.0;
        let spot = crate::player::standing_position(&planet, mid + e1 * 9.0 + up * 3.0);
        pos.0 = spot;
        let to = mid + up * 1.5 - (spot + up * 0.75);
        p.heading = crate::player::tangent(to, up);
        p.pitch = to.normalize().dot(up).asin() as f32;
        *step = 1;
    } else if *step == 1 && t > 70.0 {
        for m in factory.modules.iter().flatten() {
            info!("logging test: {:?} felled {} stock {:?} output {:?}", m.kind, m.felled, m.stock.iter().map(|s| (s.0.def().name, s.1)).collect::<Vec<_>>(), m.output.iter().map(|s| (s.0.def().name, s.1)).collect::<Vec<_>>());
        }
        *step = 2;
    }
}

/// SPACE_TRADE=<base>: carry some goods to the base's terminal, open the market at 8 s and sell
/// all berries at 9 s (screenshot shows the trade screen).
#[allow(clippy::too_many_arguments)]
fn trade_script(
    time: Res<Time>,
    planet: Res<Planet>,
    mut q: Query<(&mut Player, &mut avian3d::prelude::Position, &mut crate::items::Inventory)>,
    mut interact: ResMut<crate::interact::Interact>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    eco: Res<crate::economy::Economy>,
    mut step: Local<u8>,
) {
    let base: usize = std::env::var("SPACE_TRADE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let t = time.elapsed_secs();
    let Ok((mut p, mut pos, mut inv)) = q.single_mut() else { return };
    if *step == 0 && t > 6.0 {
        inv.add(crate::items::item::BERRIES, 40);
        inv.add(crate::items::item::IRON_PLATE, 20);
        let tp = crate::economy::terminal_pos(&planet.terrain, base);
        let up = crate::world::up_at(tp);
        let fwd = crate::player::tangent(DVec3::Y, up);
        let spot = crate::player::standing_position(&planet, tp - fwd * 2.3 + up * 1.5);
        pos.0 = spot;
        let to = tp + up * 1.2 - (spot + up * 0.75);
        p.heading = crate::player::tangent(to, up);
        p.pitch = to.normalize().dot(up).asin() as f32;
        info!("trade test: berries sell for {} cr each here, {} cr at the other base", eco.sell_price(base, crate::items::item::BERRIES), eco.sell_price(1 - base, crate::items::item::BERRIES));
        *step = 1;
    } else if *step == 1 && t > 8.0 {
        interact.pressed = true;
        *step = 2;
    } else if *step == 2 && t > 9.0 {
        keys.press(KeyCode::ShiftLeft);
        keys.press(KeyCode::Enter);
        *step = 3;
    } else if *step == 3 {
        keys.release(KeyCode::ShiftLeft);
        keys.release(KeyCode::Enter);
        info!("trade test: credits now {}", eco.credits);
        *step = 4;
    }
}

/// SPACE_SAVETEST=1: change things and press F5 at 6 s. SPACE_SAVETEST=2: report what loaded.
#[allow(clippy::too_many_arguments)]
fn save_script(
    time: Res<Time>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut q: Query<(&mut crate::items::Inventory, &avian3d::prelude::Position), With<Player>>,
    mut eco: ResMut<crate::economy::Economy>,
    mut survival: ResMut<crate::forage::Survival>,
    mut step: Local<u8>,
) {
    let phase = std::env::var("SPACE_SAVETEST").unwrap_or_default();
    let t = time.elapsed_secs();
    let Ok((mut inv, pos)) = q.single_mut() else { return };
    if *step == 0 && t > 6.0 {
        if phase == "1" {
            inv.add(crate::items::item::VOID_CRYSTAL, 3);
            inv.add(crate::items::item::IRON_PLATE, 42);
            eco.credits = 4242;
            survival.food = 33.0;
            eco.markets[1][crate::items::item::BERRIES.0 as usize].stock = 5.0;
            keys.press(KeyCode::F5);
            info!("save test: saving at {:.1?}", pos.0.to_array());
        } else {
            let items: Vec<String> = inv.slots.iter().flatten().map(|(id, n)| format!("{n} {}", id.def().name)).collect();
            info!("save test: loaded credits {}, food {:.0}, items [{}], Selene berry stock {:.0}, at {:.1?}", eco.credits, survival.food, items.join(", "), eco.markets[1][crate::items::item::BERRIES.0 as usize].stock, pos.0.to_array());
        }
        *step = 1;
    } else if *step == 1 {
        keys.release(KeyCode::F5);
        *step = 2;
    }
}

/// SPACE_AUTOPILOT=<target index>: engage the travel autopilot at 5 s, log progress.
fn autopilot_script(
    time: Res<Time>,
    mut nav: ResMut<crate::nav::Nav>,
    mut ships: Query<(Entity, &avian3d::prelude::Position, &avian3d::prelude::LinearVelocity, &mut crate::ship::Ship)>,
    planet: Res<Planet>,
    factory: Res<crate::factory::Factory>,
    mut next: Local<f32>,
    mut step: Local<u8>,
) {
    let t = time.elapsed_secs();
    if *step == 0 && t > 5.0 {
        nav.target = std::env::var("SPACE_AUTOPILOT").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        let dests = crate::nav::destinations(&planet, &factory);
        if let Ok((e, p, _, mut ship)) = ships.single_mut() {
            let name = crate::nav::engage(&mut nav, &dests, e, &mut ship, p.0);
            info!("autopilot test: engaged towards {name}");
        }
        *step = 1;
        *next = t + 1.0;
    }
    if *step >= 1 && t > *next {
        *next = t + 15.0;
        if let Ok((_, p, v, s)) = ships.single() {
            info!("autopilot test: t={t:.0}s  {}  speed {:.0} m/s  alt {:.0} m  autopilot {}", nav.status, v.0.length(), crate::world::altitude(&planet.terrain, p.0), s.autopilot);
        }
    }
}

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Prof>().add_systems(PostStartup, (aim_fly_cam, closeup.after(crate::ship::spawn_ship), forage_setup.after(crate::ship::spawn_ship)));
        if std::env::var("SPACE_FORAGE").is_ok() {
            app.add_systems(Update, forage_script);
        }
        if std::env::var("SPACE_MINE").is_ok() {
            app.add_systems(Update, mine_script.in_set(crate::interact::InteractSet::Reset));
        }
        if std::env::var("SPACE_SAVETEST").is_ok() {
            app.add_systems(PreUpdate, save_script.after(bevy::input::InputSystem));
        }
        if std::env::var("SPACE_AUTOPILOT").is_ok() {
            app.add_systems(PreUpdate, autopilot_script.after(bevy::input::InputSystem));
        }
        if std::env::var("SPACE_TRADE").is_ok() {
            app.add_systems(Update, trade_script.in_set(crate::interact::InteractSet::Reset));
        }
        if std::env::var("SPACE_LOGGING").is_ok() {
            app.add_systems(Update, logging_script);
        }
        if std::env::var("SPACE_FACTORY").is_ok() {
            app.add_systems(Update, factory_script);
        }
        if std::env::var("SPACE_START").is_ok_and(|s| s == "leo") {
            app.add_systems(PostStartup, place_in_orbit.after(crate::ship::spawn_ship));
        }
        if std::env::var("SPACE_START").is_ok_and(|s| s.ends_with("coast")) {
            // Face inland while the ship is placed (26 m ahead), then turn round to the sea.
            app.add_systems(PostStartup, face_sea::<false>.before(crate::ship::spawn_ship))
                .add_systems(PostStartup, face_sea::<true>.after(crate::ship::spawn_ship))
                .add_systems(PostStartup, aim_fly_cam_at_sea);
        }
        if let Some(pitch) = std::env::var("SPACE_PITCH").ok().and_then(|v| v.parse::<f32>().ok()) {
            app.add_systems(PostStartup, move |mut q: Query<&mut Player>| {
                if let Ok(mut p) = q.single_mut() {
                    p.pitch = pitch;
                }
            });
        }
        if std::env::var("SPACE_THIRD_PERSON").is_ok() {
            app.add_systems(PostStartup, |mut q: Query<&mut Player>| {
                if let Ok(mut p) = q.single_mut() {
                    p.third_person = true;
                }
            });
        }
        if std::env::var("SPACE_AUTOWALK").is_ok() {
            let run = std::env::var("SPACE_AUTOWALK").is_ok_and(|v| v == "run");
            app.insert_resource(AutoWalk { run }).init_resource::<WalkStats>().add_systems(Update, walk_stats);
            // Run away from the parked ship, across open ground.
            app.add_systems(
                PostStartup,
                (|mut q: Query<&mut Player>| {
                    if let Ok(mut p) = q.single_mut() {
                        p.heading = -p.heading;
                    }
                })
                .after(crate::ship::spawn_ship),
            );
        }
        if let Ok(test) = std::env::var("SPACE_SHIPTEST") {
            app.insert_resource(ShipTest(test))
                .init_resource::<ShipInput>()
                .add_systems(Update, ship_script.before(crate::ship::board_or_exit));
        }
        if std::env::var("SPACE_PROFILE").is_ok() {
            app.add_plugins((
                bevy::render::diagnostic::RenderDiagnosticsPlugin,
                bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            ))
            .add_systems(Last, profile_report)
            .add_systems(PostUpdate, (
                (|mut p: ResMut<Prof>| p.2 = Some(std::time::Instant::now())).before(avian3d::prelude::PhysicsSet::Prepare),
                (|mut p: ResMut<Prof>| if let Some(t) = p.2.take() { p.add("physics", t) }).after(avian3d::prelude::PhysicsSet::Sync),
            ))
            .add_systems(Update, |time: Res<Time>, mut prof: ResMut<Prof>| {
                // Frame times after the first 5 s (start-up and shader compilation).
                if time.elapsed_secs() > 5.0 {
                    prof.1.push(time.delta_secs() * 1000.0);
                }
            });
        }
        if let Ok(path) = std::env::var("SPACE_SCREENSHOT") {
            let min_time = std::env::var("SPACE_SHOT_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(4.0);
            app.insert_resource(ShotState { path, settled_frames: 0, taken_at: None, frames: 0, settled_since: 0.0, min_time })
                .add_systems(Update, auto_screenshot);
        }
    }
}

/// Profiling switch: is `what` listed in SPACE_HIDE (comma-separated)?
pub fn hidden(what: &str) -> bool {
    std::env::var("SPACE_HIDE").is_ok_and(|v| v.split(',').any(|x| x == what))
}

/// CPU time of our own heavy systems (SPACE_PROFILE): name -> (total seconds, calls).
#[derive(Resource, Default)]
pub struct Prof(pub std::collections::HashMap<&'static str, (f64, u32, f64)>, pub Vec<f32>, pub Option<std::time::Instant>);

impl Prof {
    pub fn add(&mut self, name: &'static str, t: std::time::Instant) {
        let dt = t.elapsed().as_secs_f64();
        let e = self.0.entry(name).or_default();
        e.0 += dt;
        e.1 += 1;
        e.2 = e.2.max(dt);
    }
}

/// Adds the time from its creation to `Prof` when dropped.
pub struct ProfGuard<'a, 'w>(pub &'a mut ResMut<'w, Prof>, pub &'static str, pub std::time::Instant);

impl Drop for ProfGuard<'_, '_> {
    fn drop(&mut self) {
        self.0.add(self.1, self.2);
    }
}

/// When the screenshot is taken, log GPU and CPU time per render pass and our systems' CPU time.
fn profile_report(
    shot: Option<Res<ShotState>>,
    store: Res<bevy::diagnostic::DiagnosticsStore>,
    prof: Option<Res<Prof>>,
    mut done: Local<bool>,
) {
    let Some(shot) = shot else { return };
    if *done || shot.taken_at.is_none() {
        return;
    }
    *done = true;
    let mut rows: Vec<(String, f64)> = store
        .iter()
        .filter_map(|d| Some((d.path().to_string(), d.average()?)))
        .filter(|(p, v)| (p.contains("elapsed_gpu") || p.contains("frame_time") || p.contains("elapsed_cpu")) && *v > 0.05)
        .collect();
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    for (p, v) in rows.iter().take(25) {
        info!("PROFILE {p}: {v:.2} ms");
    }
    if let Some(prof) = prof {
        let mut f = prof.1.clone();
        f.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if !f.is_empty() {
            let q = |p: f32| f[((f.len() - 1) as f32 * p) as usize];
            info!("PROFILE frames: {} | p50 {:.1} ms | p99 {:.1} ms | max {:.1} ms | >33ms: {}", f.len(), q(0.5), q(0.99), f[f.len() - 1], f.iter().filter(|&&x| x > 33.0).count());
        }
        for (name, (t, n, mx)) in &prof.0 {
            info!("PROFILE cpu/{name}: {:.3} ms/call, max {:.1} ms", t * 1000.0 / (*n).max(1) as f64, mx * 1000.0);
        }
    }
}

#[derive(Resource)]
struct ShotState {
    path: String,
    settled_frames: u32,
    taken_at: Option<f32>,
    /// Frames rendered since loading finished, for an average FPS figure.
    frames: u32,
    settled_since: f32,
    /// Don't take the screenshot before this many seconds.
    min_time: f32,
}

#[derive(Resource)]
struct ShipTest(String);

/// Scripted tests. The ship parks 26 m in front of the player, ramp first. Walking straight
/// ahead goes up the ramp, through the door and down the cabin to the pilot seat.
///   walkin:  walk into the ship (screenshot from inside).
///   fly:     walk in, sit at 11 s, climb until 14 s, fly forward until 17 s, then hover.
///   land:    walk in, sit at 11 s, climb, hover, descend from 15 s, get up at 25 s,
///            walk out of the door and down the ramp.
///   flywalk: walk in, sit at 11 s, climb until 14 s, get up at 14.5 s (engines keep hovering),
///            the ship flies forward on autopilot from 15 s while the player walks to the back.
fn ship_script(
    mut commands: Commands,
    ships: Query<
        (&avian3d::prelude::Position, &avian3d::prelude::Rotation, &avian3d::prelude::LinearVelocity),
        With<crate::ship::Ship>,
    >,
    mut phase: Local<u8>,
    time: Res<Time>,
    test: Res<ShipTest>,
    mut input: ResMut<ShipInput>,
    mut done: Local<[bool; 2]>,
) {
    let t = time.elapsed_secs();
    *input = ShipInput::default();
    let mut walk = t < 10.5 && test.0 != "deorbit";
    let mut once = |i: usize, at: f32, input: &mut ShipInput| {
        if t >= at && !done[i] {
            done[i] = true;
            input.toggle_board = true;
        }
    };
    match test.0.as_str() {
        "walkin" => {}
        // Start with SPACE_START=leo. Brake to a stop, drop towards the planet under control,
        // then let flight assist take over below 60 km and descend to a landing.
        "deorbit" => {
            if let (true, Ok((pos, rot, vel))) = (t > 4.0, ships.single()) {
                let up = pos.0.normalize();
                let alt = pos.0.length() - RADIUS;
                let speed = vel.0.length();
                match *phase {
                    0 => {
                        input.brake = true;
                        if speed < 2.0 {
                            *phase = 1;
                            info!("deorbit: stopped at t={t:.1}s, altitude {:.0} km", alt / 1000.0);
                        }
                    }
                    1 => {
                        // Fall at a controlled speed that shrinks as we approach 50 km.
                        let want = -up * ((alt - 50_000.0) / 10.0).clamp(300.0, 6_000.0);
                        let world = (want - vel.0) / 120.0;
                        let local = rot.0.inverse() * world;
                        let local = local / local.abs().max_element().max(1.0);
                        input.side = local.x;
                        input.vertical = local.y;
                        input.forward = -local.z;
                        input.boost = true;
                        if alt < crate::ship::ASSIST_CEILING {
                            *phase = 2;
                            info!("deorbit: flight assist took over at t={t:.1}s, speed {speed:.0} m/s");
                        }
                    }
                    2 => {
                        input.vertical = -1.0;
                        if speed < 0.5 && vel.0.dot(up).abs() < 0.5 && alt < 20_000.0 {
                            *phase = 3;
                            info!("deorbit: landed at t={t:.1}s");
                        }
                    }
                    _ => {}
                }
            }
        }
        // Fly to orbit: sit down, climb straight up past the assist ceiling, then burn
        // forward (horizontally) with boost, levelling off at ~125 km, until the lowest point
        // of the orbit clears the atmosphere (100 km).
        "ascent" => {
            once(0, 11.0, &mut input);
            if let (true, Ok((pos, rot, vel))) = (t > 11.5, ships.single()) {
                let alt = pos.0.length() - RADIUS;
                let pe = crate::physics::orbit_extremes(pos.0, vel.0).map(|o| o.1).unwrap_or(f64::MAX);
                if *phase == 0 && alt > 70_000.0 {
                    *phase = 1;
                    info!("ascent: passed 70 km at t={t:.1}s, vertical speed {:.0} m/s", vel.0.dot(pos.0.normalize()));
                }
                if *phase == 1 && pe > 105_000.0 {
                    *phase = 2;
                    info!("ascent: orbit reached at t={t:.1}s, speed {:.0} m/s", vel.0.length());
                }
                match *phase {
                    0 => input.vertical = 1.0,
                    1 => {
                        // Burn horizontally while steering the vertical speed towards a
                        // ~125 km altitude, like a real gravity turn.
                        let up = pos.0.normalize();
                        let v_up = vel.0.dot(up);
                        let v_target = ((125_000.0 - alt) / 30.0).clamp(-1500.0, 1500.0);
                        let vertical = ((v_target - v_up) / 100.0).clamp(-1.0, 1.0);
                        // Thrust along the local horizontal in the direction of travel, plus the
                        // vertical correction, converted into the ship's own axes.
                        // Match circular-orbit speed for this height horizontally.
                        let mu = crate::player::GRAVITY * RADIUS * RADIUS;
                        let v_circ = (mu / pos.0.length()).sqrt();
                        let v_h = (vel.0 - up * v_up).length();
                        let h_cmd = ((v_circ - v_h) / 200.0).clamp(-1.0, 1.0);
                        let horizontal = crate::player::tangent(vel.0, up) * h_cmd;
                        let world = horizontal + up * vertical;
                        let local = rot.0.inverse() * world;
                        let local = local / local.abs().max_element().max(1.0);
                        input.side = local.x;
                        input.vertical = local.y;
                        input.forward = -local.z;
                        input.boost = true;
                    }
                    _ => {}
                }
            }
        }
        "land" => {
            once(0, 11.0, &mut input);
            if (11.0..13.0).contains(&t) {
                input.vertical = 1.0;
            } else if (15.0..24.5).contains(&t) {
                input.vertical = -1.0;
            }
            once(1, 25.0, &mut input);
            walk |= (25.5..31.0).contains(&t);
        }
        "flywalk" => {
            once(0, 11.0, &mut input);
            if (11.0..14.0).contains(&t) {
                input.vertical = 1.0;
            }
            once(1, 14.5, &mut input);
            if t >= 15.0 {
                input.autopilot = true;
                input.forward = 1.0;
            }
            walk |= (15.5..17.5).contains(&t);
        }
        _ => {
            once(0, 11.0, &mut input);
            if (11.0..14.0).contains(&t) {
                input.vertical = 1.0;
            } else if (14.0..17.0).contains(&t) {
                input.forward = 1.0;
            }
        }
    }
    if walk {
        commands.insert_resource(AutoWalk { run: false });
    } else {
        commands.remove_resource::<AutoWalk>();
    }
}

fn auto_screenshot(
    mut commands: Commands,
    planet: Res<Planet>,
    time: Res<Time>,
    player: Query<(
        &Player,
        &avian3d::prelude::Position,
        &avian3d::prelude::LinearVelocity,
        Option<&crate::player::InShip>,
    )>,
    ships: Query<
        (&avian3d::prelude::Position, &avian3d::prelude::Rotation, &avian3d::prelude::LinearVelocity),
        (With<crate::ship::Ship>, Without<Player>),
    >,
    mode: Res<ControlMode>,
    walk: Option<Res<WalkStats>>,
    mut state: ResMut<ShotState>,
    mut exit: EventWriter<AppExit>,
) {
    if let Some(t) = state.taken_at {
        if time.elapsed_secs() - t > 2.0 {
            exit.write(AppExit::Success);
        }
        return;
    }
    // Frame rate is measured only while nothing is loading (worker threads idle).
    if planet.stats.settled {
        if state.settled_frames == 0 {
            state.frames = 0;
            state.settled_since = time.elapsed_secs();
        }
        state.settled_frames += 1;
        state.frames += 1;
    } else {
        state.settled_frames = 0;
    }
    // Shader pipelines compile asynchronously, so also give the renderer a few seconds.
    if (state.settled_frames >= 30 && time.elapsed_secs() > state.min_time) || time.elapsed_secs() > state.min_time.max(30.0) + 30.0 {
        let s = planet.stats;
        info!(
            "screenshot: {} chunks shown, max LOD {}, {:.2} M plant triangles {:?}, t={:.1}s, avg {:.0} fps",
            s.shown,
            s.max_level,
            s.flora_tris as f64 / 1e6,
            s.flora_layers.map(|t| format!("{:.2}M", t as f64 / 1e6)),
            time.elapsed_secs(),
            state.frames as f32 / (time.elapsed_secs() - state.settled_since).max(0.1)
        );
        if let Ok((_, pos, vel, Some(inside))) = player.single() {
            let l = pos.0 - inside.proxy;
            info!(
                "player: inside ship at local ({:.2}, {:.2}, {:.2}), speed relative to ship {:.2} m/s",
                l.x, l.y, l.z,
                vel.0.length()
            );
        } else if let Ok((p, pos, vel, None)) = player.single() {
            let up = pos.0.normalize();
            let feet = pos.0.length() - crate::player::HALF_HEIGHT - RADIUS - planet.terrain.surface_height(up);
            info!(
                "player: feet {:.2} m above terrain, grounded {}, speed {:.2} m/s, moved {:.1} m from spawn",
                feet,
                p.grounded,
                vel.0.length(),
                (pos.0 - p.spawn).length()
            );
        }
        for (pos, rot, vel) in &ships {
            if let Some((ap, pe)) = crate::physics::orbit_extremes(pos.0, vel.0) {
                info!(
                    "orbit: altitude {:.1} km, speed {:.0} m/s, apoapsis {:.1} km, periapsis {:.1} km",
                    (pos.0.length() - RADIUS) / 1000.0,
                    vel.0.length(),
                    ap / 1000.0,
                    pe / 1000.0
                );
            }
            let up = pos.0.normalize();
            let clearance =
                pos.0.length() - crate::ship::LEG_HEIGHT - RADIUS - planet.terrain.surface_height(up);
            let tilt = (rot.0 * DVec3::Y).dot(up).clamp(-1.0, 1.0).acos().to_degrees();
            info!(
                "ship: pads {:.2} m above terrain, tilt {:.1} deg, speed {:.2} m/s, mode {:?}",
                clearance,
                tilt,
                vel.0.length(),
                *mode
            );
        }
        if let Some(w) = walk {
            info!(
                "walk: airborne {:.1}% of {} frames, feet up to {:.2} m above terrain",
                100.0 * w.airborne as f64 / w.frames.max(1) as f64,
                w.frames,
                w.max_feet
            );
        }
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(state.path.clone()));
        state.taken_at = Some(time.elapsed_secs());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planet::SEED;

    #[test]
    #[ignore]
    fn profile_around_spots() {
        let t = Terrain::new(SEED);
        for (name, range) in [("ground", 100.0..800.0), ("mountain", 3000.0..9000.0)] {
            let d = find_spot(&t, range);
            let tangent = d.cross(DVec3::Y).normalize();
            let line: Vec<String> = (0..=20)
                .map(|i| {
                    let km = i as f64 * 2.5;
                    let p = (d + tangent * (km * 1000.0 / RADIUS)).normalize();
                    format!("{:.0}", t.height(p))
                })
                .collect();
            println!("{name} (every 2.5 km): {}", line.join(" "));
            let fine: Vec<String> = (0..=20)
                .map(|i| {
                    let p = (d + tangent * (i as f64 * 50.0 / RADIUS)).normalize();
                    format!("{:.1}", t.height(p))
                })
                .collect();
            println!("{name} (every 50 m):   {}", fine.join(" "));
        }
    }
}
