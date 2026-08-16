//! Phrona MCP server.
//!
//! Exposes the phrona library to AI agents over stdio (JSON-RPC).
//! Tools are compartmentalized per capability: per-category search,
//! suggestions, page extraction and grounded search for RAG.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::tool_router;
use rmcp::transport::stdio;
use rmcp::{ServiceExt, tool};
use schemars::JsonSchema;

use phrona::models::{Category, TimeRange};
use phrona::{PhronaConfig, ResultItem, SearchClient, SearchOptions};

#[derive(Debug, serde::Deserialize, JsonSchema)]
struct SearchParams {
    #[schemars(description = "Search query")]
    query: String,
    #[schemars(
        description = "Comma-separated engine names (default: all available for the category). See list_engines."
    )]
    #[serde(default)]
    engines: Option<String>,
    #[schemars(description = "Maximum number of results (default 10)")]
    #[serde(default)]
    max_results: Option<usize>,
    #[schemars(description = "Region code, e.g. us-en (default from client)")]
    #[serde(default)]
    region: Option<String>,
    #[schemars(description = "Language code, e.g. en")]
    #[serde(default)]
    language: Option<String>,
    #[schemars(description = "Time range: day, week, month or year")]
    #[serde(default)]
    time_range: Option<String>,
    #[schemars(description = "SafeSearch level: off, moderate or strict (default moderate)")]
    #[serde(default)]
    safesearch: Option<String>,
    #[schemars(description = "Engine-specific filter string, e.g. site:example.com")]
    #[serde(default)]
    filters: Option<String>,
    #[schemars(description = "Result page (default 1)")]
    #[serde(default)]
    page: Option<u32>,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
struct FetchParams {
    #[schemars(description = "URL to fetch and extract readable content from")]
    url: String,
    #[schemars(description = "Maximum characters of extracted text (default 8000)")]
    #[serde(default)]
    max_chars: Option<usize>,
    #[schemars(
        description = "Query used to bias the excerpt toward the relevant section (optional)"
    )]
    #[serde(default)]
    query: Option<String>,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
struct SuggestParams {
    #[schemars(description = "Partial query to complete")]
    query: String,
    #[schemars(
        description = "Source: duckduckgo, google, bing, brave, startpage, qwant or wikipedia (default: all)"
    )]
    #[serde(default)]
    source: Option<String>,
    #[schemars(description = "Region code, e.g. us-en (default us-en)")]
    #[serde(default)]
    region: Option<String>,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
struct EnginesParams {
    #[schemars(description = "Category: web, images, news, videos or books (default: all)")]
    #[serde(default)]
    category: Option<String>,
}

#[derive(Clone)]
struct PhronaMcp {
    client: std::sync::Arc<SearchClient>,
    max_results_limit: usize,
}

impl PhronaMcp {
    /// Build the server from a typed config: profile, timeout, proxies and
    /// the `max_results` clamp all come from it.
    fn with_config(cfg: &PhronaConfig) -> Self {
        Self {
            client: std::sync::Arc::new(cfg.search_client().expect("build search client")),
            max_results_limit: cfg.max_results_limit(),
        }
    }

    /// Map tool arguments to search options; invalid enums are rejected
    /// loudly instead of silently coerced.
    fn build_opts(
        p: &SearchParams,
        category: Category,
        max_results_limit: usize,
    ) -> Result<SearchOptions, String> {
        let mut opts = SearchOptions::new(p.query.clone());
        opts.category = category;
        if let Some(es) = &p.engines {
            opts.engines = es
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
        if let Some(m) = p.max_results {
            opts.max_results = m.clamp(1, max_results_limit);
        }
        opts.region = p.region.clone();
        opts.language = p.language.clone();
        if let Some(t) = &p.time_range {
            opts.time_range = Some(
                t.parse::<TimeRange>()
                    .map_err(|_| "invalid time_range, expected day|week|month|year".to_string())?,
            );
        }
        if let Some(s) = &p.safesearch {
            opts.safesearch = s
                .parse::<phrona::SafeSearch>()
                .map_err(|_| "invalid safesearch, expected off|moderate|strict".to_string())?;
        }
        opts.filters = p.filters.clone();
        if let Some(page) = p.page {
            opts.page = page.max(1);
        }
        Ok(opts)
    }

    async fn run_search(&self, p: &SearchParams, category: Category) -> String {
        let opts = match Self::build_opts(p, category, self.max_results_limit) {
            Ok(opts) => opts,
            Err(msg) => return serde_json::json!({"error": msg}).to_string(),
        };
        match self.client.search(opts).await {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_else(|e| e.to_string()),
            Err(e) => serde_json::json!({"error": e.to_string()}).to_string(),
        }
    }
}

#[tool_router(server_handler)]
impl PhronaMcp {
    #[tool(
        description = "Search the web across multiple metasearch engines. Returns ranked results with title, url, description and the engines that found each one."
    )]
    async fn web_search(&self, Parameters(p): Parameters<SearchParams>) -> String {
        self.run_search(&p, Category::Web).await
    }

    #[tool(
        description = "Search images across multiple engines. Returns direct image urls, thumbnails and dimensions."
    )]
    async fn image_search(&self, Parameters(p): Parameters<SearchParams>) -> String {
        self.run_search(&p, Category::Images).await
    }

    #[tool(
        description = "Search news across multiple engines. Returns articles with published date and source."
    )]
    async fn news_search(&self, Parameters(p): Parameters<SearchParams>) -> String {
        self.run_search(&p, Category::News).await
    }

    #[tool(
        description = "Search videos across multiple engines. Returns video url, duration, views and uploader."
    )]
    async fn video_search(&self, Parameters(p): Parameters<SearchParams>) -> String {
        self.run_search(&p, Category::Videos).await
    }

    #[tool(description = "Search books and academic material.")]
    async fn book_search(&self, Parameters(p): Parameters<SearchParams>) -> String {
        self.run_search(&p, Category::Books).await
    }

    #[tool(
        description = "Fetch a URL and extract its readable main content. Use for grounding answers on the sources returned by web_search."
    )]
    async fn fetch_page(&self, Parameters(p): Parameters<FetchParams>) -> String {
        match phrona::extract(
            self.client.http(),
            &p.url,
            p.max_chars.unwrap_or(8000),
            p.query.as_deref(),
        )
        .await
        {
            Ok(page) => serde_json::json!({
                "url": page.url,
                "title": page.title,
                "description": page.description,
                "text": page.text,
            })
            .to_string(),
            Err(e) => serde_json::json!({"error": e.to_string()}).to_string(),
        }
    }

    #[tool(description = "Get query suggestions for auto-completion from search engines.")]
    async fn suggest(&self, Parameters(p): Parameters<SuggestParams>) -> String {
        let region = p.region.as_deref().unwrap_or("us-en");
        match &p.source {
            Some(name) => {
                let Some(source) = phrona::SuggestSource::from_name(name) else {
                    return serde_json::json!({"error": format!("unknown source '{name}'")})
                        .to_string();
                };
                match phrona::suggest(self.client.http(), source, &p.query, region).await {
                    Ok(list) => {
                        serde_json::json!({"query": p.query, "source": name, "suggestions": list})
                            .to_string()
                    }
                    Err(e) => serde_json::json!({"error": e.to_string()}).to_string(),
                }
            }
            None => {
                let all = phrona::suggest_all(self.client.http(), &p.query, region).await;
                let map: serde_json::Map<String, _> = all
                    .into_iter()
                    .map(|(s, list)| (s.name().to_string(), serde_json::json!(list)))
                    .collect();
                serde_json::json!({"query": p.query, "suggestions": map}).to_string()
            }
        }
    }

    #[tool(
        description = "List the search engines available per category. Pass engine names to other tools to restrict them."
    )]
    fn list_engines(&self, Parameters(p): Parameters<EnginesParams>) -> String {
        let cats: Vec<Category> = match p.category.as_deref() {
            Some(c) => c.parse::<Category>().map(|c| vec![c]).unwrap_or_default(),
            None => Category::ALL.to_vec(),
        };
        let mut out = serde_json::Map::new();
        for cat in cats {
            let names: Vec<String> = phrona::available_engines(cat)
                .iter()
                .map(|e| e.name.clone())
                .collect();
            out.insert(cat.as_str().to_string(), serde_json::json!(names));
        }
        serde_json::json!({"engines": out}).to_string()
    }

    #[tool(
        description = "Grounded search for RAG: returns a synthesized answer plus ranked sources with content. Prefer this over web_search + fetch_page for single-shot questions."
    )]
    async fn search_grounded(&self, Parameters(p): Parameters<SearchParams>) -> String {
        let opts = match Self::build_opts(&p, Category::Web, self.max_results_limit) {
            Ok(opts) => opts,
            Err(msg) => return serde_json::json!({"error": msg}).to_string(),
        };
        match self.client.search(opts).await {
            Ok(resp) => {
                let sources: Vec<serde_json::Value> = resp
                    .results
                    .iter()
                    .enumerate()
                    .filter_map(|(i, r)| {
                        let (title, url, content) = match r {
                            ResultItem::Web(w) => (&w.title, &w.url, &w.description),
                            ResultItem::News(n) => (&n.title, &n.url, &n.description),
                            ResultItem::Video(v) => (&v.title, &v.url, &v.description),
                            ResultItem::Image(im) => (&im.title, &im.url, &im.source),
                            ResultItem::Book(b) => (&b.title, &b.url, &b.info),
                        };
                        if content.is_empty() {
                            return None;
                        }
                        Some(serde_json::json!({
                            "title": title,
                            "url": url,
                            "content": content,
                            "score": (1.0 - i as f64 * 0.05).max(0.05),
                        }))
                    })
                    .collect();
                let answer = resp.answer.clone().unwrap_or_else(|| {
                    format!(
                        "Found {} sources for \"{}\". Inspect the sources for the full picture.",
                        sources.len(),
                        resp.query
                    )
                });
                serde_json::json!({
                    "query": resp.query,
                    "answer": answer,
                    "sources": sources,
                })
                .to_string()
            }
            Err(e) => serde_json::json!({"error": e.to_string()}).to_string(),
        }
    }
}

/// Serve the MCP server over stdio (JSON-RPC 2.0, newline-delimited).
/// Blocks until the client disconnects.
pub async fn run_stdio(cfg: &PhronaConfig) -> anyhow::Result<()> {
    let service = PhronaMcp::with_config(cfg);
    let server = service.serve(stdio()).await?;
    let _ = server.waiting().await?;
    Ok(())
}

/// Serve the MCP server over a TCP listener (newline-delimited JSON-RPC,
/// the same framing as stdio). Each connection is served in its own task.
/// Blocks until the listener is closed.
pub async fn serve_tcp(listener: tokio::net::TcpListener, cfg: PhronaConfig) -> anyhow::Result<()> {
    tracing::info!("phrona-mcp listening on {}", listener.local_addr()?);
    loop {
        let (socket, _) = listener.accept().await?;
        let cfg = cfg.clone();
        tokio::spawn(async move {
            let service = PhronaMcp::with_config(&cfg);
            match service.serve(socket).await {
                Ok(server) => {
                    let _ = server.waiting().await;
                }
                Err(e) => tracing::debug!("mcp connection failed: {e}"),
            }
        });
    }
}

/// Build a TCP listener from an addr string (e.g. "127.0.0.1:8081").
pub async fn tcp_listener(addr: &str) -> anyhow::Result<tokio::net::TcpListener> {
    Ok(tokio::net::TcpListener::bind(addr).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(query: &str) -> SearchParams {
        SearchParams {
            query: query.to_string(),
            engines: None,
            max_results: None,
            region: None,
            language: None,
            time_range: None,
            safesearch: None,
            filters: None,
            page: None,
        }
    }

    #[test]
    fn build_opts_defaults() {
        let opts = PhronaMcp::build_opts(&params("rust"), Category::Web, 100).unwrap();
        assert_eq!(opts.category, Category::Web);
        assert_eq!(opts.max_results, 20);
        assert_eq!(opts.safesearch, phrona::SafeSearch::Moderate);
        assert_eq!(opts.page, 1);
    }

    #[test]
    fn build_opts_clamps_and_maps() {
        let mut p = params("rust");
        p.max_results = Some(5000);
        p.page = Some(0);
        let opts = PhronaMcp::build_opts(&p, Category::News, 50).unwrap();
        assert_eq!(opts.max_results, 50);
        assert_eq!(opts.page, 1);
        assert_eq!(opts.category, Category::News);
    }

    #[test]
    fn build_opts_rejects_bad_enums() {
        let mut p = params("rust");
        p.time_range = Some("yesterday".into());
        assert!(PhronaMcp::build_opts(&p, Category::Web, 100).is_err());
        p.time_range = None;
        p.safesearch = Some("medium".into());
        assert!(PhronaMcp::build_opts(&p, Category::Web, 100).is_err());
        p.safesearch = Some("strict".into());
        assert!(PhronaMcp::build_opts(&p, Category::Web, 100).is_ok());
    }
}
