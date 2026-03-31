// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Workspace and versioning repository trait definitions.
//!
//! This module contains traits for:
//! - `WorkspaceRepository` - For managing workspace configurations
//! - `VersioningRepository` - For tracking node version history

use raisin_error::Result;
use raisin_models as models;

use crate::scope::RepoScope;

/// Repository for managing workspaces.
///
/// All methods take a `RepoScope` (tenant + repo) since workspaces
/// are scoped at the repository level.
pub trait WorkspaceRepository: Send + Sync {
    fn get(
        &self,
        scope: RepoScope<'_>,
        id: &str,
    ) -> impl std::future::Future<Output = Result<Option<models::workspace::Workspace>>> + Send;

    fn put(
        &self,
        scope: RepoScope<'_>,
        ws: models::workspace::Workspace,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn list(
        &self,
        scope: RepoScope<'_>,
    ) -> impl std::future::Future<Output = Result<Vec<models::workspace::Workspace>>> + Send;
}

/// Versioning repository for tracking node history.
///
/// Provides functionality to create, list, and retrieve specific versions of nodes.
/// Each version captures a complete snapshot of a node's state at a specific point in time.
///
/// Note: Versioning methods do not take scope parameters because versions are
/// stored per-node (identified by node_id), not per-workspace.
pub trait VersioningRepository: Send + Sync {
    /// Create a new version of a node (without a note)
    ///
    /// # Arguments
    /// * `node` - The node to version
    ///
    /// # Returns
    /// The version number created
    ///
    /// # Note
    /// This is a convenience method that calls `create_version_with_note(node, None)`
    fn create_version(
        &self,
        node: &models::nodes::Node,
    ) -> impl std::future::Future<Output = Result<i32>> + Send {
        self.create_version_with_note(node, None)
    }

    /// List all versions for a node
    ///
    /// # Arguments
    /// * `node_id` - The ID of the node
    ///
    /// # Returns
    /// A vector of all versions for the node, ordered by version number
    fn list_versions(
        &self,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::NodeVersion>>> + Send;

    /// Get a specific version of a node
    ///
    /// # Arguments
    /// * `node_id` - The ID of the node
    /// * `version` - The version number to retrieve
    ///
    /// # Returns
    /// The requested version, or None if not found
    fn get_version(
        &self,
        node_id: &str,
        version: i32,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::NodeVersion>>> + Send;

    /// Delete all versions for a node
    ///
    /// # Arguments
    /// * `node_id` - The ID of the node whose versions should be deleted
    ///
    /// # Returns
    /// The number of versions deleted
    fn delete_all_versions(
        &self,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<usize>> + Send;

    /// Create a new version of a node with an optional comment/note
    ///
    /// # Arguments
    /// * `node` - The node to version
    /// * `note` - Optional comment describing this version
    ///
    /// # Returns
    /// The version number created
    fn create_version_with_note(
        &self,
        node: &models::nodes::Node,
        note: Option<String>,
    ) -> impl std::future::Future<Output = Result<i32>> + Send;

    /// Delete a specific version
    ///
    /// # Arguments
    /// * `node_id` - The ID of the node
    /// * `version` - The version number to delete
    ///
    /// # Returns
    /// True if the version was deleted, false if it didn't exist
    ///
    /// # Errors
    /// Returns an error if the version is published (cannot delete published versions)
    fn delete_version(
        &self,
        node_id: &str,
        version: i32,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Delete old versions, keeping only the N most recent
    ///
    /// # Arguments
    /// * `node_id` - The ID of the node
    /// * `keep_count` - Number of most recent versions to keep
    ///
    /// # Returns
    /// The number of versions deleted
    ///
    /// # Note
    /// Published versions are never deleted by this operation
    fn delete_old_versions(
        &self,
        node_id: &str,
        keep_count: usize,
    ) -> impl std::future::Future<Output = Result<usize>> + Send;

    /// Update the note/comment for a specific version
    ///
    /// # Arguments
    /// * `node_id` - The ID of the node
    /// * `version` - The version number to update
    /// * `note` - New note/comment for the version
    ///
    /// # Returns
    /// Success or error if version doesn't exist
    fn update_version_note(
        &self,
        node_id: &str,
        version: i32,
        note: Option<String>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}
