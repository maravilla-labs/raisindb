// SPDX-License-Identifier: BSL-1.1

//! Repository node handlers.
//!
//! This module provides HTTP handlers for CRUD operations on repository nodes,
//! including file uploads, command execution, translation management,
//! full-text search, and signed asset URLs.

mod assets;
mod commands;
mod commands_commit;
mod commands_translation;
mod commands_versioning;
mod get;
mod get_listing;
mod helpers;
mod post;
mod post_external;
mod post_multipart;
mod revision;
mod search;
pub(crate) mod translation_helpers;
mod upload;
mod write;

// Re-export all public handler functions at the module level
// so existing imports like `crate::handlers::repo::repo_get` continue to work.
pub use get::{repo_get, repo_get_by_id, repo_get_root};
pub use post::{repo_post, repo_post_root};
pub use revision::{repo_get_at_revision, repo_get_by_id_at_revision, repo_get_root_at_revision};
pub use write::{repo_delete, repo_put};

pub use commands::repo_execute_command;

#[cfg(feature = "storage-rocksdb")]
pub use search::{fulltext_search, FullTextSearchRequest, SearchResultItem};

pub use assets::{SignAssetRequest, SignAssetResponse};
