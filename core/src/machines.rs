//! Machine timing, reproduced tick by tick from Create 6.0.10 because the game
//! truncates to integers, so speed is not proportional to RPM.
//! See docs/formulas.md for the source of each function.

use crate::config::Config;
use crate::model::{Recipe, RecipeKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Machine {
    Press,
    Mixer,
    Millstone,
    CrushingWheels,
    Saw,
    Deployer,
    Spout,
    ItemDrain,
    MechanicalCrafter,
    Fan,
    Drill,
    Furnace,
    BlastFurnace,
    Smoker,
    Hand,
}

impl Machine {
    /// Block id used for stress config and how many of them one machine is made of.
    fn stress_block(self) -> Option<(&'static str, u32)> {
        match self {
            Machine::Press => Some(("mechanical_press", 1)),
            Machine::Mixer => Some(("mechanical_mixer", 1)),
            Machine::Millstone => Some(("millstone", 1)),
            Machine::CrushingWheels => Some(("crushing_wheel", 2)),
            Machine::Saw => Some(("mechanical_saw", 1)),
            Machine::Deployer => Some(("deployer", 1)),
            Machine::MechanicalCrafter => Some(("mechanical_crafter", 1)),
            Machine::Fan => Some(("encased_fan", 1)),
            Machine::Drill => Some(("mechanical_drill", 1)),
            _ => None,
        }
    }

    pub fn uses_batches(self) -> bool {
        matches!(self, Machine::CrushingWheels | Machine::Fan)
    }
}

/// Machines able to run a recipe, default first. Empty if the config forbids all of them.
pub fn options(recipe: &Recipe, cfg: &Config) -> Vec<Machine> {
    use Machine::*;
    match recipe.kind {
        RecipeKind::Pressing | RecipeKind::Compacting => vec![Press],
        RecipeKind::Mixing => vec![Mixer],
        RecipeKind::Milling => vec![Millstone, CrushingWheels],
        RecipeKind::Crushing => vec![CrushingWheels],
        RecipeKind::Cutting => vec![Saw],
        RecipeKind::Deploying => vec![Deployer, Hand],
        RecipeKind::Filling => vec![Spout],
        RecipeKind::Emptying => vec![ItemDrain],
        RecipeKind::Splashing | RecipeKind::Haunting => vec![Fan],
        RecipeKind::Smelting => vec![Fan, Furnace],
        RecipeKind::Blasting => vec![Fan, BlastFurnace],
        RecipeKind::Smoking => vec![Fan, Smoker],
        RecipeKind::CraftingShaped | RecipeKind::CraftingShapeless => {
            let mut list = Vec::new();
            if cfg.allow_regular_crafting_in_crafter {
                list.push(MechanicalCrafter);
            }
            if recipe.kind == RecipeKind::CraftingShapeless && cfg.allow_shapeless_in_mixer && recipe.cells > 1 {
                list.push(Mixer);
            }
            if recipe.square && cfg.allow_shaped_square_in_press {
                list.push(Press);
            }
            list.push(Hand);
            list
        }
        RecipeKind::MechanicalCrafting => vec![MechanicalCrafter],
        RecipeKind::Stonecutting => {
            if cfg.allow_stonecutting_on_saw {
                vec![Saw, Hand]
            } else {
                vec![Hand]
            }
        }
        RecipeKind::Smithing => vec![Hand],
        RecipeKind::SequencedAssembly => vec![],
    }
}

/// Machine that performs one step of a sequenced assembly.
pub fn step_machine(kind: RecipeKind) -> Machine {
    match kind {
        RecipeKind::Pressing => Machine::Press,
        RecipeKind::Cutting => Machine::Saw,
        RecipeKind::Filling => Machine::Spout,
        _ => Machine::Deployer,
    }
}

/// `PressingBehaviour.getRunningTickSpeed` and the cycle in `PressingBehaviour.tick`.
pub fn press_ticks(rpm: f64) -> u32 {
    const CYCLE: i64 = 240;
    let t = (rpm.abs() as f32 / 512.0).clamp(0.0, 1.0);
    let speed = (1.0 + t * 59.0) as i64;
    let (mut running, mut ticks) = (0i64, 0u32);
    loop {
        ticks += 1;
        if running > CYCLE {
            return ticks;
        }
        let prev = running;
        running += speed;
        if prev < CYCLE / 2 && running >= CYCLE / 2 {
            running = CYCLE / 2;
        }
    }
}

/// `MechanicalMixerBlockEntity.tick`. `duration` of 0 means the default mixing speed.
pub fn mixer_ticks(rpm: f64, duration: u32) -> u32 {
    let recipe_speed = if duration == 0 { 1.0 } else { duration as f32 / 100.0 };
    let quotient = (512.0 / rpm.abs() as f32) as u32;
    let log = if quotient == 0 { -1 } else { 31 - quotient.leading_zeros() as i64 };
    let processing = (log * (recipe_speed * 15.0).ceil() as i64 + 1).max(1);
    let (mut running, mut left, mut ticks) = (0, -1i64, 0u32);
    loop {
        ticks += 1;
        if running >= 40 {
            return ticks;
        }
        if running == 20 {
            if left < 0 {
                left = processing;
            } else {
                left -= 1;
                if left == 0 {
                    running += 1;
                    left = -1;
                }
            }
        } else {
            running += 1;
        }
    }
}

/// `MillstoneBlockEntity.tick`.
pub fn millstone_ticks(rpm: f64, duration: u32) -> u32 {
    let speed = ((rpm.abs() as f32 / 16.0) as i64).clamp(1, 512);
    (duration as i64 / speed + i64::from(duration as i64 % speed != 0)) as u32 + 1
}

/// `SawBlockEntity.tick`. The recipe applies below 5 ticks left, then 20 more ticks to eject.
pub fn saw_ticks(rpm: f64, duration: u32, stack: u32) -> u32 {
    let speed = (rpm.abs() as f32 / 24.0).clamp(1.0, 128.0);
    let mut remaining = (duration * (stack / 5).max(1)) as f32;
    let mut ticks = 0;
    while remaining >= 5.0 {
        remaining -= speed;
        ticks += 1;
    }
    ticks + (20.0 / speed).ceil() as u32
}

/// `CrushingWheelControllerBlockEntity.tick`. The whole stack is one operation.
pub fn crushing_ticks(rpm: f64, duration: u32, stack: u32) -> u32 {
    let speed = rpm.abs() as f32 / 50.0 * 4.0;
    let log = (stack.max(1) as f32).ln() / 2f32.ln();
    let mut remaining = duration as f32;
    let mut applied = false;
    let mut ticks = 0;
    loop {
        // A stack of one gives log = 0, so the division yields infinity and clamps to 20.
        let divisor = if applied { 1.0 } else { log };
        remaining -= (speed / divisor).clamp(0.25, 20.0);
        ticks += 1;
        // The tick that applies the recipe returns early, so output waits for the next one.
        if remaining < 20.0 && !applied {
            applied = true;
            continue;
        }
        if remaining <= 0.0 {
            return ticks;
        }
    }
}

/// `DeployerBlockEntity.tick`: expand 1000, retract 1000, wait 500.
pub fn deployer_ticks(rpm: f64) -> u32 {
    let speed = ((rpm.abs() * 2.0) as i64).clamp(8, 512);
    let phase = |t: i64| (t / speed + i64::from(t % speed != 0)) as u32;
    phase(1000) + phase(1000) + phase(500)
}

/// `MechanicalCrafterBlockEntity.tick`. Each hop along the chain costs 1000, the craft 2000.
/// The longest chain is assumed: every occupied cell is one hop.
pub fn crafter_ticks(rpm: f64, cells: u32) -> u32 {
    let speed = (rpm.abs() as i64).clamp(4, 250);
    let phase = |t: i64| (t / speed + 1) as u32;
    cells.saturating_sub(1) * phase(1000) + phase(2000)
}

/// `BlockBreakingKineticBlockEntity.tick` for one block of the given hardness.
pub fn drill_ticks(rpm: f64, hardness: f32) -> u32 {
    let speed = rpm.abs() as f32 / 100.0;
    let (mut progress, mut ticks) = (0, 0);
    loop {
        progress += ((speed / hardness) as i32).clamp(1, 10 - progress);
        ticks += 1;
        if progress >= 10 {
            return ticks;
        }
        ticks += (hardness / speed) as u32;
    }
}

/// Lava spreads one block per 30 ticks in the overworld, so a generator refills that slowly.
/// This depends on the layout and is an estimate, not a value from Create.
pub const GENERATOR_REFILL_TICKS: u32 = 30;

/// Drills needed on stone generators. Block hardness comes from the game: cobblestone 2, stone 1.5.
pub fn size_generator(res: &str, rpm: f64, per_min: f64, cfg: &Config) -> Load {
    let hardness = if res == "minecraft:stone" { 1.5 } else { 2.0 };
    let ticks = drill_ticks(rpm, hardness) + GENERATOR_REFILL_TICKS;
    let exact = per_min / 1200.0 * ticks as f64;
    let count = if exact <= 1e-9 { 0 } else { (exact - 1e-9).ceil() as u32 };
    let su = cfg.impact_of("mechanical_drill") * rpm.abs() * count as f64;
    Load { machine: Machine::Drill, rpm, ticks, items_per_op: 1, exact, count, su, stream_blocks: None, max_belt_rpm: None }
}

/// `FanProcessing.applyProcessing`.
pub fn fan_ticks(cfg: &Config, stack: u32) -> u32 {
    cfg.fan_processing_time * ((stack.max(1) - 1) / 16 + 1) + 1
}

/// `IAirCurrentSource.getMaxDistance`: blocks of air a single fan reaches.
pub fn fan_range(rpm: f64, cfg: &Config) -> f64 {
    let t = (rpm.abs() / cfg.fan_rotation_argmax).min(1.0);
    3.0 + (cfg.fan_push_distance - 3.0) * t
}

/// Stacks per minute one belt carries at 1 RPM: `getSpeed() / 480` blocks per tick,
/// and `BeltInventory` keeps stacks one block apart.
pub const BELT_STACKS_PER_MIN_PER_RPM: f64 = 1200.0 / 480.0;

/// How one flow of items travels. Fluids have none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transport {
    pub stack: u32,
    pub belt_rpm: f64,
    /// Belts needed at `belt_rpm`.
    pub belts: u32,
    /// The slowest single belt that would carry this flow.
    pub min_belt_rpm: f64,
    /// Funnels needed to load it, each moving one stack per `default_extraction_timer` ticks.
    pub funnels: u32,
    /// The source yields less than a stack at a time, so items must collect in a container first.
    pub needs_buffer: bool,
}

fn need(amount: f64) -> u32 {
    (amount - 1e-9).ceil().max(0.0) as u32
}

/// Sizes the transport for `per_min` items moved in stacks of `stack`.
pub fn transport(per_min: f64, stack: u32, belt_rpm: f64, source_yield: f64, cfg: &Config) -> Transport {
    let stack = stack.max(1);
    let stacks_per_min = per_min / stack as f64;
    let min_belt_rpm = stacks_per_min / BELT_STACKS_PER_MIN_PER_RPM;
    let funnel_ops = 1200.0 / cfg.default_extraction_timer.max(1) as f64;
    Transport {
        stack,
        belt_rpm,
        belts: need(min_belt_rpm / belt_rpm),
        min_belt_rpm,
        funnels: need(stacks_per_min / funnel_ops),
        needs_buffer: (stack as f64) > source_yield,
    }
}

/// Result of sizing one machine type for a required rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Load {
    pub machine: Machine,
    pub rpm: f64,
    /// Ticks per operation; an operation handles `items_per_op` inputs.
    pub ticks: u32,
    pub items_per_op: u32,
    pub exact: f64,
    pub count: u32,
    pub su: f64,
    /// Fans only: blocks of belt or air the items must stay in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_blocks: Option<f64>,
    /// Fans only: the fastest belt that keeps items in one fan's stream long enough.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_belt_rpm: Option<f64>,
}

/// Ticks per operation and how many recipe runs one operation performs.
pub fn timing(machine: Machine, recipe_kind: RecipeKind, duration: u32, cells: u32, rpm: f64, batch: u32, cfg: &Config) -> (u32, u32) {
    let batch = batch.max(1);
    match machine {
        Machine::Press => {
            let bulk = cfg.bulk_pressing && recipe_kind == RecipeKind::Pressing;
            (press_ticks(rpm), if bulk { batch } else { 1 })
        }
        Machine::Mixer => (mixer_ticks(rpm, duration), 1),
        Machine::Millstone => (millstone_ticks(rpm, if duration == 0 { 100 } else { duration }), 1),
        Machine::CrushingWheels => (crushing_ticks(rpm, if duration == 0 { 100 } else { duration }, batch), batch),
        Machine::Saw => {
            let stack = if cfg.bulk_cutting { batch } else { 1 };
            (saw_ticks(rpm, if duration == 0 { 50 } else { duration }, stack), stack)
        }
        Machine::Deployer => (deployer_ticks(rpm), 1),
        Machine::Spout | Machine::ItemDrain => (20, 1),
        Machine::MechanicalCrafter => (crafter_ticks(rpm, cells.max(1)), 1),
        Machine::Fan => (fan_ticks(cfg, batch), batch),
        Machine::Furnace => (if duration == 0 { 200 } else { duration }, 1),
        Machine::BlastFurnace | Machine::Smoker => (if duration == 0 { 100 } else { duration }, 1),
        Machine::Drill => (drill_ticks(rpm, 2.0), 1),
        Machine::Hand => (0, 1),
    }
}

/// Sizes a machine for `runs_per_min` recipe runs.
pub fn size(machine: Machine, kind: RecipeKind, duration: u32, cells: u32, rpm: f64, batch: u32, runs_per_min: f64, cfg: &Config) -> Load {
    let (ticks, items_per_op) = timing(machine, kind, duration, cells, rpm, batch, cfg);
    let ops_per_tick = runs_per_min / 1200.0 / items_per_op as f64;
    let mut exact = ops_per_tick * ticks as f64;
    let (mut stream_blocks, mut max_belt_rpm) = (None, None);
    if machine == Machine::Fan {
        // Every occupied belt block holds one stack, so stacks in the air equal blocks of stream.
        let range = fan_range(rpm, cfg);
        stream_blocks = Some(exact);
        max_belt_rpm = Some(range / ticks as f64 * 480.0);
        exact /= range;
    }
    let count = if exact <= 1e-9 { 0 } else { (exact - 1e-9).ceil() as u32 };
    let per_machine = match machine {
        // A crafter grid needs one crafter per occupied cell.
        Machine::MechanicalCrafter => cells.max(1),
        _ => 1,
    };
    let su = machine
        .stress_block()
        .map(|(block, blocks)| cfg.impact_of(block) * rpm.abs() * (blocks * per_machine * count) as f64)
        .unwrap_or(0.0);
    Load { machine, rpm, ticks, items_per_op, exact, count, su, stream_blocks, max_belt_rpm }
}
