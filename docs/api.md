# REST API reference

`cargo run -p phrona-api` (or `phrona serve`, or `phrona serve --no-mcp` for
the REST server only).

Serves the web frontend at `/`: a single page with a Search tab
(full-parameter search) and a Tools tab (suggest, extract, ground,
engines, test - every capability from the browser). JSON API at
`/v1/*`. CORS is permissive (access-control-allow-origin: *), so the API
can be called from any browser origin. Responses are JSON, all timings in
milliseconds.

## Environment

| Variable | Default | Purpose |
| --- | --- | --- |
| `PHRONA_ADDR` | `127.0.0.1:8080` | bind address |
| `PHRONA_API_KEY` | unset | if set, `/v1/*` (except `/health`) and `/search` require a key |
| `RUST_LOG` | `info` | tracing level (use `debug` for detail) |

When a key is set, clients authenticate with the header
`Authorization: Bearer <key>`, the header `x-api-key: <key>`, the query
parameter `api_key=...` or the JSON field `"api_key": "..."`.

## GET /

The web frontend (single page: Search + Tools tabs). See
[docs/frontend.md](frontend.md).

## GET /health

```json
{"status":"ok","version":"0.1.0","engines":{"web":12,"images":6,"news":4,"videos":3,"books":1},"uptime_s":42,"auth":false}
```

No auth required.

## GET /v1/engines

Optional `category` query parameter (`web | images | news | videos |
books`; default: all categories). Returns a map of category to engine
names, in priority order:

```json
{
  "web": ["duckduckgo", "google", "bing", "brave", "mojeek", "yahoo", "yandex", "startpage", "qwant", "marginalia", "wikipedia", "grokipedia"],
  "images": ["duckduckgo_images", "bing_images", "brave_images", "startpage_images", "mojeek_images", "google_images"],
  "news": ["duckduckgo_news", "bing_news", "yahoo_news", "brave_news"],
  "videos": ["duckduckgo_videos", "bing_videos", "brave_videos"],
  "books": ["annas_archive"]
}
```

Public - no auth required (same as `/health`; the frontend uses it
without a key).

## GET /v1/search

Query parameters:

| Param | Type | Default | Meaning |
| --- | --- | --- | --- |
| `q` | string | required | the query |
| `category` | string | `web` | `web`, `images`, `news`, `videos`, `books` |
| `engines` | string | all of category | comma-separated engine names |
| `page` | uint | 1 | result page |
| `max_results` | uint | 20 | max merged results to return (1-100) |
| `safesearch` | string | `moderate` | `off`, `moderate`, `strict` |
| `region` | string | unset | e.g. `us-en`, `de-de` |
| `language` | string | unset | e.g. `en` |
| `time_range` | string | unset | `day`, `week`, `month`, `year` |
| `filters` | string | unset | engine filter string (e.g. `site:github.com`) |
| `api_key` | string | - | auth when key set |

Response:

```json
{
  "query": "rust",
  "category": "web",
  "page": 1,
  "total": 8,
  "results": [
    {
      "type": "web",
      "title": "Rust Programming Language",
      "url": "https://www.rust-lang.org/",
      "description": "A language empowering everyone ...",
      "score": 1.0,
      "position": 1,
      "engines": ["bing", "brave"]
    }
  ],
  "suggestions": ["rust tutorial", "rust programming language"],
  "answer": null,
  "engines": [
    {"name": "bing", "status": "ok", "results": 10},
    {"name": "google", "status": "error", "error": "rate limited [scope=Provider, engine=google, status=429]", "scope": "Provider", "kind": "RateLimited { retry_after: Some(30s) }"}
  ],
  "elapsed_ms": 1200
}
```

Result fields by type:

- web: title, url, description, score, position, engines
- image: title, url, image_url, thumbnail_url, width, height, score, position, engines
- news: title, url, description, published, source, score, position, engines
- video: title, url, description, thumbnail_url, duration, views, uploader, score, position, engines
- book: title, url, description, author, publisher, score, position, engines

Errors: HTTP 400 (bad params, e.g. unknown category or engine, or an error
with `Query` scope), 401 (auth required / wrong key), 429 (rate limited),
500 (internal), 502/503 (egress/schema/provider failures, incl. all engines
failed, JSON `{"error": "..."}`).

## GET /v1/suggest

| Param | Type | Default | Meaning |
| --- | --- | --- | --- |
| `q` | string | required | query prefix |
| `source` | string | all sources | duckduckgo, google, bing, brave, startpage, qwant, wikipedia |
| `region` | string | `us-en` | locale |

```json
{"query":"rus","source":"bing","suggestions":["rust","rustup","russian"]}
```

Without `source`, returns a map of every source to its list.

## GET|POST /v1/extract

Readable-text extraction of a page (the same feature as `phrona extract` and
the library's `extract`). Query params (GET) or JSON body (POST):

| Field | Default | Meaning |
| --- | --- | --- |
| `url` | required | page to fetch and extract |
| `max_chars` | `5000` | max characters of extracted text (1-100000) |
| `query` | unset | bias the excerpt toward this query |

Response is the `ExtractedPage` shape:

```json
{
  "url": "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html",
  "title": "What Is Ownership? - The Rust Programming Language",
  "description": "...",
  "text": "...",
  "images": ["https://..."]
}
```

## GET /v1/test

Availability probe across every category (the same feature as
`phrona test`): runs a real search per category and reports per-engine
status, result counts and errors.

| Param | Default | Meaning |
| --- | --- | --- |
| `query` | `rust programming` | probe query |
| `category` | all categories | `web`, `images`, `news`, `videos`, `books` |
| `max_results` | `5` | merged results per category (1-10) |

Response: an array of per-category reports.

```json
[
  {
    "category": "web",
    "total": 8,
    "elapsed_ms": 1200,
    "answer": "...",
    "engines": [
      {"name": "bing", "status": "ok", "results": 10},
      {"name": "google", "status": "error", "error": "http ... 429"}
    ]
  }
]
```

## POST /search and POST /v1/tavily

Tavily-compatible endpoint for drop-in replacement in Tavily clients.

```json
{
  "query": "rust",
  "api_key": "...",
  "search_depth": "basic",
  "topic": "general",
  "days": 7,
  "max_results": 8,
  "include_images": false,
  "include_answer": false,
  "include_raw_content": false,
  "include_domains": ["example.com"],
  "exclude_domains": ["spam.net"]
}
```

| Field | Meaning |
| --- | --- |
| `query` | required |
| `api_key` | auth (any value works when `PHRONA_API_KEY` is unset) |
| `search_depth` | `basic` restricts to bing + duckduckgo (plus grokipedia when `include_answer`); `advanced` uses all web engines |
| `topic` | `general` or `news` (news: category=news, time_range=week) |
| `days` | news recency window |
| `max_results` | default 5, cap 20 |
| `include_images` | adds `images` field |
| `include_answer` | adds `answer` field; the grokipedia answer engine is queried for this |
| `include_raw_content` | adds `raw_content` (full extracted page text, capped) |
| `include_domains` / `exclude_domains` | filters returned results by host |

Response is the Tavily shape:

```json
{
  "query": "rust",
  "follow_up_questions": [],
  "answer": "",
  "images": [],
  "results": [
    {"title": "...", "url": "...", "content": "...", "score": 0.9, "raw_content": "..."}
  ],
  "response_time": 1.2
}
```

## GET|POST /v1/grounding

AI grounding for RAG: returns a synthesized extractive answer plus ranked
sources with content, all with citation-ready attribution. The library
answer (from the grokipedia answer engine) is used verbatim when present;
otherwise the strongest snippets are stitched into an extractive summary.

Query params (GET) or JSON body (POST):

```json
{"query": "serde json", "api_key": "...", "max_results": 10, "category": "web", "time_range": "week"}
```

Response:

```json
{
  "query": "serde json",
  "answer": "Extractive summary for \"serde json\":\nSource 1 (https://serde.rs/json.html): ...",
  "sources": [
    {"title": "JSON Format - serde", "url": "https://serde.rs/json.html", "content": "...", "score": 1.0}
  ],
  "response_time": 1.1
}
```

`max_results` clamps to 1-50 (default 10).

## Frontend

The static app lives in `crates/phrona-api/assets/` and is served
without embedding: edit `assets/index.html`, `assets/style.css`,
`frontend/app.js` and restart the server - no rebuild needed. The fallback
serves index.html for any other path.
