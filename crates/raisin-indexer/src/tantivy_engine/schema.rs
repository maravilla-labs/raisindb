// SPDX-License-Identifier: BSL-1.1

//! Tantivy schema building and configuration.

use tantivy::schema::*;

use super::types::SchemaFields;

/// Version of the Tantivy schema. Bump whenever the field set / tokenizers change
/// so on-disk indexes built with an older version can be detected and rebuilt.
/// v2 adds the `shape_types` field (shape-driven indexing).
pub(crate) const SCHEMA_VERSION: u32 = 2;

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

    // Shape-type identities (node_type / archetype / nested element_types).
    // Raw tokenizer keeps each whole `ns:TypeName` as a single exact term — the
    // default tokenizer would split on `:` and `_`. Multi-valued (written once
    // per distinct identity) and stored.
    let shape_types_options = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("raw")
                .set_index_option(IndexRecordOption::Basic),
        )
        .set_stored();
    let shape_types = schema_builder.add_text_field("shape_types", shape_types_options);

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
        shape_types: Some(shape_types),
    };

    (schema, fields)
}

/// Resolves [`SchemaFields`] from an index's ACTUAL schema (which may be older
/// than the current code schema). Always-present fields are required; newer
/// fields like `shape_types` resolve to `None` on a pre-v2 index, so writes and
/// searches degrade gracefully instead of referencing a non-existent field.
pub(crate) fn schema_fields(schema: &Schema) -> SchemaFields {
    let required = |name: &str| {
        schema
            .get_field(name)
            .unwrap_or_else(|_| panic!("fulltext index schema missing required field `{}`", name))
    };
    SchemaFields {
        doc_id: required("doc_id"),
        node_id: required("node_id"),
        workspace_id: required("workspace_id"),
        language: required("language"),
        path: required("path"),
        node_type: required("node_type"),
        revision_timestamp: required("revision_timestamp"),
        revision_counter: required("revision_counter"),
        created_at: required("created_at"),
        updated_at: required("updated_at"),
        name: required("name"),
        content: required("content"),
        shape_types: schema.get_field("shape_types").ok(),
    }
}
