// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Translation staleness tracking via content hashing.
//!
//! This module provides types for detecting when original content has changed
//! since a translation was created, indicating the translation may be stale.
//!
//! # Problem
//!
//! When the original language content changes (fields added, removed, or modified),
//! existing translations become stale but the system needs to detect this:
//!
//! - A new field is added → translator doesn't see it needs translation
//! - A field is edited → existing translation may no longer match the original's intent
//! - Array items are reordered/inserted → translations misalign by index
//!
//! # Solution
//!
//! Store a hash of the original content alongside each translation pointer.
//! On read, compare stored hash vs current original hash to detect staleness.
//!
//! # Example
//!
//! ```rust
//! use raisin_models::translations::{TranslationHashRecord, JsonPointer};
//! use chrono::Utc;
//!
//! let record = TranslationHashRecord::new(
//!     "a1b2c3d4e5f6...".to_string(),
//!     12345,
//! );
//!
//! // Later, when checking staleness
//! let current_hash = "f6e5d4c3b2a1...".to_string();
//! if record.original_hash != current_hash {
//!     println!("Translation is stale!");
//! }
//! ```

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Tracks the original content hash at translation time for staleness detection.
///
/// When a translation is saved, the system records the hash of the original
/// (base language) content that was being translated from. Later, when
/// displaying the translation, the current hash of the original can be
/// compared to detect if the original has changed since the translation.
///
/// # Fields
///
/// - `original_hash`: SHA-256 hash of the original field value when translation was created
/// - `original_revision`: Revision of the original node when translation was created
/// - `recorded_at`: Timestamp when this hash was recorded
///
/// # Storage
///
/// Hash records are stored separately from translation overlays, allowing
/// them to be added without changing the existing translation storage format.
///
/// # Hashing Strategy
///
/// Values are hashed by serializing to canonical JSON and computing SHA-256.
/// This ensures consistent hashing across different serialization orders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct TranslationHashRecord {
    /// SHA-256 hash of the original field value when translation was created.
    ///
    /// The hash is computed by serializing the `PropertyValue` to canonical JSON
    /// and hashing the resulting bytes. This ensures consistent comparison.
    pub original_hash: String,

    /// Revision of the original node when translation was created.
    ///
    /// Used for informational purposes and debugging - helps understand
    /// which version of the original was being translated from.
    pub original_revision: raisin_hlc::HLC,

    /// Timestamp when this hash was recorded.
    ///
    /// Typically matches the timestamp of the translation update, but
    /// stored separately for clarity.
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

impl TranslationHashRecord {
    /// Create a new TranslationHashRecord.
    ///
    /// # Arguments
    ///
    /// * `original_hash` - SHA-256 hash of the original content
    /// * `original_revision` - Revision of the source node
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use raisin_models::translations::TranslationHashRecord;
    ///
    /// let record = TranslationHashRecord::new(
    ///     "abc123def456...".to_string(),
    ///     raisin_hlc::HLC::new(12345, 0),
    /// );
    ///
    /// assert_eq!(record.original_hash, "abc123def456...");
    /// assert_eq!(record.original_revision, raisin_hlc::HLC::new(12345, 0));
    /// ```
    pub fn new(original_hash: String, original_revision: raisin_hlc::HLC) -> Self {
        TranslationHashRecord {
            original_hash,
            original_revision,
            recorded_at: chrono::Utc::now(),
        }
    }

    /// Create a TranslationHashRecord with a custom timestamp.
    ///
    /// Useful for importing historical data or testing.
    ///
    /// # Arguments
    ///
    /// * `original_hash` - SHA-256 hash of the original content
    /// * `original_revision` - Revision of the source node
    /// * `recorded_at` - Custom timestamp
    pub fn with_timestamp(
        original_hash: String,
        original_revision: raisin_hlc::HLC,
        recorded_at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        TranslationHashRecord {
            original_hash,
            original_revision,
            recorded_at,
        }
    }

    /// Check if this record indicates the original has changed.
    ///
    /// Compares the stored hash against the current hash of the original content.
    ///
    /// # Arguments
    ///
    /// * `current_hash` - Current SHA-256 hash of the original content
    ///
    /// # Returns
    ///
    /// `true` if the hashes differ (content changed), `false` if they match.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::TranslationHashRecord;
    ///
    /// let record = TranslationHashRecord::new("hash1".to_string(), 1);
    ///
    /// assert!(!record.is_stale("hash1"));  // Same hash = fresh
    /// assert!(record.is_stale("hash2"));   // Different hash = stale
    /// ```
    #[inline]
    pub fn is_stale(&self, current_hash: &str) -> bool {
        self.original_hash != current_hash
    }
}

/// Report of translation staleness for a node.
///
/// Contains categorized information about which translations are fresh,
/// stale, or missing for a given locale.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct StalenessReport {
    /// Fields with matching hash (translation is up-to-date).
    pub fresh_fields: Vec<String>,

    /// Fields where the original has changed since translation.
    pub stale_fields: Vec<StaleFieldInfo>,

    /// Fields in the original that have no translation.
    pub missing_fields: Vec<MissingFieldInfo>,

    /// Fields where staleness couldn't be determined (no hash record).
    pub unknown_fields: Vec<String>,
}

impl StalenessReport {
    /// Create a new empty StalenessReport.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if there are any stale or missing fields.
    pub fn has_issues(&self) -> bool {
        !self.stale_fields.is_empty() || !self.missing_fields.is_empty()
    }

    /// Get total count of fields that need attention.
    pub fn issues_count(&self) -> usize {
        self.stale_fields.len() + self.missing_fields.len()
    }
}

/// Information about a stale translation field.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StaleFieldInfo {
    /// JSON Pointer to the field.
    pub pointer: String,

    /// Hash of the original when translation was created.
    pub original_hash_at_translation: String,

    /// Current hash of the original field.
    pub current_original_hash: String,

    /// When the translation was created/updated.
    pub translated_at: chrono::DateTime<chrono::Utc>,
}

/// Information about a field that needs translation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MissingFieldInfo {
    /// JSON Pointer to the field.
    pub pointer: String,

    /// Current hash of the original field (for tracking if/when translated).
    pub current_original_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_record_creation() {
        let record = TranslationHashRecord::new("abc123".to_string(), raisin_hlc::HLC::new(42, 0));
        assert_eq!(record.original_hash, "abc123");
        assert_eq!(record.original_revision, raisin_hlc::HLC::new(42, 0));
    }

    #[test]
    fn test_staleness_check() {
        let record = TranslationHashRecord::new("hash1".to_string(), raisin_hlc::HLC::new(1, 0));

        assert!(!record.is_stale("hash1"), "Same hash should be fresh");
        assert!(record.is_stale("hash2"), "Different hash should be stale");
    }

    #[test]
    fn test_staleness_report() {
        let mut report = StalenessReport::new();
        assert!(!report.has_issues());
        assert_eq!(report.issues_count(), 0);

        report.stale_fields.push(StaleFieldInfo {
            pointer: "/title".to_string(),
            original_hash_at_translation: "old".to_string(),
            current_original_hash: "new".to_string(),
            translated_at: chrono::Utc::now(),
        });

        assert!(report.has_issues());
        assert_eq!(report.issues_count(), 1);
    }
}
