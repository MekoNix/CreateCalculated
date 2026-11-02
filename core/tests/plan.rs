//! Planner tests on a small hand-written recipe set.

use createcalc_core::machines::Machine;
use createcalc_core::model::{AssemblyStep, Bundle, Catalog, Heat, Output, Recipe, RecipeKind, Stack};
use createcalc_core::plan::{plan, NodeKind, NodeOverride, Plan, PlanError, PlanRequest, Target};
use createcalc_core::power::plan_power;

fn stack(res: &str, amount: f64) -> Stack {
    Stack { res: res.into(), amount }
}

fn out(res: &str, amount: f64, chance: f64) -> Output {
    Output { res: res.into(), amount, chance }
}

fn recipe(id: &str, kind: RecipeKind, inputs: Vec<Stack>, outputs: Vec<Output>) -> Recipe {
    Recipe { id: id.into(), kind, inputs, outputs, duration: 0, heat: Heat::None, cells: 0, square: false, steps: vec![], loops: 0 }
}

fn catalog() -> Catalog {
    let mut brass = recipe("brass", RecipeKind::Mixing, vec![stack("#ingots/copper", 1.0), stack("zinc", 1.0)], vec![out("brass", 2.0, 1.0)]);
    brass.heat = Heat::Heated;
    let mut crush = recipe("crush_iron", RecipeKind::Crushing, vec![stack("raw_iron", 1.0)], vec![out("crushed_iron", 1.0, 1.0), out("xp", 1.0, 0.75)]);
    crush.duration = 400;
    let mut mechanism = recipe(
        "mechanism",
        RecipeKind::SequencedAssembly,
        vec![stack("sheet", 1.0), stack("cog", 5.0)],
        vec![out("mechanism", 1.0, 0.8), out("scrap", 1.0, 0.2)],
    );
    mechanism.loops = 5;
    mechanism.steps = vec![AssemblyStep { kind: RecipeKind::Deploying, duration: 0 }];
    let mut block = recipe("block", RecipeKind::CraftingShaped, vec![stack("diamond", 9.0)], vec![out("diamond_block", 1.0, 1.0)]);
    block.cells = 9;
    block.square = true;
    let mut unblock = recipe("unblock", RecipeKind::CraftingShapeless, vec![stack("diamond_block", 1.0)], vec![out("diamond", 9.0, 1.0)]);
    unblock.cells = 1;
    let bundle = Bundle {
        recipes: vec![
            brass,
            recipe("smelt_crushed", RecipeKind::Smelting, vec![stack("crushed_iron", 1.0)], vec![out("iron", 1.0, 1.0)]),
            crush,
            recipe("cheap_sheet", RecipeKind::Pressing, vec![stack("iron", 1.0)], vec![out("sheet", 1.0, 1.0)]),
            recipe("dear_sheet", RecipeKind::Pressing, vec![stack("iron", 3.0)], vec![out("sheet", 1.0, 1.0)]),
            recipe("cog", RecipeKind::Deploying, vec![stack("iron", 1.0)], vec![out("cog", 1.0, 1.0)]),
            mechanism,
            block,
            unblock,
        ],
        tags: [("#ingots/copper".to_string(), vec!["copper".to_string(), "tin_copper".to_string()])].into(),
        names: Default::default(),
    };
    Catalog::new(bundle)
}

fn request(item: &str, per_minute: f64) -> PlanRequest {
    PlanRequest { targets: vec![Target { item: item.into(), per_minute }], ..Default::default() }
}

fn gathered(p: &Plan, res: &str) -> f64 {
    p.nodes
        .iter()
        .find_map(|n| match &n.kind {
            NodeKind::Gather { res: r, per_minute, .. } if r == res => Some(*per_minute),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn surplus(p: &Plan, res: &str) -> f64 {
    p.nodes
        .iter()
        .find_map(|n| match &n.kind {
            NodeKind::Surplus { res: r, per_minute } if r == res => Some(*per_minute),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn runs(p: &Plan, recipe_id: &str) -> f64 {
    p.nodes
        .iter()
        .find_map(|n| match &n.kind {
            NodeKind::Recipe { recipe, runs_per_min, .. } if recipe == recipe_id => Some(*runs_per_min),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-4, "{a} != {b}");
}

#[test]
fn items_without_recipe_become_gather_nodes() {
    let p = plan(&catalog(), &request("brass", 60.0)).unwrap();
    close(runs(&p, "brass"), 30.0);
    close(gathered(&p, "zinc"), 30.0);
}

#[test]
fn tag_ingredient_is_served_by_a_real_item_in_edges() {
    let p = plan(&catalog(), &request("brass", 60.0)).unwrap();
    let total_copper: f64 = ["copper", "tin_copper"].iter().map(|r| gathered(&p, r)).sum();
    close(total_copper, 30.0);
    assert!(p.edges.iter().all(|e| !e.res.starts_with('#') && !e.from.starts_with("conv:") && !e.to.starts_with("conv:")));
    assert!(p.edges.iter().any(|e| e.to == "recipe:brass" && (e.res == "copper" || e.res == "tin_copper")));
}

#[test]
fn chance_outputs_scale_and_leave_surplus() {
    let p = plan(&catalog(), &request("iron", 60.0)).unwrap();
    close(gathered(&p, "raw_iron"), 60.0);
    close(surplus(&p, "xp"), 45.0);
}

#[test]
fn sequenced_assembly_accounts_for_failures() {
    let p = plan(&catalog(), &request("mechanism", 60.0)).unwrap();
    close(runs(&p, "mechanism"), 75.0);
    close(surplus(&p, "scrap"), 15.0);
    close(runs(&p, "cog"), 375.0);
    let Some(NodeKind::Recipe { loads, .. }) = p.nodes.iter().map(|n| &n.kind).find(|k| matches!(k, NodeKind::Recipe { recipe, .. } if recipe == "mechanism")) else {
        panic!("no node")
    };
    assert_eq!(loads.len(), 1);
    assert_eq!(loads[0].machine, Machine::Deployer);
    // 75 * 5 = 375 ops a minute at 5 ticks each.
    close(loads[0].exact, 375.0 / 1200.0 * 5.0);
}

#[test]
fn cheapest_recipe_wins_and_choice_overrides_it() {
    let cat = catalog();
    let p = plan(&cat, &request("sheet", 60.0)).unwrap();
    close(runs(&p, "cheap_sheet"), 60.0);
    close(runs(&p, "dear_sheet"), 0.0);
    let alternatives = p.nodes.iter().find_map(|n| match &n.kind {
        NodeKind::Recipe { recipe, alternatives, .. } if recipe == "cheap_sheet" => Some(alternatives.clone()),
        _ => None,
    });
    assert_eq!(alternatives.unwrap(), vec!["dear_sheet".to_string()]);

    let mut forced = request("sheet", 60.0);
    forced.recipe_choice.insert("sheet".into(), "dear_sheet".into());
    let p = plan(&cat, &forced).unwrap();
    close(runs(&p, "dear_sheet"), 60.0);
    close(gathered(&p, "raw_iron"), 180.0);
}

#[test]
fn stock_cuts_the_graph() {
    let mut req = request("sheet", 60.0);
    req.stock.push("iron".into());
    let p = plan(&catalog(), &req).unwrap();
    close(runs(&p, "smelt_crushed"), 0.0);
    assert!(p.nodes.iter().any(|n| matches!(&n.kind, NodeKind::Stock { res, per_minute } if res == "iron" && (*per_minute - 60.0).abs() < 1e-4)));
}

#[test]
fn storage_cycle_gathers_the_base_form() {
    // Diamonds only cycle with their block, so they must be gathered, not the block.
    let p = plan(&catalog(), &request("diamond", 9.0)).unwrap();
    close(gathered(&p, "diamond"), 9.0);
    close(gathered(&p, "diamond_block"), 0.0);
}

#[test]
fn heat_statistic_can_be_turned_off() {
    let cat = catalog();
    let p = plan(&cat, &request("brass", 60.0)).unwrap();
    let heat = p.heat.clone().unwrap();
    // 30 runs a minute at 16 ticks: 0.4 of a mixer, so 1 mixer and 1 burner.
    assert_eq!(heat.burners_heated, 1);
    close(heat.coal_per_min, 0.75);
    let mut req = request("brass", 60.0);
    req.include_heat = false;
    assert!(plan(&cat, &req).unwrap().heat.is_none());
}

#[test]
fn per_recipe_rpm_override() {
    let cat = catalog();
    let base = plan(&cat, &request("brass", 60.0)).unwrap();
    let mut req = request("brass", 60.0);
    req.overrides.insert("brass".into(), NodeOverride { rpm: Some(64.0), ..Default::default() });
    let slow = plan(&cat, &req).unwrap();
    // 1 mixer at 256 RPM: 4 * 256 SU. 30 runs at 46 ticks = 1.15, so 2 at 64 RPM.
    close(base.total_su, 1024.0);
    close(slow.total_su, 2.0 * 4.0 * 64.0);
}

#[test]
fn batch_changes_crusher_count() {
    let cat = catalog();
    let mut req = request("iron", 600.0);
    let single = plan(&cat, &req).unwrap();
    req.batch = 64;
    let batched = plan(&cat, &req).unwrap();
    assert_eq!(single.machines["crushing_wheels"], 10); // 60 a minute each
    assert_eq!(batched.machines["crushing_wheels"], 1); // 680 a minute
}

#[test]
fn flows_balance_at_every_recipe() {
    let p = plan(&catalog(), &request("mechanism", 60.0)).unwrap();
    for n in &p.nodes {
        let NodeKind::Recipe { recipe, runs_per_min, .. } = &n.kind else { continue };
        let cat = catalog();
        let r = cat.recipe(recipe).unwrap();
        for s in &r.inputs {
            let incoming: f64 = p.edges.iter().filter(|e| e.to == n.id && e.res == s.res).map(|e| e.per_minute).sum();
            close(incoming, s.amount * runs_per_min);
        }
    }
}

#[test]
fn bad_requests_are_rejected() {
    let cat = catalog();
    assert!(matches!(plan(&cat, &PlanRequest::default()), Err(PlanError::Invalid(_))));
    let mut req = request("brass", 60.0);
    req.rpm = 300.0;
    assert!(matches!(plan(&cat, &req), Err(PlanError::Invalid(_))));
    req.rpm = 256.0;
    req.config.max_rotation_speed = 512.0;
    req.rpm = 300.0;
    assert!(plan(&cat, &req).is_ok());
    assert!(matches!(plan(&cat, &request("brass", 0.0)), Err(PlanError::Invalid(_))));
}

#[test]
fn power_sources() {
    let cfg = Default::default();
    let p = plan_power(20000.0, 64, &cfg);
    assert_eq!(p.boilers.levels, 2);
    assert_eq!(p.boilers.engines, 2);
    assert_eq!(p.boilers.tank_blocks, 8);
    assert_eq!(p.boilers.water_mb_per_tick, 20);
    assert_eq!(p.boilers.burners_kindled, 2);
    assert_eq!(p.boilers.burners_seething, 1);
    let count = |id: &str| p.sources.iter().find(|s| s.id == id).unwrap().count;
    assert_eq!(count("water_wheel"), 79); // 256 SU each
    assert_eq!(count("large_water_wheel"), 40); // 512 SU each
    assert_eq!(count("windmill_bearing"), 5); // 64 sails: 8 RPM, 4096 SU
}

#[test]
fn base_resource_beats_a_longer_recipe_unless_a_recipe_is_chosen() {
    let cat = catalog();
    // Iron can be crafted, but on this list it is simply gathered.
    let mut req = request("sheet", 60.0);
    req.base_resources = Some(vec!["iron".into()]);
    let p = plan(&cat, &req).unwrap();
    close(gathered(&p, "iron"), 60.0);
    close(runs(&p, "smelt_crushed"), 0.0);
    assert!(p.warnings.is_empty());
    // Choosing the recipe takes iron off the list.
    req.recipe_choice.insert("iron".into(), "smelt_crushed".into());
    let p = plan(&cat, &req).unwrap();
    close(gathered(&p, "iron"), 0.0);
    close(runs(&p, "smelt_crushed"), 60.0);
    assert!(!p.warnings.is_empty());
}

#[test]
fn grid_recipes_default_to_the_machine_with_least_stress() {
    let cat = catalog();
    let machine_of = |p: &Plan| {
        p.nodes.iter().find_map(|n| match &n.kind {
            NodeKind::Recipe { recipe, loads, .. } if recipe == "block" => Some(loads[0].machine),
            _ => None,
        })
    };
    let mut req = request("diamond_block", 60.0);
    // A full 3x3 of one item fits a press: 8 SU/RPM against 9 crafters at 2 SU/RPM each.
    let p = plan(&cat, &req).unwrap();
    assert_eq!(machine_of(&p), Some(Machine::Press));
    close(p.total_su, 8.0 * 256.0);
    // An explicit choice still wins.
    req.overrides.insert("block".into(), NodeOverride { machine: Some(Machine::MechanicalCrafter), ..Default::default() });
    let p = plan(&cat, &req).unwrap();
    assert_eq!(machine_of(&p), Some(Machine::MechanicalCrafter));
    // With the press disabled in config it falls back to the crafter.
    let mut req = request("diamond_block", 60.0);
    req.config.allow_shaped_square_in_press = false;
    assert_eq!(machine_of(&plan(&cat, &req).unwrap()), Some(Machine::MechanicalCrafter));
}

fn edge<'a>(p: &'a Plan, from: &str, res: &str) -> &'a createcalc_core::plan::Edge {
    p.edges.iter().find(|e| e.from == from && e.res == res).unwrap_or_else(|| panic!("no {from} {res}: {:?}", p.edges))
}

#[test]
fn transport_counts_belts_and_funnels() {
    let mut req = request("sheet", 700.0);
    let p = plan(&catalog(), &req).unwrap();
    let t = edge(&p, "recipe:cheap_sheet", "sheet").transport.clone().unwrap();
    // 256 RPM carries 640 stacks a minute, a funnel moves 150.
    assert_eq!((t.stack, t.belts, t.funnels), (1, 2, 5));
    close(t.min_belt_rpm, 280.0);
    assert!(!t.needs_buffer);

    req.stack_sizes.insert("sheet".into(), 16);
    req.belt_rpm = Some(32.0);
    let p = plan(&catalog(), &req).unwrap();
    let t = edge(&p, "recipe:cheap_sheet", "sheet").transport.clone().unwrap();
    assert_eq!((t.stack, t.belts, t.funnels), (16, 1, 1));
    close(t.min_belt_rpm, 17.5);
    // A press makes one sheet at a time, so stacks of 16 must collect somewhere first.
    assert!(t.needs_buffer);
}

#[test]
fn a_batching_machine_sets_the_stack_it_receives() {
    let mut req = request("iron", 60.0);
    req.batch = 16;
    let p = plan(&catalog(), &req).unwrap();
    assert_eq!(edge(&p, "gather:raw_iron", "raw_iron").transport.as_ref().unwrap().stack, 16);
    assert_eq!(edge(&p, "recipe:crush_iron", "crushed_iron").transport.as_ref().unwrap().stack, 16);
}

#[test]
fn a_node_is_named_after_what_it_is_used_for() {
    let cat = Catalog::new(Bundle {
        recipes: vec![
            recipe("wash", RecipeKind::Splashing, vec![stack("gravel", 1.0)], vec![out("flint", 1.0, 0.25), out("nugget", 1.0, 0.125)]),
            recipe("craft_nugget", RecipeKind::Mixing, vec![stack("ingot", 1.0)], vec![out("nugget", 9.0, 1.0)]),
        ],
        ..Default::default()
    });
    let mut req = request("nugget", 60.0);
    req.base_resources = Some(vec!["gravel".into()]);
    let p = plan(&cat, &req).unwrap();
    let info = |p: &Plan| p.nodes.iter().find_map(|n| match &n.kind {
        NodeKind::Recipe { recipe, produces, alternatives, .. } if recipe == "wash" => Some((produces.clone(), alternatives.clone())),
        _ => None,
    });
    let (produces, alternatives) = info(&p).unwrap();
    assert_eq!(produces, "nugget");
    // Alternatives are for the item in use, not for the recipe's first output.
    assert_eq!(alternatives, ["craft_nugget"]);

    // Forcing a recipe also removes the item as a by-product of others, and the rest stay listed.
    req.recipe_choice.insert("nugget".into(), "craft_nugget".into());
    let p = plan(&cat, &req).unwrap();
    assert!(info(&p).is_none());
    let others = p.nodes.iter().find_map(|n| match &n.kind {
        NodeKind::Recipe { alternatives, .. } => Some(alternatives.clone()),
        _ => None,
    });
    assert_eq!(others.unwrap(), ["wash"]);
}

#[test]
fn producers_list_main_output_first() {
    let cat = catalog();
    let cfg = Default::default();
    let ids: Vec<&str> = createcalc_core::plan::producers(&cat, "sheet", &cfg).iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["cheap_sheet", "dear_sheet"]);
    assert!(createcalc_core::plan::producers(&cat, "zinc", &cfg).is_empty());
}

fn built(p: &Plan, item: &str) -> u32 {
    p.build.iter().find(|b| b.item == item).map_or(0, |b| b.count)
}

#[test]
fn build_list_counts_every_block() {
    let cat = catalog();
    // One mixer with a basin and a burner, because brass needs heat.
    let p = plan(&cat, &request("brass", 60.0)).unwrap();
    assert_eq!(built(&p, "create:mechanical_mixer"), 1);
    assert_eq!(built(&p, "create:basin"), 1);
    assert_eq!(built(&p, "create:blaze_burner"), 1);
    // Crushing wheels come in pairs.
    let p = plan(&cat, &request("crushed_iron", 600.0)).unwrap();
    assert_eq!(built(&p, "create:crushing_wheel"), 2 * p.machines["crushing_wheels"]);
    // A press needs a basin only when compacting: pressing on a belt does not.
    let p = plan(&cat, &request("sheet", 60.0)).unwrap();
    assert_eq!(built(&p, "create:mechanical_press"), 1);
    assert_eq!(built(&p, "create:basin"), 0);
    // A 3x3 grid on crafters takes nine of them; pressed instead, it takes a press and a basin.
    let mut req = request("diamond_block", 60.0);
    let p = plan(&cat, &req).unwrap();
    assert_eq!((built(&p, "create:mechanical_press"), built(&p, "create:basin")), (1, 1));
    req.overrides.insert("block".into(), NodeOverride { machine: Some(Machine::MechanicalCrafter), ..Default::default() });
    let p = plan(&cat, &req).unwrap();
    assert_eq!(built(&p, "create:mechanical_crafter"), 9 * p.machines["mechanical_crafter"]);
}

#[test]
fn build_list_includes_transport_and_buffers() {
    let cat = catalog();
    let mut req = request("sheet", 700.0);
    let p = plan(&cat, &req).unwrap();
    let total = |f: fn(&createcalc_core::machines::Transport) -> u32, p: &Plan| -> u32 {
        // Leftovers are not transported anywhere planned.
        p.edges.iter().filter(|e| !e.to.starts_with("surplus:")).filter_map(|e| e.transport.as_ref()).map(f).sum()
    };
    assert_eq!(built(&p, "create:belt_connector"), total(|t| t.belts, &p));
    assert_eq!(built(&p, "create:andesite_funnel"), total(|t| t.funnels, &p));
    assert_eq!(built(&p, "minecraft:chest"), 0);
    req.stack_sizes.insert("sheet".into(), 16);
    assert_eq!(built(&plan(&cat, &req).unwrap(), "minecraft:chest"), 1);
}
