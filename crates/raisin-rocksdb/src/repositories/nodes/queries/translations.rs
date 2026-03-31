//! Translation key builders and collection helpers
//!
//! This module provides functions for:
//! - Building translation keys for data, index, and metadata
//! - Collecting translations for copy operations
//! - Block translation handling

use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay};
use std::collections::HashSet;

/// A single block translation entry: (block_uuid, locale, overlay, parent_revision).
type BlockTranslationEntry = (String, LocaleCode, LocaleOverlay, Option<HLC>);

impl NodeRepositoryImpl {
    pub(in crate::repositories::nodes) fn translation_data_prefix(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Vec<u8> {
        format!(
            "{}\0{}\0{}\0{}\0translations\0{}\0",
            tenant_id, repo_id, branch, workspace, node_id
        )
        .into_bytes()
    }

    pub(in crate::repositories::nodes) fn translation_data_key(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &str,
        revision: &HLC,
    ) -> Vec<u8> {
        let mut key = format!(
            "{}\0{}\0{}\0{}\0translations\0{}\0{}\0",
            tenant_id, repo_id, branch, workspace, node_id, locale
        )
        .into_bytes();
        key.extend_from_slice(&keys::encode_descending_revision(revision));
        key
    }

    pub(in crate::repositories::nodes) fn translation_index_key(
        tenant_id: &str,
        repo_id: &str,
        locale: &str,
        revision: &HLC,
        node_id: &str,
    ) -> Vec<u8> {
        let mut key = format!(
            "{}\0{}\0translation_index\0{}\0",
            tenant_id, repo_id, locale
        )
        .into_bytes();
        key.extend_from_slice(&keys::encode_descending_revision(revision));
        key.push(b'\0');
        key.extend_from_slice(node_id.as_bytes());
        key
    }

    pub(in crate::repositories::nodes) fn translation_meta_key(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &str,
        revision: &HLC,
    ) -> Vec<u8> {
        let mut key = format!(
            "{}\0{}\0{}\0{}\0trans_meta\0{}\0{}\0",
            tenant_id, repo_id, branch, workspace, node_id, locale
        )
        .into_bytes();
        key.extend_from_slice(&keys::encode_descending_revision(revision));
        key
    }

    pub(in crate::repositories::nodes) fn block_translation_prefix(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Vec<u8> {
        format!(
            "{}\0{}\0{}\0{}\0block_trans\0{}\0",
            tenant_id, repo_id, branch, workspace, node_id
        )
        .into_bytes()
    }

    pub(in crate::repositories::nodes) fn block_translation_key(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        block_uuid: &str,
        locale: &str,
        revision: &HLC,
    ) -> Vec<u8> {
        let mut key = format!(
            "{}\0{}\0{}\0{}\0block_trans\0{}\0{}\0{}\0",
            tenant_id, repo_id, branch, workspace, node_id, block_uuid, locale
        )
        .into_bytes();
        key.extend_from_slice(&keys::encode_descending_revision(revision));
        key
    }

    pub(in crate::repositories::nodes) fn collect_node_translations_for_copy(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<Vec<(LocaleCode, LocaleOverlay, Option<HLC>)>> {
        let cf_translation_data = cf_handle(&self.db, cf::TRANSLATION_DATA)?;
        let prefix = Self::translation_data_prefix(tenant_id, repo_id, branch, workspace, node_id);
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_translation_data, prefix);

        let mut seen_locales = HashSet::new();
        let mut translations = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            let suffix = &key[prefix_clone.len()..];
            if suffix.is_empty() {
                continue;
            }

            let locale_end = match suffix.iter().position(|&b| b == 0) {
                Some(idx) => idx,
                None => continue,
            };
            let locale_bytes = &suffix[..locale_end];
            let locale_str = std::str::from_utf8(locale_bytes).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Invalid locale encoding for node {}: {}",
                    node_id, e
                ))
            })?;

            if !seen_locales.insert(locale_str.to_string()) {
                continue;
            }

            let remaining = &suffix[locale_end + 1..];
            if remaining.len() < 8 {
                return Err(raisin_error::Error::storage(format!(
                    "Invalid revision encoding for translation {} on node {}",
                    locale_str, node_id
                )));
            }

            let rev_bytes: [u8; 8] = remaining[..8].try_into().map_err(|_| {
                raisin_error::Error::storage("Failed to decode translation revision bytes")
            })?;
            let parent_revision = keys::decode_descending_revision(&rev_bytes).map_err(|_| {
                raisin_error::Error::storage("Failed to decode translation revision")
            })?;

            let overlay: LocaleOverlay = serde_json::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to deserialize translation overlay for {}: {}",
                    locale_str, e
                ))
            })?;

            let locale = LocaleCode::parse(locale_str).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Invalid locale code {} on node {}: {}",
                    locale_str, node_id, e
                ))
            })?;

            translations.push((locale, overlay, Some(parent_revision)));
        }

        Ok(translations)
    }

    pub(in crate::repositories::nodes) fn collect_block_translations_for_copy(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<Vec<BlockTranslationEntry>> {
        let cf_block_trans = cf_handle(&self.db, cf::BLOCK_TRANSLATIONS)?;
        let prefix = Self::block_translation_prefix(tenant_id, repo_id, branch, workspace, node_id);
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_block_trans, prefix);

        let mut seen = HashSet::new();
        let mut translations = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            let suffix = &key[prefix_clone.len()..];
            if suffix.is_empty() {
                continue;
            }

            let block_end = match suffix.iter().position(|&b| b == 0) {
                Some(idx) => idx,
                None => continue,
            };
            let block_uuid = std::str::from_utf8(&suffix[..block_end]).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Invalid block UUID encoding for node {}: {}",
                    node_id, e
                ))
            })?;

            let remaining = &suffix[block_end + 1..];
            if remaining.is_empty() {
                continue;
            }

            let locale_end = match remaining.iter().position(|&b| b == 0) {
                Some(idx) => idx,
                None => continue,
            };
            let locale_bytes = &remaining[..locale_end];
            let locale_str = std::str::from_utf8(locale_bytes).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Invalid locale encoding for block translation on node {}: {}",
                    node_id, e
                ))
            })?;

            let key_tuple = (block_uuid.to_string(), locale_str.to_string());
            if !seen.insert(key_tuple.clone()) {
                continue;
            }

            let revision_bytes = &remaining[locale_end + 1..];
            if revision_bytes.len() < 8 {
                return Err(raisin_error::Error::storage(format!(
                    "Invalid revision encoding for block translation {}::{} on node {}",
                    locale_str, block_uuid, node_id
                )));
            }
            let rev_bytes: [u8; 8] = revision_bytes[..8].try_into().map_err(|_| {
                raisin_error::Error::storage("Failed to decode block translation revision bytes")
            })?;
            let parent_revision = keys::decode_descending_revision(&rev_bytes).map_err(|_| {
                raisin_error::Error::storage("Failed to decode block translation revision")
            })?;

            let overlay: LocaleOverlay = serde_json::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to deserialize block translation overlay {}::{} on node {}: {}",
                    locale_str, block_uuid, node_id, e
                ))
            })?;

            let locale = LocaleCode::parse(locale_str).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Invalid locale code {} for block translation on node {}: {}",
                    locale_str, node_id, e
                ))
            })?;

            translations.push((
                block_uuid.to_string(),
                locale,
                overlay,
                Some(parent_revision),
            ));
        }

        Ok(translations)
    }
}
