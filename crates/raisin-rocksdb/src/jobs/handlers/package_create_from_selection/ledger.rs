//! The engine's own record of the blobs a package build wrote.
//!
//! Anything that later reads or deletes an artifact by key — the eager delete
//! when a build record goes away, `packages_deleteArtifacts`,
//! `resources_readRange` — must not take that key from node properties:
//! those are writable content, and a forged key would turn "delete this
//! run's archive" into "delete any blob". So every committed artifact is
//! entered here, keyed by tenant, repository and storage key, naming the
//! record node that owns it; and only keys found here, for that owner, are
//! ever read or deleted.
//!
//! Layout, beside the other per-tenant namespaces in [`cf::REGISTRY`]:
//! `{tenant}\0pkgartifacts\0{repo}\0{storage_key}` → JSON [`LedgerEntry`].

use crate::{cf, cf_handle, keys::KeyBuilder, RocksDBStorage};
use raisin_binary::DynBinaryStorage;
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{BranchRepository, NodeRepository, Storage, StorageScope};
use serde::{Deserialize, Serialize};

const NAMESPACE: &str = "pkgartifacts";

/// Who owns one artifact blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub job_id: String,
    pub branch: String,
    pub workspace: String,
    /// The record node's id — stable across moves, unlike its path.
    pub node_id: String,
    pub path: String,
}

fn key(tenant: &str, repo: &str, storage_key: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant)
        .push(NAMESPACE)
        .push(repo)
        .push(storage_key)
        .build()
}

fn repo_prefix(tenant: &str, repo: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant)
        .push(NAMESPACE)
        .push(repo)
        .build_prefix()
}

fn storage_err(e: impl std::fmt::Display) -> Error {
    Error::storage(format!("artifact ledger: {e}"))
}

/// Enter an artifact. Refuses a key outside the tenant/repo artifact
/// namespace, so the ledger can never vouch for a foreign blob.
pub fn record(
    storage: &RocksDBStorage,
    tenant: &str,
    repo: &str,
    storage_key: &str,
    entry: &LedgerEntry,
) -> Result<()> {
    // The key is built by joining segments with `\0`; a value carrying one
    // could be crafted to land in, or shadow, another tenant's or repo's
    // entry. Refuse rather than escape.
    for part in [
        tenant,
        repo,
        storage_key,
        &entry.job_id,
        &entry.branch,
        &entry.workspace,
        &entry.node_id,
        &entry.path,
    ] {
        if part.is_empty() || part.contains('\0') {
            return Err(Error::Validation(
                "artifact ledger: empty or NUL-bearing segment refused".to_string(),
            ));
        }
    }
    if !raisin_core::is_artifact_key_of(tenant, repo, storage_key) {
        return Err(Error::Validation(format!(
            "artifact key '{storage_key}' is outside {tenant}/{repo}"
        )));
    }
    // An entry never changes owner: re-recording the same owner is a no-op,
    // a different owner is refused.
    if let Some(existing) = lookup(storage, tenant, repo, storage_key)? {
        if existing != *entry {
            return Err(Error::Conflict(format!(
                "artifact '{storage_key}' is already owned by another build record"
            )));
        }
        return Ok(());
    }
    let cf = cf_handle(storage.db(), cf::REGISTRY)?;
    let value = serde_json::to_vec(entry).map_err(storage_err)?;
    storage
        .db()
        .put_cf(cf, key(tenant, repo, storage_key), value)
        .map_err(storage_err)
}

/// The owner of an artifact, if the engine wrote it.
pub fn lookup(
    storage: &RocksDBStorage,
    tenant: &str,
    repo: &str,
    storage_key: &str,
) -> Result<Option<LedgerEntry>> {
    let cf = cf_handle(storage.db(), cf::REGISTRY)?;
    match storage
        .db()
        .get_cf(cf, key(tenant, repo, storage_key))
        .map_err(storage_err)?
    {
        Some(v) => Ok(serde_json::from_slice(&v).ok()),
        None => Ok(None),
    }
}

fn remove(storage: &RocksDBStorage, tenant: &str, repo: &str, storage_key: &str) -> Result<()> {
    let cf = cf_handle(storage.db(), cf::REGISTRY)?;
    storage
        .db()
        .delete_cf(cf, key(tenant, repo, storage_key))
        .map_err(storage_err)
}

/// Every artifact of `repo` whose owner matches `owns`.
fn entries_where(
    storage: &RocksDBStorage,
    tenant: &str,
    repo: &str,
    owns: impl Fn(&LedgerEntry) -> bool,
) -> Result<Vec<(String, LedgerEntry)>> {
    let cf = cf_handle(storage.db(), cf::REGISTRY)?;
    let prefix = repo_prefix(tenant, repo);
    let mut out = Vec::new();
    for item in crate::prefix_scan(storage.db(), cf, &prefix) {
        let (k, v) = item.map_err(storage_err)?;
        if !k.starts_with(&prefix) {
            break;
        }
        let storage_key = String::from_utf8_lossy(&k[prefix.len()..])
            .trim_end_matches('\0')
            .to_string();
        if let Ok(entry) = serde_json::from_slice::<LedgerEntry>(&v) {
            if owns(&entry) {
                out.push((storage_key, entry));
            }
        }
    }
    Ok(out)
}

/// Is `storage_key` still referenced by the same node on ANOTHER branch?
/// A fork copies nodes with their ids, so a forked build record shares its
/// artifact blobs; deleting the original must not break the copy.
async fn referenced_elsewhere(
    storage: &RocksDBStorage,
    tenant: &str,
    repo: &str,
    entry: &LedgerEntry,
    storage_key: &str,
) -> bool {
    let Ok(branches) = storage.branches().list_branches(tenant, repo).await else {
        // Cannot tell: keep the blob (GC reclaims a true orphan later).
        return true;
    };
    for b in branches {
        if b.name == entry.branch {
            continue;
        }
        let scope = StorageScope::new(tenant, repo, &b.name, &entry.workspace);
        if let Ok(Some(node)) = storage.nodes().get(scope, &entry.node_id, None).await {
            let names_it = node.properties.values().any(|v| match v {
                PropertyValue::Resource(r) => matches!(
                    r.metadata.as_ref().and_then(|m| m.get("storage_key")),
                    Some(PropertyValue::String(k)) if k == storage_key
                ),
                _ => false,
            });
            if names_it {
                return true;
            }
        }
    }
    false
}

/// Which record's artifacts to delete. `Node` is driven only by the engine's
/// own delete event for that node; `Path` only by `packages_deleteArtifacts`
/// after the caller is authorized for that build state node. No route takes
/// a caller-supplied node id.
pub enum Owner<'a> {
    /// The record node with this id, on this branch.
    Node { branch: &'a str, node_id: &'a str },
    /// The record at this workspace path, on this branch.
    Path {
        branch: &'a str,
        workspace: &'a str,
        path: &'a str,
    },
}

/// Delete every artifact the engine recorded for `owner`, and forget it.
/// Keys come from the ledger only, are re-checked against the tenant/repo
/// artifact namespace, and a blob the same record still names on another
/// branch is kept. Returns how many blobs were deleted.
pub async fn delete_for(
    storage: &RocksDBStorage,
    store: &dyn DynBinaryStorage,
    tenant: &str,
    repo: &str,
    owner: Owner<'_>,
) -> Result<usize> {
    let entries = entries_where(storage, tenant, repo, |e| match &owner {
        Owner::Node { branch, node_id } => e.branch == *branch && e.node_id == *node_id,
        Owner::Path {
            branch,
            workspace,
            path,
        } => e.branch == *branch && e.workspace == *workspace && e.path == *path,
    })?;
    let mut deleted = 0;
    for (storage_key, entry) in entries {
        if !raisin_core::is_artifact_key_of(tenant, repo, &storage_key) {
            tracing::warn!(%tenant, %repo, %storage_key, "artifact ledger entry outside its namespace; skipped");
            continue;
        }
        if referenced_elsewhere(storage, tenant, repo, &entry, &storage_key).await {
            tracing::info!(%storage_key, "artifact still named on another branch; kept");
            continue;
        }
        match store.delete(&storage_key).await {
            Ok(()) => {
                remove(storage, tenant, repo, &storage_key)?;
                deleted += 1;
            }
            Err(e) => tracing::warn!(%storage_key, error = %e, "could not delete artifact blob"),
        }
    }
    Ok(deleted)
}

/// Forget artifacts whose blobs a failed build already deleted.
pub fn forget(storage: &RocksDBStorage, tenant: &str, repo: &str, keys: &[String]) {
    for k in keys {
        let _ = remove(storage, tenant, repo, k);
    }
}
