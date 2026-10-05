//! Build state for compound (multi-column) indexes.
//!
//! The sibling of [`crate::spatial`]'s availability machinery, and it exists for
//! the same reason. `IndexCatalog::has_compound_index()` used to answer a
//! hardcoded `true` that no planner decision even read; the real gate was
//! "the NodeType declares one", which says nothing about whether the entries
//! exist, cover every node, or were written under the columns the declaration
//! now lists.
//!
//! A compound index is more exposed to this than most, because **the index name
//! addresses a workspace-global keyspace** — the key is
//! `…cidx\0{index_name}\0{column values}…` and carries no node type. Change a
//! column and the new entries land in the same keyspace as the old ones,
//! interleaved and mutually unintelligible. Nothing reconciles them, so
//! "declared" and "usable" are genuinely different questions.

use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;

/// What the local compound index can actually answer for one
/// (workspace, index name).
///
/// **Fails closed on purpose.** Anything other than [`CompoundAvailability::Ready`]
/// must make the planner keep the predicates and take an ordinary access path.
/// That is not a performance preference: the planner STRIPS the matched equality
/// predicates from the residual filter when it chooses a `CompoundIndexScan`, so
/// an index that is empty or stale does not merely run slowly — it returns the
/// wrong rows, with no filter left downstream to catch it.
#[derive(Debug, Clone, PartialEq)]
pub enum CompoundAvailability {
    /// The index is built and may be trusted as a complete access path.
    Ready {
        /// Highest revision covered by the build.
        built_through: HLC,
        /// Fingerprint of the declaration the entries were written under.
        definition_hash: u64,
    },
    /// No state record exists. Either never built, or built by a binary that
    /// predates this record — indistinguishable, and both mean "do not trust it".
    NotBuilt,
    /// A record exists but cannot be used. Carries the reason so `EXPLAIN` can
    /// print something an operator can act on.
    Unusable(String),
}

impl CompoundAvailability {
    /// Whether the index may be trusted as a complete access path.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// A short human-readable reason, for `EXPLAIN` and for the warn logs that
    /// fire when a declared index is passed over.
    pub fn explain_reason(&self) -> String {
        match self {
            Self::Ready { .. } => "ready".to_string(),
            Self::NotBuilt => {
                "no build state recorded for this compound index; a rebuild has been requested"
                    .to_string()
            }
            Self::Unusable(reason) => reason.clone(),
        }
    }

    /// This availability for a read AT `read` (`None`: the branch HEAD).
    ///
    /// `built_through` is the build's HISTORY FLOOR: a build writes each
    /// node's version as of the branch HEAD it read (and every version above
    /// it), never the history below — the tombstones that ended superseded
    /// tuples and every older version's entries are gone. A read pinned below
    /// the floor is therefore `Unusable` (the planner keeps the predicates and
    /// scans). A HEAD read is never below it: HEAD only advances.
    pub fn at_revision(self, read: Option<&HLC>) -> Self {
        match (&self, read) {
            (Self::Ready { built_through, .. }, Some(at)) if at < built_through => {
                Self::Unusable(format!(
                    "compound index history starts at its build ({built_through}); \
                     a read at {at} is answered by a scan"
                ))
            }
            _ => self,
        }
    }
}

/// How far along a build is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompoundBuildPhase {
    /// Entries are complete for `definition_hash` through `built_through`.
    Ready,
    /// A rebuild is running.
    ///
    /// Unlike spatial — where `Building` still describes a complete OLD entry
    /// set and stays queryable — a compound rebuild CLEARS the keyspace before
    /// it writes (`rebuild_compound_indexes` does a prefix delete first). So
    /// mid-rebuild there is no complete entry set to fall back on, and this
    /// phase is NOT queryable.
    Building,
    /// Known to be absent or invalidated.
    NotBuilt,
}

/// The persisted record, one per (tenant, repo, branch, workspace, index name).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CompoundIndexState {
    /// Record format version.
    pub v: u8,
    /// The index this describes.
    pub index_name: String,
    /// [`CompoundIndexDefinition::definition_hash`] of the declaration the
    /// entries were produced from. A mismatch against the current declaration
    /// means the keyspace holds entries under a layout the planner would
    /// misread.
    pub definition_hash: u64,
    /// The build's history floor: the branch HEAD the build read. Entries are
    /// complete for reads at or above it, and only for those (see
    /// [`CompoundAvailability::at_revision`]).
    pub built_through: HLC,
    /// Build progress.
    pub phase: CompoundBuildPhase,
    /// Nodes written during the last build. Diagnostics only — a node missing a
    /// value for any indexed column is silently dropped from the index
    /// (`crud/indexing/compound_indexes.rs`), so this being lower than the node
    /// count is expected, not an error.
    pub nodes_indexed: u64,
    /// How many times this node applied a write it could NOT index into this
    /// keyspace (a replicated upsert or a merge, neither of which writes
    /// compound entries yet) and therefore marked the index `NotBuilt`.
    ///
    /// Only ever increases. A build remembers the value it started under and
    /// stamps `Ready` only if it is unchanged when it finishes: a marker set
    /// DURING the build is for a write the build may not have seen, and a
    /// `Ready` written over it would serve that write's stale entries. A
    /// counter, not the applying revision, because replication applies
    /// revisions out of order — an older revision arriving mid-build would not
    /// move a revision-valued marker forward, and the build would not notice.
    ///
    /// Appended with a default so records written before it decode as `0`.
    #[serde(default)]
    pub stale_generation: u64,
}

impl CompoundIndexState {
    /// Record format version. **2** (plan Phase 8): an index built by an
    /// earlier writer is UNUSABLE until rebuilt. Phase 8's writer DERIVES a
    /// node's old entries from its stored version instead of scanning for
    /// them, so every entry must be one that derivation reproduces — and the
    /// earlier writers stored date-like strings in their raw spelling (the
    /// derivation now encodes them canonically, `CompoundColumnValue::text`)
    /// and overwrote superseded entries in place (no history for the
    /// revision-bounded reader). A v1 record reads `Unusable` (a scan) until
    /// the index is rebuilt — by an admin `REBUILD`, or by the sweeps when
    /// `RAISIN_COMPOUND_FORMAT_REBUILD` is on (see [`Self::is_format_upgrade`]).
    pub const VERSION: u8 = 2;

    /// Whether this record is from an older format: the index is unusable
    /// only because the binary moved on, not because anything marked it. The
    /// sweeps leave such an index to an admin rebuild by default — a format
    /// bump must not rebuild every index on every node at boot.
    pub fn is_format_upgrade(&self) -> bool {
        self.v < Self::VERSION
    }

    /// A fresh `Ready` record for a declaration that has just been built.
    pub fn ready(definition: &CompoundIndexDefinition, built_through: HLC) -> Self {
        Self {
            v: Self::VERSION,
            index_name: definition.name.clone(),
            definition_hash: definition.definition_hash(),
            built_through,
            phase: CompoundBuildPhase::Ready,
            nodes_indexed: 0,
            stale_generation: 0,
        }
    }

    /// The availability this record implies, considering the record ALONE.
    ///
    /// The caller still has to compare `definition_hash` against the CURRENT
    /// declaration — see [`Self::availability_for`] — because a record cannot
    /// know that the schema moved underneath it.
    pub fn availability(&self) -> CompoundAvailability {
        if self.v != Self::VERSION {
            return CompoundAvailability::Unusable(format!(
                "compound index state record version {} is not supported (expected {})",
                self.v,
                Self::VERSION
            ));
        }
        match self.phase {
            CompoundBuildPhase::Ready => CompoundAvailability::Ready {
                built_through: self.built_through,
                definition_hash: self.definition_hash,
            },
            // See `CompoundBuildPhase::Building` — the keyspace is cleared
            // before a rebuild writes, so there is nothing complete to serve.
            CompoundBuildPhase::Building => CompoundAvailability::Unusable(
                "compound index is being rebuilt; its entries are incomplete until it finishes"
                    .to_string(),
            ),
            CompoundBuildPhase::NotBuilt => CompoundAvailability::NotBuilt,
        }
    }

    /// The availability of this record given the declaration currently in force.
    ///
    /// This is the check that catches the case the whole module exists for: the
    /// declaration changed, so the entries in the keyspace describe a different
    /// key layout than the planner is about to build a prefix for.
    pub fn availability_for(&self, current: &CompoundIndexDefinition) -> CompoundAvailability {
        let base = self.availability();
        if !base.is_ready() {
            return base;
        }
        let expected = current.definition_hash();
        if expected != self.definition_hash {
            return CompoundAvailability::Unusable(format!(
                "compound index '{}' was built from a different declaration \
                 (built {:#x}, declared {:#x}); a rebuild is required before it can be used",
                self.index_name, self.definition_hash, expected
            ));
        }
        base
    }
}

/// The read port the planner consults. Object-safe so the catalog can hold it
/// as `Arc<dyn …>` without knowing the backend.
pub trait CompoundStateSource: Send + Sync {
    /// Availability for one index, given the declaration currently in force.
    ///
    /// Implementations MUST fail closed: a missing record, an unreadable record
    /// or a storage error all resolve to something that is not `Ready`.
    fn compound_availability(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        definition: &CompoundIndexDefinition,
    ) -> CompoundAvailability;
}

#[cfg(test)]
#[path = "compound_tests.rs"]
mod tests;
