// SPDX-License-Identifier: BSL-1.1

//! IndexingEngine trait implementation for TantivyIndexingEngine.

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::fulltext::{
    FullTextIndexJob, FullTextSearchQuery, FullTextSearchResult, IndexingEngine,
};

use super::document::create_document;
use super::language::register_language_tokenizer;
use super::properties::flatten_properties;
use super::schema::build_schema;
use super::search::execute_search;
use super::types::TantivyIndexingEngine;
use super::utils::copy_dir_recursive;

impl IndexingEngine for TantivyIndexingEngine {
    fn do_index_node(&self, job: &FullTextIndexJob, node: &Node) -> Result<()> {
        let cached = self.get_or_create_index(&job.tenant_id, &job.repo_id, &job.branch)?;
        let index = &cached.index;
        let default_lang = &job.default_language;

        register_language_tokenizer(index, default_lang)?;

        let (_schema, fields) = build_schema();
        let default_content = flatten_properties(job, &node.properties);

        let default_doc = create_document(
            job,
            node,
            default_lang,
            &node.name,
            &default_content,
            &fields,
        );
        let mut documents = vec![default_doc];

        if let Some(translations) = &node.translations {
            for lang_code in &job.supported_languages {
                if lang_code == default_lang {
                    continue;
                }

                register_language_tokenizer(index, lang_code)?;

                let translated_name = translations
                    .get(&format!("name_{}", lang_code))
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or(&node.name);

                let translated_content = default_content.clone();
                let doc = create_document(
                    job,
                    node,
                    lang_code,
                    translated_name,
                    &translated_content,
                    &fields,
                );
                documents.push(doc);
            }
        }

        let mut writer = Self::get_writer(index)?;
        let node_id_term = tantivy::Term::from_field_text(fields.node_id, &node.id);
        writer.delete_term(node_id_term);

        for doc in documents {
            writer
                .add_document(doc)
                .map_err(|e| Error::storage(format!("Failed to add document: {}", e)))?;
        }

        writer
            .commit()
            .map_err(|e| Error::storage(format!("Failed to commit index: {}", e)))?;

        Ok(())
    }

    fn do_delete_node(&self, job: &FullTextIndexJob) -> Result<()> {
        let cached = self.get_or_create_index(&job.tenant_id, &job.repo_id, &job.branch)?;
        let index = &cached.index;

        let (_schema, fields) = build_schema();
        let mut writer = Self::get_writer(index)?;

        if let Some(node_id) = &job.node_id {
            let node_id_term = tantivy::Term::from_field_text(fields.node_id, node_id);
            writer.delete_term(node_id_term);
        }

        writer
            .commit()
            .map_err(|e| Error::storage(format!("Failed to commit deletion: {}", e)))?;

        Ok(())
    }

    fn do_branch_created(&self, job: &FullTextIndexJob) -> Result<()> {
        let source_branch = job.source_branch.as_ref().ok_or_else(|| {
            Error::Validation("source_branch is required for branch_created operation".to_string())
        })?;

        let source_path = self
            .base_path
            .join(&job.tenant_id)
            .join(&job.repo_id)
            .join(source_branch);

        let target_path = self
            .base_path
            .join(&job.tenant_id)
            .join(&job.repo_id)
            .join(&job.branch);

        if !source_path.exists() {
            return Err(Error::NotFound(format!(
                "Source branch index not found: {}",
                source_branch
            )));
        }

        copy_dir_recursive(&source_path, &target_path)?;

        Ok(())
    }

    fn search(&self, query: &FullTextSearchQuery) -> Result<Vec<FullTextSearchResult>> {
        execute_search(self, query)
    }
}
