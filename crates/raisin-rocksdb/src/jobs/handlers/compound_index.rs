//! Compound index building job handler
//!
//! This module handles background compound index building operations
//! for rebuilding indexes when NodeType definitions change.

use crate::{cf, cf_handle, keys};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use rocksdb::DB;
use std::sync::Arc;

use crate::repositories::{BranchRepositoryImpl, NodeTypeRepositoryImpl, RevisionRepositoryImpl};

/// How long one node may hold the build lease. Generous: a build walks every
/// node of a type, and a lease that expires mid-build lets a second node start
/// writing into the same keyspace.
const BUILD_LEASE_TTL: std::time::Duration = std::time::Duration::from_secs(1800);

/// Handler for compound index building jobs
///
/// This handler processes CompoundIndexBuild jobs by:
/// 1. Extracting parameters from JobType
/// 2. Loading the NodeType definition to get index configuration
/// 3. Scanning all nodes of the specified node_type
/// 4. For each node, extracting column values and indexing them
pub struct CompoundIndexJobHandler {
    db: Arc<DB>,
    node_type_repo: NodeTypeRepositoryImpl,
    /// Build lease, scoped to THIS node (the key includes `node_id`).
    ///
    /// The compound keyspace and its state record are LOCAL: every node builds
    /// its own from its own records, nothing about them replicates. So the
    /// lease only has to stop two builds of one index on the SAME node from
    /// interleaving. It used to be cluster-wide, which let the node that did
    /// not need a build hold the lease while the one that did — a replica its
    /// replicated upserts had just marked `NotBuilt` — skipped with "being
    /// built elsewhere" and stayed scan-only.
    lock_manager: Option<raisin_locks::LockManagerHandle>,
    /// This node's identity in the lease key.
    node_id: String,
}

impl CompoundIndexJobHandler {
    /// Create a new compound index job handler
    ///
    /// # Arguments
    ///
    /// * `db` - RocksDB instance for all operations
    /// * `revision_repo` - Revision repository for NodeType lookups
    /// * `branch_repo` - Branch repository for NodeType lookups
    pub fn new(
        db: Arc<DB>,
        revision_repo: Arc<RevisionRepositoryImpl>,
        branch_repo: Arc<BranchRepositoryImpl>,
    ) -> Self {
        Self {
            node_type_repo: NodeTypeRepositoryImpl::new(db.clone(), revision_repo, branch_repo),
            db,
            lock_manager: None,
            // Unique per handler unless configured: a lease can then never be
            // shared with another node by accident.
            node_id: format!("process-{}", nanoid::nanoid!(8)),
        }
    }

    /// The cluster node id the build lease is scoped to.
    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = node_id.into();
        self
    }

    /// Attach the cluster lock manager. Without it the build is serialized
    /// within one process only.
    pub fn with_lock_manager(
        mut self,
        lock_manager: Option<raisin_locks::LockManagerHandle>,
    ) -> Self {
        self.lock_manager = lock_manager;
        self
    }

    /// The branch HEAD, which is what an index entry must be stamped with.
    ///
    /// NOT `HLC::now()`: an entry stamped in the future relative to every read
    /// is discarded by the MVCC filter, so the build would report success and
    /// index nothing. Same reasoning as the spatial build, and the same source
    /// the synchronous rebuild path uses.
    fn branch_head(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<HLC> {
        let cf_branches = cf_handle(&self.db, cf::BRANCHES)?;
        let branch_key = keys::branch_key(tenant_id, repo_id, branch);
        match self
            .db
            .get_cf(cf_branches, branch_key)
            .map_err(|e| Error::storage(format!("Failed to get branch: {}", e)))?
        {
            Some(data) => {
                let branch_meta: raisin_context::Branch = rmp_serde::from_slice(&data)
                    .map_err(|e| Error::storage(format!("Failed to deserialize branch: {}", e)))?;
                Ok(branch_meta.head)
            }
            None => Ok(HLC::new(0, 0)),
        }
    }

    /// Handle compound index build job
    ///
    /// Processes a CompoundIndexBuild job variant which builds the specified
    /// compound index for all nodes of the given node_type.
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing the JobType::CompoundIndexBuild variant
    /// * `_context` - Job context with tenant, repo, branch, workspace info
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Job type is not CompoundIndexBuild
    /// - NodeType doesn't exist or doesn't have the specified index
    /// - Index building fails
    pub async fn handle(&self, job: &JobInfo, _context: &JobContext) -> Result<()> {
        // Extract parameters from JobType
        let (tenant_id, repo_id, branch, workspace, node_type_name, index_name) =
            match &job.job_type {
                JobType::CompoundIndexBuild {
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    node_type_name,
                    index_name,
                } => (
                    tenant_id.as_str(),
                    repo_id.as_str(),
                    branch.as_str(),
                    workspace.as_str(),
                    node_type_name.as_str(),
                    index_name.as_str(),
                ),
                _ => {
                    return Err(Error::Validation(
                        "Expected CompoundIndexBuild job type".to_string(),
                    ))
                }
            };

        tracing::info!(
            job_id = %job.id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            workspace = %workspace,
            node_type = %node_type_name,
            index_name = %index_name,
            "Processing compound index build job"
        );

        // Take this NODE's lease before any work. A build of this index
        // already running here means ours is redundant, not failed — so this
        // returns Ok, it does not error. Another node's build never blocks
        // ours: see `lock_manager`.
        let lock_key = raisin_locks::scoped_key(
            tenant_id,
            repo_id,
            branch,
            &format!(
                "compound-index-build:{}:{workspace}:{index_name}",
                self.node_id
            ),
        );
        let lease = match &self.lock_manager {
            Some(lm) => {
                let owner = format!("compound-index-build:{index_name}");
                match lm.try_acquire(&lock_key, &owner, BUILD_LEASE_TTL).await? {
                    Some(guard) => Some(guard.token),
                    None => {
                        tracing::debug!(
                            index = %index_name,
                            "compound index is already being built on this node; skipping"
                        );
                        return Ok(());
                    }
                }
            }
            None => None,
        };

        let result = self
            .build_index(
                &job.id.to_string(),
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_type_name,
                index_name,
                0,
            )
            .await
            .map(|ended| {
                // Expected, not a failure: no retries, no ERROR (plan Phase
                // 13g). The `timestamp_backfill` repair re-requests the build.
                if let build_impl::BuildResult::MissingOrderValues(nodes) = ended {
                    tracing::warn!(
                        "{}",
                        crate::indexing::compound::build::missing_order_values_message(
                            &crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace),
                            nodes as usize,
                        )
                    );
                }
            });

        if let (Some(lm), Some(token)) = (&self.lock_manager, lease) {
            let _ = lm.release(&lock_key, token).await;
        }
        result
    }
}

#[path = "compound_index_build.rs"]
mod build_impl;
pub use build_impl::BuildResult;

#[cfg(test)]
#[path = "compound_index_tests.rs"]
mod tests;
