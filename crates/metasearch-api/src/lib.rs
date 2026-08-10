pub mod frontend;
pub mod grounding;
pub mod tavily;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use metasearch::engine;
use metasearch::models::{Category, ResultItem, SearchResponse, TimeRange};
use metasearch::{SearchClient, SearchOptions, suggest, suggest_all};

pub struct AppState {
    pub client: SearchClient,
    pub started: Instant,
    pub api_key: Option<String>,
}

impl AppState {
    pub fn new(client: SearchClient, api_key: Option<String>) -> Self {
        Self {
            client,
            started: Instant::now(),
            api_key,
        }
    }

    pub fn authorized(&self, key: Option<&str>) -> bool {
        self.api_key.as_deref().is_none_or(|want| key == Some(want))
    }
}

pub struct AppError(ErrorKind);

enum ErrorKind {
    BadRequest(String),
    Unauthorized,
    Internal(metasearch::Error),
}

impl AppError {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self(ErrorKind::BadRequest(msg.into()))
    }

    fn unauthorized() -> Self {
        Self(ErrorKind::Unauthorized)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, body) = match self.0 {
            ErrorKind::BadRequest(msg) => (StatusCode::BAD_REQUEST, json!({"error": msg})),
            ErrorKind::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                json!({"error": "invalid api key"}),
            ),
            ErrorKind::Internal(e) => {
                tracing::error!("search failed: {e}");
                (StatusCode::BAD_GATEWAY, json!({"error": e.to_string()}))
            }
        };
        (status, Json(body)).into_response()
    }
}

impl From<metasearch::Error> for AppError {
    fn from(e: metasearch::Error) -> Self {
        Self(ErrorKind::Internal(e))
    }
}

type AppResult<T> = Result<T, AppError>;

#[derive(Deserialize)]
struct SearchParams {
    q: String,
    category: Option<String>,
    engines: Option<String>,
    page: Option<u32>,
    max_results: Option<usize>,
    safesearch: Option<String>,
    region: Option<String>,
    language: Option<String>,
    time_range: Option<String>,
    filters: Option<String>,
    api_key: Option<String>,
}

fn build_options(p: &SearchParams, state: &AppState) -> AppResult<SearchOptions> {
    if !state.authorized(p.api_key.as_deref()) {
        return Err(AppError(ErrorKind::Unauthorized));
    }
    let mut opts = SearchOptions::new(p.q.clone());
    if let Some(c) = &p.category {
        opts.category = c.parse::<Category>().map_err(|_| {
            AppError::bad_request(format!(
                "invalid category '{c}', expected one of: web, images, news, videos, books"
            ))
        })?;
    }
    if let Some(es) = &p.engines {
        for name in es.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if engine::engine_by_name(name).is_none() {
                return Err(AppError::bad_request(format!(
                    "unknown engine '{name}'. Available: {}",
                    engine::list()
                        .iter()
                        .map(|e| e.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            opts.engines.push(name.to_string());
        }
    }
    if let Some(page) = p.page {
        opts.page = page.max(1);
    }
    if let Some(m) = p.max_results {
        opts.max_results = m.clamp(1, 100);
    }
    if let Some(s) = &p.safesearch {
        opts.safesearch = s.parse::<metasearch::SafeSearch>().map_err(|_| {
            AppError::bad_request("invalid safesearch, expected off|moderate|strict")
        })?;
    }
    if let Some(t) = &p.time_range {
        opts.time_range = Some(t.parse::<TimeRange>().map_err(|_| {
            AppError::bad_request("invalid time_range, expected day|week|month|year")
        })?);
    }
    opts.region = p.region.clone();
    opts.language = p.language.clone();
    opts.filters = p.filters.clone();
    Ok(opts)
}

fn header_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

fn split_results(resp: &SearchResponse) -> Vec<Value> {
    let items: Vec<Value> = resp
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let pos = (i + 1) as f64;
            let score = (1.0 - (pos - 1.0) * 0.05).max(0.05);
            match r {
                ResultItem::Web(w) => json!({
                    "type": "web",
                    "title": w.title,
                    "url": w.url,
                    "description": w.description,
                    "score": score,
                    "position": pos,
                    "engines": w.engines,
                }),
                ResultItem::Image(i) => json!({
                    "type": "image",
                    "title": i.title,
                    "url": i.url,
                    "image_url": i.image_url,
                    "thumbnail_url": i.thumbnail_url,
                    "width": i.width,
                    "height": i.height,
                    "score": score,
                    "position": pos,
                    "engines": i.engines,
                }),
                ResultItem::News(n) => json!({
                    "type": "news",
                    "title": n.title,
                    "url": n.url,
                    "description": n.description,
                    "published": n.published,
                    "source": n.source,
                    "score": score,
                    "position": pos,
                    "engines": n.engines,
                }),
                ResultItem::Video(v) => json!({
                    "type": "video",
                    "title": v.title,
                    "url": v.url,
                    "description": v.description,
                    "thumbnail_url": v.thumbnail_url,
                    "duration": v.duration,
                    "views": v.views,
                    "uploader": v.uploader,
                    "score": score,
                    "position": pos,
                    "engines": v.engines,
                }),
                ResultItem::Book(b) => json!({
                    "type": "book",
                    "title": b.title,
                    "url": b.url,
                    "description": b.info,
                    "author": b.author,
                    "publisher": b.publisher,
                    "score": score,
                    "position": pos,
                    "engines": b.engines,
                }),
            }
        })
        .collect();
    items
}

async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let web = engine::engines_for(Category::Web).len();
    let images = engine::engines_for(Category::Images).len();
    let news = engine::engines_for(Category::News).len();
    let videos = engine::engines_for(Category::Videos).len();
    Json(json!({
        "status": "ok",
        "version": metasearch::version(),
        "uptime_s": state.started.elapsed().as_secs(),
        "engines": {"web": web, "images": images, "news": news, "videos": videos},
        "auth": state.api_key.is_some(),
    }))
}

async fn engines() -> Json<Value> {
    let mut out = serde_json::Map::new();
    for cat in Category::ALL {
        out.insert(
            cat.as_str().to_string(),
            json!(
                metasearch::available_engines(cat)
                    .iter()
                    .map(|e| e.name.clone())
                    .collect::<Vec<_>>()
            ),
        );
    }
    Json(Value::Object(out))
}

async fn search_route(
    State(state): State<Arc<AppState>>,
    Query(p): Query<SearchParams>,
) -> AppResult<Json<Value>> {
    let opts = build_options(&p, &state)?;
    let resp = state.client.search(opts).await?;
    let results = split_results(&resp);
    let total = results.len();
    Ok(Json(json!({
        "query": resp.query,
        "category": resp.category.as_str(),
        "page": resp.page,
        "total": total,
        "results": results,
        "suggestions": resp.suggestions,
        "answer": resp.answer,
        "engines": resp.engines,
        "elapsed_ms": resp.elapsed_ms,
    })))
}

#[derive(Deserialize)]
struct SuggestParams {
    q: String,
    source: Option<String>,
    region: Option<String>,
}

async fn suggest_route(
    State(state): State<Arc<AppState>>,
    Query(p): Query<SuggestParams>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    if !state.authorized(header_key(&headers).as_deref()) {
        return Err(AppError(ErrorKind::Unauthorized));
    }
    let region = p.region.unwrap_or_else(|| "us-en".to_string());
    match p.source.as_deref() {
        Some(name) => {
            let source = metasearch::SuggestSource::from_name(name).ok_or_else(|| {
                AppError::bad_request(format!(
                    "unknown suggest source '{name}', expected one of: {}",
                    metasearch::SuggestSource::ALL
                        .iter()
                        .map(|s| s.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
            let suggestions = suggest(state.client.http(), source, &p.q, &region).await?;
            Ok(Json(json!({
                "query": p.q,
                "source": name,
                "suggestions": suggestions,
            })))
        }
        None => {
            let all = suggest_all(state.client.http(), &p.q, &region).await;
            let suggestions: serde_json::Map<String, Value> = all
                .into_iter()
                .map(|(s, list)| (s.name().to_string(), json!(list)))
                .collect();
            Ok(Json(json!({
                "query": p.q,
                "suggestions": suggestions,
            })))
        }
    }
}

/// Build the axum router with the given optional API key.
pub fn router(api_key: Option<String>) -> Router {
    let state = Arc::new(AppState::new(
        SearchClient::new().expect("build search client"),
        api_key,
    ));

    Router::new()
        .route("/", get(frontend::index))
        .route("/health", get(health))
        .route("/v1/engines", get(engines))
        .route("/v1/search", get(search_route))
        .route("/v1/suggest", get(suggest_route))
        .route("/v1/grounding", get(grounding::get).post(grounding::post))
        .route("/search", post(tavily::search))
        .route("/v1/tavily", post(tavily::search))
        .nest_service(
            "/static",
            tower_http::services::ServeDir::new(frontend::static_dir()),
        )
        .fallback(frontend::index)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Serve the REST API on `addr`. Blocks until the server stops.
pub async fn serve(addr: SocketAddr, api_key: Option<String>) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("metasearch-api listening on http://{addr}");
    axum::serve(listener, router(api_key)).await?;
    Ok(())
}

/// Serve using the META_ADDR / META_API_KEY environment variables.
pub async fn serve_from_env() -> anyhow::Result<()> {
    let addr = std::env::var("META_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let api_key = std::env::var("META_API_KEY").ok().filter(|k| !k.is_empty());
    serve(addr.parse()?, api_key).await
}

/// Default bind address when none is configured.
pub fn default_addr() -> SocketAddr {
    "127.0.0.1:8080".parse().expect("static addr")
}
