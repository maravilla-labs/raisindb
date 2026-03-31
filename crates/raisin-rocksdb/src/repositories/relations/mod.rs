//! Relationship repository implementation for RocksDB
//!
//! This module provides revision-aware relationship indexing with:
//! - Bidirectional indexes (forward for outgoing, reverse for incoming)
//! - Global index for cross-workspace Cypher queries
//! - Cross-workspace support
//! - Time-travel queries via max_revision
//! - Efficient prefix scans for relationship lookup
//!
//! ## Module Structure
//!
//! - `helpers`: Shared utilities for serialization, key parsing, and iteration
//! - `crud`: Add and remove relation operations
//! - `queries`: Query operations (outgoing, incoming, by type)
//! - `deletion`: Bulk deletion operations
//! - `global`: Global index scanning for cross-workspace queries

mod crud;
mod deletion;
mod global;
pub mod helpers;
mod packed;
mod queries;

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::{FullRelation, RelationRef};
use raisin_storage::scope::{BranchScope, StorageScope};
use raisin_storage::{BranchRepository, RelationRepository};
use rocksdb::DB;
use std::sync::Arc;

/// RocksDB implementation of the RelationRepository trait
#[derive(Clone)]
pub struct RelationRepositoryImpl {
    db: Arc<DB>,
    branch_repo: Arc<super::BranchRepositoryImpl>,
    packed_repo: packed::PackedRelationRepository,
}

impl RelationRepositoryImpl {
    /// Create a new RelationRepositoryImpl
    pub fn new(db: Arc<DB>, branch_repo: Arc<super::BranchRepositoryImpl>) -> Self {
        Self {
            db: db.clone(),
            branch_repo,
            packed_repo: packed::PackedRelationRepository::new(db),
        }
    }

    /// Get the current HEAD revision for a branch
    async fn get_head_revision(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<HLC> {
        self.branch_repo.get_head(tenant_id, repo_id, branch).await
    }
}

impl RelationRepository for RelationRepositoryImpl {
    async fn add_relation(
        &self,
        scope: StorageScope<'_>,
        source_node_id: &str,
        source_node_type: &str,
        relation: RelationRef,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace: source_workspace,
        } = scope;
        // Get current HEAD revision
        let revision = self.get_head_revision(tenant_id, repo_id, branch).await?;

        // 1. Write to packed storage (New Format)
        self.packed_repo
            .add_relation(
                &revision,
                tenant_id,
                repo_id,
                branch,
                source_workspace,
                source_node_id,
                &relation,
            )
            .await?;

        // 2. Write to legacy storage (Old Format) - for backward compatibility during migration
        crud::add_relation(
            &self.db,
            &revision,
            tenant_id,
            repo_id,
            branch,
            source_workspace,
            source_node_id,
            source_node_type,
            &relation,
        )
        .await
    }

    async fn remove_relation(
        &self,
        scope: StorageScope<'_>,
        source_node_id: &str,
        target_workspace: &str,
        target_node_id: &str,
    ) -> Result<bool> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace: source_workspace,
        } = scope;
        // Get current HEAD revision
        let revision = self.get_head_revision(tenant_id, repo_id, branch).await?;

        // 1. Remove from packed storage
        self.packed_repo
            .remove_relation(
                &revision,
                tenant_id,
                repo_id,
                branch,
                source_workspace,
                source_node_id,
                target_workspace,
                target_node_id,
            )
            .await?;

        // 2. Remove from legacy storage
        crud::remove_relation(
            &self.db,
            &revision,
            tenant_id,
            repo_id,
            branch,
            source_workspace,
            source_node_id,
            target_workspace,
            target_node_id,
        )
        .await
    }

    async fn get_outgoing_relations(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<RelationRef>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let head_revision;
        let max_rev = match max_revision {
            Some(r) => r,
            None => {
                head_revision = self.get_head_revision(tenant_id, repo_id, branch).await?;
                &head_revision
            }
        };

        // 1. Try to read from packed storage first
        if let Ok(Some(packed_relations)) = self
            .packed_repo
            .get_packed_relations(max_rev, tenant_id, repo_id, branch, workspace, node_id)
            .await
        {
            // Convert CompactRelation to RelationRef
            return Ok(packed_relations
                .into_iter()
                .map(|c| RelationRef {
                    target: c.target_id,
                    workspace: c.target_workspace,
                    target_node_type: c.target_node_type,
                    relation_type: c.relation_type,
                    weight: c.weight,
                })
                .collect());
        }

        // 2. Fallback to legacy storage
        queries::get_outgoing_relations(
            &self.db, max_rev, tenant_id, repo_id, branch, workspace, node_id,
        )
        .await
    }

    async fn get_incoming_relations(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<(String, String, RelationRef)>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let head_revision;
        let max_rev = match max_revision {
            Some(r) => r,
            None => {
                head_revision = self.get_head_revision(tenant_id, repo_id, branch).await?;
                &head_revision
            }
        };

        queries::get_incoming_relations(
            &self.db, max_rev, tenant_id, repo_id, branch, workspace, node_id,
        )
        .await
    }

    async fn get_relations_by_type(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        target_node_type: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<RelationRef>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let head_revision;
        let max_rev = match max_revision {
            Some(r) => r,
            None => {
                head_revision = self.get_head_revision(tenant_id, repo_id, branch).await?;
                &head_revision
            }
        };

        queries::get_relations_by_type(
            &self.db,
            max_rev,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            target_node_type,
        )
        .await
    }

    async fn remove_all_relations_for_node(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let revision = self.get_head_revision(tenant_id, repo_id, branch).await?;

        deletion::remove_all_relations_for_node(
            &self.db, &revision, tenant_id, repo_id, branch, workspace, node_id,
        )
        .await
    }

    async fn scan_relations_global(
        &self,
        scope: BranchScope<'_>,
        relation_type_filter: Option<&str>,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<(String, String, String, String, FullRelation)>> {
        let BranchScope {
            tenant_id,
            repo_id,
            branch,
        } = scope;
        let head_revision;
        let max_rev = match max_revision {
            Some(r) => r,
            None => {
                head_revision = self.get_head_revision(tenant_id, repo_id, branch).await?;
                &head_revision
            }
        };

        global::scan_relations_global(
            &self.db,
            max_rev,
            tenant_id,
            repo_id,
            branch,
            relation_type_filter,
        )
        .await
    }
}
