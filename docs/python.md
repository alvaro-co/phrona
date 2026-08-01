# Python bindings reference

`metasearch` Python package built from `crates/metasearch-python` with pyo3
0.24. Python 3.8-3.13 (3.14 crashes in pyo3 0.24 - use a 3.13 or older
interpreter).

## Building the wheel

```bash
uv build                       # needs [build-system] setuptools-rust in pyproject.toml
uv pip install dist/metasearch-0.1.0-*.whl --python <python3.12 venv>
```

The wheel tags the platform (`cp312-cp312-linux_x86_64` etc.). On other
platforms use `cargo build --release -p metasearch-python` and copy the
cdylib as `metasearch.so` into a venv site-packages.

## API

```python
import metasearch

metasearch.version()                # "0.1.0"

metasearch.engines("web")           # ["bing","brave",...]
metasearch.engines("images")

metasearch.search("rust", engines=["bing", "brave"], max_results=10)
# {'query': 'rust', 'category': 'web', 'page': 1, 'total': 8,
#  'results': [{'type': 'web', 'title': ..., 'url': ..., 'description': ...,
#               'score': ..., 'position': ..., 'engines': [...]}],
#  'suggestions': [...], 'answer': None,
#  'engines': [{'name': 'bing', 'status': 'ok', 'results': 10}],
#  'elapsed_ms': 1200}

metasearch.suggest("rus", source="bing")       # ['rust', 'rustup', ...]

metasearch.extract("https://example.com", max_chars=5000, query="hello")
# {'title': ..., 'description': ..., 'text': ..., 'images': [...]}
```

### Client class

```python
client = metasearch.Client(profile="chrome", timeout=20)
client.search("rust", category="web", engines=None, page=1, max_results=30,
              safesearch="moderate", region=None, language=None,
              time_range=None, filters=None)
client.suggest("rus", source=None, region="us-en")   # all sources if source None
client.extract("https://example.com", max_chars=8000, query=None)
client.engines("news")
```

All result values are plain Python dicts/lists/str/int/float - no wrapper
objects, JSON-serializable by construction. `extract` uses manual
serialization of `ExtractedPage` (pyo3 class specialization).

`search` keyword arguments mirror `SearchOptions`; `engines=None` means all
engines of the category. `profile` accepts "chrome", "firefox", "edge",
"safari", "opera", "okhttp" (or a numeric profile). `timeout` is seconds.
