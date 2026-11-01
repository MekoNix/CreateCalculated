//! Recipe data shared by the importer and the planner.
//!
//! Resource ids are plain strings:
//! `create:brass_ingot` is an item, `fluid:minecraft:water` a fluid,
//! `#c:ingots/iron` an item tag and `#fluid:c:milk` a fluid tag.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub type ResId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeKind {
    Pressing,
    Compacting,
    Mixing,
    Milling,
    Crushing,
    Cutting,
    Deploying,
    Filling,
    Emptying,
    Splashing,
    Haunting,
    Smelting,
    Blasting,
    Smoking,
    CraftingShaped,
    CraftingShapeless,
    MechanicalCrafting,
    Stonecutting,
    Smithing,
    SequencedAssembly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Heat {
    #[default]
    None,
    Heated,
    Superheated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stack {
    pub res: ResId,
    pub amount: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub res: ResId,
    pub amount: f64,
    /// Probability from 0 to 1. For sequenced assembly this is the normalised weight.
    pub chance: f64,
}

impl Output {
    pub fn expected(&self) -> f64 {
        self.amount * self.chance
    }
}

/// One step of a sequenced assembly loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssemblyStep {
    pub kind: RecipeKind,
    pub duration: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub kind: RecipeKind,
    pub inputs: Vec<Stack>,
    pub outputs: Vec<Output>,
    /// Ticks as written in the recipe. 0 means the machine's own default applies.
    #[serde(default)]
    pub duration: u32,
    #[serde(default)]
    pub heat: Heat,
    /// Occupied grid cells for crafting recipes.
    #[serde(default)]
    pub cells: u32,
    /// True for a 2x2 or 3x3 grid filled with one ingredient, which a press can compact.
    #[serde(default)]
    pub square: bool,
    #[serde(default)]
    pub steps: Vec<AssemblyStep>,
    #[serde(default)]
    pub loops: u32,
}

impl Recipe {
    pub fn main_output(&self) -> Option<&ResId> {
        self.outputs.first().map(|o| &o.res)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Names {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub en: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ru: Option<String>,
}

/// Everything the importer extracts from the mod JARs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bundle {
    pub recipes: Vec<Recipe>,
    /// Flattened tag members, keyed by tag id such as `#c:ingots/iron`.
    pub tags: BTreeMap<ResId, Vec<ResId>>,
    #[serde(default)]
    pub names: BTreeMap<ResId, Names>,
}

/// A bundle with lookup indexes.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub bundle: Bundle,
    by_id: HashMap<String, usize>,
    producers: HashMap<ResId, Vec<usize>>,
}

impl Catalog {
    pub fn new(bundle: Bundle) -> Self {
        let mut by_id = HashMap::new();
        let mut producers: HashMap<ResId, Vec<usize>> = HashMap::new();
        for (i, r) in bundle.recipes.iter().enumerate() {
            by_id.insert(r.id.clone(), i);
            for o in &r.outputs {
                let list = producers.entry(o.res.clone()).or_default();
                if !list.contains(&i) {
                    list.push(i);
                }
            }
        }
        Self { bundle, by_id, producers }
    }

    pub fn recipe(&self, id: &str) -> Option<&Recipe> {
        self.by_id.get(id).map(|&i| &self.bundle.recipes[i])
    }

    pub fn recipes(&self) -> &[Recipe] {
        &self.bundle.recipes
    }

    pub fn producer_indexes(&self, res: &str) -> &[usize] {
        self.producers.get(res).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn tag_members(&self, tag: &str) -> &[ResId] {
        self.bundle.tags.get(tag).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Display name, preferring the requested language. Falls back to the id.
    pub fn name(&self, res: &str, lang: &str) -> String {
        let names = self.bundle.names.get(res);
        let picked = names.and_then(|n| if lang == "ru" { n.ru.clone().or(n.en.clone()) } else { n.en.clone().or(n.ru.clone()) });
        picked.unwrap_or_else(|| res.to_string())
    }

    /// All items and fluids that some recipe produces or consumes, without tags.
    pub fn resources(&self) -> Vec<ResId> {
        let mut seen: Vec<ResId> = self
            .bundle
            .recipes
            .iter()
            .flat_map(|r| r.outputs.iter().map(|o| o.res.clone()).chain(r.inputs.iter().map(|i| i.res.clone())))
            .filter(|r| !r.starts_with('#'))
            .collect();
        seen.sort();
        seen.dedup();
        seen
    }
}
