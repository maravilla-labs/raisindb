//! Which repair to run, by slug.

use crate::cf;
use serde::{Deserialize, Serialize};

/// Which repair to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepairKind {
    /// Tombstone ORDERED_CHILDREN entries deletes left live (plan item 2.1).
    OrderedChildren,
    /// Rewrite merge's legacy `\0` PATH_INDEX tombstones as `T` (item 2.2).
    PathTombstone,
    /// Write `NODE_PATH` at every legacy full-`Node` blob revision whose
    /// embedded path `NODE_PATH` does not answer (Phase 10 backfill).
    NodePath,
    /// Put every PROPERTY_INDEX entry of each node's newest version at that
    /// version's revision, through the one entry derivation (plan Phase 7).
    /// Completing it on a branch is what lets `index.skip_unchanged` skip
    /// there (see `property_index.rs`).
    PropertyIndex,
    /// Sample nodes and check their newest version's PROPERTY_INDEX entries
    /// are live; on a miss, reset the branch's rebuild state and queue the
    /// rebuild. Writes no index data.
    PropertyIndexVerify,
    /// Run-collapse GC (plan Phase 9): delete index versions whose next-older
    /// version has the same state, strictly below the causal-stability
    /// watermark, on PROPERTY / REFERENCE / ORDERED_CHILDREN / UNIQUE /
    /// COMPOUND (`history_gc::collapse`). Deletes, never writes entries.
    CollapseRuns,
    /// Re-emit every stored translation version (`TRANSLATION_DATA` and
    /// `BLOCK_TRANSLATIONS`, tombstones included) at its ORIGINAL revision as
    /// replication ops, so replicas that never received translations converge
    /// with history intact (plan Phase 11 item 5). Writes no local data.
    ResyncTranslations,
    /// Build the localized name index of a branch (plan Phase 12): stream
    /// the branch's NODES and put every node's localized segments as of the
    /// build's pinned revision, through the one selector and the one writer;
    /// then count sibling collisions and stamp each workspace's state record
    /// `Ready` (`crate::localized_name::rebuild`). Queued automatically per
    /// branch (on by default), and by the admin fan-out like every repair.
    LocalizedNames,
    /// Store `T` at every node delete for each block overlay live there
    /// (plan Phase 11c): the tombstones node deletes never wrote before.
    /// Reads already end those overlays (the read rule); this lets retention
    /// GC reclaim them and raw scans see them deleted. Queued automatically
    /// per branch (`auto_block_overlays`), and by the admin fan-out.
    BlockOverlayTombstones,
    /// Compound index builds that run by themselves (plan Phase 13f): the
    /// built-in workspace indexes not `Ready` on this node, older-format
    /// indexes (unless `RAISIN_COMPOUND_FORMAT_REBUILD=0`), every other
    /// unready declared index of the branch, and the DROP of workspace
    /// indexes no longer declared. Queued automatically per branch
    /// (`compound_builds.rs`), and by the admin fan-out.
    CompoundBuilds,
    /// Give every live node whose NEWEST version has no `created_at` /
    /// `updated_at` the timestamps its history implies (first and newest
    /// revision), through the repository write funnel as the system actor —
    /// a new revision, every derived index maintained, replicated, no node
    /// event (plan Phase 13g). Unblocks compound indexes over a system
    /// timestamp (the built-in `@__children_by_created_at`). Queued
    /// automatically per branch (`auto_timestamps.rs`,
    /// `RAISIN_TIMESTAMP_BACKFILL=0` turns that off), and by the admin
    /// fan-out.
    TimestampBackfill,
    /// Derive `NODE_DELETES` from the branch's `NODES` tombstones (the
    /// entries deletes made before the index existed never wrote), then
    /// stamp the branch `Ready`, which switches its translation reads from
    /// the node-history walk to the index (`crate::node_delete_index`).
    /// Queued automatically per branch (`node_delete_index::auto`,
    /// `RAISIN_NODE_DELETE_INDEX_AUTO=0` turns that off), and by the admin
    /// fan-out.
    NodeDeleteIndex,
}

impl RepairKind {
    /// Stable name, used in the state record key and the job type.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::OrderedChildren => "ordered_children",
            Self::PathTombstone => "path_tombstone",
            Self::NodePath => "node_path",
            Self::PropertyIndex => "property_index",
            Self::PropertyIndexVerify => "property_index_verify",
            Self::CollapseRuns => "collapse_runs",
            Self::ResyncTranslations => "resync_translations",
            Self::LocalizedNames => "localized_names",
            Self::BlockOverlayTombstones => "block_overlay_tombstones",
            Self::CompoundBuilds => "compound_builds",
            Self::TimestampBackfill => "timestamp_backfill",
            Self::NodeDeleteIndex => "node_delete_index",
        }
    }

    /// Every repair, in the order the console lists them.
    pub const ALL: [Self; 12] = [
        Self::OrderedChildren,
        Self::PathTombstone,
        Self::NodePath,
        Self::PropertyIndex,
        Self::PropertyIndexVerify,
        Self::CollapseRuns,
        Self::ResyncTranslations,
        Self::LocalizedNames,
        Self::BlockOverlayTombstones,
        Self::CompoundBuilds,
        Self::TimestampBackfill,
        Self::NodeDeleteIndex,
    ];

    /// The inverse of [`Self::slug`], over [`Self::ALL`].
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.slug() == slug)
    }

    /// The CF the repair writes, which the disk precheck sizes. (Collapse
    /// touches several and prechecks each itself; see `history_gc::collapse`.)
    pub(super) fn column_family(&self) -> &'static str {
        match self {
            Self::CollapseRuns => cf::PROPERTY_INDEX,
            Self::OrderedChildren => cf::ORDERED_CHILDREN,
            Self::PathTombstone => cf::PATH_INDEX,
            Self::NodePath => cf::NODE_PATH,
            Self::PropertyIndex | Self::PropertyIndexVerify => cf::PROPERTY_INDEX,
            // It writes the oplog, sized by what it re-emits.
            Self::ResyncTranslations => cf::TRANSLATION_DATA,
            Self::LocalizedNames => cf::LOCALIZED_NAME_INDEX,
            Self::BlockOverlayTombstones => cf::BLOCK_TRANSLATIONS,
            Self::CompoundBuilds => cf::COMPOUND_INDEX,
            // New node versions (it sizes each chunk's output itself).
            Self::TimestampBackfill => cf::NODES,
            Self::NodeDeleteIndex => cf::NODE_DELETES,
        }
    }

    /// Whether the repair writes index data (the verify only reads).
    pub(super) fn writes(&self) -> bool {
        !matches!(self, Self::PropertyIndexVerify)
    }

    /// Whether the repair INSERTS entries (possibly below existing ones), and
    /// so holds the `(branch, CF)` exclusion against run-collapse while it
    /// runs. Collapse itself only deletes, and takes the exclusive side.
    pub(super) fn inserts(&self) -> bool {
        !matches!(
            self,
            Self::PropertyIndexVerify
                | Self::CollapseRuns
                | Self::ResyncTranslations
                // Ordinary writes through the write funnel, at fresh
                // revisions (or in place, as any `versionable: false` edit).
                | Self::TimestampBackfill
        )
    }
}

#[cfg(test)]
mod tests {
    use super::RepairKind;

    #[test]
    fn every_repair_round_trips_through_its_slug() {
        for kind in RepairKind::ALL {
            assert_eq!(RepairKind::from_slug(kind.slug()), Some(kind));
        }
        assert_eq!(RepairKind::from_slug("no_such_repair"), None);
    }
}
