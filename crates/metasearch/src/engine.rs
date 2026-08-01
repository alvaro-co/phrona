use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::client::HttpClient;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult};
use crate::options::SearchOptions;

/// Per-search context handed to every engine.
pub struct EngineContext<'a> {
    pub client: &'a HttpClient,
    pub opts: &'a SearchOptions,
    /// Shared cross-engine caches (vqd tokens, anti-bot tokens).
    pub shared: &'a EngineShared,
}

#[derive(Default)]
pub struct EngineShared {
    pub vqd: Mutex<HashMap<String, (Instant, String)>>,
    pub sc: Mutex<Option<(Instant, String)>>,
}

impl EngineShared {
    pub fn new() -> Self {
        Self::default()
    }
}

const CACHE_TTL: Duration = Duration::from_secs(3600);

impl EngineShared {
    pub async fn vqd_get(&self, key: &str) -> Option<String> {
        let m = self.vqd.lock().await;
        m.get(key)
            .filter(|(at, _)| at.elapsed() < CACHE_TTL)
            .map(|(_, v)| v.clone())
    }

    pub async fn vqd_set(&self, key: &str, value: String) {
        self.vqd
            .lock()
            .await
            .insert(key.to_string(), (Instant::now(), value));
    }

    pub async fn sc_get(&self) -> Option<String> {
        let m = self.sc.lock().await;
        m.as_ref()
            .filter(|(at, _)| at.elapsed() < CACHE_TTL)
            .map(|(_, v)| v.clone())
    }

    pub async fn sc_set(&self, value: String) {
        *self.sc.lock().await = Some((Instant::now(), value));
    }
}

#[async_trait]
pub trait Engine: Send + Sync {
    fn name(&self) -> &'static str;
    fn category(&self) -> Category;
    fn max_page(&self) -> u32 {
        1
    }
    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>>;
}

/// All engines, ordered by priority (best first).
pub fn list() -> &'static [&'static dyn Engine] {
    &[
        &DuckDuckGo,
        &Google,
        &Bing,
        &Brave,
        &Mojeek,
        &Yahoo,
        &Yandex,
        &Startpage,
        &Qwant,
        &Wikipedia,
        &Grokipedia,
        &DuckDuckGoImages,
        &BingImages,
        &BraveImages,
        &StartpageImages,
        &MojeekImages,
        &GoogleImages,
        &DuckDuckGoNews,
        &BingNews,
        &YahooNews,
        &BraveNews,
        &DuckDuckGoVideos,
        &BingVideos,
        &BraveVideos,
        &AnnasArchive,
    ]
}

pub fn engines_for(category: Category) -> Vec<&'static dyn Engine> {
    list()
        .iter()
        .copied()
        .filter(|e| e.category() == category)
        .collect()
}

pub fn engine_by_name(name: &str) -> Option<&'static dyn Engine> {
    list().iter().copied().find(|e| e.name() == name)
}

/// Category of an engine given its registered name.
pub fn category_of_engine(name: &str) -> Category {
    engine_by_name(name)
        .map(|e| e.category())
        .unwrap_or(Category::Web)
}

/// Resolve the engine set for a search: explicit list, or all for the category.
pub fn resolve(opts: &SearchOptions, category: Category) -> Vec<&'static dyn Engine> {
    let all = engines_for(category);
    if opts.engines.is_empty() {
        all
    } else {
        let wanted: Vec<String> = opts
            .engines
            .iter()
            .flat_map(|e| e.split(','))
            .map(|e| e.trim().to_ascii_lowercase())
            .collect();
        all.into_iter()
            .filter(|e| wanted.iter().any(|w| w == e.name()))
            .collect()
    }
}

/// Fetch a URL body as text (used by the `extract` feature).
pub async fn fetch_text(client: &HttpClient, url: &str) -> Result<String> {
    let resp = client.get(url).await?;
    if !resp.status().is_success() {
        return Err(crate::error::Error::Http(format!(
            "status {}",
            resp.status()
        )));
    }
    let bytes = resp.bytes().await.map_err(Error::from)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub use crate::engines::annas_archive::AnnasArchive;
pub use crate::engines::bing::Bing;
pub use crate::engines::bing_images::BingImages;
pub use crate::engines::bing_news::BingNews;
pub use crate::engines::bing_videos::BingVideos;
pub use crate::engines::brave::Brave;
pub use crate::engines::brave_images::BraveImages;
pub use crate::engines::brave_news::BraveNews;
pub use crate::engines::brave_videos::BraveVideos;
pub use crate::engines::duckduckgo::DuckDuckGo;
pub use crate::engines::duckduckgo_images::DuckDuckGoImages;
pub use crate::engines::duckduckgo_news::DuckDuckGoNews;
pub use crate::engines::duckduckgo_videos::DuckDuckGoVideos;
pub use crate::engines::google::Google;
pub use crate::engines::google_images::GoogleImages;
pub use crate::engines::grokipedia::Grokipedia;
pub use crate::engines::mojeek::Mojeek;
pub use crate::engines::mojeek_images::MojeekImages;
pub use crate::engines::qwant::Qwant;
pub use crate::engines::startpage::Startpage;
pub use crate::engines::startpage_images::StartpageImages;
pub use crate::engines::wikipedia::Wikipedia;
pub use crate::engines::yahoo::Yahoo;
pub use crate::engines::yahoo_news::YahooNews;
pub use crate::engines::yandex::Yandex;
