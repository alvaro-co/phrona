# MetaSearchRS

High-performance metasearch engine library written in Rust, with Python
bindings (pyo3), a REST API, an MCP server for AI agents, and a web frontend.

Requires Python >= 3.9 (bindings compiled for 3.12 by default).

```python
import metasearch
metasearch.search("rust programming", engines=["bing", "brave"])
metasearch.suggest("rus")
metasearch.extract("https://doc.rust-lang.org/book/")
```

Documentation: `docs/`. Upstream references: `docs/upstream/`.
