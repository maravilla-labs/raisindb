//! What runs BESIDE a collapse slice: retention GC (which deletes the other
//! end of the same pair), an in-place writer (which rewrites the very key a
//! slice doomed), and an inserter waiting for a slice over clean data.

use super::env::{node, options, put_on, Env, REPO, TENANT, WS};
use super::gc_tests::{keep, titled};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_rocksdb::management::async_indexing::repair::{CommitHook, RepairOptions};
use raisin_rocksdb::management::cf_exclusion;
use raisin_rocksdb::management::history_gc::run_history_gc;
use raisin_rocksdb::{cf, keys};
use raisin_storage::scope::BranchScope;
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn only_property_index(hook: CommitHook) -> RepairOptions {
    RepairOptions {
        collapse_cfs: Some(vec![cf::PROPERTY_INDEX.to_string()]),
        before_commit: Some(hook),
        ..options()
    }
}

/// A hook that runs `work` on another thread the first time a commit starts
/// (inside the slice, holds taken), then gives it time to land.
fn race(
    work: impl FnOnce() + Send + 'static,
) -> (CommitHook, Arc<Mutex<Option<std::thread::JoinHandle<()>>>>) {
    let work = Mutex::new(Some(work));
    let handle = Arc::new(Mutex::new(None));
    let slot = handle.clone();
    let hook = CommitHook(Arc::new(move || {
        let Some(work) = work.lock().unwrap().take() else {
            return;
        };
        *slot.lock().unwrap() = Some(std::thread::spawn(work));
        std::thread::sleep(Duration::from_millis(300));
    }));
    (hook, handle)
}

fn join(handle: &Mutex<Option<std::thread::JoinHandle<()>>>) {
    handle
        .lock()
        .unwrap()
        .take()
        .expect("the hook ran")
        .join()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retention_gc_racing_a_collapse_slice_keeps_the_group() -> Result<()> {
    let env = Env::new().await?;
    env.add("main", node("n", "/n", "v")).await?;
    let mut again = node("n", "/n", "v");
    again.properties.insert(
        "round".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("1".to_string()),
    );
    // title = v re-put: the group (title, v, n) holds v@r1 and v@r2.
    env.put("main", again).await?;
    env.add("main", node("o", "/o", "other")).await?;
    assert_eq!(titled(&env, "main", "v").await?, ["n"]);

    // Retention (keep the newest revision) decides "keep r2, delete r1" while
    // the slice has decided "delete r2, the twin of r1".
    let storage = env.storage.clone();
    let (hook, gc) = race(move || {
        run_history_gc(&storage, &keep(1)).expect("retention GC");
    });
    let reports = env
        .collapse(Some("main"), only_property_index(hook))
        .await?;
    assert!(reports[0].completed, "{:?}", reports[0]);
    join(&gc);
    assert_eq!(
        titled(&env, "main", "v").await?,
        ["n"],
        "retention GC and collapse together emptied the group"
    );
    Ok(())
}

#[tokio::test]
async fn collapse_reports_busy_while_retention_gc_runs() -> Result<()> {
    let env = Env::new().await?;
    for v in 0..3 {
        env.put("main", node("n", "/n", "v")).await?;
        env.put("main", node(&format!("o{v}"), &format!("/o{v}"), "o"))
            .await?;
    }
    let before = env.raw(cf::PROPERTY_INDEX, "main");
    let pruning = cf_exclusion::enter_pruner(env.storage.db());
    let opts = RepairOptions {
        collapse_cfs: Some(vec![cf::PROPERTY_INDEX.to_string()]),
        ..options()
    };
    let busy = env.collapse(Some("main"), opts.clone()).await?;
    assert_eq!(busy[0].collapse.busy, vec![cf::PROPERTY_INDEX.to_string()]);
    assert_eq!(env.raw(cf::PROPERTY_INDEX, "main"), before);
    drop(pruning);
    let done = env.collapse(Some("main"), opts).await?;
    assert!(done[0].completed);
    assert!(env.raw(cf::PROPERTY_INDEX, "main").len() < before.len());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collapse_slice_over_clean_data_releases_the_exclusion() -> Result<()> {
    let env = Env::new().await?;
    for i in 0..10 {
        for v in 0..3 {
            let mut n = node(&format!("p{i}"), &format!("/p{i}"), "t");
            n.properties.insert(
                "round".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(v.to_string()),
            );
            env.put("main", n).await?;
        }
    }
    // Collapsed once: a re-run finds nothing to delete.
    env.collapse(Some("main"), options()).await?;
    let scanned = env.raw(cf::PROPERTY_INDEX, "main").len();
    assert!(scanned > 40, "{scanned}");

    // An inserter (a fork or merge copy) arrives during the first slice.
    let commits = Arc::new(AtomicUsize::new(0));
    let entered_at = Arc::new(AtomicUsize::new(usize::MAX));
    let (hook, inserter) = {
        let db = env.storage.clone();
        let (commits, entered_at) = (commits.clone(), entered_at.clone());
        let counting = commits.clone();
        let (inner, handle) = race(move || {
            let _held =
                cf_exclusion::enter_inserter(db.db(), TENANT, REPO, "main", cf::PROPERTY_INDEX);
            entered_at.store(commits.load(Ordering::SeqCst), Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
        });
        let hook = CommitHook(Arc::new(move || {
            counting.fetch_add(1, Ordering::SeqCst);
            (inner.0)();
        }));
        (hook, handle)
    };
    let reports = env
        .collapse(
            Some("main"),
            RepairOptions {
                collapse_slice_keys: 10,
                ..only_property_index(hook)
            },
        )
        .await?;
    join(&inserter);
    let report = &reports[0];
    assert!(report.completed, "{report:?}");
    assert_eq!(
        report.collapse.column_families[cf::PROPERTY_INDEX].deleted,
        0
    );
    let total = commits.load(Ordering::SeqCst);
    assert!(
        total > 3,
        "slices must end on keys scanned, not deletes: {total}"
    );
    let entered = entered_at.load(Ordering::SeqCst);
    assert!(
        entered < total,
        "the inserter waited for the whole scan ({entered} of {total} commits)"
    );
    Ok(())
}

const PING: &str = "test:CollapsePing";

/// A `versionable: false` type: an update rewrites the node at its current
/// revision instead of minting one.
async fn register_non_versionable(env: &Env) -> Result<()> {
    let ty = NodeType {
        id: Some(PING.to_string()),
        strict: Some(false),
        name: PING.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: vec!["*".to_string()],
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(false),
        immutable: None,
        publishable: Some(true),
        auditable: Some(false),
        indexable: Some(true),
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    };
    env.storage
        .node_types()
        .upsert(
            BranchScope::new(TENANT, REPO, "main"),
            ty,
            CommitMetadata::system("seed type"),
        )
        .await?;
    Ok(())
}

fn ping(title: &str) -> raisin_models::nodes::Node {
    let mut n = node("n", "/n", title);
    n.node_type = PING.to_string();
    n
}

/// `n`'s PROPERTY_INDEX `title` entries: `(value bytes in key, revision, live)`.
pub(super) fn title_entries(env: &Env, value: &str) -> Vec<(HLC, bool)> {
    let prefix = keys::property_index_key_versioned(
        TENANT,
        REPO,
        "main",
        WS,
        "title",
        value,
        &HLC::new(0, 0),
        "n",
        false,
    );
    let (group, _) = super::env::split(&prefix, false).unwrap();
    env.raw(cf::PROPERTY_INDEX, "main")
        .into_iter()
        .filter_map(|(k, v)| {
            let (g, rev) = super::env::split(&k, false)?;
            (g == group).then(|| (rev, !keys::is_tombstone_value(&v)))
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_place_rewrite_of_a_doomed_key_is_not_deleted() -> Result<()> {
    let env = Env::new().await?;
    register_non_versionable(&env).await?;
    env.add("main", ping("v")).await?;
    let [(r1, true)] = title_entries(&env, "v")[..] else {
        panic!("one live v entry");
    };
    // An older entry of the same group: collapse folds v@r1 into it.
    let r0 = HLC::new(r1.timestamp_ms - 1, 0);
    let db = env.storage.db();
    let older =
        keys::property_index_key_versioned(TENANT, REPO, "main", WS, "title", "v", &r0, "n", false);
    let live = env
        .raw(cf::PROPERTY_INDEX, "main")
        .into_iter()
        .find(|(k, _)| super::env::split(k, false).is_some_and(|(_, r)| r == r1))
        .unwrap()
        .1;
    db.put_cf(db.cf_handle(cf::PROPERTY_INDEX).unwrap(), older, live)
        .unwrap();
    env.add("main", node("o", "/o", "other")).await?;

    // The in-place update (v -> w AT r1) lands after the slice decided to
    // delete key v@r1 and before it commits.
    let storage = env.storage.clone();
    let rt = tokio::runtime::Handle::current();
    let wrote = Arc::new(AtomicBool::new(false));
    let flag = wrote.clone();
    let (hook, writer) = race(move || {
        rt.block_on(put_on(&storage, "main", ping("w")))
            .expect("in-place write");
        flag.store(true, Ordering::SeqCst);
    });
    let reports = env
        .collapse(Some("main"), only_property_index(hook))
        .await?;
    join(&writer);
    assert!(reports[0].completed && wrote.load(Ordering::SeqCst));
    assert_eq!(
        title_entries(&env, "w"),
        vec![(r1, true)],
        "the update must have been in place"
    );
    assert!(
        titled(&env, "main", "v").await?.is_empty(),
        "n still matches v"
    );
    assert_eq!(titled(&env, "main", "w").await?, ["n"]);
    Ok(())
}
