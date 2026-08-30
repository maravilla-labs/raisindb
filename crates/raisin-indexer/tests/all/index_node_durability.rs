// SPDX-License-Identifier: BSL-1.1

//! Durability of the full-text indexing write paths.
//!
//! `do_index_node` is what the live per-node indexing job calls, once per node,
//! from `spawn_blocking`. Each call opens and drops its own 50 MB `IndexWriter`,
//! and tantivy's directory lock is EXCLUSIVE and NON-BLOCKING: while one writer
//! is alive, a second `Index::writer()` fails immediately with `LockBusy`. So
//! two indexing jobs landing on the same (tenant, repo, branch) at the same time
//! used to make one of them fail — and a failed per-node job means that node is
//! simply absent from search, permanently, until a rebuild. `do_batch_index`
//! looked reliable next to it only because a batch is ONE writer for many nodes.
//!
//! Measured before the fix, 8 concurrent `do_index_node` calls: **1 of 8**
//! documents indexed, 7 `LockBusy` errors. After: 8 of 8.
//!
//! Every assertion here reads through a BRAND-NEW `TantivyIndexingEngine` on the
//! same directory, so the cached `IndexReader` (`ReloadPolicy::OnCommitWithDelay`)
//! cannot mask or fake a result — a missing doc is genuinely missing from disk.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::fulltext::{
    FullTextIndexJob, FullTextSearchQuery, IndexingEngine, JobKind, NodeIndexPlan,
};

use raisin_indexer::{BatchIndexContext, TantivyIndexingEngine};

const TENANT: &str = "t";
const REPO: &str = "r";
const BRANCH: &str = "main";
const WS: &str = "content";
const TERM: &str = "zephyrine";

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A unique scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "raisin-indexer-durability-{}-{}-{}",
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

fn node(id: &str) -> Node {
    let mut properties = HashMap::new();
    properties.insert(
        "content".to_string(),
        PropertyValue::String(format!("the {TERM} report for {id}")),
    );
    Node {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/docs/{id}"),
        node_type: "test:Doc".to_string(),
        workspace: Some(WS.to_string()),
        properties,
        ..Default::default()
    }
}

fn job(node_id: &str, revision: u64) -> FullTextIndexJob {
    FullTextIndexJob {
        job_id: format!("job-{node_id}"),
        kind: JobKind::AddNode,
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        workspace_id: WS.to_string(),
        branch: BRANCH.to_string(),
        revision: HLC::new(revision, 0),
        node_id: Some(node_id.to_string()),
        source_branch: None,
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
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

/// Search through a FRESH engine (empty index cache, fresh reader) so the answer
/// reflects what is actually committed on disk, not a stale cached reader.
fn ids_on_disk(base: &PathBuf) -> Vec<String> {
    let engine = TantivyIndexingEngine::new(base.clone(), 64 * 1024 * 1024).expect("engine");
    let mut ids: Vec<String> = engine
        .search(&FullTextSearchQuery {
            tenant_id: TENANT.to_string(),
            repo_id: REPO.to_string(),
            workspace_ids: Some(vec![WS.to_string()]),
            branch: BRANCH.to_string(),
            language: "en".to_string(),
            query: TERM.to_string(),
            limit: 1000,
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

fn expected_ids(count: usize) -> Vec<String> {
    let mut ids: Vec<String> = (0..count).map(|i| format!("doc-{i:02}")).collect();
    ids.sort();
    ids
}

/// The regression: index N nodes one at a time, exactly as the per-node job does,
/// then assert every one of them survived. Before the fix, the last document was
/// intermittently absent from `meta.json` forever.
#[test]
fn single_node_indexing_never_loses_a_document() {
    const NODES: usize = 8;
    // Several independent rounds, because the loss was intermittent.
    const ROUNDS: usize = 6;

    for round in 0..ROUNDS {
        let scratch = Scratch::new(&format!("seq-{round}"));
        let engine =
            TantivyIndexingEngine::new(scratch.0.clone(), 64 * 1024 * 1024).expect("engine");

        for i in 0..NODES {
            let id = format!("doc-{i:02}");
            engine
                .do_index_node_with_plan(&job(&id, 1000 + i as u64), &node(&id), &plan())
                .unwrap_or_else(|e| panic!("round {round}: index {id}: {e}"));
        }

        assert_eq!(
            ids_on_disk(&scratch.0),
            expected_ids(NODES),
            "round {round}: a single-node-indexed document is missing from the committed index"
        );
    }
}

/// Re-indexing the same node repeatedly must leave exactly one live document.
/// This is the delete-then-add ordering inside the single-node path: the delete
/// must remove the previous revision without removing the doc just added.
#[test]
fn reindexing_the_same_node_keeps_exactly_one_document() {
    let scratch = Scratch::new("same-node");
    let engine = TantivyIndexingEngine::new(scratch.0.clone(), 64 * 1024 * 1024).expect("engine");

    for revision in 0..10u64 {
        engine
            .do_index_node_with_plan(&job("doc-00", 1000 + revision), &node("doc-00"), &plan())
            .expect("index");
    }

    assert_eq!(
        ids_on_disk(&scratch.0),
        vec!["doc-00".to_string()],
        "re-indexing one node should leave exactly one live document"
    );
}

/// A single-node index followed by a delete followed by another index must end
/// with the node present — the writer lifecycles must not race each other.
#[test]
fn index_delete_reindex_ends_with_the_node_present() {
    let scratch = Scratch::new("cycle");
    let engine = TantivyIndexingEngine::new(scratch.0.clone(), 64 * 1024 * 1024).expect("engine");

    for revision in 0..5u64 {
        engine
            .do_index_node_with_plan(&job("doc-00", 1000 + revision), &node("doc-00"), &plan())
            .expect("index");
        engine
            .do_delete_node(&job("doc-00", 1000 + revision))
            .expect("delete");
    }
    engine
        .do_index_node_with_plan(&job("doc-00", 2000), &node("doc-00"), &plan())
        .expect("final index");

    assert_eq!(
        ids_on_disk(&scratch.0),
        vec!["doc-00".to_string()],
        "the last index must survive the preceding delete"
    );
}

/// THE REGRESSION. Several per-node indexing jobs in flight at once against the
/// same index — exactly what the job runner produces, since each job hops to
/// `spawn_blocking` independently.
///
/// Before the engine owned the writer lifecycle this indexed 1 of 8 documents
/// and returned seven "Failed to acquire Lockfile: LockBusy" errors. Nothing
/// downstream repairs that: the node is missing from search until a rebuild.
#[test]
fn concurrent_single_node_indexing_loses_nothing() {
    const NODES: usize = 8;

    let scratch = Scratch::new("concurrent");
    let engine =
        Arc::new(TantivyIndexingEngine::new(scratch.0.clone(), 64 * 1024 * 1024).expect("engine"));
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let handles: Vec<_> = (0..NODES)
        .map(|i| {
            let engine = Arc::clone(&engine);
            let errors = Arc::clone(&errors);
            std::thread::spawn(move || {
                let id = format!("doc-{i:02}");
                if let Err(e) =
                    engine.do_index_node_with_plan(&job(&id, 1000 + i as u64), &node(&id), &plan())
                {
                    errors.lock().unwrap().push(format!("{id}: {e}"));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("indexing thread panicked");
    }

    let errors = errors.lock().unwrap().clone();
    assert!(
        errors.is_empty(),
        "concurrent single-node indexing errored: {errors:#?}"
    );
    assert_eq!(
        ids_on_disk(&scratch.0),
        expected_ids(NODES),
        "a concurrently indexed document is missing from the committed index"
    );
}

/// A bulk reindex (`do_batch_index`) and the live per-node path share one index
/// directory, so they must share one writer slot too. Running them concurrently
/// must not cost either side a document.
#[test]
fn a_batch_and_the_single_node_path_do_not_collide() {
    const BATCHED: usize = 4;
    const SINGLES: usize = 4;

    let scratch = Scratch::new("batch-vs-single");
    let engine =
        Arc::new(TantivyIndexingEngine::new(scratch.0.clone(), 64 * 1024 * 1024).expect("engine"));
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let mut handles = Vec::new();

    {
        let engine = Arc::clone(&engine);
        let errors = Arc::clone(&errors);
        handles.push(std::thread::spawn(move || {
            let context = BatchIndexContext {
                tenant_id: TENANT.to_string(),
                repo_id: REPO.to_string(),
                branch: BRANCH.to_string(),
                workspace_id: WS.to_string(),
                default_language: "en".to_string(),
                supported_languages: vec!["en".to_string()],
            };
            let node_plans = (0..BATCHED)
                .map(|i| (node(&format!("doc-{i:02}")), plan()))
                .collect();
            if let Err(e) = engine.do_batch_index(&context, node_plans, vec![]) {
                errors.lock().unwrap().push(format!("batch: {e}"));
            }
        }));
    }

    for i in BATCHED..BATCHED + SINGLES {
        let engine = Arc::clone(&engine);
        let errors = Arc::clone(&errors);
        handles.push(std::thread::spawn(move || {
            let id = format!("doc-{i:02}");
            if let Err(e) =
                engine.do_index_node_with_plan(&job(&id, 1000 + i as u64), &node(&id), &plan())
            {
                errors.lock().unwrap().push(format!("{id}: {e}"));
            }
        }));
    }

    for h in handles {
        h.join().expect("indexing thread panicked");
    }

    let errors = errors.lock().unwrap().clone();
    assert!(
        errors.is_empty(),
        "batch and single-node indexing collided: {errors:#?}"
    );
    assert_eq!(
        ids_on_disk(&scratch.0),
        expected_ids(BATCHED + SINGLES),
        "a document was lost when a batch ran alongside the per-node path"
    );
}
