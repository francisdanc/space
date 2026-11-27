//! Planet-scale erosion: a landscape-evolution simulation on a cube-sphere grid.
//!
//! Each step:
//! 1. **Flow routing** with Priority-Flood (Barnes et al. 2014), seeded from the oceans: every
//!    land cell gets a downstream "receiver", depressions are routed through, and the visit
//!    order is a topological order from the coast upstream.
//! 2. **Drainage area**: each cell's catchment, accumulated downstream.
//! 3. **Stream-power incision**, solved implicitly (Braun & Willett 2013):
//!    dh/dt = -K A^m S, so big rivers cut deep valleys while small catchments barely change.
//! 4. **Hillslope diffusion**, softening slopes between valleys.
//!
//! The result is stored as a height *change* (plus a river "wetness" map) that the terrain adds
//! to its procedural height. The grid is coarse (cells of ~20 km), so this shapes river valleys
//! and drainage basins at planet scale; finer gullies come from eroded noise in terrain.rs.
//! Results are cached to disk per seed, since the simulation takes several seconds.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::{Read, Write};
use std::path::PathBuf;

use bevy::math::DVec3;

use crate::terrain::{RADIUS, dir_to_face_uv, face_dir, face_uv_of, neighbor_face};

/// Bump when the coarse terrain or the simulation changes, so stale caches are rebuilt.
const CACHE_VERSION: u32 = 3;
/// Cells of padding around each face (filled from the neighbouring faces) for seamless
/// bicubic sampling.
const PAD: usize = 4;
/// Within this many cells of a face edge, samples are blended with the neighbouring face.
const EDGE_BLEND: f64 = 1.5;

#[derive(Clone, Copy, Debug)]
pub struct ErosionParams {
    /// Cells per cube-face edge.
    pub n: usize,
    pub iterations: usize,
    /// Stream-power erodibility × time step.
    pub k_dt: f64,
    /// Drainage-area exponent.
    pub m: f64,
    /// Hillslope diffusion per iteration (0..0.25).
    pub diffusion: f64,
}

impl Default for ErosionParams {
    fn default() -> Self {
        Self { n: 512, iterations: 15, k_dt: 0.035, m: 0.5, diffusion: 0.015 }
    }
}

/// The simulation result, padded per face for sampling.
pub struct ErosionMap {
    n: usize,
    /// Height change in metres, 6 padded faces.
    delta: Vec<f32>,
    /// River wetness 0..1 (log drainage area), same layout.
    wetness: Vec<f32>,
}

/// Index of a cell on the unpadded simulation grid.
#[inline]
fn cell(n: usize, face: usize, i: usize, j: usize) -> usize {
    (face * n + j) * n + i
}

/// Face uv of a cell centre (may be outside [-1, 1] for virtual padding cells).
#[inline]
fn cell_uv(n: usize, i: i64, j: i64) -> (f64, f64) {
    let s = 2.0 / n as f64;
    (-1.0 + (i as f64 + 0.5) * s, -1.0 + (j as f64 + 0.5) * s)
}

/// Nearest grid cell to a direction.
fn dir_to_cell(n: usize, d: DVec3) -> usize {
    let (face, u, v) = dir_to_face_uv(d);
    let to_i = |t: f64| (((t + 1.0) * 0.5 * n as f64).floor() as i64).clamp(0, n as i64 - 1) as usize;
    cell(n, face as usize, to_i(u), to_i(v))
}

struct Grid {
    n: usize,
    dirs: Vec<DVec3>,
    /// Up to 8 neighbours per cell (u32::MAX = none) and the distance to each, in metres.
    neighbors: Vec<[u32; 8]>,
    dist: Vec<[f32; 8]>,
}

impl Grid {
    fn new(n: usize) -> Self {
        let total = 6 * n * n;
        let mut dirs = vec![DVec3::ZERO; total];
        for f in 0..6 {
            for j in 0..n {
                for i in 0..n {
                    let (u, v) = cell_uv(n, i as i64, j as i64);
                    dirs[cell(n, f, i, j)] = face_dir(f as u8, u, v);
                }
            }
        }
        let mut neighbors = vec![[u32::MAX; 8]; total];
        let mut dist = vec![[0.0f32; 8]; total];
        for f in 0..6 {
            for j in 0..n {
                for i in 0..n {
                    let c = cell(n, f, i, j);
                    let mut k = 0;
                    for dj in -1i64..=1 {
                        for di in -1i64..=1 {
                            if di == 0 && dj == 0 {
                                continue;
                            }
                            // Offsets past the face edge land on the neighbouring face.
                            let (u, v) = cell_uv(n, i as i64 + di, j as i64 + dj);
                            let nb = dir_to_cell(n, face_dir(f as u8, u, v));
                            if nb != c && !neighbors[c][..k].contains(&(nb as u32)) {
                                neighbors[c][k] = nb as u32;
                                dist[c][k] = (dirs[c].angle_between(dirs[nb]) * RADIUS) as f32;
                                k += 1;
                            }
                        }
                    }
                }
            }
        }
        Self { n, dirs, neighbors, dist }
    }
}

/// Order-preserving key for a float in a min-heap.
#[inline]
fn heap_key(h: f64, idx: usize) -> Reverse<(u64, u32)> {
    let b = h.to_bits();
    let ordered = if b >> 63 == 1 { !b } else { b | (1 << 63) };
    Reverse((ordered, idx as u32))
}

/// Priority-Flood from the ocean: returns (receiver per cell, visit order).
fn route(grid: &Grid, h: &[f64]) -> (Vec<u32>, Vec<u32>) {
    let (receiver, order, _) = route_levels(grid, h);
    (receiver, order)
}

/// Priority-Flood that also returns the filled surface: the level water would stand at in each
/// cell (above the ground in depressions, i.e. lakes).
fn route_levels(grid: &Grid, h: &[f64]) -> (Vec<u32>, Vec<u32>, Vec<f64>) {
    let total = h.len();
    let mut receiver = vec![u32::MAX; total];
    let mut visited = vec![false; total];
    let mut level = h.to_vec();
    let mut order = Vec::with_capacity(total);
    let mut heap = BinaryHeap::with_capacity(total / 4);
    for (c, &hc) in h.iter().enumerate() {
        if hc <= 0.0 {
            visited[c] = true;
            heap.push(heap_key(hc, c));
        }
    }
    if heap.is_empty() {
        // No ocean: drain everything towards the lowest point.
        let lowest = (0..total).min_by(|&a, &b| h[a].total_cmp(&h[b])).unwrap();
        visited[lowest] = true;
        heap.push(heap_key(h[lowest], lowest));
    }
    while let Some(Reverse((_, c))) = heap.pop() {
        let c = c as usize;
        order.push(c as u32);
        for &nb in &grid.neighbors[c] {
            if nb == u32::MAX {
                continue;
            }
            let nb = nb as usize;
            if !visited[nb] {
                visited[nb] = true;
                receiver[nb] = c as u32;
                // Fill depressions just enough to keep draining downhill.
                level[nb] = level[nb].max(level[c] + 1e-3);
                heap.push(heap_key(level[nb], nb));
            }
        }
    }
    (receiver, order, level)
}

/// Drainage on the final (eroded) coarse terrain, per simulation cell.
pub struct Hydrology {
    pub n: usize,
    /// Cell centre directions.
    pub dirs: Vec<DVec3>,
    /// Final coarse height (m).
    pub height: Vec<f64>,
    /// Filled water level (m): above `height` in lakes.
    pub level: Vec<f64>,
    /// Downstream cell (u32::MAX for the sea).
    pub receiver: Vec<u32>,
    /// Upstream catchment in cells.
    pub area: Vec<f64>,
}

/// Run the landscape-evolution model on heights from `height_fn`. Returns
/// (initial heights, final heights, drainage area in cells).
fn simulate(
    params: &ErosionParams,
    grid: &Grid,
    height_fn: &(dyn Fn(DVec3) -> f64 + Sync),
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let n = grid.n;
    let total = 6 * n * n;
    // Sample the initial terrain, one thread per face.
    let mut h0 = vec![0.0f64; total];
    std::thread::scope(|s| {
        for (f, chunk) in h0.chunks_mut(n * n).enumerate() {
            let dirs = &grid.dirs[f * n * n..(f + 1) * n * n];
            s.spawn(move || {
                for (out, d) in chunk.iter_mut().zip(dirs) {
                    *out = height_fn(*d);
                }
            });
        }
    });

    let mut h = h0.clone();
    let mut area = vec![1.0f64; total];
    let cell_area = (RADIUS * std::f64::consts::FRAC_PI_2 / n as f64).powi(2);
    for _ in 0..params.iterations {
        let (receiver, order) = route(grid, &h);

        // Drainage area, accumulated from the headwaters down.
        area.iter_mut().for_each(|a| *a = 1.0);
        for &c in order.iter().rev() {
            let r = receiver[c as usize];
            if r != u32::MAX {
                area[r as usize] += area[c as usize];
            }
        }

        // Implicit stream-power incision, from the coast upstream.
        for &c in &order {
            let c = c as usize;
            let r = receiver[c];
            if r == u32::MAX || h0[c] <= 0.0 {
                continue;
            }
            let r = r as usize;
            let base = h[r].max(0.0);
            if h[c] <= base {
                continue;
            }
            let k = grid.neighbors[c].iter().position(|&x| x as usize == r).unwrap();
            let len = grid.dist[c][k] as f64;
            let f = params.k_dt * (area[c] * cell_area).powf(params.m) / len;
            h[c] = (h[c] + f * base) / (1.0 + f);
        }

        // Hillslope diffusion on land.
        let prev = h.clone();
        for c in 0..total {
            if h0[c] <= 0.0 {
                continue;
            }
            let mut sum = 0.0;
            let mut cnt = 0.0;
            for &nb in &grid.neighbors[c] {
                if nb != u32::MAX {
                    sum += prev[nb as usize].max(0.0);
                    cnt += 1.0;
                }
            }
            h[c] += params.diffusion * (sum / cnt - prev[c]);
        }
    }
    (h0, h, area)
}

impl ErosionMap {
    pub fn generate(params: ErosionParams, height_fn: &(dyn Fn(DVec3) -> f64 + Sync)) -> Self {
        let grid = Grid::new(params.n);
        let (h0, h, area) = simulate(&params, &grid, height_fn);
        let n = params.n;
        let wet: Vec<f64> = area.iter().map(|a| (a.log10() / 4.0).clamp(0.0, 1.0)).collect();
        let delta: Vec<f64> = h.iter().zip(&h0).map(|(a, b)| if *b > 0.0 { a - b } else { 0.0 }).collect();

        // Pad every face with values from its neighbours for seamless sampling.
        let p = n + 2 * PAD;
        let mut pdelta = vec![0.0f32; 6 * p * p];
        let mut pwet = vec![0.0f32; 6 * p * p];
        for f in 0..6 {
            for j in 0..p {
                for i in 0..p {
                    let (gi, gj) = (i as i64 - PAD as i64, j as i64 - PAD as i64);
                    let (dv, wv) = if (0..n as i64).contains(&gi) && (0..n as i64).contains(&gj) {
                        let c = cell(n, f, gi as usize, gj as usize);
                        (delta[c], wet[c])
                    } else {
                        // Interpolate the neighbouring face's own grid at this exact spot, so
                        // both sides of a cube edge see the same field.
                        let (u, v) = cell_uv(n, gi, gj);
                        let (nf, nu, nv) = dir_to_face_uv(face_dir(f as u8, u, v));
                        (bilinear(n, &delta, nf as usize, nu, nv), bilinear(n, &wet, nf as usize, nu, nv))
                    };
                    pdelta[(f * p + j) * p + i] = dv as f32;
                    pwet[(f * p + j) * p + i] = wv as f32;
                }
            }
        }
        Self { n, delta: pdelta, wetness: pwet }
    }

    /// Route water over the final coarse terrain (`coarse` is the simulation's input height):
    /// receivers, drainage areas and the filled water level of every cell.
    pub fn hydrology(&self, coarse: &(dyn Fn(DVec3) -> f64 + Sync)) -> Hydrology {
        let n = self.n;
        let grid = Grid::new(n);
        let p = n + 2 * PAD;
        let total = 6 * n * n;
        let mut height = vec![0.0f64; total];
        std::thread::scope(|s| {
            for (f, chunk) in height.chunks_mut(n * n).enumerate() {
                let dirs = &grid.dirs[f * n * n..(f + 1) * n * n];
                let delta = &self.delta;
                s.spawn(move || {
                    for (k, (out, d)) in chunk.iter_mut().zip(dirs).enumerate() {
                        let (i, j) = (k % n, k / n);
                        let h0 = coarse(*d);
                        *out = if h0 > 0.0 { h0 + delta[(f * p + j + PAD) * p + i + PAD] as f64 } else { h0 };
                    }
                });
            }
        });
        let (receiver, order, level) = route_levels(&grid, &height);
        let mut area = vec![1.0f64; total];
        for &c in order.iter().rev() {
            let r = receiver[c as usize];
            if r != u32::MAX {
                area[r as usize] += area[c as usize];
            }
        }
        if std::env::var("HYDRO_STATS").is_ok() {
            let land = height.iter().filter(|h| **h > 0.0).count();
            for th in [4.0, 10.0, 25.0, 50.0, 100.0, 200.0] {
                let c = (0..total).filter(|&c| height[c] > 0.0 && level[c] - height[c] >= th).count();
                println!("depressions >= {th} m: {c} cells of {land} land cells");
            }
        }
        Hydrology { n, dirs: grid.dirs, height, level, receiver, area }
    }

    /// Height change (m) and river wetness (0..1) at a direction, smoothly interpolated.
    pub fn sample(&self, d: DVec3) -> (f64, f64) {
        let (face, u, v) = dir_to_face_uv(d);
        let mut val = self.sample_face(face, u, v);
        // Near a cube edge, blend 50/50 (exactly at the edge) with the neighbouring face's own
        // interpolation, so both sides agree and there is no seam.
        let n = self.n as f64;
        for (coord, axis) in [(u, 0usize), (v, 1usize)] {
            let cells_to_edge = (1.0 - coord.abs()) * 0.5 * n;
            if cells_to_edge < EDGE_BLEND {
                let other = neighbor_face(face, axis, coord.signum());
                let (ou, ov) = face_uv_of(other, d);
                let o = self.sample_face(other, ou, ov);
                let w = 0.5 + 0.5 * smooth(cells_to_edge / EDGE_BLEND);
                val = (val.0 * w + o.0 * (1.0 - w), val.1 * w + o.1 * (1.0 - w));
            }
        }
        (val.0, val.1.clamp(0.0, 1.0))
    }

    /// Bicubic sample on one (padded) face; (u, v) may lie a little outside [-1, 1].
    fn sample_face(&self, face: u8, u: f64, v: f64) -> (f64, f64) {
        let n = self.n as f64;
        let x = (u + 1.0) * 0.5 * n - 0.5 + PAD as f64;
        let y = (v + 1.0) * 0.5 * n - 0.5 + PAD as f64;
        let (ix, iy) = (x.floor(), y.floor());
        let (fx, fy) = (x - ix, y - iy);
        let p = self.n + 2 * PAD;
        let base = face as usize * p * p;
        let (ix, iy) = (ix as i64, iy as i64);
        let wx = catmull_rom(fx);
        let wy = catmull_rom(fy);
        let (mut dsum, mut wsum) = (0.0, 0.0);
        for (bj, wyj) in wy.iter().enumerate() {
            let jj = (iy - 1 + bj as i64).clamp(0, p as i64 - 1) as usize;
            for (bi, wxi) in wx.iter().enumerate() {
                let ii = (ix - 1 + bi as i64).clamp(0, p as i64 - 1) as usize;
                let w = wxi * wyj;
                dsum += w * self.delta[base + jj * p + ii] as f64;
                wsum += w * self.wetness[base + jj * p + ii] as f64;
            }
        }
        (dsum, wsum)
    }

    fn cache_path(seed: u64, n: usize) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("cache")
            .join(format!("erosion_{seed}_{n}_v{CACHE_VERSION}.bin"))
    }

    fn save(&self, seed: u64) -> std::io::Result<()> {
        let path = Self::cache_path(seed, self.n);
        std::fs::create_dir_all(path.parent().unwrap())?;
        let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
        f.write_all(b"ERO1")?;
        f.write_all(&(self.n as u32).to_le_bytes())?;
        for v in self.delta.iter().chain(&self.wetness) {
            f.write_all(&v.to_le_bytes())?;
        }
        Ok(())
    }

    fn load(seed: u64, n: usize) -> Option<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(Self::cache_path(seed, n)).ok()?.read_to_end(&mut bytes).ok()?;
        let p = n + 2 * PAD;
        let len = 6 * p * p;
        if bytes.len() != 8 + 8 * len || &bytes[..4] != b"ERO1" || bytes[4..8] != (n as u32).to_le_bytes() {
            return None;
        }
        let floats: Vec<f32> =
            bytes[8..].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        Some(Self { n, delta: floats[..len].to_vec(), wetness: floats[len..].to_vec() })
    }

    /// Load the cached map for this planet or run the simulation (and cache it).
    pub fn load_or_generate(seed: u64, params: ErosionParams, height_fn: &(dyn Fn(DVec3) -> f64 + Sync)) -> Self {
        if let Some(map) = Self::load(seed, params.n) {
            return map;
        }
        println!("Simulating erosion for planet {seed} ({}x{} cells per face, first run only)...", params.n, params.n);
        let start = std::time::Instant::now();
        let map = Self::generate(params, height_fn);
        println!("Erosion simulated in {:.1} s", start.elapsed().as_secs_f64());
        if let Err(e) = map.save(seed) {
            eprintln!("could not cache erosion map: {e}");
        }
        map
    }
}

/// Bilinear sample of an unpadded per-cell field on one face (clamped at the face border).
fn bilinear(n: usize, field: &[f64], face: usize, u: f64, v: f64) -> f64 {
    let x = ((u + 1.0) * 0.5 * n as f64 - 0.5).clamp(0.0, n as f64 - 1.0);
    let y = ((v + 1.0) * 0.5 * n as f64 - 0.5).clamp(0.0, n as f64 - 1.0);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(n - 1), (y0 + 1).min(n - 1));
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let at = |i: usize, j: usize| field[cell(n, face, i, j)];
    let a = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
    let b = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
    a + (b - a) * fy
}

#[inline]
fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Catmull-Rom weights for the 4 samples around fraction t.
#[inline]
fn catmull_rom(t: f64) -> [f64; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::Terrain;

    fn small() -> ErosionParams {
        ErosionParams { n: 96, ..Default::default() }
    }

    #[test]
    fn every_land_cell_drains_to_the_sea() {
        let t = Terrain::new(1337);
        let grid = Grid::new(32);
        let h: Vec<f64> = grid.dirs.iter().map(|d| t.coarse_height(*d)).collect();
        let (receiver, order) = route(&grid, &h);
        assert_eq!(order.len(), h.len(), "flood must reach every cell");
        for c in 0..h.len() {
            // Follow receivers: must end at an ocean cell without looping.
            let mut x = c;
            let mut steps = 0;
            while receiver[x] != u32::MAX {
                x = receiver[x] as usize;
                steps += 1;
                assert!(steps < h.len(), "routing loop");
            }
            assert!(h[x] <= 0.0, "cell {c} drains to a non-ocean cell");
        }
    }

    #[test]
    fn erosion_carves_valleys_and_never_raises_mountains() {
        let t = Terrain::new(1337);
        let grid = Grid::new(small().n);
        let (h0, h, area) = simulate(&small(), &grid, &|d| t.coarse_height(d));
        let land: Vec<usize> = (0..h0.len()).filter(|&c| h0[c] > 0.0).collect();
        let max_cut = land.iter().map(|&c| h0[c] - h[c]).fold(0.0, f64::max);
        let mean_cut = land.iter().map(|&c| h0[c] - h[c]).sum::<f64>() / land.len() as f64;
        let peak_raise = land.iter().map(|&c| h[c] - h0[c]).fold(f64::MIN, f64::max);
        // Among uplands, cells with big catchments (rivers) lose a larger share of their height
        // than cells with small ones (ridges).
        let upland: Vec<usize> = land.iter().copied().filter(|&c| h0[c] > 300.0).collect();
        let frac = |cs: &[usize]| cs.iter().map(|&c| (h0[c] - h[c]) / h0[c]).sum::<f64>() / cs.len() as f64;
        let mut by_area = upland.clone();
        by_area.sort_by(|&a, &b| area[b].total_cmp(&area[a]));
        let k = (by_area.len() / 10).max(1);
        let river = frac(&by_area[..k]);
        let ridge = frac(&by_area[by_area.len() - k..]);
        println!(
            "max cut {max_cut:.0} m, mean cut {mean_cut:.0} m, max raise {peak_raise:.0} m,              upland rivers lose {:.0}% vs ridges {:.0}%",
            river * 100.0,
            ridge * 100.0
        );
        assert!(max_cut > 300.0, "erosion too weak");
        assert!(river > ridge * 1.2, "rivers should cut deeper than ridges");
        // Diffusion can fill valleys a little, but nothing should be built up much.
        assert!(peak_raise < 600.0);
    }

    #[test]
    fn sampling_is_seamless_across_cube_edges() {
        let t = Terrain::new(1337);
        let map = ErosionMap::generate(small(), &|d| t.coarse_height(d));
        // Compare samples a small step apart: straddling a face edge, and (as a baseline for
        // how much the field normally changes over that distance) inside a face.
        let step = |a: DVec3, b: DVec3| (map.sample(a).0 - map.sample(b).0).abs();
        let mut edge_worst: f64 = 0.0;
        let mut inner_worst: f64 = 0.0;
        for f in 0..6u8 {
            for k in 0..60 {
                let s = -0.8 + k as f64 * (1.6 / 59.0);
                for (u, v) in [(1.0, s), (-1.0, s), (s, 1.0), (s, -1.0)] {
                    let edge = face_dir(f, u, v);
                    let inward = (face_dir(f, u * 0.999, v * 0.999) - edge).normalize() * 2e-4;
                    edge_worst = edge_worst.max(step((edge + inward).normalize(), (edge - inward).normalize()));
                    let inner = face_dir(f, u * 0.5, v * 0.5);
                    inner_worst = inner_worst.max(step((inner + inward).normalize(), (inner - inward).normalize()));
                }
            }
        }
        println!("worst change over 2.5 km: across face edges {edge_worst:.1} m, inside faces {inner_worst:.1} m");
        assert!(edge_worst < inner_worst * 2.0 + 10.0, "seam across a face edge");
    }
}

#[cfg(test)]
mod map_image {
    use super::*;
    use crate::terrain::Terrain;

    /// Writes shots/erosion_map.ppm: a shaded relief map of a mountainous region before (left)
    /// and after (middle) erosion, and with the river network (right).
    /// `cargo test --release erosion_map_image -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn erosion_map_image() {
        let plain = Terrain::new(crate::planet::SEED);
        let eroded = Terrain::with_erosion(crate::planet::SEED);
        // Centre on the most mountainous land.
        let mut center = DVec3::Y;
        let mut best = f64::MIN;
        for k in 0..20_000 {
            let y = 1.0 - 2.0 * (k as f64 + 0.5) / 20_000.0;
            let r = (1.0 - y * y).sqrt();
            let d = DVec3::new(r * (k as f64 * 2.4).cos(), y, r * (k as f64 * 2.4).sin());
            let h = plain.coarse_height(d);
            if h > best {
                best = h;
                center = d;
            }
        }
        let (w, h) = (640usize, 384usize);
        let span = 3_000_000.0; // metres across
        let e1 = center.any_orthonormal_vector();
        let e2 = center.cross(e1);
        let dir = |x: usize, y: usize| {
            let px = (x as f64 / w as f64 - 0.5) * span;
            let py = (y as f64 / h as f64 - 0.5) * span * h as f64 / w as f64;
            (center * RADIUS + e1 * px + e2 * py).normalize()
        };
        let before: Vec<f64> = (0..w * h).map(|i| plain.coarse_height(dir(i % w, i / w))).collect();
        let after: Vec<(f64, f64)> = (0..w * h)
            .map(|i| {
                let d = dir(i % w, i / w);
                let (delta, wet) = eroded.erosion_at(d);
                (plain.coarse_height(d) + delta, wet)
            })
            .collect();
        let pixel_m = span / w as f64;
        let shade = |hs: &dyn Fn(usize) -> f64, x: usize, y: usize| -> [u8; 3] {
            let i = y * w + x;
            let hx = hs(y * w + (x + 1).min(w - 1)) - hs(y * w + x.saturating_sub(1));
            let hy = hs((y + 1).min(h - 1) * w + x) - hs(y.saturating_sub(1) * w + x);
            let n = DVec3::new(-hx * 6.0, -hy * 6.0, 2.0 * pixel_m).normalize();
            let light = n.dot(DVec3::new(-0.6, -0.6, 0.55).normalize()).max(0.0) * 0.85 + 0.15;
            let v = hs(i);
            let c = if v <= 0.0 {
                [30.0, 60.0, 120.0]
            } else {
                let t = (v / 6000.0).clamp(0.0, 1.0);
                [80.0 + 150.0 * t, 120.0 + 90.0 * t, 60.0 + 170.0 * t]
            };
            let k = if v <= 0.0 { 1.0 } else { light };
            [(c[0] * k) as u8, (c[1] * k) as u8, (c[2] * k) as u8]
        };
        let mut img = vec![0u8; w * 3 * h * 3];
        for y in 0..h {
            for x in 0..w {
                let a = shade(&|i| before[i], x, y);
                let b = shade(&|i| after[i].0, x, y);
                let mut c = b;
                let wet = after[y * w + x].1;
                if after[y * w + x].0 > 0.0 && wet > 0.5 {
                    let t = ((wet - 0.5) / 0.3).clamp(0.0, 1.0);
                    c = [(c[0] as f64 * (1.0 - t) + 30.0 * t) as u8, (c[1] as f64 * (1.0 - t) + 90.0 * t) as u8, (c[2] as f64 * (1.0 - t) + 220.0 * t) as u8];
                }
                for (panel, px) in [a, b, c].iter().enumerate() {
                    let o = (y * w * 3 + panel * w + x) * 3;
                    img[o..o + 3].copy_from_slice(px);
                }
            }
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shots/erosion_map.ppm");
        let mut out = format!("P6\n{} {}\n255\n", w * 3, h).into_bytes();
        out.extend_from_slice(&img);
        std::fs::write(&path, out).unwrap();
        println!("wrote {}", path.display());
    }
}
