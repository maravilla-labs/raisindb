// SPDX-License-Identifier: BSL-1.1

//! Language-specific tokenization and stemming support.
//!
//! # One registry, one caller
//!
//! Every analyzer this engine uses is registered in exactly one place —
//! [`register_analyzers`], called from `index_manager::get_or_create_index`,
//! which is the only code that constructs or opens an `Index`. That matters:
//! an analyzer is looked up **by name, lazily**, by both the `IndexWriter` and
//! the `QueryParser`. Registering from the write paths (as this used to, once
//! per job in `indexing_impl` and again in `batch`) leaves a freshly-opened
//! index searchable-but-unanalyzable whenever a search arrives before any write
//! in this process — and adds two mirrored call sites that can drift apart.
//!
//! # Why the schema, not the job, picks the analyzer
//!
//! Tantivy binds an analyzer to a *field*, via the tokenizer name recorded in
//! the on-disk schema. Language here is a property of the *document* (a node is
//! indexed once per supported language). Those two cannot be reconciled by
//! naming one analyzer and swapping its contents — a single index holds several
//! languages at once.
//!
//! So the schema carries a per-language field PAIR (`name__de`, `content__de`,
//! …) alongside the language-neutral `name` / `content`. A document is written
//! into the neutral fields plus the pair for its own language; a search is
//! parsed over the neutral fields plus the pair for the query's language. Both
//! sides therefore run the same analyzer over the same field, which is the only
//! property that actually has to hold.

use raisin_error::Result;
use tantivy::tokenizer::{
    Language as StemmerLanguage, LowerCaser, RemoveLongFilter, Stemmer, TextAnalyzer,
};
use tantivy::Index;

use super::cjk::ScriptAwareTokenizer;

/// Name of the language-neutral analyzer, recorded in the schema for `name`
/// and `content`. Not `"default"`: that one is
/// `SimpleTokenizer + RemoveLongFilter(40) + LowerCaser`, whose 40-BYTE limit
/// deletes any unspaced CJK sentence (see [`super::cjk`]).
pub(crate) const BASE_ANALYZER: &str = "raisin_text";

/// Upper bound on an indexed term, in BYTES.
///
/// `RemoveLongFilter` exists to keep pathological input (base64 blobs, minified
/// JS, long URLs) out of the term dictionary; it is not a language decision.
/// Tantivy's default of 40 is too tight for that job once multi-byte scripts
/// are in play — a 21-character German compound with two umlauts already
/// exceeds it. CJK terms are bounded structurally by the bigram tokenizer
/// (at most two characters, so ≤ 8 bytes) and are unaffected by this value.
const MAX_TERM_BYTES: usize = 80;

/// The languages with a Snowball stemmer, in the order their fields appear in
/// the schema. This list is the single source of truth: the schema builds a
/// field pair per entry, and [`register_analyzers`] registers an analyzer per
/// entry. Adding a language means adding it here, and bumping
/// `schema::SCHEMA_VERSION` so existing indexes are rebuilt.
pub(crate) const STEMMED_LANGUAGES: &[&str] = &[
    "en", "de", "fr", "es", "it", "pt", "ru", "ar", "da", "nl", "fi", "hu", "no", "ro", "sv", "tr",
];

/// Maps ISO 639-1 language codes to tantivy's `Language` enum.
pub(crate) fn get_stemmer_language(lang_code: &str) -> Option<StemmerLanguage> {
    match lang_code {
        "en" => Some(StemmerLanguage::English),
        "de" => Some(StemmerLanguage::German),
        "fr" => Some(StemmerLanguage::French),
        "es" => Some(StemmerLanguage::Spanish),
        "it" => Some(StemmerLanguage::Italian),
        "pt" => Some(StemmerLanguage::Portuguese),
        "ru" => Some(StemmerLanguage::Russian),
        "ar" => Some(StemmerLanguage::Arabic),
        "da" => Some(StemmerLanguage::Danish),
        "nl" => Some(StemmerLanguage::Dutch),
        "fi" => Some(StemmerLanguage::Finnish),
        "hu" => Some(StemmerLanguage::Hungarian),
        "no" => Some(StemmerLanguage::Norwegian),
        "ro" => Some(StemmerLanguage::Romanian),
        "sv" => Some(StemmerLanguage::Swedish),
        "tr" => Some(StemmerLanguage::Turkish),
        _ => None,
    }
}

/// The analyzer name recorded in the schema for a language's stemmed fields.
pub(crate) fn stemmed_analyzer_name(lang_code: &str) -> String {
    format!("{}_stemmer", lang_code)
}

/// Schema field name holding `name` analysed for `lang_code`.
pub(crate) fn stemmed_name_field(lang_code: &str) -> String {
    format!("name__{}", lang_code)
}

/// Schema field name holding `content` analysed for `lang_code`.
pub(crate) fn stemmed_content_field(lang_code: &str) -> String {
    format!("content__{}", lang_code)
}

/// The language-neutral chain, shared by every analyzer here.
fn base_chain() -> tantivy::tokenizer::TextAnalyzerBuilder<impl tantivy::tokenizer::Tokenizer> {
    TextAnalyzer::builder(ScriptAwareTokenizer)
        .filter(RemoveLongFilter::limit(MAX_TERM_BYTES))
        .filter(LowerCaser)
}

/// Registers every analyzer the schema can name, on one index.
///
/// Idempotent and cheap (17 small builders), so it can run on every index open.
/// Registering analyzers a given on-disk schema never names is harmless —
/// lookup is by name at use time — and is what lets an index created before a
/// language was added keep working unchanged.
pub(crate) fn register_analyzers(index: &Index) -> Result<()> {
    let tokenizers = index.tokenizers();
    tokenizers.register(BASE_ANALYZER, base_chain().build());

    for lang in STEMMED_LANGUAGES {
        let Some(stemmer_lang) = get_stemmer_language(lang) else {
            // Unreachable while STEMMED_LANGUAGES and get_stemmer_language agree;
            // skipping rather than panicking keeps a typo from taking the whole
            // index offline.
            tracing::warn!(language = %lang, "No stemmer for a declared stemmed language");
            continue;
        };
        tokenizers.register(
            &stemmed_analyzer_name(lang),
            base_chain().filter(Stemmer::new(stemmer_lang)).build(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantivy::tokenizer::TokenStream;

    fn analyze(analyzer: &mut TextAnalyzer, text: &str) -> Vec<String> {
        let mut stream = analyzer.token_stream(text);
        let mut out = Vec::new();
        while stream.advance() {
            out.push(stream.token().text.clone());
        }
        out
    }

    #[test]
    fn every_declared_stemmed_language_has_a_stemmer() {
        for lang in STEMMED_LANGUAGES {
            assert!(
                get_stemmer_language(lang).is_some(),
                "STEMMED_LANGUAGES contains `{lang}` with no stemmer; the schema would \
                 declare a field whose analyzer is never registered"
            );
        }
    }

    #[test]
    fn the_german_analyzer_collapses_an_inflection() {
        let mut de = base_chain()
            .filter(Stemmer::new(StemmerLanguage::German))
            .build();
        assert_eq!(
            analyze(&mut de, "Datenbanken"),
            analyze(&mut de, "Datenbank")
        );
    }

    #[test]
    fn the_base_analyzer_keeps_cjk_text() {
        let mut base = base_chain().build();
        let tokens = analyze(&mut base, "データベースは情報を保存するシステムです");
        assert!(
            tokens.iter().any(|t| t == "情報"),
            "CJK terms must survive the length filter; got {tokens:?}"
        );
    }

    #[test]
    fn the_base_analyzer_still_drops_pathological_terms() {
        let mut base = base_chain().build();
        let blob = "a".repeat(MAX_TERM_BYTES + 1);
        assert!(analyze(&mut base, &blob).is_empty());
    }
}
