//! COMPOUND_INDEX: one derivation, one writer, one delete tombstoner, one
//! definitions cache (plan Phase 8).
//!
//! - [`entries`] — a version's entries (`group` = key minus revision and node)
//!   and the key grammar every reader parses with.
//! - [`writer`] — [`write_compound_delta`]: old tuple derived from the
//!   baseline, tombstone at the write revision, unchanged tuples skipped
//!   under a proven predecessor. No workspace scan, no in-place overwrite.
//! - [`delete`] — the delete tombstoner's compound part.
//! - [`build`] — the one build pass (job and REBUILD): re-derives the
//!   keyspace as of a HEAD floor the planner then refuses reads below.
//! - [`keyspace`] — one index's keyspace as a unit: the process-wide lock
//!   every builder takes, and the clear.
//! - [`defs`] — node-type definitions (compound + unique), cached off the
//!   apply hot path; [`cold`] — what an apply path does when they are not.
//! - [`workspace_defs`] — WORKSPACE-owned declarations (plan Phase 13e): every
//!   node of the workspace, whatever its type. Added by the writer, the delete
//!   tombstoner and the builds themselves, never by their callers.
//!
//! The reader (`repositories/compound_index.rs`) decides newest-per-`(tuple,
//! node)` at or below the read revision, so a tombstone of one tuple never
//! hides the node's other tuple and history reads see the entry as of then.

pub mod build;
mod build_gate;
mod cache;
pub mod cold;
pub mod defs;
pub mod delete;
pub mod entries;
pub mod group;
pub mod keyspace;
mod pace;
mod refresh;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;
mod wanted;
pub mod workspace_defs;
pub mod writer;

pub use defs::{DefsSet, TypeIndexDefs};
pub use delete::tombstone_compound_for_delete;
pub use entries::{compound_entries, entry_key, parse_entry_key, CompoundGroup};
pub use group::compound_group_walks_capped;
pub use writer::{
    skipped_unchanged_compound_entries, tombstone_superseded_compound, types_of,
    write_compound_delta, writes_compound, CompoundCounts, LIVE,
};
