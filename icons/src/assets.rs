//! The resources of one or more JARs, merged in memory. JARs are only ever read as data.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Larger entries are not textures or models of any mod we expect.
const MAX_ENTRY: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub struct Assets {
    /// Bytes by path in the JAR, later JARs replace earlier ones.
    files: BTreeMap<String, Vec<u8>>,
}

/// What a JAR offers, known before deciding what else has to be downloaded.
#[derive(Debug, Default, Clone)]
pub struct JarInfo {
    pub has_minecraft: bool,
    pub mods: Vec<String>,
}

fn wanted(name: &str) -> bool {
    let mut parts = name.split('/');
    if parts.next() != Some("assets") || parts.next().is_none() {
        return false;
    }
    let kind = parts.next();
    let by_kind = matches!(kind, Some("models") | Some("textures"));
    by_kind && [".json", ".png", ".mcmeta", ".obj", ".mtl"].iter().any(|s| name.ends_with(s))
}

impl Assets {
    pub fn insert(&mut self, path: impl Into<String>, data: impl Into<Vec<u8>>) {
        self.files.insert(path.into(), data.into());
    }

    pub fn add_jar(&mut self, path: &Path) -> Result<(), String> {
        let name = path.display();
        let file = File::open(path).map_err(|e| format!("{name}: {e}"))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{name}: {e}"))?;
        for i in 0..zip.len() {
            let entry = zip.by_index(i).map_err(|e| format!("{name}: {e}"))?;
            if entry.is_dir() || !wanted(entry.name()) {
                continue;
            }
            let entry_name = entry.name().to_string();
            let mut data = Vec::new();
            if entry.take(MAX_ENTRY).read_to_end(&mut data).is_ok() {
                self.files.insert(entry_name, data);
            }
        }
        Ok(())
    }

    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub fn text(&self, path: &str) -> Option<&str> {
        self.get(path).and_then(|b| std::str::from_utf8(b).ok())
    }

    pub fn json(&self, path: &str) -> Option<serde_json::Value> {
        serde_json::from_slice(self.get(path)?).ok()
    }

    /// Every item that has a model, as `namespace:id`. Armour trim variants are left out:
    /// they are overrides of an item, and the game builds their textures at runtime.
    pub fn items(&self) -> Vec<String> {
        self.files.keys().filter_map(|p| item_of(p)).collect()
    }

    /// Every fluid that has a still texture, as `fluid:namespace:id`.
    pub fn fluids(&self) -> Vec<String> {
        let mut list: Vec<String> = self.files.keys().filter_map(|p| fluid_of(p)).collect();
        list.sort();
        list.dedup();
        list
    }
}

fn item_of(path: &str) -> Option<String> {
    let rest = path.strip_prefix("assets/")?.strip_suffix(".json")?;
    let (ns, rest) = rest.split_once("/models/item/")?;
    (is_safe_name(ns) && is_safe_name(rest) && !rest.ends_with("_trim")).then(|| format!("{ns}:{rest}"))
}

fn fluid_of(path: &str) -> Option<String> {
    let rest = path.strip_prefix("assets/")?.strip_suffix("_still.png")?;
    let (ns, rest) = rest.split_once("/textures/")?;
    let name = rest.strip_prefix("fluid/").or_else(|| rest.strip_prefix("block/"))?;
    (is_safe_name(ns) && is_safe_name(name)).then(|| format!("fluid:{ns}:{name}"))
}

/// Names from a JAR become file names, so they must not be able to leave the output folder.
pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && name.bytes().all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'))
}

/// `create:block/x` gives `(create, block/x)`, a bare path belongs to the game.
pub fn split_location(location: &str) -> (&str, &str) {
    location.split_once(':').unwrap_or(("minecraft", location))
}

pub fn normalize(location: &str) -> String {
    let (ns, path) = split_location(location);
    format!("{ns}:{path}")
}

pub fn model_path(location: &str) -> String {
    let (ns, path) = split_location(location);
    format!("assets/{ns}/models/{path}.json")
}

pub fn texture_path(location: &str) -> String {
    let (ns, path) = split_location(location);
    format!("assets/{ns}/textures/{path}.png")
}

/// Reads what a JAR contains without loading it. Slugs on Modrinth usually match mod ids.
pub fn peek_jar(path: &Path) -> Result<JarInfo, String> {
    let name = path.display();
    let file = File::open(path).map_err(|e| format!("{name}: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{name}: {e}"))?;
    let mut info = JarInfo { has_minecraft: zip.by_name("assets/minecraft/textures/block/stone.png").is_ok(), mods: Vec::new() };
    if let Ok(mut entry) = zip.by_name("META-INF/neoforge.mods.toml") {
        let mut text = String::new();
        if entry.read_to_string(&mut text).is_ok() {
            if let Ok(toml::Value::Table(t)) = text.parse::<toml::Value>() {
                for m in t.get("mods").and_then(|m| m.as_array()).into_iter().flatten() {
                    if let Some(id) = m.get("modId").and_then(|v| v.as_str()) {
                        info.mods.push(id.to_string());
                    }
                }
            }
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_default_to_the_game() {
        assert_eq!(model_path("block/stone"), "assets/minecraft/models/block/stone.json");
        assert_eq!(texture_path("create:item/x"), "assets/create/textures/item/x.png");
    }

    #[test]
    fn names_that_escape_the_output_folder_are_refused() {
        let mut a = Assets::default();
        a.insert("assets/create/models/item/cogwheel.json", "{}");
        a.insert("assets/create/models/item/../../../../evil.json", "{}");
        a.insert("assets/../x/models/item/evil.json", "{}");
        a.insert("assets/create/models/item/nested/part.json", "{}");
        a.insert("assets/create/models/item/cardboard_boots_gold_trim.json", "{}");
        assert_eq!(a.items(), vec!["create:cogwheel"]);
    }

    #[test]
    fn fluids_come_from_still_textures() {
        let mut a = Assets::default();
        a.insert("assets/minecraft/textures/block/water_still.png", "");
        a.insert("assets/minecraft/textures/block/water_flow.png", "");
        a.insert("assets/create/textures/fluid/honey_still.png", "");
        assert_eq!(a.fluids(), vec!["fluid:create:honey", "fluid:minecraft:water"]);
    }
}
