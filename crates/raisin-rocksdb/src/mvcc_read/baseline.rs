//! The ONE baseline reader: a node's stored version at or before a revision,
//! decoded through the one decoder.
//!
//! Every writer that diffs an incoming node against what it supersedes reads
//! the superseded version here — the replication apply path
//! (`applicator/node_baseline.rs`) and the Phase 7 delta writers
//! (`indexing::property_delta::resolve_baseline`). One seek on
//! `{t}\0{r}\0{b}\0{ws}\0nodes\0{id}\0`, never a branch scan, and a
//! repository-written `StorageNode` blob gets its path from NODE_PATH as of the
//! version's own revision (decoding it as a `Node` returned `path = ""`).

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::DB;

/// A stored version: the revision it is stored at, and the node — `None` when
/// that version is a delete tombstone.
pub(crate) type StoredVersion = (HLC, Option<Node>);

/// The newest version of `node_id` in `workspace` at or before `max_revision`
/// (the newest at all when `None`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn node_version_at_or_before(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    max_revision: Option<&HLC>,
) -> Result<Option<StoredVersion>> {
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let Some((revision, value)) = super::newest_at_or_before(db, cf_nodes, &prefix, max_revision)?
    else {
        return Ok(None);
    };
    if keys::is_tombstone_value(&value) {
        return Ok(Some((revision, None)));
    }
    let mut node = super::deserialize_node_with_path(
        db, &value, tenant_id, repo_id, branch, workspace, node_id, &revision, &revision,
    )?;
    // A repository-written blob carries no workspace: the key is the authority.
    node.workspace = Some(workspace.to_string());
    Ok(Some((revision, Some(node))))
}

/// The greatest HLC strictly below `revision`, or `None` below the first.
pub(crate) fn predecessor(revision: &HLC) -> Option<HLC> {
    match (revision.timestamp_ms, revision.counter) {
        (0, 0) => None,
        (ts, 0) => Some(HLC::new(ts - 1, u64::MAX)),
        (ts, counter) => Some(HLC::new(ts, counter - 1)),
    }
}

/// The newest version of `node_id` STRICTLY below `revision` (Phase 2.8's
/// `load_node_before`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn node_version_before(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    revision: &HLC,
) -> Result<Option<StoredVersion>> {
    match predecessor(revision) {
        Some(bound) => node_version_at_or_before(
            db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            Some(&bound),
        ),
        None => Ok(None),
    }
}

/// Every stored version of `node_id` STRICTLY ABOVE `revision`, oldest first
/// (normally none: a write at a fresh revision is the newest).
#[allow(clippy::too_many_arguments)]
pub(crate) fn node_versions_above(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    revision: &HLC,
) -> Result<Vec<StoredVersion>> {
    let mut out = Vec::new();
    scan_versions(
        db,
        (tenant_id, repo_id, branch, workspace),
        node_id,
        Some(revision),
        |at, value| {
            let node = if keys::is_tombstone_value(value) {
                None
            } else {
                let mut node = super::deserialize_node_with_path(
                    db, value, tenant_id, repo_id, branch, workspace, node_id, &at, &at,
                )?;
                node.workspace = Some(workspace.to_string());
                Some(node)
            };
            out.push((at, node));
            Ok(())
        },
    )?;
    out.reverse();
    Ok(out)
}

/// Visit every version of `node_id` strictly `above` the given revision (all
/// of them when `None`), newest first.
fn scan_versions(
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    above: Option<&HLC>,
    mut visit: impl FnMut(HLC, &[u8]) -> Result<()>,
) -> Result<()> {
    let (tenant_id, repo_id, branch, workspace) = scope;
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let mut opts = rocksdb::ReadOptions::default();
    opts.set_total_order_seek(true);
    if let Some(upper) = crate::prefix_successor(&prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf_nodes, opts);
    iter.seek(&prefix);
    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        if !key.starts_with(&prefix) || key.len() != prefix.len() + 16 {
            break;
        }
        let Ok(at) = keys::extract_revision_from_key(key) else {
            iter.next();
            continue;
        };
        if above.is_some_and(|above| at <= *above) {
            break; // newest first: everything after is older
        }
        visit(at, value)?;
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::predecessor;
    use raisin_hlc::HLC;

    #[test]
    fn predecessor_is_strictly_below() {
        assert_eq!(predecessor(&HLC::new(5, 3)), Some(HLC::new(5, 2)));
        assert_eq!(predecessor(&HLC::new(5, 0)), Some(HLC::new(4, u64::MAX)));
        assert_eq!(predecessor(&HLC::new(0, 0)), None);
        assert!(predecessor(&HLC::new(5, 0)).unwrap() < HLC::new(5, 0));
    }
}
