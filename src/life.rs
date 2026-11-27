//! Life on a planet: its biosphere, generated from the planet's seed.
//!
//! A [`Biosphere`] lists the planet's plant species (with meshes and climate preferences), its
//! animal species and, on some worlds, an intelligent species with a civilization. The home
//! planet uses a hand-tuned Earth-like set (including a giant-conifer and fern rainforest);
//! other planets get alien life generated from their seed: new body plans, palettes and
//! climate niches. Fauna and civilizations are described as data here, ready for creature and
//! settlement systems to use the same climate envelopes and placement grids as the flora.

use std::sync::Arc;

use crate::items::item;
use crate::flora::{Envelope, FloraCollision, FloraSpecies, Forage, GroundEffect, Layer, ground};
use crate::plants::{self, Rgb, Rng, Template};
use crate::textures::flora_layer::BARK_SPRUCE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiosphereKind {
    Barren,
    Earthlike,
    Alien,
}

/// Bare-ground colours, before vegetation.
#[derive(Clone, Copy, Debug)]
pub struct GroundPalette {
    pub soil_dry: Rgb,
    pub soil_wet: Rgb,
    pub sand: Rgb,
    pub rock: Rgb,
    pub snow: Rgb,
}

#[derive(Clone, Copy, Debug)]
pub enum BodyPlan {
    Quadruped,
    Hexapod,
    Biped,
    Avian,
    Serpentine,
    Floater,
}

#[derive(Clone, Copy, Debug)]
pub enum Diet {
    Grazer,
    Browser,
    Predator,
    Omnivore,
    Filter,
}

/// An animal species (not yet spawned in the world).
#[derive(Clone, Debug)]
pub struct FaunaSpecies {
    pub name: String,
    pub body: BodyPlan,
    pub size_m: f32,
    pub diet: Diet,
    pub habitat: Envelope,
    pub group_size: (u32, u32),
    pub sapient: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum TechLevel {
    Tribal,
    Agrarian,
    Industrial,
    Spacefaring,
}

/// An intelligent species' civilization.
#[derive(Clone, Debug)]
pub struct Civilization {
    /// Index into `Biosphere::fauna`.
    pub species: usize,
    pub tech: TechLevel,
    /// Settlements per 10 000 km² of suitable land.
    pub settlement_density: f32,
    /// Where they prefer to settle.
    pub habitat: Envelope,
}

pub struct Biosphere {
    pub kind: BiosphereKind,
    pub ground: GroundPalette,
    /// Colour of distant forest canopy.
    pub canopy_color: Rgb,
    /// Multipliers for the terrain texture layers (white = the photo-scanned colour). Alien
    /// worlds recolour the same materials.
    pub layer_tints: [Rgb; 8],
    pub flora: Vec<FloraSpecies>,
    pub fauna: Vec<FaunaSpecies>,
    pub civilization: Option<Civilization>,
}

fn arc3(t: [Option<Template>; 3]) -> [Option<Arc<Template>>; 3] {
    t.map(|o| o.map(Arc::new))
}

fn env(t: (f32, f32, f32, f32), m: (f32, f32, f32, f32), max_height: f32, river_bonus: f32) -> Envelope {
    Envelope { temperature: t, moisture: m, max_height, river_bonus }
}

const EARTH_GROUND: GroundPalette = GroundPalette {
    soil_dry: [0.30, 0.24, 0.15],
    soil_wet: [0.09, 0.07, 0.045],
    sand: [0.45, 0.40, 0.27],
    rock: [0.13, 0.115, 0.10],
    snow: [0.85, 0.87, 0.92],
};

/// Builder for species, to keep the tables below readable.
struct S {
    name: &'static str,
    layer: Layer,
    envelope: Envelope,
    density: f32,
    scale: (f32, f32),
    min_slope_cos: f32,
    align: f32,
    collision: Option<FloraCollision>,
    ground: GroundEffect,
}

impl S {
    fn with(self, tiers: [Option<Template>; 3]) -> FloraSpecies {
        FloraSpecies {
            name: self.name.to_string(),
            layer: self.layer,
            envelope: self.envelope,
            density: self.density,
            scale: self.scale,
            min_slope_cos: self.min_slope_cos,
            align: self.align,
            collision: self.collision,
            ground: self.ground,
            tiers: arc3(tiers),
            forage: None,
            timber: None,
        }
    }
}

fn no_cover() -> GroundEffect {
    GroundEffect { layer: ground::GRASS, cover: 0.0, canopy: false }
}

fn cover(layer: usize, cover: f32) -> GroundEffect {
    GroundEffect { layer, cover, canopy: false }
}

fn canopy(layer: usize, cover: f32) -> GroundEffect {
    GroundEffect { layer, cover, canopy: true }
}

impl Biosphere {
    pub fn barren() -> Self {
        Self { kind: BiosphereKind::Barren, ground: EARTH_GROUND, canopy_color: [0.03, 0.06, 0.03], layer_tints: [[1.0; 3]; 8], flora: Vec::new(), fauna: Vec::new(), civilization: None }
    }

    /// The home planet: Earth-like plants, including a giant-conifer and fern rainforest.
    pub fn earthlike(seed: u64) -> Self {
        let s = |k: u64| seed.wrapping_mul(31).wrapping_add(k);
        let wet_forest = (0.46, 0.6, 1.01, 1.02);
        let flora = vec![
            // --- Giant-conifer rainforest (cool, very wet): huge trees, ferns, logs, moss.
            S {
                name: "giant conifer",
                layer: Layer::Canopy,
                envelope: env((2.0, 7.0, 19.0, 24.0), wet_forest, 1800.0, 0.3),
                density: 0.55,
                scale: (0.7, 1.25),
                min_slope_cos: 0.8,
                align: 0.0,
                collision: Some(FloraCollision::Trunk { radius: 1.9, height: 30.0 }),
                ground: canopy(ground::LITTER, 1.0),
            }
            .with(plants::giant_conifer([0.85, 1.0, 0.85], s(1)))
            .timber(item::REDWOOD_LOG, 6, plants::stump(1.9, crate::textures::flora_layer::BARK_REDWOOD, [1.25, 0.88, 0.72])),
            S {
                name: "young fir",
                layer: Layer::Understory,
                envelope: env((3.0, 8.0, 18.0, 23.0), wet_forest, 2000.0, 0.0),
                density: 0.12,
                scale: (0.3, 0.7),
                min_slope_cos: 0.75,
                align: 0.0,
                collision: Some(FloraCollision::Trunk { radius: 0.35, height: 6.0 }),
                ground: no_cover(),
            }
            .with(plants::conifer(BARK_SPRUCE, [1.0, 1.08, 0.95], 0.2, s(2)))
            .timber(item::SOFTWOOD_LOG, 1, plants::stump(0.35, BARK_SPRUCE, [1.0, 1.0, 1.0])),
            S {
                name: "sword fern",
                layer: Layer::Ground,
                envelope: env((2.0, 7.0, 20.0, 25.0), (0.46, 0.6, 1.01, 1.02), 2000.0, 0.2),
                density: 0.85,
                scale: (0.8, 1.5),
                min_slope_cos: 0.6,
                align: 0.6,
                collision: None,
                ground: cover(ground::MOSS, 0.7),
            }
            .with(plants::fern([1.0, 1.0, 0.95], 7, s(3))),
            S {
                name: "fallen log",
                layer: Layer::Understory,
                envelope: env((3.0, 8.0, 18.0, 23.0), wet_forest, 1800.0, 0.0),
                density: 0.05,
                scale: (0.8, 1.6),
                min_slope_cos: 0.85,
                align: 0.9,
                collision: Some(FloraCollision::Log { radius: 0.55, length: 8.0 }),
                ground: no_cover(),
            }
            .with(plants::log()),
            S {
                name: "mossy boulder",
                layer: Layer::Understory,
                envelope: env((0.0, 6.0, 20.0, 26.0), (0.5, 0.65, 1.01, 1.02), 3000.0, 0.0),
                density: 0.05,
                scale: (0.5, 1.8),
                min_slope_cos: 0.5,
                align: 1.0,
                collision: Some(FloraCollision::Sphere { radius: 0.85, y: 0.35 }),
                ground: no_cover(),
            }
            .with(plants::boulder(true, [1.0, 1.0, 1.0], s(5))),
            S {
                name: "forest mushrooms",
                layer: Layer::Ground,
                envelope: env((4.0, 8.0, 18.0, 22.0), (0.6, 0.75, 1.01, 1.02), 1800.0, 0.0),
                density: 0.06,
                scale: (0.8, 1.6),
                min_slope_cos: 0.7,
                align: 0.8,
                collision: None,
                ground: no_cover(),
            }
            .with(plants::mushroom([0.7, 0.65, 0.55], [0.55, 0.12, 0.04], s(6)))
            .forage(Forage {
                item: item::MUSHROOM,
                amount: (2, 4),
                verb: "Pick mushrooms",
                regrow: 900.0,
                bonus: vec![(item::CHANTERELLE, 0.08), (item::TRUFFLE, 0.015)],
                reach: 1.8,
                height: 0.1,
                bare: None,
            }),
            // --- Boreal forest (cold).
            S {
                name: "spruce",
                layer: Layer::Canopy,
                envelope: env((-7.0, -2.0, 7.0, 13.0), (0.3, 0.45, 1.01, 1.02), 3000.0, 0.2),
                density: 0.45,
                scale: (0.6, 1.4),
                min_slope_cos: 0.7,
                align: 0.0,
                collision: Some(FloraCollision::Trunk { radius: 0.4, height: 8.0 }),
                ground: canopy(ground::LITTER, 0.9),
            }
            .with(plants::conifer(BARK_SPRUCE, [0.85, 0.95, 0.95], 0.35, s(7)))
            .timber(item::SOFTWOOD_LOG, 4, plants::stump(0.4, BARK_SPRUCE, [1.0, 1.0, 1.0])),
            S {
                name: "dwarf pine",
                layer: Layer::Understory,
                envelope: env((-9.0, -5.0, 3.0, 8.0), (0.2, 0.3, 1.01, 1.02), 4200.0, 0.0),
                density: 0.25,
                scale: (0.15, 0.4),
                min_slope_cos: 0.6,
                align: 0.2,
                collision: Some(FloraCollision::Trunk { radius: 0.12, height: 3.0 }),
                ground: cover(ground::MOSS, 0.4),
            }
            .with(plants::conifer(BARK_SPRUCE, [1.0, 1.0, 0.8], 0.6, s(8))),
            S {
                name: "tundra shrub",
                layer: Layer::Ground,
                envelope: env((-15.0, -10.0, 0.0, 4.0), (0.2, 0.3, 1.01, 1.02), 4500.0, 0.0),
                density: 0.4,
                scale: (0.5, 1.0),
                min_slope_cos: 0.6,
                align: 0.7,
                collision: None,
                ground: cover(ground::MOSS, 0.5),
            }
            .with(plants::dry_shrub([1.0, 1.0, 1.0], [0.95, 0.85, 0.6], s(9))),
            // --- Temperate broadleaf forest and meadows.
            S {
                name: "broadleaf tree",
                layer: Layer::Canopy,
                envelope: env((5.0, 10.0, 24.0, 30.0), (0.3, 0.4, 0.68, 0.82), 1600.0, 0.3),
                density: 0.4,
                scale: (0.7, 1.3),
                min_slope_cos: 0.75,
                align: 0.0,
                collision: Some(FloraCollision::Trunk { radius: 0.6, height: 8.0 }),
                ground: canopy(ground::LITTER, 0.8),
            }
            .with(plants::broadleaf([1.0, 1.0, 0.9], s(10)))
            .timber(item::HARDWOOD_LOG, 4, plants::stump(0.6, crate::textures::flora_layer::BARK_BROADLEAF, [1.0, 1.0, 1.0])),
            S {
                name: "berry bush",
                layer: Layer::Understory,
                envelope: env((3.0, 8.0, 24.0, 30.0), (0.3, 0.42, 0.85, 1.0), 2000.0, 0.0),
                density: 0.16,
                scale: (0.7, 1.4),
                min_slope_cos: 0.6,
                align: 0.3,
                collision: None,
                ground: cover(ground::GRASS, 0.3),
            }
            .with(plants::bush([0.9, 1.0, 0.85], Some([0.6, 0.04, 0.05]), s(11)))
            .forage(Forage {
                item: item::BERRIES,
                amount: (5, 10),
                verb: "Pick berries",
                regrow: 480.0,
                bonus: Vec::new(),
                reach: 2.2,
                height: 0.8,
                bare: Some(arc3(plants::bush([0.9, 1.0, 0.85], None, s(11)))),
            }),
            S {
                name: "wild apple tree",
                layer: Layer::Understory,
                envelope: env((4.0, 9.0, 22.0, 28.0), (0.35, 0.45, 0.8, 0.9), 1800.0, 0.2),
                density: 0.05,
                scale: (0.34, 0.46),
                min_slope_cos: 0.75,
                align: 0.0,
                collision: Some(FloraCollision::Trunk { radius: 0.65, height: 6.0 }),
                ground: cover(ground::GRASS, 0.2),
            }
            .with(plants::fruit_tree([1.0, 1.05, 0.85], Some([0.62, 0.08, 0.05]), s(19)))
            .forage(Forage {
                item: item::APPLE,
                amount: (3, 6),
                verb: "Pick apples",
                regrow: 900.0,
                bonus: vec![(item::AMBER, 0.03)],
                reach: 5.0,
                height: 8.0,
                bare: Some(arc3(plants::fruit_tree([1.0, 1.05, 0.85], None, s(19)))),
            }),
            S {
                name: "flowering bush",
                layer: Layer::Understory,
                envelope: env((10.0, 15.0, 28.0, 34.0), (0.35, 0.5, 0.8, 0.95), 1500.0, 0.0),
                density: 0.08,
                scale: (0.7, 1.3),
                min_slope_cos: 0.6,
                align: 0.3,
                collision: None,
                ground: no_cover(),
            }
            .with(plants::bush([1.0, 1.0, 0.95], Some([0.85, 0.35, 0.6]), s(12)))
            .forage(Forage {
                item: item::HERBS,
                amount: (1, 3),
                verb: "Gather herbs",
                regrow: 600.0,
                bonus: Vec::new(),
                reach: 2.2,
                height: 0.8,
                bare: Some(arc3(plants::bush([1.0, 1.0, 0.95], None, s(12)))),
            }),
            S {
                name: "bracken",
                layer: Layer::Ground,
                envelope: env((6.0, 10.0, 22.0, 27.0), (0.38, 0.48, 0.7, 0.85), 1800.0, 0.0),
                density: 0.35,
                scale: (0.7, 1.2),
                min_slope_cos: 0.6,
                align: 0.6,
                collision: None,
                ground: cover(ground::GRASS, 0.4),
            }
            .with(plants::fern([1.2, 1.1, 0.75], 6, s(13))),
            S {
                name: "grass",
                layer: Layer::Detail,
                envelope: env((0.0, 6.0, 28.0, 35.0), (0.15, 0.25, 0.6, 0.8), 3000.0, 0.0),
                density: 0.8,
                scale: (0.7, 1.4),
                min_slope_cos: 0.55,
                align: 0.8,
                collision: None,
                ground: cover(ground::GRASS, 0.9),
            }
            .with(plants::grass([1.0, 1.0, 1.0], s(14))),
            S {
                name: "yellow flowers",
                layer: Layer::Detail,
                envelope: env((8.0, 12.0, 26.0, 30.0), (0.3, 0.4, 0.7, 0.8), 2000.0, 0.0),
                density: 0.08,
                scale: (0.8, 1.2),
                min_slope_cos: 0.7,
                align: 0.5,
                collision: None,
                ground: no_cover(),
            }
            .with(plants::flower([0.1, 0.2, 0.05], [0.9, 0.7, 0.05], s(15))),
            S {
                name: "violet flowers",
                layer: Layer::Detail,
                envelope: env((4.0, 9.0, 20.0, 26.0), (0.35, 0.45, 0.8, 0.9), 2500.0, 0.0),
                density: 0.06,
                scale: (0.8, 1.2),
                min_slope_cos: 0.7,
                align: 0.5,
                collision: None,
                ground: no_cover(),
            }
            .with(plants::flower([0.1, 0.2, 0.05], [0.45, 0.15, 0.7], s(16))),
            // --- Dry lands.
            S {
                name: "cactus",
                layer: Layer::Understory,
                envelope: env((15.0, 21.0, 40.0, 46.0), (0.0, 0.02, 0.2, 0.3), 2500.0, 0.0),
                density: 0.07,
                scale: (0.5, 1.4),
                min_slope_cos: 0.7,
                align: 0.0,
                collision: Some(FloraCollision::Trunk { radius: 0.35, height: 3.5 }),
                ground: no_cover(),
            }
            .with(plants::cactus([0.12, 0.2, 0.08], Some([0.75, 0.12, 0.3]), s(17)))
            .forage(Forage {
                item: item::CACTUS_FRUIT,
                amount: (1, 3),
                verb: "Pick cactus fruit",
                regrow: 1200.0,
                bonus: Vec::new(),
                reach: 2.0,
                height: 1.6,
                bare: Some(arc3(plants::cactus([0.12, 0.2, 0.08], None, s(17)))),
            }),
            S {
                name: "dry shrub",
                layer: Layer::Ground,
                envelope: env((-5.0, 2.0, 35.0, 45.0), (0.02, 0.08, 0.22, 0.32), 3000.0, 0.0),
                density: 0.25,
                scale: (0.6, 1.4),
                min_slope_cos: 0.6,
                align: 0.5,
                collision: None,
                ground: cover(ground::DIRT, 0.3),
            }
            .with(plants::dry_shrub([1.0, 1.0, 1.0], [1.1, 0.9, 0.5], s(18))),
            S {
                name: "boulder",
                layer: Layer::Understory,
                envelope: env((-40.0, -30.0, 45.0, 55.0), (-0.01, 0.0, 0.5, 0.65), 6000.0, 0.0),
                density: 0.03,
                scale: (0.4, 2.0),
                min_slope_cos: 0.4,
                align: 1.0,
                collision: Some(FloraCollision::Sphere { radius: 0.85, y: 0.35 }),
                ground: no_cover(),
            }
            .with(plants::boulder(false, [1.0, 0.97, 0.93], s(19))),
        ];
        let fauna = vec![
            fauna("forest browser", BodyPlan::Quadruped, 1.6, Diet::Browser, env((3.0, 8.0, 18.0, 24.0), wet_forest, 1800.0, 0.2), (2, 6)),
            fauna("canopy glider", BodyPlan::Avian, 0.6, Diet::Omnivore, env((3.0, 8.0, 18.0, 24.0), wet_forest, 2000.0, 0.0), (1, 3)),
            fauna("plains grazer", BodyPlan::Quadruped, 2.0, Diet::Grazer, env((2.0, 8.0, 26.0, 32.0), (0.2, 0.3, 0.6, 0.75), 2500.0, 0.3), (8, 40)),
            fauna("ridge stalker", BodyPlan::Quadruped, 2.4, Diet::Predator, env((-10.0, -2.0, 20.0, 28.0), (0.2, 0.3, 1.0, 1.01), 3500.0, 0.0), (1, 2)),
            fauna("dune runner", BodyPlan::Biped, 1.1, Diet::Omnivore, env((15.0, 22.0, 40.0, 46.0), (0.0, 0.02, 0.2, 0.3), 2500.0, 0.0), (3, 12)),
        ];
        Self {
            kind: BiosphereKind::Earthlike,
            ground: EARTH_GROUND,
            canopy_color: [0.025, 0.05, 0.025],
            layer_tints: [[1.0; 3]; 8],
            flora,
            fauna,
            civilization: None,
        }
    }

    /// An alien biosphere generated from `seed`: new plant forms, palette and niches, animals,
    /// and sometimes a civilization.
    pub fn alien(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let hue = rng.next();
        let foliage = hsv(hue, rng.range(0.5, 0.85), rng.range(0.25, 0.5));
        let accent = hsv(hue + rng.range(0.25, 0.6), rng.range(0.6, 0.95), rng.range(0.5, 0.9));
        let pale = hsv(hue + rng.range(-0.1, 0.1), rng.range(0.2, 0.5), rng.range(0.4, 0.7));
        let soil_hue = rng.next();
        let ground = GroundPalette {
            soil_dry: hsv(soil_hue, rng.range(0.2, 0.5), rng.range(0.2, 0.4)),
            soil_wet: hsv(soil_hue, rng.range(0.3, 0.6), rng.range(0.05, 0.15)),
            sand: hsv(soil_hue + 0.05, rng.range(0.2, 0.45), rng.range(0.35, 0.55)),
            rock: hsv(rng.next(), rng.range(0.05, 0.3), rng.range(0.1, 0.2)),
            snow: hsv(rng.next(), rng.range(0.0, 0.15), 0.88),
        };
        // Random climate niches spread over the planet's range of climates.
        let niche = |rng: &mut Rng| {
            let t = rng.range(-8.0, 26.0);
            let w = rng.range(6.0, 12.0);
            let m = rng.range(0.15, 0.85);
            let mw = rng.range(0.15, 0.35);
            env((t - w * 1.6, t - w, t + w, t + w * 1.6), (m - mw * 1.6, m - mw, m + mw, m + mw * 1.6), rng.range(1500.0, 4000.0), rng.range(0.0, 0.5))
        };
        let mut flora = Vec::new();
        let canopy_kinds = 1 + (rng.next() * 2.0) as u32;
        for k in 0..canopy_kinds {
            let e = niche(&mut rng);
            let (tiers, coll) = if (k + seed as u32) % 2 == 0 {
                (plants::alien_mushroom_tree(pale, foliage, accent, seed + 10 + k as u64), FloraCollision::Trunk { radius: 1.0, height: 12.0 })
            } else {
                (plants::alien_spire(foliage, accent, rng.range(0.3, 2.0), seed + 10 + k as u64), FloraCollision::Trunk { radius: 1.2, height: 10.0 })
            };
            flora.push(S {
                name: leak(alien_name(&mut rng)),
                layer: Layer::Canopy,
                envelope: e,
                density: rng.range(0.3, 0.6),
                scale: (0.6, 1.4),
                min_slope_cos: 0.75,
                align: 0.0,
                collision: Some(coll),
                ground: canopy(ground::LITTER, 0.9),
            }
            .with(tiers));
        }
        let e = niche(&mut rng);
        flora.push(S {
            name: leak(alien_name(&mut rng)),
            layer: Layer::Understory,
            envelope: e,
            density: rng.range(0.15, 0.35),
            scale: (0.6, 1.5),
            min_slope_cos: 0.6,
            align: 0.2,
            collision: Some(FloraCollision::Sphere { radius: 1.0, y: 2.0 }),
            ground: cover(ground::MOSS, 0.4),
        }
        .with(plants::alien_tendril(pale, accent, 2 + (rng.next() * 2.0) as u32, seed + 20)));
        let e = niche(&mut rng);
        flora.push(S {
            name: leak(alien_name(&mut rng)),
            layer: Layer::Understory,
            envelope: e,
            density: rng.range(0.1, 0.25),
            scale: (0.7, 1.6),
            min_slope_cos: 0.6,
            align: 0.4,
            collision: None,
            ground: cover(ground::DIRT, 0.3),
        }
        .with(plants::alien_bulbs(foliage, accent, seed + 21)));
        flora.push(S {
            name: "alien boulder",
            layer: Layer::Understory,
            envelope: env((-60.0, -50.0, 60.0, 70.0), (-0.01, 0.0, 1.01, 1.02), 8000.0, 0.0),
            density: 0.04,
            scale: (0.4, 2.2),
            min_slope_cos: 0.4,
            align: 1.0,
            collision: Some(FloraCollision::Sphere { radius: 0.85, y: 0.35 }),
            ground: no_cover(),
        }
        .with(plants::boulder(false, ground.rock.map(|c| (c * 6.0).clamp(0.3, 1.4)), seed + 22)));
        let e = niche(&mut rng);
        flora.push(S {
            name: leak(alien_name(&mut rng)),
            layer: Layer::Ground,
            envelope: e,
            density: rng.range(0.4, 0.8),
            scale: (0.7, 1.5),
            min_slope_cos: 0.6,
            align: 0.7,
            collision: None,
            ground: cover(ground::MOSS, 0.6),
        }
        .with(plants::alien_fan(foliage, accent, seed + 23)));
        let e = niche(&mut rng);
        flora.push(S {
            name: leak(alien_name(&mut rng)),
            layer: Layer::Detail,
            envelope: e,
            density: rng.range(0.5, 0.9),
            scale: (0.6, 1.4),
            min_slope_cos: 0.55,
            align: 0.8,
            collision: None,
            ground: cover(ground::GRASS, 0.8),
        }
        .with(plants::grass_ribbons(pale, seed + 24)));

        // Animals: a few species with random body plans and niches; maybe one is sapient.
        let plans = [BodyPlan::Quadruped, BodyPlan::Hexapod, BodyPlan::Biped, BodyPlan::Avian, BodyPlan::Serpentine, BodyPlan::Floater];
        let diets = [Diet::Grazer, Diet::Browser, Diet::Predator, Diet::Omnivore, Diet::Filter];
        let count = 3 + (rng.next() * 6.0) as usize;
        let mut fauna_list = Vec::new();
        for _ in 0..count {
            let size = 0.2 * 30f32.powf(rng.next());
            let lo = 1 + (rng.next() * 4.0) as u32;
            fauna_list.push(FaunaSpecies {
                name: alien_name(&mut rng),
                body: plans[(rng.next() * plans.len() as f32) as usize % plans.len()],
                size_m: size,
                diet: diets[(rng.next() * diets.len() as f32) as usize % diets.len()],
                habitat: niche(&mut rng),
                group_size: (lo, lo * (1 + (rng.next() * 8.0) as u32)),
                sapient: false,
            });
        }
        let civilization = if rng.next() < 0.4 {
            let who = (rng.next() * fauna_list.len() as f32) as usize % fauna_list.len();
            fauna_list[who].sapient = true;
            fauna_list[who].size_m = rng.range(1.0, 3.0);
            fauna_list[who].diet = Diet::Omnivore;
            let techs = [TechLevel::Tribal, TechLevel::Agrarian, TechLevel::Industrial, TechLevel::Spacefaring];
            Some(Civilization {
                species: who,
                tech: techs[(rng.next() * 4.0) as usize % 4],
                settlement_density: rng.range(0.5, 6.0),
                habitat: fauna_list[who].habitat,
            })
        } else {
            None
        };
        // Recolour the photo-scanned ground: vegetation layers take the foliage hue, mineral
        // layers the soil and rock palette.
        let norm = |c: Rgb| {
            let m = c[0].max(c[1]).max(c[2]).max(1e-3);
            let t = [c[0] / m, c[1] / m, c[2] / m];
            plants::mix(t, [1.0; 3], 0.25)
        };
        let veg = norm(foliage);
        let grass_green = [0.55, 1.0, 0.45];
        let veg_tint = [veg[0] / grass_green[0], veg[1] / grass_green[1], veg[2] / grass_green[2]].map(|v| v.min(2.2));
        let soil = norm(ground.soil_dry);
        let rock = norm(ground.rock);
        let layer_tints = [veg_tint, soil, veg_tint, soil, norm(ground.sand), rock, norm(ground.snow), [1.0; 3]];
        Self {
            kind: BiosphereKind::Alien,
            ground,
            canopy_color: plants::mix(foliage, [0.0; 3], 0.4),
            layer_tints,
            flora,
            fauna: fauna_list,
            civilization,
        }
    }

    /// One-paragraph description for logs and the HUD.
    pub fn summary(&self) -> String {
        let plants: Vec<&str> = self.flora.iter().map(|s| s.name.as_str()).collect();
        let animals: Vec<String> = self
            .fauna
            .iter()
            .map(|f| {
                format!(
                    "{} ({:?} {:?}, {:.1} m, groups of {}-{}{})",
                    f.name,
                    f.body,
                    f.diet,
                    f.size_m,
                    f.group_size.0,
                    f.group_size.1,
                    if f.sapient { ", sapient" } else { "" }
                )
            })
            .collect();
        let civ = match &self.civilization {
            Some(c) => format!(
                "{:?} civilization of {}, {:.1} settlements per 10 000 km2 between {:.0} and {:.0} C",
                c.tech,
                self.fauna[c.species].name,
                c.settlement_density,
                c.habitat.temperature.0,
                c.habitat.temperature.3
            ),
            None => "no civilization".to_string(),
        };
        format!("{:?} biosphere. Plants: {}. Animals: {}. {}.", self.kind, plants.join(", "), animals.join(", "), civ)
    }
}

fn fauna(name: &str, body: BodyPlan, size_m: f32, diet: Diet, habitat: Envelope, group_size: (u32, u32)) -> FaunaSpecies {
    FaunaSpecies { name: name.to_string(), body, size_m, diet, habitat, group_size, sapient: false }
}

/// Species names live as long as the program (there are only a handful per planet).
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Pronounceable made-up name.
fn alien_name(rng: &mut Rng) -> String {
    const ONSETS: [&str; 16] = ["k", "v", "th", "z", "m", "r", "s", "x", "l", "dr", "q", "n", "y", "gh", "t", "br"];
    const VOWELS: [&str; 8] = ["a", "e", "i", "o", "u", "ae", "y", "oo"];
    const CODAS: [&str; 8] = ["", "n", "r", "th", "x", "l", "s", "k"];
    let mut s = String::new();
    for _ in 0..(2 + (rng.next() * 2.0) as usize) {
        s += ONSETS[(rng.next() * 16.0) as usize % 16];
        s += VOWELS[(rng.next() * 8.0) as usize % 8];
    }
    s += CODAS[(rng.next() * 8.0) as usize % 8];
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// HSV (all 0..1, hue wraps) to linear RGB.
fn hsv(h: f32, s: f32, v: f32) -> Rgb {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    // Into linear space (roughly), as vertex colours are linear.
    [(r + m).powf(2.2), (g + m).powf(2.2), (b + m).powf(2.2)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alien_biospheres_vary_with_seed() {
        let a = Biosphere::alien(1);
        let b = Biosphere::alien(2);
        println!("{}", a.summary());
        println!("{}", b.summary());
        assert!(a.flora.len() >= 6 && b.flora.len() >= 6);
        assert_ne!(a.summary(), b.summary());
        // Deterministic.
        assert_eq!(Biosphere::alien(1).summary(), a.summary());
        let civs = (0..50).filter(|s| Biosphere::alien(*s).civilization.is_some()).count();
        println!("{civs}/50 alien worlds have a civilization");
        assert!((5..45).contains(&civs));
    }
}
