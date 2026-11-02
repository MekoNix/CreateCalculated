//! Turns a model into quads in block units, the way the game's FaceBakery and
//! NeoForge's OBJ loader do. Vertices stay in the game's order, so winding decides
//! which side of a quad is visible.

use crate::assets::{split_location, Assets};
use crate::model::{Dir, Element, ElementRotation, Face, Geometry, Model};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    pub pos: [[f64; 3]; 4],
    /// From 0 to 1 across the texture.
    pub uv: [[f64; 2]; 4],
    /// Texture location, already resolved.
    pub texture: String,
    pub tint: i32,
    pub normal: [f64; 3],
}

#[derive(Debug, Default)]
pub struct Mesh {
    pub quads: Vec<Quad>,
    /// Texture references that lead nowhere. The game would draw them magenta.
    pub unresolved: Vec<String>,
}

pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn normalized(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (length > 1e-9).then(|| [v[0] / length, v[1] / length, v[2] / length])
}

/// Normal of a quad from its winding. A degenerate first corner falls back to the next.
pub fn quad_normal(pos: &[[f64; 3]; 4]) -> Option<[f64; 3]> {
    let (a, b, c, d) = (pos[0], pos[1], pos[2], pos[3]);
    normalized(cross(sub(b, a), sub(c, a))).or_else(|| normalized(cross(sub(c, a), sub(d, a))))
}

/// Which corner of the element each vertex takes, 0 for `from` and 1 for `to`.
/// Same order as the game's FaceInfo.
const fn corners(dir: Dir) -> [[usize; 3]; 4] {
    match dir {
        Dir::Down => [[0, 0, 1], [0, 0, 0], [1, 0, 0], [1, 0, 1]],
        Dir::Up => [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]],
        Dir::North => [[1, 1, 0], [1, 0, 0], [0, 0, 0], [0, 1, 0]],
        Dir::South => [[0, 1, 1], [0, 0, 1], [1, 0, 1], [1, 1, 1]],
        Dir::West => [[0, 1, 0], [0, 0, 0], [0, 0, 1], [0, 1, 1]],
        Dir::East => [[1, 1, 1], [1, 0, 1], [1, 0, 0], [1, 1, 0]],
    }
}

fn default_uv(dir: Dir, from: [f64; 3], to: [f64; 3]) -> [f64; 4] {
    match dir {
        Dir::Down => [from[0], 16.0 - to[2], to[0], 16.0 - from[2]],
        Dir::Up => [from[0], from[2], to[0], to[2]],
        Dir::North => [16.0 - to[0], 16.0 - to[1], 16.0 - from[0], 16.0 - from[1]],
        Dir::South => [from[0], 16.0 - to[1], to[0], 16.0 - from[1]],
        Dir::West => [from[2], 16.0 - to[1], to[2], 16.0 - from[1]],
        Dir::East => [16.0 - to[2], 16.0 - to[1], 16.0 - from[2], 16.0 - from[1]],
    }
}

/// `p` and the origin in the same units. Rescaling stretches the two other axes so a
/// rotated element keeps its size on the grid, as the game does for 22.5 and 45 degrees.
fn rotate_about(p: [f64; 3], r: &ElementRotation) -> [f64; 3] {
    let d = sub(p, r.origin);
    let (sin, cos) = r.angle.to_radians().sin_cos();
    let rotated = match r.axis {
        0 => [d[0], d[1] * cos - d[2] * sin, d[1] * sin + d[2] * cos],
        1 => [d[0] * cos + d[2] * sin, d[1], -d[0] * sin + d[2] * cos],
        _ => [d[0] * cos - d[1] * sin, d[0] * sin + d[1] * cos, d[2]],
    };
    let mut scale = [1.0; 3];
    if r.rescale {
        let base = if r.angle.abs() == 22.5 { 22.5_f64 } else { 45.0_f64 };
        let stretch = 1.0 / base.to_radians().cos();
        for (axis, slot) in scale.iter_mut().enumerate() {
            if axis != r.axis {
                *slot = stretch;
            }
        }
    }
    [rotated[0] * scale[0] + r.origin[0], rotated[1] * scale[1] + r.origin[1], rotated[2] * scale[2] + r.origin[2]]
}

fn element_face(element: &Element, face: &Face, texture: String) -> Option<Quad> {
    let uv = face.uv.unwrap_or_else(|| default_uv(face.dir, element.from, element.to));
    let mut pos = [[0.0; 3]; 4];
    let mut tex = [[0.0; 2]; 4];
    for (i, corner) in corners(face.dir).iter().enumerate() {
        let mut p = [0.0; 3];
        for axis in 0..3 {
            p[axis] = if corner[axis] == 0 { element.from[axis] } else { element.to[axis] };
        }
        if let Some(rotation) = &element.rotation {
            p = rotate_about(p, rotation);
        }
        pos[i] = p.map(|v| v / 16.0);
        let shifted = (i + (face.rotation / 90) as usize) % 4;
        let u = if shifted < 2 { uv[0] } else { uv[2] };
        let v = if shifted == 0 || shifted == 3 { uv[1] } else { uv[3] };
        tex[i] = [u / 16.0, v / 16.0];
    }
    let normal = quad_normal(&pos)?;
    Some(Quad { pos, uv: tex, texture, tint: face.tint, normal })
}

/// One flat layer of a generated item: the front side of the game's thin slab.
pub fn flat_quad(texture: &str, tint: i32) -> Quad {
    let z = 8.5 / 16.0;
    Quad {
        pos: [[0.0, 1.0, z], [0.0, 0.0, z], [1.0, 0.0, z], [1.0, 1.0, z]],
        uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
        texture: texture.to_string(),
        tint,
        normal: [0.0, 0.0, 1.0],
    }
}

fn generated_layers(model: &Model, mesh: &mut Mesh) {
    let mut layers: Vec<(u32, &String)> =
        model.textures.iter().filter_map(|(key, value)| Some((key.strip_prefix("layer")?.parse().ok()?, value))).collect();
    layers.sort_by_key(|l| l.0);
    for (index, reference) in layers {
        match model.texture(reference) {
            Some(texture) => mesh.quads.push(flat_quad(&texture, index as i32)),
            None => mesh.unresolved.push(reference.clone()),
        }
    }
}

/// Reads an OBJ next to its MTL. Coordinates are in blocks already. Polygons with more
/// than four corners become fans of triangles, a triangle being a quad with a repeated corner.
fn obj_quads(assets: &Assets, model: &Model, location: &str, flip_v: bool, mesh: &mut Mesh) -> Result<(), String> {
    let (ns, path) = split_location(location);
    let file = format!("assets/{ns}/{path}");
    let text = assets.text(&file).ok_or_else(|| format!("missing {file}"))?;
    let folder = file.rsplit_once('/').map_or("", |(dir, _)| dir);

    let mut positions: Vec<[f64; 3]> = Vec::new();
    let mut coords: Vec<[f64; 2]> = Vec::new();
    let mut normals: Vec<[f64; 3]> = Vec::new();
    let mut materials: HashMap<String, String> = HashMap::new();
    let mut current: Option<String> = None;

    fn floats<const N: usize>(rest: &str) -> [f64; N] {
        let mut out = [0.0; N];
        for (slot, word) in out.iter_mut().zip(rest.split_whitespace()) {
            *slot = word.parse().unwrap_or(0.0);
        }
        out
    }
    fn index(word: Option<&str>, len: usize) -> Option<usize> {
        let n: i64 = word?.parse().ok()?;
        let i = if n < 0 { len as i64 + n } else { n - 1 };
        (0..len as i64).contains(&i).then_some(i as usize)
    }

    for line in text.lines() {
        let line = line.trim();
        let (keyword, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match keyword {
            "v" => positions.push(floats::<3>(rest)),
            "vt" => coords.push(floats::<2>(rest)),
            "vn" => normals.push(floats::<3>(rest)),
            "mtllib" => {
                let library = format!("{folder}/{}", rest.trim());
                let mut name = None;
                for material_line in assets.text(&library).unwrap_or("").lines() {
                    let material_line = material_line.trim();
                    let (key, value) = material_line.split_once(char::is_whitespace).unwrap_or((material_line, ""));
                    match key {
                        "newmtl" => name = Some(value.trim().to_string()),
                        "map_Kd" => {
                            if let Some(name) = &name {
                                materials.insert(name.clone(), value.trim().to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            "usemtl" => current = materials.get(rest.trim()).cloned(),
            "f" => {
                let Some(reference) = &current else { continue };
                let Some(texture) = model.texture(reference) else {
                    mesh.unresolved.push(reference.clone());
                    continue;
                };
                struct Corner {
                    position: [f64; 3],
                    uv: [f64; 2],
                    normal: Option<[f64; 3]>,
                }
                let corners: Vec<Corner> = rest
                    .split_whitespace()
                    .filter_map(|word| {
                        let mut parts = word.split('/');
                        let position = positions[index(parts.next(), positions.len())?];
                        let t = index(parts.next(), coords.len()).map_or([0.0; 2], |i| coords[i]);
                        let normal = index(parts.next(), normals.len()).map(|i| normals[i]);
                        Some(Corner { position, uv: [t[0], if flip_v { 1.0 - t[1] } else { t[1] }], normal })
                    })
                    .collect();
                if corners.len() < 3 {
                    continue;
                }
                let mut push = |picks: [usize; 4]| {
                    let pos = picks.map(|i| corners[i].position);
                    let uv = picks.map(|i| corners[i].uv);
                    let mut sum = [0.0; 3];
                    for i in picks {
                        if let Some(n) = corners[i].normal {
                            sum = [sum[0] + n[0], sum[1] + n[1], sum[2] + n[2]];
                        }
                    }
                    if let Some(normal) = normalized(sum).or_else(|| quad_normal(&pos)) {
                        mesh.quads.push(Quad { pos, uv, texture: texture.clone(), tint: -1, normal });
                    }
                };
                match corners.len() {
                    3 => push([0, 1, 2, 2]),
                    4 => push([0, 1, 2, 3]),
                    n => {
                        for i in 1..n - 1 {
                            push([0, i, i + 1, i + 1]);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// All quads of a model, in drawing order.
pub fn build(assets: &Assets, model: &Model) -> Result<Mesh, String> {
    let mut mesh = Mesh::default();
    append(assets, model, &mut mesh)?;
    Ok(mesh)
}

fn append(assets: &Assets, model: &Model, mesh: &mut Mesh) -> Result<(), String> {
    match &model.geometry {
        Some(Geometry::Elements(elements)) => {
            for element in elements {
                for face in &element.faces {
                    match model.texture(&face.texture) {
                        Some(texture) => mesh.quads.extend(element_face(element, face, texture)),
                        None => mesh.unresolved.push(face.texture.clone()),
                    }
                }
            }
        }
        Some(Geometry::Generated) => generated_layers(model, mesh),
        Some(Geometry::Obj { location, flip_v }) => obj_quads(assets, model, location, *flip_v, mesh)?,
        Some(Geometry::Composite(parts)) => {
            for part in parts {
                append(assets, part, mesh)?;
            }
        }
        Some(Geometry::Entity) | None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::model_path;
    use crate::model;

    fn mesh_of(json: &str) -> Mesh {
        let mut a = Assets::default();
        a.insert(model_path("test:m"), json);
        build(&a, &model::load(&a, "test:m").unwrap()).unwrap()
    }

    #[test]
    fn faces_point_outwards() {
        let m = mesh_of(
            r##"{"textures":{"x":"test:x"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{
                "down":{"texture":"#x"},"up":{"texture":"#x"},"north":{"texture":"#x"},
                "south":{"texture":"#x"},"west":{"texture":"#x"},"east":{"texture":"#x"}}}]}"##,
        );
        let normals: Vec<[f64; 3]> = m.quads.iter().map(|q| q.normal).collect();
        assert_eq!(
            normals,
            vec![[0.0, -1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]
        );
    }

    #[test]
    fn default_uv_follows_the_element_position() {
        let m = mesh_of(r##"{"textures":{"x":"test:x"},"elements":[{"from":[2,4,6],"to":[10,12,14],"faces":{"north":{"texture":"#x"}}}]}"##);
        // Looking at the north face from outside, the higher x is on the left.
        let q = &m.quads[0];
        assert_eq!(q.pos[0], [10.0 / 16.0, 12.0 / 16.0, 6.0 / 16.0]);
        assert_eq!(q.uv[0], [6.0 / 16.0, 4.0 / 16.0]);
        assert_eq!(q.uv[2], [14.0 / 16.0, 12.0 / 16.0]);
    }

    #[test]
    fn face_rotation_shifts_uv_between_vertices() {
        let m = mesh_of(r##"{"textures":{"x":"test:x"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#x","uv":[0,0,16,16],"rotation":90}}}]}"##);
        let q = &m.quads[0];
        assert_eq!(q.uv[0], [0.0, 1.0]);
        assert_eq!(q.uv[1], [1.0, 1.0]);
        assert_eq!(q.uv[2], [1.0, 0.0]);
        assert_eq!(q.uv[3], [0.0, 0.0]);
    }

    #[test]
    fn element_rotation_turns_vertices_and_normal() {
        let m = mesh_of(
            r##"{"textures":{"x":"test:x"},"elements":[{"from":[0,0,8],"to":[16,16,8],
                "rotation":{"origin":[8,8,8],"axis":"y","angle":45},"faces":{"south":{"texture":"#x"}}}]}"##,
        );
        let n = m.quads[0].normal;
        let s = 0.5_f64.sqrt();
        assert!((n[0] - s).abs() < 1e-9 && n[1].abs() < 1e-9 && (n[2] - s).abs() < 1e-9, "{n:?}");
    }

    #[test]
    fn rescale_keeps_the_element_on_the_grid() {
        let m = mesh_of(
            r##"{"textures":{"x":"test:x"},"elements":[{"from":[0,0,8],"to":[16,16,8],
                "rotation":{"origin":[8,8,8],"axis":"y","angle":45,"rescale":true},"faces":{"south":{"texture":"#x"}}}]}"##,
        );
        // A 45 degree plane stretched back to the full block width on x.
        let xs: Vec<f64> = m.quads[0].pos.iter().map(|p| p[0]).collect();
        let width = xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
        assert!((width - 1.0).abs() < 1e-9, "{width}");
    }

    #[test]
    fn unresolved_textures_are_reported_not_drawn() {
        let m = mesh_of(r##"{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#nope"}}}]}"##);
        assert!(m.quads.is_empty());
        assert_eq!(m.unresolved, vec!["#nope"]);
    }

    #[test]
    fn obj_reads_materials_flips_v_and_fans_polygons() {
        let mut a = Assets::default();
        a.insert(model_path("test:m"), r##"{"loader":"neoforge:obj","flip_v":true,"model":"test:models/o.obj","textures":{"skin":"test:t"}}"##);
        a.insert("assets/test/models/o.mtl", "newmtl plain\nmap_Kd #skin\n");
        a.insert(
            "assets/test/models/o.obj",
            "mtllib o.mtl\nusemtl plain\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv -1 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\n\
             f 1/1/1 2/2/1 3/3/1 4/4/1\nf 1/1 2/2 3/3\nf 1/1 2/2 3/3 4/4 5/1\n",
        );
        let m = build(&a, &model::load(&a, "test:m").unwrap()).unwrap();
        assert_eq!(m.quads.len(), 1 + 1 + 3);
        assert_eq!(m.quads[0].texture, "test:t");
        assert_eq!(m.quads[0].uv[0], [0.0, 1.0]);
        assert_eq!(m.quads[0].uv[2], [1.0, 0.0]);
        assert_eq!(m.quads[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(m.quads[1].normal, [0.0, 0.0, 1.0]);
    }
}
