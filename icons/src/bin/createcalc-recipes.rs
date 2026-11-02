//! Downloads what recipes are read from and builds the bundle.

use createcalc_core::import::{add_names, import_jars};
use createcalc_icons::fetch::Fetcher;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: createcalc-recipes [options]

Downloads the game from Mojang, NeoForge from its Maven repository and Create and the mods
you ask for from Modrinth, then builds the recipe bundle. Nothing is run, the JARs are only read.

  --mod SLUG[@VERSION]  also use this Modrinth project, repeat for more mods
  --minecraft VERSION   game version, default 1.21.1
  -o, --out FILE        where the bundle goes, default data/bundle.json
  --jars DIR            where downloads are kept, default data/jars
  --update              ask for newer versions even when a download exists
";

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1)
}

fn main() {
    let mut mods = vec!["create".to_string()];
    let mut game = "1.21.1".to_string();
    let mut out = PathBuf::from("data/bundle.json");
    let mut cache = PathBuf::from("data/jars");
    let mut update = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| fail(&format!("{arg} needs a value\n{USAGE}")));
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return;
            }
            "--mod" => mods.push(value()),
            "--minecraft" => game = value(),
            "-o" | "--out" => out = value().into(),
            "--jars" => cache = value().into(),
            "--update" => update = true,
            other => fail(&format!("unknown argument {other}\n{USAGE}")),
        }
    }

    let mut fetcher = Fetcher::new(&cache);
    fetcher.update = update;
    // Order matters: mods replace recipes of the game and extend tags from NeoForge.
    let mut jars = vec![
        fetcher.minecraft(&game).unwrap_or_else(|e| fail(&e)),
        fetcher.neoforge(&game).unwrap_or_else(|e| fail(&e)),
    ];
    for spec in &mods {
        jars.push(fetcher.modrinth(spec, &game, "neoforge").unwrap_or_else(|e| fail(&e)));
    }

    let refs: Vec<&Path> = jars.iter().map(PathBuf::as_path).collect();
    let (mut bundle, stats) = import_jars(&refs).unwrap_or_else(|e| fail(&e));
    // Item names fall back to English, so missing translations must not stop the recipes.
    let russian = fetcher.minecraft_lang(&game, "ru_ru").and_then(|path| {
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    });
    match russian {
        Ok(lang_file) => add_names(&mut bundle.names, &lang_file, true),
        Err(e) => eprintln!("no Russian names for Minecraft items: {e}"),
    }
    if bundle.recipes.is_empty() {
        fail("no recipes were found in the downloaded files");
    }
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| fail(&format!("{}: {e}", dir.display())));
    }
    let json = serde_json::to_string(&bundle).unwrap_or_else(|e| fail(&e.to_string()));
    // A half written bundle would stop the server, so it only appears once complete.
    let part = out.with_extension("json.part");
    std::fs::write(&part, json)
        .and_then(|()| std::fs::rename(&part, &out))
        .unwrap_or_else(|e| fail(&format!("{}: {e}", out.display())));

    println!("recipes: {}, tags: {}, names: {}", stats.recipes, bundle.tags.len(), bundle.names.len());
    println!("skipped by condition: {}", stats.skipped_by_condition);
    if !stats.failed.is_empty() {
        println!("could not read {}:", stats.failed.len());
        for line in stats.failed.iter().take(20) {
            println!("  {line}");
        }
    }
    println!("written to {}", out.display());
}
