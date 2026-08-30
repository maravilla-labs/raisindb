//! The embedding SPEC: every input that decides what a stored vector is, and
//! the chunk-id vocabulary that decides where it lives.
//!
//! # Why a spec hash and not a text hash
//!
//! [`crate::models::EmbeddingData::text_hash`] hashes the chunk's text and
//! nothing else, which cannot answer "is this embedding stale?" in the general
//! case:
//!
//! - **Chunking config changed.** Boundaries shift, so most chunk texts change
//!   and are re-embedded *incidentally* — but if the new config yields FEWER
//!   chunks, the surplus high-index chunks are not rewritten at all. They linger
//!   and keep matching queries: stale fragments of a document that no longer
//!   splits that way.
//! - **The extraction pipeline improved** (a plugin gains `.docx`, a walker
//!   learns a container type) without the text of an already-extracted chunk
//!   changing. Nothing re-embeds, so the corpus stays on the old pipeline
//!   forever with no signal anywhere.
//! - **The embedder changed.** Today this relocates the row (the embedder hash
//!   is a key segment), so it reads as *absent* rather than *stale* — but it is
//!   an input to the vector and belongs in the identity of the vector, not only
//!   in its address.
//!
//! So the stored identity is a hash over EVERY input: the extracted text, the
//! embedder, the chunking configuration, and the pipeline version. Any change to
//! any of them re-embeds exactly what it should, and nothing else does.
//!
//! The vector-index PLAN (which fields of which types are embeddable,
//! `include_name` / `include_path`) is deliberately not a separate component: it
//! is a pure function that produces the extracted text, so it is covered
//! transitively and cannot drift out of sync with what was actually embedded.
//!
//! # Stability
//!
//! The hash is SHA-256 over LENGTH-PREFIXED, tagged fields, truncated to 64
//! bits. Length prefixes, not separators, for the same reason
//! `SecretContext`'s AAD uses them: `("ab", "c")` must not collide with
//! `("a", "bc")`. SHA-256 rather than `DefaultHasher` because a stored hash is
//! compared against one computed by a LATER BINARY — std's hasher explicitly
//! does not promise stability across releases, and a silent algorithm change
//! would re-embed every corpus in the fleet with no cause visible anywhere.

use raisin_ai::config::{ChunkingConfig, EmbedderId};
use ring::digest::SHA256;

/// Version of the extraction + embedding pipeline.
///
/// Bump this when a change alters what the pipeline WOULD produce for text it
/// has already embedded — a better extractor, a fixed walker, a changed
/// normalisation. Every embedding then reports stale exactly once and is
/// regenerated on its next job; nothing else changes.
///
/// Do NOT bump it for a change that already shows up in another spec component
/// (the text, the embedder, the chunking config) — that would re-embed the whole
/// fleet's corpus for nothing.
pub const EMBEDDING_PIPELINE_VERSION: u32 = 1;

/// `JobContext` metadata key meaning "re-embed even if the spec says nothing
/// changed".
///
/// The staleness check makes a re-run of unchanged work a no-op, which is what
/// stops a periodic pass from calling the provider for a whole corpus every
/// time. But an operator reaching for FORCE is repairing something the spec
/// cannot see — a truncated vector, a provider that returned garbage, an index
/// that lost a snapshot — so that request has to survive the check.
///
/// It lives here, in the vocabulary crate both sides already depend on, so the
/// enqueuer and the handler cannot spell it differently; and it travels in
/// metadata rather than in `JobType` because the job type is a persisted,
/// parsed, migrated wire format and this is one boolean about one dispatch.
pub const FORCE_REEMBED_KEY: &str = "force_reembed";

/// Every input that decides what a node's vectors are.
///
/// Borrowed on purpose: it is built per node inside the job handler, hashed, and
/// dropped.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddingSpec<'a> {
    /// The full extracted text, BEFORE chunking. Covers the index plan, the
    /// name/path settings and any extractor output transitively.
    pub text: &'a str,
    /// Which model produced (or will produce) the vectors.
    pub embedder: &'a EmbedderId,
    /// How the text is split. `None` means "no chunking configured" — one whole
    /// document embedding — and hashes differently from any configured value.
    pub chunking: Option<&'a ChunkingConfig>,
    /// Which NAMED embedding spec this is, or `None` for the default one.
    ///
    /// One node can carry several named embeddings — see
    /// `raisin_hnsw::chunk_id`. They live in different key namespaces, so the
    /// name is not needed to keep their identities apart; it is hashed anyway
    /// so a row can be checked against the spec it CLAIMS to be, not only
    /// against where it happens to sit.
    pub spec_name: Option<&'a str>,
}

impl<'a> EmbeddingSpec<'a> {
    /// Build a spec from the three inputs the job handler has resolved.
    pub fn new(
        text: &'a str,
        embedder: &'a EmbedderId,
        chunking: Option<&'a ChunkingConfig>,
    ) -> Self {
        Self {
            text,
            embedder,
            chunking,
            spec_name: None,
        }
    }

    /// Name this spec, for a node that carries more than one embedding.
    ///
    /// `None` is the default spec and leaves the hash byte-identical to what a
    /// build without named specs computed — deliberately, so introducing the
    /// feature does not report every existing row stale and regenerate a whole
    /// corpus for a field nobody set.
    pub fn for_spec(mut self, spec_name: Option<&'a str>) -> Self {
        self.spec_name = spec_name;
        self
    }

    /// The stable 64-bit identity of these inputs.
    ///
    /// Two runs agree iff every input agrees. A serialization failure of the
    /// chunking config yields a value that can never match a stored one, so the
    /// outcome of "we cannot describe the config" is a re-embed, never a
    /// wrongly-skipped one — this decision gates whether stale data is served,
    /// so it fails CLOSED.
    pub fn hash(&self) -> u64 {
        let mut ctx = ring::digest::Context::new(&SHA256);

        field(&mut ctx, "v", &EMBEDDING_PIPELINE_VERSION.to_be_bytes());
        field(&mut ctx, "text", self.text.as_bytes());
        // The embedder's own canonical form — the same string that becomes the
        // key segment, so the identity in the row and the address of the row
        // cannot describe different models.
        field(&mut ctx, "embedder", self.embedder.to_key_hash().as_bytes());

        let chunking = match self.chunking {
            None => "none".to_string(),
            Some(c) => match serde_json::to_string(c) {
                Ok(s) => format!("some:{s}"),
                Err(e) => format!("unserializable:{}:{e}", uuid::Uuid::new_v4()),
            },
        };
        field(&mut ctx, "chunking", chunking.as_bytes());

        // Only when named — see `for_spec`. The tag/length framing makes any
        // two present values unambiguous, and an absent one is the default
        // spec, whose digest must not move.
        if let Some(name) = self.spec_name {
            field(&mut ctx, "spec", name.as_bytes());
        }

        let out = ctx.finish();
        let mut first8 = [0u8; 8];
        first8.copy_from_slice(&out.as_ref()[..8]);
        u64::from_be_bytes(first8)
    }
}

/// Feed one tagged, length-prefixed field into the digest.
fn field(ctx: &mut ring::digest::Context, tag: &str, bytes: &[u8]) {
    ctx.update(&(tag.len() as u64).to_be_bytes());
    ctx.update(tag.as_bytes());
    ctx.update(&(bytes.len() as u64).to_be_bytes());
    ctx.update(bytes);
}

// The chunk-id vocabulary that used to live here now lives in
// `raisin_hnsw::chunk_id`, next to the parse that is its inverse.
//
// It moved because a FORMAT here and a PARSE there is the drift this codebase
// keeps paying for: `SearchResult::new` had to take an id apart with its own
// `rfind('#')`, and adding a second `#`-delimited component (the embedding SPEC
// name) would have made the two disagree about what a chunk id even is. One
// module now owns both directions, and this crate's consumers — all of them in
// `raisin-rocksdb`, which already depends on `raisin-hnsw` — call it directly
// rather than through a re-export that would need a new dependency edge.

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_ai::config::{OverlapConfig, SplitterType};

    fn embedder() -> EmbedderId {
        EmbedderId::new("ollama", "bge-m3", 1024)
    }

    #[test]
    fn identical_inputs_hash_identically() {
        let e = embedder();
        let c = ChunkingConfig::default();
        assert_eq!(
            EmbeddingSpec::new("hello", &e, Some(&c)).hash(),
            EmbeddingSpec::new("hello", &e, Some(&c)).hash(),
            "a steady-state run must recognise its own work"
        );
    }

    /// The four ways an embedding goes stale, each independently detected.
    #[test]
    fn every_input_moves_the_hash() {
        let e = embedder();
        let c = ChunkingConfig::default();
        let base = EmbeddingSpec::new("hello", &e, Some(&c)).hash();

        // 1. text
        assert_ne!(base, EmbeddingSpec::new("hello!", &e, Some(&c)).hash());

        // 2. embedder (same width, different model — the case the storage key
        //    alone would still relocate, but which must also read as stale)
        let e2 = EmbedderId::new("ollama", "mxbai-embed-large", 1024);
        assert_ne!(base, EmbeddingSpec::new("hello", &e2, Some(&c)).hash());

        // 3. chunking config — including a change that yields FEWER chunks
        let mut c2 = ChunkingConfig {
            chunk_size: 4096,
            ..ChunkingConfig::default()
        };
        assert_ne!(base, EmbeddingSpec::new("hello", &e, Some(&c2)).hash());
        c2.chunk_size = c.chunk_size;
        c2.overlap = OverlapConfig::Tokens(7);
        assert_ne!(base, EmbeddingSpec::new("hello", &e, Some(&c2)).hash());
        c2.overlap = OverlapConfig::Tokens(64);
        c2.splitter = SplitterType::Markdown;
        assert_ne!(base, EmbeddingSpec::new("hello", &e, Some(&c2)).hash());

        // 4. chunking present vs absent
        assert_ne!(base, EmbeddingSpec::new("hello", &e, None).hash());
    }

    /// A length prefix, not a separator: the components must not be able to
    /// smear into one another.
    #[test]
    fn field_boundaries_are_unambiguous() {
        let e = embedder();
        assert_ne!(
            EmbeddingSpec::new("ab", &e, None).hash(),
            EmbeddingSpec::new("a", &e, None).hash(),
        );
    }

    #[test]
    fn named_specs_have_distinct_identities_and_the_default_one_is_unmoved() {
        let e = embedder();
        let base = EmbeddingSpec::new("hello", &e, None).hash();

        // Introducing named specs must not restate every existing row.
        assert_eq!(
            base,
            EmbeddingSpec::new("hello", &e, None).for_spec(None).hash()
        );

        let doc = EmbeddingSpec::new("hello", &e, None)
            .for_spec(Some("doc"))
            .hash();
        let ocr = EmbeddingSpec::new("hello", &e, None)
            .for_spec(Some("ocr"))
            .hash();
        assert_ne!(base, doc);
        assert_ne!(doc, ocr);
    }
}
