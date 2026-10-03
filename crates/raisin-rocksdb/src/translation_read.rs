//! The one reader of `TRANSLATION_DATA` versions.
//!
//! The repository (`repositories/translations`) and the transaction
//! (`transaction/context/translations`) each used to decode this CF by hand,
//! and they disagreed: the repository let a newest tombstone hide a locale,
//! the transaction skipped the tombstone and fell through to an OLDER live
//! version — so a deleted translation came back on every SQL/WS read. Both now
//! call these functions.
//!
//! Key: `{tenant}\0{repo}\0{branch}\0{ws}\0translations\0{node_id}\0{locale}\0{~revision}`.
//! The revision is 16 binary bytes (usually not UTF-8, may contain `\0`), so
//! only the locale segment is ever decoded.
//!
//! Rule, for one locale and for the listing alike: the NEWEST version at or
//! before the bound decides. A tombstone (`b"T"`) there means "absent" — never
//! "look further back".

use crate::repositories::nodes::helpers::is_tombstone;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::LocaleOverlay;
use rocksdb::DB;

#[cfg(test)]
mod tests;

/// `{tenant}\0{repo}\0{branch}\0{ws}\0translations\0{node_id}\0`
pub(crate) fn node_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Vec<u8> {
    format!("{tenant_id}\0{repo_id}\0{branch}\0{workspace}\0translations\0{node_id}\0").into_bytes()
}

/// `{node_prefix}{locale}\0` — every version of one locale.
pub(crate) fn locale_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
) -> Vec<u8> {
    let mut prefix = node_prefix(tenant_id, repo_id, branch, workspace, node_id);
    prefix.extend_from_slice(locale.as_bytes());
    prefix.push(0);
    prefix
}

/// The version of one locale that a read sees.
pub(crate) struct TranslationVersion {
    /// The exact key read — what a transaction records for conflict detection.
    pub key: Vec<u8>,
    /// `None` when that version is a tombstone (the translation is deleted).
    pub overlay: Option<LocaleOverlay>,
}

/// The newest version of `locale` at or before `max_revision` (the newest at
/// all when `None`). `Ok(None)` when the locale never had one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_version(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
    max_revision: Option<&HLC>,
) -> Result<Option<TranslationVersion>> {
    let prefix = locale_prefix(tenant_id, repo_id, branch, workspace, node_id, locale);
    let cf = crate::cf_handle(db, crate::cf::TRANSLATION_DATA)?;

    let found = crate::mvcc_read::newest_at_or_before_with(
        db,
        cf,
        &prefix,
        max_revision,
        |revision, value| -> Result<TranslationVersion> {
            let mut key = prefix.clone();
            key.extend_from_slice(&revision.encode_descending());
            let overlay = if is_tombstone(value) {
                None
            } else {
                Some(serde_json::from_slice(value).map_err(|e| {
                    raisin_error::Error::storage(format!(
                        "Failed to deserialize LocaleOverlay: {}",
                        e
                    ))
                })?)
            };
            Ok(TranslationVersion { key, overlay })
        },
    )?;
    found.transpose()
}

/// The newest live overlay of `locale` at or before `max_revision`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_overlay(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
    max_revision: Option<&HLC>,
) -> Result<Option<LocaleOverlay>> {
    Ok(read_version(
        db,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        locale,
        max_revision,
    )?
    .and_then(|version| version.overlay))
}

/// Every locale of `node_id` whose newest version at or before
/// `max_revision` is live, in key order (each listed once).
pub(crate) fn live_locales(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    max_revision: Option<&HLC>,
) -> Result<Vec<String>> {
    let prefix = node_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let cf = crate::cf_handle(db, crate::cf::TRANSLATION_DATA)?;

    // A locale's versions are contiguous and newest first, so the first
    // version within the bound decides it; `decided` is that locale.
    let mut decided: Option<Vec<u8>> = None;
    let mut locales = Vec::new();

    for item in crate::prefix_scan(db, cf, &prefix) {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        let Some(suffix) = key.strip_prefix(prefix.as_slice()) else {
            break;
        };
        let Some(locale_end) = suffix.iter().position(|b| *b == 0) else {
            continue;
        };
        let locale_bytes = &suffix[..locale_end];
        if decided.as_deref() == Some(locale_bytes) {
            continue;
        }
        if let Some(max) = max_revision {
            match crate::keys::extract_revision_from_key(&key) {
                Ok(revision) if &revision <= max => {}
                // Newer than the bound, or unreadable: not this version.
                _ => continue,
            }
        }
        decided = Some(locale_bytes.to_vec());

        if is_tombstone(&value) {
            continue;
        }
        match std::str::from_utf8(locale_bytes) {
            Ok(locale) => locales.push(locale.to_string()),
            Err(_) => tracing::warn!(node_id, "Skipping translation with a non-UTF-8 locale"),
        }
    }

    Ok(locales)
}
