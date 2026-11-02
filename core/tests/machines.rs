//! Expected values are worked out by hand from the game's tick logic.

use createcalc_core::config::Config;
use createcalc_core::machines::*;
use createcalc_core::model::RecipeKind;

#[test]
fn press_cycle() {
    // speed (int)lerp(0.5, 1, 60) = 30: 4 ticks to 120, 5 to pass 240, 1 to finish.
    assert_eq!(press_ticks(256.0), 10);
    // speed (int)lerp(0.125, 1, 60) = 8: 15 ticks to 120, 16 to pass 240, 1 to finish.
    assert_eq!(press_ticks(64.0), 32);
}

#[test]
fn mixer_cycle() {
    // The head stays down in steady work, so a recipe takes the timer: log2 * 15 + 1.
    assert_eq!(mixer_ticks(256.0, 0), 16);
    // 512 / 128 = 4 and 512 / 93 floors to 5, both give log2 = 2.
    assert_eq!(mixer_ticks(128.0, 0), 31);
    assert_eq!(mixer_ticks(93.0, 0), 31);
    assert_eq!(mixer_ticks(64.0, 0), 46);
}

#[test]
fn millstone_cycle() {
    assert_eq!(millstone_ticks(256.0, 100), 7); // 100 / 16 rounds up to 7
    assert_eq!(millstone_ticks(8.0, 50), 50); // below 16 RPM the speed stays 1
}

#[test]
fn saw_cycle() {
    // speed 10.67: 5 ticks to fall below 5 from 50, then 2 ticks for the 20 eject ticks.
    assert_eq!(saw_ticks(256.0, 50, 1), 7);
}

#[test]
fn crushing_depends_on_stack() {
    // A single item always loses 20 per tick regardless of RPM.
    assert_eq!(crushing_ticks(256.0, 400, 1), 20);
    assert_eq!(crushing_ticks(64.0, 400, 1), 20);
    // 16 items at 256 RPM: 20.48 / 4 = 5.12 per tick until below 20.
    assert_eq!(crushing_ticks(256.0, 400, 16), 76);
    assert_eq!(crushing_ticks(256.0, 400, 64), 113);
}

#[test]
fn deployer_cycle() {
    assert_eq!(deployer_ticks(256.0), 5); // 512 per tick: 2 + 2 + 1
    assert_eq!(deployer_ticks(64.0), 20); // 128 per tick: 8 + 8 + 4
    assert_eq!(deployer_ticks(1.0), 313); // clamped to 8 per tick
}

#[test]
fn crafter_cycle() {
    // speed 250: craft 2000 / 250 + 1 = 9, each hop 1000 / 250 + 1 = 5.
    assert_eq!(crafter_ticks(256.0, 1), 9);
    assert_eq!(crafter_ticks(256.0, 9), 49);
}

#[test]
fn drill_breaks_a_block_in_ten_hits() {
    // 2.56 / 2 truncates to 1 progress per hit, 2 / 2.56 truncates to no wait between hits.
    assert_eq!(drill_ticks(256.0, 2.0), 10);
    // At 64 RPM a hit waits (int)(2 / 0.64) = 3 ticks: 10 hits and 9 waits.
    assert_eq!(drill_ticks(64.0, 2.0), 37);
    let cfg = Config::default();
    // 10 ticks of drilling and 1 of refilling, as measured: 109 blocks a minute per drill.
    let one = size_generator("minecraft:cobblestone", 256.0, 109.0, &cfg);
    assert_eq!(one.count, 1);
    assert_eq!(one.su, 4.0 * 256.0);
    assert_eq!(size_generator("minecraft:cobblestone", 256.0, 110.0, &cfg).count, 2);
}

#[test]
fn fan_time_steps_every_16_items() {
    let cfg = Config::default();
    assert_eq!(fan_ticks(&cfg, 1), 151);
    assert_eq!(fan_ticks(&cfg, 16), 151);
    assert_eq!(fan_ticks(&cfg, 17), 301);
    assert_eq!(fan_range(256.0, &cfg), 20.0);
    assert_eq!(fan_range(128.0, &cfg), 11.5);
}

#[test]
fn stress_counts_every_block() {
    let cfg = Config::default();
    // 60 runs a minute on a press at 256 RPM: 1 per 20 ticks, cycle 10, so half a press.
    let press = size(Machine::Press, RecipeKind::Pressing, 0, 0, 256.0, 1, 60.0, &cfg);
    assert_eq!(press.count, 1);
    assert!((press.exact - 0.5).abs() < 1e-9);
    assert_eq!(press.su, 8.0 * 256.0);
    // Crushing wheels work in pairs.
    let wheels = size(Machine::CrushingWheels, RecipeKind::Crushing, 400, 0, 256.0, 1, 50.0, &cfg);
    assert_eq!(wheels.count, 1);
    assert_eq!(wheels.su, 2.0 * 8.0 * 256.0);
    // A 9-cell grid needs 9 crafters.
    let crafter = size(Machine::MechanicalCrafter, RecipeKind::CraftingShaped, 0, 9, 256.0, 1, 10.0, &cfg);
    assert_eq!(crafter.su, 9.0 * 2.0 * 256.0);
}

#[test]
fn fan_reports_stream_length() {
    let cfg = Config::default();
    // 158 items a minute one by one: 158 / 1200 * 151 = 19.9 blocks, which fits one fan.
    let fan = size(Machine::Fan, RecipeKind::Splashing, 0, 0, 256.0, 1, 158.0, &cfg);
    assert!((fan.stream_blocks.unwrap() - 19.88).abs() < 0.01);
    assert_eq!(fan.count, 1);
    // 160 items a minute no longer fit into a 20 block stream.
    assert_eq!(size(Machine::Fan, RecipeKind::Splashing, 0, 0, 256.0, 1, 160.0, &cfg).count, 2);
    assert!((fan.max_belt_rpm.unwrap() - 63.6).abs() < 0.1);
}

#[test]
fn server_toml_overrides_defaults() {
    let mut cfg = Config::default();
    let toml = r#"
[recipes]
    bulkPressing = true
[kinetics]
    maxRotationSpeed = 1024
    [kinetics.encasedFan]
        fanProcessingTime = 100
    [kinetics.stressValues.v2.impact]
        mechanical_press = 16.0
    [kinetics.stressValues.v2.capacity]
        water_wheel = 64.0
"#;
    cfg.apply_server_toml(toml).unwrap();
    assert!(cfg.bulk_pressing);
    assert_eq!(cfg.max_rotation_speed, 1024.0);
    assert_eq!(cfg.fan_processing_time, 100);
    assert_eq!(cfg.impact_of("mechanical_press"), 16.0);
    assert_eq!(cfg.capacity_of("water_wheel"), 64.0);
    // Untouched values keep their defaults.
    assert_eq!(cfg.impact_of("deployer"), 4.0);
    assert!(cfg.apply_server_toml("not = [valid").is_err());
}
