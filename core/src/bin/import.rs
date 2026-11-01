//! Builds the recipe bundle from local JARs:
//! `createcalc-import -o data/bundle.json minecraft.jar neoforge-universal.jar create.jar`

use std::path::{Path, PathBuf};

fn main() {
    let mut output = PathBuf::from("data/bundle.json");
    let mut jars: Vec<PathBuf> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-o" {
            output = PathBuf::from(args.next().unwrap_or_else(|| fail("-o needs a path")));
        } else {
            jars.push(arg.into());
        }
    }
    if jars.is_empty() {
        fail("usage: createcalc-import [-o bundle.json] <minecraft.jar> <neoforge.jar> <mod.jar>...");
    }
    let refs: Vec<&Path> = jars.iter().map(PathBuf::as_path).collect();
    let (bundle, stats) = createcalc_core::import::import_jars(&refs).unwrap_or_else(|e| fail(&e));
    if let Some(dir) = output.parent() {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| fail(&e.to_string()));
    }
    let json = serde_json::to_string(&bundle).unwrap_or_else(|e| fail(&e.to_string()));
    std::fs::write(&output, json).unwrap_or_else(|e| fail(&e.to_string()));

    println!("recipes: {}", stats.recipes);
    println!("tags: {}, names: {}", bundle.tags.len(), bundle.names.len());
    println!("skipped by condition: {}", stats.skipped_by_condition);
    println!("skipped types: {:?}", stats.skipped_types);
    println!("unknown conditions: {:?}", stats.unknown_conditions);
    println!("failed: {}", stats.failed.len());
    for line in stats.failed.iter().take(30) {
        println!("  {line}");
    }
    println!("written to {}", output.display());
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1)
}
