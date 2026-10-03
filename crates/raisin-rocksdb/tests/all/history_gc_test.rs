//! History GC removes superseded MVCC versions without changing any read at
//! HEAD, at a tag, or after the cutoff.
//!
//! Before it existed, every write added a permanent version to `nodes` and to
//! every index the node touched; an overwrite deploy or a periodic data tick
//! grew the database by a full copy each time, and RocksDB compaction could
//! not help because an old version is a live key.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::management::history_gc::{
    retention, run_history_gc, GcOptions, HistoryRetention,
};
use raisin_rocksdb::{fractional_index, RocksDBStorage};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, NodeRepository, PropertyIndexRepository, RegistryRepository, RepoScope,
    RepositoryManagementRepository, Storage, StorageScope, TagRepository, WorkspaceRepository,
};
use tempfile::TempDir;

const TENANT: &str = "gc-test";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

async fn setup() -> Result<(Arc<RocksDBStorage>, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(temp_dir.path())?);
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

fn node(id: &str, title: &str) -> Node {
    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String(title.to_string()),
    );
    Node {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/{id}"),
        node_type: "raisin:Folder".to_string(),
        properties,
        order_key: fractional_index::first(),
        parent: Some("/".to_string()),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    }
}

async fn put(storage: &Arc<RocksDBStorage>, n: &Node) -> Result<()> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message("write")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    tx.put_node(WORKSPACE, n).await?;
    tx.commit().await?;
    Ok(())
}

async fn delete(storage: &Arc<RocksDBStorage>, id: &str) -> Result<()> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message("delete")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.delete_node(WORKSPACE, id).await?;
    tx.commit().await?;
    Ok(())
}

async fn head(storage: &Arc<RocksDBStorage>) -> Result<raisin_hlc::HLC> {
    Ok(storage
        .branches()
        .get_branch(TENANT, REPO, BRANCH)
        .await?
        .expect("branch")
        .head)
}

/// Number of stored versions of a node in the `nodes` column family.
fn node_versions(storage: &RocksDBStorage, id: &str) -> usize {
    let prefix = format!("{TENANT}\0{REPO}\0{BRANCH}\0{WORKSPACE}\0nodes\0{id}\0");
    let cf = storage.db().cf_handle("nodes").unwrap();
    storage
        .db()
        .prefix_iterator_cf(cf, prefix.as_bytes())
        .flatten()
        .take_while(|(k, _)| k.starts_with(prefix.as_bytes()))
        .filter(|(k, _)| k.len() == prefix.len() + 16)
        .count()
}

fn aggressive() -> GcOptions {
    GcOptions {
        retention_override: Some(HistoryRetention {
            keep_days: Some(0),
            keep_revisions: None,
        }),
        min_age: Duration::ZERO,
        ..GcOptions::default()
    }
}

async fn titled(storage: &Arc<RocksDBStorage>, title: &str) -> Result<Vec<String>> {
    storage
        .property_index()
        .find_by_property(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "title",
            &PropertyValue::String(title.to_string()),
            false,
            None, // max_revision: branch HEAD
        )
        .await
}

#[tokio::test]
async fn gc_prunes_superseded_versions_and_keeps_every_retained_read() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let scope = StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE);

    for v in 1..=2 {
        put(&storage, &node("page", &format!("v{v}"))).await?;
    }
    let tagged = head(&storage).await?;
    storage
        .tags()
        .create_tag(TENANT, REPO, "release", &tagged, "test", None, false)
        .await?;
    for v in 3..=5 {
        put(&storage, &node("page", &format!("v{v}"))).await?;
    }
    put(&storage, &node("gone", "doomed")).await?;
    put(&storage, &node("gone", "doomed again")).await?;
    delete(&storage, "gone").await?;

    assert_eq!(node_versions(&storage, "page"), 5);

    // A dry run reports and changes nothing.
    let dry = run_history_gc(
        &storage,
        &GcOptions {
            dry_run: true,
            ..aggressive()
        },
    )?;
    assert!(dry.versions_deleted > 0);
    assert_eq!(node_versions(&storage, "page"), 5);

    let report = run_history_gc(&storage, &aggressive())?;
    assert_eq!(report.versions_deleted, dry.versions_deleted);
    assert!(report.column_families["property_index"].versions_deleted > 0);

    // HEAD and the tag survive; the three untagged intermediate versions don't.
    assert_eq!(node_versions(&storage, "page"), 2);
    let now = storage
        .nodes()
        .get(scope, "page", None)
        .await?
        .expect("head");
    assert_eq!(
        now.properties.get("title"),
        Some(&PropertyValue::String("v5".into()))
    );
    let then = storage
        .nodes()
        .get(scope, "page", Some(&tagged))
        .await?
        .expect("tagged");
    assert_eq!(
        then.properties.get("title"),
        Some(&PropertyValue::String("v2".into()))
    );

    // A deleted node leaves nothing behind, and stays deleted.
    assert_eq!(node_versions(&storage, "gone"), 0);
    assert!(storage.nodes().get(scope, "gone", None).await?.is_none());

    // Indexes answer exactly as before at HEAD.
    assert_eq!(titled(&storage, "v5").await?, vec!["page".to_string()]);
    assert!(titled(&storage, "v3").await?.is_empty());
    assert!(titled(&storage, "doomed again").await?.is_empty());

    // A second pass finds nothing more to do.
    let again = run_history_gc(&storage, &aggressive())?;
    assert_eq!(again.versions_deleted, 0);
    Ok(())
}

#[tokio::test]
async fn keep_all_and_stored_policies_are_respected() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    for v in 1..=4 {
        put(&storage, &node("page", &format!("v{v}"))).await?;
    }

    // No policy anywhere: nothing is pruned.
    let report = run_history_gc(
        &storage,
        &GcOptions {
            min_age: Duration::ZERO,
            ..GcOptions::default()
        },
    )?;
    assert_eq!(report.versions_deleted, 0);
    assert_eq!(node_versions(&storage, "page"), 4);

    // A repository policy keeping the last 2 revisions applies to main.
    retention::set_policy(
        storage.db(),
        TENANT,
        REPO,
        retention::ALL_BRANCHES,
        Some(&HistoryRetention {
            keep_days: None,
            keep_revisions: Some(2),
        }),
    )?;
    let report = run_history_gc(
        &storage,
        &GcOptions {
            min_age: Duration::ZERO,
            ..GcOptions::default()
        },
    )?;
    assert!(report.versions_deleted > 0);
    // The 2nd-newest revision is the cutoff: its state and HEAD remain.
    assert_eq!(node_versions(&storage, "page"), 2);

    // The min_age floor protects fresh writes whatever the policy says.
    put(&storage, &node("page", "v5")).await?;
    let report = run_history_gc(
        &storage,
        &GcOptions {
            retention_override: Some(HistoryRetention {
                keep_days: Some(0),
                keep_revisions: None,
            }),
            ..GcOptions::default()
        },
    )?;
    assert_eq!(report.versions_deleted, 0);
    assert_eq!(node_versions(&storage, "page"), 3);
    Ok(())
}

#[tokio::test]
async fn orphaned_blobs_are_reported_only_when_nothing_mentions_them() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let old_key = "gc-test/2026/09/29/aaaaaaaaaaaaaaaaaaaaa.rap";
    let shared_key = "2026/09/29/bbbbbbbbbbbbbbbbbbbbb.jpg";

    let mut n = node("pkg", "pkg");
    n.properties
        .insert("key".into(), PropertyValue::String(old_key.into()));
    n.properties
        .insert("thumb".into(), PropertyValue::String(shared_key.into()));
    put(&storage, &n).await?;

    // New upload: a new blob, and the thumbnail now only as a URL.
    let mut n = node("pkg", "pkg");
    n.properties.insert(
        "key".into(),
        PropertyValue::String("gc-test/2026/09/30/ccccccccccccccccccccc.rap".into()),
    );
    n.properties.insert(
        "thumb_url".into(),
        PropertyValue::String(format!("http://localhost/files/{shared_key}")),
    );
    put(&storage, &n).await?;

    let report = run_history_gc(&storage, &aggressive())?;
    assert_eq!(report.orphaned_blobs, vec![old_key.to_string()]);
    Ok(())
}

#[tokio::test]
async fn the_sweep_deletes_listed_blobs_nothing_mentions() -> Result<()> {
    use raisin_binary::{BinaryStorage, FilesystemBinaryStorage};
    use raisin_rocksdb::management::history_gc::run_gc_and_sweep_blobs;

    let (storage, tmp) = setup().await?;
    let uploads = tmp.path().join("uploads");
    let bin = FilesystemBinaryStorage::new(&uploads, None);
    let stored = |ext: &'static str, tenant: &'static str| {
        let bin = &bin;
        async move {
            bin.put_bytes(b"bytes", None, Some(ext), None, Some(tenant))
                .await
                .unwrap()
                .key
        }
    };
    let named = stored("rap", TENANT).await;
    let as_url = stored("jpg", TENANT).await;
    let orphan = stored("rap", TENANT).await;
    let other_tenant = stored("rap", "other").await;
    std::fs::write(uploads.join("notes.txt"), b"not a blob").unwrap();

    let mut n = node("pkg", "pkg");
    n.properties
        .insert("key".into(), PropertyValue::String(named.clone()));
    n.properties.insert(
        "thumb_url".into(),
        PropertyValue::String(format!("http://localhost/files/{as_url}")),
    );
    put(&storage, &n).await?;

    let exists = |key: &str| uploads.join(key).exists();
    let opts = GcOptions {
        min_age: Duration::ZERO,
        blob_min_age: Duration::ZERO,
        ..GcOptions::default()
    };

    // Too young: an upload is stored before the node that names it.
    let young = run_gc_and_sweep_blobs(storage.clone(), &bin, GcOptions::default()).await?;
    assert_eq!(young.unreferenced_blobs, 0);
    assert!(exists(&orphan));

    let dry = run_gc_and_sweep_blobs(
        storage.clone(),
        &bin,
        GcOptions {
            dry_run: true,
            ..opts.clone()
        },
    )
    .await?;
    assert_eq!(dry.blobs_listed, Some(5));
    assert_eq!(dry.unreferenced_blobs, 2);
    assert_eq!(dry.blobs_deleted, 0);
    assert!(exists(&orphan) && exists(&other_tenant));

    // A tenant-scoped run only judges that tenant's blobs.
    let scoped = run_gc_and_sweep_blobs(
        storage.clone(),
        &bin,
        GcOptions {
            tenant: Some(TENANT.into()),
            ..opts.clone()
        },
    )
    .await?;
    assert_eq!(scoped.blobs_deleted, 1);
    assert!(!exists(&orphan));
    assert!(exists(&other_tenant));

    // A stale derived-index row names no node version: it does not keep a blob.
    let stale = format!(
        "{TENANT}\0{REPO}\0{BRANCH}\0{WORKSPACE}\0prop\0resource\0{{\"key\":\"{other_tenant}\"}}\0rev\0pkg"
    );
    let cf = storage.db().cf_handle("property_index").unwrap();
    storage.db().put_cf(cf, stale.as_bytes(), b"").unwrap();

    let global = run_gc_and_sweep_blobs(storage.clone(), &bin, opts).await?;
    assert_eq!(global.blobs_deleted, 1);
    assert_eq!(global.blob_bytes_deleted, 5);
    assert!(!exists(&other_tenant));

    // Named by key, mentioned as a URL, or not blob-shaped: all kept.
    assert!(exists(&named));
    assert!(exists(&as_url));
    assert!(uploads.join("notes.txt").exists());
    Ok(())
}

#[tokio::test]
async fn retained_history_is_reported_not_deleted() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    for v in 1..=3 {
        put(&storage, &node("page", &format!("v{v}"))).await?;
    }
    // A window reaching past every write keeps the history and says so.
    let report = run_history_gc(
        &storage,
        &GcOptions {
            retention_override: Some(HistoryRetention {
                keep_days: Some(1),
                keep_revisions: None,
            }),
            min_age: Duration::ZERO,
            ..GcOptions::default()
        },
    )?;
    assert_eq!(report.versions_deleted, 0);
    assert_eq!(report.column_families["nodes"].versions_retained, 2);
    assert!(report.bytes_retained > 0);
    assert_eq!(node_versions(&storage, "page"), 3);

    let report = run_history_gc(&storage, &aggressive())?;
    assert_eq!(report.column_families["nodes"].versions_deleted, 2);
    assert_eq!(report.column_families["nodes"].versions_retained, 0);
    Ok(())
}
