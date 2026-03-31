//! Key encoding functions for translation storage.
//!
//! This module provides all key encoding functions for the translation repository,
//! following consistent patterns to minimize duplication.

use raisin_hlc::HLC;

/// Helper to build a base key with tenant, repo, branch, workspace, and entity type
fn base_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    entity_type: &str,
) -> Vec<u8> {
    format!(
        "{}\0{}\0{}\0{}\0{}\0",
        tenant_id, repo_id, branch, workspace, entity_type
    )
    .into_bytes()
}

/// Helper to build a base key without branch/workspace (for indexes)
fn index_base_key(tenant_id: &str, repo_id: &str, entity_type: &str) -> Vec<u8> {
    format!("{}\0{}\0{}\0", tenant_id, repo_id, entity_type).into_bytes()
}

/// Encode a translation data key
///
/// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0translations\0{node_id}\0{locale}\0{~revision}`
pub(super) fn translation_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
    revision: &HLC,
) -> Vec<u8> {
    let mut key = base_key(tenant_id, repo_id, branch, workspace, "translations");
    key.extend_from_slice(node_id.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(&crate::keys::encode_descending_revision(revision));
    key
}

/// Encode a translation prefix key (for iteration)
///
/// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0translations\0{node_id}\0{locale}\0`
pub(super) fn translation_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
) -> Vec<u8> {
    let mut key = base_key(tenant_id, repo_id, branch, workspace, "translations");
    key.extend_from_slice(node_id.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key
}

/// Encode a block translation key
///
/// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0block_trans\0{node_id}\0{block_uuid}\0{locale}\0{~revision}`
pub(super) fn block_translation_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    block_uuid: &str,
    locale: &str,
    revision: &HLC,
) -> Vec<u8> {
    let mut key = base_key(tenant_id, repo_id, branch, workspace, "block_trans");
    key.extend_from_slice(node_id.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(block_uuid.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(&crate::keys::encode_descending_revision(revision));
    key
}

/// Encode a block translation prefix key
///
/// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0block_trans\0{node_id}\0{block_uuid}\0{locale}\0`
pub(super) fn block_translation_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    block_uuid: &str,
    locale: &str,
) -> Vec<u8> {
    let mut key = base_key(tenant_id, repo_id, branch, workspace, "block_trans");
    key.extend_from_slice(node_id.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(block_uuid.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key
}

/// Encode a translation index key (reverse lookup: locale -> nodes)
///
/// Format: `{tenant}\0{repo}\0translation_index\0{locale}\0{~revision}\0{node_id}`
pub(super) fn translation_index_key(
    tenant_id: &str,
    repo_id: &str,
    locale: &str,
    revision: &HLC,
    node_id: &str,
) -> Vec<u8> {
    let mut key = index_base_key(tenant_id, repo_id, "translation_index");
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(&crate::keys::encode_descending_revision(revision));
    key.push(b'\0');
    key.extend_from_slice(node_id.as_bytes());
    key
}

/// Encode translation index prefix for iteration
///
/// Format: `{tenant}\0{repo}\0translation_index\0{locale}\0`
pub(super) fn translation_index_prefix(tenant_id: &str, repo_id: &str, locale: &str) -> Vec<u8> {
    let mut key = index_base_key(tenant_id, repo_id, "translation_index");
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key
}

/// Encode a key for storing translation metadata
///
/// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0trans_meta\0{node_id}\0{locale}\0{~revision}`
pub(super) fn translation_meta_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
    revision: &HLC,
) -> Vec<u8> {
    let mut key = base_key(tenant_id, repo_id, branch, workspace, "trans_meta");
    key.extend_from_slice(node_id.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(&crate::keys::encode_descending_revision(revision));
    key
}

/// Get translation metadata prefix
///
/// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0trans_meta\0{node_id}\0{locale}\0`
pub(super) fn translation_meta_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
) -> Vec<u8> {
    let mut key = base_key(tenant_id, repo_id, branch, workspace, "trans_meta");
    key.extend_from_slice(node_id.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key
}

/// Encode a translation hash record key
///
/// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0trans_hash\0{node_id}\0{locale}\0{pointer}`
///
/// Unlike other translation keys, hash records don't use revision in the key -
/// they represent the current state of a translation's staleness tracking.
pub(super) fn translation_hash_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
    pointer: &str,
) -> Vec<u8> {
    let mut key = base_key(tenant_id, repo_id, branch, workspace, "trans_hash");
    key.extend_from_slice(node_id.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(pointer.as_bytes());
    key
}

/// Encode a translation hash prefix key (for listing all hashes for a node/locale)
///
/// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0trans_hash\0{node_id}\0{locale}\0`
pub(super) fn translation_hash_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
) -> Vec<u8> {
    let mut key = base_key(tenant_id, repo_id, branch, workspace, "trans_hash");
    key.extend_from_slice(node_id.as_bytes());
    key.push(b'\0');
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_translation_key_encoding() {
        let revision = HLC::new(42, 0);
        let key = translation_key(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "node123",
            "fr-FR",
            &revision,
        );

        let key_str = String::from_utf8_lossy(&key[..key.len() - 16]).to_string();
        assert!(key_str.contains("tenant1"));
        assert!(key_str.contains("repo1"));
        assert!(key_str.contains("main"));
        assert!(key_str.contains("workspace1"));
        assert!(key_str.contains("translations"));
        assert!(key_str.contains("node123"));
        assert!(key_str.contains("fr-FR"));
    }

    #[test]
    fn test_block_translation_key_encoding() {
        let revision = HLC::new(100, 0);
        let key = block_translation_key(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "node123",
            "block-uuid-456",
            "de-DE",
            &revision,
        );

        let key_str = String::from_utf8_lossy(&key[..key.len() - 16]).to_string();
        assert!(key_str.contains("block_trans"));
        assert!(key_str.contains("block-uuid-456"));
        assert!(key_str.contains("de-DE"));
    }

    #[test]
    fn test_translation_index_key_encoding() {
        let revision = HLC::new(200, 0);
        let key = translation_index_key("tenant1", "repo1", "es-MX", &revision, "node789");

        let key_str = String::from_utf8_lossy(&key).to_string();
        assert!(key_str.contains("translation_index"));
        assert!(key_str.contains("es-MX"));
        assert!(key_str.contains("node789"));
    }
}
