use std::sync::Arc;
use std::time::Instant;

use crate::client::{HttpClient, Profile};
use crate::dedup::{GroupedResult, group};
use crate::engine::{EngineContext, EngineShared, resolve};
use crate::error::{Error, Result};
use crate::models::{Category, EngineReport, RawResult, ResultItem, SearchResponse, WebResult};
use crate::options::SearchOptions;
use crate::rank::rank;

/// High-level search client. Shares one HTTP client across engines.
pub struct SearchClient {
    http: HttpClient,
    shared: Arc<EngineShared>,
}

impl SearchClient {
    /// Build a client with default settings.
    pub fn new() -> Result<Self> {
        Self::with_profile(Profile::Chrome)
    }

    pub fn with_profile(profile: Profile) -> Result<Self> {
        Self::with_options(profile, None, None)
    }

    pub fn with_options(
        profile: Profile,
        timeout: Option<std::time::Duration>,
        proxies: Option<Vec<String>>,
    ) -> Result<Self> {
        let mut b = HttpClient::builder().profile(profile);
        if let Some(t) = timeout {
            b = b.timeout(t);
        }
        if let Some(proxies) = proxies {
            b = b.proxy(proxies.into_iter().next());
        }
        Ok(Self {
            http: b.build()?,
            shared: Arc::new(EngineShared::new()),
        })
    }

    pub fn http(&self) -> &HttpClient {
        &self.http
    }

    pub fn shared(&self) -> &EngineShared {
        &self.shared
    }

    /// Run a search across all enabled engines for the category.
    pub async fn search(&self, opts: SearchOptions) -> Result<SearchResponse> {
        let started = Instant::now();
        let category = opts.category;
        let engines = resolve(&opts, category);
        if engines.is_empty() {
            return Err(Error::Engine(format!(
                "no engines available for category {category:?}"
            )));
        }

        let futs = engines.iter().map(|engine| {
            let ctx = EngineContext {
                client: &self.http,
                opts: &opts,
                shared: &self.shared,
            };
            async move {
                let r = engine.search(&ctx).await;
                (engine.name(), r)
            }
        });
        let outcomes: Vec<(&str, crate::error::Result<Vec<RawResult>>)> =
            futures::future::join_all(futs).await;

        let mut answers: Vec<RawResult> = Vec::new();
        let mut raw: Vec<RawResult> = Vec::new();
        let mut reports: Vec<EngineReport> = Vec::new();
        let mut any = false;
        for (name, result) in outcomes {
            match result {
                Ok(items) => {
                    if items.is_empty() {
                        reports.push(EngineReport {
                            name: name.to_string(),
                            status: "empty".into(),
                            results: 0,
                            error: None,
                        });
                        continue;
                    }
                    any = true;
                    let n = items.len();
                    let (answers_part, raw_part): (Vec<_>, Vec<_>) =
                        items.into_iter().partition(|r| r.url.is_empty());
                    answers.extend(answers_part);
                    raw.extend(raw_part);
                    reports.push(EngineReport {
                        name: name.to_string(),
                        status: "ok".into(),
                        results: n,
                        error: None,
                    });
                }
                Err(e) => {
                    reports.push(EngineReport {
                        name: name.to_string(),
                        status: "error".into(),
                        results: 0,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        if !any {
            return Err(Error::NoResults(
                reports
                    .iter()
                    .filter_map(|r| r.error.as_ref().map(|e| format!("{}: {}", r.name, e)))
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }

        let answer = answers
            .into_iter()
            .map(|a| a.description)
            .max_by_key(|a| a.chars().count());

        let groups = group(raw);
        let ranked = rank(groups, &opts.query);
        let mut results: Vec<ResultItem> = Vec::new();
        for (score, g) in ranked.into_iter() {
            let item = to_result_item(g, score, results.len());
            if let Some(item) = item {
                results.push(item);
            }
            if results.len() >= opts.max_results {
                break;
            }
        }

        let suggestions = if category == Category::Web && opts.page == 1 {
            crate::engines::suggest::suggest_all(&self.http, &opts.query, &opts.region_param())
                .await
                .into_iter()
                .flat_map(|(_, s)| s)
                .filter(|s| !s.is_empty())
                .take(10)
                .collect()
        } else {
            Vec::new()
        };

        Ok(SearchResponse {
            query: opts.query.clone(),
            category,
            page: opts.page,
            total: results.len(),
            results,
            suggestions,
            answer,
            engines: reports,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// Blocking API. Works both from plain threads and from inside a tokio
    /// runtime (uses the ambient runtime when present).
    pub fn search_sync(&self, opts: SearchOptions) -> Result<SearchResponse> {
        block_on(self.search(opts))
    }
}

pub fn to_result_item(g: GroupedResult, score: f64, idx: usize) -> Option<ResultItem> {
    let raw = g.result;
    let category = crate::engine::category_of_engine(&raw.engine);
    let position = idx + 1;
    match category {
        Category::Web => Some(ResultItem::Web(WebResult {
            title: raw.title,
            url: raw.url,
            description: raw.description,
            engines: g.engines,
            position,
            score,
        })),
        Category::Images => Some(ResultItem::Image(crate::models::ImageResult {
            title: raw.title,
            url: raw.url,
            image_url: raw.image_url,
            thumbnail_url: raw.thumbnail_url,
            width: raw.width,
            height: raw.height,
            source: raw.source,
            engines: g.engines,
            position,
        })),
        Category::News => Some(ResultItem::News(crate::models::NewsResult {
            title: raw.title,
            url: raw.url,
            description: raw.description,
            published: raw.published,
            source: raw.source,
            image_url: raw.image_url,
            engines: g.engines,
            position,
        })),
        Category::Videos => Some(ResultItem::Video(crate::models::VideoResult {
            title: raw.title,
            url: raw.url,
            description: raw.description,
            duration: raw.duration,
            published: raw.published,
            uploader: raw.uploader,
            views: raw.views,
            thumbnail_url: raw.thumbnail_url,
            engines: g.engines,
            position,
        })),
        Category::Books => Some(ResultItem::Book(crate::models::BookResult {
            title: raw.title,
            author: raw.author,
            publisher: raw.publisher,
            info: raw.description,
            url: raw.url,
            thumbnail_url: raw.thumbnail_url,
            engines: g.engines,
            position,
        })),
    }
}

/// Block a future on the ambient runtime, or a shared one otherwise.
pub fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(fut),
        Err(_) => RUNTIME
            .get_or_init(|| tokio::runtime::Runtime::new().expect("tokio runtime"))
            .block_on(fut),
    }
}

static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

/// Convenience: single-shot async search with default client.
pub async fn search(opts: SearchOptions) -> Result<SearchResponse> {
    SearchClient::new()?.search(opts).await
}

/// Convenience: single-shot blocking search.
pub fn search_sync(opts: SearchOptions) -> Result<SearchResponse> {
    SearchClient::new()?.search_sync(opts)
}

/// List engines available for a category (name + metadata).
pub fn available_engines(category: Category) -> Vec<crate::models::EngineReport> {
    crate::engine::engines_for(category)
        .iter()
        .map(|e| EngineReport {
            name: e.name().to_string(),
            status: "enabled".into(),
            results: 0,
            error: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::models::RawResult;

    fn raw(title: &str, url: &str) -> RawResult {
        RawResult {
            title: title.into(),
            url: url.into(),
            description: "desc".into(),
            engine: "bing".into(),
            ..Default::default()
        }
    }

    #[test]
    fn merge_keeps_results_and_answers() {
        // answer marker (empty url) + real results must all survive merging
        let items = vec![
            raw("answer", ""),
            raw("A", "https://example.com/a?utm_source=x"),
            raw("B", "https://example.org/b"),
        ];
        let (answers, rest): (Vec<_>, Vec<_>) =
            items.clone().into_iter().partition(|r| r.url.is_empty());
        assert_eq!(answers.len(), 1);
        assert_eq!(rest.len(), 2);
        let groups = crate::dedup::group(rest);
        assert_eq!(groups.len(), 2);
        // dedup strips tracking params
        assert_eq!(
            crate::dedup::dedup_key(&items[1].url),
            "https://example.com/a"
        );
    }
}
