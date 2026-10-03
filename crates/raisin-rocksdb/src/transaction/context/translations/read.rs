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

use crate::transaction::RocksDBTransaction;

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

    // The newest version decides; a tombstone there means deleted — never
    // fall through to an older live version. Same reader as the repository.
    let Some(version) = crate::translation_read::read_version(
        &tx.db, &tenant_id, &repo_id, &branch, workspace, node_id, locale, None,
    )?
    else {
        tracing::debug!(
            "TXN get_translation: no translation found for node_id={}, locale={}",
            node_id,
            locale
        );
        return Ok(None);
    };

    // Record read for conflict detection — a tombstone is a read too.
    tx.record_read(version.key)?;

    tracing::debug!(
        "TXN get_translation: node_id={}, locale={}, present={}",
        node_id,
        locale,
        version.overlay.is_some()
    );
    Ok(version.overlay)
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

    // Locales whose newest version is live: same reader as the repository,
    // so a deleted (tombstoned) locale is not listed.
    let mut locales: Vec<String> = crate::translation_read::live_locales(
        &tx.db, &tenant_id, &repo_id, &branch, workspace, node_id, None,
    )?;

    // Overlay this transaction's own uncommitted writes: a stored translation
    // adds its locale, a deleted one (`None`) removes it.
    {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

        for ((ws, nid, loc), overlay_opt) in &cache.translations {
            if ws != workspace || nid != node_id {
                continue;
            }
            match overlay_opt {
                Some(_) if !locales.contains(loc) => locales.push(loc.clone()),
                Some(_) => {}
                None => locales.retain(|l| l != loc),
            }
        }
    }

    let result = locales;

    tracing::debug!(
        "TXN list_translations_for_node: node_id={}, found {} locales",
        node_id,
        result.len()
    );

    Ok(result)
}
