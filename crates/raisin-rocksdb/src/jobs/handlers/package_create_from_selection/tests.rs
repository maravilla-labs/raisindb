//! End-to-end tests of the streaming build against a real RocksDB store and
//! a filesystem blob store: submit → run the handler → read what it wrote.

use super::archive::{content_hash, serialize_node};
use super::inventory::InventoryLine;
use super::submit::{submit_package_build, PackageBuildRequest, RecordRefWire};
use super::types::SelectedPath;
use super::PackageCreateFromSelectionHandler;
use crate::RocksDBStorage;
use raisin_binary::{
    erase, BinaryStorage, BlobMeta, BlobWriter, BoxFuture, ByteReadStream, DynBinaryStorage,
    FilesystemBinaryStorage, StorageSpace, WriteHint,
};
use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::Resource;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_storage::jobs::{JobId, JobStatus};
use raisin_storage::scope::{BranchScope, StorageScope};
use raisin_storage::{
    BranchRepository, CommitMetadata, CreateNodeOptions, NodeRepository, NodeTypeRepository,
    RegistryRepository, RepositoryManagementRepository, Storage,
};
use sha2::Digest;
use std::collections::HashMap;
use std::io::Read;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

const T: &str = "pkg-t";
const R: &str = "pkg-r";
const B: &str = "main";
const RECORD: &str = "/packages/site/b-1";

struct Fixture {
    _dir: tempfile::TempDir,
    storage: Arc<RocksDBStorage>,
    blobs: Arc<FilesystemBinaryStorage>,
    blob_dir: std::path::PathBuf,
    /// Keys of the seeded asset blobs, which a build must leave alone.
    seeded_blobs: Vec<String>,
}

fn folder_type() -> NodeType {
    NodeType {
        id: Some("raisin:Folder".to_string()),
        strict: Some(false),
        name: "raisin:Folder".to_string(),
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
        versionable: Some(true),
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
    }
}

async fn create(storage: &RocksDBStorage, ws: &str, path: &str, props: Vec<(&str, PropertyValue)>) {
    let node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        node_type: "raisin:Folder".to_string(),
        properties: props.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        ..Default::default()
    };
    storage
        .nodes()
        .create_deep_node(
            StorageScope::new(T, R, B, ws),
            path,
            node,
            "raisin:Folder",
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                operation_meta: None,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("create {ws}:{path}: {e}"));
}

fn file_resource(key: &str, size: usize, mime: &str) -> PropertyValue {
    let mut metadata = HashMap::new();
    metadata.insert("storage_key".to_string(), PropertyValue::String(key.into()));
    PropertyValue::Resource(Resource {
        uuid: "u".into(),
        name: None,
        size: Some(size as i64),
        mime_type: Some(mime.into()),
        url: Some(key.into()),
        metadata: Some(metadata),
        is_loaded: Some(true),
        is_external: Some(false),
        created_at: chrono::Utc::now().into(),
        updated_at: chrono::Utc::now().into(),
    })
}

/// A site of `pages` pages in nested sections, three assets (one spans many
/// stream chunks, one has no stored file) and the build record.
async fn fixture(pages: usize, big_asset: usize) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path().join("db")).unwrap());
    storage
        .registry()
        .register_tenant(T, HashMap::new())
        .await
        .unwrap();
    storage
        .repository_management()
        .create_repository(
            T,
            R,
            RepositoryConfig {
                default_language: "en".into(),
                supported_languages: vec!["en".into()],
                locale_fallback_chains: HashMap::new(),
                default_branch: B.into(),
                description: None,
                tags: HashMap::new(),
                localized_names: Default::default(),
            },
        )
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(T, R, B, "system", None, None, false, false)
        .await
        .unwrap();
    storage
        .node_types()
        .upsert(
            BranchScope::new(T, R, B),
            folder_type(),
            CommitMetadata::system("seed"),
        )
        .await
        .unwrap();
    for ws in ["content", "assets", "settings", "packages"] {
        let mut w = Workspace::new(ws.to_string());
        w.config.default_branch = B.to_string();
        WorkspaceService::new(storage.clone())
            .put(T, R, w)
            .await
            .unwrap();
    }

    let blob_dir = dir.path().join("blobs");
    let blobs = Arc::new(FilesystemBinaryStorage::new(&blob_dir, None));
    storage.set_blob_store(erase(blobs.clone()));

    for i in 0..pages {
        let section = i % 7;
        create(
            &storage,
            "content",
            &format!("/site/s{section}/page-{i}"),
            vec![
                ("title", PropertyValue::String(format!("Page {i}"))),
                ("body", PropertyValue::String("lorem ipsum ".repeat(20))),
            ],
        )
        .await;
    }

    let mut seeded_blobs = Vec::new();
    let small = blobs
        .put_bytes(
            b"\x89PNG small",
            Some("image/png"),
            Some("png"),
            None,
            Some(T),
        )
        .await
        .unwrap();
    let big: Vec<u8> = (0..big_asset).map(|i| (i * 31 % 251) as u8).collect();
    let big_obj = blobs
        .put_bytes(&big, Some("video/mp4"), Some("mp4"), None, Some(T))
        .await
        .unwrap();
    seeded_blobs.push(small.key.clone());
    seeded_blobs.push(big_obj.key.clone());
    create(
        &storage,
        "assets",
        "/media/logo.png",
        vec![("file", file_resource(&small.key, 10, "image/png"))],
    )
    .await;
    create(
        &storage,
        "assets",
        "/media/clip.mp4",
        vec![("file", file_resource(&big_obj.key, big.len(), "video/mp4"))],
    )
    .await;
    create(
        &storage,
        "assets",
        "/media/gone.pdf",
        vec![(
            "file",
            file_resource("pkg-t/2026/01/01/missing.pdf", 5, "application/pdf"),
        )],
    )
    .await;

    create(
        &storage,
        "settings",
        RECORD,
        vec![("status", PropertyValue::String("queued".into()))],
    )
    .await;

    Fixture {
        _dir: dir,
        storage,
        blobs,
        blob_dir,
        seeded_blobs,
    }
}

fn request(key: &str) -> PackageBuildRequest {
    PackageBuildRequest {
        name: "export-site".into(),
        version: "1.0.1".into(),
        selected_paths: vec![
            SelectedPath {
                workspace: "content".into(),
                path: "/site/*".into(),
            },
            SelectedPath {
                workspace: "assets".into(),
                path: "/*".into(),
            },
        ],
        include_node_types: true,
        record: Some(RecordRefWire {
            workspace: "settings".into(),
            path: RECORD.into(),
        }),
        register_package: false,
        idempotency_key: Some(key.into()),
        title: Some("Site".into()),
        description: None,
        author: None,
    }
}

async fn run(
    f: &Fixture,
    handler: &PackageCreateFromSelectionHandler,
    job_id: &str,
) -> raisin_error::Result<Option<serde_json::Value>> {
    let id = JobId(job_id.to_string());
    let job = f.storage.job_registry().get_job_info(&id).await.unwrap();
    let ctx = f.storage.job_data_store().get(T, &id).unwrap().unwrap();
    f.storage.job_registry().mark_running(&id).await.unwrap();
    f.storage.job_registry().mark_executing(&id).await.unwrap();
    handler.handle(&job, &ctx).await
}

fn handler(f: &Fixture, floor: Option<u64>) -> PackageCreateFromSelectionHandler {
    PackageCreateFromSelectionHandler::new(f.storage.clone(), f.storage.job_registry().clone())
        .with_space_floor(floor.or(Some(0)))
}

async fn record(f: &Fixture) -> HashMap<String, PropertyValue> {
    f.storage
        .nodes()
        .get_by_path(StorageScope::new(T, R, B, "settings"), RECORD, None)
        .await
        .unwrap()
        .unwrap()
        .properties
}

fn string(p: &HashMap<String, PropertyValue>, k: &str) -> Option<String> {
    match p.get(k) {
        Some(PropertyValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn resource_key(p: &HashMap<String, PropertyValue>, k: &str) -> (String, String) {
    let Some(PropertyValue::Resource(r)) = p.get(k) else {
        panic!("{k} is not a Resource: {:?}", p.get(k));
    };
    let meta = r.metadata.as_ref().unwrap();
    let get = |m: &str| match meta.get(m) {
        Some(PropertyValue::String(s)) => s.clone(),
        other => panic!("{k}.{m}: {other:?}"),
    };
    (get("storage_key"), get("sha256"))
}

/// Every blob in the store, partials included, minus the seeded ones.
async fn leftover_blobs(f: &Fixture) -> Vec<String> {
    let mut out: Vec<String> = f
        .blobs
        .list_blobs()
        .await
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|b| b.key)
        .filter(|k| !f.seeded_blobs.contains(k))
        .collect();
    if let Ok(d) = std::fs::read_dir(f.blob_dir.join(raisin_binary::PARTIAL_DIR)) {
        out.extend(d.map(|e| format!(".partial/{:?}", e.unwrap().file_name())));
    }
    out
}

fn requester() -> AuthContext {
    AuthContext::system()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_build_streams_an_archive_whose_inventory_verifies() {
    let f = fixture(300, 3 * 1024 * 1024 + 17).await;
    let submitted = submit_package_build(&f.storage, T, R, B, &request("k1"), &requester())
        .await
        .unwrap();
    assert!(!submitted.existing);

    // E13: the same key returns the same job.
    let again = submit_package_build(&f.storage, T, R, B, &request("k1"), &requester())
        .await
        .unwrap();
    assert_eq!(again.job_id, submitted.job_id);
    assert!(again.existing);

    let h = handler(&f, None);
    let result = run(&f, &h, &submitted.job_id).await.unwrap().unwrap();

    // --- the record ---------------------------------------------------------
    let rec = record(&f).await;
    assert_eq!(string(&rec, "status").as_deref(), Some("completed"));
    assert_eq!(string(&rec, "phase").as_deref(), Some("done"));
    assert_eq!(
        string(&rec, "job_id").as_deref(),
        Some(submitted.job_id.as_str())
    );
    let (archive_key, archive_sha) = resource_key(&rec, "artifact");
    let (manifest_key, manifest_sha) = resource_key(&rec, "manifest");
    let (inventory_key, inventory_sha) = resource_key(&rec, "inventory");
    let _ = resource_key(&rec, "log");
    assert_eq!(
        string(&rec, "sha256").as_deref(),
        Some(archive_sha.as_str())
    );
    let Some(PropertyValue::Object(est)) = rec.get("estimate") else {
        panic!("no estimate");
    };
    assert!(matches!(est.get("items"), Some(PropertyValue::Integer(n)) if *n > 0));
    let Some(PropertyValue::Array(log_tail)) = rec.get("log_tail") else {
        panic!("no log_tail");
    };
    assert!(!log_tail.is_empty());
    // A missing stored file is a warning, not a failure.
    let Some(PropertyValue::Object(counters)) = rec.get("counters") else {
        panic!("no counters");
    };
    assert_eq!(counters.get("warnings"), Some(&PropertyValue::Integer(1)));

    // --- size and checksum, from the stream (E23) ---------------------------
    let archive = f.blobs.get(&archive_key).await.unwrap();
    assert_eq!(hex::encode(sha2::Sha256::digest(&archive)), archive_sha);
    assert_eq!(
        rec.get("size_bytes"),
        Some(&PropertyValue::Integer(archive.len() as i64))
    );
    assert_eq!(result["sha256"], archive_sha);
    assert_eq!(result["size_bytes"], archive.len() as u64);

    // --- the archive reads back, every inventory hash verifies (E8) --------
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive.to_vec())).unwrap();
    let mut inventory_text = String::new();
    zip.by_name("inventory.jsonl")
        .unwrap()
        .read_to_string(&mut inventory_text)
        .unwrap();
    let sidecar = f.blobs.get(&inventory_key).await.unwrap();
    assert_eq!(
        sidecar.as_ref(),
        inventory_text.as_bytes(),
        "sidecar == entry"
    );
    assert_eq!(hex::encode(sha2::Sha256::digest(&sidecar)), inventory_sha);
    let lines: Vec<InventoryLine> = inventory_text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // 300 pages + /site + 7 sections, 3 assets + /media. (The fixture keeps
    // its node type in the registry only, with no `nodetypes` workspace node
    // to archive.)
    assert_eq!(lines.len(), 300 + 1 + 7 + 3 + 1);
    let keys: Vec<(String, String)> = lines
        .iter()
        .map(|l| (l.ws.clone(), l.path.clone()))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort_by(|a, b| {
        a.0.as_bytes()
            .cmp(b.0.as_bytes())
            .then(a.1.as_bytes().cmp(b.1.as_bytes()))
    });
    assert_eq!(keys, sorted, "inventory sorted by (ws, path)");

    for line in &lines {
        let node = if line.ws == "nodetypes" {
            continue;
        } else {
            f.storage
                .nodes()
                .get_by_path(StorageScope::new(T, R, B, &line.ws), &line.path, None)
                .await
                .unwrap()
                .unwrap()
        };
        assert_eq!(line.id, node.id);
        assert_eq!(line.hash, content_hash(serialize_node(&node).unwrap().1));
        if let Some(bin) = &line.bin {
            let name = format!("content/{}{}", line.ws, line.path);
            let mut bytes = Vec::new();
            zip.by_name(&name).unwrap().read_to_end(&mut bytes).unwrap();
            assert_eq!(bin.bytes, bytes.len() as u64);
            assert_eq!(bin.sha256, hex::encode(sha2::Sha256::digest(&bytes)));
        }
    }
    let clip = lines.iter().find(|l| l.path == "/media/clip.mp4").unwrap();
    assert_eq!(clip.bin.as_ref().unwrap().bytes, 3 * 1024 * 1024 + 17);
    assert!(lines
        .iter()
        .find(|l| l.path == "/media/gone.pdf")
        .unwrap()
        .bin
        .is_none());

    // --- the manifest sidecar equals the entry (E24) ------------------------
    let mut manifest_entry = String::new();
    zip.by_name("manifest.yaml")
        .unwrap()
        .read_to_string(&mut manifest_entry)
        .unwrap();
    let manifest_blob = f.blobs.get(&manifest_key).await.unwrap();
    assert_eq!(manifest_blob.as_ref(), manifest_entry.as_bytes());
    assert_eq!(
        hex::encode(sha2::Sha256::digest(&manifest_blob)),
        manifest_sha
    );
    assert!(manifest_entry.contains("content:/site/*"));
    assert!(manifest_entry.contains("assets:/*"));
    assert!(manifest_entry.contains("title: Site"));

    // --- E4: nothing under packages: ----------------------------------------
    assert!(f
        .storage
        .nodes()
        .get_by_path(StorageScope::new(T, R, B, "packages"), "/export-site", None)
        .await
        .unwrap()
        .is_none());

    // Only the four committed blobs (archive, inventory, manifest, log).
    let mut left = leftover_blobs(&f).await;
    left.sort();
    let mut expected = vec![
        archive_key,
        inventory_key,
        manifest_key,
        resource_key(&rec, "log").0,
    ];
    expected.sort();
    assert_eq!(left, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_requested_on_the_record_leaves_no_object() {
    let f = fixture(50, 1024).await;
    let submitted = submit_package_build(&f.storage, T, R, B, &request("k2"), &requester())
        .await
        .unwrap();
    // The caller asks for a cancel before the job gets going.
    let tx_storage = f.storage.clone();
    let mut node = tx_storage
        .nodes()
        .get_by_path(StorageScope::new(T, R, B, "settings"), RECORD, None)
        .await
        .unwrap()
        .unwrap();
    node.properties.insert(
        "status".into(),
        PropertyValue::String("cancel_requested".into()),
    );
    tx_storage
        .nodes()
        .update(
            StorageScope::new(T, R, B, "settings"),
            node,
            raisin_storage::UpdateNodeOptions {
                validate_schema: false,
                allow_type_change: false,
                operation_meta: None,
            },
        )
        .await
        .unwrap();

    let started = std::time::Instant::now();
    let h = handler(&f, None);
    assert!(run(&f, &h, &submitted.job_id).await.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    let rec = record(&f).await;
    assert_eq!(string(&rec, "status").as_deref(), Some("cancelled"));
    assert!(leftover_blobs(&f).await.is_empty());
    assert_eq!(
        f.storage
            .job_registry()
            .get_status(&JobId(submitted.job_id.clone()))
            .await
            .unwrap(),
        JobStatus::Cancelled
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_preflight_refuses_before_writing_anything() {
    let f = fixture(20, 1024).await;
    let submitted = submit_package_build(&f.storage, T, R, B, &request("k3"), &requester())
        .await
        .unwrap();
    // A floor larger than any disk.
    let h = handler(&f, Some(u64::MAX / 4));
    let err = run(&f, &h, &submitted.job_id).await.unwrap_err();
    assert!(err.to_string().contains("insufficient_storage"), "{err}");
    let rec = record(&f).await;
    assert_eq!(string(&rec, "status").as_deref(), Some("failed"));
    let Some(PropertyValue::Object(e)) = rec.get("error") else {
        panic!("no error object");
    };
    assert_eq!(
        e.get("code"),
        Some(&PropertyValue::String("insufficient_storage".into()))
    );
    assert!(leftover_blobs(&f).await.is_empty());
}

/// A store whose free space collapses after the first query, to simulate a
/// volume filling up while the archive is being written.
struct Shrinking {
    inner: Arc<dyn DynBinaryStorage>,
    queries: AtomicUsize,
    after_first: AtomicU64,
}

impl DynBinaryStorage for Shrinking {
    fn begin_write(&self, hint: WriteHint) -> BoxFuture<'_, anyhow::Result<Box<dyn BlobWriter>>> {
        self.inner.begin_write(hint)
    }
    fn get_stream<'a>(
        &'a self,
        key: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<(u64, ByteReadStream)>> {
        self.inner.get_stream(key)
    }
    fn get_range<'a>(
        &'a self,
        key: &'a str,
        range: Range<u64>,
    ) -> BoxFuture<'a, anyhow::Result<ByteReadStream>> {
        self.inner.get_range(key, range)
    }
    fn head<'a>(&'a self, key: &'a str) -> BoxFuture<'a, anyhow::Result<Option<BlobMeta>>> {
        self.inner.head(key)
    }
    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        self.inner.delete(key)
    }
    fn available_space(&self) -> BoxFuture<'_, anyhow::Result<Option<StorageSpace>>> {
        let n = self.queries.fetch_add(1, Ordering::SeqCst);
        let available = if n == 0 {
            u64::MAX / 2
        } else {
            self.after_first.load(Ordering::SeqCst)
        };
        Box::pin(async move {
            Ok(Some(StorageSpace {
                available,
                total: u64::MAX,
            }))
        })
    }
    fn discard_partial<'a>(&'a self, id: &'a str) -> BoxFuture<'a, anyhow::Result<bool>> {
        self.inner.discard_partial(id)
    }
    fn sweep_partials(&self, d: std::time::Duration) -> BoxFuture<'_, anyhow::Result<usize>> {
        self.inner.sweep_partials(d)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_out_of_space_mid_build_fails_and_deletes_the_partial() {
    // One asset larger than the re-check interval, so the check fires while
    // its bytes are streaming.
    let f = fixture(5, 70 * 1024 * 1024).await;
    f.storage.set_blob_store(Arc::new(Shrinking {
        inner: erase(f.blobs.clone()),
        queries: AtomicUsize::new(0),
        after_first: AtomicU64::new(0),
    }));
    let submitted = submit_package_build(&f.storage, T, R, B, &request("k4"), &requester())
        .await
        .unwrap();
    let h = handler(&f, Some(1));
    let err = run(&f, &h, &submitted.job_id).await.unwrap_err();
    assert!(err.to_string().contains("insufficient_storage"), "{err}");
    let rec = record(&f).await;
    let Some(PropertyValue::Object(e)) = rec.get("error") else {
        panic!("no error object");
    };
    assert_eq!(
        e.get("code"),
        Some(&PropertyValue::String("insufficient_storage".into()))
    );
    assert!(leftover_blobs(&f).await.is_empty(), "partial removed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_installed_package_name_is_refused() {
    let f = fixture(1, 16).await;
    create(
        &f.storage,
        "packages",
        "/export-site",
        vec![("installed", PropertyValue::Boolean(true))],
    )
    .await;
    let err = submit_package_build(&f.storage, T, R, B, &request("k5"), &requester())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("name_invalid"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_function_binding_submits_reads_and_cancels() {
    let f = fixture(10, 16).await;
    crate::package_build::install(&f.storage);
    let scope = || crate::package_build::CallScope {
        tenant: T,
        repo: R,
        branch: B,
    };
    let req = serde_json::to_value(request("k6")).unwrap();
    let out = crate::package_build::call(scope(), &requester(), "createFromSelection", req.clone())
        .await
        .unwrap();
    let job_id = out["job_id"].as_str().unwrap().to_string();
    assert_eq!(out["existing"], false);
    // Same key, same job (E13).
    let again = crate::package_build::call(scope(), &requester(), "createFromSelection", req)
        .await
        .unwrap();
    assert_eq!(again["job_id"], job_id.as_str());
    assert_eq!(again["existing"], true);

    let status = crate::package_build::call(
        scope(),
        &requester(),
        "status",
        serde_json::json!({ "jobId": job_id }),
    )
    .await
    .unwrap();
    assert_eq!(status["status"], "scheduled");

    // Never started: cancel takes the hard path and the job ends cancelled.
    let cancelled = crate::package_build::call(
        scope(),
        &requester(),
        "cancel",
        serde_json::json!({ "jobId": job_id }),
    )
    .await
    .unwrap();
    assert_eq!(cancelled["cancelled"], true);
    assert_eq!(
        f.storage
            .job_registry()
            .get_status(&JobId(job_id.clone()))
            .await
            .unwrap(),
        JobStatus::Cancelled
    );

    // Unknown job, unknown method, anonymous caller.
    let unknown = crate::package_build::call(
        scope(),
        &requester(),
        "status",
        serde_json::json!({ "jobId": "nope" }),
    )
    .await
    .unwrap();
    assert!(unknown.is_null());
    assert!(
        crate::package_build::call(scope(), &requester(), "explode", serde_json::json!({}))
            .await
            .is_err()
    );
    let anon = AuthContext::anonymous();
    assert!(crate::package_build::call(
        scope(),
        &anon,
        "status",
        serde_json::json!({ "jobId": job_id })
    )
    .await
    .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_build_interrupted_by_a_restart_reports_lost_and_discards_its_partial() {
    use tokio::io::AsyncWriteExt;
    let f = fixture(5, 16).await;
    let submitted = submit_package_build(&f.storage, T, R, B, &request("k7"), &requester())
        .await
        .unwrap();
    // The first attempt got as far as writing: the record says running for
    // this job, and its partial archive is on disk.
    let scope = StorageScope::new(T, R, B, "settings");
    let mut node = f
        .storage
        .nodes()
        .get_by_path(scope, RECORD, None)
        .await
        .unwrap()
        .unwrap();
    node.properties
        .insert("status".into(), PropertyValue::String("running".into()));
    node.properties.insert(
        "job_id".into(),
        PropertyValue::String(submitted.job_id.clone()),
    );
    f.storage
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
    let mut w = BinaryStorage::begin_write(
        &*f.blobs,
        WriteHint::new().partial_id(format!("pkgbuild-{}", submitted.job_id)),
    )
    .await
    .unwrap();
    w.write_all(b"half an archive").await.unwrap();
    std::mem::forget(w);
    assert!(!leftover_blobs(&f).await.is_empty());

    // The job system re-queued it after the restart.
    let h = handler(&f, None);
    let err = run(&f, &h, &submitted.job_id).await.unwrap_err();
    assert!(err.to_string().contains("lost"), "{err}");
    let rec = record(&f).await;
    assert_eq!(string(&rec, "status").as_deref(), Some("failed"));
    let Some(PropertyValue::Object(e)) = rec.get("error") else {
        panic!("no error object");
    };
    assert_eq!(e.get("code"), Some(&PropertyValue::String("lost".into())));
    assert!(leftover_blobs(&f).await.is_empty(), "partial discarded");
}

/// Peak memory of a build, for a scaled flat-memory measurement (E21).
///
/// Run it twice in separate processes with different sizes and compare the
/// peak RSS the OS reports (`/usr/bin/time -l` on macOS, `-v` on Linux):
///
/// ```sh
/// PKG_BENCH_MB=128 /usr/bin/time -l <test-binary> --ignored rss_bench --nocapture
/// PKG_BENCH_MB=640 /usr/bin/time -l <test-binary> --ignored rss_bench --nocapture
/// ```
///
/// The asset is written to the blob store in 1 MiB chunks, so the fixture
/// itself holds no more than one chunk either.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "measurement; writes PKG_BENCH_MB twice to a temp dir"]
async fn rss_bench() {
    use tokio::io::AsyncWriteExt;
    let mb: usize = std::env::var("PKG_BENCH_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64);
    let f = fixture(2_000, 16).await;
    let mut w = BinaryStorage::begin_write(&*f.blobs, WriteHint::new().ext("mp4"))
        .await
        .unwrap();
    let chunk: Vec<u8> = (0..1024 * 1024).map(|i| (i % 251) as u8).collect();
    for _ in 0..mb {
        w.write_all(&chunk).await.unwrap();
    }
    let big = w.commit().await.unwrap();
    create(
        &f.storage,
        "assets",
        "/media/bench.mp4",
        vec![(
            "file",
            file_resource(&big.key, mb * 1024 * 1024, "video/mp4"),
        )],
    )
    .await;
    drop(chunk);

    let submitted = submit_package_build(&f.storage, T, R, B, &request("bench"), &requester())
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let result = run(&f, &handler(&f, None), &submitted.job_id)
        .await
        .unwrap()
        .unwrap();
    println!(
        "rss_bench: {} MiB asset → archive {} bytes in {:?}",
        mb,
        result["size_bytes"],
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_node_pointing_at_another_tenants_blob_is_not_packed() {
    let f = fixture(3, 16).await;
    // Another tenant's blob, and a node in THIS tenant whose Resource names
    // its key — node properties are writable content.
    let foreign = f
        .blobs
        .put_bytes(
            b"other tenant's secret",
            Some("application/pdf"),
            Some("pdf"),
            None,
            Some("other-tenant"),
        )
        .await
        .unwrap();
    assert!(foreign.key.starts_with("other-tenant/"));
    create(
        &f.storage,
        "assets",
        "/media/stolen.pdf",
        vec![("file", file_resource(&foreign.key, 21, "application/pdf"))],
    )
    .await;

    let submitted = submit_package_build(&f.storage, T, R, B, &request("k8"), &requester())
        .await
        .unwrap();
    run(&f, &handler(&f, None), &submitted.job_id)
        .await
        .unwrap();

    let rec = record(&f).await;
    let (archive_key, _) = resource_key(&rec, "artifact");
    // Artifacts are minted under this tenant and repository.
    assert!(
        raisin_core::is_artifact_key_of(T, R, &archive_key),
        "{archive_key}"
    );
    let archive = f.blobs.get(&archive_key).await.unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive.to_vec())).unwrap();
    assert!(zip.by_name("content/assets/media/stolen.pdf").is_err());
    let all: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(!all.iter().any(|n| n.ends_with("/stolen.pdf")));
    let mut inv = String::new();
    zip.by_name("inventory.jsonl")
        .unwrap()
        .read_to_string(&mut inv)
        .unwrap();
    let line: InventoryLine = inv
        .lines()
        .map(|l| serde_json::from_str::<InventoryLine>(l).unwrap())
        .find(|l| l.path == "/media/stolen.pdf")
        .unwrap();
    assert!(line.bin.is_none());
    assert_eq!(line.bin_skipped.as_deref(), Some("foreign_key"));
    // The genuinely missing one is reported as such, the foreign one too.
    let Some(PropertyValue::Object(counters)) = rec.get("counters") else {
        panic!("no counters");
    };
    assert_eq!(counters.get("warnings"), Some(&PropertyValue::Integer(2)));
}
