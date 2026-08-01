//! MetaSearchRS MCP server.
//!
//! Exposes the metasearch library to AI agents over stdio (JSON-RPC).
//! Tools are compartmentalized per capability: per-category search,
//! suggestions, page extraction and grounded search for RAG.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::tool_router;
use rmcp::transport::stdio;
use rmcp::{ServiceExt, tool};
use schemars::JsonSchema;

use metasearch::models::{Category, TimeRange};
use metasearch::{ResultItem, SearchClient, SearchOptions};

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
    #[schemars(description = "Time range: day, week, month or year")]
    #[serde(default)]
    time_range: Option<String>,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
struct FetchParams {
    #[schemars(description = "URL to fetch and extract readable content from")]
    url: String,
    #[schemars(description = "Maximum characters of extracted text (default 8000)")]
    #[serde(default)]
    max_chars: Option<usize>,
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
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
struct EnginesParams {
    #[schemars(description = "Category: web, images, news, videos or books (default: all)")]
    #[serde(default)]
    category: Option<String>,
}

#[derive(Clone)]
struct MetaSearchMcp {
    client: std::sync::Arc<SearchClient>,
}

impl MetaSearchMcp {
    fn new() -> Self {
        Self {
            client: std::sync::Arc::new(SearchClient::new().expect("build search client")),
        }
    }

    fn build_opts(p: &SearchParams, category: Category) -> SearchOptions {
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
            opts.max_results = m.clamp(1, 100);
        }
        opts.region = p.region.clone();
        if let Some(t) = &p.time_range {
            opts.time_range = t.parse::<TimeRange>().ok();
        }
        opts
    }

    async fn run_search(&self, p: &SearchParams, category: Category) -> String {
        match self.client.search(Self::build_opts(p, category)).await {
            Ok(resp) => {
                let items: Vec<serde_json::Value> = resp
                    .results
                    .iter()
                    .map(|r| serde_json::to_value(r).unwrap_or(serde_json::Value::Null))
                    .collect();
                serde_json::to_string_pretty(&serde_json::json!({
                    "query": resp.query,
                    "total": resp.total,
                    "results": items,
                    "suggestions": resp.suggestions,
                }))
                .unwrap_or_else(|e| e.to_string())
            }
            Err(e) => serde_json::json!({"error": e.to_string()}).to_string(),
        }
    }
}

#[tool_router(server_handler)]
impl MetaSearchMcp {
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
        match metasearch::extract(
            self.client.http(),
            &p.url,
            p.max_chars.unwrap_or(8000),
            None,
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
        let region = "us-en";
        match &p.source {
            Some(name) => {
                let Some(source) = metasearch::SuggestSource::from_name(name) else {
                    return serde_json::json!({"error": format!("unknown source '{name}'")})
                        .to_string();
                };
                match metasearch::suggest(self.client.http(), source, &p.query, region).await {
                    Ok(list) => {
                        serde_json::json!({"query": p.query, "source": name, "suggestions": list})
                            .to_string()
                    }
                    Err(e) => serde_json::json!({"error": e.to_string()}).to_string(),
                }
            }
            None => {
                let all = metasearch::suggest_all(self.client.http(), &p.query, region).await;
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
            let names: Vec<String> = metasearch::available_engines(cat)
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
        let opts = Self::build_opts(&p, Category::Web);
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = MetaSearchMcp::new();
    let server = service.serve(stdio()).await?;
    let _ = server.waiting().await?;
    Ok(())
}
