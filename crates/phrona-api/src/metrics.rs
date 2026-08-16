//! Prometheus metrics for the REST API.
//!
//! Exposes `GET /metrics` in the Prometheus text exposition format with
//! strictly bounded cardinality: labels are drawn only from the fixed set of
//! endpoints and engine names — never from search queries or target URLs.
//!
//! Metric families:
//! - `phrona_http_requests_total{endpoint,status}` (counter)
//! - `phrona_http_request_duration_seconds{endpoint}` (histogram)
//! - `phrona_engine_requests_total{engine,status}` (counter)
//! - `phrona_engine_errors_total{engine,scope,kind}` (counter)
//! - `phrona_engine_duration_seconds{engine}` (histogram)

use std::sync::OnceLock;
use std::time::Instant;

use axum::extract::Request;
use axum::http::header::CONTENT_TYPE;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder,
};

use phrona::EngineObserver;

const HTTP_BUCKETS: [f64; 10] = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0];
const ENGINE_BUCKETS: [f64; 11] = [0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0];

/// Registered metric families and the registry they are gathered from.
pub struct Metrics {
    registry: Registry,
    http_requests: IntCounterVec,
    http_duration: HistogramVec,
    engine_requests: IntCounterVec,
    engine_errors: IntCounterVec,
    engine_duration: HistogramVec,
}

impl Metrics {
    fn new() -> Self {
        let registry = Registry::new();
        let http_requests = IntCounterVec::new(
            Opts::new(
                "phrona_http_requests_total",
                "Total HTTP requests handled by the API, by endpoint and response status",
            ),
            &["endpoint", "status"],
        )
        .expect("static metric");
        let http_duration = HistogramVec::new(
            HistogramOpts::new(
                "phrona_http_request_duration_seconds",
                "HTTP request handling duration, by endpoint",
            )
            .buckets(HTTP_BUCKETS.to_vec()),
            &["endpoint"],
        )
        .expect("static metric");
        let engine_requests = IntCounterVec::new(
            Opts::new(
                "phrona_engine_requests_total",
                "Engine requests by outcome (ok|empty|error) and engine",
            ),
            &["engine", "status"],
        )
        .expect("static metric");
        let engine_errors = IntCounterVec::new(
            Opts::new(
                "phrona_engine_errors_total",
                "Engine failures by error scope and kind",
            ),
            &["engine", "scope", "kind"],
        )
        .expect("static metric");
        let engine_duration = HistogramVec::new(
            HistogramOpts::new(
                "phrona_engine_duration_seconds",
                "Engine request duration, by engine",
            )
            .buckets(ENGINE_BUCKETS.to_vec()),
            &["engine"],
        )
        .expect("static metric");
        for m in [
            Box::new(http_requests.clone()) as Box<dyn prometheus::core::Collector>,
            Box::new(http_duration.clone()),
            Box::new(engine_requests.clone()),
            Box::new(engine_errors.clone()),
            Box::new(engine_duration.clone()),
        ] {
            registry.register(m).expect("unique metric families");
        }
        Self {
            registry,
            http_requests,
            http_duration,
            engine_requests,
            engine_errors,
            engine_duration,
        }
    }
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

/// The process-wide metrics registry (lazily initialized on first use).
fn global() -> &'static Metrics {
    METRICS.get_or_init(Metrics::new)
}

/// Counts every HTTP request after the inner service responded: endpoint
/// (request path) and status code are the only labels.
pub async fn http_layer(req: Request, next: Next) -> Response {
    let started = Instant::now();
    let endpoint = req.uri().path().to_string();
    let resp = next.run(req).await;
    let m = global();
    let status = resp.status().as_str().to_string();
    m.http_requests
        .with_label_values(&[endpoint.as_str(), status.as_str()])
        .inc();
    m.http_duration
        .with_label_values(&[&endpoint])
        .observe(started.elapsed().as_secs_f64());
    resp
}

/// Core-crate observer forwarding engine outcomes into Prometheus.
///
/// Attach it to the search client via
/// `client.with_observer(Arc::new(metrics::EngineMetricsObserver))`.
#[derive(Default)]
pub struct EngineMetricsObserver;

impl EngineObserver for EngineMetricsObserver {
    fn on_engine_done(
        &self,
        engine: &str,
        status: &str,
        scope: Option<&str>,
        kind: Option<&str>,
        elapsed: std::time::Duration,
    ) {
        let m = global();
        m.engine_requests.with_label_values(&[engine, status]).inc();
        if let (Some(scope), Some(kind)) = (scope, kind) {
            m.engine_errors
                .with_label_values(&[engine, scope, kind])
                .inc();
        }
        m.engine_duration
            .with_label_values(&[engine])
            .observe(elapsed.as_secs_f64());
    }
}

/// `GET /metrics` — Prometheus text exposition of every registered family.
/// Deliberately unauthenticated so scrapers do not need to carry API keys.
pub async fn metrics_route() -> Response {
    let m = global();
    let mut buf = Vec::new();
    let encoder = TextEncoder::new();
    let families = m.registry.gather();
    // Gather is infallible here: every family was registered with a valid
    // name and label set. An encoding failure still yields an empty body
    // rather than a panic.
    let _ = encoder.encode(&families, &mut buf);
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        buf,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The registry is process-global, so all assertions share one test to
    // avoid parallel-test interference.
    #[tokio::test]
    async fn metrics_record_and_expose_all_families() {
        let m = global();
        m.http_requests.with_label_values(&["/health", "200"]).inc();
        m.http_duration
            .with_label_values(&["/health"])
            .observe(0.01);
        let observer = EngineMetricsObserver;
        observer.on_engine_done(
            "bing",
            "ok",
            None,
            None,
            std::time::Duration::from_millis(250),
        );
        observer.on_engine_done(
            "bing",
            "empty",
            None,
            None,
            std::time::Duration::from_millis(10),
        );
        observer.on_engine_done(
            "google",
            "error",
            Some("Provider"),
            Some("Timeout"),
            std::time::Duration::from_secs(3),
        );

        assert_eq!(
            m.engine_requests.with_label_values(&["bing", "ok"]).get(),
            1
        );
        assert_eq!(
            m.engine_requests
                .with_label_values(&["bing", "empty"])
                .get(),
            1
        );
        assert_eq!(
            m.engine_errors
                .with_label_values(&["google", "Provider", "Timeout"])
                .get(),
            1
        );

        let text = scrape().await;
        for family in [
            "phrona_http_requests_total",
            "phrona_http_request_duration_seconds",
            "phrona_engine_requests_total",
            "phrona_engine_errors_total",
            "phrona_engine_duration_seconds",
        ] {
            assert!(
                text.contains(&format!("# TYPE {family}")),
                "missing family {family} in:\n{text}"
            );
        }
        assert!(text.contains("phrona_http_requests_total{endpoint=\"/health\",status=\"200\"} 1"));
        assert!(text.contains("phrona_engine_requests_total{engine=\"bing\",status=\"ok\"} 1"));
        // labels are emitted alphabetically in the text format
        assert!(text.contains(
            "phrona_engine_errors_total{engine=\"google\",kind=\"Timeout\",scope=\"Provider\"} 1"
        ));

        // a second failure increments the counter
        observer.on_engine_done(
            "google",
            "error",
            Some("Provider"),
            Some("Timeout"),
            std::time::Duration::from_secs(2),
        );
        assert_eq!(
            m.engine_errors
                .with_label_values(&["google", "Provider", "Timeout"])
                .get(),
            2
        );
    }

    async fn scrape() -> String {
        let resp = metrics_route().await;
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }
}
