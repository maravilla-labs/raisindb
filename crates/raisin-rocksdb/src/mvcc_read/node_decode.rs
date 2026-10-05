//! Decoding one `NODES` blob into a `Node`.
//!
//! The ONE decoder for a raw node blob: the repository read path
//! (`NodeRepositoryImpl::deserialize_node_with_path*` delegates here), the
//! transaction read path, replication's baseline reader, merge, conflict
//! listing, the index rebuilds and the repairs. Two blob formats live in
//! `NODES`:
//!
//! - a `StorageNode`, which carries no path — the path comes from `NODE_PATH`;
//! - the legacy full `Node`, which embeds its own (the transaction write path,
//!   merge and replication apply before Phases 10/10b; read forever, written
//!   by nothing). It is map-encoded, so it ALSO
//!   decodes as a `StorageNode`; the embedded path survives in
//!   `StorageNode::embedded_path`.
//!
//! Which path wins is the Phase 10 read rule, [`super::materialize_path`]: the
//! newer, by revision, of `NODE_PATH` (newest at or below the read) and the
//! embedded path (at the blob's revision).

use super::source::{DbRead, VersionedRead};
use super::{materialize_path_in, EmbeddedPath, NodeScope};
use crate::repositories::nodes::{PropertiesMode, StorageNodeHead};
use crate::StorageNode;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::DB;

/// Decode `bytes`, the `NODES` value of `node_id` stored at `blob_revision`,
/// with its path as of `read_at` (the read's snapshot — NOT the blob's
/// revision: a later ancestor move writes `NODE_PATH` above it).
#[allow(clippy::too_many_arguments)]
pub(crate) fn deserialize_node_with_path(
    db: &DB,
    bytes: &[u8],
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    read_at: &HLC,
    blob_revision: &HLC,
) -> Result<Node> {
    deserialize_node_with_path_as(
        db,
        bytes,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        read_at,
        blob_revision,
        PropertiesMode::Load,
    )
}

/// [`deserialize_node_with_path`], optionally without decoding the properties
/// (`PropertiesMode::Skip` returns an empty map).
///
/// The legacy positional full-`Node` fallback always decodes everything: it
/// is rare, and correctness there is worth more than the saving.
#[allow(clippy::too_many_arguments)]
pub(crate) fn deserialize_node_with_path_as(
    db: &DB,
    bytes: &[u8],
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    read_at: &HLC,
    blob_revision: &HLC,
    mode: PropertiesMode,
) -> Result<Node> {
    let scope = NodeScope {
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
    };
    deserialize_node_with_path_in(&mut DbRead(db), bytes, scope, read_at, blob_revision, mode)
}

/// [`deserialize_node_with_path_as`] over any read source — the batched
/// reader passes its snapshot-pinned iterators, so the path it materializes
/// comes from the same view as the blob. ONE body for both.
pub(crate) fn deserialize_node_with_path_in(
    src: &mut impl VersionedRead,
    bytes: &[u8],
    scope: NodeScope<'_>,
    read_at: &HLC,
    blob_revision: &HLC,
    mode: PropertiesMode,
) -> Result<Node> {
    let mut path_for = |embedded: Option<&str>| {
        materialize_path_in(
            src,
            scope,
            read_at,
            embedded.map(|path| EmbeddedPath {
                blob_revision,
                path,
            }),
        )
    };

    // The head reader accepts everything the full one does, and its path
    // lookup is the same one: if either fails here it would fail below too,
    // so a failure goes straight to the legacy format instead of paying for
    // the StorageNode attempt twice.
    match mode {
        PropertiesMode::Skip => {
            if let Ok(head) = rmp_serde::from_slice::<StorageNodeHead>(bytes) {
                if let Ok(path) = path_for(head.embedded_path()) {
                    return Ok(head.into_node_without_properties(path));
                }
            }
        }
        PropertiesMode::Load => {
            if let Ok(storage_node) = rmp_serde::from_slice::<StorageNode>(bytes) {
                if let Ok(path) = path_for(storage_node.embedded_path()) {
                    return Ok(storage_node.into_node(path));
                }
            }
        }
    }

    // Fallback: the legacy full `Node` (a positional encoding does not decode
    // as a StorageNode). Its embedded path still answers to the read rule; when
    // the rule cannot decide (deleted / no entry), the blob's own path stands.
    let mut node: Node = rmp_serde::from_slice(bytes).map_err(|e| {
        // The byte dump is built only here — it used to be allocated on every
        // read.
        let first_bytes: Vec<u8> = bytes.iter().take(20).copied().collect();
        let as_string = String::from_utf8_lossy(&bytes[..std::cmp::min(100, bytes.len())]);
        tracing::error!(
            node_id = %scope.node_id,
            workspace = %scope.workspace,
            bytes_len = bytes.len(),
            first_bytes = ?first_bytes,
            as_string = %as_string,
            error = %e,
            "Failed to deserialize node - raw bytes shown"
        );
        raisin_error::Error::storage(format!("Deserialization error: {}", e))
    })?;
    let embedded = Some(node.path.as_str()).filter(|p| !p.is_empty());
    if let Ok(path) = path_for(embedded) {
        node.path = path;
    }
    Ok(node)
}

/// Decode one raw `NODES` entry — key and value, as a scan holds them — with
/// its path as of `read_at`. The blob's revision is the key's trailer. A
/// tombstone (or an unparseable key) is an error, so a scan that skips
/// undecodable entries skips it too.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_entry_with_path(
    db: &DB,
    key: &[u8],
    value: &[u8],
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    read_at: &HLC,
) -> Result<Node> {
    if crate::repositories::nodes::helpers::is_tombstone(value) {
        return Err(raisin_error::Error::storage(format!(
            "node {node_id} is deleted at this version"
        )));
    }
    let blob_revision = crate::keys::extract_revision_from_key(key)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    deserialize_node_with_path(
        db,
        value,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        read_at,
        &blob_revision,
    )
}

/// The path a blob embeds — `Some` only for a legacy full-`Node` blob with a
/// non-empty path. Skips the properties.
pub(crate) fn embedded_path_of(bytes: &[u8]) -> Option<String> {
    if let Ok(head) = rmp_serde::from_slice::<StorageNodeHead>(bytes) {
        return head.embedded_path().map(str::to_string);
    }
    rmp_serde::from_slice::<Node>(bytes)
        .ok()
        .map(|node| node.path)
        .filter(|path| !path.is_empty())
}

/// Decode a raw `NODES` value WITHOUT consulting `NODE_PATH`.
///
/// For readers that must look at the column family itself rather than through
/// a repository — the MVCC index oracle compares `cf::NODES` against an
/// independent model and may not route through the code it is checking. The
/// format order matches [`deserialize_node_with_path`]: `StorageNode` first,
/// then the legacy full `Node`. Returns the node (its `path` is whatever the
/// blob embeds — empty for a `StorageNode`) and the blob's `parent_id` when
/// the format carries one.
#[doc(hidden)]
pub fn decode_node_blob(bytes: &[u8]) -> Result<(Node, Option<String>)> {
    if let Ok(storage_node) = rmp_serde::from_slice::<StorageNode>(bytes) {
        let parent_id = storage_node.parent_id.clone();
        let path = storage_node.embedded_path().unwrap_or_default().to_string();
        return Ok((storage_node.into_node(path), parent_id));
    }
    let node: Node = rmp_serde::from_slice(bytes)
        .map_err(|e| raisin_error::Error::storage(format!("Deserialization error: {e}")))?;
    Ok((node, None))
}
