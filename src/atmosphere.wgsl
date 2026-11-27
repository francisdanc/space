// Single-scattering atmosphere (Rayleigh + Mie), ray-marched per pixel.
//
// Drawn on the back faces of a sphere around the planet, with depth testing off, as a
// premultiplied-alpha overlay: out = inscattered light + scene * (1 - alpha).
// The scene depth from the depth prepass stops the ray at terrain/ships (aerial perspective).
//
// Precision: the world is camera-relative and planet-sized, so values like |c|^2 - R^2 would
// cancel catastrophically in f32. The CPU computes those in f64 (k_ground, k_top, shell) and the
// shader only ever works with differences that stay small near the camera.

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, globals},
    prepass_utils,
    view_transformations::{frag_coord_to_ndc, position_ndc_to_world},
}
#import space::atmo::{
    atmo, PI, VIEW_STEPS, sphere_hits, height_along, integrate, sun_at_sea, wave_slope, vnoise,
    water_optics,
}

// Premultiplied colour (rgb) and coverage (a) of the sea surface at distance t_w along d, with
// the scene (sea floor) at t_scene.
fn shade_water(d: vec3<f32>, t_w: f32, t_scene: f32, r: f32, mu: f32, jitter: f32) -> vec4<f32> {
    let p = d * t_w;
    let up = normalize(p - atmo.center);
    let q = atmo.cam_offset + p;
    // Wave normal: tilt the local up by the slope's tangential part.
    let slope = wave_slope(q, t_w, 1.0);
    let n = normalize(up - (slope - up * dot(slope, up)));
    let w = water_optics(p, up, n, d, 0.0, max(t_scene - t_w, 0.0), jitter);
    var rgb = w.rgb;
    var alpha = w.alpha;

    // Surf: a band of foam along the waterline that runs up the beach and back every few
    // seconds, broken up by noise, plus thin streaks just offshore.
    let floor_depth = -height_along(t_scene, r, mu, atmo.k_ground).x;
    if (floor_depth < 1.2 && t_w < 4000.0) {
        let tm = globals.time;
        let n1 = vnoise(q * 0.35 + vec3(0.0, tm * 0.15, 0.0));
        let n2 = vnoise(q * 1.7 - vec3(tm * 0.3, 0.0, 0.0));
        let surge = 0.18 + 0.14 * sin(tm * 0.55 + n1 * 4.0);
        let band = 1.0 - smoothstep(surge * 0.4, surge, floor_depth);
        let streak = smoothstep(0.55, 0.75, n1) * (1.0 - smoothstep(0.3, 1.2, floor_depth)) * 0.6;
        let foam = clamp((band + streak) * smoothstep(0.25, 0.65, n2 + band * 0.4), 0.0, 1.0);
        let foam_col = 0.75 * w.e_in / PI;
        rgb = mix(rgb, foam_col, foam);
        alpha = mix(alpha, 1.0, foam);
    }
    return vec4(rgb, alpha);
}

// Camera under the sea: the water is the medium. Light fades with depth, distant things fade
// into blue-green, and the surface overhead glows.
fn underwater(d: vec3<f32>, r: f32, mu: f32, t_scene: f32) -> vec4<f32> {
    let up = -normalize(atmo.center);
    let cam_depth = -atmo.k_ground / (r + atmo.bottom_radius);
    let e_sun = sun_at_sea(up);
    let light = (e_sun * max(dot(up, atmo.sun_dir), 0.0) + vec3((e_sun.r + e_sun.g + e_sun.b) * 0.12 + 0.002))
        * exp(-vec3(0.35, 0.08, 0.05) * cam_depth);
    let fog_col = vec3(0.01, 0.06, 0.08) * light / PI;
    let surf = sphere_hits(r, mu, atmo.k_ground).y;
    var path = t_scene;
    if (surf > 0.0) {
        path = min(path, surf);
    }
    let fog = 1.0 - exp(-0.05 * path);
    if (surf > 0.0 && surf < t_scene) {
        // The underside of the surface: bright inside Snell's window, a dim mirror outside it.
        let window = smoothstep(0.62, 0.7, mu);
        let surface = mix(fog_col * 1.5, fog_col * 6.0 + e_sun * 0.02, window);
        return vec4(mix(surface, fog_col, fog), 1.0);
    }
    return vec4(fog_col * fog, fog);
}

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
    let d = normalize(in.world_position.xyz - view.world_position);

    // Distance to whatever opaque thing is in this pixel (terrain, ship, stars at "infinity").
    var t_scene = 1e30;
    let depth = prepass_utils::prepass_depth(in.position, sample_index);
    if (depth > 0.0) {
        let ndc = frag_coord_to_ndc(vec4(in.position.xy, depth, 1.0));
        t_scene = length(position_ndc_to_world(ndc) - view.world_position);
    }

    let up = -normalize(atmo.center);
    let mu = dot(d, up);
    let r = atmo.r_cam;
    // Few steps, jittered per pixel and frame (TAA averages the noise instead of banding).
    let jitter = fract(52.9829189 * fract(dot(in.position.xy + vec2(f32(view.frame_count % 64u) * 5.588, 0.0), vec2(0.06711056, 0.00583715))));

    if (atmo.k_ground < 0.0) {
        return underwater(d, r, mu, t_scene);
    }

    let top = sphere_hits(r, mu, atmo.k_top);
    if (top.y <= 0.0) {
        return vec4(0.0);
    }
    let t0 = max(top.x, 0.0);
    var t1 = top.y;
    // The sea: a perfect sphere at sea level, wherever it lies in front of the scene.
    var water = vec4(0.0);
    let ground = sphere_hits(r, mu, atmo.k_ground);
    if (ground.x > 0.0) {
        t1 = min(t1, ground.x);
        if (ground.x < t_scene) {
            water = shade_water(d, ground.x, t_scene, r, mu, jitter);
        }
    }
    t1 = min(t1, t_scene);
    if (t1 <= t0) {
        return water;
    }
    let a = integrate(vec3(0.0), d, r, mu, atmo.k_ground, t0, t1, VIEW_STEPS, jitter);
    // Air in front of the water: out = A + T * (W + (1 - w) * scene).
    let t_avg = (a.transmittance.r + a.transmittance.g + a.transmittance.b) / 3.0;
    return vec4(a.inscatter + a.transmittance * water.rgb, clamp(1.0 - t_avg * (1.0 - water.a), 0.0, 1.0));
}
