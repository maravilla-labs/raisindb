// SPDX-License-Identifier: BSL-1.1

// TODO(v0.2): Update deprecated API usages and remove dead code
#![allow(deprecated)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

//! Minimal Axum HTTP transport for RaisinDB

pub mod error;
mod errors;
mod extractors;
pub mod middleware;
mod routes;
#[cfg(feature = "storage-rocksdb")]
pub use routes::operator_package_routes;
pub mod state;
mod types;
pub mod upload_processors;
pub(crate) mod util;
mod handlers {
    pub mod admin;
    #[cfg(feature = "storage-rocksdb")]
    pub mod admin_users;
    #[cfg(feature = "storage-rocksdb")]
    pub mod ai;
    pub mod archetypes;
    pub mod audit;
    #[cfg(feature = "storage-rocksdb")]
    pub mod auth;
    pub mod branches;
    pub mod commit;
    pub mod context;
    pub mod conversations;
    pub mod element_types;
    #[cfg(feature = "storage-rocksdb")]
    pub mod embeddings;
    pub mod functions;
    pub mod history;
    #[cfg(feature = "storage-rocksdb")]
    pub mod hybrid_search;
    #[cfg(feature = "storage-rocksdb")]
    pub mod identity_auth;
    #[cfg(feature = "storage-rocksdb")]
    pub mod identity_users;
    pub mod inbox;
    #[cfg(feature = "storage-rocksdb")]
    pub mod integrations;
    pub mod locks;
    pub mod management;
    pub mod mcp;
    pub mod mixins;
    pub mod node_types;
    #[cfg(feature = "storage-rocksdb")]
    pub mod oauth_as;
    pub mod packages;
    #[cfg(feature = "storage-rocksdb")]
    pub mod processing_rules;
    #[cfg(feature = "storage-rocksdb")]
    pub mod profile;
    pub mod query;
    pub mod registry;
    #[cfg(feature = "storage-rocksdb")]
    pub mod replication;
    pub mod repo;
    pub mod repositories;
    pub mod revisions;
    pub mod scheduler;
    #[cfg(feature = "storage-rocksdb")]
    pub mod sql;
    pub mod static_site;
    pub mod system_definitions;
    #[cfg(feature = "storage-rocksdb")]
    pub mod system_updates;
    pub mod tags;
    pub mod translations;
    pub mod uploads;
    pub mod webhooks;
    #[cfg(feature = "storage-rocksdb")]
    pub mod workspace_access;
    pub mod workspaces;
}
/// Identity-provisioning primitives, re-exported for the operator surface.
///
/// `raisin-server` mounts superadmin-gated identity-user endpoints under
/// `/management/admin/*`. Those handlers need the same building blocks the
/// customer-facing `/api/.../identity-users` handlers use, so the pieces are
/// exposed here rather than duplicated. Nothing here is tenant- or
/// application-specific: callers supply the repositories and roles they want.
#[cfg(feature = "storage-rocksdb")]
pub mod identity_provisioning {
    pub use crate::handlers::identity_auth::helpers::{validate_email, validate_password};
    pub use crate::handlers::identity_auth::user_node::ensure_user_node;
    pub use crate::handlers::identity_users::IdentityUserResponse;
}

// Note: router() is only available when s3 feature is disabled (for tests)
// Production code uses router_with_bin_and_audit() directly
#[cfg(not(feature = "s3"))]
pub use state::router;
pub use state::router_with_bin_and_audit;
