//! Recipe data shared by the importer and the planner.
//!
//! Resource ids are plain strings:
//! `create:brass_ingot` is an item, `fluid:minecraft:water` a fluid,
//! `#c:ingots/iron` an item tag and `#fluid:c:milk` a fluid tag.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

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

/// Items that stand for a tag, their names and what they replace.
const TAG_DEFAULTS: &str = include_str!("../data/tag_defaults.txt");

/// A bundle with lookup indexes.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub bundle: Bundle,
    by_id: HashMap<String, usize>,
    producers: HashMap<ResId, Vec<usize>>,
    tag_defaults: BTreeMap<ResId, ResId>,
    skins: HashSet<ResId>,
}

impl Catalog {
    pub fn new(mut bundle: Bundle) -> Self {
        let (tag_defaults, skins) = apply_tag_defaults(&mut bundle);
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
        Self { bundle, by_id, producers, tag_defaults, skins }
    }

    /// The item that stands for a tag when any member would do.
    pub fn tag_default(&self, tag: &str) -> Option<&ResId> {
        self.tag_defaults.get(tag)
    }

    pub fn tag_defaults(&self) -> &BTreeMap<ResId, ResId> {
        &self.tag_defaults
    }

    /// Whether an item is only another look of an item that stands for it, such as a wood species.
    pub fn is_skin(&self, res: &str) -> bool {
        self.skins.contains(res)
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

/// Reads the list of default items: names them in the bundle and works out which item stands for
/// each tag and which items are only another look of one.
fn apply_tag_defaults(bundle: &mut Bundle) -> (BTreeMap<ResId, ResId>, HashSet<ResId>) {
    let mut rank: HashMap<&str, usize> = HashMap::new();
    let mut renamed = HashSet::new();
    let mut same_thing = Vec::new();
    let lines = TAG_DEFAULTS.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("# "));
    for (i, line) in lines.enumerate() {
        let mut fields = line.split('|').map(str::trim);
        let item = fields.next().unwrap_or_default();
        rank.insert(item, i);
        if let (Some(en), Some(ru)) = (fields.next(), fields.next()) {
            bundle.names.insert(item.to_string(), Names { en: Some(en.to_string()), ru: Some(ru.to_string()) });
            renamed.insert(item);
        }
        if let Some(tag) = fields.next() {
            same_thing.push((item, tag));
        }
    }
    let defaults = bundle
        .tags
        .iter()
        .filter_map(|(tag, members)| {
            let best = members.iter().filter(|m| rank.contains_key(m.as_str())).min_by_key(|m| rank[m.as_str()])?;
            Some((tag.clone(), best.clone()))
        })
        .collect();
    let mut skins = HashSet::new();
    for (item, tag) in same_thing {
        let members = bundle.tags.get(tag).map(Vec::as_slice).unwrap_or(&[]);
        skins.extend(members.iter().filter(|m| m.as_str() != item && !renamed.contains(m.as_str())).cloned());
    }
    (defaults, skins)
}
