// Leaf cards: the foliage atlas (albedo + alpha, normal, AO/roughness) in our own bindings.
//
// Prepass: alpha test against the atlas, write depth (and normals / motion vectors).
// Main pass: runs with an *equal* depth test and no depth writes (graphics.rs,
// FoliageExt::specialize), so it only ever sees the leaf texels the prepass kept, and has no
// `discard`: early depth testing stays on and each pixel is shaded once, not once per
// overlapping leaf layer.

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    prepass_bindings,
    mesh_view_bindings::view,
}
#else
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}
#endif

@group(2) @binding(100) var albedo_tex: texture_2d<f32>;
@group(2) @binding(101) var atlas_smp: sampler;
@group(2) @binding(102) var normal_tex: texture_2d<f32>;
@group(2) @binding(103) var arh_tex: texture_2d<f32>;

const CUTOFF: f32 = 0.45;

#ifdef PREPASS_PIPELINE
#ifndef PREPASS_FRAGMENT
// Shadow maps and depth-only prepasses: just the alpha test.
@fragment
fn fragment(in: VertexOutput) {
    if (textureSample(albedo_tex, atlas_smp, in.uv).a < CUTOFF) {
        discard;
    }
}
#else
@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    if (textureSample(albedo_tex, atlas_smp, in.uv).a < CUTOFF) {
        discard;
    }
    var out: FragmentOutput;
#ifdef NORMAL_PREPASS
    var n = normalize(in.world_normal);
    if (!is_front) { n = -n; }
    out.normal = vec4(n * 0.5 + vec3(0.5), 1.0);
#endif
#ifdef MOTION_VECTOR_PREPASS
    let clip_t = view.unjittered_clip_from_world * in.world_position;
    let clip = clip_t.xy / clip_t.w;
    let prev_t = prepass_bindings::previous_view_uniforms.clip_from_world * in.previous_world_position;
    let prev = prev_t.xy / prev_t.w;
    out.motion_vector = (clip - prev) * vec2(0.5, -0.5);
#endif
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.frag_depth = in.unclipped_depth;
#endif
    return out;
}
#endif
#else
@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let a = textureSample(albedo_tex, atlas_smp, in.uv);
#ifndef DEPTH_PREPASS
    if (a.a < CUTOFF) {
        discard;
    }
#endif
    let t = textureSample(normal_tex, atlas_smp, in.uv).xyz * 2.0 - 1.0;
    let m = textureSample(arh_tex, atlas_smp, in.uv);

    var n = normalize(in.world_normal);
    if (!is_front) {
        n = -n;
    }
    // Tangent frame from screen-space derivatives (cards carry no tangent attribute, which
    // keeps their vertices small; see Mikkelsen / Schüler's cotangent frame).
    let dp1 = dpdx(in.world_position.xyz);
    let dp2 = dpdy(in.world_position.xyz);
    let duv1 = dpdx(in.uv);
    let duv2 = dpdy(in.uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let tg = dp2perp * duv1.x + dp1perp * duv2.x;
    let bt = dp2perp * duv1.y + dp1perp * duv2.y;
    let inv = inverseSqrt(max(max(dot(tg, tg), dot(bt, bt)), 1e-20));
    pbr_input.N = normalize(tg * inv * t.x + bt * inv * t.y + n * t.z);

    pbr_input.material.base_color = vec4(a.rgb * in.color.rgb, 1.0);
    pbr_input.diffuse_occlusion = vec3(m.r);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
#endif
