# Engine reference

26 engines in 5 categories, all static stateless `Engine` instances
registered in `crates/phrona/src/engine.rs` (order = priority).

| Name | Category | Endpoint |
| --- | --- | --- |
| duckduckgo | web | `https://html.duckduckgo.com/html/` |
| google | web | `https://www.google.com/search` |
| bing | web | `https://www.bing.com/search` |
| brave | web | `https://search.brave.com/search` |
| mojeek | web | `https://www.mojeek.com/search` |
| yahoo | web | `https://search.yahoo.com/search` |
| yandex | web | `https://yandex.com/search/site/` |
| startpage | web | `https://www.startpage.com/sp/search` |
| qwant | web | `https://api.qwant.com/v3/search/web` |
| marginalia | web | `https://old-search.marginalia.nu/search` |
| wikipedia | web | `https://en.wikipedia.org/w/api.php` |
| grokipedia | web | `https://grokipedia.com/page/...` |
| duckduckgo_images | images | `https://duckduckgo.com/i.js` |
| bing_images | images | `https://www.bing.com/images/async` |
| brave_images | images | `https://search.brave.com/images` |
| startpage_images | images | `https://www.startpage.com/sp/search` |
| mojeek_images | images | `https://www.mojeek.com/search` |
| google_images | images | `https://www.google.com/search` (tbm=isch) |
| duckduckgo_news | news | `https://duckduckgo.com/news.js` |
| bing_news | news | `https://www.bing.com/news/infinitescrollajax` |
| yahoo_news | news | `https://news.search.yahoo.com/search` |
| brave_news | news | `https://search.brave.com/news` |
| duckduckgo_videos | videos | `https://duckduckgo.com/v.js` |
| bing_videos | videos | `https://www.bing.com/videos/asyncv2` |
| brave_videos | videos | `https://search.brave.com/videos` |
| annas_archive | books | `https://annas-archive.gd/search` |

Suggestion sources (7): duckduckgo, google, bing, brave, startpage, qwant,
wikipedia.

## Engine details and quirks

- **bing** - HTML; `first=` paging; news via the
  `infinitescrollajax` JSON endpoint with a parallel fetch of the current
  article headline; videos from `asyncv2` with the result markup inside a
  `<noscript>` block (must be unwrapped before HTML parsing - see
  `regex_strip_noscript` in `bing_videos.rs`).
- **brave** - JSON API blobs extracted from HTML `data-serpapi` scripts
  (js_to_json); suggestion + results endpoints combined; thumbnail URLs are
  base64-encoded (`/g:ce/...`), decoded with URL-safe and standard base64
  (`brave_b64_decode`); page 2 via `?q=query&spellcheck=false&s=...`.
- **duckduckgo** - HTML endpoint with a plain GET (no `vqd` needed; the
  POST variant triggers the anomaly/bot page); images/news/videos use the
  JSON `vqd`-signed endpoints - the token is fetched from
  `duckduckgo.com/?vqd=...` once and cached in `EngineShared.vqd` per key
  (TTL 1 h).
- **google / google_images** - `/search` with `nfpr=1` (no autocorrect) and
  modern selectors (`[jscontroller=SC7lYd]`, `div[data-sncf]`) plus featured
  snippet as an answer; `tbm=isch` for images; **429-blocks from datacenter IPs** - the parser degrades to
  zero results gracefully and the HTTP-semantics check classifies the
  captcha page, so fixtures are never polluted.
- **marginalia** - `old-search.marginalia.nu`; only queries of 3 words or
  fewer with plain alphanumeric text are issued (the site's own limit),
  otherwise the engine returns nothing.
- **mojeek / mojeek_images** - HTML, images also served through the same
  HTML page; **403 on some networks** - same graceful degradation.
- **qwant** - JSON API `api.qwant.com/v3` for results and suggestions;
  **DataDome captcha on some networks**.
- **yandex** - `search/site/` endpoint (site-restricted Yandex HTML, the
  only variant reliably parseable); the standard `/search` may show the
  "there are no search results" anti-bot page - that page is also treated
  as a block page.
- **startpage** - token dance: GET `/sp/search` to obtain the `sc`
  token (cached in `EngineShared.sc`), then POST search; suggestions from
  `/suggestions`; images via POST with `tbm=isch`.
- **wikipedia** - OpenSearch API, cross-article "Search the web" for extra
  URLs; results get a strong ranking bonus.
- **grokipedia** - typeahead + `/page/` fetches; the typeahead snippet is
  returned as an **answer marker** (empty URL, capped at a sentence
  boundary) so responses carry a real `answer` field (used verbatim by
  `include_answer` in the Tavily endpoint and by the grounding endpoint),
  plus the page result itself.
- **yahoo / yahoo_news** - HTML, `fsr=1`-free parsing; news page has
  separate layout.
- **annas_archive** - HTML, publisher/info extraction, `link=t` ISBN
  variant kept.

## Response classification

Every engine funnels its HTTP response through `util::check_response(engine,
&resp, MediaType)` in `engines/util.rs`, which classifies purely from HTTP
metadata (RFC 9110) - never from body phrasing:

- header-level anti-bot signals first: `cf-mitigated` / `cf-challenge` /
  `cf-ray`+403 → `Blocked(Cloudflare)`, `x-datadome` → `Blocked(Captcha)`;
- `429 Too Many Requests` → `RateLimited` (honoring the `Retry-After`
  header);
- `403 Forbidden` → `Blocked(BotDetection)`;
- any other non-2xx status → `UpstreamUnavailable`;
- a 2xx response whose `Content-Type` contradicts the expected media type
  (`MediaType::Html` or `MediaType::Json`) → `MalformedPayload` (a
  structural deviation, not a rate limit).

A 2xx response of the expected type is trusted as-is; whether it contains
results is the parser's job - an empty parse is an honest "no results", not a
hidden failure. JSON endpoints additionally run the body through
`util::parse_json_body`, which validates that the payload is actually JSON
(grammar check; failures classify as `MalformedPayload`), and Qwant/DDG check
the API `status` field.

## Fixtures

`crates/phrona/tests/fixtures/` holds 26 captured live pages (one per
engine, plus qwant/videos/suggest variants) plus a `meta.json` sidecar that
records each capture's `status`, `content_type` and `parsed` verdict. Tests
run fully offline and never hit the network.

`fetch_fixtures` only keeps a capture that is 2xx, carries the expected
Content-Type, and whose own parser extracts at least one result (content
validation - no marker-string sniffing). The verdict is written to
`meta.json`; the `parse_fixture` tests consult it via
`util::fixture_parses(name)` and skip captures that were not genuine SERPs.
Re-capture with:

```bash
cargo run -p phrona --bin fetch_fixtures [engine...]
```

`dbg_parse` prints parsed results for a fixture:

```bash
cargo run -p phrona --bin dbg_parse -- bing
```
