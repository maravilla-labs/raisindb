//! Queueing one compound index build (the sweep's producer).

use super::super::RocksDBStorage;
use raisin_error::Result;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;

impl RocksDBStorage {
    /// Queue a build of `definition` unless it is usable, or owned by the
    /// `compound_builds` repair (a built-in index, an older state format —
    /// plan Phase 13f), whose targeted link it asks for instead (`automatic`).
    /// Returns whether it queued a per-index build.
    /// `owner` is the declaring node type's name, or `workspace:{name}` for a
    /// workspace-owned index (informational: the build job tells the two
    /// apart by the index's keyspace name).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn queue_if_unusable(
        &self,
        state: &crate::compound_state::CompoundStateStore,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        owner: &str,
        definition: &CompoundIndexDefinition,
        automatic: super::compound_sweep::AutomaticBuilds,
    ) -> Result<bool> {
        use raisin_storage::compound::CompoundStateSource;
        let availability =
            state.compound_availability(tenant_id, repo_id, branch, workspace, definition);
        if availability.is_ready() {
            return Ok(false);
        }
        let format_upgrade = state
            .get(tenant_id, repo_id, branch, workspace, &definition.name)
            .ok()
            .flatten()
            .is_some_and(|record| record.is_format_upgrade());
        let builtin =
            raisin_models::workspace::builtin_indexes::is_builtin_stored_name(&definition.name);
        if format_upgrade && !crate::compound_state::format_rebuild_enabled() {
            // Unusable only because the record format moved on, and the
            // automatic format rebuild is switched off: an admin decision
            // (`REBUILD … compound`).
            tracing::debug!(
                index = %definition.name,
                workspace = %workspace,
                "compound index has an older state format; left to an admin rebuild"
            );
            return Ok(false);
        }
        if format_upgrade || builtin {
            // The `compound_builds` repair owns these (plan Phase 13f): one
            // branch at a time, paced — never a job per index per branch at
            // once.
            if automatic == super::compound_sweep::AutomaticBuilds::Request {
                crate::management::async_indexing::repair::request_compound_builds(
                    &self.db, tenant_id, repo_id, branch,
                );
            }
            return Ok(false);
        }
        tracing::info!(
            index = %definition.name,
            owner = %owner,
            workspace = %workspace,
            detail = %availability.explain_reason(),
            "compound index is not usable; queueing a build"
        );
        self.queue_compound_index_build(
            tenant_id,
            repo_id,
            branch,
            workspace,
            owner,
            &definition.name,
        )
        .await?;
        Ok(true)
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
}
