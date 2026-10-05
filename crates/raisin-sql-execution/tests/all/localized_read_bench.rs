//! What a `WHERE locale = …` tree read costs per row, against the same read in
//! the default language — the shape a site's navigation issues several times
//! per page render.
//!
//! Run (release, it is a measurement):
//!   cargo test --release -p raisin-sql-execution --test all -- \
//!     --ignored --nocapture localized_read_bench
//!
//! The tree is `/site/s{i}/c{j}/p{k}` (5 × 6 × 6 pages, ~215 nodes). Most
//! nodes carry an `fr` overlay with a translated `title` and `__node_name`, a
//! few carry none (fallback to the base language), and a few are Hidden in
//! `fr` — a hidden section hides its whole subtree from `__localized_path`.
//!
//! The non-ignored test in this module is the equivalence check: the rows the
//! statement-scoped read produces are compared, byte for byte, with what the
//! straightforward per-node storage calls answer for the same tree.

use futures::StreamExt;
use raisin_context::RepositoryConfig;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository, RepoScope,
    RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;

type Store = raisin_rocksdb::RocksDBStorage;
const T: &str = "lrb_tenant";
const R: &str = "lrb_repo";
const B: &str = "main";
const WS: &str = "stories";

fn config() -> RepositoryConfig {
    RepositoryConfig {
        default_language: "de".to_string(),
        supported_languages: vec!["de".into(), "fr".into(), "fr-CH".into()],
        default_branch: B.to_string(),
        ..RepositoryConfig::default()
    }
}

fn engine(storage: &Arc<Store>) -> QueryEngine<Store> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(storage.clone(), T, R, B)
        .with_catalog(Arc::new(catalog))
        .with_repository_config(config())
        .with_auth(AuthContext::system())
}

type Rows = Vec<BTreeMap<String, String>>;

async fn rows(engine: &QueryEngine<Store>, sql: &str) -> Rows {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("query failed [{sql}]: {e}"));
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        out.push(
            row.columns
                .into_iter()
                .map(|(k, v)| {
                    let k = k.rsplit('.').next().unwrap_or(&k).to_string();
                    (k, format!("{v:?}"))
                })
                .collect(),
        );
    }
    out
}

/// Body size in 27-byte repetitions (`BENCH_BODY`, default 20 ≈ 0.5 KB).
/// Production pages carry tens of KB of content per node record.
fn body_reps() -> usize {
    std::env::var("BENCH_BODY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
}

/// What a node looks like in the seeded tree.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fr {
    /// Translated title and name.
    Named,
    /// Translated title only: the URL segment falls back to the canonical one.
    TitleOnly,
    /// No overlay at all.
    None,
    /// Hidden in `fr`.
    Hidden,
}

fn fr_of(depth: usize, i: usize, j: usize, k: usize) -> Fr {
    match depth {
        1 if i == 3 => Fr::Hidden,
        1 if i == 4 => Fr::None,
        2 if j == 2 && i == 1 => Fr::Hidden,
        2 if j == 4 => Fr::TitleOnly,
        3 if k == 5 => Fr::None,
        3 if k == 3 && j == 1 => Fr::Hidden,
        3 if k == 2 => Fr::TitleOnly,
        _ => Fr::Named,
    }
}

/// Seed the tree, then edit every node `edits` times AFTER it was translated
/// (an edited page carries node revisions above its overlay, which is what
/// every overlay read's node-delete check walks); returns every id by path.
async fn seed(edits: usize) -> (Arc<Store>, TempDir, HashMap<String, String>) {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(Store::new(dir.path()).unwrap());
    storage
        .registry()
        .register_tenant(T, HashMap::new())
        .await
        .unwrap();
    storage
        .repository_management()
        .create_repository(T, R, config())
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(T, R, B, "test", None, None, false, false)
        .await
        .unwrap();
    storage
        .workspaces()
        .put(
            RepoScope::new(T, R),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .unwrap();

    let mut entries: Vec<(String, Fr)> = vec![("/site".to_string(), Fr::Named)];
    for i in 0..5 {
        entries.push((format!("/site/s{i}"), fr_of(1, i, 0, 0)));
        for j in 0..6 {
            entries.push((format!("/site/s{i}/c{j}"), fr_of(2, i, j, 0)));
            for k in 0..6 {
                entries.push((format!("/site/s{i}/c{j}/p{k}"), fr_of(3, i, j, k)));
            }
        }
    }

    let translations = raisin_core::TranslationService::new(storage.clone());
    let fr = LocaleCode::parse("fr").unwrap();
    let mut ids = HashMap::new();
    for (path, kind) in entries {
        let name = path.rsplit('/').next().unwrap().to_string();
        let parent = path
            .rsplitn(2, '/')
            .nth(1)
            .and_then(|p| p.rsplit('/').next())
            .filter(|p| !p.is_empty())
            .map(str::to_string);
        let node = Node {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.clone(),
            path: path.clone(),
            parent,
            node_type: "raisin:Folder".to_string(),
            properties: HashMap::from([
                (
                    "title".to_string(),
                    PropertyValue::String(format!("Titel {name}")),
                ),
                (
                    "body".to_string(),
                    PropertyValue::String("Lorem ipsum dolor sit amet ".repeat(body_reps())),
                ),
            ]),
            created_at: Some(chrono::Utc::now()),
            ..Node::default()
        };
        let id = node.id.clone();
        storage
            .nodes()
            .create(
                StorageScope::new(T, R, B, WS),
                node,
                CreateNodeOptions {
                    validate_schema: false,
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    operation_meta: None,
                },
            )
            .await
            .unwrap();
        let mut data = HashMap::new();
        if matches!(kind, Fr::Named | Fr::TitleOnly) {
            data.insert(
                JsonPointer::new("/title"),
                PropertyValue::String(format!("Titre {name}")),
            );
        }
        if kind == Fr::Named {
            data.insert(
                JsonPointer::new("/__node_name"),
                PropertyValue::String(format!("{name}-fr")),
            );
        }
        if !data.is_empty() {
            translations
                .update_translation(T, R, B, WS, &id, &fr, data, "t", None)
                .await
                .unwrap();
        }
        if kind == Fr::Hidden {
            translations
                .hide_node(T, R, B, WS, &id, &fr, "t", None)
                .await
                .unwrap();
        }
        ids.insert(path, id);
    }
    let scope = StorageScope::new(T, R, B, WS);
    for edit in 0..edits {
        for id in ids.values() {
            let mut node = storage.nodes().get(scope, id, None).await.unwrap().unwrap();
            node.properties
                .insert("rev".to_string(), PropertyValue::String(edit.to_string()));
            storage
                .nodes()
                .update(
                    scope,
                    node,
                    raisin_storage::UpdateNodeOptions {
                        validate_schema: false,
                        allow_type_change: false,
                        operation_meta: None,
                    },
                )
                .await
                .unwrap();
        }
    }
    // Settle the LSM shape the seeding left (memtables, L0 files, running
    // compactions) so every case reads the same tree, as a long-lived
    // database's would be.
    let db = storage.db();
    for cf in [
        "nodes",
        "translation_data",
        "block_translations",
        "path_index",
        "node_path",
    ] {
        if let Some(handle) = db.cf_handle(cf) {
            db.compact_range_cf(&handle, None::<&[u8]>, None::<&[u8]>);
        }
    }
    (storage, dir, ids)
}

const BASE: &str = "SELECT path, properties->>'title' AS t FROM stories \
                    WHERE DESCENDANT_OF('/site/s1')";

/// Wall time per run, in µs: the BEST of ten rounds' means (each round
/// `iters / 10` runs) — a shared machine's noise only ever adds time.
async fn time(engine: &QueryEngine<Store>, sql: &str, iters: usize) -> (f64, usize) {
    let n = rows(engine, sql).await.len();
    let per_round = (iters / 10).max(1);
    let mut best = f64::MAX;
    for _ in 0..10 {
        let start = Instant::now();
        for _ in 0..per_round {
            rows(engine, sql).await;
        }
        best = best.min(start.elapsed().as_secs_f64() * 1e6 / per_round as f64);
    }
    (best, n)
}

#[tokio::test]
#[ignore = "localized tree-read benchmark; run with --release --ignored --nocapture"]
async fn localized_read_bench() {
    let iters: usize = std::env::var("BENCH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let edits: usize = std::env::var("BENCH_EDITS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let (storage, _dir, ids) = seed(edits).await;
    let e = engine(&storage);

    let cases = [
        ("default", BASE.to_string()),
        ("fr", format!("{BASE} AND locale = 'fr'")),
        (
            "fr+name",
            BASE.replace("AS t", "AS t, __node_name") + " AND locale = 'fr'",
        ),
        (
            "fr+lpath",
            BASE.replace("AS t", "AS t, __localized_path") + " AND locale = 'fr'",
        ),
        (
            "fr+both",
            BASE.replace("AS t", "AS t, __node_name, __localized_path") + " AND locale = 'fr'",
        ),
        ("fr-CH", format!("{BASE} AND locale = 'fr-CH'")),
    ];
    println!(
        "\n=== localized tree read ({} nodes, {edits} edits after translation, {iters} iters) ===",
        ids.len()
    );
    // One untimed pass over every case first: the block cache and the
    // compactions the seeding left behind then do not land on one case.
    for (_, sql) in &cases {
        rows(&e, sql).await;
    }
    let mut base_us = 0.0;
    for (label, sql) in &cases {
        let (us, n) = time(&e, sql, iters).await;
        if *label == "default" {
            base_us = us;
        }
        println!(
            "  {label:<9} {:>8.1} µs  {n:>3} rows  {:>6.1} µs/row  {:>5.2}x default",
            us,
            us / n.max(1) as f64,
            us / base_us
        );
    }

    // Per-call cost of the storage primitives a localized row used to issue.
    let node_id = &ids["/site/s1/c3/p1"];
    let fr = LocaleCode::parse("fr").unwrap();
    let head = storage.branches().get_head(T, R, B).await.unwrap();
    let reps = 2000;
    let start = Instant::now();
    for _ in 0..reps {
        raisin_storage::TranslationRepository::get_translation(
            storage.translations(),
            T,
            R,
            B,
            WS,
            node_id,
            &fr,
            &head,
        )
        .await
        .unwrap();
    }
    println!(
        "\n  get_translation              {:>6.2} µs/call",
        start.elapsed().as_secs_f64() * 1e6 / reps as f64
    );
    let chain = vec![fr.clone(), LocaleCode::parse("de").unwrap()];
    let start = Instant::now();
    for _ in 0..reps {
        raisin_storage::TranslationRepository::get_block_translations_for_node(
            storage.translations(),
            T,
            R,
            B,
            WS,
            node_id,
            &chain,
            &head,
        )
        .await
        .unwrap();
    }
    println!(
        "  get_block_translations       {:>6.2} µs/call",
        start.elapsed().as_secs_f64() * 1e6 / reps as f64
    );
    let names = storage.localized_names().expect("localized names");
    let scope = StorageScope::new(T, R, B, WS);
    let start = Instant::now();
    for _ in 0..reps {
        names.node_name(scope, node_id, "fr", Some(&head)).unwrap();
    }
    println!(
        "  node_name                    {:>6.2} µs/call",
        start.elapsed().as_secs_f64() * 1e6 / reps as f64
    );
    let start = Instant::now();
    for _ in 0..reps {
        names
            .localized_path(scope, node_id, "fr", Some(&head))
            .unwrap();
    }
    println!(
        "  localized_path (depth 4)     {:>6.2} µs/call",
        start.elapsed().as_secs_f64() * 1e6 / reps as f64
    );
    println!();
}

/// The statement-scoped read answers exactly what the per-node storage calls
/// answer: same rows, same order, same translated values, same names and
/// localized paths — over hidden ancestors, missing overlays, a fallback chain
/// (`fr-CH` → `fr` → `de`) and renamed segments.
#[tokio::test]
async fn localized_tree_read_matches_per_node_resolution() {
    let (storage, _dir, _ids) = seed(2).await;
    let e = engine(&storage);
    let names = storage.localized_names().expect("localized names");
    let head = storage.branches().get_head(T, R, B).await.unwrap();
    let scope = StorageScope::new(T, R, B, WS);
    let resolver = raisin_core::services::translation_resolver::TranslationResolver::new(
        Arc::new(storage.translations().clone()),
        config(),
    );

    for subtree in ["/site", "/site/s1", "/site/s3", "/site/s1/c2"] {
        // The untranslated tree, in the order the scan walks it.
        let canonical = rows(
            &e,
            &format!("SELECT id, path FROM stories WHERE DESCENDANT_OF('{subtree}')"),
        )
        .await;
        for locale in ["fr", "fr-CH", "de"] {
            let sql = format!(
                "SELECT id, path, properties->>'title' AS t, __node_name, __localized_path \
                 FROM stories WHERE DESCENDANT_OF('{subtree}') AND locale = '{locale}'"
            );
            let got = rows(&e, &sql).await;

            // The straightforward answer: one resolver call and one name /
            // path lookup per node, nothing shared between rows.
            let code = LocaleCode::parse(locale).unwrap();
            let mut want: Rows = Vec::new();
            for row in &canonical {
                let id = row["id"]
                    .trim_start_matches("String(\"")
                    .trim_end_matches("\")");
                let node = storage
                    .nodes()
                    .get(scope, id, Some(&head))
                    .await
                    .unwrap()
                    .unwrap();
                let Some(node) = resolver
                    .resolve_node(T, R, B, WS, node, &code, &head)
                    .await
                    .unwrap()
                else {
                    continue;
                };
                let text = |v: Option<String>| match v {
                    Some(s) => format!("{:?}", PropertyValue::String(s)),
                    None => format!("{:?}", PropertyValue::Null),
                };
                let title = node
                    .properties
                    .get("title")
                    .map(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or(None);
                let mut r = BTreeMap::new();
                r.insert("id".to_string(), row["id"].clone());
                r.insert("path".to_string(), row["path"].clone());
                r.insert("t".to_string(), text(title));
                r.insert(
                    "__node_name".to_string(),
                    text(names.node_name(scope, id, locale, Some(&head)).unwrap()),
                );
                r.insert(
                    "__localized_path".to_string(),
                    text(
                        names
                            .localized_path(scope, id, locale, Some(&head))
                            .unwrap(),
                    ),
                );
                want.push(r);
            }
            assert_eq!(got, want, "{subtree} in {locale}");
            if locale == "fr" && subtree == "/site/s1" {
                // The fixture really exercises what it claims to.
                assert!(got.iter().any(|r| r["__localized_path"].contains("Null")));
                assert!(got.iter().any(|r| r["__node_name"].contains("Null")));
                assert!(got.len() < canonical.len(), "something is hidden");
            }
        }
    }
}
