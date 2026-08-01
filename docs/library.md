# Rust library reference

The core crate is `metasearch` (`crates/metasearch`). Everything public is
re-exported from the crate root.

## HTTP client

`HttpClient` wraps wreq with browser impersonation.

```rust
use metasearch::{HttpClient, HttpClientBuilder, Profile};

let client = HttpClient::builder()
    .profile(Profile::Chrome)      // or Chrome149, Firefox, Safari, Random, ...
    .timeout(std::time::Duration::from_secs(15))
    .cookies(true)
    .redirects(10)
    .header("Accept-Language", "en-US,en;q=0.9")
    .proxy(Some("socks5://127.0.0.1:9050".into()))
    .build()?;

let resp = client.get("https://example.com").await?;
let body = resp.bytes().await?;
```

Profiles: `Chrome` (148), `Chrome100`, `Chrome120`, `Chrome131`, `Chrome140`,
`Chrome149`, `Firefox` (148), `Firefox139`, `Edge` (148), `Safari` (26),
`Opera` (131), `OkHttp`, `Random` (random per request). `Random` is useful
for bot-heavy sites but costs pool efficiency.

`HttpClient` methods: `get`, `get_with_headers`, `post_form`, `post_form_with_headers`.

## Options

`SearchOptions` (all fields public, `Default` and `SearchOptions::new(query)`):

```rust
pub struct SearchOptions {
    pub query: String,
    pub category: Category,          // Web | Images | News | Videos | Books
    pub engines: Vec<String>,        // empty = all engines for the category
    pub page: u32,
    pub max_results: usize,
    pub safesearch: SafeSearch,      // Off | Moderate | Strict
    pub region: Option<String>,      // e.g. "us-en", "de-de"
    pub language: Option<String>,    // e.g. "en"
    pub time_range: Option<TimeRange>, // Day | Week | Month | Year
    pub filters: Option<String>,     // engine-specific filter string
    pub profile: Profile,
    pub timeout: Duration,
    pub proxies: Vec<String>,        // used as a pool, first is tried first
}
```

`Category`, `SafeSearch` and `TimeRange` implement `FromStr` so
`"images".parse::<Category>()` works. `Category::ALL` lists categories.

## Search

```rust
use metasearch::{SearchClient, SearchOptions, ResultItem};

let client = SearchClient::new()?;                  // Chrome profile
let opts = SearchOptions {
    max_results: 30,
    engines: vec!["bing".into(), "brave".into(), "startpage".into()],
    ..SearchOptions::new("rust programming")
};
let resp = client.search(opts).await?;              // async
// or from a non-async context:
let resp = client.search_sync(opts)?;
```

`SearchResponse` fields:

```rust
pub struct SearchResponse {
    pub query: String,
    pub category: Category,
    pub page: u32,
    pub total: usize,                 // number of results kept after merge
    pub results: Vec<ResultItem>,     // typed union
    pub suggestions: Vec<String>,     // on web category, page 1
    pub answer: Option<String>,       // from answer engines (grokipedia)
    pub engines: Vec<EngineReport>,   // per-engine status/result count/error
    pub elapsed_ms: u64,
}
```

`ResultItem` is a tagged enum (`{"type": "web" | "image" | ...}` in JSON):

- `Web`: title, url, description, engines, position, score
- `Image`: title, url, image_url, thumbnail_url, width, height, source
- `News`: title, url, description, published, source, image_url
- `Video`: title, url, description, duration, published, uploader, views, thumbnail_url
- `Book`: title, author, publisher, info, url, thumbnail_url

Convenience helpers: `metasearch::search(opts).await`, `metasearch::search_sync(opts)`.

`EngineReport { name, status, results, error }` reports what each engine
did ("ok", "empty" or "error"), so callers can degrade gracefully.

## Suggestions

```rust
use metasearch::{SuggestSource, suggest, suggest_all};

let list = suggest(&client.http(), SuggestSource::Bing, "rust", "us-en").await?;
let all  = suggest_all(&client.http(), "rust", "us-en").await; // all sources
```

Sources: DuckDuckGo, Google, Bing, Brave, Startpage, Qwant, Wikipedia.

## Page extraction (AI grounding)

```rust
use metasearch::{extract, ExtractedPage};

let page: ExtractedPage = extract(&client.http(), "https://doc.rust-lang.org/book/", 8000, None).await?;
// page.title, page.description, page.text, page.images
```

`extract_from_html` is the pure HTML variant. Pass a `query` to bias the
excerpt selection toward the relevant fragment.

## Engines

`metasearch::engine` exposes:

- `engine::list()` - all registered engines
- `engine::engines_for(category)` - engines of a category
- `engine::engine_by_name(name)` - lookup
- `metasearch::available_engines(category)` - names for the REST API

## Errors

`metasearch::Error` covers HTTP, parse, engine and no-results failures; every
variant implements `Display`. Engines never panic on malformed responses.

## Merging, dedup, ranking

- `dedup::dedup_key(url)` - normalizes a URL (lowercase host, strips tracking
  params: utm_*, fbclid, gclid, ref, source, si, spm, s, ...).
- `dedup::group(raw)` - groups results; empty-URL items become answer markers.
- `rank::rank(groups, query)` - scores by cross-engine agreement (+1.5 per
  extra engine), position (10/pos, capped), Wikipedia/Grokipedia bonus, and
  query-term text match on title/description.

## Sync vs async

Both APIs exist everywhere. `search_sync` runs the ambient tokio runtime when
present, otherwise a shared internal runtime, so it is safe to call from
plain threads and from within async contexts.

## Feature flags

None - the crate builds with default features.
