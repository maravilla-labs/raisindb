//! Query operations for translations.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay};
use rocksdb::DB;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::error_ext::ResultExt;

use super::keys;

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
    let iter = crate::prefix_scan(&db, &cf, &prefix);

    for item in iter {
        let (key, _value) = item.rocksdb_err()?;

        // Key format: {prefix}{~revision:16}\0{node_id}. The revision is 16
        // BINARY bytes that can contain `\0` and are usually not valid UTF-8,
        // so take the node id by POSITION: decoding or splitting the whole
        // suffix silently dropped the node.
        let Some(suffix) = key.strip_prefix(prefix.as_slice()) else {
            break;
        };
        if suffix.len() <= 17 || suffix[16] != 0 {
            continue;
        }
        if let Ok(node_id) = std::str::from_utf8(&suffix[17..]) {
            node_ids.insert(node_id.to_string());
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
    let mut result = HashMap::new();

    // Per node: the newest version at or before `revision`; a tombstone there
    // means deleted (it used to reach the JSON decoder and fail the whole
    // batch). One reader shared with the single-locale and transaction paths.
    for node_id in node_ids {
        if let Some(overlay) = crate::translation_read::read_overlay(
            db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            locale.as_str(),
            Some(revision),
        )? {
            result.insert(node_id.clone(), overlay);
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
