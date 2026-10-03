//! Copied nodes are spatially indexed.
//!
//! Tree copy, cross-branch promotion and deep create stage their nodes through
//! the repository's batch writer (`add_node_to_batch_with_parent_id`), which
//! wrote every index family except the spatial one. The copy carried its
//! geometry, the index reported healthy, and every `ST_DWITHIN` on the copy
//! answered zero rows. These query the INDEX after a copy, for a top-level and a
//! nested geometry path.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{GeoJson, PropertyValue};
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::repositories::spatial_index::SpatialIndexRepository;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::spatial::SpatialPreFilter;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope,
};
use tempfile::TempDir;

const TENANT: &str = "copy-spatial";
const REPO: &str = "repo";
const WS: &str = "places";
const FLAT: &str = "location";
const NESTED: &str = "venue.geo";
const LON: f64 = 8.5402;
const LAT: f64 = 47.3782;

async fn setup() -> (Arc<RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(temp_dir.path()).expect("storage"));
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await
        .expect("tenant");
    storage
        .repository_management()
        .create_repository(
            TENANT,
            REPO,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".to_string()],
                locale_fallback_chains: HashMap::new(),
                default_branch: "main".to_string(),
                description: None,
                tags: HashMap::new(),
            },
        )
        .await
        .expect("repo");
    for branch in ["main", "publish"] {
        storage
            .branches()
            .create_branch(TENANT, REPO, branch, "system", None, None, false, false)
            .await
            .expect("branch");
    }
    let mut workspace = Workspace::new(WS.to_string());
    workspace.config.default_branch = "main".to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, REPO, workspace)
        .await
        .expect("workspace");
    (storage, temp_dir)
}

fn options() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    }
}

fn node(path: &str, properties: HashMap<String, PropertyValue>) -> Node {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        path: path.to_string(),
        node_type: "test:Place".to_string(),
        properties,
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    }
}

fn place(path: &str) -> Node {
    let point = || PropertyValue::Geometry(GeoJson::point(LON, LAT));
    node(
        path,
        HashMap::from([
            (FLAT.to_string(), point()),
            (
                "venue".to_string(),
                PropertyValue::Object(HashMap::from([("geo".to_string(), point())])),
            ),
        ]),
    )
}

/// Node ids an INDEX-BACKED radius query finds on `branch`.
fn within(storage: &RocksDBStorage, branch: &str, property: &str) -> Vec<String> {
    let mut ids: Vec<String> = SpatialIndexRepository::new(storage.db().clone())
        .find_within_radius(
            TENANT,
            REPO,
            branch,
            WS,
            property,
            LON,
            LAT,
            100.0,
            &HLC::new(u64::MAX / 2, 0),
            1000,
            raisin_rocksdb::spatial::INDEX_PRECISIONS,
            &SpatialPreFilter::default(),
        )
        .expect("radius query")
        .into_iter()
        .map(|r| r.node_id)
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn copied_geometry_is_spatially_indexed() {
    let (storage, _dir) = setup().await;
    let nodes = storage.nodes();
    let main = StorageScope::new(TENANT, REPO, "main", WS);

    nodes
        .create(main, node("/folder", HashMap::new()), options())
        .await
        .expect("folder");
    nodes
        .create(main, node("/target", HashMap::new()), options())
        .await
        .expect("target");
    let original = place("/folder/hb");
    nodes
        .create(main, original.clone(), options())
        .await
        .expect("place");
    assert_eq!(within(&storage, "main", FLAT), vec![original.id.clone()]);

    // Same-branch tree copy: the copy is a NEW node and must be found too.
    nodes
        .copy_node_tree(main, "/folder", "/target", None, None)
        .await
        .expect("tree copy");
    let copy = nodes
        .get_by_path(main, "/target/folder/hb", None)
        .await
        .expect("get")
        .expect("copied node");
    for property in [FLAT, NESTED] {
        let found = within(&storage, "main", property);
        assert!(
            found.contains(&copy.id),
            "tree copy of '{property}' is not spatially indexed: {found:?}"
        );
    }

    // Cross-branch promotion onto an empty branch.
    nodes
        .copy_nodes_across_branches(
            TENANT,
            REPO,
            "main",
            "publish",
            WS,
            &["/folder".to_string()],
            true,
            false,
            None,
            None,
        )
        .await
        .expect("promote");
    for property in [FLAT, NESTED] {
        assert_eq!(
            within(&storage, "publish", property),
            vec![original.id.clone()],
            "promoted '{property}' is not spatially indexed on the target branch"
        );
    }
}
