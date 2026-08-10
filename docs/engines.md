# Engine reference

25 engines in 5 categories, all static stateless `Engine` instances
registered in `crates/metasearch/src/engine.rs` (order = priority).

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

- **bing** - HTML; `first=0` paging (max_page 67); news via the
  `infinitescrollajax` JSON endpoint with a parallel fetch of the current
  article headline; videos from `asyncv2` with the result markup inside a
  `<noscript>` block (must be unwrapped before HTML parsing - see
  `regex_strip_noscript` in `bing_videos.rs`).
- **brave** - JSON API blobs extracted from HTML `data-serpapi` scripts
  (js_to_json); suggestion + results endpoints combined; thumbnail URLs are
  base64-encoded (`/g:ce/...`), decoded with URL-safe and standard base64
  (`brave_b64_decode`); page 2 via `?q=query&spellcheck=false&s=...`.
- **duckduckgo** - HTML endpoint; needs the `vqd` token from
  `duckduckgo.com/?vqd=...` - cached in `EngineShared.vqd` per key
  (TTL 1 h); images/news/videos use the JSON `vqd`-signed endpoints; every
  engine request adds `a=-1`, `vqd`, `o=json` where relevant.
- **google / google_images** - `/search` with `udm=14` (plain HTML) and
  `tbm=isch`; **429-blocks from datacenter IPs** - the parser degrades to
  zero results gracefully and `is_block_page` marks the captcha page so
  fixtures are never polluted.
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

## Block-page detection

`is_block_page(text)` in `engines/util.rs` recognizes captcha / JS-required /
rate-limit / bot pages (enablejs, captcha-delivery, "403 - forbidden",
"anomaly", "there are no search results", "too many requests", "page
requires javascript", "SolveSimpleChallenge", "enable javascript"). It is
used by:

- `fetch_fixtures` - never overwrites a good fixture with a block page;
- the `parse_fixture` tests - skip fixtures that are block pages.

## Fixtures

`crates/metasearch/tests/fixtures/` holds 25 captured live pages (one per
engine, plus qwant/videos/suggest variants). Tests run fully offline and
never hit the network; `is_block_page` keeps the corpus clean. Re-capture
with:

```bash
cargo run -p metasearch --bin fetch_fixtures [engine...]
```

`dbg_parse` prints parsed results for a fixture:

```bash
cargo run -p metasearch --bin dbg_parse -- bing
```
