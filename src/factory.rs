//! Modular factories: drills on ore deposits, conveyors, smelters and storage boxes, snapped to
//! the 2 m grid of a factory site and simulated continuously.
//!
//! - B toggles build mode. 1-4 picks a module, R rotates it, E places it (paying its cost from
//!   the inventory), X takes down the module you look at (full refund, contents included).
//! - The first module placed somewhere starts a site; modules within 150 m snap to its grid.
//! - Every module passes what it makes to the module in front of it (the arrow on conveyors):
//!   drills mine the deposit under them, conveyors carry items one cell a second, smelters turn
//!   ore into plates, wire, silicon or alloy, storage boxes collect anything.
//! - Out of build mode, E on a module collects its output (or loads ore into a smelter).
//!
//! The state (`Factory`) is plain data, simulated here and saved by save.rs; entities are only
//! its visuals and colliders.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use avian3d::prelude::*;
use bevy::image::ImageLoaderSettings;
use bevy::math::{DQuat, DVec3, IVec2};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::forage::Toasts;
use crate::interact::{Focus, FocusKind, Interact, View, aim};
use crate::items::{Inventory, ItemId, item};
use crate::mining::{Deposit, Mining, deposits_near};
use crate::planet::{Planet, WorldPos};
use crate::player::{ControlMode, Player};
use crate::world::Body;

/// Grid cell size (m).
pub const CELL: f64 = 2.0;
/// New modules within this distance of a site join its grid.
const SITE_RADIUS: f64 = 150.0;

static BUILDING: AtomicBool = AtomicBool::new(false);

/// True while build mode is on (other systems leave their keys alone).
pub fn building() -> bool {
    BUILDING.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub enum ModuleKind {
    Drill,
    Conveyor,
    Smelter,
    Storage,
    TitaniumDrill,
    PlasmaDrill,
    LoggingStation,
    Sawmill,
}

impl ModuleKind {
    pub const ALL: [ModuleKind; 8] = [
        ModuleKind::Drill,
        ModuleKind::Conveyor,
        ModuleKind::Smelter,
        ModuleKind::Storage,
        ModuleKind::TitaniumDrill,
        ModuleKind::PlasmaDrill,
        ModuleKind::LoggingStation,
        ModuleKind::Sawmill,
    ];

    /// Drills: the hardest ore tier they can cut (see mining::ore_tier).
    pub fn drill_tier(self) -> Option<u8> {
        match self {
            ModuleKind::Drill => Some(1),
            ModuleKind::TitaniumDrill => Some(2),
            ModuleKind::PlasmaDrill => Some(3),
            _ => None,
        }
    }

    /// Modules that turn one item into another.
    pub fn processor(self) -> bool {
        matches!(self, ModuleKind::Smelter | ModuleKind::Sawmill)
    }

    pub fn name(self) -> &'static str {
        match self {
            ModuleKind::Drill => "Mining drill",
            ModuleKind::Conveyor => "Conveyor",
            ModuleKind::Smelter => "Smelter",
            ModuleKind::Storage => "Storage box",
            ModuleKind::TitaniumDrill => "Titanium drill",
            ModuleKind::PlasmaDrill => "Plasma drill",
            ModuleKind::LoggingStation => "Logging station",
            ModuleKind::Sawmill => "Sawmill",
        }
    }

    /// What it costs to build (refunded when taken down).
    pub fn cost(self) -> &'static [(ItemId, u32)] {
        match self {
            ModuleKind::Drill => &[(item::IRON_PLATE, 8), (item::COPPER_WIRE, 4)],
            ModuleKind::Conveyor => &[(item::IRON_PLATE, 1)],
            // Built from raw ore, so the first one bootstraps everything else.
            ModuleKind::Smelter => &[(item::IRON_ORE, 15), (item::COPPER_ORE, 5)],
            ModuleKind::Storage => &[(item::IRON_PLATE, 6)],
            // Titanium alloy: smelt titanium ore (needs this drill...) or buy it at Selene Station.
            ModuleKind::TitaniumDrill => &[(item::TITANIUM_ALLOY, 10), (item::IRON_PLATE, 10), (item::COPPER_WIRE, 6)],
            ModuleKind::PlasmaDrill => &[(item::TITANIUM_ALLOY, 25), (item::SILICON, 12), (item::COPPER_WIRE, 12)],
            ModuleKind::LoggingStation => &[(item::IRON_PLATE, 10), (item::COPPER_WIRE, 4)],
            ModuleKind::Sawmill => &[(item::IRON_PLATE, 6), (item::COPPER_WIRE, 2)],
        }
    }
}

/// Processing recipes: (module, input) -> (output, how many, seconds).
pub fn recipe(kind: ModuleKind, input: ItemId) -> Option<(ItemId, u32, f32)> {
    match (kind, input) {
        (ModuleKind::Smelter, item::IRON_ORE) => Some((item::IRON_PLATE, 1, 2.0)),
        (ModuleKind::Smelter, item::COPPER_ORE) => Some((item::COPPER_WIRE, 1, 2.0)),
        (ModuleKind::Smelter, item::QUARTZ) => Some((item::SILICON, 1, 4.0)),
        (ModuleKind::Smelter, item::TITANIUM_ORE) => Some((item::TITANIUM_ALLOY, 1, 5.0)),
        (ModuleKind::Sawmill, item::SOFTWOOD_LOG) => Some((item::LUMBER, 3, 2.0)),
        (ModuleKind::Sawmill, item::HARDWOOD_LOG) => Some((item::HARDWOOD_PLANKS, 2, 3.0)),
        (ModuleKind::Sawmill, item::REDWOOD_LOG) => Some((item::REDWOOD_BEAMS, 2, 4.0)),
        _ => None,
    }
}

/// How far a logging station reaches, and how long felling and bucking one tree takes.
pub const LOGGING_RADIUS: f64 = 24.0;
const FELL_SECONDS: f32 = 15.0;
/// Felled trees grow back after this long (real seconds).
const TREE_REGROW: f64 = 3.0 * 3600.0;

/// A factory site: a grid frame on a body's surface.
#[derive(Clone, Serialize, Deserialize)]
pub struct Site {
    pub body: Body,
    pub origin: [f64; 3],
    pub e1: [f64; 3],
    pub e2: [f64; 3],
}

impl Site {
    fn origin(&self) -> DVec3 {
        DVec3::from(self.origin)
    }
    fn axes(&self) -> (DVec3, DVec3) {
        (DVec3::from(self.e1), DVec3::from(self.e2))
    }
    fn up(&self) -> DVec3 {
        crate::world::up_at(self.origin())
    }
    /// Cell containing a point.
    fn cell_of(&self, p: DVec3) -> IVec2 {
        let (e1, e2) = self.axes();
        let d = p - self.origin();
        IVec2::new((d.dot(e1) / CELL).round() as i32, (d.dot(e2) / CELL).round() as i32)
    }
    /// Unit vector of a facing (0..3).
    fn dir(&self, facing: u8) -> DVec3 {
        let (e1, e2) = self.axes();
        [e2, e1, -e2, -e1][facing as usize % 4]
    }
}

fn step(facing: u8) -> IVec2 {
    [IVec2::new(0, 1), IVec2::new(1, 0), IVec2::new(0, -1), IVec2::new(-1, 0)][facing as usize % 4]
}

/// One placed module (plain data).
#[derive(Clone, Serialize, Deserialize)]
pub struct Module {
    pub kind: ModuleKind,
    pub site: usize,
    pub cell: [i32; 2],
    pub facing: u8,
    /// Smelter input / storage contents.
    pub stock: Vec<(ItemId, u32)>,
    /// Finished items waiting to move on.
    pub output: Vec<(ItemId, u32)>,
    pub progress: f32,
    /// Conveyor: item in transit and how far along (0..1).
    pub carried: Option<(ItemId, f32)>,
    /// Drill: the deposit cell it mines.
    pub deposit: Option<u64>,
    /// Logging station: trees felled so far.
    #[serde(default)]
    pub felled: u32,
}

fn count(v: &[(ItemId, u32)]) -> u32 {
    v.iter().map(|s| s.1).sum()
}

fn put(v: &mut Vec<(ItemId, u32)>, id: ItemId, n: u32) {
    match v.iter_mut().find(|s| s.0 == id) {
        Some(s) => s.1 += n,
        None => v.push((id, n)),
    }
}

fn take_one(v: &mut Vec<(ItemId, u32)>) -> Option<ItemId> {
    let s = v.first_mut()?;
    let id = s.0;
    s.1 -= 1;
    if s.1 == 0 {
        v.remove(0);
    }
    Some(id)
}

impl Module {
    fn accepts(&self, id: ItemId) -> bool {
        match self.kind {
            ModuleKind::Conveyor => self.carried.is_none(),
            ModuleKind::Smelter | ModuleKind::Sawmill => recipe(self.kind, id).is_some() && count(&self.stock) < 20,
            ModuleKind::Storage => count(&self.stock) < 500,
            _ => false,
        }
    }

    fn accept(&mut self, id: ItemId) {
        match self.kind {
            ModuleKind::Conveyor => self.carried = Some((id, 0.0)),
            _ => put(&mut self.stock, id, 1),
        }
    }
}

/// All factory state.
#[derive(Resource, Default, Serialize, Deserialize)]
pub struct Factory {
    pub sites: Vec<Site>,
    pub modules: Vec<Option<Module>>,
    #[serde(skip)]
    grid: HashMap<(usize, [i32; 2]), usize>,
    /// Visual and collider entities per module index.
    #[serde(skip)]
    entities: HashMap<usize, (Entity, Option<Entity>)>,
    /// Item cubes on conveyors.
    #[serde(skip)]
    cubes: HashMap<usize, Entity>,
}

impl Factory {
    /// Rebuild the grid index and forget visuals (after loading).
    pub fn reindex(&mut self, commands: &mut Commands) {
        self.grid.clear();
        for (i, m) in self.modules.iter().enumerate() {
            if let Some(m) = m {
                self.grid.insert((m.site, m.cell), i);
            }
        }
        for (_, (e, c)) in self.entities.drain() {
            commands.entity(e).despawn();
            if let Some(c) = c {
                commands.entity(c).despawn();
            }
        }
        for (_, e) in self.cubes.drain() {
            commands.entity(e).despawn();
        }
    }

    /// Add a site (tests, loading).
    pub fn add_site(&mut self, site: Site) -> usize {
        self.sites.push(site);
        self.sites.len() - 1
    }

    /// Add a module (tests); its visuals appear on the next frame.
    pub fn insert(&mut self, m: Module) -> usize {
        let idx = self.modules.len();
        self.grid.insert((m.site, m.cell), idx);
        self.modules.push(Some(m));
        idx
    }

    fn site_near(&self, p: DVec3) -> Option<usize> {
        let body = crate::world::nearest(p);
        self.sites.iter().position(|s| s.body == body && (s.origin() - p).length() < SITE_RADIUS)
    }

    /// Ground position and orientation of a module.
    fn place(&self, t: &crate::terrain::Terrain, m: &Module) -> (DVec3, DQuat) {
        let s = &self.sites[m.site];
        let (e1, e2) = s.axes();
        let p = s.origin() + e1 * (m.cell[0] as f64 * CELL) + e2 * (m.cell[1] as f64 * CELL);
        let ground = crate::world::ground_point(t, p);
        let up = s.up();
        (ground, crate::player::basis(s.dir(m.facing), up))
    }
}

/// Build mode state.
#[derive(Resource)]
struct Build {
    on: bool,
    kind: ModuleKind,
    facing: u8,
    ghost: Option<Entity>,
    /// Where the ghost is: (site or new frame, cell, valid).
    spot: Option<(Option<usize>, IVec2, DVec3, DVec3, DVec3, bool)>,
}

#[derive(Resource)]
struct Looks {
    panel: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
    accent: Handle<StandardMaterial>,
    wood: Handle<StandardMaterial>,
    metal: Handle<StandardMaterial>,
    floor: Handle<StandardMaterial>,
    glow: Handle<StandardMaterial>,
    ghost_ok: Handle<StandardMaterial>,
    ghost_bad: Handle<StandardMaterial>,
    cube: Handle<Mesh>,
    item_mats: HashMap<ItemId, Handle<StandardMaterial>>,
}

#[derive(Component)]
pub struct ModuleMarker(pub usize);

#[derive(Component)]
struct HelpText;

pub struct FactoryPlugin;

impl Plugin for FactoryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Factory>()
            .insert_resource(Build { on: false, kind: ModuleKind::Smelter, facing: 0, ghost: None, spot: None })
            .add_systems(Startup, setup)
            .add_systems(Update, (build_input, ghost).chain().in_set(crate::interact::InteractSet::Reset))
            .add_systems(Update, offer.in_set(crate::interact::InteractSet::Offer))
            .add_systems(Update, (place_or_remove, use_module, simulate, sync_visuals).chain().in_set(crate::interact::InteractSet::Act));
    }
}

fn setup(mut commands: Commands, assets: Res<AssetServer>, mut materials: ResMut<Assets<StandardMaterial>>, mut meshes: ResMut<Assets<Mesh>>) {
    let mut mat = |name: &str, metallic: f32, rough: f32| {
        let albedo = assets.load(format!("textures/industrial/{name}_albedo.png"));
        let normal = assets.load_with_settings(format!("textures/industrial/{name}_normal.png"), |s: &mut ImageLoaderSettings| s.is_srgb = false);
        materials.add(StandardMaterial {
            base_color_texture: Some(albedo),
            normal_map_texture: Some(normal),
            metallic,
            perceptual_roughness: rough,
            ..default()
        })
    };
    let panel = mat("hull", 0.3, 0.5);
    let dark = mat("hull_dark", 0.5, 0.45);
    let accent = mat("accent", 0.2, 0.5);
    let metal = mat("metal", 0.9, 0.3);
    let floor = mat("floor", 0.6, 0.5);
    let wood = mat("wood", 0.0, 0.8);
    let glow = materials.add(StandardMaterial { base_color: Color::srgb(1.0, 0.5, 0.1), emissive: LinearRgba::rgb(8.0, 2.5, 0.4), ..default() });
    let ghost = |c: Color| StandardMaterial { base_color: c, alpha_mode: AlphaMode::Blend, unlit: true, ..default() };
    let ghost_ok = materials.add(ghost(Color::srgba(0.3, 1.0, 0.4, 0.35)));
    let ghost_bad = materials.add(ghost(Color::srgba(1.0, 0.3, 0.2, 0.35)));
    let cube = meshes.add(Cuboid::new(0.4, 0.4, 0.4));
    commands.insert_resource(Looks { panel, dark, accent, metal, floor, glow, wood, ghost_ok, ghost_bad, cube, item_mats: HashMap::new() });
    commands.spawn((
        HelpText,
        Text::new(""),
        TextFont { font_size: 16.0, ..default() },
        TextColor(Color::srgb(0.9, 0.9, 0.8)),
        Node { position_type: PositionType::Absolute, left: Val::Px(14.0), bottom: Val::Px(60.0), ..default() },
    ));
}

/// Parts of a module: (mesh, material, local offset, which look).
fn parts(kind: ModuleKind) -> Vec<(Mesh, usize, Vec3)> {
    // Looks: 0 panel, 1 dark, 2 accent, 3 metal, 4 floor, 5 glow.
    let cub = |x: f32, y: f32, z: f32| Mesh::from(Cuboid::new(x, y, z)).with_generated_tangents().unwrap();
    let cyl = |r: f32, h: f32| Mesh::from(Cylinder::new(r, h)).with_generated_tangents().unwrap();
    match kind {
        ModuleKind::Drill => vec![
            (cub(1.9, 0.35, 1.9), 4, Vec3::Y * 0.175),
            (cub(0.2, 2.6, 0.2), 1, Vec3::new(0.8, 1.6, 0.8)),
            (cub(0.2, 2.6, 0.2), 1, Vec3::new(-0.8, 1.6, 0.8)),
            (cub(0.2, 2.6, 0.2), 1, Vec3::new(0.8, 1.6, -0.8)),
            (cub(0.2, 2.6, 0.2), 1, Vec3::new(-0.8, 1.6, -0.8)),
            (cub(1.9, 0.25, 1.9), 2, Vec3::Y * 2.95),
            (cyl(0.32, 3.4), 3, Vec3::Y * 1.2),
            (cub(0.9, 0.9, 0.9), 0, Vec3::Y * 2.2),
            (cub(0.5, 0.5, 0.3), 1, Vec3::new(0.0, 0.6, -1.0)),
        ],
        ModuleKind::Conveyor => vec![
            (cub(1.7, 0.18, 1.95), 1, Vec3::Y * 0.12),
            (cub(1.3, 0.05, 1.95), 4, Vec3::Y * 0.235),
            (cub(0.15, 0.32, 1.95), 2, Vec3::new(0.78, 0.2, 0.0)),
            (cub(0.15, 0.32, 1.95), 2, Vec3::new(-0.78, 0.2, 0.0)),
            // Direction arrow (two bars) on the front half.
            (cub(0.12, 0.03, 0.55), 5, Vec3::new(0.18, 0.27, -0.45)),
            (cub(0.12, 0.03, 0.55), 5, Vec3::new(-0.18, 0.27, -0.45)),
        ],
        ModuleKind::Smelter => vec![
            (cub(1.9, 0.3, 1.9), 4, Vec3::Y * 0.15),
            (cub(1.7, 1.7, 1.7), 0, Vec3::Y * 1.15),
            (cub(0.8, 0.45, 0.06), 5, Vec3::new(0.0, 0.9, -0.87)),
            (cub(1.75, 0.18, 1.75), 2, Vec3::Y * 1.9),
            (cyl(0.25, 1.6), 1, Vec3::new(0.5, 2.7, 0.5)),
            (cub(0.4, 0.4, 0.2), 1, Vec3::new(0.0, 0.5, 0.95)),
        ],
        ModuleKind::TitaniumDrill | ModuleKind::PlasmaDrill => {
            let plasma = kind == ModuleKind::PlasmaDrill;
            let mut v = vec![
                (cub(1.95, 0.4, 1.95), 4, Vec3::Y * 0.2),
                (cub(0.26, 3.4, 0.26), 3, Vec3::new(0.78, 2.0, 0.78)),
                (cub(0.26, 3.4, 0.26), 3, Vec3::new(-0.78, 2.0, 0.78)),
                (cub(0.26, 3.4, 0.26), 3, Vec3::new(0.78, 2.0, -0.78)),
                (cub(0.26, 3.4, 0.26), 3, Vec3::new(-0.78, 2.0, -0.78)),
                (cub(1.95, 0.3, 1.95), 1, Vec3::Y * 3.75),
                (cyl(0.4, 4.2), if plasma { 5 } else { 3 }, Vec3::Y * 1.6),
                (cub(1.1, 1.0, 1.1), 2, Vec3::Y * 2.9),
                (cub(0.5, 0.5, 0.3), 1, Vec3::new(0.0, 0.6, -1.0)),
            ];
            if plasma {
                v.push((cyl(0.55, 0.25), 5, Vec3::Y * 0.55));
                v.push((cyl(0.2, 0.9), 5, Vec3::Y * 4.3));
            }
            v
        }
        ModuleKind::LoggingStation => vec![
            (cub(1.9, 0.3, 1.9), 4, Vec3::Y * 0.15),
            (cub(1.2, 1.1, 1.2), 0, Vec3::Y * 0.85),
            (cub(0.25, 3.6, 0.25), 3, Vec3::new(0.0, 3.0, 0.0)),
            // Crane boom with a saw head at the end.
            (cub(0.2, 0.2, 2.6), 1, Vec3::new(0.0, 4.6, -1.1)),
            (cyl(0.45, 0.08), 3, Vec3::new(0.0, 4.3, -2.3)),
            (cub(0.7, 0.35, 0.06), 5, Vec3::new(0.0, 1.0, -0.62)),
            // A few logs stacked beside it.
            (cyl(0.18, 1.6), 6, Vec3::new(0.75, 0.48, 0.55)),
            (cyl(0.18, 1.6), 6, Vec3::new(0.75, 0.48, 0.15)),
        ],
        ModuleKind::Sawmill => vec![
            (cub(1.9, 0.25, 1.9), 4, Vec3::Y * 0.125),
            (cub(1.8, 0.75, 0.9), 1, Vec3::Y * 0.6),
            (cub(1.85, 0.06, 0.95), 6, Vec3::Y * 1.0),
            (cyl(0.5, 0.04), 3, Vec3::new(0.0, 1.05, 0.0)),
            (cub(1.2, 1.0, 0.5), 0, Vec3::new(0.0, 1.5, 0.65)),
            (cub(0.6, 0.25, 0.06), 5, Vec3::new(0.0, 1.6, 0.38)),
            (cub(1.6, 0.18, 0.5), 6, Vec3::new(0.0, 0.35, -0.75)),
        ],
        ModuleKind::Storage => vec![
            (cub(1.9, 0.2, 1.9), 4, Vec3::Y * 0.1),
            (cub(1.7, 1.3, 1.7), 0, Vec3::Y * 0.85),
            (cub(1.8, 0.15, 1.8), 2, Vec3::Y * 1.55),
            (cub(0.15, 1.3, 0.15), 1, Vec3::new(0.85, 0.85, 0.85)),
            (cub(0.15, 1.3, 0.15), 1, Vec3::new(-0.85, 0.85, 0.85)),
            (cub(0.15, 1.3, 0.15), 1, Vec3::new(0.85, 0.85, -0.85)),
            (cub(0.15, 1.3, 0.15), 1, Vec3::new(-0.85, 0.85, -0.85)),
        ],
    }
}

fn look_mat(l: &Looks, k: usize) -> Handle<StandardMaterial> {
    [&l.panel, &l.dark, &l.accent, &l.metal, &l.floor, &l.glow, &l.wood][k].clone()
}

fn module_height(kind: ModuleKind) -> f64 {
    match kind {
        ModuleKind::Drill => 3.1,
        ModuleKind::Conveyor => 0.3,
        ModuleKind::Smelter => 2.0,
        ModuleKind::Storage => 1.6,
        ModuleKind::TitaniumDrill | ModuleKind::PlasmaDrill => 3.9,
        ModuleKind::LoggingStation => 1.4,
        ModuleKind::Sawmill => 1.2,
    }
}

fn spawn_module(commands: &mut Commands, meshes: &mut Assets<Mesh>, looks: &Looks, idx: usize, kind: ModuleKind, pos: DVec3, rot: DQuat) -> (Entity, Option<Entity>) {
    let e = commands
        .spawn((ModuleMarker(idx), Transform::from_rotation(rot.as_quat()), WorldPos(pos), Visibility::default()))
        .with_children(|c| {
            for (mesh, look, offset) in parts(kind) {
                c.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(look_mat(looks, look)), Transform::from_translation(offset)));
            }
        })
        .id();
    // Conveyors are low enough to walk over.
    let collider = (kind != ModuleKind::Conveyor).then(|| {
        let h = module_height(kind);
        let up = rot * DVec3::Y;
        commands.spawn((RigidBody::Static, Collider::cuboid(1.9, h, 1.9), Position(pos + up * (h * 0.5)), Rotation(rot), Transform::default())).id()
    });
    (e, collider)
}

fn build_input(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<ControlMode>,
    mut build: ResMut<Build>,
    mut commands: Commands,
    mut toasts: ResMut<Toasts>,
) {
    if *mode != ControlMode::Walk {
        build.on = false;
    } else if keys.just_pressed(KeyCode::KeyB) {
        build.on = !build.on;
        toasts.push(if build.on { "Build mode: 1-8 pick, R rotate, E place, X remove, B exit" } else { "Build mode off" }, Color::srgb(0.6, 0.85, 1.0));
    }
    BUILDING.store(build.on, Ordering::Relaxed);
    if !build.on {
        if let Some(g) = build.ghost.take() {
            commands.entity(g).despawn();
        }
        return;
    }
    let digits = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8];
    for (k, key) in digits.iter().enumerate() {
        if keys.just_pressed(*key) && build.kind != ModuleKind::ALL[k] {
            build.kind = ModuleKind::ALL[k];
            if let Some(g) = build.ghost.take() {
                commands.entity(g).despawn();
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyR) {
        build.facing = (build.facing + 1) % 4;
    }
}

/// Where the ghost goes: a few metres ahead on the ground, snapped to a nearby site's grid.
fn ghost(
    mut commands: Commands,
    view: Res<View>,
    planet: Res<Planet>,
    factory: Res<Factory>,
    mut build: ResMut<Build>,
    looks: Option<Res<Looks>>,
    player: Query<(&Player, &Inventory)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut ghosts: Query<(&mut WorldPos, &mut Transform)>,
    mut help: Query<&mut Text, With<HelpText>>,
) {
    let Some(looks) = looks else { return };
    if let Ok(mut t) = help.single_mut() {
        let s = if build.on {
            let costs: Vec<String> = build.kind.cost().iter().map(|(id, n)| format!("{n} {}", id.def().name)).collect();
            let list: Vec<String> = ModuleKind::ALL.iter().enumerate().map(|(i, k)| format!("{}{} {}", if *k == build.kind { ">" } else { " " }, i + 1, k.name())).collect();
            let what = match build.kind {
                ModuleKind::Drill => "Mines iron, copper and quartz. Place on a deposit.".to_string(),
                ModuleKind::TitaniumDrill => "Also titanium and helium-3. Place on a deposit.".to_string(),
                ModuleKind::PlasmaDrill => "Cuts anything: platinum and void crystal too.".to_string(),
                ModuleKind::LoggingStation => format!("Fells trees within {LOGGING_RADIUS:.0} m (spruce, broadleaf, redwood)."),
                ModuleKind::Sawmill => "Logs -> lumber, hardwood planks, redwood beams.".to_string(),
                ModuleKind::Smelter => "Ore -> plates, wire, silicon, titanium alloy.".to_string(),
                ModuleKind::Conveyor => "Moves items forward (arrow).".to_string(),
                ModuleKind::Storage => "Holds 500 items.".to_string(),
            };
            format!("BUILD  {}\n       {}\n{}\ncost: {}\nR rotate   E place   X remove   B exit", list[..4].join("  "), list[4..].join("  "), what, costs.join(", "))
        } else {
            String::new()
        };
        if t.0 != s {
            t.0 = s;
        }
    }
    build.spot = None;
    if !build.on {
        return;
    }
    let Some((eye, look)) = view.eye else { return };
    let Ok((p, inv)) = player.single() else { return };
    let up = crate::world::up_at(eye);
    let ahead = crate::player::tangent(look, up);
    let target = crate::world::ground_point(&planet.terrain, eye + ahead * 5.0);
    let (site, cell, pos, e1, e2) = match factory.site_near(target) {
        Some(si) => {
            let s = &factory.sites[si];
            let cell = s.cell_of(target);
            let (e1, e2) = s.axes();
            let p = s.origin() + e1 * (cell.x as f64 * CELL) + e2 * (cell.y as f64 * CELL);
            (Some(si), cell, crate::world::ground_point(&planet.terrain, p), e1, e2)
        }
        None => {
            let e2 = crate::player::tangent(p.heading, up);
            let e1 = e2.cross(up);
            (None, IVec2::ZERO, target, e1, e2)
        }
    };
    let free = site.is_none_or(|si| !factory.grid.contains_key(&(si, [cell.x, cell.y])));
    let affordable = build.kind.cost().iter().all(|(id, n)| inv.count(*id) >= *n);
    let on_deposit = match build.kind.drill_tier() {
        Some(tier) => deposits_near(&planet.terrain, pos, 3.0).iter().any(|d| crate::mining::ore_tier(d.ore) <= tier),
        None => true,
    };
    let has_trees = build.kind != ModuleKind::LoggingStation || !crate::flora::trees_near(&planet.flora, pos, LOGGING_RADIUS).is_empty();
    let valid = free && affordable && on_deposit && has_trees;
    build.spot = Some((site, cell, pos, e1, e2, valid));
    let dir = [e2, e1, -e2, -e1][build.facing as usize];
    let rot = crate::player::basis(dir, up);
    match build.ghost {
        Some(g) => {
            if let Ok((mut wp, mut t)) = ghosts.get_mut(g) {
                wp.0 = pos;
                t.rotation = rot.as_quat();
            }
            commands.entity(g).insert_recursive::<Children>(MeshMaterial3d(if valid { looks.ghost_ok.clone() } else { looks.ghost_bad.clone() }));
        }
        None => {
            let mat = if valid { looks.ghost_ok.clone() } else { looks.ghost_bad.clone() };
            let g = commands
                .spawn((Transform::from_rotation(rot.as_quat()), WorldPos(pos), Visibility::default()))
                .with_children(|c| {
                    for (mesh, _, offset) in parts(build.kind) {
                        c.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(mat.clone()), Transform::from_translation(offset), bevy::pbr::NotShadowCaster));
                    }
                })
                .id();
            build.ghost = Some(g);
        }
    }
}

fn place_or_remove(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    interact: Res<Interact>,
    build: Res<Build>,
    planet: Res<Planet>,
    mining: Res<Mining>,
    mut factory: ResMut<Factory>,
    mut player: Query<&mut Inventory, With<Player>>,
    mut toasts: ResMut<Toasts>,
    looks: Option<Res<Looks>>,
    mut meshes: ResMut<Assets<Mesh>>,
    view: Res<View>,
    markers: Query<(&ModuleMarker, &WorldPos)>,
) {
    let Some(looks) = looks else { return };
    if !build.on {
        return;
    }
    let Ok(mut inv) = player.single_mut() else { return };
    // X: take down the module you are looking at.
    if keys.just_pressed(KeyCode::KeyX) {
        let best = markers
            .iter()
            .filter_map(|(m, wp)| aim(&view, wp.0 + crate::world::up_at(wp.0) * 0.8, 6.0).map(|s| (s, m.0)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, idx)) = best {
            if let Some(m) = factory.modules[idx].take() {
                for (id, n) in m.kind.cost() {
                    inv.add(*id, *n);
                }
                for (id, n) in m.stock.iter().chain(m.output.iter()) {
                    inv.add(*id, *n);
                }
                if let Some((id, _)) = m.carried {
                    inv.add(id, 1);
                }
                factory.grid.remove(&(m.site, m.cell));
                if let Some((e, c)) = factory.entities.remove(&idx) {
                    commands.entity(e).despawn();
                    if let Some(c) = c {
                        commands.entity(c).despawn();
                    }
                }
                if let Some(e) = factory.cubes.remove(&idx) {
                    commands.entity(e).despawn();
                }
                toasts.push(format!("Removed {} (refunded)", m.kind.name()), Color::srgb(0.6, 0.85, 1.0));
            }
        }
        return;
    }
    if !interact.pressed {
        return;
    }
    let Some((site, cell, pos, e1, e2, valid)) = build.spot else { return };
    if !valid {
        let missing: Vec<String> = build.kind.cost().iter().filter(|(id, n)| inv.count(*id) < *n).map(|(id, n)| format!("{} {}", n - inv.count(*id).min(*n), id.def().name)).collect();
        let why = if !missing.is_empty() {
            format!("Need {}", missing.join(", "))
        } else if build.kind.drill_tier().is_some() {
            match deposits_near(&planet.terrain, pos, 3.0).first() {
                Some(d) => format!("{} is too hard for this drill: needs {}", d.ore.def().name, crate::mining::tier_tool(crate::mining::ore_tier(d.ore))),
                None => "A drill must stand on an ore deposit".to_string(),
            }
        } else if build.kind == ModuleKind::LoggingStation {
            format!("No trees within {LOGGING_RADIUS:.0} m")
        } else {
            "That spot is taken".to_string()
        };
        toasts.push(why, Color::srgb(1.0, 0.45, 0.3));
        return;
    }
    for (id, n) in build.kind.cost() {
        for _ in 0..*n {
            inv.take_one(*id);
        }
    }
    let site = site.unwrap_or_else(|| {
        factory.sites.push(Site { body: crate::world::nearest(pos), origin: pos.to_array(), e1: e1.to_array(), e2: e2.to_array() });
        factory.sites.len() - 1
    });
    let deposit = build
        .kind
        .drill_tier()
        .and_then(|tier| deposits_near(&planet.terrain, pos, 3.0).into_iter().find(|d| mining.remaining(d) > 0 && crate::mining::ore_tier(d.ore) <= tier).map(|d| d.cell));
    let m = Module { kind: build.kind, site, cell: [cell.x, cell.y], facing: build.facing, stock: Vec::new(), output: Vec::new(), progress: 0.0, carried: None, deposit, felled: 0 };
    let idx = factory.modules.len();
    let (gpos, rot) = factory_place(&factory, &planet, &m);
    let ents = spawn_module(&mut commands, &mut meshes, &looks, idx, m.kind, gpos, rot);
    factory.grid.insert((site, m.cell), idx);
    factory.entities.insert(idx, ents);
    factory.modules.push(Some(m));
    toasts.push(format!("Built {}", build.kind.name()), Color::srgb(0.6, 0.85, 1.0));
}

fn factory_place(factory: &Factory, planet: &Planet, m: &Module) -> (DVec3, DQuat) {
    factory.place(&planet.terrain, m)
}

/// Offer modules to the interaction focus (outside build mode).
fn offer(view: Res<View>, build: Res<Build>, factory: Res<Factory>, mining: Res<Mining>, planet: Res<Planet>, markers: Query<(Entity, &ModuleMarker, &WorldPos)>, mut focus: ResMut<Focus>) {
    if build.on {
        return;
    }
    for (e, m, wp) in &markers {
        let Some(Some(module)) = factory.modules.get(m.0) else { continue };
        let Some(score) = aim(&view, wp.0 + crate::world::up_at(wp.0) * 0.9, 4.0) else { continue };
        let out = count(&module.output) + count(&module.stock);
        let prompt = match module.kind {
            ModuleKind::Drill | ModuleKind::TitaniumDrill | ModuleKind::PlasmaDrill => {
                let left = module.deposit.and_then(|c| deposit_by_cell(&planet, wp.0, c)).map(|d| mining.remaining(&d));
                match left {
                    Some(l) if l > 0 => format!("Drill: {} left  ({} ready)  E: collect", l, count(&module.output)),
                    _ => format!("Drill: no ore under it  ({} ready)  E: collect", count(&module.output)),
                }
            }
            ModuleKind::Smelter => format!("Smelter: {} queued, {} done   E: collect / load ore", count(&module.stock), count(&module.output)),
            ModuleKind::Sawmill => format!("Sawmill: {} logs queued, {} done   E: collect / load logs", count(&module.stock), count(&module.output)),
            ModuleKind::LoggingStation => format!("Logging station: {} trees felled  ({} logs ready)  E: collect", module.felled, count(&module.output)),
            ModuleKind::Storage => format!("Storage: {out} items   E: take all"),
            ModuleKind::Conveyor => format!("Conveyor{}", module.carried.map_or(String::new(), |c| format!(": {}", c.0.def().name))),
        };
        focus.offer(FocusKind::Module(e), score, prompt);
    }
}

fn deposit_by_cell(planet: &Planet, near: DVec3, cell: u64) -> Option<Deposit> {
    deposits_near(&planet.terrain, near, 4.0).into_iter().find(|d| d.cell == cell)
}

/// E on a module: collect its output, or load ore into a smelter.
fn use_module(
    interact: Res<Interact>,
    focus: Res<Focus>,
    build: Res<Build>,
    markers: Query<&ModuleMarker>,
    mut factory: ResMut<Factory>,
    mut player: Query<&mut Inventory, With<Player>>,
    mut toasts: ResMut<Toasts>,
) {
    if build.on || !interact.pressed {
        return;
    }
    let Some(f) = &focus.0 else { return };
    let FocusKind::Module(e) = f.kind else { return };
    let Ok(marker) = markers.get(e) else { return };
    let Ok(mut inv) = player.single_mut() else { return };
    let Some(Some(m)) = factory.modules.get_mut(marker.0) else { return };
    let mut got = 0;
    let mut collect = |v: &mut Vec<(ItemId, u32)>, inv: &mut Inventory| {
        for s in v.iter_mut() {
            let n = inv.add(s.0, s.1);
            s.1 -= n;
            got += n;
        }
        v.retain(|s| s.1 > 0);
    };
    collect(&mut m.output, &mut inv);
    if m.kind == ModuleKind::Storage {
        collect(&mut m.stock, &mut inv);
    }
    if got > 0 {
        toasts.push(format!("Collected {got} items"), Color::srgb(0.8, 0.95, 0.8));
    } else if m.kind.processor() {
        // Load processable items from the inventory.
        let kind = m.kind;
        let ore = inv.slots.iter().flatten().map(|s| s.0).find(|id| recipe(kind, *id).is_some());
        if let Some(id) = ore {
            let n = inv.count(id).min(20 - count(&m.stock).min(20));
            for _ in 0..n {
                inv.take_one(id);
            }
            put(&mut m.stock, id, n);
            toasts.push(format!("Loaded {n} {}", id.def().name), Color::srgb(0.8, 0.95, 0.8));
        } else {
            toasts.push(if kind == ModuleKind::Sawmill { "No logs to saw" } else { "No ore to smelt" }, Color::srgb(1.0, 0.45, 0.3));
        }
    }
}

/// Run every module: mine, smelt, move items along.
fn simulate(time: Res<Time>, mut planet: ResMut<Planet>, mut factory: ResMut<Factory>, mut mining: ResMut<Mining>, mut regrowth: ResMut<crate::forage::Regrowth>) {
    let now = time.elapsed_secs_f64();
    let dt = time.delta_secs();
    let n = factory.modules.len();
    for i in 0..n {
        let Some(mut m) = factory.modules[i].take() else { continue };
        match m.kind {
            ModuleKind::Drill | ModuleKind::TitaniumDrill | ModuleKind::PlasmaDrill => {
                if count(&m.output) < 10 {
                    if let Some(cell) = m.deposit {
                        let (pos, _) = factory.place(&planet.terrain, &m);
                        if let Some(d) = deposit_by_cell(&planet, pos, cell) {
                            // Bigger drills cut faster.
                            let speed = [1.0, 1.0, 1.5, 2.2][m.kind.drill_tier().unwrap_or(1) as usize];
                            let per = 3.0 * (1.0 + 0.6 * d.ore.def().rarity as u8 as f32) / speed;
                            m.progress += dt;
                            if m.progress >= per {
                                m.progress = 0.0;
                                if mining.take(&d) {
                                    put(&mut m.output, d.ore, 1);
                                }
                            }
                        }
                    }
                }
            }
            ModuleKind::Smelter | ModuleKind::Sawmill => {
                if count(&m.output) < 20 {
                    if let Some(&(input, _)) = m.stock.first() {
                        if let Some((out, n, secs)) = recipe(m.kind, input) {
                            m.progress += dt;
                            if m.progress >= secs {
                                m.progress = 0.0;
                                take_one(&mut m.stock);
                                put(&mut m.output, out, n);
                            }
                        }
                    }
                }
            }
            ModuleKind::LoggingStation => {
                if count(&m.output) < 20 {
                    m.progress += dt;
                    if m.progress >= FELL_SECONDS {
                        m.progress = 0.0;
                        let (pos, _) = factory.place(&planet.terrain, &m);
                        // Fell the nearest standing tree in reach: logs by species, more from big trees.
                        let trees = crate::flora::trees_near(&planet.flora, pos, LOGGING_RADIUS);
                        let nearest = trees.into_iter().min_by(|a, b| (a.base - pos).length_squared().total_cmp(&(b.base - pos).length_squared()));
                        if let Some(t) = nearest {
                            if let Some(timber) = planet.flora.bio.flora[t.species].timber.clone() {
                                let logs = ((timber.logs as f32 * t.scale).round() as u32).max(1);
                                put(&mut m.output, timber.log, logs);
                                m.felled += 1;
                                planet.flora.harvested.write().unwrap().insert(t.cell);
                                regrowth.0.push((t.cell, t.base, now + TREE_REGROW));
                                planet.rebuild_at(t.base);
                            }
                        }
                    }
                }
            }
            ModuleKind::Conveyor => {
                if let Some((id, p)) = m.carried {
                    let p = (p + dt).min(1.0);
                    m.carried = Some((id, p));
                    if p >= 1.0 {
                        let front = IVec2::from(m.cell) + step(m.facing);
                        if let Some(&j) = factory.grid.get(&(m.site, [front.x, front.y])) {
                            if let Some(Some(next)) = factory.modules.get_mut(j) {
                                if next.accepts(id) {
                                    next.accept(id);
                                    m.carried = None;
                                }
                            }
                        }
                    }
                }
            }
            ModuleKind::Storage => {}
        }
        // Hand finished items to the module in front.
        if m.kind != ModuleKind::Conveyor && m.kind != ModuleKind::Storage {
            if let Some(&(id, _)) = m.output.first() {
                let front = IVec2::from(m.cell) + step(m.facing);
                if let Some(&j) = factory.grid.get(&(m.site, [front.x, front.y])) {
                    if let Some(Some(next)) = factory.modules.get_mut(j) {
                        if next.accepts(id) {
                            next.accept(id);
                            take_one(&mut m.output);
                        }
                    }
                }
            }
        }
        factory.modules[i] = Some(m);
    }
}

/// Spawn visuals for modules that have none (after loading), and move item cubes along belts.
#[allow(clippy::too_many_arguments)]
fn sync_visuals(
    mut commands: Commands,
    planet: Res<Planet>,
    mut factory: ResMut<Factory>,
    looks: Option<ResMut<Looks>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cubes: Query<&mut WorldPos, Without<ModuleMarker>>,
) {
    let Some(mut looks) = looks else { return };
    let n = factory.modules.len();
    for i in 0..n {
        let Some(m) = factory.modules[i].clone() else { continue };
        if !factory.entities.contains_key(&i) {
            let (pos, rot) = factory.place(&planet.terrain, &m);
            let ents = spawn_module(&mut commands, &mut meshes, &looks, i, m.kind, pos, rot);
            factory.entities.insert(i, ents);
        }
        if m.kind != ModuleKind::Conveyor {
            continue;
        }
        match m.carried {
            Some((id, p)) => {
                let (pos, _) = factory.place(&planet.terrain, &m);
                let s = &factory.sites[m.site];
                let at = pos + s.up() * 0.45 + s.dir(m.facing) * (CELL * (p as f64 - 0.5));
                match factory.cubes.get(&i).copied() {
                    Some(e) => {
                        if let Ok(mut wp) = cubes.get_mut(e) {
                            wp.0 = at;
                        }
                    }
                    None => {
                        let color = id.def().rarity.color();
                        let mat = looks.item_mats.entry(id).or_insert_with(|| materials.add(StandardMaterial { base_color: item_color(id).unwrap_or(color), perceptual_roughness: 0.6, ..default() })).clone();
                        let e = commands.spawn((Mesh3d(looks.cube.clone()), MeshMaterial3d(mat), Transform::default(), WorldPos(at))).id();
                        factory.cubes.insert(i, e);
                    }
                }
            }
            None => {
                if let Some(e) = factory.cubes.remove(&i) {
                    commands.entity(e).despawn();
                }
            }
        }
    }
}

fn item_color(id: ItemId) -> Option<Color> {
    Some(match id {
        item::IRON_ORE => Color::srgb(0.45, 0.15, 0.08),
        item::COPPER_ORE => Color::srgb(0.15, 0.45, 0.35),
        item::QUARTZ => Color::srgb(0.9, 0.85, 0.9),
        item::TITANIUM_ORE => Color::srgb(0.5, 0.55, 0.65),
        item::IRON_PLATE => Color::srgb(0.6, 0.6, 0.62),
        item::COPPER_WIRE => Color::srgb(0.8, 0.45, 0.2),
        item::SILICON => Color::srgb(0.25, 0.25, 0.35),
        item::TITANIUM_ALLOY => Color::srgb(0.75, 0.8, 0.9),
        item::SOFTWOOD_LOG | item::HARDWOOD_LOG => Color::srgb(0.45, 0.32, 0.2),
        item::REDWOOD_LOG => Color::srgb(0.55, 0.25, 0.15),
        item::LUMBER => Color::srgb(0.85, 0.72, 0.5),
        item::HARDWOOD_PLANKS => Color::srgb(0.6, 0.4, 0.25),
        item::REDWOOD_BEAMS => Color::srgb(0.7, 0.35, 0.22),
        _ => return None,
    })
}
