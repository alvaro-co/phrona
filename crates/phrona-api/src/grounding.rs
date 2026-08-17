//! AI grounding endpoint: extractive answers over ranked search results.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use phrona::SearchOptions;
use phrona::models::{Category, TimeRange};

use crate::{AppError, AppResult, AppState, HeaderAuth, JsonBody, JsonQuery};

/// POST /v1/grounding - AI grounding request; credentials via headers or
/// the JSON body.
#[derive(Deserialize)]
pub struct GroundingRequest {
    /// The search query to ground an answer on.
    pub query: String,
    #[serde(default)]
    /// Optional API key; the `Authorization` header is the preferred fallback.
    pub api_key: Option<String>,
    #[serde(default)]
    /// Maximum number of sources to return (clamped to 1..=50).
    pub max_results: Option<usize>,
    #[serde(default)]
    /// Optional search category (`web`, `images`, `news`, `videos`, `books`).
    pub category: Option<String>,
    #[serde(default)]
    /// Optional time range filter (`day`, `week`, `month`, `year`).
    pub time_range: Option<String>,
}

/// GET /v1/grounding?query=... - same feature; auth is header-only.
#[derive(Deserialize)]
pub struct GroundingGetParams {
    /// The search query to ground an answer on.
    pub query: String,
    #[serde(default)]
    /// Maximum number of sources to return (clamped to 1..=50).
    pub max_results: Option<usize>,
    #[serde(default)]
    /// Optional search category (`web`, `images`, `news`, `videos`, `books`).
    pub category: Option<String>,
    #[serde(default)]
    /// Optional time range filter (`day`, `week`, `month`, `year`).
    pub time_range: Option<String>,
}

/// The response to a grounding request: an extractive answer plus the
/// ranked sources it was built from.
#[derive(Serialize)]
pub struct GroundingResponse {
    /// The query the answer was built for.
    pub query: String,
    /// Extractive answer synthesized from the sources.
    pub answer: String,
    /// Ranked sources supporting the answer.
    pub sources: Vec<GroundingSource>,
    /// Wall-clock time spent searching, in seconds.
    pub response_time: f64,
}

/// One ranked source for a grounding answer.
#[derive(Serialize)]
pub struct GroundingSource {
    /// Page title of the source.
    pub title: String,
    /// URL of the source.
    pub url: String,
    /// Readable text content used for grounding.
    pub content: String,
    /// Positional relevance score (1.0 down to 0.05).
    pub score: f64,
}

/// Build an extractive answer from the top results. The library answer
/// (e.g. from grokipedia) takes precedence; otherwise the strongest
/// snippets are stitched together without claiming LLM generation.
fn synthesize_answer(resp: &phrona::SearchResponse, sources: &[GroundingSource]) -> String {
    if let Some(a) = &resp.answer
        && !a.trim().is_empty()
    {
        return a.clone();
    }
    if sources.is_empty() {
        return format!("No results found for \"{}\".", resp.query);
    }
    let mut parts = Vec::new();
    for (i, s) in sources.iter().take(3).enumerate() {
        let content = s.content.trim();
        if content.is_empty() {
            continue;
        }
        let excerpt = content.chars().take(400).collect::<String>();
        parts.push(format!("Source {} ({}): {excerpt}", i + 1, s.url));
    }
    if parts.is_empty() {
        return format!("{} sources found for \"{}\".", sources.len(), resp.query);
    }
    format!(
        "Extractive summary for \"{}\":\n{}",
        resp.query,
        parts.join("\n")
    )
}

/// `GET /v1/grounding?query=...`: header-auth variant of the grounding
/// endpoint.
pub async fn get(
    State(state): State<Arc<AppState>>,
    auth: HeaderAuth,
    JsonQuery(p): JsonQuery<GroundingGetParams>,
) -> AppResult<impl IntoResponse> {
    if !state.authorized(auth.key()) {
        return Err(AppError::unauthorized());
    }
    run(
        &state,
        &p.query,
        p.max_results,
        p.category.as_deref(),
        p.time_range.as_deref(),
    )
    .await
}

/// `POST /v1/grounding`: body variant of the grounding endpoint; the API key
/// may come from the `Authorization` header or the JSON body.
pub async fn post(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    JsonBody(p): JsonBody<GroundingRequest>,
) -> AppResult<impl IntoResponse> {
    if !state.authorized(crate::auth_key(&headers, p.api_key.as_deref()).as_deref()) {
        return Err(AppError::unauthorized());
    }
    run(
        &state,
        &p.query,
        p.max_results,
        p.category.as_deref(),
        p.time_range.as_deref(),
    )
    .await
}

async fn run(
    state: &AppState,
    query: &str,
    max_results: Option<usize>,
    category: Option<&str>,
    time_range: Option<&str>,
) -> AppResult<Json<GroundingResponse>> {
    let mut opts = SearchOptions::new(query.to_string());
    opts.max_results = max_results.unwrap_or(10).clamp(1, 50);
    if let Some(c) = category {
        opts.category = c.parse::<Category>().map_err(|_| {
            AppError::bad_request(
                "invalid category, expected one of: web, images, news, videos, books",
            )
        })?;
    }
    if let Some(t) = time_range {
        opts.time_range = Some(t.parse::<TimeRange>().map_err(|_| {
            AppError::bad_request("invalid time_range, expected day|week|month|year")
        })?);
    }

    let started = std::time::Instant::now();
    let resp = state.client.search(opts).await?;
    let response_time = started.elapsed().as_secs_f64();

    let mut sources: Vec<GroundingSource> = Vec::new();
    for (i, r) in resp.results.iter().enumerate() {
        let score = (1.0 - i as f64 * 0.05).max(0.05);
        let (title, url, content) = match r {
            phrona::ResultItem::Web(w) => (&w.title, &w.url, &w.description),
            phrona::ResultItem::News(n) => (&n.title, &n.url, &n.description),
            phrona::ResultItem::Video(v) => (&v.title, &v.url, &v.description),
            phrona::ResultItem::Image(i) => (&i.title, &i.url, &i.source),
            phrona::ResultItem::Book(b) => (&b.title, &b.url, &b.info),
        };
        sources.push(GroundingSource {
            title: title.clone(),
            url: url.clone(),
            content: content.clone(),
            score,
        });
    }

    let answer = synthesize_answer(&resp, &sources);
    Ok(Json(GroundingResponse {
        query: resp.query.clone(),
        answer,
        sources,
        response_time,
    }))
}
