//! Admin UI - Embedded web interface for RaisinDB administration
//!
//! This module provides the embedded admin console built with React and TailwindCSS.
//! The static assets are embedded in the binary at compile time using rust-embed.

use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = ".admin-console-dist"]
struct AdminAssets;

/// Serves the admin UI static assets or falls back to index.html for SPA routing
pub async fn serve_admin_ui(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Remove /admin prefix if present
    let path = path.strip_prefix("admin").unwrap_or(path);
    let path = path.trim_start_matches('/');

    // Try to serve the requested file
    if let Some(content) = AdminAssets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.as_ref())
            .body(Body::from(content.data))
            .unwrap();
    }

    // For SPA routing, fallback to index.html
    if let Some(index) = AdminAssets::get("index.html") {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html")
            .body(Body::from(index.data))
            .unwrap();
    }

    // If index.html is also missing (shouldn't happen), return 404
    (StatusCode::NOT_FOUND, "Admin UI not found").into_response()
}
