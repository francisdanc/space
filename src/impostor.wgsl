// Tree impostors (impostor.rs): each vertex carries the tree's centre (position), the quad
// corner (uv, -1..1), its radius and atlas slot + yaw (uv_b) and a brightness tint (color).
// The vertex shader turns the quad to face the camera; the fragment shader blends the four
// baked views nearest to the actual view direction (bilinear in azimuth and elevation).

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{prepass_io::FragmentOutput, prepass_bindings}
#else
#import bevy_pbr::{
    pbr_types,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_view},
    forward_io::FragmentOutput,
}
#endif

struct ImpostorParams {
    // The planet centre in render space (the camera sits at the origin).
    planet_center: vec4<f32>,
}

@group(2) @binding(100) var<uniform> params: ImpostorParams;
@group(2) @binding(101) var albedo_tex: texture_2d_array<f32>;
@group(2) @binding(102) var atlas_smp: sampler;
@group(2) @binding(103) var normal_tex: texture_2d_array<f32>;

const AZ: f32 = 8.0;
const EL: f32 = 4.0;
const TAU: f32 = 6.28318530718;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
#ifdef PREPASS_PIPELINE
    @location(1) uv: vec2<f32>,
    @location(2) uv_b: vec2<f32>,
    @location(7) color: vec4<f32>,
#else
    @location(2) uv: vec2<f32>,
    @location(3) uv_b: vec2<f32>,
    @location(5) color: vec4<f32>,
#endif
}

struct VOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    // Corner in 0..1 frame coordinates.
    @location(1) uv: vec2<f32>,
    // Frame coordinates: continuous azimuth (0..AZ) and elevation (0..EL-1) index.
    @location(2) @interpolate(flat) frame: vec2<f32>,
    @location(3) @interpolate(flat) slot_tint: vec2<f32>,
    // Quad basis: right, up, towards the camera.
    @location(4) @interpolate(flat) right: vec3<f32>,
    @location(5) @interpolate(flat) up: vec3<f32>,
    @location(6) @interpolate(flat) toward: vec3<f32>,
#ifdef MOTION_VECTOR_PREPASS
    @location(7) previous_world_position: vec4<f32>,
#endif
}

// Shortest-arc rotation taking +Y to `up`, applied to `w` (glam's Quat::from_rotation_arc).
fn arc_from_y(up: vec3<f32>, w: vec3<f32>) -> vec3<f32> {
    let k = vec3(up.z, 0.0, -up.x); // cross(Y, up)
    let s = length(k);
    if (s < 1e-5) {
        return w;
    }
    let kn = k / s;
    let c = up.y;
    return w * c + cross(kn, w) * s + kn * dot(kn, w) * (1.0 - c);
}

// Elevation angle -> continuous row index (rows at 0, 0.38, 0.8, 1.25 rad; impostor.rs).
fn elevation_index(e: f32) -> f32 {
    if (e <= 0.0) { return 0.0; }
    if (e <= 0.38) { return e / 0.38; }
    if (e <= 0.8) { return 1.0 + (e - 0.38) / 0.42; }
    if (e <= 1.25) { return 2.0 + (e - 0.8) / 0.45; }
    return 3.0;
}

@vertex
fn vertex(v: Vertex) -> VOut {
    var out: VOut;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let center = (world_from_local * vec4(v.position, 1.0)).xyz;
    let radius = v.uv_b.x;
    let slot = floor(v.uv_b.y);
    let yaw = fract(v.uv_b.y) * TAU;

    let up = normalize(center - params.planet_center.xyz);
    let toward = normalize(view.world_position - center);
    // Same screen axes as the bake: right = up x toward, quad up = toward x right.
    var right = cross(up, toward);
    if (length(right) < 1e-4) {
        right = arc_from_y(up, vec3(1.0, 0.0, 0.0));
    }
    right = normalize(right);
    let qup = cross(toward, right);

    // Side-on, the quad is cut to the tree's width; from above it opens up to the full circle.
    let el_s = clamp(dot(toward, up), 0.0, 1.0);
    let w = mix(v.color.g, 1.0, smoothstep(0.2, 0.9, el_s));
    // Pulled a little towards the camera so the quad doesn't cut into the ground on slopes.
    let offset = (right * v.uv.x * w + qup * v.uv.y) * radius + toward * radius * 0.3;
    let world = center + offset;
    out.world_position = vec4(world, 1.0);
    out.position = position_world_to_clip(world);
    out.uv = vec2(v.uv.x * w * 0.5 + 0.5, 0.5 - v.uv.y * 0.5);

    // View direction in the tree's own frame (its yaw around its up axis).
    let lx = arc_from_y(up, vec3(cos(yaw), 0.0, -sin(yaw)));
    let lz = arc_from_y(up, vec3(sin(yaw), 0.0, cos(yaw)));
    let az = atan2(dot(toward, lz), dot(toward, lx));
    let el = asin(clamp(dot(toward, up), -1.0, 1.0));
    out.frame = vec2(fract(az / TAU) * AZ, elevation_index(el));
    out.slot_tint = vec2(slot, v.color.r);
    out.right = right;
    out.up = qup;
    out.toward = toward;

#ifdef MOTION_VECTOR_PREPASS
    let prev = mesh_functions::get_previous_world_from_local(v.instance_index);
    out.previous_world_position = vec4((prev * vec4(v.position, 1.0)).xyz + offset, 1.0);
#endif
    return out;
}

struct Sampled {
    albedo: vec4<f32>,
    normal: vec3<f32>,
}

fn sample_frame(az: f32, el: f32, uv: vec2<f32>, slot: i32) -> Sampled {
    let a = ((az % AZ) + AZ) % AZ;
    let t = vec2((a + uv.x) / AZ, (el + uv.y) / EL);
    var s: Sampled;
    s.albedo = textureSample(albedo_tex, atlas_smp, t, slot);
    s.normal = textureSample(normal_tex, atlas_smp, t, slot).xyz * 2.0 - 1.0;
    return s;
}

// Interleaved gradient noise (Jimenez 2014): a per-pixel threshold that TAA averages out.
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715))));
}

// One baked view per pixel, chosen stochastically between the four around the actual
// direction (weights = bilinear in azimuth/elevation); TAA blends the result. Two texture
// reads instead of eight.
fn impostor(in: VOut) -> Sampled {
    let slot = i32(in.slot_tint.x);
    // Keep a texel of margin so neighbouring frames don't bleed in.
    let uv = clamp(in.uv, vec2(0.004), vec2(0.996));
    let a0 = floor(in.frame.x);
    let fa = in.frame.x - a0;
    let e0 = min(floor(in.frame.y), EL - 2.0);
    let fe = clamp(in.frame.y - e0, 0.0, 1.0);
    let n = ign(in.position.xy + vec2(f32(view.frame_count % 64u) * 5.588, 0.0));
    let n2 = fract(n * 7.31 + 0.37);
    let az = a0 + select(0.0, 1.0, n < fa);
    let el = e0 + select(0.0, 1.0, n2 < fe);
    return sample_frame(az, el, uv, slot);
}

fn world_normal(in: VOut, n: vec3<f32>) -> vec3<f32> {
    return normalize(in.right * n.x + in.up * n.y + in.toward * max(n.z, 0.05));
}

#ifdef PREPASS_PIPELINE
@fragment
fn fragment(in: VOut) -> FragmentOutput {
    let s = impostor(in);
    if (s.albedo.a < 0.5) {
        discard;
    }
    var out: FragmentOutput;
#ifdef NORMAL_PREPASS
    out.normal = vec4(world_normal(in, s.normal) * 0.5 + vec3(0.5), 1.0);
#endif
#ifdef MOTION_VECTOR_PREPASS
    let clip_t = view.unjittered_clip_from_world * in.world_position;
    let clip = clip_t.xy / clip_t.w;
    let prev_t = prepass_bindings::previous_view_uniforms.clip_from_world * in.previous_world_position;
    let prev = prev_t.xy / prev_t.w;
    out.motion_vector = (clip - prev) * vec2(0.5, -0.5);
#endif
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.frag_depth = in.position.z;
#endif
    return out;
}
#else
@fragment
fn fragment(in: VOut) -> FragmentOutput {
    let s = impostor(in);
    // Depth-equal test without depth writes (graphics.rs): early depth testing still works.
    if (s.albedo.a < 0.5) {
        discard;
    }
    var pbr_input = pbr_types::pbr_input_new();
    pbr_input.frag_coord = in.position;
    pbr_input.world_position = in.world_position;
    let n = world_normal(in, s.normal);
    pbr_input.world_normal = n;
    pbr_input.N = n;
    pbr_input.is_orthographic = view.clip_from_view[3].w == 1.0;
    pbr_input.V = calculate_view(in.world_position, pbr_input.is_orthographic);
    pbr_input.material.base_color = vec4(s.albedo.rgb * in.slot_tint.y, 1.0);
    pbr_input.material.perceptual_roughness = 0.75;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3(0.35);
    pbr_input.material.diffuse_transmission = 0.25;
    pbr_input.flags = pbr_input.flags | MESH_FLAGS_SHADOW_RECEIVER_BIT;
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
#endif
