//! Tavily-compatible `/search` endpoint.

use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::{Json, http::HeaderMap};
use serde::{Deserialize, Serialize};

use phrona::SearchOptions;
use phrona::models::{Category, ResultItem, TimeRange};

use crate::{AppError, AppResult, AppState, JsonBody};

/// Tavily-compatible request body.
///
/// The Tavily API (<https://docs.tavily.com>) is the de-facto standard for
/// AI search. Clients such as `tavily-python` can target this server by
/// setting `base_url` to it and calling `/search`.
#[derive(Deserialize)]
pub struct TavilyRequest {
    /// The search query.
    pub query: String,
    #[serde(default)]
    /// Optional API key accepted in the body for Tavily SDK compatibility.
    pub api_key: Option<String>,
    #[serde(default)]
    /// `basic` (two engines) or `advanced` (all engines).
    pub search_depth: Option<String>,
    #[serde(default)]
    /// Optional `news` topic; anything else searches the web.
    pub topic: Option<String>,
    #[serde(default)]
    /// Recent-window in days, mapped to a `TimeRange`.
    pub days: Option<u32>,
    #[serde(default)]
    /// Maximum number of results to return (clamped to 1..=20).
    pub max_results: Option<usize>,
    #[serde(default)]
    /// Whether to populate the `images` field via a dedicated image search.
    pub include_images: bool,
    #[serde(default)]
    /// Whether to return an answer (library answer, e.g. grokipedia).
    pub include_answer: bool,
    #[serde(default)]
    /// Whether to fetch and attach raw page text for each result.
    pub include_raw_content: bool,
    /// Accepted for compatibility with Tavily clients; only meaningful for
    /// Tavily's image-search endpoint, which this server does not expose.
    #[serde(default)]
    pub include_image_descriptions: bool,
    #[serde(default)]
    /// Domains to restrict results to (as `site:` filters).
    pub include_domains: Option<Vec<String>>,
    #[serde(default)]
    /// Domains to exclude (as `-site:` filters).
    pub exclude_domains: Option<Vec<String>>,
}

/// Tavily-compatible response body.
#[derive(Serialize)]
pub struct TavilyResponse {
    /// The query echoed back.
    pub query: String,
    /// Reserved for Tavily compatibility; always empty.
    pub follow_up_questions: Vec<String>,
    /// Wall-clock time spent searching, in seconds.
    pub response_time: f64,
    /// Optional answer, populated when `include_answer` is set.
    pub answer: Option<String>,
    /// Image URLs, populated when `include_images` is set.
    pub images: Option<Vec<String>>,
    /// The ranked search results.
    pub results: Vec<TavilyResult>,
}

/// One result of a Tavily-compatible search.
#[derive(Serialize)]
pub struct TavilyResult {
    /// Page title of the result.
    pub title: String,
    /// URL of the result.
    pub url: String,
    /// Readable content snippet or description.
    pub content: String,
    /// Positional relevance score (1.0 down to 0.05).
    pub score: f64,
    /// Raw page text, populated when `include_raw_content` is set.
    pub raw_content: Option<String>,
}

fn days_to_range(days: u32) -> Option<TimeRange> {
    Some(match days {
        0..=1 => TimeRange::Day,
        2..=7 => TimeRange::Week,
        8..=30 => TimeRange::Month,
        _ => TimeRange::Year,
    })
}

fn apply_domains(query: &mut String, include: &[String], exclude: &[String]) {
    if !include.is_empty() {
        let sites: Vec<String> = include.iter().map(|d| format!("site:{d}")).collect();
        query.push_str(&format!(" ({})", sites.join(" OR ")));
    }
    for d in exclude {
        query.push_str(&format!(" -site:{d}"));
    }
}

fn to_tavily_result(r: &ResultItem, pos: usize) -> (String, String, String, f64) {
    let score = (1.0 - pos as f64 * 0.05).max(0.05);
    match r {
        ResultItem::Web(w) => (w.title.clone(), w.url.clone(), w.description.clone(), score),
        ResultItem::News(n) => (n.title.clone(), n.url.clone(), n.description.clone(), score),
        ResultItem::Video(v) => (v.title.clone(), v.url.clone(), v.description.clone(), score),
        ResultItem::Image(i) => (i.title.clone(), i.url.clone(), i.source.clone(), score),
        ResultItem::Book(b) => (b.title.clone(), b.url.clone(), b.info.clone(), score),
    }
}

/// `POST /search`: Tavily-compatible search. Accepts the same body shape as
/// the Tavily API (query, optional `api_key`, `search_depth`, `topic`,
/// `days`, `max_results`, `include_*` flags and domain filters).
pub async fn search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    JsonBody(req): JsonBody<TavilyRequest>,
) -> AppResult<impl IntoResponse> {
    // Tavily SDKs pass credentials as `api_key` in the JSON body
    // (langchain-tavily, llama-index) or as headers; both are honored.
    let key = crate::auth_key(&headers, req.api_key.as_deref());
    if !state.authorized(key.as_deref()) {
        return Err(AppError::unauthorized());
    }

    let mut opts = SearchOptions::new(req.query.clone());
    let topic_is_news = matches!(req.topic.as_deref(), Some("news"));
    opts.category = if topic_is_news {
        Category::News
    } else {
        Category::Web
    };
    let depth = req.search_depth.as_deref().unwrap_or("basic");
    // "advanced" is honored by querying every engine in the category;
    // anything else is rejected loudly instead of silently coerced.
    if !matches!(depth, "basic" | "advanced") {
        return Err(AppError::bad_request(format!(
            "invalid search_depth '{depth}', expected 'basic' or 'advanced'"
        )));
    }
    if depth == "basic" {
        let mut engines = match opts.category {
            Category::News => vec!["bing_news".into(), "duckduckgo_news".into()],
            _ => vec!["bing".into(), "duckduckgo".into()],
        };
        if req.include_answer {
            engines.push("grokipedia".into());
        }
        opts.engines = engines;
    }
    if let Some(days) = req.days {
        opts.time_range = days_to_range(days);
    } else if topic_is_news {
        // news topic without an explicit window: last week, like Tavily
        opts.time_range = Some(TimeRange::Week);
    }
    opts.max_results = req.max_results.unwrap_or(5).clamp(1, 20);
    if let Some(include) = &req.include_domains {
        apply_domains(&mut opts.query, include, &[]);
    }
    if let Some(exclude) = &req.exclude_domains {
        apply_domains(&mut opts.query, &[], exclude);
    }

    let started = std::time::Instant::now();
    let resp = state.client.search(opts).await?;
    let response_time = started.elapsed().as_secs_f64();

    let limit = resp.total.min(req.max_results.unwrap_or(5).clamp(1, 20));
    let mut results: Vec<TavilyResult> = Vec::with_capacity(limit);
    let mut images: Vec<String> = Vec::new();
    for (i, r) in resp.results.iter().take(limit).enumerate() {
        let (title, url, content, score) = to_tavily_result(r, i);
        if let ResultItem::Image(img) = r
            && !img.image_url.is_empty()
        {
            images.push(img.image_url.clone());
        }
        results.push(TavilyResult {
            title,
            url,
            content,
            score,
            raw_content: None,
        });
    }

    if req.include_images {
        // Tavily's `images` field lists image results alongside the web
        // hits; run a dedicated image search to populate it honestly.
        let mut img_opts = SearchOptions::new(req.query.clone());
        img_opts.category = Category::Images;
        img_opts.max_results = limit.clamp(1, 8);
        if let Ok(img_resp) = state.client.search(img_opts).await {
            for r in img_resp.results.iter().take(8) {
                if let ResultItem::Image(img) = r
                    && !img.image_url.is_empty()
                {
                    images.push(img.image_url.clone());
                }
            }
        }
    }

    if req.include_raw_content {
        let client = state.client.http();
        let urls: Vec<String> = results.iter().map(|r| r.url.clone()).collect();
        let pages = phrona::extract_many(client, &urls, 8000, Some(&req.query)).await;
        for (r, page) in results.iter_mut().zip(pages) {
            r.raw_content = Some(match page {
                Ok(p) => p.text,
                Err(e) => format!("extract failed: {e}"),
            });
        }
    }

    Ok(Json(TavilyResponse {
        query: req.query.clone(),
        follow_up_questions: Vec::new(),
        response_time,
        answer: req.include_answer.then_some(resp.answer.clone()).flatten(),
        images: (req.include_images && !images.is_empty()).then_some(images),
        results,
    }))
}
