//! Per-run bookkeeping for [`super::compaction::SpatialPruneFilter`].
//!
//! Split out of the filter so the state machine's shape is readable on its own.

use crate::keys::ParsedSpatialKey;

/// The `\0`-separated scope of one geohash cell: everything in the key up to and
/// including the geohash.
///
/// Held owned so the filter can compare the next key's borrowed fields against
/// it without allocating on the common "same prefix" path.
#[derive(Debug, Default)]
pub(super) struct CellPrefix {
    tenant_id: String,
    repo_id: String,
    branch: String,
    workspace: String,
    property_name: String,
    geohash: String,
}

impl CellPrefix {
    /// Cheapest-discriminating field first: within one compaction the geohash
    /// changes far more often than the tenant.
    pub(super) fn matches(&self, parsed: &ParsedSpatialKey<'_>) -> bool {
        self.geohash == parsed.geohash
            && self.property_name == parsed.property_name
            && self.workspace == parsed.workspace
            && self.branch == parsed.branch
            && self.repo_id == parsed.repo_id
            && self.tenant_id == parsed.tenant_id
    }

    pub(super) fn adopt(&mut self, parsed: &ParsedSpatialKey<'_>) {
        fn set(dst: &mut String, src: &str) {
            dst.clear();
            dst.push_str(src);
        }
        set(&mut self.tenant_id, parsed.tenant_id);
        set(&mut self.repo_id, parsed.repo_id);
        set(&mut self.branch, parsed.branch);
        set(&mut self.workspace, parsed.workspace);
        set(&mut self.property_name, parsed.property_name);
        set(&mut self.geohash, parsed.geohash);
    }
}

/// What the filter has decided about one node inside the current cell prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NodeState {
    /// `n` entries kept so far. Older ones are still eligible.
    Kept(usize),
    /// Nothing further for this node in this cell survives. Reached either
    /// because retention has been exhausted (every later entry is strictly
    /// older, so both bounds stay violated) or because the node's newest entry
    /// was a droppable tombstone.
    PurgeRest,
}
