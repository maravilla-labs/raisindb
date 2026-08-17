//! BLOCK-level translation operations inside a transaction.
//!
//! Block overlays are keyed by `{node_id}\0{block_uuid}\0{locale}`, a key space of
//! their own that the node-level operations in `read`/`write` never touch. Anything
//! that walks "this node's translations" — copying a node, most of all — has to
//! walk this one too, or the copy silently arrives with fewer translations than the
//! original and nothing reports it.
//!
//! Same read-your-writes contract as the node-level pair: writes land in the batch
//! AND in the read cache, so a later read in the same transaction sees them.

use raisin_error::Result;
use raisin_models::translations::LocaleOverlay;

use crate::transaction::types::is_tombstone;
use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle, keys};

/// `{tenant}\0{repo}\0{branch}\0{ws}\0block_trans\0{node_id}\0` — the node-scoped prefix.
fn node_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> String {
    format!("{tenant_id}\0{repo_id}\0{branch}\0{workspace}\0block_trans\0{node_id}\0")
}

fn tx_scope(tx: &RocksDBTransaction) -> Result<(String, String, String)> {
    let meta = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
    Ok((
        meta.tenant_id.to_string(),
        meta.repo_id.to_string(),
        meta.branch.as_ref().map(|b| b.to_string()).ok_or_else(|| {
            raisin_error::Error::Validation("Branch not set in transaction".into())
        })?,
    ))
}

/// Store a block overlay (batched, and visible to later reads in this transaction).
pub async fn store_block_translation(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
    block_uuid: &str,
    locale: &str,
    overlay: LocaleOverlay,
) -> Result<()> {
    let (tenant_id, repo_id, branch) = tx_scope(tx)?;
    let revision = tx.get_or_allocate_transaction_revision()?;

    let overlay_json = serde_json::to_vec(&overlay)
        .map_err(|e| raisin_error::Error::storage(format!("JSON serialization error: {}", e)))?;

    let mut key = format!(
        "{}{block_uuid}\0{locale}\0",
        node_prefix(&tenant_id, &repo_id, &branch, workspace, node_id)
    )
    .into_bytes();
    key.extend_from_slice(&keys::encode_descending_revision(&revision));

    {
        let mut batch = tx
            .batch
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        let cf_block = cf_handle(&tx.db, cf::BLOCK_TRANSLATIONS)?;
        batch.put_cf(cf_block, &key, overlay_json);
    }

    {
        let mut cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        cache.block_translations.insert(
            (
                workspace.to_string(),
                node_id.to_string(),
                block_uuid.to_string(),
                locale.to_string(),
            ),
            Some(overlay),
        );
    }

    tx.record_write(key)?;
    Ok(())
}

/// Read one block overlay, newest revision first, cache before storage.
pub async fn get_block_translation(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
    block_uuid: &str,
    locale: &str,
) -> Result<Option<LocaleOverlay>> {
    {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        let cache_key = (
            workspace.to_string(),
            node_id.to_string(),
            block_uuid.to_string(),
            locale.to_string(),
        );
        if let Some(cached) = cache.block_translations.get(&cache_key) {
            return Ok(cached.clone());
        }
    }

    let (tenant_id, repo_id, branch) = tx_scope(tx)?;
    let prefix = format!(
        "{}{block_uuid}\0{locale}\0",
        node_prefix(&tenant_id, &repo_id, &branch, workspace, node_id)
    );

    let cf_block = cf_handle(&tx.db, cf::BLOCK_TRANSLATIONS)?;
    for item in tx.db.prefix_iterator_cf(cf_block, &prefix) {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        if !key.starts_with(prefix.as_bytes()) {
            break;
        }
        if is_tombstone(&value) {
            continue;
        }
        let overlay: LocaleOverlay = serde_json::from_slice(&value).map_err(|e| {
            raisin_error::Error::storage(format!("JSON deserialization error: {}", e))
        })?;
        tx.record_read(key.to_vec())?;
        return Ok(Some(overlay));
    }

    Ok(None)
}

/// Every `(block_uuid, locale)` this node has a block overlay for.
///
/// One prefix scan. Uncommitted writes from this transaction are folded in from the
/// read cache, so a copy that stores overlays and then re-lists sees its own work.
pub async fn list_block_translations_for_node(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
) -> Result<Vec<(String, String)>> {
    let (tenant_id, repo_id, branch) = tx_scope(tx)?;
    let prefix = node_prefix(&tenant_id, &repo_id, &branch, workspace, node_id);

    let cf_block = cf_handle(&tx.db, cf::BLOCK_TRANSLATIONS)?;
    let mut found = std::collections::HashSet::new();

    for item in tx.db.prefix_iterator_cf(cf_block, &prefix) {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        if !key.starts_with(prefix.as_bytes()) {
            break;
        }
        if is_tombstone(&value) {
            continue;
        }
        let Some(suffix) = key.strip_prefix(prefix.as_bytes()) else {
            continue;
        };
        // {block_uuid}\0{locale}\0{~revision} — split on BYTES: the trailing
        // encoded revision is not valid UTF-8, so decoding the suffix as a whole
        // silently drops the entry.
        let mut parts = suffix.splitn(3, |b| *b == 0);
        let (Some(block_uuid), Some(locale)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(block_uuid), Ok(locale)) =
            (std::str::from_utf8(block_uuid), std::str::from_utf8(locale))
        else {
            continue;
        };
        // `orphaned` occupies the locale position but is a marker, not a locale.
        if locale == "orphaned" {
            continue;
        }
        found.insert((block_uuid.to_string(), locale.to_string()));
    }

    {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        for ((ws, nid, block_uuid, locale), overlay) in cache.block_translations.iter() {
            if ws == workspace && nid == node_id && overlay.is_some() {
                found.insert((block_uuid.clone(), locale.clone()));
            }
        }
    }

    Ok(found.into_iter().collect())
}
