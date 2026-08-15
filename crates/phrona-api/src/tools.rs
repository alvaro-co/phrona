use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{Value, json};

use phrona::models::Category;

use crate::{AppError, AppResult, AppState, JsonBody, JsonQuery};

/// GET /v1/extract?url=...&max_chars=...&query=... - readable-text
/// extraction of a page (the same feature as `ms extract`).
#[derive(Deserialize)]
pub struct ExtractParams {
    url: String,
    #[serde(default)]
    max_chars: Option<usize>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
}

/// GET /v1/test?query=...&category=...&max_results=... - availability probe
/// across every category (the same feature as `ms test`).
#[derive(Deserialize)]
pub struct TestParams {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    api_key: Option<String>,
}

pub async fn extract_get(
    State(state): State<Arc<AppState>>,
    JsonQuery(p): JsonQuery<ExtractParams>,
) -> AppResult<impl IntoResponse> {
    if !state.authorized(p.api_key.as_deref()) {
        return Err(AppError::unauthorized());
    }
    run_extract(&state, &p).await
}

pub async fn extract_post(
    State(state): State<Arc<AppState>>,
    JsonBody(p): JsonBody<ExtractParams>,
) -> AppResult<impl IntoResponse> {
    if !state.authorized(p.api_key.as_deref()) {
        return Err(AppError::unauthorized());
    }
    run_extract(&state, &p).await
}

async fn run_extract(
    state: &AppState,
    p: &ExtractParams,
) -> AppResult<Json<phrona::ExtractedPage>> {
    let max_chars = p.max_chars.unwrap_or(5000).clamp(1, 100_000);
    let page = phrona::extract(state.client.http(), &p.url, max_chars, p.query.as_deref()).await?;
    Ok(Json(page))
}

pub async fn test(
    State(state): State<Arc<AppState>>,
    JsonQuery(p): JsonQuery<TestParams>,
) -> AppResult<Json<Value>> {
    if !state.authorized(p.api_key.as_deref()) {
        return Err(AppError::unauthorized());
    }
    let cats: Vec<Category> = match p.category.as_deref() {
        Some(c) => vec![c.parse::<Category>().map_err(|_| {
            AppError::bad_request(
                "invalid category, expected one of: web, images, news, videos, books",
            )
        })?],
        None => Category::ALL.to_vec(),
    };
    let query = p.query.unwrap_or_else(|| "rust programming".to_string());
    let max_results = p.max_results.unwrap_or(5).clamp(1, 10);

    let mut out = Vec::new();
    for cat in cats {
        let mut opts = phrona::SearchOptions::new(query.clone());
        opts.category = cat;
        opts.max_results = max_results;
        match state.client.search(opts).await {
            Ok(resp) => out.push(json!({
                "category": cat.as_str(),
                "total": resp.total,
                "elapsed_ms": resp.elapsed_ms,
                "answer": resp.answer,
                "engines": resp.engines,
            })),
            Err(e) => out.push(json!({
                "category": cat.as_str(),
                "total": 0,
                "elapsed_ms": 0,
                "answer": null,
                "engines": [],
                "error": e.to_string(),
            })),
        }
    }
    Ok(Json(Value::Array(out)))
}
