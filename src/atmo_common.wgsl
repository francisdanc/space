// Shared by the atmosphere/ocean pass (atmosphere.wgsl) and inland water (water.wgsl):
// the atmosphere parameters, single scattering, and the optics of a water surface.
#define_import_path space::atmo

#import bevy_pbr::mesh_view_bindings::{view, globals}

struct Atmosphere {
    center: vec3<f32>,      // planet centre relative to the camera (m)
    r_cam: f32,             // camera distance from the planet centre (m)
    sun_dir: vec3<f32>,     // unit vector towards the sun
    k_ground: f32,          // r_cam^2 - bottom_radius^2
    rayleigh: vec3<f32>,    // Rayleigh scattering coefficients at sea level (1/m)
    k_top: f32,             // r_cam^2 - top_radius^2
    bottom_radius: f32,
    top_radius: f32,
    rayleigh_scale: f32,    // scale heights (m)
    mie_scale: f32,
    mie: f32,               // Mie scattering coefficient at sea level (1/m)
    mie_g: f32,             // Mie asymmetry
    sun_intensity: f32,
    shell: f32,             // top_radius^2 - bottom_radius^2
    cam_offset: vec3<f32>,  // camera position (planet frame) modulo WAVE_PERIOD
    sun_illuminance: f32,   // lux on the surface with the sun overhead (terrain lighting)
}

@group(2) @binding(0) var<uniform> atmo: Atmosphere;

const PI: f32 = 3.14159265;
// Few steps, jittered per pixel and frame (TAA averages the noise instead of banding).
const VIEW_STEPS: i32 = 16;
const LIGHT_STEPS: i32 = 4;
const MIE_EXTINCTION: f32 = 1.11;

// Ray from a point at radius r with direction cosine mu (to local up) against a sphere,
// where k = r^2 - R^2. Returns (near, far) distances, or (-1, -1) if missed.
// Uses the numerically stable quadratic form so the small root keeps precision.
fn sphere_hits(r: f32, mu: f32, k: f32) -> vec2<f32> {
    let b = r * mu;
    let disc = b * b - k;
    if (disc < 0.0) {
        return vec2(-1.0, -1.0);
    }
    let s = sqrt(disc);
    var q = -b - s;
    if (b < 0.0) {
        q = -b + s;
    }
    var t2 = 0.0;
    if (abs(q) > 1e-6) {
        t2 = k / q;
    }
    return vec2(min(q, t2), max(q, t2));
}

// Height above bottom_radius after travelling t along a ray from (r, mu), with k = r^2 - Rg^2.
// Returns (height, r_t^2 - Rg^2).
fn height_along(t: f32, r: f32, mu: f32, k: f32) -> vec2<f32> {
    let num = k + t * (t + 2.0 * r * mu);
    let rg = atmo.bottom_radius;
    let rt = sqrt(max(rg * rg + num, 0.0));
    return vec2(num / (rt + rg), num);
}

// Optical depth (Rayleigh, Mie) from a point towards the sun. z = 1 if the sun is behind the planet.
fn light_depth(h: f32, num: f32, mu_s: f32) -> vec3<f32> {
    let r = atmo.bottom_radius + h;
    let ground = sphere_hits(r, mu_s, num);
    if (ground.x > 0.0) {
        return vec3(0.0, 0.0, 1.0);
    }
    let top = sphere_hits(r, mu_s, num - atmo.shell).y;
    if (top <= 0.0) {
        return vec3(0.0);
    }
    let seg = top / f32(LIGHT_STEPS);
    var od = vec2(0.0);
    for (var i = 0; i < LIGHT_STEPS; i++) {
        let hh = height_along((f32(i) + 0.5) * seg, r, mu_s, num).x;
        od += vec2(exp(-hh / atmo.rayleigh_scale), exp(-hh / atmo.mie_scale)) * seg;
    }
    return vec3(od, 0.0);
}

struct Scatter {
    inscatter: vec3<f32>,
    transmittance: vec3<f32>,
}

// Single scattering along a ray that starts at `o` (camera-relative), at distance r0 from the
// planet centre, with direction d (mu0 = cos to local up, k0 = r0^2 - R^2), between t0 and t1.
fn integrate(o: vec3<f32>, d: vec3<f32>, r0: f32, mu0: f32, k0: f32, t0: f32, t1: f32, steps: i32, jitter: f32) -> Scatter {
    let seg = (t1 - t0) / f32(steps);
    var od_view = vec2(0.0);
    var sum_r = vec3(0.0);
    var sum_m = vec3(0.0);
    let mie_ext = atmo.mie * MIE_EXTINCTION;
    for (var i = 0; i < steps; i++) {
        let t = t0 + (f32(i) + jitter) * seg;
        let hn = height_along(t, r0, mu0, k0);
        let h = max(hn.x, 0.0);
        let dr = exp(-h / atmo.rayleigh_scale) * seg;
        let dm = exp(-h / atmo.mie_scale) * seg;
        od_view += vec2(dr, dm) * 0.5;
        let p = o + d * t - atmo.center;
        let mu_s = dot(normalize(p), atmo.sun_dir);
        let ld = light_depth(h, hn.y, mu_s);
        if (ld.z < 0.5) {
            let tau = atmo.rayleigh * (od_view.x + ld.x) + vec3(mie_ext * (od_view.y + ld.y));
            let attn = exp(-tau);
            sum_r += attn * dr;
            sum_m += attn * dm;
        }
        od_view += vec2(dr, dm) * 0.5;
    }
    let c = dot(d, atmo.sun_dir);
    let phase_r = 3.0 / (16.0 * PI) * (1.0 + c * c);
    let g = atmo.mie_g;
    let g2 = g * g;
    let phase_m = 3.0 / (8.0 * PI) * ((1.0 - g2) * (1.0 + c * c)) / ((2.0 + g2) * pow(1.0 + g2 - 2.0 * g * c, 1.5));
    var out: Scatter;
    out.inscatter = atmo.sun_intensity * (sum_r * atmo.rayleigh * phase_r + sum_m * atmo.mie * phase_m);
    out.transmittance = exp(-(atmo.rayleigh * od_view.x + vec3(mie_ext * od_view.y)));
    return out;
}

// ---------------------------------------------------------------------------------- water

const WAVE_PERIOD: f32 = 4096.0;
const G: f32 = 9.81;

// Sunlight reaching sea level where the local up is `up` (per channel, 0 at night), in the
// same exposed units the terrain is lit with.
fn sun_at_sea(up: vec3<f32>) -> vec3<f32> {
    let mu_s = dot(up, atmo.sun_dir);
    let ld = light_depth(0.0, 0.0, mu_s);
    if (ld.z > 0.5) {
        return vec3(0.0);
    }
    let t = exp(-(atmo.rayleigh * ld.x + vec3(atmo.mie * MIE_EXTINCTION * ld.y)));
    return t * atmo.sun_illuminance * view.exposure;
}

// Slope of the sea surface at periodic position q: a sum of travelling waves whose wave vectors
// are integer vectors times 2 pi / WAVE_PERIOD (so the pattern wraps seamlessly), with
// deep-water dispersion and a wind-sea spectrum (long swells gentle, short chop steeper), all
// running roughly downwind. Waves too short to resolve at this distance fade out.
fn wave_slope(q: vec3<f32>, dist: f32, amp: f32) -> vec3<f32> {
    var dirs = array<vec3<f32>, 12>(
        vec3(37.0, -61.0, 22.0), vec3(-53.0, 18.0, 47.0), vec3(29.0, 44.0, -58.0), vec3(-71.0, -9.0, -31.0),
        vec3(13.0, 67.0, 41.0), vec3(61.0, 27.0, 35.0), vec3(-23.0, -49.0, 57.0), vec3(47.0, -38.0, -45.0),
        vec3(-31.0, 59.0, -26.0), vec3(55.0, -14.0, -49.0), vec3(-8.0, -72.0, -29.0), vec3(66.0, 41.0, -11.0),
    );
    var scales = array<f32, 12>(1.0, 2.0, 3.0, 4.0, 6.0, 9.0, 13.0, 18.0, 26.0, 37.0, 53.0, 75.0);
    var g = vec3(0.0);
    let t = globals.time;
    for (var i = 0; i < 12; i++) {
        let kv = dirs[i] * scales[i] * (2.0 * PI / WAVE_PERIOD);
        let k = length(kv);
        let wavelength = 2.0 * PI / k;
        let fade = 1.0 - smoothstep(wavelength * 80.0, wavelength * 320.0, dist);
        // Slope amplitude: ~0.03 for 50 m swell up to ~0.09 for 1 m chop.
        let steep = 0.09 * pow(wavelength, -0.28) * fade * amp;
        let phase = dot(kv, q) - sqrt(G * k) * t + f32(i) * 1.7;
        g += (kv / k) * (steep * cos(phase));
    }
    return g;
}

fn hash3(p: vec3<f32>) -> f32 {
    let q = fract(p * 0.1031);
    let r = q + dot(q, q.yzx + 33.33);
    return fract((r.x + r.y) * r.z);
}

// Smooth value noise (0..1).
fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let n000 = hash3(i);
    let n100 = hash3(i + vec3(1.0, 0.0, 0.0));
    let n010 = hash3(i + vec3(0.0, 1.0, 0.0));
    let n110 = hash3(i + vec3(1.0, 1.0, 0.0));
    let n001 = hash3(i + vec3(0.0, 0.0, 1.0));
    let n101 = hash3(i + vec3(1.0, 0.0, 1.0));
    let n011 = hash3(i + vec3(0.0, 1.0, 1.0));
    let n111 = hash3(i + vec3(1.0, 1.0, 1.0));
    return mix(mix(mix(n000, n100, u.x), mix(n010, n110, u.x), u.y), mix(mix(n001, n101, u.x), mix(n011, n111, u.x), u.y), u.z);
}

// Optics of a water surface at camera-relative point p (local up `up`, wave normal n, seen
// along d), standing `level` m above sea level, with `path` m of water in front of whatever
// lies behind it: sky reflection (the atmosphere integrated along the reflected ray), sun
// glitter, light scattered back up from inside the water, and how much of the bottom shows.
struct WaterOut {
    rgb: vec3<f32>,
    alpha: f32,
    // Light falling on the water (for foam).
    e_in: vec3<f32>,
}

fn water_optics(p: vec3<f32>, up: vec3<f32>, n: vec3<f32>, d: vec3<f32>, level: f32, path: f32, jitter: f32) -> WaterOut {
    let v = -d;
    let ndv = max(dot(n, v), 1e-3);
    var rd = reflect(d, n);
    let mu_r0 = dot(rd, up);
    if (mu_r0 < 0.02) {
        rd = normalize(rd + up * (0.02 - mu_r0));
    }
    let mu_r = dot(rd, up);
    let r0 = atmo.bottom_radius + level;
    let k0 = level * (2.0 * atmo.bottom_radius + level);
    let exit = sphere_hits(r0, mu_r, k0 - atmo.shell).y;
    let sky = integrate(p, rd, r0, mu_r, k0, 0.0, max(exit, 1.0), 6, jitter).inscatter;
    let fresnel = 0.02 + 0.98 * pow(1.0 - ndv, 5.0);

    let e_sun = sun_at_sea(up);
    let l = atmo.sun_dir;
    let hv = normalize(l + v);
    let ndh = max(dot(n, hv), 0.0);
    let ndl = dot(n, l);
    let a2 = 0.006;
    let dd = ndh * ndh * (a2 - 1.0) + 1.0;
    let spec_d = a2 / (PI * dd * dd);
    let f_sun = 0.02 + 0.98 * pow(1.0 - max(dot(hv, v), 0.0), 5.0);
    let spec = e_sun * min(spec_d * f_sun / (4.0 * ndv), 60.0) * step(0.0, ndl);

    let opacity = 1.0 - exp(-0.22 * path);
    let e_amb = (e_sun.r + e_sun.g + e_sun.b) * 0.12 + 0.002;
    let e_in = e_sun * max(dot(up, l), 0.0) + vec3(e_amb);
    let albedo = mix(vec3(0.06, 0.26, 0.24), vec3(0.008, 0.045, 0.075), opacity);
    let body = albedo * e_in / PI;

    var out: WaterOut;
    out.rgb = fresnel * sky + spec + (1.0 - fresnel) * opacity * body;
    out.alpha = 1.0 - (1.0 - fresnel) * (1.0 - opacity);
    out.e_in = e_in;
    return out;
}
