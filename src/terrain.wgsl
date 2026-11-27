// Terrain material: 7 photo-scanned ground layers (+ water) splatted by per-vertex weights,
// triplanar-mapped in planet coordinates, height-blended, with two tiling scales and macro
// variation to hide repetition. Lighting is Bevy's PBR (shadows, SSAO, environment light).
//
// Per-vertex weights: color = (grass, forest litter, moss, dirt), uv = (sand, rock),
// uv_b = (snow, distant canopy). All land weights 0 = water.
//
// Texture coordinates come from world_position + cam_offset, where cam_offset is the camera's
// planet position modulo PERIOD (computed in f64 on the CPU). Every tile size divides PERIOD,
// so the wrap is invisible and coordinates stay small and precise near the camera.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, globals},
}

struct TerrainParams {
    cam_offset: vec4<f32>,
    canopy: vec4<f32>,
    tints: array<vec4<f32>, 8>,
    planet_center: vec4<f32>,
}

@group(2) @binding(100) var<uniform> terrain: TerrainParams;
@group(2) @binding(101) var albedo_tex: texture_2d_array<f32>;
@group(2) @binding(102) var albedo_smp: sampler;
@group(2) @binding(103) var normal_tex: texture_2d_array<f32>;
@group(2) @binding(104) var normal_smp: sampler;
@group(2) @binding(105) var arh_tex: texture_2d_array<f32>;
@group(2) @binding(106) var arh_smp: sampler;
// Planet-wide layer weights, ~10 km per texel (surface_map.rs).
@group(2) @binding(107) var surface0_tex: texture_cube<f32>;
@group(2) @binding(108) var surface_smp: sampler;
@group(2) @binding(109) var surface1_tex: texture_cube<f32>;

const PERIOD: f32 = 4096.0;
const FAR_SCALE: f32 = 4.0;

struct Surface {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    ao: f32,
    rough: f32,
    height: f32,
}

// Tile size (metres) of each layer.
fn tile(layer: i32) -> f32 {
    switch layer {
        case 5: { return 8.0; }      // rock
        case 2, 3, 4, 6: { return 4.0; }
        default: { return 2.0; }     // grass, litter
    }
}

// Periodic value noise (period PERIOD) for large-scale variation.
fn hash3(p: vec3<i32>) -> f32 {
    var h = u32(p.x) * 73856093u ^ u32(p.y) * 19349663u ^ u32(p.z) * 83492791u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    return f32(h & 0xffffu) / 65535.0;
}

fn vnoise(p: vec3<f32>, cell: f32) -> f32 {
    let n = i32(PERIOD / cell);
    let q = p / cell;
    let i = floor(q);
    let f = q - i;
    let u = f * f * (3.0 - 2.0 * f);
    let ii = vec3<i32>(i);
    var acc = 0.0;
    for (var k = 0; k < 8; k++) {
        let o = vec3<i32>(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        let c = ((ii + o) % n + n) % n;
        let w = select(1.0 - u.x, u.x, o.x == 1) * select(1.0 - u.y, u.y, o.y == 1) * select(1.0 - u.z, u.z, o.z == 1);
        acc += w * hash3(c);
    }
    return acc;
}

// Triplanar sample of one layer at one scale ("whiteout" normal blending, after Ben Golus).
fn sample_layer(layer: i32, p: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, bw: vec3<f32>, n: vec3<f32>, scale: f32) -> Surface {
    var s: Surface;
    s.albedo = vec3(0.0);
    s.ao = 0.0;
    s.rough = 0.0;
    s.height = 0.0;
    var nsum = vec3(0.0);
    let k = 1.0 / scale;
    // X plane (uv = zy), Y plane (uv = xz), Z plane (uv = xy).
    if (bw.x > 0.02) {
        let uv = p.zy * k;
        let a = textureSampleGrad(albedo_tex, albedo_smp, uv, layer, dpx.zy * k, dpy.zy * k);
        let t = textureSampleGrad(normal_tex, normal_smp, uv, layer, dpx.zy * k, dpy.zy * k).xyz * 2.0 - 1.0;
        let m = textureSampleGrad(arh_tex, arh_smp, uv, layer, dpx.zy * k, dpy.zy * k);
        let tn = vec3(t.xy + n.zy, abs(t.z) * n.x);
        nsum += tn.zyx * bw.x;
        s.albedo += a.rgb * bw.x; s.ao += m.r * bw.x; s.rough += m.g * bw.x; s.height += m.b * bw.x;
    }
    if (bw.y > 0.02) {
        let uv = p.xz * k;
        let a = textureSampleGrad(albedo_tex, albedo_smp, uv, layer, dpx.xz * k, dpy.xz * k);
        let t = textureSampleGrad(normal_tex, normal_smp, uv, layer, dpx.xz * k, dpy.xz * k).xyz * 2.0 - 1.0;
        let m = textureSampleGrad(arh_tex, arh_smp, uv, layer, dpx.xz * k, dpy.xz * k);
        let tn = vec3(t.xy + n.xz, abs(t.z) * n.y);
        nsum += tn.xzy * bw.y;
        s.albedo += a.rgb * bw.y; s.ao += m.r * bw.y; s.rough += m.g * bw.y; s.height += m.b * bw.y;
    }
    if (bw.z > 0.02) {
        let uv = p.xy * k;
        let a = textureSampleGrad(albedo_tex, albedo_smp, uv, layer, dpx.xy * k, dpy.xy * k);
        let t = textureSampleGrad(normal_tex, normal_smp, uv, layer, dpx.xy * k, dpy.xy * k).xyz * 2.0 - 1.0;
        let m = textureSampleGrad(arh_tex, arh_smp, uv, layer, dpx.xy * k, dpy.xy * k);
        let tn = vec3(t.xy + n.xy, abs(t.z) * n.z);
        nsum += tn.xyz * bw.z;
        s.albedo += a.rgb * bw.z; s.ao += m.r * bw.z; s.rough += m.g * bw.z; s.height += m.b * bw.z;
    }
    let total = max(bw.x * f32(bw.x > 0.02) + bw.y * f32(bw.y > 0.02) + bw.z * f32(bw.z > 0.02), 1e-4);
    s.albedo /= total; s.ao /= total; s.rough /= total; s.height /= total;
    s.normal = normalize(nsum);
    return s;
}

// A layer at the right scale(s) for this distance.
fn layer_surface(layer: i32, p: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, bw: vec3<f32>, n: vec3<f32>, dist: f32) -> Surface {
    let s = tile(layer);
    let far = smoothstep(12.0, 60.0, dist);
    var out: Surface;
    if (far < 0.99) {
        out = sample_layer(layer, p, dpx, dpy, bw, n, s);
    }
    if (far > 0.01) {
        let f = sample_layer(layer, p, dpx, dpy, bw, n, s * FAR_SCALE);
        if (far >= 0.99) {
            out = f;
        } else {
            out.albedo = mix(out.albedo, f.albedo, far);
            out.normal = normalize(mix(out.normal, f.normal, far));
            out.ao = mix(out.ao, f.ao, far);
            out.rough = mix(out.rough, f.rough, far);
            out.height = mix(out.height, f.height, far);
        }
    }
    out.albedo *= terrain.tints[layer].rgb;
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let n = normalize(in.world_normal);
    let p = in.world_position.xyz + terrain.cam_offset.xyz;
    let dpx = dpdx(p);
    let dpy = dpdy(p);
    let dist = length(in.world_position.xyz - view.world_position);

    var w = array<f32, 8>(in.color.r, in.color.g, in.color.b, in.color.a, in.uv.x, in.uv.y, in.uv_b.x, 0.0);
    var canopy = in.uv_b.y;

    // Far away the chunk vertices are tens of km apart; take the layer weights per pixel from
    // the planet-wide surface map instead, so biomes and coasts don't turn into blocky patches.
    let dir = normalize(in.world_position.xyz - terrain.planet_center.xyz);
    let m0 = textureSample(surface0_tex, surface_smp, dir);
    let m1 = textureSample(surface1_tex, surface_smp, dir);
    let map_w = smoothstep(400000.0, 1200000.0, dist);
    let mw = array<f32, 8>(m0.r, m0.g, m0.b, m0.a, m1.r, m1.g, m1.b, 0.0);
    for (var i = 0; i < 7; i++) { w[i] = mix(w[i], mw[i], map_w); }
    canopy = mix(canopy, m1.a, map_w);
    var land = 0.0;
    for (var i = 0; i < 7; i++) { land += w[i]; }

    var bw = pow(abs(n), vec3(4.0));
    bw /= (bw.x + bw.y + bw.z);

    var albedo = vec3(0.0);
    var normal = n;
    var ao = 1.0;
    var rough = 0.9;

    if (land > 0.02) {
        // The three strongest layers.
        var idx = array<i32, 3>(0, 0, 0);
        var best = array<f32, 3>(-1.0, -1.0, -1.0);
        for (var i = 0; i < 7; i++) {
            let v = w[i];
            if (v > best[0]) {
                best[2] = best[1]; idx[2] = idx[1];
                best[1] = best[0]; idx[1] = idx[0];
                best[0] = v; idx[0] = i;
            } else if (v > best[1]) {
                best[2] = best[1]; idx[2] = idx[1];
                best[1] = v; idx[1] = i;
            } else if (v > best[2]) {
                best[2] = v; idx[2] = i;
            }
        }
        var s: array<Surface, 3>;
        var hw = array<f32, 3>(0.0, 0.0, 0.0);
        var hmax = -10.0;
        for (var k = 0; k < 3; k++) {
            if (best[k] > 0.01) {
                s[k] = layer_surface(idx[k], p, dpx, dpy, bw, n, dist);
                // Height-based blending: higher texels (stones, tufts) win at transitions.
                hw[k] = s[k].height * 0.6 + best[k] / land;
                hmax = max(hmax, hw[k]);
            }
        }
        var wsum = 0.0;
        for (var k = 0; k < 3; k++) {
            hw[k] = select(0.0, max(hw[k] - (hmax - 0.25), 0.0), best[k] > 0.01);
            wsum += hw[k];
        }
        var nb = vec3(0.0);
        ao = 0.0; rough = 0.0;
        for (var k = 0; k < 3; k++) {
            let f = hw[k] / max(wsum, 1e-4);
            albedo += s[k].albedo * f;
            nb += s[k].normal * f;
            ao += s[k].ao * f;
            rough += s[k].rough * f;
        }
        normal = normalize(nb);

        // Large-scale variation so repeating tiles don't show from a distance.
        let macro_v = vnoise(p, 32.0) * 0.6 + vnoise(p, 256.0) * 0.4;
        albedo *= 0.8 + 0.4 * macro_v;

        // Distant forest: the canopy colour stands in for trees that are no longer drawn.
        if (canopy > 0.01) {
            let leafy = terrain.canopy.rgb * (0.65 + 0.7 * vnoise(p, 16.0));
            albedo = mix(albedo, leafy, canopy);
            rough = mix(rough, 0.95, canopy);
            normal = normalize(mix(normal, n, canopy));
        }
    }
    if (land < 0.98) {
        // Water: deep colour, mirror-like, with animated wave normals in the surface plane.
        let t = globals.time;
        let q = p * 0.25;
        let g = vec3(
            sin(q.x * 1.7 + t * 1.1) + 0.5 * sin(q.z * 3.1 - t * 1.7) + 0.3 * sin((q.y + q.x) * 5.3 + t * 2.3),
            sin(q.y * 1.9 - t * 0.9) + 0.5 * sin(q.x * 2.9 + t * 1.5) + 0.3 * sin((q.z - q.y) * 4.7 - t * 2.1),
            sin(q.z * 2.3 + t * 1.3) + 0.5 * sin(q.y * 3.7 - t * 1.2) + 0.3 * sin((q.x - q.z) * 6.1 + t * 1.9),
        );
        let fade = 1.0 - smoothstep(50.0, 600.0, dist);
        let wave_n = normalize(n + (g - n * dot(g, n)) * 0.04 * fade);
        let water_w = 1.0 - clamp(land, 0.0, 1.0);
        albedo = mix(albedo, vec3(0.01, 0.035, 0.045), water_w);
        normal = normalize(mix(normal, wave_n, water_w));
        rough = mix(rough, 0.04, water_w);
        ao = mix(ao, 1.0, water_w);
    }

    pbr_input.material.base_color = vec4(albedo, 1.0);
    pbr_input.material.perceptual_roughness = clamp(rough, 0.03, 1.0);
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3(0.5);
    pbr_input.diffuse_occlusion = vec3(ao);
    pbr_input.N = normal;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
