//! Turns a request into a production graph by solving a linear program.
//!
//! Variables are recipe runs per minute, raw sources and surplus for each resource.
//! Every resource balances: produced + sourced = consumed + surplus + demanded.
//! The objective minimises raw material cost, with a tiny cost per run to avoid pointless loops.

use crate::config::Config;
use crate::machines::{self, Load, Machine, Transport};
use crate::model::{Catalog, Heat, Recipe, RecipeKind, ResId};
use crate::power::{CAKE_BURN_TICKS, COAL_BURN_TICKS};
use good_lp::{constraint, variable, variables, Expression, Solution, SolverModel, Variable};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

const RUN_COST: f64 = 0.01;
/// Cost of a recipeless item that is not on the base resource list.
const UNUSUAL_RAW_COST: f64 = 1000.0;
const STRIPPED_RAW_COST: f64 = 1.5;
/// Taking another member of a tag than its default must save more than a step of work,
/// see `Catalog::tag_default`. Raw materials cost far more, so real savings still win.
const OTHER_TAG_MEMBER_COST: f64 = RUN_COST;
/// Full automation prices: endless generators, then farms, then digging by hand.
const AUTO_GENERATOR_COST: f64 = 0.01;
const AUTO_FARM_COST: f64 = 5.0;
const AUTO_MINED_COST: f64 = 100.0;
const GENERATORS: [&str; 4] = ["minecraft:cobblestone", "minecraft:stone", "fluid:minecraft:water", "fluid:minecraft:lava"];
const FLUID_COST_PER_MB: f64 = 1e-3;
/// Editable lists of what players normally gather.
const MINED_RESOURCES: &str = include_str!("../data/mined_resources.txt");
const FARM_RESOURCES: &str = include_str!("../data/farm_resources.txt");
const MATERIAL_RESOURCES: &str = include_str!("../data/material_resources.txt");
const EPS: f64 = 1e-6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub item: ResId,
    pub per_minute: f64,
}

/// How raw materials are valued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Everything on the resource lists costs the same.
    #[default]
    Normal,
    /// Prefers endless generators and farms, and digs by hand only when nothing else works.
    FullAuto,
}

/// Where a gathered resource comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Generator,
    Farm,
    Mined,
    /// A ready material, only with `stop_at_materials`.
    Material,
    Unusual,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeOverride {
    pub rpm: Option<f64>,
    pub machine: Option<Machine>,
    pub batch: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanRequest {
    pub targets: Vec<Target>,
    /// RPM for every machine without its own override.
    pub rpm: f64,
    pub config: Config,
    /// Resources taken from storage: free, and nothing is planned above them.
    pub stock: Vec<ResId>,
    /// Forces the recipe that makes an item, by item id.
    pub recipe_choice: BTreeMap<ResId, String>,
    pub disabled_recipes: Vec<String>,
    /// Per-recipe settings, by recipe id.
    pub overrides: BTreeMap<String, NodeOverride>,
    /// Batch size for crushing wheels and fans without an override.
    pub batch: u32,
    /// Report blaze burners and fuel.
    pub include_heat: bool,
    /// Replaces the built-in base resource list when set. Entries may be tags.
    pub base_resources: Option<Vec<ResId>>,
    pub mode: Mode,
    /// RPM of belts, the machine RPM when unset.
    pub belt_rpm: Option<f64>,
    /// Stack size on belts where neither `stack_sizes` nor the receiving machine sets one.
    pub stack: u32,
    /// Stack size per item.
    pub stack_sizes: BTreeMap<ResId, u32>,
    /// Stops at ready materials such as ingots instead of going down to ore.
    pub stop_at_materials: bool,
    /// Extra recipes hung off the scheme, applied in order.
    pub branches: Vec<Branch>,
}

/// A recipe added to the scheme from one of its items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub recipe: String,
    /// The item the branch starts from, which the recipe consumes.
    pub input: ResId,
    /// Output of the recipe's main item per minute. When unset the branch takes all the
    /// leftover `input`, and follows it when the rest of the scheme changes.
    pub per_minute: Option<f64>,
}

impl Default for PlanRequest {
    fn default() -> Self {
        Self {
            targets: vec![],
            rpm: 256.0,
            config: Config::default(),
            stock: vec![],
            recipe_choice: BTreeMap::new(),
            disabled_recipes: vec![],
            overrides: BTreeMap::new(),
            batch: 1,
            include_heat: true,
            base_resources: None,
            mode: Mode::Normal,
            belt_rpm: None,
            stack: 1,
            stack_sizes: BTreeMap::new(),
            stop_at_materials: false,
            branches: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    /// A recipe running on machines.
    Recipe {
        recipe: String,
        /// What the plan needs from this recipe: the target, else the output used the most.
        produces: ResId,
        runs_per_min: f64,
        /// One entry, or one per step for sequenced assembly.
        loads: Vec<Load>,
        heat: Heat,
        burners: u32,
        /// Other recipes that make `produces`, those with it as the main output first.
        alternatives: Vec<String>,
        machine_options: Vec<Machine>,
    },
    /// No way to produce it: gather by hand.
    Gather { res: ResId, per_minute: f64, origin: Origin },
    /// An endless source in full automation. Fluids have no load.
    Generate { res: ResId, per_minute: f64, load: Option<Load> },
    /// Taken from storage at the user's request.
    Stock { res: ResId, per_minute: f64 },
    /// Unused by-product or excess, an end of the graph.
    Surplus { res: ResId, per_minute: f64 },
    /// What was asked for.
    Target { res: ResId, per_minute: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    #[serde(flatten)]
    pub kind: NodeKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub res: ResId,
    pub per_minute: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<Transport>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeatStats {
    pub burners_heated: u32,
    pub burners_superheated: u32,
    /// Coal per minute for the heated burners.
    pub coal_per_min: f64,
    /// Blaze cakes per minute for the superheated burners.
    pub cake_per_min: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub total_su: f64,
    pub machines: BTreeMap<String, u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heat: Option<HeatStats>,
    pub warnings: Vec<String>,
    /// Full automation only: resources that still have to be dug out or found by hand.
    pub manual: Vec<ResId>,
    /// Everything to craft or place to build this scheme. Shafts, pipes and power are not included.
    pub build: Vec<BuildItem>,
    /// What each branch came to.
    pub branches: Vec<ResolvedBranch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedBranch {
    pub recipe: String,
    pub item: ResId,
    pub per_minute: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildItem {
    pub item: ResId,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlanError {
    Invalid(String),
    Solver(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::Invalid(m) => write!(f, "invalid request: {m}"),
            PlanError::Solver(m) => write!(f, "solver failed: {m}"),
        }
    }
}

impl std::error::Error for PlanError {}

/// Parses a resource list, expanding tags.
fn parse_list(cat: &Catalog, entries: impl Iterator<Item = String>, origin: Origin, out: &mut HashMap<ResId, Origin>) {
    for entry in entries {
        if entry.starts_with('#') {
            for member in cat.tag_members(&entry) {
                out.insert(member.clone(), origin);
            }
        } else {
            out.insert(entry, origin);
        }
    }
}

fn file_lines(text: &str) -> impl Iterator<Item = String> + '_ {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("# ")).map(String::from)
}

/// Resources that may be gathered, and where they come from.
/// Choosing a recipe for one takes it off the list.
fn gatherable(cat: &Catalog, req: &PlanRequest) -> HashMap<ResId, Origin> {
    let mut out = HashMap::new();
    match &req.base_resources {
        Some(list) => parse_list(cat, list.iter().cloned(), Origin::Farm, &mut out),
        None => {
            parse_list(cat, file_lines(MINED_RESOURCES), Origin::Mined, &mut out);
            parse_list(cat, file_lines(FARM_RESOURCES), Origin::Farm, &mut out);
            // The log tag holds bark blocks too, and those are crafted from logs. The flower tag
            // holds the wither rose, which grows only where a wither has killed something.
            out.retain(|r, _| !r.ends_with("_wood") && !r.ends_with("_hyphae") && r != "minecraft:wither_rose");
            parse_list(cat, GENERATORS.iter().map(|g| g.to_string()), Origin::Generator, &mut out);
        }
    }
    if req.stop_at_materials {
        parse_list(cat, file_lines(MATERIAL_RESOURCES), Origin::Material, &mut out);
    }
    out.retain(|r, _| !req.recipe_choice.contains_key(r));
    out
}

/// Whether the walk stops at a resource instead of looking for recipes that make it.
/// In full automation mined resources stay open, so gravel can come from cobblestone.
fn stops_walk(req: &PlanRequest, origin: Origin) -> bool {
    req.mode == Mode::Normal || origin != Origin::Mined
}

/// Whether a recipe can run at all under this config.
fn runnable(r: &Recipe, cfg: &Config) -> bool {
    r.kind == RecipeKind::SequencedAssembly || !machines::options(r, cfg).is_empty()
}

/// Recipes the request allows, by index in the catalog.
fn allowed(cat: &Catalog, req: &PlanRequest) -> HashSet<usize> {
    let disabled: HashSet<&str> = req.disabled_recipes.iter().map(String::as_str).collect();
    let chosen: HashSet<&str> = req.recipe_choice.values().map(String::as_str).collect();
    cat.recipes()
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            if disabled.contains(r.id.as_str()) || !runnable(r, &req.config) {
                return false;
            }
            // A forced recipe is the only source of its item, by-products of others included,
            // unless the other recipe was forced too.
            chosen.contains(r.id.as_str()) || !r.outputs.iter().any(|o| req.recipe_choice.contains_key(&o.res))
        })
        .map(|(i, _)| i)
        .collect()
}

/// Recipes able to make an item, those with it as the main output first.
pub fn producers<'a>(cat: &'a Catalog, item: &str, cfg: &Config) -> Vec<&'a Recipe> {
    let mut list: Vec<&Recipe> = cat.producer_indexes(item).iter().map(|&i| &cat.recipes()[i]).filter(|r| runnable(r, cfg)).collect();
    list.sort_by_key(|r| r.main_output().map(String::as_str) != Some(item));
    list
}

/// Walks back from the targets to find the resources and recipes involved.
fn reach(cat: &Catalog, req: &PlanRequest, allowed: &HashSet<usize>, base: &HashMap<ResId, Origin>) -> (BTreeSet<ResId>, BTreeSet<usize>) {
    let stock: HashSet<&str> = req.stock.iter().map(String::as_str).collect();
    let mut resources = BTreeSet::new();
    let mut recipes = BTreeSet::new();
    let mut queue: VecDeque<ResId> = req.targets.iter().map(|t| t.item.clone()).collect();
    while let Some(res) = queue.pop_front() {
        // Nothing is planned above stock or a gathered resource.
        let stop = base.get(&res).is_some_and(|o| stops_walk(req, *o));
        if !resources.insert(res.clone()) || stock.contains(res.as_str()) || stop {
            continue;
        }
        if res.starts_with('#') {
            queue.extend(cat.tag_members(&res).iter().cloned());
            continue;
        }
        for &i in cat.producer_indexes(&res) {
            if allowed.contains(&i) && recipes.insert(i) {
                queue.extend(cat.recipes()[i].inputs.iter().map(|s| s.res.clone()));
            }
        }
    }
    // By-products of chosen recipes need a balance too.
    for &i in &recipes {
        resources.extend(cat.recipes()[i].outputs.iter().map(|o| o.res.clone()));
    }
    (resources, recipes)
}

/// Resources that can actually be made from things with no recipe, walking forward.
fn derivable(cat: &Catalog, resources: &BTreeSet<ResId>, recipes: &BTreeSet<usize>, stock: &HashSet<&str>, base: &HashMap<ResId, Origin>) -> HashSet<ResId> {
    let produced: HashSet<&ResId> = recipes.iter().flat_map(|&i| cat.recipes()[i].outputs.iter().map(|o| &o.res)).collect();
    let mut known: HashSet<ResId> = resources
        .iter()
        .filter(|r| stock.contains(r.as_str()) || base.contains_key(*r) || (!r.starts_with('#') && !produced.contains(r)))
        .cloned()
        .collect();
    loop {
        let before = known.len();
        for res in resources.iter().filter(|r| r.starts_with('#')) {
            if cat.tag_members(res).iter().any(|m| known.contains(m)) {
                known.insert(res.clone());
            }
        }
        for &i in recipes {
            let r = &cat.recipes()[i];
            if r.inputs.iter().all(|s| known.contains(&s.res)) {
                known.extend(r.outputs.iter().map(|o| o.res.clone()));
            }
        }
        if known.len() == before {
            return known;
        }
    }
}

/// Raw cost of each resource. A compressed form such as a storage block costs as much as
/// its parts, otherwise gathering one block would look cheaper than nine diamonds.
fn raw_costs(cat: &Catalog, recipes: &BTreeSet<usize>, resources: &BTreeSet<ResId>, base: &HashMap<ResId, Origin>, mode: Mode) -> HashMap<ResId, f64> {
    let mut cost: HashMap<ResId, f64> = resources
        .iter()
        .map(|r| {
            let unit = if r.contains("fluid:") { FLUID_COST_PER_MB } else { 1.0 };
            // Stripping is extra work, so plain logs win when a recipe accepts both.
            let stripped = if r.contains(":stripped_") { STRIPPED_RAW_COST } else { 1.0 };
            let price = match (base.get(r), mode) {
                // Not scaled for fluids: a potion nobody can make must not beat digging for redstone.
                (None, _) => return (r.clone(), UNUSUAL_RAW_COST),
                (Some(_), Mode::Normal) => stripped,
                (Some(Origin::Generator), Mode::FullAuto) => AUTO_GENERATOR_COST,
                (Some(Origin::Farm), Mode::FullAuto) => AUTO_FARM_COST * stripped,
                (Some(_), Mode::FullAuto) => AUTO_MINED_COST,
            };
            (r.clone(), unit * price)
        })
        .collect();
    let list: Vec<&Recipe> = recipes.iter().map(|&i| &cat.recipes()[i]).collect();
    for a in &list {
        let ([input], [output]) = (a.inputs.as_slice(), a.outputs.as_slice()) else { continue };
        let is_inverse = list.iter().any(|b| match (b.inputs.as_slice(), b.outputs.as_slice()) {
            ([bi], [bo]) => bi.res == output.res && bo.res == input.res && bo.amount / bi.amount == input.amount / output.amount,
            _ => false,
        });
        if is_inverse && input.amount > output.amount {
            let part = cost.get(&input.res).copied().unwrap_or(1.0);
            // Slightly over the sum of its parts, so a tie never gathers the block itself.
            cost.insert(output.res.clone(), part * input.amount / output.amount * 1.01);
        }
    }
    cost
}

/// Items a run yields. Charging by output rather than by run keeps a block crafted
/// and then broken up from looking cheaper than handling its nine items directly.
fn output_units(r: &Recipe) -> f64 {
    let units: f64 = r.outputs.iter().map(|o| if o.res.contains("fluid:") { o.expected() / 1000.0 } else { o.expected() }).sum();
    units.max(1.0)
}

/// Recipes that consume an item, directly or through a tag.
pub fn consumers<'a>(cat: &'a Catalog, item: &str, cfg: &Config) -> Vec<&'a Recipe> {
    cat.recipes().iter().filter(|r| runnable(r, cfg) && r.inputs.iter().any(|s| takes(cat, &s.res, item))).collect()
}

fn takes(cat: &Catalog, input: &str, item: &str) -> bool {
    input == item || cat.tag_members(input).iter().any(|m| m == item)
}

/// Plans the scheme with its branches. A branch on all the leftovers needs the scheme before it
/// to know how much is left, so each one is resolved on top of the previous.
pub fn plan(cat: &Catalog, req: &PlanRequest) -> Result<Plan, PlanError> {
    if req.branches.is_empty() {
        return solve(cat, req);
    }
    let mut current = req.clone();
    current.branches = vec![];
    // A branch must not rename the recipes it hangs off, so names come from the scheme without it.
    let named: HashMap<String, ResId> = solve(cat, &current)?
        .nodes
        .into_iter()
        .filter_map(|n| match n.kind {
            NodeKind::Recipe { recipe, produces, .. } => Some((recipe, produces)),
            _ => None,
        })
        .collect();
    let mut resolved = Vec::new();
    let mut warnings = Vec::new();
    for b in &req.branches {
        let r = cat.recipe(&b.recipe).ok_or_else(|| PlanError::Invalid(format!("no recipe {}", b.recipe)))?;
        let main = r.main_output().ok_or_else(|| PlanError::Invalid(format!("{} has no output", b.recipe)))?.clone();
        let needed = r.inputs.iter().find(|s| takes(cat, &s.res, &b.input));
        let needed = needed.ok_or_else(|| PlanError::Invalid(format!("{} does not use {}", b.recipe, b.input)))?.amount;
        let per_minute = match b.per_minute {
            Some(rate) => rate,
            None => {
                let left: f64 = solve(cat, &current)?
                    .nodes
                    .iter()
                    .filter_map(|n| match &n.kind {
                        NodeKind::Surplus { res, per_minute } if res == &b.input => Some(*per_minute),
                        _ => None,
                    })
                    .sum();
                let per_run: f64 = r.outputs.iter().filter(|o| o.res == main).map(|o| o.expected()).sum();
                left / needed * per_run
            }
        };
        if per_minute <= EPS {
            warnings.push(format!("{}: no leftover {} for this branch", b.recipe, b.input));
            continue;
        }
        match current.targets.iter_mut().find(|t| t.item == main) {
            Some(t) => t.per_minute += per_minute,
            None => current.targets.push(Target { item: main.clone(), per_minute }),
        }
        current.recipe_choice.insert(main.clone(), b.recipe.clone());
        resolved.push(ResolvedBranch { recipe: b.recipe.clone(), item: main, per_minute });
    }
    let mut result = solve(cat, &current)?;
    for node in &mut result.nodes {
        let NodeKind::Recipe { recipe, produces, alternatives, .. } = &mut node.kind else { continue };
        if let Some(name) = named.get(recipe) {
            *alternatives = alternatives_for(cat, req, recipe, name);
            *produces = name.clone();
        }
    }
    result.branches = resolved;
    result.warnings.extend(warnings);
    Ok(result)
}

fn solve(cat: &Catalog, req: &PlanRequest) -> Result<Plan, PlanError> {
    if req.targets.is_empty() {
        return Err(PlanError::Invalid("no targets".into()));
    }
    let check_rpm = |rpm: f64| {
        if rpm <= 0.0 || rpm > req.config.max_rotation_speed {
            Err(PlanError::Invalid(format!("rpm {rpm} is outside 0..{}", req.config.max_rotation_speed)))
        } else {
            Ok(())
        }
    };
    check_rpm(req.rpm)?;
    if let Some(rpm) = req.belt_rpm {
        check_rpm(rpm)?;
    }
    for o in req.overrides.values() {
        if let Some(rpm) = o.rpm {
            check_rpm(rpm)?;
        }
    }
    for t in &req.targets {
        if t.per_minute <= 0.0 {
            return Err(PlanError::Invalid(format!("{}: rate must be positive", t.item)));
        }
    }

    let stock: HashSet<&str> = req.stock.iter().map(String::as_str).collect();
    let allowed = allowed(cat, req);
    let base = gatherable(cat, req);
    let (resources, recipes) = reach(cat, req, &allowed, &base);
    let known = derivable(cat, &resources, &recipes, &stock, &base);
    let costs = raw_costs(cat, &recipes, &resources, &base, req.mode);

    let mut vars = variables!();
    let runs: BTreeMap<usize, Variable> = recipes.iter().map(|&i| (i, vars.add(variable().min(0.0)))).collect();
    let mut sources: BTreeMap<&ResId, Variable> = BTreeMap::new();
    let mut surplus: BTreeMap<&ResId, Variable> = BTreeMap::new();
    // Conversions from a member into its tag, so the cheapest member wins.
    let mut conversions: BTreeMap<(ResId, ResId), Variable> = BTreeMap::new();
    let produced: HashSet<&ResId> = recipes.iter().flat_map(|&i| cat.recipes()[i].outputs.iter().map(|o| &o.res)).collect();

    for res in &resources {
        // A tag only passes items through, so leftovers stay on the real item.
        if !res.starts_with('#') {
            surplus.insert(res, vars.add(variable().min(0.0)));
        }
        if res.starts_with('#') {
            let members = cat.tag_members(res);
            for m in members.iter().filter(|m| resources.contains(*m)) {
                conversions.insert((res.clone(), m.clone()), vars.add(variable().min(0.0)));
            }
            // An empty tag cannot be converted from anything.
            if !stock.contains(res.as_str()) && members.is_empty() {
                sources.insert(res, vars.add(variable().min(0.0)));
            }
        } else if stock.contains(res.as_str()) || base.contains_key(res) || !produced.contains(res) || !known.contains(res) {
            sources.insert(res, vars.add(variable().min(0.0)));
        }
        if res.starts_with('#') && stock.contains(res.as_str()) {
            sources.insert(res, vars.add(variable().min(0.0)));
        }
    }

    let mut objective = Expression::from(0.0);
    for (&i, &v) in &runs {
        objective += RUN_COST * output_units(&cat.recipes()[i]) * v;
    }
    for (res, &v) in &sources {
        if !stock.contains(res.as_str()) {
            objective += costs.get(*res).copied().unwrap_or(1.0) * v;
        }
    }
    // Wood species and the like cost the same, so without this the solver picks any of them.
    for ((tag, member), &v) in &conversions {
        if cat.tag_default(tag).is_some_and(|d| d != member) {
            objective += OTHER_TAG_MEMBER_COST * v;
        }
    }

    // Leftovers are free on purpose. Any price makes the planner build machines just to destroy
    // or compress them, and raw material prices already discourage producing too much.

    let mut model = vars.minimise(objective).using(good_lp::default_solver);
    let demand = |res: &str| req.targets.iter().filter(|t| t.item == res).map(|t| t.per_minute).sum::<f64>();
    for res in &resources {
        let mut balance = Expression::from(0.0);
        for (&i, &v) in &runs {
            let r = &cat.recipes()[i];
            let out: f64 = r.outputs.iter().filter(|o| &o.res == res).map(|o| o.expected()).sum();
            let inp: f64 = r.inputs.iter().filter(|s| &s.res == res).map(|s| s.amount).sum();
            if out != 0.0 || inp != 0.0 {
                balance += (out - inp) * v;
            }
        }
        for ((tag, member), &v) in &conversions {
            if tag == res {
                balance += v;
            } else if member == res {
                balance -= v;
            }
        }
        if let Some(&s) = sources.get(res) {
            balance += s;
        }
        if let Some(&w) = surplus.get(res) {
            balance -= w;
        }
        model = model.with(constraint!(balance == demand(res)));
    }
    let solution = model.solve().map_err(|e| PlanError::Solver(e.to_string()))?;

    build_plan(cat, req, Solved {
        runs: runs.iter().map(|(&i, &v)| (i, solution.value(v))).collect(),
        sources: sources.iter().map(|(r, &v)| ((*r).clone(), solution.value(v))).collect(),
        surplus: surplus.iter().map(|(r, &v)| ((*r).clone(), solution.value(v))).collect(),
        conversions: conversions.iter().map(|(k, &v)| (k.clone(), solution.value(v))).collect(),
    }, &stock, &base)
}

struct Solved {
    runs: BTreeMap<usize, f64>,
    sources: BTreeMap<ResId, f64>,
    surplus: BTreeMap<ResId, f64>,
    conversions: BTreeMap<(ResId, ResId), f64>,
}

fn size_recipe(r: &Recipe, req: &PlanRequest, runs_per_min: f64) -> (Vec<Load>, Vec<Machine>) {
    let cfg = &req.config;
    let over = req.overrides.get(&r.id);
    let rpm = over.and_then(|o| o.rpm).unwrap_or(req.rpm);
    let batch = over.and_then(|o| o.batch).unwrap_or(req.batch);
    if !r.steps.is_empty() {
        let ops = runs_per_min * r.loops as f64;
        let loads = r
            .steps
            .iter()
            .map(|s| machines::size(machines::step_machine(s.kind), s.kind, s.duration, 1, rpm, 1, ops, cfg))
            .collect();
        return (loads, vec![]);
    }
    let options = machines::options(r, cfg);
    let sized = |m: Machine| machines::size(m, r.kind, r.duration, r.cells, rpm, batch, runs_per_min, cfg);
    let chosen = over.and_then(|o| o.machine).filter(|m| options.contains(m));
    let is_grid_recipe = matches!(r.kind, RecipeKind::CraftingShaped | RecipeKind::CraftingShapeless);
    let load = match chosen {
        Some(m) => sized(m),
        // A crafting recipe may also fit a press or mixer, and those need far less stress.
        None if is_grid_recipe => options
            .iter()
            .filter(|m| **m != Machine::Hand)
            .map(|m| sized(*m))
            .min_by(|a, b| a.su.total_cmp(&b.su))
            .unwrap_or_else(|| sized(Machine::Hand)),
        None => sized(options.first().copied().unwrap_or(Machine::Hand)),
    };
    (vec![load], options)
}

fn machine_key(m: Machine) -> String {
    serde_json::to_value(m).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

fn build_plan(cat: &Catalog, req: &PlanRequest, solved: Solved, stock: &HashSet<&str>, base: &HashMap<ResId, Origin>) -> Result<Plan, PlanError> {
    let mut nodes = Vec::new();
    let mut suppliers: BTreeMap<ResId, Vec<(String, f64)>> = BTreeMap::new();
    let mut consumers: BTreeMap<ResId, Vec<(String, f64)>> = BTreeMap::new();
    let (mut total_su, mut machine_counts) = (0.0, BTreeMap::new());
    let (mut heated, mut superheated) = (0, 0);
    let mut warnings = Vec::new();
    let mut manual = Vec::new();

    for (&i, &x) in &solved.runs {
        if x <= EPS {
            continue;
        }
        let r = &cat.recipes()[i];
        let id = format!("recipe:{}", r.id);
        let (loads, machine_options) = size_recipe(r, req, x);
        for load in &loads {
            total_su += load.su;
            *machine_counts.entry(machine_key(load.machine)).or_insert(0u32) += load.count;
            if let (Some(max), Some(_)) = (load.max_belt_rpm, load.stream_blocks) {
                if max < 1.0 {
                    warnings.push(format!("{}: fan stream is too short for any belt speed", r.id));
                }
            }
        }
        let burners = if r.heat == Heat::None { 0 } else { loads.iter().map(|l| l.count).sum() };
        match r.heat {
            Heat::Heated => heated += burners,
            Heat::Superheated => superheated += burners,
            Heat::None => {}
        }
        nodes.push(Node {
            id: id.clone(),
            kind: NodeKind::Recipe {
                recipe: r.id.clone(),
                produces: r.main_output().cloned().unwrap_or_default(),
                runs_per_min: x,
                loads,
                heat: r.heat,
                burners,
                alternatives: vec![],
                machine_options,
            },
        });
        for o in &r.outputs {
            suppliers.entry(o.res.clone()).or_default().push((id.clone(), x * o.expected()));
        }
        for s in &r.inputs {
            consumers.entry(s.res.clone()).or_default().push((id.clone(), x * s.amount));
        }
    }
    for (res, &v) in &solved.sources {
        if v <= EPS {
            continue;
        }
        let in_stock = stock.contains(res.as_str());
        let origin = base.get(res).copied().unwrap_or(Origin::Unusual);
        if !in_stock && origin == Origin::Unusual {
            warnings.push(format!("{res}: not a usual resource and no way to craft it was found"));
        }
        let auto = req.mode == Mode::FullAuto;
        if !in_stock && auto && matches!(origin, Origin::Mined | Origin::Unusual) {
            manual.push(res.clone());
        }
        let generated = !in_stock && auto && origin == Origin::Generator;
        let id = format!("{}:{res}", if in_stock { "stock" } else if generated { "generate" } else { "gather" });
        let kind = if in_stock {
            NodeKind::Stock { res: res.clone(), per_minute: v }
        } else if generated {
            let load = (!res.contains("fluid:")).then(|| machines::size_generator(res, req.rpm, v, &req.config));
            if let Some(l) = &load {
                total_su += l.su;
                *machine_counts.entry(machine_key(l.machine)).or_insert(0u32) += l.count;
            }
            NodeKind::Generate { res: res.clone(), per_minute: v, load }
        } else {
            NodeKind::Gather { res: res.clone(), per_minute: v, origin }
        };
        nodes.push(Node { id: id.clone(), kind });
        suppliers.entry(res.clone()).or_default().push((id, v));
    }
    for (res, &v) in &solved.surplus {
        if v <= EPS {
            continue;
        }
        let id = format!("surplus:{res}");
        nodes.push(Node { id: id.clone(), kind: NodeKind::Surplus { res: res.clone(), per_minute: v } });
        consumers.entry(res.clone()).or_default().push((id, v));
    }
    // The same item asked for twice is one end of the graph.
    let mut wanted: BTreeMap<&ResId, f64> = BTreeMap::new();
    for t in &req.targets {
        *wanted.entry(&t.item).or_default() += t.per_minute;
    }
    for (item, per_minute) in wanted {
        let id = format!("target:{item}");
        nodes.push(Node { id: id.clone(), kind: NodeKind::Target { res: item.clone(), per_minute } });
        consumers.entry(item.clone()).or_default().push((id, per_minute));
    }
    for ((tag, member), &v) in &solved.conversions {
        if v <= EPS {
            continue;
        }
        let id = format!("conv:{tag}:{member}");
        consumers.entry(member.clone()).or_default().push((id.clone(), v));
        suppliers.entry(tag.clone()).or_default().push((id, v));
    }

    // Split each resource's supply among its consumers in proportion.
    let mut raw_edges = Vec::new();
    for (res, list) in &consumers {
        let supply = suppliers.get(res).map(Vec::as_slice).unwrap_or(&[]);
        let total: f64 = supply.iter().map(|s| s.1).sum();
        for (to, need) in list {
            for (from, have) in supply {
                let flow = need * have / total;
                if flow > EPS {
                    raw_edges.push(Edge { from: from.clone(), to: to.clone(), res: res.clone(), per_minute: flow, transport: None });
                }
            }
        }
    }
    let mut edges = collapse_conversions(raw_edges);
    name_recipe_nodes(cat, req, &mut nodes, &edges);
    add_transport(cat, req, &nodes, &mut edges);
    let build = build_list(cat, &nodes, &edges);

    let heat = req.include_heat.then(|| HeatStats {
        burners_heated: heated,
        burners_superheated: superheated,
        coal_per_min: heated as f64 * 1200.0 / COAL_BURN_TICKS,
        cake_per_min: superheated as f64 * 1200.0 / CAKE_BURN_TICKS,
    });
    Ok(Plan { nodes, edges, total_su, machines: machine_counts, heat, warnings, manual, build, branches: vec![] })
}

/// Works out what each recipe is there for, and which other recipes could do it.
fn name_recipe_nodes(cat: &Catalog, req: &PlanRequest, nodes: &mut [Node], edges: &[Edge]) {
    for node in nodes {
        let NodeKind::Recipe { recipe, produces, alternatives, .. } = &mut node.kind else { continue };
        let out: Vec<&Edge> = edges.iter().filter(|e| e.from == node.id).collect();
        let goal = out.iter().find(|e| e.to.starts_with("target:"));
        let used = out.iter().filter(|e| !e.to.starts_with("surplus:")).max_by(|a, b| a.per_minute.total_cmp(&b.per_minute));
        if let Some(e) = goal.or(used) {
            *produces = e.res.clone();
        }
        *alternatives = alternatives_for(cat, req, recipe, produces);
    }
}

fn alternatives_for(cat: &Catalog, req: &PlanRequest, recipe: &str, produces: &str) -> Vec<String> {
    let disabled: HashSet<&str> = req.disabled_recipes.iter().map(String::as_str).collect();
    producers(cat, produces, &req.config)
        .into_iter()
        .filter(|r| r.id != recipe && !disabled.contains(r.id.as_str()))
        .map(|r| r.id.clone())
        .collect()
}

/// Items for one machine of a recipe. Basin work needs a basin under the machine.
fn machine_items(machine: Machine, kind: RecipeKind, cells: u32) -> Vec<(&'static str, u32)> {
    let basin = ("create:basin", 1);
    match machine {
        Machine::Press if kind == RecipeKind::Pressing => vec![("create:mechanical_press", 1)],
        Machine::Press => vec![("create:mechanical_press", 1), basin],
        Machine::Mixer => vec![("create:mechanical_mixer", 1), basin],
        Machine::Millstone => vec![("create:millstone", 1)],
        Machine::CrushingWheels => vec![("create:crushing_wheel", 2)],
        Machine::Saw => vec![("create:mechanical_saw", 1)],
        Machine::Deployer => vec![("create:deployer", 1)],
        Machine::Spout => vec![("create:spout", 1)],
        Machine::ItemDrain => vec![("create:item_drain", 1)],
        Machine::MechanicalCrafter => vec![("create:mechanical_crafter", cells.max(1))],
        Machine::Fan => vec![("create:encased_fan", 1)],
        Machine::Drill => vec![("create:mechanical_drill", 1)],
        Machine::Furnace => vec![("minecraft:furnace", 1)],
        Machine::BlastFurnace => vec![("minecraft:blast_furnace", 1)],
        Machine::Smoker => vec![("minecraft:smoker", 1)],
        Machine::Hand => vec![],
    }
}

/// Totals what has to be built. Belts are counted as separate lines, their length is up to the layout.
fn build_list(cat: &Catalog, nodes: &[Node], edges: &[Edge]) -> Vec<BuildItem> {
    let mut items: BTreeMap<&str, u32> = BTreeMap::new();
    for node in nodes {
        match &node.kind {
            NodeKind::Recipe { recipe, loads, burners, .. } => {
                let Some(r) = cat.recipe(recipe) else { continue };
                for (i, load) in loads.iter().enumerate() {
                    let kind = r.steps.get(i).map_or(r.kind, |s| s.kind);
                    for (item, each) in machine_items(load.machine, kind, r.cells) {
                        *items.entry(item).or_default() += each * load.count;
                    }
                }
                if *burners > 0 {
                    *items.entry("create:blaze_burner").or_default() += burners;
                }
            }
            NodeKind::Generate { load: Some(load), .. } => {
                *items.entry("create:mechanical_drill").or_default() += load.count;
            }
            _ => {}
        }
    }
    for e in edges.iter().filter(|e| !e.to.starts_with("surplus:")) {
        let Some(t) = &e.transport else { continue };
        *items.entry("create:belt_connector").or_default() += t.belts;
        *items.entry("create:andesite_funnel").or_default() += t.funnels;
        if t.needs_buffer {
            *items.entry("minecraft:chest").or_default() += 1;
        }
    }
    items.into_iter().map(|(item, count)| BuildItem { item: item.to_string(), count }).collect()
}

fn add_transport(cat: &Catalog, req: &PlanRequest, nodes: &[Node], edges: &mut [Edge]) {
    let by_id: HashMap<&str, &NodeKind> = nodes.iter().map(|n| (n.id.as_str(), &n.kind)).collect();
    let belt_rpm = req.belt_rpm.unwrap_or(req.rpm);
    for e in edges {
        if e.res.contains("fluid:") {
            continue;
        }
        // A machine working in batches sets the stack it receives.
        let wanted = match by_id.get(e.to.as_str()) {
            Some(NodeKind::Recipe { loads, .. }) => loads.first().map(|l| l.items_per_op).filter(|n| *n > 1),
            _ => None,
        };
        let stack = req.stack_sizes.get(&e.res).copied().or(wanted).unwrap_or(req.stack);
        // What the source hands over at once.
        let source_yield = match by_id.get(e.from.as_str()) {
            Some(NodeKind::Recipe { recipe, loads, .. }) => {
                let per_run: f64 = cat.recipe(recipe).map_or(1.0, |r| r.outputs.iter().filter(|o| o.res == e.res).map(|o| o.amount).sum());
                per_run * loads.first().map_or(1, |l| l.items_per_op) as f64
            }
            Some(NodeKind::Generate { .. }) => 1.0,
            _ => f64::INFINITY,
        };
        e.transport = Some(machines::transport(e.per_minute, stack, belt_rpm, source_yield, &req.config));
    }
}

/// Removes tag conversion nodes, joining flows so edges name the real item.
fn collapse_conversions(raw: Vec<Edge>) -> Vec<Edge> {
    let (conv, plain): (Vec<Edge>, Vec<Edge>) = raw.into_iter().partition(|e| e.from.starts_with("conv:") || e.to.starts_with("conv:"));
    let mut joined: BTreeMap<(String, String, ResId), f64> = BTreeMap::new();
    for e in plain {
        *joined.entry((e.from, e.to, e.res)).or_default() += e.per_minute;
    }
    let mut outgoing: BTreeMap<&str, Vec<&Edge>> = BTreeMap::new();
    let mut outgoing_total: BTreeMap<&str, f64> = BTreeMap::new();
    for e in conv.iter().filter(|e| e.from.starts_with("conv:")) {
        outgoing.entry(&e.from).or_default().push(e);
        *outgoing_total.entry(&e.from).or_default() += e.per_minute;
    }
    for e in conv.iter().filter(|e| e.to.starts_with("conv:")) {
        let total = outgoing_total.get(e.to.as_str()).copied().unwrap_or(0.0);
        for out in outgoing.get(e.to.as_str()).into_iter().flatten() {
            *joined.entry((e.from.clone(), out.to.clone(), e.res.clone())).or_default() += e.per_minute * out.per_minute / total;
        }
    }
    joined
        .into_iter()
        .filter(|((from, to, _), flow)| from != to && *flow > EPS)
        .map(|((from, to, res), per_minute)| Edge { from, to, res, per_minute, transport: None })
        .collect()
}
