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
    /// Highest revision covered by the build.
    pub built_through: HLC,
    /// Build progress.
    pub phase: CompoundBuildPhase,
    /// Nodes written during the last build. Diagnostics only — a node missing a
    /// value for any indexed column is silently dropped from the index
    /// (`crud/indexing/compound_indexes.rs`), so this being lower than the node
    /// count is expected, not an error.
    pub nodes_indexed: u64,
}

impl CompoundIndexState {
    pub const VERSION: u8 = 1;

    /// A fresh `Ready` record for a declaration that has just been built.
    pub fn ready(definition: &CompoundIndexDefinition, built_through: HLC) -> Self {
        Self {
            v: Self::VERSION,
            index_name: definition.name.clone(),
            definition_hash: definition.definition_hash(),
            built_through,
            phase: CompoundBuildPhase::Ready,
            nodes_indexed: 0,
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
mod tests {
    use super::*;
    use raisin_models::nodes::properties::schema::{CompoundColumnType, CompoundIndexColumn};

    fn def(name: &str, cols: &[(&str, CompoundColumnType)]) -> CompoundIndexDefinition {
        CompoundIndexDefinition {
            name: name.to_string(),
            columns: cols
                .iter()
                .map(|(p, t)| CompoundIndexColumn {
                    property: p.to_string(),
                    column_type: t.clone(),
                    ascending: None,
                })
                .collect(),
            has_order_column: false,
        }
    }

    #[test]
    fn a_ready_record_matching_its_declaration_is_ready() {
        let d = def("idx", &[("status", CompoundColumnType::String)]);
        let state = CompoundIndexState::ready(&d, HLC::new(7, 0));
        assert!(state.availability_for(&d).is_ready());
    }

    /// The case the record exists for. Entries in the keyspace were written
    /// under the OLD columns; reading them through the new layout is silent
    /// corruption, so a changed declaration must read as unusable.
    #[test]
    fn a_changed_declaration_makes_the_build_unusable() {
        let built = def("idx", &[("status", CompoundColumnType::String)]);
        let state = CompoundIndexState::ready(&built, HLC::new(7, 0));

        let reordered = def(
            "idx",
            &[
                ("buyer", CompoundColumnType::String),
                ("status", CompoundColumnType::String),
            ],
        );
        match state.availability_for(&reordered) {
            CompoundAvailability::Unusable(reason) => {
                assert!(reason.contains("different declaration"), "{reason}");
            }
            other => panic!("expected Unusable, got {other:?}"),
        }
    }

    /// Column ORDER is identity: `(a, b)` and `(b, a)` produce different key
    /// bytes, so they must not share a fingerprint.
    #[test]
    fn column_order_changes_the_fingerprint() {
        let ab = def(
            "idx",
            &[
                ("a", CompoundColumnType::String),
                ("b", CompoundColumnType::String),
            ],
        );
        let ba = def(
            "idx",
            &[
                ("b", CompoundColumnType::String),
                ("a", CompoundColumnType::String),
            ],
        );
        assert_ne!(ab.definition_hash(), ba.definition_hash());
    }

    /// A type change alters the ENCODING (`Integer` is big-endian bytes,
    /// `String` is UTF-8), so it must invalidate the build too.
    #[test]
    fn column_type_changes_the_fingerprint() {
        let as_string = def("idx", &[("qty", CompoundColumnType::String)]);
        let as_int = def("idx", &[("qty", CompoundColumnType::Integer)]);
        assert_ne!(as_string.definition_hash(), as_int.definition_hash());
    }

    /// A rebuild CLEARS the keyspace before writing, so mid-build there is no
    /// complete generation to serve — unlike spatial, where `Building` stays
    /// queryable against the older entries.
    #[test]
    fn a_building_record_is_not_queryable() {
        let d = def("idx", &[("status", CompoundColumnType::String)]);
        let mut state = CompoundIndexState::ready(&d, HLC::new(7, 0));
        state.phase = CompoundBuildPhase::Building;
        assert!(!state.availability_for(&d).is_ready());
    }

    #[test]
    fn an_unsupported_record_version_is_unusable() {
        let d = def("idx", &[("status", CompoundColumnType::String)]);
        let mut state = CompoundIndexState::ready(&d, HLC::new(7, 0));
        state.v = CompoundIndexState::VERSION + 1;
        assert!(matches!(
            state.availability_for(&d),
            CompoundAvailability::Unusable(_)
        ));
    }
}
