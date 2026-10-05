//! The compound build itself (split out of `compound_index.rs`): shared by the
//! per-index build job and the automatic `compound_builds` chain (plan Phase
//! 13f), under the process-wide keyspace lock
//! (`indexing::compound::keyspace`).

use super::CompoundIndexJobHandler;
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;

impl CompoundIndexJobHandler {
    /// Build one compound index: `owner` is the declaring (or any carrying)
    /// node type's name, or `workspace:{name}` for a workspace-owned index
    /// (told apart by the `@` keyspace name). `max_bytes_per_sec` paces the
    /// writing pass (0: unlimited). Returns whether the index ends `Ready`.
    ///
    /// Takes the keyspace lock first, so two builders of one index on this
    /// node queue rather than interleave; one that waited finds the index
    /// `Ready` and returns without rebuilding it.
    ///
    /// A mark that arrives DURING a build (a replicated write whose definitions
    /// were cold, a merge) makes the final `Ready` lose its compare-and-set; the
    /// build then runs again, a bounded number of times — the request that
    /// marked it was usually what queued this very job, so waiting for another
    /// trigger would leave the index scan-only.
    #[allow(clippy::too_many_arguments)]
    pub async fn build_index(
        &self,
        label: &str,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        owner: &str,
        index_name: &str,
        max_bytes_per_sec: u64,
    ) -> Result<bool> {
        let _keyspace = crate::indexing::compound::keyspace::lock(
            &self.db, tenant_id, repo_id, branch, workspace, index_name,
        )
        .await;
        const ATTEMPTS: usize = 3;
        for attempt in 1..=ATTEMPTS {
            let (index_def, wanted) = self
                .declaration(tenant_id, repo_id, branch, workspace, owner, index_name)
                .await?;
            if attempt == 1 && self.ready(tenant_id, repo_id, branch, workspace, &index_def) {
                tracing::debug!(
                    index = %index_name,
                    "compound index is already Ready (built while this build waited)"
                );
                return Ok(true);
            }
            if self
                .build_once(
                    label,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &index_def,
                    wanted,
                    max_bytes_per_sec,
                )
                .await?
            {
                return Ok(true);
            }
            tracing::info!(
                job = %label,
                index = %index_name,
                attempt,
                "Compound index build finished behind a newer stale mark; building again"
            );
        }
        Ok(false)
    }

    fn ready(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        definition: &CompoundIndexDefinition,
    ) -> bool {
        use raisin_storage::compound::CompoundStateSource;
        crate::compound_state::CompoundStateStore::new(self.db.clone())
            .compound_availability(tenant_id, repo_id, branch, workspace, definition)
            .is_ready()
    }

    /// The definitions READ FROM STORAGE (never cache-first, so a cache
    /// lagging a declaration change cannot hand the build the old columns),
    /// inheritance included: every type whose resolved declarations carry
    /// this index NAME writes into its keyspace. The same answer warms the
    /// cache the replication apply path maintains the index from.
    async fn declaration(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type_name: &str,
        index_name: &str,
    ) -> Result<(
        CompoundIndexDefinition,
        crate::indexing::compound::build::Wanted,
    )> {
        use crate::indexing::compound::build;
        if CompoundIndexDefinition::is_workspace_index_name(index_name) {
            // A WORKSPACE-owned index (plan Phase 13e): declared on the
            // workspace record (or built in, Phase 13f), carried by every node
            // of the workspace.
            let declared = crate::indexing::compound::workspace_defs::current(
                &self.db, tenant_id, repo_id, workspace,
            )?;
            let index_def = declared
                .iter()
                .find(|idx| idx.name == index_name)
                .cloned()
                .ok_or_else(|| {
                    Error::NotFound(format!(
                        "Compound index '{index_name}' is not declared by workspace '{workspace}'"
                    ))
                })?;
            return Ok((
                index_def.clone(),
                build::Wanted::every_type(vec![index_def]),
            ));
        }
        let scope = raisin_storage::BranchScope::new(tenant_id, repo_id, branch);
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
        Ok((index_def, wanted))
    }

    /// One build pass; `Ok(true)` when it stamped `Ready`. The scans run on a
    /// blocking thread (they stream a whole workspace, and a paced pass
    /// sleeps between batches).
    #[allow(clippy::too_many_arguments)]
    async fn build_once(
        &self,
        label: &str,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        index_def: &CompoundIndexDefinition,
        wanted: crate::indexing::compound::build::Wanted,
        max_bytes_per_sec: u64,
    ) -> Result<bool> {
        use crate::indexing::compound::{build, keyspace};
        let index_name = index_def.name.as_str();
        let scope = Scope {
            db: self.db.clone(),
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
        };

        // Refuse BEFORE the clear: a node the build cannot place (or index)
        // would lose its entries to the clear and never get them back, and a
        // volume that cannot take the build's output must not be filled.
        let head = self.branch_head(tenant_id, repo_id, branch)?;
        scope
            .blocking(wanted.clone(), move |db, ctx, wanted| {
                build::precheck(db, ctx, wanted, &head)
            })
            .await?;

        // Register the build BEFORE clearing or reading any node: a mark that
        // arrives after this point clears the build's ticket and makes the
        // final `Ready` lose — see `compound_state::marker` / `build_cas`.
        let state_store = crate::compound_state::CompoundStateStore::new(self.db.clone());
        let started_under = state_store.begin_rebuild(
            tenant_id,
            repo_id,
            branch,
            workspace,
            index_def,
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
        keyspace::clear(&self.db, tenant_id, repo_id, branch, workspace, index_name)?;
        let floor = self.branch_head(tenant_id, repo_id, branch)?;
        let outcome = scope
            .blocking(wanted, move |db, ctx, wanted| {
                build::write_paced(db, ctx, wanted, &floor, max_bytes_per_sec)
            })
            .await?;
        if !outcome.complete() {
            state_store.mark_not_built(tenant_id, repo_id, branch, workspace, index_name)?;
            build::refuse_unplaceable(
                &crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace),
                &outcome,
            )?;
        }

        // Stamp the state record LAST, and only on success: this flips the
        // planner's fail-closed gate open — for reads at or above the floor.
        // Compare-and-set against marks that arrived during the build.
        let mut state = raisin_storage::compound::CompoundIndexState::ready(index_def, floor);
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
                job = %label,
                index = %index_name,
                nodes = outcome.nodes,
                entries = outcome.entries,
                floor = %floor,
                "Compound index build completed"
            );
        }
        Ok(stamped)
    }
}

/// Owned scope of one build, for its blocking passes.
struct Scope {
    db: std::sync::Arc<rocksdb::DB>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    workspace: String,
}

impl Scope {
    async fn blocking<T: Send + 'static>(
        &self,
        wanted: crate::indexing::compound::build::Wanted,
        pass: impl FnOnce(
                &rocksdb::DB,
                &crate::indexing::IndexCtx<'_>,
                &crate::indexing::compound::build::Wanted,
            ) -> Result<T>
            + Send
            + 'static,
    ) -> Result<T> {
        let (db, t, r, b, w) = (
            self.db.clone(),
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            self.workspace.clone(),
        );
        tokio::task::spawn_blocking(move || {
            let ctx = crate::indexing::IndexCtx::new(&t, &r, &b, &w);
            pass(&db, &ctx, &wanted)
        })
        .await
        .map_err(|e| Error::storage(format!("compound build task failed: {e}")))?
    }
}
