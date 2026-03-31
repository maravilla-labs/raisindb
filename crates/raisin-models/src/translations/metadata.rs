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

//! Translation metadata and revision tracking.
//!
//! This module provides metadata structures for tracking translation changes
//! over time, similar to how Git tracks commits. Each translation update
//! creates a [`TranslationMeta`] entry that records:
//!
//! - The locale being translated
//! - The revision number (HLC timestamp)
//! - The parent revision (for change history)
//! - Who made the change and when
//! - A commit message describing the change
//!
//! # Design Philosophy
//!
//! The metadata system follows a Git-like model:
//!
//! - **Revision Chain**: Each translation change links to its parent
//! - **Actor Attribution**: Every change records who made it
//! - **Temporal Ordering**: HLC timestamps ensure consistent ordering
//! - **Audit Trail**: Full history of translation changes
//!
//! # Use Cases
//!
//! - **Translation History**: Track who translated what and when
//! - **Change Review**: Review translation changes before publishing
//! - **Rollback**: Revert to previous translation versions
//! - **Audit Compliance**: Meet regulatory requirements for change tracking
//! - **Conflict Resolution**: Resolve concurrent translation updates
//!
//! # Integration with Revision System
//!
//! Translation metadata is stored alongside the translation overlays in
//! RocksDB. Each locale's translation history is maintained separately,
//! allowing independent evolution of different locales.
//!
//! # Examples
//!
//! ## Creating translation metadata
//!
//! ```rust
//! use raisin_models::translations::{TranslationMeta, LocaleCode};
//!
//! let locale = LocaleCode::parse("fr-FR").unwrap();
//! let meta = TranslationMeta::new(
//!     locale,
//!     42,                              // revision number (HLC)
//!     Some(41),                         // parent revision
//!     "translator@example.com".to_string(),
//!     "Add French translation for product page".to_string(),
//! );
//!
//! assert_eq!(meta.revision, 42);
//! assert!(!meta.is_system);
//! ```
//!
//! ## System-generated metadata
//!
//! ```rust
//! use raisin_models::translations::{TranslationMeta, LocaleCode};
//!
//! let locale = LocaleCode::parse("en").unwrap();
//! let meta = TranslationMeta::system(
//!     locale,
//!     1,
//!     "Initial translation setup".to_string(),
//! );
//!
//! assert_eq!(meta.actor, "system");
//! assert!(meta.is_system);
//! ```

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::LocaleCode;

/// Metadata for a translation revision.
///
/// Records information about a translation change, including who made it,
/// when, and why. This provides a full audit trail for translation updates
/// similar to Git commits.
///
/// # Fields
///
/// - `locale`: The locale this translation applies to
/// - `revision`: HLC timestamp uniquely identifying this change
/// - `parent_revision`: Previous translation state (if any)
/// - `timestamp`: When the translation was made
/// - `actor`: Who made the translation (user ID, email, etc.)
/// - `message`: Human-readable description of the change
/// - `is_system`: Whether this was a system-generated change
///
/// # HLC Timestamps
///
/// Revisions use Hybrid Logical Clock (HLC) timestamps, which provide:
/// - **Total ordering**: All events can be ordered consistently
/// - **Causality**: Parent-child relationships are preserved
/// - **Distribution**: Works across multiple nodes in a cluster
///
/// # Storage
///
/// TranslationMeta is serialized using MessagePack and stored in RocksDB
/// with keys that allow efficient:
/// - Lookup by locale and revision
/// - Enumeration of all revisions for a locale
/// - Retrieval of the latest revision
///
/// # Examples
///
/// ## User translation
///
/// ```rust
/// use raisin_models::translations::{TranslationMeta, LocaleCode};
///
/// let locale = LocaleCode::parse("es").unwrap();
/// let meta = TranslationMeta::new(
///     locale,
///     100,
///     Some(99),
///     "maria@example.com".to_string(),
///     "Translate product descriptions".to_string(),
/// );
///
/// assert_eq!(meta.revision, 100);
/// assert_eq!(meta.parent_revision, Some(99));
/// assert!(!meta.is_system);
/// ```
///
/// ## System translation
///
/// ```rust
/// use raisin_models::translations::{TranslationMeta, LocaleCode};
///
/// let locale = LocaleCode::parse("en").unwrap();
/// let meta = TranslationMeta::system(
///     locale,
///     1,
///     "Initialize base language".to_string(),
/// );
///
/// assert_eq!(meta.actor, "system");
/// assert!(meta.is_system);
/// assert!(meta.parent_revision.is_none());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TranslationMeta {
    /// Locale of this translation.
    ///
    /// Identifies which language/region this translation applies to.
    pub locale: LocaleCode,

    /// Revision number allocated for this translation change.
    ///
    /// Uses HLC (Hybrid Logical Clock) timestamp for distributed consistency.
    /// Higher values are newer revisions.
    pub revision: raisin_hlc::HLC,

    /// Parent revision (previous translation state).
    ///
    /// `None` for initial translations or when history is truncated.
    /// Otherwise points to the previous revision of this locale's translation.
    pub parent_revision: Option<raisin_hlc::HLC>,

    /// Timestamp of the translation update.
    ///
    /// Records when the translation was made in wall-clock time.
    /// For audit and display purposes.
    pub timestamp: chrono::DateTime<chrono::Utc>,

    /// Actor who made the translation.
    ///
    /// Typically a user ID, email address, or service account name.
    /// For system changes, this is "system".
    pub actor: String,

    /// Commit message describing the translation change.
    ///
    /// Human-readable description of what was translated or changed.
    /// Similar to a Git commit message.
    pub message: String,

    /// Whether this is a system-generated translation.
    ///
    /// `true` for automated or system changes, `false` for user changes.
    /// Useful for filtering or displaying translation history.
    #[serde(default)]
    pub is_system: bool,
}

impl TranslationMeta {
    /// Create a new TranslationMeta for a user-generated translation.
    ///
    /// # Arguments
    ///
    /// * `locale` - The locale being translated
    /// * `revision` - HLC timestamp for this change
    /// * `parent_revision` - Previous revision (if any)
    /// * `actor` - Who made the translation (user ID, email, etc.)
    /// * `message` - Description of the translation change
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::{TranslationMeta, LocaleCode};
    ///
    /// let locale = LocaleCode::parse("de-DE").unwrap();
    /// let meta = TranslationMeta::new(
    ///     locale,
    ///     42,
    ///     Some(41),
    ///     "hans@example.com".to_string(),
    ///     "Add German translations for UI".to_string(),
    /// );
    ///
    /// assert_eq!(meta.revision, 42);
    /// assert!(!meta.is_system);
    /// ```
    pub fn new(
        locale: LocaleCode,
        revision: raisin_hlc::HLC,
        parent_revision: Option<raisin_hlc::HLC>,
        actor: String,
        message: String,
    ) -> Self {
        TranslationMeta {
            locale,
            revision,
            parent_revision,
            timestamp: chrono::Utc::now(),
            actor,
            message,
            is_system: false,
        }
    }

    /// Create a system-generated TranslationMeta.
    ///
    /// System translations are automatically generated changes, such as:
    /// - Initial translation setup
    /// - Automated imports
    /// - Batch operations
    /// - Schema migrations
    ///
    /// # Arguments
    ///
    /// * `locale` - The locale being translated
    /// * `revision` - HLC timestamp for this change
    /// * `message` - Description of the system change
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::{TranslationMeta, LocaleCode};
    ///
    /// let locale = LocaleCode::parse("en").unwrap();
    /// let meta = TranslationMeta::system(
    ///     locale,
    ///     1,
    ///     "Initialize base language content".to_string(),
    /// );
    ///
    /// assert_eq!(meta.actor, "system");
    /// assert!(meta.is_system);
    /// assert!(meta.parent_revision.is_none());
    /// ```
    pub fn system(locale: LocaleCode, revision: raisin_hlc::HLC, message: String) -> Self {
        TranslationMeta {
            locale,
            revision,
            parent_revision: None,
            timestamp: chrono::Utc::now(),
            actor: "system".to_string(),
            message,
            is_system: true,
        }
    }

    /// Create a TranslationMeta with a custom timestamp.
    ///
    /// Useful for importing historical translations or replay scenarios.
    ///
    /// # Arguments
    ///
    /// * `locale` - The locale being translated
    /// * `revision` - HLC timestamp for this change
    /// * `parent_revision` - Previous revision (if any)
    /// * `timestamp` - Custom timestamp for the change
    /// * `actor` - Who made the translation
    /// * `message` - Description of the translation change
    /// * `is_system` - Whether this is a system change
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::{TranslationMeta, LocaleCode};
    /// use chrono::Utc;
    ///
    /// let locale = LocaleCode::parse("ja").unwrap();
    /// let timestamp = Utc::now();
    /// let meta = TranslationMeta::with_timestamp(
    ///     locale,
    ///     42,
    ///     Some(41),
    ///     timestamp,
    ///     "yuki@example.com".to_string(),
    ///     "Import historical translations".to_string(),
    ///     false,
    /// );
    ///
    /// assert_eq!(meta.timestamp, timestamp);
    /// ```
    pub fn with_timestamp(
        locale: LocaleCode,
        revision: raisin_hlc::HLC,
        parent_revision: Option<raisin_hlc::HLC>,
        timestamp: chrono::DateTime<chrono::Utc>,
        actor: String,
        message: String,
        is_system: bool,
    ) -> Self {
        TranslationMeta {
            locale,
            revision,
            parent_revision,
            timestamp,
            actor,
            message,
            is_system,
        }
    }

    /// Check if this is the initial translation (no parent).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::{TranslationMeta, LocaleCode};
    ///
    /// let locale = LocaleCode::parse("en").unwrap();
    /// let initial = TranslationMeta::system(locale, 1, "Initial".to_string());
    /// assert!(initial.is_initial());
    ///
    /// let locale = LocaleCode::parse("fr").unwrap();
    /// let update = TranslationMeta::new(
    ///     locale,
    ///     2,
    ///     Some(1),
    ///     "user@example.com".to_string(),
    ///     "Update".to_string(),
    /// );
    /// assert!(!update.is_initial());
    /// ```
    #[inline]
    pub fn is_initial(&self) -> bool {
        self.parent_revision.is_none()
    }

    /// Get the age of this translation in the given time unit.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::{TranslationMeta, LocaleCode};
    /// use chrono::{Duration, Utc};
    ///
    /// let locale = LocaleCode::parse("en").unwrap();
    /// let meta = TranslationMeta::system(locale, 1, "Test".to_string());
    ///
    /// // Just created, should be very recent
    /// let age = Utc::now().signed_duration_since(meta.timestamp);
    /// assert!(age.num_seconds() < 5);
    /// ```
    pub fn age(&self) -> chrono::Duration {
        chrono::Utc::now().signed_duration_since(self.timestamp)
    }

    /// Check if this translation is older than the given duration.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::{TranslationMeta, LocaleCode};
    /// use chrono::Duration;
    ///
    /// let locale = LocaleCode::parse("en").unwrap();
    /// let meta = TranslationMeta::system(locale, 1, "Test".to_string());
    ///
    /// assert!(!meta.is_older_than(Duration::hours(1)));
    /// assert!(!meta.is_older_than(Duration::days(1)));
    /// ```
    pub fn is_older_than(&self, duration: chrono::Duration) -> bool {
        self.age() > duration
    }
}

/// Builder for creating TranslationMeta with fluent API.
///
/// Provides a convenient way to construct TranslationMeta with optional fields.
///
/// # Examples
///
/// ```rust
/// use raisin_models::translations::{TranslationMetaBuilder, LocaleCode};
///
/// let locale = LocaleCode::parse("en").unwrap();
/// let meta = TranslationMetaBuilder::new(locale, 42, "user@example.com".to_string())
///     .message("Add translations".to_string())
///     .parent_revision(Some(41))
///     .build();
///
/// assert_eq!(meta.revision, 42);
/// assert_eq!(meta.parent_revision, Some(41));
/// ```
pub struct TranslationMetaBuilder {
    locale: LocaleCode,
    revision: raisin_hlc::HLC,
    parent_revision: Option<raisin_hlc::HLC>,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
    actor: String,
    message: Option<String>,
    is_system: bool,
}

impl TranslationMetaBuilder {
    /// Create a new TranslationMetaBuilder.
    ///
    /// # Arguments
    ///
    /// * `locale` - The locale being translated
    /// * `revision` - HLC timestamp for this change
    /// * `actor` - Who made the translation
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::{TranslationMetaBuilder, LocaleCode};
    ///
    /// let locale = LocaleCode::parse("en").unwrap();
    /// let builder = TranslationMetaBuilder::new(
    ///     locale,
    ///     42,
    ///     "user@example.com".to_string()
    /// );
    /// ```
    pub fn new(locale: LocaleCode, revision: raisin_hlc::HLC, actor: String) -> Self {
        TranslationMetaBuilder {
            locale,
            revision,
            parent_revision: None,
            timestamp: None,
            actor,
            message: None,
            is_system: false,
        }
    }

    /// Set the parent revision.
    pub fn parent_revision(mut self, parent: Option<raisin_hlc::HLC>) -> Self {
        self.parent_revision = parent;
        self
    }

    /// Set a custom timestamp.
    pub fn timestamp(mut self, timestamp: chrono::DateTime<chrono::Utc>) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    /// Set the commit message.
    pub fn message(mut self, message: String) -> Self {
        self.message = Some(message);
        self
    }

    /// Mark this as a system-generated translation.
    pub fn system(mut self) -> Self {
        self.is_system = true;
        self
    }

    /// Build the TranslationMeta.
    ///
    /// Uses default values for any fields not explicitly set:
    /// - `message`: Empty string
    /// - `timestamp`: Current time
    /// - `is_system`: false
    pub fn build(self) -> TranslationMeta {
        TranslationMeta {
            locale: self.locale,
            revision: self.revision,
            parent_revision: self.parent_revision,
            timestamp: self.timestamp.unwrap_or_else(chrono::Utc::now),
            actor: self.actor,
            message: self.message.unwrap_or_default(),
            is_system: self.is_system,
        }
    }
}
