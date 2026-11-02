//! Item and block model JSON: the parent chain, texture variables, the GUI transform.

use crate::assets::{model_path, normalize, Assets};
use serde_json::Value;
use std::collections::BTreeMap;

const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Degrees, applied as the game does: X, then local Y, then local Z.
    pub rotation: [f64; 3],
    /// In block units.
    pub translation: [f64; 3],
    pub scale: [f64; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self { rotation: [0.0; 3], translation: [0.0; 3], scale: [1.0; 3] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Dir {
    pub const ALL: [(Dir, &'static str); 6] =
        [(Dir::Down, "down"), (Dir::Up, "up"), (Dir::North, "north"), (Dir::South, "south"), (Dir::West, "west"), (Dir::East, "east")];
}

#[derive(Debug, Clone)]
pub struct Face {
    pub dir: Dir,
    /// In model units 0 to 16. Without it the game derives them from the element's position.
    pub uv: Option<[f64; 4]>,
    pub texture: String,
    pub rotation: u32,
    pub tint: i32,
}

#[derive(Debug, Clone)]
pub struct ElementRotation {
    pub origin: [f64; 3],
    pub axis: usize,
    pub angle: f64,
    pub rescale: bool,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub rotation: Option<ElementRotation>,
    pub faces: Vec<Face>,
}

#[derive(Debug, Clone)]
pub enum Geometry {
    Elements(Vec<Element>),
    /// `builtin/generated`: flat layers from `layer0`, `layer1` and so on.
    Generated,
    /// `neoforge:obj`.
    Obj { location: String, flip_v: bool },
    /// `neoforge:composite`.
    Composite(Vec<Model>),
    /// `builtin/entity`: drawn by game code, there is no geometry in the files.
    Entity,
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    pub textures: BTreeMap<String, String>,
    pub geometry: Option<Geometry>,
    pub gui: Option<Transform>,
    /// `gui_light: front`, the flat lighting of generated items.
    pub front_light: Option<bool>,
}

impl Model {
    /// Follows references until a texture location comes out. The game reads a face's
    /// texture as a variable name with or without the `#`, so a bare word is tried as a
    /// variable first and only then taken as a location.
    pub fn texture(&self, reference: &str) -> Option<String> {
        let mut current = reference.to_string();
        for _ in 0..MAX_DEPTH {
            match current.strip_prefix('#') {
                Some(variable) => current = self.textures.get(variable)?.clone(),
                None => match self.textures.get(&current) {
                    Some(value) if *value != current => current = value.clone(),
                    _ => return Some(current),
                },
            }
        }
        None
    }
}

fn numbers<const N: usize>(v: Option<&Value>, default: [f64; N]) -> [f64; N] {
    let Some(list) = v.and_then(Value::as_array) else { return default };
    let mut out = default;
    for (slot, value) in out.iter_mut().zip(list) {
        *slot = value.as_f64().unwrap_or(*slot);
    }
    out
}

fn parse_transform(v: &Value) -> Transform {
    let translation = numbers(v.get("translation"), [0.0; 3]).map(|t| t.clamp(-80.0, 80.0) / 16.0);
    let scale = numbers(v.get("scale"), [1.0; 3]).map(|s| s.clamp(-4.0, 4.0));
    Transform { rotation: numbers(v.get("rotation"), [0.0; 3]), translation, scale }
}

fn parse_element(v: &Value) -> Option<Element> {
    let from = numbers(v.get("from"), [0.0; 3]);
    let to = numbers(v.get("to"), [16.0; 3]);
    let rotation = v.get("rotation").and_then(|r| {
        let axis = match r.get("axis")?.as_str()? {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            _ => return None,
        };
        Some(ElementRotation {
            origin: numbers(r.get("origin"), [8.0; 3]),
            axis,
            angle: r.get("angle").and_then(Value::as_f64).unwrap_or(0.0),
            rescale: r.get("rescale").and_then(Value::as_bool).unwrap_or(false),
        })
    });
    let faces = v.get("faces")?;
    let faces = Dir::ALL
        .iter()
        .filter_map(|(dir, name)| {
            let face = faces.get(*name)?;
            Some(Face {
                dir: *dir,
                uv: face.get("uv").map(|_| numbers(face.get("uv"), [0.0; 4])),
                texture: face.get("texture")?.as_str()?.to_string(),
                rotation: face.get("rotation").and_then(Value::as_u64).unwrap_or(0) as u32,
                tint: face.get("tintindex").and_then(Value::as_i64).unwrap_or(-1) as i32,
            })
        })
        .collect();
    Some(Element { from, to, rotation, faces })
}

fn parse_geometry(assets: &Assets, json: &Value, depth: usize) -> Result<Option<Geometry>, String> {
    match json.get("loader").and_then(Value::as_str) {
        Some("neoforge:obj") => {
            let location = json.get("model").and_then(Value::as_str).ok_or("obj model without a file")?;
            let flip_v = json.get("flip_v").and_then(Value::as_bool).unwrap_or(false);
            return Ok(Some(Geometry::Obj { location: location.to_string(), flip_v }));
        }
        Some("neoforge:composite") => {
            let hidden = |name: &str| json.pointer(&format!("/visibility/{name}")).and_then(Value::as_bool) == Some(false);
            let mut parts = Vec::new();
            for (name, child) in json.get("children").and_then(Value::as_object).into_iter().flatten() {
                if !hidden(name) {
                    parts.push(from_json(assets, child, depth + 1)?);
                }
            }
            return Ok(Some(Geometry::Composite(parts)));
        }
        _ => {}
    }
    let elements: Vec<Element> = json.get("elements").and_then(Value::as_array).into_iter().flatten().filter_map(parse_element).collect();
    Ok((!elements.is_empty()).then_some(Geometry::Elements(elements)))
}

fn from_json(assets: &Assets, json: &Value, depth: usize) -> Result<Model, String> {
    let mut model = Model::default();
    let mut current = json.clone();
    let mut seen: Vec<String> = Vec::new();
    loop {
        if seen.len() + depth > MAX_DEPTH {
            return Err("model chain too deep".into());
        }
        for (key, value) in current.get("textures").and_then(Value::as_object).into_iter().flatten() {
            if let Some(texture) = value.as_str() {
                model.textures.entry(key.clone()).or_insert_with(|| texture.to_string());
            }
        }
        if model.gui.is_none() {
            model.gui = current.pointer("/display/gui").map(parse_transform);
        }
        if model.front_light.is_none() {
            model.front_light = current.get("gui_light").and_then(Value::as_str).map(|l| l == "front");
        }
        if model.geometry.is_none() {
            model.geometry = parse_geometry(assets, &current, depth)?;
        }
        let Some(parent) = current.get("parent").and_then(Value::as_str) else { break };
        let parent = normalize(parent);
        match parent.as_str() {
            "minecraft:builtin/generated" => {
                model.geometry.get_or_insert(Geometry::Generated);
                break;
            }
            "minecraft:builtin/entity" => {
                model.geometry.get_or_insert(Geometry::Entity);
                break;
            }
            _ => {}
        }
        if seen.contains(&parent) {
            return Err(format!("parent cycle at {parent}"));
        }
        current = assets.json(&model_path(&parent)).ok_or_else(|| format!("missing model {parent}"))?;
        seen.push(parent);
    }
    // Parts of a composite see the variables of the model around them.
    if let Some(Geometry::Composite(parts)) = &mut model.geometry {
        for part in parts {
            for (key, value) in &model.textures {
                part.textures.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }
    Ok(model)
}

/// Loads a model by location and resolves its whole parent chain.
pub fn load(assets: &Assets, location: &str) -> Result<Model, String> {
    let json = assets.json(&model_path(location)).ok_or_else(|| format!("missing model {location}"))?;
    from_json(assets, &json, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets(models: &[(&str, &str)]) -> Assets {
        let mut a = Assets::default();
        for (location, text) in models {
            a.insert(model_path(location), *text);
        }
        a
    }

    #[test]
    fn parent_chain_merges_child_first() {
        let a = assets(&[
            ("test:item/thing", r##"{"parent":"test:block/mid","textures":{"all":"test:block/child"}}"##),
            ("test:block/mid", r##"{"parent":"test:block/base","textures":{"side":"#all","all":"test:block/mid"}}"##),
            (
                "test:block/base",
                r##"{"gui_light":"side","display":{"gui":{"rotation":[30,225,0],"scale":[0.5,0.5,0.5]}},"textures":{"top":"#side"},
                     "elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#top"}}}]}"##,
            ),
        ]);
        let m = load(&a, "test:item/thing").unwrap();
        assert_eq!(m.texture("#top").as_deref(), Some("test:block/child"));
        assert_eq!(m.gui.unwrap().rotation, [30.0, 225.0, 0.0]);
        assert_eq!(m.gui.unwrap().scale, [0.5; 3]);
        assert_eq!(m.front_light, Some(false));
        assert!(matches!(m.geometry, Some(Geometry::Elements(ref e)) if e.len() == 1));
    }

    #[test]
    fn child_elements_and_display_replace_the_parents() {
        let a = assets(&[
            (
                "test:a",
                r##"{"parent":"test:b","display":{"gui":{"scale":[2,2,2]}},
                     "elements":[{"from":[1,1,1],"to":[2,2,2],"faces":{}}]}"##,
            ),
            (
                "test:b",
                r##"{"display":{"gui":{"scale":[3,3,3]}},
                     "elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{}},{"from":[0,0,0],"to":[1,1,1],"faces":{}}]}"##,
            ),
        ]);
        let m = load(&a, "test:a").unwrap();
        assert_eq!(m.gui.unwrap().scale, [2.0; 3]);
        assert!(matches!(m.geometry, Some(Geometry::Elements(ref e)) if e.len() == 1 && e[0].from == [1.0; 3]));
    }

    #[test]
    fn bare_parents_belong_to_the_game() {
        let a = assets(&[
            ("test:a", r##"{"parent":"item/generated","textures":{"layer0":"test:x"}}"##),
            ("minecraft:item/generated", r##"{"parent":"builtin/generated","gui_light":"front"}"##),
        ]);
        let m = load(&a, "test:a").unwrap();
        assert!(matches!(m.geometry, Some(Geometry::Generated)));
        assert_eq!(m.front_light, Some(true));
    }

    #[test]
    fn cycles_and_gaps_are_errors_not_hangs() {
        let a = assets(&[("test:a", r##"{"parent":"test:b"}"##), ("test:b", r##"{"parent":"test:a"}"##), ("test:c", r##"{"parent":"test:none"}"##)]);
        assert!(load(&a, "test:a").unwrap_err().contains("cycle"));
        assert!(load(&a, "test:c").unwrap_err().contains("missing"));
    }

    #[test]
    fn looping_texture_variables_resolve_to_nothing() {
        let a = assets(&[("test:a", r##"{"textures":{"x":"#y","y":"#x","z":"#missing"}}"##)]);
        let m = load(&a, "test:a").unwrap();
        assert_eq!(m.texture("#x"), None);
        assert_eq!(m.texture("#z"), None);
    }

    #[test]
    fn a_bare_word_is_a_variable_when_one_exists() {
        let a = assets(&[("test:a", r##"{"textures":{"all":"test:block/x"}}"##)]);
        let m = load(&a, "test:a").unwrap();
        assert_eq!(m.texture("all").as_deref(), Some("test:block/x"));
        assert_eq!(m.texture("test:block/y").as_deref(), Some("test:block/y"));
    }

    #[test]
    fn display_translation_is_in_sixteenths() {
        let a = assets(&[("test:a", r##"{"display":{"gui":{"translation":[8,0,-4]}}}"##)]);
        assert_eq!(load(&a, "test:a").unwrap().gui.unwrap().translation, [0.5, 0.0, -0.25]);
    }

    #[test]
    fn composite_parts_inherit_outer_textures_and_honour_visibility() {
        let a = assets(&[(
            "test:a",
            r##"{"loader":"neoforge:composite","textures":{"0":"test:outer"},"visibility":{"off":false},"children":{
                "on":{"elements":[{"from":[0,0,0],"to":[1,1,1],"faces":{"up":{"texture":"#0"}}}]},
                "off":{"elements":[{"from":[0,0,0],"to":[1,1,1],"faces":{"up":{"texture":"#0"}}}]}}}"##,
        )]);
        let m = load(&a, "test:a").unwrap();
        let Some(Geometry::Composite(parts)) = &m.geometry else { panic!("not a composite") };
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].texture("#0").as_deref(), Some("test:outer"));
    }
}
