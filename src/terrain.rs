//! Planet surface: height function, cube-sphere chunk addressing and chunk mesh generation.
//! Everything here is pure CPU code (no Bevy ECS) so it runs on worker threads and is unit-testable.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};
use std::sync::Arc;

use bevy::math::DVec3;

use crate::erosion::{ErosionMap, ErosionParams};
use crate::flora::{FloraContext, FloraMesh, SurfaceGrid, build_chunk_flora};
use crate::noise::Perlin;

/// Planet radius in metres (Earth-sized).
pub const RADIUS: f64 = 6_371_000.0;
/// Quads per chunk edge.
pub const N: usize = 32;
/// Deepest quadtree level. Level 18 gives ~1.2 m between vertices.
pub const MAX_LEVEL: u8 = 18;

pub struct Terrain {
    continents: Perlin,
    mountains: Perlin,
    detail: Perlin,
    climate: Perlin,
    /// Planet-scale erosion (river valleys); `None` for quick tests.
    erosion: Option<Arc<ErosionMap>>,
    /// Rivers and lakes (from the erosion's drainage); `None` for quick tests.
    pub hydro: Option<Arc<crate::hydro::Hydro>>,
    /// Levelled pads (direction, height, radius m), e.g. under bases.
    pads: Vec<(DVec3, f64, f64)>,
}

/// Climate and landscape at a point, independent of level of detail (so plant placement never
/// changes when terrain chunks split or merge).
#[derive(Clone, Copy, Debug, Default)]
pub struct Ecology {
    /// Mean temperature, degrees C.
    pub temperature: f32,
    /// 0 (arid) .. 1 (very wet).
    pub moisture: f32,
    /// Large-scale height above sea level (m), without fine detail.
    pub height: f32,
    /// Closeness to big rivers, 0..1.
    pub wetness: f32,
    /// Closeness to the coast, 0..1.
    pub coast: f32,
}

#[inline]
pub fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Terrain {
    /// Terrain without planet-scale erosion (fast to create; used by tests and as the input to
    /// the erosion simulation).
    pub fn new(seed: u64) -> Self {
        Self {
            continents: Perlin::new(seed),
            mountains: Perlin::new(seed.wrapping_add(1)),
            detail: Perlin::new(seed.wrapping_add(2)),
            climate: Perlin::new(seed.wrapping_add(3)),
            erosion: None,
            hydro: None,
            pads: Vec::new(),
        }
    }

    /// The planet as played: terrain plus simulated erosion (cached on disk after the first run).
    pub fn with_erosion(seed: u64) -> Self {
        let base = Terrain::new(seed);
        let map = ErosionMap::load_or_generate(seed, ErosionParams::default(), &|d| base.coarse_height(d));
        let start = std::time::Instant::now();
        let hydro = crate::hydro::Hydro::build(&map.hydrology(&|d| base.coarse_height(d)), seed, &|d| base.ecology(d).moisture);
        println!("Rivers and lakes: {} river segments, {} lake cells ({:.1} s)", hydro.river_count, hydro.lake_cells, start.elapsed().as_secs_f64());
        Self { erosion: Some(Arc::new(map)), hydro: Some(Arc::new(hydro)), ..base }
    }

    /// Continental shape: below 0 is ocean, rising inland. Also returns the land and
    /// mountain-belt masks.
    fn continental(&self, d: DVec3) -> (f64, f64, f64) {
        let c = self.continents.fbm(d * 1.1, 8) * 2.2 + 0.05;
        let land = smoothstep(-0.05, 0.25, c);
        let belt = smoothstep(0.0, 0.3, self.continents.fbm(d * 2.5 + DVec3::splat(31.7), 4) + 0.05);
        (c, land, belt)
    }

    /// Large-scale relief: continents, mountain ranges and rolling hills.
    fn large_scale(&self, d: DVec3, cl: (f64, f64, f64), mountain_octaves: u32, hill_octaves: u32) -> f64 {
        let (c, land, belt) = cl;
        let mut h = c * 2500.0;
        let r = self.mountains.ridged(d * 12.0, mountain_octaves);
        h += r * r * 9000.0 * land * belt;
        // Rolling hills (~80 km), with eroded noise: smooth slopes, sharper crests.
        h += self.detail.eroded_fbm(d * 80.0, hill_octaves, d, 1.0) * 1300.0 * (0.3 + land);
        h
    }

    /// The terrain the erosion simulation works on: large-scale relief only (features smaller
    /// than its ~20 km cells would just alias).
    pub fn coarse_height(&self, d: DVec3) -> f64 {
        self.large_scale(d, self.continental(d), 5, 3)
    }

    /// Height change from the erosion simulation and river wetness at `d`.
    #[inline]
    pub fn erosion_at(&self, d: DVec3) -> (f64, f64) {
        self.erosion.as_ref().map_or((0.0, 0.0), |e| e.sample(d))
    }

    /// Terrain height in metres above sea level for a unit direction. Negative = sea floor.
    pub fn height(&self, d: DVec3) -> f64 {
        let cl = self.continental(d);
        let (_, land, belt) = cl;
        let mut h = self.large_scale(d, cl, 11, 5);
        // River valleys carved by the planet-scale erosion simulation.
        h += self.erosion_at(d).0;

        // "Roughness" field: some regions are plains, others rugged hill country.
        let rough = smoothstep(-0.2, 0.3, self.detail.fbm(d * 20.0 + DVec3::splat(5.0), 3));

        // Ridged hills (~16 km), then eroded gullies and finer detail for close-up flying.
        let r2 = self.mountains.ridged(d * 400.0 + DVec3::splat(7.0), 7);
        h += (r2 - 0.2) * 3000.0 * land * (0.25 + 0.75 * rough) * (0.4 + belt);
        h += self.detail.eroded_fbm(d * 3000.0 + DVec3::splat(11.3), 6, d, 1.5) * 320.0 * land * (0.3 + rough);
        h += self.detail.fbm(d * 60000.0 + DVec3::splat(-4.9), 5) * 14.0 * land;
        // Riverbeds, banks and lake beds.
        if let Some(hy) = &self.hydro {
            h = hy.carve(d, h);
        }
        for &(dir, ph, radius) in &self.pads {
            let dist = (d - dir).length() * RADIUS;
            if dist < radius * 2.5 {
                h = ph + (h - ph) * smoothstep(radius, radius * 2.5, dist);
            }
        }
        h
    }

    /// Level the ground around `dir` (radius m) to its current height.
    pub fn add_pad(&mut self, dir: DVec3, radius: f64) {
        let h = self.height(dir);
        self.pads.push((dir.normalize(), h, radius));
    }

    /// Is `dir` on a levelled pad?
    pub fn on_pad(&self, dir: DVec3, margin: f64) -> bool {
        self.pads.iter().any(|&(p, _, r)| (dir - p).length() * RADIUS < r + margin)
    }

    /// Inland water (rivers, lakes) at `d`, if any.
    #[inline]
    pub fn water(&self, d: DVec3) -> Option<crate::hydro::Water> {
        self.hydro.as_ref().and_then(|hy| hy.water(d))
    }

    /// Height of the water surface at `d` (sea level or a river or lake), if there is water.
    pub fn water_level(&self, d: DVec3) -> Option<f64> {
        let h = self.height(d);
        if h < 0.0 {
            return Some(0.0);
        }
        self.water(d).map(|w| w.level).filter(|&l| l > h)
    }

    /// Climate and large-scale landscape at `d` (cheap; no fine detail).
    pub fn ecology(&self, d: DVec3) -> Ecology {
        let cl = self.continental(d);
        let (delta, wetness) = self.erosion_at(d);
        let height = (self.large_scale(d, cl, 5, 3) + delta).max(0.0);
        let coast = 1.0 - smoothstep(0.05, 0.45, cl.0);
        let lat = d.y.abs();
        let temperature =
            31.0 - 38.0 * lat.powf(1.6) - 6.5 * height / 1000.0 + self.climate.fbm(d * 3.0, 3) * 7.0;
        // Subtropical dry belts around 30 degrees latitude (sin = 0.5), like Earth's deserts.
        let dry_belt = 0.3 * (-((lat - 0.47) / 0.12).powi(2)).exp();
        let moisture = 0.48 + self.climate.fbm(d * 2.2 + DVec3::splat(17.0), 4) * 1.1 + coast * 0.25 + wetness * 0.35
            - dry_belt
            - ((temperature - 20.0).max(0.0) * 0.03);
        // Ground watered by a nearby river or lake.
        let riparian = self.hydro.as_ref().map_or(0.0, |hy| hy.riparian(d));
        // Towards lush-but-not-swamp moisture (grass, bushes and broadleaf trees all grow there),
        // from drier or wetter surroundings alike.
        let moisture = moisture + (0.62 - moisture) * riparian * 0.85;
        let wetness = wetness.max(riparian * 0.8);
        Ecology {
            temperature: temperature as f32,
            moisture: moisture.clamp(0.0, 1.0) as f32,
            height: height as f32,
            wetness: wetness as f32,
            coast: coast as f32,
        }
    }

    /// Height of the visible surface (oceans are flat at sea level).
    #[inline]
    pub fn surface_height(&self, d: DVec3) -> f64 {
        let h = self.height(d).max(0.0);
        self.water(d).map_or(h, |w| h.max(w.level))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkKey {
    pub face: u8,
    pub level: u8,
    pub x: u32,
    pub y: u32,
}

/// (face normal, face u-axis). The v-axis is normal × u, which makes triangles wind CCW from outside.
const FACES: [(DVec3, DVec3); 6] = [
    (DVec3::X, DVec3::Y),
    (DVec3::NEG_X, DVec3::Y),
    (DVec3::Y, DVec3::Z),
    (DVec3::NEG_Y, DVec3::Z),
    (DVec3::Z, DVec3::X),
    (DVec3::NEG_Z, DVec3::X),
];

/// Inverse of [`face_dir`]: which cube face a direction is on, and its (u, v) there.
pub fn dir_to_face_uv(d: DVec3) -> (u8, f64, f64) {
    let mut face = 0;
    let mut best = f64::MIN;
    for (i, (n, _)) in FACES.iter().enumerate() {
        let dot = d.dot(*n);
        if dot > best {
            best = dot;
            face = i;
        }
    }
    let (n, a) = FACES[face];
    let b = n.cross(a);
    let t = d / d.dot(n);
    let u = t.dot(a).atan() / FRAC_PI_4;
    let v = t.dot(b).atan() / FRAC_PI_4;
    (face as u8, u, v)
}

/// (u, v) of a direction projected onto a given face (may be outside [-1, 1] near its edges).
pub fn face_uv_of(face: u8, d: DVec3) -> (f64, f64) {
    let (n, a) = FACES[face as usize];
    let b = n.cross(a);
    let t = d / d.dot(n);
    (t.dot(a).atan() / FRAC_PI_4, t.dot(b).atan() / FRAC_PI_4)
}

/// The face across an edge of `face`: past u = sign (axis 0) or v = sign (axis 1).
pub fn neighbor_face(face: u8, axis: usize, sign: f64) -> u8 {
    let (n, a) = FACES[face as usize];
    let dir = if axis == 0 { a } else { n.cross(a) } * sign;
    FACES.iter().position(|(fnorm, _)| fnorm.dot(dir) > 0.5).unwrap() as u8
}

/// Map face coordinates (u, v) in [-1, 1] to a unit direction. The tan warp evens out cell sizes.
pub fn face_dir(face: u8, u: f64, v: f64) -> DVec3 {
    let (n, a) = FACES[face as usize];
    let b = n.cross(a);
    (n + a * (u * FRAC_PI_4).tan() + b * (v * FRAC_PI_4).tan()).normalize()
}

impl ChunkKey {
    pub fn root(face: u8) -> Self {
        Self { face, level: 0, x: 0, y: 0 }
    }

    /// Size in face coordinates (a face spans 2.0).
    pub fn size(&self) -> f64 {
        2.0 / (1u64 << self.level) as f64
    }

    pub fn uv0(&self) -> (f64, f64) {
        let s = self.size();
        (-1.0 + self.x as f64 * s, -1.0 + self.y as f64 * s)
    }

    pub fn center_dir(&self) -> DVec3 {
        let (u0, v0) = self.uv0();
        let h = self.size() * 0.5;
        face_dir(self.face, u0 + h, v0 + h)
    }

    /// Approximate edge length on the planet surface, in metres.
    pub fn world_size(&self) -> f64 {
        RADIUS * FRAC_PI_2 * self.size() * 0.5
    }

    pub fn children(&self) -> [ChunkKey; 4] {
        let (l, x, y) = (self.level + 1, self.x * 2, self.y * 2);
        let f = self.face;
        [
            ChunkKey { face: f, level: l, x, y },
            ChunkKey { face: f, level: l, x: x + 1, y },
            ChunkKey { face: f, level: l, x, y: y + 1 },
            ChunkKey { face: f, level: l, x: x + 1, y: y + 1 },
        ]
    }
}

/// CPU-side mesh data for one chunk. Positions are relative to `center` so f32 is precise enough.
pub struct ChunkMesh {
    pub center: DVec3,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Texture-layer weights (grass, litter, moss, dirt); see terrain.wgsl.
    pub colors: Vec<[f32; 4]>,
    /// More weights: (sand, rock) and (snow, distant canopy).
    pub uv0: Vec<[f32; 2]>,
    pub uv1: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// Plants growing on this chunk (merged), same origin.
    pub flora: FloraMesh,
    /// River and lake surfaces, same origin.
    pub water: WaterMesh,
}

/// Texture-layer weights at a vertex (see terrain.wgsl): ground left by the vegetation, bare
/// soil or sand, shores, rock on steep slopes, snow where it is cold enough. Under water all
/// land weights are zero. `far` (0..1) shows forests by their canopy on distant terrain.
pub(crate) fn terrain_weights(ctx: &FloraContext, h: f64, slope: f64, eco: &Ecology, far: f32) -> [f32; 8] {
    use crate::flora::ground::*;
    let mut out = [0.0f32; 8];
    if h < 0.0 {
        // Sea floor (the water itself is drawn by atmosphere.wgsl): sand in the shallows, silt
        // further down, rock on steep underwater slopes.
        let deep = smoothstep(-15.0, -120.0, h) as f32;
        let cliff = smoothstep(0.85, 0.65, slope) as f32;
        out[crate::flora::ground::SAND] = (1.0 - deep) * (1.0 - cliff);
        out[crate::flora::ground::DIRT] = deep * (1.0 - cliff);
        out[crate::flora::ground::ROCK] = cliff;
        return out;
    }
    let (mut w, canopy) = ctx.ground_cover(eco);
    let cover: f32 = w.iter().sum();
    if cover > 1.0 {
        w.iter_mut().for_each(|x| *x /= cover);
    }
    // Bare ground between plants: soil where it is wet, sand/dust where it is dry.
    let bare = (1.0 - cover).max(0.0);
    let wet = eco.moisture;
    w[DIRT] += bare * (0.25 + 0.75 * wet);
    w[SAND] += bare * (1.0 - wet) * 0.75;
    let blend = |w: &mut [f32; LAND_LAYERS], layer: usize, amount: f64| {
        let a = amount.clamp(0.0, 1.0) as f32;
        w.iter_mut().for_each(|x| *x *= 1.0 - a);
        w[layer] += a;
    };
    // Beaches (near the coast) and banks of big rivers.
    let shore = (eco.coast as f64 * 1.5).max(eco.wetness as f64).min(1.0);
    blend(&mut w, SAND, shore * (1.0 - smoothstep(2.0, 8.0, h)));
    // Cliffs.
    blend(&mut w, ROCK, smoothstep(0.85, 0.68, slope));
    // Snow where it is cold enough at this exact height (lapse rate 6.5 C per km).
    let temp = eco.temperature as f64 - 6.5 * (h - eco.height as f64) / 1000.0;
    blend(&mut w, SNOW, smoothstep(-1.0, -5.0, temp) * smoothstep(0.6, 0.8, slope));
    let total: f32 = w.iter().sum::<f32>().max(1e-4);
    for (o, x) in out.iter_mut().zip(w) {
        *o = x / total;
    }
    out[7] = canopy * far * (1.0 - smoothstep(0.85, 0.7, slope) as f32);
    out
}

/// Inland water surface of a chunk: triangles at the river/lake level wherever any corner is
/// under water (the depth test against the terrain draws the exact shoreline). uv.x carries the
/// water level and the tangent the flow direction (xyz) and gradient (w), for water.wgsl.
#[derive(Default)]
pub struct WaterMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub tangents: Vec<[f32; 4]>,
}

fn water_mesh(dirs: &[DVec3], hs: &[f64], water: &[Option<crate::hydro::Water>], g: usize, center: DVec3) -> WaterMesh {
    let mut m = WaterMesh::default();
    if water.iter().all(|w| w.is_none()) {
        return m;
    }
    // Only the chunk's own cells (the grid has a one-cell border).
    for j in 1..g - 2 {
        for i in 1..g - 2 {
            let a = j * g + i;
            let (b, c, d) = (a + 1, a + g, a + g + 1);
            for tri in [[a, b, d], [a, d, c]] {
                let Some(wet) = tri.iter().filter_map(|&k| water[k]).max_by(|x, y| x.level.total_cmp(&y.level)) else { continue };
                if !tri.iter().any(|&k| hs[k] < water[k].map_or(wet.level, |w| w.level) + 0.2) {
                    continue;
                }
                for &k in &tri {
                    let w = water[k].unwrap_or(wet);
                    let p = dirs[k] * (RADIUS + w.level) - center;
                    m.positions.push(p.as_vec3().to_array());
                    m.normals.push(dirs[k].as_vec3().to_array());
                    m.uvs.push([w.level as f32, 0.0]);
                    let f = w.flow.as_vec3();
                    m.tangents.push([f.x, f.y, f.z, w.slope as f32]);
                }
            }
        }
    }
    m
}

pub fn build_chunk(ctx: &FloraContext, key: ChunkKey) -> ChunkMesh {
    let t = &*ctx.terrain;
    let (u0, v0) = key.uv0();
    let step = key.size() / N as f64;
    // Sample grid with a one-cell border so normals are continuous across chunk edges.
    let g = N + 3;
    let mut pts = Vec::with_capacity(g * g);
    let mut hs = Vec::with_capacity(g * g);
    let mut dirs = Vec::with_capacity(g * g);
    for j in 0..g {
        for i in 0..g {
            let u = u0 + (i as f64 - 1.0) * step;
            let v = v0 + (j as f64 - 1.0) * step;
            let d = face_dir(key.face, u, v);
            let h = t.height(d);
            pts.push(d * (RADIUS + h));
            hs.push(h);
            dirs.push(d);
        }
    }
    // Inland water (rivers, lakes) on the sample grid: (level, flow, gradient).
    let water: Vec<Option<crate::hydro::Water>> = dirs.iter().map(|&d| t.water(d)).collect();
    let water_level: Vec<f64> = water.iter().map(|w| w.map_or(f64::NEG_INFINITY, |w| w.level)).collect();

    // Normals for every grid point (border points just point up).
    let mut grid_normals = dirs.clone();
    for j in 1..g - 1 {
        for i in 1..g - 1 {
            let gi = j * g + i;
            let dx = pts[gi + 1] - pts[gi - 1];
            let dy = pts[gi + g] - pts[gi - g];
            grid_normals[gi] = dx.cross(dy).try_normalize().unwrap_or(dirs[gi]);
        }
    }

    let center = key.center_dir() * RADIUS;
    // Canopy colour under thinned-out distant trees, matching the share that was dropped
    // (flora.rs min_rank), so forests keep their tone as they thin with distance.
    let far = match key.level {
        16.. => 0.0,
        15 => 0.45,
        14 => 0.7,
        13 => 0.88,
        _ => 1.0,
    };
    let vn = N + 1;
    let mut positions = Vec::with_capacity(vn * vn + 4 * N);
    let mut normals = Vec::with_capacity(vn * vn + 4 * N);
    let mut colors = Vec::with_capacity(vn * vn + 4 * N);
    let mut uv0 = Vec::with_capacity(vn * vn + 4 * N);
    let mut uv1 = Vec::with_capacity(vn * vn + 4 * N);
    for j in 0..vn {
        for i in 0..vn {
            let gi = (j + 1) * g + (i + 1);
            let n = grid_normals[gi];
            positions.push((pts[gi] - center).as_vec3().to_array());
            normals.push(n.as_vec3().to_array());
            let eco = t.ecology(dirs[gi]);
            let h_rel = if hs[gi] < water_level[gi] + 1.5 { hs[gi] - water_level[gi] } else { hs[gi] };
            let w = terrain_weights(ctx, h_rel, n.dot(dirs[gi]), &eco, far);
            colors.push([w[0], w[1], w[2], w[3]]);
            uv0.push([w[4], w[5]]);
            uv1.push([w[6], w[7]]);
        }
    }

    let mut indices = Vec::with_capacity(N * N * 6 + 4 * N * 6);
    for j in 0..N {
        for i in 0..N {
            let a = (j * vn + i) as u32;
            let b = a + 1;
            let c = a + vn as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }

    // Skirts: a strip hanging down from the chunk border hides cracks between LOD levels.
    let mut perim = Vec::with_capacity(4 * N);
    for i in 0..N {
        perim.push((i, 0));
    }
    for j in 0..N {
        perim.push((N, j));
    }
    for i in (1..=N).rev() {
        perim.push((i, N));
    }
    for j in (1..=N).rev() {
        perim.push((0, j));
    }
    let depth = key.world_size() / N as f64 * 0.5 + 20.0;
    let skirt_start = positions.len() as u32;
    for &(i, j) in &perim {
        let vi = j * vn + i;
        let gi = (j + 1) * g + (i + 1);
        let p = pts[gi] - dirs[gi] * depth - center;
        positions.push(p.as_vec3().to_array());
        normals.push(normals[vi]);
        colors.push(colors[vi]);
        uv0.push(uv0[vi]);
        uv1.push(uv1[vi]);
    }
    let pl = perim.len();
    for k in 0..pl {
        let (ai, aj) = perim[k];
        let (bi, bj) = perim[(k + 1) % pl];
        let a = (aj * vn + ai) as u32;
        let b = (bj * vn + bi) as u32;
        let sa = skirt_start + k as u32;
        let sb = skirt_start + ((k + 1) % pl) as u32;
        indices.extend_from_slice(&[a, b, sb, a, sb, sa]);
    }

    let water_mesh = water_mesh(&dirs, &hs, &water, g, center);
    let surface = SurfaceGrid { key, g, heights: &hs, normals: &grid_normals, water: &water_level };
    let flora = build_chunk_flora(ctx, &surface, center);
    ChunkMesh { center, positions, normals, colors, uv0, uv1, indices, flora, water: water_mesh }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_are_plausible() {
        let t = Terrain::new(7);
        let (mut lo, mut hi, mut land) = (f64::MAX, f64::MIN, 0);
        let n = 4000;
        for i in 0..n {
            // Fibonacci sphere sampling
            let y = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
            let r = (1.0 - y * y).sqrt();
            let phi = i as f64 * 2.399_963;
            let d = DVec3::new(r * phi.cos(), y, r * phi.sin());
            let h = t.height(d);
            assert!(h.is_finite());
            lo = lo.min(h);
            hi = hi.max(h);
            if h > 0.0 {
                land += 1;
            }
        }
        let frac = land as f64 / n as f64;
        println!("height range {lo:.0}..{hi:.0} m, land fraction {frac:.2}");
        assert!(lo < -500.0 && hi > 1500.0 && hi < 15000.0);
        assert!(frac > 0.15 && frac < 0.85);
    }

    #[test]
    fn chunk_mesh_is_valid_and_faces_outward() {
        let ctx = FloraContext::new(Arc::new(Terrain::new(7)), crate::life::Biosphere::barren(), 7, Vec::new());
        for key in [ChunkKey::root(0), ChunkKey::root(3), ChunkKey { face: 4, level: 12, x: 1234, y: 2345 }] {
            let m = build_chunk(&ctx, key);
            let nv = m.positions.len() as u32;
            assert_eq!(nv as usize, (N + 1) * (N + 1) + 4 * N);
            assert!(m.indices.iter().all(|&i| i < nv));
            assert!(m.positions.iter().flatten().all(|v| v.is_finite()));
            // First grid triangle should face away from the planet centre.
            let p = |i: u32| bevy::math::Vec3::from(m.positions[i as usize]);
            let (a, b, c) = (p(m.indices[0]), p(m.indices[1]), p(m.indices[2]));
            let n = (b - a).cross(c - a);
            let out = (m.center.as_vec3() + a).normalize();
            assert!(n.dot(out) > 0.0, "triangle winding faces inward for {key:?}");
        }
    }

    #[test]
    fn adjacent_faces_meet() {
        // The +X face at u=+1 edge and the +Y face should share points along the cube edge.
        let a = face_dir(0, 1.0, 0.3);
        assert!((a.length() - 1.0).abs() < 1e-12);
        let found = (0..6u8).any(|f| f != 0 && {
            // search the other faces' edges for this direction
            (0..=200).any(|k| {
                let s = -1.0 + k as f64 * 0.01;
                [face_dir(f, 1.0, s), face_dir(f, -1.0, s), face_dir(f, s, 1.0), face_dir(f, s, -1.0)]
                    .iter()
                    .any(|d| (*d - a).length() < 0.02)
            })
        });
        assert!(found);
    }
}

#[cfg(test)]
mod relief {
    use super::*;

    /// Typical height change over a few distances, on land. Used to tune the terrain.
    #[test]
    #[ignore]
    fn relief_stats() {
        let t = Terrain::new(1337);
        for step_m in [50.0, 500.0, 2500.0, 20000.0] {
            let mut diffs = Vec::new();
            let n = 20000;
            for i in 0..n {
                let y = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
                let r = (1.0 - y * y).sqrt();
                let phi = i as f64 * 2.399_963;
                let d = DVec3::new(r * phi.cos(), y, r * phi.sin());
                let h = t.height(d);
                if h < 50.0 { continue; }
                let tan = d.cross(DVec3::Y).normalize();
                let h2 = t.height((d + tan * step_m / RADIUS).normalize());
                diffs.push((h2 - h).abs());
            }
            diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let q = |f: f64| diffs[(diffs.len() as f64 * f) as usize];
            println!("over {step_m:>6} m: median {:>7.1}  p90 {:>7.1}  p99 {:>7.1}", q(0.5), q(0.9), q(0.99));
        }
    }
}
