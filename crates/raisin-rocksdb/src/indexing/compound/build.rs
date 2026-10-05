//! THE compound build pass, shared by the per-index build job
//! (`jobs/handlers/compound_index.rs`) and the workspace `REBUILD`
//! (`management/async_indexing/compound_rebuild.rs`).
//!
//! A build re-derives the keyspace AS OF a branch HEAD the caller read after
//! clearing — the build's HISTORY FLOOR, stamped as `built_through`:
//!
//! - each node's newest version at or below the floor is written at that
//!   version's own revision, its path materialized at the floor;
//! - every version ABOVE the floor (a write that committed while the build
//!   ran, or a version stranded above HEAD) is replayed oldest first at its own
//!   revision, with the tombstones that end the tuples it drops — so a HEAD
//!   read never picks a version the scan would not, and the index agrees with
//!   NODES once HEAD passes a stranded version.
//!
//! History BELOW the floor is not re-derived (an ancestor move re-keys
//! `__parent_path` without a NODES version, so NODES alone cannot reproduce
//! it). The planner refuses the index for reads below the floor instead
//! (`CompoundAvailability::at_revision`); they scan.
//!
//! The pass streams: one node's versions in memory at a time, bounded batches.
//! A node that cannot be decoded or placed in the tree (no path) is COUNTED,
//! never silently dropped: [`precheck`] refuses before anything is cleared,
//! and a writing pass that meets one must not stamp `Ready`.

use super::entries::{compound_entries, entry_key, CompoundGroup};
use super::writer::LIVE;
use crate::indexing::IndexCtx;
use crate::keys::{self, TOMBSTONE_VALUE as TOMBSTONE};
use crate::{cf, cf_handle};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};
use std::collections::{BTreeSet, HashMap};

/// Per node type, the declarations a build writes (types absent: none).
pub type Wanted = HashMap<String, Vec<CompoundIndexDefinition>>;

/// What one pass saw.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BuildOutcome {
    /// Nodes with at least one version of a wanted type.
    pub nodes: usize,
    /// Live entries written.
    pub entries: usize,
    /// Nodes that could not be decoded or placed (no path).
    pub unplaceable: usize,
}

/// Entries per committed batch.
const BATCH_ENTRIES: usize = 10_000;

/// Just enough of a stored node to decide whether the build wants it (the
/// untagged `PropertyValue` decode stays out of the scan for other types).
#[derive(serde::Deserialize)]
struct NodeTypeProbe {
    node_type: String,
}

enum Raw {
    Deleted,
    OtherType,
    Wanted(Vec<u8>),
}

/// Before anything is cleared: the disk headroom check and a read-only pass
/// that refuses when any node cannot be placed — the clear would delete
/// entries the build cannot write back.
pub fn precheck(db: &DB, ctx: &IndexCtx<'_>, wanted: &Wanted, floor: &HLC) -> Result<()> {
    crate::management::async_indexing::repair::check_headroom(db, cf::COMPOUND_INDEX)?;
    let seen = run(db, ctx, wanted, floor, false)?;
    refuse_unplaceable(ctx, &seen)
}

/// The writing pass (after the caller cleared the keyspace and read `floor`).
/// The caller stamps `Ready` only when [`BuildOutcome::unplaceable`] is 0.
pub fn write(db: &DB, ctx: &IndexCtx<'_>, wanted: &Wanted, floor: &HLC) -> Result<BuildOutcome> {
    run(db, ctx, wanted, floor, true)
}

/// The error a build returns when nodes could not be placed.
pub fn refuse_unplaceable(ctx: &IndexCtx<'_>, seen: &BuildOutcome) -> Result<()> {
    if seen.unplaceable == 0 {
        return Ok(());
    }
    Err(Error::storage(format!(
        "refusing to build compound indexes for {}/{}/{}/{}: {} node(s) could not be \
         decoded or placed in the tree (no path); the index stays unusable (scan) until \
         that is repaired",
        ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace, seen.unplaceable
    )))
}

fn run(
    db: &DB,
    ctx: &IndexCtx<'_>,
    wanted: &Wanted,
    floor: &HLC,
    writing: bool,
) -> Result<BuildOutcome> {
    let mut out = BuildOutcome::default();
    if wanted.values().all(Vec::is_empty) {
        return Ok(out);
    }
    let prefix = keys::KeyBuilder::new()
        .push(ctx.tenant_id)
        .push(ctx.repo_id)
        .push(ctx.branch)
        .push(ctx.workspace)
        .push("nodes")
        .build_prefix();
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let mut pass = Pass {
        db,
        ctx,
        wanted,
        floor,
        writing,
        batch: WriteBatch::default(),
        out: &mut out,
    };
    // Keys are `…\0nodes\0{id}\0{~rev}`, newest first per id: collect the
    // versions above the floor and the first at or below it, skip the rest.
    let mut current: Option<String> = None;
    let mut versions: Vec<(HLC, Raw)> = Vec::new();
    let mut reached_floor = false;
    for item in crate::prefix_scan(db, cf_nodes, &prefix) {
        let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;
        if !key.starts_with(&prefix) {
            break;
        }
        let Some(id) = key[prefix.len()..]
            .split(|&b| b == 0)
            .next()
            .and_then(|b| std::str::from_utf8(b).ok())
        else {
            continue;
        };
        if current.as_deref() != Some(id) {
            if let Some(done) = current.take() {
                pass.node(&done, std::mem::take(&mut versions))?;
            }
            current = Some(id.to_string());
            reached_floor = false;
        }
        if reached_floor {
            continue;
        }
        let Ok(revision) = keys::extract_revision_from_key(&key) else {
            tracing::warn!(node_id = %id, "compound build: unparseable node key revision");
            continue;
        };
        reached_floor = revision <= *floor;
        versions.push((revision, pass.classify(id, &value)));
    }
    if let Some(done) = current.take() {
        pass.node(&done, versions)?;
    }
    pass.flush()?;
    drop(pass);
    Ok(out)
}

struct Pass<'a, 'o> {
    db: &'a DB,
    ctx: &'a IndexCtx<'a>,
    wanted: &'a Wanted,
    floor: &'a HLC,
    writing: bool,
    batch: WriteBatch,
    out: &'o mut BuildOutcome,
}

impl Pass<'_, '_> {
    fn classify(&self, id: &str, value: &[u8]) -> Raw {
        if value.is_empty()
            || keys::is_tombstone_value(value)
            || crate::repositories::is_node_tombstone(value)
        {
            return Raw::Deleted;
        }
        match rmp_serde::from_slice::<NodeTypeProbe>(value) {
            Ok(probe) if !self.wanted.contains_key(&probe.node_type) => Raw::OtherType,
            Ok(_) => Raw::Wanted(value.to_vec()),
            Err(e) => {
                // The full decode has the last word.
                tracing::debug!(node_id = %id, "node type probe failed; full decode: {}", e);
                Raw::Wanted(value.to_vec())
            }
        }
    }

    /// One node: `versions` newest first, ending at its newest version at or
    /// below the floor (when it has one).
    fn node(&mut self, id: &str, mut versions: Vec<(HLC, Raw)>) -> Result<()> {
        if !versions
            .iter()
            .any(|(_, raw)| matches!(raw, Raw::Wanted(_)))
        {
            return Ok(());
        }
        self.out.nodes += 1;
        versions.reverse();
        let mut decoded: Vec<(HLC, Option<Node>)> = Vec::with_capacity(versions.len());
        for (revision, raw) in versions {
            let Raw::Wanted(bytes) = raw else {
                decoded.push((revision, None));
                continue;
            };
            // The floor version's path as a HEAD read at the floor sees it; a
            // version above the floor's as of its own revision.
            let read_at = if revision <= *self.floor {
                *self.floor
            } else {
                revision
            };
            let c = self.ctx;
            match crate::mvcc_read::deserialize_node_with_path(
                self.db,
                &bytes,
                c.tenant_id,
                c.repo_id,
                c.branch,
                c.workspace,
                id,
                &read_at,
                &revision,
            ) {
                Ok(node) if !node.path.is_empty() => decoded.push((revision, Some(node))),
                Ok(_) => return self.unplaceable(id, "no path in NODE_PATH"),
                Err(e) => return self.unplaceable(id, &e.to_string()),
            }
        }
        if !self.writing {
            return Ok(());
        }
        let cf_compound = cf_handle(self.db, cf::COMPOUND_INDEX)?;
        let mut prev: BTreeSet<CompoundGroup> = BTreeSet::new();
        for (revision, node) in decoded {
            let cur = node
                .map(|n| {
                    let defs = self.wanted.get(&n.node_type).map_or(&[][..], Vec::as_slice);
                    compound_entries(defs, self.ctx, &n)
                })
                .unwrap_or_default();
            for group in prev.difference(&cur) {
                self.batch
                    .put_cf(cf_compound, entry_key(group, &revision, id), TOMBSTONE);
            }
            for group in cur.difference(&prev) {
                self.batch
                    .put_cf(cf_compound, entry_key(group, &revision, id), LIVE);
                self.out.entries += 1;
            }
            prev = cur;
        }
        if self.batch.len() >= BATCH_ENTRIES {
            self.flush()?;
        }
        Ok(())
    }

    fn unplaceable(&mut self, id: &str, why: &str) -> Result<()> {
        tracing::warn!(node_id = %id, reason = %why, "compound build: node cannot be placed");
        self.out.unplaceable += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.batch.is_empty() {
            return Ok(());
        }
        self.db
            .write(std::mem::take(&mut self.batch))
            .map_err(|e| Error::storage(format!("compound build batch write failed: {e}")))
    }
}
