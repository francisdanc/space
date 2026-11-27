// Bark and rock: photo-scanned PBR textures from a texture array (layer index in uv_b.x),
// tangent-space normal mapping, vertex colour as tint and baked occlusion.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}

@group(2) @binding(100) var albedo_tex: texture_2d_array<f32>;
@group(2) @binding(101) var albedo_smp: sampler;
@group(2) @binding(102) var normal_tex: texture_2d_array<f32>;
@group(2) @binding(103) var normal_smp: sampler;
@group(2) @binding(104) var arh_tex: texture_2d_array<f32>;
@group(2) @binding(105) var arh_smp: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let layer = i32(round(in.uv_b.x));
    let a = textureSample(albedo_tex, albedo_smp, in.uv, layer);
    let t = textureSample(normal_tex, normal_smp, in.uv, layer).xyz * 2.0 - 1.0;
    let m = textureSample(arh_tex, arh_smp, in.uv, layer);

    var n = normalize(in.world_normal);
    if (!is_front) {
        n = -n;
    }
    let tangent = normalize(in.world_tangent.xyz - n * dot(in.world_tangent.xyz, n));
    let bitangent = cross(n, tangent) * in.world_tangent.w;
    pbr_input.N = normalize(tangent * t.x - bitangent * t.y + n * t.z);

    pbr_input.material.base_color = vec4(a.rgb * in.color.rgb, 1.0);
    pbr_input.material.perceptual_roughness = clamp(m.g, 0.3, 1.0);
    pbr_input.material.metallic = 0.0;
    pbr_input.diffuse_occlusion = vec3(m.r);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
