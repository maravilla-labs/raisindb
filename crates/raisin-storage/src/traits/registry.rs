// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Registry and tree repository trait definitions.
//!
//! This module contains traits for:
//! - `RegistryRepository` - For tracking tenants and deployments
//! - `TreeRepository` - For content-addressed tree storage (Git-like)

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models as models;
use std::collections::HashMap;

use crate::scope::RepoScope;

/// Registry for tracking tenants and deployments
///
/// Note: Tenant-level operations (`register_tenant`, `get_tenant`, etc.) do not
/// use scope types because they operate at the tenant level (single `tenant_id`),
/// which is below the minimum `RepoScope` granularity. Deployment operations
/// also remain with individual parameters since they span tenant + deployment_key.
pub trait RegistryRepository: Send + Sync {
    // Tenant operations
    fn register_tenant(
        &self,
        tenant_id: &str,
        metadata: HashMap<String, String>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn get_tenant(
        &self,
        tenant_id: &str,
    ) -> impl std::future::Future<Output = Result<Option<models::registry::TenantRegistration>>> + Send;

    fn list_tenants(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<models::registry::TenantRegistration>>> + Send;

    fn update_tenant_last_seen(
        &self,
        tenant_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    // Deployment operations
    fn register_deployment(
        &self,
        tenant_id: &str,
        deployment_key: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn get_deployment(
        &self,
        tenant_id: &str,
        deployment_key: &str,
    ) -> impl std::future::Future<Output = Result<Option<models::registry::DeploymentRegistration>>> + Send;

    fn list_deployments(
        &self,
        tenant_id: Option<&str>,
    ) -> impl std::future::Future<Output = Result<Vec<models::registry::DeploymentRegistration>>> + Send;

    fn update_deployment_nodetype_version(
        &self,
        tenant_id: &str,
        deployment_key: &str,
        version: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn update_deployment_last_seen(
        &self,
        tenant_id: &str,
        deployment_key: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// Repository interface for content-addressed tree storage.
///
/// Provides Git-like immutable trees for revision snapshots. Each tree is identified
/// by the BLAKE3 hash of its serialized contents, enabling structural sharing and
/// efficient time-travel queries.
///
/// # Scoped Architecture
///
/// All methods take a `RepoScope` (tenant + repo) since trees are
/// repository-scoped, not branch-scoped.
///
/// # Tree Structure
///
/// - Each tree is a sorted list of TreeEntry items
/// - Tree ID = BLAKE3(serialized_entries)
/// - Unchanged subtrees share the same tree_id across revisions
/// - Reads never touch HEAD - directly navigate from commit's root_tree_id
///
/// # Key Operations
///
/// - `build_leaf`: Create a tree from entries, returns content hash
/// - `iter_tree`: Read entries from a tree, supports pagination
/// - `get_tree_entry`: Lookup single entry by key
pub trait TreeRepository: Send + Sync {
    /// Build a leaf tree from entries and return its content-addressed ID.
    ///
    /// Entries will be sorted by entry_key before hashing. Same entries always
    /// produce the same tree_id (structural sharing).
    ///
    /// # Arguments
    ///
    /// * `scope` - Repository scope (tenant + repo)
    /// * `entries` - List of entries (will be sorted)
    ///
    /// # Returns
    ///
    /// 32-byte BLAKE3 hash of the serialized entries
    fn build_leaf(
        &self,
        scope: RepoScope<'_>,
        entries: &[models::tree::TreeEntry],
    ) -> impl std::future::Future<Output = Result<[u8; 32]>> + Send;

    /// Iterate entries in a tree with pagination support.
    ///
    /// Returns entries in sorted order by entry_key.
    ///
    /// # Arguments
    ///
    /// * `scope` - Repository scope (tenant + repo)
    /// * `tree_id` - Content hash of the tree
    /// * `start_after` - Resume after this key (exclusive, for pagination)
    /// * `limit` - Maximum entries to return
    fn iter_tree(
        &self,
        scope: RepoScope<'_>,
        tree_id: &[u8; 32],
        start_after: Option<&str>,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<models::tree::TreeEntry>>> + Send;

    /// Get a single tree entry by key.
    ///
    /// # Arguments
    ///
    /// * `scope` - Repository scope (tenant + repo)
    /// * `tree_id` - Content hash of the tree
    /// * `entry_key` - Key to lookup
    fn get_tree_entry(
        &self,
        scope: RepoScope<'_>,
        tree_id: &[u8; 32],
        entry_key: &str,
    ) -> impl std::future::Future<Output = Result<Option<models::tree::TreeEntry>>> + Send;

    /// Get the root tree ID for a specific revision.
    ///
    /// This is the entry point for reading the repository state at a revision.
    /// Uses the denormalized tree_id stored at commit time.
    ///
    /// # Arguments
    ///
    /// * `scope` - Repository scope (tenant + repo)
    /// * `revision` - Revision number
    ///
    /// # Returns
    ///
    /// 32-byte tree_id, or None if revision doesn't exist
    fn get_root_tree_id(
        &self,
        scope: RepoScope<'_>,
        revision: &HLC,
    ) -> impl std::future::Future<Output = Result<Option<[u8; 32]>>> + Send;
}
