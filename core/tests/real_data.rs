//! Sanity checks on real recipes. The bundle is generated locally and never committed,
//! so these skip themselves when `data/bundle.json` is missing. See docs/import.md.

use createcalc_core::model::{Bundle, Catalog};
use createcalc_core::plan::{plan, Mode, NodeKind, Plan, PlanRequest, Target};

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
