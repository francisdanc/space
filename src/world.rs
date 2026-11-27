//! The bodies of the home system: the planet (centred at the origin) and its moon.
//!
//! Everything that walks, flies or stands on the ground asks this module which body is
//! nearest, which way is up, how strong gravity is (the sum of both bodies) and how high the
//! ground is, instead of assuming the planet.

use std::sync::OnceLock;

use bevy::math::DVec3;

use crate::noise::Perlin;
use crate::terrain::{RADIUS, Terrain};

/// Moon radius (m): a small moon, a little over half of ours.
pub const MOON_RADIUS: f64 = 1_100_000.0;
/// Moon centre in the planet frame: ~38,000 km out, a few minutes' flight with boost.
pub const MOON_CENTER: DVec3 = DVec3::new(30_000_000.0, 9_000_000.0, -21_000_000.0);
/// Surface gravity (m/s²).
pub const MOON_G: f64 = 1.62;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, serde::Serialize, serde::Deserialize)]
pub enum Body {
    Planet,
    Moon,
}

impl Body {
    pub fn center(self) -> DVec3 {
        match self {
            Body::Planet => DVec3::ZERO,
            Body::Moon => MOON_CENTER,
        }
    }

    pub fn radius(self) -> f64 {
        match self {
            Body::Planet => RADIUS,
            Body::Moon => MOON_RADIUS,
        }
    }

    /// Gravitational parameter (m³/s²).
    pub fn mu(self) -> f64 {
        match self {
            Body::Planet => crate::player::GRAVITY * RADIUS * RADIUS,
            Body::Moon => MOON_G * MOON_RADIUS * MOON_RADIUS,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Body::Planet => "Home",
            Body::Moon => "Selene",
        }
    }
}

/// The body whose surface is nearest to `pos`.
pub fn nearest(pos: DVec3) -> Body {
    let alt = |b: Body| (pos - b.center()).length() - b.radius();
    if alt(Body::Moon) < alt(Body::Planet) { Body::Moon } else { Body::Planet }
}

/// Local up: away from the centre of the nearest body.
pub fn up_at(pos: DVec3) -> DVec3 {
    (pos - nearest(pos).center()).normalize()
}

/// The moon's sphere of influence (m): a * (m_moon / m_planet)^(2/5).
pub fn moon_soi() -> f64 {
    MOON_CENTER.length() * (Body::Moon.mu() / Body::Planet.mu()).powf(0.4)
}

/// Gravitational acceleration at `pos`. Patched conics (as in many space games): within the
/// moon's sphere of influence only the moon pulls (the moon is treated as falling freely around
/// the planet, so the planet's pull cancels out locally); outside it only the planet.
pub fn gravity(pos: DVec3) -> DVec3 {
    let b = if (pos - MOON_CENTER).length() < moon_soi() { Body::Moon } else { Body::Planet };
    let r = b.center() - pos;
    let d2 = r.length_squared().max(1.0);
    r / d2.sqrt() * (b.mu() / d2)
}

/// Height of the solid ground above the body's radius in direction `dir` from its centre.
pub fn ground(t: &Terrain, b: Body, dir: DVec3) -> f64 {
    match b {
        Body::Planet => t.height(dir),
        Body::Moon => moon().height(dir),
    }
}

/// Height of the visible surface (water included) above the body's radius.
pub fn surface(t: &Terrain, b: Body, dir: DVec3) -> f64 {
    match b {
        Body::Planet => t.surface_height(dir),
        Body::Moon => moon().height(dir),
    }
}

/// Height above the nearest body's radius (no terrain lookup).
pub fn altitude_raw(pos: DVec3) -> f64 {
    let b = nearest(pos);
    (pos - b.center()).length() - b.radius()
}

/// Height above the visible surface of the nearest body.
pub fn altitude(t: &Terrain, pos: DVec3) -> f64 {
    let b = nearest(pos);
    let rel = pos - b.center();
    let r = rel.length();
    r - b.radius() - surface(t, b, rel / r)
}

/// Point on the visible surface straight below `pos` (nearest body).
pub fn surface_point(t: &Terrain, pos: DVec3) -> DVec3 {
    let b = nearest(pos);
    let d = (pos - b.center()).normalize();
    b.center() + d * (b.radius() + surface(t, b, d))
}

/// Point on the solid ground straight below `pos` (nearest body).
pub fn ground_point(t: &Terrain, pos: DVec3) -> DVec3 {
    let b = nearest(pos);
    let d = (pos - b.center()).normalize();
    b.center() + d * (b.radius() + ground(t, b, d))
}

// ----------------------------------------------------------------------------------- moon

/// The moon's surface: grey highlands, darker smooth maria, and craters at every scale from
/// basins hundreds of km across to pits a few metres wide. A flat pad is levelled for the
/// station (see `economy::bases`).
pub struct MoonTerrain {
    noise: Perlin,
    seed: u64,
    /// Levelled pads: (direction, height, radius m).
    pads: Vec<(DVec3, f64, f64)>,
}

pub fn moon() -> &'static MoonTerrain {
    static MOON: OnceLock<MoonTerrain> = OnceLock::new();
    MOON.get_or_init(|| {
        let mut m = MoonTerrain { noise: Perlin::new(crate::planet::SEED ^ 0x5E1E), seed: crate::planet::SEED ^ 0x5E1E, pads: Vec::new() };
        let site = moon_station_dir();
        let h = m.height(site);
        m.pads.push((site, h, 160.0));
        m
    })
}

/// Where the moon station stands: on the near side, facing the planet and the sun.
pub fn moon_station_dir() -> DVec3 {
    let towards_planet = (-MOON_CENTER).normalize();
    let sun = crate::sky::sun_dir().as_dvec3();
    (towards_planet * 0.6 + sun * 0.8).normalize()
}

fn hash(a: i64, b: i64, c: i64, seed: u64, k: u64) -> f64 {
    let mut x = (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (c as u64).wrapping_mul(0x1656_67B1_9E37_79F9)
        ^ seed
        ^ k.wrapping_mul(0x27D4_EB2F_1656_67C5);
    x = (x ^ (x >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 29)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (x >> 11) as f64 / (1u64 << 53) as f64
}

impl MoonTerrain {
    /// Height (m) above MOON_RADIUS.
    pub fn height(&self, d: DVec3) -> f64 {
        let mut h = self.noise.fbm(d * 3.0, 5) * 1800.0 + self.noise.fbm(d * 40.0 + DVec3::splat(3.3), 4) * 250.0;
        h += self.noise.fbm(d * 900.0 + DVec3::splat(-7.1), 4) * 6.0;
        // Craters: random bowls on a 3D cell grid at each scale (cell size in m).
        for (k, &(cell, density)) in [(260_000.0, 0.35), (60_000.0, 0.45), (14_000.0, 0.5), (3_000.0, 0.5), (600.0, 0.45), (120.0, 0.35)].iter().enumerate() {
            let p = d * (MOON_RADIUS / cell);
            let base = p.floor();
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let c = base + DVec3::new(dx as f64, dy as f64, dz as f64);
                        let (ci, cj, ck) = (c.x as i64, c.y as i64, c.z as i64);
                        if hash(ci, cj, ck, self.seed, k as u64 * 7) > density {
                            continue;
                        }
                        let centre = c + DVec3::new(
                            hash(ci, cj, ck, self.seed, k as u64 * 7 + 1),
                            hash(ci, cj, ck, self.seed, k as u64 * 7 + 2),
                            hash(ci, cj, ck, self.seed, k as u64 * 7 + 3),
                        );
                        // Radius in cells; the crater sits where the sphere passes near its centre.
                        let r = 0.15 + 0.3 * hash(ci, cj, ck, self.seed, k as u64 * 7 + 4);
                        let x = (p - centre).length() / r;
                        if x > 2.0 {
                            continue;
                        }
                        let radius_m = r * cell;
                        let depth = radius_m * 0.18;
                        let rim = depth * 0.25;
                        h += if x < 1.0 { depth * (x * x - 1.0) + rim } else { rim * (-(x - 1.0) * 4.0).exp() * (2.0 - x) };
                    }
                }
            }
        }
        for &(dir, ph, radius) in &self.pads {
            let dist = (d - dir).length() * MOON_RADIUS;
            if dist < radius * 2.0 {
                let t = crate::terrain::smoothstep(radius, radius * 2.0, dist);
                h = ph + (h - ph) * t;
            }
        }
        h
    }

    /// Dark mare basalt (0..1): low, smooth plains.
    pub fn mare(&self, d: DVec3) -> f64 {
        crate::terrain::smoothstep(0.05, -0.25, self.noise.fbm(d * 2.0 + DVec3::splat(9.0), 3))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gravity_and_nearest_body() {
        let on_planet = DVec3::Y * (RADIUS + 2.0);
        assert_eq!(nearest(on_planet), Body::Planet);
        assert!((gravity(on_planet).length() - 9.81).abs() < 0.05);
        let on_moon = MOON_CENTER + DVec3::X * (MOON_RADIUS + 2.0);
        assert_eq!(nearest(on_moon), Body::Moon);
        assert!((gravity(on_moon).length() - MOON_G).abs() < 0.05);
        assert!(up_at(on_moon).dot(DVec3::X) > 0.999);
    }

    #[test]
    fn moon_heights_are_plausible() {
        let m = moon();
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for i in 0..2000 {
            let y = 1.0 - 2.0 * (i as f64 + 0.5) / 2000.0;
            let r = (1.0 - y * y).sqrt();
            let phi = i as f64 * 2.399963;
            let h = m.height(DVec3::new(r * phi.cos(), y, r * phi.sin()));
            lo = lo.min(h);
            hi = hi.max(h);
        }
        assert!(lo > -25_000.0 && hi < 25_000.0, "moon heights {lo}..{hi}");
        assert!(hi - lo > 2_000.0, "moon is too flat: {lo}..{hi}");
    }
}
