//! Partial property updates, applied at commit against the stored state.
//!
//! `patch_node_properties` does NOT read-merge-stage when it is called: it
//! records the patch, and the commit applies it ([`apply_deferred_patches`])
//! while holding the nodes' commit lock (`indexing::node_lock`, the lock every
//! commit of a node already takes around its write). The node is read there,
//! the patch merged into what is stored at that moment and the result staged
//! through `put_node`, so the revision is allocated after every earlier writer
//! of the node landed and nothing it wrote is overwritten with stale values.
//!
//! Callers take no lock and check no version; per property the later write
//! wins. A node this transaction already wrote is patched immediately (its
//! staged version is what the commit writes anyway).

use raisin_error::{Error, Result};
use raisin_storage::node_patch::{PatchTarget, PropertyPatch};

use crate::transaction::RocksDBTransaction;

/// Record `patch` for `target`; returns the node id.
pub async fn patch_node_properties(
    tx: &RocksDBTransaction,
    workspace: &str,
    target: &PatchTarget,
    patch: PropertyPatch,
) -> Result<String> {
    // Resolve the id now (it keys the commit lock). The read also applies
    // the caller's read rights: a node they cannot see is not found.
    let node = match target {
        PatchTarget::Id(id) => super::read::get_node(tx, workspace, id).await?,
        PatchTarget::Path(path) => super::read::get_node_by_path(tx, workspace, path).await?,
    }
    .ok_or_else(|| Error::NotFound(format!("Node not found: {}", describe(target))))?;

    let key = (workspace.to_string(), node.id.clone());
    let written_here = {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| Error::storage(format!("Lock error: {}", e)))?;
        cache.nodes.contains_key(&key) || cache.moved_nodes.contains_key(&key)
    };
    if written_here {
        let mut node = node;
        patch.apply(&mut node.properties);
        super::put_node(tx, workspace, &node).await?;
        return Ok(key.1);
    }

    let mut cache = tx
        .read_cache
        .lock()
        .map_err(|e| Error::storage(format!("Lock error: {}", e)))?;
    match cache.deferred_patches.iter_mut().find(|(k, _)| *k == key) {
        Some((_, earlier)) => earlier.then(patch),
        None => cache.deferred_patches.push((key.clone(), patch)),
    }
    Ok(key.1)
}

fn describe(target: &PatchTarget) -> &str {
    match target {
        PatchTarget::Id(id) => id,
        PatchTarget::Path(path) => path,
    }
}

/// At commit: lock every node the transaction writes (the patched ones
/// included, one sorted acquisition), then read each patched node as stored
/// NOW, merge its patch and stage it. Returns the held guard — the commit
/// keeps it for its write instead of locking again — or `None` when there is
/// nothing deferred.
pub(in crate::transaction) async fn apply_deferred_patches(
    tx: &RocksDBTransaction,
) -> Result<Option<crate::indexing::NodeWriteGuard>> {
    let patches = {
        let mut cache = tx
            .read_cache
            .lock()
            .map_err(|e| Error::storage(format!("Lock error: {}", e)))?;
        std::mem::take(&mut cache.deferred_patches)
    };
    if patches.is_empty() {
        return Ok(None);
    }
    let (tenant_id, repo_id, branch) = super::create::metadata_of(tx)?;
    let ids = {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| Error::storage(format!("Lock error: {}", e)))?;
        let mut ids: Vec<String> = cache
            .nodes
            .keys()
            .chain(cache.moved_nodes.keys())
            .chain(cache.delta_checks.keys())
            .map(|(_, id)| id.clone())
            .collect();
        ids.extend(patches.iter().map(|((_, id), _)| id.clone()));
        ids
    };
    let guard = crate::indexing::lock_nodes(
        &tx.db,
        &tenant_id,
        &repo_id,
        &branch,
        ids.iter().map(String::as_str),
    )
    .await;

    for ((workspace, node_id), patch) in patches {
        let mut node = super::read::get_node(tx, &workspace, &node_id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node not found: {}", node_id)))?;
        patch.apply(&mut node.properties);
        super::put_node(tx, &workspace, &node).await?;
    }
    Ok(Some(guard))
}
