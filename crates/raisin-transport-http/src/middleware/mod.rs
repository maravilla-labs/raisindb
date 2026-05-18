// SPDX-License-Identifier: BSL-1.1

//! HTTP middleware layers for authentication, CORS, tenant initialization,
//! and path parsing.

mod auth;
mod cors;
mod parsing;
mod path_helpers;
mod scoped_tenant;
mod tenant;
pub mod types;

// Re-export all public middleware functions to preserve `crate::middleware::*` paths.
pub use auth::require_superadmin_token_middleware;
pub use parsing::raisin_parsing_middleware;
pub use scoped_tenant::ScopedTenant;
pub use tenant::ensure_tenant_middleware;
pub use types::{RaisinContext, TenantInfo};

#[cfg(feature = "storage-rocksdb")]
pub use auth::{optional_auth_middleware, require_admin_auth_middleware, require_auth_middleware};

#[cfg(feature = "storage-rocksdb")]
pub use cors::{repo_auth_cors_middleware, unified_cors_middleware};

#[cfg(feature = "storage-rocksdb")]
pub use types::AuthPrincipal;
