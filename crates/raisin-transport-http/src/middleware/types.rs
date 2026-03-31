// SPDX-License-Identifier: BSL-1.1

//! Shared types for middleware layers.

/// Enum to represent the type of authenticated principal.
#[cfg(feature = "storage-rocksdb")]
#[derive(Debug, Clone)]
pub enum AuthPrincipal {
    /// Admin user (console/API user with AdminClaims).
    Admin(raisin_rocksdb::AdminClaims),
    /// Regular user identity (with AuthClaims from user JWT).
    User(Box<raisin_models::auth::AuthClaims>),
}

/// Parsed request context containing repository, branch, workspace, path, and metadata.
#[derive(Debug, Clone)]
pub struct RaisinContext {
    pub repo_name: String,
    pub branch_name: String,
    pub workspace_name: String,
    pub cleaned_path: String,
    pub original_path: String,
    pub file_extension: Option<String>,
    pub is_version: bool,
    pub version_id: Option<i32>,
    pub is_command: bool,
    pub command_name: Option<String>,
    pub property_path: Option<String>,
    pub archetype: String,
}

/// Tenant context extracted from request.
#[derive(Debug, Clone)]
pub struct TenantInfo {
    pub tenant_id: String,
    pub deployment_key: String,
}
