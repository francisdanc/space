//! Deterministic 3D gradient (Perlin) noise in f64, plus fBm and ridged multifractal.
//! f64 everywhere so planet-scale coordinates keep precision down to centimetres.

use bevy::math::DVec3;

pub struct Perlin {
    perm: [u8; 512],
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn fade_d(t: f64) -> f64 {
    30.0 * t * t * (t - 1.0) * (t - 1.0)
}

/// The gradient vector that `grad` dots with the offset.
#[inline]
fn grad_vec(hash: u8) -> DVec3 {
    let h = hash & 15;
    let su = if h & 1 == 0 { 1.0 } else { -1.0 };
    let sv = if h & 2 == 0 { 1.0 } else { -1.0 };
    let mut g = DVec3::ZERO;
    if h < 8 {
        g.x += su;
    } else {
        g.y += su;
    }
    if h < 4 {
        g.y += sv;
    } else if h == 12 || h == 14 {
        g.x += sv;
    } else {
        g.z += sv;
    }
    g
}

#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

#[inline]
fn grad(hash: u8, x: f64, y: f64, z: f64) -> f64 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

/// Per-octave offsets so octaves don't line up at the origin.
#[inline]
fn octave_offset(i: u32) -> DVec3 {
    let f = i as f64;
    DVec3::new(f * 17.31 + 0.5, f * -9.17 + 3.1, f * 5.73 - 7.7)
}

impl Perlin {
    pub fn new(seed: u64) -> Self {
        let mut p = [0u8; 256];
        for (i, v) in p.iter_mut().enumerate() {
            *v = i as u8;
        }
        let mut s = seed;
        for i in (1..256).rev() {
            let j = (splitmix(&mut s) % (i as u64 + 1)) as usize;
            p.swap(i, j);
        }
        let mut perm = [0u8; 512];
        for i in 0..512 {
            perm[i] = p[i & 255];
        }
        Self { perm }
    }

    /// Raw noise, roughly in [-1, 1].
    #[inline]
    pub fn get(&self, p: DVec3) -> f64 {
        let fx = p.x.floor();
        let fy = p.y.floor();
        let fz = p.z.floor();
        let xi = (fx as i64 & 255) as usize;
        let yi = (fy as i64 & 255) as usize;
        let zi = (fz as i64 & 255) as usize;
        let x = p.x - fx;
        let y = p.y - fy;
        let z = p.z - fz;
        let u = fade(x);
        let v = fade(y);
        let w = fade(z);
        let pm = &self.perm;
        let a = pm[xi] as usize + yi;
        let aa = pm[a] as usize + zi;
        let ab = pm[a + 1] as usize + zi;
        let b = pm[xi + 1] as usize + yi;
        let ba = pm[b] as usize + zi;
        let bb = pm[b + 1] as usize + zi;
        lerp(
            w,
            lerp(
                v,
                lerp(u, grad(pm[aa], x, y, z), grad(pm[ba], x - 1.0, y, z)),
                lerp(u, grad(pm[ab], x, y - 1.0, z), grad(pm[bb], x - 1.0, y - 1.0, z)),
            ),
            lerp(
                v,
                lerp(
                    u,
                    grad(pm[aa + 1], x, y, z - 1.0),
                    grad(pm[ba + 1], x - 1.0, y, z - 1.0),
                ),
                lerp(
                    u,
                    grad(pm[ab + 1], x, y - 1.0, z - 1.0),
                    grad(pm[bb + 1], x - 1.0, y - 1.0, z - 1.0),
                ),
            ),
        )
    }

    /// Noise value and its gradient with respect to `p`.
    pub fn get_d(&self, p: DVec3) -> (f64, DVec3) {
        let fx = p.x.floor();
        let fy = p.y.floor();
        let fz = p.z.floor();
        let xi = (fx as i64 & 255) as usize;
        let yi = (fy as i64 & 255) as usize;
        let zi = (fz as i64 & 255) as usize;
        let x = p.x - fx;
        let y = p.y - fy;
        let z = p.z - fz;
        let (u, v, w) = (fade(x), fade(y), fade(z));
        let (du, dv, dw) = (fade_d(x), fade_d(y), fade_d(z));
        let pm = &self.perm;
        let a = pm[xi] as usize + yi;
        let aa = pm[a] as usize + zi;
        let ab = pm[a + 1] as usize + zi;
        let b = pm[xi + 1] as usize + yi;
        let ba = pm[b] as usize + zi;
        let bb = pm[b + 1] as usize + zi;
        // Corner gradients and their dot products with the offset to each corner.
        let g = [
            grad_vec(pm[aa]),
            grad_vec(pm[ba]),
            grad_vec(pm[ab]),
            grad_vec(pm[bb]),
            grad_vec(pm[aa + 1]),
            grad_vec(pm[ba + 1]),
            grad_vec(pm[ab + 1]),
            grad_vec(pm[bb + 1]),
        ];
        let o = |dx: f64, dy: f64, dz: f64| DVec3::new(x - dx, y - dy, z - dz);
        let n000 = g[0].dot(o(0.0, 0.0, 0.0));
        let n100 = g[1].dot(o(1.0, 0.0, 0.0));
        let n010 = g[2].dot(o(0.0, 1.0, 0.0));
        let n110 = g[3].dot(o(1.0, 1.0, 0.0));
        let n001 = g[4].dot(o(0.0, 0.0, 1.0));
        let n101 = g[5].dot(o(1.0, 0.0, 1.0));
        let n011 = g[6].dot(o(0.0, 1.0, 1.0));
        let n111 = g[7].dot(o(1.0, 1.0, 1.0));
        // Trilinear blend written as a polynomial so it can be differentiated.
        let k1 = n100 - n000;
        let k2 = n010 - n000;
        let k3 = n001 - n000;
        let k4 = n110 - n010 - n100 + n000;
        let k5 = n011 - n001 - n010 + n000;
        let k6 = n101 - n001 - n100 + n000;
        let k7 = n111 - n011 - n101 + n001 - n110 + n010 + n100 - n000;
        let value = n000 + k1 * u + k2 * v + k3 * w + k4 * u * v + k5 * v * w + k6 * w * u + k7 * u * v * w;
        let lerp3 = |c: [DVec3; 8]| {
            let x0 = c[0].lerp(c[1], u);
            let x1 = c[2].lerp(c[3], u);
            let x2 = c[4].lerp(c[5], u);
            let x3 = c[6].lerp(c[7], u);
            x0.lerp(x1, v).lerp(x2.lerp(x3, v), w)
        };
        let grad = lerp3(g)
            + DVec3::new(
                du * (k1 + k4 * v + k6 * w + k7 * v * w),
                dv * (k2 + k5 * w + k4 * u + k7 * w * u),
                dw * (k3 + k6 * u + k5 * v + k7 * u * v),
            );
        (value, grad)
    }

    /// "Eroded" fBm: each octave is damped where the octaves below it are steep, so slopes stay
    /// smooth while ridges sharpen and gullies form, like weathered terrain (after Inigo
    /// Quilez's derivative-damped fBm). `normal` is the surface direction: only the slope along
    /// the surface counts. Roughly in [-0.6, 0.6].
    pub fn eroded_fbm(&self, p: DVec3, octaves: u32, normal: DVec3, strength: f64) -> f64 {
        let mut sum = 0.0;
        let mut amp = 1.0;
        let mut freq = 1.0;
        let mut norm = 0.0;
        let mut slope = DVec3::ZERO;
        for i in 0..octaves {
            let (n, g) = self.get_d(p * freq + octave_offset(i));
            slope += g - normal * g.dot(normal);
            sum += amp * n / (1.0 + strength * slope.length_squared());
            norm += amp;
            amp *= 0.5;
            freq *= 2.03;
        }
        sum / norm
    }

    /// Fractal Brownian motion, normalised so the result is roughly in [-0.7, 0.7].
    pub fn fbm(&self, p: DVec3, octaves: u32) -> f64 {
        let mut sum = 0.0;
        let mut amp = 1.0;
        let mut freq = 1.0;
        let mut norm = 0.0;
        for i in 0..octaves {
            sum += amp * self.get(p * freq + octave_offset(i));
            norm += amp;
            amp *= 0.5;
            freq *= 2.03;
        }
        sum / norm
    }

    /// Ridged multifractal: sharp mountain ridges, result roughly in [0, 1].
    pub fn ridged(&self, p: DVec3, octaves: u32) -> f64 {
        let mut sum = 0.0;
        let mut amp = 0.5;
        let mut freq = 1.0;
        let mut weight = 1.0;
        let mut norm = 0.0;
        for i in 0..octaves {
            let mut n = 1.0 - self.get(p * freq + octave_offset(i)).abs();
            n *= n;
            n *= weight;
            weight = (n * 2.0).clamp(0.0, 1.0);
            sum += n * amp;
            norm += amp;
            amp *= 0.5;
            freq *= 2.1;
        }
        sum / norm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_bounded() {
        let a = Perlin::new(42);
        let b = Perlin::new(42);
        let mut max: f64 = 0.0;
        for i in 0..10_000 {
            let p = DVec3::new(i as f64 * 0.137, i as f64 * 0.071 - 50.0, i as f64 * -0.0313);
            let v = a.get(p);
            assert_eq!(v, b.get(p));
            assert!(v.is_finite());
            max = max.max(v.abs());
        }
        assert!(max <= 1.1, "noise out of range: {max}");
        assert!(max > 0.3, "noise suspiciously flat: {max}");
    }

    #[test]
    fn derivatives_match_value_and_finite_differences() {
        let n = Perlin::new(9);
        let e = 1e-5;
        for i in 0..500 {
            let p = DVec3::new(i as f64 * 0.377 - 40.0, i as f64 * 0.191, i as f64 * -0.533 + 7.0);
            let (v, g) = n.get_d(p);
            assert!((v - n.get(p)).abs() < 1e-9, "value mismatch at {p}");
            let fd = DVec3::new(
                (n.get(p + DVec3::X * e) - n.get(p - DVec3::X * e)) / (2.0 * e),
                (n.get(p + DVec3::Y * e) - n.get(p - DVec3::Y * e)) / (2.0 * e),
                (n.get(p + DVec3::Z * e) - n.get(p - DVec3::Z * e)) / (2.0 * e),
            );
            assert!((g - fd).length() < 1e-4, "gradient {g} vs finite difference {fd} at {p}");
        }
    }
}
