//! Runs the calculator: checks that recipes and pictures are there, then serves them.
//!
//! Usage: see `USAGE`.

use createcalc_core::model::{Bundle, Catalog};
use createcalc_server::app;
use std::ffi::{OsStr, OsString};
use std::io::IsTerminal;
use std::path::{Path as FsPath, PathBuf};
use std::process::Command;
use std::sync::Arc;

const USAGE: &str = "usage: createcalc-server [bundle.json] [address] [icons folder]

Serves the calculator page and its API. Defaults: data/bundle.json 127.0.0.1:3000 data/icons.
Use the address 0.0.0.0:3000 to reach it from other computers. There is no login.
When recipes or pictures are missing it offers to build them, without a terminal it stops.

  -h, --help     show this text
  -V, --version  show the version
";

/// How to run one of the setup tools: the binary next to this one if it was built, otherwise
/// through cargo, because `cargo run --bin createcalc-server` builds only the server.
fn tool_command(tool: &str, exe_dir: Option<&FsPath>) -> (OsString, Vec<String>) {
    let sibling = exe_dir.map(|d| d.join(format!("{tool}{}", std::env::consts::EXE_SUFFIX)));
    match sibling {
        Some(file) if file.is_file() => (file.into_os_string(), vec![]),
        _ => ("cargo".into(), ["run", "--release", "--bin", tool, "--"].map(String::from).to_vec()),
    }
}

/// Runs a setup tool, handing it the terminal. Returns whether it succeeded.
fn run_tool(tool: &str, args: &[&OsStr]) -> bool {
    let exe = std::env::current_exe().ok();
    let (program, first) = tool_command(tool, exe.as_deref().and_then(FsPath::parent));
    matches!(Command::new(program).args(first).args(args).status(), Ok(s) if s.success())
}

/// Setup messages follow the language of the terminal, the page has its own switch.
fn russian() -> bool {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|v| std::env::var(v).ok().filter(|l| !l.is_empty()));
    locale.is_some_and(|l| l.starts_with("ru"))
}

/// The command that runs a tool from here, for messages.
fn tool_hint(tool: &str) -> String {
    let exe = std::env::current_exe().ok();
    let (program, args) = tool_command(tool, exe.as_deref().and_then(FsPath::parent));
    let mut line = vec![program.to_string_lossy().into_owned()];
    line.extend(args);
    line.pop_if(|last| last == "--");
    line.join(" ")
}

fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_lowercase().as_str(), "" | "y" | "yes" | "д" | "да")
}

/// Asks at the terminal, the default being yes. Without a terminal nobody is there to answer.
fn ask(question: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    println!("{question} [Y/n]");
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok() && is_yes(&answer)
}

/// The server needs recipes and pictures, so it offers to build what is missing or stops.
fn ensure_data(bundle: &str, icons: &FsPath) {
    let no_recipes = !FsPath::new(bundle).is_file();
    let no_pictures = !icons.join("index.json").is_file();
    if !no_recipes && !no_pictures {
        return;
    }
    let ru = russian();
    let mut hint = Vec::new();
    if no_recipes {
        println!("{}", if ru { format!("Рецептов нет: {bundle} не найден.") } else { format!("No recipes: {bundle} was not found.") });
        hint.push(tool_hint("createcalc-recipes"));
    }
    if no_pictures {
        let index = icons.join("index.json");
        println!("{}", if ru { format!("Картинок предметов нет: {} не найден.", index.display()) } else { format!("No item pictures: {} was not found.", index.display()) });
        hint.push(tool_hint("createcalc-icons"));
    }
    let hint = format!("{}\n{}", if ru { "Собрать вручную:" } else { "To build them by hand run:" }, hint.join("\n"));
    let question = if ru {
        "Скачать Minecraft, NeoForge и Create (примерно 60 МБ) и собрать рецепты и картинки сейчас? Файлы останутся только на этом компьютере."
    } else {
        "Download Minecraft, NeoForge and Create (about 60 MB) and build the recipes and pictures now? The files stay on this computer."
    };
    if !ask(question) {
        eprintln!("{hint}");
        std::process::exit(1);
    }
    // Pictures of tags come from the recipes, so those are built first.
    let built = (!no_recipes || (run_tool("createcalc-recipes", &[OsStr::new("--out"), OsStr::new(bundle)]) && FsPath::new(bundle).is_file()))
        && (!no_pictures || run_tool("createcalc-icons", &[OsStr::new("--out"), icons.as_os_str(), OsStr::new("--bundle"), OsStr::new(bundle)]));
    if !built {
        eprintln!("{} {hint}", if ru { "Собрать не удалось." } else { "The build failed." });
        std::process::exit(1);
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1).peekable();
    match args.peek().map(String::as_str) {
        Some("-h" | "--help") => {
            print!("{USAGE}");
            return;
        }
        Some("-V" | "--version") => {
            println!("createcalc-server {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some(flag) if flag.starts_with('-') => {
            eprintln!("unknown argument {flag}\n{USAGE}");
            std::process::exit(2);
        }
        _ => {}
    }
    let path = args.next().unwrap_or_else(|| "data/bundle.json".into());
    let address = args.next().unwrap_or_else(|| "127.0.0.1:3000".into());
    let icons = PathBuf::from(args.next().unwrap_or_else(|| "data/icons".into()));
    ensure_data(&path, &icons);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(1)
    });
    let bundle: Bundle = serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("{path} is not a valid bundle: {e}");
        std::process::exit(1)
    });
    println!("{} recipes loaded", bundle.recipes.len());
    let listener = tokio::net::TcpListener::bind(&address).await.unwrap_or_else(|e| {
        eprintln!("cannot bind {address}: {e}");
        std::process::exit(1)
    });
    println!("listening on http://{address}");
    if let Err(e) = axum::serve(listener, app(Arc::new(Catalog::new(bundle)), icons)).await {
        eprintln!("server stopped: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_are_found_next_to_the_server_or_through_cargo() {
        let folder = std::env::temp_dir().join(format!("createcalc-tool-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let (program, args) = tool_command("createcalc-recipes", Some(&folder));
        assert_eq!(program, "cargo");
        assert!(args.iter().any(|a| a == "createcalc-recipes"));
        assert_eq!(tool_command("createcalc-icons", None).0, "cargo");

        let tool = folder.join(format!("createcalc-recipes{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&tool, "").unwrap();
        let (program, args) = tool_command("createcalc-recipes", Some(&folder));
        assert_eq!(program, tool.into_os_string());
        assert!(args.is_empty());
        assert_eq!(tool_command("createcalc-icons", Some(&folder)).0, "cargo");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn answers() {
        for yes in ["", "y", "Y", "yes", "д", "Да", " y\n"] {
            assert!(is_yes(yes), "{yes}");
        }
        for no in ["n", "no", "нет", "x"] {
            assert!(!is_yes(no), "{no}");
        }
    }
}
