//! Serves the static web frontend.
//!
//! `index.html`, `style.css` and `app.js` live in the repository `frontend/`
//! directory and are served from disk so editing does not require rebuilds.

use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::Response;

const FRONTEND_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../frontend");

/// Resolve the frontend directory. `$PHRONA_FRONTEND_DIR` overrides the
/// compile-time default so packaged binaries can serve the assets from a
/// stable path.
pub fn frontend_dir() -> std::path::PathBuf {
    std::env::var_os("PHRONA_FRONTEND_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(FRONTEND_DIR))
}

fn serve(name: &str, mime: &'static str) -> Response {
    match std::fs::read(frontend_dir().join(name)) {
        Ok(body) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime)
            .body(Body::from(body))
            .unwrap_or_else(|_| Response::new(Body::empty())),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("not found"))
            .unwrap(),
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
