# Python examples

Two runnable scripts demonstrating the `phrona` Python package.

## Prerequisites

The bindings require CPython 3.9-3.14.

```bash
uv build                              # from the repo root
uv venv --python 3.12 /tmp/msenv
uv pip install --python /tmp/msenv/bin/python dist/phrona-0.2.0-*.whl
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
