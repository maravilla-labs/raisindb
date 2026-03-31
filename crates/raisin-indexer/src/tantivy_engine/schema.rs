// SPDX-License-Identifier: BSL-1.1

//! Tantivy schema building and configuration.

use tantivy::schema::*;

use super::types::SchemaFields;

/// Builds the Tantivy schema for RaisinDB documents.
pub(crate) fn build_schema() -> (Schema, SchemaFields) {
    let mut schema_builder = Schema::builder();

    let doc_id = schema_builder.add_text_field("doc_id", STRING | STORED);
    let node_id = schema_builder.add_text_field("node_id", STRING | STORED);
    let workspace_id = schema_builder.add_text_field("workspace_id", STRING | STORED);
    let language = schema_builder.add_text_field("language", STRING | STORED);
    let path = schema_builder.add_text_field("path", STRING | STORED);
    let node_type = schema_builder.add_text_field("node_type", STRING | STORED);

    let revision_timestamp = schema_builder.add_u64_field("revision_timestamp", INDEXED | STORED);
    let revision_counter = schema_builder.add_u64_field("revision_counter", INDEXED | STORED);
    let created_at = schema_builder.add_date_field("created_at", INDEXED | STORED);
    let updated_at = schema_builder.add_date_field("updated_at", INDEXED | STORED);

    let text_options = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("default")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();

    let name = schema_builder.add_text_field("name", text_options.clone());
    let content = schema_builder.add_text_field("content", text_options);

    let schema = schema_builder.build();
    let fields = SchemaFields {
        doc_id,
        node_id,
        workspace_id,
        language,
        path,
        node_type,
        revision_timestamp,
        revision_counter,
        created_at,
        updated_at,
        name,
        content,
    };

    (schema, fields)
}
