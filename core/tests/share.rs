//! Share links must read back exactly, stay short, and keep meaning the same forever.

use createcalc_core::config::Config;
use createcalc_core::machines::{Funnel, Machine};
use createcalc_core::model::{Bundle, Catalog, Heat, Output, Recipe, RecipeKind, Stack};
use createcalc_core::plan::{plan, Branch, Mode, NodeOverride, PlanRequest, Target};
use createcalc_core::share::{clashes, decode, encode, fingerprint};

fn recipe(id: &str, input: &str, output: &str) -> Recipe {
    Recipe {
        id: id.into(),
        kind: RecipeKind::Pressing,
        inputs: vec![Stack { res: input.into(), amount: 1.0 }],
        outputs: vec![Output { res: output.into(), amount: 1.0, chance: 1.0 }],
        duration: 0,
        heat: Heat::None,
        cells: 0,
        square: false,
        steps: vec![],
        loops: 0,
    }
}

fn catalog() -> Catalog {
    Catalog::new(Bundle {
        recipes: vec![
            recipe("r:plate", "m:iron", "m:plate"),
            recipe("r:plate2", "m:gold", "m:plate"),
            recipe("r:wire", "m:plate", "m:wire"),
        ],
        tags: Default::default(),
        names: Default::default(),
    })
}

fn one(item: &str, per_minute: f64) -> PlanRequest {
    PlanRequest { targets: vec![Target { item: item.into(), per_minute }], ..Default::default() }
}

/// Every field a scheme can have, away from its default.
fn full() -> PlanRequest {
    let mut config = Config::default();
    config.max_rotation_speed = 512.0;
    config.fan_processing_time = 100;
    config.bulk_pressing = true;
    config.allow_stonecutting_on_saw = false;
    config.impact.insert("mechanical_press".into(), 16.0);
    config.capacity.insert("water_wheel".into(), 64.0);
    PlanRequest {
        targets: vec![Target { item: "m:wire".into(), per_minute: 60.0 }, Target { item: "m:plate".into(), per_minute: 7.5 }],
        rpm: 128.0,
        mode: Mode::FullAuto,
        include_heat: false,
        stock: vec!["m:gold".into()],
        recipe_choice: [("m:plate".to_string(), "r:plate2".to_string())].into(),
        overrides: [("r:wire".to_string(), NodeOverride { rpm: Some(64.5), machine: Some(Machine::Press), batch: Some(16) })].into(),
        belt_rpm: Some(32.0),
        stack_sizes: [("m:wire".to_string(), 16)].into(),
        branches: vec![
            Branch { recipe: "r:wire".into(), input: "m:plate".into(), per_minute: None },
            Branch { recipe: "r:wire".into(), input: "m:plate".into(), per_minute: Some(2.5) },
        ],
        batch: 4,
        stack: 64,
        funnel: Funnel::Brass,
        config,
        ..Default::default()
    }
}

fn json(r: &PlanRequest) -> serde_json::Value {
    serde_json::to_value(r).unwrap()
}

#[test]
fn every_field_reads_back_exactly() {
    let cat = catalog();
    let code = encode(&full(), 128);
    let d = decode(&cat, &code).unwrap();
    assert_eq!(json(&d.request), json(&full()));
    assert_eq!(d.sails, 128);
    assert!(d.has_config);
    assert_eq!(d.missing, 0);
}

#[test]
fn defaults_read_back_as_defaults() {
    let cat = catalog();
    let d = decode(&cat, &encode(&one("m:wire", 60.0), 64)).unwrap();
    assert_eq!(json(&d.request), json(&one("m:wire", 60.0)));
    assert_eq!(d.sails, 64);
    assert!(!d.has_config);
}

#[test]
fn what_a_link_means_never_changes() {
    // Old links are out in the world: if this fails, keep the old reading and add a new format.
    assert_eq!(fingerprint("create:andesite_alloy") & 63, 40);
    assert_eq!(encode(&one("create:andesite_alloy", 60.0), 64), "BABozlic4D");
    let auto = PlanRequest { mode: Mode::FullAuto, rpm: 128.0, ..one("create:andesite_alloy", 60.0) };
    assert_eq!(encode(&auto, 64), "BDBozlic4DgI");
}

#[test]
fn codes_stay_short_and_safe_in_an_address() {
    assert!(encode(&one("create:andesite_alloy", 60.0), 64).len() <= 10);
    let code = encode(&full(), 128);
    assert!(code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "{code}");
}

#[test]
fn rates_keep_three_decimals() {
    let cat = catalog();
    for rate in [0.5, 7.5, 0.125, 1234.0, 0.001] {
        let d = decode(&cat, &encode(&one("m:wire", rate), 64)).unwrap();
        assert_eq!(d.request.targets[0].per_minute, rate);
    }
}

#[test]
fn an_item_gone_from_the_game_is_left_out_and_counted() {
    let cat = catalog();
    let mut request = one("m:wire", 60.0);
    request.targets.push(Target { item: "m:removed".into(), per_minute: 1.0 });
    request.recipe_choice.insert("m:plate".into(), "r:removed".into());
    request.stock.push("m:removed".into());
    let d = decode(&cat, &encode(&request, 64)).unwrap();
    assert_eq!(d.request.targets.len(), 1);
    assert!(d.request.recipe_choice.is_empty() && d.request.stock.is_empty());
    assert_eq!(d.missing, 3);
}

#[test]
fn broken_codes_are_errors_and_never_panic() {
    let cat = catalog();
    for bad in ["", "A", "Z", "B", "BA", "BAB", "BABozli", "B!", "BABozlic4DX", "B_____________", &"B".repeat(9000), "Я"] {
        assert!(decode(&cat, bad).is_err(), "{bad}");
    }
    // Every cut of a real code either fails or reads as something, without a panic.
    let code = encode(&full(), 128);
    for end in 0..code.len() {
        let _ = decode(&cat, &code[..end]);
    }
}

#[test]
fn a_newer_mask_bit_is_refused_rather_than_misread() {
    assert!(decode(&catalog(), "B_____B").is_err());
}

fn real_catalog() -> Option<Catalog> {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../data/bundle.json")).ok()?;
    Some(Catalog::new(serde_json::from_str::<Bundle>(&text).unwrap()))
}

#[test]
fn real_ids_have_no_clashes() {
    let Some(cat) = real_catalog() else { return };
    assert_eq!(clashes(&cat), Vec::<(String, String)>::new());
}

#[test]
fn a_real_scheme_gives_the_same_plan_through_a_link() {
    let Some(cat) = real_catalog() else { return };
    let request = PlanRequest {
        targets: vec![Target { item: "create:andesite_alloy".into(), per_minute: 60.0 }, Target { item: "create:analog_lever".into(), per_minute: 30.0 }],
        mode: Mode::FullAuto,
        rpm: 128.0,
        funnel: Funnel::Brass,
        ..Default::default()
    };
    let code = encode(&request, 64);
    assert!(code.len() <= 25, "{code}");
    let read = decode(&cat, &code).unwrap();
    assert_eq!(read.missing, 0);
    let (a, b) = (plan(&cat, &request).unwrap(), plan(&cat, &read.request).unwrap());
    assert_eq!(serde_json::to_value(&a).unwrap(), serde_json::to_value(&b).unwrap());
}
