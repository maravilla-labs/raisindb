// SPDX-License-Identifier: BSL-1.1

//! Which vector space an index holds.
//!
//! A vector index is only navigable if every vector in it came out of the SAME
//! embedder. Two models of different widths fail closed — usearch rejects the
//! query — but two models of the SAME width do not: they occupy unrelated
//! regions of R^n, every distance is finite, every ranking is plausible, and
//! nothing logs. No width check can ever catch that. Only partitioning can.
//!
//! The `cf::EMBEDDINGS` key has always known this:
//!
//! ```text
//! {tenant}\0{repo}\0{branch}\0{workspace}\0{embedder_hash}\0{kind}\0{source}\0{chunk}\0{rev}
//!                                          ^^^^^^^^^^^^^^^   ^^^^
//!                                          segments 5 and 6
//! ```
//!
//! The index did not, so there was one index per branch and enabling a second
//! embedder (image vectors, or simply changing model) made that one index
//! unloadable for BOTH — text search goes down when image search comes up.
//!
//! # Why an opaque newtype
//!
//! `raisin-hnsw`'s only raisin dependencies are `raisin-error` and `raisin-hlc`
//! (deliberately — `EmbedderId` / `EmbeddingKind` live in `raisin-ai::config`,
//! which pulls candle and tesseract). So this crate cannot name the types the
//! token is derived from. It takes the rendered token instead.
//!
//! The **single** rendering lives beside `EmbedderId`, as
//! `EmbeddingPartition::to_index_token()` in `raisin-ai::config`, and a unit
//! test there asserts its bytes equal segments 5 and 6 of the `cf::EMBEDDINGS`
//! key. One derivation, two renderings, one test that fails on drift. Do not
//! add a second way to build this string.
//!
//! A newtype rather than `&str` because `add_embedding` would otherwise take
//! four same-typed strings in a row and the compiler would not care which order
//! they arrived in.

use std::fmt;

/// The vector space an index holds: `{embedder_hash}{kind_char}`.
///
/// e.g. `"nX7pQ2mA8bTT"` — an 11-character base64url embedder hash followed by
/// `T` (text) or `I` (image).
///
/// # Invariants
///
/// The token is used as a FILE STEM, so it must contain no path separator, no
/// `.` (which would make `foo.hnsw` ambiguous with a branch called `foo`), and
/// no NUL. [`PartitionId::is_valid_token`] is the one check; it is applied when
/// a token comes from outside the process (a directory listing, a wire message)
/// rather than on every construction, because the in-process producer is
/// `to_index_token()` whose alphabet is `URL_SAFE_NO_PAD` base64 plus `T`/`I`
/// and cannot violate it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PartitionId(String);

impl PartitionId {
    /// Wrap a rendered partition token.
    ///
    /// The caller is expected to have produced it with
    /// `EmbeddingPartition::to_index_token()`. Use [`Self::parse`] for a token
    /// that came from a file name or a peer.
    pub fn new(token: impl Into<String>) -> Self {
        let token = token.into();
        debug_assert!(
            Self::is_valid_token(&token),
            "partition token {token:?} is not usable as a file stem"
        );
        Self(token)
    }

    /// Wrap a token that came from OUTSIDE this process, rejecting anything
    /// that could escape the index directory or collide with a sidecar.
    pub fn parse(token: &str) -> Option<Self> {
        Self::is_valid_token(token).then(|| Self(token.to_string()))
    }

    /// Is this string usable as an index file stem?
    pub fn is_valid_token(token: &str) -> bool {
        !token.is_empty()
            && token.len() <= 64
            && token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }

    /// The token itself.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The kind character the token ends in — `'T'` for text, `'I'` for image.
    ///
    /// The token is `{embedder_hash}{kind_char}` and this crate cannot name
    /// `EmbeddingKind` (see the module docs), so the character is returned raw
    /// and `EmbeddingKind::from_key_char` interprets it on the `raisin-ai` side.
    ///
    /// It lives HERE, beside `is_valid_token`, because this module is the one
    /// that knows the token's shape. A reader that sliced the last byte off the
    /// string itself would be a second, silent definition of the layout — and
    /// the whole reason `PartitionId` is an opaque newtype is that there is
    /// exactly one.
    pub fn kind_char(&self) -> Option<char> {
        self.0.chars().next_back()
    }

    /// The embedder hash the token starts with — the token minus its trailing
    /// kind character, i.e. segment 5 of the `cf::EMBEDDINGS` key.
    ///
    /// It lives HERE for exactly the reason [`Self::kind_char`] does: this
    /// module is the one that knows the token is `{embedder_hash}{kind_char}`.
    /// A caller that wrote `&token[..token.len() - 1]` would be a second,
    /// silent definition of the layout — and the two would only disagree the
    /// day the token grows a third field, at which point every `cf::EMBEDDINGS`
    /// lookup keyed on it reads an empty prefix and reports "this node has no
    /// stored vector" about a node that has one.
    ///
    /// Returns `None` for a token with no kind character to strip.
    pub fn embedder_hash(&self) -> Option<&str> {
        let kind = self.0.chars().next_back()?;
        Some(&self.0[..self.0.len() - kind.len_utf8()])
    }
}

impl fmt::Display for PartitionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dot_would_make_the_file_stem_ambiguous() {
        // Exactly the collision that `.with_extension("hnsw")` used to produce
        // for branches `release.2` and `release.3`.
        assert!(!PartitionId::is_valid_token("release.2"));
        assert!(PartitionId::parse("release.2").is_none());
    }

    #[test]
    fn a_separator_or_traversal_is_refused() {
        assert!(PartitionId::parse("../../etc").is_none());
        assert!(PartitionId::parse("a/b").is_none());
        assert!(PartitionId::parse("").is_none());
        assert!(PartitionId::parse("a\0b").is_none());
    }

    #[test]
    fn the_kind_char_is_the_last_byte_of_the_token() {
        assert_eq!(PartitionId::new("nX7pQ2mA8b-T").kind_char(), Some('T'));
        assert_eq!(PartitionId::new("nX7pQ2mA8b-I").kind_char(), Some('I'));
    }

    /// The hash and the kind character are the two halves of one token, and a
    /// `cf::EMBEDDINGS` lookup needs BOTH: segment 5 is the hash, segment 6 the
    /// kind. Splitting them anywhere but here is how the two segments drift.
    #[test]
    fn the_embedder_hash_is_the_token_minus_its_kind_char() {
        let p = PartitionId::new("nX7pQ2mA8b-T");
        assert_eq!(p.embedder_hash(), Some("nX7pQ2mA8b-"));
        assert_eq!(p.kind_char(), Some('T'));
        // The two halves reassemble into the token they came from.
        assert_eq!(
            format!("{}{}", p.embedder_hash().unwrap(), p.kind_char().unwrap()),
            p.as_str()
        );
    }

    #[test]
    fn a_real_token_round_trips() {
        // base64url alphabet is A-Za-z0-9-_ , plus the one-character kind.
        let p = PartitionId::parse("nX7pQ2mA8b-T").unwrap();
        assert_eq!(p.as_str(), "nX7pQ2mA8b-T");
        assert_eq!(p.to_string(), "nX7pQ2mA8b-T");
    }
}
