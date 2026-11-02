//! Sanity checks on real recipes. The bundle is generated locally and never committed,
//! so these skip themselves when `data/bundle.json` is missing.

use createcalc_core::model::{Bundle, Catalog};
use createcalc_core::plan::{plan, Branch, Mode, NodeKind, Plan, PlanRequest, Target};

fn catalog() -> Option<Catalog> {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../data/bundle.json")).ok()?;
    let bundle: Bundle = serde_json::from_str(&text).unwrap();
    Some(Catalog::new(bundle))
}

fn solve(cat: &Catalog, item: &str) -> Plan {
    plan(cat, &PlanRequest { targets: vec![Target { item: item.into(), per_minute: 60.0 }], ..Default::default() }).unwrap()
}

fn gathered(p: &Plan) -> Vec<&str> {
    p.nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Gather { res, .. } => Some(res.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn common_items_use_sensible_sources() {
    let Some(cat) = catalog() else { return };
    assert_eq!(gathered(&solve(&cat, "minecraft:iron_ingot")), ["minecraft:raw_iron"]);
    let brass = gathered(&solve(&cat, "create:brass_ingot")).join(" ");
    assert!(brass.contains("raw_copper") && brass.contains("raw_zinc"), "{brass}");
    let alloy = gathered(&solve(&cat, "create:andesite_alloy")).join(" ");
    assert!(alloy.contains("minecraft:andesite"), "{alloy}");
    assert!(!alloy.contains("redstone_ore"), "{alloy}");
}

#[test]
fn iron_does_not_come_from_iron_blocks() {
    let Some(cat) = catalog() else { return };
    let p = solve(&cat, "minecraft:iron_ingot");
    assert!(!p.nodes.iter().any(|n| n.id.contains("iron_block")), "{:?}", p.nodes);
    assert!(!p.edges.iter().any(|e| e.res == "minecraft:iron_block"));
    // Nor by way of a raw block crafted only to be broken up again.
    let tube = solve(&cat, "create:electron_tube");
    assert!(!tube.nodes.iter().any(|n| n.id.contains("raw_iron_block")), "{:?}", tube.nodes);
}

#[test]
fn every_target_has_supply_without_unusual_raws() {
    let Some(cat) = catalog() else { return };
    for item in ["create:precision_mechanism", "create:electron_tube", "create:mechanical_crafter", "create:cogwheel"] {
        let p = solve(&cat, item);
        assert!(p.warnings.is_empty(), "{item}: {:?}", p.warnings);
        assert!(p.edges.iter().any(|e| e.to == format!("target:{item}")), "{item} has no incoming edge");
    }
}

fn auto(cat: &Catalog, item: &str) -> Plan {
    plan(cat, &PlanRequest { targets: vec![Target { item: item.into(), per_minute: 60.0 }], mode: Mode::FullAuto, ..Default::default() }).unwrap()
}

#[test]
fn full_auto_builds_iron_and_alloy_from_generators() {
    let Some(cat) = catalog() else { return };
    for item in ["minecraft:iron_ingot", "create:andesite_alloy"] {
        let p = auto(&cat, item);
        assert!(p.manual.is_empty(), "{item}: {:?}", p.manual);
        assert!(p.nodes.iter().any(|n| matches!(&n.kind, NodeKind::Generate { res, .. } if res == "minecraft:cobblestone")), "{item}");
        assert!(gathered(&p).is_empty(), "{item}: {:?}", gathered(&p));
    }
}

#[test]
fn full_auto_names_what_still_needs_mining() {
    let Some(cat) = catalog() else { return };
    let p = auto(&cat, "create:brass_ingot");
    let manual = p.manual.join(" ");
    assert!(manual.contains("zinc") && manual.contains("copper"), "{manual}");
}

#[test]
fn leftovers_are_kept_rather_than_destroyed_in_a_loop() {
    let Some(cat) = catalog() else { return };
    let p = auto(&cat, "minecraft:iron_ingot");
    assert!(!p.nodes.iter().any(|n| n.id.contains("granite")), "{:?}", p.nodes.iter().map(|n| &n.id).collect::<Vec<_>>());
    assert!(p.nodes.iter().any(|n| matches!(&n.kind, NodeKind::Surplus { res, .. } if res == "minecraft:flint")));
}

#[test]
fn full_auto_mines_rather_than_conjures_a_potion() {
    let Some(cat) = catalog() else { return };
    let p = auto(&cat, "minecraft:redstone");
    assert!(p.manual.iter().all(|m| !m.contains("potion")), "{:?}", p.manual);
    assert_eq!(gathered(&p), ["minecraft:redstone"]);
}

#[test]
fn materials_stop_at_ingots() {
    let Some(cat) = catalog() else { return };
    let mut req = PlanRequest { targets: vec![Target { item: "create:mechanical_press".into(), per_minute: 1.0 }], ..Default::default() };
    req.stop_at_materials = true;
    let p = plan(&cat, &req).unwrap();
    let list = gathered(&p).join(" ");
    assert!(list.contains("minecraft:iron_ingot") && list.contains("create:andesite_alloy"), "{list}");
    assert!(!list.contains("raw_"), "{list}");
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}

#[test]
fn arrows_use_up_the_flint_from_washing_gravel() {
    let Some(cat) = catalog() else { return };
    let mut req = PlanRequest { targets: vec![Target { item: "minecraft:iron_ingot".into(), per_minute: 60.0 }], mode: Mode::FullAuto, ..Default::default() };
    req.branches = vec![Branch { recipe: "minecraft:arrow".into(), input: "minecraft:flint".into(), per_minute: None }];
    let p = plan(&cat, &req).unwrap();
    assert!((p.branches[0].per_minute - 4320.0).abs() < 0.01, "{:?}", p.branches);
    assert!(!p.nodes.iter().any(|n| matches!(&n.kind, NodeKind::Surplus { res, .. } if res == "minecraft:flint")));
    // Flint is now the larger flow, but washing is still there for the nuggets.
    let wash = p.nodes.iter().find_map(|n| match &n.kind {
        NodeKind::Recipe { recipe, produces, .. } if recipe == "create:splashing/gravel" => Some(produces.as_str()),
        _ => None,
    });
    assert_eq!(wash, Some("minecraft:iron_nugget"));
}

#[test]
fn several_targets_use_each_others_leftovers() {
    let Some(cat) = catalog() else { return };
    let req = PlanRequest {
        targets: vec![Target { item: "minecraft:iron_ingot".into(), per_minute: 60.0 }, Target { item: "minecraft:arrow".into(), per_minute: 240.0 }],
        mode: Mode::FullAuto,
        ..Default::default()
    };
    let p = plan(&cat, &req).unwrap();
    // Arrows take flint from washing gravel for the iron, so no gravel is milled for them.
    assert!(!p.nodes.iter().any(|n| matches!(&n.kind, NodeKind::Recipe { recipe, .. } if recipe == "create:milling/gravel")));
    assert!(p.edges.iter().any(|e| e.res == "minecraft:flint" && e.to == "recipe:minecraft:arrow" && e.from == "recipe:create:splashing/gravel"));
}

/// Every kind of wood costs the same, so what a plan uses must not depend on the solver's mood.
fn other_species(res: &str) -> bool {
    ["acacia", "bamboo", "birch", "cherry", "crimson", "dark_oak", "jungle", "mangrove", "spruce", "warped"].iter().any(|s| res.contains(s))
}

fn items_used(p: &Plan) -> Vec<&str> {
    p.edges.iter().map(|e| e.res.as_str()).collect()
}

#[test]
fn wood_is_oak_whatever_the_mode() {
    let Some(cat) = catalog() else { return };
    for mode in [Mode::Normal, Mode::FullAuto] {
        for item in ["create:analog_lever", "create:andesite_casing", "create:cart_assembler", "create:white_seat", "minecraft:stick", "minecraft:chest"] {
            let p = plan(&cat, &PlanRequest { targets: vec![Target { item: item.into(), per_minute: 60.0 }], mode, ..Default::default() }).unwrap();
            let used = items_used(&p);
            assert!(!used.iter().any(|r| other_species(r)), "{item}: {used:?}");
        }
    }
    let lever = solve(&cat, "create:analog_lever");
    let raw = gathered(&lever);
    assert_eq!(raw.iter().filter(|r| r.contains("oak")).collect::<Vec<_>>(), [&"minecraft:oak_log"], "{raw:?}");
    assert_eq!(cat.name("minecraft:oak_log", "en"), "Wood");
    assert_eq!(cat.name("minecraft:oak_planks", "ru"), "Доски");
}

#[test]
fn glass_comes_from_plain_sand_and_black_dye_from_ink() {
    let Some(cat) = catalog() else { return };
    for mode in [Mode::Normal, Mode::FullAuto] {
        let glass = plan(&cat, &PlanRequest { targets: vec![Target { item: "minecraft:glass".into(), per_minute: 60.0 }], mode, ..Default::default() }).unwrap();
        assert!(!items_used(&glass).contains(&"minecraft:red_sand"), "{:?}", items_used(&glass));
        let dye = plan(&cat, &PlanRequest { targets: vec![Target { item: "minecraft:black_dye".into(), per_minute: 60.0 }], mode, ..Default::default() }).unwrap();
        assert!(!items_used(&dye).contains(&"minecraft:wither_rose"), "{:?}", items_used(&dye));
    }
}

#[test]
fn stripped_wood_comes_from_a_deployer_with_an_axe() {
    let Some(cat) = catalog() else { return };
    for mode in [Mode::Normal, Mode::FullAuto] {
        let p = plan(&cat, &PlanRequest { targets: vec![Target { item: "minecraft:stripped_oak_log".into(), per_minute: 60.0 }], mode, ..Default::default() }).unwrap();
        assert_eq!(gathered(&p), ["minecraft:oak_log"]);
        let deployer = p.nodes.iter().any(|n| matches!(&n.kind, NodeKind::Recipe { recipe, .. } if recipe == "createcalc:stripping/minecraft/oak_log"));
        assert!(deployer, "{:?}", p.nodes);
        assert!(p.build.iter().any(|b| b.item == "minecraft:iron_axe" && b.count == 1), "{:?}", p.build);
    }
    // Every species and bamboo has its recipe, a stripped log has none.
    for id in ["minecraft/warped_stem", "minecraft/crimson_hyphae", "minecraft/bamboo_block", "minecraft/dark_oak_wood"] {
        assert!(cat.recipe(&format!("createcalc:stripping/{id}")).is_some(), "{id}");
    }
    assert!(cat.recipe("createcalc:stripping/minecraft/stripped_oak_log").is_none());
}

#[test]
fn bark_blocks_are_crafted_rather_than_gathered() {
    let Some(cat) = catalog() else { return };
    assert_eq!(gathered(&solve(&cat, "minecraft:oak_wood")), ["minecraft:oak_log"]);
}

#[test]
fn a_recipe_asking_for_a_species_still_gets_it() {
    let Some(cat) = catalog() else { return };
    assert_eq!(gathered(&solve(&cat, "minecraft:acacia_planks")), ["minecraft:acacia_log"]);
}

#[test]
fn build_materials_use_plain_planks() {
    let Some(cat) = catalog() else { return };
    let targets = vec![Target { item: "create:mechanical_press".into(), per_minute: 1.0 }, Target { item: "minecraft:chest".into(), per_minute: 1.0 }];
    let p = plan(&cat, &PlanRequest { targets, stop_at_materials: true, ..Default::default() }).unwrap();
    let raw = gathered(&p);
    assert!(raw.contains(&"minecraft:oak_planks"), "{raw:?}");
    assert!(!raw.iter().any(|r| other_species(r)), "{raw:?}");
}

#[test]
fn every_default_item_exists_and_every_skin_has_a_default() {
    let Some(cat) = catalog() else { return };
    let text = include_str!("../data/tag_defaults.txt");
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("# ")) {
        let mut fields = line.split('|').map(str::trim);
        let item = fields.next().unwrap();
        assert!(cat.bundle.tags.values().any(|m| m.iter().any(|x| x == item)), "{item} is in no tag");
        if let Some(tag) = fields.nth(2) {
            assert_eq!(cat.tag_default(tag).map(String::as_str), Some(item), "{tag}");
        }
    }
    assert!(cat.is_skin("minecraft:acacia_planks") && cat.is_skin("minecraft:crimson_stem"));
    assert!(!cat.is_skin("minecraft:oak_planks") && !cat.is_skin("minecraft:stone_button") && !cat.is_skin("create:acacia_window"));
}
