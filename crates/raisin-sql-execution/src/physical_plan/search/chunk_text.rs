// SPDX-License-Identifier: BSL-1.1

//! The TEXT of the chunk that answered — what a RAG caller actually needs.
//!
//! A search row says which chunk of a document matched (`chunk_index`). This
//! module answers the next question: *what did that chunk say?* Without it a
//! chatbot has a document id and an ordinal, and has to go and re-read and
//! re-chunk the document itself to find the passage — reproducing the chunker's
//! exact behaviour, which is the sort of duplication that drifts.
//!
//! # Why a span rather than the stored text
//!
//! The chunk's own text is not stored in full: `chunk_content` keeps only the
//! first 200 characters, as a debugging preview. Storing every chunk verbatim
//! would hold the document twice — once in `__extracted_text` and again as
//! chunks. Instead the writer records a byte RANGE
//! ([`raisin_embeddings::ChunkSpan`]) and this module slices `__extracted_text`
//! by it. The text is already in hand: the emit loop has fetched the node and
//! applied row-level security to it before we are called, so slicing costs no
//! extra read and cannot return a field the caller may not see.
//!
//! # The fallback is EXPLICIT, and that is the point
//!
//! A span can be absent (a row written before spans existed, or the default
//! spec, whose text is synthesized and stored nowhere) and it can be stale (the
//! node's text changed after the vector was written). Returning a silent
//! 200-character stub where a full passage was expected is the failure this
//! module exists to avoid: the caller cannot tell a short chunk from a
//! truncated one, and a RAG answer built on the stub is confidently wrong.
//!
//! So every row also reports [`ChunkTextSource`], and a slice is verified
//! against the hash the writer stored for that chunk before it is returned.

use raisin_embeddings::models::EmbeddingData;
use raisin_embeddings::EmbeddingStorage;
use raisin_hnsw::PartitionId;
use raisin_models::nodes::Node;

use super::fusion::FusedHit;

/// Where a row's `chunk_text` came from — storage state, never permission
/// state.
///
/// A caller reads this to know what it is holding. It deliberately does NOT
/// distinguish "row-level security removed the text" from "no text was ever
/// stored": both are [`Unavailable`](ChunkTextSource::Unavailable), because a
/// column that told them apart would be a differential oracle for what a
/// permission hides. The difference goes to the operator log, not the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkTextSource {
    /// The chunk's exact text, sliced from the document by its stored span and
    /// verified against the writer's hash.
    Exact,
    /// The stored 200-character preview. The chunk is longer than this.
    Excerpt,
    /// No text is available for this row.
    Unavailable,
}

impl ChunkTextSource {
    /// The value rendered in the `chunk_text_source` column.
    pub fn as_str(self) -> &'static str {
        match self {
            ChunkTextSource::Exact => "exact",
            ChunkTextSource::Excerpt => "excerpt",
            ChunkTextSource::Unavailable => "unavailable",
        }
    }
}

/// A row's chunk text and where it came from.
#[derive(Debug, Clone)]
pub struct ResolvedChunkText {
    pub text: Option<String>,
    pub source: ChunkTextSource,
}

impl ResolvedChunkText {
    /// Nothing to show.
    pub fn unavailable() -> Self {
        Self {
            text: None,
            source: ChunkTextSource::Unavailable,
        }
    }
}

/// Resolve the text of the chunk this hit matched.
///
/// `node` must already have passed row-level security — the extracted text is
/// read out of its properties, so a permission whose field filter removed
/// `__extracted_text` yields `Unavailable` by construction rather than by a
/// check that could be forgotten.
///
/// Never returns an error: a row whose text cannot be resolved is a row with no
/// text, not a failed query. A storage error is logged and degrades like any
/// other absence — a search that fails outright because a *preview* could not be
/// read would be a worse answer than the one without it.
pub fn resolve<S: EmbeddingStorage + ?Sized>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    hit: &FusedHit,
    node: &Node,
) -> ResolvedChunkText {
    // A hit with no vector leg matched lexically only. There is no chunk.
    let (Some(chunk_index), Some(partition_token)) = (hit.chunk_index, hit.partition.as_deref())
    else {
        return ResolvedChunkText::unavailable();
    };

    let Some(partition) = PartitionId::parse(partition_token) else {
        return ResolvedChunkText::unavailable();
    };
    let (Some(embedder_hash), Some(kind)) = (partition.embedder_hash(), partition.kind_char())
    else {
        return ResolvedChunkText::unavailable();
    };

    // The NAMESPACED source id: `{node}` for the default spec, `{node}#{spec}`
    // for a named one. This is the `{source_id}` key segment, so getting it
    // wrong reads nothing rather than reading the wrong thing.
    let source_id = raisin_hnsw::namespaced_source_id(&hit.key.1, hit.spec.as_deref());

    let stored = match storage.get_source_chunk(
        tenant_id,
        repo_id,
        branch,
        &hit.key.0,
        embedder_hash,
        kind,
        &source_id,
        chunk_index,
        None,
    ) {
        Ok(Some(data)) => data,
        Ok(None) => return ResolvedChunkText::unavailable(),
        Err(e) => {
            tracing::debug!(
                node_id = %hit.key.1,
                chunk_index,
                error = %e,
                "Could not read the stored chunk for its text; the row is emitted without one"
            );
            return ResolvedChunkText::unavailable();
        }
    };

    if let Some(text) = exact_text(&stored, node) {
        return ResolvedChunkText {
            text: Some(text),
            source: ChunkTextSource::Exact,
        };
    }

    match &stored.chunk_content {
        Some(preview) if !preview.is_empty() => ResolvedChunkText {
            text: Some(preview.clone()),
            source: ChunkTextSource::Excerpt,
        },
        _ => ResolvedChunkText::unavailable(),
    }
}

/// Slice the document by this chunk's stored span, if that is sound.
///
/// Returns `None` — falling the caller back to the preview — when the span is
/// absent, when the document is gone or unreadable, when the range no longer
/// lands inside it, or when what it slices is **not what was embedded**.
fn exact_text(stored: &EmbeddingData, node: &Node) -> Option<String> {
    let span = stored.chunk_span?;
    let document = raisin_models::nodes::extracted_text(&node.properties)?;
    let slice = span.slice(document)?;

    // THE verification, and the reason a stale span is a downgrade rather than
    // a wrong answer.
    //
    // `text_hash` is the hash of this chunk's content as it was embedded, so a
    // correct span over unchanged text reproduces it exactly. When it does not,
    // the document has been rewritten since the vector was made and the range
    // now covers a different passage — text that reads perfectly and answers a
    // different question. Nothing downstream could detect that; there is no
    // second copy to compare against and no length or shape that looks wrong.
    // So we check here, and fall back to the preview, which is at least the
    // text this vector was actually built from.
    if raisin_embeddings::hash_chunk_text(slice) != stored.text_hash {
        tracing::debug!(
            source_id = %stored.source_id,
            chunk_index = stored.chunk_index,
            "The stored chunk span no longer matches the document text; falling back \
             to the excerpt. The node was rewritten after this vector was generated."
        );
        return None;
    }

    Some(slice.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_embeddings::config::EmbeddingProvider;
    use raisin_embeddings::ChunkSpan;
    use raisin_embeddings::{EmbedderId, EmbeddingKind};
    use raisin_models::nodes::properties::PropertyValue;
    use std::collections::HashMap;

    const DOC: &str = "Ticketing opens in March. Audience engagement follows in April. \
                       The platform unifies both.";

    /// A node carrying a successful extraction artifact.
    fn node_with(text: &str) -> Node {
        let mut properties = HashMap::new();
        properties.insert(
            "__extract_status".to_string(),
            PropertyValue::String("ok".to_string()),
        );
        properties.insert(
            "__extracted_text".to_string(),
            PropertyValue::String(text.to_string()),
        );
        Node {
            id: "n1".to_string(),
            name: "doc.pdf".to_string(),
            path: "/doc.pdf".to_string(),
            node_type: "raisin:Asset".to_string(),
            properties,
            ..Default::default()
        }
    }

    /// A stored row for `span`, whose `text_hash` is the hash of whatever
    /// `embedded` says was actually embedded.
    fn stored(span: Option<ChunkSpan>, embedded: &str, preview: Option<&str>) -> EmbeddingData {
        #[allow(deprecated)]
        EmbeddingData {
            vector: vec![0.0; 4],
            embedder_id: EmbedderId::new("ollama", "bge-m3", 4),
            embedding_kind: EmbeddingKind::Text,
            source_id: "n1#doc".to_string(),
            chunk_index: 1,
            total_chunks: 3,
            chunk_content: preview.map(str::to_string),
            generated_at: chrono::Utc::now(),
            text_hash: raisin_embeddings::hash_chunk_text(embedded),
            spec_hash: Some(1),
            chunk_span: span,
            model: "bge-m3".to_string(),
            provider: EmbeddingProvider::Ollama,
        }
    }

    #[test]
    fn a_valid_span_yields_the_exact_passage() {
        let passage = "Audience engagement follows in April.";
        let start = DOC.find(passage).unwrap();
        let span = ChunkSpan::new(start, start + passage.len()).unwrap();

        let got = exact_text(&stored(Some(span), passage, None), &node_with(DOC));
        assert_eq!(got.as_deref(), Some(passage));
    }

    /// The document was rewritten after the vector was made, so the range now
    /// covers a DIFFERENT passage — text that reads perfectly and answers a
    /// different question. This is the case the hash check exists for, and the
    /// only one where a wrong answer would otherwise be undetectable.
    #[test]
    fn a_stale_span_is_refused_rather_than_mis_sliced() {
        let passage = "Audience engagement follows in April.";
        let start = DOC.find(passage).unwrap();
        let span = ChunkSpan::new(start, start + passage.len()).unwrap();
        let row = stored(Some(span), passage, None);

        // Same LENGTH, different content — so the span still lands in bounds
        // and the only thing that can catch the swap is the hash.
        let rewritten = DOC.replace("Audience engagement", "Warehouse logistics");
        assert_eq!(
            rewritten.len(),
            DOC.len(),
            "the span must still be in range"
        );

        assert_eq!(
            exact_text(&row, &node_with(&rewritten)),
            None,
            "a span over rewritten text must not be returned as the chunk"
        );
    }

    #[test]
    fn a_row_without_a_span_has_no_exact_text() {
        assert_eq!(
            exact_text(&stored(None, "anything", None), &node_with(DOC)),
            None
        );
    }

    #[test]
    fn a_node_without_extracted_text_has_no_exact_text() {
        let span = ChunkSpan::new(0, 10).unwrap();
        let bare = Node {
            id: "n1".to_string(),
            ..Default::default()
        };
        assert_eq!(
            exact_text(&stored(Some(span), &DOC[0..10], None), &bare),
            None
        );
    }

    /// The document may have been truncated at `MAX_INLINE_EXTRACT_BYTES` since
    /// the span was written. Out of range must decline, not panic.
    #[test]
    fn a_span_past_the_end_declines() {
        let span = ChunkSpan::new(0, 10_000).unwrap();
        assert_eq!(
            exact_text(&stored(Some(span), "x", None), &node_with(DOC)),
            None
        );
    }

    #[test]
    fn the_source_labels_are_the_documented_strings() {
        assert_eq!(ChunkTextSource::Exact.as_str(), "exact");
        assert_eq!(ChunkTextSource::Excerpt.as_str(), "excerpt");
        assert_eq!(ChunkTextSource::Unavailable.as_str(), "unavailable");
    }
}
