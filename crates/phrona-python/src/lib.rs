//! Python bindings for the phrona library.
//!
//! ```python
//! import phrona
//! phrona.search("rust programming", engines=["bing", "brave"])
//! phrona.suggest("rus")
//! phrona.extract("https://doc.rust-lang.org/book/")
//! ```

use std::sync::LazyLock;
use std::time::Duration;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use phrona_core::{Category, Profile, SearchClient, SearchOptions};

/// Dedicated multi-threaded runtime for all blocking calls. Network I/O runs
/// on this runtime with the Python GIL released, so Python threads and
/// asyncio loops are never blocked.
static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
});

fn parse_profile(s: &str) -> PyResult<Profile> {
    let name = s.trim().to_ascii_lowercase();
    let p = match name.as_str() {
        "chrome" | "chrome148" => Profile::Chrome,
        "chrome100" => Profile::Chrome100,
        "chrome120" => Profile::Chrome120,
        "chrome131" => Profile::Chrome131,
        "chrome140" => Profile::Chrome140,
        "chrome149" => Profile::Chrome149,
        "firefox" | "firefox148" => Profile::Firefox,
        "firefox139" => Profile::Firefox139,
        "edge" | "edge148" => Profile::Edge,
        "safari" | "safari26" => Profile::Safari,
        "opera" | "opera131" => Profile::Opera,
        "okhttp" => Profile::OkHttp,
        "random" => Profile::Random,
        _ => return Err(PyValueError::new_err(format!("unknown profile '{s}'"))),
    };
    Ok(p)
}

fn parse_category(s: &str) -> PyResult<Category> {
    s.parse::<Category>().map_err(|_| {
        PyValueError::new_err("category must be one of: web, images, news, videos, books")
    })
}

fn to_py(v: &impl serde::Serialize) -> PyResult<Py<PyAny>> {
    let j =
        serde_json::to_value(v).map_err(|e| PyValueError::new_err(format!("serialize: {e}")))?;
    Python::attach(|py| json_to_py(py, &j))
}

/// Convert a serde_json::Value into the matching Python object.
fn json_to_py(py: Python<'_>, v: &serde_json::Value) -> PyResult<Py<PyAny>> {
    let o: Py<PyAny> = match v {
        serde_json::Value::Null => py.None(),
        serde_json::Value::Bool(b) => (*b).into_pyobject(py)?.to_owned().into_any().unbind(),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_pyobject(py)?.into_any().unbind()
            } else {
                n.as_f64()
                    .unwrap_or(0.0)
                    .into_pyobject(py)?
                    .into_any()
                    .unbind()
            }
        }
        serde_json::Value::String(s) => s.into_pyobject(py)?.into_any().unbind(),
        serde_json::Value::Array(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(json_to_py(py, item)?)?;
            }
            list.into_any().unbind()
        }
        serde_json::Value::Object(map) => {
            let d = PyDict::new(py);
            for (k, val) in map {
                d.set_item(k, json_to_py(py, val)?)?;
            }
            d.into_any().unbind()
        }
    };
    Ok(o)
}

/// A metasearch client. Safe to share across threads.
#[pyclass]
struct Client {
    client: SearchClient,
}

#[pymethods]
impl Client {
    #[new]
    #[pyo3(signature = (profile="chrome", timeout=15.0))]
    fn new(profile: &str, timeout: f64) -> PyResult<Self> {
        let client = SearchClient::with_options(
            parse_profile(profile)?,
            Some(Duration::from_secs_f64(timeout.max(1.0))),
            None,
            phrona_core::TargetPolicy::default(),
        )
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self { client })
    }

    /// Search all engines for a query. Returns a dict with results, engines
    /// report, suggestions and elapsed time.
    #[pyo3(signature = (query, category="web", engines=None, page=1, max_results=20,
                        safesearch="moderate", region=None, language=None,
                        time_range=None, filters=None))]
    #[allow(clippy::too_many_arguments)]
    fn search(
        &self,
        py: Python<'_>,
        query: &str,
        category: &str,
        engines: Option<Vec<String>>,
        page: u32,
        max_results: usize,
        safesearch: &str,
        region: Option<String>,
        language: Option<String>,
        time_range: Option<String>,
        filters: Option<String>,
    ) -> PyResult<Py<PyAny>> {
        let mut opts = SearchOptions::new(query);
        opts.category = parse_category(category)?;
        opts.engines = engines.unwrap_or_default();
        opts.page = page.max(1);
        opts.max_results = max_results.clamp(1, 200);
        opts.safesearch = safesearch.parse::<phrona_core::SafeSearch>().map_err(|_| {
            PyValueError::new_err("safesearch must be one of: off, moderate, strict")
        })?;
        opts.region = region;
        opts.language = language;
        opts.time_range = time_range
            .map(|t| {
                t.parse::<phrona_core::TimeRange>().map_err(|_| {
                    PyValueError::new_err("time_range must be one of: day, week, month, year")
                })
            })
            .transpose()?;
        opts.filters = filters;
        let resp = py
            .detach(|| {
                RUNTIME
                    .block_on(self.client.search(opts))
                    .map_err(|e| e.to_string())
            })
            .map_err(PyValueError::new_err)?;
        to_py(&resp)
    }

    /// Query suggestions. source: duckduckgo, google, bing, brave, startpage,
    /// qwant or wikipedia. None returns all sources.
    #[pyo3(signature = (query, source=None, region="us-en"))]
    fn suggest(
        &self,
        py: Python<'_>,
        query: &str,
        source: Option<String>,
        region: &str,
    ) -> PyResult<Py<PyAny>> {
        let http = self.client.http();
        let value = py
            .detach(|| -> Result<serde_json::Value, String> {
                match source {
                    Some(name) => {
                        let s = phrona_core::SuggestSource::from_name(&name).ok_or_else(|| {
                            format!(
                                "unknown source '{name}', expected one of: {}",
                                phrona_core::SuggestSource::ALL
                                    .iter()
                                    .map(|s| s.name())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        })?;
                        let list = RUNTIME
                            .block_on(phrona_core::suggest(http, s, query, region))
                            .map_err(|e| e.to_string())?;
                        Ok(serde_json::json!({"query": query, "source": name, "suggestions": list}))
                    }
                    None => {
                        let all = RUNTIME.block_on(phrona_core::suggest_all(http, query, region));
                        let map: serde_json::Map<String, serde_json::Value> = all
                            .into_iter()
                            .map(|(s, list)| (s.name().to_string(), serde_json::json!(list)))
                            .collect();
                        Ok(serde_json::json!({"query": query, "suggestions": map}))
                    }
                }
            })
            .map_err(PyValueError::new_err)?;
        to_py(&value)
    }

    /// Fetch a URL and extract its readable main content (AI grounding).
    #[pyo3(signature = (url, max_chars=8000, query=None))]
    fn extract(
        &self,
        py: Python<'_>,
        url: &str,
        max_chars: usize,
        query: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let page = py
            .detach(|| {
                RUNTIME
                    .block_on(phrona_core::extract(
                        self.client.http(),
                        url,
                        max_chars,
                        query,
                    ))
                    .map_err(|e| e.to_string())
            })
            .map_err(PyValueError::new_err)?;
        to_py(&page)
    }

    /// List available engines per category.
    #[pyo3(signature = (category=None))]
    fn engines(&self, py: Python<'_>, category: Option<String>) -> PyResult<Py<PyAny>> {
        let out = py.detach(|| {
            RUNTIME.block_on(async {
                let mut out = serde_json::Map::new();
                let cats: Vec<Category> = match category {
                    Some(c) => vec![parse_category(&c)?],
                    None => Category::ALL.to_vec(),
                };
                for cat in cats {
                    let names: Vec<String> = phrona_core::available_engines(cat)
                        .iter()
                        .map(|e| e.name.clone())
                        .collect();
                    out.insert(cat.as_str().to_string(), serde_json::json!(names));
                }
                Ok::<_, PyErr>(serde_json::Value::Object(out))
            })
        });
        to_py(&out?)
    }
}

fn build_client(profile: &str, timeout: f64) -> PyResult<Client> {
    Client::new(profile, timeout)
}

/// One-shot search with a default client. Same parameters as Client.search.
#[pyfunction]
#[pyo3(signature = (query, category="web", engines=None, page=1, max_results=20,
                    safesearch="moderate", region=None, language=None,
                    time_range=None, filters=None, profile="chrome", timeout=15.0))]
#[allow(clippy::too_many_arguments)]
fn search(
    py: Python<'_>,
    query: &str,
    category: &str,
    engines: Option<Vec<String>>,
    page: u32,
    max_results: usize,
    safesearch: &str,
    region: Option<String>,
    language: Option<String>,
    time_range: Option<String>,
    filters: Option<String>,
    profile: &str,
    timeout: f64,
) -> PyResult<Py<PyAny>> {
    let client = build_client(profile, timeout)?;
    client.search(
        py,
        query,
        category,
        engines,
        page,
        max_results,
        safesearch,
        region,
        language,
        time_range,
        filters,
    )
}

/// One-shot suggestions with a default client.
#[pyfunction]
#[pyo3(signature = (query, source=None, region="us-en"))]
fn suggest(
    py: Python<'_>,
    query: &str,
    source: Option<String>,
    region: &str,
) -> PyResult<Py<PyAny>> {
    build_client("chrome", 15.0)?.suggest(py, query, source, region)
}

/// One-shot page extraction with a default client.
#[pyfunction]
#[pyo3(signature = (url, max_chars=8000, query=None))]
fn extract(
    py: Python<'_>,
    url: &str,
    max_chars: usize,
    query: Option<&str>,
) -> PyResult<Py<PyAny>> {
    build_client("chrome", 15.0)?.extract(py, url, max_chars, query)
}

/// One-shot engines listing with a default client.
#[pyfunction]
#[pyo3(signature = (category=None))]
fn engines(py: Python<'_>, category: Option<String>) -> PyResult<Py<PyAny>> {
    build_client("chrome", 15.0)?.engines(py, category)
}

#[pyfunction]
fn version() -> String {
    phrona_core::version().to_string()
}

#[pymodule]
fn phrona(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Client>()?;
    m.add_function(wrap_pyfunction!(search, m)?)?;
    m.add_function(wrap_pyfunction!(suggest, m)?)?;
    m.add_function(wrap_pyfunction!(extract, m)?)?;
    m.add_function(wrap_pyfunction!(engines, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    Ok(())
}
