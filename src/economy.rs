//! Bases and the economy.
//!
//! Two bases trade in the home system: **Haven Outpost** on the planet and **Selene Station** on
//! the moon. Each runs a market for every item: a stock level that its own production raises
//! and its consumption lowers, and a price set by supply and demand:
//!
//!   price = base value x (target stock / stock)^0.7      (clamped to 0.3x .. 4x)
//!   d(stock)/dt = production - consumption x sqrt(stock / target)
//!
//! so stocks settle where production meets consumption: a base that makes something has a lot
//! of it and sells it cheap, a base that needs something pays well for it. Selling raises the
//! stock and lowers the price, buying does the reverse, and both recover over time. Bases
//! specialise (Haven grows food and wants fuel and parts; Selene mines titanium, iron and
//! helium-3 but must import food), so carrying goods between them pays. Now and then a
//! shortage or a glut hits one market. The base buys at 90% and sells at 110% of its price.
//!
//! Game time: one real minute is one game hour.

use std::sync::OnceLock;

use avian3d::prelude::*;
use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::forage::Toasts;
use crate::interact::{Focus, FocusKind, Interact, View, aim};
use crate::items::{ITEMS, Inventory, ItemId};
use crate::planet::{Planet, WorldPos};
use crate::player::Player;
use crate::terrain::{RADIUS, Terrain};
use crate::world::{Body, MOON_CENTER, MOON_RADIUS, moon, moon_station_dir};

/// Game hours per real second.
pub const HOURS_PER_SEC: f64 = 1.0 / 60.0;

pub const BASE_NAMES: [&str; 2] = ["Haven Outpost", "Selene Station"];

static HAVEN: OnceLock<DVec3> = OnceLock::new();

/// Find Haven Outpost's site: temperate, sunlit, gentle lowland near water, and level a pad
/// there. Call once on the terrain before it is shared.
pub fn found_haven(t: &mut Terrain) -> DVec3 {
    let sun = crate::sky::sun_dir().as_dvec3();
    let n = 60_000;
    let mut best = (f64::MIN, DVec3::Y);
    for i in 0..n {
        let y = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
        let r = (1.0 - y * y).sqrt();
        let phi = i as f64 * 2.399_963;
        let d = DVec3::new(r * phi.cos(), y, r * phi.sin());
        let light = d.dot(sun);
        if light < 0.55 {
            continue;
        }
        let h = t.height(d);
        if !(20.0..250.0).contains(&h) || t.water_level(d).is_some() {
            continue;
        }
        let side = d.any_orthonormal_vector();
        let relief: f64 = [side, d.cross(side), -side, -d.cross(side)].iter().map(|o| (t.height((d + *o * (80.0 / RADIUS)).normalize()) - h).abs()).sum();
        let e = t.ecology(d);
        let temperate = 1.0 - ((e.temperature - 15.0).abs() / 10.0).min(1.0) as f64;
        let wet = 1.0 - ((e.moisture - 0.5).abs() * 2.0).min(1.0) as f64;
        let score = light + temperate + wet * 0.5 + e.coast as f64 * 0.5 - relief / 40.0;
        if score > best.0 {
            best = (score, d);
        }
    }
    let d = best.1;
    t.add_pad(d, 60.0);
    let _ = HAVEN.set(d);
    d
}

/// Ground position of a base's centre, and its body.
pub fn base_site(t: &Terrain, base: usize) -> (Body, DVec3) {
    match base {
        0 => {
            let d = *HAVEN.get().expect("Haven not founded");
            (Body::Planet, d * (RADIUS + t.height(d)))
        }
        _ => {
            let d = moon_station_dir();
            (Body::Moon, MOON_CENTER + d * (MOON_RADIUS + moon().height(d)))
        }
    }
}

/// Local frame of a base: (up, forward, right).
fn base_frame(t: &Terrain, base: usize) -> (DVec3, DVec3, DVec3) {
    let (_, p) = base_site(t, base);
    let up = crate::world::up_at(p);
    let fwd = crate::player::tangent(DVec3::Y, up);
    (up, fwd, fwd.cross(up))
}

/// Where the trade terminal stands at a base.
pub fn terminal_pos(t: &Terrain, base: usize) -> DVec3 {
    let (_, p) = base_site(t, base);
    let (_, fwd, right) = base_frame(t, base);
    crate::world::ground_point(t, p - fwd * 6.0 - right * 8.0)
}

/// A good spot to start or land near a base (on the pad, clear of the buildings).
pub fn landing_spot(t: &Terrain, base: usize) -> DVec3 {
    let (_, p) = base_site(t, base);
    let (_, fwd, _) = base_frame(t, base);
    crate::world::ground_point(t, p - fwd * 22.0)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MarketItem {
    pub stock: f64,
    pub target: f64,
    /// Per game hour.
    pub produce: f64,
    pub consume: f64,
    /// Price an hour ago (for the trend arrow).
    #[serde(default)]
    pub last_price: f64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MarketEvent {
    pub base: usize,
    pub item: ItemId,
    /// Multiplier on consumption (shortage) or production (glut).
    pub shortage: bool,
    pub hours_left: f64,
}

#[derive(Resource, Serialize, Deserialize)]
pub struct Economy {
    /// [base][item]
    pub markets: Vec<Vec<MarketItem>>,
    pub events: Vec<MarketEvent>,
    pub credits: i64,
    pub hour: f64,
    pub news: Vec<String>,
}

/// How a base relates to an item: producer, balanced, consumer...
#[derive(Clone, Copy)]
enum Role {
    Producer,
    Balanced,
    MildConsumer,
    Consumer,
}

fn role(base: usize, id: ItemId) -> Role {
    use crate::items::item::*;
    use Role::*;
    let food = [BERRIES, APPLE, MUSHROOM, CACTUS_FRUIT];
    match base {
        0 => match id {
            x if food.contains(&x) => Producer,
            CHANTERELLE | TRUFFLE | HERBS | TITANIUM_ORE | PLATINUM_ORE | VOID_CRYSTAL | IRON_PLATE | COPPER_WIRE => MildConsumer,
            HELIUM3 | SILICON | TITANIUM_ALLOY | REDWOOD_BEAMS => Consumer,
            HARDWOOD_PLANKS | REDWOOD_LOG => MildConsumer,
            _ => Balanced,
        },
        _ => match id {
            x if food.contains(&x) => Consumer,
            CHANTERELLE | TRUFFLE | HERBS | IRON_PLATE | COPPER_WIRE => Consumer,
            IRON_ORE | TITANIUM_ORE | HELIUM3 => Producer,
            COPPER_ORE | QUARTZ | SILICON => MildConsumer,
            // No trees on the moon: everything wooden is imported.
            SOFTWOOD_LOG | HARDWOOD_LOG | REDWOOD_LOG | LUMBER | HARDWOOD_PLANKS | REDWOOD_BEAMS => Consumer,
            _ => Balanced,
        },
    }
}

fn price_of(id: ItemId, m: &MarketItem) -> f64 {
    id.def().value as f64 * (m.target / m.stock.max(0.5)).powf(0.7).clamp(0.3, 4.0)
}

impl Economy {
    pub fn new() -> Self {
        let markets = (0..2)
            .map(|b| {
                ITEMS
                    .iter()
                    .enumerate()
                    .map(|(i, d)| {
                        let id = ItemId(i as u16);
                        let target = (2000.0 / (d.value as f64).sqrt()).round();
                        let (p, c) = match role(b, id) {
                            Role::Producer => (0.3, 0.15),
                            Role::Balanced => (0.2, 0.2),
                            Role::MildConsumer => (0.15, 0.2),
                            Role::Consumer => (0.1, 0.25),
                        };
                        // Start at equilibrium: production = consumption x sqrt(stock / target).
                        let stock = target * (p / c) * (p / c);
                        let mut m = MarketItem { stock, target, produce: p * target, consume: c * target, last_price: 0.0 };
                        m.last_price = price_of(id, &m);
                        m
                    })
                    .collect()
            })
            .collect();
        Self { markets, events: Vec::new(), credits: 150, hour: 0.0, news: vec!["Welcome to the home system. Sell what you gather; buy low, sell high.".into()] }
    }

    pub fn price(&self, base: usize, id: ItemId) -> f64 {
        price_of(id, &self.markets[base][id.0 as usize])
    }

    /// What the base pays you for one.
    pub fn sell_price(&self, base: usize, id: ItemId) -> i64 {
        (self.price(base, id) * 0.9).floor().max(1.0) as i64
    }

    /// What one costs you.
    pub fn buy_price(&self, base: usize, id: ItemId) -> i64 {
        (self.price(base, id) * 1.1).ceil() as i64
    }

    /// Advance the markets by `hours`.
    pub fn tick(&mut self, hours: f64) {
        let before = self.hour;
        self.hour += hours;
        for (b, market) in self.markets.iter_mut().enumerate() {
            for (i, m) in market.iter_mut().enumerate() {
                let mut p = m.produce;
                let mut c = m.consume;
                for e in self.events.iter().filter(|e| e.base == b && e.item.0 as usize == i) {
                    if e.shortage { c *= 3.0 } else { p *= 3.0 }
                }
                m.stock = (m.stock + (p - c * (m.stock / m.target).max(0.0).sqrt()) * hours).max(0.0);
            }
        }
        // Once per game hour: remember prices (trends), age events, maybe start one.
        if before.floor() != self.hour.floor() {
            for b in 0..self.markets.len() {
                for i in 0..ITEMS.len() {
                    let id = ItemId(i as u16);
                    let p = self.price(b, id);
                    self.markets[b][i].last_price = p;
                }
            }
            self.events.iter_mut().for_each(|e| e.hours_left -= 1.0);
            self.events.retain(|e| e.hours_left > 0.0);
            let h = self.hour.floor() as u64;
            let roll = |k: u64| {
                let mut x = h.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ k.wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ crate::planet::SEED;
                x = (x ^ (x >> 31)).wrapping_mul(0x94D0_49BB_1331_11EB);
                (x >> 11) as f64 / (1u64 << 53) as f64
            };
            if roll(1) < 0.09 {
                let base = (roll(2) * 2.0) as usize % 2;
                let item = ItemId((roll(3) * ITEMS.len() as f64) as u16 % ITEMS.len() as u16);
                let shortage = roll(4) < 0.6;
                self.events.push(MarketEvent { base, item, shortage, hours_left: 6.0 + roll(5) * 10.0 });
                let what = if shortage { "Shortage" } else { "Glut" };
                self.news.push(format!("{what} of {} at {}", item.def().name, BASE_NAMES[base]));
                if self.news.len() > 4 {
                    self.news.remove(0);
                }
            }
        }
    }
}

impl Default for Economy {
    fn default() -> Self {
        Self::new()
    }
}

/// Which base's trade screen is open, and the selected row.
#[derive(Resource, Default)]
pub struct Trade {
    pub open: Option<usize>,
    pub row: usize,
}

#[derive(Component)]
struct TradePanel;
#[derive(Component)]
struct CreditsText;
#[derive(Component)]
struct NavText;

pub struct EconomyPlugin;

impl Plugin for EconomyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Economy>()
            .init_resource::<Trade>()
            .add_systems(PostStartup, (spawn_bases, spawn_ui))
            .add_systems(Update, offer.in_set(crate::interact::InteractSet::Offer))
            .add_systems(Update, (tick, trade_input, trade_ui, nav_ui).chain().in_set(crate::interact::InteractSet::Act));
    }
}

fn tick(time: Res<Time>, mut eco: ResMut<Economy>) {
    eco.tick(time.delta_secs_f64() * HOURS_PER_SEC);
}

fn spawn_bases(
    mut commands: Commands,
    planet: Res<Planet>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let t = &planet.terrain;
    let tex = |m: &mut Assets<StandardMaterial>, name: &str, metallic: f32| {
        let normal = assets.load_with_settings(format!("textures/industrial/{name}_normal.png"), |s: &mut bevy::image::ImageLoaderSettings| s.is_srgb = false);
        m.add(StandardMaterial { base_color_texture: Some(assets.load(format!("textures/industrial/{name}_albedo.png"))), normal_map_texture: Some(normal), metallic, perceptual_roughness: 0.5, ..default() })
    };
    let panel = tex(&mut materials, "hull", 0.3);
    let dark = tex(&mut materials, "hull_dark", 0.5);
    let accent = tex(&mut materials, "accent", 0.2);
    let floor = tex(&mut materials, "floor", 0.5);
    let screen = materials.add(StandardMaterial { base_color: Color::srgb(0.05, 0.2, 0.25), emissive: LinearRgba::rgb(0.4, 2.5, 3.5), ..default() });
    let beacon = materials.add(StandardMaterial { base_color: Color::srgb(0.3, 0.8, 1.0), emissive: LinearRgba::rgb(3.0, 9.0, 14.0), unlit: true, ..default() });
    let window = materials.add(StandardMaterial { base_color: Color::srgb(1.0, 0.85, 0.5), emissive: LinearRgba::rgb(6.0, 4.5, 2.0), ..default() });
    for base in 0..2 {
        let (_, centre) = base_site(t, base);
        let (up, fwd, right) = base_frame(t, base);
        let rot = crate::player::basis(fwd, up);
        let at = |x: f64, z: f64| crate::world::ground_point(t, centre + right * x + fwd * z);
        let mut part = |commands: &mut Commands, mesh: Mesh, mat: &Handle<StandardMaterial>, pos: DVec3, local_rot: Quat, scale: Vec3, collide: Option<DVec3>| {
            let m = meshes.add(mesh.with_generated_tangents().unwrap_or_else(|_| Mesh::from(Cuboid::default())));
            commands.spawn((Mesh3d(m), MeshMaterial3d(mat.clone()), Transform { rotation: rot.as_quat() * local_rot, scale, ..default() }, WorldPos(pos)));
            if let Some(half) = collide {
                let r = rot * local_rot.as_dquat();
                commands.spawn((RigidBody::Static, Collider::cuboid(half.x * 2.0, half.y * 2.0, half.z * 2.0), Position(pos + r * DVec3::new(0.0, half.y, 0.0)), Rotation(r), Transform::default()));
            }
        };
        // Landing pad.
        part(&mut commands, Mesh::from(Cylinder::new(16.0, 0.3)), &floor, at(0.0, -22.0) + up * 0.05, Quat::IDENTITY, Vec3::ONE, None);
        part(&mut commands, Mesh::from(Torus::new(15.0, 15.8)), &accent, at(0.0, -22.0) + up * 0.22, Quat::IDENTITY, Vec3::new(1.0, 0.15, 1.0), None);
        // Habitat domes and a hangar.
        for (x, z, r) in [(-14.0, 6.0, 7.0), (2.0, 16.0, 5.5)] {
            part(&mut commands, Mesh::from(Sphere::new(r as f32).mesh().uv(32, 16)), &panel, at(x, z) - up * 0.5, Quat::IDENTITY, Vec3::new(1.0, 0.65, 1.0), Some(DVec3::new(r * 0.7, r * 0.6, r * 0.7)));
            part(&mut commands, Mesh::from(Cylinder::new((r * 1.02) as f32, 0.8)), &dark, at(x, z) + up * 0.2, Quat::IDENTITY, Vec3::ONE, None);
            part(&mut commands, Mesh::from(Cuboid::new(2.2, 1.0, 0.1)), &window, at(x, z) + up * (r * 0.35) + fwd * (-r * 0.85), Quat::IDENTITY, Vec3::ONE, None);
        }
        part(&mut commands, Mesh::from(Cuboid::new(14.0, 7.0, 12.0)), &panel, at(16.0, 4.0) + up * 3.5, Quat::IDENTITY, Vec3::ONE, Some(DVec3::new(7.0, 3.5, 6.0)));
        part(&mut commands, Mesh::from(Cuboid::new(14.4, 0.6, 12.4)), &accent, at(16.0, 4.0) + up * 7.1, Quat::IDENTITY, Vec3::ONE, None);
        part(&mut commands, Mesh::from(Cuboid::new(9.0, 5.5, 0.2)), &dark, at(16.0, -2.1) + up * 2.75, Quat::IDENTITY, Vec3::ONE, None);
        // Comms mast and beacon (visible from far away).
        part(&mut commands, Mesh::from(Cylinder::new(0.4, 30.0)), &dark, at(-4.0, 22.0) + up * 15.0, Quat::IDENTITY, Vec3::ONE, Some(DVec3::new(0.5, 15.0, 0.5)));
        part(&mut commands, Mesh::from(Cylinder::new(1.2, 400.0)), &beacon, at(-4.0, 22.0) + up * 230.0, Quat::IDENTITY, Vec3::ONE, None);
        // Trade terminal.
        let tp = terminal_pos(t, base);
        part(&mut commands, Mesh::from(Cuboid::new(1.2, 1.6, 0.8)), &dark, tp + up * 0.8, Quat::IDENTITY, Vec3::ONE, Some(DVec3::new(0.6, 0.9, 0.4)));
        part(&mut commands, Mesh::from(Cuboid::new(1.0, 0.7, 0.06)), &screen, tp + up * 1.35 - fwd * 0.42, Quat::from_rotation_x(-0.3), Vec3::ONE, None);
        part(&mut commands, Mesh::from(Cuboid::new(1.3, 0.12, 0.9)), &accent, tp + up * 1.65, Quat::IDENTITY, Vec3::ONE, None);
        // Lights.
        for (x, z) in [(0.0, -22.0), (-14.0, 6.0), (16.0, -4.0), (6.0, 14.0)] {
            commands.spawn((PointLight { intensity: 400_000.0, range: 40.0, color: Color::srgb(1.0, 0.9, 0.75), shadows_enabled: false, ..default() }, Transform::default(), WorldPos(at(x, z) + up * 6.0)));
        }
        let _ = DQuat::IDENTITY;
    }
}

fn spawn_ui(mut commands: Commands) {
    commands.spawn((
        TradePanel,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(22.0),
            top: Val::Px(70.0),
            padding: UiRect::all(Val::Px(12.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            min_width: Val::Px(760.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 0.88)),
        Visibility::Hidden,
    ));
    commands.spawn((
        CreditsText,
        Text::new(""),
        TextFont { font_size: 17.0, ..default() },
        TextColor(Color::srgb(1.0, 0.85, 0.4)),
        Node { position_type: PositionType::Absolute, left: Val::Px(14.0), bottom: Val::Px(40.0), ..default() },
    ));
    commands.spawn((
        NavText,
        Text::new(""),
        TextFont { font_size: 14.0, ..default() },
        TextColor(Color::srgb(0.6, 0.85, 1.0)),
        Node { position_type: PositionType::Absolute, right: Val::Px(16.0), bottom: Val::Px(14.0), ..default() },
    ));
}

fn offer(view: Res<View>, planet: Res<Planet>, mut focus: ResMut<Focus>) {
    for base in 0..2 {
        let tp = terminal_pos(&planet.terrain, base);
        if let Some(score) = aim(&view, tp + crate::world::up_at(tp) * 1.3, 3.0) {
            focus.offer(FocusKind::Terminal(base), score, format!("E: trade at {}", BASE_NAMES[base]));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn trade_input(
    keys: Res<ButtonInput<KeyCode>>,
    interact: Res<Interact>,
    focus: Res<Focus>,
    mut trade: ResMut<Trade>,
    mut eco: ResMut<Economy>,
    mut player: Query<(&mut Inventory, &avian3d::prelude::Position), With<Player>>,
    planet: Res<Planet>,
    mut toasts: ResMut<Toasts>,
) {
    let Ok((mut inv, pos)) = player.single_mut() else { return };
    match trade.open {
        None => {
            if interact.pressed {
                if let Some(f) = &focus.0 {
                    if let FocusKind::Terminal(b) = f.kind {
                        trade.open = Some(b);
                    }
                }
            }
        }
        Some(base) => {
            // Walking away closes it.
            if keys.just_pressed(KeyCode::KeyE) || keys.just_pressed(KeyCode::Escape) || (pos.0 - terminal_pos(&planet.terrain, base)).length() > 6.0 {
                trade.open = None;
                return;
            }
            let n = ITEMS.len();
            if keys.just_pressed(KeyCode::ArrowDown) {
                trade.row = (trade.row + 1) % n;
            }
            if keys.just_pressed(KeyCode::ArrowUp) {
                trade.row = (trade.row + n - 1) % n;
            }
            let id = ItemId(trade.row as u16);
            let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
            if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::ArrowRight) {
                let want = if shift { inv.count(id) } else { 1 };
                let mut sold = 0;
                let mut got = 0;
                for _ in 0..want {
                    if !inv.take_one(id) {
                        break;
                    }
                    let p = eco.sell_price(base, id);
                    eco.credits += p;
                    got += p;
                    eco.markets[base][id.0 as usize].stock += 1.0;
                    sold += 1;
                }
                if sold > 0 {
                    toasts.push(format!("Sold {sold} {} for {got} cr", id.def().name), Color::srgb(1.0, 0.85, 0.4));
                }
            }
            if keys.just_pressed(KeyCode::ArrowLeft) {
                let want = if shift { 10 } else { 1 };
                let mut bought = 0;
                let mut spent = 0;
                for _ in 0..want {
                    let p = eco.buy_price(base, id);
                    if eco.markets[base][id.0 as usize].stock < 1.0 || eco.credits < p || inv.add(id, 1) == 0 {
                        break;
                    }
                    eco.credits -= p;
                    spent += p;
                    eco.markets[base][id.0 as usize].stock -= 1.0;
                    bought += 1;
                }
                if bought > 0 {
                    toasts.push(format!("Bought {bought} {} for {spent} cr", id.def().name), Color::srgb(1.0, 0.85, 0.4));
                } else {
                    toasts.push("Can't buy (credits, stock or space)", Color::srgb(1.0, 0.45, 0.3));
                }
            }
        }
    }
}

fn trade_ui(
    mut commands: Commands,
    trade: Res<Trade>,
    eco: Res<Economy>,
    player: Query<&Inventory, With<Player>>,
    mut panel: Query<(Entity, &mut Visibility), With<TradePanel>>,
    mut credits: Query<&mut Text, With<CreditsText>>,
    mut last: Local<Option<(usize, usize, i64, u64, u32)>>,
) {
    if let Ok(mut t) = credits.single_mut() {
        let s = format!("{} cr", eco.credits);
        if t.0 != s {
            t.0 = s;
        }
    }
    let Ok((panel, mut vis)) = panel.single_mut() else { return };
    let Some(base) = trade.open else {
        *vis = Visibility::Hidden;
        *last = None;
        return;
    };
    *vis = Visibility::Visible;
    let Ok(inv) = player.single() else { return };
    // Rebuild when anything shown changes (prices drift every frame, so throttle by the hour).
    let key = (base, trade.row, eco.credits, (eco.hour * 20.0) as u64, inv.value());
    if *last == Some(key) {
        return;
    }
    *last = Some(key);
    commands.entity(panel).despawn_related::<Children>();
    commands.entity(panel).with_children(|c| {
        let txt = |c: &mut ChildSpawnerCommands, s: String, size: f32, color: Color| {
            c.spawn((Text::new(s), TextFont { font_size: size, ..default() }, TextColor(color)));
        };
        txt(c, format!("{} - market   (day {}, {:02}:00)", BASE_NAMES[base], (eco.hour / 24.0) as u64 + 1, (eco.hour % 24.0) as u64), 20.0, Color::WHITE);
        txt(c, format!("Credits: {} cr", eco.credits), 16.0, Color::srgb(1.0, 0.85, 0.4));
        txt(c, format!("{:<20} {:>9} {:>8} {:>8} {:>7} {:>6}", "Item", "rarity", "sell", "buy", "stock", "you"), 14.0, Color::srgb(0.6, 0.6, 0.6));
        for i in 0..ITEMS.len() {
            let id = ItemId(i as u16);
            let d = id.def();
            let m = &eco.markets[base][i];
            let p = eco.price(base, id);
            let trend = if p > m.last_price * 1.01 { " ^" } else if p < m.last_price * 0.99 { " v" } else { "  " };
            let sel = if i == trade.row { ">" } else { " " };
            let line = format!(
                "{sel}{:<19} {:>9} {:>6} cr {:>5} cr{trend} {:>7.0} {:>6}",
                d.name,
                d.rarity.name(),
                eco.sell_price(base, id),
                eco.buy_price(base, id),
                m.stock,
                inv.count(id)
            );
            let color = if i == trade.row { Color::WHITE } else { d.rarity.color() };
            txt(c, line, 15.0, color);
        }
        for n in eco.news.iter().rev().take(2) {
            txt(c, format!("News: {n}"), 13.0, Color::srgb(0.6, 0.85, 1.0));
        }
        txt(c, "Up/Down select   Enter sell 1 (Shift: all)   Left buy 1 (Shift: 10)   E close".into(), 13.0, Color::srgb(0.6, 0.6, 0.6));
    });
}

/// Distances to the bases, bottom right.
fn nav_ui(cam: Res<crate::camera::CameraWorld>, planet: Res<Planet>, mut q: Query<&mut Text, With<NavText>>, mut timer: Local<f32>, time: Res<Time>) {
    *timer -= time.delta_secs();
    if *timer > 0.0 {
        return;
    }
    *timer = 0.25;
    let Ok(mut t) = q.single_mut() else { return };
    let fmt = |m: f64| if m < 1000.0 { format!("{m:.0} m") } else if m < 1e6 { format!("{:.1} km", m / 1000.0) } else { format!("{:.0} km", m / 1000.0) };
    let lines: Vec<String> = (0..2).map(|b| format!("{}: {}", BASE_NAMES[b], fmt((base_site(&planet.terrain, b).1 - cam.pos).length()))).collect();
    let s = lines.join("\n");
    if t.0 != s {
        t.0 = s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::item;

    #[test]
    fn markets_follow_supply_and_demand() {
        let mut e = Economy::new();
        // Haven produces food, Selene needs it: food is dearer on the moon.
        assert!(e.price(1, item::BERRIES) > e.price(0, item::BERRIES) * 3.0);
        // Selene mines helium-3, Haven needs it.
        assert!(e.price(0, item::HELIUM3) > e.price(1, item::HELIUM3) * 3.0);
        // Dumping supply lowers the price; it recovers over time.
        let before = e.price(0, item::IRON_PLATE);
        e.markets[0][item::IRON_PLATE.0 as usize].stock += 300.0;
        let flooded = e.price(0, item::IRON_PLATE);
        assert!(flooded < before * 0.7, "{before} -> {flooded}");
        for _ in 0..240 {
            e.tick(0.25);
        }
        let recovered = e.price(0, item::IRON_PLATE);
        assert!(recovered > flooded * 1.2 && (recovered - before).abs() < before * 0.15, "{before} -> {flooded} -> {recovered}");
    }
}
