//! Sanity checks on real recipes. The bundle is generated locally and never committed,
//! so these skip themselves when `data/bundle.json` is missing. See docs/import.md.

use createcalc_core::model::{Bundle, Catalog};
use createcalc_core::plan::{plan, NodeKind, Plan, PlanRequest, Target};

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
