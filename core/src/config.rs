//! Create server config. Defaults mirror Create 6.0.10 (`CKinetics`, `CRecipes`, `AllBlocks`);
//! a user can override them or supply their own `create-server.toml`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub max_rotation_speed: f64,
    pub fan_processing_time: u32,
    pub fan_rotation_argmax: f64,
    pub fan_push_distance: f64,
    pub windmill_sails_per_rpm: u32,
    pub bulk_pressing: bool,
    pub bulk_cutting: bool,
    pub allow_shapeless_in_mixer: bool,
    pub allow_shaped_square_in_press: bool,
    pub allow_regular_crafting_in_crafter: bool,
    pub allow_stonecutting_on_saw: bool,
    /// SU per RPM consumed, by block id without namespace.
    pub impact: BTreeMap<String, f64>,
    /// SU per RPM provided, by block id without namespace.
    pub capacity: BTreeMap<String, f64>,
}

impl Default for Config {
    fn default() -> Self {
        let map = |pairs: &[(&str, f64)]| pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        Self {
            max_rotation_speed: 256.0,
            fan_processing_time: 150,
            fan_rotation_argmax: 256.0,
            fan_push_distance: 20.0,
            windmill_sails_per_rpm: 8,
            bulk_pressing: false,
            bulk_cutting: false,
            allow_shapeless_in_mixer: true,
            allow_shaped_square_in_press: true,
            allow_regular_crafting_in_crafter: true,
            allow_stonecutting_on_saw: true,
            impact: map(&[
                ("mechanical_press", 8.0),
                ("crushing_wheel", 8.0),
                ("mechanical_mixer", 4.0),
                ("millstone", 4.0),
                ("mechanical_saw", 4.0),
                ("deployer", 4.0),
                ("mechanical_crafter", 2.0),
                ("encased_fan", 2.0),
            ]),
            capacity: map(&[
                ("water_wheel", 32.0),
                ("large_water_wheel", 128.0),
                ("windmill_bearing", 512.0),
                ("steam_engine", 1024.0),
            ]),
        }
    }
}

impl Config {
    pub fn impact_of(&self, block: &str) -> f64 {
        self.impact.get(block).copied().unwrap_or(0.0)
    }

    pub fn capacity_of(&self, block: &str) -> f64 {
        self.capacity.get(block).copied().unwrap_or(0.0)
    }

    /// Applies a `create-server.toml` on top of this config. Unknown keys are ignored.
    pub fn apply_server_toml(&mut self, text: &str) -> Result<(), String> {
        let root: toml::Value = text.parse().map_err(|e: toml::de::Error| e.message().to_string())?;
        self.walk(&root, "");
        Ok(())
    }

    fn walk(&mut self, value: &toml::Value, section: &str) {
        let Some(table) = value.as_table() else { return };
        for (key, v) in table {
            if v.is_table() {
                self.walk(v, key);
                continue;
            }
            let num = v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
            match (section, key.as_str(), num, v.as_bool()) {
                ("impact", block, Some(n), _) => {
                    self.impact.insert(block.to_string(), n);
                }
                ("capacity", block, Some(n), _) => {
                    self.capacity.insert(block.to_string(), n);
                }
                (_, "maxRotationSpeed", Some(n), _) => self.max_rotation_speed = n,
                (_, "fanProcessingTime", Some(n), _) => self.fan_processing_time = n as u32,
                (_, "fanRotationArgmax", Some(n), _) => self.fan_rotation_argmax = n,
                (_, "fanPushDistance", Some(n), _) => self.fan_push_distance = n,
                (_, "windmillSailsPerRPM", Some(n), _) => self.windmill_sails_per_rpm = n as u32,
                (_, "bulkPressing", _, Some(b)) => self.bulk_pressing = b,
                (_, "bulkCutting", _, Some(b)) => self.bulk_cutting = b,
                (_, "allowShapelessInMixer", _, Some(b)) => self.allow_shapeless_in_mixer = b,
                (_, "allowShapedSquareInPress", _, Some(b)) => self.allow_shaped_square_in_press = b,
                (_, "allowRegularCraftingInCrafter", _, Some(b)) => self.allow_regular_crafting_in_crafter = b,
                (_, "allowStonecuttingOnSaw", _, Some(b)) => self.allow_stonecutting_on_saw = b,
                _ => {}
            }
        }
    }
}
