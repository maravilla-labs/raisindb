// SPDX-License-Identifier: BSL-1.1

//! URL path extraction helpers.
//!
//! Functions for extracting repository IDs from various URL patterns
//! used across middleware layers.

/// Extract repository name from URL path.
///
/// Handles patterns like:
/// - /api/repository/{repo}/...
/// - /api/sql/{repo}
/// - /api/audit/{repo}/...
/// - /api/management/{repo}/...
pub(super) fn extract_repo_from_path(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    // Need at least 3 segments: ["api", "something", "{repo}", ...]
    if segments.len() >= 3 && segments[0] == "api" {
        return Some(segments[2].to_string());
    }

    None
}

/// Extract repository ID from various URL patterns.
///
/// Handles:
/// - `/auth/{repo}/*` (auth routes)
/// - `/api/repository/{repo}/*` (repository API)
/// - `/api/sql/{repo}/*` (SQL API)
/// - `/api/repos/{repo}/*` (package routes)
/// - `/api/functions/{repo}/*` (function routes)
/// - `/api/webhooks/{repo}/*` (webhook routes)
/// - `/api/triggers/{repo}/*` (trigger routes)
/// - `/api/search/{repo}` (hybrid search)
/// - `/api/packages/{repo}/*` (package commands)
pub(super) fn extract_repo_from_any_path(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    // Pattern: /auth/{repo}/...
    if segments.len() >= 2 && segments[0] == "auth" {
        let potential_repo = segments[1];
        if ![
            "providers",
            "refresh",
            "logout",
            "sessions",
            "me",
            "register",
            "login",
            "magic-link",
            "oidc",
            "cli",
        ]
        .contains(&potential_repo)
        {
            return Some(potential_repo.to_string());
        }
    }

    // Pattern: /api/{route_type}/{repo}/...
    if segments.len() >= 3 && segments[0] == "api" {
        let route_type = segments[1];
        match route_type {
            "repository" | "sql" | "repos" | "functions" | "webhooks" | "triggers" | "search"
            | "packages" | "flows" | "files" => {
                return Some(segments[2].to_string());
            }
            _ => {}
        }
    }

    None
}

/// Extract repo ID from auth path patterns like `/auth/{repo}/register`.
pub(super) fn extract_repo_from_auth_path(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if segments.len() >= 2 && segments[0] == "auth" {
        let potential_repo = segments[1];
        if ![
            "providers",
            "refresh",
            "logout",
            "sessions",
            "me",
            "register",
            "login",
            "magic-link",
            "oidc",
        ]
        .contains(&potential_repo)
        {
            return Some(potential_repo.to_string());
        }
    }

    None
}
