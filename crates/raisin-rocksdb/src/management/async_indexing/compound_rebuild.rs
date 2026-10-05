//! Rebuild every compound index of a workspace (`REBUILD … compound`).
//!
//! Clears the workspace's compound keyspace and re-derives it through the ONE
//! build pass (`indexing::compound::build`, shared with the per-index build
//! job): every node's version as of the branch HEAD read after the clear —
//! the history FLOOR stamped as `built_through` — at that version's own
//! revision, plus every version above it. Reads below the floor are refused
//! by the planner and scan. Definitions (inheritance included) are read from
//! storage, never cache-first, and the same read gives the state hashes.
//!
//! The pass is prechecked BEFORE anything is cleared: disk headroom (for the
//! output too), and a read-only pass that refuses when a node cannot be
//! placed or indexed — the clear would otherwise delete entries the build
//! cannot write back. The NodeType indexes and the workspace's own indexes
//! (plan Phase 13e, and since 13f the built-in one on EVERY workspace) are
//! prechecked as two separate passes, so a node only the workspace pass
//! wants (any type: workspace indexes carry every node) cannot refuse the
//! rebuild of the NodeType indexes, nor the reverse; the type pass wants only
//! the types that carry an index.
//!
//! Every index rebuilt here is held under its keyspace lock
//! (`indexing::compound::keyspace::lock`, taken in name order) for the whole
//! rebuild, like every other builder: an automatic `compound_builds` link or
//! a per-index job of the same index queues behind it instead of writing into
//! the keyspace this clears.

use crate::indexing::compound::{build, defs, keyspace};
use crate::indexing::IndexCtx;
use crate::RocksDBStorage;
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_storage::compound::CompoundIndexState;
use raisin_storage::{BranchScope, NodeTypeRepository, RebuildStats};

use super::helpers::{clear_compound_indexes, get_current_revision};

/// Rebuild compound (multi-column) indexes for all nodes in a workspace.
pub(super) async fn rebuild_compound_indexes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    stats: &mut RebuildStats,
) -> Result<()> {
    tracing::info!("Rebuilding compound indexes");
    let db = storage.db();
    let scope = BranchScope::new(tenant_id, repo_id, branch);
    let ctx = IndexCtx::new(tenant_id, repo_id, branch, workspace);
    let fresh = defs::fresh_branch(db, &storage.node_types, scope, &[]).await?;
    // The two passes: the types that carry a NodeType index, and the
    // workspace's own indexes (every node carries them).
    let type_pass: build::Wanted = fresh
        .iter()
        .filter(|(_, d)| !d.compound.is_empty())
        .map(|(name, d)| (name.clone(), d.compound.clone()))
        .collect();
    let type_defs = declared_compound_indexes(storage, scope).await?;
    let workspace_defs =
        crate::indexing::compound::workspace_defs::current(db, tenant_id, repo_id, workspace)?
            .to_vec();
    let workspace_pass = build::Wanted::every_type(workspace_defs.clone());

    // Every index this may rebuild, locked in name order before anything is
    // read, so no other builder of this process writes into a keyspace this
    // clears (and two admin rebuilds take the locks in the same order).
    let mut names: Vec<&str> = type_defs
        .iter()
        .chain(&workspace_defs)
        .map(|d| d.name.as_str())
        .collect();
    names.sort_unstable();
    names.dedup();
    let mut keyspaces = Vec::with_capacity(names.len());
    for name in names {
        keyspaces.push(keyspace::lock(db, tenant_id, repo_id, branch, workspace, name).await);
    }

    // Precheck each pass on its own; a refused pass leaves its indexes as
    // they are and is reported after the other one is rebuilt.
    let head = get_current_revision(storage, tenant_id, repo_id, branch).await?;
    let mut wanted = build::Wanted::default();
    let mut declared: Vec<CompoundIndexDefinition> = Vec::new();
    let mut refused: Option<Error> = None;
    if !type_defs.is_empty() {
        match build::precheck(db, &ctx, &type_pass, &head) {
            Ok(()) => {
                wanted.by_type = type_pass.by_type;
                declared.extend(type_defs);
            }
            Err(e) => refused = Some(e),
        }
    }
    if !workspace_defs.is_empty() {
        match build::precheck(db, &ctx, &workspace_pass, &head) {
            Ok(()) => {
                wanted.every_type = workspace_pass.every_type;
                declared.extend(workspace_defs);
            }
            Err(e) => {
                refused.get_or_insert(e);
            }
        }
    }
    if declared.is_empty() {
        return refused.map_or(Ok(()), Err);
    }

    // Everything about to be rebuilt goes `Building` FIRST: the clear below
    // empties the keyspace, and the planner treats `Building` as unusable.
    // Each registration returns the build's ticket: a replicated write that
    // marks the index stale mid-rebuild (or another registration) clears it,
    // and the `Ready` below then loses its compare-and-set instead of
    // stamping over that mark.
    let state_store = crate::compound_state::CompoundStateStore::new(storage.db.clone());
    let mut started_under = Vec::with_capacity(declared.len());
    for definition in &declared {
        started_under.push(state_store.begin_rebuild(
            tenant_id,
            repo_id,
            branch,
            workspace,
            definition,
            raisin_hlc::HLC::new(0, 0),
        )?);
    }

    // The clear and re-derive insert below existing entries: hold the
    // (branch, COMPOUND) against run-collapse until the build is written.
    let _inserting = crate::management::cf_exclusion::enter_inserter_async(
        db,
        tenant_id,
        repo_id,
        branch,
        crate::cf::COMPOUND_INDEX,
    )
    .await;
    // Clear, THEN read the floor and scan: a write committed before the floor
    // read is in the scan, one after it writes its own entries over the
    // cleared keyspace. Both passes: the whole workspace keyspace (entries of
    // indexes nothing declares any more go too). One refused: only the
    // keyspaces being rebuilt.
    if refused.is_none() {
        clear_compound_indexes(storage, tenant_id, repo_id, branch, workspace).await?;
    } else {
        for definition in &declared {
            keyspace::clear(db, tenant_id, repo_id, branch, workspace, &definition.name)?;
        }
    }
    let floor = get_current_revision(storage, tenant_id, repo_id, branch).await?;
    let outcome = build::write(db, &ctx, &wanted, &floor)?;
    stats.items_processed += outcome.nodes as u64;
    if !outcome.complete() {
        // Left `Building` (unusable): nothing stamps `Ready` over a hole.
        build::refuse_unplaceable(&ctx, &outcome)?;
    }

    // Only NOW does the state flip to `Ready`, and only for the declaration
    // each index was actually built from. A rebuild that dies partway leaves
    // `Building` in place — which reads as unusable, not as ready-and-empty.
    for (definition, started) in declared.iter().zip(started_under) {
        let mut state = CompoundIndexState::ready(definition, floor);
        state.nodes_indexed = outcome.nodes as u64;
        if !state_store.complete_build(tenant_id, repo_id, branch, workspace, state, started)? {
            tracing::info!(
                index_name = %definition.name,
                "compound rebuild finished behind a newer stale mark; left NotBuilt"
            );
        }
    }
    refused.map_or(Ok(()), Err)
}

/// Every compound index declared by ANY NodeType on this branch.
///
/// Deliberately branch-wide rather than per-node-type: an index NAME addresses
/// a workspace-global keyspace, so "is this index built" is a question about
/// the branch, not about one type. Deduplicated by name, first declaration
/// wins (matching `engine/helpers.rs::load_all_compound_indexes`).
pub(crate) async fn declared_compound_indexes(
    storage: &RocksDBStorage,
    scope: BranchScope<'_>,
) -> Result<Vec<CompoundIndexDefinition>> {
    let node_types = storage.node_types.list(scope, None).await?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for node_type in node_types {
        for index in node_type.compound_indexes.unwrap_or_default() {
            // Never a NodeType's: a workspace keyspace name (plan Phase 13e).
            if CompoundIndexDefinition::is_workspace_index_name(&index.name) {
                continue;
            }
            if seen.insert(index.name.clone()) {
                out.push(index);
            }
        }
    }
    Ok(out)
}
