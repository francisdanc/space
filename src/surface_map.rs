//! A planet-wide map of the terrain's texture-layer weights, baked into two cube maps at
//! startup with the same rules the chunk builder uses per vertex (`terrain_weights`).
//!
//! Far away, terrain chunks are coarse (a level-2 chunk has ~78 km between vertices), and
//! weights interpolated across such triangles turn into blocky patches from orbit. The terrain
//! shader blends towards this map, ~10 km per texel, where the vertices are coarser than it.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
    TextureViewDimension,
};

use crate::flora::FloraContext;
use crate::terrain::{RADIUS, terrain_weights};

/// Texels per cube-face edge.
pub const RES: usize = 1024;

/// Direction through the centre of texel (i, j) of cube face `face`, in the GPU's cube-map
/// convention (faces +X, -X, +Y, -Y, +Z, -Z; row 0 at the top).
pub fn texel_dir(face: usize, i: usize, j: usize, res: usize) -> DVec3 {
    let s = (i as f64 + 0.5) / res as f64 * 2.0 - 1.0;
    let t = (j as f64 + 0.5) / res as f64 * 2.0 - 1.0;
    match face {
        0 => DVec3::new(1.0, -t, -s),
        1 => DVec3::new(-1.0, -t, s),
        2 => DVec3::new(s, 1.0, t),
        3 => DVec3::new(s, -1.0, -t),
        4 => DVec3::new(s, -t, 1.0),
        _ => DVec3::new(-s, -t, -1.0),
    }
    .normalize()
}

/// Bake one face: (weights 0..3, weights 4..7) as RGBA8.
fn bake_face(ctx: &FloraContext, face: usize, res: usize) -> (Vec<u8>, Vec<u8>) {
    let t = &*ctx.terrain;
    let dirs: Vec<DVec3> = (0..res * res).map(|k| texel_dir(face, k % res, k / res, res)).collect();
    let hs: Vec<f64> = dirs.iter().map(|&d| t.height(d)).collect();
    let mut a = vec![0u8; res * res * 4];
    let mut b = vec![0u8; res * res * 4];
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    for j in 0..res {
        for i in 0..res {
            let k = j * res + i;
            // Slope from the height differences to the neighbouring texels (one-sided at edges).
            let (i0, i1) = (i.saturating_sub(1), (i + 1).min(res - 1));
            let (j0, j1) = (j.saturating_sub(1), (j + 1).min(res - 1));
            let dx = (dirs[j * res + i1] - dirs[j * res + i0]).length() * RADIUS;
            let dy = (dirs[j1 * res + i] - dirs[j0 * res + i]).length() * RADIUS;
            let gx = (hs[j * res + i1] - hs[j * res + i0]) / dx;
            let gy = (hs[j1 * res + i] - hs[j0 * res + i]) / dy;
            let slope = 1.0 / (1.0 + gx * gx + gy * gy).sqrt();
            let eco = t.ecology(dirs[k]);
            let w = terrain_weights(ctx, hs[k], slope, &eco, 1.0);
            a[k * 4..k * 4 + 4].copy_from_slice(&[q(w[0]), q(w[1]), q(w[2]), q(w[3])]);
            b[k * 4..k * 4 + 4].copy_from_slice(&[q(w[4]), q(w[5]), q(w[6]), q(w[7])]);
        }
    }
    (a, b)
}

/// Full mip chain of one RGBA8 face (box filter).
fn with_mips(level0: Vec<u8>, res: usize) -> Vec<u8> {
    let mut out = level0.clone();
    let mut level = level0;
    let mut size = res;
    while size > 1 {
        let half = size / 2;
        let mut next = vec![0u8; half * half * 4];
        for y in 0..half {
            for x in 0..half {
                for c in 0..4 {
                    let p = |dx: usize, dy: usize| level[((2 * y + dy) * size + 2 * x + dx) * 4 + c] as u32;
                    next[(y * half + x) * 4 + c] = ((p(0, 0) + p(1, 0) + p(0, 1) + p(1, 1) + 2) / 4) as u8;
                }
            }
        }
        out.extend_from_slice(&next);
        level = next;
        size = half;
    }
    out
}

fn cube_image(faces: Vec<Vec<u8>>, res: usize) -> Image {
    let mips = (res as f32).log2() as u32 + 1;
    let mut data = Vec::new();
    for f in faces {
        data.extend_from_slice(&with_mips(f, res));
    }
    Image {
        data: Some(data),
        texture_descriptor: TextureDescriptor {
            label: Some("surface_map"),
            size: Extent3d { width: res as u32, height: res as u32, depth_or_array_layers: 6 },
            mip_level_count: mips,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            address_mode_w: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            anisotropy_clamp: 8,
            ..default()
        }),
        texture_view_descriptor: Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..default()
        }),
        asset_usage: RenderAssetUsages::RENDER_WORLD,
        ..default()
    }
}

/// Cube maps with the same layer weights everywhere (bodies without biomes, like the moon).
pub fn constant(images: &mut Assets<Image>, w0: [u8; 4], w1: [u8; 4]) -> (Handle<Image>, Handle<Image>) {
    let res = 4;
    let face = |w: [u8; 4]| (0..res * res).flat_map(|_| w).collect::<Vec<u8>>();
    (
        images.add(cube_image((0..6).map(|_| face(w0)).collect(), res)),
        images.add(cube_image((0..6).map(|_| face(w1)).collect(), res)),
    )
}

/// Bake both cube maps (in parallel, one thread per face strip).
pub fn bake(ctx: &FloraContext, images: &mut Assets<Image>) -> (Handle<Image>, Handle<Image>) {
    let start = std::time::Instant::now();
    let faces: Vec<(Vec<u8>, Vec<u8>)> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..6).map(|f| s.spawn(move || bake_face(ctx, f, RES))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let (a, b): (Vec<_>, Vec<_>) = faces.into_iter().unzip();
    info!("surface map baked ({RES}x{RES} per face) in {:.2} s", start.elapsed().as_secs_f32());
    (images.add(cube_image(a, RES)), images.add(cube_image(b, RES)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texel_dirs_land_on_their_own_face() {
        // The major axis of each texel's direction must be the face's axis and sign.
        let axes = [DVec3::X, DVec3::NEG_X, DVec3::Y, DVec3::NEG_Y, DVec3::Z, DVec3::NEG_Z];
        for (f, axis) in axes.iter().enumerate() {
            for &(i, j) in &[(0, 0), (7, 3), (15, 15)] {
                let d = texel_dir(f, i, j, 16);
                assert!(d.dot(*axis) >= d.abs().max_element() - 1e-12, "face {f} texel {i},{j}: {d}");
            }
        }
    }
}
