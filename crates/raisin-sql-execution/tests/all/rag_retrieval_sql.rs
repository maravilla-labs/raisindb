//! The SQL the `ai-tools` retrieval component (`/lib/raisin/ai/ask`,
//! `/lib/raisin/ai/search-documents`, source in `tooling/ai-tools-rag`) sends,
//! run against a real RocksDB + Tantivy engine.
//!
//! The component scopes a search by path prefix, node type and content kind
//! with a `WHERE` over `HYBRID_SEARCH(...)`. That only works because the table
//! function evaluates a `WHERE` above it as a residual INSIDE its own fetch
//! loop — rows count toward the limit only once they pass it. A plan builder
//! that dropped the filter (it once did: see `where_over_a_table_function_
//! actually_runs`) would make every one of these scopes a silent no-op, and the
//! component's own tests, which run against a fake host, could not notice.
//!
//! The statements below are the component's, character for character: its
//! `retrieve::tests::CONTRACT` asserts it generates exactly these, and this
//! file asserts the engine answers them correctly. Parameters go through
//! `raisin_sql::substitute_params`, as they do on the function SQL path.

use futures::StreamExt;
use raisin_indexer::{BatchIndexContext, TantivyIndexingEngine};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_sql_execution::{QueryEngine, Row, StaticCatalog};
use raisin_storage::fulltext::NodeIndexPlan;
use raisin_storage::{BranchRepository, CreateNodeOptions, NodeRepository, Storage, StorageScope};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_rag_sql";
const REPO: &str = "r_rag_sql";
const BRANCH: &str = "main";
const TERM: &str = "zebracorn";

/// `tooling/ai-tools-rag/src/retrieve.rs` — `CONTRACT_SELECT`.
const SELECT: &str = "SELECT node_id, path, name, node_type, workspace_id, score, fulltext_rank, vector_rank, chunk_index, chunk_text, chunk_text_source, properties->>'title' AS title, properties->>'file_type' AS file_type, properties->>'url' AS url FROM HYBRID_SEARCH($1, 40, workspaces => $2, granularity => 'chunk', vector_weight => 0)";

/// `tooling/ai-tools-rag/src/retrieve.rs` — `CONTRACT`, the filters.
const DEFAULT_KINDS_UNDER_BAP: &str = " WHERE ((path = $3 OR path LIKE $4)) AND (node_type <> 'raisin:Asset' OR properties->>'file_type' IS NULL OR NOT (properties->>'file_type' LIKE 'image/%'))";
const PAGES_AND_DOCUMENTS_NO_BLUEPRINTS: &str = " WHERE ((path = $3 OR path LIKE $4)) AND (node_type <> 'raisin:Asset' OR (node_type = 'raisin:Asset' AND (properties->>'file_type' IS NULL OR NOT (properties->>'file_type' LIKE 'image/%' OR properties->>'file_type' LIKE 'video/%' OR properties->>'file_type' LIKE 'audio/%')))) AND node_type <> $5";
const PER_WORKSPACE_PREFIXES: &str = " WHERE ((workspace_id = $5 AND (path = $3 OR path LIKE $4)) OR (workspace_id = $8 AND (path = $6 OR path LIKE $7))) AND (node_type <> 'raisin:Asset' OR properties->>'file_type' IS NULL OR NOT (properties->>'file_type' LIKE 'image/%'))";
const IMAGES_ONLY: &str = " WHERE ((path = $3 OR path LIKE $4)) AND ((node_type = 'raisin:Asset' AND properties->>'file_type' LIKE 'image/%'))";

struct Fixture {
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
    reader: Arc<TantivyIndexingEngine>,
    _db: TempDir,
    _index: TempDir,
}

async fn register_workspace(storage: &raisin_rocksdb::RocksDBStorage, name: &str) {
    use raisin_storage::{RepoScope, WorkspaceRepository};
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace {
                name: name.to_string(),
                description: None,
                allowed_node_types: vec![],
                allowed_root_node_types: vec![],
                depends_on: Vec::new(),
                initial_structure: None,
                created_at: raisin_models::StorageTimestamp::now(),
                updated_at: None,
                config: Default::default(),
                compound_indexes: None,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("register workspace {name}: {e}"));
}

/// (workspace, path, node_type, file_type)
const NODES: [(&str, &str, &str, Option<&str>); 8] = [
    ("stories", "/bap/team", "raisin:Document", None),
    ("stories", "/bap/blueprint", "studio:Blueprint", None),
    ("stories", "/bapx/other", "raisin:Document", None),
    ("stories", "/demo/ceo", "raisin:Document", None),
    ("assets", "/bap/ceo.jpg", "raisin:Asset", Some("image/jpeg")),
    (
        "assets",
        "/bap/report.pdf",
        "raisin:Asset",
        Some("application/pdf"),
    ),
    ("assets", "/bap/clip.mp4", "raisin:Asset", Some("video/mp4")),
    (
        "assets",
        "/demo/x.pdf",
        "raisin:Asset",
        Some("application/pdf"),
    ),
];

async fn fixture() -> Fixture {
    let db = TempDir::new().expect("db temp dir");
    let index = TempDir::new().expect("index temp dir");
    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(db.path()).expect("storage"));
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    let index_path = index.path().to_path_buf();
    let open = || {
        Arc::new(
            TantivyIndexingEngine::new(index_path.clone(), 64 * 1024 * 1024)
                .expect("tantivy engine"),
        )
    };
    let writer = open();

    for ws in ["stories", "assets"] {
        register_workspace(&storage, ws).await;
        let mut to_index: Vec<(Node, NodeIndexPlan)> = Vec::new();
        for (i, (_, path, node_type, file_type)) in
            NODES.iter().enumerate().filter(|(_, n)| n.0 == ws)
        {
            let name = path.rsplit('/').next().unwrap().to_string();
            let mut props = HashMap::new();
            props.insert(
                "title".to_string(),
                PropertyValue::String(format!("Title of {name}")),
            );
            props.insert(
                "content".to_string(),
                PropertyValue::String(format!("the {TERM} passage of {path}")),
            );
            if let Some(ft) = file_type {
                props.insert(
                    "file_type".to_string(),
                    PropertyValue::String(ft.to_string()),
                );
                props.insert(
                    "__extracted_text".to_string(),
                    PropertyValue::String(format!("Body of {name}: {TERM}")),
                );
            }
            let node = Node {
                id: format!("n{i}"),
                path: path.to_string(),
                name,
                parent: Some("/".to_string()),
                node_type: node_type.to_string(),
                properties: props,
                ..Default::default()
            };
            let scope = StorageScope::new(TENANT, REPO, BRANCH, ws);
            storage
                .nodes()
                .create(
                    scope,
                    node.clone(),
                    CreateNodeOptions {
                        validate_parent_allows_child: false,
                        validate_workspace_allows_type: false,
                        ..Default::default()
                    },
                )
                .await
                .unwrap_or_else(|e| panic!("create {path}: {e}"));
            let stored = storage
                .nodes()
                .get(StorageScope::new(TENANT, REPO, BRANCH, ws), &node.id, None)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("read back {path}"));
            to_index.push((
                stored,
                NodeIndexPlan {
                    node_type: node_type.to_string(),
                    legacy_index_all_strings: true,
                    ..Default::default()
                },
            ));
        }
        writer
            .do_batch_index(
                &BatchIndexContext {
                    tenant_id: TENANT.to_string(),
                    repo_id: REPO.to_string(),
                    branch: BRANCH.to_string(),
                    workspace_id: ws.to_string(),
                    default_language: "en".to_string(),
                    supported_languages: vec!["en".to_string()],
                },
                to_index,
                vec![],
            )
            .expect("batch index");
    }

    Fixture {
        storage,
        // Opened after the writer committed, so it reads the index from disk.
        reader: open(),
        _db: db,
        _index: index,
    }
}

impl Fixture {
    fn engine(&self) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
        let mut catalog = StaticCatalog::default_nodes_schema();
        catalog.register_workspace("stories".to_string());
        catalog.register_workspace("assets".to_string());
        QueryEngine::new(
            self.storage.clone(),
            TENANT.to_string(),
            REPO.to_string(),
            BRANCH.to_string(),
        )
        .with_catalog(Arc::new(catalog))
        .with_indexing_engine(self.reader.clone())
    }

    async fn rows(&self, sql: &str, params: &[Value]) -> Vec<Row> {
        let sql = raisin_sql::substitute_params(sql, params).expect("substitute params");
        let mut stream = self
            .engine()
            .execute(&sql)
            .await
            .unwrap_or_else(|e| panic!("query failed: {e}\nSQL: {sql}"));
        let mut out = Vec::new();
        while let Some(row) = stream.next().await {
            out.push(row.unwrap_or_else(|e| panic!("row error: {e}\nSQL: {sql}")));
        }
        out
    }

    /// `workspace:path` of every row, sorted.
    async fn hits(&self, filter: &str, params: &[Value]) -> Vec<String> {
        let mut all = vec![json!(TERM), json!("stories, assets")];
        all.extend_from_slice(params);
        let mut out: Vec<String> = self
            .rows(&format!("{SELECT}{filter}"), &all)
            .await
            .iter()
            .map(|r| format!("{}:{}", column(r, "workspace_id"), column(r, "path")))
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

/// Columns are emitted qualified (`hybrid_search.path`), so match on suffix.
fn column(row: &Row, name: &str) -> String {
    row.columns
        .iter()
        .find_map(|(key, value)| {
            let matches = key == name || key.ends_with(&format!(".{name}"));
            match (matches, value) {
                (true, PropertyValue::String(s)) => Some(s.clone()),
                _ => None,
            }
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn the_unfiltered_leg_sees_every_seeded_node() {
    let f = fixture().await;
    assert_eq!(
        f.hits("", &[]).await.len(),
        NODES.len(),
        "fixture: not every node is in the index"
    );
}

#[tokio::test]
async fn a_path_prefix_and_the_default_kinds_scope_the_search() {
    let f = fixture().await;
    assert_eq!(
        f.hits(DEFAULT_KINDS_UNDER_BAP, &[json!("/bap"), json!("/bap/%")])
            .await,
        vec![
            "assets:/bap/clip.mp4",
            "assets:/bap/report.pdf",
            "stories:/bap/blueprint",
            "stories:/bap/team",
        ],
        "no /demo, no /bapx (a prefix is a path segment), and no image"
    );
}

#[tokio::test]
async fn include_kinds_and_exclude_node_types_narrow_further() {
    let f = fixture().await;
    assert_eq!(
        f.hits(
            PAGES_AND_DOCUMENTS_NO_BLUEPRINTS,
            &[json!("/bap"), json!("/bap/%"), json!("studio:Blueprint")]
        )
        .await,
        vec!["assets:/bap/report.pdf", "stories:/bap/team"],
    );
}

#[tokio::test]
async fn a_workspace_qualified_prefix_applies_to_that_workspace_only() {
    let f = fixture().await;
    assert_eq!(
        f.hits(
            PER_WORKSPACE_PREFIXES,
            &[
                json!("/bap"),
                json!("/bap/%"),
                json!("stories"),
                json!("/demo"),
                json!("/demo/%"),
                json!("assets"),
            ]
        )
        .await,
        vec![
            "assets:/demo/x.pdf",
            "stories:/bap/blueprint",
            "stories:/bap/team"
        ],
    );
}

#[tokio::test]
async fn images_come_back_only_when_asked_for() {
    let f = fixture().await;
    assert_eq!(
        f.hits(IMAGES_ONLY, &[json!("/bap"), json!("/bap/%")]).await,
        vec!["assets:/bap/ceo.jpg"],
    );
}

/// The residual is inside the fetch loop, so a scope costs no rows: ask for
/// one, get the one in scope, even though every out-of-scope match ranks too.
#[tokio::test]
async fn the_limit_counts_rows_after_the_scope() {
    let f = fixture().await;
    let sql = format!("{}{IMAGES_ONLY}", SELECT.replace("$1, 40,", "$1, 1,"));
    let rows = f
        .rows(
            &sql,
            &[
                json!(TERM),
                json!("stories, assets"),
                json!("/bap"),
                json!("/bap/%"),
            ],
        )
        .await;
    assert_eq!(rows.len(), 1);
    assert_eq!(column(&rows[0], "path"), "/bap/ceo.jpg");
}

/// The batched reads that give a lexical-only hit its passage text.
#[tokio::test]
async fn the_passage_text_reads_answer() {
    let f = fixture().await;
    let pages = f
        .rows(
            "SELECT path, properties FROM 'stories' WHERE path IN ($1, $2)",
            &[json!("/bap/team"), json!("/demo/ceo")],
        )
        .await;
    assert_eq!(pages.len(), 2);

    let docs = f
        .rows(
            "SELECT path, properties->>'title' AS title, properties->>'description' AS description, \
             properties->>'caption' AS caption, properties->>'alt_text' AS alt_text, \
             SUBSTRING(properties->>'__extracted_text', 1, 60000) AS body FROM 'assets' WHERE path IN ($1)",
            &[json!("/bap/report.pdf")],
        )
        .await;
    assert_eq!(docs.len(), 1);
    assert_eq!(
        column(&docs[0], "body"),
        format!("Body of report.pdf: {TERM}")
    );
    assert_eq!(column(&docs[0], "title"), "Title of report.pdf");
}
