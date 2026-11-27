// Rivers and lakes: water surface meshes at the river/lake level (terrain.rs water_mesh),
// shaded with the same optics as the sea (atmo_common.wgsl): depth-based colour from the
// depth prepass, sky reflection, sun glitter. Rivers flow: their ripples slide downstream
// (two cross-faded phases so offsets stay bounded), and steep stretches run white.
//
// Vertex data: uv.x = water level (m above sea level), tangent = flow direction (xyz) and
// gradient (w); zero flow on lakes.

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, globals},
    prepass_utils,
    view_transformations::{frag_coord_to_ndc, position_ndc_to_world},
}
#import space::atmo::{atmo, PI, wave_slope, vnoise, water_optics}

@fragment
fn fragment(
#ifdef MULTISAMPLED
    @builtin(sample_index) sample_index: u32,
#endif
    in: VertexOutput,
) -> @location(0) vec4<f32> {
#ifndef MULTISAMPLED
    let sample_index = 0u;
#endif
    let p = in.world_position.xyz - view.world_position;
    let t_w = length(p);
    let d = p / t_w;

    var t_scene = 1e30;
    let depth = prepass_utils::prepass_depth(in.position, sample_index);
    if (depth > 0.0) {
        let ndc = frag_coord_to_ndc(vec4(in.position.xy, depth, 1.0));
        t_scene = length(position_ndc_to_world(ndc) - view.world_position);
    }

    let up = normalize(in.world_normal);
    let level = in.uv.x;
    let flow = in.world_tangent.xyz;
    let gradient = in.world_tangent.w;
    let flowing = length(flow) > 0.5;
    let q = atmo.cam_offset + p;

    // Ripples carried downstream: two copies of the wave pattern sliding along the flow, each
    // reset every cycle while the other is at full weight.
    let speed = select(0.0, clamp(0.8 + gradient * 300.0, 0.8, 4.0), flowing);
    let cycle = 6.0;
    let ph0 = fract(globals.time * speed / cycle);
    let ph1 = fract(ph0 + 0.5);
    let amp = select(0.45, 0.6, flowing);
    let s0 = wave_slope(q - flow * (ph0 * cycle), t_w, amp);
    let s1 = wave_slope(q - flow * (ph1 * cycle) + vec3(17.3, 5.1, 9.7), t_w, amp);
    let wgt = abs(ph0 * 2.0 - 1.0);
    let slope = mix(s0, s1, 1.0 - wgt);
    let n = normalize(up - (slope - up * dot(slope, up)));

    let path = max(t_scene - t_w, 0.0);
    let jitter = fract(52.9829189 * fract(dot(in.position.xy + vec2(f32(view.frame_count % 64u) * 5.588, 0.0), vec2(0.06711056, 0.00583715))));
    let w = water_optics(p, up, n, d, level, path, jitter);
    var rgb = w.rgb;
    var alpha = w.alpha;

    // White water: rapids on steep stretches, and a little foam where the water is very
    // shallow along the banks; both streak downstream with the current.
    let floor_depth = path * abs(dot(d, up));
    let drift = q - flow * (globals.time * speed);
    let n1 = vnoise(drift * vec3(0.9, 0.9, 0.9));
    let n2 = vnoise(drift * 3.1 + vec3(7.0, 1.0, 3.0));
    let rapids = smoothstep(0.003, 0.02, gradient) * smoothstep(0.35, 0.7, n1 * 0.7 + n2 * 0.5);
    let bank = (1.0 - smoothstep(0.0, 0.25, floor_depth)) * smoothstep(0.45, 0.75, n2) * select(0.3, 0.7, flowing);
    let foam = clamp(rapids + bank, 0.0, 1.0) * (1.0 - smoothstep(800.0, 2500.0, t_w));
    rgb = mix(rgb, 0.75 * w.e_in / PI, foam);
    alpha = mix(alpha, 1.0, foam);
    return vec4(rgb, alpha);
}
