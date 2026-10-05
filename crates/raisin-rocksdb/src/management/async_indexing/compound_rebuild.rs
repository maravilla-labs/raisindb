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
//! The pass is prechecked BEFORE anything is cleared: disk headroom, and a
//! read-only pass that refuses when a node cannot be placed — the clear would
//! otherwise delete entries the build cannot write back.

use crate::indexing::compound::{build, defs};
use crate::indexing::IndexCtx;
use crate::RocksDBStorage;
use raisin_error::Result;
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
    let scope = BranchScope::new(tenant_id, repo_id, branch);
    let ctx = IndexCtx::new(tenant_id, repo_id, branch, workspace);
    let fresh = defs::fresh_branch(storage.db(), &storage.node_types, scope, &[]).await?;
    let wanted: build::Wanted = fresh
        .iter()
        .map(|(name, d)| (name.clone(), d.compound.clone()))
        .collect();
    build::precheck(
        storage.db(),
        &ctx,
        &wanted,
        &get_current_revision(storage, tenant_id, repo_id, branch).await?,
    )?;

    // Everything declared on this branch is about to be rebuilt, so mark it
    // all `Building` FIRST: the clear below empties the keyspace, and the
    // planner treats `Building` as unusable. `begin_rebuild` also returns the
    // generation each build runs under: a replicated write that marks the
    // index stale mid-rebuild advances it, and the `Ready` below then loses
    // its compare-and-set instead of stamping over that mark.
    let state_store = crate::compound_state::CompoundStateStore::new(storage.db.clone());
    let declared = declared_compound_indexes(storage, scope).await?;
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

    // Clear, THEN read the floor and scan: a write committed before the floor
    // read is in the scan, one after it writes its own entries over the
    // cleared keyspace.
    clear_compound_indexes(storage, tenant_id, repo_id, branch, workspace).await?;
    let floor = get_current_revision(storage, tenant_id, repo_id, branch).await?;
    let outcome = build::write(storage.db(), &ctx, &wanted, &floor)?;
    stats.items_processed += outcome.nodes as u64;
    if outcome.unplaceable > 0 {
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
    Ok(())
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
            if seen.insert(index.name.clone()) {
                out.push(index);
            }
        }
    }
    Ok(out)
}
