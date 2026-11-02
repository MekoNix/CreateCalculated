//! Builds a [`Bundle`] from mod JARs. Game data is never stored in this repository:
//! every user generates the bundle locally from their own files.

use crate::model::{AssemblyStep, Bundle, Heat, Names, Output, Recipe, RecipeKind, ResId, STRIPPING_PREFIX, Stack};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Default, Clone)]
pub struct ImportStats {
    pub recipes: usize,
    pub skipped_by_condition: usize,
    /// Recipe types the planner has no machine for, with their counts.
    pub skipped_types: BTreeMap<String, usize>,
    /// Recipes of a known type that could not be parsed, as `id: reason`.
    pub failed: Vec<String>,
    pub unknown_conditions: BTreeSet<String>,
}

/// One input to the importer: JSON documents and language files by their path in the JAR.
#[derive(Default)]
struct Pack {
    /// Path to parsed JSON, later packs override earlier ones with the same path.
    recipes: BTreeMap<String, Value>,
    tags: Vec<(String, String, Value)>,
    lang: Vec<(String, Value)>,
    mods: BTreeSet<String>,
}

fn read_jar(path: &Path, pack: &mut Pack) -> Result<(), String> {
    let name = path.display();
    let file = File::open(path).map_err(|e| format!("{name}: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{name}: {e}"))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("{name}: {e}"))?;
        let entry_name = entry.name().to_string();
        let is_mods_toml = entry_name == "META-INF/neoforge.mods.toml";
        let is_data = entry_name.starts_with("data/") && entry_name.ends_with(".json");
        let is_lang = entry_name.starts_with("assets/")
            && (entry_name.ends_with("/lang/en_us.json") || entry_name.ends_with("/lang/ru_ru.json"));
        if !is_mods_toml && !is_data && !is_lang {
            continue;
        }
        let mut text = String::new();
        if entry.read_to_string(&mut text).is_err() {
            continue;
        }
        if is_mods_toml {
            if let Ok(toml::Value::Table(t)) = text.parse::<toml::Value>() {
                for m in t.get("mods").and_then(|m| m.as_array()).into_iter().flatten() {
                    if let Some(id) = m.get("modId").and_then(|v| v.as_str()) {
                        pack.mods.insert(id.to_string());
                    }
                }
            }
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(&text) else { continue };
        if is_lang {
            pack.lang.push((entry_name, json));
            continue;
        }
        let parts: Vec<&str> = entry_name.split('/').collect();
        if parts.len() < 4 {
            continue;
        }
        let namespace = parts[1];
        let rest = parts[3..].join("/");
        let rest = rest.trim_end_matches(".json");
        match (parts[2], parts.get(3)) {
            ("recipe", _) => {
                pack.recipes.insert(format!("{namespace}:{rest}"), json);
            }
            ("tags", Some(&"item")) => {
                let tag = parts[4..].join("/");
                pack.tags.push((format!("#{namespace}:{}", tag.trim_end_matches(".json")), "item".into(), json));
            }
            ("tags", Some(&"fluid")) => {
                let tag = parts[4..].join("/");
                pack.tags.push((format!("#fluid:{namespace}:{}", tag.trim_end_matches(".json")), "fluid".into(), json));
            }
            _ => {}
        }
    }
    Ok(())
}

fn namespace_of(id: &str) -> &str {
    id.trim_start_matches("fluid:").split(':').next().unwrap_or("")
}

/// Applies tag files in order, honouring `replace`, then flattens nested tags.
fn resolve_tags(pack: &Pack) -> BTreeMap<ResId, Vec<ResId>> {
    let mut raw: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (tag, kind, json) in &pack.tags {
        let list = raw.entry(tag.clone()).or_default();
        if json.get("replace").and_then(Value::as_bool).unwrap_or(false) {
            list.clear();
        }
        for v in json.get("values").and_then(Value::as_array).into_iter().flatten() {
            let id = v.as_str().or_else(|| v.get("id").and_then(Value::as_str));
            let Some(id) = id else { continue };
            let entry = match (id.strip_prefix('#'), kind.as_str()) {
                (Some(t), "fluid") => format!("#fluid:{t}"),
                (Some(t), _) => format!("#{t}"),
                (None, "fluid") => format!("fluid:{id}"),
                (None, _) => id.to_string(),
            };
            list.push(entry);
        }
    }
    fn flatten(tag: &str, raw: &BTreeMap<String, Vec<String>>, seen: &mut BTreeSet<String>, out: &mut BTreeSet<String>) {
        if !seen.insert(tag.to_string()) {
            return;
        }
        for member in raw.get(tag).into_iter().flatten() {
            if member.starts_with('#') {
                flatten(member, raw, seen, out);
            } else {
                out.insert(member.clone());
            }
        }
    }
    raw.keys()
        .map(|tag| {
            let mut out = BTreeSet::new();
            flatten(tag, &raw, &mut BTreeSet::new(), &mut out);
            // Tags list optional entries from mods that are not installed.
            // Flowing variants are world states, recipes never produce them.
            let members = out.into_iter().filter(|m| pack.mods.contains(namespace_of(m)) && !m.contains(":flowing_")).collect();
            (tag.clone(), members)
        })
        .collect()
}

fn condition_holds(c: &Value, pack: &Pack, tags: &BTreeMap<ResId, Vec<ResId>>, stats: &mut ImportStats) -> bool {
    let kind = c.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "neoforge:mod_loaded" => c.get("modid").and_then(Value::as_str).is_some_and(|m| pack.mods.contains(m)),
        "neoforge:not" => c.get("value").is_some_and(|v| !condition_holds(v, pack, tags, stats)),
        "neoforge:and" => c.get("values").and_then(Value::as_array).is_some_and(|l| l.iter().all(|v| condition_holds(v, pack, tags, stats))),
        "neoforge:or" => c.get("values").and_then(Value::as_array).is_some_and(|l| l.iter().any(|v| condition_holds(v, pack, tags, stats))),
        "neoforge:tag_empty" => {
            let tag = c.get("tag").and_then(Value::as_str).unwrap_or("");
            tags.get(&format!("#{tag}")).is_none_or(Vec::is_empty)
        }
        "neoforge:item_exists" => c.get("item").and_then(Value::as_str).is_some_and(|i| pack.mods.contains(namespace_of(i))),
        "neoforge:true" => true,
        "neoforge:false" => false,
        other => {
            stats.unknown_conditions.insert(other.to_string());
            true
        }
    }
}

struct Parser<'a> {
    tags: &'a mut BTreeMap<ResId, Vec<ResId>>,
}

impl Parser<'_> {
    /// Returns the resource an ingredient stands for and its amount.
    fn ingredient(&mut self, v: &Value) -> Result<(ResId, f64), String> {
        if let Some(list) = v.as_array() {
            return self.alternatives(list);
        }
        let amount = v.get("amount").and_then(Value::as_f64);
        let count = v.get("count").and_then(Value::as_f64).unwrap_or(1.0);
        if let Some(children) = v.get("children").and_then(Value::as_array) {
            return self.alternatives(children);
        }
        if v.get("type").and_then(Value::as_str) == Some("neoforge:difference") {
            return self.difference(v);
        }
        // A fluid with components, such as a specific potion, is its own resource.
        if let (Some(f), Some(a)) = (v.get("fluids").and_then(Value::as_str), amount) {
            let potion = v.pointer("/components/minecraft:potion_contents/potion").and_then(Value::as_str);
            return Ok((potion.map_or(format!("fluid:{f}"), |p| format!("fluid:{f}[{p}]")), a));
        }
        if let Some(t) = v.get("fluid_tag").and_then(Value::as_str) {
            return Ok((format!("#fluid:{t}"), amount.unwrap_or(1.0)));
        }
        if let Some(f) = v.get("fluid").and_then(Value::as_str) {
            return Ok((format!("fluid:{f}"), amount.unwrap_or(1.0)));
        }
        if let (Some(t), Some(a)) = (v.get("tag").and_then(Value::as_str), amount) {
            return Ok((format!("#fluid:{t}"), a));
        }
        if let Some(t) = v.get("tag").and_then(Value::as_str) {
            return Ok((format!("#{t}"), count));
        }
        if let Some(i) = v.get("item").and_then(Value::as_str) {
            return Ok((i.to_string(), count));
        }
        if let Some(i) = v.get("id").and_then(Value::as_str) {
            return Ok((i.to_string(), count));
        }
        Err(format!("unsupported ingredient {v}"))
    }

    /// `neoforge:difference`: members of `base` without those of `subtracted`.
    fn difference(&mut self, v: &Value) -> Result<(ResId, f64), String> {
        let (base, amount) = self.ingredient(v.get("base").ok_or("difference without base")?)?;
        let (subtracted, _) = self.ingredient(v.get("subtracted").ok_or("difference without subtracted")?)?;
        let members_of = |res: &ResId, tags: &BTreeMap<ResId, Vec<ResId>>| {
            if res.starts_with('#') { tags.get(res).cloned().unwrap_or_default() } else { vec![res.clone()] }
        };
        let removed = members_of(&subtracted, self.tags);
        let members: Vec<Value> = members_of(&base, self.tags).into_iter().filter(|m| !removed.contains(m)).map(|m| serde_json::json!({ "item": m })).collect();
        let (res, _) = self.alternatives(&members)?;
        Ok((res, amount))
    }

    fn alternatives(&mut self, list: &[Value]) -> Result<(ResId, f64), String> {
        let mut members = BTreeSet::new();
        let mut amount = 1.0;
        for v in list {
            let (res, a) = self.ingredient(v)?;
            amount = a;
            if res.starts_with('#') {
                members.extend(self.tags.get(&res).cloned().unwrap_or_default());
            } else {
                members.insert(res);
            }
        }
        let members: Vec<ResId> = members.into_iter().collect();
        if members.is_empty() {
            return Err("empty ingredient".into());
        }
        if members.len() == 1 {
            return Ok((members[0].clone(), amount));
        }
        let id = format!("#any:{}", members.join("|"));
        self.tags.insert(id.clone(), members);
        Ok((id, amount))
    }

    fn output(&mut self, v: &Value) -> Result<Output, String> {
        let id = v.get("id").and_then(Value::as_str).ok_or_else(|| format!("output without id: {v}"))?;
        let chance = v.get("chance").and_then(Value::as_f64).unwrap_or(1.0);
        match v.get("amount").and_then(Value::as_f64) {
            Some(amount) => Ok(Output { res: format!("fluid:{id}"), amount, chance }),
            None => Ok(Output { res: id.to_string(), amount: v.get("count").and_then(Value::as_f64).unwrap_or(1.0), chance }),
        }
    }

    fn outputs(&mut self, v: &Value) -> Result<Vec<Output>, String> {
        v.get("results").and_then(Value::as_array).ok_or("recipe without results")?.iter().map(|o| self.output(o)).collect()
    }

    fn one(&mut self, v: &Value, field: &str) -> Result<Output, String> {
        self.output(v.get(field).ok_or_else(|| format!("recipe without {field}"))?)
    }

    fn ingredients(&mut self, v: &Value) -> Result<Vec<Stack>, String> {
        let list = v.get("ingredients").and_then(Value::as_array).ok_or("recipe without ingredients")?;
        let mut out = Vec::with_capacity(list.len());
        for i in list {
            let (res, amount) = self.ingredient(i)?;
            out.push(Stack { res, amount });
        }
        Ok(merge(out))
    }

    /// Reads `key` and `pattern`. Returns the inputs, occupied cells and whether it is a filled square.
    fn shaped(&mut self, v: &Value) -> Result<(Vec<Stack>, u32, bool), String> {
        let pattern: Vec<&str> = v.get("pattern").and_then(Value::as_array).ok_or("no pattern")?.iter().filter_map(Value::as_str).collect();
        let key = v.get("key").and_then(Value::as_object).ok_or("no key")?;
        let mut parsed: HashMap<char, (ResId, f64)> = HashMap::new();
        for (k, ing) in key {
            if let Some(c) = k.chars().next() {
                parsed.insert(c, self.ingredient(ing)?);
            }
        }
        let mut inputs = Vec::new();
        let mut cells = 0;
        for c in pattern.iter().flat_map(|row| row.chars()).filter(|c| *c != ' ') {
            let (res, amount) = parsed.get(&c).ok_or_else(|| format!("pattern char {c} not in key"))?;
            inputs.push(Stack { res: res.clone(), amount: *amount });
            cells += 1;
        }
        let side = pattern.len() as u32;
        let square = (side == 2 || side == 3) && cells == side * side && merge(inputs.clone()).len() == 1;
        Ok((merge(inputs), cells, square))
    }

    fn cooking(&mut self, v: &Value, default_time: u32) -> Result<(Vec<Stack>, Vec<Output>, u32), String> {
        let (res, amount) = self.ingredient(v.get("ingredient").ok_or("no ingredient")?)?;
        let time = v.get("cookingtime").and_then(Value::as_u64).map(|t| t as u32).unwrap_or(default_time);
        Ok((vec![Stack { res, amount }], vec![self.one(v, "result")?], time))
    }

    fn recipe(&mut self, id: &str, kind: RecipeKind, v: &Value) -> Result<Recipe, String> {
        use RecipeKind::*;
        let mut r = Recipe {
            id: id.to_string(),
            kind,
            inputs: vec![],
            outputs: vec![],
            duration: v.get("processing_time").and_then(Value::as_u64).unwrap_or(0) as u32,
            heat: match v.get("heat_requirement").and_then(Value::as_str) {
                Some("heated") => Heat::Heated,
                Some("superheated") => Heat::Superheated,
                _ => Heat::None,
            },
            cells: 0,
            square: false,
            steps: vec![],
            loops: 0,
        };
        match kind {
            Smelting | Blasting | Smoking => {
                let default = if kind == Smelting { 200 } else { 100 };
                (r.inputs, r.outputs, r.duration) = self.cooking(v, default)?;
            }
            CraftingShaped | MechanicalCrafting => {
                (r.inputs, r.cells, r.square) = self.shaped(v)?;
                r.outputs = vec![self.one(v, "result")?];
            }
            CraftingShapeless => {
                r.inputs = self.ingredients(v)?;
                r.cells = r.inputs.iter().map(|s| s.amount as u32).sum();
                r.outputs = vec![self.one(v, "result")?];
            }
            Stonecutting => {
                let (res, amount) = self.ingredient(v.get("ingredient").ok_or("no ingredient")?)?;
                r.inputs = vec![Stack { res, amount }];
                r.outputs = vec![self.one(v, "result")?];
            }
            Smithing => {
                for field in ["template", "base", "addition"] {
                    let (res, amount) = self.ingredient(v.get(field).ok_or_else(|| format!("no {field}"))?)?;
                    r.inputs.push(Stack { res, amount });
                }
                r.outputs = vec![self.one(v, "result")?];
            }
            SequencedAssembly => self.sequenced(v, &mut r)?,
            _ => {
                r.inputs = self.ingredients(v)?;
                if kind == Deploying && v.get("keep_held_item").and_then(Value::as_bool).unwrap_or(false) {
                    r.inputs.pop();
                }
                r.outputs = self.outputs(v)?;
            }
        }
        if r.outputs.is_empty() {
            return Err("no outputs".into());
        }
        Ok(r)
    }

    /// `results` holds weights, so they are normalised into probabilities per finished item.
    fn sequenced(&mut self, v: &Value, r: &mut Recipe) -> Result<(), String> {
        let (base, amount) = self.ingredient(v.get("ingredient").ok_or("no ingredient")?)?;
        r.loops = v.get("loops").and_then(Value::as_u64).unwrap_or(1) as u32;
        let transitional = v.get("transitional_item").and_then(|t| t.get("id")).and_then(Value::as_str).unwrap_or("");
        let mut inputs = vec![Stack { res: base, amount }];
        for step in v.get("sequence").and_then(Value::as_array).ok_or("no sequence")? {
            let kind = step.get("type").and_then(Value::as_str).and_then(recipe_kind).ok_or("unknown assembly step")?;
            let keep = step.get("keep_held_item").and_then(Value::as_bool).unwrap_or(false);
            let held = step.get("ingredients").and_then(Value::as_array).ok_or("step without ingredients")?;
            for (i, ingredient) in held.iter().enumerate() {
                let (res, amount) = self.ingredient(ingredient)?;
                let is_kept = keep && i + 1 == held.len();
                if res != transitional && !is_kept {
                    inputs.push(Stack { res, amount: amount * r.loops as f64 });
                }
            }
            let duration = step.get("processing_time").and_then(Value::as_u64).unwrap_or(0) as u32;
            r.steps.push(AssemblyStep { kind, duration });
        }
        r.inputs = merge(inputs);
        let mut outputs = self.outputs(v)?;
        let total: f64 = outputs.iter().map(|o| o.chance).sum();
        for o in &mut outputs {
            o.chance /= total;
        }
        r.outputs = outputs;
        Ok(())
    }
}

/// Sums repeated resources, keeping first-seen order.
fn merge(list: Vec<Stack>) -> Vec<Stack> {
    let mut out: Vec<Stack> = Vec::new();
    for s in list {
        match out.iter_mut().find(|o| o.res == s.res) {
            Some(o) => o.amount += s.amount,
            None => out.push(s),
        }
    }
    out
}

fn recipe_kind(t: &str) -> Option<RecipeKind> {
    use RecipeKind::*;
    Some(match t {
        "create:pressing" => Pressing,
        "create:compacting" => Compacting,
        "create:mixing" => Mixing,
        "create:milling" => Milling,
        "create:crushing" => Crushing,
        "create:cutting" => Cutting,
        "create:deploying" | "create:item_application" | "create:sandpaper_polishing" => Deploying,
        "create:filling" => Filling,
        "create:emptying" => Emptying,
        "create:splashing" => Splashing,
        "create:haunting" => Haunting,
        "minecraft:smelting" => Smelting,
        "minecraft:blasting" => Blasting,
        "minecraft:smoking" => Smoking,
        "minecraft:crafting_shaped" => CraftingShaped,
        "minecraft:crafting_shapeless" => CraftingShapeless,
        "create:mechanical_crafting" => MechanicalCrafting,
        "minecraft:stonecutting" => Stonecutting,
        "minecraft:smithing_transform" => Smithing,
        "create:sequenced_assembly" => SequencedAssembly,
        _ => return None,
    })
}

/// Imports JARs in order; later JARs override recipes and extend tags of earlier ones.
/// Put the base game first, then NeoForge, then mods.
pub fn import_jars(paths: &[&Path]) -> Result<(Bundle, ImportStats), String> {
    let mut pack = Pack::default();
    pack.mods.insert("minecraft".into());
    for p in paths {
        read_jar(p, &mut pack)?;
    }
    let mut tags = resolve_tags(&pack);
    let mut stats = ImportStats::default();
    let mut recipes = Vec::new();
    let recipe_docs = std::mem::take(&mut pack.recipes);
    for (id, doc) in &recipe_docs {
        let snapshot = tags.clone();
        let conditions = doc.get("neoforge:conditions").and_then(Value::as_array);
        if !conditions.into_iter().flatten().all(|c| condition_holds(c, &pack, &snapshot, &mut stats)) {
            stats.skipped_by_condition += 1;
            continue;
        }
        let type_name = doc.get("type").and_then(Value::as_str).unwrap_or("(none)");
        let Some(kind) = recipe_kind(type_name) else {
            *stats.skipped_types.entry(type_name.to_string()).or_default() += 1;
            continue;
        };
        match (Parser { tags: &mut tags }).recipe(id, kind, doc) {
            Ok(r) => recipes.push(r),
            Err(e) => stats.failed.push(format!("{id}: {e}")),
        }
    }
    add_stripping_recipes(&mut recipes, &tags);
    stats.recipes = recipes.len();
    let names = collect_names(&pack);
    Ok((Bundle { recipes, tags, names }, stats))
}

/// An axe strips logs through the game's code rather than a recipe file: Create only shows it in
/// JEI (`LogStrippingFakeRecipes`), and a deployer does it by using the axe on a placed log
/// (`DeployerHandler`). So these recipes are made here, for every log that has a stripped form.
pub fn add_stripping_recipes(recipes: &mut Vec<Recipe>, tags: &BTreeMap<ResId, Vec<ResId>>) {
    let logs: BTreeSet<&ResId> = ["#minecraft:logs", "#minecraft:bamboo_blocks"].iter().filter_map(|t| tags.get(*t)).flatten().collect();
    for log in &logs {
        let Some((ns, path)) = log.split_once(':') else { continue };
        let stripped = format!("{ns}:stripped_{path}");
        if !logs.contains(&stripped) {
            continue;
        }
        recipes.push(Recipe {
            id: format!("{STRIPPING_PREFIX}{ns}/{path}"),
            kind: RecipeKind::Deploying,
            inputs: vec![Stack { res: (*log).clone(), amount: 1.0 }],
            outputs: vec![Output { res: stripped, amount: 1.0, chance: 1.0 }],
            duration: 0,
            heat: Heat::None,
            cells: 0,
            square: false,
            steps: vec![],
            loops: 0,
        });
    }
}

fn collect_names(pack: &Pack) -> BTreeMap<ResId, Names> {
    let mut names: BTreeMap<ResId, Names> = BTreeMap::new();
    for (path, json) in &pack.lang {
        add_names(&mut names, json, path.ends_with("ru_ru.json"));
    }
    names
}

/// Adds the item and fluid names of one language file. Minecraft keeps its translations
/// outside the JAR, so this is also how they get into a bundle.
pub fn add_names(names: &mut BTreeMap<ResId, Names>, lang_file: &Value, russian: bool) {
    let Some(entries) = lang_file.as_object() else { return };
    let mut set = |id: String, text: &str| {
        let entry = names.entry(id).or_default();
        if russian {
            entry.ru = Some(text.to_string());
        } else {
            entry.en = Some(text.to_string());
        }
    };
    for (key, value) in entries {
        let Some(text) = value.as_str() else { continue };
        let mut parts = key.splitn(3, '.');
        let (Some(kind), Some(ns), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        if path.contains('.') {
            continue;
        }
        match kind {
            "item" | "block" => {
                set(format!("{ns}:{path}"), text);
                // The game names water and lava only as blocks.
                if kind == "block" && ns == "minecraft" && (path == "water" || path == "lava") {
                    set(format!("fluid:{ns}:{path}"), text);
                }
            }
            "fluid" | "fluid_type" => set(format!("fluid:{ns}:{path}"), text),
            _ => {}
        }
    }
}
