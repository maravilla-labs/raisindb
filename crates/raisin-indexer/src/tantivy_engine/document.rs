// SPDX-License-Identifier: BSL-1.1

//! Document creation for Tantivy indexing.

use raisin_models::nodes::Node;
use raisin_storage::fulltext::FullTextIndexJob;
use tantivy::{doc, TantivyDocument};

use super::types::SchemaFields;

/// Creates a Tantivy document for a node in a specific language.
pub(crate) fn create_document(
    job: &FullTextIndexJob,
    node: &Node,
    language: &str,
    name: &str,
    content: &str,
    fields: &SchemaFields,
) -> TantivyDocument {
    let doc_id = format!("{}-{}-{}-{}", node.id, job.branch, job.revision, language);

    let created_at = node
        .created_at
        .map(|dt| tantivy::DateTime::from_timestamp_millis(dt.timestamp_millis()))
        .unwrap_or_else(|| tantivy::DateTime::from_timestamp_millis(0));
    let updated_at = node
        .updated_at
        .map(|dt| tantivy::DateTime::from_timestamp_millis(dt.timestamp_millis()))
        .unwrap_or_else(|| tantivy::DateTime::from_timestamp_millis(0));

    doc!(
        fields.doc_id => doc_id,
        fields.node_id => node.id.clone(),
        fields.workspace_id => job.workspace_id.clone(),
        fields.language => language.to_string(),
        fields.path => node.path.clone(),
        fields.node_type => node.node_type.clone(),
        fields.revision_timestamp => job.revision.timestamp_ms,
        fields.revision_counter => job.revision.counter,
        fields.created_at => created_at,
        fields.updated_at => updated_at,
        fields.name => name.to_string(),
        fields.content => content.to_string(),
    )
}
