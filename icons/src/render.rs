//! Software renderer that reproduces how the game draws an item in a GUI slot:
//! the `display.gui` transform, an orthographic view, back-face culling, a depth test
//! and the same two directional lights.

use crate::mesh::Quad;
use crate::model::Transform;
use crate::texture::Image;

type M3 = [[f64; 3]; 3];

fn mul(a: M3, b: M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, slot) in row.iter_mut().enumerate() {
            *slot = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn apply(m: M3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| (0..3).map(|k| m[i][k] * v[k]).sum())
}

fn rot_x(a: f64) -> M3 {
    let (s, c) = a.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}

fn rot_y(a: f64) -> M3 {
    let (s, c) = a.sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn rot_z(a: f64) -> M3 {
    let (s, c) = a.sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The two light directions of `Lighting.setupFor3DItems` and `setupForFlatItems`, in the
/// space where the GUI's flipped Y has already been applied to normals.
fn lights(front: bool) -> [[f64; 3]; 2] {
    use std::f64::consts::PI;
    let base = mul(rot_y(-PI / 8.0), rot_x(PI * 3.0 / 4.0));
    let matrix = if front {
        base
    } else {
        let flip = [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]];
        mul(mul(mul(flip, rot_y(1.0821041)), rot_x(3.2375858)), base)
    };
    [apply(matrix, unit([0.2, 1.0, -0.7])), apply(matrix, unit([-0.2, 1.0, 0.7]))]
}

/// `minecraft_mix_light` from the game's shaders.
fn brightness(lights: &[[f64; 3]; 2], normal: [f64; 3]) -> f64 {
    let sum = dot(lights[0], normal).max(0.0) + dot(lights[1], normal).max(0.0);
    (sum * 0.6 + 0.4).min(1.0)
}

/// A quad with its texture loaded and its colour tint decided.
pub struct Prepared<'a> {
    pub quad: &'a Quad,
    pub image: &'a Image,
    pub tint: [f64; 3],
}

struct Sampler<'a> {
    image: &'a Image,
    u: (i64, i64),
    v: (i64, i64),
}

impl<'a> Sampler<'a> {
    fn new(p: &Prepared<'a>) -> Self {
        let range = |values: [f64; 4], size: u32| {
            let size = size as f64;
            let low = values.iter().cloned().fold(f64::MAX, f64::min) * size;
            let high = values.iter().cloned().fold(f64::MIN, f64::max) * size;
            let first = (low + 1e-6).floor() as i64;
            (first, ((high - 1e-6).ceil() as i64 - 1).max(first))
        };
        Self {
            image: p.image,
            u: range(p.quad.uv.map(|t| t[0]), p.image.width),
            v: range(p.quad.uv.map(|t| t[1]), p.image.height),
        }
    }

    /// Nearest texel, kept inside the face's own UV rectangle so that a sample on the
    /// edge does not pick up the neighbouring part of the texture.
    fn get(&self, u: f64, v: f64) -> [u8; 4] {
        let (w, h) = (self.image.width as i64, self.image.height as i64);
        let x = ((u * w as f64).floor() as i64).clamp(self.u.0, self.u.1).rem_euclid(w);
        let y = ((v * h as f64).floor() as i64).clamp(self.v.0, self.v.1).rem_euclid(h);
        self.image.get(x as u32, y as u32)
    }
}

fn blend_over(under: [u8; 4], over: [u8; 4]) -> [u8; 4] {
    let (ua, oa) = (under[3] as f64 / 255.0, over[3] as f64 / 255.0);
    let a = oa + ua * (1.0 - oa);
    if a <= 0.0 {
        return [0; 4];
    }
    let mix = |i: usize| ((over[i] as f64 * oa + under[i] as f64 * ua * (1.0 - oa)) / a).round() as u8;
    [mix(0), mix(1), mix(2), (a * 255.0).round() as u8]
}

/// Draws quads into a square image of the given size.
pub fn draw(quads: &[Prepared], gui: &Transform, front_light: bool, size: u32) -> Image {
    let rotation = {
        let [x, y, z] = gui.rotation.map(f64::to_radians);
        mul(mul(rot_x(x), rot_y(y)), rot_z(z))
    };
    let project = |p: [f64; 3]| {
        let scaled = [(p[0] - 0.5) * gui.scale[0], (p[1] - 0.5) * gui.scale[1], (p[2] - 0.5) * gui.scale[2]];
        let q = apply(rotation, scaled);
        [q[0] + gui.translation[0], q[1] + gui.translation[1], q[2] + gui.translation[2]]
    };

    // Only faces towards the viewer count, counter-clockwise as seen from +z.
    let visible: Vec<(&Prepared, [[f64; 3]; 4])> = quads
        .iter()
        .filter_map(|p| {
            let s = p.quad.pos.map(project);
            let area: f64 = (0..4).map(|i| s[i][0] * s[(i + 1) % 4][1] - s[(i + 1) % 4][0] * s[i][1]).sum();
            (area > 1e-9).then_some((p, s))
        })
        .collect();

    // The game lets a model spill over its slot. A picture has no slot around it, so
    // what would be cut off is scaled to fit instead.
    let extent = visible.iter().flat_map(|(_, s)| s.iter()).fold(0.0_f64, |m, v| m.max(v[0].abs()).max(v[1].abs()));
    let fit = if extent > 0.5 + 1e-6 { 0.5 / extent } else { 1.0 };

    let lights = lights(front_light);
    let side = size as usize;
    let mut out = Image::new(size, size);
    let mut depth = vec![f64::NEG_INFINITY; side * side];
    let mut stamp = vec![usize::MAX; side * side];
    let mut translucent: Vec<(usize, f64, usize, [u8; 4])> = Vec::new();
    const TIE: f64 = 1e-4;

    for (order, (prepared, screen)) in visible.iter().enumerate() {
        let quad = prepared.quad;
        let normal = {
            let n = [quad.normal[0] / gui.scale[0], quad.normal[1] / gui.scale[1], quad.normal[2] / gui.scale[2]];
            let n = unit(apply(rotation, n));
            [n[0], -n[1], n[2]]
        };
        let shade = brightness(&lights, normal);
        let tone = [0, 1, 2].map(|i| prepared.tint[i] * shade);
        let sampler = Sampler::new(prepared);
        let pixel = |v: [f64; 3]| [(v[0] * fit + 0.5) * size as f64, (0.5 - v[1] * fit) * size as f64, v[2]];
        let s = screen.map(pixel);

        for [a, b, c] in [[0, 1, 2], [0, 2, 3]] {
            let (pa, pb, pc) = (s[a], s[b], s[c]);
            let det = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pc[0] - pa[0]) * (pb[1] - pa[1]);
            if det.abs() < 1e-9 {
                continue;
            }
            let bound = |values: [f64; 3], limit: usize| {
                let low = values.iter().cloned().fold(f64::MAX, f64::min).floor().max(0.0) as usize;
                let high = (values.iter().cloned().fold(f64::MIN, f64::max).ceil() as i64).clamp(0, limit as i64) as usize;
                (low, high)
            };
            let (x0, x1) = bound([pa[0], pb[0], pc[0]], side);
            let (y0, y1) = bound([pa[1], pb[1], pc[1]], side);
            for y in y0..y1 {
                for x in x0..x1 {
                    let index = y * side + x;
                    if stamp[index] == order {
                        continue;
                    }
                    let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                    let wb = ((fx - pa[0]) * (pc[1] - pa[1]) - (pc[0] - pa[0]) * (fy - pa[1])) / det;
                    let wc = ((pb[0] - pa[0]) * (fy - pa[1]) - (fx - pa[0]) * (pb[1] - pa[1])) / det;
                    let wa = 1.0 - wb - wc;
                    if wa < -1e-6 || wb < -1e-6 || wc < -1e-6 {
                        continue;
                    }
                    let (ta, tb, tc) = (quad.uv[a], quad.uv[b], quad.uv[c]);
                    let texel = sampler.get(wa * ta[0] + wb * tb[0] + wc * tc[0], wa * ta[1] + wb * tb[1] + wc * tc[1]);
                    stamp[index] = order;
                    // The game's cutout shaders discard below 0.1.
                    if texel[3] < 26 {
                        continue;
                    }
                    let z = wa * pa[2] + wb * pb[2] + wc * pc[2];
                    let lit = |i: usize| (texel[i] as f64 * tone[i]).round().clamp(0.0, 255.0) as u8;
                    let color = [lit(0), lit(1), lit(2), texel[3]];
                    if texel[3] == 255 {
                        if z >= depth[index] - TIE {
                            depth[index] = z;
                            out.pixels[index] = color;
                        }
                    } else {
                        translucent.push((index, z, order, color));
                    }
                }
            }
        }
    }

    // Translucent pieces go over the solid picture from far to near, ties in drawing order.
    translucent.sort_by(|a, b| (a.0, a.1, a.2).partial_cmp(&(b.0, b.1, b.2)).unwrap_or(std::cmp::Ordering::Equal));
    for (index, z, _, color) in translucent {
        if z >= depth[index] - TIE {
            out.pixels[index] = blend_over(out.pixels[index], color);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_faces_get_the_games_shading() {
        // Normals as they are after [30, 225, 0] and the GUI's Y flip.
        let l = lights(false);
        let view = |n: [f64; 3]| {
            let r = mul(rot_x(30f64.to_radians()), rot_y(225f64.to_radians()));
            let v = apply(r, n);
            [v[0], -v[1], v[2]]
        };
        let top = brightness(&l, view([0.0, 1.0, 0.0]));
        let east = brightness(&l, view([1.0, 0.0, 0.0]));
        let north = brightness(&l, view([0.0, 0.0, -1.0]));
        assert_eq!(top, 1.0);
        assert!((east - 0.65).abs() < 0.01, "{east}");
        assert!((north - 0.40).abs() < 0.01, "{north}");
    }

    #[test]
    fn flat_items_are_not_shaded() {
        assert_eq!(brightness(&lights(true), [0.0, 0.0, 1.0]), 1.0);
    }

    #[test]
    fn translucent_blending_keeps_straight_alpha() {
        assert_eq!(blend_over([0; 4], [10, 20, 30, 128]), [10, 20, 30, 128]);
        assert_eq!(blend_over([200, 200, 200, 255], [0, 0, 0, 255]), [0, 0, 0, 255]);
        let mid = blend_over([200, 0, 0, 255], [0, 0, 200, 128]);
        assert_eq!(mid[3], 255);
        assert!(mid[0] > 90 && mid[0] < 110 && mid[2] > 90 && mid[2] < 110, "{mid:?}");
    }
}
