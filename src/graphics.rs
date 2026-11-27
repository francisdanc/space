//! Rendering setup: the textured terrain material and the camera's post-processing
//! (HDR, TonyMcMapface tonemapping, bloom, SSAO, temporal anti-aliasing).

use bevy::asset::{load_internal_asset, weak_handle};
use bevy::core_pipeline::bloom::Bloom;
use bevy::core_pipeline::experimental::taa::{TemporalAntiAliasPlugin, TemporalAntiAliasing};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::math::DVec3;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderRef};

use crate::camera::CameraWorld;
use crate::textures::Textures;

const TERRAIN_SHADER: Handle<Shader> = weak_handle!("2c6a8b1e-55f4-4f0e-9e1d-7a3b9c0d4e21");
const FLORA_SOLID_SHADER: Handle<Shader> = weak_handle!("9d41f7a2-0b3c-4e6f-8a15-c2e7d9b04f63");
const FOLIAGE_SHADER: Handle<Shader> = weak_handle!("e2a47c90-61d3-4b8f-9f05-7d1c3a6b28e4");
const IMPOSTOR_SHADER: Handle<Shader> = weak_handle!("5b8e2c17-3f4a-4d9b-a6e1-0c7f92d3b845");
/// Terrain texture coordinates wrap with this period (m); every tile size divides it.
const TEXTURE_PERIOD: f64 = 4096.0;

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExt>;
pub type FloraSolidMaterial = ExtendedMaterial<StandardMaterial, FloraSolidExt>;
pub type ImpostorMaterial = ExtendedMaterial<StandardMaterial, ImpostorExt>;
pub type FoliageMaterial = ExtendedMaterial<StandardMaterial, FoliageExt>;

pub struct GraphicsPlugin;

impl Plugin for GraphicsPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, TERRAIN_SHADER, "terrain.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, FLORA_SOLID_SHADER, "flora_solid.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, IMPOSTOR_SHADER, "impostor.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, FOLIAGE_SHADER, "foliage.wgsl", Shader::from_wgsl);
        app.add_plugins((
            MaterialPlugin::<TerrainMaterial>::default(),
            MaterialPlugin::<FloraSolidMaterial>::default(),
            MaterialPlugin::<ImpostorMaterial>::default(),
            MaterialPlugin::<FoliageMaterial>::default(),
            TemporalAntiAliasPlugin,
        ));
    }
}

pub use params::TerrainParams;

// The ShaderType derive generates helpers that trip the dead-code lint.
#[allow(dead_code)]
mod params {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    #[derive(ShaderType, Clone, Copy, Debug, Default, Reflect)]
    pub struct TerrainParams {
        /// Camera position (planet frame) modulo the texture period.
        pub cam_offset: Vec4,
        /// Colour of distant forest canopy (linear RGB).
        pub canopy: Vec4,
        /// Per-layer colour multipliers (white on Earth-like worlds, palette shifts on alien ones).
        pub tints: [Vec4; 8],
        /// The planet centre relative to the camera (render space), for surface-map lookups.
        pub planet_center: Vec4,
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct TerrainExt {
    #[uniform(100)]
    pub params: TerrainParams,
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub albedo: Handle<Image>,
    #[texture(103, dimension = "2d_array")]
    #[sampler(104)]
    pub normal: Handle<Image>,
    #[texture(105, dimension = "2d_array")]
    #[sampler(106)]
    pub arh: Handle<Image>,
    /// Planet-wide layer weights (see surface_map.rs): layers 0..3 and 4..7.
    #[texture(107, dimension = "cube")]
    #[sampler(108)]
    pub surface0: Handle<Image>,
    #[texture(109, dimension = "cube")]
    pub surface1: Handle<Image>,
}

impl MaterialExtension for TerrainExt {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(TERRAIN_SHADER)
    }
}

/// Bark and rock for plants: texture arrays indexed by the layer stored in each vertex.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct FloraSolidExt {
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub albedo: Handle<Image>,
    #[texture(102, dimension = "2d_array")]
    #[sampler(103)]
    pub normal: Handle<Image>,
    #[texture(104, dimension = "2d_array")]
    #[sampler(105)]
    pub arh: Handle<Image>,
}

impl MaterialExtension for FloraSolidExt {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(FLORA_SOLID_SHADER)
    }
}

/// Distant trees as camera-facing quads showing baked views (impostor.rs, impostor.wgsl).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct ImpostorExt {
    /// x, y, z: the planet centre relative to the camera.
    #[uniform(100)]
    pub planet_center: Vec4,
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub albedo: Handle<Image>,
    #[texture(103, dimension = "2d_array")]
    pub normal: Handle<Image>,
}

/// Alpha-tested materials drawn after a depth prepass: the main pass only needs to shade the
/// texels the prepass kept, so it tests depth for equality and leaves out `discard` (which
/// would disable early depth testing and shade every overlapping layer).
fn depth_equal_main_pass(descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor, mesh_key: bevy::pbr::MeshPipelineKey) {
    if descriptor.label.as_deref().is_some_and(|l| l.ends_with("opaque_mesh_pipeline")) && mesh_key.contains(bevy::pbr::MeshPipelineKey::DEPTH_PREPASS) {
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_compare = bevy::render::render_resource::CompareFunction::Equal;
            ds.depth_write_enabled = false;
        }
    }
}

/// Leaf cards (foliage.wgsl): the standard material with a depth-equal main pass.
/// The leaf atlas lives in the extension's own bindings, sampled by our own prepass and main
/// fragment shaders, so the alpha test is under our control.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct FoliageExt {
    #[texture(100)]
    #[sampler(101)]
    pub albedo: Handle<Image>,
    #[texture(102)]
    pub normal: Handle<Image>,
    #[texture(103)]
    pub arh: Handle<Image>,
}

impl MaterialExtension for FoliageExt {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(FOLIAGE_SHADER)
    }
    fn prepass_fragment_shader() -> ShaderRef {
        ShaderRef::Handle(FOLIAGE_SHADER)
    }
    fn specialize(
        _: &bevy::pbr::MaterialExtensionPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _: &bevy::render::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialExtensionKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        depth_equal_main_pass(descriptor, key.mesh_key);
        Ok(())
    }
}

impl MaterialExtension for ImpostorExt {
    fn specialize(
        _: &bevy::pbr::MaterialExtensionPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _: &bevy::render::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialExtensionKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        depth_equal_main_pass(descriptor, key.mesh_key);
        Ok(())
    }
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(IMPOSTOR_SHADER)
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(IMPOSTOR_SHADER)
    }
    fn prepass_vertex_shader() -> ShaderRef {
        ShaderRef::Handle(IMPOSTOR_SHADER)
    }
    fn prepass_fragment_shader() -> ShaderRef {
        ShaderRef::Handle(IMPOSTOR_SHADER)
    }
}

pub fn impostor_material(atlas: (Handle<Image>, Handle<Image>)) -> ImpostorMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            alpha_mode: AlphaMode::Mask(0.5),
            diffuse_transmission: 0.25,
            cull_mode: None,
            double_sided: true,
            ..default()
        },
        extension: ImpostorExt { planet_center: Vec4::ZERO, albedo: atlas.0, normal: atlas.1 },
    }
}

pub fn flora_solid_material(textures: &Textures) -> FloraSolidMaterial {
    ExtendedMaterial {
        base: StandardMaterial { perceptual_roughness: 0.8, cull_mode: None, double_sided: true, ..default() },
        extension: FloraSolidExt {
            albedo: textures.flora.albedo.clone(),
            normal: textures.flora.normal.clone(),
            arh: textures.flora.arh.clone(),
        },
    }
}

/// Foliage cards (needles, leaves, fronds, grass): alpha-tested, two-sided, slightly
/// translucent so leaves glow when lit from behind.
pub fn foliage_material(textures: &Textures) -> FoliageMaterial {
    ExtendedMaterial {
        extension: FoliageExt {
            albedo: textures.foliage.albedo.clone(),
            normal: textures.foliage.normal.clone(),
            arh: textures.foliage.arh.clone(),
        },
        base: StandardMaterial {
        alpha_mode: AlphaMode::Mask(0.45),
        perceptual_roughness: 0.65,
        reflectance: 0.35,
        diffuse_transmission: 0.3,
        double_sided: true,
        cull_mode: None,
        ..default()
    } }
}

/// The terrain material, shared by every chunk.
pub fn terrain_material(
    textures: &Textures,
    canopy: Vec3,
    tints: [Vec3; 8],
    surface: (Handle<Image>, Handle<Image>),
) -> TerrainMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            perceptual_roughness: 0.9,
            // Skirts are wound arbitrarily; drawing both sides is simpler than fixing them up.
            cull_mode: None,
            ..default()
        },
        extension: TerrainExt {
            params: TerrainParams {
                cam_offset: Vec4::ZERO,
                canopy: canopy.extend(1.0),
                tints: tints.map(|t| t.extend(1.0)),
                planet_center: Vec4::ZERO,
            },
            albedo: textures.terrain.albedo.clone(),
            normal: textures.terrain.normal.clone(),
            arh: textures.terrain.arh.clone(),
            surface0: surface.0,
            surface1: surface.1,
        },
    }
}

/// Keep the terrain's texture coordinates anchored to the planet as the camera moves, and
/// tell the impostors where the planet centre is.
pub fn update_terrain_material(
    cam: Res<CameraWorld>,
    planet: Res<crate::planet::Planet>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    mut impostors: ResMut<Assets<ImpostorMaterial>>,
) {
    if let Some(m) = impostors.get_mut(&planet.impostor_material) {
        m.extension.planet_center = (-cam.pos).as_vec3().extend(0.0);
    }
    let wrap = |v: f64| v.rem_euclid(TEXTURE_PERIOD) as f32;
    let off = DVec3::new(wrap(cam.pos.x) as f64, wrap(cam.pos.y) as f64, wrap(cam.pos.z) as f64);
    if let Some(m) = materials.get_mut(&planet.terrain_material) {
        m.extension.params.cam_offset = off.as_vec3().extend(0.0);
        // The planet centre is the world origin; the camera sits at the render origin.
        m.extension.params.planet_center = (-cam.pos).as_vec3().extend(0.0);
    }
}

/// Post-processing for the main camera.
pub fn camera_effects() -> impl Bundle {
    (
        Camera { hdr: true, ..default() },
        Tonemapping::TonyMcMapface,
        Bloom::NATURAL,
        Msaa::Off,
        TemporalAntiAliasing::default(),
    )
}

/// Screen-space ambient occlusion (off with SPACE_HIDE=ssao, for profiling).
pub fn ssao() -> Option<ScreenSpaceAmbientOcclusion> {
    (!crate::debug::hidden("ssao")).then_some(ScreenSpaceAmbientOcclusion {
        quality_level: if crate::debug::hidden("ssao_low") { ScreenSpaceAmbientOcclusionQualityLevel::Low } else { ScreenSpaceAmbientOcclusionQualityLevel::Medium },
        constant_object_thickness: 0.5,
    })
}
