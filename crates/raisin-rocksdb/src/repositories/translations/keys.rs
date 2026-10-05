//! THE key builder for translation storage (plan Phase 11, item 3).
//!
//! Every `TRANSLATION_DATA`, `BLOCK_TRANSLATIONS`, `TRANSLATION_INDEX` and
//! `trans_meta` / `trans_hash` key in the crate comes from here; about fifteen
//! hand-rolled `format!` copies used to live at the call sites. Parsing a key
//! back is `key_parse.rs`.

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
pub(crate) fn translation_key(
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

/// `{tenant}\0{repo}\0{branch}\0{ws}\0translations\0{node_id}\0` — every
/// version of every locale of one node.
pub(crate) fn translation_node_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Vec<u8> {
    let mut key = base_key(tenant_id, repo_id, branch, workspace, "translations");
    key.extend_from_slice(node_id.as_bytes());
    key.push(b'\0');
    key
}

/// `{node prefix}{locale}\0` — every version of one locale.
pub(crate) fn translation_locale_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
) -> Vec<u8> {
    let mut key = translation_node_prefix(tenant_id, repo_id, branch, workspace, node_id);
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key
}

/// The orphan marker of one block: it sits in the LOCALE position
/// (`…\0{block_uuid}\0orphaned\0{~revision}`), so every block reader skips
/// the locale [`BLOCK_ORPHAN_MARKER`].
pub(crate) fn block_orphan_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    block_uuid: &str,
    revision: &HLC,
) -> Vec<u8> {
    block_translation_key(
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        block_uuid,
        BLOCK_ORPHAN_MARKER,
        revision,
    )
}

/// The locale slot value of a block orphan marker.
pub(crate) const BLOCK_ORPHAN_MARKER: &str = "orphaned";

/// The `TRANSLATION_INDEX` value of a live entry (a deleted one is `T`). The
/// transaction writer used to store the node id here and the repository an
/// empty value; nothing ever read it, and one value keeps `T` unambiguous.
pub(crate) const INDEX_LIVE: &[u8] = b"";

/// Encode a block translation key
///
/// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0block_trans\0{node_id}\0{block_uuid}\0{locale}\0{~revision}`
pub(crate) fn block_translation_key(
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
pub(crate) fn block_translation_prefix(
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

/// Encode a NODE-scoped block translation prefix — every block, every locale.
///
/// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0block_trans\0{node_id}\0`
///
/// One scan over this answers "does this node have any block overlays at all, and
/// which?", which is what both the resolver (so it can skip the property walk
/// entirely) and COPY (so it can carry them) need.
pub(crate) fn block_translations_node_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Vec<u8> {
    let mut key = base_key(tenant_id, repo_id, branch, workspace, "block_trans");
    key.extend_from_slice(node_id.as_bytes());
    key.push(b'\0');
    key
}

/// Encode a translation index key (reverse lookup: locale -> nodes)
///
/// Format: `{tenant}\0{repo}\0translation_index\0{locale}\0{~revision}\0{node_id}`
pub(crate) fn translation_index_key(
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
pub(crate) fn translation_index_prefix(tenant_id: &str, repo_id: &str, locale: &str) -> Vec<u8> {
    let mut key = index_base_key(tenant_id, repo_id, "translation_index");
    key.extend_from_slice(locale.as_bytes());
    key.push(b'\0');
    key
}

/// Encode a key for storing translation metadata
///
/// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0trans_meta\0{node_id}\0{locale}\0{~revision}`
pub(crate) fn translation_meta_key(
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
pub(crate) fn translation_meta_prefix(
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
pub(crate) fn translation_hash_key(
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
pub(crate) fn translation_hash_prefix(
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
