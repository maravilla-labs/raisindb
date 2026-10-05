//! The scan half of `timestamp_backfill`: bounded chunks of a branch's NODES,
//! one node group at a time, picking the live nodes whose newest version has
//! no `created_at` / `updated_at` and the timestamps their history implies.

use super::super::node_key_parse::parse_node_key;
use crate::repositories::nodes::helpers::is_tombstone;
use crate::{cf, cf_handle, keys};
use chrono::{DateTime, Utc};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use rocksdb::{ReadOptions, DB};

/// A node the backfill writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Candidate {
    pub(super) workspace: String,
    pub(super) node_id: String,
    /// The physical time of the node's oldest retained version on the branch.
    pub(super) created_at: DateTime<Utc>,
    /// The physical time of its newest version.
    pub(super) updated_at: DateTime<Utc>,
    /// The newest version's blob size (pacing and headroom estimates).
    pub(super) blob_bytes: u64,
}

/// One chunk of the scan.
#[derive(Debug, Default)]
pub(super) struct Chunk {
    pub(super) candidates: Vec<Candidate>,
    /// Live nodes seen in the chunk.
    pub(super) live: u64,
    /// Newest versions that could not be decoded.
    pub(super) undecodable: u64,
    /// Bytes of the newest versions read (charged to the pacing).
    pub(super) bytes_read: u64,
    /// The node-group key (`{…}nodes\0{id}\0`) of the last node finished;
    /// `None` when the branch has no node after the cursor.
    pub(super) last_group: Option<Vec<u8>>,
}

/// The node in progress: its newest version and the oldest revision so far.
struct Group {
    key: Vec<u8>,
    workspace: String,
    node_id: String,
    newest: HLC,
    newest_value: Vec<u8>,
    oldest: HLC,
}

/// Up to `max_nodes` node groups of `(tenant, repo, branch)`'s NODES strictly
/// after the group `after` (a previous chunk's `last_group`), ending early
/// once the newest versions read reach `max_bytes`. Read synchronously, so
/// the iterator never crosses an `.await`.
pub(super) fn scan_chunk(
    db: &DB,
    (tenant_id, repo_id, branch): &(String, String, String),
    after: Option<&[u8]>,
    (max_nodes, max_bytes): (usize, u64),
) -> Result<Chunk> {
    let branch_prefix = keys::branch_prefix(tenant_id, repo_id, branch);
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    opts.fill_cache(false);
    if let Some(upper) = crate::prefix_successor(&branch_prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf_handle(db, cf::NODES)?, opts);
    match after {
        // Past every version of the node the cursor names (its group
        // prefix's successor; a revision byte can itself be 0xFF).
        Some(after) => match crate::prefix_successor(after) {
            Some(next) => iter.seek(&next),
            None => return Ok(Chunk::default()),
        },
        None => iter.seek(&branch_prefix),
    }
    let mut chunk = Chunk::default();
    let mut current: Option<Group> = None;
    let mut finished = 0usize;
    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        let Some((workspace, node_id, revision)) = parse_node_key(&branch_prefix, key) else {
            iter.next();
            continue;
        };
        let group_key = &key[..key.len() - 16];
        match &mut current {
            // Keys run newest first: a later key of the group is older.
            Some(group) if group.key == group_key => group.oldest = revision,
            _ => {
                if let Some(done) = current.take() {
                    finish(&mut chunk, done);
                    finished += 1;
                    if finished >= max_nodes || chunk.bytes_read >= max_bytes {
                        return Ok(chunk);
                    }
                }
                current = Some(Group {
                    key: group_key.to_vec(),
                    workspace: workspace.to_string(),
                    node_id: node_id.to_string(),
                    newest: revision,
                    newest_value: value.to_vec(),
                    oldest: revision,
                });
            }
        }
        iter.next();
    }
    iter.status().map_err(|e| Error::storage(e.to_string()))?;
    if let Some(done) = current {
        finish(&mut chunk, done);
    }
    Ok(chunk)
}

fn finish(chunk: &mut Chunk, group: Group) {
    chunk.last_group = Some(group.key);
    if is_tombstone(&group.newest_value) {
        return; // deleted: nothing to backfill
    }
    chunk.live += 1;
    chunk.bytes_read += group.newest_value.len() as u64;
    let node = match crate::mvcc_read::decode_node_blob(&group.newest_value) {
        Ok((node, _)) => node,
        Err(e) => {
            chunk.undecodable += 1;
            tracing::warn!(
                workspace = %group.workspace,
                node_id = %group.node_id,
                error = %e,
                "timestamp_backfill: newest version undecodable; skipped"
            );
            return;
        }
    };
    if node.created_at.is_some() && node.updated_at.is_some() {
        return;
    }
    chunk.candidates.push(Candidate {
        workspace: group.workspace,
        node_id: group.node_id,
        created_at: physical_time(&group.oldest),
        updated_at: physical_time(&group.newest),
        blob_bytes: group.newest_value.len() as u64,
    });
}

/// An HLC's physical time as a timestamp (millisecond precision).
pub(super) fn physical_time(revision: &HLC) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(revision.timestamp_ms as i64).unwrap_or_default()
}
