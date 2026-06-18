// SPDX-License-Identifier: BSL-1.1

//! Document creation for Tantivy indexing.

use raisin_models::nodes::Node;
use raisin_storage::fulltext::FullTextIndexJob;
use tantivy::TantivyDocument;

use super::types::SchemaFields;

/// Creates a Tantivy document for a node in a specific language.
///
/// `shape_types` is multi-valued: the field is written once per distinct shape
/// identity (node_type / archetype / nested element_types), so a `doc!` macro
/// (fixed-arity) can't express it — we build a mutable document instead.
pub(crate) fn create_document(
    job: &FullTextIndexJob,
    node: &Node,
    language: &str,
    name: &str,
    content: &str,
    shape_types: &[String],
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

    let mut doc = TantivyDocument::new();
    doc.add_text(fields.doc_id, &doc_id);
    doc.add_text(fields.node_id, &node.id);
    doc.add_text(fields.workspace_id, &job.workspace_id);
    doc.add_text(fields.language, language);
    doc.add_text(fields.path, &node.path);
    doc.add_text(fields.node_type, &node.node_type);
    doc.add_u64(fields.revision_timestamp, job.revision.timestamp_ms);
    doc.add_u64(fields.revision_counter, job.revision.counter);
    doc.add_date(fields.created_at, created_at);
    doc.add_date(fields.updated_at, updated_at);
    doc.add_text(fields.name, name);
    doc.add_text(fields.content, content);
    // Only written when the on-disk schema has the field (v2+); a pre-v2 index
    // omits shape_types until it is rebuilt.
    if let Some(shape_field) = fields.shape_types {
        for shape in shape_types {
            doc.add_text(shape_field, shape);
        }
    }
    doc
}
