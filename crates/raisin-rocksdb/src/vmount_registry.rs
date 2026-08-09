//! Which repositories actually contain a `raisin:VirtualMount`.
//!
//! # Why this exists
//!
//! The virtual-mount sync check runs every 60 seconds and has to find the mounts
//! that are due. Without a registry the only way to do that is to ask every
//! repository — `list_tenants` → `list_repositories` → a
//! `list_by_type("raisin:VirtualMount")` per repo — and almost every repository
//! in a deployment has never had a mount. Measured on production with frame
//! pointers, `VirtualMountSyncHandler::handle` was **51% of total server CPU**,
//! essentially all of it that fan-out. It is not a regression: the shape has
//! been there since the feature's first commit (`e201f92`). The volume is the
//! bug.
//!
//! This module is the index that makes the tick O(repos with mounts).
//!
//! # Where the entries come from — ONE write point per direction
//!
//! The registry is derived state, and the failure mode of derived state is a
//! writer and its sibling drifting apart (CLAUDE.md's #1 recurring bug class).
//! A missed entry here is *silent*: the mount simply never syncs, with no error
//! anywhere. So the entry is written **in the same `WriteBatch` as the mount
//! node itself**, at the funnels every node write already passes through —
//! exactly how the spatial and reference indexes are written alongside their
//! subject node:
//!
//! | direction | funnel | call site |
//! |---|---|---|
//! | write | transaction path (`add_node` + `put_node`) | `transaction/context/nodes/create/storage.rs::write_node_to_batch` |
//! | write | repository path (add / update / move / reorder / rebalance) | `repositories/nodes/crud/indexing/mod.rs::add_node_indexes_to_batch_with_parent_id` |
//! | delete | every delete path (transaction, repository, cascade) | `tombstones::add_node_tombstones` |
//!
//! Because the registry entry rides the node's own batch, it cannot desync from
//! the node's existence through any code path — including replication apply,
//! which funnels through the same two writers.
//!
//! Belt and braces on top of that: [`super::jobs::handlers::virtual_mount_sync`]
//! runs a slow full reconcile (see `registry_backfill.rs`) that both backfills
//! mounts predating this registry and shouts if the fast path ever missed one.
//!
//! # Membership is EXISTENCE, not enabled-ness
//!
//! A disabled or paused mount keeps its entry. Gating on `enabled` would buy
//! nothing — `is_due` already rejects it for free once the repo is scanned — and
//! would add a second condition the write points must agree about, which is the
//! drift this design exists to prevent.
//!
//! # Key layout
//!
//! `{tenant}\0vmounts\0{repo}\0{branch}\0{mount_node_id}` in [`cf::REGISTRY`],
//! value `b"1"`. Namespaced beside the existing `{tenant}\0repos\0…` entries
//! that `list_repositories_for_tenant` already scans, so no new column family is
//! needed.
//!
//! One key **per mount**, not per repo: a repo with two mounts that loses one
//! must stay listed, and a per-repo key would have the delete strand the
//! survivor. The branch segment is there because a fork that copies a mount
//! config node writes its own entry — which the reconcile then prunes, since no
//! scan ever reads a non-default branch for mounts (see `check::config_branch`).

use std::collections::BTreeSet;

use raisin_error::Result;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};

use crate::{cf, cf_handle, keys};

/// The node type whose existence this registry tracks.
pub const MOUNT_NODE_TYPE: &str = "raisin:VirtualMount";

/// Namespace segment inside [`cf::REGISTRY`].
const NAMESPACE: &str = "vmounts";

/// Registry value. The keys carry all the information; the value exists only
/// because RocksDB needs one. Nothing deserializes it — the list helpers read
/// KEYS only.
const PRESENT: &[u8] = b"1";

/// One registry entry, as parsed back out of its key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegistryEntry {
    pub repo: String,
    pub branch: String,
    pub mount_id: String,
}

/// `{tenant}\0vmounts\0{repo}\0{branch}\0{mount_id}`
pub fn registry_key(tenant: &str, repo: &str, branch: &str, mount_id: &str) -> Vec<u8> {
    keys::KeyBuilder::new()
        .push(tenant)
        .push(NAMESPACE)
        .push(repo)
        .push(branch)
        .push(mount_id)
        .build()
}

/// Scan prefix covering every mount entry of one tenant.
pub fn tenant_prefix(tenant: &str) -> Vec<u8> {
    keys::KeyBuilder::new()
        .push(tenant)
        .push(NAMESPACE)
        .build_prefix()
}

/// Stage the registry entry for `node`, if it is a mount.
///
/// Called from the node-write funnels with the node's own batch, so the entry
/// and the node commit together or not at all. A no-op — one string compare,
/// before any column-family lookup — for every other node type, which is all of
/// them on the hot path.
pub fn record_node_write(
    batch: &mut WriteBatch,
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node: &Node,
) -> Result<()> {
    if node.node_type != MOUNT_NODE_TYPE {
        return Ok(());
    }
    let cf_registry = cf_handle(db, cf::REGISTRY)?;
    batch.put_cf(
        cf_registry,
        registry_key(tenant_id, repo_id, branch, &node.id),
        PRESENT,
    );
    Ok(())
}

/// Stage removal of the registry entry for `node`, if it is a mount.
///
/// A real `delete_cf`, not an MVCC tombstone: the registry is a live derived set
/// with no revision dimension, and the whole point is that the prefix scan
/// shrinks when a mount goes away.
pub fn record_node_delete(
    batch: &mut WriteBatch,
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node: &Node,
) -> Result<()> {
    if node.node_type != MOUNT_NODE_TYPE {
        return Ok(());
    }
    let cf_registry = cf_handle(db, cf::REGISTRY)?;
    batch.delete_cf(
        cf_registry,
        registry_key(tenant_id, repo_id, branch, &node.id),
    );
    Ok(())
}

/// Every registry entry for one tenant, parsed from KEYS only.
pub fn list_entries(db: &DB, tenant: &str) -> Result<Vec<RegistryEntry>> {
    let cf_registry = cf_handle(db, cf::REGISTRY)?;
    let prefix = tenant_prefix(tenant);

    let mut out = Vec::new();
    for item in db.prefix_iterator_cf(cf_registry, &prefix) {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;
        // `prefix_iterator_cf` is bounded by the CF's prefix extractor, not by
        // the seek key, so it can hand back rows past the prefix. Every other
        // scan in this crate leans on the extractor; check explicitly, because
        // over-reading here would list repos of the WRONG tenant.
        if !key.starts_with(&prefix) {
            break;
        }
        if let Some(entry) = parse_entry(&key) {
            out.push(entry);
        }
    }
    Ok(out)
}

/// The repositories of `tenant` that hold at least one mount, deduplicated.
pub fn list_repos_with_mounts(db: &DB, tenant: &str) -> Result<Vec<String>> {
    let repos: BTreeSet<String> = list_entries(db, tenant)?
        .into_iter()
        .map(|e| e.repo)
        .collect();
    Ok(repos.into_iter().collect())
}

/// Whether one specific entry is present. Used by the reconcile to name the
/// entries the fast path missed.
pub fn contains(db: &DB, tenant: &str, repo: &str, branch: &str, mount_id: &str) -> Result<bool> {
    let cf_registry = cf_handle(db, cf::REGISTRY)?;
    Ok(db
        .get_cf(cf_registry, registry_key(tenant, repo, branch, mount_id))
        .map_err(|e| raisin_error::Error::storage(format!("Registry read failed: {}", e)))?
        .is_some())
}

/// Parse `{tenant}\0vmounts\0{repo}\0{branch}\0{mount_id}`.
///
/// `splitn(5, …)` so a mount id containing a separator (ids don't, but the
/// parse should not be the thing that assumes it) lands whole in the last field
/// instead of silently truncating.
fn parse_entry(key: &[u8]) -> Option<RegistryEntry> {
    let key_str = String::from_utf8_lossy(key);
    let parts: Vec<&str> = key_str.splitn(5, '\0').collect();
    if parts.len() < 5 || parts[1] != NAMESPACE {
        return None;
    }
    Some(RegistryEntry {
        repo: parts[2].to_string(),
        branch: parts[3].to_string(),
        mount_id: parts[4].to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_round_trips_through_the_parser() {
        let key = registry_key("acme", "site", "main", "mount-1");
        assert_eq!(
            parse_entry(&key),
            Some(RegistryEntry {
                repo: "site".into(),
                branch: "main".into(),
                mount_id: "mount-1".into(),
            })
        );
    }

    /// The namespace has to keep mount entries out of the `{tenant}\0repos\0…`
    /// scan and vice versa — they share a column family.
    #[test]
    fn the_repos_namespace_is_not_confused_for_a_mount_entry() {
        let repos_key = keys::KeyBuilder::new()
            .push("acme")
            .push("repos")
            .push("site")
            .build();
        assert_eq!(parse_entry(&repos_key), None);

        let prefix = tenant_prefix("acme");
        assert!(!repos_key.starts_with(&prefix));
        assert!(registry_key("acme", "site", "main", "m1").starts_with(&prefix));
    }

    /// A tenant whose name is a prefix of another must not scan into it.
    #[test]
    fn one_tenants_prefix_does_not_reach_another() {
        let prefix = tenant_prefix("acme");
        assert!(!registry_key("acme-eu", "site", "main", "m1").starts_with(&prefix));
    }
}
