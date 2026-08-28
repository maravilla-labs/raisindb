//! `add_node` - Optimized path for creating new nodes in a transaction
//!
//! Unlike `put_node`, this only handles CREATE (no update path).

use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalContext;

use crate::transaction::RocksDBTransaction;

use super::super::{
    cache, coercion, indexing, metadata, ordering, references, storage, tracking, validation,
};
use super::rls;

/// Create a new node in the transaction (optimized for new nodes)
///
/// This is an optimized version of `put_node` for new nodes only.
/// It validates as CREATE and skips existence checks.
///
/// # Fast Path
///
/// Unlike `put_node`, this method:
/// - Only validates as CREATE (no existence check)
/// - Appends to end of ordered children (no existence check)
/// - Always tracks as Added operation
///
/// # Read-Your-Writes
///
/// When creating initial_structure children, the parent node may have been
/// created earlier in this same transaction and only exists in the write batch.
/// We check the transaction's read cache first for read-your-writes semantics.
pub async fn add_node(tx: &RocksDBTransaction, workspace: &str, node: &Node) -> Result<()> {
    // 1. Normalize parent field from path before saving
    let mut normalized_node = metadata::normalize_parent(node);

    // 2. Resolve path-based references
    references::resolve_references(tx, &mut normalized_node.properties, workspace).await?;

    // 2a. Coerce LocationField properties ({lat, lng} -> GeoJSON Point)
    coercion::coerce_location_fields(tx, &mut normalized_node).await?;

    // 3. Extract metadata (tenant, repo, branch)
    let (tenant_id, repo_id, branch) = metadata::extract_metadata(tx)?;

    // 3b. Stamp authorship for this CREATE. Mirrors put_node so every create
    // path records who made the node. Actor: auth context → raw actor →
    // "anonymous". Don't overwrite an explicitly-supplied created_by.
    //
    // `principal_id`, like put_node: a flow, agent or trigger runs under
    // `AuthContext::system()`, and taking `actor_id()` here would record the
    // word "system" for every one of them.
    let actor = tx
        .get_auth_context()?
        .and_then(|a| a.principal_id())
        .or(tx.get_actor()?)
        .unwrap_or_else(|| "anonymous".to_string());
    normalized_node.updated_by = Some(actor.clone());
    if normalized_node.created_by.is_none() {
        normalized_node.created_by = Some(actor);
    }

    // Stamp timestamps for this CREATE at the same layer as authorship (see
    // Node::ensure_write_timestamps for why every write path must do this).
    normalized_node.ensure_write_timestamps();

    // 3a. Check CREATE permission
    rls::check_create_permission(tx, &normalized_node, workspace).await?;

    // 3c. Reserve the path against concurrent creators BEFORE the existence
    // check below. The existence check alone is a TOCTOU race: two concurrent
    // transactions can both see "no node at path" and both commit, yielding two
    // physical rows at one path. The reservation is held until this
    // transaction's batch is durably written (or it rolls back / is dropped),
    // and reserve-then-check ordering guarantees the loser either conflicts
    // here or sees the winner's committed row in the check below.
    tx.reserve_create_path(
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node.path,
    )?;

    // 4. Check for path conflict in transaction cache (read-your-writes)
    let cached_existing =
        super::super::super::read::get_node_by_path(tx, workspace, &normalized_node.path).await?;
    if let Some(existing_node) = cached_existing {
        tracing::warn!(
            "ADD_NODE: Path conflict detected! Path '{}' already exists with id='{}', refusing to create duplicate with id='{}'",
            normalized_node.path,
            existing_node.id,
            normalized_node.id
        );
        return Err(raisin_error::Error::Conflict(format!(
            "Node with path '{}' already exists (id={})",
            normalized_node.path, existing_node.id
        )));
    }

    // 5. Validate as new node
    validation::validate_create(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        false, // skip parent validation
    )
    .await?;

    // 5a. Schema validation
    if tx.is_validate_schema_enabled() {
        let validator = tx.create_validator();
        validator.validate_node(workspace, &normalized_node).await?;
    }

    // 5b. Check unique property constraints
    validation::check_unique_constraints(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
    )
    .await?;

    // 6. Get or allocate the single transaction HLC
    let revision = tx.get_or_allocate_transaction_revision()?;

    tracing::info!(
        "TXN add_node: node_id={}, path={}, revision={}",
        normalized_node.id,
        normalized_node.path,
        revision
    );

    // 6a. Seal any plaintext sitting in a field declared `encrypted: true`,
    // replacing it with a `secret://` reference.
    //
    // This slot is load-bearing on both sides: the validator above still saw
    // PLAINTEXT (so its constraints mean something), and everything below —
    // the read cache, the node blob, and above all the property / unique /
    // compound index writers — must only ever see the reference. An index
    // entry keyed on a plaintext password turns `properties->>'password'` into
    // an oracle, permanently.
    //
    // The vaulter is the node repository's, shared with the repository write
    // path so the two cannot drift; the memo is the transaction's, so two
    // writes of one node under one HLC mint one secret version. See
    // `crate::vaulting`.
    // `updated_by` was stamped from the same actor a few steps above; reading
    // it back avoids cloning the actor string on every write.
    let vault_actor = normalized_node
        .updated_by
        .clone()
        .unwrap_or_else(|| "anonymous".to_string());
    let minted_secrets = tx
        .node_repo
        .vault_encrypted_fields(
            crate::vaulting::VaultScope {
                tenant_id: &tenant_id,
                repo_id: &repo_id,
                branch: &branch,
                actor: &vault_actor,
            },
            &mut normalized_node,
            Some(&tx.vaulted_secrets),
        )
        .await?;

    // 6b. Replicate the sealed bytes BEFORE the node that references them.
    // Same `(tenant, repo)` lane as the node operation this transaction will
    // capture at commit, and earlier in it — which is what makes a peer's causal
    // buffer hold the node snapshot until the secret has landed. See
    // `replication/operation_capture/secret_ops.rs`.
    tx.node_repo
        .capture_secret_versions(&tenant_id, &repo_id, &branch, &vault_actor, &minted_secrets)
        .await;

    // 6. Update read cache for read-your-writes semantics
    cache::update_read_cache(tx, workspace, &normalized_node, None)?;

    // 7. Write node to batch
    let node_key = storage::write_node_to_batch(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        &revision,
    )?;
    tx.record_write(node_key)?;

    // 8. Write path index (no tombstone for add_node)
    storage::write_path_index(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node.path,
        &normalized_node.id,
        &revision,
        None,
    )?;

    // 9. Index all properties
    indexing::index_node_properties(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        &revision,
    )?;

    // 10. Index references
    indexing::index_node_references(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        &revision,
    )?;

    // 10a. Index unique properties
    indexing::index_unique_properties(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        &revision,
    )
    .await?;

    // 10b. Index compound indexes (multi-column). Mirrors the repository path so
    // SQL-created nodes are visible to compound-index scans.
    indexing::index_compound_indexes(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        &revision,
    )
    .await?;

    // 11. Add ORDERED_CHILDREN index entry (FAST PATH)
    let parent_id = ordering::lookup_parent_id(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
    )
    .await?;

    if let Some(parent_id_val) = parent_id {
        let order_label = ordering::add_ordered_child_fast(
            tx,
            &tenant_id,
            &repo_id,
            &branch,
            workspace,
            &parent_id_val,
            &normalized_node,
            &revision,
        )?;

        normalized_node.order_key = order_label;
    }

    // 12. Track creation
    tracking::track_create(tx, workspace, &normalized_node, revision)?;

    Ok(())
}
