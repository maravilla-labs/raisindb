//! An HNSW index on the wire: the graph file AND its sidecar, or nothing.
//!
//! # The bug this exists to close
//!
//! The transfer path used to send the `.hnsw` graph file alone. A grep for
//! "meta" across the whole module returned only `fs::metadata`. On the
//! receiving node `persistence::view_from_file` sees no `.hnsw.meta`, concludes
//! "this must be an old bincode-format index", and hands it to
//! `migration::migrate_from_old_format`, which bincode-deserialises a usearch
//! file and fails. The index is gone — and `ingest_index` had already renamed
//! the receiver's own healthy index to `.backup` to make room for it.
//!
//! Reproduced before the fix: a 5-vector, 8-dimensional index saved as
//! `main.hnsw` + `main.hnsw.meta` transferred as 992 bytes, landed as
//! `main.hnsw` alone, and loading it returned
//! `Failed to deserialize old index format`.
//!
//! # Framing
//!
//! ```text
//! "RHNSWB1"  (7 bytes)
//! graph_len  (u64 little-endian)
//! meta_len   (u64 little-endian)
//! graph bytes
//! meta bytes
//! ```
//!
//! One blob, so the replication message shape is unchanged and the existing
//! CRC32 covers both halves. A payload without the magic is REFUSED rather than
//! written: a bare graph file is exactly the shape that destroys an index, so
//! "reject a peer we cannot understand" is strictly better than "overwrite a
//! working index with something unloadable".

use raisin_error::{Error, Result};

/// Magic prefix identifying a bundled index payload.
const MAGIC: &[u8; 7] = b"RHNSWB1";
const HEADER_LEN: usize = MAGIC.len() + 8 + 8;

/// Pack a graph file and its sidecar into one wire payload.
pub fn pack(graph: &[u8], meta: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + graph.len() + meta.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(graph.len() as u64).to_le_bytes());
    out.extend_from_slice(&(meta.len() as u64).to_le_bytes());
    out.extend_from_slice(graph);
    out.extend_from_slice(meta);
    out
}

/// Split a wire payload back into (graph, sidecar).
///
/// Errors — never silently falls back to "treat the whole thing as the graph".
/// That fallback is the bug.
pub fn unpack(data: &[u8]) -> Result<(&[u8], &[u8])> {
    if data.len() < HEADER_LEN || &data[..MAGIC.len()] != MAGIC {
        return Err(Error::storage(
            "HNSW index payload is not a bundle: it carries no `.hnsw.meta` sidecar. \
             A bare graph file is read as an old bincode index and destroys itself on \
             load, so it is refused rather than ingested. The sending peer is running a \
             build from before the sidecar was shipped."
                .to_string(),
        ));
    }

    let graph_len = u64::from_le_bytes(data[7..15].try_into().unwrap()) as usize;
    let meta_len = u64::from_le_bytes(data[15..23].try_into().unwrap()) as usize;

    let expected = HEADER_LEN
        .checked_add(graph_len)
        .and_then(|n| n.checked_add(meta_len));
    if expected != Some(data.len()) {
        return Err(Error::storage(format!(
            "HNSW index bundle is truncated or corrupt: header declares {graph_len} graph \
             bytes and {meta_len} sidecar bytes, payload is {} bytes",
            data.len()
        )));
    }
    if meta_len == 0 {
        return Err(Error::storage(
            "HNSW index bundle carries an EMPTY `.hnsw.meta` sidecar. The sidecar holds \
             the node-id mapping, the width and the metric; without it the graph is not \
             an index."
                .to_string(),
        ));
    }

    let graph_end = HEADER_LEN + graph_len;
    Ok((&data[HEADER_LEN..graph_end], &data[graph_end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_round_trips() {
        let packed = pack(b"graph-bytes", b"{\"meta\":1}");
        let (graph, meta) = unpack(&packed).unwrap();
        assert_eq!(graph, b"graph-bytes");
        assert_eq!(meta, b"{\"meta\":1}");
    }

    #[test]
    fn a_bare_graph_file_is_refused_not_guessed_at() {
        // The exact payload the old sender produced.
        let err = unpack(b"usearch native index bytes").unwrap_err();
        assert!(err.to_string().contains("not a bundle"), "{err}");
    }

    #[test]
    fn a_truncated_bundle_is_refused() {
        let mut packed = pack(b"graph", b"meta");
        packed.truncate(packed.len() - 2);
        assert!(unpack(&packed).is_err());
    }

    #[test]
    fn an_empty_sidecar_is_refused() {
        let packed = pack(b"graph", b"");
        let err = unpack(&packed).unwrap_err();
        assert!(err.to_string().contains("EMPTY"), "{err}");
    }
}
