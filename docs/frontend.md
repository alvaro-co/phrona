# Web frontend

A static, dependency-free SPA in `frontend/` (index.html, style.css,
app.js), served by the API server at `/` (see
[docs/api.md](api.md)). Material 3 inspired: dynamic color tokens as CSS
variables, light/dark themes, rounded chips and cards, elevation, focus
rings. No build step, no framework, no npm.

## Pages / behavior

- Single search page. Search box with debounced live suggestions
  (180 ms) from `/v1/suggest` (per-source, merged, deduplicated).
- Category chips: web, images, news, videos, books - switch category
  re-queries the current query.
- Engine chips (loaded from `/v1/engines`): toggle engines on/off.
- Controls: region (text), language (text), time range (select), max
  results (select).
- Results: card list per type. Images render as a responsive grid with
  thumbnail, title and source; videos show thumbnail, duration badge,
  uploader, views, published; news shows date and source; books show
  author/publisher.
- Answer banner when an answer engine produced one.
- Error banner when all engines fail; partial results still render.
- Theme toggle (light/dark) persisted in `localStorage`.
- Accessibility: focus-visible styles, aria-labels, keyboard-enter submits.

## Files

| File | Role |
| --- | --- |
| `index.html` | shell + controls + result container |
| `style.css` | Material 3 tokens, layout, cards, chips, responsive grid |
| `app.js` | state, fetch wrapper, rendering, theming |

The server reads these files from disk per request (no embedding), so
editing them takes effect on reload without a rebuild.

## API contract used

- `GET /v1/engines?category=...`
- `GET /v1/suggest?q=...&region=...`
- `GET /v1/search?q=...&category=...&engines=...&max_results=...&region=...&language=...&time_range=...&safesearch=...`

All responses are the shapes documented in [docs/api.md](api.md).
