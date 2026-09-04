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

/// Environment opt-in for automatic session refresh. Accepts
/// `PHRONA_AUTO_BOOTSTRAP` (canonical) and the config-layer alias
/// `PHRONA_ENGINES_AUTO_BOOTSTRAP`; truthy values: 1/true/yes/on.
fn env_auto_bootstrap() -> Option<bool> {
    for key in ["PHRONA_AUTO_BOOTSTRAP", "PHRONA_ENGINES_AUTO_BOOTSTRAP"] {
        if let Ok(v) = std::env::var(key) {
            return Some(matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ));
        }
    }
    None
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
        let mut client = Self::build(profile, timeout, proxies.unwrap_or_default(), policy)?;
        // opt-in: no browser is ever launched unless explicitly
        // enabled via builder, config, or environment
        client.auto_bootstrap = env_auto_bootstrap().unwrap_or(false);
        Self::warm_start(&client);
        Ok(client)
    }

    /// Shared constructor without any cache warm-start; callers run
    /// [`Self::warm_start`] once after applying their own pins.
    fn build(
        profile: Profile,
        timeout: std::time::Duration,
        proxies: Vec<String>,
        policy: TargetPolicy,
    ) -> Result<Self> {
        let pool = ProxyPool::new(proxies, profile, timeout, policy)?;
        Ok(Self {
            pool,
            shared: Arc::new(EngineShared::new()),
            concurrency: MAX_CONCURRENT_ENGINES,
            observer: Arc::new(NoopEngineObserver),
            auto_bootstrap: false,
        })
    }

    /// Build a client from a [`PhronaConfig`]: impersonation profile,
    /// timeout, proxy pool, domain policy and per-search concurrency limit.
    pub fn with_config(cfg: &PhronaConfig) -> Result<Self> {
        let mut client = Self::build(
            cfg.profile(),
            cfg.timeout(),
            cfg.engines.proxies.clone(),
            TargetPolicy::from_security(&cfg.security),
        )?;
        client.concurrency = cfg.concurrency_limit().max(1);
        // config key wins when set, but the canonical environment alias
        // opts in too (mirrors `with_options`, which reads the env
        // directly): a bare `PHRONA_AUTO_BOOTSTRAP=1` must work on every
        // construction path, not just the non-config one
        client.auto_bootstrap = cfg.engines.auto_bootstrap || env_auto_bootstrap().unwrap_or(false);
        // sole owner here, so `get_mut` cannot fail; floored to avoid
        // token-thrash from tiny operator values
        if let Some(shared) = Arc::get_mut(&mut client.shared) {
            shared.cache_ttl = std::time::Duration::from_secs(cfg.search.cache_ttl_secs.max(60));
        }
        for (engine, cookies) in &cfg.engines.bootstrap_cookies {
            client.shared.set_bootstrap(engine, cookies.clone());
        }
        // manual pins win over the local cache
        for engine in cfg.engines.bootstrap_cookies.keys() {
            client.shared.bootstrap_at.write().remove(engine);
        }
        Self::warm_start(&client);
        Ok(client)
    }

    /// Load sessions from the local cache (`phrona.cookies.json` next to
    /// the config) and seed per-engine refresh clocks from their ages, so
    /// restarts reuse recent sessions without any browsing.
    fn warm_start(client: &SearchClient) {
        for (engine, _, _) in crate::bootstrap::SEEDS {
            if let Some((jar, at)) = crate::bootstrap::load_cached(engine) {
                if jar.is_empty() {
                    continue;
                }
                if client.shared.bootstrap_for(engine).is_some() {
                    continue; // an explicit pin already provides this engine
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(at);
                client.shared.set_bootstrap(engine, jar);
                client
                    .shared
                    .seed_bootstrap_age(engine, now.saturating_sub(at));
            }
        }
    }

    /// Enable/disable automatic session refresh via a brief headless
    /// browser when a bootstrap engine is blocked. Off by default; the
    /// `PHRONA_AUTO_BOOTSTRAP` environment variable sets the initial
    /// value for every client.
    pub fn with_auto_bootstrap(mut self, enabled: bool) -> Self {
        self.auto_bootstrap = enabled;
        self
    }

    /// Whether automatic session refresh is currently enabled.
    pub fn auto_bootstrap_enabled(&self) -> bool {
        self.auto_bootstrap
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

    /// Override the per-search engine concurrency cap (floored at 1).
    /// Chainable; surfaces use it to apply `search.concurrency_limit`
    /// without rebuilding the client.
    pub fn with_concurrency(mut self, limit: usize) -> Self {
        self.concurrency = limit.max(1);
        self
    }

    /// Override the TTL of the engine-scoped token caches (`vqd`/`sc`),
    /// floored at 60s like the config path. Chainable.
    pub fn with_cache_ttl(mut self, secs: u64) -> Self {
        if let Some(shared) = Arc::get_mut(&mut self.shared) {
            shared.cache_ttl = std::time::Duration::from_secs(secs.max(60));
        }
        self
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

    /// Record a successful engine run (empty or not): merge its items into
    /// `answers`/`raw`, notify the observer, and return its report. The
    /// caller files it (`push` for first runs, `replace` for retries).
    fn record_ok(
        &self,
        name: &'static str,
        items: Vec<RawResult>,
        elapsed: std::time::Duration,
        answers: &mut Vec<RawResult>,
        raw: &mut Vec<RawResult>,
    ) -> EngineReport {
        let rep = if items.is_empty() {
            Self::empty_report(name)
        } else {
            let n = items.len();
            let (answers_part, raw_part): (Vec<_>, Vec<_>) =
                items.into_iter().partition(|r| r.url.is_empty());
            answers.extend(answers_part);
            raw.extend(raw_part);
            Self::ok_report(name, n)
        };
        self.notify(
            name,
            &rep.status,
            rep.scope.as_deref(),
            rep.kind.as_deref(),
            elapsed,
        );
        rep
    }

    /// Record a failed engine run: notify the observer and return its
    /// report for filing.
    fn record_err(
        &self,
        name: &'static str,
        e: &Error,
        elapsed: std::time::Duration,
    ) -> EngineReport {
        let rep = Self::err_report(name, e);
        self.notify(
            name,
            &rep.status,
            rep.scope.as_deref(),
            rep.kind.as_deref(),
            elapsed,
        );
        rep
    }

    /// Notify the observer of a completed engine run.
    fn notify(
        &self,
        name: &'static str,
        status: &str,
        scope: Option<&str>,
        kind: Option<&str>,
        elapsed: std::time::Duration,
    ) {
        self.observer
            .on_engine_done(name, status, scope, kind, elapsed);
    }

    /// Replace the report for a retried engine in place (matched by name;
    /// reports arrive in completion order, so positional updates are wrong).
    fn replace_report(&self, reports: &mut [EngineReport], name: &str, report: EngineReport) {
        if let Some(slot) = reports.iter_mut().find(|r| r.name == name) {
            *slot = report;
        }
    }

    /// Build the success report for an engine run returning `n` items.
    fn ok_report(name: &str, n: usize) -> EngineReport {
        EngineReport {
            name: name.to_string(),
            status: "ok".into(),
            results: n,
            error: None,
            scope: None,
            kind: None,
        }
    }

    /// Build the report for an engine run returning zero items.
    fn empty_report(name: &str) -> EngineReport {
        EngineReport {
            name: name.to_string(),
            status: "empty".into(),
            results: 0,
            error: None,
            scope: None,
            kind: None,
        }
    }

    /// Build the failure report for an engine run.
    fn err_report(name: &str, e: &Error) -> EngineReport {
        EngineReport {
            name: name.to_string(),
            status: "error".into(),
            results: 0,
            error: Some(e.to_string()),
            scope: Some(format!("{:?}", e.scope())),
            kind: Some(format!("{:?}", e.kind())),
        }
    }
    /// Run one engine under the concurrency semaphore. Never panics: a closed
    /// semaphore (only possible during shutdown) surfaces as an internal error.
    async fn run_engine(
        engine: &'static dyn crate::engine::Engine,
        client: &HttpClient,
        shared: &EngineShared,
        sem: &Semaphore,
        opts: &SearchOptions,
    ) -> (&'static str, Result<Vec<RawResult>>, std::time::Duration) {
        let started = Instant::now();
        let _permit = match sem.acquire().await {
            Ok(p) => p,
            Err(_) => {
                return (
                    engine.name(),
                    Err(Error::internal("orchestrator", "shutting down")),
                    started.elapsed(),
                );
            }
        };
        let ctx = EngineContext {
            client,
            opts,
            shared,
        };
        let r = engine.search(&ctx).await;
        (engine.name(), r, started.elapsed())
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
        // normalize once at the choke point so no engine does page
        // arithmetic on a zero page (underflow) and blank queries never
        // become meaningless upstream traffic
        let mut opts = opts;
        opts.page = opts.page.max(1);
        if opts.query.trim().is_empty() {
            return Err(Error::invalid_query(
                "orchestrator",
                "query must not be empty",
            ));
        }
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
            Self::run_engine(*engine, self.pool.get_client(), &self.shared, &sem, &opts)
        });
        let mut in_flight = FuturesUnordered::from_iter(futs);

        let scrape = async move {
            let mut answers: Vec<RawResult> = Vec::new();
            let mut raw: Vec<RawResult> = Vec::new();
            let mut reports: Vec<EngineReport> = Vec::new();
            // per-engine session-refresh candidacy, keyed by engine name
            // (reports alone only carry stringified kinds)
            let mut refreshable: std::collections::HashMap<&'static str, bool> =
                std::collections::HashMap::new();
            let mut any_ok = false;

            // Deadline-aware: a hung engine must not stall past `deadline`
            // waiting on `next()`.
            while !in_flight.is_empty() {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let next = tokio::time::timeout(remaining, in_flight.next()).await;
                let Some((name, result, elapsed)) = next.ok().flatten() else {
                    break;
                };
                match result {
                    Ok(items) => {
                        any_ok = true;
                        refreshable.insert(name, false);
                        let rep = self.record_ok(name, items, elapsed, &mut answers, &mut raw);
                        reports.push(rep);
                    }
                    Err(e) => {
                        refreshable.insert(name, e.may_require_session());
                        let rep = self.record_err(name, &e, elapsed);
                        reports.push(rep);
                    }
                }
                if !opts.probe_all && raw.len() >= max_results {
                    break;
                }
            }
            (raw, answers, reports, refreshable, any_ok)
        };

        let suggestions = async {
            if category == Category::Web && opts.page == 1 {
                // bounded like the scrape: suggestions must not outlive the
                // search deadline on their own
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Vec::new();
                }
                let client = self.pool.get_client();
                let region = opts.region_param();
                let fut = crate::engines::suggest::suggest_all(client, &opts.query, &region);
                tokio::time::timeout(remaining, fut)
                    .await
                    .ok()
                    .map(|all| {
                        all.into_iter()
                            .flat_map(|(_, s)| s)
                            .filter(|s| !s.is_empty())
                            .take(10)
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        };

        let ((mut raw, mut answers, mut reports, refreshable, mut any_ok), suggestions): (
            _,
            Vec<String>,
        ) = tokio::join!(scrape, suggestions);

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
                        // failures that a fresh browser session may fix
                        // (anti-bot blocks, transport failures)
                        && refreshable.get(r.name.as_str()).copied().unwrap_or(false)
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
                // harvest concurrently, each attempt bounded: a stuck
                // browser (or a slow first-time download) must not stall
                // the search indefinitely, and sequential harvests could
                // stack past any reasonable deadline
                let jobs = stale.iter().map(|engine| {
                    let name = engine.name();
                    async move {
                        let res = tokio::time::timeout(
                            crate::bootstrap::HARVEST_TIMEOUT,
                            tokio::task::spawn_blocking(move || {
                                crate::bootstrap::harvest_blocking(name)
                            }),
                        )
                        .await;
                        (name, res)
                    }
                });
                let mut pending = FuturesUnordered::from_iter(jobs);
                while let Some((name, res)) = pending.next().await {
                    match res {
                        Ok(Ok(Ok(jar))) => {
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
                        Ok(Ok(Err(e))) => {
                            if std::env::var_os("PHRONA_DEBUG_BOOTSTRAP").is_some() {
                                eprintln!("[dbg bootstrap] {name}: harvest failed: {e}");
                            }
                        }
                        Ok(Err(_)) => {
                            if std::env::var_os("PHRONA_DEBUG_BOOTSTRAP").is_some() {
                                eprintln!("[dbg bootstrap] {name}: harvest task panicked");
                            }
                        }
                        Err(_) => {
                            if std::env::var_os("PHRONA_DEBUG_BOOTSTRAP").is_some() {
                                eprintln!(
                                    "[dbg bootstrap] {name}: harvest timed out after {}s",
                                    crate::bootstrap::HARVEST_TIMEOUT.as_secs()
                                );
                            }
                        }
                    }
                    // spacing applies to attempts, not just successes: a
                    // hanging browser must not re-hang every search
                    self.shared.mark_bootstrap_refreshed(name);
                }

                // rerun the blocked engines once with fresh cookies
                let sem2 = Arc::new(Semaphore::new(self.concurrency));
                let futs = stale.iter().map(|engine| {
                    Self::run_engine(*engine, self.pool.get_client(), &self.shared, &sem2, &opts)
                });
                let mut retry = FuturesUnordered::from_iter(futs);
                while !retry.is_empty() {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    let next = tokio::time::timeout(remaining, retry.next()).await;
                    let Some((name, result, elapsed)) = next.ok().flatten() else {
                        break;
                    };
                    match result {
                        Ok(items) => {
                            any_ok = true;
                            let rep = self.record_ok(name, items, elapsed, &mut answers, &mut raw);
                            self.replace_report(&mut reports, name, rep);
                        }
                        Err(e) => {
                            if std::env::var_os("PHRONA_DEBUG_BOOTSTRAP").is_some() {
                                eprintln!("[dbg bootstrap] {name} retry failed: {e}");
                            }
                            let rep = self.record_err(name, &e, elapsed);
                            self.replace_report(&mut reports, name, rep);
                        }
                    }
                }
            }
        }

        // deterministic output: reports arrived in completion order
        reports.sort_by(|a, b| a.name.cmp(&b.name));

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
/// index (position becomes `idx + 1`). Always returns `Some` today; the
/// `Option` reserves future fallible mappings without breaking callers.
///
/// Newer categories reuse existing shapes on purpose (wire-compatible):
/// `Code` and `Archives` render as web results (extra metadata folded
/// into the description), `Papers` as book results (author/publisher/info
/// fit papers exactly).
pub fn to_result_item(g: GroupedResult, score: f64, idx: usize) -> Option<ResultItem> {
    let raw = g.result;
    let category = crate::engine::category_of_engine(&raw.engine);
    let position = idx + 1;
    match category {
        Category::Web | Category::Code | Category::Archives => Some(ResultItem::Web(WebResult {
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
        Category::Books | Category::Papers => Some(ResultItem::Book(crate::models::BookResult {
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
/// Inside a multi-thread runtime this yields the worker while blocking
/// instead of deadlocking; on a current-thread runtime (or when no runtime
/// exists and the shared one is used) it blocks directly. Prefer
/// [`SearchClient::search`] from async code.
pub fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
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

    #[tokio::test]
    async fn blank_query_is_rejected_without_network() {
        let client = SearchClient::new().unwrap();
        for q in ["", "   ", "\t\n "] {
            let err = client
                .search(SearchOptions::new(q))
                .await
                .expect_err("blank query must fail");
            assert!(
                matches!(err.kind(), crate::error::ErrorKind::InvalidQuery { .. }),
                "got: {err}"
            );
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
            // newer categories reuse wire shapes (see to_result_item docs)
            ("github", "web"),
            ("archive_org", "web"),
            ("arxiv", "book"),
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
