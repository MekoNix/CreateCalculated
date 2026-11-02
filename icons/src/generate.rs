//! Renders every item and fluid, writes the pictures and the index the page reads.

use crate::assets::{is_safe_name, Assets};
use crate::mesh::{self, flat_quad, Mesh};
use crate::model::{self, Geometry, Transform};
use crate::render::{draw, Prepared};
use crate::texture::{self, Image};
use crate::{entity, tint};
use createcalc_core::model::Bundle;
use serde_json::json;
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// A regular model from the files.
    Model,
    /// A box model copied from game code, see [`entity`].
    GameCode,
    /// No geometry available, a texture stands in.
    Texture,
}

/// Create's custom item renderers add these parts to the base model. At rest they sit
/// where the files put them, so they can simply be drawn along.
const EXTRA_PARTS: &[(&str, &[&str])] = &[
    ("create:potato_cannon", &["create:item/potato_cannon/cog"]),
    ("create:wand_of_symmetry", &["create:item/wand_of_symmetry/core", "create:item/wand_of_symmetry/core_glow", "create:item/wand_of_symmetry/bits"]),
    ("create:handheld_worldshaper", &["create:item/handheld_worldshaper/core", "create:item/handheld_worldshaper/core_glow", "create:item/handheld_worldshaper/accelerator"]),
    ("create:extendo_grip", &["create:item/extendo_grip/cog"]),
];

pub struct Rendered {
    pub image: Image,
    pub source: Source,
    /// Texture references or files that were not found, those faces are left out.
    pub missing: Vec<String>,
}

pub struct Renderer<'a> {
    assets: &'a Assets,
    size: u32,
    textures: HashMap<String, Option<Rc<Image>>>,
}

impl<'a> Renderer<'a> {
    pub fn new(assets: &'a Assets, size: u32) -> Self {
        Self { assets, size, textures: HashMap::new() }
    }

    fn texture(&mut self, location: &str) -> Option<Rc<Image>> {
        if let Some(found) = self.textures.get(location) {
            return found.clone();
        }
        let image = texture::load(self.assets, location).map(Rc::new);
        self.textures.insert(location.to_string(), image.clone());
        image
    }

    /// Returns None when nothing visible came out.
    fn paint(&mut self, id: &str, quads: &Mesh, gui: &Transform, front_light: bool, missing: &mut Vec<String>) -> Option<Image> {
        let images: Vec<Option<Rc<Image>>> = quads.quads.iter().map(|q| self.texture(&q.texture)).collect();
        let mut prepared = Vec::with_capacity(quads.quads.len());
        for (quad, image) in quads.quads.iter().zip(&images) {
            match image {
                Some(image) => prepared.push(Prepared { quad, image, tint: tint::for_item(id, quad.tint) }),
                None => missing.push(quad.texture.clone()),
            }
        }
        let image = draw(&prepared, gui, front_light, self.size);
        (!image.is_blank()).then_some(image)
    }

    /// A single texture filling the picture, used when an item has nothing better.
    fn flat(&mut self, id: &str, location: &str, missing: &mut Vec<String>) -> Option<Image> {
        let quads = Mesh { quads: vec![flat_quad(location, -1)], unresolved: Vec::new() };
        self.paint(id, &quads, &Transform::default(), true, missing)
    }

    pub fn item(&mut self, id: &str) -> Result<Rendered, String> {
        let (namespace, name) = id.split_once(':').ok_or_else(|| format!("bad id {id}"))?;
        let mut missing = Vec::new();
        let mut reason = String::new();
        let mut stand_in = None;
        match model::load(self.assets, &format!("{namespace}:item/{name}")) {
            Ok(model) => {
                let is_entity = matches!(model.geometry, Some(Geometry::Entity));
                let mut quads = if is_entity { entity::mesh(id).unwrap_or_default() } else { mesh::build(self.assets, &model)? };
                for part in EXTRA_PARTS.iter().filter(|(item, _)| *item == id).flat_map(|(_, parts)| parts.iter()) {
                    if let Ok(extra) = model::load(self.assets, part).and_then(|m| mesh::build(self.assets, &m)) {
                        quads.quads.extend(extra.quads);
                    }
                }
                missing.extend(quads.unresolved.iter().cloned());
                let gui = model.gui.unwrap_or_default();
                if let Some(image) = self.paint(id, &quads, &gui, model.front_light.unwrap_or(false), &mut missing) {
                    let source = if is_entity { Source::GameCode } else { Source::Model };
                    return Ok(Rendered { image, source, missing });
                }
                // A particle texture says little about an item drawn by game code, a dragon
                // head would come out as a square of soul sand. Letters are more honest.
                if is_entity {
                    return Err("drawn by game code".to_string());
                }
                reason = "model draws nothing".to_string();
                stand_in = model.texture("#particle");
            }
            Err(e) => reason.push_str(&e),
        }
        let candidates = [stand_in, Some(format!("{namespace}:item/{name}")), Some(format!("{namespace}:block/{name}"))];
        for location in candidates.into_iter().flatten() {
            let mut ignored = Vec::new();
            if let Some(image) = self.flat(id, &location, &mut ignored) {
                return Ok(Rendered { image, source: Source::Texture, missing });
            }
        }
        Err(reason)
    }

    fn still_texture(&mut self, namespace: &str, name: &str) -> Option<(String, Rc<Image>)> {
        ["fluid", "block"].iter().find_map(|folder| {
            let location = format!("{namespace}:{folder}/{name}_still");
            self.texture(&location).map(|image| (location, image))
        })
    }

    /// A fluid is the first frame of its still texture, flat, as recipe viewers show it.
    pub fn fluid(&mut self, id: &str) -> Result<Image, String> {
        let (namespace, name, variant) = split_fluid(id).ok_or_else(|| format!("bad id {id}"))?;
        let found = self.still_texture(namespace, name).or_else(|| {
            // Tags can name a fluid under a namespace that has no texture for it, such as
            // the game's milk, which only mods give a picture. Any fluid of that name will do.
            let others: Vec<String> = self.assets.fluids().into_iter().filter(|f| f.rsplit(':').next() == Some(name)).collect();
            others.iter().find_map(|f| split_fluid(f).and_then(|(ns, _, _)| self.still_texture(ns, name)))
        });
        let (location, image) = found.ok_or("no still texture")?;
        let quad = flat_quad(&location, -1);
        let prepared = [Prepared { quad: &quad, image: &image, tint: tint::for_fluid(name, variant) }];
        let out = draw(&prepared, &Transform::default(), true, self.size);
        if out.is_blank() {
            return Err("blank still texture".into());
        }
        Ok(out)
    }
}

/// `fluid:create:potion[minecraft:strength]` into `(create, potion, minecraft:strength)`.
fn split_fluid(id: &str) -> Option<(&str, &str, Option<&str>)> {
    let rest = id.strip_prefix("fluid:")?;
    let (base, variant) = match rest.split_once('[') {
        Some((base, variant)) => (base, Some(variant.strip_suffix(']')?)),
        None => (rest, None),
    };
    let (namespace, name) = base.split_once(':')?;
    Some((namespace, name, variant))
}

/// Where an id's picture goes under the output folder. Ids can come from a JAR or a
/// bundle, so anything that could point outside the folder gets no file at all.
pub fn relative_path(id: &str) -> Option<String> {
    if id.starts_with("fluid:") {
        let (namespace, name, variant) = split_fluid(id)?;
        if !is_safe_name(namespace) || !is_safe_name(name) {
            return None;
        }
        return Some(match variant {
            Some(variant) => {
                let variant = variant.replace(':', "_");
                if !is_safe_name(&variant) {
                    return None;
                }
                format!("fluids/{namespace}/{name}/{variant}.png")
            }
            None => format!("fluids/{namespace}/{name}.png"),
        });
    }
    let (namespace, name) = id.split_once(':')?;
    (is_safe_name(namespace) && is_safe_name(name)).then(|| format!("items/{namespace}/{name}.png"))
}

#[derive(Debug, Default)]
pub struct Coverage {
    pub items: (usize, usize),
    pub fluids: (usize, usize),
    pub tags: (usize, usize),
    pub missing: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Report {
    pub written: usize,
    pub by_source: BTreeMap<String, usize>,
    /// Items that only got a texture as a stand-in.
    pub stand_ins: Vec<String>,
    pub failed: Vec<(String, String)>,
    /// Items with faces left out because a texture was not found.
    pub incomplete: Vec<(String, Vec<String>)>,
    pub coverage: Option<Coverage>,
    pub bytes: u64,
}

fn coverage(bundle: &Bundle, icons: &BTreeMap<String, String>) -> Coverage {
    let used: BTreeSet<&str> = bundle
        .recipes
        .iter()
        .flat_map(|r| r.inputs.iter().map(|s| s.res.as_str()).chain(r.outputs.iter().map(|o| o.res.as_str())))
        .collect();
    let mut report = Coverage::default();
    for id in used {
        let slot = if id.starts_with('#') {
            &mut report.tags
        } else if id.starts_with("fluid:") {
            &mut report.fluids
        } else {
            &mut report.items
        };
        slot.1 += 1;
        if icons.contains_key(id) {
            slot.0 += 1;
        } else {
            report.missing.push(id.to_string());
        }
    }
    report
}

/// Everything that has to exist as a file: items and fluids from the JARs, plus what
/// the bundle names, such as potion fluids.
fn targets(assets: &Assets, bundle: Option<&Bundle>) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut items: BTreeSet<String> = assets.items().into_iter().collect();
    let mut fluids: BTreeSet<String> = assets.fluids().into_iter().collect();
    let Some(bundle) = bundle else { return (items, fluids) };
    let used = bundle.recipes.iter().flat_map(|r| r.inputs.iter().map(|s| &s.res).chain(r.outputs.iter().map(|o| &o.res)));
    for res in used.chain(bundle.tags.values().flatten()) {
        if res.starts_with('#') {
            continue;
        }
        if res.starts_with("fluid:") {
            fluids.insert(res.clone());
        } else {
            items.insert(res.clone());
        }
    }
    (items, fluids)
}

/// Renders everything into `out` and writes `index.json` there last.
pub fn generate(assets: &Assets, bundle: Option<&Bundle>, size: u32, out: &Path) -> Result<Report, String> {
    let (items, fluids) = targets(assets, bundle);
    let mut renderer = Renderer::new(assets, size);
    let mut report = Report::default();
    let mut icons: BTreeMap<String, String> = BTreeMap::new();
    let mut version = Sha1::new();

    let mut save = |id: &String, image: &Image, report: &mut Report| -> Result<(), String> {
        let Some(relative) = relative_path(id) else {
            report.failed.push((id.clone(), "unsafe id".into()));
            return Ok(());
        };
        let path = out.join(&relative);
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        let png = image.encode_png()?;
        std::fs::write(&path, &png).map_err(|e| format!("{}: {e}", path.display()))?;
        version.update(relative.as_bytes());
        version.update(&png);
        report.written += 1;
        report.bytes += png.len() as u64;
        icons.insert(id.clone(), relative);
        Ok(())
    };

    for id in &items {
        match renderer.item(id) {
            Ok(rendered) => {
                *report.by_source.entry(format!("{:?}", rendered.source)).or_default() += 1;
                if rendered.source == Source::Texture {
                    report.stand_ins.push(id.clone());
                }
                if !rendered.missing.is_empty() {
                    let mut list = rendered.missing.clone();
                    list.sort();
                    list.dedup();
                    report.incomplete.push((id.clone(), list));
                }
                save(id, &rendered.image, &mut report)?;
            }
            Err(reason) => report.failed.push((id.clone(), reason)),
        }
    }
    for id in &fluids {
        match renderer.fluid(id) {
            Ok(image) => {
                *report.by_source.entry("Fluid".to_string()).or_default() += 1;
                save(id, &image, &mut report)?;
            }
            Err(reason) => report.failed.push((id.clone(), reason)),
        }
    }

    // A tag has no picture of its own, it shows its first member that has one.
    if let Some(bundle) = bundle {
        for (tag, members) in &bundle.tags {
            if let Some(path) = members.iter().find_map(|m| icons.get(m)).cloned() {
                icons.insert(tag.clone(), path);
            }
        }
        report.coverage = Some(coverage(bundle, &icons));
    }

    let version: String = version.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect();
    let index = json!({ "version": version, "size": size, "icons": icons });
    let text = serde_json::to_string(&index).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    std::fs::write(out.join("index.json"), text).map_err(|e| format!("index.json: {e}"))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_the_output_folder() {
        assert_eq!(relative_path("create:cogwheel").as_deref(), Some("items/create/cogwheel.png"));
        assert_eq!(relative_path("fluid:minecraft:water").as_deref(), Some("fluids/minecraft/water.png"));
        assert_eq!(relative_path("fluid:create:potion[minecraft:strength]").as_deref(), Some("fluids/create/potion/minecraft_strength.png"));
        assert_eq!(relative_path("../x:y"), None);
        assert_eq!(relative_path("create:../../evil"), None);
        assert_eq!(relative_path("fluid:create:a/b"), None);
        assert_eq!(relative_path("fluid:create:potion[../x]"), None);
    }
}
