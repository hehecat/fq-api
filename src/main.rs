mod crypto;
mod fontmap;
mod models;
mod services;
mod state;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use serde::Deserialize;
use std::{net::SocketAddr, sync::Arc};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default = "default_page")]
    page: u32,
    #[serde(default = "default_size")]
    size: u32,
}

fn default_page() -> u32 {
    0
}
fn default_size() -> u32 {
    10
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,fq_api=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let state = Arc::new(AppState::new()?);
    if let Err(e) = state.crypto.load_key_from_disk() {
        tracing::warn!("load local aes key failed: {e:#}");
    } else {
        tracing::info!("crypto keys ready: {:?}", state.crypto.any_key().map(|(v, _)| v));
    }
    tracing::info!(
        "font map dir: {} (set FQ_FONT_MAP_DIR to override)",
        state.fonts.map_dir().display()
    );

    let app = Router::new()
        .route("/", get(health))
        .route("/health", get(health))
        .route("/api/search", get(search))
        .route("/api/book/{book_id}", get(book_detail))
        .route("/api/book/{book_id}/directory", get(directory))
        .route("/api/book/{book_id}/chapter/{item_id}", get(chapter))
        .route("/api/book/{book_id}/chapter/{item_id}/raw", get(chapter_raw))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(18080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("fq-api listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "service": "fq-api",
        "features": ["search", "book", "directory", "chapter", "font_decode"],
        "has_aes_key": state.crypto.any_key().is_some(),
        "key_version": state.crypto.any_key().map(|(v,_)| v),
        "font_map_dir": state.fonts.map_dir().display().to_string(),
    }))
}

async fn search(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SearchQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let page = q.page;
    let size = q.size.clamp(1, 30);
    let data = state.web.search_books(&q.q, page, size).await?;
    Ok(Json(serde_json::json!({ "code": 0, "data": data })))
}

async fn book_detail(
    State(state): State<Arc<AppState>>,
    Path(book_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let data = state.web.book_detail(&book_id).await?;
    Ok(Json(serde_json::json!({ "code": 0, "data": data })))
}

async fn directory(
    State(state): State<Arc<AppState>>,
    Path(book_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let data = state.web.directory(&book_id).await?;
    Ok(Json(serde_json::json!({ "code": 0, "data": data })))
}

async fn chapter(
    State(state): State<Arc<AppState>>,
    Path((book_id, item_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let data = state.chapter.get_chapter(&book_id, &item_id, false).await?;
    Ok(Json(serde_json::json!({ "code": 0, "data": data })))
}

async fn chapter_raw(
    State(state): State<Arc<AppState>>,
    Path((book_id, item_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let data = state.chapter.get_chapter(&book_id, &item_id, true).await?;
    Ok(Json(serde_json::json!({ "code": 0, "data": data })))
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    msg: String,
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            msg: format!("{e:#}"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = Json(serde_json::json!({
            "code": self.status.as_u16(),
            "message": self.msg
        }));
        (self.status, body).into_response()
    }
}
