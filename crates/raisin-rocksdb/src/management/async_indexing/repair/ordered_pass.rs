//! Pass 2 of the ORDERED_CHILDREN repair: verify every live entry at HEAD.
//!
//! Walks ORDERED_CHILDREN per `(workspace, parent)`, decides each
//! `(label, child)` by its newest entry at or below HEAD, and tombstones a live
//! one when its child
//!
//! - has no live NODES version at HEAD (deleted, or every version GC-dropped):
//!   at the delete revision when one is known and newer than the entry, else
//!   just after the entry's own revision; or
//! - is live but placed under ANOTHER parent (a move that left its old entry
//!   live): at the move revision when NODE_PATH shows it, else just after the
//!   entry. Placement is the same answer the `has_children` probe uses.
//!
//! Memory is one label's decided children: keys sort `{label}\0{~rev}\0{child}`
//! within a parent, so a label's entries are contiguous, and the cursor is
//! checkpointed at every label boundary — a parent with a million children
//! commits in bounded batches like everything else.

use super::cursor::BoundedWriter;
use super::ordered_children::{
    iterate_from, successor, OrderedChildrenCounts, Scope, PASS_ORDERED,
};
use crate::repositories::nodes::{child_is_under, node_path_at, parse_ordered_child_key};
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;
use std::collections::HashSet;

/// The `(workspace, parent)` group in progress.
struct Group {
    prefix: Vec<u8>,
    workspace: String,
    parent_id: String,
    /// The parent's path at HEAD; `None` when it cannot be learned, in which
    /// case placement is not judged (the entry gets the benefit of the doubt).
    parent_path: Option<String>,
}

/// Pass 2. Returns `false` when the run must stop early.
pub(super) fn ordered_pass(
    db: &DB,
    scope: &Scope<'_>,
    writer: &mut BoundedWriter<'_>,
    counts: &mut OrderedChildrenCounts,
) -> Result<bool> {
    writer.begin_pass(PASS_ORDERED);
    let branch_prefix = scope.branch_prefix();
    let cursor = writer
        .state()
        .cursor
        .as_deref()
        .and_then(|h| hex::decode(h).ok());
    let mut iter = iterate_from(db, cf::ORDERED_CHILDREN, &branch_prefix, cursor.as_deref())?;
    let cf_nodes = cf_handle(db, cf::NODES)?;

    let mut group: Option<Group> = None;
    // The label in progress and the children already decided under it.
    let mut label: Option<String> = None;
    let mut decided: HashSet<String> = HashSet::new();
    let mut last_key: Option<Vec<u8>> = None;

    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        let key = key.to_vec();
        let value = value.to_vec();
        iter.next();

        // `{ws}\0ordered\0{parent}\0` after the branch prefix.
        let rest = &key[branch_prefix.len()..];
        let mut parts = rest.splitn(4, |b| *b == 0);
        let (Some(ws), Some(tag), Some(parent)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if tag != b"ordered" {
            continue;
        }
        let prefix_len = branch_prefix.len() + ws.len() + tag.len() + parent.len() + 3;
        let Some(parsed) = parse_ordered_child_key(&key, &key[..prefix_len]) else {
            continue; // the parent's META record, or an unparseable key
        };

        let new_group = group.as_ref().is_none_or(|g| g.prefix != key[..prefix_len]);
        if new_group || label.as_deref() != Some(parsed.order_label) {
            // A label boundary: the previous label is fully decided.
            if let Some(done) = last_key.take() {
                if !writer.checkpoint(PASS_ORDERED, &done)? {
                    return Ok(false);
                }
            }
            label = Some(parsed.order_label.to_string());
            decided.clear();
        }
        if new_group {
            let workspace = String::from_utf8_lossy(ws).into_owned();
            let parent_id = String::from_utf8_lossy(parent).into_owned();
            let parent_path = if parent_id == "/" {
                Some("/".to_string())
            } else {
                let (t, r, b) = (scope.tenant_id, scope.repo_id, scope.branch);
                node_path_at(db, t, r, b, &workspace, &parent_id, scope.head.as_ref())?
            };
            group = Some(Group {
                prefix: key[..prefix_len].to_vec(),
                workspace,
                parent_id,
                parent_path,
            });
        }
        last_key = Some(key.clone());
        let group = group.as_ref().expect("set above");

        let Some(entry_rev) = parsed.revision() else {
            continue;
        };
        if scope.head.is_some_and(|head| entry_rev > head) {
            continue; // stranded above HEAD: not this branch's state yet
        }
        if !decided.insert(parsed.child_id.to_string()) || keys::is_tombstone_value(&value) {
            continue;
        }

        // A live entry. Is its child live at HEAD?
        let node_prefix = keys::node_key_prefix(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            &group.workspace,
            parsed.child_id,
        );
        let newest = crate::mvcc_read::newest_at_or_before_with(
            db,
            cf_nodes,
            &node_prefix,
            scope.head.as_ref(),
            |rev, v| (rev, keys::is_tombstone_value(v)),
        )?;
        let at = match newest {
            Some((_, false)) => {
                // Live child: stale only if it now lives under another parent.
                let Some(parent_path) = group.parent_path.as_deref() else {
                    continue;
                };
                let (t, r, b) = (scope.tenant_id, scope.repo_id, scope.branch);
                if child_is_under(
                    db,
                    t,
                    r,
                    b,
                    &group.workspace,
                    parsed.child_id,
                    &value,
                    parent_path,
                    scope.head.as_ref(),
                )? {
                    continue;
                }
                counts.misplaced_entries += 1;
                move_revision(db, scope, &group.workspace, parsed.child_id, parent_path)?
                    .filter(|moved| moved > &entry_rev)
                    .unwrap_or_else(|| successor(&entry_rev))
            }
            Some((deleted_at, true)) => {
                // The NODES pass tombstones these at the delete revision; a
                // dry run cannot see its uncounted writes, so skip rather
                // than count the same entry twice.
                if writer.dry_run() && deleted_at > entry_rev {
                    continue;
                }
                counts.orphan_entries += 1;
                if deleted_at > entry_rev {
                    deleted_at
                } else {
                    successor(&entry_rev)
                }
            }
            None => {
                counts.orphan_entries += 1;
                counts.without_delete_revision += 1;
                successor(&entry_rev)
            }
        };
        let tombstone_key = keys::ordered_child_key_versioned(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            &group.workspace,
            &group.parent_id,
            parsed.order_label,
            &at,
            parsed.child_id,
        );
        writer.put(cf::ORDERED_CHILDREN, &tombstone_key, keys::TOMBSTONE_VALUE)?;
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    if let Some(done) = last_key {
        if !writer.checkpoint(PASS_ORDERED, &done)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The revision at which `child_id` left `parent_path`, from its NODE_PATH
/// history at or below HEAD: the oldest version of the run of newest versions
/// that are NOT under `parent_path`. `None` when NODE_PATH has no such run
/// (the transaction write path stores no NODE_PATH).
fn move_revision(
    db: &DB,
    scope: &Scope<'_>,
    workspace: &str,
    child_id: &str,
    parent_path: &str,
) -> Result<Option<HLC>> {
    let prefix = keys::node_path_key_prefix(
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        workspace,
        child_id,
    );
    let mut iter = iterate_from(db, cf::NODE_PATH, &prefix, None)?;
    let mut moved: Option<HLC> = None;
    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        let Some(rev) = key
            .len()
            .checked_sub(16)
            .and_then(|at| HLC::decode_descending(&key[at..]).ok())
        else {
            iter.next();
            continue;
        };
        if scope.head.is_some_and(|head| rev > head) {
            iter.next();
            continue;
        }
        if keys::is_tombstone_value(value)
            || parent_of(&String::from_utf8_lossy(value)) == parent_path
        {
            break;
        }
        moved = Some(rev);
        iter.next();
    }
    Ok(moved)
}

/// `/a/b` -> `/a`; `/a` -> `/`.
fn parent_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/",
        Some((parent, _)) => parent,
    }
}
