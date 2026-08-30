// SPDX-License-Identifier: BSL-1.1

//! Language analysis of the full-text index: stemming and CJK segmentation.
//!
//! Two defects this pins down:
//!
//! * **Stemming was dead code.** `language.rs` registered 16 `{lang}_stemmer`
//!   analyzers, but no schema field ever named one — `name`/`content` were
//!   pinned to tantivy's `"default"`. Every language, English included, was
//!   indexed unstemmed.
//! * **CJK text was DELETED at index time.** `"default"` is
//!   `SimpleTokenizer + RemoveLongFilter(40 bytes) + LowerCaser`, and
//!   `SimpleTokenizer` splits on non-alphanumerics only. An unspaced Japanese
//!   sentence is therefore ONE token, ~14+ characters, well past 40 bytes — and
//!   `RemoveLongFilter` drops it. Not ranked poorly: absent.
//!
//! The German assertion is deliberately built so the *prefix-fuzzy distance-1*
//! fallback in `search.rs` cannot explain it. The query is the LONGER form
//! (`Datenbanken`) and the indexed term the shorter (`Datenbank`): edit distance
//! 2, and the query is not a prefix of the term. Only a stemmer collapses them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::fulltext::{
    FullTextIndexJob, FullTextSearchQuery, IndexingEngine, JobKind, NodeIndexPlan,
};

use raisin_indexer::TantivyIndexingEngine;

const TENANT: &str = "t";
const REPO: &str = "r";
const BRANCH: &str = "main";
const WS: &str = "content";

static SEQ: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "raisin-indexer-language-{}-{}-{}",
            std::process::id(),
            tag,
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn node(id: &str, name: &str, content: &str) -> Node {
    let mut properties = HashMap::new();
    properties.insert(
        "content".to_string(),
        PropertyValue::String(content.to_string()),
    );
    Node {
        id: id.to_string(),
        name: name.to_string(),
        path: format!("/docs/{id}"),
        node_type: "test:Doc".to_string(),
        workspace: Some(WS.to_string()),
        properties,
        ..Default::default()
    }
}

fn job(node_id: &str, language: &str) -> FullTextIndexJob {
    FullTextIndexJob {
        job_id: format!("job-{node_id}"),
        kind: JobKind::AddNode,
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        workspace_id: WS.to_string(),
        branch: BRANCH.to_string(),
        revision: HLC::new(1000, 0),
        node_id: Some(node_id.to_string()),
        source_branch: None,
        default_language: language.to_string(),
        supported_languages: vec![language.to_string()],
        properties_to_index: None,
    }
}

fn plan() -> NodeIndexPlan {
    NodeIndexPlan {
        node_type: "test:Doc".to_string(),
        legacy_index_all_strings: true,
        ..Default::default()
    }
}

/// Search through a FRESH engine so the answer reflects committed disk state,
/// not a cached `IndexReader` (`ReloadPolicy::OnCommitWithDelay`).
fn hits(base: &PathBuf, language: &str, query: &str) -> Vec<String> {
    let engine = TantivyIndexingEngine::new(base.clone(), 64 * 1024 * 1024).expect("engine");
    let mut ids: Vec<String> = engine
        .search(&FullTextSearchQuery {
            tenant_id: TENANT.to_string(),
            repo_id: REPO.to_string(),
            workspace_ids: Some(vec![WS.to_string()]),
            branch: BRANCH.to_string(),
            language: language.to_string(),
            query: query.to_string(),
            limit: 100,
            revision: None,
            shape_types: None,
        })
        .expect("search")
        .into_iter()
        .map(|hit| hit.node_id)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

fn index_one(base: &PathBuf, id: &str, language: &str, name: &str, content: &str) {
    let engine = TantivyIndexingEngine::new(base.clone(), 64 * 1024 * 1024).expect("engine");
    engine
        .do_index_node_with_plan(&job(id, language), &node(id, name, content), &plan())
        .expect("index");
}

/// German: an inflected query must reach the uninflected indexed term.
///
/// `Datenbanken` -> `Datenbank` is edit distance 2 and not a prefix relation, so
/// the distance-1 prefix-fuzzy fallback cannot produce this hit. A stemmer can.
#[test]
fn german_inflection_matches_via_stemming_not_fuzzy() {
    let scratch = Scratch::new("de-stem");
    index_one(
        &scratch.0,
        "doc-de",
        "de",
        "Handbuch",
        "Die Datenbank speichert Informationen dauerhaft.",
    );

    assert_eq!(
        hits(&scratch.0, "de", "Datenbank"),
        vec!["doc-de".to_string()],
        "control: the exact German term must match"
    );
    assert_eq!(
        hits(&scratch.0, "de", "Datenbanken"),
        vec!["doc-de".to_string()],
        "the plural 'Datenbanken' must reach the indexed 'Datenbank' via the German stemmer"
    );
}

/// The same, on the `name` field rather than `content`.
#[test]
fn german_stemming_applies_to_the_name_field() {
    let scratch = Scratch::new("de-name");
    index_one(&scratch.0, "doc-de", "de", "Datenbank", "unrelated prose");

    assert_eq!(
        hits(&scratch.0, "de", "Datenbanken"),
        vec!["doc-de".to_string()],
        "stemming must apply to `name`, not only `content`"
    );
}

/// English stemming, to show the fix is not German-specific.
#[test]
fn english_inflection_matches_via_stemming() {
    let scratch = Scratch::new("en-stem");
    index_one(
        &scratch.0,
        "doc-en",
        "en",
        "Handbook",
        "The database stores information permanently.",
    );

    assert_eq!(
        hits(&scratch.0, "en", "storing"),
        vec!["doc-en".to_string()],
        "'storing' must reach the indexed 'stores' via the English stemmer"
    );
}

/// Japanese: an unspaced sentence must survive indexing and be findable.
///
/// Pre-fix this returned nothing at all — the whole sentence was a single
/// >40-byte token and `RemoveLongFilter` deleted it.
#[test]
fn japanese_tokens_survive_indexing() {
    let scratch = Scratch::new("ja");
    index_one(
        &scratch.0,
        "doc-ja",
        "ja",
        "説明書",
        "データベースは情報を保存するシステムです",
    );

    assert_eq!(
        hits(&scratch.0, "ja", "情報"),
        vec!["doc-ja".to_string()],
        "a two-character Japanese term inside an unspaced sentence must be findable"
    );
    assert_eq!(
        hits(&scratch.0, "ja", "保存"),
        vec!["doc-ja".to_string()],
        "a second Japanese term from the same sentence must be findable"
    );
}

/// Chinese: same structural problem, same fix.
#[test]
fn chinese_tokens_survive_indexing() {
    let scratch = Scratch::new("zh");
    index_one(
        &scratch.0,
        "doc-zh",
        "zh",
        "手册",
        "数据库永久地存储信息和其他内容",
    );

    assert_eq!(
        hits(&scratch.0, "zh", "存储"),
        vec!["doc-zh".to_string()],
        "a Chinese term inside an unspaced sentence must be findable"
    );
}

/// Mixed CJK + Latin in one field: neither side may cannibalise the other.
#[test]
fn mixed_script_content_indexes_both_scripts() {
    let scratch = Scratch::new("mixed");
    index_one(
        &scratch.0,
        "doc-mix",
        "ja",
        "RaisinDB 説明書",
        "RaisinDB は情報を保存します",
    );

    assert_eq!(
        hits(&scratch.0, "ja", "RaisinDB"),
        vec!["doc-mix".to_string()],
        "the Latin token in mixed-script text must still be findable"
    );
    assert_eq!(
        hits(&scratch.0, "ja", "情報"),
        vec!["doc-mix".to_string()],
        "the CJK token in mixed-script text must still be findable"
    );
}

// ---------------------------------------------------------------------------
// Migration: a pre-v3 index must stay internally consistent, not half-migrated.
// ---------------------------------------------------------------------------

/// Reproduces the v2 on-disk schema exactly: `name` / `content` on tantivy's
/// `"default"` analyzer, `shape_types` on `raw`, and no per-language pairs.
fn build_legacy_v2_index(dir: &PathBuf) {
    use tantivy::schema::*;

    let mut b = Schema::builder();
    b.add_text_field("doc_id", STRING | STORED);
    b.add_text_field("node_id", STRING | STORED);
    b.add_text_field("workspace_id", STRING | STORED);
    b.add_text_field("language", STRING | STORED);
    b.add_text_field("path", STRING | STORED);
    b.add_text_field("node_type", STRING | STORED);
    b.add_u64_field("revision_timestamp", INDEXED | STORED);
    b.add_u64_field("revision_counter", INDEXED | STORED);
    b.add_date_field("created_at", INDEXED | STORED);
    b.add_date_field("updated_at", INDEXED | STORED);

    let text_options = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("default")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();
    b.add_text_field("name", text_options.clone());
    b.add_text_field("content", text_options);
    b.add_text_field(
        "shape_types",
        TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("raw")
                    .set_index_option(IndexRecordOption::Basic),
            )
            .set_stored(),
    );

    let index_dir = dir.join(TENANT).join(REPO).join(BRANCH);
    std::fs::create_dir_all(&index_dir).expect("create legacy index dir");
    tantivy::Index::create_in_dir(&index_dir, b.build()).expect("create legacy index");
    std::fs::write(index_dir.join("raisin_schema_version"), "2").expect("write v2 sidecar");
}

/// A v2 index keeps working — and keeps its OLD analysis — until it is rebuilt.
///
/// This is the whole migration contract in one test. The analyzer name lives in
/// the on-disk schema, so tantivy applies the v2 analyzer to both the write and
/// the query side of a v2 index; the stemmed field pairs simply do not exist
/// there, and `schema_fields` resolves them to an empty map, so neither
/// `create_document` nor `build_text_query` references them. The result is an
/// index that is consistently old, never a mix of old segments and new terms.
#[test]
fn a_pre_v3_index_stays_consistent_and_unstemmed_until_rebuilt() {
    let scratch = Scratch::new("legacy");
    build_legacy_v2_index(&scratch.0);

    let engine = TantivyIndexingEngine::new(scratch.0.clone(), 64 * 1024 * 1024).expect("engine");
    assert!(
        engine.is_index_stale(TENANT, REPO, BRANCH),
        "a v2 index must report stale so the rebuild lever fires"
    );

    // Writing to it must not panic on the missing per-language fields.
    engine
        .do_index_node_with_plan(
            &job("doc-de", "de"),
            &node(
                "doc-de",
                "Handbuch",
                "Die Datenbank speichert Informationen.",
            ),
            &plan(),
        )
        .expect("indexing a node into a legacy index must still succeed");

    // Old behaviour, intact: exact terms match, inflections do not.
    assert_eq!(
        hits(&scratch.0, "de", "Datenbank"),
        vec!["doc-de".to_string()],
        "a legacy index must keep matching what it always matched"
    );
    assert!(
        hits(&scratch.0, "de", "Datenbanken").is_empty(),
        "a legacy index must NOT half-acquire stemming; it needs a rebuild"
    );
}

/// A fresh index records the current schema version, so it is not immediately
/// reported stale (which would make the dev tenant rebuild on every boot).
#[test]
fn a_fresh_index_is_not_reported_stale() {
    let scratch = Scratch::new("fresh-version");
    index_one(
        &scratch.0,
        "doc-en",
        "en",
        "Handbook",
        "the database stores",
    );

    let engine = TantivyIndexingEngine::new(scratch.0.clone(), 64 * 1024 * 1024).expect("engine");
    assert!(!engine.is_index_stale(TENANT, REPO, BRANCH));
}
