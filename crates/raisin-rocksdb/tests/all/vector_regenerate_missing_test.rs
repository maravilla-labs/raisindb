//! `vector/regenerate` queues an embedding for every eligible node that has
//! none, not only for stored embeddings with the wrong dimensions.
//!
//! Before, regenerate scanned only the embeddings column family, so a node
//! whose embedding job had died at max retries (embedder down) was invisible
//! to it and stayed out of semantic search until something edited it.
//! Eligibility is the node-event trigger's rule: a type with `indexable:
//! false`, or whose `index_types` leave out `Vector`, is not queued.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use raisin_ai::config::{EmbedderId, EmbeddingKind};
use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_embeddings::config::{EmbeddingProvider, TenantEmbeddingConfig};
use raisin_embeddings::{EmbeddingData, EmbeddingStorage, TenantEmbeddingConfigStore};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::schema::IndexType;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::{
    fractional_index, MaintenanceJobHandler, RocksDBConfig, RocksDBEmbeddingStorage, RocksDBStorage,
};
use raisin_storage::jobs::{JobContext, JobType};
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeTypeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage,
};
use tempfile::TempDir;

const TENANT: &str = "regen-missing";
const REPO: &str = "site";
const BRANCH: &str = "main";
const WS: &str = "default";
const DIMS: usize = 8;

fn node_type(name: &str, indexable: bool, index_types: Option<Vec<IndexType>>) -> NodeType {
    NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
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
        indexable: Some(indexable),
        index_types,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    }
}

fn node(id: &str, node_type: &str) -> Node {
    let mut properties = HashMap::new();
    properties.insert("title".to_string(), PropertyValue::String(id.to_string()));
    Node {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/{id}"),
        node_type: node_type.to_string(),
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

fn embedding(source_id: &str, dims: usize) -> EmbeddingData {
    EmbeddingData {
        vector: vec![0.5; dims],
        embedder_id: EmbedderId::new("ollama", "test-model", dims),
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

#[tokio::test]
async fn regenerate_queues_eligible_nodes_without_an_embedding() -> Result<()> {
    let dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);

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
                default_language: "de".to_string(),
                supported_languages: vec!["de".to_string()],
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
    for nt in [
        node_type("raisin:Folder", true, None),
        node_type("test:FulltextOnly", true, Some(vec![IndexType::Fulltext])),
        node_type("test:Hidden", false, None),
    ] {
        storage
            .node_types()
            .upsert(
                BranchScope::new(TENANT, REPO, BRANCH),
                nt,
                CommitMetadata::system("seed node type"),
            )
            .await?;
    }
    let mut workspace = Workspace::new(WS.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, REPO, workspace)
        .await?;

    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_actor("test")?;
    tx.set_message("seed")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    for (id, nt) in [
        ("missing", "raisin:Folder"),
        ("current", "raisin:Folder"),
        ("stale", "raisin:Folder"),
        ("fulltextonly", "test:FulltextOnly"),
        ("hidden", "test:Hidden"),
    ] {
        tx.add_node(WS, &node(id, nt)).await?;
    }
    tx.commit().await?;

    let mut embedding_config = TenantEmbeddingConfig::new(TENANT.to_string());
    embedding_config.enabled = true;
    embedding_config.provider = EmbeddingProvider::Ollama;
    embedding_config.model = "test-model".to_string();
    embedding_config.dimensions = DIMS;
    storage
        .tenant_embedding_config_repository()
        .set_config(&embedding_config)
        .expect("store embedding config");

    // "current" has a correct embedding, "stale" one of the wrong length.
    let embeddings = RocksDBEmbeddingStorage::new(storage.db().clone());
    let revision = HLC::new(1_000, 0);
    embeddings.store_embedding(
        TENANT,
        REPO,
        BRANCH,
        WS,
        "current",
        &revision,
        &embedding("current", DIMS),
    )?;
    embeddings.store_embedding(
        TENANT,
        REPO,
        BRANCH,
        WS,
        "stale",
        &revision,
        &embedding("stale", DIMS * 2),
    )?;

    let registry = storage.job_registry().clone();
    let regenerate_id = registry
        .register_job(
            JobType::VectorRegenerate,
            TENANT.to_string(),
            None,
            None,
            None,
        )
        .await?;
    let regenerate_job = registry.get_job_info(&regenerate_id).await?;
    let context = JobContext {
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        workspace_id: String::new(),
        revision: HLC::new(0, 0),
        metadata: HashMap::new(),
    };
    let result = MaintenanceJobHandler::new(storage.clone())
        .handle(&regenerate_job, &context)
        .await?
        .expect("regenerate result");

    let queued: HashSet<String> = registry
        .list_jobs_by_tenant(TENANT)
        .await
        .into_iter()
        .filter_map(|job| match job.job_type {
            JobType::EmbeddingGenerate { node_id } => Some(node_id),
            _ => None,
        })
        .collect();

    assert!(
        queued.contains("missing"),
        "a node without an embedding is queued: {result}"
    );
    assert!(
        queued.contains("stale"),
        "a dimension mismatch is still queued: {result}"
    );
    assert!(
        !queued.contains("current"),
        "a current embedding is left alone: {result}"
    );
    assert!(
        !queued.contains("fulltextonly"),
        "a type without Vector in index_types is not eligible: {result}"
    );
    assert!(
        !queued.contains("hidden"),
        "an indexable:false type is not eligible: {result}"
    );
    assert_eq!(result["skipped"], 1, "{result}");
    assert!(
        result["missing"].as_u64().unwrap() >= 1,
        "the result counts the missing nodes: {result}"
    );
    Ok(())
}
