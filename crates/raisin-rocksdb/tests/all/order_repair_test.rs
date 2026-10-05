//! The streaming ORDERED_CHILDREN and PATH_INDEX repairs.
//!
//! The data these repair was written by the old delete tombstoner, which keyed
//! the ORDERED_CHILDREN tombstone by the parent's NAME, so the child's real
//! entry stayed live. The fixed writer no longer produces that, so each test
//! fabricates it: delete normally, then remove the (correct) tombstone the
//! delete wrote — exactly the state an old binary left behind.

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_rocksdb::management::async_indexing::repair::{
    load_state, run_repair, RepairKind, RepairOptions,
};
use raisin_rocksdb::{cf, keys, RocksDBStorage};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, DeleteNodeOptions, NodeRepository, RegistryRepository,
    RepoScope, RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use std::collections::HashMap;
use tempfile::TempDir;

const TENANT: &str = "repair-tenant";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

fn scope(branch: &str) -> StorageScope<'_> {
    StorageScope::new(TENANT, REPO, branch, WORKSPACE)
}

async fn setup() -> Result<(RocksDBStorage, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path())?;
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;
    storage
        .repository_management()
        .create_repository(
            TENANT,
            REPO,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".to_string()],
                locale_fallback_chains: HashMap::new(),
                default_branch: BRANCH.to_string(),
                description: None,
                tags: HashMap::new(),
                localized_names: Default::default(),
            },
        )
        .await?;
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WORKSPACE.to_string()),
        )
        .await?;
    Ok((storage, temp_dir))
}

async fn create_with_id(storage: &RocksDBStorage, id: &str, path: &str) -> Result<()> {
    let name = path.rsplit('/').next().unwrap().to_string();
    let parent = match path.rsplitn(2, '/').nth(1) {
        Some(p) if !p.is_empty() => Some(p.to_string()),
        _ => None,
    };
    let node = Node {
        id: id.to_string(),
        path: path.to_string(),
        name,
        parent,
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    };
    let options = CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    };
    storage.nodes().create(scope(BRANCH), node, options).await
}

async fn delete(storage: &RocksDBStorage, id: &str) -> Result<()> {
    storage
        .nodes()
        .delete(scope(BRANCH), id, DeleteNodeOptions::default())
        .await?;
    Ok(())
}

/// Every `(label, child, live)` decision under `parent_id` on `branch`,
/// newest entry per pair — read raw, so no reader can mask an entry.
fn entries(storage: &RocksDBStorage, branch: &str, parent_id: &str) -> Vec<(String, String, bool)> {
    entries_at(storage, branch, parent_id, None)
}

/// [`entries`] as of `at` (entries above it ignored).
fn entries_at(
    storage: &RocksDBStorage,
    branch: &str,
    parent_id: &str,
    at: Option<HLC>,
) -> Vec<(String, String, bool)> {
    let prefix = keys::ordered_children_prefix(TENANT, REPO, branch, WORKSPACE, parent_id);
    let db = storage.db();
    let cf = db.cf_handle(cf::ORDERED_CHILDREN).unwrap();
    let mut decided = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in db.prefix_iterator_cf(cf, &prefix) {
        let (key, value) = item.unwrap();
        if !key.starts_with(&prefix) {
            break;
        }
        let suffix = &key[prefix.len()..];
        let Some(label_end) = suffix.iter().position(|b| *b == 0) else {
            continue;
        };
        let child_start = label_end + 18;
        if suffix.len() <= child_start {
            continue;
        }
        let revision = HLC::decode_descending(&suffix[label_end + 1..label_end + 17]).unwrap();
        if at.is_some_and(|at| revision > at) {
            continue;
        }
        let label = String::from_utf8_lossy(&suffix[..label_end]).into_owned();
        let child = String::from_utf8_lossy(&suffix[child_start..]).into_owned();
        if decided.insert((label.clone(), child.clone())) {
            out.push((label, child, !keys::is_tombstone_value(&value)));
        }
    }
    out
}

fn live_at(storage: &RocksDBStorage, parent_id: &str, child: &str, at: Option<HLC>) -> usize {
    entries_at(storage, BRANCH, parent_id, at)
        .iter()
        .filter(|(_, c, live)| c == child && *live)
        .count()
}

fn live_children(storage: &RocksDBStorage, branch: &str, parent_id: &str) -> Vec<String> {
    entries(storage, branch, parent_id)
        .into_iter()
        .filter(|(_, _, live)| *live)
        .map(|(_, child, _)| child)
        .collect()
}

/// Remove every ORDERED_CHILDREN tombstone of `child_id` under `parent_id` —
/// the state the name-keyed delete tombstoner left behind.
fn forget_order_tombstones(
    storage: &RocksDBStorage,
    branch: &str,
    parent_id: &str,
    child_id: &str,
) {
    let prefix = keys::ordered_children_prefix(TENANT, REPO, branch, WORKSPACE, parent_id);
    let db = storage.db();
    let cf = db.cf_handle(cf::ORDERED_CHILDREN).unwrap();
    let doomed: Vec<Vec<u8>> = db
        .prefix_iterator_cf(cf, &prefix)
        .map(|item| item.unwrap())
        .take_while(|(key, _)| key.starts_with(&prefix))
        .filter(|(key, value)| {
            keys::is_tombstone_value(value) && key.ends_with(format!("\0{child_id}").as_bytes())
        })
        .map(|(key, _)| key.to_vec())
        .collect();
    for key in doomed {
        db.delete_cf(cf, key).unwrap();
    }
}

fn options() -> RepairOptions {
    RepairOptions {
        check_headroom: false,
        max_bytes_per_sec: 0,
        ..RepairOptions::default()
    }
}

async fn repair(storage: &RocksDBStorage, branch: Option<&str>, opts: RepairOptions) -> Result<()> {
    let reports = run_repair(
        storage,
        TENANT,
        REPO,
        branch,
        RepairKind::OrderedChildren,
        opts,
    )
    .await?;
    assert!(reports.iter().all(|r| r.completed), "{reports:?}");
    Ok(())
}

#[tokio::test]
async fn order_repair_cascade_delete() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    create_with_id(&storage, "p", "/p").await?;
    create_with_id(&storage, "a", "/p/a").await?;
    create_with_id(&storage, "x", "/p/a/x").await?;
    delete(&storage, "p").await?; // cascade: p, a and x at one revision
    for (parent, child) in [("/", "p"), ("p", "a"), ("a", "x")] {
        forget_order_tombstones(&storage, BRANCH, parent, child);
        assert!(live_children(&storage, BRANCH, parent).contains(&child.to_string()));
    }

    repair(&storage, Some(BRANCH), options()).await?;

    for (parent, child) in [("/", "p"), ("p", "a"), ("a", "x")] {
        assert!(
            !live_children(&storage, BRANCH, parent).contains(&child.to_string()),
            "{child} still listed under {parent}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn order_repair_delete_then_recreate() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    create_with_id(&storage, "p", "/p").await?;
    // Deleted and re-created twice: a tombstone is owed per delete, each at
    // its own delete revision.
    let mut while_deleted = Vec::new();
    for _ in 0..2 {
        create_with_id(&storage, "c", "/p/c").await?;
        delete(&storage, "c").await?;
        forget_order_tombstones(&storage, BRANCH, "p", "c");
        while_deleted.push(storage.branches().get_head(TENANT, REPO, BRANCH).await?);
    }
    create_with_id(&storage, "c", "/p/c").await?;
    for at in &while_deleted {
        assert!(
            live_at(&storage, "p", "c", Some(*at)) > 0,
            "stale before repair at {at}"
        );
    }

    repair(&storage, Some(BRANCH), options()).await?;

    for at in &while_deleted {
        assert_eq!(live_at(&storage, "p", "c", Some(*at)), 0, "deleted at {at}");
    }
    assert_eq!(
        live_at(&storage, "p", "c", None),
        1,
        "the re-created child stays listed, once"
    );
    Ok(())
}

#[tokio::test]
async fn order_repair_after_gc_dropped_nodes_versions() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    create_with_id(&storage, "p", "/p").await?;
    create_with_id(&storage, "g", "/p/g").await?;
    // History GC dropped every NODES version of `g`; its entry survived.
    let db = storage.db();
    let cf_nodes = db.cf_handle(cf::NODES).unwrap();
    let node_prefix = keys::node_key_prefix(TENANT, REPO, BRANCH, WORKSPACE, "g");
    let versions: Vec<Vec<u8>> = db
        .prefix_iterator_cf(cf_nodes, &node_prefix)
        .map(|item| item.unwrap().0.to_vec())
        .take_while(|key| key.starts_with(&node_prefix))
        .collect();
    for key in versions {
        db.delete_cf(cf_nodes, key).unwrap();
    }
    assert!(live_children(&storage, BRANCH, "p").contains(&"g".to_string()));

    let reports = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::OrderedChildren,
        options(),
    )
    .await?;
    assert_eq!(reports[0].ordered.without_delete_revision, 1, "{reports:?}");
    assert!(!live_children(&storage, BRANCH, "p").contains(&"g".to_string()));
    Ok(())
}

#[tokio::test]
async fn order_repair_fork_after_delete() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    create_with_id(&storage, "p", "/p").await?;
    create_with_id(&storage, "c", "/p/c").await?;
    delete(&storage, "c").await?;
    forget_order_tombstones(&storage, BRANCH, "p", "c");
    storage
        .branches()
        .create_branch(
            TENANT,
            REPO,
            "fork",
            "system",
            None,
            Some(BRANCH.to_string()),
            false,
            false,
        )
        .await?;
    assert!(live_children(&storage, "fork", "p").contains(&"c".to_string()));

    // Every branch, the fork included.
    repair(&storage, None, options()).await?;

    for branch in [BRANCH, "fork"] {
        assert!(
            !live_children(&storage, branch, "p").contains(&"c".to_string()),
            "{branch}"
        );
    }
    Ok(())
}

async fn many_deleted_children(storage: &RocksDBStorage, count: usize) -> Result<()> {
    create_with_id(storage, "p", "/p").await?;
    for i in 0..count {
        let id = format!("c{i:03}");
        create_with_id(storage, &id, &format!("/p/{id}")).await?;
        delete(storage, &id).await?;
        forget_order_tombstones(storage, BRANCH, "p", &id);
    }
    assert_eq!(live_children(storage, BRANCH, "p").len(), count);
    Ok(())
}

#[tokio::test]
async fn repair_resumes_after_crash() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    many_deleted_children(&storage, 30).await?;

    // "Crash" after two committed batches.
    let crashed = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::OrderedChildren,
        RepairOptions {
            batch_bytes: 256,
            stop_after_batches: Some(2),
            ..options()
        },
    )
    .await?;
    assert!(!crashed[0].completed);
    let state = load_state(
        storage.db(),
        TENANT,
        REPO,
        BRANCH,
        "ordered_children",
        "local",
    )?
    .expect("the state record survives the crash");
    assert_eq!(state.status, "running");
    assert!(state.cursor.is_some());
    let left = live_children(&storage, BRANCH, "p").len();
    assert!(left > 0 && left < 30, "part done, part left: {left}");

    let resumed = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::OrderedChildren,
        RepairOptions {
            batch_bytes: 256,
            ..options()
        },
    )
    .await?;
    assert!(resumed[0].resumed && resumed[0].completed, "{resumed:?}");
    assert!(live_children(&storage, BRANCH, "p").is_empty());
    let state = load_state(
        storage.db(),
        TENANT,
        REPO,
        BRANCH,
        "ordered_children",
        "local",
    )?
    .unwrap();
    assert_eq!(state.status, "done");
    Ok(())
}

#[tokio::test]
async fn repair_streams_within_memory_bound() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    many_deleted_children(&storage, 50).await?;
    const BATCH: usize = 1024;

    // A dry run counts and writes nothing.
    let dry = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::OrderedChildren,
        RepairOptions {
            dry_run: true,
            batch_bytes: BATCH,
            ..options()
        },
    )
    .await?;
    assert_eq!(dry[0].writes.written, 50, "{dry:?}");
    assert_eq!(live_children(&storage, BRANCH, "p").len(), 50);

    let run = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::OrderedChildren,
        RepairOptions {
            batch_bytes: BATCH,
            ..options()
        },
    )
    .await?;
    let writes = &run[0].writes;
    assert_eq!(writes.written, 50, "{writes:?}");
    assert!(
        writes.batches > 1,
        "streamed in several batches: {writes:?}"
    );
    assert!(
        writes.max_batch_bytes <= (BATCH + 512) as u64,
        "no batch beyond the bound plus one entry: {writes:?}"
    );
    assert!(live_children(&storage, BRANCH, "p").is_empty());

    // Clean data: a second run writes nothing.
    let again = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::OrderedChildren,
        options(),
    )
    .await?;
    assert_eq!(again[0].writes.written, 0, "{again:?}");

    // One parent whose children lost every NODES version: the ORDERED_CHILDREN
    // pass does all the writing, and must stream too — it checkpoints per
    // label, not per parent.
    create_with_id(&storage, "q", "/q").await?;
    for i in 0..50 {
        let id = format!("g{i:03}");
        create_with_id(&storage, &id, &format!("/q/{id}")).await?;
        drop_node_versions(&storage, &id);
    }
    assert_eq!(live_children(&storage, BRANCH, "q").len(), 50);
    let run = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::OrderedChildren,
        RepairOptions {
            batch_bytes: BATCH,
            ..options()
        },
    )
    .await?;
    let writes = &run[0].writes;
    assert_eq!(run[0].ordered.without_delete_revision, 50, "{run:?}");
    assert!(
        writes.batches > 1,
        "one parent streamed in several batches: {writes:?}"
    );
    assert!(
        writes.max_batch_bytes <= (BATCH + 512) as u64,
        "no batch beyond the bound plus one entry: {writes:?}"
    );
    assert!(live_children(&storage, BRANCH, "q").is_empty());
    Ok(())
}

/// Delete every NODES version of `id` — what `history_gc` leaves behind.
fn drop_node_versions(storage: &RocksDBStorage, id: &str) {
    let db = storage.db();
    let cf_nodes = db.cf_handle(cf::NODES).unwrap();
    let node_prefix = keys::node_key_prefix(TENANT, REPO, BRANCH, WORKSPACE, id);
    let versions: Vec<Vec<u8>> = db
        .prefix_iterator_cf(cf_nodes, &node_prefix)
        .map(|item| item.unwrap().0.to_vec())
        .take_while(|key| key.starts_with(&node_prefix))
        .collect();
    for key in versions {
        db.delete_cf(cf_nodes, key).unwrap();
    }
}

/// A child moved to another parent by a writer that left its old entry live:
/// the child is alive, so only placement shows the entry is stale.
#[tokio::test]
async fn order_repair_moved_child_stale_entry() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    create_with_id(&storage, "p", "/p").await?;
    create_with_id(&storage, "q", "/q").await?;
    create_with_id(&storage, "c", "/p/c").await?;
    create_with_id(&storage, "s", "/p/s").await?;
    storage
        .nodes()
        .move_node(scope(BRANCH), "c", "/q/c", None)
        .await?;
    forget_order_tombstones(&storage, BRANCH, "p", "c");
    assert!(live_children(&storage, BRANCH, "p").contains(&"c".to_string()));

    let reports = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::OrderedChildren,
        options(),
    )
    .await?;
    assert_eq!(reports[0].ordered.misplaced_entries, 1, "{reports:?}");
    assert_eq!(live_children(&storage, BRANCH, "p"), vec!["s".to_string()]);
    assert_eq!(live_children(&storage, BRANCH, "q"), vec!["c".to_string()]);

    // Clean data: a second run writes nothing.
    let again = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::OrderedChildren,
        options(),
    )
    .await?;
    assert_eq!(again[0].writes.written, 0, "{again:?}");
    Ok(())
}

#[tokio::test]
async fn path_tombstone_repair_rewrites_nul_markers() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    create_with_id(&storage, "a", "/a").await?;
    let marker_key = keys::path_index_key_versioned(
        TENANT,
        REPO,
        BRANCH,
        WORKSPACE,
        "/a",
        &HLC::new(u64::MAX / 2, 0),
    );
    let db = storage.db();
    let cf_path = db.cf_handle(cf::PATH_INDEX).unwrap();
    db.put_cf(cf_path, &marker_key, b"\x00").unwrap();

    let reports = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::PathTombstone,
        options(),
    )
    .await?;
    assert_eq!(reports[0].writes.written, 1);
    assert_eq!(
        db.get_cf(cf_path, &marker_key).unwrap().as_deref(),
        Some(&b"T"[..])
    );

    let again = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::PathTombstone,
        options(),
    )
    .await?;
    assert_eq!(again[0].writes.written, 0);
    Ok(())
}
