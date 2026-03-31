//! AIProviderTrait implementation for LocalCandleProvider.

use async_trait::async_trait;

use crate::model_cache::{ModelCapabilities, ModelInfo};
use crate::provider::{AIProviderTrait, ProviderError, Result};
use crate::types::{CompletionRequest, CompletionResponse, Message};

use super::model::LocalModel;
use super::LocalCandleProvider;

#[async_trait]
impl AIProviderTrait for LocalCandleProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let local_model = LocalModel::from_model_id(&request.model).ok_or_else(|| {
            ProviderError::InvalidModel(format!(
                "Unknown local model '{}'. Supported: moondream, blip, clip",
                request.model
            ))
        })?;

        if local_model == LocalModel::Clip {
            return Err(ProviderError::UnsupportedOperation(
                "CLIP is an embedding model and doesn't support chat completion. Use generate_embedding() instead.".to_string()
            ));
        }

        let (image_base64, _media_type) = Self::extract_image_from_messages(&request.messages)
            .ok_or_else(|| {
                ProviderError::RequestFailed(
                    "No image found in messages. Local vision models require an image.".to_string(),
                )
            })?;

        let image_bytes = Self::decode_image(&image_base64)?;
        let prompt = Self::extract_prompt_from_messages(&request.messages);

        let _model_path = self.ensure_model_downloaded(local_model).await?;

        #[cfg(feature = "candle")]
        {
            let response_text = match local_model {
                LocalModel::Moondream | LocalModel::MoondreamQuantized => {
                    let mut guard = self.get_moondream(&_model_path)?;
                    let captioner = guard.as_mut().ok_or_else(|| {
                        ProviderError::ProviderNotAvailable("Moondream not initialized".to_string())
                    })?;

                    captioner
                        .caption_with_prompt(&image_bytes, &prompt)
                        .map_err(|e| {
                            ProviderError::RequestFailed(format!(
                                "Moondream inference failed: {}",
                                e
                            ))
                        })?
                }
                LocalModel::Blip | LocalModel::BlipQuantized => {
                    let mut guard = self.get_blip(&_model_path)?;
                    let captioner = guard.as_mut().ok_or_else(|| {
                        ProviderError::ProviderNotAvailable("BLIP not initialized".to_string())
                    })?;

                    captioner.caption_image(&image_bytes).map_err(|e| {
                        ProviderError::RequestFailed(format!("BLIP inference failed: {}", e))
                    })?
                }
                LocalModel::Clip => {
                    unreachable!("CLIP completion check happens above")
                }
            };

            Ok(CompletionResponse {
                message: Message::assistant(response_text),
                model: request.model,
                usage: None,
                stop_reason: Some("stop".to_string()),
            })
        }

        #[cfg(not(feature = "candle"))]
        {
            let _ = (image_bytes, prompt, local_model);
            Err(ProviderError::ProviderNotAvailable(
                "Candle feature not enabled. Rebuild with --features candle".to_string(),
            ))
        }
    }

    fn provider_name(&self) -> &str {
        "local"
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    fn supports_tools(&self) -> bool {
        false
    }

    fn available_models(&self) -> Vec<String> {
        vec![
            "moondream".to_string(),
            "moondream-quantized".to_string(),
            "blip".to_string(),
            "blip-quantized".to_string(),
            "clip".to_string(),
        ]
    }

    async fn list_available_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![
            ModelInfo::new("moondream", "Moondream 2")
                .with_capabilities(ModelCapabilities {
                    chat: true,
                    streaming: false,
                    tools: false,
                    embeddings: false,
                    vision: true,
                })
                .with_description("Promptable vision-language model for detailed image captioning"),
            ModelInfo::new("moondream-quantized", "Moondream 2 (Quantized)")
                .with_capabilities(ModelCapabilities {
                    chat: true,
                    streaming: false,
                    tools: false,
                    embeddings: false,
                    vision: true,
                })
                .with_description("Faster CPU inference, smaller model size"),
            ModelInfo::new("blip", "BLIP Large")
                .with_capabilities(ModelCapabilities {
                    chat: true,
                    streaming: false,
                    tools: false,
                    embeddings: false,
                    vision: true,
                })
                .with_description("Fast single-caption model for quick image descriptions"),
            ModelInfo::new("blip-quantized", "BLIP Large (Quantized)")
                .with_capabilities(ModelCapabilities {
                    chat: true,
                    streaming: false,
                    tools: false,
                    embeddings: false,
                    vision: true,
                })
                .with_description("Fastest CPU inference, smallest model size"),
            ModelInfo::new("clip", "CLIP ViT-B/32")
                .with_capabilities(ModelCapabilities {
                    chat: false,
                    streaming: false,
                    tools: false,
                    embeddings: true,
                    vision: true,
                })
                .with_description("Image embeddings for semantic search and similarity"),
        ])
    }

    async fn generate_embedding(&self, text: &str, model: &str) -> Result<Vec<f32>> {
        let local_model = LocalModel::from_model_id(model).ok_or_else(|| {
            ProviderError::InvalidModel(format!("Unknown local model '{}'", model))
        })?;

        if !local_model.supports_embeddings() {
            return Err(ProviderError::UnsupportedOperation(format!(
                "Model '{}' doesn't support embeddings. Use 'clip' for embeddings.",
                model
            )));
        }

        let _model_path = self.ensure_model_downloaded(local_model).await?;

        #[cfg(feature = "candle")]
        {
            let guard = self.get_clip(&_model_path)?;
            let embedder = guard.as_ref().ok_or_else(|| {
                ProviderError::ProviderNotAvailable("CLIP not initialized".to_string())
            })?;

            let is_base64 = text.len() > 100
                && !text.contains(' ')
                && text
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=');

            if is_base64 {
                let image_bytes = Self::decode_image(text)?;
                embedder.embed_image(&image_bytes).map_err(|e| {
                    ProviderError::RequestFailed(format!("CLIP embedding failed: {}", e))
                })
            } else {
                embedder.embed_text(text).map_err(|e| {
                    ProviderError::RequestFailed(format!("CLIP text embedding failed: {}", e))
                })
            }
        }

        #[cfg(not(feature = "candle"))]
        {
            let _ = (text, local_model);
            Err(ProviderError::ProviderNotAvailable(
                "Candle feature not enabled. Rebuild with --features candle".to_string(),
            ))
        }
    }
}
