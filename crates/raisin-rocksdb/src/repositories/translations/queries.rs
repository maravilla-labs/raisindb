//! Query operations for translations.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay};
use rocksdb::DB;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::error_ext::ResultExt;

use super::keys;

/// Every node of `(branch, workspace)` with a live translation in `locale` as
/// of `revision`.
///
/// `TRANSLATION_INDEX` is REPO-WIDE — `{tenant}\0{repo}\0translation_index\0
/// {locale}\0{~revision:16}\0{node_id}`, no branch, no workspace — so it can
/// only say which nodes MAY qualify. Letting its newest entry decide (as this
/// did) made a delete on one branch hide the node from every other branch's
/// listing, and a translation written only on a branch list the node on
/// `main`. So the index yields CANDIDATES — every node with an entry at or
/// before the bound, live or `T`: each version write stages its index entry at
/// the same revision, so no qualifying node lacks one — and the one reader
/// (`translation_read::read_overlay`) decides each on the caller's branch and
/// workspace. The revision is 16 BINARY bytes that can contain `\0` and are
/// usually not valid UTF-8, so the node id is taken by POSITION.
#[allow(clippy::too_many_arguments)]
pub(super) async fn list_nodes_with_translation(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    locale: &LocaleCode,
    revision: &HLC,
) -> Result<Vec<String>> {
    crate::translation_history::ensure_complete_at(db, tenant_id, repo_id, branch, revision)?;
    let cf = crate::cf_handle(db, crate::cf::TRANSLATION_INDEX)?;
    let prefix = keys::translation_index_prefix(tenant_id, repo_id, locale.as_str());

    let mut candidates = HashSet::new();
    let mut node_ids = Vec::new();
    for item in crate::prefix_scan(&db, &cf, &prefix) {
        let (key, _) = item.rocksdb_err()?;
        let Some(suffix) = key.strip_prefix(prefix.as_slice()) else {
            break;
        };
        if suffix.len() <= 17 || suffix[16] != 0 {
            continue;
        }
        let Ok(entry_revision) = crate::keys::decode_descending_revision(&suffix[..16]) else {
            continue;
        };
        if &entry_revision > revision {
            continue;
        }
        let Ok(node_id) = std::str::from_utf8(&suffix[17..]) else {
            continue;
        };
        if !candidates.insert(node_id.to_string()) {
            continue;
        }
        if crate::translation_read::read_overlay(
            db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            locale.as_str(),
            Some(revision),
        )?
        .is_some()
        {
            node_ids.push(node_id.to_string());
        }
    }

    Ok(node_ids)
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
    crate::translation_history::ensure_complete_at(db, tenant_id, repo_id, branch, revision)?;
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
