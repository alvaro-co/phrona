# phrona (Python)

Python bindings for the [phrona](https://crates.io/crates/phrona)
high-performance metasearch engine (Rust core via pyo3). Queries 26 search
engines across 5 categories in parallel and returns merged, deduplicated and
ranked results.

## Install

```bash
pip install phrona
```

Wheels are published for CPython 3.9-3.14 on linux (x86_64 / aarch64),
macOS (x86_64 / arm64) and Windows (x86_64), plus an sdist for other
platforms (needs a Rust toolchain).

## Quick start

```python
import phrona

phrona.search("rust programming", engines=["bing", "brave"])
phrona.suggest("rus")
```

## Features

- 26 engines, 5 categories (web, images, news, videos, books) + suggestions.
- Impersonated HTTP/2 with TLS fingerprint spoofing.
- Cross-engine merging: dedup, ranking, per-engine error reporting.
- Time ranges, safesearch, regions, per-engine filter strings, proxies.

See the [Python reference](https://github.com/alvaro-co/phrona/blob/main/docs/python.md)
for the full API.

## License

AGPL-3.0 — see the main repository for details.