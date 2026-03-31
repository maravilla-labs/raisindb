// SPDX-License-Identifier: BSL-1.1

//! Path parsing middleware for repository API routes.
//!
//! Handles sophisticated path parsing including repository/branch/workspace
//! extraction, file extension detection, version markers, command markers,
//! and property path extraction via `@` notation.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use percent_encoding::percent_decode_str;

use super::types::RaisinContext;

/// Middleware that parses repository paths and extracts metadata.
///
/// This middleware handles sophisticated path parsing for the repository API:
/// - Extracts repository, branch, and workspace names from path
/// - Detects file extensions (.yaml, .yml, .png, etc.)
/// - Parses version markers (raisin:version/{id})
/// - Parses command markers (raisin:cmd/{command})
/// - Extracts property paths using @ notation
/// - URL decodes and normalizes paths
///
/// Expected URL format: /api/repository/{repo}/{branch}/{workspace}/path
/// The parsed context is stored in request extensions as `RaisinContext`.
pub async fn raisin_parsing_middleware(
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let archetype = req
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|val| val.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Extract and decode URI path
    let uri_path = percent_decode_str(req.uri().path())
        .decode_utf8()
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_string();

    // Determine which API prefix we're handling
    let prefix = if uri_path.starts_with("/api/repository/") {
        "/api/repository/"
    } else if uri_path.starts_with("/api/preview/") {
        "/api/preview/nodes/"
    } else {
        // Not our route, continue without parsing
        return Ok(next.run(req).await);
    };

    // Remove prefix to get workspace and path
    let without_prefix = &uri_path[prefix.len()..];
    let mut cleaned_path = without_prefix.to_string();
    let mut property_path = None;

    // Check for '@' in the path to extract the property path
    if let Some(at_index) = cleaned_path.rfind('@') {
        let after_at = &cleaned_path[(at_index + 1)..];
        let prop_end = after_at.find('/').unwrap_or(after_at.len());
        let prop_name = &after_at[..prop_end];
        let remainder = &after_at[prop_end..];

        property_path = Some(prop_name.to_string());
        cleaned_path = format!("{}{}", &cleaned_path[..at_index], remainder);
    }

    let segments: Vec<&str> = cleaned_path.split('/').collect();

    if segments.len() < 3 {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Parse path segments based on format:
    // OLD format: repo/branch/workspace/path...
    // NEW format: repo/branch/head/workspace/path... OR repo/branch/rev/{n}/workspace/path...
    let (repo_name, branch_name, workspace_name, path_start_index) = {
        let repo = segments[0].to_string();
        let branch = segments[1].to_string();

        if segments.len() >= 4 && (segments[2] == "head" || segments[2] == "rev") {
            if segments[2] == "rev" {
                if segments.len() < 5 {
                    return Err(StatusCode::BAD_REQUEST);
                }
                let workspace = segments[4].to_string();
                (repo, branch, workspace, 5)
            } else {
                let workspace = segments[3].to_string();
                (repo, branch, workspace, 4)
            }
        } else {
            let workspace = segments[2].to_string();
            (repo, branch, workspace, 3)
        }
    };

    let mut is_version = false;
    let mut version_id: Option<i32> = None;
    let mut is_command = false;
    let mut command_name: Option<String> = None;
    let mut cleaned_segments = Vec::new();

    // Process each segment after repo/branch/workspace
    let mut i = path_start_index;
    while i < segments.len() {
        let segment = segments[i];

        if segment == "raisin:version" {
            is_version = true;
            if let Some(id_segment) = segments.get(i + 1) {
                version_id = id_segment.parse::<i32>().ok();
                #[allow(unused_assignments)]
                {
                    i += 1;
                }
            }
            break;
        } else if segment == "raisin:cmd" {
            is_command = true;
            if let Some(cmd_segment) = segments.get(i + 1) {
                command_name = Some(cmd_segment.to_string());
                #[allow(unused_assignments)]
                {
                    i += 1;
                }
            }
            break;
        } else {
            cleaned_segments.push(segment);
        }

        i += 1;
    }

    // Construct the cleaned path with leading slash
    let cleaned_path = if cleaned_segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", cleaned_segments.join("/"))
    };
    let original_path = cleaned_path.clone();

    // Extract file extension if present (but preserve full path)
    let file_extension = if let Some(index) = cleaned_path.rfind('.') {
        let last_segment_start = cleaned_path.rfind('/').map(|i| i + 1).unwrap_or(0);
        if index > last_segment_start {
            Some(cleaned_path[(index + 1)..].to_string())
        } else {
            None
        }
    } else {
        None
    };

    let context = RaisinContext {
        repo_name,
        branch_name,
        workspace_name,
        cleaned_path,
        original_path,
        file_extension,
        is_version,
        version_id,
        is_command,
        command_name,
        property_path,
        archetype,
    };

    req.extensions_mut().insert(context);

    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{middleware, routing::get, Extension, Router};
    use tower::ServiceExt;

    async fn test_handler(Extension(ctx): Extension<RaisinContext>) -> String {
        format!(
            "repo={}, branch={}, ws={}, path={}, ext={:?}, ver={}, cmd={}",
            ctx.repo_name,
            ctx.branch_name,
            ctx.workspace_name,
            ctx.cleaned_path,
            ctx.file_extension,
            ctx.is_version,
            ctx.is_command
        )
    }

    #[tokio::test]
    async fn parses_basic_path() {
        let app = Router::new()
            .route("/api/repository/{*path}", get(test_handler))
            .layer(middleware::from_fn(raisin_parsing_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repository/myrepo/mybranch/myworkspace/some/node/path")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn parses_with_extension() {
        let app = Router::new()
            .route("/api/repository/{*path}", get(test_handler))
            .layer(middleware::from_fn(raisin_parsing_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repository/myrepo/mybranch/myworkspace/node.yaml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn parses_version_marker() {
        let app = Router::new()
            .route("/api/repository/{*path}", get(test_handler))
            .layer(middleware::from_fn(raisin_parsing_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repository/myrepo/mybranch/myworkspace/node/raisin:version/5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn parses_command_marker() {
        let app = Router::new()
            .route("/api/repository/{*path}", get(test_handler))
            .layer(middleware::from_fn(raisin_parsing_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repository/myrepo/mybranch/myworkspace/node/raisin:cmd/publish")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn parses_property_path() {
        let app = Router::new()
            .route("/api/repository/{*path}", get(test_handler))
            .layer(middleware::from_fn(raisin_parsing_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repository/myrepo/mybranch/myworkspace/node@properties.file")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
