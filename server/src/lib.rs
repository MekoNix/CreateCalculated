//! HTTP API over the core, and the calculator page.

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use createcalc_core::config::Config;
use createcalc_core::model::Catalog;
use createcalc_core::plan::{consumers, plan, producers, Plan, PlanError, PlanRequest};
use createcalc_core::power::{plan_power, PowerPlan};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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
    /// Only another look of an item that stands for it, a search leaves these out.
    skin: bool,
}

async fn items(State(cat): State<Shared>, Query(query): Query<ItemQuery>) -> Json<Vec<Item>> {
    let lang = query.lang.as_deref().unwrap_or("ru");
    let needle = query.q.to_lowercase();
    let limit = query.limit.unwrap_or(50);
    let mut list: Vec<Item> = cat
        .resources()
        .into_iter()
        // A search finds one item per thing. The full list keeps the rest, a recipe may name them.
        .filter(|id| needle.is_empty() || !cat.is_skin(id))
        .map(|id| Item { name: cat.name(&id, lang), has_recipe: !cat.producer_indexes(&id).is_empty(), skin: cat.is_skin(&id), id })
        .filter(|i| needle.is_empty() || i.id.contains(&needle) || i.name.to_lowercase().contains(&needle))
        .collect();
    // The closest names first, or the limit could cut off the item a short word asks for.
    list.sort_by_cached_key(|i| {
        let name = i.name.to_lowercase();
        if name == needle { 0 } else if name.starts_with(&needle) { 1 } else { 2 }
    });
    list.truncate(limit);
    Json(list)
}

/// Which item stands for a tag when any member would do.
async fn tag_defaults(State(cat): State<Shared>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(cat.tag_defaults()).unwrap_or_default())
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

/// Pictures live on the user's machine. Without them this is a 404 and
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
const PAGE: &str = include_str!("../page.html");

/// The whole site. Pictures are read from `icons`, the rest is in memory.
pub fn app(cat: Arc<Catalog>, icons: PathBuf) -> Router {
    let pictures = Router::new().route("/api/icons/{*path}", get(icon_file)).with_state(Arc::new(icons));
    Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/items", get(items))
        .route("/api/tags/defaults", get(tag_defaults))
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

#[cfg(test)]
mod tests {
    use super::*;

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
