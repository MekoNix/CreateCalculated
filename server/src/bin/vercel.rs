//! The calculator as a Vercel function: the same API, with the recipes built into the program
//! because a function has no data folder. Pictures and the page are static files there, see
//! packaging/vercel.sh. Built only with `--features vercel`.

use createcalc_core::model::{Bundle, Catalog};
use createcalc_server::app;
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceBuilder;
use vercel_runtime::axum::VercelLayer;

const BUNDLE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../data/bundle.json"));

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    let bundle: Bundle = serde_json::from_str(BUNDLE)?;
    let router = app(Arc::new(Catalog::new(bundle)), PathBuf::new());
    let service = ServiceBuilder::new().layer(VercelLayer::new()).service(router);
    vercel_runtime::run(service).await
}
