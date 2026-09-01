//! Processing defaults for asset handling (image captioning, embeddings).

use serde::{Deserialize, Serialize};

/// Default Moondream model for image captioning.
/// Moondream is the preferred default because it supports promptable generation
/// for different output types (alt-text vs description).
pub const DEFAULT_CAPTION_MODEL: &str = "vikhyatk/moondream2";

/// Default CLIP model for image embeddings.
pub const DEFAULT_IMAGE_EMBEDDING_MODEL: &str = "openai/clip-vit-base-patch32";

/// Default settings for asset processing (image captioning, embeddings).
///
/// These settings apply when no rule-level override is specified.
/// The priority order is: Rule setting > Tenant default > System default.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProcessingDefaults {
    /// Default caption model ID.
    /// If None, uses the system default (Salesforce/blip-image-captioning-large).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub caption_model: Option<String>,

    /// Default embedding model ID for images.
    /// If None, uses the system default (openai/clip-vit-base-patch32).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub embedding_model: Option<String>,

    /// Whether to generate image captions by default.
    /// If None, defaults to true for image assets.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub generate_image_caption: Option<bool>,

    /// Whether to generate image embeddings by default.
    /// If None, defaults to true for image assets.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub generate_image_embedding: Option<bool>,

    /// Whether to extract text from PDFs by default.
    /// If None, defaults to true for PDF assets.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extract_pdf_text: Option<bool>,

    /// Languages OCR should recognise, as Tesseract codes (`eng`, `deu`).
    ///
    /// A tenant that works in German says so ONCE here instead of on every
    /// rule in every repository. A rule that names its own list still wins —
    /// see the priority order in this struct's docs.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ocr_languages: Option<Vec<String>>,

    /// Per-word OCR confidence floor (0-100) for this tenant.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ocr_min_word_confidence: Option<u8>,
}

impl ProcessingDefaults {
    /// Get the effective caption model, falling back to system default.
    pub fn effective_caption_model(&self) -> &str {
        self.caption_model
            .as_deref()
            .unwrap_or(DEFAULT_CAPTION_MODEL)
    }

    /// Get the effective embedding model, falling back to system default.
    pub fn effective_embedding_model(&self) -> &str {
        self.embedding_model
            .as_deref()
            .unwrap_or(DEFAULT_IMAGE_EMBEDDING_MODEL)
    }

    /// Fold this tenant's OCR defaults UNDER a rule's settings.
    ///
    /// This is the "Rule setting > Tenant default > System default" priority
    /// this struct has documented since it was written, and until now nothing
    /// implemented it — the asset path read the rule set and never loaded a
    /// tenant config at all. A doc comment asserting a precedence that no code
    /// applies is worse than none, because it is what a reader trusts instead
    /// of checking.
    ///
    /// The system default is deliberately NOT applied here. It belongs at the
    /// point of use, where "unset" still needs to be distinguishable from
    /// "explicitly set to the same value as the default".
    pub fn apply_ocr_defaults(&self, rule: &mut crate::rules::ProcessingSettings) {
        if rule.ocr_languages.is_none() {
            rule.ocr_languages = self.ocr_languages.clone();
        }
        if rule.ocr_min_word_confidence.is_none() {
            rule.ocr_min_word_confidence = self.ocr_min_word_confidence;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::ProcessingSettings;

    fn tenant() -> ProcessingDefaults {
        ProcessingDefaults {
            ocr_languages: Some(vec!["deu".into(), "eng".into()]),
            ocr_min_word_confidence: Some(60),
            ..Default::default()
        }
    }

    /// The priority this struct has documented since it was written, and which
    /// nothing implemented until now: a rule that states a value keeps it.
    #[test]
    fn a_rule_setting_wins_over_the_tenant_default() {
        let mut rule = ProcessingSettings {
            ocr_languages: Some(vec!["fra".into()]),
            ocr_min_word_confidence: Some(80),
            ..Default::default()
        };
        tenant().apply_ocr_defaults(&mut rule);
        assert_eq!(rule.ocr_languages, Some(vec!["fra".to_string()]));
        assert_eq!(rule.ocr_min_word_confidence, Some(80));
    }

    #[test]
    fn the_tenant_default_fills_only_what_the_rule_left_unset() {
        let mut rule = ProcessingSettings {
            ocr_languages: Some(vec!["fra".into()]),
            ..Default::default()
        };
        tenant().apply_ocr_defaults(&mut rule);
        assert_eq!(
            rule.ocr_languages,
            Some(vec!["fra".to_string()]),
            "the rule spoke, so the tenant must not override it"
        );
        assert_eq!(
            rule.ocr_min_word_confidence,
            Some(60),
            "the rule was silent here, so the tenant answers"
        );
    }

    /// A tenant with nothing configured must leave the rule exactly as it was —
    /// it must not overwrite settings with `None` on its way past.
    #[test]
    fn an_empty_tenant_default_changes_nothing() {
        let mut rule = ProcessingSettings {
            ocr_languages: Some(vec!["ita".into()]),
            ocr_min_word_confidence: Some(40),
            ..Default::default()
        };
        ProcessingDefaults::default().apply_ocr_defaults(&mut rule);
        assert_eq!(rule.ocr_languages, Some(vec!["ita".to_string()]));
        assert_eq!(rule.ocr_min_word_confidence, Some(40));
    }
}
