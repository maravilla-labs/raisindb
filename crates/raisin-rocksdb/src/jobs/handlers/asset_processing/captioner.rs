//! Cached captioner supporting both BLIP and Moondream models

use raisin_ai::{BlipCaptioner, MoondreamCaptioner};

/// Cached captioner supporting both BLIP and Moondream models
pub(super) enum CachedCaptioner {
    Blip {
        captioner: BlipCaptioner,
        model_id: String,
    },
    Moondream {
        captioner: MoondreamCaptioner,
        model_id: String,
    },
}

impl CachedCaptioner {
    pub fn model_id(&self) -> &str {
        match self {
            CachedCaptioner::Blip { model_id, .. } => model_id,
            CachedCaptioner::Moondream { model_id, .. } => model_id,
        }
    }

    /// Generate caption and alt-text
    ///
    /// For Moondream, generates separate outputs using different prompts (or custom prompts if provided).
    /// For BLIP, generates a single caption and derives alt-text from it (ignores custom prompts).
    pub fn generate(
        &mut self,
        image_bytes: &[u8],
        custom_alt_text_prompt: Option<&str>,
        custom_description_prompt: Option<&str>,
    ) -> std::result::Result<(String, String), raisin_ai::CandleError> {
        match self {
            CachedCaptioner::Blip { captioner, .. } => {
                let caption = captioner.caption_image(image_bytes)?;
                let alt_text = raisin_ai::candle::blip::generate_alt_text_from_caption(&caption);
                Ok((caption, alt_text))
            }
            CachedCaptioner::Moondream { captioner, .. } => {
                let description = if let Some(prompt) = custom_description_prompt {
                    captioner.caption_with_prompt(image_bytes, prompt)?
                } else {
                    captioner.generate_description(image_bytes)?
                };

                let alt_text = if let Some(prompt) = custom_alt_text_prompt {
                    captioner.caption_with_prompt(image_bytes, prompt)?
                } else {
                    captioner.generate_alt_text(image_bytes)?
                };

                Ok((description, alt_text))
            }
        }
    }

    /// Generate keywords for an image.
    ///
    /// Only supported by Moondream. BLIP returns empty vector.
    pub fn generate_keywords(
        &mut self,
        image_bytes: &[u8],
        custom_keywords_prompt: Option<&str>,
    ) -> std::result::Result<Vec<String>, raisin_ai::CandleError> {
        match self {
            CachedCaptioner::Blip { .. } => Ok(Vec::new()),
            CachedCaptioner::Moondream { captioner, .. } => {
                if let Some(prompt) = custom_keywords_prompt {
                    captioner.generate_keywords_with_prompt(image_bytes, prompt)
                } else {
                    captioner.generate_keywords(image_bytes)
                }
            }
        }
    }
}
