//! Whether the localized name index may answer a read, from a workspace's
//! build state record (`state`).

use super::state::{BuildStatus, LocalizedNameState};
use raisin_hlc::HLC;

/// Whether the index may answer a read of `(branch, workspace)` at `bound`
/// under the repository's current `fingerprint`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Use the index.
    Ready { built_from_rev: Option<HLC> },
    /// Ready, but the read is below `built_from_rev`: fall back.
    BelowBuild { built_from_rev: HLC },
    /// A build is running.
    Building,
    /// No usable build (none, invalidated, or under another configuration).
    NotBuilt,
}

impl Availability {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// Decide [`Availability`] from the stored record.
pub fn availability(
    state: Option<&LocalizedNameState>,
    fingerprint: &str,
    bound: Option<&HLC>,
) -> Availability {
    let Some(state) = state else {
        return Availability::NotBuilt;
    };
    if state.fingerprint != fingerprint {
        return Availability::NotBuilt;
    }
    match state.status {
        BuildStatus::NotBuilt => Availability::NotBuilt,
        BuildStatus::Building => Availability::Building,
        BuildStatus::Ready => match (state.built_from_rev, bound) {
            (Some(built), Some(bound)) if *bound < built => Availability::BelowBuild {
                built_from_rev: built,
            },
            (built, _) => Availability::Ready {
                built_from_rev: built,
            },
        },
    }
}
