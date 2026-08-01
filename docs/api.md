# REST API reference

`cargo run -p metasearch-api`

Serves the web frontend at `/` and JSON at `/v1/*`. CORS is permissive
(access-control-allow-origin: *), so the API can be called from any browser
origin. Responses are JSON, all timings in milliseconds.

## Environment

| Variable | Default | Purpose |
| --- | --- | --- |
| `META_ADDR` | `127.0.0.1:8080` | bind address |
| `META_API_KEY` | unset | if set, `/v1/*` (except `/health`) and `/search` require a key |
| `RUST_LOG` | `info` | tracing level (use `debug` for detail) |

When a key is set, clients authenticate with the header
`Authorization: Bearer <key>`, the query parameter `api_key=...` or the JSON
field `"api_key": "..."`.

## GET /

The web frontend (index.html). See [docs/frontend.md](frontend.md).

## GET /health

```json
{"status":"ok","engines":{"web":11,"images":6,"news":4,"videos":3,"books":1},"uptime_s":42}
```

No auth required.

## GET /v1/engines

```json
{"category":"web","engines":["bing","brave","mojeek","qwant","startpage","wikipedia","grokipedia","yahoo","yandex","duckduckgo","google"]}
```

Optional `category` query parameter. Auth required if a key is set.

## GET /v1/search

Query parameters:

| Param | Type | Default | Meaning |
| --- | --- | --- | --- |
| `q` | string | required | the query |
| `category` | string | `web` | `web`, `images`, `news`, `videos`, `books` |
| `engines` | string | all of category | comma-separated engine names |
| `page` | uint | 1 | result page |
| `max_results` | uint | 30 | max merged results to return |
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
    {"name": "google", "status": "error", "error": "http ... 429"}
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

Errors: HTTP 400 (bad params, e.g. unknown category or engine), 401
(auth required / wrong key), 502 (all engines failed, JSON `{"error": "..."}`).

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
| `api_key` | auth (any value works when `META_API_KEY` is unset) |
| `search_depth` | `basic` or `advanced` (advanced sets `max_results` to 20) |
| `topic` | `general` or `news` (news: category=news, time_range=week) |
| `days` | news recency window |
| `max_results` | default 10 |
| `include_images` | adds `images` field |
| `include_answer` | adds `answer` field (from the LLM-style answer engine) |
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

AI grounding for RAG: returns a verbatim excerpt from the page best matching
the query plus ranked sources, all with citation-ready attribution.

Query params (GET) or JSON body (POST):

```json
{"query": "serde json", "search": {"max_results": 5}, "max_excerpt_chars": 500}
```

Response:

```json
{
  "query": "serde json",
  "summary": "short answer (derived from the top excerpt)",
  "excerpt": "verbatim page text around the best match",
  "page_url": "https://serde.rs/json.html",
  "page_title": "JSON Format - serde",
  "sources": [{"title": "...", "url": "...", "score": 0.9}]
}
```

`search` is a partial GET /v1/search parameter set (query comes from the
top-level `query`).

## Frontend

The static app lives in `frontend/` next to the workspace and is served
without embedding: edit `frontend/index.html`, `frontend/style.css`,
`frontend/app.js` and restart the server - no rebuild needed. The fallback
serves index.html for any other path.
