//! Reconciling **configured intent** against **local index reality**.
//!
//! This is the module that decides three things, and it is the only one allowed
//! to decide them, because they have to agree with each other:
//!
//! 1. **What precision set a write must emit** ([`resolve_write_policy`]).
//! 2. **Whether a build is owed** ([`WritePolicyResolution::needs_build`]).
//! 3. **What a query may trust while the two disagree**
//!    ([`availability_in_rebuild_window`]).
//!
//! # The bug this exists to kill
//!
//! The write path used to resolve its policy from the LOCAL STATE RECORD — the
//! cache of what the index was last built under — and only consulted the
//! configured policy when no record existed at all. So an operator who changed
//! the precision set through the admin surface changed nothing: every subsequent
//! write kept emitting the old precisions, the state record kept describing them,
//! and the record and the configuration never converged. The state record is for
//! *build phase and progress*; it is **not** the source of truth for
//! configuration.
//!
//! # The rule
//!
//! - Writes reconcile **towards** the configuration, emitting `configured ∪
//!   indexed` while the two differ, so a row written mid-migration is findable
//!   under both sets.
//! - Queries read **reality**, narrowed to what is provably complete (below).
//! - The gap between them is closed by a `SpatialIndexBuild` job, which is the
//!   only thing that flips the state record to the new policy.

use raisin_hlc::HLC;
use raisin_models::nodes::properties::{sorted_precisions, SpatialCoverMode, SpatialPolicy};
use raisin_storage::spatial::{SpatialAvailability, SpatialBuildPhase, SpatialIndexState};

/// Why a `SpatialIndexBuild` is owed for one (workspace, property).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildTrigger {
    /// No local state record at all: data written before spatial indexing
    /// existed, or a geometry property this instance has never indexed.
    NeverBuilt,
    /// A record exists but says the entries are not there (integrity sweep, or a
    /// build that was marked and never ran).
    MarkedNotBuilt,
    /// The configured policy no longer matches what the entries were produced
    /// under. **This is the case reconciliation used to miss entirely**, which is
    /// why an already-built index was never rebuilt after a policy change.
    PolicyChanged {
        indexed_hash: u64,
        configured_hash: u64,
    },
}

impl BuildTrigger {
    /// Whether the job must run in `rebuild` (tombstone-then-re-emit) mode.
    ///
    /// A policy change has to REMOVE the entries the new policy no longer wants;
    /// a gap-fill only has to add the ones that were never written. Getting this
    /// wrong leaves a shrunk precision set with live entries at precisions the
    /// configuration dropped.
    pub fn rebuild(&self) -> bool {
        matches!(self, Self::PolicyChanged { .. })
    }
}

/// Configured intent, local reality, and what a write must do about the gap.
#[derive(Debug, Clone)]
pub struct WritePolicyResolution {
    /// Replicated intent — `WorkspaceConfig.spatial` resolved for this property.
    pub configured: SpatialPolicy,
    /// What the local index currently holds, when a state record exists.
    pub indexed: Option<SpatialPolicy>,
    /// What this write must emit: `configured ∪ indexed` (see
    /// [`union_policy`]).
    pub write: SpatialPolicy,
    /// Why a build is owed, if one is.
    pub needs_build: Option<BuildTrigger>,
}

/// Resolve the write policy from CONFIGURED intent, using the local state record
/// only for what it is actually for.
///
/// `state` is the local build record. It contributes the precisions that already
/// have entries (so the union keeps them findable) and the build phase — never
/// the configuration.
pub fn resolve_write_policy(
    configured: SpatialPolicy,
    state: Option<&SpatialIndexState>,
) -> WritePolicyResolution {
    let Some(state) = state else {
        return WritePolicyResolution {
            write: configured.clone(),
            indexed: None,
            needs_build: Some(BuildTrigger::NeverBuilt),
            configured,
        };
    };

    let indexed = state.policy();
    let configured_hash = configured.policy_hash();
    let indexed_hash = state.policy_hash;

    let needs_build = if indexed_hash != configured_hash {
        Some(BuildTrigger::PolicyChanged {
            indexed_hash,
            configured_hash,
        })
    } else if state.phase == SpatialBuildPhase::NotBuilt {
        Some(BuildTrigger::MarkedNotBuilt)
    } else {
        None
    };

    let write = if indexed_hash == configured_hash {
        configured.clone()
    } else {
        union_policy(&configured, &indexed)
    };

    WritePolicyResolution {
        configured,
        indexed: Some(indexed),
        write,
        needs_build,
    }
}

/// The policy that keeps a row findable under BOTH the old and the new
/// configuration for the duration of the migration window.
///
/// - **Precisions** are the union: a query planned at either set finds the row.
/// - **Cover** widens to `Extent` if either side uses it: a centroid-only write
///   would leave a large geometry unreachable from a query that only touches its
///   edge cells, which is a missing result, not a slow one.
/// - **SRID default** and **bucket property** come from the configuration, which
///   is the value the rebuild will settle on. The bucket is a *value*-side
///   discriminator, and the planner is told to stop trusting it for the duration
///   of the window (see [`availability_in_rebuild_window`]), so a half-migrated
///   bucket cannot reject a real match.
pub fn union_policy(configured: &SpatialPolicy, indexed: &SpatialPolicy) -> SpatialPolicy {
    let mut precisions = configured.precisions.clone();
    precisions.extend(indexed.precisions.iter().copied());
    SpatialPolicy {
        precisions: sorted_precisions(precisions),
        srid: configured.srid,
        bucket_property: configured.bucket_property.clone(),
        cover: if configured.cover == SpatialCoverMode::Extent
            || indexed.cover == SpatialCoverMode::Extent
        {
            SpatialCoverMode::Extent
        } else {
            SpatialCoverMode::Centroid
        },
    }
}

/// What a query may trust while local reality and configured intent disagree.
///
/// # The decision, stated once
///
/// **A rebuild serves from the OLD cells until the new ones are complete, and
/// fails LOUDLY (falls back to a scan) when no precision is complete.** Silent
/// partial results are never acceptable.
///
/// Concretely:
///
/// * **Phase `Ready`, hashes differ** (policy changed, build not started or not
///   yet picked up): every row is present at the *indexed* precisions — new
///   writes emit `configured ∪ indexed` — so the indexed set stays complete and
///   is what the planner gets.
/// * **Phase `Building`, hashes differ**: the job tombstones each node's existing
///   entries before re-emitting them at the new policy, so a node already
///   processed no longer has entries at precisions the new policy dropped, while
///   a node not yet processed has none at precisions the new policy added.
///   The only precisions that are complete for BOTH are the **intersection**, so
///   that is what the planner gets.
/// * **Intersection empty** (a rebuild from, say, `[8,7]` to `[5,4]`): no
///   precision is complete, so the answer is [`SpatialAvailability::Unusable`],
///   which makes the planner retain the spatial predicate and take an ordinary
///   access path. Slow and correct, never fast and wrong.
/// * **Bucket property changed**: entries written before and after the change
///   carry *different* discriminators, so a `bucket_eq` pre-filter built from
///   either one would reject real matches. The bucket is therefore withheld for
///   the duration of the window, which only costs the pre-filter optimisation —
///   the original predicate is always in the residual filter.
///
/// The window closes when the build job stamps the new policy into the state
/// record, at which point hashes match and this function is a pass-through.
pub fn availability_in_rebuild_window(
    state: &SpatialIndexState,
    configured: &SpatialPolicy,
) -> SpatialAvailability {
    let base = state.availability();
    let SpatialAvailability::Ready {
        precisions,
        built_through,
        bucket_property,
    } = base
    else {
        return base;
    };

    if state.policy_hash == configured.policy_hash() {
        return SpatialAvailability::Ready {
            precisions,
            built_through,
            bucket_property,
        };
    }

    // Only trust the bucket while both sides agree on which property it is.
    let bucket_property = if state.bucket_property == configured.bucket_property {
        bucket_property
    } else {
        None
    };

    if state.phase != SpatialBuildPhase::Building {
        // Not rebuilding yet: writes emit `configured ∪ indexed`, so the indexed
        // precisions are still a complete set.
        return SpatialAvailability::Ready {
            precisions,
            built_through,
            bucket_property,
        };
    }

    let common = intersect_precisions(&precisions, &configured.precisions);
    if common.is_empty() {
        return SpatialAvailability::Unusable(format!(
            "spatial index is being rebuilt from precisions {:?} to {:?}; the two sets \
             are disjoint, so no precision is complete until the build finishes",
            precisions, configured.precisions
        ));
    }

    SpatialAvailability::Ready {
        precisions: common,
        built_through,
        bucket_property,
    }
}

/// Whether [`availability_in_rebuild_window`] could return anything other than
/// the record's own [`SpatialIndexState::availability`].
///
/// Lets the query path skip the workspace-record lookup that resolving the
/// configured policy costs. Two cases can change the answer, and neither is the
/// steady state:
///
/// * a build is in progress, so the entry set may be a mixture of two policies;
/// * a bucket property is in play, so a `bucket_eq` pre-filter could be built
///   from a discriminator the entries no longer carry.
///
/// Everything else answers from the record alone: while the configuration and
/// the index disagree but no build is running, writes emit `configured ∪
/// indexed`, so the record's own precisions remain a complete set.
pub fn needs_configured_check(state: &SpatialIndexState) -> bool {
    state.phase == SpatialBuildPhase::Building || state.bucket_property.is_some()
}

/// Precisions present in both sets, finest first.
fn intersect_precisions(a: &[usize], b: &[usize]) -> Vec<usize> {
    let mut out: Vec<usize> = a.iter().copied().filter(|p| b.contains(p)).collect();
    out.sort_unstable_by(|x, y| y.cmp(x));
    out.dedup();
    out
}

/// A fresh `Ready` state record for a property being indexed for the first time.
///
/// Thin wrapper kept here so the "first write creates the record from CONFIGURED
/// intent" rule lives beside the rest of the reconciliation rules.
pub fn initial_state(configured: &SpatialPolicy, revision: HLC) -> SpatialIndexState {
    SpatialIndexState::ready(configured, revision)
}

#[cfg(test)]
mod tests;
