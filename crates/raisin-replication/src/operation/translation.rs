//! The state of one translation version on the wire (plan Phase 11).

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleOverlay};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// What one `TRANSLATION_DATA` / `BLOCK_TRANSLATIONS` version holds.
///
/// Three states, not two: `Hidden` is a stored overlay (the node is hidden in
/// the locale) and `Deleted` is the `T` tombstone (the locale has no overlay
/// at all, so the fallback chain applies). The pre-Phase-11 op folded both
/// into one `delete_translation`, so a replica could not tell them apart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReplicatedOverlay {
    /// A properties overlay.
    Properties {
        data: HashMap<JsonPointer, PropertyValue>,
    },
    /// The node is hidden in this locale.
    Hidden,
    /// The version is a tombstone.
    Deleted,
}

impl ReplicatedOverlay {
    /// `None` is a tombstone.
    pub fn from_stored(overlay: Option<&LocaleOverlay>) -> Self {
        match overlay {
            Some(LocaleOverlay::Properties { data }) => Self::Properties { data: data.clone() },
            Some(LocaleOverlay::Hidden) => Self::Hidden,
            None => Self::Deleted,
        }
    }

    /// The stored form: `None` is a tombstone.
    pub fn into_stored(self) -> Option<LocaleOverlay> {
        match self {
            Self::Properties { data } => Some(LocaleOverlay::Properties { data }),
            Self::Hidden => Some(LocaleOverlay::Hidden),
            Self::Deleted => None,
        }
    }
}
