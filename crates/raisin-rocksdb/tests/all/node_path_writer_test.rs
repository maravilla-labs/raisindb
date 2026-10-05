//! Phase 10 / 10b: the node record and `NODE_PATH`.
//!
//! Before Phase 10, `put_node`/`add_node` stored the full `Node` (path
//! embedded) and no `NODE_PATH` entry, and the decoder let `NODE_PATH` win
//! whenever it had ANY entry — so a node created through the repository and
//! renamed through `put_node` read back its pre-rename path. Phase 10 added
//! the read rule (the newer of `NODE_PATH` and the embedded path) and the
//! `node_path` backfill; Phase 10b made every writer store ONE format — a
//! `StorageNode` plus `NODE_PATH` through the one record writer — with no gate.
//!
//! Legacy bytes still exist on disk in older databases (and arrive in a
//! checkpoint from an older peer), so the read rule and the backfill are
//! tested against them. No production switch writes them any more:
//! [`legacy_record`] rewrites a committed record RAW into exactly what a
//! pre-Phase-10 `put_node` left — the full `Node` blob and no `NODE_PATH`
//! entry at that revision.

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_rocksdb::management::async_indexing::repair::{
    load_state, run_repair, RepairKind, RepairOptions,
};
use raisin_rocksdb::{cf, keys, RocksDBStorage};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository, RepoScope,
    RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use std::collections::HashMap;
use tempfile::TempDir;

pub(crate) const TENANT: &str = "np-tenant";
pub(crate) const REPO: &str = "repo";
pub(crate) const BRANCH: &str = "main";
pub(crate) const WS: &str = "default";

pub(crate) fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

pub(crate) async fn setup() -> Result<(RocksDBStorage, TempDir)> {
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
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await?;
    Ok((storage, temp_dir))
}

pub(crate) fn folder(id: &str, path: &str) -> Node {
    let name = path.rsplit('/').next().unwrap().to_string();
    let parent = match path.rsplitn(2, '/').nth(1) {
        Some(p) if !p.is_empty() => Some(p.to_string()),
        _ => None,
    };
    Node {
        id: id.to_string(),
        path: path.to_string(),
        name,
        parent,
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    }
}

pub(crate) async fn head(storage: &RocksDBStorage) -> Result<HLC> {
    storage.branches().get_head(TENANT, REPO, BRANCH).await
}

/// Create through the REPOSITORY (StorageNode + NODE_PATH); returns HEAD.
pub(crate) async fn repo_create(storage: &RocksDBStorage, node: Node) -> Result<HLC> {
    let options = CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    };
    storage.nodes().create(scope(), node, options).await?;
    head(storage).await
}

/// One transaction `put_node`; returns HEAD.
pub(crate) async fn tx_put(storage: &RocksDBStorage, node: &Node) -> Result<HLC> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message("put")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    tx.put_node(WS, node).await?;
    tx.commit().await?;
    head(storage).await
}

/// What a pre-Phase-10 binary's `put_node` wrote, reproduced: a transaction
/// `put_node`, then its record rewritten raw by [`legacy_record`]. Returns
/// HEAD.
pub(crate) async fn legacy_tx_put(storage: &RocksDBStorage, node: &Node) -> Result<HLC> {
    let rev = tx_put(storage, node).await?;
    legacy_record(storage, &node.id, &rev);
    Ok(rev)
}

/// TEST-ONLY: rewrite `id`'s record at `at` into the pre-Phase-10 format —
/// the full `Node` blob with its path embedded, and NO `NODE_PATH` entry at
/// that revision. PATH_INDEX is left alone: both formats wrote it the same.
pub(crate) fn legacy_record(storage: &RocksDBStorage, id: &str, at: &HLC) {
    let db = storage.db();
    let cf_nodes = db.cf_handle(cf::NODES).unwrap();
    let cf_node_path = db.cf_handle(cf::NODE_PATH).unwrap();
    let entry_key = keys::node_path_key_versioned(TENANT, REPO, BRANCH, WS, id, at);
    let path = db
        .get_cf(cf_node_path, &entry_key)
        .unwrap()
        .expect("the one writer wrote NODE_PATH at the record's revision");
    let (mut node, _) = raisin_rocksdb::decode_node_blob(&blob_at(storage, id, at)).unwrap();
    node.path = String::from_utf8(path).unwrap();
    db.put_cf(
        cf_nodes,
        keys::node_key_versioned(TENANT, REPO, BRANCH, WS, id, at),
        rmp_serde::to_vec_named(&node).unwrap(),
    )
    .unwrap();
    db.delete_cf(cf_node_path, entry_key).unwrap();
}

pub(crate) async fn path_at(
    storage: &RocksDBStorage,
    id: &str,
    at: Option<&HLC>,
) -> Option<String> {
    storage
        .nodes()
        .get(scope(), id, at)
        .await
        .unwrap()
        .map(|n| n.path)
}

pub(crate) async fn id_at_path(
    storage: &RocksDBStorage,
    path: &str,
    at: Option<&HLC>,
) -> Option<String> {
    storage
        .nodes()
        .get_by_path(scope(), path, at)
        .await
        .unwrap()
        .map(|n| n.id)
}

/// Every `(revision, path)` NODE_PATH holds for `id`, oldest first — read raw.
pub(crate) fn node_path_entries(storage: &RocksDBStorage, id: &str) -> Vec<(HLC, String)> {
    let prefix = keys::node_path_key_prefix(TENANT, REPO, BRANCH, WS, id);
    let db = storage.db();
    let cf = db.cf_handle(cf::NODE_PATH).unwrap();
    let mut out: Vec<(HLC, String)> = db
        .prefix_iterator_cf(cf, &prefix)
        .flatten()
        .take_while(|(k, _)| k.starts_with(&prefix))
        .map(|(k, v)| {
            (
                keys::extract_revision_from_key(&k).unwrap(),
                String::from_utf8_lossy(&v).into_owned(),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The raw NODES blob of `id` at exactly `at`.
pub(crate) fn blob_at(storage: &RocksDBStorage, id: &str, at: &HLC) -> Vec<u8> {
    let db = storage.db();
    let cf = db.cf_handle(cf::NODES).unwrap();
    db.get_cf(
        cf,
        keys::node_key_versioned(TENANT, REPO, BRANCH, WS, id, at),
    )
    .unwrap()
    .expect("node blob")
}

pub(crate) fn backfill_options() -> RepairOptions {
    RepairOptions {
        check_headroom: false,
        max_bytes_per_sec: 0,
        ..RepairOptions::default()
    }
}

/// The plan's stale-path witness: repository create (NODE_PATH /a at r1),
/// then a rename through the OLD put_node (full blob /b at r2, no entry). It
/// read back /a — at HEAD and at r2 — because NODE_PATH always won. Then the
/// same history through today's one writer.
#[tokio::test]
async fn repo_created_then_put_node_rename_reads_new_path() -> Result<()> {
    let (storage, _dir) = setup().await?;
    let r1 = repo_create(&storage, folder("n1", "/a")).await?;
    let r2 = legacy_tx_put(&storage, &folder("n1", "/b")).await?;

    assert_eq!(path_at(&storage, "n1", None).await.as_deref(), Some("/b"));
    assert_eq!(
        path_at(&storage, "n1", Some(&r2)).await.as_deref(),
        Some("/b")
    );
    assert_eq!(
        path_at(&storage, "n1", Some(&r1)).await.as_deref(),
        Some("/a")
    );
    assert_eq!(
        id_at_path(&storage, "/b", None).await.as_deref(),
        Some("n1")
    );
    assert_eq!(id_at_path(&storage, "/a", None).await, None);

    // The same history through the one record writer: identical answers.
    let r3 = repo_create(&storage, folder("n2", "/c")).await?;
    let r4 = tx_put(&storage, &folder("n2", "/d")).await?;
    assert_eq!(path_at(&storage, "n2", None).await.as_deref(), Some("/d"));
    assert_eq!(
        path_at(&storage, "n2", Some(&r4)).await.as_deref(),
        Some("/d")
    );
    assert_eq!(
        path_at(&storage, "n2", Some(&r3)).await.as_deref(),
        Some("/c")
    );
    Ok(())
}

/// The transaction path stores a StorageNode (no embedded path) and a
/// NODE_PATH entry at the write's revision — there is no other format. A
/// legacy record (written raw) still reads back through the read rule, and
/// its next write goes through the one writer.
#[tokio::test]
async fn put_node_writes_node_path() -> Result<()> {
    let (storage, _dir) = setup().await?;
    let r1 = tx_put(&storage, &folder("new", "/new")).await?;
    assert_eq!(
        node_path_entries(&storage, "new"),
        vec![(r1, "/new".to_string())]
    );
    let (decoded, _) = raisin_rocksdb::decode_node_blob(&blob_at(&storage, "new", &r1))?;
    assert_eq!(decoded.path, "", "a StorageNode blob embeds no path");
    assert_eq!(
        path_at(&storage, "new", None).await.as_deref(),
        Some("/new")
    );

    let r2 = legacy_tx_put(&storage, &folder("old", "/old")).await?;
    assert!(node_path_entries(&storage, "old").is_empty());
    let (decoded, _) = raisin_rocksdb::decode_node_blob(&blob_at(&storage, "old", &r2))?;
    assert_eq!(decoded.path, "/old", "the legacy blob embeds its path");
    assert_eq!(
        path_at(&storage, "old", None).await.as_deref(),
        Some("/old")
    );

    let r3 = tx_put(&storage, &folder("old", "/old2")).await?;
    assert_eq!(
        node_path_entries(&storage, "old"),
        vec![(r3, "/old2".to_string())]
    );
    let (decoded, _) = raisin_rocksdb::decode_node_blob(&blob_at(&storage, "old", &r3))?;
    assert_eq!(decoded.path, "");
    assert_eq!(
        path_at(&storage, "old", Some(&r2)).await.as_deref(),
        Some("/old")
    );
    Ok(())
}

/// Legacy bytes ABOVE one-format records: the one writer has run
/// (StorageNode + NODE_PATH /b at r2), then a legacy full blob /c lands at r3
/// with no entry — what a checkpoint from a peer still on a pre-Phase-10
/// binary carries (a downgrade below Phase 10b itself is unsupported). The
/// read rule reads /c, because the embedded path is newer than the stale
/// entry.
#[tokio::test]
async fn downgrade_rename_through_old_put_node_reads_new_path() -> Result<()> {
    let (storage, _dir) = setup().await?;
    let r1 = tx_put(&storage, &folder("n1", "/a")).await?;
    let r2 = tx_put(&storage, &folder("n1", "/b")).await?;
    let r3 = legacy_tx_put(&storage, &folder("n1", "/c")).await?;

    assert_eq!(path_at(&storage, "n1", None).await.as_deref(), Some("/c"));
    assert_eq!(
        path_at(&storage, "n1", Some(&r3)).await.as_deref(),
        Some("/c")
    );
    assert_eq!(
        path_at(&storage, "n1", Some(&r2)).await.as_deref(),
        Some("/b")
    );
    assert_eq!(
        path_at(&storage, "n1", Some(&r1)).await.as_deref(),
        Some("/a")
    );
    assert_eq!(
        id_at_path(&storage, "/c", None).await.as_deref(),
        Some("n1")
    );

    // And through the one writer again: a further rename reads back as well.
    tx_put(&storage, &folder("n1", "/d")).await?;
    assert_eq!(path_at(&storage, "n1", None).await.as_deref(), Some("/d"));
    assert_eq!(
        path_at(&storage, "n1", Some(&r3)).await.as_deref(),
        Some("/c")
    );
    Ok(())
}

/// The backfill writes NODE_PATH at EVERY legacy revision whose path the
/// index does not already answer — not only the latest, not only nodes with
/// no entry — and afterwards NODE_PATH alone answers every revision.
#[tokio::test]
async fn node_path_backfill_writes_at_each_divergent_revision() -> Result<()> {
    let (storage, _dir) = setup().await?;
    let r1 = repo_create(&storage, folder("n1", "/a")).await?; // entry /a
    let r2 = legacy_tx_put(&storage, &folder("n1", "/b")).await?; // legacy /b
    let r3 = legacy_tx_put(&storage, &folder("n1", "/c")).await?; // legacy /c
    let r4 = legacy_tx_put(&storage, &folder("n1", "/c")).await?; // legacy /c, same path
    let r5 = legacy_tx_put(&storage, &folder("solo", "/solo")).await?; // legacy, no entry

    let dry = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::NodePath,
        RepairOptions {
            dry_run: true,
            ..backfill_options()
        },
    )
    .await?;
    assert_eq!(dry[0].node_path.written, 3, "r2, r3 and solo@r5: {dry:?}");
    assert!(
        node_path_entries(&storage, "n1").len() == 1,
        "a dry run writes nothing"
    );

    let reports = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::NodePath,
        backfill_options(),
    )
    .await?;
    assert!(reports[0].completed);
    assert_eq!(reports[0].node_path.legacy_versions, 4);
    assert_eq!(reports[0].node_path.written, 3);
    assert_eq!(
        node_path_entries(&storage, "n1"),
        vec![
            (r1, "/a".to_string()),
            (r2, "/b".to_string()),
            (r3, "/c".to_string()),
        ],
        "r4 needs no entry: r3's already answers it"
    );
    assert_eq!(
        node_path_entries(&storage, "solo"),
        vec![(r5, "/solo".to_string())]
    );

    // Reads are unchanged, and the index alone now agrees at every revision.
    for (at, want) in [(r1, "/a"), (r2, "/b"), (r3, "/c"), (r4, "/c")] {
        assert_eq!(
            path_at(&storage, "n1", Some(&at)).await.as_deref(),
            Some(want)
        );
        let indexed = node_path_entries(&storage, "n1")
            .into_iter()
            .filter(|(rev, _)| rev <= &at)
            .last()
            .map(|(_, p)| p);
        assert_eq!(indexed.as_deref(), Some(want), "NODE_PATH alone at {at}");
    }

    // Idempotent: a second run finds nothing to write.
    let again = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::NodePath,
        backfill_options(),
    )
    .await?;
    assert_eq!(again[0].node_path.written, 0);
    let state = load_state(storage.db(), TENANT, REPO, BRANCH, "node_path", "local")?
        .expect("state record");
    assert_eq!(state.status, "done");
    Ok(())
}

/// Kill the backfill at a batch boundary, run it again: it resumes from the
/// persisted cursor (not from the start), and the end state equals an
/// uninterrupted run's.
#[tokio::test]
async fn node_path_backfill_resumes_after_crash() -> Result<()> {
    let (storage, _dir) = setup().await?;
    let mut legacy = Vec::new();
    for i in 0..12 {
        let id = format!("n{i:02}");
        repo_create(&storage, folder(&id, &format!("/{id}"))).await?;
        let rev = legacy_tx_put(&storage, &folder(&id, &format!("/{id}-renamed"))).await?;
        legacy.push((id, rev));
    }

    // Tiny batches: every node fills one, so the crash hook stops early.
    let crashed = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::NodePath,
        RepairOptions {
            batch_bytes: 1,
            stop_after_batches: Some(4),
            ..backfill_options()
        },
    )
    .await?;
    assert!(!crashed[0].completed, "the crash hook stopped the run");
    let written_before = crashed[0].node_path.written;
    assert!(written_before > 0 && written_before < 12, "{crashed:?}");
    let state = load_state(storage.db(), TENANT, REPO, BRANCH, "node_path", "local")?
        .expect("state record");
    assert_eq!(state.status, "running");
    assert!(state.cursor.is_some());

    let resumed = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::NodePath,
        RepairOptions {
            batch_bytes: 1,
            ..backfill_options()
        },
    )
    .await?;
    assert!(resumed[0].resumed, "picked up from the cursor");
    assert!(resumed[0].completed);
    assert!(
        resumed[0].node_path.versions < 2 * 12,
        "resumed, not restarted: {resumed:?}"
    );
    assert_eq!(
        written_before + resumed[0].node_path.written,
        12,
        "no node skipped, none written twice"
    );

    // The end state of an uninterrupted run: one entry per legacy revision.
    for (id, rev) in &legacy {
        let entries = node_path_entries(&storage, id);
        assert_eq!(entries.len(), 2, "{id}: {entries:?}");
        assert_eq!(entries[1], (*rev, format!("/{id}-renamed")));
    }
    let again = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::NodePath,
        backfill_options(),
    )
    .await?;
    assert_eq!(again[0].node_path.written, 0);
    Ok(())
}
