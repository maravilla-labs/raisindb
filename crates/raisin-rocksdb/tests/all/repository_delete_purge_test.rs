//! Deleting a repository deletes ALL of its data.
//!
//! `delete_repository` used to remove the registry entry and nothing else, so
//! a repository recreated under the same id came back with the old one's
//! nodes, translations, embeddings and indexes (seen on 2026-09-29: a
//! recreated `studio` repository held inquiry topics from before the delete).
//!
//! The test fills a repository through the real writers (nodes, node types,
//! workspaces, branches, revisions, embeddings, jobs, job queues) and puts a key
//! of each repository-scoped layout into every other column family, deletes it,
//! recreates the same id and scans EVERY column family for anything left.
//! Two neighbours in the same tenant — `keep`, and `doomed2`, whose id has the
//! deleted one as a prefix — must come through byte for byte.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use raisin_ai::config::{EmbedderId, EmbeddingKind};
use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_embeddings::config::EmbeddingProvider;
use raisin_embeddings::{EmbeddingData, EmbeddingJob, EmbeddingJobStore, EmbeddingStorage};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::{
    cf, fractional_index, PersistedJobEntry, RocksDBConfig, RocksDBEmbeddingJobStore,
    RocksDBEmbeddingStorage, RocksDBStorage, RocksDbJobStore,
};
use raisin_storage::jobs::{JobContext, JobId, JobStatus, JobType};
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, CommitMetadata, FullTextIndexJob, FullTextJobStore, JobKind,
    NodeTypeRepository, RegistryRepository, RepositoryManagementRepository, Storage,
    WorkspaceRepository,
};
use tempfile::TempDir;

const TENANT: &str = "purge-test";
const DOOMED: &str = "doomed";
const NEIGHBOURS: [&str; 2] = ["keep", "doomed2"];
const BRANCH: &str = "main";
const WS: &str = "default";

/// Column families that hold no repository data (tenant-wide or cluster
/// bookkeeping). Everything else gets a repository-scoped key below.
const NOT_REPO_SCOPED: &[&str] = &[
    cf::APPLIED_OPS,
    cf::IDENTITIES,
    cf::IDENTITY_EMAIL_INDEX,
    cf::SESSIONS,
    cf::ADMIN_USERS,
    cf::TENANT_AI_CONFIG,
    cf::TENANT_AUTH_CONFIG,
    cf::TENANT_EMBEDDING_CONFIG,
    cf::QUERY_EMBEDDINGS,
];

/// Column families whose keys do not start with `{tenant}\0{repo}\0` and are
/// seeded through their own writers.
const OWN_LAYOUT: &[&str] = &[
    cf::REGISTRY,
    cf::JOB_DATA,
    cf::JOB_METADATA,
    cf::FULLTEXT_JOBS,
    cf::EMBEDDING_JOBS,
    cf::SYSTEM_UPDATE_HASHES,
    cf::INDEX_STATUS,
];

fn repo_config() -> RepositoryConfig {
    RepositoryConfig {
        default_language: "de".to_string(),
        supported_languages: vec!["de".to_string(), "en".to_string()],
        locale_fallback_chains: HashMap::new(),
        default_branch: BRANCH.to_string(),
        description: None,
        tags: HashMap::new(),
        localized_names: Default::default(),
    }
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

fn node(name: &str) -> Node {
    let mut properties = HashMap::new();
    properties.insert("title".to_string(), PropertyValue::String(name.to_string()));
    Node {
        id: format!("{name}-id"),
        name: name.to_string(),
        path: format!("/{name}"),
        node_type: "raisin:Folder".to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: Some("user".to_string()),
        created_by: Some("user".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WS.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

fn embedding(source_id: &str) -> EmbeddingData {
    EmbeddingData {
        vector: vec![0.5; 4],
        embedder_id: EmbedderId::new("ollama", "test-model", 4),
        embedding_kind: EmbeddingKind::Text,
        source_id: source_id.to_string(),
        chunk_index: 0,
        total_chunks: 1,
        chunk_content: None,
        generated_at: chrono::Utc::now(),
        text_hash: 1,
        spec_hash: Some(1),
        chunk_span: None,
        model: "test-model".to_string(),
        provider: EmbeddingProvider::Ollama,
    }
}

fn job_context(repo: &str) -> JobContext {
    JobContext {
        tenant_id: TENANT.to_string(),
        repo_id: repo.to_string(),
        branch: BRANCH.to_string(),
        workspace_id: WS.to_string(),
        revision: HLC::new(1_000, 0),
        metadata: HashMap::new(),
    }
}

/// Create `repo` and write data into every column family it can own.
/// Returns the ids of its jobs.
async fn populate(storage: &Arc<RocksDBStorage>, repo: &str) -> Result<Vec<JobId>> {
    storage
        .repository_management()
        .create_repository(TENANT, repo, repo_config())
        .await?;
    storage
        .branches()
        .create_branch(TENANT, repo, BRANCH, "system", None, None, false, false)
        .await?;
    storage
        .node_types()
        .upsert(
            BranchScope::new(TENANT, repo, BRANCH),
            folder_type(),
            CommitMetadata::system("seed folder type"),
        )
        .await?;
    let mut workspace = Workspace::new(WS.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, repo, workspace)
        .await?;

    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, repo)?;
    tx.set_branch(BRANCH)?;
    tx.set_actor("test")?;
    tx.set_message("seed")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    for name in ["alpha", "beta"] {
        tx.add_node(WS, &node(name)).await?;
    }
    tx.commit().await?;

    RocksDBEmbeddingStorage::new(storage.db().clone()).store_embedding(
        TENANT,
        repo,
        BRANCH,
        WS,
        "alpha-id",
        &HLC::new(1_000, 0),
        &embedding("alpha-id"),
    )?;

    // A job the live registry knows about, and a finished one that exists
    // only in the job column families (metadata, context, history).
    let registry = storage.job_registry();
    let live = JobId::new();
    storage.job_data_store().put(&live, &job_context(repo))?;
    registry
        .register_job_with_id(
            live.clone(),
            JobType::EmbeddingGenerate {
                node_id: "alpha-id".to_string(),
            },
            TENANT.to_string(),
            None,
            None,
            None,
        )
        .await?;
    let finished = JobId::new();
    storage.job_metadata_store().put_with_context(
        &finished,
        &PersistedJobEntry {
            id: finished.as_str().to_string(),
            job_type: JobType::EmbeddingGenerate {
                node_id: "beta-id".to_string(),
            },
            status: JobStatus::Completed,
            tenant: TENANT.to_string(),
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            error: None,
            progress: None,
            result: None,
            retry_count: 0,
            max_retries: 3,
            last_heartbeat: None,
            timeout_seconds: 60,
            next_retry_at: None,
            executing_since: None,
        },
        &job_context(repo),
    )?;

    RocksDbJobStore::new(storage.db().clone()).enqueue(&FullTextIndexJob {
        job_id: format!("ft-{repo}"),
        kind: JobKind::AddNode,
        tenant_id: TENANT.to_string(),
        repo_id: repo.to_string(),
        workspace_id: WS.to_string(),
        branch: BRANCH.to_string(),
        revision: HLC::new(1_000, 0),
        node_id: Some("alpha-id".to_string()),
        source_branch: None,
        default_language: "de".to_string(),
        supported_languages: vec!["de".to_string()],
        properties_to_index: None,
    })?;
    RocksDBEmbeddingJobStore::new(storage.db().clone()).enqueue(&EmbeddingJob::add_node(
        TENANT.to_string(),
        repo.to_string(),
        BRANCH.to_string(),
        WS.to_string(),
        "alpha-id".to_string(),
        HLC::new(1_000, 0),
    ))?;

    // One key of the repository's layout in every other column family.
    let db = storage.db();
    for name in raisin_rocksdb::all_column_family_names() {
        if NOT_REPO_SCOPED.contains(&name) || OWN_LAYOUT.contains(&name) {
            continue;
        }
        let handle = db.cf_handle(name).unwrap();
        db.put_cf(handle, format!("{TENANT}\0{repo}\0seed\0{name}"), b"x")
            .unwrap();
        // Keys that end at the repository id exist too (PROCESSING_RULES).
        db.put_cf(handle, format!("{TENANT}\0{repo}"), b"x")
            .unwrap();
    }
    let status = db.cf_handle(cf::INDEX_STATUS).unwrap();
    for kind in ["prop_index", "compound_index", "spatial_index"] {
        db.put_cf(
            status,
            format!("{kind}\0{TENANT}\0{repo}\0{BRANCH}\0{WS}"),
            b"x",
        )
        .unwrap();
    }
    db.put_cf(status, format!("{TENANT}\0{repo}\0{BRANCH}\0x"), b"x")
        .unwrap();
    let hashes = db.cf_handle(cf::SYSTEM_UPDATE_HASHES).unwrap();
    db.put_cf(
        hashes,
        format!("{TENANT}:{repo}:nodetype:raisin:Folder"),
        b"x",
    )
    .unwrap();

    Ok(vec![live, finished])
}

/// Every key in the database that belongs to `repo`, per column family,
/// found by looking at the keys and values themselves — not through the
/// purge's own classification.
fn repo_keys(storage: &RocksDBStorage, repo: &str) -> BTreeMap<String, Vec<(Vec<u8>, Vec<u8>)>> {
    let db = storage.db();
    let mut found = BTreeMap::new();
    for name in raisin_rocksdb::all_column_family_names() {
        let handle = db.cf_handle(name).unwrap();
        let mut hits = Vec::new();
        for item in db.iterator_cf(handle, rocksdb::IteratorMode::Start) {
            let (key, value) = item.unwrap();
            let segments: Vec<&[u8]> = key.split(|b| *b == 0).take(3).collect();
            let seg = |i: usize| segments.get(i).copied();
            let by_key = (seg(0) == Some(TENANT.as_bytes()) && seg(1) == Some(repo.as_bytes()))
                || (seg(1) == Some(TENANT.as_bytes()) && seg(2) == Some(repo.as_bytes()))
                || (seg(0) == Some(TENANT.as_bytes())
                    && seg(1) == Some(b"repos".as_slice())
                    && seg(2) == Some(repo.as_bytes()))
                || key.starts_with(format!("{TENANT}:{repo}:").as_bytes());
            let by_value = match name {
                n if n == cf::JOB_DATA => {
                    rmp_serde::from_slice::<JobContext>(&value).is_ok_and(|c| c.repo_id == repo)
                }
                n if n == cf::FULLTEXT_JOBS => rmp_serde::from_slice::<FullTextIndexJob>(&value)
                    .is_ok_and(|j| j.repo_id == repo),
                n if n == cf::EMBEDDING_JOBS => {
                    rmp_serde::from_slice::<EmbeddingJob>(&value).is_ok_and(|j| j.repo_id == repo)
                }
                _ => false,
            };
            if by_key || by_value {
                hits.push((key.to_vec(), value.to_vec()));
            }
        }
        if !hits.is_empty() {
            found.insert(name.to_string(), hits);
        }
    }
    found
}

#[tokio::test]
async fn delete_removes_every_key_and_a_recreated_repository_starts_empty() -> Result<()> {
    let dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;

    let doomed_jobs = populate(&storage, DOOMED).await?;
    let mut neighbour_jobs = Vec::new();
    for repo in NEIGHBOURS {
        neighbour_jobs.extend(populate(&storage, repo).await?);
    }

    // Every repository-owning column family really holds data of the doomed
    // repository, so "nothing left" below means something.
    let before = repo_keys(&storage, DOOMED);
    for name in raisin_rocksdb::all_column_family_names() {
        if NOT_REPO_SCOPED.contains(&name) || name == cf::JOB_METADATA {
            continue; // JOB_METADATA keys carry no repository; checked by id below.
        }
        assert!(
            before.contains_key(name),
            "the test seeded nothing into {name}"
        );
    }
    let neighbours_before: Vec<_> = NEIGHBOURS.iter().map(|r| repo_keys(&storage, r)).collect();

    assert!(
        storage
            .repository_management()
            .delete_repository(TENANT, DOOMED)
            .await?
    );

    let left = repo_keys(&storage, DOOMED);
    assert!(
        left.is_empty(),
        "keys of the deleted repository survived: {left:#?}"
    );
    for id in &doomed_jobs {
        assert!(storage.job_data_store().get(TENANT, id)?.is_none());
        assert!(storage.job_metadata_store().get(TENANT, id)?.is_none());
        assert!(storage.job_registry().get_job_info(id).await.is_err());
    }

    // Recreated under the same id: only the new registry entry.
    storage
        .repository_management()
        .create_repository(TENANT, DOOMED, repo_config())
        .await?;
    let recreated = repo_keys(&storage, DOOMED);
    assert_eq!(
        recreated.keys().collect::<Vec<_>>(),
        vec![cf::REGISTRY],
        "a recreated repository holds only its registry entry: {recreated:#?}"
    );
    assert!(storage
        .branches()
        .list_branches(TENANT, DOOMED)
        .await?
        .is_empty());
    assert!(storage
        .workspaces()
        .list(raisin_storage::scope::RepoScope::new(TENANT, DOOMED))
        .await?
        .is_empty());

    // The neighbours, one of them a prefix match, are untouched.
    for (repo, before) in NEIGHBOURS.iter().zip(neighbours_before) {
        assert_eq!(
            repo_keys(&storage, repo),
            before,
            "{repo} was changed by the delete"
        );
    }
    for id in &neighbour_jobs {
        assert!(storage.job_data_store().get(TENANT, id)?.is_some());
        assert!(
            storage.job_metadata_store().get(TENANT, id)?.is_some()
                || storage.job_registry().get_job_info(id).await.is_ok()
        );
    }
    Ok(())
}

fn replicating_storage(dir: &TempDir, node_id: &str) -> Arc<RocksDBStorage> {
    let mut config = RocksDBConfig::default();
    config.path = dir.path().to_path_buf();
    config.replication_enabled = true;
    config.cluster_node_id = Some(node_id.to_string());
    Arc::new(RocksDBStorage::with_config(config).unwrap())
}

/// In a cluster the delete travels as a `DeleteRepository` operation, and a
/// peer applying it removes its own copy — before, the operation type existed
/// but was never captured or applied, so peers kept the repository and could
/// replicate it back.
#[tokio::test]
async fn a_replicated_delete_purges_the_peer_too() -> Result<()> {
    let (dir_a, dir_b) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let node_a = replicating_storage(&dir_a, "node-a");
    let node_b = replicating_storage(&dir_b, "node-b");
    for storage in [&node_a, &node_b] {
        storage
            .registry()
            .register_tenant(TENANT, HashMap::new())
            .await?;
        populate(storage, DOOMED).await?;
        populate(storage, "keep").await?;
    }

    assert!(
        node_a
            .repository_management()
            .delete_repository(TENANT, DOOMED)
            .await?
    );

    // The delete is in node A's operation log, and is all that is left of
    // the repository there: the log is how peers learn of it.
    let oplog = raisin_rocksdb::OpLogRepository::new(node_a.db().clone());
    let delete_op = oplog
        .get_all_operations(TENANT, DOOMED)?
        .into_values()
        .flatten()
        .find(|op| {
            matches!(
                op.op_type,
                raisin_replication::OpType::DeleteRepository { .. }
            )
        })
        .expect("the delete is captured for replication");
    let left_on_a = repo_keys(&node_a, DOOMED);
    assert_eq!(
        left_on_a.keys().collect::<Vec<_>>(),
        vec![cf::OPERATION_LOG],
        "{left_on_a:#?}"
    );

    let keep_before = repo_keys(&node_b, "keep");
    raisin_rocksdb::replication::OperationApplicator::new(
        node_b.db().clone(),
        node_b.event_bus(),
        Arc::new(node_b.branches_impl().clone()),
    )
    .apply_operation(&delete_op)
    .await?;

    let left_on_b = repo_keys(&node_b, DOOMED);
    assert!(
        left_on_b.is_empty(),
        "the peer kept data of the deleted repository: {left_on_b:#?}"
    );
    assert!(
        !node_b
            .repository_management()
            .repository_exists(TENANT, DOOMED)
            .await?
    );
    assert_eq!(
        repo_keys(&node_b, "keep"),
        keep_before,
        "the peer's other repository changed"
    );
    Ok(())
}
