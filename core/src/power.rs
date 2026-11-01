//! Power section: how much of each source covers a stress load, and the heat statistic.

use crate::config::Config;
use serde::{Deserialize, Serialize};

/// Regular burner fuel: coal burn time in ticks (`BlazeBurnerBlockEntity`).
pub const COAL_BURN_TICKS: f64 = 1600.0;
/// `superheated_blaze_burner_fuels.json`: blaze cake burn time.
pub const CAKE_BURN_TICKS: f64 = 3200.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Boilers {
    /// Total boiler level. One level gives 16 RPM x steam engine capacity.
    pub levels: u32,
    pub engines: u32,
    pub tank_blocks: u32,
    pub water_mb_per_tick: u32,
    /// Blaze burners on normal fuel (1 level each).
    pub burners_kindled: u32,
    /// Blaze burners on blaze cake (2 levels each).
    pub burners_seething: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub su_each: f64,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerPlan {
    pub su: f64,
    pub boilers: Boilers,
    pub sources: Vec<Source>,
}

fn need(su: f64, each: f64) -> u32 {
    if each <= 0.0 {
        0
    } else {
        (su / each - 1e-9).ceil().max(0.0) as u32
    }
}

/// `sails` is the sail block count of one windmill.
pub fn plan_power(su: f64, sails: u32, cfg: &Config) -> PowerPlan {
    let level_su = 16.0 * cfg.capacity_of("steam_engine");
    let levels = need(su, level_su);
    let boilers = Boilers {
        levels,
        engines: levels,
        tank_blocks: levels * 4,
        water_mb_per_tick: levels * 10,
        burners_kindled: levels,
        burners_seething: levels.div_ceil(2),
    };
    let windmill_rpm = (sails / cfg.windmill_sails_per_rpm.max(1)).clamp(1, 16) as f64;
    let mut source = |id: &str, su_each: f64| Source { id: id.to_string(), su_each, count: need(su, su_each) };
    let sources = vec![
        // WaterWheelBlockEntity: 8 RPM / size, the large wheel has size 2.
        source("water_wheel", cfg.capacity_of("water_wheel") * 8.0),
        source("large_water_wheel", cfg.capacity_of("large_water_wheel") * 4.0),
        source("windmill_bearing", cfg.capacity_of("windmill_bearing") * windmill_rpm),
        source("steam_engine_level", level_su),
    ];
    PowerPlan { su, boilers, sources }
}
