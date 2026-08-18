// SPDX-License-Identifier: BSL-1.1

//! URL path extraction helpers.
//!
//! Functions for extracting repository IDs from various URL patterns
//! used across middleware layers.

/// Segments that can follow `/auth/` without naming a repository.
///
/// Every one of these is a real route registered in `routes/auth.rs` (see the
/// `/auth/providers`, `/auth/login`, `/auth/me`, `/auth/cli`,
/// `/auth/change-password`, ... block there). Without the exclusion,
/// `/auth/login` would be read as repo `"login"`, which is worse than the
/// "default" fallback it replaces: permissions would resolve against a repo
/// that cannot exist instead of one that at least might.
///
/// Note the two sibling extractors below carry their own, slightly shorter,
/// inline copies of this list. They feed CORS origin resolution rather than
/// permission resolution, and quietly widening them here would change which
/// origins are allowed on `/auth/cli/*` and `/auth/change-password`. That is a
/// separate concern from this fix, so they are deliberately left alone.
const AUTH_RESERVED_SEGMENTS: &[&str] = &[
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
    "change-password",
];

/// Extract repository name from URL path.
///
/// Handles patterns like:
/// - /api/repository/{repo}/...
/// - /api/sql/{repo}
/// - /api/audit/{repo}/...
/// - /api/management/{repo}/...
/// - /mcp/{repo}/{branch}/{slug}
/// - /auth/{repo}/... (but not the repo-less /auth/login, /auth/me, ...)
pub(super) fn extract_repo_from_path(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    // Need at least 3 segments: ["api", "something", "{repo}", ...]
    if segments.len() >= 3 && segments[0] == "api" {
        return Some(segments[2].to_string());
    }

    // MCP transport: ["mcp", "{repo}", "{branch}", "{slug}"]. The repo is the
    // segment immediately after the `mcp` root so auth resolves permissions
    // against the correct repository rather than falling back to "default".
    if segments.len() >= 2 && segments[0] == "mcp" {
        return Some(segments[1].to_string());
    }

    // Static-content endpoint: ["resources", "{repo}", "{branch}", "{ws}", ...].
    // Without this, an anonymous request to a public static subtree resolves
    // the anonymous role against the "default" repo and is denied even though
    // the target repo grants it — uri-list MCP widgets can never load.
    if segments.len() >= 2 && segments[0] == "resources" {
        return Some(segments[1].to_string());
    }

    // Repo-scoped auth routes: ["auth", "{repo}", ...] — /auth/{repo}/me,
    // /auth/{repo}/login, /auth/{repo}/refresh, ... (routes/auth.rs:111-152).
    //
    // This arm is the fix for a production incident: `resolve_user_auth_context`
    // (middleware/auth.rs) resolves permissions against whatever repo this
    // function reports, so `GET /auth/studio/me` fell through to `"default"`,
    // resolved a user who has no grants in that repo, and returned a body with
    // `"roles": []` for a user whose Studio roles were in fact intact. The two
    // other callers in middleware/auth.rs — the anonymous-context installer and
    // the admin-impersonation resolver — had the same blind spot on these paths
    // and are fixed by the same arm.
    //
    // It sits last on purpose: it is the only arm that can misfire on a
    // repo-less path, so the unambiguous prefixes get first refusal.
    if segments.len() >= 2
        && segments[0] == "auth"
        && !AUTH_RESERVED_SEGMENTS.contains(&segments[1])
    {
        return Some(segments[1].to_string());
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

    // Pattern: /mcp/{repo}/{branch}/{slug}
    if segments.len() >= 2 && segments[0] == "mcp" {
        return Some(segments[1].to_string());
    }

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
            | "packages" | "flows" | "files" | "conversations" | "inbox" | "workspaces" => {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression this file's `auth` arm exists for: a repo-scoped auth
    /// route must report its repo, or `resolve_user_auth_context` resolves
    /// permissions against "default" and reports an empty `roles` list.
    #[test]
    fn auth_repo_scoped_paths_yield_the_repo() {
        assert_eq!(
            extract_repo_from_path("/auth/studio/me"),
            Some("studio".to_string())
        );
        assert_eq!(
            extract_repo_from_path("/auth/studio/login"),
            Some("studio".to_string())
        );
        assert_eq!(
            extract_repo_from_path("/auth/studio/magic-link/verify"),
            Some("studio".to_string())
        );
    }

    /// The repo-less auth routes must NOT be read as repo "me"/"login"/...
    #[test]
    fn auth_global_routes_have_no_repo() {
        for path in [
            "/auth/me",
            "/auth/login",
            "/auth/register",
            "/auth/refresh",
            "/auth/logout",
            "/auth/providers",
            "/auth/sessions",
            "/auth/sessions/abc123",
            "/auth/magic-link",
            "/auth/oidc/google/callback",
            "/auth/cli/login",
            "/auth/change-password",
        ] {
            assert_eq!(extract_repo_from_path(path), None, "path: {path}");
        }
    }

    /// A bare `/auth` has no second segment to mistake for a repo.
    #[test]
    fn bare_auth_has_no_repo() {
        assert_eq!(extract_repo_from_path("/auth"), None);
        assert_eq!(extract_repo_from_path("/auth/"), None);
    }

    /// The pre-existing arms must keep behaving exactly as before: the auth arm
    /// was appended last so it can never shadow them.
    #[test]
    fn existing_prefixes_are_unchanged() {
        assert_eq!(
            extract_repo_from_path("/api/repository/studio/nodes"),
            Some("studio".to_string())
        );
        assert_eq!(
            extract_repo_from_path("/api/sql/studio"),
            Some("studio".to_string())
        );
        assert_eq!(
            extract_repo_from_path("/mcp/studio/main/my-slug"),
            Some("studio".to_string())
        );
        assert_eq!(
            extract_repo_from_path("/resources/studio/main/ws/logo.png"),
            Some("studio".to_string())
        );
        // Too short to carry a repo, and not an auth path.
        assert_eq!(extract_repo_from_path("/api/health"), None);
        assert_eq!(extract_repo_from_path("/"), None);
    }

    /// `extract_repo_from_any_path` already handled `/auth/{repo}/*`; pinning it
    /// here documents that the two functions now agree on the auth cases that
    /// matter, even though their reserved-segment lists still differ (`cli` and
    /// `change-password` — see `AUTH_RESERVED_SEGMENTS`).
    #[test]
    fn any_path_agrees_on_repo_scoped_auth() {
        assert_eq!(
            extract_repo_from_any_path("/auth/studio/me"),
            extract_repo_from_path("/auth/studio/me")
        );
        assert_eq!(extract_repo_from_any_path("/auth/login"), None);
        assert_eq!(extract_repo_from_any_path("/auth/me"), None);
    }
}
