//! Orchestration: concurrent engines, merge, suggestions.

use std::sync::Arc;
use std::time::Instant;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::Semaphore;

use crate::client::{HttpClient, Profile, ProxyPool, TargetPolicy};
use crate::config::PhronaConfig;
use crate::dedup::{GroupedResult, group};
use crate::engine::{EngineContext, EngineShared, resolve};
use crate::error::{Error, Result};
use crate::models::{Category, EngineReport, RawResult, ResultItem, SearchResponse, WebResult};
use crate::options::SearchOptions;
use crate::rank::rank;

/// Default maximum number of simultaneous outbound engine requests per
/// search (overridable via [`SearchClient::with_config`] /
/// `search.concurrency_limit`).
const MAX_CONCURRENT_ENGINES: usize = 8;

/// Observes completed engine requests. Implemented by higher layers (e.g.
/// the REST API's Prometheus metrics); the default is a no-op so libraries
/// and CLI tools never pay for telemetry they don't serve.
///
/// `status` is one of `ok`, `empty` or `error`. `scope`/`kind` describe the
/// failure reason and are `None` on success.
pub trait EngineObserver: Send + Sync {
    /// Called after an engine request completes, with the engine name, its
    /// status (`ok` / `empty` / `error`), optional structured failure labels
    /// and the elapsed time.
    fn on_engine_done(
        &self,
        engine: &str,
        status: &str,
        scope: Option<&str>,
        kind: Option<&str>,
        elapsed: std::time::Duration,
    );
}

/// Default observer that does nothing.
#[derive(Default)]
pub struct NoopEngineObserver;

impl EngineObserver for NoopEngineObserver {
    fn on_engine_done(
        &self,
        _engine: &str,
        _status: &str,
        _scope: Option<&str>,
        _kind: Option<&str>,
        _elapsed: std::time::Duration,
    ) {
    }
}

/// High-level search client. Shares a persistent pool of impersonated HTTP
/// clients (one per proxy) across engines; each engine task is pinned to one
/// client so multi-step flows keep the same proxy and cookie jar.
pub struct SearchClient {
    pool: ProxyPool,
    shared: Arc<EngineShared>,
    concurrency: usize,
    observer: Arc<dyn EngineObserver>,
    /// Whether blocked bootstrap engines may be refreshed by briefly
    /// launching a headless browser (see `crate::bootstrap`).
    /// Default: disabled - browser use is opt-in.
    auto_bootstrap: bool,
}

impl SearchClient {
    /// Build a client with default settings.
    pub fn new() -> Result<Self> {
        Self::with_options(Profile::Chrome, None, None, TargetPolicy::default())
    }

    /// Build a client with explicit transport settings: impersonation
    /// profile, per-request timeout, an optional list of proxy URLs (one
    /// pooled client per proxy, used round-robin; empty = direct), and the
    /// operator's domain allow/deny policy.
    pub fn with_options(
        profile: Profile,
        timeout: Option<std::time::Duration>,
        proxies: Option<Vec<String>>,
        policy: TargetPolicy,
    ) -> Result<Self> {
        let timeout = timeout.unwrap_or_else(|| std::time::Duration::from_secs(10));
        let pool = ProxyPool::new(proxies.unwrap_or_default(), profile, timeout, policy)?;
        let client = Self {
            pool,
            shared: Arc::new(EngineShared::new()),
            concurrency: MAX_CONCURRENT_ENGINES,
            observer: Arc::new(NoopEngineObserver),
            // opt-in: no browser is ever launched unless explicitly enabled
            auto_bootstrap: false,
        };
        // local cookie cache warm start (phrona.cookies.json next to the
        // config): restarts reuse harvested sessions instead of re-harvesting
        for (engine, _, _) in crate::bootstrap::SEEDS {
            if let Some((jar, _at)) = crate::bootstrap::load_cached(engine) {
                if !jar.is_empty() {
                    client.shared.set_bootstrap(engine, jar);
                }
            }
        }
        Ok(client)
    }

    /// Build a client from a [`PhronaConfig`]: impersonation profile,
    /// timeout, proxy pool, domain policy and per-search concurrency limit.
    pub fn with_config(cfg: &PhronaConfig) -> Result<Self> {
        let mut client = Self::with_options(
            cfg.profile(),
            Some(cfg.timeout()),
            Some(cfg.engines.proxies.clone()),
            TargetPolicy::from_security(&cfg.security),
        )?;
        client.concurrency = cfg.concurrency_limit().max(1);
        client.auto_bootstrap = cfg.engines.auto_bootstrap;
        for (engine, cookies) in &cfg.engines.bootstrap_cookies {
            client.shared.set_bootstrap(engine, cookies.clone());
        }
        // local cookie cache (phrona.cookies.json next to the config):
        // warm start so restarts don't re-harvest
        for (engine, _, _) in crate::bootstrap::SEEDS {
            if cfg.engines.bootstrap_cookies.contains_key(*engine) {
                continue; // manual pinning wins
            }
            if let Some((jar, _at)) = crate::bootstrap::load_cached(engine) {
                if !jar.is_empty() {
                    client.shared.set_bootstrap(engine, jar);
                }
            }
        }
        Ok(client)
    }

    /// Enable/disable silent headless cookie harvesting on blocks.
    pub fn with_auto_bootstrap(mut self, enabled: bool) -> Self {
        self.auto_bootstrap = enabled;
        self
    }

    /// Per-engine spacing between automatic harvest attempts.
    fn refresh_spacing_ok(&self, engine: &str) -> bool {
        match self.shared.bootstrap_at.read().get(engine) {
            Some(at) => at.elapsed() >= crate::bootstrap::min_refresh_interval(engine),
            None => true,
        }
    }

    /// Register session cookies for an engine on this client in place
    /// (interior-mutable variant of [`Self::with_bootstrap_cookie`]).
    pub fn register_bootstrap_cookie(&self, engine: impl Into<String>, cookies: impl Into<String>) {
        self.shared.set_bootstrap(&engine.into(), cookies);
    }

    /// Register operator-supplied session cookies for an engine (e.g.
    /// Google's `__Secure-ENID`). Chainable builder method.
    pub fn with_bootstrap_cookie(
        self,
        engine: impl Into<String>,
        cookies: impl Into<String>,
    ) -> Self {
        self.shared.set_bootstrap(&engine.into(), cookies);
        self
    }

    /// The configured per-search engine concurrency cap.
    pub fn concurrency_limit(&self) -> usize {
        self.concurrency
    }

    /// Attach an observer notified after every engine request completes
    /// (`ok` / `empty` / `error` plus scope, kind and elapsed time).
    pub fn with_observer(mut self, observer: Arc<dyn EngineObserver>) -> Self {
        self.observer = observer;
        self
    }

    /// The first (or only) pooled client — used by non-engine flows such as
    /// `extract` and `suggest`.
    pub fn http(&self) -> &HttpClient {
        self.pool.first()
    }

    /// Run a search across all enabled engines for the category.
    ///
    /// Each engine task is assigned one sticky `HttpClient` from the proxy
    /// pool, and runs under a [`Semaphore`] limiting concurrency to the
    /// client's concurrency cap (default `MAX_CONCURRENT_ENGINES`).
    /// Engines run concurrently (`FuturesUnordered`) under a single adaptive
    /// deadline (`opts.timeout`). As soon as the merged result set reaches
    /// `opts.max_results` the remaining in-flight engine futures are dropped
    /// (cancelled) and we return early. An engine that returns an `Ok` —
    /// even with zero results — counts as a success; an error is only raised
    /// when every engine failed. On page 1 of Web searches, suggestions are
    /// fetched in parallel with the scraping via `tokio::join!`.
    pub async fn search(&self, opts: SearchOptions) -> Result<SearchResponse> {
        let started = Instant::now();
        let deadline = started + opts.timeout;
        let max_results = opts.max_results;
        let category = opts.category;
        let engines = resolve(&opts, category);
        if engines.is_empty() {
            return Err(Error::invalid_query(
                "orchestrator",
                "no engines available for category",
            ));
        }

        let sem = Arc::new(Semaphore::new(self.concurrency));

        let futs = engines.iter().map(|engine| {
            let client = self.pool.get_client();
            let shared = Arc::clone(&self.shared);
            let sem = Arc::clone(&sem);
            let opts = &opts;
            async move {
                let ctx = EngineContext {
                    client,
                    opts,
                    shared: &shared,
                };
                let started = Instant::now();
                let _permit = sem.acquire().await.expect("semaphore closed");
                let r = engine.search(&ctx).await;
                (engine.name(), r, started.elapsed())
            }
        });
        let mut in_flight = FuturesUnordered::from_iter(futs);

        let scrape = async move {
            let mut answers: Vec<RawResult> = Vec::new();
            let mut raw: Vec<RawResult> = Vec::new();
            let mut reports: Vec<EngineReport> = Vec::new();
            let mut any_ok = false;

            while let Some((name, result, elapsed)) = in_flight.next().await {
                if Instant::now() >= deadline {
                    drop(in_flight);
                    break;
                }
                match result {
                    Ok(items) => {
                        any_ok = true;
                        if items.is_empty() {
                            self.observer
                                .on_engine_done(name, "empty", None, None, elapsed);
                            reports.push(EngineReport {
                                name: name.to_string(),
                                status: "empty".into(),
                                results: 0,
                                error: None,
                                scope: None,
                                kind: None,
                            });
                            continue;
                        }
                        let n = items.len();
                        self.observer
                            .on_engine_done(name, "ok", None, None, elapsed);
                        let (answers_part, raw_part): (Vec<_>, Vec<_>) =
                            items.into_iter().partition(|r| r.url.is_empty());
                        answers.extend(answers_part);
                        raw.extend(raw_part);
                        reports.push(EngineReport {
                            name: name.to_string(),
                            status: "ok".into(),
                            results: n,
                            error: None,
                            scope: None,
                            kind: None,
                        });
                    }
                    Err(e) => {
                        let scope = format!("{:?}", e.scope());
                        let kind = format!("{:?}", e.kind());
                        self.observer.on_engine_done(
                            name,
                            "error",
                            Some(&scope),
                            Some(&kind),
                            elapsed,
                        );
                        reports.push(EngineReport {
                            name: name.to_string(),
                            status: "error".into(),
                            results: 0,
                            error: Some(e.to_string()),
                            scope: Some(scope),
                            kind: Some(kind),
                        });
                    }
                }
                if !opts.probe_all && raw.len() >= max_results {
                    drop(in_flight);
                    break;
                }
            }
            (raw, answers, reports, any_ok)
        };

        let suggestions = async {
            if category == Category::Web && opts.page == 1 {
                let client = self.pool.get_client();
                crate::engines::suggest::suggest_all(client, &opts.query, &opts.region_param())
                    .await
                    .into_iter()
                    .flat_map(|(_, s)| s)
                    .filter(|s| !s.is_empty())
                    .take(10)
                    .collect()
            } else {
                Vec::new()
            }
        };

        let ((mut raw, mut answers, mut reports, any_ok), suggestions) =
            tokio::join!(scrape, suggestions);

        // Silent bypass: engines whose anti-bot trusts only real-browser
        // cookies get one headless harvest + retry when blocked.
        if self.auto_bootstrap {
            // NB: reports arrive in COMPLETION order - match by name
            let by_name: std::collections::HashMap<&str, &'static dyn crate::engine::Engine> =
                engines.iter().map(|e| (e.name(), *e)).collect();
            let stale: Vec<&'static dyn crate::engine::Engine> = reports
                .iter()
                .filter(|r| {
                    r.status == "error"
                        // ErrorKind::Debug renders as "Blocked(...)" /
                        // "NetworkFailure" - both mean the session cookies
                        // may be missing/stale for a bootstrap engine
                        && r.kind.as_deref().is_some_and(|k| {
                            k.starts_with("Blocked") || k.starts_with("NetworkFailure")
                        })
                        && crate::bootstrap::seed_for(&r.name).is_some()
                        && self.shared.bootstrap_stale(&r.name)
                        && self.refresh_spacing_ok(&r.name)
                })
                .filter_map(|r| by_name.get(r.name.as_str()).copied())
                .collect();
            if std::env::var_os("PHRONA_DEBUG_BOOTSTRAP").is_some() && !stale.is_empty() {
                eprintln!(
                    "[dbg bootstrap] refreshing {:?}",
                    stale.iter().map(|e| e.name()).collect::<Vec<_>>()
                );
            }
            if !stale.is_empty() {
                for engine in &stale {
                    let name = engine.name();
                    match tokio::task::spawn_blocking({
                        move || crate::bootstrap::harvest_blocking(name)
                    })
                    .await
                    {
                        Ok(Ok(jar)) => {
                            if std::env::var_os("PHRONA_DEBUG_BOOTSTRAP").is_some() {
                                eprintln!("[dbg bootstrap] {name}: harvested {} bytes", jar.len());
                            }
                            // persist for the next run of a local install
                            let name2 = name;
                            let jar2 = jar.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                crate::bootstrap::store_cached(name2, &jar2)
                            })
                            .await;
                            self.shared.set_bootstrap(name, jar);
                        }
                        Ok(Err(e)) => {
                            if std::env::var_os("PHRONA_DEBUG_BOOTSTRAP").is_some() {
                                eprintln!("[dbg bootstrap] {name}: harvest failed: {e}");
                            }
                            continue;
                        }
                        Err(_) => continue,
                    }
                    self.shared.mark_bootstrap_refreshed(name);
                }

                // rerun the blocked engines once with fresh cookies
                let sem2 = Arc::new(Semaphore::new(self.concurrency));
                let futs = stale.iter().map(|engine| {
                    let client = self.pool.get_client();
                    let shared = Arc::clone(&self.shared);
                    let sem = Arc::clone(&sem2);
                    let opts = &opts;
                    async move {
                        let ctx = EngineContext {
                            client,
                            opts,
                            shared: &shared,
                        };
                        let started = Instant::now();
                        let _permit = sem.acquire().await.expect("semaphore closed");
                        (engine.name(), engine.search(&ctx).await, started.elapsed())
                    }
                });
                let mut retry = FuturesUnordered::from_iter(futs);
                while let Some((name, result, elapsed)) = retry.next().await {
                    let slot = reports.iter_mut().find(|r| r.name == name);
                    match result {
                        Ok(items) if items.is_empty() => {
                            if let Some(r) = slot {
                                *r = EngineReport {
                                    name: name.to_string(),
                                    status: "empty".into(),
                                    results: 0,
                                    error: None,
                                    scope: None,
                                    kind: None,
                                };
                            }
                            self.observer
                                .on_engine_done(name, "empty", None, None, elapsed);
                        }
                        Ok(items) => {
                            let n = items.len();
                            if let Some(r) = slot {
                                *r = EngineReport {
                                    name: name.to_string(),
                                    status: "ok".into(),
                                    results: n,
                                    error: None,
                                    scope: None,
                                    kind: None,
                                };
                            }
                            self.observer
                                .on_engine_done(name, "ok", None, None, elapsed);
                            let (a_part, r_part): (Vec<_>, Vec<_>) =
                                items.into_iter().partition(|x| x.url.is_empty());
                            answers.extend(a_part);
                            raw.extend(r_part);
                        }
                        Err(e) => {
                            if std::env::var_os("PHRONA_DEBUG_BOOTSTRAP").is_some() {
                                eprintln!("[dbg bootstrap] {name} retry failed: {e}");
                            }
                            let scope_s = format!("{:?}", e.scope());
                            let kind_s = format!("{:?}", e.kind());
                            if let Some(r) = slot {
                                *r = EngineReport {
                                    name: name.to_string(),
                                    status: "error".into(),
                                    results: 0,
                                    error: Some(e.to_string()),
                                    scope: Some(scope_s.clone()),
                                    kind: Some(kind_s.clone()),
                                };
                            }
                            self.observer.on_engine_done(
                                name,
                                "error",
                                Some(&scope_s),
                                Some(&kind_s),
                                elapsed,
                            );
                        }
                    }
                }
            }
        }

        if !any_ok {
            // Availability probing wants the full per-engine report even for
            // a category where every engine failed; normal searches surface
            // the failure as an error instead.
            if opts.probe_all {
                return Ok(SearchResponse {
                    query: opts.query.clone(),
                    category,
                    page: opts.page,
                    total: 0,
                    results: Vec::new(),
                    suggestions,
                    answer: None,
                    engines: reports,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });
            }
            let details = reports
                .iter()
                .filter_map(|r| r.error.as_ref().map(|e| format!("{}: {}", r.name, e)))
                .collect();
            return Err(Error::all_failed("orchestrator", details));
        }

        let answer = answers
            .into_iter()
            .map(|a| a.description)
            .max_by_key(|a| a.chars().count());

        let groups = group(raw);
        let ranked = rank(groups, &opts.query);
        let mut results: Vec<ResultItem> = Vec::new();
        for (raw_score, g) in ranked.into_iter() {
            // unified cross-category score, normalized to (0.001, 1.000),
            // derived from the raw score `rank` already computed
            let score = crate::rank::normalize_score(raw_score);
            let item = to_result_item(g, score, results.len());
            if let Some(item) = item {
                results.push(item);
            }
            if results.len() >= opts.max_results {
                break;
            }
        }

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

    /// Blocking API for use from plain (non-tokio) threads.
    ///
    /// Calling this from inside an active Tokio runtime would deadlock or
    /// panic (`block_on` inside a worker thread), so it refuses and asks the
    /// caller to use the async [`SearchClient::search`] instead.
    pub fn search_sync(&self, opts: SearchOptions) -> Result<SearchResponse> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(Error::internal(
                "search",
                "search_sync cannot be called from within an active Tokio runtime thread pool; use async search().await instead",
            ));
        }
        block_on(self.search(opts))
    }
}

/// Convert a merged, ranked group into a typed [`ResultItem`] for the
/// response. The category is inferred from the engine that introduced the
/// result; unknown engines map to `Web`. `idx` is the zero-based result
/// index (position becomes `idx + 1`). Returns `None` only when a result
/// carries no URL and cannot be placed.
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
            score,
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
            score,
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
            score,
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
            score,
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
            scope: None,
            kind: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::dedup::GroupedResult;
    use crate::models::RawResult;
    use crate::models::ResultItem;
    use crate::search::to_result_item;
    use crate::{SearchClient, SearchOptions};

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
        assert_eq!(
            crate::dedup::dedup_key(&items[1].url),
            "https://example.com/a"
        );
    }

    #[test]
    fn search_sync_refuses_inside_active_runtime() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let client = SearchClient::new().unwrap();
            let err = client.search_sync(SearchOptions::new("x")).unwrap_err();
            assert!(
                err.to_string().contains("search_sync cannot be called"),
                "got: {err}"
            );
        });
    }

    #[test]
    fn to_result_item_covers_all_categories() {
        for (engine, expect) in [
            ("bing", "web"),
            ("bing_images", "image"),
            ("bing_news", "news"),
            ("bing_videos", "video"),
            ("annas_archive", "book"),
        ] {
            let mut r = raw("title", "https://example.com/x");
            r.engine = engine.into();
            let g = GroupedResult {
                result: r,
                engines: vec!["engine1".into(), "engine2".into()],
                count: 2,
            };
            let item = to_result_item(g, 0.9, 3).expect("engine maps to a category");
            match item {
                ResultItem::Web(w) => {
                    assert_eq!(expect, "web");
                    assert_eq!(w.position, 4);
                    assert_eq!(w.score, 0.9);
                    assert_eq!(w.url, "https://example.com/x");
                    assert_eq!(w.engines, ["engine1", "engine2"]);
                }
                ResultItem::Image(i) => {
                    assert_eq!(expect, "image");
                    assert_eq!(i.position, 4);
                    assert_eq!(i.title, "title");
                }
                ResultItem::News(n) => {
                    assert_eq!(expect, "news");
                    assert_eq!(n.position, 4);
                    assert_eq!(n.description, "desc");
                }
                ResultItem::Video(v) => {
                    assert_eq!(expect, "video");
                    assert_eq!(v.position, 4);
                    assert_eq!(v.uploader, "");
                }
                ResultItem::Book(b) => {
                    assert_eq!(expect, "book");
                    assert_eq!(b.position, 4);
                    assert_eq!(b.author, "");
                }
            }
        }
        // unknown engines map to web
        let mut r = raw("t", "https://example.com/y");
        r.engine = "not_an_engine".into();
        assert!(matches!(
            to_result_item(
                GroupedResult {
                    result: r,
                    engines: vec![],
                    count: 1
                },
                0.5,
                0,
            ),
            Some(ResultItem::Web(_))
        ));
    }
}
