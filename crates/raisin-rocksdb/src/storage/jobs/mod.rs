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
        self.sweep_compound_index_builds_inner(tenant_id, repo_id, branch, workspace, None)
            .await
    }

    /// Sweep only the indexes declared by ONE node type.
    ///
    /// This is what a NodeType create/update event wants. That event fires
    /// once PER TYPE, and it used to answer by sweeping every type in every
    /// workspace of the branch — so a package deploy upserting 223 node types
    /// re-entered the whole sweep 223 times, each pass listing every node type
    /// and consulting build state for every declared index in every workspace.
    /// A change to one type can only invalidate that type's own indexes
    /// (`invalidate_changed_compound_state` marks exactly those), so the other
    /// 222 passes were re-deriving an answer nothing had changed.
    ///
    /// The narrowing means an UNRELATED index left un-ready is no longer
    /// healed by whatever schema event happens to pass next. That is what the
    /// boot sweep is for, and relying on a coincidence was never the design.
    pub async fn sweep_compound_index_builds_for_type(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type_name: &str,
    ) -> Result<usize> {
        self.sweep_compound_index_builds_inner(
            tenant_id,
            repo_id,
            branch,
            workspace,
            Some(node_type_name),
        )
        .await
    }

    async fn sweep_compound_index_builds_inner(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        only_node_type: Option<&str>,
    ) -> Result<usize> {
        use raisin_storage::compound::CompoundStateSource;
        use raisin_storage::NodeTypeRepository;

        // A compound index belongs to exactly ONE node type, and the build
        // handler's `scan_nodes_by_type` matches that name EXACTLY (a
        // subtype's nodes are not indexed by its base type's declaration).
        // So a workspace that cannot hold the owning type has nothing to
        // index, and queueing a build for it buys a full keyspace scan that
        // provably finds zero rows.
        //
        // Skipping those is not tidiness. Measured against the studio package
        // on 2026-09-09: 276 declared compound indexes x 37 workspaces =
        // 10,212 build jobs per branch, against the RocksDB backend's default
        // `max_active_jobs_per_tenant` of 5000. The sweep alone put the tenant
        // over its job cap, and once there EVERY write was refused with
        // "Tenant 'default' has 5000 non-terminal jobs registered" — a schema
        // sweep taking the whole database read-write. Filtering by containment
        // takes that same package to roughly one build per declared index.
        //
        // `allowed_node_types` is a DECLARATION, not a write-time constraint:
        // a node of an unlisted type can exist. What that costs is bounded and
        // it is not wrong answers — the planner's availability gate fails
        // CLOSED, so a query against an unbuilt index falls back to a scan and
        // returns the same rows more slowly, and listing the type in the
        // workspace makes the next sweep build it. An empty list means
        // "unrestricted"; an unreadable workspace falls open to the old
        // behaviour.
        let allowed_types: Option<std::collections::HashSet<String>> = {
            use raisin_storage::{Storage, WorkspaceRepository};
            match self
                .workspaces()
                .get(
                    raisin_storage::RepoScope::new(tenant_id, repo_id),
                    workspace,
                )
                .await
            {
                Ok(Some(ws)) if !ws.allowed_node_types.is_empty() => {
                    Some(ws.allowed_node_types.iter().cloned().collect())
                }
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!(
                        tenant = %tenant_id,
                        repo = %repo_id,
                        workspace = %workspace,
                        error = %e,
                        "compound index sweep: could not read workspace containment; sweeping every declared index"
                    );
                    None
                }
            }
        };

        // Containment is settled from the NAME alone, so answer the "this type
        // cannot live here" case before reading any NodeType at all. For a
        // per-type sweep that is the whole cost in the common case: one
        // workspace lookup and out.
        if let Some(name) = only_node_type {
            if allowed_types
                .as_ref()
                .is_some_and(|allowed| !allowed.contains(name))
            {
                return Ok(0);
            }
        }

        // A point lookup when the caller named a type; the full listing only
        // for the catch-all sweep. Listing every node type once per workspace
        // per schema event is what made a deploy's 223 events expensive even
        // when they queued nothing.
        let scope = raisin_storage::BranchScope::new(tenant_id, repo_id, branch);
        let node_types = match only_node_type {
            Some(name) => self
                .node_types
                .get(scope, name, None)
                .await?
                .into_iter()
                .collect::<Vec<_>>(),
            None => self.node_types.list(scope, None).await?,
        };

        let state = crate::compound_state::CompoundStateStore::new(self.db.clone());
        let mut queued = 0usize;
        let mut seen = std::collections::HashSet::new();

        for node_type in node_types {
            let Some(indexes) = node_type.compound_indexes.as_ref() else {
                continue;
            };
            if allowed_types
                .as_ref()
                .is_some_and(|allowed| !allowed.contains(&node_type.name))
            {
                tracing::trace!(
                    node_type = %node_type.name,
                    workspace = %workspace,
                    "compound index sweep: node type cannot live in this workspace; skipping its indexes"
                );
                continue;
            }
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
