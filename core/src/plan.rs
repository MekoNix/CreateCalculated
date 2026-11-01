//! Turns a request into a production graph by solving a linear program.
//!
//! Variables are recipe runs per minute, raw sources and surplus for each resource.
//! Every resource balances: produced + sourced = consumed + surplus + demanded.
//! The objective minimises raw material cost, with a tiny cost per run to avoid pointless loops.

use crate::config::Config;
use crate::machines::{self, Load, Machine};
use crate::model::{Catalog, Heat, Recipe, ResId};
use crate::power::{CAKE_BURN_TICKS, COAL_BURN_TICKS};
use good_lp::{constraint, variable, variables, Expression, Solution, SolverModel, Variable};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

const RUN_COST: f64 = 0.01;
const SURPLUS_COST: f64 = 0.02;
/// Cost of a recipeless item that is not on the base resource list.
const UNUSUAL_RAW_COST: f64 = 1000.0;
const STRIPPED_RAW_COST: f64 = 1.5;
const FLUID_COST_PER_MB: f64 = 1e-3;
/// Editable list of what players normally gather.
const BASE_RESOURCES: &str = include_str!("../data/base_resources.txt");
const EPS: f64 = 1e-6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub item: ResId,
    pub per_minute: f64,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    /// A recipe running on machines.
    Recipe {
        recipe: String,
        runs_per_min: f64,
        /// One entry, or one per step for sequenced assembly.
        loads: Vec<Load>,
        heat: Heat,
        burners: u32,
        /// Other recipes that make the same main output.
        alternatives: Vec<String>,
        machine_options: Vec<Machine>,
    },
    /// No way to produce it: gather by hand.
    Gather { res: ResId, per_minute: f64 },
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

/// Resources that may be gathered at normal cost. Choosing a recipe for one takes it off the list.
fn base_resources(cat: &Catalog, req: &PlanRequest) -> HashSet<ResId> {
    let builtin = || BASE_RESOURCES.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("# ")).map(String::from).collect::<Vec<_>>();
    let entries = req.base_resources.clone().unwrap_or_else(builtin);
    let mut out = HashSet::new();
    for entry in entries {
        if entry.starts_with('#') {
            out.extend(cat.tag_members(&entry).iter().cloned());
        } else {
            out.insert(entry);
        }
    }
    out.retain(|r| !req.recipe_choice.contains_key(r));
    out
}

/// Recipes the request allows, by index in the catalog.
fn allowed(cat: &Catalog, req: &PlanRequest) -> HashSet<usize> {
    let disabled: HashSet<&str> = req.disabled_recipes.iter().map(String::as_str).collect();
    cat.recipes()
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            if disabled.contains(r.id.as_str()) {
                return false;
            }
            if r.kind != crate::model::RecipeKind::SequencedAssembly && machines::options(r, &req.config).is_empty() {
                return false;
            }
            // A forced recipe rules out others whose main output is that item.
            !r.main_output().and_then(|m| req.recipe_choice.get(m)).is_some_and(|chosen| chosen != &r.id)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Walks back from the targets to find the resources and recipes involved.
fn reach(cat: &Catalog, req: &PlanRequest, allowed: &HashSet<usize>, base: &HashSet<ResId>) -> (BTreeSet<ResId>, BTreeSet<usize>) {
    let stock: HashSet<&str> = req.stock.iter().map(String::as_str).collect();
    let mut resources = BTreeSet::new();
    let mut recipes = BTreeSet::new();
    let mut queue: VecDeque<ResId> = req.targets.iter().map(|t| t.item.clone()).collect();
    while let Some(res) = queue.pop_front() {
        // Nothing is planned above stock or a gathered resource.
        if !resources.insert(res.clone()) || stock.contains(res.as_str()) || base.contains(&res) {
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
fn derivable(cat: &Catalog, resources: &BTreeSet<ResId>, recipes: &BTreeSet<usize>, stock: &HashSet<&str>, base: &HashSet<ResId>) -> HashSet<ResId> {
    let produced: HashSet<&ResId> = recipes.iter().flat_map(|&i| cat.recipes()[i].outputs.iter().map(|o| &o.res)).collect();
    let mut known: HashSet<ResId> = resources
        .iter()
        .filter(|r| stock.contains(r.as_str()) || base.contains(*r) || (!r.starts_with('#') && !produced.contains(r)))
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
fn raw_costs(cat: &Catalog, recipes: &BTreeSet<usize>, resources: &BTreeSet<ResId>, base: &HashSet<ResId>) -> HashMap<ResId, f64> {
    let mut cost: HashMap<ResId, f64> = resources
        .iter()
        .map(|r| {
            let unit = if r.contains("fluid:") { FLUID_COST_PER_MB } else { 1.0 };
            // Stripping is extra work, so plain logs win when a recipe accepts both.
            let listed = if r.contains(":stripped_") { STRIPPED_RAW_COST } else { 1.0 };
            (r.clone(), if base.contains(r) { unit * listed } else { unit * UNUSUAL_RAW_COST })
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
            cost.insert(output.res.clone(), part * input.amount / output.amount);
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

pub fn plan(cat: &Catalog, req: &PlanRequest) -> Result<Plan, PlanError> {
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
    let base = base_resources(cat, req);
    let (resources, recipes) = reach(cat, req, &allowed, &base);
    let known = derivable(cat, &resources, &recipes, &stock, &base);
    let costs = raw_costs(cat, &recipes, &resources, &base);

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
        } else if stock.contains(res.as_str()) || base.contains(res) || !produced.contains(res) || !known.contains(res) {
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

    for &v in surplus.values() {
        objective += SURPLUS_COST * v;
    }

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
    }, &allowed, &stock, &base)
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
    let machine = over.and_then(|o| o.machine).filter(|m| options.contains(m)).or(options.first().copied()).unwrap_or(Machine::Hand);
    (vec![machines::size(machine, r.kind, r.duration, r.cells, rpm, batch, runs_per_min, cfg)], options)
}

fn machine_key(m: Machine) -> String {
    serde_json::to_value(m).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

fn build_plan(cat: &Catalog, req: &PlanRequest, solved: Solved, allowed: &HashSet<usize>, stock: &HashSet<&str>, base: &HashSet<ResId>) -> Result<Plan, PlanError> {
    let mut nodes = Vec::new();
    let mut suppliers: BTreeMap<ResId, Vec<(String, f64)>> = BTreeMap::new();
    let mut consumers: BTreeMap<ResId, Vec<(String, f64)>> = BTreeMap::new();
    let (mut total_su, mut machine_counts) = (0.0, BTreeMap::new());
    let (mut heated, mut superheated) = (0, 0);
    let mut warnings = Vec::new();

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
        let alternatives = r
            .main_output()
            .map(|m| {
                cat.producer_indexes(m)
                    .iter()
                    .filter(|&&j| j != i && allowed.contains(&j) && cat.recipes()[j].main_output() == Some(m))
                    .map(|&j| cat.recipes()[j].id.clone())
                    .collect()
            })
            .unwrap_or_default();
        nodes.push(Node {
            id: id.clone(),
            kind: NodeKind::Recipe { recipe: r.id.clone(), runs_per_min: x, loads, heat: r.heat, burners, alternatives, machine_options },
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
        if !in_stock && !base.contains(res) {
            warnings.push(format!("{res}: not a usual resource and no way to craft it was found"));
        }
        let id = format!("{}:{res}", if in_stock { "stock" } else { "gather" });
        let kind = if in_stock { NodeKind::Stock { res: res.clone(), per_minute: v } } else { NodeKind::Gather { res: res.clone(), per_minute: v } };
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
    for t in &req.targets {
        let id = format!("target:{}", t.item);
        nodes.push(Node { id: id.clone(), kind: NodeKind::Target { res: t.item.clone(), per_minute: t.per_minute } });
        consumers.entry(t.item.clone()).or_default().push((id, t.per_minute));
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
                    raw_edges.push(Edge { from: from.clone(), to: to.clone(), res: res.clone(), per_minute: flow });
                }
            }
        }
    }
    let edges = collapse_conversions(raw_edges);

    let heat = req.include_heat.then(|| HeatStats {
        burners_heated: heated,
        burners_superheated: superheated,
        coal_per_min: heated as f64 * 1200.0 / COAL_BURN_TICKS,
        cake_per_min: superheated as f64 * 1200.0 / CAKE_BURN_TICKS,
    });
    Ok(Plan { nodes, edges, total_su, machines: machine_counts, heat, warnings })
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
        .map(|((from, to, res), per_minute)| Edge { from, to, res, per_minute })
        .collect()
}
