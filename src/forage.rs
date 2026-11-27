//! Foraging and low-key survival: pick berries, fruit, mushrooms and herbs from the planet's
//! plants, carry them in the inventory, eat to keep the hunger meter up.
//!
//! - Look at a foragable plant within reach: the HUD shows "E: Pick berries".
//! - Picked plants are re-meshed in their bare form (or vanish, for mushrooms) and regrow
//!   after a while. Every harvest can turn up rarer finds (chanterelles, truffles, amber).
//! - Hunger drains slowly (faster when running). R eats the cheapest food you carry.
//!   Starving doesn't kill you: you just can't run and walk slower.
//! - I toggles the inventory.
//!
//! Picked state lives in `FloraContext::harvested`, beside the procedural world, which stays a
//! pure function of the seed.

use bevy::math::DVec3;
use bevy::prelude::*;

use crate::flora::forageables_near;
use crate::items::{Inventory, ItemId, Rarity};
use crate::planet::Planet;
use crate::player::{ControlMode, InShip, Player};

/// Hunger points: 100 = full.
#[derive(Resource)]
pub struct Survival {
    pub food: f32,
}

impl Survival {
    pub fn starving(&self) -> bool {
        self.food <= 0.0
    }
}

/// Full to empty in about 45 minutes of walking (twice as fast when running).
const HUNGER_PER_SEC: f32 = 100.0 / 2700.0;

/// The plant the player is looking at, if it can be picked.
#[derive(Resource, Default)]
pub struct ForageTarget(pub Option<Target>);

#[derive(Clone, Copy)]
pub struct Target {
    pub cell: u64,
    pub species: usize,
    pub point: DVec3,
}

/// Actions this frame (keyboard, or a test script).
#[derive(Resource, Default)]
pub struct ForageInput {
    pub harvest: bool,
    pub eat: bool,
}

/// Picked plants waiting to regrow: (cell, where, game time when it regrows).
#[derive(Resource, Default)]
pub struct Regrowth(pub Vec<(u64, DVec3, f64)>);

/// Short messages ("+6 Wild berries"), newest last, with their age.
#[derive(Resource, Default)]
pub struct Toasts(pub Vec<(String, Color, f32)>);

impl Toasts {
    /// "+n Name", merged into the newest message if it is for the same item and recent.
    pub fn push_merged(&mut self, name: &str, n: u32, color: Color) {
        if let Some(last) = self.0.last_mut() {
            if let Some(rest) = last.0.strip_prefix('+') {
                if let Some((count, item)) = rest.split_once(' ') {
                    if item == name && last.2 < 2.5 {
                        if let Ok(c) = count.parse::<u32>() {
                            last.0 = format!("+{} {name}", c + n);
                            last.2 = 0.0;
                            return;
                        }
                    }
                }
            }
        }
        self.push(format!("+{n} {name}"), color);
    }

    pub fn push(&mut self, text: impl Into<String>, color: Color) {
        self.0.push((text.into(), color, 0.0));
        if self.0.len() > 6 {
            self.0.remove(0);
        }
    }
}

/// Recently scanned nearby plants (scanning is not free: it evaluates the procedural placement).
#[derive(Resource, Default)]
struct NearbyCache {
    at: Option<DVec3>,
    time: f64,
    plants: Vec<crate::flora::Forageable>,
}

#[derive(Resource)]
pub struct InventoryOpen(pub bool);

#[derive(Component)]
struct PromptText;
#[derive(Component)]
struct FoodFill;
#[derive(Component)]
struct FoodLabel;
#[derive(Component)]
struct ToastList;
#[derive(Component)]
struct InventoryPanel;

pub struct ForagePlugin;

impl Plugin for ForagePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Survival { food: 80.0 })
            .insert_resource(InventoryOpen(false))
            .init_resource::<ForageTarget>()
            .init_resource::<ForageInput>()
            .init_resource::<Regrowth>()
            .init_resource::<Toasts>()
            .init_resource::<NearbyCache>()
            .add_systems(Startup, spawn_hud)
            .add_systems(Update, read_keys.in_set(crate::interact::InteractSet::Reset))
            .add_systems(Update, find_target.in_set(crate::interact::InteractSet::Offer))
            .add_systems(Update, (harvest, regrow, hunger, update_hud).chain().in_set(crate::interact::InteractSet::Act));
    }
}

fn read_keys(keys: Res<ButtonInput<KeyCode>>, mode: Res<ControlMode>, mut input: ResMut<ForageInput>, mut open: ResMut<InventoryOpen>) {
    if *mode == ControlMode::Walk && !crate::factory::building() {
        input.eat |= keys.just_pressed(KeyCode::KeyR);
    }
    if keys.just_pressed(KeyCode::KeyI) {
        open.0 = !open.0;
    }
}

/// Eye position and look direction of the player.
fn eye(p: &Player, pos: DVec3) -> (DVec3, DVec3) {
    let up = pos.normalize();
    let pitch = p.pitch as f64;
    (pos + up * 0.75, (p.heading * pitch.cos() + up * pitch.sin()).normalize())
}

fn find_target(
    time: Res<Time>,
    mode: Res<ControlMode>,
    planet: Res<Planet>,
    player: Query<(&Player, &avian3d::prelude::Position, Option<&InShip>)>,
    mut cache: ResMut<NearbyCache>,
    mut target: ResMut<ForageTarget>,
    mut focus: ResMut<crate::interact::Focus>,
) {
    target.0 = None;
    let Ok((p, pos, in_ship)) = player.single() else { return };
    if *mode != ControlMode::Walk || in_ship.is_some() {
        return;
    }
    let now = time.elapsed_secs_f64();
    let moved = cache.at.is_none_or(|a| (a - pos.0).length() > 1.0);
    if moved || now - cache.time > 0.5 {
        cache.plants = forageables_near(&planet.flora, pos.0, 7.0);
        cache.at = Some(pos.0);
        cache.time = now;
    }
    let picked = planet.flora.harvested.read().unwrap();
    let (eye, look) = eye(p, pos.0);
    let mut best: Option<(f64, Target)> = None;
    for f in &cache.plants {
        if picked.contains(&f.cell) {
            continue;
        }
        let v = f.point - eye;
        let dist = v.length();
        if dist > f.reach + 0.8 {
            continue;
        }
        let angle = (v / dist).dot(look).clamp(-1.0, 1.0).acos();
        // Aim at it, or stand right next to it.
        let score = if dist < 1.2 { angle * 0.5 } else { angle };
        if score < 0.45 && best.is_none_or(|b| score < b.0) {
            best = Some((score, Target { cell: f.cell, species: f.species, point: f.point }));
        }
    }
    target.0 = best.map(|b| b.1);
    if let Some((score, t)) = best {
        let verb = planet.flora.bio.flora[t.species].forage.as_ref().map_or("Pick", |f| f.verb);
        focus.offer(crate::interact::FocusKind::Forage, score, format!("E: {verb}"));
    }
}

/// Deterministic-ish random number in 0..1 from a cell id and a counter.
fn roll(cell: u64, k: u64) -> f32 {
    let mut x = cell ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 29)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (x >> 40) as f32 / (1u64 << 24) as f32
}

#[allow(clippy::too_many_arguments)]
fn harvest(
    time: Res<Time>,
    mut input: ResMut<ForageInput>,
    target: Res<ForageTarget>,
    mut planet: ResMut<Planet>,
    mut player: Query<&mut Inventory, With<Player>>,
    mut survival: ResMut<Survival>,
    mut regrowth: ResMut<Regrowth>,
    mut toasts: ResMut<Toasts>,
    mut harvests: Local<u64>,
    interact: Res<crate::interact::Interact>,
    focus: Res<crate::interact::Focus>,
) {
    let Ok(mut inv) = player.single_mut() else { return };
    if std::mem::take(&mut input.eat) {
        eat(&mut inv, &mut survival, &mut toasts);
    }
    let scripted = std::mem::take(&mut input.harvest);
    if !(scripted || (interact.pressed && focus.is(crate::interact::FocusKind::Forage))) {
        return;
    }
    let Some(t) = target.0 else { return };
    let Some(f) = planet.flora.bio.flora[t.species].forage.clone() else { return };
    *harvests += 1;
    // Seeded by the plant and the harvest count, so each pick differs but is reproducible.
    let salt = *harvests ^ (time.elapsed_secs_f64() * 1000.0) as u64;
    let n = f.amount.0 + (roll(t.cell, salt) * (f.amount.1 - f.amount.0 + 1) as f32) as u32;
    let mut gained: Vec<(ItemId, u32)> = vec![(f.item, n.min(f.amount.1))];
    for (k, &(item, chance)) in f.bonus.iter().enumerate() {
        if roll(t.cell, salt.wrapping_add(17 + k as u64)) < chance {
            gained.push((item, 1));
        }
    }
    for (item, count) in gained {
        let added = inv.add(item, count);
        let def = item.def();
        if added > 0 {
            let note = if def.rarity >= Rarity::Rare { format!("  ({}!)", def.rarity.name()) } else { String::new() };
            toasts.push(format!("+{added} {}{note}", def.name), def.rarity.color());
        }
        if added < count {
            toasts.push("Inventory full", Color::srgb(1.0, 0.4, 0.3));
        }
    }
    planet.flora.harvested.write().unwrap().insert(t.cell);
    regrowth.0.push((t.cell, t.point, time.elapsed_secs_f64() + f.regrow));
    planet.rebuild_at(t.point);
}

/// Eat the cheapest food carried (save the valuable finds for selling).
fn eat(inv: &mut Inventory, survival: &mut Survival, toasts: &mut Toasts) {
    if survival.food >= 99.0 {
        toasts.push("Not hungry", Color::srgb(0.8, 0.8, 0.8));
        return;
    }
    let food = inv
        .slots
        .iter()
        .flatten()
        .map(|s| s.0)
        .filter(|id| id.def().food > 0.0)
        .min_by_key(|id| id.def().value);
    match food {
        Some(id) => {
            inv.take_one(id);
            let def = id.def();
            survival.food = (survival.food + def.food).min(100.0);
            toasts.push(format!("Ate {} (+{:.0} food)", def.name.to_lowercase(), def.food), Color::srgb(0.9, 0.85, 0.6));
        }
        None => toasts.push("Nothing to eat", Color::srgb(1.0, 0.4, 0.3)),
    }
}

fn regrow(time: Res<Time>, mut regrowth: ResMut<Regrowth>, mut planet: ResMut<Planet>) {
    let now = time.elapsed_secs_f64();
    let due: Vec<(u64, DVec3)> = regrowth.0.iter().filter(|r| r.2 <= now).map(|r| (r.0, r.1)).collect();
    if due.is_empty() {
        return;
    }
    regrowth.0.retain(|r| r.2 > now);
    for (cell, at) in due {
        planet.flora.harvested.write().unwrap().remove(&cell);
        planet.rebuild_at(at);
    }
}

fn hunger(time: Res<Time>, mode: Res<ControlMode>, keys: Res<ButtonInput<KeyCode>>, mut survival: ResMut<Survival>, mut toasts: ResMut<Toasts>) {
    let before = survival.food;
    let running = *mode == ControlMode::Walk && keys.pressed(KeyCode::ShiftLeft);
    survival.food = (survival.food - HUNGER_PER_SEC * if running { 2.0 } else { 1.0 } * time.delta_secs()).max(0.0);
    if before > 25.0 && survival.food <= 25.0 {
        toasts.push("You're getting hungry (R to eat)", Color::srgb(1.0, 0.75, 0.3));
    }
    if before > 0.0 && survival.food <= 0.0 {
        toasts.push("Starving: too weak to run", Color::srgb(1.0, 0.35, 0.3));
    }
}

// ------------------------------------------------------------------------------------- HUD

fn spawn_hud(mut commands: Commands) {
    let font = |size: f32| TextFont { font_size: size, ..default() };
    // Prompt under the crosshair.
    commands.spawn((
        PromptText,
        Text::new(""),
        font(20.0),
        TextColor(Color::WHITE),
        TextLayout::new_with_justify(JustifyText::Center),
        Node { position_type: PositionType::Absolute, top: Val::Percent(54.0), width: Val::Percent(100.0), justify_content: JustifyContent::Center, ..default() },
    ));
    // Food bar, bottom left.
    commands
        .spawn(Node { position_type: PositionType::Absolute, left: Val::Px(14.0), bottom: Val::Px(14.0), flex_direction: FlexDirection::Column, row_gap: Val::Px(4.0), ..default() })
        .with_children(|c| {
            c.spawn((FoodLabel, Text::new("Food"), font(15.0), TextColor(Color::srgb(0.9, 0.9, 0.85))));
            c.spawn((Node { width: Val::Px(220.0), height: Val::Px(10.0), ..default() }, BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55))))
                .with_children(|bar| {
                    bar.spawn((FoodFill, Node { width: Val::Percent(80.0), height: Val::Percent(100.0), ..default() }, BackgroundColor(Color::srgb(0.85, 0.6, 0.2))));
                });
        });
    // Pickup messages, top right.
    commands.spawn((
        ToastList,
        Node { position_type: PositionType::Absolute, right: Val::Px(16.0), top: Val::Px(14.0), flex_direction: FlexDirection::Column, align_items: AlignItems::FlexEnd, row_gap: Val::Px(2.0), ..default() },
    ));
    // Inventory, right side (I).
    commands.spawn((
        InventoryPanel,
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(16.0),
            top: Val::Px(190.0),
            padding: UiRect::all(Val::Px(10.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(3.0),
            min_width: Val::Px(300.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.75)),
        Visibility::Hidden,
    ));
}

#[allow(clippy::too_many_arguments)]
fn update_hud(
    focus: Res<crate::interact::Focus>,
    mut commands: Commands,
    time: Res<Time>,
    target: Res<ForageTarget>,
    planet: Res<Planet>,
    survival: Res<Survival>,
    open: Res<InventoryOpen>,
    mut toasts: ResMut<Toasts>,
    inv: Query<Ref<Inventory>, With<Player>>,
    mut prompt: Query<&mut Text, With<PromptText>>,
    mut fill: Query<(&mut Node, &mut BackgroundColor), With<FoodFill>>,
    mut label: Query<&mut Text, (With<FoodLabel>, Without<PromptText>)>,
    toast_list: Query<Entity, With<ToastList>>,
    mut panel: Query<(Entity, &mut Visibility), With<InventoryPanel>>,
    mut shown_toasts: Local<usize>,
    mut panel_open: Local<bool>,
) {
    if let Ok(mut t) = prompt.single_mut() {
        let _ = (&target, &planet);
        let text = focus.0.as_ref().map_or(String::new(), |f| f.prompt.clone());
        if t.0 != text {
            t.0 = text;
        }
    }
    let food = survival.food;
    if let Ok((mut node, mut bg)) = fill.single_mut() {
        node.width = Val::Percent(food);
        bg.0 = if food < 25.0 { Color::srgb(0.9, 0.3, 0.2) } else { Color::srgb(0.85, 0.6, 0.2) };
    }
    if let Ok(mut l) = label.single_mut() {
        let s = format!("Food {:.0}%{}", food, if survival.starving() { "  (starving)" } else { "" });
        if l.0 != s {
            l.0 = s;
        }
    }

    // Toasts fade after 4 s.
    let dt = time.delta_secs();
    toasts.0.iter_mut().for_each(|t| t.2 += dt);
    let before = toasts.0.len();
    toasts.0.retain(|t| t.2 < 4.0);
    if let Ok(list) = toast_list.single() {
        if toasts.0.len() != *shown_toasts || before != toasts.0.len() || toasts.is_changed() {
            commands.entity(list).despawn_related::<Children>();
            commands.entity(list).with_children(|c| {
                for (text, color, _) in &toasts.0 {
                    c.spawn((Text::new(text.clone()), TextFont { font_size: 18.0, ..default() }, TextColor(*color)));
                }
            });
            *shown_toasts = toasts.0.len();
        }
    }

    // Inventory panel: rebuilt when opened or when the contents change.
    let Ok(inv) = inv.single() else { return };
    if let Ok((panel_e, mut vis)) = panel.single_mut() {
        *vis = if open.0 { Visibility::Visible } else { Visibility::Hidden };
        if open.0 && (inv.is_changed() || !*panel_open) {
            commands.entity(panel_e).despawn_related::<Children>();
            commands.entity(panel_e).with_children(|c| {
                c.spawn((Text::new(format!("Inventory  ({} credits)", inv.value())), TextFont { font_size: 18.0, ..default() }, TextColor(Color::WHITE)));
                let mut any = false;
                for (id, n) in inv.slots.iter().flatten() {
                    any = true;
                    let d = id.def();
                    c.spawn((
                        Text::new(format!("{n:>3} x {}   [{}]", d.name, d.rarity.name())),
                        TextFont { font_size: 16.0, ..default() },
                        TextColor(d.rarity.color()),
                    ));
                }
                if !any {
                    c.spawn((Text::new("(empty)"), TextFont { font_size: 16.0, ..default() }, TextColor(Color::srgb(0.6, 0.6, 0.6))));
                }
                c.spawn((Text::new("E pick   R eat   I close"), TextFont { font_size: 13.0, ..default() }, TextColor(Color::srgb(0.6, 0.6, 0.6))));
            });
        }
        *panel_open = open.0;
    }
}
