//! The transaction's append-label minter.
//!
//! A transaction's appends are not committed until it commits, so the parent's
//! last label is read from the transaction's own `last_order_labels` first and
//! from the database only when this transaction has not appended there yet.
//! Every transactional append — `add_ordered_child`, `add_ordered_child_fast`
//! and `move_node_tree` — mints through [`next_append_label_tx`] and records
//! what it minted through [`record_appended_label_tx`]. The move used to read
//! the database alone: a create and a move into one parent in one transaction
//! minted byte-identical labels (same fractional part, same transaction HLC
//! suffix), and a later reorder between the two failed with "First label must
//! be less than second label".

use raisin_error::Result;
use raisin_hlc::HLC;

use crate::transaction::RocksDBTransaction;

/// The label for appending a new last child under `parent_id` in `tx`.
pub(crate) fn next_append_label_tx(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
    revision: &HLC,
) -> Result<String> {
    let cached = {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        cache
            .last_order_labels
            .get(&(workspace.to_string(), parent_id.to_string()))
            .cloned()
    };
    let last = match cached {
        Some(label) => Some(label),
        None => tx
            .node_repo
            .get_last_order_label(tenant_id, repo_id, branch, workspace, parent_id)?,
    };
    Ok(crate::repositories::nodes::mint_append_label(
        last.as_deref(),
        parent_id,
        revision,
    ))
}

/// Remember `label` as `parent_id`'s last label for the rest of `tx`, so the
/// next append in this transaction mints after it.
pub(crate) fn record_appended_label_tx(
    tx: &RocksDBTransaction,
    workspace: &str,
    parent_id: &str,
    label: &str,
) -> Result<()> {
    let mut cache = tx
        .read_cache
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
    cache.last_order_labels.insert(
        (workspace.to_string(), parent_id.to_string()),
        label.to_string(),
    );
    Ok(())
}
