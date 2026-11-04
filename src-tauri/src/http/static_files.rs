//! Serves the frontend bundle so a browser can load the app.
//!
//! Release builds embed `dist/` in the binary (`pnpm run build` runs first
//! via `beforeBuildCommand`); debug builds read it from disk, so run
//! `pnpm run build` to refresh it. Unknown paths fall back to `index.html`.

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../dist"]
#[allow_missing = true]
struct Assets;

pub async fn handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let asset = if path.is_empty() {
        None
    } else {
        Assets::get(path).map(|file| (path.to_string(), file))
    };
    let Some((path, file)) =
        asset.or_else(|| Assets::get("index.html").map(|f| ("index.html".into(), f)))
    else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "The frontend isn't built. Run `pnpm run build`, then reload.",
        )
            .into_response();
    };

    // Hashed assets can be cached; index.html must not be
    let cache = if path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    Response::builder()
        .header(header::CONTENT_TYPE, file.metadata.mimetype())
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(file.data.into_owned()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
