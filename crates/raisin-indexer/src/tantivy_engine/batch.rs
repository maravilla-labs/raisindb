// SPDX-License-Identifier: BSL-1.1

//! Batch indexing operations for bulk performance.

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::fulltext::FullTextIndexJob;

use super::document::create_document;
use super::language::register_language_tokenizer;
use super::properties::flatten_properties;
use super::schema::build_schema;
use super::types::{BatchIndexContext, TantivyIndexingEngine};

impl TantivyIndexingEngine {
    /// Batch index multiple nodes with a single Tantivy commit.
    pub fn do_batch_index(
        &self,
        context: &BatchIndexContext,
        nodes: Vec<Node>,
        delete_node_ids: Vec<String>,
    ) -> Result<usize> {
        if nodes.is_empty() && delete_node_ids.is_empty() {
            return Ok(0);
        }

        tracing::debug!(
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            nodes_to_index = nodes.len(),
            nodes_to_delete = delete_node_ids.len(),
            "Starting batch index operation"
        );

        let cached =
            self.get_or_create_index(&context.tenant_id, &context.repo_id, &context.branch)?;
        let index = &cached.index;

        register_language_tokenizer(index, &context.default_language)?;
        for lang in &context.supported_languages {
            register_language_tokenizer(index, lang)?;
        }

        let (_schema, fields) = build_schema();
        let mut writer = Self::get_writer(index)?;
        let mut processed = 0;

        for node_id in &delete_node_ids {
            let term = tantivy::Term::from_field_text(fields.node_id, node_id);
            writer.delete_term(term);
            processed += 1;
        }

        for node in &nodes {
            let node_id_term = tantivy::Term::from_field_text(fields.node_id, &node.id);
            writer.delete_term(node_id_term);

            let temp_job = FullTextIndexJob {
                job_id: "batch".to_string(),
                kind: raisin_storage::fulltext::JobKind::AddNode,
                tenant_id: context.tenant_id.clone(),
                repo_id: context.repo_id.clone(),
                workspace_id: context.workspace_id.clone(),
                branch: context.branch.clone(),
                revision: raisin_hlc::HLC::new(0, 0),
                node_id: Some(node.id.clone()),
                source_branch: None,
                default_language: context.default_language.clone(),
                supported_languages: context.supported_languages.clone(),
                properties_to_index: None,
            };

            let default_content = flatten_properties(&temp_job, &node.properties);
            let default_doc = create_document(
                &temp_job,
                node,
                &context.default_language,
                &node.name,
                &default_content,
                &fields,
            );

            writer
                .add_document(default_doc)
                .map_err(|e| Error::storage(format!("Failed to add document: {}", e)))?;

            if let Some(translations) = &node.translations {
                for lang_code in &context.supported_languages {
                    if lang_code == &context.default_language {
                        continue;
                    }

                    let translated_name = translations
                        .get(&format!("name_{}", lang_code))
                        .and_then(|v| match v {
                            PropertyValue::String(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .unwrap_or(&node.name);

                    let doc = create_document(
                        &temp_job,
                        node,
                        lang_code,
                        translated_name,
                        &default_content,
                        &fields,
                    );

                    writer
                        .add_document(doc)
                        .map_err(|e| Error::storage(format!("Failed to add document: {}", e)))?;
                }
            }

            processed += 1;
        }

        writer
            .commit()
            .map_err(|e| Error::storage(format!("Failed to commit batch: {}", e)))?;

        tracing::debug!(
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            nodes_indexed = nodes.len(),
            nodes_deleted = delete_node_ids.len(),
            total_processed = processed,
            "Batch index completed successfully"
        );

        Ok(processed)
    }
}
