// SPDX-License-Identifier: BSL-1.1

//! Tantivy schema building and configuration.

use std::collections::HashMap;

use tantivy::schema::*;

use super::language::{
    stemmed_analyzer_name, stemmed_content_field, stemmed_name_field, BASE_ANALYZER,
    STEMMED_LANGUAGES,
};
use super::types::{SchemaFields, StemmedFields};

/// Version of the Tantivy schema. Bump whenever the field set / tokenizers change
/// so on-disk indexes built with an older version can be detected and rebuilt.
/// v2 adds the `shape_types` field (shape-driven indexing).
/// v3 moves `name`/`content` off tantivy's `"default"` analyzer onto the
/// CJK-safe `raisin_text`, and adds a stemmed field pair per language.
///
/// **v3 requires a REINDEX to take effect.** The analyzer name lives in the
/// on-disk schema, so an index built at v2 keeps being written and searched with
/// the v2 analyzer — consistently, but unstemmed and with CJK still dropped —
/// until it is rebuilt. That is deliberate: it is a clean either/or rather than
/// a half-migration where old segments and new segments disagree about what a
/// term is. `is_index_stale` reports every v2 index as stale, which makes the
/// dev tenant auto-rebuild, escalates `reconcile_fulltext_index` to a full
/// rebuild, and logs the operator warning in production.
pub(crate) const SCHEMA_VERSION: u32 = 3;

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

    // Language-neutral, and the ONLY stored copy of the text. `raisin_text`
    // rather than tantivy's `"default"`: see `language::BASE_ANALYZER`.
    let text_options = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(BASE_ANALYZER)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();

    let name = schema_builder.add_text_field("name", text_options.clone());
    let content = schema_builder.add_text_field("content", text_options);

    // One stemmed pair per language. Not stored — these exist only to be
    // matched against; `extract_results` always reads the neutral `name`.
    //
    // A language a document never uses costs nothing on disk: tantivy writes
    // postings only for fields a document actually carries.
    let stemmed_options = |analyzer: &str| {
        TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(analyzer)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
    };
    let mut stemmed = HashMap::with_capacity(STEMMED_LANGUAGES.len());
    for lang in STEMMED_LANGUAGES {
        let analyzer = stemmed_analyzer_name(lang);
        let name_field =
            schema_builder.add_text_field(&stemmed_name_field(lang), stemmed_options(&analyzer));
        let content_field =
            schema_builder.add_text_field(&stemmed_content_field(lang), stemmed_options(&analyzer));
        stemmed.insert(
            (*lang).to_string(),
            StemmedFields {
                name: name_field,
                content: content_field,
            },
        );
    }

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
        stemmed,
    };

    (schema, fields)
}

/// Resolves [`SchemaFields`] from an index's ACTUAL schema (which may be older
/// than the current code schema). Always-present fields are required; newer
/// fields like `shape_types` and the per-language stemmed pairs resolve to
/// `None` / an empty map on an older index, so writes and searches degrade
/// gracefully instead of referencing a non-existent field.
pub(crate) fn schema_fields(schema: &Schema) -> SchemaFields {
    let required = |name: &str| {
        schema
            .get_field(name)
            .unwrap_or_else(|_| panic!("fulltext index schema missing required field `{}`", name))
    };

    // Both halves of a pair or neither: a half-present pair would mean writing
    // stemmed content that nothing searches, or searching a field nothing writes.
    let mut stemmed = HashMap::new();
    for lang in STEMMED_LANGUAGES {
        if let (Ok(name), Ok(content)) = (
            schema.get_field(&stemmed_name_field(lang)),
            schema.get_field(&stemmed_content_field(lang)),
        ) {
            stemmed.insert((*lang).to_string(), StemmedFields { name, content });
        }
    }

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
        stemmed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_freshly_built_schema_resolves_every_stemmed_pair() {
        let (schema, built) = build_schema();
        let resolved = schema_fields(&schema);
        assert_eq!(resolved.stemmed.len(), STEMMED_LANGUAGES.len());
        for lang in STEMMED_LANGUAGES {
            let a = built.stemmed.get(*lang).expect("built pair");
            let b = resolved.stemmed.get(*lang).expect("resolved pair");
            assert_eq!(a.name, b.name);
            assert_eq!(a.content, b.content);
        }
    }

    #[test]
    fn the_text_fields_name_the_cjk_safe_analyzer() {
        let (schema, fields) = build_schema();
        for field in [fields.name, fields.content] {
            let FieldType::Str(options) = schema.get_field_entry(field).field_type() else {
                panic!("name/content must be text fields");
            };
            let indexing = options
                .get_indexing_options()
                .expect("name/content must be indexed");
            assert_eq!(indexing.tokenizer(), BASE_ANALYZER);
        }
    }
}
