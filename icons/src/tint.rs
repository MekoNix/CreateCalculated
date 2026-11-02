//! Colours the game applies from code rather than from files, as far as icons need them:
//! the same values its item colour handlers use when no world is around.

fn rgb(hex: u32) -> [f64; 3] {
    [(hex >> 16 & 255) as f64 / 255.0, (hex >> 8 & 255) as f64 / 255.0, (hex & 255) as f64 / 255.0]
}

const WHITE: [f64; 3] = [1.0, 1.0, 1.0];
const LEATHER: u32 = 0xA06540;
const POTION_DEFAULT: u32 = 0x385DC6;
const GRASS: u32 = 0x91BD59;
const FOLIAGE: u32 = 0x48B518;
pub const WATER: u32 = 0x3F76E4;

/// `DyeColor.getTextureDiffuseColor`.
const DYES: [(&str, u32); 16] = [
    ("white", 0xF9FFFE),
    ("orange", 0xF9801D),
    ("magenta", 0xC74EBD),
    ("light_blue", 0x3AB3DA),
    ("yellow", 0xFED83D),
    ("lime", 0x80C71F),
    ("pink", 0xF38BAA),
    ("gray", 0x474F52),
    ("light_gray", 0x9D9D97),
    ("cyan", 0x169C9C),
    ("purple", 0x8932B8),
    ("blue", 0x3C44AA),
    ("brown", 0x835432),
    ("green", 0x5E7C16),
    ("red", 0xB02E26),
    ("black", 0x1D1D21),
];

fn foliage(name: &str) -> Option<u32> {
    match name {
        "spruce_leaves" => Some(0x619961),
        "birch_leaves" => Some(0x80A755),
        "mangrove_leaves" => Some(0x92C648),
        "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves" | "vine" => Some(FOLIAGE),
        _ => None,
    }
}

/// Colour for one tint index of an item. Spawn eggs are not listed: each has its own pair
/// of colours in game code, and they stay grey.
pub fn for_item(id: &str, index: i32) -> [f64; 3] {
    if index < 0 {
        return WHITE;
    }
    let (namespace, name) = id.split_once(':').unwrap_or(("minecraft", id));
    if namespace != "minecraft" {
        return WHITE;
    }
    let color = match (name, index) {
        ("grass_block" | "short_grass" | "tall_grass" | "fern" | "large_fern", 0) => Some(GRASS),
        ("lily_pad", 0) => Some(0x71C35C),
        ("potion" | "splash_potion" | "lingering_potion" | "tipped_arrow", 0) => Some(POTION_DEFAULT),
        ("leather_helmet" | "leather_chestplate" | "leather_leggings" | "leather_boots" | "leather_horse_armor", 0) => Some(LEATHER),
        ("firework_star", 1) => Some(0x8A8A8A),
        ("filled_map", 1) => Some(0x46402E),
        (_, 0) => foliage(name).or_else(|| name.strip_suffix("_banner").and_then(dye)),
        _ => None,
    };
    color.map_or(WHITE, rgb)
}

pub fn dye(name: &str) -> Option<u32> {
    DYES.iter().find(|(dye, _)| *dye == name).map(|(_, color)| *color)
}

/// `fluid:create:potion[minecraft:strength]` carries its potion in the brackets.
/// Colours are from `MobEffects`, long and strong variants share the base colour.
pub fn for_fluid(name: &str, variant: Option<&str>) -> [f64; 3] {
    if name == "water" {
        return rgb(WATER);
    }
    let Some(variant) = variant else {
        return if name == "potion" { rgb(POTION_DEFAULT) } else { WHITE };
    };
    let potion = variant.rsplit(':').next().unwrap_or(variant);
    let potion = potion.strip_prefix("long_").or_else(|| potion.strip_prefix("strong_")).unwrap_or(potion);
    rgb(match potion {
        "swiftness" => 0x33EBFF,
        "slowness" => 0x8BAFE0,
        "strength" => 0xFFC700,
        "healing" => 0xF82423,
        "harming" => 0xA9656A,
        "leaping" => 0xFDFF84,
        "regeneration" => 0xCD5CAB,
        "fire_resistance" => 0xFF9900,
        "water_breathing" => 0x98DAC0,
        "invisibility" => 0xF6F6F6,
        "night_vision" => 0xC2FF66,
        "weakness" => 0x484D48,
        "poison" => 0x87A363,
        "luck" => 0x59C106,
        "slow_falling" => 0xF3CFB9,
        "turtle_master" => 0x8E8E8E,
        "wind_charged" => 0xBDC9FF,
        "weaving" => 0x78695A,
        "oozing" => 0x99FFA3,
        "infested" => 0x8C9B8C,
        _ => POTION_DEFAULT,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tints_apply_only_to_their_layer() {
        assert_eq!(for_item("minecraft:leather_helmet", 0), rgb(LEATHER));
        assert_eq!(for_item("minecraft:leather_helmet", 1), WHITE);
        assert_eq!(for_item("minecraft:potion", 0), rgb(POTION_DEFAULT));
        assert_eq!(for_item("minecraft:potion", 1), WHITE);
        assert_eq!(for_item("create:leather_helmet", 0), WHITE);
        assert_eq!(for_item("minecraft:stone", -1), WHITE);
    }

    #[test]
    fn fluids_take_their_potion_colour() {
        assert_eq!(for_fluid("potion", Some("minecraft:long_strength")), rgb(0xFFC700));
        assert_eq!(for_fluid("water", None), rgb(WATER));
        assert_eq!(for_fluid("lava", None), WHITE);
    }
}
