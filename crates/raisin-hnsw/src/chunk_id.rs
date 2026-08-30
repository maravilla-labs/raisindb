// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The index-id vocabulary: how a node, an embedding SPEC and a chunk index
//! are packed into the one flat id space the HNSW index has, and taken back
//! apart again.
//!
//! # Why format and parse live in ONE file
//!
//! The writer (`jobs/handlers/embedding/handler.rs`), the superseded-chunk
//! sweep, the delete handler and [`crate::types::SearchResult`] all name the
//! same string. A `format!` in one and a hand-rolled `rfind` in another is how
//! a chunk becomes unreachable by the code meant to remove it — and how a
//! `SearchResult` ends up carrying a `node_id` that `storage.nodes().get()`
//! cannot fetch. So the grammar is written down once, here, and everything
//! else calls it.
//!
//! # The grammar
//!
//! ```text
//! index_id := node_id                               -- default spec, unchunked
//!           | node_id "#" chunk_index               -- default spec, chunked
//!           | node_id "#" spec_name "#" chunk_index -- NAMED spec, always suffixed
//! ```
//!
//! `chunk_index` is a non-empty run of ASCII digits. `spec_name` must match
//! [`is_valid_spec_name`], whose first character is a lowercase ASCII LETTER —
//! so a spec name can never be mistaken for a chunk index, in either direction,
//! at any position. That is the whole reason the grammar is shaped this way.
//!
//! A named spec ALWAYS carries its chunk index, even when the document produced
//! a single chunk. Without that rule `{node}#{spec}` and `{node}#{chunk}` would
//! be distinguishable only by inspecting the suffix, and the reading of an id
//! would depend on what the spec happened to be called. The default (unnamed)
//! spec keeps the bare node id for a single chunk, because that is what every
//! legacy row and every existing reader already contains.
//!
//! # Why `#` is safe as a delimiter
//!
//! Node ids are nanoid-generated (`_-0-9a-zA-Z`) or the fixed workspace-root
//! constant; none of them can contain `#`. The pre-existing single-`#` parse
//! already depended on that. Where the assumption cannot be proved the parse
//! FAILS CLOSED and returns the id whole: a wrongly-stripped id names a node
//! that does not exist, which is a silent miss, while an unstripped one is at
//! worst an id that was already unfetchable.

/// A parsed index id: which node, which named embedding spec, which chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedIndexId {
    /// The SOURCE node id — the id to fetch from node storage. Never carries a
    /// `#spec` or `#chunk` suffix.
    pub node_id: String,
    /// The named embedding spec this vector belongs to, or `None` for the
    /// default one (the node's own authored fields).
    pub spec: Option<String>,
    /// Zero-based chunk index within the source text.
    pub chunk_index: usize,
}

/// Longest accepted spec name. Bounded because the name becomes a key segment
/// in `cf::EMBEDDINGS` and an id in the index; an unbounded name would make an
/// id whose length is decided by tenant data.
pub const MAX_SPEC_NAME_LEN: usize = 32;

/// Is `name` usable as an embedding-spec name?
///
/// `[a-z][a-z0-9_-]{0,31}`. The leading lowercase LETTER is load-bearing, not
/// cosmetic: it is what makes a spec name and a chunk index disjoint languages,
/// which is what lets [`parse_index_id`] decide without ambiguity. `#` and NUL
/// are rejected implicitly — the first would nest a delimiter, the second would
/// cut the `cf::EMBEDDINGS` key in half.
pub fn is_valid_spec_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_SPEC_NAME_LEN {
        return false;
    }
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// The `cf::EMBEDDINGS` `{source_id}` key segment for one node under one spec.
///
/// The CHUNK is not part of it — that store has its own `{chunk_idx:04}`
/// segment. Only the spec namespace goes here, which is precisely what makes N
/// named embeddings per node a pure key-VALUE change with no key-FORMAT change
/// and no migration: the default spec's segment stays byte-identical to the
/// bare node id it has always been.
pub fn namespaced_source_id(node_id: &str, spec: Option<&str>) -> String {
    match spec {
        Some(s) => format!("{node_id}#{s}"),
        None => node_id.to_string(),
    }
}

/// The HNSW id of chunk `index`, always in explicitly-chunked form.
pub fn chunk_index_id(node_id: &str, spec: Option<&str>, index: usize) -> String {
    match spec {
        Some(s) => format!("{node_id}#{s}#{index}"),
        None => format!("{node_id}#{index}"),
    }
}

/// The HNSW id under which chunk `index` of `node_id` is indexed.
///
/// The DEFAULT spec keeps the bare node id when the document produced a single
/// chunk (legacy shape, and what every existing row holds). A NAMED spec always
/// carries its chunk index — see the module docs for why that rule is what
/// makes the grammar decidable.
pub fn chunk_source_id(
    node_id: &str,
    spec: Option<&str>,
    index: usize,
    total_chunks: usize,
) -> String {
    if spec.is_some() || total_chunks > 1 {
        chunk_index_id(node_id, spec, index)
    } else {
        node_id.to_string()
    }
}

/// Every HNSW id one embedding run writes for `node_id` under `spec`.
pub fn chunk_id_set(node_id: &str, spec: Option<&str>, total_chunks: usize) -> Vec<String> {
    if spec.is_some() || total_chunks > 1 {
        (0..total_chunks)
            .map(|i| chunk_index_id(node_id, spec, i))
            .collect()
    } else {
        vec![node_id.to_string()]
    }
}

/// Split a `cf::EMBEDDINGS` `{source_id}` key segment back into node and spec.
///
/// The inverse of [`namespaced_source_id`], and the only one. It fails closed
/// the same way [`parse_index_id`] does: a suffix that is not a valid spec name
/// stays part of the node id rather than being stripped.
pub fn split_source_id(source_id: &str) -> (&str, Option<&str>) {
    match source_id.split_once('#') {
        Some((node_id, spec)) if is_valid_spec_name(spec) => (node_id, Some(spec)),
        _ => (source_id, None),
    }
}

/// The HNSW id of a row that is already IN `cf::EMBEDDINGS`, addressed by that
/// row's own key segment and the chunk count the row records.
///
/// Anything reconstructing the index from storage — the rebuild, and any audit
/// that counts what the rebuild would write — needs this, and it needs it to
/// agree with the writer byte for byte. The writer calls [`chunk_source_id`]
/// with the `(node, spec, total_chunks)` it has in hand; a reader has only the
/// stored key and the stored `total_chunks`, so it goes through here and lands
/// on the same function. A rebuild that re-derived the id itself would silently
/// re-index a chunked document under one bare node id and drop every other
/// chunk on the floor — which is exactly what it did before this existed.
pub fn index_id_for_stored(source_id: &str, chunk_index: usize, total_chunks: usize) -> String {
    let (node_id, spec) = split_source_id(source_id);
    chunk_source_id(node_id, spec, chunk_index, total_chunks)
}

/// Take an index id apart into node, spec and chunk index.
///
/// The inverse of [`chunk_source_id`], and the ONLY parse of this grammar. It
/// fails closed at every step: a suffix it cannot prove is a chunk index, or a
/// second component it cannot prove is a spec name, is left as part of the node
/// id rather than stripped.
pub fn parse_index_id(index_id: &str) -> ParsedIndexId {
    let whole = || ParsedIndexId {
        node_id: index_id.to_string(),
        spec: None,
        chunk_index: 0,
    };

    let Some(hash) = index_id.rfind('#') else {
        return whole();
    };

    let head = &index_id[..hash];
    let tail = &index_id[hash + 1..];

    // Step 1: is the last component a chunk index? Only a non-empty run of
    // ASCII digits is; anything else means this is not a chunked id at all and
    // the whole string is the node id.
    let Some(chunk_index) = parse_chunk_component(tail) else {
        return whole();
    };

    // Step 2: is the component before it a spec name? A spec name starts with a
    // lowercase letter and a chunk index is all digits, so this test can never
    // consume a chunk index. The only string it could misread is a node id
    // ending in a `#`-delimited lowercase word — which node ids cannot be, `#`
    // not being in the nanoid alphabet (see the module docs).
    if let Some(hash2) = head.rfind('#') {
        let spec = &head[hash2 + 1..];
        if is_valid_spec_name(spec) {
            return ParsedIndexId {
                node_id: head[..hash2].to_string(),
                spec: Some(spec.to_string()),
                chunk_index,
            };
        }
    }

    ParsedIndexId {
        node_id: head.to_string(),
        spec: None,
        chunk_index,
    }
}

fn parse_chunk_component(s: &str) -> Option<usize> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<usize>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_spec_single_chunk_is_the_bare_node_id() {
        assert_eq!(chunk_source_id("abc", None, 0, 1), "abc");
        assert_eq!(chunk_id_set("abc", None, 1), vec!["abc".to_string()]);
        let p = parse_index_id("abc");
        assert_eq!(p.node_id, "abc");
        assert_eq!(p.spec, None);
        assert_eq!(p.chunk_index, 0);
    }

    #[test]
    fn default_spec_chunked_round_trips() {
        let id = chunk_source_id("abc", None, 3, 9);
        assert_eq!(id, "abc#3");
        let p = parse_index_id(&id);
        assert_eq!(p.node_id, "abc");
        assert_eq!(p.spec, None);
        assert_eq!(p.chunk_index, 3);
    }

    #[test]
    fn named_spec_round_trips_even_for_a_single_chunk() {
        // The rule that makes the grammar decidable: a named spec always
        // carries its chunk index.
        let id = chunk_source_id("abc", Some("doc"), 0, 1);
        assert_eq!(id, "abc#doc#0");
        let p = parse_index_id(&id);
        assert_eq!(p.node_id, "abc");
        assert_eq!(p.spec.as_deref(), Some("doc"));
        assert_eq!(p.chunk_index, 0);
    }

    #[test]
    fn named_spec_chunked_round_trips() {
        let id = chunk_source_id("abc", Some("doc"), 12, 40);
        assert_eq!(id, "abc#doc#12");
        let p = parse_index_id(&id);
        assert_eq!(p.node_id, "abc");
        assert_eq!(p.spec.as_deref(), Some("doc"));
        assert_eq!(p.chunk_index, 12);
    }

    #[test]
    fn every_generated_id_round_trips_to_a_fetchable_node_id() {
        // types.rs warns that getting this wrong yields a SearchResult whose
        // node_id is unfetchable. Assert it over the whole generated space.
        for spec in [None, Some("doc"), Some("a"), Some("x9_-")] {
            for total in [1usize, 2, 5, 100] {
                for id in chunk_id_set("nodeXYZ", spec, total) {
                    let p = parse_index_id(&id);
                    assert_eq!(p.node_id, "nodeXYZ", "id {id}");
                    assert_eq!(p.spec.as_deref(), spec, "id {id}");
                    assert!(p.chunk_index < total, "id {id}");
                }
            }
        }
    }

    #[test]
    fn a_spec_name_can_never_be_read_as_a_chunk_index() {
        // Digits-first names are rejected, so the two languages are disjoint.
        assert!(!is_valid_spec_name("3"));
        assert!(!is_valid_spec_name("0doc"));
        assert!(!is_valid_spec_name(""));
        assert!(!is_valid_spec_name("Doc"));
        assert!(!is_valid_spec_name("do#c"));
        assert!(!is_valid_spec_name(&"a".repeat(MAX_SPEC_NAME_LEN + 1)));
        assert!(is_valid_spec_name("doc"));
        assert!(is_valid_spec_name("word-2007"));
    }

    #[test]
    fn a_non_chunk_suffix_leaves_the_id_whole() {
        // Fail closed: never hand back a source id that does not exist.
        for id in ["abc#", "abc#x", "abc#doc", "abc#12x"] {
            let p = parse_index_id(id);
            assert_eq!(p.node_id, id, "id {id}");
            assert_eq!(p.spec, None);
            assert_eq!(p.chunk_index, 0);
        }
    }

    #[test]
    fn source_id_namespacing_leaves_the_default_spec_byte_identical() {
        // The no-migration claim, pinned.
        assert_eq!(namespaced_source_id("abc", None), "abc");
        assert_eq!(namespaced_source_id("abc", Some("doc")), "abc#doc");
    }
}
