//! Compound index building job handler
//!
//! This module handles background compound index building operations
//! for rebuilding indexes when NodeType definitions change.

use crate::{cf, cf_handle, keys};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use rocksdb::{WriteBatch, DB};
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
            .build(
                job,
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_type_name,
                index_name,
            )
            .await;

        if let (Some(lm), Some(token)) = (&self.lock_manager, lease) {
            let _ = lm.release(&lock_key, token).await;
        }
        result
    }

    /// The build itself, split out so the lease above is always released.
    ///
    /// A mark that arrives DURING a build (a replicated write whose definitions
    /// were cold, a merge) makes the final `Ready` lose its compare-and-set; the
    /// build then runs again, a bounded number of times — the request that
    /// marked it was usually what queued this very job, so waiting for another
    /// trigger would leave the index scan-only.
    #[allow(clippy::too_many_arguments)]
    async fn build(
        &self,
        job: &JobInfo,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type_name: &str,
        index_name: &str,
    ) -> Result<()> {
        const ATTEMPTS: usize = 3;
        for attempt in 1..=ATTEMPTS {
            if self
                .build_once(
                    job,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    node_type_name,
                    index_name,
                )
                .await?
            {
                return Ok(());
            }
            tracing::info!(
                job_id = %job.id,
                index = %index_name,
                attempt,
                "Compound index build finished behind a newer stale mark; building again"
            );
        }
        Ok(())
    }

    /// One build pass; `Ok(true)` when it stamped `Ready`.
    #[allow(clippy::too_many_arguments)]
    async fn build_once(
        &self,
        job: &JobInfo,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type_name: &str,
        index_name: &str,
    ) -> Result<bool> {
        use crate::indexing::compound::build;
        let scope = raisin_storage::BranchScope::new(tenant_id, repo_id, branch);
        // The definitions READ FROM STORAGE (never cache-first, so a cache
        // lagging a declaration change cannot hand the build the old columns),
        // inheritance included: every type whose resolved declarations carry
        // this index NAME writes into its keyspace. The same answer warms the
        // cache the replication apply path maintains the index from.
        let fresh = crate::indexing::compound::defs::fresh_branch(
            &self.db,
            &self.node_type_repo,
            scope,
            &[],
        )
        .await?;
        let declaring = fresh
            .get(node_type_name)
            .ok_or_else(|| Error::NotFound(format!("NodeType '{}' not found", node_type_name)))?;
        let index_def = declaring
            .compound
            .iter()
            .find(|idx| idx.name == index_name)
            .cloned()
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "Compound index '{}' not found in NodeType '{}'",
                    index_name, node_type_name
                ))
            })?;
        let wanted: build::Wanted = fresh
            .iter()
            .filter_map(|(name, defs)| {
                let def = defs.compound.iter().find(|d| d.name == index_name)?;
                Some((name.clone(), vec![def.clone()]))
            })
            .collect();
        let ctx = crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace);

        // Refuse BEFORE the clear: a node the build cannot place would lose
        // its entries to the clear and never get them back.
        build::precheck(
            &self.db,
            &ctx,
            &wanted,
            &self.branch_head(tenant_id, repo_id, branch)?,
        )?;

        // Register the build BEFORE clearing or reading any node: a mark that
        // arrives after this point advances the generation and makes the
        // final `Ready` lose — see `compound_state::marker`.
        let state_store = crate::compound_state::CompoundStateStore::new(self.db.clone());
        let started_under = state_store.begin_rebuild(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &index_def,
            self.branch_head(tenant_id, repo_id, branch)?,
        )?;

        // The clear and re-derive insert below existing entries: hold the
        // (branch, COMPOUND) against run-collapse until the build is written.
        let _inserting = crate::management::cf_exclusion::enter_inserter_async(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            crate::cf::COMPOUND_INDEX,
        )
        .await;
        // Clear this index's keyspace (both tags), THEN read the floor and
        // scan: a write committed before the floor read is in the scan, one
        // after it writes its own entries over the cleared keyspace.
        self.clear_index(tenant_id, repo_id, branch, workspace, index_name)?;
        let floor = self.branch_head(tenant_id, repo_id, branch)?;
        let outcome = build::write(&self.db, &ctx, &wanted, &floor)?;
        if outcome.unplaceable > 0 {
            state_store.mark_not_built(tenant_id, repo_id, branch, workspace, index_name)?;
            build::refuse_unplaceable(&ctx, &outcome)?;
        }

        // Stamp the state record LAST, and only on success: this flips the
        // planner's fail-closed gate open — for reads at or above the floor.
        // Compare-and-set against marks that arrived during the build.
        let mut state = raisin_storage::compound::CompoundIndexState::ready(&index_def, floor);
        state.nodes_indexed = outcome.nodes as u64;
        let stamped = state_store.complete_build(
            tenant_id,
            repo_id,
            branch,
            workspace,
            state,
            started_under,
        )?;
        if stamped {
            tracing::info!(
                job_id = %job.id,
                nodes = outcome.nodes,
                entries = outcome.entries,
                floor = %floor,
                "Compound index build completed"
            );
        }
        Ok(stamped)
    }

    /// Delete every entry of one index (both tags) — the keyspace is rebuilt
    /// from the nodes right after.
    fn clear_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        index_name: &str,
    ) -> Result<()> {
        let cf_compound = cf_handle(&self.db, cf::COMPOUND_INDEX)?;
        for published in [false, true] {
            let prefix = keys::compound_index_prefix(
                tenant_id,
                repo_id,
                branch,
                workspace,
                index_name,
                &[],
                published,
            );
            let mut batch = WriteBatch::default();
            for item in crate::prefix_scan(&self.db, cf_compound, &prefix) {
                let (key, _) = item.map_err(|e| Error::storage(e.to_string()))?;
                if !key.starts_with(&prefix) {
                    break;
                }
                batch.delete_cf(cf_compound, key);
                if batch.len() >= 10_000 {
                    self.db
                        .write(std::mem::take(&mut batch))
                        .map_err(|e| Error::storage(e.to_string()))?;
                }
            }
            self.db
                .write(batch)
                .map_err(|e| Error::storage(e.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "compound_index_tests.rs"]
mod tests;
