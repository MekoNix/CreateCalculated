//! What the importer adds beyond recipe files: names from a language file and stripping recipes.

use createcalc_core::import::{add_names, add_stripping_recipes};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn a_language_file_adds_to_names_already_there() {
    let mut names = BTreeMap::new();
    add_names(&mut names, &json!({"block.minecraft.cobblestone": "Cobblestone", "advancements.story.root.title": "Minecraft"}), false);
    add_names(&mut names, &json!({"block.minecraft.cobblestone": "Булыжник", "item.minecraft.stick": "Палка"}), true);
    let stone = &names["minecraft:cobblestone"];
    assert_eq!(stone.en.as_deref(), Some("Cobblestone"));
    assert_eq!(stone.ru.as_deref(), Some("Булыжник"));
    assert_eq!(names["minecraft:stick"].en, None);
    assert_eq!(names.len(), 2);
}

#[test]
fn water_and_lava_are_named_through_their_blocks() {
    let mut names = BTreeMap::new();
    add_names(&mut names, &json!({"block.minecraft.lava": "Лава"}), true);
    assert_eq!(names["fluid:minecraft:lava"].ru.as_deref(), Some("Лава"));
}

#[test]
fn logs_with_a_stripped_form_get_a_deployer_recipe() {
    let list = |items: &[&str]| items.iter().map(|i| i.to_string()).collect::<Vec<_>>();
    let tags = BTreeMap::from([
        ("#minecraft:logs".to_string(), list(&["minecraft:oak_log", "minecraft:stripped_oak_log", "mod:strange_log"])),
        ("#minecraft:bamboo_blocks".to_string(), list(&["minecraft:bamboo_block", "minecraft:stripped_bamboo_block"])),
    ]);
    let mut recipes = vec![];
    add_stripping_recipes(&mut recipes, &tags);
    let made: Vec<(&str, &str)> = recipes.iter().map(|r| (r.inputs[0].res.as_str(), r.outputs[0].res.as_str())).collect();
    assert_eq!(made, [("minecraft:bamboo_block", "minecraft:stripped_bamboo_block"), ("minecraft:oak_log", "minecraft:stripped_oak_log")]);
    assert_eq!(recipes[1].id, "createcalc:stripping/minecraft/oak_log");
}
