use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Web,
    Images,
    News,
    Videos,
    Books,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Web,
        Category::Images,
        Category::News,
        Category::Videos,
        Category::Books,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Category::Web => "web",
            Category::Images => "images",
            Category::News => "news",
            Category::Videos => "videos",
            Category::Books => "books",
        }
    }
}

impl std::str::FromStr for Category {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "web" | "text" | "general" => Ok(Category::Web),
            "images" | "image" | "img" => Ok(Category::Images),
            "news" => Ok(Category::News),
            "videos" | "video" | "vid" => Ok(Category::Videos),
            "books" | "book" => Ok(Category::Books),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SafeSearch {
    Off,
    Moderate,
    Strict,
}

impl std::str::FromStr for SafeSearch {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "off" | "none" | "0" => Ok(SafeSearch::Off),
            "moderate" | "1" => Ok(SafeSearch::Moderate),
            "strict" | "on" | "2" => Ok(SafeSearch::Strict),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeRange {
    Day,
    Week,
    Month,
    Year,
}

impl std::str::FromStr for TimeRange {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "day" | "d" | "24h" => Ok(TimeRange::Day),
            "week" | "w" | "7d" => Ok(TimeRange::Week),
            "month" | "m" | "30d" => Ok(TimeRange::Month),
            "year" | "y" | "365d" => Ok(TimeRange::Year),
            _ => Err(()),
        }
    }
}

/// A unified raw result produced by an engine; fields not applicable to the
/// category are left empty.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RawResult {
    pub title: String,
    pub url: String,
    pub description: String,
    pub image_url: String,
    pub thumbnail_url: String,
    pub width: u32,
    pub height: u32,
    pub published: Option<String>,
    pub source: String,
    pub author: String,
    pub duration: String,
    pub views: u64,
    pub publisher: String,
    pub uploader: String,
    pub engine: String,
    pub position: u32,
}

impl RawResult {
    pub fn new(engine: &str) -> Self {
        RawResult {
            engine: engine.to_string(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebResult {
    pub title: String,
    pub url: String,
    pub description: String,
    pub engines: Vec<String>,
    pub position: usize,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageResult {
    pub title: String,
    pub url: String,
    pub image_url: String,
    pub thumbnail_url: String,
    pub width: u32,
    pub height: u32,
    pub source: String,
    pub engines: Vec<String>,
    pub position: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsResult {
    pub title: String,
    pub url: String,
    pub description: String,
    pub published: Option<String>,
    pub source: String,
    pub image_url: String,
    pub engines: Vec<String>,
    pub position: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoResult {
    pub title: String,
    pub url: String,
    pub description: String,
    pub duration: String,
    pub published: Option<String>,
    pub uploader: String,
    pub views: u64,
    pub thumbnail_url: String,
    pub engines: Vec<String>,
    pub position: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookResult {
    pub title: String,
    pub author: String,
    pub publisher: String,
    pub info: String,
    pub url: String,
    pub thumbnail_url: String,
    pub engines: Vec<String>,
    pub position: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ResultItem {
    Web(WebResult),
    Image(ImageResult),
    News(NewsResult),
    Video(VideoResult),
    Book(BookResult),
}

impl ResultItem {
    pub fn engine_list(&self) -> &[String] {
        match self {
            ResultItem::Web(r) => &r.engines,
            ResultItem::Image(r) => &r.engines,
            ResultItem::News(r) => &r.engines,
            ResultItem::Video(r) => &r.engines,
            ResultItem::Book(r) => &r.engines,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineReport {
    pub name: String,
    pub status: String,
    pub results: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub category: Category,
    pub page: u32,
    pub total: usize,
    pub results: Vec<ResultItem>,
    pub suggestions: Vec<String>,
    pub answer: Option<String>,
    pub engines: Vec<EngineReport>,
    pub elapsed_ms: u64,
}

impl SearchResponse {
    pub fn web(&self) -> impl Iterator<Item = &WebResult> {
        self.results.iter().filter_map(|r| match r {
            ResultItem::Web(w) => Some(w),
            _ => None,
        })
    }
}
