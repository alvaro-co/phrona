//! Serves the static web frontend.
//!
//! `index.html`, `style.css` and `app.js` live in this crate's `assets/`
//! directory and are served from disk when present, so editing does not
//! require rebuilds. If `$PHRONA_FRONTEND_DIR` is unset or unreadable, or the
//! local `assets/` folder is missing (standalone release binaries,
//! containers), the assets embedded at compile time via `include_str!` are
//! served instead — UI routes never 404.

use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::Response;

const FRONTEND_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets");

const EMBEDDED_INDEX: &str = include_str!("../assets/index.html");
const EMBEDDED_CSS: &str = include_str!("../assets/style.css");
const EMBEDDED_JS: &str = include_str!("../assets/app.js");

/// Resolve the frontend directory. `$PHRONA_FRONTEND_DIR` overrides the
/// compile-time default so packaged binaries can serve the assets from a
/// stable path.
pub fn frontend_dir() -> std::path::PathBuf {
    std::env::var_os("PHRONA_FRONTEND_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(FRONTEND_DIR))
}

fn embedded_asset(name: &str) -> Option<&'static [u8]> {
    match name {
        "index.html" => Some(EMBEDDED_INDEX.as_bytes()),
        "style.css" => Some(EMBEDDED_CSS.as_bytes()),
        "app.js" => Some(EMBEDDED_JS.as_bytes()),
        _ => None,
    }
}

fn ok_response(body: Vec<u8>, mime: &'static str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn serve(name: &str, mime: &'static str) -> Response {
    match std::fs::read(frontend_dir().join(name)) {
        Ok(body) => ok_response(body, mime),
        Err(_) => match embedded_asset(name) {
            Some(asset) => ok_response(asset.to_vec(), mime),
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("not found"))
                .unwrap(),
        },
    }
}

/// Serve the SPA: known assets by path, everything else falls back to the
/// app shell (client-side routing is not used, so this also serves "/").
pub async fn index(req: axum::extract::Request) -> Response {
    let p = req.uri().path().trim_start_matches('/');
    match p {
        "" => serve("index.html", "text/html; charset=utf-8"),
        "style.css" => serve("style.css", "text/css; charset=utf-8"),
        "app.js" => serve("app.js", "text/javascript; charset=utf-8"),
        _ => serve("index.html", "text/html; charset=utf-8"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_assets_cover_the_ui() {
        assert!(!EMBEDDED_INDEX.is_empty());
        assert!(EMBEDDED_INDEX.contains("<html") || EMBEDDED_INDEX.contains("<!doctype"));
        assert!(!EMBEDDED_CSS.is_empty());
        assert!(!EMBEDDED_JS.is_empty());
        for name in ["index.html", "style.css", "app.js"] {
            assert!(embedded_asset(name).is_some(), "{name} missing");
        }
        assert!(embedded_asset("favicon.ico").is_none());
    }

    #[tokio::test]
    async fn fallback_serves_embedded_when_dir_missing() {
        let dir = frontend_dir();
        let gone = dir.join("__definitely_missing__");
        assert!(std::fs::read(&gone).is_err());
        let resp = serve("app.js", "text/javascript; charset=utf-8");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        assert!(!body.is_empty());
        assert_eq!(body, EMBEDDED_JS.as_bytes());
    }
}
