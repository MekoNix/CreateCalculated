//! Items the game draws from code (`builtin/entity`) have no geometry in the files.
//! Their box models are copied here as numbers from the game's model definitions; the
//! textures still come from the user's own JARs.

use crate::mesh::{quad_normal, Mesh, Quad};
use crate::tint;
use std::f64::consts::{FRAC_PI_2, PI};

type M3 = [[f64; 3]; 3];

/// A rotation or scale followed by a translation, like a level of the game's pose stack.
#[derive(Clone, Copy)]
struct Pose {
    m: M3,
    t: [f64; 3],
}

const IDENTITY: Pose = Pose { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t: [0.0; 3] };

impl Pose {
    fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        [0, 1, 2].map(|i| self.m[i][0] * p[0] + self.m[i][1] * p[1] + self.m[i][2] * p[2] + self.t[i])
    }

    /// `self * other`: `other` acts first, as with successive pose stack calls.
    fn then(&self, other: Pose) -> Pose {
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, slot) in row.iter_mut().enumerate() {
                *slot = (0..3).map(|k| self.m[i][k] * other.m[k][j]).sum();
            }
        }
        Pose { m, t: self.apply(other.t) }
    }

    fn translate(self, x: f64, y: f64, z: f64) -> Pose {
        self.then(Pose { t: [x, y, z], ..IDENTITY })
    }

    fn scale(self, x: f64, y: f64, z: f64) -> Pose {
        self.then(Pose { m: [[x, 0.0, 0.0], [0.0, y, 0.0], [0.0, 0.0, z]], t: [0.0; 3] })
    }

    fn rot_x(self, a: f64) -> Pose {
        let (s, c) = a.sin_cos();
        self.then(Pose { m: [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]], t: [0.0; 3] })
    }

    fn rot_y(self, a: f64) -> Pose {
        let (s, c) = a.sin_cos();
        self.then(Pose { m: [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]], t: [0.0; 3] })
    }

    fn rot_z(self, a: f64) -> Pose {
        let (s, c) = a.sin_cos();
        self.then(Pose { m: [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]], t: [0.0; 3] })
    }
}

#[derive(Clone, Copy)]
struct Cube {
    tex: (f64, f64),
    origin: [f64; 3],
    size: [f64; 3],
    grow: f64,
    north_only: bool,
}

const fn cube(tex: (f64, f64), origin: [f64; 3], size: [f64; 3]) -> Cube {
    Cube { tex, origin, size, grow: 0.0, north_only: false }
}

/// A model part: its cubes under an offset and a rotation, as in `PartPose`.
struct Part<'a> {
    offset: [f64; 3],
    rotation: [f64; 3],
    cubes: &'a [Cube],
}

const fn part(cubes: &[Cube]) -> Part<'_> {
    Part { offset: [0.0; 3], rotation: [0.0; 3], cubes }
}

const fn moved(offset: [f64; 3], cubes: &[Cube]) -> Part<'_> {
    Part { offset, rotation: [0.0; 3], cubes }
}

const fn posed(offset: [f64; 3], rotation: [f64; 3], cubes: &[Cube]) -> Part<'_> {
    Part { offset, rotation, cubes }
}

struct Layer<'a> {
    texture: &'a str,
    /// Texture size the UV numbers are written for.
    tex_size: (f64, f64),
    tint: i32,
    pose: Pose,
    parts: &'a [Part<'a>],
}

/// Box mapping of the game's `ModelPart.Cube`.
fn add_cube(mesh: &mut Mesh, layer: &Layer, to_block: Pose, c: &Cube) {
    let [x0, y0, z0] = c.origin.map(|v| v - c.grow);
    let (x1, y1, z1) = (c.origin[0] + c.size[0] + c.grow, c.origin[1] + c.size[1] + c.grow, c.origin[2] + c.size[2] + c.grow);
    let [dx, dy, dz] = c.size;
    let (u, v) = c.tex;
    let (u1, u2, u3, u4, u5) = (u + dz, u + dz + dx, u + dz + dx + dx, u + dz + dx + dz, u + dz + dx + dz + dx);
    let (v1, v2) = (v + dz, v + dz + dy);
    let a = [x0, y0, z0];
    let b = [x1, y0, z0];
    let e = [x1, y1, z0];
    let f = [x0, y1, z0];
    let g = [x0, y0, z1];
    let h = [x1, y0, z1];
    let i = [x1, y1, z1];
    let j = [x0, y1, z1];
    let all: [([[f64; 3]; 4], [f64; 4]); 6] = [
        ([h, g, a, b], [u1, v, u2, v1]),
        ([e, f, j, i], [u2, v1, u3, v]),
        ([a, g, j, f], [u, v1, u1, v2]),
        ([b, a, f, e], [u1, v1, u2, v2]),
        ([h, b, e, i], [u2, v1, u4, v2]),
        ([g, h, i, j], [u4, v1, u5, v2]),
    ];
    for (index, (corners, [ua, va, ub, vb])) in all.iter().enumerate() {
        if c.north_only && index != 3 {
            continue;
        }
        let pos = corners.map(|p| to_block.apply(p.map(|v| v / 16.0)));
        let (w, h) = layer.tex_size;
        let uv = [[ub / w, va / h], [ua / w, va / h], [ua / w, vb / h], [ub / w, vb / h]];
        if let Some(normal) = quad_normal(&pos) {
            mesh.quads.push(Quad { pos, uv, texture: layer.texture.to_string(), tint: layer.tint, normal });
        }
    }
}

fn mesh_of(layers: &[Layer]) -> Mesh {
    let mut mesh = Mesh::default();
    for layer in layers {
        for part in layer.parts {
            let [ox, oy, oz] = part.offset.map(|v| v / 16.0);
            let [rx, ry, rz] = part.rotation;
            let to_block = layer.pose.translate(ox, oy, oz).rot_z(rz).rot_y(ry).rot_x(rx);
            for c in part.cubes {
                add_cube(&mut mesh, layer, to_block, c);
            }
        }
    }
    mesh
}

fn chest(texture: &str) -> Mesh {
    let body = [cube((0.0, 19.0), [1.0, 0.0, 1.0], [14.0, 10.0, 14.0])];
    let lid = [cube((0.0, 0.0), [1.0, 0.0, 0.0], [14.0, 5.0, 14.0])];
    let lock = [cube((0.0, 0.0), [7.0, -2.0, 14.0], [2.0, 4.0, 1.0])];
    let parts = [part(&body), moved([0.0, 9.0, 1.0], &lid), moved([0.0, 9.0, 1.0], &lock)];
    mesh_of(&[Layer { texture, tex_size: (64.0, 64.0), tint: -1, pose: IDENTITY, parts: &parts }])
}

fn shulker_box(texture: &str) -> Mesh {
    let base = [cube((0.0, 28.0), [-8.0, -8.0, -8.0], [16.0, 8.0, 16.0])];
    let lid = [cube((0.0, 0.0), [-8.0, -16.0, -8.0], [16.0, 12.0, 16.0])];
    let parts = [moved([0.0, 24.0, 0.0], &base), moved([0.0, 24.0, 0.0], &lid)];
    let pose = IDENTITY.translate(0.5, 0.5, 0.5).scale(1.0, -1.0, -1.0).translate(0.0, -1.0, 0.0);
    mesh_of(&[Layer { texture, tex_size: (64.0, 64.0), tint: -1, pose, parts: &parts }])
}

fn bed(texture: &str) -> Mesh {
    let piece = |foot: bool| IDENTITY.translate(0.0, 0.5625, if foot { -1.0 } else { 0.0 }).rot_x(FRAC_PI_2).translate(0.5, 0.5, 0.5).rot_z(PI).translate(-0.5, -0.5, -0.5);
    let head_main = [cube((0.0, 0.0), [0.0, 0.0, 0.0], [16.0, 16.0, 6.0])];
    let head_left = [cube((50.0, 6.0), [0.0, 6.0, 0.0], [3.0, 3.0, 3.0])];
    let head_right = [cube((50.0, 18.0), [-16.0, 6.0, 0.0], [3.0, 3.0, 3.0])];
    let foot_main = [cube((0.0, 22.0), [0.0, 0.0, 0.0], [16.0, 16.0, 6.0])];
    let foot_left = [cube((50.0, 0.0), [0.0, 6.0, -16.0], [3.0, 3.0, 3.0])];
    let foot_right = [cube((50.0, 12.0), [-16.0, 6.0, -16.0], [3.0, 3.0, 3.0])];
    let head = [
        part(&head_main),
        posed([0.0; 3], [FRAC_PI_2, 0.0, FRAC_PI_2], &head_left),
        posed([0.0; 3], [FRAC_PI_2, 0.0, PI], &head_right),
    ];
    let foot = [
        part(&foot_main),
        posed([0.0; 3], [FRAC_PI_2, 0.0, 0.0], &foot_left),
        posed([0.0; 3], [FRAC_PI_2, 0.0, PI * 1.5], &foot_right),
    ];
    mesh_of(&[
        Layer { texture, tex_size: (64.0, 64.0), tint: -1, pose: piece(false), parts: &head },
        Layer { texture, tex_size: (64.0, 64.0), tint: -1, pose: piece(true), parts: &foot },
    ])
}

fn banner() -> Mesh {
    let flag = [cube((0.0, 0.0), [-10.0, 0.0, -2.0], [20.0, 40.0, 1.0])];
    let pole = [cube((44.0, 0.0), [-1.0, -30.0, -1.0], [2.0, 42.0, 2.0])];
    let bar = [cube((0.0, 42.0), [-10.0, -32.0, -1.0], [20.0, 2.0, 2.0])];
    let k = 2.0 / 3.0;
    let pose = IDENTITY.translate(0.5, 0.5, 0.5).scale(k, -k, -k);
    let base = [part(&pole), part(&bar), moved([0.0, -32.0, 0.0], &flag)];
    let cloth = [moved([0.0, -32.0, 0.0], &flag)];
    mesh_of(&[
        Layer { texture: "minecraft:entity/banner_base", tex_size: (64.0, 64.0), tint: -1, pose, parts: &base },
        Layer { texture: "minecraft:entity/banner/base", tex_size: (64.0, 64.0), tint: 0, pose, parts: &cloth },
    ])
}

fn shield() -> Mesh {
    let plate = [cube((0.0, 0.0), [-6.0, -11.0, -2.0], [12.0, 22.0, 1.0])];
    let handle = [cube((26.0, 0.0), [-1.0, -3.0, -1.0], [2.0, 6.0, 6.0])];
    let parts = [part(&plate), part(&handle)];
    let pose = IDENTITY.scale(1.0, -1.0, -1.0);
    mesh_of(&[Layer { texture: "minecraft:entity/shield_base_nopattern", tex_size: (64.0, 64.0), tint: -1, pose, parts: &parts }])
}

fn conduit() -> Mesh {
    let shell = [cube((0.0, 0.0), [-3.0, -3.0, -3.0], [6.0, 6.0, 6.0])];
    let parts = [part(&shell)];
    let pose = IDENTITY.translate(0.5, 0.5, 0.5);
    mesh_of(&[Layer { texture: "minecraft:entity/conduit/base", tex_size: (32.0, 16.0), tint: -1, pose, parts: &parts }])
}

fn skull(texture: &str, tex_height: f64) -> Mesh {
    let head = [cube((0.0, 0.0), [-4.0, -8.0, -4.0], [8.0, 8.0, 8.0])];
    let parts = [posed([0.0; 3], [0.0, PI, 0.0], &head)];
    let pose = IDENTITY.translate(0.5, 0.0, 0.5).scale(-1.0, -1.0, 1.0);
    mesh_of(&[Layer { texture, tex_size: (64.0, tex_height), tint: -1, pose, parts: &parts }])
}

fn decorated_pot() -> Mesh {
    let neck = [
        Cube { grow: -0.1, ..cube((0.0, 0.0), [4.0, 17.0, 4.0], [8.0, 3.0, 8.0]) },
        Cube { grow: 0.2, ..cube((0.0, 5.0), [5.0, 20.0, 5.0], [6.0, 1.0, 6.0]) },
    ];
    let lid = [cube((-14.0, 13.0), [0.0; 3], [14.0, 0.0, 14.0])];
    let side = [Cube { north_only: true, ..cube((1.0, 0.0), [0.0; 3], [14.0, 16.0, 0.0]) }];
    let base = [
        posed([0.0, 37.0, 16.0], [PI, 0.0, 0.0], &neck),
        moved([1.0, 16.0, 1.0], &lid),
        moved([1.0, 0.0, 1.0], &lid),
    ];
    let sides = [
        posed([15.0, 16.0, 1.0], [0.0, 0.0, PI], &side),
        posed([1.0, 16.0, 1.0], [0.0, -FRAC_PI_2, PI], &side),
        posed([15.0, 16.0, 15.0], [0.0, FRAC_PI_2, PI], &side),
        posed([1.0, 16.0, 15.0], [PI, 0.0, 0.0], &side),
    ];
    mesh_of(&[
        Layer { texture: "minecraft:entity/decorated_pot/decorated_pot_base", tex_size: (32.0, 32.0), tint: -1, pose: IDENTITY, parts: &base },
        Layer { texture: "minecraft:entity/decorated_pot/decorated_pot_side", tex_size: (16.0, 16.0), tint: -1, pose: IDENTITY, parts: &sides },
    ])
}

/// The hand-copied model of an item drawn by game code, if there is one.
pub fn mesh(id: &str) -> Option<Mesh> {
    let name = id.strip_prefix("minecraft:")?;
    let dye = |suffix: &str| name.strip_suffix(suffix).filter(|d| tint::dye(d).is_some());
    Some(match name {
        "chest" => chest("minecraft:entity/chest/normal"),
        "trapped_chest" => chest("minecraft:entity/chest/trapped"),
        "ender_chest" => chest("minecraft:entity/chest/ender"),
        "shulker_box" => shulker_box("minecraft:entity/shulker/shulker"),
        "shield" => shield(),
        "conduit" => conduit(),
        "decorated_pot" => decorated_pot(),
        "skeleton_skull" => skull("minecraft:entity/skeleton/skeleton", 32.0),
        "wither_skeleton_skull" => skull("minecraft:entity/skeleton/wither_skeleton", 32.0),
        "creeper_head" => skull("minecraft:entity/creeper/creeper", 32.0),
        "zombie_head" => skull("minecraft:entity/zombie/zombie", 64.0),
        "player_head" => skull("minecraft:entity/player/wide/steve", 64.0),
        _ => {
            if dye("_banner").is_some() {
                banner()
            } else if let Some(color) = dye("_bed") {
                bed(&format!("minecraft:entity/bed/{color}"))
            } else {
                shulker_box(&format!("minecraft:entity/shulker/shulker_{}", dye("_shulker_box")?))
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_items_have_models_and_unknown_do_not() {
        assert!(mesh("minecraft:chest").is_some());
        assert_eq!(mesh("minecraft:white_bed").unwrap().quads[0].texture, "minecraft:entity/bed/white");
        assert!(mesh("minecraft:red_banner").is_some());
        assert!(mesh("minecraft:trident").is_none());
        assert!(mesh("create:chest").is_none());
        assert!(mesh("minecraft:not_a_color_bed").is_none());
    }

    #[test]
    fn chest_box_mapping_follows_the_game() {
        let m = chest("t");
        // The first cube is the body at (1,0,1), 14 by 10 by 14: its south face.
        let south = &m.quads[5];
        assert_eq!(south.normal, [0.0, 0.0, 1.0]);
        assert_eq!(south.uv[0], [56.0 / 64.0, 33.0 / 64.0]);
        assert_eq!(south.uv[2], [42.0 / 64.0, 43.0 / 64.0]);
    }

    #[test]
    fn north_only_cubes_keep_one_face() {
        let m = decorated_pot();
        // Four sides, each a single face, follow the neck and the lids.
        let sides = m.quads.iter().filter(|q| q.texture.ends_with("decorated_pot_side")).count();
        assert_eq!(sides, 4);
    }
}
