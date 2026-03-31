// SPDX-License-Identifier: BSL-1.1

//! Request context helpers for HTTP handlers.
//!
//! This module provides utilities to reduce boilerplate in handlers by consolidating
//! common context extraction patterns into reusable components.
//!
//! # Example
//!
//! Before:
//! ```ignore
//! pub async fn my_handler(
//!     State(state): State<AppState>,
//!     Path((repo, branch, ws)): Path<(String, String, String)>,
//!     auth: Option<Extension<AuthContext>>,
//! ) -> Result<Json<...>, ApiError> {
//!     let tenant_id = "default"; // TODO: Extract from middleware/auth
//!     let auth_context = auth.map(|Extension(ctx)| ctx);
//!     let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);
//!     // ...
//! }
//! ```
//!
//! After:
//! ```ignore
//! pub async fn my_handler(
//!     State(state): State<AppState>,
//!     ctx: RequestContext,
//! ) -> Result<Json<...>, ApiError> {
//!     let nodes_svc = ctx.node_service(&state);
//!     // ...
//! }
//! ```

use axum::{
    extract::{FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
use raisin_core::NodeService;
use raisin_models::auth::AuthContext;

use crate::middleware::TenantInfo;
use crate::state::AppState;

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;
#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;

#[cfg(feature = "storage-rocksdb")]
type Store = RocksDBStorage;
#[cfg(not(feature = "storage-rocksdb"))]
type Store = InMemoryStorage;

/// Request context containing tenant, repository, branch, workspace, and auth information.
///
/// This struct consolidates the common context extraction pattern used across handlers.
/// It extracts:
/// - `tenant_id`: From `TenantInfo` extension (set by `ensure_tenant_middleware`), defaults to "default"
/// - `repo`: From URL path parameter
/// - `branch`: From URL path parameter
/// - `workspace`: From URL path parameter
/// - `auth_context`: From `AuthContext` extension (set by auth middleware), if present
///
/// # Path Format
///
/// Expects URL paths in the format: `/{repo}/{branch}/{workspace}/...`
///
/// # Usage
///
/// ```ignore
/// use crate::handlers::context::RequestContext;
///
/// pub async fn my_handler(
///     State(state): State<AppState>,
///     ctx: RequestContext,
/// ) -> Result<Json<...>, ApiError> {
///     let nodes_svc = ctx.node_service(&state);
///     // Use nodes_svc...
/// }
/// ```
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// Tenant identifier (from TenantInfo or "default")
    pub tenant_id: String,
    /// Repository name (from URL path)
    pub repo: String,
    /// Branch name (from URL path)
    pub branch: String,
    /// Workspace name (from URL path)
    pub workspace: String,
    /// Optional authentication context (from auth middleware)
    pub auth_context: Option<AuthContext>,
}

impl RequestContext {
    /// Create a new RequestContext with explicit values.
    ///
    /// This is useful for testing or when you need to construct a context manually.
    pub fn new(
        tenant_id: impl Into<String>,
        repo: impl Into<String>,
        branch: impl Into<String>,
        workspace: impl Into<String>,
        auth_context: Option<AuthContext>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo: repo.into(),
            branch: branch.into(),
            workspace: workspace.into(),
            auth_context,
        }
    }

    /// Create a NodeService configured with this context.
    ///
    /// This is the primary way to get a workspace-scoped NodeService from a RequestContext.
    pub fn node_service(&self, state: &AppState) -> NodeService<Store> {
        state.node_service_for_context(
            &self.tenant_id,
            &self.repo,
            &self.branch,
            &self.workspace,
            self.auth_context.clone(),
        )
    }

    /// Get a reference to the tenant ID.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// Get a reference to the repository name.
    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// Get a reference to the branch name.
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// Get a reference to the workspace name.
    pub fn workspace(&self) -> &str {
        &self.workspace
    }

    /// Check if this context has authentication.
    pub fn is_authenticated(&self) -> bool {
        self.auth_context.is_some()
    }
}

impl<S> FromRequestParts<S> for RequestContext
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Extract tenant_id from TenantInfo extension (set by ensure_tenant_middleware)
        let tenant_id = parts
            .extensions
            .get::<TenantInfo>()
            .map(|t| t.tenant_id.clone())
            .unwrap_or_else(|| "default".to_string());

        // Extract auth_context from AuthContext extension (set by auth middleware)
        let auth_context = parts.extensions.get::<AuthContext>().cloned();

        // Extract repo, branch, workspace from path
        // Expected path format: /{repo}/{branch}/{workspace}/...
        let Path((repo, branch, workspace)): Path<(String, String, String)> =
            Path::from_request_parts(parts, state)
                .await
                .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid path: expected /{repo}/{branch}/{workspace}"))?;

        Ok(RequestContext {
            tenant_id,
            repo,
            branch,
            workspace,
            auth_context,
        })
    }
}

/// Minimal request context containing only tenant and authentication.
///
/// Use this for handlers that don't have repo/branch/workspace in the path,
/// such as management endpoints.
///
/// # Usage
///
/// ```ignore
/// use crate::handlers::context::TenantContext;
///
/// pub async fn my_handler(
///     State(state): State<AppState>,
///     ctx: TenantContext,
/// ) -> Result<Json<...>, ApiError> {
///     let tenant_id = ctx.tenant_id();
///     // ...
/// }
/// ```
#[derive(Debug, Clone)]
pub struct TenantContext {
    /// Tenant identifier (from TenantInfo or "default")
    pub tenant_id: String,
    /// Optional authentication context (from auth middleware)
    pub auth_context: Option<AuthContext>,
}

impl TenantContext {
    /// Create a new TenantContext with explicit values.
    pub fn new(tenant_id: impl Into<String>, auth_context: Option<AuthContext>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            auth_context,
        }
    }

    /// Get a reference to the tenant ID.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// Check if this context has authentication.
    pub fn is_authenticated(&self) -> bool {
        self.auth_context.is_some()
    }
}

impl<S> FromRequestParts<S> for TenantContext
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract tenant_id from TenantInfo extension (set by ensure_tenant_middleware)
        let tenant_id = parts
            .extensions
            .get::<TenantInfo>()
            .map(|t| t.tenant_id.clone())
            .unwrap_or_else(|| "default".to_string());

        // Extract auth_context from AuthContext extension (set by auth middleware)
        let auth_context = parts.extensions.get::<AuthContext>().cloned();

        Ok(TenantContext {
            tenant_id,
            auth_context,
        })
    }
}

/// Request context for repo-scoped endpoints (without workspace).
///
/// Use this for handlers that have repo in the path but not workspace,
/// such as repository management endpoints.
///
/// # Path Format
///
/// Expects URL paths in the format: `/{repo}/...`
///
/// # Usage
///
/// ```ignore
/// use crate::handlers::context::RepoContext;
///
/// pub async fn my_handler(
///     State(state): State<AppState>,
///     ctx: RepoContext,
/// ) -> Result<Json<...>, ApiError> {
///     let repo = ctx.repo();
///     // ...
/// }
/// ```
#[derive(Debug, Clone)]
pub struct RepoContext {
    /// Tenant identifier (from TenantInfo or "default")
    pub tenant_id: String,
    /// Repository name (from URL path)
    pub repo: String,
    /// Optional authentication context (from auth middleware)
    pub auth_context: Option<AuthContext>,
}

impl RepoContext {
    /// Create a new RepoContext with explicit values.
    pub fn new(
        tenant_id: impl Into<String>,
        repo: impl Into<String>,
        auth_context: Option<AuthContext>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo: repo.into(),
            auth_context,
        }
    }

    /// Get a reference to the tenant ID.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// Get a reference to the repository name.
    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// Check if this context has authentication.
    pub fn is_authenticated(&self) -> bool {
        self.auth_context.is_some()
    }
}

impl<S> FromRequestParts<S> for RepoContext
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Extract tenant_id from TenantInfo extension (set by ensure_tenant_middleware)
        let tenant_id = parts
            .extensions
            .get::<TenantInfo>()
            .map(|t| t.tenant_id.clone())
            .unwrap_or_else(|| "default".to_string());

        // Extract auth_context from AuthContext extension (set by auth middleware)
        let auth_context = parts.extensions.get::<AuthContext>().cloned();

        // Extract repo from path
        let Path(repo): Path<String> = Path::from_request_parts(parts, state)
            .await
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid path: expected /{repo}"))?;

        Ok(RepoContext {
            tenant_id,
            repo,
            auth_context,
        })
    }
}

// ============================================================================
// Helper functions for handlers that can't use extractor-based approach
// ============================================================================

use axum::Extension;

/// Extract auth context from optional Extension.
///
/// This is a helper for handlers that use `Option<Extension<AuthContext>>` pattern.
///
/// # Example
///
/// Before:
/// ```ignore
/// let auth_context = auth.map(|Extension(ctx)| ctx);
/// ```
///
/// After:
/// ```ignore
/// let auth_context = extract_auth(auth);
/// ```
pub fn extract_auth(auth: Option<Extension<AuthContext>>) -> Option<AuthContext> {
    auth.map(|Extension(ctx)| ctx)
}

/// Get the default tenant ID.
///
/// This is a helper that provides a consistent default value.
/// In the future, this will be extracted from middleware.
///
/// # Example
///
/// Before:
/// ```ignore
/// let tenant_id = "default"; // TODO: Extract from middleware/auth
/// ```
///
/// After:
/// ```ignore
/// let tenant_id = default_tenant_id();
/// ```
pub fn default_tenant_id() -> &'static str {
    "default"
}

/// Get the default repository ID.
///
/// This is a helper that provides a consistent default value.
/// In the future, this will be extracted from the path.
pub fn default_repo_id() -> &'static str {
    "main"
}

/// Get the default branch name.
///
/// This is a helper that provides a consistent default value.
pub fn default_branch() -> &'static str {
    "main"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_context_new() {
        let ctx = RequestContext::new("tenant1", "myrepo", "main", "content", None);
        assert_eq!(ctx.tenant_id(), "tenant1");
        assert_eq!(ctx.repo(), "myrepo");
        assert_eq!(ctx.branch(), "main");
        assert_eq!(ctx.workspace(), "content");
        assert!(!ctx.is_authenticated());
    }

    #[test]
    fn test_request_context_with_auth() {
        let auth = AuthContext::system();
        let ctx = RequestContext::new("default", "repo", "main", "ws", Some(auth));
        assert!(ctx.is_authenticated());
    }

    #[test]
    fn test_tenant_context_new() {
        let ctx = TenantContext::new("tenant1", None);
        assert_eq!(ctx.tenant_id(), "tenant1");
        assert!(!ctx.is_authenticated());
    }

    #[test]
    fn test_repo_context_new() {
        let ctx = RepoContext::new("tenant1", "myrepo", None);
        assert_eq!(ctx.tenant_id(), "tenant1");
        assert_eq!(ctx.repo(), "myrepo");
        assert!(!ctx.is_authenticated());
    }
}
