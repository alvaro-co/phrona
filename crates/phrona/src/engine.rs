use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::RwLock;

use crate::client::HttpClient;
use crate::error::Result;
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
    /// `parking_lot::RwLock`: synchronous reads/writes, never blocks the
    /// async runtime (no `.await` involved).
    pub vqd: RwLock<HashMap<String, (Instant, String)>>,
    pub sc: RwLock<Option<(Instant, String)>>,
}

impl EngineShared {
    pub fn new() -> Self {
        Self::default()
    }
}

const CACHE_TTL: Duration = Duration::from_secs(3600);

impl EngineShared {
    pub fn vqd_get(&self, key: &str) -> Option<String> {
        let m = self.vqd.read();
        m.get(key)
            .filter(|(at, _)| at.elapsed() < CACHE_TTL)
            .map(|(_, v)| v.clone())
    }

    pub fn vqd_set(&self, key: &str, value: String) {
        self.vqd
            .write()
            .insert(key.to_string(), (Instant::now(), value));
    }

    pub fn sc_get(&self) -> Option<String> {
        let m = self.sc.read();
        m.as_ref()
            .filter(|(at, _)| at.elapsed() < CACHE_TTL)
            .map(|(_, v)| v.clone())
    }

    pub fn sc_set(&self, value: String) {
        *self.sc.write() = Some((Instant::now(), value));
    }
}

#[async_trait]
pub trait Engine: Send + Sync {
    fn name(&self) -> &'static str;
    fn category(&self) -> Category;
    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>>;
}

/// All engines, ordered by priority (best first).
pub fn list() -> &'static [&'static dyn Engine] {
    &[
        &crate::engines::duckduckgo::DuckDuckGo,
        &crate::engines::google::Google,
        &crate::engines::bing::Bing,
        &crate::engines::brave::Brave,
        &crate::engines::mojeek::Mojeek,
        &crate::engines::yahoo::Yahoo,
        &crate::engines::yandex::Yandex,
        &crate::engines::startpage::Startpage,
        &crate::engines::qwant::Qwant,
        &crate::engines::marginalia::Marginalia,
        &crate::engines::wikipedia::Wikipedia,
        &crate::engines::grokipedia::Grokipedia,
        &crate::engines::duckduckgo_images::DuckDuckGoImages,
        &crate::engines::bing_images::BingImages,
        &crate::engines::brave_images::BraveImages,
        &crate::engines::startpage_images::StartpageImages,
        &crate::engines::mojeek_images::MojeekImages,
        &crate::engines::google_images::GoogleImages,
        &crate::engines::duckduckgo_news::DuckDuckGoNews,
        &crate::engines::bing_news::BingNews,
        &crate::engines::yahoo_news::YahooNews,
        &crate::engines::brave_news::BraveNews,
        &crate::engines::duckduckgo_videos::DuckDuckGoVideos,
        &crate::engines::bing_videos::BingVideos,
        &crate::engines::brave_videos::BraveVideos,
        &crate::engines::annas_archive::AnnasArchive,
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
