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

        /* TEXT FIRST, and it has to be before the image extraction below.
         *
         * Every other local model REQUIRES an image and errors without one, so
         * a text request routed through that check fails with "No image found
         * in messages" — which reads as a malformed request rather than as a
         * model that simply answers prompts. */
        if local_model.is_text() {
            return self.complete_text(request, local_model).await;
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
                LocalModel::Qwen25Coder => {
                    unreachable!("text models return via complete_text above")
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

    /// Still false: the text model generates in one blocking call rather than
    /// yielding a stream, and claiming otherwise would make callers wait for a
    /// `stream_complete` that falls back to this anyway.
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Qwen2.5-Instruct is trained for tool calls, but this provider does not
    /// yet parse them out of the text, so the honest answer is no. Saying yes
    /// would have agent code hand it tools and then read an empty tool_calls
    /// array as "the model chose not to call one".
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
            "qwen2.5-coder".to_string(),
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

impl LocalCandleProvider {
    /// Answer a TEXT prompt with the local Qwen model.
    ///
    /// Kept beside the vision path rather than inside it because almost nothing
    /// is shared: no image decode, no captioner, and a different failure mode —
    /// the vision models fail when an image is missing, this one fails when the
    /// weights are.
    async fn complete_text(
        &self,
        request: CompletionRequest,
        local_model: LocalModel,
    ) -> Result<CompletionResponse> {
        // Downloads on first use (~1.1 GB) — the whole point of a local default
        // is that nobody has to fetch it by hand first.
        let _model_path = self.ensure_model_downloaded(local_model).await?;

        #[cfg(feature = "candle")]
        {
            use crate::candle::ChatTurn;

            /* `system` ON THE REQUEST IS A REAL TURN. The engine carries it as
             * a field rather than as a message, and ChatML has no other place
             * to put it, so dropping it here would silently discard every
             * instruction a caller gave — the failure that looks like a model
             * ignoring its prompt. */
            let mut turns: Vec<ChatTurn<'_>> = Vec::new();
            if let Some(system) = request.system.as_deref() {
                if !system.trim().is_empty() {
                    turns.push(ChatTurn {
                        role: "system",
                        content: system,
                    });
                }
            }
            /* ChatML names roles as bare strings, and `Role` has no Display —
             * deliberately mapped here rather than derived, because `Tool` has
             * no ChatML role of its own and folding it into `user` is a choice,
             * not a formatting detail. */
            let rendered: Vec<(&'static str, String)> = request
                .messages
                .iter()
                .map(|m| {
                    let role = match m.role {
                        crate::types::Role::System => "system",
                        crate::types::Role::Assistant => "assistant",
                        crate::types::Role::User | crate::types::Role::Tool => "user",
                    };
                    (role, m.effective_text())
                })
                .filter(|(_, text): &(&str, String)| !text.trim().is_empty())
                .collect();
            for (role, text) in &rendered {
                turns.push(ChatTurn {
                    role,
                    content: text.as_str(),
                });
            }

            let prompt = crate::candle::QwenGenerator::build_prompt(&turns);
            /* CAPPED WELL BELOW WHAT A CALLER ASKS FOR. Agents routinely set
             * max_tokens to 4096 for a cloud model where that is seconds; here
             * it is minutes, and a structured answer — which is what this model
             * is for — is comfortably under 1024. The cap is what keeps a
             * default configuration from pinning the machine. */
            let max_tokens = (request.max_tokens.unwrap_or(1024) as usize).min(1024);
            let temperature = request.temperature.map(|t| t as f64);

            let text = {
                let mut guard = self.get_qwen(&_model_path)?;
                let generator = guard.as_mut().ok_or_else(|| {
                    ProviderError::ProviderNotAvailable("Qwen not initialized".to_string())
                })?;
                // Fixed seed: a temperature of 0 is greedy and the seed is
                // unused, and when a caller does ask for temperature they want
                // variety across turns, not across restarts — a wandering
                // default would make a failing prompt impossible to reproduce.
                generator
                    .generate(&prompt, max_tokens, temperature, 42)
                    .map_err(|e| {
                        ProviderError::RequestFailed(format!("Qwen inference failed: {}", e))
                    })?
            };

            Ok(CompletionResponse {
                message: Message::assistant(text),
                model: request.model,
                usage: None,
                stop_reason: Some("stop".to_string()),
            })
        }

        #[cfg(not(feature = "candle"))]
        {
            let _ = request;
            Err(ProviderError::ProviderNotAvailable(
                "Candle feature not enabled. Rebuild with --features candle".to_string(),
            ))
        }
    }
}
