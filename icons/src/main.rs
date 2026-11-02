//! Builds inventory icons from JARs.

use createcalc_core::model::Bundle;
use createcalc_icons::assets::{peek_jar, Assets};
use createcalc_icons::fetch::Fetcher;
use createcalc_icons::generate::generate;
use std::collections::BTreeSet;
use std::path::PathBuf;

const USAGE: &str = "usage: createcalc-icons [options] [file.jar ...]

Takes the JARs you name and downloads whatever is still missing: the game from Mojang,
Create and the mods you ask for from Modrinth. Later JARs replace earlier ones.

  --mod SLUG[@VERSION]  also use this Modrinth project, repeat for more mods
  --minecraft VERSION   game version, default 1.21.1
  --loader NAME         mod loader on Modrinth, default neoforge
  -o, --out DIR         where the pictures go, default data/icons
  --bundle FILE         recipes for tag pictures and coverage, default data/bundle.json
  --size N              picture size in pixels, default 128
  --jars DIR            where downloads are kept, default data/jars
  --update              ask Modrinth for newer versions even when a download exists
  --no-download         use only the JARs named here
";

/// Mods every calculator needs. Addons go here or on the command line.
const DEFAULT_MODS: &[&str] = &["create"];

struct Args {
    jars: Vec<PathBuf>,
    mods: Vec<String>,
    minecraft: String,
    loader: String,
    out: PathBuf,
    bundle: PathBuf,
    size: u32,
    cache: PathBuf,
    update: bool,
    download: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        jars: Vec::new(),
        mods: DEFAULT_MODS.iter().map(|m| m.to_string()).collect(),
        minecraft: "1.21.1".into(),
        loader: "neoforge".into(),
        out: "data/icons".into(),
        bundle: "data/bundle.json".into(),
        size: 128,
        cache: "data/jars".into(),
        update: false,
        download: true,
    };
    let mut list = std::env::args().skip(1);
    let value = |list: &mut std::iter::Skip<std::env::Args>, name: &str| list.next().unwrap_or_else(|| fail(&format!("{name} needs a value")));
    while let Some(arg) = list.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--mod" => args.mods.push(value(&mut list, "--mod")),
            "--minecraft" => args.minecraft = value(&mut list, "--minecraft"),
            "--loader" => args.loader = value(&mut list, "--loader"),
            "-o" | "--out" => args.out = value(&mut list, "--out").into(),
            "--bundle" => args.bundle = value(&mut list, "--bundle").into(),
            "--size" => {
                args.size = value(&mut list, "--size").parse().ok().filter(|s| (16..=1024).contains(s)).unwrap_or_else(|| fail("--size must be from 16 to 1024"));
            }
            "--jars" => args.cache = value(&mut list, "--jars").into(),
            "--update" => args.update = true,
            "--no-download" => args.download = false,
            other if other.starts_with('-') => fail(&format!("unknown option {other}\n\n{USAGE}")),
            other => args.jars.push(other.into()),
        }
    }
    args
}

fn main() {
    let args = parse_args();

    let mut has_minecraft = false;
    let mut present: BTreeSet<String> = BTreeSet::new();
    for jar in &args.jars {
        let info = peek_jar(jar).unwrap_or_else(|e| fail(&e));
        has_minecraft |= info.has_minecraft;
        present.extend(info.mods);
    }

    // The game goes first and downloaded mods last, so mods can replace vanilla files.
    let mut order: Vec<PathBuf> = Vec::new();
    let mut fetcher = Fetcher::new(&args.cache);
    fetcher.update = args.update;
    if !has_minecraft {
        if !args.download {
            fail("no JAR with the game's textures given, and --no-download forbids fetching one");
        }
        order.push(fetcher.minecraft(&args.minecraft).unwrap_or_else(|e| fail(&e)));
    }
    order.extend(args.jars.iter().cloned());
    if args.download {
        for spec in &args.mods {
            let slug = spec.split('@').next().unwrap_or(spec);
            if !present.contains(&slug.replace('-', "_")) && !present.contains(slug) {
                order.push(fetcher.modrinth(spec, &args.minecraft, &args.loader).unwrap_or_else(|e| fail(&e)));
            }
        }
    }

    let mut assets = Assets::default();
    for jar in &order {
        println!("reading {}", jar.display());
        assets.add_jar(jar).unwrap_or_else(|e| fail(&e));
    }

    let bundle: Option<Bundle> = match std::fs::read_to_string(&args.bundle) {
        Ok(text) => Some(serde_json::from_str(&text).unwrap_or_else(|e| fail(&format!("{}: {e}", args.bundle.display())))),
        Err(_) => {
            println!("no {}, so no pictures for tags and no coverage figures", args.bundle.display());
            None
        }
    };

    let report = generate(&assets, bundle.as_ref(), args.size, &args.out).unwrap_or_else(|e| fail(&e));

    println!("written: {} pictures, {} KiB, in {}", report.written, report.bytes / 1024, args.out.display());
    println!("by source: {:?}", report.by_source);
    if !report.stand_ins.is_empty() {
        println!("texture instead of a model: {}", report.stand_ins.join(", "));
    }
    println!("no picture: {}", report.failed.len());
    for (id, reason) in report.failed.iter().take(40) {
        println!("  {id}: {reason}");
    }
    if !report.incomplete.is_empty() {
        println!("drawn with missing textures: {}", report.incomplete.len());
        for (id, missing) in report.incomplete.iter().take(20) {
            println!("  {id}: {}", missing.join(", "));
        }
    }
    if let Some(c) = &report.coverage {
        println!("recipes use {} items, {} with a picture", c.items.1, c.items.0);
        println!("recipes use {} fluids, {} with a picture", c.fluids.1, c.fluids.0);
        println!("recipes use {} tags, {} with a picture", c.tags.1, c.tags.0);
        for id in c.missing.iter().take(40) {
            println!("  no picture: {id}");
        }
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1)
}
