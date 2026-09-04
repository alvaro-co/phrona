//! Page extraction and engine-test endpoints.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{Value, json};

use phrona::models::Category;

use crate::{AppError, AppResult, AppState, HeaderAuth, JsonBody, JsonQuery};

/// GET /v1/extract?url=...&max_chars=...&query=... - readable-text
/// extraction of a page (the same feature as `phrona extract`). Auth is
/// header-only: query-string credentials are rejected.
#[derive(Deserialize)]
pub struct ExtractGetParams {
    url: String,
    #[serde(default)]
    max_chars: Option<usize>,
    #[serde(default)]
    query: Option<String>,
}

/// POST /v1/extract - same feature, credentials via headers or body.
#[derive(Deserialize)]
pub struct ExtractPostParams {
    url: String,
    #[serde(default)]
    max_chars: Option<usize>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
}

/// GET /v1/test?query=...&category=...&max_results=... - availability probe
/// across every category (the same feature as `phrona test`).
#[derive(Deserialize)]
pub struct TestParams {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    max_results: Option<usize>,
}

/// `GET /v1/extract?url=...`: header-auth variant of the page extraction
/// endpoint.
pub async fn extract_get(
    State(state): State<Arc<AppState>>,
    auth: HeaderAuth,
    JsonQuery(p): JsonQuery<ExtractGetParams>,
) -> AppResult<impl IntoResponse> {
    if !state.authorized(auth.key()) {
        return Err(AppError::unauthorized());
    }
    run_extract(&state, &p.url, p.max_chars, p.query.as_deref()).await
}

/// `POST /v1/extract`: body variant of the page extraction endpoint.
pub async fn extract_post(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    JsonBody(p): JsonBody<ExtractPostParams>,
) -> AppResult<impl IntoResponse> {
    if !state.authorized(crate::auth_key(&headers, p.api_key.as_deref()).as_deref()) {
        return Err(AppError::unauthorized());
    }
    run_extract(&state, &p.url, p.max_chars, p.query.as_deref()).await
}

async fn run_extract(
    state: &AppState,
    url: &str,
    max_chars: Option<usize>,
    query: Option<&str>,
) -> AppResult<Json<phrona::ExtractedPage>> {
    let max_chars = max_chars.unwrap_or(5000).clamp(1, 100_000);
    let page = phrona::extract(state.client.http(), url, max_chars, query).await?;
    Ok(Json(page))
}

/// `GET /v1/test`: probe engine availability across every category (or a
/// single one) and return a per-category/per-engine report.
pub async fn test(
    State(state): State<Arc<AppState>>,
    auth: HeaderAuth,
    JsonQuery(p): JsonQuery<TestParams>,
) -> AppResult<Json<Value>> {
    if !state.authorized(auth.key()) {
        return Err(AppError::unauthorized());
    }
    let cats: Vec<Category> = match p.category.as_deref() {
        Some(c) => vec![c.parse::<Category>().map_err(|_| {
            AppError::bad_request(format!(
                "invalid category, expected one of: {}",
                Category::list_str()
            ))
        })?],
        None => Category::ALL.to_vec(),
    };
    let query = p.query.unwrap_or_else(|| "rust programming".to_string());
    let max_results = p.max_results.unwrap_or(5).clamp(1, 10);

    // categories probe concurrently: sequential probing stacks 8
    // per-category deadlines into a minute-plus response (`join_all`
    // preserves input order, so the report stays stable)
    let client = &state.client;
    let probes = cats.into_iter().map(|cat| {
        let mut opts = phrona::SearchOptions::new(query.clone());
        opts.category = cat;
        opts.max_results = max_results;
        // availability probing must observe every engine, not stop at the
        // first ones that fill max_results
        opts.probe_all = true;
        async move {
            match client.search(opts).await {
                Ok(resp) => json!({
                    "category": cat.as_str(),
                    "total": resp.total,
                    "elapsed_ms": resp.elapsed_ms,
                    "answer": resp.answer,
                    "engines": resp.engines,
                }),
                Err(e) => json!({
                    "category": cat.as_str(),
                    "total": 0,
                    "elapsed_ms": 0,
                    "answer": null,
                    "engines": [],
                    "error": e.to_string(),
                }),
            }
        }
    });
    let out: Vec<Value> = futures::future::join_all(probes).await;
    Ok(Json(Value::Array(out)))
}
