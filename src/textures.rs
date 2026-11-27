//! Loads the material textures (tools/textures/build_textures.py) into GPU texture arrays with
//! full mipmap chains and anisotropic filtering.
//!
//! Bevy 0.16 does not generate mipmaps for PNGs, and without them distant textures shimmer and
//! cost bandwidth, so the chains are built here on the CPU (in parallel, once at startup):
//! colour maps are filtered in linear space, normal maps are renormalised, and the foliage
//! atlas keeps its alpha coverage at every level so leaves don't thin out in the distance.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
    TextureViewDimension,
};

/// Terrain layers, in texture-array order (see build_textures.py).
pub const TERRAIN_LAYERS: [&str; 8] = ["grass", "forest_litter", "moss", "dirt", "sand", "rock", "snow", "water"];
/// Bark/rock layers for plants, in texture-array order.
pub mod flora_layer {
    pub const BARK_REDWOOD: f32 = 0.0;
    pub const BARK_SPRUCE: f32 = 1.0;
    pub const BARK_BROADLEAF: f32 = 2.0;
    pub const ROCK_MOSSY: f32 = 3.0;
    pub const ROCK_GREY: f32 = 4.0;
    /// Plain white: the vertex colour alone (flowers, mushrooms, alien plants).
    pub const PLAIN: f32 = 5.0;
    /// Seamless dense foliage, for the solid inner crowns of trees.
    pub const NEEDLE_MASS: f32 = 6.0;
    pub const LEAF_MASS: f32 = 7.0;
    pub const COUNT: u32 = 8;
}
/// Foliage atlas cells (2x2): needles, leaves, fern, grass.
pub mod card {
    pub const NEEDLES: u32 = 0;
    pub const LEAVES: u32 = 1;
    pub const FERN: u32 = 2;
    pub const GRASS: u32 = 3;
}

#[derive(Clone)]
pub struct TextureSet {
    pub albedo: Handle<Image>,
    pub normal: Handle<Image>,
    pub arh: Handle<Image>,
}

#[derive(Resource, Clone)]
pub struct Textures {
    pub terrain: TextureSet,
    pub flora: TextureSet,
    pub foliage: TextureSet,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// sRGB colour (filtered in linear space).
    Color,
    /// sRGB colour with alpha cut-out: alpha coverage preserved per mip.
    CutoutColor,
    /// Tangent-space normal map.
    Normal,
    /// Plain linear data.
    Linear,
}

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/textures");

pub fn load_textures(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let start = std::time::Instant::now();
    let jobs: Vec<(&str, u32, Kind)> = vec![
        ("terrain_albedo", 8, Kind::Color),
        ("terrain_normal", 8, Kind::Normal),
        ("terrain_arh", 8, Kind::Linear),
        ("flora_albedo", flora_layer::COUNT, Kind::Color),
        ("flora_normal", flora_layer::COUNT, Kind::Normal),
        ("flora_arh", flora_layer::COUNT, Kind::Linear),
        ("foliage_albedo", 1, Kind::CutoutColor),
        ("foliage_normal", 1, Kind::Normal),
        ("foliage_arh", 1, Kind::Linear),
    ];
    let built: Vec<Image> = std::thread::scope(|s| {
        let handles: Vec<_> = jobs.iter().map(|&(name, layers, kind)| s.spawn(move || build_image(name, layers, kind))).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut h = built.into_iter().map(|img| images.add(img));
    let mut set = || TextureSet { albedo: h.next().unwrap(), normal: h.next().unwrap(), arh: h.next().unwrap() };
    let terrain = set();
    let flora = set();
    let foliage = set();
    info!("textures loaded and mipmapped in {:.2} s", start.elapsed().as_secs_f32());
    commands.insert_resource(Textures { terrain, flora, foliage });
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// Halve an RGBA8 image.
fn downsample(src: &[u8], w: usize, kind: Kind, lut: &[f32; 256]) -> Vec<u8> {
    let half = w / 2;
    let mut out = vec![0u8; half * half * 4];
    for y in 0..half {
        for x in 0..half {
            let idx = |dx: usize, dy: usize| ((2 * y + dy) * w + 2 * x + dx) * 4;
            let px = [idx(0, 0), idx(1, 0), idx(0, 1), idx(1, 1)];
            let o = (y * half + x) * 4;
            match kind {
                Kind::Color | Kind::CutoutColor => {
                    for c in 0..3 {
                        let v: f32 = px.iter().map(|&i| lut[src[i + c] as usize]).sum::<f32>() * 0.25;
                        out[o + c] = linear_to_srgb(v);
                    }
                    out[o + 3] = (px.iter().map(|&i| src[i + 3] as u32).sum::<u32>() / 4) as u8;
                }
                Kind::Normal => {
                    let mut n = Vec3::ZERO;
                    for &i in &px {
                        n += Vec3::new(src[i] as f32, src[i + 1] as f32, src[i + 2] as f32) / 127.5 - Vec3::ONE;
                    }
                    let n = n.normalize_or(Vec3::Z) * 127.5 + Vec3::splat(127.5);
                    out[o..o + 3].copy_from_slice(&[n.x as u8, n.y as u8, n.z as u8]);
                    out[o + 3] = 255;
                }
                Kind::Linear => {
                    for c in 0..4 {
                        out[o + c] = (px.iter().map(|&i| src[i + c] as u32).sum::<u32>() / 4) as u8;
                    }
                }
            }
        }
    }
    out
}

/// Fraction of pixels whose alpha passes the cut-out threshold when scaled by `scale`.
fn coverage(img: &[u8], scale: f32) -> f32 {
    let n = img.len() / 4;
    let pass = img.chunks_exact(4).filter(|p| p[3] as f32 * scale >= 127.5).count();
    pass as f32 / n as f32
}

fn build_image(name: &str, layers: u32, kind: Kind) -> Image {
    let path = format!("{DIR}/{name}.png");
    let img = image::open(&path).unwrap_or_else(|e| panic!("{path}: {e} (run tools/textures/build_textures.py)")).to_rgba8();
    let w = img.width() as usize;
    let layer_h = img.height() as usize / layers as usize;
    assert_eq!(w, layer_h, "{name}: layers must be square");
    let mips = (w as f32).log2() as u32 + 1;
    let mut lut = [0f32; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = srgb_to_linear(i as u8);
    }
    let raw = img.into_raw();
    let mut data = Vec::with_capacity(raw.len() * 4 / 3 + 64);
    for l in 0..layers as usize {
        let mut level = raw[l * w * w * 4..(l + 1) * w * w * 4].to_vec();
        let target = if kind == Kind::CutoutColor { coverage(&level, 1.0) } else { 0.0 };
        let mut size = w;
        data.extend_from_slice(&level);
        for _ in 1..mips {
            level = downsample(&level, size, kind, &lut);
            size /= 2;
            if kind == Kind::CutoutColor && size >= 4 {
                // Scale alpha so the cut-out keeps the same coverage as the full-size image.
                let (mut lo, mut hi) = (0.5f32, 4.0f32);
                for _ in 0..12 {
                    let mid = 0.5 * (lo + hi);
                    if coverage(&level, mid) < target { lo = mid } else { hi = mid }
                }
                for p in level.chunks_exact_mut(4) {
                    p[3] = (p[3] as f32 * hi).min(255.0) as u8;
                }
            }
            data.extend_from_slice(&level);
        }
    }
    let format = match kind {
        Kind::Color | Kind::CutoutColor => TextureFormat::Rgba8UnormSrgb,
        _ => TextureFormat::Rgba8Unorm,
    };
    let array = layers > 1;
    Image {
        data: Some(data),
        texture_descriptor: TextureDescriptor {
            label: None,
            size: Extent3d { width: w as u32, height: w as u32, depth_or_array_layers: layers },
            mip_level_count: mips,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            anisotropy_clamp: 16,
            ..default()
        }),
        texture_view_descriptor: array.then(|| TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        }),
        asset_usage: RenderAssetUsages::RENDER_WORLD,
        ..default()
    }
}
