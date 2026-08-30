// SPDX-License-Identifier: BSL-1.1

//! Full-text search implementation for TantivyIndexingEngine.

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_storage::fulltext::{FullTextSearchQuery, FullTextSearchResult};
use tantivy::schema::Value;

use super::query::{build_hlc_le_query, contains_wildcards, wildcard_to_regex};
use super::types::{SchemaFields, TantivyIndexingEngine};

/// Executes a full-text search query against the Tantivy index.
pub(crate) fn execute_search(
    engine: &TantivyIndexingEngine,
    query: &FullTextSearchQuery,
) -> Result<Vec<FullTextSearchResult>> {
    let cached = engine.get_or_create_index(&query.tenant_id, &query.repo_id, &query.branch)?;
    let index = &cached.index;
    let reader = &cached.reader;

    let fields = &cached.fields;
    let searcher = reader.searcher();

    let text_query: Box<dyn tantivy::query::Query> = if contains_wildcards(&query.query) {
        build_wildcard_query(&query.query, fields)?
    } else {
        build_text_query(index, &query.query, &query.language, fields)?
    };

    let language_term = tantivy::Term::from_field_text(fields.language, &query.language);
    let language_query =
        tantivy::query::TermQuery::new(language_term, tantivy::schema::IndexRecordOption::Basic);

    let mut must_clauses: Vec<(tantivy::query::Occur, Box<dyn tantivy::query::Query>)> = vec![
        (
            tantivy::query::Occur::Must,
            Box::new(language_query) as Box<dyn tantivy::query::Query>,
        ),
        (tantivy::query::Occur::Must, text_query),
    ];

    add_workspace_filter(&mut must_clauses, &query.workspace_ids, fields);

    if let Some(revision) = query.revision {
        let revision_query = build_hlc_le_query(
            fields.revision_timestamp,
            fields.revision_counter,
            &revision,
        );
        must_clauses.push((tantivy::query::Occur::Must, revision_query));
    }

    add_shape_type_filter(&mut must_clauses, &query.shape_types, fields);

    let final_query = tantivy::query::BooleanQuery::new(must_clauses);

    let limit = query.limit.min(1000);
    let top_docs = searcher
        .search(
            &final_query,
            &tantivy::collector::TopDocs::with_limit(limit),
        )
        .map_err(|e| Error::storage(format!("Search failed: {}", e)))?;

    extract_results(&searcher, top_docs, fields)
}

/// Wildcards run over the language-NEUTRAL fields only.
///
/// A regex over a stemmed term dictionary matches stems, not words, so
/// `Datenbank*` would silently mean "starts with the stem of Datenbank" — the
/// pattern the user wrote no longer describes what is being matched. Keeping
/// wildcards on the unstemmed text is the predictable behaviour, and the
/// neutral fields carry every document's full text.
fn build_wildcard_query(
    query_str: &str,
    fields: &SchemaFields,
) -> Result<Box<dyn tantivy::query::Query>> {
    tracing::debug!("Wildcard query detected: '{}'", query_str);
    let regex_pattern = wildcard_to_regex(query_str);
    tracing::debug!("Converted to regex: '{}'", regex_pattern);

    let name_regex = tantivy::query::RegexQuery::from_pattern(&regex_pattern, fields.name)
        .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?;

    let content_regex = tantivy::query::RegexQuery::from_pattern(&regex_pattern, fields.content)
        .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?;

    Ok(Box::new(tantivy::query::BooleanQuery::new(vec![
        (
            tantivy::query::Occur::Should,
            Box::new(name_regex) as Box<dyn tantivy::query::Query>,
        ),
        (
            tantivy::query::Occur::Should,
            Box::new(content_regex) as Box<dyn tantivy::query::Query>,
        ),
    ])))
}

/// The ordinary text query: the language-neutral fields with typo tolerance,
/// plus this language's stemmed pair for morphology.
///
/// The two are complementary and neither subsumes the other:
///
/// * **Fuzzy** is edit distance. It reaches `Datenbnak` -> `Datenbank`, and it
///   is deliberately confined to the neutral fields — an edit-distance-1 walk
///   over a *stemmed* dictionary conflates unrelated stems and inflates recall
///   with nonsense.
/// * **Stemming** is morphology. It reaches `Datenbanken` -> `Datenbank`, which
///   is edit distance 2 and not a prefix relation, so fuzzy cannot.
///
/// Tantivy analyses each query term with the analyzer of the field it is being
/// parsed against, so the stemmed clause below is stemmed on the query side
/// too — which is the only way it can meet the stemmed terms in the index.
///
/// A language with no stemmer (ja, zh, ko, th, …) and an index built before the
/// pairs existed both fall back to the neutral fields alone, exactly matching
/// what `create_document` wrote.
fn build_text_query(
    index: &tantivy::Index,
    query_str: &str,
    language: &str,
    fields: &SchemaFields,
) -> Result<Box<dyn tantivy::query::Query>> {
    let stemmed = fields.stemmed.get(language).copied();
    tracing::debug!(
        query = %query_str,
        language = %language,
        stemmed = stemmed.is_some(),
        "Building text query"
    );

    let mut search_fields = vec![fields.name, fields.content];
    if let Some(stemmed) = stemmed {
        search_fields.push(stemmed.name);
        search_fields.push(stemmed.content);
    }

    let mut query_parser = tantivy::query::QueryParser::for_index(index, search_fields);

    query_parser.set_field_fuzzy(fields.name, true, 1, true);
    query_parser.set_field_fuzzy(fields.content, true, 1, true);

    Ok(Box::new(query_parser.parse_query(query_str).map_err(
        |e| Error::Validation(format!("Invalid search query: {}", e)),
    )?))
}

fn add_workspace_filter(
    must_clauses: &mut Vec<(tantivy::query::Occur, Box<dyn tantivy::query::Query>)>,
    workspace_ids: &Option<Vec<String>>,
    fields: &SchemaFields,
) {
    match workspace_ids {
        None => {
            tracing::debug!("Cross-workspace search (no workspace filter)");
        }
        Some(workspace_ids) if workspace_ids.len() == 1 => {
            let workspace_term =
                tantivy::Term::from_field_text(fields.workspace_id, &workspace_ids[0]);
            let workspace_query = tantivy::query::TermQuery::new(
                workspace_term,
                tantivy::schema::IndexRecordOption::Basic,
            );
            tracing::debug!("Single workspace search: {}", workspace_ids[0]);
            must_clauses.push((
                tantivy::query::Occur::Must,
                Box::new(workspace_query) as Box<dyn tantivy::query::Query>,
            ));
        }
        Some(workspace_ids) => {
            let workspace_clauses: Vec<(tantivy::query::Occur, Box<dyn tantivy::query::Query>)> =
                workspace_ids
                    .iter()
                    .map(|ws_id| {
                        let term = tantivy::Term::from_field_text(fields.workspace_id, ws_id);
                        let query = tantivy::query::TermQuery::new(
                            term,
                            tantivy::schema::IndexRecordOption::Basic,
                        );
                        (
                            tantivy::query::Occur::Should,
                            Box::new(query) as Box<dyn tantivy::query::Query>,
                        )
                    })
                    .collect();
            let workspace_or_query = tantivy::query::BooleanQuery::new(workspace_clauses);
            tracing::debug!("Multiple workspace search: {:?}", workspace_ids);
            must_clauses.push((
                tantivy::query::Occur::Must,
                Box::new(workspace_or_query) as Box<dyn tantivy::query::Query>,
            ));
        }
    }
}

/// Restrict to nodes carrying ANY of the given shape-type identities.
///
/// Mirrors `add_workspace_filter`: one value becomes a `TermQuery`, several
/// become a `Should` boolean. `fields.shape_types` is absent on an index built
/// before the field existed, and the filter then degrades to no filter rather
/// than to no results.
///
/// The field is multi-valued (node_type + archetype + nested element_type), so
/// this OVER-MATCHES a node_type predicate on purpose. It is a candidate-pool
/// widener under an authoritative residual filter, never an authorisation or a
/// correctness mechanism.
fn add_shape_type_filter(
    must_clauses: &mut Vec<(tantivy::query::Occur, Box<dyn tantivy::query::Query>)>,
    shape_types: &Option<Vec<String>>,
    fields: &SchemaFields,
) {
    let (Some(shape_types), Some(shape_field)) = (shape_types, fields.shape_types) else {
        return;
    };
    if shape_types.is_empty() {
        return;
    }

    let term_query = |value: &str| {
        tantivy::query::TermQuery::new(
            tantivy::Term::from_field_text(shape_field, value),
            tantivy::schema::IndexRecordOption::Basic,
        )
    };

    if shape_types.len() == 1 {
        tracing::debug!("Shape-type exact filter: {}", shape_types[0]);
        must_clauses.push((
            tantivy::query::Occur::Must,
            Box::new(term_query(&shape_types[0])) as Box<dyn tantivy::query::Query>,
        ));
        return;
    }

    let clauses: Vec<(tantivy::query::Occur, Box<dyn tantivy::query::Query>)> = shape_types
        .iter()
        .map(|value| {
            (
                tantivy::query::Occur::Should,
                Box::new(term_query(value)) as Box<dyn tantivy::query::Query>,
            )
        })
        .collect();
    tracing::debug!("Shape-type filter (any of): {:?}", shape_types);
    must_clauses.push((
        tantivy::query::Occur::Must,
        Box::new(tantivy::query::BooleanQuery::new(clauses)) as Box<dyn tantivy::query::Query>,
    ));
}

fn extract_results(
    searcher: &tantivy::Searcher,
    top_docs: Vec<(f32, tantivy::DocAddress)>,
    fields: &SchemaFields,
) -> Result<Vec<FullTextSearchResult>> {
    let mut results = Vec::with_capacity(top_docs.len());

    for (score, doc_address) in top_docs {
        let retrieved_doc: tantivy::TantivyDocument = searcher
            .doc(doc_address)
            .map_err(|e| Error::storage(format!("Failed to retrieve document: {}", e)))?;

        let node_id = retrieved_doc
            .get_first(fields.node_id)
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::storage("Document missing node_id"))?
            .to_string();

        let workspace_id = retrieved_doc
            .get_first(fields.workspace_id)
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::storage("Document missing workspace_id"))?
            .to_string();

        let name = retrieved_doc
            .get_first(fields.name)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let node_type = retrieved_doc
            .get_first(fields.node_type)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let path = retrieved_doc
            .get_first(fields.path)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let revision = {
            let timestamp_ms = retrieved_doc
                .get_first(fields.revision_timestamp)
                .and_then(|v| v.as_u64());
            let counter = retrieved_doc
                .get_first(fields.revision_counter)
                .and_then(|v| v.as_u64());

            match (timestamp_ms, counter) {
                (Some(ts), Some(c)) => Some(HLC::new(ts, c)),
                _ => None,
            }
        };

        results.push(FullTextSearchResult {
            node_id,
            workspace_id,
            score,
            name,
            node_type,
            path,
            revision,
        });
    }

    Ok(results)
}
