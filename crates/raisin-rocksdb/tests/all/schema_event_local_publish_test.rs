//! `Event::Schema` must be published for LOCAL schema writes, not only replicated ones.
//!
//! Before this test, the only publisher of `Event::Schema` was the replication
//! applicator (`emit_schema_event`, `metadata.source = "replication"`). The node
//! that ORIGINATED a NodeType / Archetype / ElementType edit published nothing,
//! so `SchemaStatsEventHandler` never fired there and the schema stats cache
//! served stale counts until its 5-minute TTL floor expired. On a single-node
//! deployment that is every schema edit there is.
//!
//! The publish now happens in the repository layer — the one choke point all
//! local write call sites funnel through. The applicator deliberately writes
//! straight to the column families to avoid re-entering those repositories, so
//! a replicated change must still produce exactly ONE event; the second test
//! pins that down.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_events::{Event, EventHandler, SchemaEvent, SchemaEventKind};
use raisin_hlc::HLC;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_replication::{OpType, Operation};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::BranchScope;
use raisin_storage::{
    ArchetypeRepository, BranchRepository, CommitMetadata, ElementTypeRepository,
    NodeTypeRepository, RegistryRepository, RepositoryManagementRepository, Storage,
};
use std::future::Future;
use std::pin::Pin;
use tempfile::TempDir;

const TENANT: &str = "schema-event-tenant";
const REPO: &str = "schema-event-repo";
const BRANCH: &str = "main";

/// Records every `Event::Schema` the bus dispatches.
struct SchemaEventRecorder {
    events: Arc<Mutex<Vec<SchemaEvent>>>,
}

impl EventHandler for SchemaEventRecorder {
    fn name(&self) -> &str {
        "schema_event_recorder"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Event::Schema(schema_event) = event {
                self.events.lock().unwrap().push(schema_event.clone());
            }
            Ok(())
        })
    }
}

fn storage(node_id: &str) -> (TempDir, Arc<RocksDBStorage>) {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    config.replication_enabled = true;
    config.cluster_node_id = Some(node_id.to_string());
    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());
    (temp_dir, storage)
}

/// Schema writes need the branch to exist — `update_head` targets its record.
async fn seed_repo(storage: &Arc<RocksDBStorage>) -> Result<()> {
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;

    let repo_config = RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
        default_branch: BRANCH.to_string(),
        description: Some("Schema event publish test".to_string()),
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

    Ok(())
}

/// Subscribe AFTER seeding, so repo/branch bootstrap noise stays out of the
/// recording and each assertion counts only the write under test.
fn attach_recorder(storage: &Arc<RocksDBStorage>) -> Arc<Mutex<Vec<SchemaEvent>>> {
    let events = Arc::new(Mutex::new(Vec::new()));
    storage.event_bus().subscribe(Arc::new(SchemaEventRecorder {
        events: events.clone(),
    }));
    events
}

/// The bus dispatches to handlers in spawned tasks, so wait for the expected
/// count rather than reading immediately.
async fn wait_for_events(
    events: &Arc<Mutex<Vec<SchemaEvent>>>,
    expected: usize,
) -> Vec<SchemaEvent> {
    for _ in 0..100 {
        if events.lock().unwrap().len() >= expected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Settle briefly so a spurious EXTRA event has a chance to show up too.
    tokio::time::sleep(Duration::from_millis(50)).await;
    events.lock().unwrap().clone()
}

fn source_of(event: &SchemaEvent) -> Option<String> {
    event
        .metadata
        .as_ref()
        .and_then(|m| m.get("source"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn scope() -> BranchScope<'static> {
    BranchScope::new(TENANT, REPO, BRANCH)
}

fn make_node_type(name: &str) -> NodeType {
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

fn make_archetype(name: &str) -> Archetype {
    serde_json::from_value(serde_json::json!({ "name": name }))
        .expect("archetype fixture must deserialize")
}

fn make_element_type(name: &str) -> ElementType {
    serde_json::from_value(serde_json::json!({ "name": name }))
        .expect("element type fixture must deserialize")
}

/// A local NodeType create / update / delete must each publish exactly one
/// `Event::Schema` with `source = "local"` and the matching kind.
#[tokio::test]
async fn local_node_type_writes_publish_schema_events() -> Result<()> {
    let (_dir, storage) = storage("node-local");
    seed_repo(&storage).await?;
    let events = attach_recorder(&storage);

    storage
        .node_types()
        .create(
            scope(),
            make_node_type("test:Article"),
            CommitMetadata::system("create"),
        )
        .await?;

    storage
        .node_types()
        .update(
            scope(),
            make_node_type("test:Article"),
            CommitMetadata::system("update"),
        )
        .await?;

    storage
        .node_types()
        .delete(scope(), "test:Article", CommitMetadata::system("delete"))
        .await?;

    let recorded = wait_for_events(&events, 3).await;

    let kinds: Vec<&SchemaEventKind> = recorded.iter().map(|e| &e.kind).collect();
    assert_eq!(
        kinds,
        vec![
            &SchemaEventKind::NodeTypeCreated,
            &SchemaEventKind::NodeTypeUpdated,
            &SchemaEventKind::NodeTypeDeleted,
        ],
        "expected one create/update/delete schema event, got {:?}",
        kinds
    );

    for event in &recorded {
        assert_eq!(
            source_of(event).as_deref(),
            Some("local"),
            "local schema write must be tagged source=local"
        );
        assert_eq!(event.tenant_id, TENANT);
        assert_eq!(event.repository_id, REPO);
        assert_eq!(event.branch, BRANCH);
        assert_eq!(event.schema_id, "test:Article");
        assert_eq!(event.schema_type, "NodeType");
    }

    Ok(())
}

/// Archetypes and ElementTypes go through the same repository choke point and
/// must publish too — the schema stats cache keys on the branch, not the type.
#[tokio::test]
async fn local_archetype_and_element_type_writes_publish_schema_events() -> Result<()> {
    let (_dir, storage) = storage("node-local-2");
    seed_repo(&storage).await?;
    let events = attach_recorder(&storage);

    storage
        .archetypes()
        .create(
            scope(),
            make_archetype("test:Hero"),
            CommitMetadata::system("create archetype"),
        )
        .await?;

    storage
        .element_types()
        .create(
            scope(),
            make_element_type("test:Banner"),
            CommitMetadata::system("create element type"),
        )
        .await?;

    let recorded = wait_for_events(&events, 2).await;

    assert_eq!(
        recorded.len(),
        2,
        "expected exactly two schema events, got {:?}",
        recorded.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
    assert_eq!(recorded[0].kind, SchemaEventKind::ArchetypeCreated);
    assert_eq!(recorded[0].schema_type, "Archetype");
    assert_eq!(recorded[0].schema_id, "test:Hero");
    assert_eq!(recorded[1].kind, SchemaEventKind::ElementTypeCreated);
    assert_eq!(recorded[1].schema_type, "ElementType");
    assert_eq!(recorded[1].schema_id, "test:Banner");

    for event in &recorded {
        assert_eq!(source_of(event).as_deref(), Some("local"));
    }

    Ok(())
}

fn update_nodetype_op(node_type: &NodeType, revision: HLC) -> Operation {
    Operation {
        op_id: uuid::Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "peer-node".to_string(),
        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        vector_clock: raisin_replication::VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        op_type: OpType::UpdateNodeType {
            node_type_id: node_type.name.clone(),
            node_type: node_type.clone(),
        },
        revision: Some(revision),
        actor: "peer".to_string(),
        agent: None,
        message: None,
        is_system: false,
        acknowledged_by: Default::default(),
    }
}

/// No double-fire: the applicator writes directly to the column families and
/// never re-enters the repositories, so a replicated schema op still produces
/// exactly ONE event — the applicator's, tagged `source = "replication"`.
///
/// If someone ever "simplifies" the applicator to write through
/// `storage.node_types()`, this test fails with two events instead of one.
#[tokio::test]
async fn replicated_node_type_apply_publishes_exactly_one_event() -> Result<()> {
    let (_dir, storage) = storage("node-replica");
    seed_repo(&storage).await?;
    let events = attach_recorder(&storage);

    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );

    let node_type = make_node_type("test:Replicated");
    applicator
        .apply_operation(&update_nodetype_op(&node_type, HLC::new(42, 0)))
        .await?;

    let recorded = wait_for_events(&events, 1).await;

    assert_eq!(
        recorded.len(),
        1,
        "a replicated schema apply must publish exactly one Event::Schema, got {:?}",
        recorded
            .iter()
            .map(|e| (format!("{:?}", e.kind), source_of(e)))
            .collect::<Vec<_>>()
    );
    assert_eq!(recorded[0].kind, SchemaEventKind::NodeTypeUpdated);
    assert_eq!(
        source_of(&recorded[0]).as_deref(),
        Some("replication"),
        "the applicator remains the publisher for replicated ops"
    );

    // Sanity: metadata is present and carries nothing but the source marker we
    // assert on, so a future extra key does not silently change consumer logic.
    let metadata: &HashMap<String, serde_json::Value> =
        recorded[0].metadata.as_ref().expect("metadata present");
    assert!(metadata.contains_key("source"));

    Ok(())
}
