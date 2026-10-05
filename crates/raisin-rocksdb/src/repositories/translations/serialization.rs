//! Serialization helpers for translation records. Overlays themselves are
//! encoded by the one translation writer (`crate::translation_write`) and
//! decoded by the one reader (`crate::translation_read`).

use raisin_error::{Error, Result};
use raisin_models::translations::TranslationMeta;
use raisin_storage::RevisionMeta;

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
