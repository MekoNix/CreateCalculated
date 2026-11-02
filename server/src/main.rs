//! HTTP API over the core. See docs/api.md.
//!
//! Usage: `createcalc-server [bundle.json] [address] [icons folder]`, defaults
//! `data/bundle.json 127.0.0.1:3000 data/icons`.

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use createcalc_core::config::Config;
use createcalc_core::model::{Bundle, Catalog};
use createcalc_core::plan::{consumers, plan, producers, Plan, PlanError, PlanRequest};
use createcalc_core::power::{plan_power, PowerPlan};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::{Path as FsPath, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

type Shared = Arc<Catalog>;

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

#[derive(Deserialize)]
struct ItemQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Serialize)]
struct Item {
    id: String,
    name: String,
    has_recipe: bool,
}

async fn items(State(cat): State<Shared>, Query(query): Query<ItemQuery>) -> Json<Vec<Item>> {
    let lang = query.lang.as_deref().unwrap_or("ru");
    let needle = query.q.to_lowercase();
    let limit = query.limit.unwrap_or(50);
    let list = cat
        .resources()
        .into_iter()
        .map(|id| Item { name: cat.name(&id, lang), has_recipe: !cat.producer_indexes(&id).is_empty(), id })
        .filter(|i| needle.is_empty() || i.id.contains(&needle) || i.name.to_lowercase().contains(&needle))
        .take(limit)
        .collect();
    Json(list)
}

async fn recipe(State(cat): State<Shared>, Path(id): Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    let r = cat.recipe(&id).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("no recipe {id}")))?;
    Ok(Json(serde_json::to_value(r).unwrap_or_default()))
}

#[derive(Deserialize)]
struct ProducerQuery {
    item: String,
}

/// Recipes that can make an item, those with it as the main output first.
async fn item_producers(State(cat): State<Shared>, Query(query): Query<ProducerQuery>) -> Json<serde_json::Value> {
    let list = producers(&cat, &query.item, &Config::default());
    Json(serde_json::to_value(list).unwrap_or_default())
}

/// Recipes that use an item, directly or through a tag.
async fn item_consumers(State(cat): State<Shared>, Query(query): Query<ProducerQuery>) -> Json<serde_json::Value> {
    let list = consumers(&cat, &query.item, &Config::default());
    Json(serde_json::to_value(list).unwrap_or_default())
}

async fn default_config() -> Json<Config> {
    Json(Config::default())
}

/// Reads a user's `create-server.toml` into a config the client sends back with plans.
async fn parse_config(body: String) -> Result<Json<Config>, ApiError> {
    let mut config = Config::default();
    config.apply_server_toml(&body).map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(config))
}

async fn make_plan(State(cat): State<Shared>, Json(req): Json<PlanRequest>) -> Result<Json<Plan>, ApiError> {
    plan(&cat, &req).map(Json).map_err(|e| match e {
        PlanError::Invalid(_) => ApiError(StatusCode::BAD_REQUEST, e.to_string()),
        PlanError::Solver(_) => ApiError(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
    })
}

#[derive(Deserialize)]
struct PowerRequest {
    su: f64,
    #[serde(default = "default_sails")]
    sails: u32,
    #[serde(default)]
    config: Config,
}

fn default_sails() -> u32 {
    64
}

async fn power(Json(req): Json<PowerRequest>) -> Result<Json<PowerPlan>, ApiError> {
    if !req.su.is_finite() || req.su < 0.0 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "su must be zero or more".into()));
    }
    Ok(Json(plan_power(req.su, req.sails, &req.config)))
}

/// Only names createcalc-icons writes, so a request cannot reach anything else on the disk.
fn is_icon_path(path: &str) -> bool {
    let plain = |part: &str| !part.is_empty() && !part.starts_with('.') && part.bytes().all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'));
    (path == "index.json" || path.ends_with(".png")) && path.split('/').all(plain)
}

/// Pictures live on the user's machine, see docs/icons.md. Without them this is a 404 and
/// the page keeps its letter placeholders. The page asks with the index version in the
/// URL, so pictures can be cached for good while the index is always checked.
async fn icon_file(State(folder): State<Arc<PathBuf>>, Path(path): Path<String>) -> Response {
    if !is_icon_path(&path) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(bytes) = std::fs::read(folder.join(&path)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (kind, cache) = if path == "index.json" { ("application/json", "no-cache") } else { ("image/png", "public, max-age=31536000, immutable") };
    ([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, cache)], bytes).into_response()
}

/// The page is built in, so opening the server's address is all a player on the network needs.
const PAGE: &str = include_str!("../../mockups/graph.html");

fn app(cat: Shared, icons: PathBuf) -> Router {
    let pictures = Router::new().route("/api/icons/{*path}", get(icon_file)).with_state(Arc::new(icons));
    Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/items", get(items))
        .route("/api/recipes/{*id}", get(recipe))
        .route("/api/producers", get(item_producers))
        .route("/api/consumers", get(item_consumers))
        .route("/api/config/default", get(default_config))
        .route("/api/config/parse", post(parse_config))
        .route("/api/plan", post(make_plan))
        .route("/api/power", post(power))
        .with_state(cat)
        .merge(pictures)
        .layer(CorsLayer::permissive())
}

/// How to run the picture tool: the binary next to this one if it was built, otherwise through
/// cargo, because `cargo run --bin createcalc-server` builds only the server.
fn icons_command(exe_dir: Option<&FsPath>) -> (OsString, Vec<&'static str>) {
    let sibling = exe_dir.map(|d| d.join(format!("createcalc-icons{}", std::env::consts::EXE_SUFFIX)));
    match sibling {
        Some(file) if file.is_file() => (file.into_os_string(), vec![]),
        _ => ("cargo".into(), vec!["run", "--release", "--bin", "createcalc-icons", "--"]),
    }
}

/// Offers to build the pictures when there are none. Only a person at a terminal is asked,
/// and a refusal is remembered so the question does not come back on every start.
fn offer_icons(icons: &FsPath, bundle: &str) {
    let declined = icons.join(".no-install-prompt");
    if icons.join("index.json").is_file() {
        return;
    }
    let hint = "Картинок предметов нет, на странице будут буквы. Установить: cargo run --release --bin createcalc-icons";
    if declined.exists() || !std::io::stdin().is_terminal() {
        println!("{hint}");
        return;
    }
    println!("Картинок предметов нет, на странице будут буквы.");
    println!("Скачать Minecraft и Create (примерно 50 МБ) и собрать картинки сейчас? Они останутся только на этом компьютере. [Y/n]");
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() || !matches!(answer.trim().to_lowercase().as_str(), "" | "y" | "yes" | "д" | "да") {
        if std::fs::create_dir_all(icons).and_then(|()| std::fs::write(&declined, "")).is_err() {
            eprintln!("не удалось запомнить ответ, вопрос повторится при следующем запуске");
        }
        println!("{hint}");
        return;
    }
    let exe = std::env::current_exe().ok();
    let (program, args) = icons_command(exe.as_deref().and_then(FsPath::parent));
    let status = Command::new(program).args(args).arg("--out").arg(icons).arg("--bundle").arg(bundle).status();
    if !matches!(status, Ok(s) if s.success()) {
        eprintln!("Картинки не собрались, сервер запустится с буквами. {hint}");
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "data/bundle.json".into());
    let address = args.next().unwrap_or_else(|| "127.0.0.1:3000".into());
    let icons = PathBuf::from(args.next().unwrap_or_else(|| "data/icons".into()));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}. Build it first, see docs/import.md");
        std::process::exit(1)
    });
    let bundle: Bundle = serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("{path} is not a valid bundle: {e}");
        std::process::exit(1)
    });
    println!("{} recipes loaded", bundle.recipes.len());
    offer_icons(&icons, &path);
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
    fn picture_tool_is_found_next_to_the_server_or_through_cargo() {
        let folder = std::env::temp_dir().join(format!("createcalc-tool-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let (program, args) = icons_command(Some(&folder));
        assert_eq!(program, "cargo");
        assert!(args.contains(&"createcalc-icons"));
        assert_eq!(icons_command(None).0, "cargo");

        let tool = folder.join(format!("createcalc-icons{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&tool, "").unwrap();
        let (program, args) = icons_command(Some(&folder));
        assert_eq!(program, tool.into_os_string());
        assert!(args.is_empty());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn icon_paths_cannot_leave_the_folder() {
        assert!(is_icon_path("index.json"));
        assert!(is_icon_path("items/create/cogwheel.png"));
        assert!(is_icon_path("fluids/create/potion/minecraft_strength.png"));
        for bad in ["../secret.png", "items/../../secret.png", "/etc/passwd.png", "items//x.png", "items/.hidden.png", "bundle.json", "items/x.txt", "items\\x.png", ""] {
            assert!(!is_icon_path(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn icons_are_served_when_present_and_404_otherwise() {
        let folder = std::env::temp_dir().join(format!("createcalc-server-test-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("items/test")).unwrap();
        std::fs::write(folder.join("items/test/a.png"), [1, 2, 3]).unwrap();
        std::fs::write(folder.join("index.json"), "{}").unwrap();
        std::fs::write(folder.join("private.txt"), "no").unwrap();
        let state = Arc::new(folder.clone());
        let ask = |path: &str| icon_file(State(state.clone()), Path(path.to_string()));

        let png = ask("items/test/a.png").await;
        assert_eq!(png.status(), StatusCode::OK);
        assert_eq!(png.headers()[header::CONTENT_TYPE], "image/png");
        assert!(png.headers()[header::CACHE_CONTROL].to_str().unwrap().contains("immutable"));
        let index = ask("index.json").await;
        assert_eq!(index.status(), StatusCode::OK);
        assert_eq!(index.headers()[header::CACHE_CONTROL], "no-cache");
        assert_eq!(ask("items/test/missing.png").await.status(), StatusCode::NOT_FOUND);
        assert_eq!(ask("private.txt").await.status(), StatusCode::NOT_FOUND);

        std::fs::remove_dir_all(&folder).unwrap();
        assert_eq!(ask("index.json").await.status(), StatusCode::NOT_FOUND);
    }
}
