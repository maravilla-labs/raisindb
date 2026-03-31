//! Query operations for translations.

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay};
use rocksdb::DB;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::error_ext::ResultExt;

use super::{keys, serialization};

/// List all nodes that have a translation in the given locale
pub(super) async fn list_nodes_with_translation(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    locale: &LocaleCode,
    _revision: &HLC,
) -> Result<Vec<String>> {
    let cf = crate::cf_handle(db, crate::cf::TRANSLATION_INDEX)?;

    let prefix = keys::translation_index_prefix(tenant_id, repo_id, locale.as_str());

    let mut node_ids = HashSet::new();
    let iter = db.prefix_iterator_cf(&cf, &prefix);

    for item in iter {
        let (key, _value) = item.rocksdb_err()?;

        // Parse node_id from key
        // Key format: {prefix}{~revision}\0{node_id}
        if let Some(suffix) = key.strip_prefix(prefix.as_slice()) {
            if let Ok(suffix_str) = std::str::from_utf8(suffix) {
                // Skip revision, get node_id (after \0)
                if let Some(node_id) = suffix_str.split('\0').nth(1) {
                    node_ids.insert(node_id.to_string());
                }
            }
        }
    }

    Ok(node_ids.into_iter().collect())
}

/// Batch fetch translations for multiple nodes
pub(super) async fn get_translations_batch(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_ids: &[String],
    locale: &LocaleCode,
    revision: &HLC,
) -> Result<HashMap<String, LocaleOverlay>> {
    let cf = crate::cf_handle(db, crate::cf::TRANSLATION_DATA)?;
    let mut result = HashMap::new();

    // Strategy: Use prefix iteration for each node to find the most recent translation
    // at or before the requested revision.
    //
    // Since translations use descending revision order, the first matching entry
    // will be the newest one at or before the requested revision.
    //
    // TODO: Future optimization - build a separate index for faster batch lookups

    for node_id in node_ids {
        // Use prefix iteration to find most recent translation at or before revision
        let prefix = keys::translation_prefix(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            locale.as_str(),
        );

        let mut iter = db.prefix_iterator_cf(&cf, &prefix);

        // Iterate to find the first (most recent) translation at or before requested revision
        while let Some(Ok((key, value))) = iter.next() {
            // Verify key is within our prefix
            if !key.starts_with(&prefix) {
                break; // No more translations for this node
            }

            // Extract the revision from the key
            // Key format: {prefix}{~revision}
            // The revision bytes are the last 16 bytes of the key
            if key.len() < 16 {
                tracing::warn!(
                    "get_translations_batch: malformed key for node_id={}, skipping",
                    node_id
                );
                continue;
            }

            let revision_bytes = &key[key.len() - 16..];
            if revision_bytes.len() < 16 {
                tracing::warn!(
                    "get_translations_batch: invalid revision bytes for node_id={}, skipping",
                    node_id
                );
                continue;
            }

            let key_revision = crate::keys::decode_descending_revision(revision_bytes)
                .map_err(|e| Error::storage(format!("Failed to decode revision: {}", e)))?;

            // Check if this revision is at or before the requested revision
            if &key_revision <= revision {
                // Found a valid translation - deserialize and add to result
                let overlay = serialization::deserialize_overlay(&value)?;

                result.insert(node_id.clone(), overlay);
                break; // Found the newest valid translation for this node
            }

            // Otherwise, this revision is too new, keep iterating to find older one
        }
    }

    tracing::debug!(
        "get_translations_batch: fetched {} translations for {} node_ids in locale {} at revision {}",
        result.len(),
        node_ids.len(),
        locale.as_str(),
        revision
    );

    Ok(result)
}
