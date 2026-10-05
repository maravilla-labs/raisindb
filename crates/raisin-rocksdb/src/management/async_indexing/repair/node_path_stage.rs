//! The `node_path` backfill's staged entries, re-checked right before they
//! are written.
//!
//! The scan decides an entry is needed long before its batch commits — up to
//! a full batch of nodes later. In that window an IN-PLACE writer (a
//! `versionable=false` update, a replicated in-place upsert) can rewrite the
//! node at the very revision R the entry is for, storing a path-less blob and
//! its own `NODE_PATH(R)`. Writing the staged entry after that overwrote the
//! writer's path with the legacy blob's stale one, for good: the blob is no
//! longer legacy, so no later run looks at it again.
//!
//! So entries are not queued when they are decided. They are held here and,
//! at commit, re-checked under the in-place guard's WRITE side
//! (`repositories/nodes/crud/indexing/in_place_guard.rs`), which every
//! in-place writer holds the read side of around its own write: an entry is
//! written only if the blob at R is STILL the legacy blob embedding that path
//! and `NODE_PATH` still holds nothing at R.

use super::cursor::BoundedWriter;
use super::node_path_backfill::{NodePathCounts, Scope, PASS_NODE_PATH};
use crate::mvcc_read::embedded_path_of;
use crate::repositories::nodes::helpers::is_tombstone;
use crate::{cf, cf_handle};
use raisin_error::Result;
use rocksdb::DB;

/// One decided entry: `NODE_PATH` at `entry_key` := `path`, valid while the
/// blob at `node_key` (same node, same revision) is the legacy blob that
/// embeds `path`.
struct Staged {
    node_key: Vec<u8>,
    entry_key: Vec<u8>,
    path: String,
}

/// The entries decided since the last commit.
#[derive(Default)]
pub(super) struct Stage {
    entries: Vec<Staged>,
    bytes: usize,
}

impl Stage {
    pub(super) fn push(&mut self, node_key: Vec<u8>, entry_key: Vec<u8>, path: String) {
        self.bytes += entry_key.len() + path.len();
        self.entries.push(Staged {
            node_key,
            entry_key,
            path,
        });
    }

    /// Bytes the entries would add to the batch.
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Commit the writer's batch plus every staged entry that survives the
    /// re-check, as `running`. Returns `false` when the run must stop (the
    /// crash hook fired).
    pub(super) fn commit(
        &mut self,
        db: &DB,
        scope: &Scope<'_>,
        writer: &mut BoundedWriter<'_>,
        counts: &mut NodePathCounts,
    ) -> Result<bool> {
        let entries = std::mem::take(&mut self.entries);
        self.bytes = 0;
        writer.commit_prepared("running", |writer| {
            let guard = (!entries.is_empty() && !writer.dry_run()).then(|| {
                crate::repositories::nodes::backfill_write_guard(
                    scope.tenant_id,
                    scope.repo_id,
                    scope.branch,
                )
            });
            for entry in still_valid(db, entries, counts)? {
                writer.put(cf::NODE_PATH, &entry.entry_key, entry.path.as_bytes())?;
                counts.written += 1;
            }
            Ok(guard)
        })?;
        Ok(!writer.stop_requested())
    }
}

/// The entries whose blob is still the legacy one and whose `NODE_PATH` slot
/// is still empty — read NOW, in one multi-get per CF.
fn still_valid(db: &DB, entries: Vec<Staged>, counts: &mut NodePathCounts) -> Result<Vec<Staged>> {
    if entries.is_empty() {
        return Ok(entries);
    }
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let cf_node_path = cf_handle(db, cf::NODE_PATH)?;
    let blobs = db.multi_get_cf(entries.iter().map(|e| (&cf_nodes, e.node_key.as_slice())));
    let slots = db.multi_get_cf(
        entries
            .iter()
            .map(|e| (&cf_node_path, e.entry_key.as_slice())),
    );
    let mut out = Vec::with_capacity(entries.len());
    for ((entry, blob), slot) in entries.into_iter().zip(blobs).zip(slots) {
        let blob = blob.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        let slot = slot.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        let legacy = blob.is_some_and(|v| {
            !is_tombstone(&v) && embedded_path_of(&v).as_deref() == Some(entry.path.as_str())
        });
        if !legacy {
            counts.changed_during_scan += 1;
            continue;
        }
        match slot {
            None => out.push(entry),
            // Written meanwhile with the same path: nothing left to do.
            Some(value) if value == entry.path.as_bytes() => {}
            Some(_) => {
                tracing::warn!(
                    embedded_path = %entry.path,
                    pass = PASS_NODE_PATH,
                    "node_path backfill: NODE_PATH gained a different entry at the legacy blob's \
                     own revision while the scan ran; leaving it"
                );
                counts.conflicts += 1;
            }
        }
    }
    Ok(out)
}
