//! Names read from a language file.

use createcalc_core::import::add_names;
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
