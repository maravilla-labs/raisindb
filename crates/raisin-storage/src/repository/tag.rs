// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Tag management storage trait

use raisin_context::Tag;
use raisin_error::Result;
use raisin_hlc::HLC;

/// Tag management storage operations.
///
/// Provides operations for managing Git-like immutable tags pointing to specific revisions.
pub trait TagRepository: Send + Sync {
    /// Create a new tag pointing to a revision
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `tag_name` - Name for the new tag (e.g., "v1.0.0")
    /// * `revision` - Revision (HLC timestamp) this tag points to
    /// * `created_by` - Actor who created the tag
    /// * `message` - Optional annotation message
    /// * `protected` - Whether the tag is protected from deletion
    ///
    /// # Returns
    /// The created tag
    fn create_tag(
        &self,
        tenant_id: &str,
        repo_id: &str,
        tag_name: &str,
        revision: &HLC,
        created_by: &str,
        message: Option<String>,
        protected: bool,
    ) -> impl std::future::Future<Output = Result<Tag>> + Send;

    /// Get tag information
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `tag_name` - Tag name
    ///
    /// # Returns
    /// Tag information if it exists
    fn get_tag(
        &self,
        tenant_id: &str,
        repo_id: &str,
        tag_name: &str,
    ) -> impl std::future::Future<Output = Result<Option<Tag>>> + Send;

    /// List all tags in a repository
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    ///
    /// # Returns
    /// Vector of tags sorted by name
    fn list_tags(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<Tag>>> + Send;

    /// Delete a tag
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `tag_name` - Tag name
    ///
    /// # Returns
    /// `true` if deleted, `false` if not found
    fn delete_tag(
        &self,
        tenant_id: &str,
        repo_id: &str,
        tag_name: &str,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;
}
