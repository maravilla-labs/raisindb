// SPDX-License-Identifier: BSL-1.1

//! Language-specific tokenization and stemming support.

use raisin_error::Result;
use tantivy::tokenizer::{Language as StemmerLanguage, Stemmer};
use tantivy::Index;

/// Maps ISO 639-1 language codes to tantivy-stemmers Language enum.
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

/// Registers language-specific tokenizers with the index.
pub(crate) fn register_language_tokenizer(index: &Index, language: &str) -> Result<()> {
    if let Some(stemmer_lang) = get_stemmer_language(language) {
        let tokenizer_name = format!("{}_stemmer", language);
        let tokenizer = tantivy::tokenizer::TextAnalyzer::builder(
            tantivy::tokenizer::SimpleTokenizer::default(),
        )
        .filter(tantivy::tokenizer::LowerCaser)
        .filter(Stemmer::new(stemmer_lang))
        .build();
        index.tokenizers().register(&tokenizer_name, tokenizer);
    }
    Ok(())
}
