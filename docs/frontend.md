# Web frontend

A single static, dependency-free page in `frontend/` (index.html,
style.css, app.js), served by the API server at `/` (see
[docs/api.md](api.md)). Material 3 inspired: dynamic color tokens as CSS
variables, light/dark themes, rounded chips and cards, focus rings. No
build step, no framework, no npm.

## The page

Two tabs, one page:

**Search** - full search playground, exposing the same parameters as the
library:

- Search box with debounced live suggestions (180 ms) from `/v1/suggest`
  (merged across all sources, deduplicated).
- Category chips: web, images, news, videos, books.
- Engine chips (loaded from `/v1/engines`): toggle engines on/off; the
  empty selection means "all engines of the category".
- Parameters: region, language, time range, safesearch, max results,
  page, filters (`site:...`, DDG image filters, ...), suggestions
  on/off.
- Results: cards per type - images render as a responsive grid, videos
  show thumbnail/duration/views/uploader, news shows date/source, books
  show author/publisher. Each card shows the engines that found it.
- Answer banner when an answer engine produced one.
- Collapsible engine report: per-engine status, result count and error
  (the same `EngineReport` the API returns).
- JSON view toggle: the raw `/v1/search` response.
- Pagination (prev/next).
- The full query is kept in the URL hash: results are shareable and
  survive reload (e.g. `/#q=rust&category=web&max_results=20`).

**Tools** - the same operations as the `ms` CLI, in the browser, each
against the live API with a JSON view toggle:

| Tool | Endpoint | Equals |
| --- | --- | --- |
| suggest | `/v1/suggest` | `ms suggest` |
| extract | `/v1/extract` | `ms extract` |
| ground | `/v1/grounding` | `ms ground` |
| engines | `/v1/engines` | `ms engines` |
| test | `/v1/test` | `ms test` |

## Files

| File | Role |
| --- | --- |
| `index.html` | shell + search controls + tools forms |
| `style.css` | theme tokens, layout, cards, chips, tables |
| `app.js` | state, search, rendering, tools (one file, sectioned) |

The server reads these files from disk per request (no embedding), so
editing them takes effect on reload without a rebuild.

## API contract used

- `GET /v1/engines`
- `GET /v1/suggest?q=...&region=...`
- `GET /v1/search?q=...&category=...&engines=...&max_results=...&page=...&region=...&language=...&time_range=...&safesearch=...&filters=...`
- `GET /v1/extract?url=...&max_chars=...&query=...`
- `GET /v1/grounding?query=...&max_results=...&category=...&time_range=...`
- `GET /v1/test?query=...&category=...&max_results=...`

All responses are the shapes documented in [docs/api.md](api.md).
