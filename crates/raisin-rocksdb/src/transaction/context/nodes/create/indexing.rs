//! Property and reference indexing for node creation
//!
//! This module handles indexing of node properties and references to enable
//! efficient querying and backlink lookups. The property and unique halves
//! delegate to the shared delta writers (plan Phase 7).

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

use crate::repositories::nodes::PropertyWrite;
use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle, keys};

/// Index a node's properties (custom, pseudo-properties, IS_A / HAS_MIXIN
/// membership) and its geometries.
///
/// The PROPERTY_INDEX half is the ONE writer,
/// `crate::indexing::write_property_index_delta`: `write` says what the entries
/// are diffed against (a create, a full put against the replaced version, or a
/// proven predecessor whose unchanged entries are skipped) and whether the
/// revision is a reused in-place one. See `indexing::property_delta`.
#[allow(clippy::too_many_arguments)]
pub(in crate::transaction::context::nodes) fn index_node_properties(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
    write: PropertyWrite<'_>,
) -> Result<()> {
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    crate::indexing::write_property_index_delta(
        &mut batch,
        crate::indexing::PropertyIndexTarget::from_db(&tx.db)?,
        &crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace),
        write.baseline,
        node,
        revision,
        write.in_place,
    )?;

    // Index geometry properties in the spatial index (within same batch for
    // atomicity). Delegates to the ONE shared spatial writer in
    // `crate::indexing::spatial`, which the repository and replication apply paths
    // also call — so a new index type or a change to the cell derivation cannot land
    // on one path and be forgotten on the others.
    let ctx = crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace);
    let spatial_state = crate::spatial_state::SpatialStateStore::new(tx.db.clone());
    let policies =
        crate::indexing::NodeSpatialPolicies::from_local_state(&spatial_state, &ctx, node);
    let targets = crate::indexing::SpatialIndexTargets::from_db(tx.db.as_ref())?;

    crate::indexing::write_node_spatial_indexes(
        &mut batch, &targets, &ctx, node, revision, &policies,
    )?;

    // Create the state record on first write of a geometry property, in the SAME
    // batch. This is what preserves zero-opt-in automatic indexing: a brand-new
    // workspace is queryable immediately, with no admin action and no config.
    //
    // Ranges over the SAME walked path set the writer above used. Iterating
    // `node.properties` flat here left every nested geometry with entries but no
    // state record, and a missing record reads as `NotBuilt` — so the planner
    // refused the index and every nested query fell back to a full scan
    // permanently, with correct rows and no error to notice.
    for property_path in &crate::indexing::indexed_geometry_paths(&node.properties) {
        spatial_state.ensure_for_write(
            &mut batch,
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_path,
            policies.for_property(property_path),
            *revision,
        )?;
    }

    Ok(())
}

/// Tombstone old spatial index entries for a node's geometry properties.
///
/// Called before re-indexing during updates to prevent stale geohash entries.
///
/// `new_node` lets an unchanged geometry keep its entries: the re-write reproduces
/// byte-identical keys and values, so tombstoning and re-putting would be pure MVCC
/// churn on every update to any *other* property of the node.
pub(super) fn tombstone_spatial_properties(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    old_node: &Node,
    new_node: Option<&Node>,
    revision: &HLC,
) -> Result<()> {
    let ctx = crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace);
    let spatial_state = crate::spatial_state::SpatialStateStore::new(tx.db.clone());
    let policies =
        crate::indexing::NodeSpatialPolicies::from_local_state(&spatial_state, &ctx, old_node);
    let targets = crate::indexing::SpatialIndexTargets::from_db(tx.db.as_ref())?;

    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    crate::indexing::tombstone_superseded_spatial_indexes(
        &mut batch, &targets, &ctx, old_node, new_node, revision, &policies,
    )
}

/// Stage the UNIQUE_INDEX change from `old` (the version replaced, `None` on
/// create) to `node`: tombstone the claims that changed, put every new one
/// (claims are never skipped — see `write_unique_delta`). The
/// NodeTypes are read before the batch is locked (no await under the lock).
#[allow(clippy::too_many_arguments)]
pub(super) async fn write_unique_properties(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    old: Option<&Node>,
    node: &Node,
    revision: &HLC,
    in_place: bool,
) -> Result<()> {
    let old_props = match old {
        Some(old) => {
            tx.node_repo
                .unique_property_names(tenant_id, repo_id, branch, &old.node_type)
                .await?
        }
        None => Vec::new(),
    };
    let new_props = tx
        .node_repo
        .unique_property_names(tenant_id, repo_id, branch, &node.node_type)
        .await?;
    if old_props.is_empty() && new_props.is_empty() {
        return Ok(());
    }

    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
    crate::repositories::nodes::write_unique_delta(
        &mut batch,
        &tx.db,
        &crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace),
        old.map(|node| crate::repositories::nodes::UniqueSide {
            node,
            properties: &old_props,
        }),
        crate::repositories::nodes::UniqueSide {
            node,
            properties: &new_props,
        },
        revision,
        in_place,
    )
}

/// Stage a node's COMPOUND_INDEX write within the transaction batch, through
/// the one writer (`indexing::compound`) and against the SAME baseline as the
/// property index: the old tuple derived from the version replaced and
/// tombstoned at the revision, unchanged tuples skipped only under a proven
/// predecessor (the commit re-check corrects a stale one).
///
/// The transaction (SQL DML) path historically wrote no compound entries at
/// all, then wrote only the type's OWN declarations (not inherited ones) and
/// tombstoned by a workspace-wide scan. Definitions are resolved before the
/// batch lock (no await under it).
#[allow(clippy::too_many_arguments)]
pub(in crate::transaction::context::nodes) async fn write_compound_indexes(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    baseline: crate::indexing::Baseline<'_>,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    let ctx = crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace);
    let types = crate::indexing::compound::types_of(&baseline, node);
    let defs = tx.node_repo.index_defs(&ctx, &types).await?;
    if !crate::indexing::compound::writes_compound(&tx.db, &ctx, &defs)? {
        return Ok(());
    }
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
    crate::indexing::compound::write_compound_delta(
        &mut batch, &tx.db, &ctx, &defs, baseline, node, revision,
    )?;
    Ok(())
}

/// Index references for a node
///
/// Creates both forward and reverse reference indexes:
/// - Forward: source_node_id + property_path -> reference
/// - Reverse: target_workspace + target_path -> source_node_id + property_path
///
/// These indexes enable efficient reference queries and backlink lookups.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `node` - The node whose references to index
/// * `revision` - The HLC revision for versioning
///
/// # Errors
///
/// Returns error if:
/// - Lock is poisoned
/// - Serialization fails
#[allow(clippy::too_many_arguments)]
pub(super) fn index_node_references(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    let cf_reference = cf_handle(&tx.db, cf::REFERENCE_INDEX)?;

    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    crate::repositories::nodes::add_reference_index_entries(
        &mut batch,
        cf_reference,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node,
        revision,
    )
}

/// Tombstone stale reference-index entries on update.
///
/// Every reference of `old_node` that `new_node` no longer carries gets a
/// TOMBSTONE (forward + reverse, both publish variants) at the new revision —
/// otherwise `REFERENCES()`/backlinks keep matching the node forever.
pub(super) fn tombstone_stale_reference_indexes(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    old_node: &Node,
    new_node: &Node,
    revision: &HLC,
) -> Result<()> {
    let cf_reference = cf_handle(&tx.db, cf::REFERENCE_INDEX)?;

    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    crate::repositories::add_stale_reference_tombstones(
        &mut batch,
        cf_reference,
        tenant_id,
        repo_id,
        branch,
        workspace,
        old_node,
        new_node,
        revision,
    );

    Ok(())
}
