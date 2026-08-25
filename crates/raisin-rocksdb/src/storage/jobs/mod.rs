//! Background job system initialization and management
//!
//! This module handles initialization of the unified job system, including:
//! - Job handler registry setup
//! - Worker pool creation and startup
//! - Event handler subscription
//! - Job restoration after crash/restart
//! - Watchdog and cleanup tasks

mod flow_events;
mod init_system;
mod restore;
pub(crate) mod spatial;

use super::RocksDBStorage;
use raisin_error::Result;

impl RocksDBStorage {
    /// Queue a background job to build property index for a tenant/repo/branch/workspace
    ///
    /// This method creates a PropertyIndexBuild job and queues it in the job system.
    /// The job will be processed asynchronously by the worker pool.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    ///
    /// # Returns
    ///
    /// Returns the JobId for tracking the job status
    pub async fn queue_property_index_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<raisin_storage::jobs::JobId> {
        use raisin_hlc::HLC;
        use raisin_storage::jobs::{JobContext, JobType};
        use std::collections::HashMap;

        // Create job context
        let context = JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            workspace_id: workspace.to_string(),
            revision: HLC::new(0, 0), // Not applicable for index build
            metadata: HashMap::new(),
        };

        // Store job context BEFORE registering so dispatch can never
        // observe the job without its context.
        let job_id = raisin_storage::jobs::JobId::new();
        self.job_data_store.put(&job_id, &context)?;

        // Register job under the pre-generated ID
        self.job_registry
            .register_job_with_id(
                job_id.clone(),
                JobType::PropertyIndexBuild {
                    tenant_id: tenant_id.to_string(),
                    repo_id: repo_id.to_string(),
                    branch: branch.to_string(),
                    workspace: workspace.to_string(),
                },
                tenant_id.to_string(),
                None,
                None,
                None,
            )
            .await?;

        tracing::info!(
            job_id = %job_id,
            tenant = %tenant_id,
            repo = %repo_id,
            branch = %branch,
            workspace = %workspace,
            "Queued property index build job"
        );

        Ok(job_id)
    }

    /// Queue a build for every compound index on a branch that is not
    /// currently usable.
    ///
    /// This is the migration path AND the steady-state repair, deliberately one
    /// mechanism rather than three. It covers:
    ///
    /// - an existing database upgraded to a binary that has build state at all
    ///   (every index reads `NotBuilt` on first boot);
    /// - a declaration changed by package install, YAML edit or `ALTER … ADD`,
    ///   which `invalidate_changed_compound_state` has just marked stale;
    /// - a branch fork, which inherits no state because `cf::INDEX_STATUS` is
    ///   excluded from branch copy.
    ///
    /// A steady-state call writes NOTHING: every index answers `Ready`, the
    /// loop queues nothing, and the sweep costs one NodeType listing. That is
    /// what makes it safe to run periodically.
    ///
    /// Returns how many builds were queued.
    pub async fn sweep_compound_index_builds(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<usize> {
        use raisin_storage::compound::CompoundStateSource;
        use raisin_storage::NodeTypeRepository;

        let node_types = self
            .node_types
            .list(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                None,
            )
            .await?;

        let state = crate::compound_state::CompoundStateStore::new(self.db.clone());
        let mut queued = 0usize;
        let mut seen = std::collections::HashSet::new();

        for node_type in node_types {
            let Some(indexes) = node_type.compound_indexes.as_ref() else {
                continue;
            };
            for definition in indexes {
                // One keyspace per index NAME, so one build per name — even if
                // two NodeTypes declare it. Which is itself a misconfiguration,
                // warned about at upsert.
                if !seen.insert(definition.name.clone()) {
                    continue;
                }
                let availability =
                    state.compound_availability(tenant_id, repo_id, branch, workspace, definition);
                if availability.is_ready() {
                    continue;
                }
                tracing::info!(
                    index = %definition.name,
                    node_type = %node_type.name,
                    workspace = %workspace,
                    detail = %availability.explain_reason(),
                    "compound index is not usable; queueing a build"
                );
                self.queue_compound_index_build(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node_type.name,
                    &definition.name,
                )
                .await?;
                queued += 1;
            }
        }
        Ok(queued)
    }

    /// Queue a build for one compound index.
    ///
    /// This is the producer `JobType::CompoundIndexBuild` never had — the
    /// handler and its dispatch arm already existed with nothing to feed them.
    ///
    /// Called when a compound index is declared or changed, and by the boot
    /// sweep for declarations that have no build state. It is what makes the
    /// fail-closed planner gate self-healing instead of an upgrade flag day:
    /// an index reads `NotBuilt`, a build is queued, and the index comes back
    /// on its own.
    ///
    /// # Not deduplicated across a cluster
    ///
    /// `JobRegistry`'s dedup map is an in-memory `HashMap` — per PROCESS, not
    /// per cluster. On an N-node deployment every node reaches this for the
    /// same index. The HANDLER takes a `raisin_locks` lease to serialize that;
    /// see `jobs/handlers/compound_index.rs`. Do not rely on the queue alone.
    pub async fn queue_compound_index_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type_name: &str,
        index_name: &str,
    ) -> Result<raisin_storage::jobs::JobId> {
        use raisin_hlc::HLC;
        use raisin_storage::jobs::{JobContext, JobType};
        use std::collections::HashMap;

        let context = JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            workspace_id: workspace.to_string(),
            revision: HLC::new(0, 0), // Not applicable for an index build
            metadata: HashMap::new(),
        };

        // Context BEFORE registration, so dispatch can never observe a job
        // without its context.
        let job_id = raisin_storage::jobs::JobId::new();
        self.job_data_store.put(&job_id, &context)?;

        // Within this process, collapse repeats: a boot sweep and a NodeType
        // upsert can both reach here for the same index, and rebuilding twice
        // is pure waste.
        let dedup_key = format!("compound:{tenant_id}:{repo_id}:{branch}:{workspace}:{index_name}");

        let registered = self
            .job_registry
            .register_job_with_id_idempotent(
                job_id.clone(),
                JobType::CompoundIndexBuild {
                    tenant_id: tenant_id.to_string(),
                    repo_id: repo_id.to_string(),
                    branch: branch.to_string(),
                    workspace: workspace.to_string(),
                    node_type_name: node_type_name.to_string(),
                    index_name: index_name.to_string(),
                },
                tenant_id.to_string(),
                dedup_key,
                None,
            )
            .await?;

        if registered {
            tracing::info!(
                job_id = %job_id,
                tenant = %tenant_id,
                repo = %repo_id,
                branch = %branch,
                workspace = %workspace,
                index = %index_name,
                "Queued compound index build job"
            );
        } else {
            tracing::debug!(
                index = %index_name,
                "Compound index build already queued in this process; not re-queued"
            );
        }

        Ok(job_id)
    }

    /// Queue a spatial index build (or rebuild) for a workspace.
    ///
    /// `property = None` covers every geometry-valued property in the workspace.
    /// `rebuild = true` re-emits every entry and tombstones superseded ones;
    /// `false` fills gaps only.
    ///
    /// # Idempotency
    ///
    /// `JobType::SpatialIndexBuild`'s `dedup_key` is
    /// `spatial:{tenant}:{repo}:{branch}:{ws}:{property|*}`, so a duplicate request
    /// while one is queued or running collapses onto the existing job — the same
    /// mechanism the fulltext path uses.
    ///
    /// # Scope
    ///
    /// **LOCAL to this node.** The spatial index is derived local state, so a repair
    /// must be run on each node (or via each node's HTTP endpoint). Cluster-wide
    /// fan-out happens through *configuration*: `WorkspaceConfig.spatial` is
    /// replicated, so a policy change reaches every peer, and each peer then observes
    /// the `policy_hash` mismatch and schedules its own build.
    pub async fn queue_spatial_index_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: Option<&str>,
        rebuild: bool,
    ) -> Result<raisin_storage::jobs::JobId> {
        spatial::enqueue_spatial_index_build(
            &self.job_registry,
            &self.job_data_store,
            tenant_id,
            repo_id,
            branch,
            workspace,
            property,
            rebuild,
        )
        .await
    }

    /// Get the master encryption key.
    ///
    /// Delegates to the shared `raisin-crypto` loader, which reads
    /// `RAISIN_MASTER_KEY` with the legacy `EMBEDDING_MASTER_KEY` fallback.
    ///
    /// # Errors
    ///
    /// Returns an error if neither variable is set, or if the value present is
    /// not 32 bytes of hex.
    fn get_master_encryption_key() -> Result<[u8; 32]> {
        raisin_crypto::master_key_with_embedding_fallback()?.ok_or_else(|| {
            raisin_error::Error::Validation(
                "RAISIN_MASTER_KEY environment variable not set".to_string(),
            )
        })
    }
}
