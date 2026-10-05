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

use crate::transaction::RocksDBTransaction;
use crate::translation_write::OverlayTarget;

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

    let key = {
        let mut batch = tx
            .batch
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        let target = OverlayTarget {
            tenant_id: &tenant_id,
            repo_id: &repo_id,
            branch: &branch,
            workspace,
            node_id,
            block_uuid: Some(block_uuid),
            locale,
        };
        crate::translation_write::stage_version(
            &tx.db,
            &mut batch,
            &target,
            Some(&overlay),
            &revision,
        )?
    };

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
    // The newest version decides; a tombstone there means deleted — this
    // reader used to skip it and return an OLDER live version.
    let Some(version) = crate::translation_read::read_block_version(
        &tx.db, &tenant_id, &repo_id, &branch, workspace, node_id, block_uuid, locale, None,
    )?
    else {
        return Ok(None);
    };
    tx.record_read(version.key)?;
    Ok(version.overlay)
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
    let mut found: std::collections::HashSet<(String, String)> =
        crate::translation_read::live_block_overlays(
            &tx.db, &tenant_id, &repo_id, &branch, workspace, node_id, None,
        )?
        .into_iter()
        .collect();

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
