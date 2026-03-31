//! Translation management service for creating and updating translations.
//!
//! This module provides the service layer for managing translation overlays:
//! - Creating and updating translations for nodes
//! - Batch translation updates
//! - Hiding nodes in specific locales
//! - Deleting translations
//!
//! # Translation Workflow
//!
//! 1. Client requests translation update with locale and property values
//! 2. Service validates the translation (optional - property schema checking)
//! 3. Service creates a LocaleOverlay with the translated properties
//! 4. Service stores the overlay via TranslationRepository
//! 5. Service creates a revision entry for the translation change
//!
//! # Revision Tracking
//!
//! Each translation operation creates a new revision with:
//! - Locale identifier
//! - Timestamp
//! - Actor (user who made the change)
//! - Optional commit message
//!
//! This enables:
//! - Translation history and audit trails
//! - Time-travel queries to view translations at specific points
//! - Rollback capabilities

mod operations;

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_storage::Storage;
use std::collections::HashMap;
use std::sync::Arc;

/// Translation management service for creating and updating translations.
pub struct TranslationService<S: Storage> {
    pub(super) storage: Arc<S>,
}

/// Request to update a translation for a single node.
#[derive(Debug, Clone)]
pub struct TranslationUpdate {
    /// Node ID to translate
    pub node_id: String,
    /// Locale code for the translation
    pub locale: LocaleCode,
    /// Property translations as JsonPointer -> PropertyValue map
    pub translations: HashMap<JsonPointer, PropertyValue>,
    /// Optional commit message
    pub message: Option<String>,
}

/// Request to batch update multiple translations.
#[derive(Debug, Clone)]
pub struct BatchTranslationUpdate {
    /// List of translation updates to apply
    pub updates: Vec<TranslationUpdate>,
    /// Actor (user) performing the batch update
    pub actor: String,
    /// Optional batch commit message (applied to all updates)
    pub message: Option<String>,
}

/// Result of a translation update operation.
#[derive(Debug, Clone)]
pub struct TranslationUpdateResult {
    /// Node ID that was updated
    pub node_id: String,
    /// Locale that was updated
    pub locale: LocaleCode,
    /// Revision (HLC timestamp) created for this update
    pub revision: raisin_hlc::HLC,
    /// Timestamp of the update
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Result of a batch translation update operation.
#[derive(Debug, Clone)]
pub struct BatchUpdateResult {
    /// Successful updates
    pub succeeded: Vec<TranslationUpdateResult>,
    /// Failed updates with error messages
    pub failed: Vec<(String, LocaleCode, String)>, // (node_id, locale, error)
}

impl<S: Storage> TranslationService<S> {
    /// Create a new translation service with the given storage.
    pub fn new(storage: Arc<S>) -> Self {
        Self { storage }
    }
}

#[cfg(test)]
mod tests;
