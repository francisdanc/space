//! Tree impostors: each big plant species is rendered once at startup, on the CPU, from
//! `AZ` x `EL` directions into a texture-array layer (albedo + alpha, normal + depth). Far
//! away, every tree is drawn as one camera-facing quad that shows the nearest baked views
//! (impostor.wgsl), instead of hundreds of triangles of branches and leaf cards.
//!
//! This is the "middle layer" of the three every large open world uses (meshes up close,
//! impostors for kilometres, a canopy colour in the terrain beyond); see
//! reports/Game foliage and planet rendering.md. Serious Sam 4 uses 8 directions; this adds
//! elevation rows because the camera here can fly over the forest.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::{Vec2, Vec3};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
    TextureViewDimension,
};

use crate::flora::{FloraContext, Layer};
use crate::life::Biosphere;
use crate::plants::Template;

/// Azimuth steps around the tree.
pub const AZ: usize = 8;
/// Elevation rows (radians above the horizon), matching impostor.wgsl.
pub const ELEVATIONS: [f32; 4] = [0.0, 0.38, 0.8, 1.25];
pub const EL: usize = ELEVATIONS.len();
/// Pixels per frame edge.
pub const FRAME: usize = 192;
/// Supersampling factor while rasterising.
const SS: usize = 2;

/// Where a species' impostor lives and how big it is (template units, before instance scale).
#[derive(Clone, Copy, Debug)]
pub struct ImpostorInfo {
    pub slot: u32,
    /// Height of the bounding sphere's centre above the plant's base.
    pub center_y: f32,
    pub radius: f32,
    /// Horizontal half-width as a fraction of `radius` (tall trees are much narrower than
    /// their bounding sphere; side-on quads are cut to this width).
    pub width: f32,
}

/// Which species get impostors: the big ones (trees, bushes, boulders), from their most detailed
/// template.
pub fn layout(bio: &Biosphere) -> Vec<Option<ImpostorInfo>> {
    let mut slot = 0;
    bio.flora
        .iter()
        .map(|s| {
            if !matches!(s.layer, Layer::Canopy | Layer::Understory) {
                return None;
            }
            let t = s.tiers.iter().flatten().next()?;
            let (center_y, radius) = bounds(t)?;
            let half_w = t.solid.pos.iter().chain(t.cards.pos.iter()).chain(t.fill.pos.iter()).map(|p| p.x.hypot(p.z)).fold(0.0f32, f32::max);
            let info = ImpostorInfo { slot, center_y, radius, width: (half_w * 1.05 / radius).clamp(0.2, 1.0) };
            slot += 1;
            Some(info)
        })
        .collect()
}

fn bounds(t: &Template) -> Option<(f32, f32)> {
    let pts = t.solid.pos.iter().chain(t.cards.pos.iter()).chain(t.fill.pos.iter());
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for p in pts.clone() {
        lo = lo.min(p.y);
        hi = hi.max(p.y);
    }
    if lo > hi {
        return None;
    }
    let cy = 0.5 * (lo + hi);
    let c = Vec3::new(0.0, cy, 0.0);
    let r = pts.map(|p| (*p - c).length()).fold(0.0f32, f32::max);
    Some((cy, r.max(0.1)))
}

/// Unit direction (tree-local) from the tree towards a camera in frame (az, el).
pub fn view_dir(az: usize, el: usize) -> Vec3 {
    let a = az as f32 / AZ as f32 * std::f32::consts::TAU;
    let e = ELEVATIONS[el];
    Vec3::new(e.cos() * a.cos(), e.sin(), e.cos() * a.sin())
}

/// Screen axes for a view direction (same convention as the shader).
fn basis(d: Vec3) -> (Vec3, Vec3) {
    let right = Vec3::Y.cross(d).normalize_or(Vec3::X);
    let up = d.cross(right);
    (right, up)
}

/// A texture downsampled for baking, in linear colour.
struct Tex {
    w: usize,
    layers: usize,
    /// Linear RGB + alpha, layer-major.
    px: Vec<[f32; 4]>,
}

impl Tex {
    fn load(path: &str, layers: usize, size: usize) -> Tex {
        let img = image::open(path).unwrap_or_else(|e| panic!("{path}: {e}")).to_rgba8();
        let w = img.width() as usize;
        let lh = img.height() as usize / layers;
        let mut px = Vec::with_capacity(size * size * layers);
        for l in 0..layers {
            let sub = image::imageops::crop_imm(&img, 0, (l * lh) as u32, w as u32, lh as u32).to_image();
            // Premultiply before filtering so transparent texels don't bleed dark colours.
            let lin: Vec<[f32; 4]> = sub
                .pixels()
                .map(|p| {
                    let a = p[3] as f32 / 255.0;
                    let c = |v: u8| srgb_to_linear(v) * a;
                    [c(p[0]), c(p[1]), c(p[2]), a]
                })
                .collect();
            let k = w / size;
            for y in 0..size {
                for x in 0..size {
                    let mut acc = [0.0f32; 4];
                    for dy in 0..k {
                        for dx in 0..k {
                            let q = lin[(y * k + dy) * w + x * k + dx];
                            for c in 0..4 {
                                acc[c] += q[c];
                            }
                        }
                    }
                    let n = (k * k) as f32;
                    let a = acc[3] / n;
                    let un = |v: f32| if a > 1e-4 { v / n / a } else { 0.0 };
                    px.push([un(acc[0]), un(acc[1]), un(acc[2]), a]);
                }
            }
        }
        Tex { w: size, layers, px }
    }

    fn sample(&self, uv: Vec2, layer: usize) -> [f32; 4] {
        let l = layer.min(self.layers - 1);
        let x = ((uv.x.rem_euclid(1.0)) * self.w as f32) as usize % self.w;
        let y = ((uv.y.rem_euclid(1.0)) * self.w as f32) as usize % self.w;
        self.px[l * self.w * self.w + y * self.w + x]
    }
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

/// One rendered frame at supersampled resolution.
struct Frame {
    s: usize,
    depth: Vec<f32>,
    col: Vec<[f32; 3]>,
    nrm: Vec<Vec3>,
}

/// Rasterise one part of a template into the frame (orthographic, along -d).
#[allow(clippy::too_many_arguments)]
fn raster(f: &mut Frame, part: &crate::plants::Part, cards: bool, d: Vec3, right: Vec3, up: Vec3, c: Vec3, r: f32, bark: &Tex, leaves: &Tex) {
    let s = f.s as f32;
    let proj = |p: Vec3| {
        let q = p - c;
        Vec3::new((q.dot(right) / r * 0.5 + 0.5) * s, (0.5 - q.dot(up) / r * 0.5) * s, q.dot(d))
    };
    for tri in part.idx.chunks_exact(3) {
        let [i0, i1, i2] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let (p0, p1, p2) = (proj(part.pos[i0]), proj(part.pos[i1]), proj(part.pos[i2]));
        let area = (p1.x - p0.x) * (p2.y - p0.y) - (p2.x - p0.x) * (p1.y - p0.y);
        if area.abs() < 1e-6 {
            continue;
        }
        // Screen y points down, so a front face (towards the camera) has negative area here.
        let back = area > 0.0;
        let x0 = p0.x.min(p1.x).min(p2.x).floor().max(0.0) as usize;
        let x1 = (p0.x.max(p1.x).max(p2.x).ceil() as usize).min(f.s);
        let y0 = p0.y.min(p1.y).min(p2.y).floor().max(0.0) as usize;
        let y1 = (p0.y.max(p1.y).max(p2.y).ceil() as usize).min(f.s);
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((p1.x - px) * (p2.y - py) - (p2.x - px) * (p1.y - py)) / area;
                let w1 = ((p2.x - px) * (p0.y - py) - (p0.x - px) * (p2.y - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = p0.z * w0 + p1.z * w1 + p2.z * w2;
                let k = y * f.s + x;
                if z <= f.depth[k] {
                    continue;
                }
                let uv = part.uv[i0] * w0 + part.uv[i1] * w1 + part.uv[i2] * w2;
                let texel = if cards {
                    leaves.sample(uv, 0)
                } else {
                    bark.sample(uv, part.uv2[i0].x.round() as usize)
                };
                if cards && texel[3] < 0.4 {
                    continue;
                }
                let vc = |i: usize, ch: usize| part.col[i][ch];
                let col = [0, 1, 2].map(|ch| texel[ch] * (vc(i0, ch) * w0 + vc(i1, ch) * w1 + vc(i2, ch) * w2));
                let mut n = (part.nrm[i0] * w0 + part.nrm[i1] * w1 + part.nrm[i2] * w2).normalize_or(d);
                if back {
                    n = -n;
                }
                f.depth[k] = z;
                f.col[k] = col;
                f.nrm[k] = n;
            }
        }
    }
}

/// Render one species: returns (albedo RGBA8 sRGB, normal+depth RGBA8) for a layer of
/// (AZ*FRAME) x (EL*FRAME) pixels.
fn bake_species(t: &Template, info: &ImpostorInfo, bark: &Tex, leaves: &Tex) -> (Vec<u8>, Vec<u8>) {
    let (w, h) = (AZ * FRAME, EL * FRAME);
    let mut albedo = vec![0u8; w * h * 4];
    let mut normal = vec![0u8; w * h * 4];
    let c = Vec3::new(0.0, info.center_y, 0.0);
    let r = info.radius;
    for el in 0..EL {
        for az in 0..AZ {
            let d = view_dir(az, el);
            let (right, up) = basis(d);
            let s = FRAME * SS;
            let mut f = Frame { s, depth: vec![f32::MIN; s * s], col: vec![[0.0; 3]; s * s], nrm: vec![Vec3::ZERO; s * s] };
            raster(&mut f, &t.solid, false, d, right, up, c, r, bark, leaves);
            raster(&mut f, &t.cards, true, d, right, up, c, r, bark, leaves);
            raster(&mut f, &t.fill, true, d, right, up, c, r, bark, leaves);
            // Downsample: alpha = coverage, colour/normal = mean of covered samples.
            for y in 0..FRAME {
                for x in 0..FRAME {
                    let (mut n, mut col, mut nrm, mut dep) = (0, [0.0f32; 3], Vec3::ZERO, 0.0);
                    for dy in 0..SS {
                        for dx in 0..SS {
                            let k = (y * SS + dy) * s + x * SS + dx;
                            if f.depth[k] > f32::MIN {
                                n += 1;
                                for ch in 0..3 {
                                    col[ch] += f.col[k][ch];
                                }
                                nrm += f.nrm[k];
                                dep += f.depth[k];
                            }
                        }
                    }
                    let o = ((el * FRAME + y) * w + az * FRAME + x) * 4;
                    if n > 0 {
                        let inv = 1.0 / n as f32;
                        for ch in 0..3 {
                            albedo[o + ch] = linear_to_srgb(col[ch] * inv);
                        }
                        albedo[o + 3] = (n as f32 / (SS * SS) as f32 * 255.0).round() as u8;
                        // Normal in the frame's basis: x right, y up, z towards the camera.
                        let nl = nrm.normalize_or(d);
                        let nb = Vec3::new(nl.dot(right), nl.dot(up), nl.dot(d));
                        let e = |v: f32| ((v * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
                        normal[o..o + 3].copy_from_slice(&[e(nb.x), e(nb.y), e(nb.z)]);
                        normal[o + 3] = e(dep * inv / r);
                    }
                }
            }
        }
    }
    dilate(&mut albedo, &mut normal, w, h);
    (albedo, normal)
}

/// Spread colours and normals into transparent pixels so mipmaps don't fade edges to black.
fn dilate(albedo: &mut [u8], normal: &mut [u8], w: usize, h: usize) {
    let mut filled: Vec<bool> = albedo.chunks_exact(4).map(|p| p[3] > 0).collect();
    for _ in 0..8 {
        let prev = filled.clone();
        for y in 0..h {
            for x in 0..w {
                let k = y * w + x;
                if prev[k] {
                    continue;
                }
                let (mut n, mut a, mut b) = (0u32, [0u32; 3], [0u32; 3]);
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if prev[j] {
                        n += 1;
                        for c in 0..3 {
                            a[c] += albedo[j * 4 + c] as u32;
                            b[c] += normal[j * 4 + c] as u32;
                        }
                    }
                }
                if n > 0 {
                    for c in 0..3 {
                        albedo[k * 4 + c] = (a[c] / n) as u8;
                        normal[k * 4 + c] = (b[c] / n) as u8;
                    }
                    filled[k] = true;
                }
            }
        }
    }
}

/// Mip chain of one RGBA8 layer (w x h, both powers of two times FRAME). Alpha keeps the
/// coverage of level 0 at the 0.5 cut-off, like the foliage cards (textures.rs).
fn mips(level0: Vec<u8>, w: usize, h: usize, cutout: bool, count: u32) -> Vec<u8> {
    let coverage = |img: &[u8], scale: f32| img.chunks_exact(4).filter(|p| p[3] as f32 * scale >= 127.5).count() as f32 / (img.len() / 4) as f32;
    let target = if cutout { coverage(&level0, 1.0) } else { 0.0 };
    let mut out = level0.clone();
    let (mut level, mut lw, mut lh) = (level0, w, h);
    for _ in 1..count {
        let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                let idx = |dx: usize, dy: usize| ((2 * y + dy).min(lh - 1) * lw + (2 * x + dx).min(lw - 1)) * 4;
                let px = [idx(0, 0), idx(1, 0), idx(0, 1), idx(1, 1)];
                let o = (y * nw + x) * 4;
                if cutout {
                    // Colour weighted by alpha (texels outside the leaves don't count).
                    let aw: Vec<f32> = px.iter().map(|&i| level[i + 3] as f32 + 1.0).collect();
                    let sw: f32 = aw.iter().sum();
                    for c in 0..3 {
                        let v: f32 = px.iter().zip(&aw).map(|(&i, w)| srgb_to_linear(level[i + c]) * w).sum::<f32>() / sw;
                        next[o + c] = linear_to_srgb(v);
                    }
                    next[o + 3] = (px.iter().map(|&i| level[i + 3] as u32).sum::<u32>() / 4) as u8;
                } else {
                    for c in 0..4 {
                        next[o + c] = (px.iter().map(|&i| level[i + c] as u32).sum::<u32>() / 4) as u8;
                    }
                }
            }
        }
        if cutout && nw >= 4 {
            let (mut lo, mut hi) = (0.5f32, 4.0f32);
            for _ in 0..12 {
                let mid = 0.5 * (lo + hi);
                if coverage(&next, mid) < target { lo = mid } else { hi = mid }
            }
            for p in next.chunks_exact_mut(4) {
                p[3] = (p[3] as f32 * hi).min(255.0) as u8;
            }
        }
        out.extend_from_slice(&next);
        level = next;
        lw = nw;
        lh = nh;
    }
    out
}

fn array_image(layers: Vec<Vec<u8>>, w: usize, h: usize, format: TextureFormat, cutout: bool) -> Image {
    // Stop the chain while a frame is still at least 4 px, so frames don't bleed into each other.
    let count = (FRAME as f32 / 4.0).log2() as u32 + 1;
    let n = layers.len().max(1) as u32;
    let mut data = Vec::new();
    for l in layers {
        data.extend_from_slice(&mips(l, w, h, cutout, count));
    }
    if data.is_empty() {
        // No big plants on this planet: a single transparent layer keeps the binding valid.
        data = mips(vec![0u8; w * h * 4], w, h, false, count);
    }
    Image {
        data: Some(data),
        texture_descriptor: TextureDescriptor {
            label: Some("impostors"),
            size: Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: n },
            mip_level_count: count,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            ..default()
        }),
        texture_view_descriptor: Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() }),
        asset_usage: RenderAssetUsages::RENDER_WORLD,
        ..default()
    }
}

/// Bake every species' impostor (one thread per species). Returns (albedo, normal) arrays.
pub fn bake(ctx: &FloraContext, images: &mut Assets<Image>) -> (Handle<Image>, Handle<Image>) {
    let start = std::time::Instant::now();
    let dir = crate::game_dir().join("assets/textures");
    let dir = dir.display();
    let bark = Tex::load(&format!("{dir}/flora_albedo.png"), crate::textures::flora_layer::COUNT as usize, 64);
    let leaves = Tex::load(&format!("{dir}/foliage_albedo.png"), 1, 512);
    let jobs: Vec<(&Template, ImpostorInfo)> = ctx
        .bio
        .flora
        .iter()
        .zip(&ctx.impostors)
        .filter_map(|(s, i)| Some((&**s.tiers.iter().flatten().next()?, (*i)?)))
        .collect();
    let baked: Vec<(Vec<u8>, Vec<u8>)> = std::thread::scope(|s| {
        let hs: Vec<_> = jobs.iter().map(|(t, i)| s.spawn(|| bake_species(t, i, &bark, &leaves))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let (a, n): (Vec<_>, Vec<_>) = baked.into_iter().unzip();
    let (w, h) = (AZ * FRAME, EL * FRAME);
    info!("impostors baked for {} species in {:.2} s", jobs.len(), start.elapsed().as_secs_f32());
    if let Ok(path) = std::env::var("SPACE_IMPOSTOR_DUMP") {
        if let Some(first) = a.first() {
            let _ = image::save_buffer(&path, first, w as u32, h as u32, image::ColorType::Rgba8);
        }
    }
    (
        images.add(array_image(a, w, h, TextureFormat::Rgba8UnormSrgb, true)),
        images.add(array_image(n, w, h, TextureFormat::Rgba8Unorm, false)),
    )
}
