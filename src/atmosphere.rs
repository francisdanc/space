//! Physically based atmosphere: Rayleigh + Mie single scattering (see atmosphere.wgsl), plus the
//! matching sunlight colour at the camera, so sunsets redden the terrain and night is dark.

use bevy::asset::{load_internal_asset, weak_handle};
use bevy::math::DVec3;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey, NotShadowCaster};
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, Face, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError,
};

use crate::camera::CameraWorld;
use crate::planet::WorldPos;
use crate::sky::{Stars, sun_dir};
use crate::terrain::RADIUS;

const SHADER: Handle<Shader> = weak_handle!("6f3c2a71-8e0b-4d5a-9c41-2b7d93e15a08");
/// WGSL module `space::atmo` (atmo_common.wgsl), shared with inland water.
const COMMON_SHADER: Handle<Shader> = weak_handle!("a8d3e5f1-27c4-4b69-8e0d-5f1a6c93b742");
const WATER_SHADER: Handle<Shader> = weak_handle!("3c71b9e2-5d48-4a0f-b6e3-92d4f17a0c5e");

/// Top of the atmosphere above sea level (m).
pub const ATMOSPHERE_HEIGHT: f64 = 100_000.0;
/// Earth-like coefficients.
const RAYLEIGH: DVec3 = DVec3::new(5.8e-6, 13.5e-6, 33.1e-6);
const RAYLEIGH_SCALE: f64 = 8_000.0;
const MIE: f64 = 4.0e-6;
const MIE_SCALE: f64 = 1_200.0;
const MIE_EXTINCTION: f64 = 1.11;
/// Brightness of the sky relative to the scene lighting.
const SUN_INTENSITY: f32 = 16.0;
/// Sunlight on the terrain when the sun is overhead (lux).
const SUN_ILLUMINANCE: f32 = 7_000.0;

pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, COMMON_SHADER, "atmo_common.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, SHADER, "atmosphere.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, WATER_SHADER, "water.wgsl", Shader::from_wgsl);
        app.add_plugins((MaterialPlugin::<AtmosphereMaterial>::default(), MaterialPlugin::<WaterMaterial>::default()))
            .add_systems(Startup, spawn_atmosphere);
        let water = app.world_mut().resource_mut::<Assets<WaterMaterial>>().add(WaterMaterial { params: AtmosphereParams::default() });
        app.insert_resource(WaterHandle(water));
    }
}

pub use params::AtmosphereParams;

// The ShaderType derive generates helper functions that trip the dead-code lint.
#[allow(dead_code)]
mod params {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    #[derive(ShaderType, Clone, Copy, Debug, Default)]
    pub struct AtmosphereParams {
        pub center: Vec3,
        pub r_cam: f32,
        pub sun_dir: Vec3,
        pub k_ground: f32,
        pub rayleigh: Vec3,
        pub k_top: f32,
        pub bottom_radius: f32,
        pub top_radius: f32,
        pub rayleigh_scale: f32,
        pub mie_scale: f32,
        pub mie: f32,
        pub mie_g: f32,
        pub sun_intensity: f32,
        pub shell: f32,
        /// Camera position in the planet frame modulo the wave period (precise wave coordinates).
        pub cam_offset: Vec3,
        /// Sunlight on the surface when the sun is overhead (lux), to light water like terrain.
        pub sun_illuminance: f32,
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct AtmosphereMaterial {
    #[uniform(0)]
    params: AtmosphereParams,
}

impl Material for AtmosphereMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER)
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    /// Drawn after every other transparent mesh (rivers, lakes), so they get aerial perspective.
    fn depth_bias(&self) -> f32 {
        1.0e10
    }

    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Draw the inside (back faces) of the shell, which covers every view direction whether
        // the camera is inside or outside, and ignore depth: the shader clips against the depth
        // prepass itself.
        descriptor.primitive.cull_mode = Some(Face::Front);
        if let Some(ds) = &mut descriptor.depth_stencil {
            ds.depth_compare = CompareFunction::Always;
            ds.depth_write_enabled = false;
        }
        Ok(())
    }
}

#[derive(Resource)]
pub struct AtmosphereHandle(Handle<AtmosphereMaterial>);

/// Rivers and lakes (water.wgsl): the water optics need the same atmosphere parameters.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct WaterMaterial {
    #[uniform(0)]
    params: AtmosphereParams,
}

impl Material for WaterMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(WATER_SHADER)
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[derive(Resource)]
pub struct WaterHandle(pub Handle<WaterMaterial>);

fn spawn_atmosphere(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<AtmosphereMaterial>>,
) {
    let top = RADIUS + ATMOSPHERE_HEIGHT;
    let handle = materials.add(AtmosphereMaterial { params: AtmosphereParams::default() });
    // A little bigger than the real top so the faceted mesh fully contains the true sphere.
    let mesh = Sphere::new((top * 1.01) as f32).mesh().ico(5).unwrap();
    commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(handle.clone()),
        Transform::default(),
        WorldPos(DVec3::ZERO),
        NotShadowCaster,
    ));
    commands.insert_resource(AtmosphereHandle(handle));
}

/// Period (m) of the ocean wave pattern; every wave's wavelength divides it (atmosphere.wgsl).
const WAVE_PERIOD: f64 = 4096.0;

fn wrap(v: f64) -> f64 {
    v.rem_euclid(WAVE_PERIOD)
}

/// Fraction of sunlight (per colour channel) reaching `pos`. Zero if the planet is in the way.
pub fn sun_transmittance(pos: DVec3, sun: DVec3) -> DVec3 {
    let top = RADIUS + ATMOSPHERE_HEIGHT;
    let r = pos.length();
    let mu = pos.dot(sun) / r;
    // Planet in the way?
    let k_ground = (r - RADIUS) * (r + RADIUS);
    let b = r * mu;
    if b < 0.0 && b * b - k_ground > 0.0 {
        return DVec3::ZERO;
    }
    let k_top = (r - top) * (r + top);
    let disc = b * b - k_top;
    if disc <= 0.0 {
        return DVec3::ONE;
    }
    let t_far = -b + disc.sqrt();
    if t_far <= 0.0 {
        return DVec3::ONE;
    }
    let steps = 32;
    let seg = t_far / steps as f64;
    let (mut od_r, mut od_m) = (0.0, 0.0);
    for i in 0..steps {
        let p = pos + sun * ((i as f64 + 0.5) * seg);
        let h = (p.length() - RADIUS).max(0.0);
        od_r += (-h / RAYLEIGH_SCALE).exp() * seg;
        od_m += (-h / MIE_SCALE).exp() * seg;
    }
    let tau = RAYLEIGH * od_r + DVec3::splat(MIE * MIE_EXTINCTION * od_m);
    DVec3::new((-tau.x).exp(), (-tau.y).exp(), (-tau.z).exp())
}

/// Update the shader inputs, the sunlight and the ambient light for the camera's position.
/// Runs after the camera has been placed for this frame.
#[allow(clippy::too_many_arguments)]
pub fn update_atmosphere(
    cam: Res<CameraWorld>,
    handle: Res<AtmosphereHandle>,
    mut materials: ResMut<Assets<AtmosphereMaterial>>,
    water: Res<WaterHandle>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    mut sun_light: Query<&mut DirectionalLight>,
    mut ambient: ResMut<AmbientLight>,
    mut stars: Query<&mut Visibility, With<Stars>>,
) {
    let sun = sun_dir().as_dvec3();
    let top = RADIUS + ATMOSPHERE_HEIGHT;
    let r = cam.pos.length();
    if let Some(m) = materials.get_mut(&handle.0) {
        m.params = AtmosphereParams {
            center: (-cam.pos).as_vec3(),
            r_cam: r as f32,
            sun_dir: sun.as_vec3(),
            k_ground: ((r - RADIUS) * (r + RADIUS)) as f32,
            rayleigh: RAYLEIGH.as_vec3(),
            k_top: ((r - top) * (r + top)) as f32,
            bottom_radius: RADIUS as f32,
            top_radius: top as f32,
            rayleigh_scale: RAYLEIGH_SCALE as f32,
            mie_scale: MIE_SCALE as f32,
            mie: MIE as f32,
            mie_g: 0.76,
            sun_intensity: SUN_INTENSITY,
            shell: ((top - RADIUS) * (top + RADIUS)) as f32,
            cam_offset: DVec3::new(wrap(cam.pos.x), wrap(cam.pos.y), wrap(cam.pos.z)).as_vec3(),
            sun_illuminance: SUN_ILLUMINANCE,
        };
        let params = m.params;
        if let Some(w) = water_materials.get_mut(&water.0) {
            w.params = params;
        }
    }

    // Sunlight as seen from the camera: reddens at sunset, gone at night.
    let t = sun_transmittance(cam.pos, sun);
    let lum = (t.x + t.y + t.z) / 3.0;
    if let Ok(mut light) = sun_light.single_mut() {
        let tint = t / t.max_element().max(1e-6);
        light.color = Color::linear_rgb(tint.x as f32, tint.y as f32, tint.z as f32);
        light.illuminance = SUN_ILLUMINANCE * (lum as f32).powf(0.7);
    }
    // Sky light: strong in a sunlit atmosphere, dim in space or at night.
    let air = (-(r - RADIUS).max(0.0) / 15_000.0).exp();
    ambient.brightness = 6.0 + 260.0 * (lum * air) as f32;
    // Stars fade out against a bright daytime sky.
    if let Ok(mut v) = stars.single_mut() {
        *v = if lum * air > 0.15 { Visibility::Hidden } else { Visibility::Visible };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sunlight_reddens_towards_the_horizon_and_vanishes_at_night() {
        let up = DVec3::Y;
        let pos = up * (RADIUS + 2.0);
        let noon = sun_transmittance(pos, up);
        let low = sun_transmittance(pos, DVec3::new(1.0, 0.05, 0.0).normalize());
        let night = sun_transmittance(pos, DVec3::new(1.0, -0.3, 0.0).normalize());
        let space = sun_transmittance(up * (RADIUS + 400_000.0), DVec3::X);
        assert!(noon.x > 0.8 && noon.z > 0.6, "noon {noon}");
        assert!(low.x > low.z * 3.0, "low sun should be red: {low}");
        assert_eq!(night, DVec3::ZERO);
        assert!(space.min_element() > 0.99);
    }
}
