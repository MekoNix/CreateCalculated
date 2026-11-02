//! Share links: a plan written out as a short text, so nothing has to be stored anywhere.
//!
//! A code holds only what differs from the defaults. Ids become 5 characters of a hash rather than
//! a position in a list, so new recipes and addons leave old links working: whoever reads a code
//! hashes the ids it knows and looks them up.
//!
//! Links live forever, so how something already written is read must never change. Add a mask bit
//! or a table entry at the end, or a new format letter for anything bigger.

use crate::config::Config;
use crate::machines::{Funnel, Machine};
use crate::model::{Catalog, ResId};
use crate::plan::{Branch, Mode, NodeOverride, PlanRequest, Target};
use std::collections::HashMap;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const FORMAT_1: char = 'B';
const MAX_CODE_LEN: usize = 8000;

const FULL_AUTO: u64 = 1 << 0;
const RPM: u64 = 1 << 1;
const STOCK: u64 = 1 << 2;
const CHOICE: u64 = 1 << 3;
const BRANCHES: u64 = 1 << 4;
const OVERRIDES: u64 = 1 << 5;
const STACK_SIZES: u64 = 1 << 6;
const BELT_RPM: u64 = 1 << 7;
const BATCH: u64 = 1 << 8;
const STACK: u64 = 1 << 9;
const NO_HEAT: u64 = 1 << 10;
const BRASS: u64 = 1 << 11;
const SAILS: u64 = 1 << 12;
const CONFIG: u64 = 1 << 13;
const KNOWN_BITS: u64 = (1 << 14) - 1;

const OVERRIDE_RPM: u64 = 1;
const OVERRIDE_MACHINE: u64 = 2;
const OVERRIDE_BATCH: u64 = 4;

const DEFAULT_SAILS: u32 = 64;

/// Positions are written into links, so entries are only ever added at the end.
const MACHINES: [Machine; 15] = [
    Machine::Press,
    Machine::Mixer,
    Machine::Millstone,
    Machine::CrushingWheels,
    Machine::Saw,
    Machine::Deployer,
    Machine::Spout,
    Machine::ItemDrain,
    Machine::MechanicalCrafter,
    Machine::Fan,
    Machine::Drill,
    Machine::Furnace,
    Machine::BlastFurnace,
    Machine::Smoker,
    Machine::Hand,
];
const IMPACT: [&str; 9] = ["mechanical_press", "crushing_wheel", "mechanical_mixer", "millstone", "mechanical_saw", "deployer", "mechanical_drill", "mechanical_crafter", "encased_fan"];
const CAPACITY: [&str; 4] = ["water_wheel", "large_water_wheel", "windmill_bearing", "steam_engine"];
/// Config fields are numbered: these for plain values, then the stress tables after them.
const IMPACT_FROM: u64 = 100;
const CAPACITY_FROM: u64 = 200;

/// What a link carries, the page's settings as well as the request.
pub struct Decoded {
    pub request: PlanRequest,
    pub sails: u32,
    /// Whether the config differs from Create's defaults.
    pub has_config: bool,
    /// How many ids or settings this calculator no longer knows, and left out.
    pub missing: usize,
}

/// Hash of an id, as written in links. FNV-1a folded to 30 bits, which is exactly 5 characters.
pub fn fingerprint(id: &str) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in id.bytes() {
        hash = (hash ^ byte as u32).wrapping_mul(16777619);
    }
    (hash ^ (hash >> 30)) & ((1 << 30) - 1)
}

struct Writer(String);

impl Writer {
    /// A number in groups of 5 bits, with the sixth saying that more follow.
    fn number(&mut self, mut value: u64) {
        loop {
            let low = (value & 31) as usize;
            value >>= 5;
            self.0.push(ALPHABET[low | if value > 0 { 32 } else { 0 }] as char);
            if value == 0 {
                return;
            }
        }
    }

    fn id(&mut self, id: &str) {
        let mut hash = fingerprint(id);
        for _ in 0..5 {
            self.0.push(ALPHABET[(hash & 63) as usize] as char);
            hash >>= 6;
        }
    }

    /// Whole numbers stay short, others keep 3 decimals.
    fn real(&mut self, value: f64) {
        self.number(real_bits(value));
    }
}

fn real_bits(value: f64) -> u64 {
    let value = if value.is_finite() { value.max(0.0) } else { 0.0 };
    if value.fract() == 0.0 {
        (value as u64) << 1
    } else {
        ((value * 1000.0).round() as u64) << 1 | 1
    }
}

fn bits_real(bits: u64) -> f64 {
    if bits & 1 == 1 {
        (bits >> 1) as f64 / 1000.0
    } else {
        (bits >> 1) as f64
    }
}

/// Config values that differ from the defaults, by their number.
fn config_changes(cfg: &Config) -> Vec<(u64, f64)> {
    let d = Config::default();
    let flag = |v: bool| if v { 1.0 } else { 0.0 };
    let plain = [
        (cfg.max_rotation_speed, d.max_rotation_speed),
        (cfg.fan_processing_time as f64, d.fan_processing_time as f64),
        (cfg.fan_rotation_argmax, d.fan_rotation_argmax),
        (cfg.fan_push_distance, d.fan_push_distance),
        (cfg.windmill_sails_per_rpm as f64, d.windmill_sails_per_rpm as f64),
        (cfg.default_extraction_timer as f64, d.default_extraction_timer as f64),
        (flag(cfg.bulk_pressing), flag(d.bulk_pressing)),
        (flag(cfg.bulk_cutting), flag(d.bulk_cutting)),
        (flag(cfg.allow_shapeless_in_mixer), flag(d.allow_shapeless_in_mixer)),
        (flag(cfg.allow_shaped_square_in_press), flag(d.allow_shaped_square_in_press)),
        (flag(cfg.allow_regular_crafting_in_crafter), flag(d.allow_regular_crafting_in_crafter)),
        (flag(cfg.allow_stonecutting_on_saw), flag(d.allow_stonecutting_on_saw)),
    ];
    let mut out: Vec<(u64, f64)> = plain.iter().enumerate().filter(|(_, (a, b))| a != b).map(|(i, (a, _))| (i as u64, *a)).collect();
    for (i, key) in IMPACT.iter().enumerate() {
        if cfg.impact_of(key) != d.impact_of(key) {
            out.push((IMPACT_FROM + i as u64, cfg.impact_of(key)));
        }
    }
    for (i, key) in CAPACITY.iter().enumerate() {
        if cfg.capacity_of(key) != d.capacity_of(key) {
            out.push((CAPACITY_FROM + i as u64, cfg.capacity_of(key)));
        }
    }
    out
}

/// Sets one config value by its number. False means the number is unknown here.
fn apply_config(cfg: &mut Config, field: u64, value: f64) -> bool {
    let flag = value != 0.0;
    match field {
        0 => cfg.max_rotation_speed = value,
        1 => cfg.fan_processing_time = value as u32,
        2 => cfg.fan_rotation_argmax = value,
        3 => cfg.fan_push_distance = value,
        4 => cfg.windmill_sails_per_rpm = value as u32,
        5 => cfg.default_extraction_timer = value as u32,
        6 => cfg.bulk_pressing = flag,
        7 => cfg.bulk_cutting = flag,
        8 => cfg.allow_shapeless_in_mixer = flag,
        9 => cfg.allow_shaped_square_in_press = flag,
        10 => cfg.allow_regular_crafting_in_crafter = flag,
        11 => cfg.allow_stonecutting_on_saw = flag,
        f if (IMPACT_FROM..IMPACT_FROM + IMPACT.len() as u64).contains(&f) => {
            cfg.impact.insert(IMPACT[(f - IMPACT_FROM) as usize].to_string(), value);
        }
        f if (CAPACITY_FROM..CAPACITY_FROM + CAPACITY.len() as u64).contains(&f) => {
            cfg.capacity.insert(CAPACITY[(f - CAPACITY_FROM) as usize].to_string(), value);
        }
        _ => return false,
    }
    true
}

/// Writes a request as a code. Settings only the planner itself uses, such as `stop_at_materials`,
/// are not part of a scheme and stay out.
pub fn encode(request: &PlanRequest, sails: u32) -> String {
    let d = PlanRequest::default();
    let config = config_changes(&request.config);
    let checks = [
        (FULL_AUTO, request.mode == Mode::FullAuto),
        (RPM, request.rpm != d.rpm),
        (STOCK, !request.stock.is_empty()),
        (CHOICE, !request.recipe_choice.is_empty()),
        (BRANCHES, !request.branches.is_empty()),
        (OVERRIDES, !request.overrides.is_empty()),
        (STACK_SIZES, !request.stack_sizes.is_empty()),
        (BELT_RPM, request.belt_rpm.is_some()),
        (BATCH, request.batch != d.batch),
        (STACK, request.stack != d.stack),
        (NO_HEAT, !request.include_heat),
        (BRASS, request.funnel == Funnel::Brass),
        (SAILS, sails != DEFAULT_SAILS),
        (CONFIG, !config.is_empty()),
    ];
    let mask = checks.iter().filter(|(_, on)| *on).fold(0, |m, (bit, _)| m | bit);

    let mut w = Writer(String::new());
    w.0.push(FORMAT_1);
    w.number(mask);
    w.number(request.targets.len() as u64);
    for t in &request.targets {
        w.id(&t.item);
        w.real(t.per_minute);
    }
    if mask & RPM != 0 {
        w.real(request.rpm);
    }
    if mask & STOCK != 0 {
        w.number(request.stock.len() as u64);
        request.stock.iter().for_each(|s| w.id(s));
    }
    if mask & CHOICE != 0 {
        w.number(request.recipe_choice.len() as u64);
        for (item, recipe) in &request.recipe_choice {
            w.id(item);
            w.id(recipe);
        }
    }
    if mask & BRANCHES != 0 {
        w.number(request.branches.len() as u64);
        for b in &request.branches {
            w.id(&b.recipe);
            w.id(&b.input);
            // Zero means all the leftovers, so amounts are shifted by one.
            w.number(b.per_minute.map_or(0, |v| real_bits(v) + 1));
        }
    }
    if mask & OVERRIDES != 0 {
        w.number(request.overrides.len() as u64);
        for (recipe, o) in &request.overrides {
            w.id(recipe);
            let inner = [(OVERRIDE_RPM, o.rpm.is_some()), (OVERRIDE_MACHINE, o.machine.is_some()), (OVERRIDE_BATCH, o.batch.is_some())];
            w.number(inner.iter().filter(|(_, on)| *on).fold(0, |m, (bit, _)| m | bit));
            if let Some(rpm) = o.rpm {
                w.real(rpm);
            }
            if let Some(machine) = o.machine {
                w.number(MACHINES.iter().position(|m| *m == machine).unwrap_or(0) as u64);
            }
            if let Some(batch) = o.batch {
                w.number(batch as u64);
            }
        }
    }
    if mask & STACK_SIZES != 0 {
        w.number(request.stack_sizes.len() as u64);
        for (item, stack) in &request.stack_sizes {
            w.id(item);
            w.number(*stack as u64);
        }
    }
    if let Some(rpm) = request.belt_rpm {
        w.real(rpm);
    }
    if mask & BATCH != 0 {
        w.number(request.batch as u64);
    }
    if mask & STACK != 0 {
        w.number(request.stack as u64);
    }
    if mask & SAILS != 0 {
        w.number(sails as u64);
    }
    if mask & CONFIG != 0 {
        w.number(config.len() as u64);
        for (field, value) in &config {
            w.number(*field);
            w.real(*value);
        }
    }
    w.0
}

struct Reader<'a> {
    rest: std::slice::Iter<'a, u8>,
}

impl Reader<'_> {
    fn sextet(&mut self) -> Result<u64, String> {
        let byte = *self.rest.next().ok_or("the code ends too early")?;
        ALPHABET.iter().position(|c| *c == byte).map(|p| p as u64).ok_or_else(|| "the code has a wrong character".to_string())
    }

    fn number(&mut self) -> Result<u64, String> {
        let (mut value, mut shift) = (0u64, 0);
        loop {
            let part = self.sextet()?;
            if shift > 55 {
                return Err("a number in the code is too long".into());
            }
            value |= (part & 31) << shift;
            shift += 5;
            if part & 32 == 0 {
                return Ok(value);
            }
        }
    }

    fn id(&mut self) -> Result<u32, String> {
        let mut hash = 0;
        for i in 0..5 {
            hash |= (self.sextet()? as u32) << (6 * i);
        }
        Ok(hash)
    }

    fn real(&mut self) -> Result<f64, String> {
        Ok(bits_real(self.number()?))
    }

    fn small(&mut self) -> Result<u32, String> {
        Ok(self.number()?.clamp(1, u32::MAX as u64) as u32)
    }
}

/// Lookup tables from the hash back to the id. Sorted ids make a clash resolve the same way everywhere.
struct Known {
    items: HashMap<u32, ResId>,
    recipes: HashMap<u32, String>,
}

impl Known {
    fn new(cat: &Catalog) -> Self {
        let mut items = HashMap::new();
        let mut ids = cat.resources();
        ids.extend(cat.bundle.tags.keys().cloned());
        ids.sort();
        for id in ids {
            items.entry(fingerprint(&id)).or_insert(id);
        }
        let mut recipe_ids: Vec<&String> = cat.recipes().iter().map(|r| &r.id).collect();
        recipe_ids.sort();
        let mut recipes = HashMap::new();
        for id in recipe_ids {
            recipes.entry(fingerprint(id)).or_insert_with(|| id.clone());
        }
        Self { items, recipes }
    }
}

/// Ids that would share a hash and so read back wrong. A test keeps this empty for the real data.
pub fn clashes(cat: &Catalog) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut ids = cat.resources();
    ids.extend(cat.bundle.tags.keys().cloned());
    let recipes: Vec<String> = cat.recipes().iter().map(|r| r.id.clone()).collect();
    for list in [ids, recipes] {
        let mut seen: HashMap<u32, String> = HashMap::new();
        for id in list {
            if let Some(other) = seen.insert(fingerprint(&id), id.clone()) {
                if other != id {
                    out.push((other, id));
                }
            }
        }
    }
    out
}

/// Reads a code. Ids this calculator does not know are left out and counted, so a link from before
/// an item was removed still opens, while a code that makes no sense is an error.
pub fn decode(cat: &Catalog, code: &str) -> Result<Decoded, String> {
    if code.len() > MAX_CODE_LEN {
        return Err("the code is too long".into());
    }
    let mut r = Reader { rest: code.as_bytes().iter() };
    match r.rest.next().map(|b| *b as char) {
        Some(FORMAT_1) => {}
        Some(_) => return Err("this link comes from another version".into()),
        None => return Err("the code is empty".into()),
    }
    let mask = r.number()?;
    if mask & !KNOWN_BITS != 0 {
        return Err("this link comes from a newer version".into());
    }
    let known = Known::new(cat);
    let mut missing = 0;
    let mut item = |hash: u32| {
        let found = known.items.get(&hash).cloned();
        missing += usize::from(found.is_none());
        found
    };
    let mut request = PlanRequest::default();
    let mut other_missing = 0;

    for _ in 0..r.number()? {
        let (hash, per_minute) = (r.id()?, r.real()?);
        if let Some(item) = item(hash) {
            request.targets.push(Target { item, per_minute });
        }
    }
    if mask & FULL_AUTO != 0 {
        request.mode = Mode::FullAuto;
    }
    if mask & RPM != 0 {
        request.rpm = r.real()?;
    }
    if mask & STOCK != 0 {
        for _ in 0..r.number()? {
            let hash = r.id()?;
            request.stock.extend(item(hash));
        }
    }
    if mask & CHOICE != 0 {
        for _ in 0..r.number()? {
            let (item_hash, recipe_hash) = (r.id()?, r.id()?);
            let recipe = known.recipes.get(&recipe_hash).cloned();
            other_missing += usize::from(recipe.is_none());
            if let (Some(i), Some(recipe)) = (item(item_hash), recipe) {
                request.recipe_choice.insert(i, recipe);
            }
        }
    }
    if mask & BRANCHES != 0 {
        for _ in 0..r.number()? {
            let (recipe_hash, input_hash, amount) = (r.id()?, r.id()?, r.number()?);
            let recipe = known.recipes.get(&recipe_hash).cloned();
            other_missing += usize::from(recipe.is_none());
            if let (Some(recipe), Some(input)) = (recipe, item(input_hash)) {
                let per_minute = (amount > 0).then(|| bits_real(amount - 1));
                request.branches.push(Branch { recipe, input, per_minute });
            }
        }
    }
    if mask & OVERRIDES != 0 {
        for _ in 0..r.number()? {
            let recipe_hash = r.id()?;
            let inner = r.number()?;
            let mut o = NodeOverride::default();
            if inner & OVERRIDE_RPM != 0 {
                o.rpm = Some(r.real()?);
            }
            if inner & OVERRIDE_MACHINE != 0 {
                let position = r.number()? as usize;
                o.machine = MACHINES.get(position).copied();
                other_missing += usize::from(o.machine.is_none());
            }
            if inner & OVERRIDE_BATCH != 0 {
                o.batch = Some(r.small()?);
            }
            match known.recipes.get(&recipe_hash) {
                Some(recipe) => {
                    request.overrides.insert(recipe.clone(), o);
                }
                None => other_missing += 1,
            }
        }
    }
    if mask & STACK_SIZES != 0 {
        for _ in 0..r.number()? {
            let (hash, stack) = (r.id()?, r.small()?);
            if let Some(i) = item(hash) {
                request.stack_sizes.insert(i, stack);
            }
        }
    }
    if mask & BELT_RPM != 0 {
        request.belt_rpm = Some(r.real()?);
    }
    if mask & BATCH != 0 {
        request.batch = r.small()?;
    }
    if mask & STACK != 0 {
        request.stack = r.small()?;
    }
    if mask & NO_HEAT != 0 {
        request.include_heat = false;
    }
    if mask & BRASS != 0 {
        request.funnel = Funnel::Brass;
    }
    let sails = if mask & SAILS != 0 { r.small()? } else { DEFAULT_SAILS };
    if mask & CONFIG != 0 {
        for _ in 0..r.number()? {
            let (field, value) = (r.number()?, r.real()?);
            other_missing += usize::from(!apply_config(&mut request.config, field, value));
        }
    }
    if r.rest.next().is_some() {
        return Err("the code goes on after its end".into());
    }
    Ok(Decoded { sails, has_config: mask & CONFIG != 0, missing: missing + other_missing, request })
}
