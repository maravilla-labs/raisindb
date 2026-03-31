//! Serialization and deserialization helpers for translation data.

use raisin_error::{Error, Result};
use raisin_models::translations::{LocaleOverlay, TranslationMeta};
use raisin_storage::RevisionMeta;

/// Serialize a LocaleOverlay to JSON bytes
pub(super) fn serialize_overlay(overlay: &LocaleOverlay) -> Result<Vec<u8>> {
    serde_json::to_vec(overlay)
        .map_err(|e| Error::storage(format!("Failed to serialize LocaleOverlay: {}", e)))
}

/// Deserialize a LocaleOverlay from JSON bytes
pub(super) fn deserialize_overlay(bytes: &[u8]) -> Result<LocaleOverlay> {
    serde_json::from_slice(bytes)
        .map_err(|e| Error::storage(format!("Failed to deserialize LocaleOverlay: {}", e)))
}

/// Serialize TranslationMeta to JSON bytes
pub(super) fn serialize_translation_meta(meta: &TranslationMeta) -> Result<Vec<u8>> {
    serde_json::to_vec(meta)
        .map_err(|e| Error::storage(format!("Failed to serialize TranslationMeta: {}", e)))
}

/// Deserialize TranslationMeta from JSON bytes
pub(super) fn deserialize_translation_meta(bytes: &[u8]) -> Result<TranslationMeta> {
    serde_json::from_slice(bytes)
        .map_err(|e| Error::storage(format!("Failed to deserialize TranslationMeta: {}", e)))
}

/// Serialize RevisionMeta to MessagePack bytes
pub(super) fn serialize_revision_meta(meta: &RevisionMeta) -> Result<Vec<u8>> {
    rmp_serde::to_vec(meta)
        .map_err(|e| Error::storage(format!("Failed to serialize RevisionMeta: {}", e)))
}
