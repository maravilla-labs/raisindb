//! Compound index building job handler
//!
//! This module handles background compound index building operations
//! for rebuilding indexes when NodeType definitions change.

use crate::{cf, cf_handle, keys};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::CompoundColumnValue;
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
    /// Cluster-wide build lease.
    ///
    /// `JobRegistry` dedup is per-PROCESS, so on an N-node deployment every
    /// node queues and runs this job for the same index. Two nodes rebuilding
    /// one keyspace concurrently interleave entries stamped at different
    /// revisions and each stamps `Ready` over the other. `None` (locks
    /// disabled, or the `inprocess` backend) serializes within ONE node only —
    /// which is exactly the caveat in the `[locks]` docs.
    lock_manager: Option<raisin_locks::LockManagerHandle>,
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
        }
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

        // Take the cluster lease BEFORE any work. Another node already
        // rebuilding this index means our copy is redundant, not failed — so
        // this returns Ok, it does not error.
        let lock_key = raisin_locks::scoped_key(
            tenant_id,
            repo_id,
            branch,
            &format!("compound-index-build:{workspace}:{index_name}"),
        );
        let lease = match &self.lock_manager {
            Some(lm) => {
                let owner = format!("compound-index-build:{index_name}");
                match lm.try_acquire(&lock_key, &owner, BUILD_LEASE_TTL).await? {
                    Some(guard) => Some(guard.token),
                    None => {
                        tracing::debug!(
                            index = %index_name,
                            "compound index is being built elsewhere; skipping"
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
        // Load NodeType definition
        use raisin_storage::NodeTypeRepository;
        let node_type = self
            .node_type_repo
            .get(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                node_type_name,
                None,
            )
            .await?
            .ok_or_else(|| Error::NotFound(format!("NodeType '{}' not found", node_type_name)))?;

        // Find the compound index definition
        let compound_indexes = node_type.compound_indexes.as_ref().ok_or_else(|| {
            Error::NotFound(format!(
                "NodeType '{}' has no compound indexes",
                node_type_name
            ))
        })?;

        let index_def = compound_indexes
            .iter()
            .find(|idx| idx.name == index_name)
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "Compound index '{}' not found in NodeType '{}'",
                    index_name, node_type_name
                ))
            })?;

        tracing::debug!(
            job_id = %job.id,
            index_columns = index_def.columns.len(),
            has_order_column = index_def.has_order_column,
            "Loaded compound index definition"
        );

        // Scan all nodes of this type using direct DB access
        let nodes =
            self.scan_nodes_by_type(tenant_id, repo_id, branch, workspace, node_type_name)?;

        tracing::info!(
            job_id = %job.id,
            total_nodes = nodes.len(),
            "Scanned nodes to index"
        );

        let head_revision = self.branch_head(tenant_id, repo_id, branch)?;

        // Build index entries in batches for performance
        let batch_size = 1000;
        let mut indexed_count = 0;
        let mut skipped_count = 0;

        for chunk in nodes.chunks(batch_size) {
            let mut batch = WriteBatch::default();
            let cf_compound = cf_handle(&self.db, cf::COMPOUND_INDEX)?;

            // Branch HEAD, not `HLC::now()` — see `branch_head`.
            let revision = head_revision;

            for node in chunk {
                // Extract column values from the node
                let mut column_values = Vec::with_capacity(index_def.columns.len());

                for column_def in &index_def.columns {
                    match crate::repositories::NodeRepositoryImpl::extract_compound_column_value(
                        node,
                        &column_def.property,
                        &column_def.column_type,
                    ) {
                        Some(value) => column_values.push(value),
                        None => {
                            // Skip this node if any required column is missing
                            tracing::trace!(
                                job_id = %job.id,
                                node_id = %node.id,
                                missing_property = %column_def.property,
                                "Skipping node: missing column value"
                            );
                            skipped_count += 1;
                            break;
                        }
                    }
                }

                // Only index if we got all required columns
                if column_values.len() == index_def.columns.len() {
                    let is_published = node.published_at.is_some();
                    let key = keys::compound_index_key_versioned(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        index_name,
                        &column_values,
                        &revision,
                        &node.id,
                        is_published,
                    );

                    batch.put_cf(cf_compound, key, b"");
                    indexed_count += 1;
                }
            }

            // Write the batch
            self.db
                .write(batch)
                .map_err(|e| Error::storage(e.to_string()))?;

            tracing::debug!(
                job_id = %job.id,
                indexed = indexed_count,
                skipped = skipped_count,
                "Batch indexed"
            );
        }

        // Stamp the state record LAST, and only on success. This is what flips
        // the planner's fail-closed gate open for this index — so a build that
        // errors out above leaves the previous answer in force rather than
        // advertising an index it did not finish writing.
        //
        // `index_def` is stamped rather than the index NAME alone: the record
        // carries the declaration's fingerprint, which is how a later
        // declaration change is detected as stale instead of silently misread.
        let state_store = crate::compound_state::CompoundStateStore::new(self.db.clone());
        let mut state =
            raisin_storage::compound::CompoundIndexState::ready(index_def, head_revision);
        state.nodes_indexed = indexed_count as u64;
        state_store.put(tenant_id, repo_id, branch, workspace, &state)?;

        tracing::info!(
            job_id = %job.id,
            total_nodes = nodes.len(),
            indexed_count = indexed_count,
            skipped_count = skipped_count,
            "Compound index build completed"
        );

        Ok(())
    }

    /// Scan nodes of a specific type using direct DB access
    ///
    /// Scans the NODES column family and filters by node_type.
    fn scan_nodes_by_type(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type_name: &str,
    ) -> Result<Vec<Node>> {
        // Scan all nodes in this workspace and filter by type
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .build_prefix();

        let cf_nodes = cf_handle(&self.db, cf::NODES)?;

        let iter = self.db.iterator_cf(
            cf_nodes,
            rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
        );

        let mut nodes = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        for item in iter {
            let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix) {
                break;
            }

            // Skip tombstone markers (empty value)
            if value.is_empty() {
                continue;
            }

            // Deserialize node
            let node: Node = match rmp_serde::from_slice(&value) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!("Failed to deserialize node: {}", e);
                    continue;
                }
            };

            // Filter by node_type
            if node.node_type != node_type_name {
                continue;
            }

            // Deduplicate by node_id (we only want the latest version)
            if seen_ids.contains(&node.id) {
                continue;
            }
            seen_ids.insert(node.id.clone());

            nodes.push(node);
        }

        Ok(nodes)
    }
}
