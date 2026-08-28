//! End-to-end locale round-trips: `UPDATE … FOR LOCALE` writes, `WHERE locale =`
//! reads, over the real storage stack.
//!
//! Everything about translations was covered by unit tests on either side of the
//! boundary — the pointer parser, and `merge_into_map` against a hand-built node —
//! and nothing joined the two. That left three behaviours unpinned that consumers
//! very much depend on:
//!
//!   * a uuid-indexed SET path (`content[uuid='…'].headline`) surviving the write
//!     and being resolved on the way back out, at depth;
//!   * two writes to the same (node, locale) ACCUMULATING rather than the second
//!     silently replacing the first;
//!   * a fallback chain resolving most-specific-wins, not last-in-chain-wins.

use futures::StreamExt;
use raisin_context::RepositoryConfig;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const WS: &str = "pages";
const BLOCK: &str = "block-1";

async fn setup() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    let storage = Arc::new(storage);
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("workspace");
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "test:Page" })).expect("nt"),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .expect("nodetype");
    (storage, temp_dir)
}

/// The engine as the HTTP/WS transports build it: with the repository config, which
/// is what makes a locale predicate mean anything at all.
fn engine(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    config: RepositoryConfig,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_repository_config(config)
        .with_auth(AuthContext::system())
}

fn config(supported: &[&str]) -> RepositoryConfig {
    RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: supported.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

async fn run(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
    }
}

/// Read `/page` in `locale` and return its properties as JSON.
async fn read_props(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    locale: &str,
) -> serde_json::Value {
    let sql =
        format!("SELECT properties FROM {WS} WHERE path = '/page' AND locale = '{locale}' LIMIT 1");
    let mut stream = engine
        .execute(&sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let row = stream
        .next()
        .await
        .unwrap_or_else(|| panic!("no row for locale '{locale}'"))
        .expect("row");
    let props = row.columns.get("properties").expect("properties column");
    serde_json::to_value(props).expect("properties as json")
}

/// `properties.content[0].headline` — the nested leaf the uuid path addresses.
fn headline(props: &serde_json::Value) -> String {
    props
        .pointer("/content/0/headline")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("no /content/0/headline in {props}"))
        .to_string()
}

fn title(props: &serde_json::Value) -> String {
    props
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("no title in {props}"))
        .to_string()
}

async fn insert_page(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>) {
    let props = serde_json::json!({
        "title": "Home",
        "content": [{ "uuid": BLOCK, "headline": "Hello", "tagline": "Untouched" }],
    })
    .to_string();
    run(
        engine,
        &format!(
            "INSERT INTO {WS} (id, path, node_type, properties) VALUES \
             ('page-1','/page','test:Page','{props}'::JSONB)"
        ),
    )
    .await;
}

/// A uuid-indexed SET path must survive the write and resolve on read — the claim
/// that a SQL locale read is somehow shallower than the REST `?lang=` one. Both go
/// through the same `TranslationResolver`; this pins it.
#[tokio::test]
async fn for_locale_write_round_trips_through_a_locale_select() {
    let (storage, _td) = setup().await;
    let e = engine(&storage, config(&["en", "de"]));
    insert_page(&e).await;

    run(
        &e,
        &format!(
            "UPDATE {WS} FOR LOCALE 'de' SET title = 'Startseite', \
             content[uuid='{BLOCK}'].headline = 'Hallo' WHERE path = '/page'"
        ),
    )
    .await;

    let de = read_props(&e, "de").await;
    assert_eq!(title(&de), "Startseite", "top-level leaf must translate");
    assert_eq!(
        headline(&de),
        "Hallo",
        "uuid-indexed leaf inside an array must translate"
    );
    assert_eq!(
        de.pointer("/content/0/tagline").and_then(|v| v.as_str()),
        Some("Untouched"),
        "an untranslated sibling must fall through to the base language"
    );

    // The base language is untouched by translating.
    let en = read_props(&e, "en").await;
    assert_eq!(title(&en), "Home");
    assert_eq!(headline(&en), "Hello");
}

/// A store is a whole-overlay put, so a second statement used to REPLACE the first
/// one's fields — a page translated block by block kept only the last block, and a
/// machine pass after a human one erased the human's work. Silently, both times.
#[tokio::test]
async fn a_second_for_locale_write_does_not_erase_the_first() {
    let (storage, _td) = setup().await;
    let e = engine(&storage, config(&["en", "de"]));
    insert_page(&e).await;

    run(
        &e,
        &format!("UPDATE {WS} FOR LOCALE 'de' SET title = 'Startseite' WHERE path = '/page'"),
    )
    .await;
    run(
        &e,
        &format!(
            "UPDATE {WS} FOR LOCALE 'de' SET content[uuid='{BLOCK}'].headline = 'Hallo' \
             WHERE path = '/page'"
        ),
    )
    .await;

    let de = read_props(&e, "de").await;
    assert_eq!(
        title(&de),
        "Startseite",
        "the first write's field must survive the second write"
    );
    assert_eq!(headline(&de), "Hallo");
}

/// Re-translating one field replaces that field and leaves the rest alone.
#[tokio::test]
async fn re_translating_one_field_keeps_the_others() {
    let (storage, _td) = setup().await;
    let e = engine(&storage, config(&["en", "de"]));
    insert_page(&e).await;

    run(
        &e,
        &format!(
            "UPDATE {WS} FOR LOCALE 'de' SET title = 'Startseite', \
             content[uuid='{BLOCK}'].headline = 'Hallo' WHERE path = '/page'"
        ),
    )
    .await;
    run(
        &e,
        &format!("UPDATE {WS} FOR LOCALE 'de' SET title = 'Startseite (neu)' WHERE path = '/page'"),
    )
    .await;

    let de = read_props(&e, "de").await;
    assert_eq!(title(&de), "Startseite (neu)");
    assert_eq!(headline(&de), "Hallo", "the untouched field must remain");
}

/// An engine built WITHOUT the repository config silently ignores the locale
/// predicate and answers with base-language content.
///
/// This is the whole of the functions-runtime bug: `raisin-functions` built its
/// `QueryEngine` without `.with_repository_config(...)`, so `WHERE locale = 'de'`
/// inside a server function read English and reported nothing wrong. The test
/// pins the mechanism from both sides so the omission cannot come back as a
/// mystery — the write is unaffected either way, only the read.
#[tokio::test]
async fn without_the_repository_config_a_locale_predicate_is_a_silent_no_op() {
    let (storage, _td) = setup().await;
    let configured = engine(&storage, config(&["en", "de"]));
    insert_page(&configured).await;
    run(
        &configured,
        &format!("UPDATE {WS} FOR LOCALE 'de' SET title = 'Startseite' WHERE path = '/page'"),
    )
    .await;

    // Same storage, same query — an engine wired the way the functions runtime
    // used to wire it.
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    let unconfigured = QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(AuthContext::system());

    assert_eq!(
        title(&read_props(&configured, "de").await),
        "Startseite",
        "with the repository config the overlay resolves"
    );
    assert_eq!(
        title(&read_props(&unconfigured, "de").await),
        "Home",
        "without it the SAME query silently returns the base language"
    );
}

/// An `ElementField` — an element held DIRECTLY on a property, like a page's
/// `hero` — is addressed by field name (`/hero/headline`). Only elements inside an
/// ARRAY were navigable, so translating an embedded element made the whole page
/// fail to load in that language: `Cannot navigate through non-object property at
/// path segment 'hero'`, a 400 on the read, with the base language unreachable too.
#[tokio::test]
async fn a_translated_element_field_resolves_instead_of_failing_the_read() {
    let (storage, _td) = setup().await;
    let e = engine(&storage, config(&["en", "de"]));

    // `hero` is a single embedded element; `content` is the array case beside it.
    let props = serde_json::json!({
        "title": "Home",
        "hero": { "uuid": "masthead", "headline": "Hello", "lead": "Untouched" },
        "content": [{ "uuid": BLOCK, "headline": "Hello", "tagline": "Untouched" }],
    })
    .to_string();
    run(
        &e,
        &format!(
            "INSERT INTO {WS} (id, path, node_type, properties) VALUES \
             ('page-1','/page','test:Page','{props}'::JSONB)"
        ),
    )
    .await;

    run(
        &e,
        &format!("UPDATE {WS} FOR LOCALE 'de' SET hero.headline = 'Hallo' WHERE path = '/page'"),
    )
    .await;

    let de = read_props(&e, "de").await;
    assert_eq!(
        de.pointer("/hero/headline").and_then(|v| v.as_str()),
        Some("Hallo"),
        "a field inside an embedded element must translate"
    );
    assert_eq!(
        de.pointer("/hero/lead").and_then(|v| v.as_str()),
        Some("Untouched"),
        "its untranslated siblings must survive"
    );
}

/// A pointer whose target has changed shape is STALE, not fatal. It used to abort
/// the entire localized read, which meant one bad pointer could take a page down in
/// a language with no way to fix it from the editor.
#[tokio::test]
async fn a_pointer_that_cannot_be_navigated_is_skipped_not_fatal() {
    let (storage, _td) = setup().await;
    let e = engine(&storage, config(&["en", "de"]));
    insert_page(&e).await;

    // `/title/nested` navigates THROUGH a string — impossible by construction.
    run(
        &e,
        &format!("UPDATE {WS} FOR LOCALE 'de' SET title.nested = 'Nirgendwo' WHERE path = '/page'"),
    )
    .await;
    run(
        &e,
        &format!(
            "UPDATE {WS} FOR LOCALE 'de' SET content[uuid='{BLOCK}'].headline = 'Hallo' \
             WHERE path = '/page'"
        ),
    )
    .await;

    let de = read_props(&e, "de").await;
    assert_eq!(
        headline(&de),
        "Hallo",
        "the good pointer still applies alongside the impossible one"
    );
}

/// Merging makes a translation sticky, so there has to be a way to take one back:
/// setting a pointer to NULL clears it and the field falls back to the base
/// language again. Without it, "this shouldn't be translated after all" could only
/// be expressed by dropping the whole locale.
#[tokio::test]
async fn setting_a_translation_to_null_clears_just_that_field() {
    let (storage, _td) = setup().await;
    let e = engine(&storage, config(&["en", "de"]));
    insert_page(&e).await;

    run(
        &e,
        &format!(
            "UPDATE {WS} FOR LOCALE 'de' SET title = 'Startseite', \
             content[uuid='{BLOCK}'].headline = 'Hallo' WHERE path = '/page'"
        ),
    )
    .await;
    run(
        &e,
        &format!("UPDATE {WS} FOR LOCALE 'de' SET title = NULL WHERE path = '/page'"),
    )
    .await;

    let de = read_props(&e, "de").await;
    assert_eq!(
        title(&de),
        "Home",
        "a cleared field must fall back to the base language"
    );
    assert_eq!(
        headline(&de),
        "Hallo",
        "clearing one field must not touch the others"
    );
}

/// A fallback chain runs most-specific first: `de-CH` overlays `de` overlays the
/// base. Applying the chain in order let the LEAST specific locale win, so a
/// regional variant was overwritten by the language it falls back to.
#[tokio::test]
async fn a_regional_locale_wins_over_the_language_it_falls_back_to() {
    let (storage, _td) = setup().await;
    let e = engine(&storage, config(&["en", "de", "de-CH"]));
    insert_page(&e).await;

    run(
        &e,
        &format!(
            "UPDATE {WS} FOR LOCALE 'de' SET title = 'Startseite', \
             content[uuid='{BLOCK}'].headline = 'Hallo' WHERE path = '/page'"
        ),
    )
    .await;
    run(
        &e,
        &format!("UPDATE {WS} FOR LOCALE 'de-CH' SET title = 'Startsyte' WHERE path = '/page'"),
    )
    .await;

    let ch = read_props(&e, "de-CH").await;
    assert_eq!(
        title(&ch),
        "Startsyte",
        "the regional overlay must win over its fallback language"
    );
    assert_eq!(
        headline(&ch),
        "Hallo",
        "a field only the fallback language translates must still show through"
    );
}

/// A REFERENCE inlined by `resolve()` must be read in the query's language too.
///
/// This is the one the locale plumbing missed for a long time, and it is invisible
/// from either side on its own: the scan executors translate the row before
/// projection, so the SELECTED node came back correctly translated, while
/// `resolve()` fetched every referenced node straight out of storage with no
/// locale at all. A page read in `de` therefore rendered a German headline over an
/// untranslated referenced author — half a document, with the translation sitting
/// in the database the whole time and nothing reporting a problem.
#[tokio::test]
async fn resolve_reads_referenced_nodes_in_the_query_locale() {
    let (storage, _td) = setup().await;
    let e = engine(&storage, config(&["en", "de"]));

    // An author, and a page that REFERENCES it rather than repeating it.
    let author = serde_json::json!({ "title": "Markus", "role": "Founder & Director" }).to_string();
    run(
        &e,
        &format!(
            "INSERT INTO {WS} (id, path, node_type, properties) VALUES \
             ('author-1','/author','test:Page','{author}'::JSONB)"
        ),
    )
    .await;

    let page = serde_json::json!({
        "title": "Team",
        "author": { "raisin:ref": "author-1", "raisin:workspace": WS },
    })
    .to_string();
    run(
        &e,
        &format!(
            "INSERT INTO {WS} (id, path, node_type, properties) VALUES \
             ('page-2','/team','test:Page','{page}'::JSONB)"
        ),
    )
    .await;

    // Both documents are translated — the page itself, and the node it points at.
    run(
        &e,
        &format!("UPDATE {WS} FOR LOCALE 'de' SET title = 'Das Team' WHERE path = '/team'"),
    )
    .await;
    run(
        &e,
        &format!(
            "UPDATE {WS} FOR LOCALE 'de' SET role = 'Gründer & Direktor' WHERE path = '/author'"
        ),
    )
    .await;

    let sql = format!(
        "SELECT resolve(properties, 2) AS properties FROM {WS} \
         WHERE path = '/team' AND locale = 'de' LIMIT 1"
    );
    let mut stream = e.execute(&sql).await.expect("resolve select");
    let row = stream.next().await.expect("a row").expect("row ok");
    let props: serde_json::Value =
        serde_json::to_value(row.columns.get("properties").expect("properties")).expect("json");

    assert_eq!(
        props.get("title").and_then(|v| v.as_str()),
        Some("Das Team"),
        "the selected row translates, as it always did"
    );
    assert_eq!(
        props.pointer("/author/role").and_then(|v| v.as_str()),
        Some("Gründer & Direktor"),
        "the REFERENCED node must be inlined in the query's locale, not the base language"
    );
    assert_eq!(
        props.pointer("/author/title").and_then(|v| v.as_str()),
        Some("Markus"),
        "an untranslated field of a referenced node still falls back to the base language"
    );

    // And the base language is unaffected: no locale predicate, no translation.
    let sql_en = format!(
        "SELECT resolve(properties, 2) AS properties FROM {WS} WHERE path = '/team' LIMIT 1"
    );
    let mut stream = e.execute(&sql_en).await.expect("resolve select en");
    let row = stream.next().await.expect("a row").expect("row ok");
    let props: serde_json::Value =
        serde_json::to_value(row.columns.get("properties").expect("properties")).expect("json");
    assert_eq!(
        props.pointer("/author/role").and_then(|v| v.as_str()),
        Some("Founder & Director"),
        "a read with no locale must still be the base language"
    );
}
