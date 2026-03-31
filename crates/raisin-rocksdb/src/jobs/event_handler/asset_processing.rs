//! Asset processing logic for the event handler
//!
//! Handles extraction of file metadata (storage keys, content hashes, MIME types)
//! from node properties, and builds processing options based on configured rules.

use super::UnifiedJobEventHandler;
use raisin_ai::RuleMatchContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::jobs::{AssetProcessingOptions, JobContext, PdfExtractionStrategy};
use raisin_storage::ProcessingRulesRepository;

impl UnifiedJobEventHandler {
    /// Check if an asset node should be processed for AI features
    ///
    /// Returns true if:
    /// 1. Node type is "raisin:Asset"
    /// 2. File property exists with a storage_key (file is ready)
    /// 3. Processing rules allow this asset to be processed
    ///
    /// # Arguments
    /// * `node` - The node to check (required for file property inspection)
    pub(crate) fn should_process_asset(&self, node: &raisin_models::nodes::Node) -> bool {
        // Only process raisin:Asset nodes
        if node.node_type != "raisin:Asset" {
            return false;
        }

        // Check if file property exists and has storage_key (file is ready)
        let has_storage_key = self.extract_storage_key(node).is_some();

        if !has_storage_key {
            tracing::debug!(
                node_id = %node.id,
                "Skipping asset processing: no storage_key (file not ready)"
            );
            return false;
        }

        true
    }

    /// Extract a string value from a PropertyValue
    fn property_value_as_str(pv: &PropertyValue) -> Option<String> {
        match pv {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Extract storage_key from a node's file property
    ///
    /// Handles both Resource and Object property value types
    pub(crate) fn extract_storage_key(&self, node: &raisin_models::nodes::Node) -> Option<String> {
        node.properties.get("file").and_then(|file_prop| {
            match file_prop {
                PropertyValue::Resource(res) => res
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("storage_key"))
                    .and_then(Self::property_value_as_str),
                PropertyValue::Object(obj) => {
                    // Also check nested metadata.storage_key for Object format
                    obj.get("metadata")
                        .and_then(|m| {
                            if let PropertyValue::Object(inner) = m {
                                inner
                                    .get("storage_key")
                                    .and_then(Self::property_value_as_str)
                            } else {
                                None
                            }
                        })
                        // Fallback to direct storage_key
                        .or_else(|| obj.get("storage_key").and_then(Self::property_value_as_str))
                }
                _ => None,
            }
        })
    }

    /// Extract content_hash from a node's file property
    ///
    /// Used for deduplication - prevents re-processing the same binary
    pub(crate) fn extract_content_hash(&self, node: &raisin_models::nodes::Node) -> Option<String> {
        node.properties
            .get("file")
            .and_then(|file_prop| match file_prop {
                PropertyValue::Resource(res) => res
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("content_hash"))
                    .and_then(Self::property_value_as_str),
                PropertyValue::Object(obj) => obj
                    .get("metadata")
                    .and_then(|m| {
                        if let PropertyValue::Object(inner) = m {
                            inner
                                .get("content_hash")
                                .and_then(Self::property_value_as_str)
                        } else {
                            None
                        }
                    })
                    .or_else(|| {
                        obj.get("content_hash")
                            .and_then(Self::property_value_as_str)
                    }),
                _ => None,
            })
    }

    /// Extract MIME type from a node's file property
    pub(crate) fn extract_mime_type(&self, node: &raisin_models::nodes::Node) -> Option<String> {
        node.properties
            .get("file")
            .and_then(|file_prop| match file_prop {
                PropertyValue::Resource(res) => res.mime_type.clone(),
                PropertyValue::Object(obj) => obj
                    .get("mime_type")
                    .and_then(Self::property_value_as_str)
                    .or_else(|| obj.get("mimeType").and_then(Self::property_value_as_str)),
                _ => None,
            })
    }

    /// Build asset processing options based on file type and processing rules
    ///
    /// Looks up processing rules configured in admin-console for the repository.
    /// Falls back to MIME-type based defaults if no rules are configured.
    pub(crate) async fn get_asset_processing_options(
        &self,
        node: &raisin_models::nodes::Node,
        context: &JobContext,
    ) -> AssetProcessingOptions {
        let mime_type = self.extract_mime_type(node);
        let content_hash = self.extract_content_hash(node);
        let mime = mime_type.as_deref().unwrap_or("");

        // Try to find matching processing rule
        if let Ok(Some(rule_set)) = self
            .processing_rules
            .get_rules(raisin_storage::RepoScope::new(
                &context.tenant_id,
                &context.repo_id,
            ))
            .await
        {
            // Build match context from node
            let match_context = RuleMatchContext::new()
                .with_node_type(&node.node_type)
                .with_path(&node.path)
                .with_mime_type(mime)
                .with_workspace(&context.workspace_id);

            // Find matching rule
            if let Some(rule) = rule_set.find_matching_rule(&match_context) {
                tracing::debug!(
                    rule_id = %rule.id,
                    rule_name = %rule.name,
                    node_path = %node.path,
                    mime_type = %mime,
                    "Matched processing rule"
                );

                let settings = &rule.settings;
                return AssetProcessingOptions {
                    extract_pdf_text: mime == "application/pdf",
                    generate_image_embedding: settings.generate_image_embedding.unwrap_or(false),
                    generate_image_caption: settings.generate_image_caption.unwrap_or(false),
                    pdf_strategy: settings
                        .pdf_strategy
                        .map(|s| match s {
                            raisin_ai::PdfStrategy::Auto => PdfExtractionStrategy::Auto,
                            raisin_ai::PdfStrategy::NativeOnly => PdfExtractionStrategy::NativeOnly,
                            raisin_ai::PdfStrategy::OcrOnly => PdfExtractionStrategy::OcrOnly,
                            raisin_ai::PdfStrategy::ForceOcr => PdfExtractionStrategy::ForceOcr,
                        })
                        .unwrap_or_default(),
                    store_extracted_text: settings.store_extracted_text.unwrap_or(true),
                    trigger_embedding: settings.trigger_embedding.unwrap_or(true),
                    content_hash,
                    caption_model: settings.caption_model.clone(),
                    embedding_model: settings.embedding_model.clone(),
                    alt_text_prompt: settings.alt_text_prompt.clone(),
                    description_prompt: settings.description_prompt.clone(),
                    generate_keywords: settings
                        .generate_keywords
                        .unwrap_or(mime.starts_with("image/")),
                    keywords_prompt: settings.keywords_prompt.clone(),
                };
            }
        }

        // Fall back to MIME-type based defaults (no rules configured)
        let is_pdf = mime == "application/pdf";
        let is_image = mime.starts_with("image/");

        tracing::debug!(
            node_path = %node.path,
            mime_type = %mime,
            is_pdf = %is_pdf,
            is_image = %is_image,
            "No processing rule matched, using MIME-type defaults"
        );

        AssetProcessingOptions {
            extract_pdf_text: is_pdf,
            generate_image_embedding: is_image,
            generate_image_caption: is_image,
            pdf_strategy: Default::default(),
            store_extracted_text: true,
            trigger_embedding: true,
            content_hash,
            caption_model: None,         // Use system default (Moondream)
            embedding_model: None,       // Use system default (CLIP)
            alt_text_prompt: None,       // Use default prompts
            description_prompt: None,    // Use default prompts
            generate_keywords: is_image, // Default to generating keywords for images
            keywords_prompt: None,       // Use default prompt
        }
    }
}
