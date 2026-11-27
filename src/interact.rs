//! What the player is looking at and can use with E: plants to pick, ore to mine, factory
//! modules, trade terminals. Each kind offers itself with an aim score every frame (lower is
//! better); the best one gets the prompt and the E key.

use bevy::math::DVec3;
use bevy::prelude::*;

use crate::player::{ControlMode, InShip, Player};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FocusKind {
    Forage,
    Mine,
    Module(Entity),
    Terminal(usize),
}

#[derive(Clone)]
pub struct FocusItem {
    pub kind: FocusKind,
    pub score: f64,
    pub prompt: String,
}

/// The best thing to interact with this frame.
#[derive(Resource, Default)]
pub struct Focus(pub Option<FocusItem>);

impl Focus {
    pub fn offer(&mut self, kind: FocusKind, score: f64, prompt: impl Into<String>) {
        if self.0.as_ref().is_none_or(|f| score < f.score) {
            self.0 = Some(FocusItem { kind, score, prompt: prompt.into() });
        }
    }

    pub fn is(&self, kind: FocusKind) -> bool {
        self.0.as_ref().is_some_and(|f| f.kind == kind)
    }
}

/// E this frame: tapped, and held down. Test scripts can set these too.
#[derive(Resource, Default)]
pub struct Interact {
    pub pressed: bool,
    pub held: bool,
    /// Scripted hold (tests): keeps `held` true.
    pub force_hold: bool,
}

/// The player's eyes and view direction, when on foot outside a ship.
#[derive(Resource, Default, Clone, Copy)]
pub struct View {
    pub eye: Option<(DVec3, DVec3)>,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum InteractSet {
    Reset,
    Offer,
    Act,
}

pub struct InteractPlugin;

impl Plugin for InteractPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Focus>()
            .init_resource::<Interact>()
            .init_resource::<View>()
            .configure_sets(Update, (InteractSet::Reset, InteractSet::Offer, InteractSet::Act).chain().after(crate::player::player_control))
            .add_systems(Update, reset.in_set(InteractSet::Reset));
    }
}

fn reset(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<ControlMode>,
    player: Query<(&Player, &avian3d::prelude::Position, Option<&InShip>)>,
    mut focus: ResMut<Focus>,
    mut input: ResMut<Interact>,
    mut view: ResMut<View>,
) {
    focus.0 = None;
    let walking = *mode == ControlMode::Walk;
    input.pressed = (walking && keys.just_pressed(KeyCode::KeyE)) || std::mem::take(&mut input.pressed);
    input.held = (walking && keys.pressed(KeyCode::KeyE)) || input.force_hold;
    view.eye = None;
    if let Ok((p, pos, in_ship)) = player.single() {
        if walking && in_ship.is_none() {
            let up = crate::world::up_at(pos.0);
            let pitch = p.pitch as f64;
            view.eye = Some((pos.0 + up * 0.75, (p.heading * pitch.cos() + up * pitch.sin()).normalize()));
        }
    }
}

/// Aim score for a point within `reach` of the eye: the angle off the view direction (radians),
/// halved when standing right next to it. `None` if out of reach or not looked at.
pub fn aim(view: &View, point: DVec3, reach: f64) -> Option<f64> {
    let (eye, look) = view.eye?;
    let v = point - eye;
    let dist = v.length();
    if dist > reach {
        return None;
    }
    let angle = (v / dist.max(1e-6)).dot(look).clamp(-1.0, 1.0).acos();
    let score = if dist < 1.2 { angle * 0.5 } else { angle };
    (score < 0.5).then_some(score)
}
