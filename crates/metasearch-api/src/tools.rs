use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::{Json, http::HeaderMap};
use serde::Deserialize;
use serde_json::{Value, json};

use metasearch::models::Category;

use crate::{AppError, AppResult, AppState};

/// GET /v1/extract?url=...&max_chars=...&query=... - readable-text
/// extraction of a page (the same feature as `ms extract`).
#[derive(Deserialize)]
pub struct ExtractParams {
    url: String,
    #[serde(default)]
    max_chars: Option<usize>,
    #[serde(default)]
    query: Option<String>,
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
}

fn header_key(headers: &HeaderMap) -> Option<String> {
    crate::api_key_from_headers(headers)
}

pub async fn extract_get(
    State(state): State<Arc<AppState>>,
    Query(p): Query<ExtractParams>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let key = header_key(&headers);
    if !state.authorized(key.as_deref()) {
        return Err(AppError::unauthorized());
    }
    run_extract(&state, &p).await
}

pub async fn extract_post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(p): Json<ExtractParams>,
) -> AppResult<impl IntoResponse> {
    let key = header_key(&headers);
    if !state.authorized(key.as_deref()) {
        return Err(AppError::unauthorized());
    }
    run_extract(&state, &p).await
}

async fn run_extract(
    state: &AppState,
    p: &ExtractParams,
) -> AppResult<Json<metasearch::ExtractedPage>> {
    let max_chars = p.max_chars.unwrap_or(5000).clamp(1, 100_000);
    let page =
        metasearch::extract(state.client.http(), &p.url, max_chars, p.query.as_deref()).await?;
    Ok(Json(page))
}

pub async fn test(
    State(state): State<Arc<AppState>>,
    Query(p): Query<TestParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let key = header_key(&headers);
    if !state.authorized(key.as_deref()) {
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
        let mut opts = metasearch::SearchOptions::new(query.clone());
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
