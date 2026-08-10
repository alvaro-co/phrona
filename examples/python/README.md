# Python examples

Two runnable scripts demonstrating the `metasearch` Python package.

## Prerequisites

The bindings require CPython 3.8-3.13 (3.14 crashes in pyo3 0.24).

```bash
uv build                              # from the repo root
uv venv --python 3.12 /tmp/msenv
uv pip install --python /tmp/msenv/bin/python dist/metasearch-0.1.0-*.whl
```

## Run

```bash
/tmp/msenv/bin/python examples/python/basic.py "rust programming"
/tmp/msenv/bin/python examples/python/client.py "rust ownership"
```

## What each script shows

| Script | Demonstrates |
| --- | --- |
| `basic.py` | `version()`, `engines(category)`, module-level `search()` with full options, the result/engine report dict shapes |
| `client.py` | the reusable `Client` class (profile + timeout), `suggest()` single and all sources, `extract()` with query-biased excerpts, per-category engine counts |

## Notes

- All functions return plain Python dicts/lists - JSON-serializable by
  construction, no wrapper objects.
- `search()` keyword arguments mirror the Rust `SearchOptions`: category,
  engines, page, max_results, safesearch, region, language, time_range,
  filters.
- `engines=None` means "all engines of the category".

For the full API see `docs/python.md`.
