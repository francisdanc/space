//! Rivers and lakes, from the drainage of the eroded planet (erosion.rs).
//!
//! **Rivers** follow the flow graph of the simulation grid (cells of ~20 km): every cell whose
//! catchment is big enough is joined to its downstream neighbour by a segment. Width and depth
//! grow with the square root of the catchment, and the water level falls monotonically
//! downstream (it is the Priority-Flood level). Segments are straight on the grid; a smooth
//! domain warp of the query point makes them meander.
//!
//! **Lakes** are the depressions Priority-Flood had to fill: cells whose water level stands above
//! the ground. Their shores come from a smooth weight around the lake cells and the terrain.
//!
//! `carve` shapes the terrain (channel, banks, floodplain, lake beds) inside `Terrain::height`,
//! so every user of the terrain (meshes, physics, plants) sees the same riverbeds, and `water`
//! tells where inland water stands and how it flows (water meshes, swimming, floating).

use bevy::math::DVec3;

use crate::erosion::Hydrology;
use crate::noise::Perlin;
use crate::terrain::{RADIUS, dir_to_face_uv, smoothstep};

/// Smallest catchment (in ~400 km² cells) that makes a river.
const MIN_AREA: f64 = 24.0;
/// A filled depression counts as a lake where water stands at least this deep (m).
const MIN_LAKE_DEPTH: f64 = 15.0;
/// Basins in drier climates than this stay dry (salt flats, playas).
const MIN_LAKE_MOISTURE: f32 = 0.45;
/// Meander warp: (amplitude in m, wavelength in m) per octave.
const MEANDERS: [(f64, f64); 3] = [(3500.0, 40_000.0), (900.0, 8_000.0), (200.0, 1_500.0)];

#[derive(Clone, Copy, Debug)]
struct Segment {
    a: DVec3,
    b: DVec3,
    level: [f32; 2],
    half_width: [f32; 2],
    depth: [f32; 2],
}

/// Inland water at a point.
#[derive(Clone, Copy, Debug)]
pub struct Water {
    /// Height of the water surface above sea level (m).
    pub level: f64,
    /// Downstream direction (unit, tangent to the surface); zero on lakes.
    pub flow: DVec3,
    /// River gradient (m per m): steep stretches are white water.
    pub slope: f64,
    /// River width (m); 0 on lakes.
    pub width: f64,
}

/// Nearest river to a point.
#[derive(Clone, Copy, Debug)]
struct RiverHit {
    dist: f64,
    level: f64,
    half_width: f64,
    depth: f64,
    flow: DVec3,
    slope: f64,
}

pub struct Hydro {
    n: usize,
    segments: Vec<Segment>,
    /// Spatial index (CSR): segments that pass near each grid cell.
    cell_start: Vec<u32>,
    cell_items: Vec<u32>,
    /// Lake water level per grid cell (NaN: no lake).
    lake: Vec<f32>,
    cell_dirs: Vec<DVec3>,
    warp: Perlin,
    pub river_count: usize,
    pub lake_cells: usize,
}

fn cell_of(n: usize, d: DVec3) -> usize {
    let (face, u, v) = dir_to_face_uv(d);
    let to_i = |t: f64| (((t + 1.0) * 0.5 * n as f64).floor() as i64).clamp(0, n as i64 - 1) as usize;
    (face as usize * n + to_i(v)) * n + to_i(u)
}

/// River width (m) for a catchment of `area` cells (each ~400 km²).
fn width_for(area: f64) -> f64 {
    (14.0 * area.sqrt()).min(2500.0)
}

impl Hydro {
    pub fn build(h: &Hydrology, seed: u64, moisture: &(dyn Fn(DVec3) -> f32 + Sync)) -> Self {
        let n = h.n;
        let total = 6 * n * n;
        let mut segments = Vec::new();
        let mut lake = vec![f32::NAN; total];
        let mut lake_cells = 0;
        for c in 0..total {
            if h.height[c] <= 0.0 {
                continue;
            }
            if h.level[c] - h.height[c] >= MIN_LAKE_DEPTH && moisture(h.dirs[c]) >= MIN_LAKE_MOISTURE {
                lake[c] = h.level[c] as f32;
                lake_cells += 1;
            }
            let r = h.receiver[c];
            if r == u32::MAX || h.area[c] < MIN_AREA {
                continue;
            }
            let r = r as usize;
            let w0 = width_for(h.area[c]);
            let w1 = width_for(h.area[r].max(h.area[c]));
            let level = |k: usize| if h.height[k] <= 0.0 { 0.0 } else { h.level[k] };
            segments.push(Segment {
                a: h.dirs[c],
                b: h.dirs[r],
                level: [level(c) as f32, level(r) as f32],
                half_width: [(w0 * 0.5) as f32, (w1 * 0.5) as f32],
                depth: [(1.5 + w0 * 0.02) as f32, (1.5 + w1 * 0.02) as f32],
            });
        }
        // Spatial index: each segment is listed in the 3x3 cells around both of its ends.
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); total];
        let step = 2.0 / n as f64;
        for (k, s) in segments.iter().enumerate() {
            for end in [s.a, s.b] {
                let (face, u, v) = dir_to_face_uv(end);
                for dj in -1..=1 {
                    for di in -1..=1 {
                        let d = crate::terrain::face_dir(face, u + di as f64 * step, v + dj as f64 * step);
                        let c = cell_of(n, d);
                        if !lists[c].contains(&(k as u32)) {
                            lists[c].push(k as u32);
                        }
                    }
                }
            }
        }
        let mut cell_start = Vec::with_capacity(total + 1);
        let mut cell_items = Vec::new();
        for l in &lists {
            cell_start.push(cell_items.len() as u32);
            cell_items.extend_from_slice(l);
        }
        cell_start.push(cell_items.len() as u32);
        let river_count = segments.len();
        Self {
            n,
            segments,
            cell_start,
            cell_items,
            lake,
            cell_dirs: h.dirs.clone(),
            warp: Perlin::new(seed.wrapping_add(77)),
            river_count,
            lake_cells,
        }
    }

    /// The query point moved by the meander warp.
    fn warped(&self, d: DVec3) -> DVec3 {
        let mut off = DVec3::ZERO;
        for (k, &(amp, wavelength)) in MEANDERS.iter().enumerate() {
            let f = RADIUS / wavelength;
            let p = d * f + DVec3::splat(k as f64 * 19.3);
            let v = DVec3::new(
                self.warp.fbm(p, 2),
                self.warp.fbm(p + DVec3::new(31.4, 0.0, 0.0), 2),
                self.warp.fbm(p + DVec3::new(0.0, 47.1, 0.0), 2),
            );
            off += v * (amp / RADIUS);
        }
        (d + off).normalize()
    }

    fn river(&self, dw: DVec3) -> Option<RiverHit> {
        let c = cell_of(self.n, dw);
        let items = &self.cell_items[self.cell_start[c] as usize..self.cell_start[c + 1] as usize];
        let p = dw * RADIUS;
        let mut best: Option<RiverHit> = None;
        for &k in items {
            let s = &self.segments[k as usize];
            let (a, b) = (s.a * RADIUS, s.b * RADIUS);
            let ab = b - a;
            let len2 = ab.length_squared().max(1.0);
            let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
            let dist = (p - (a + ab * t)).length();
            if best.is_none_or(|h| dist < h.dist) {
                let lerp = |v: [f32; 2]| v[0] as f64 + (v[1] as f64 - v[0] as f64) * t;
                let len = len2.sqrt();
                let up = dw;
                let flow = (ab - up * ab.dot(up)).normalize_or_zero();
                best = Some(RiverHit {
                    dist,
                    level: lerp(s.level),
                    half_width: lerp(s.half_width),
                    depth: lerp(s.depth),
                    flow,
                    slope: ((s.level[0] - s.level[1]) as f64 / len).max(0.0),
                });
            }
        }
        best
    }

    /// Lake weight (0..1, shore at 0.5) and level around a point.
    fn lake(&self, dw: DVec3) -> Option<(f64, f64)> {
        let c = cell_of(self.n, dw);
        let n = self.n;
        let cell_m = RADIUS * std::f64::consts::FRAC_PI_2 / n as f64;
        let (face, u, v) = dir_to_face_uv(dw);
        let step = 2.0 / n as f64;
        let mut best: Option<(f64, f64)> = None;
        for dj in -1..=1 {
            for di in -1..=1 {
                let k = if di == 0 && dj == 0 { c } else { cell_of(n, crate::terrain::face_dir(face, u + di as f64 * step, v + dj as f64 * step)) };
                let level = self.lake[k];
                if level.is_nan() {
                    continue;
                }
                let dist = (self.cell_dirs[k] - dw).length() * RADIUS;
                let w = smoothstep(cell_m * 0.95, cell_m * 0.35, dist);
                if w > 0.0 && best.is_none_or(|b| w > b.0) {
                    best = Some((w, level as f64));
                }
            }
        }
        best
    }

    /// The terrain height at `d` with riverbeds, banks and lake beds shaped into `h`.
    pub fn carve(&self, d: DVec3, h: f64) -> f64 {
        let dw = self.warped(d);
        let mut out = h;
        if let Some(r) = self.river(dw) {
            let (w, l) = (r.half_width, r.level);
            // The valley widens where the river cuts through high ground, so its walls stay walkable.
            let valley = (w * 6.0).max(400.0) + (h - l).max(0.0) * 2.5;
            if r.dist < valley && l > 0.0 {
                let x = r.dist;
                let profile = if x < w {
                    l - r.depth * (1.0 - (x / w).powi(2))
                } else {
                    l + ((x - w) * 0.05).min(6.0)
                };
                let b = 1.0 - smoothstep(w * 1.5, valley, x);
                out = out + (out.min(profile) - out) * b;
                // Keep a low bank so the water never stands above the ground next to it.
                if x >= w && x < w * 2.0 {
                    out = out.max(l + 0.4);
                }
                if x < w {
                    out = out.min(profile);
                }
            }
        }
        if let Some((wt, level)) = self.lake(dw) {
            // Basin profile by lake weight: the bed below the water inside (shore at 0.5), and
            // around it ground rising gently away from the shore (~400 m over the outer band),
            // which hills higher than that are lowered to, so shores are slopes, not cliffs.
            let outer = ((0.35 - wt) / 0.35).clamp(0.0, 1.0);
            let profile = level + 2.0 - 4.0 * smoothstep(0.35, 0.65, wt) - 22.0 * smoothstep(0.65, 1.0, wt)
                + 400.0 * outer.powf(1.5);
            out += (out.min(profile) - out) * smoothstep(0.0, 0.12, wt);
            // Rim: ground lower than the water just outside the lake is raised to the shore.
            if wt < 0.5 {
                out += (out.max(profile.min(level + 2.0)) - out) * smoothstep(0.05, 0.3, wt);
            }
        }
        out
    }

    /// How strongly the ground at `d` is watered by a nearby river or lake (0..1): riverbanks
    /// and lakeshores are green even in dry country.
    pub fn riparian(&self, d: DVec3) -> f64 {
        let dw = self.warped(d);
        let mut w = 0.0f64;
        if let Some(r) = self.river(dw) {
            if r.level > 0.0 {
                let reach = 120.0 + r.half_width * 4.0;
                w = (-(r.dist - r.half_width).max(0.0) / reach).exp();
            }
        }
        if let Some((wt, _)) = self.lake(dw) {
            w = w.max(smoothstep(0.15, 0.5, wt));
        }
        w
    }

    /// Inland water at `d`, if any: the water surface stands at `level` wherever the ground (as
    /// carved) is below it.
    pub fn water(&self, d: DVec3) -> Option<Water> {
        let dw = self.warped(d);
        if let Some((wt, level)) = self.lake(dw) {
            if wt > 0.5 {
                return Some(Water { level, flow: DVec3::ZERO, slope: 0.0, width: 0.0 });
            }
        }
        let r = self.river(dw)?;
        (r.dist < r.half_width * 1.6 && r.level > 0.0).then_some(Water { level: r.level, flow: r.flow, slope: r.slope, width: r.half_width * 2.0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prints the river network and lake statistics of the real planet.
    #[test]
    #[ignore]
    fn hydro_stats() {
        let t = crate::terrain::Terrain::with_erosion(crate::planet::SEED);
        let hy = t.hydro.as_ref().unwrap();
        println!("river segments: {}, lake cells: {}", hy.river_count, hy.lake_cells);
        let widths: Vec<f32> = hy.segments.iter().map(|s| s.half_width[0] * 2.0).collect();
        let max = widths.iter().cloned().fold(0.0, f32::max);
        println!("widest river: {max:.0} m");
        let lakes: Vec<f32> = hy.lake.iter().filter(|l| !l.is_nan()).cloned().collect();
        println!("lake cells with level: {}", lakes.len());
    }
}
