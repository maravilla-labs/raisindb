//! Block-level translation service for managing Composite translations.
//!
//! This module provides specialized services for managing translations of individual
//! blocks within Composite properties:
//! - UUID-based tracking (translations follow blocks when reordered)
//! - Orphan handling (mark translations when blocks are deleted)
//! - Batch operations for multiple blocks
//! - Cleanup utilities for orphaned translations
//!
//! # Composite Translation Model
//!
//! Composites are array properties that contain Block objects, each with:
//! - A stable UUID that persists across reordering
//! - A type field indicating the block type (text, image, etc.)
//! - Various content properties specific to the block type
//!
//! When translating a Composite:
//! - Each block's translation is stored separately by its UUID
//! - Translations follow blocks when they're reordered
//! - When a block is deleted, its translation is marked as orphaned (not deleted)
//! - Orphaned translations can be cleaned up after a grace period
//!
//! # Example Composite Structure
//!
//! ```json
//! {
//!   "content": [
//!     {
//!       "uuid": "block-123",
//!       "type": "text",
//!       "text": "Hello world"
//!     },
//!     {
//!       "uuid": "block-456",
//!       "type": "image",
//!       "src": "/images/photo.jpg",
//!       "caption": "A beautiful photo"
//!     }
//!   ]
//! }
//! ```
//!
//! Translation for block-123 in French:
//! ```json
//! {
//!   "/text": "Bonjour le monde"
//! }
//! ```
//!
//! # Orphan Handling
//!
//! When a block is deleted from the master content:
//! 1. Call `mark_blocks_orphaned()` with the deleted block UUIDs
//! 2. Repository stores a tombstone marker with the deletion revision
//! 3. Orphaned translations are excluded from normal queries
//! 4. Periodically run `cleanup_orphaned_blocks()` to purge old orphans

mod operations;

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_storage::TranslationRepository;
use std::collections::HashMap;
use std::sync::Arc;

/// Block translation service for managing Composite translations.
pub struct BlockTranslationService<R: TranslationRepository> {
    pub(super) repository: Arc<R>,
}

/// Request to update a single block's translation.
#[derive(Debug, Clone)]
pub struct BlockTranslationUpdate {
    /// Block UUID
    pub block_uuid: String,
    /// Node ID containing the block
    pub node_id: String,
    /// Locale code for the translation
    pub locale: LocaleCode,
    /// Property translations within the block (JsonPointer -> PropertyValue)
    pub translations: HashMap<JsonPointer, PropertyValue>,
    /// Optional commit message
    pub message: Option<String>,
}

/// Request to batch update multiple block translations.
#[derive(Debug, Clone)]
pub struct BatchBlockTranslationUpdate {
    /// List of block translation updates
    pub updates: Vec<BlockTranslationUpdate>,
    /// Actor performing the batch update
    pub actor: String,
    /// Optional batch commit message
    pub message: Option<String>,
}

/// Result of a block translation update operation.
#[derive(Debug, Clone)]
pub struct BlockTranslationUpdateResult {
    /// Block UUID that was updated
    pub block_uuid: String,
    /// Node ID containing the block
    pub node_id: String,
    /// Locale that was updated
    pub locale: LocaleCode,
    /// Revision (HLC timestamp) created for this update
    pub revision: raisin_hlc::HLC,
    /// Timestamp of the update
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Result of a batch block translation update operation.
#[derive(Debug, Clone)]
pub struct BatchBlockUpdateResult {
    /// Successful updates
    pub succeeded: Vec<BlockTranslationUpdateResult>,
    /// Failed updates with error messages
    pub failed: Vec<(String, String, LocaleCode, String)>, // (node_id, block_uuid, locale, error)
}

impl<R: TranslationRepository> BlockTranslationService<R> {
    /// Create a new block translation service with the given repository.
    pub fn new(repository: Arc<R>) -> Self {
        Self { repository }
    }
}

#[cfg(test)]
mod tests;
