//! The compound-index build sweep and its job producer (moved out of
//! `storage/jobs/mod.rs`; plan Phase 13e added the workspace's own indexes).

use super::super::RocksDBStorage;
use raisin_error::Result;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;

/// What a sweep does with a build the `compound_builds` repair owns (plan
/// Phase 13f: a built-in workspace index, an older-format record).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AutomaticBuilds {
    /// Ask for that branch's targeted link (an event, a cold drain).
    Request,
    /// Nothing: the chain after start covers it (the boot sweep).
    Defer,
}

impl RocksDBStorage {
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
    /// - a branch fork whose source index was not `Ready` (a fork inherits
    ///   only `Ready` records, `compound_state::fork`).
    ///
    /// The BUILT-IN workspace indexes and indexes whose record is merely an
    /// OLDER FORMAT are not queued as per-index jobs: they belong to the
    /// `compound_builds` repair (plan Phase 13f), whose targeted link this
    /// asks for (older formats only while `RAISIN_COMPOUND_FORMAT_REBUILD` is
    /// not `0`, see `compound_state::format_rebuild_enabled`).
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
        self.sweep_compound_index_builds_inner(
            tenant_id,
            repo_id,
            branch,
            workspace,
            None,
            AutomaticBuilds::Request,
        )
        .await
    }

    /// [`Self::sweep_compound_index_builds`] for the BOOT sweep: the builds
    /// the `compound_builds` repair owns (built-in indexes, older formats —
    /// plan Phase 13f) are left to its chain, which starts after the job
    /// system and walks one branch at a time. Requesting them here would
    /// queue a link for every branch at once.
    pub async fn sweep_compound_index_builds_at_boot_for(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<usize> {
        self.sweep_compound_index_builds_inner(
            tenant_id,
            repo_id,
            branch,
            workspace,
            None,
            AutomaticBuilds::Defer,
        )
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
            AutomaticBuilds::Request,
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
        automatic: AutomaticBuilds,
    ) -> Result<usize> {
        use raisin_storage::NodeTypeRepository;

        // A compound index is declared by ONE node type (subtypes inherit it,
        // and the build indexes every type that carries it). A workspace that
        // cannot hold the declaring type is treated as having nothing to
        // index; one holding only a subtype stays NotBuilt — fail closed, a
        // scan — until the declaring type is allowed there.
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
                // A workspace keyspace name is never a NodeType's.
                if CompoundIndexDefinition::is_workspace_index_name(&definition.name) {
                    continue;
                }
                // One keyspace per index NAME, so one build per name — even if
                // two NodeTypes declare it. Which is itself a misconfiguration,
                // warned about at upsert.
                if !seen.insert(definition.name.clone()) {
                    continue;
                }
                if self
                    .queue_if_unusable(
                        &state,
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &node_type.name,
                        definition,
                        automatic,
                    )
                    .await?
                {
                    queued += 1;
                }
            }
        }

        // The workspace's OWN indexes (plan Phase 13e) — every node of the
        // workspace carries them, so containment does not apply. Read through
        // the writers' reader, which also fails closed any record a changed
        // declaration no longer vouches for. A per-type sweep has nothing to
        // say about them.
        if only_node_type.is_none() {
            let declared = crate::indexing::compound::workspace_defs::current(
                &self.db, tenant_id, repo_id, workspace,
            )?;
            for definition in declared.iter() {
                if self
                    .queue_if_unusable(
                        &state,
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &format!("workspace:{workspace}"),
                        definition,
                        automatic,
                    )
                    .await?
                {
                    queued += 1;
                }
            }
        }
        Ok(queued)
    }
}
