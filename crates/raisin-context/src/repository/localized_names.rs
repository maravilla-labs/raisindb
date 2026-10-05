//! Localized node names (plan Phase 12): the repository settings of the
//! localized name index.

use serde::{Deserialize, Serialize};

/// Settings of the localized name index.
///
/// A node's name in a locale is its TRANSLATED node name: the reserved
/// overlay pointer `/__node_name` of its translation in that locale (it gets
/// history, forks, copies and deletes from the translation substrate for
/// free). A node without one keeps its canonical name in that locale; the
/// default language never has localized names (owner decision, 2026-10-04:
/// there is no base-property source).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalizedNameConfig {
    /// Refuse a write that gives a node the same effective name as a sibling
    /// in the same locale. Only takes effect on a branch whose index has been
    /// rebuilt with zero collisions; until then collisions resolve
    /// deterministically (the newest claim wins) and are reported.
    ///
    /// Never `skip_serializing_if`: the repository record is msgpack
    /// POSITIONAL (`rmp_serde::to_vec`), so a skipped field shifts every
    /// later one into the wrong slot.
    #[serde(default)]
    pub enforce_unique: bool,
}
