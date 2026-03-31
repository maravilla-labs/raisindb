//! Translation read operations
//!
//! This module contains the implementation of translation read operations for transactions:
//! - `get_translation`: Get a locale overlay for a node
//! - `list_translations_for_node`: List all available locales for a node
//!
//! # Key Features
//!
//! ## Read-Your-Writes Semantics
//!
//! All read operations check the in-memory cache first, ensuring that uncommitted
//! changes made earlier in the transaction are visible to later operations.

use raisin_error::Result;
use raisin_models::translations::LocaleOverlay;
use std::collections::HashSet;

use crate::transaction::types::is_tombstone;
use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle};

/// Get a translation (locale overlay) for a node
///
/// Checks the read cache first for read-your-writes semantics.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `node_id` - The ID of the node
/// * `locale` - The locale code (e.g., "en", "fr")
///
/// # Returns
///
/// Ok(Some(overlay)) if found, Ok(None) if not found
pub async fn get_translation(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
    locale: &str,
) -> Result<Option<LocaleOverlay>> {
    // Check read cache first (read-your-writes)
    {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

        let cache_key = (
            workspace.to_string(),
            node_id.to_string(),
            locale.to_string(),
        );
        if let Some(cached) = cache.translations.get(&cache_key) {
            tracing::debug!(
                "TXN get_translation: cache hit for node_id={}, locale={}",
                node_id,
                locale
            );
            return Ok(cached.clone());
        }
    }

    // Not in cache, read from database
    let (tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        (
            meta.tenant_id.clone(),
            meta.repo_id.clone(),
            meta.branch.clone().ok_or_else(|| {
                raisin_error::Error::Validation("Branch not set in transaction".into())
            })?,
        )
    };

    // Build prefix for this translation (all revisions)
    let prefix = format!(
        "{}\0{}\0{}\0{}\0translations\0{}\0{}\0",
        tenant_id, repo_id, branch, workspace, node_id, locale
    );

    let cf_translation_data = cf_handle(&tx.db, cf::TRANSLATION_DATA)?;
    let iter = tx.db.prefix_iterator_cf(cf_translation_data, &prefix);

    // Find the first (newest) non-tombstone entry
    for item in iter {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Verify key matches prefix
        if !key.starts_with(prefix.as_bytes()) {
            break;
        }

        // Skip tombstones
        if is_tombstone(&value) {
            continue;
        }

        // Deserialize LocaleOverlay
        let overlay: LocaleOverlay = serde_json::from_slice(&value).map_err(|e| {
            raisin_error::Error::storage(format!("JSON deserialization error: {}", e))
        })?;

        tracing::debug!(
            "TXN get_translation: found translation for node_id={}, locale={}",
            node_id,
            locale
        );

        // Record read for conflict detection
        tx.record_read(key.to_vec())?;

        return Ok(Some(overlay));
    }

    tracing::debug!(
        "TXN get_translation: no translation found for node_id={}, locale={}",
        node_id,
        locale
    );

    Ok(None)
}

/// List all available locales for a node
///
/// Returns the set of locale codes that have translations for this node.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `node_id` - The ID of the node
///
/// # Returns
///
/// Ok(Vec<String>) with locale codes
pub async fn list_translations_for_node(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
) -> Result<Vec<String>> {
    let (tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        (
            meta.tenant_id.clone(),
            meta.repo_id.clone(),
            meta.branch.clone().ok_or_else(|| {
                raisin_error::Error::Validation("Branch not set in transaction".into())
            })?,
        )
    };

    // Build prefix for all translations of this node
    let prefix = format!(
        "{}\0{}\0{}\0{}\0translations\0{}\0",
        tenant_id, repo_id, branch, workspace, node_id
    );

    let cf_translation_data = cf_handle(&tx.db, cf::TRANSLATION_DATA)?;
    let iter = tx.db.prefix_iterator_cf(cf_translation_data, &prefix);

    let mut locales = HashSet::new();

    // Collect unique locales
    for item in iter {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Verify key matches prefix
        if !key.starts_with(prefix.as_bytes()) {
            break;
        }

        // Skip tombstones
        if is_tombstone(&value) {
            continue;
        }

        // Extract locale from key
        // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0translations\0{node_id}\0{locale}\0{~revision}
        let key_str = String::from_utf8_lossy(&key);
        let parts: Vec<&str> = key_str.split('\0').collect();
        if parts.len() >= 7 {
            let locale = parts[6].to_string();
            locales.insert(locale);
        }
    }

    // Also check read cache for uncommitted translations
    {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

        for ((ws, nid, loc), overlay_opt) in &cache.translations {
            if ws == workspace && nid == node_id && overlay_opt.is_some() {
                locales.insert(loc.clone());
            }
        }
    }

    let result: Vec<String> = locales.into_iter().collect();

    tracing::debug!(
        "TXN list_translations_for_node: node_id={}, found {} locales",
        node_id,
        result.len()
    );

    Ok(result)
}
