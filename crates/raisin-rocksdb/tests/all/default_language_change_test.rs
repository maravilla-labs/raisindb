//! Changing a repository's default language after creation.
//!
//! Base content is stored in the default language and full-text indexed under
//! it. Changing the default must (a) refuse while overlays in the new language
//! exist, (b) move base content to the new language's full-text documents via
//! a rebuild of every branch, and (c) reach replication peers, which rebuild
//! their own indexes.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::translations::{JsonPointer, LocaleCode, LocaleOverlay, TranslationMeta};
use raisin_models::workspace::Workspace;
use raisin_replication::{OpType, Operation};
use raisin_rocksdb::management::default_language::{
    count_translation_overlays, enqueue_fulltext_rebuild_all_branches,
    DefaultLanguageReindexHandler, META_REINDEX_REASON, REINDEX_REASON_DEFAULT_LANGUAGE,
};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::{fractional_index, OpLogRepository};
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_storage::jobs::JobType;
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, CommitMetadata, FullTextSearchQuery, IndexingEngine, NodeTypeRepository,
    RegistryRepository, RepositoryManagementRepository, Storage, TranslationRepository,
};
use tempfile::TempDir;
use uuid::Uuid;

const TENANT: &str = "deflang-test";
const REPO: &str = "site";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

fn build_node(path: &str, title: &str) -> Node {
    let name = path.trim_start_matches('/').to_string();
    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String(title.to_string()),
    );

    Node {
        id: Uuid::new_v4().to_string(),
        name,
        path: path.to_string(),
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
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

/// A repository whose default language is `en`, with branch `main` and one
/// workspace. `cluster_node_id` turns replication capture on.
async fn setup_storage(cluster_node_id: Option<&str>) -> Result<(Arc<RocksDBStorage>, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    if let Some(node_id) = cluster_node_id {
        config.replication_enabled = true;
        config.cluster_node_id = Some(node_id.to_string());
    }
    let storage = Arc::new(RocksDBStorage::with_config(config)?);

    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;

    let repo_config = RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
        default_branch: BRANCH.to_string(),
        description: Some("Default language change test".to_string()),
        tags: HashMap::new(),
    };
    storage
        .repository_management()
        .create_repository(TENANT, REPO, repo_config)
        .await?;

    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;

    // Seed the node type BEFORE creating the workspace: WorkspaceService::put
    // bootstraps a ROOT node, which requires the default folder type.
    let folder_type = NodeType {
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
    };
    storage
        .node_types()
        .upsert(
            BranchScope::new(TENANT, REPO, BRANCH),
            folder_type,
            CommitMetadata::system("seed folder type"),
        )
        .await?;

    let mut workspace = Workspace::new(WORKSPACE.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, REPO, workspace)
        .await?;

    Ok((storage, temp_dir))
}

async fn add_node(storage: &Arc<RocksDBStorage>, node: &Node) -> Result<()> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_actor("test")?;
    tx.set_message("seed")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    tx.add_node(WORKSPACE, node).await?;
    tx.commit().await
}

async fn store_overlay(
    storage: &Arc<RocksDBStorage>,
    node_id: &str,
    locale: &str,
    overlay: LocaleOverlay,
    physical: u64,
) -> Result<()> {
    let locale = LocaleCode::parse(locale)?;
    let meta = TranslationMeta::system(
        locale.clone(),
        HLC::new(physical, 0),
        "test overlay".to_string(),
    );
    storage
        .translations()
        .store_translation(
            TENANT, REPO, BRANCH, WORKSPACE, node_id, &locale, &overlay, &meta,
        )
        .await
}

fn title_overlay(title: &str) -> LocaleOverlay {
    let mut data = HashMap::new();
    data.insert(
        JsonPointer::new("/title"),
        PropertyValue::String(title.to_string()),
    );
    LocaleOverlay::properties(data)
}

fn search(
    engine: &raisin_indexer::tantivy_engine::TantivyIndexingEngine,
    language: &str,
    query: &str,
) -> usize {
    engine
        .search(&FullTextSearchQuery {
            tenant_id: TENANT.to_string(),
            repo_id: REPO.to_string(),
            workspace_ids: None,
            branch: BRANCH.to_string(),
            language: language.to_string(),
            query: query.to_string(),
            limit: 10,
            revision: None,
            shape_types: None,
        })
        .map(|hits| hits.len())
        .unwrap_or_else(|e| panic!("search failed: {e}"))
}

/// Search until the reader has picked up the last commit (it reloads asynchronously).
async fn search_eventually(
    engine: &raisin_indexer::tantivy_engine::TantivyIndexingEngine,
    language: &str,
    query: &str,
) -> usize {
    let mut found = 0;
    for _ in 0..50 {
        found = search(engine, language, query);
        if found > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    found
}

async fn change_default(storage: &Arc<RocksDBStorage>, language: &str) -> Result<Option<String>> {
    let repo_mgmt = storage.repository_management();
    let mut config = repo_mgmt
        .get_repository(TENANT, REPO)
        .await?
        .expect("repository")
        .config;
    let previous = config.set_default_language(language);
    repo_mgmt
        .update_repository_config(TENANT, REPO, config)
        .await?;
    Ok(previous)
}

/// FulltextRebuild jobs queued for the repository, as (branch, reason).
async fn rebuild_jobs(storage: &Arc<RocksDBStorage>) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for job in storage.job_registry().list_jobs_by_tenant(TENANT).await {
        if !matches!(job.job_type, JobType::FulltextRebuild) {
            continue;
        }
        let context = storage
            .job_data_store()
            .get(TENANT, &job.id)
            .unwrap()
            .expect("a queued job has its context");
        if context.repo_id == REPO {
            let reason = context
                .metadata
                .get(META_REINDEX_REASON)
                .and_then(|v| v.as_str())
                .map(str::to_string);
            out.push((context.branch, reason));
        }
    }
    out
}

#[tokio::test]
async fn changing_en_to_de_moves_base_content_to_the_german_index() -> Result<()> {
    let (storage, dir) = setup_storage(None).await?;
    add_node(&storage, &build_node("/fluege", "Flugplan und Abflüge")).await?;

    let engine = Arc::new(raisin_indexer::tantivy_engine::TantivyIndexingEngine::new(
        dir.path().join("tantivy"),
        10,
    )?);
    raisin_rocksdb::management::rebuild_fulltext_index(&storage, &engine, TENANT, REPO, BRANCH)
        .await?;
    assert!(
        search_eventually(&engine, "en", "flu*").await >= 1,
        "before the change, base content is filed under the old default"
    );
    assert_eq!(search(&engine, "de", "flu*"), 0);

    assert_eq!(
        count_translation_overlays(&storage, TENANT, REPO, "de")
            .await?
            .total(),
        0
    );
    assert_eq!(
        change_default(&storage, "de").await?,
        Some("en".to_string())
    );

    let config = storage
        .repository_management()
        .get_repository(TENANT, REPO)
        .await?
        .unwrap()
        .config;
    assert_eq!(config.default_language, "de");
    assert_eq!(config.supported_languages, vec!["en", "de"]);

    // One rebuild per branch, marked with why it was queued.
    let jobs = enqueue_fulltext_rebuild_all_branches(&storage, TENANT, REPO).await?;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].branch, BRANCH);
    assert_eq!(
        rebuild_jobs(&storage).await,
        vec![(
            BRANCH.to_string(),
            Some(REINDEX_REASON_DEFAULT_LANGUAGE.to_string())
        )]
    );

    // What the worker does with each queued FulltextRebuild.
    for job in &jobs {
        raisin_rocksdb::management::rebuild_fulltext_index(
            &storage,
            &engine,
            TENANT,
            REPO,
            &job.branch,
        )
        .await?;
    }

    assert!(
        search_eventually(&engine, "de", "flu*").await >= 1,
        "after the change, base content is found in German"
    );
    assert_eq!(
        search(&engine, "en", "flu*"),
        0,
        "and no longer under English"
    );
    Ok(())
}

#[tokio::test]
async fn overlays_in_the_new_default_are_counted_and_deleted_ones_are_not() -> Result<()> {
    let (storage, _dir) = setup_storage(None).await?;
    let a = build_node("/a", "Alpha");
    let b = build_node("/b", "Beta");
    let c = build_node("/c", "Gamma");
    add_node(&storage, &a).await?;
    add_node(&storage, &b).await?;
    add_node(&storage, &c).await?;

    // a: live German overlay; b: German overlay deleted since (empty overlay
    // is how a translation is deleted); c: hidden in German (counts).
    store_overlay(&storage, &a.id, "de", title_overlay("Alpha DE"), 100).await?;
    store_overlay(&storage, &b.id, "de", title_overlay("Beta DE"), 100).await?;
    store_overlay(
        &storage,
        &b.id,
        "de",
        LocaleOverlay::properties(HashMap::new()),
        200,
    )
    .await?;
    store_overlay(&storage, &c.id, "de", LocaleOverlay::hidden(), 100).await?;
    // A French overlay is not a German one.
    store_overlay(&storage, &a.id, "fr", title_overlay("Alpha FR"), 100).await?;

    let de = count_translation_overlays(&storage, TENANT, REPO, "de").await?;
    assert_eq!(de.node_overlays, 2, "{de:?}");
    assert_eq!(de.block_overlays, 0, "{de:?}");
    assert_eq!(
        count_translation_overlays(&storage, TENANT, REPO, "fr")
            .await?
            .total(),
        1
    );
    assert_eq!(
        count_translation_overlays(&storage, TENANT, REPO, "it")
            .await?
            .total(),
        0
    );
    Ok(())
}

#[tokio::test]
async fn changing_to_the_same_default_is_a_no_op() -> Result<()> {
    let (storage, _dir) = setup_storage(None).await?;
    let before = storage
        .repository_management()
        .get_repository(TENANT, REPO)
        .await?
        .unwrap()
        .config;
    assert_eq!(change_default(&storage, "en").await?, None);
    let after = storage
        .repository_management()
        .get_repository(TENANT, REPO)
        .await?
        .unwrap()
        .config;
    assert_eq!(before, after);
    Ok(())
}

/// The newest captured `UpdateRepository` operation of the origin.
fn latest_update_repository_op(storage: &Arc<RocksDBStorage>) -> Operation {
    let oplog = OpLogRepository::new(storage.db().clone());
    let mut ops: Vec<Operation> = oplog
        .get_all_operations(TENANT, REPO)
        .unwrap()
        .into_values()
        .flatten()
        .filter(|op| matches!(op.op_type, OpType::UpdateRepository { .. }))
        .collect();
    ops.sort_by_key(|op| op.op_seq);
    ops.pop()
        .expect("an UpdateRepository operation was captured")
}

#[tokio::test]
async fn a_replicated_default_language_change_is_applied_and_reindexed_on_the_peer() -> Result<()> {
    let (origin, _origin_dir) = setup_storage(Some("node-origin")).await?;
    let (peer, _peer_dir) = setup_storage(None).await?;
    peer.event_bus()
        .subscribe(Arc::new(DefaultLanguageReindexHandler::new(peer.clone())));
    let applicator = OperationApplicator::new(
        peer.db().clone(),
        peer.event_bus(),
        Arc::new(peer.branches_impl().clone()),
    );

    // A replicated update that leaves the default alone queues nothing.
    applicator
        .apply_operation(&latest_update_repository_op(&origin))
        .await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(rebuild_jobs(&peer).await.is_empty());

    // The origin changes its default; the peer applies the captured operation.
    change_default(&origin, "de").await?;
    let op = latest_update_repository_op(&origin);
    match &op.op_type {
        OpType::UpdateRepository { repository, .. } => {
            assert_eq!(repository.config.default_language, "de")
        }
        _ => unreachable!(),
    }
    applicator.apply_operation(&op).await?;

    let config = peer
        .repository_management()
        .get_repository(TENANT, REPO)
        .await?
        .unwrap()
        .config;
    assert_eq!(config.default_language, "de");
    assert_eq!(config.supported_languages, vec!["en", "de"]);

    // The peer's own full-text index is rebuilt (handlers run asynchronously).
    let mut jobs = Vec::new();
    for _ in 0..50 {
        jobs = rebuild_jobs(&peer).await;
        if !jobs.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        jobs,
        vec![(
            BRANCH.to_string(),
            Some(REINDEX_REASON_DEFAULT_LANGUAGE.to_string())
        )]
    );
    Ok(())
}
