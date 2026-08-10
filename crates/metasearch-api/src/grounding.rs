use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::{Json, http::HeaderMap};
use serde::{Deserialize, Serialize};

use metasearch::SearchOptions;
use metasearch::models::{Category, TimeRange};

use crate::{AppError, AppResult, AppState};

/// AI grounding endpoint: returns the top sources plus an extractive
/// answer for a query, ready for retrieval-augmented generation.
#[derive(Deserialize)]
pub struct GroundingRequest {
    pub query: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub max_results: Option<usize>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub time_range: Option<String>,
}

#[derive(Serialize)]
pub struct GroundingResponse {
    pub query: String,
    pub answer: String,
    pub sources: Vec<GroundingSource>,
    pub response_time: f64,
}

#[derive(Serialize)]
pub struct GroundingSource {
    pub title: String,
    pub url: String,
    pub content: String,
    pub score: f64,
}

/// Build an extractive answer from the top results. The library answer
/// (e.g. from grokipedia) takes precedence; otherwise the strongest
/// snippets are stitched together without claiming LLM generation.
fn synthesize_answer(resp: &metasearch::SearchResponse, sources: &[GroundingSource]) -> String {
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

pub async fn get(
    State(state): State<Arc<AppState>>,
    Query(p): Query<GroundingRequest>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let header_key = crate::api_key_from_headers(&headers);
    let key = p.api_key.as_deref().or(header_key.as_deref());
    if !state.authorized(key) {
        return Err(AppError::unauthorized());
    }
    run(&state, &p).await
}

pub async fn post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(p): Json<GroundingRequest>,
) -> AppResult<impl IntoResponse> {
    let header_key = crate::api_key_from_headers(&headers);
    let key = p.api_key.as_deref().or(header_key.as_deref());
    if !state.authorized(key) {
        return Err(AppError::unauthorized());
    }
    run(&state, &p).await
}

async fn run(state: &AppState, p: &GroundingRequest) -> AppResult<Json<GroundingResponse>> {
    let mut opts = SearchOptions::new(p.query.clone());
    opts.max_results = p.max_results.unwrap_or(10).clamp(1, 50);
    if let Some(c) = &p.category {
        opts.category = c.parse::<Category>().map_err(|_| {
            AppError::bad_request(
                "invalid category, expected one of: web, images, news, videos, books",
            )
        })?;
    }
    if let Some(t) = &p.time_range {
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
            metasearch::ResultItem::Web(w) => (&w.title, &w.url, &w.description),
            metasearch::ResultItem::News(n) => (&n.title, &n.url, &n.description),
            metasearch::ResultItem::Video(v) => (&v.title, &v.url, &v.description),
            metasearch::ResultItem::Image(i) => (&i.title, &i.url, &i.source),
            metasearch::ResultItem::Book(b) => (&b.title, &b.url, &b.info),
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
