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
    /// Optional search category (`web`, `images`, `news`, `videos`,
    /// `books`, `code`, `papers`, `archives`).
    pub category: Option<String>,
    #[serde(default)]
    /// Optional time range filter (`day`, `week`, `month`, `year`).
    pub time_range: Option<String>,
    #[serde(default)]
    /// Optional comma-separated engine restriction (as in `/v1/search`).
    pub engines: Option<String>,
    #[serde(default)]
    /// Optional region hint (`us-en`).
    pub region: Option<String>,
    #[serde(default)]
    /// Optional language hint (`en`).
    pub language: Option<String>,
    #[serde(default)]
    /// Optional safesearch level (`off`, `moderate`, `strict`).
    pub safesearch: Option<String>,
    #[serde(default)]
    /// Optional engine-specific filter string.
    pub filters: Option<String>,
    #[serde(default)]
    /// Result page (1-based), like `phrona ground --page`.
    pub page: Option<u32>,
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
    /// Optional search category (`web`, `images`, `news`, `videos`,
    /// `books`, `code`, `papers`, `archives`).
    pub category: Option<String>,
    #[serde(default)]
    /// Optional time range filter (`day`, `week`, `month`, `year`).
    pub time_range: Option<String>,
    #[serde(default)]
    /// Optional comma-separated engine restriction (as in `/v1/search`).
    pub engines: Option<String>,
    #[serde(default)]
    /// Optional region hint (`us-en`).
    pub region: Option<String>,
    #[serde(default)]
    /// Optional language hint (`en`).
    pub language: Option<String>,
    #[serde(default)]
    /// Optional safesearch level (`off`, `moderate`, `strict`).
    pub safesearch: Option<String>,
    #[serde(default)]
    /// Optional engine-specific filter string.
    pub filters: Option<String>,
    #[serde(default)]
    /// Result page (1-based).
    pub page: Option<u32>,
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
    run(&state, &params_from_get(&p)).await
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
    run(&state, &params_from_post(&p)).await
}

/// Shared option set between the GET and POST variants (minus auth).
struct GroundingParams {
    query: String,
    page: Option<u32>,
    max_results: Option<usize>,
    category: Option<String>,
    time_range: Option<String>,
    engines: Option<String>,
    region: Option<String>,
    language: Option<String>,
    safesearch: Option<String>,
    filters: Option<String>,
}

fn params_from_get(p: &GroundingGetParams) -> GroundingParams {
    GroundingParams {
        query: p.query.clone(),
        page: p.page,
        max_results: p.max_results,
        category: p.category.clone(),
        time_range: p.time_range.clone(),
        engines: p.engines.clone(),
        region: p.region.clone(),
        language: p.language.clone(),
        safesearch: p.safesearch.clone(),
        filters: p.filters.clone(),
    }
}

fn params_from_post(p: &GroundingRequest) -> GroundingParams {
    GroundingParams {
        query: p.query.clone(),
        page: p.page,
        max_results: p.max_results,
        category: p.category.clone(),
        time_range: p.time_range.clone(),
        engines: p.engines.clone(),
        region: p.region.clone(),
        language: p.language.clone(),
        safesearch: p.safesearch.clone(),
        filters: p.filters.clone(),
    }
}

async fn run(state: &AppState, p: &GroundingParams) -> AppResult<Json<GroundingResponse>> {
    let mut opts = SearchOptions::new(p.query.clone());
    opts.page = p.page.unwrap_or(1).max(1);
    // aligned with the CLI and web UI ground default
    opts.max_results = p.max_results.unwrap_or(8).clamp(1, 50);
    if let Some(c) = &p.category {
        opts.category = c.parse::<Category>().map_err(|_| {
            AppError::bad_request(format!(
                "invalid category, expected one of: {}",
                Category::list_str()
            ))
        })?;
    }
    if let Some(t) = &p.time_range {
        opts.time_range = Some(t.parse::<TimeRange>().map_err(|_| {
            AppError::bad_request("invalid time_range, expected day|week|month|year")
        })?);
    }
    if let Some(e) = &p.engines {
        opts.engines = e
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for name in &opts.engines {
            if phrona::engine::engine_by_name(name).is_none() {
                return Err(AppError::bad_request(format!(
                    "unknown engine '{name}'; see /v1/engines"
                )));
            }
        }
    }
    opts.region = p.region.clone().filter(|r| !r.trim().is_empty());
    opts.language = p.language.clone().filter(|l| !l.trim().is_empty());
    if let Some(s) = &p.safesearch {
        opts.safesearch = s.parse::<phrona::SafeSearch>().map_err(|_| {
            AppError::bad_request("invalid safesearch, expected off|moderate|strict")
        })?;
    }
    opts.filters = p.filters.clone().filter(|f| !f.trim().is_empty());

    let started = std::time::Instant::now();
    let resp = state.client.search(opts).await?;
    let response_time = started.elapsed().as_secs_f64();

    let mut sources: Vec<GroundingSource> = Vec::new();
    for (i, r) in resp.results.iter().enumerate() {
        let score = phrona::rank::positional_score(i);
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
