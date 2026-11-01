//! HTTP API over the core. See docs/api.md.
//!
//! Usage: `createcalc-server [bundle.json] [address]`, defaults `data/bundle.json 127.0.0.1:3000`.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use createcalc_core::config::Config;
use createcalc_core::model::{Bundle, Catalog};
use createcalc_core::plan::{plan, Plan, PlanError, PlanRequest};
use createcalc_core::power::{plan_power, PowerPlan};
use serde::{Deserialize, Serialize};
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

async fn recipe(State(cat): State<Shared>, axum::extract::Path(id): axum::extract::Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    let r = cat.recipe(&id).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("no recipe {id}")))?;
    Ok(Json(serde_json::to_value(r).unwrap_or_default()))
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

fn app(cat: Shared) -> Router {
    Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/items", get(items))
        .route("/api/recipes/{*id}", get(recipe))
        .route("/api/config/default", get(default_config))
        .route("/api/config/parse", post(parse_config))
        .route("/api/plan", post(make_plan))
        .route("/api/power", post(power))
        .layer(CorsLayer::permissive())
        .with_state(cat)
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "data/bundle.json".into());
    let address = args.next().unwrap_or_else(|| "127.0.0.1:3000".into());
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}. Build it first, see docs/import.md");
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
    if let Err(e) = axum::serve(listener, app(Arc::new(Catalog::new(bundle)))).await {
        eprintln!("server stopped: {e}");
        std::process::exit(1);
    }
}
