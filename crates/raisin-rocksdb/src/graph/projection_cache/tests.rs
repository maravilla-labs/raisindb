use super::*;
use tempfile::TempDir;

fn create_test_storage() -> (RocksDBStorage, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
    (storage, temp_dir)
}

fn test_key() -> ProjectionKey {
    ProjectionKey {
        tenant_id: "t1".to_string(),
        repo_id: "r1".to_string(),
        branch: "main".to_string(),
        config_id: "pagerank-social".to_string(),
    }
}

fn create_test_projection() -> GraphProjection {
    let nodes = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let edges = vec![
        ("a".to_string(), "b".to_string()),
        ("b".to_string(), "c".to_string()),
    ];
    GraphProjection::from_parts(nodes, edges)
}

#[test]
fn test_store_and_load() {
    let (storage, _dir) = create_test_storage();
    let key = test_key();
    let projection = create_test_projection();

    // Store
    GraphProjectionStore::store(&key, &projection, "rev1".to_string(), &storage).unwrap();

    // Load
    let loaded = GraphProjectionStore::load(&key, &storage).unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.node_count(), 3);
    assert_eq!(loaded.edge_count(), 2);
}

#[test]
fn test_load_nonexistent() {
    let (storage, _dir) = create_test_storage();
    let key = test_key();

    let loaded = GraphProjectionStore::load(&key, &storage).unwrap();
    assert!(loaded.is_none());
}

#[test]
fn test_mark_stale_and_load_returns_none() {
    let (storage, _dir) = create_test_storage();
    let key = test_key();
    let projection = create_test_projection();

    GraphProjectionStore::store(&key, &projection, "rev1".to_string(), &storage).unwrap();
    GraphProjectionStore::mark_stale(&key, &storage).unwrap();

    // Stale projection should return None
    let loaded = GraphProjectionStore::load(&key, &storage).unwrap();
    assert!(loaded.is_none());

    // But meta should still be loadable (and show stale)
    let meta = GraphProjectionStore::load_meta(&key, &storage).unwrap();
    assert!(meta.is_some());
    assert!(meta.unwrap().is_stale());
}

#[test]
fn test_mark_branch_stale() {
    let (storage, _dir) = create_test_storage();
    let projection = create_test_projection();

    // Store two projections on same branch
    let key1 = ProjectionKey {
        tenant_id: "t1".to_string(),
        repo_id: "r1".to_string(),
        branch: "main".to_string(),
        config_id: "pagerank".to_string(),
    };
    let key2 = ProjectionKey {
        tenant_id: "t1".to_string(),
        repo_id: "r1".to_string(),
        branch: "main".to_string(),
        config_id: "wcc".to_string(),
    };

    GraphProjectionStore::store(&key1, &projection, "rev1".to_string(), &storage).unwrap();
    GraphProjectionStore::store(&key2, &projection, "rev1".to_string(), &storage).unwrap();

    // Mark branch stale
    GraphProjectionStore::mark_branch_stale("t1", "r1", "main", &storage).unwrap();

    // Both should be stale
    assert!(GraphProjectionStore::load(&key1, &storage)
        .unwrap()
        .is_none());
    assert!(GraphProjectionStore::load(&key2, &storage)
        .unwrap()
        .is_none());
}

#[test]
fn test_delete() {
    let (storage, _dir) = create_test_storage();
    let key = test_key();
    let projection = create_test_projection();

    GraphProjectionStore::store(&key, &projection, "rev1".to_string(), &storage).unwrap();
    GraphProjectionStore::delete(&key, &storage).unwrap();

    assert!(GraphProjectionStore::load(&key, &storage)
        .unwrap()
        .is_none());
}

#[test]
fn test_overwrite() {
    let (storage, _dir) = create_test_storage();
    let key = test_key();

    // Store with 3 nodes
    let proj1 = create_test_projection();
    GraphProjectionStore::store(&key, &proj1, "rev1".to_string(), &storage).unwrap();

    // Overwrite with 2 nodes
    let proj2 = GraphProjection::from_parts(
        vec!["x".to_string(), "y".to_string()],
        vec![("x".to_string(), "y".to_string())],
    );
    GraphProjectionStore::store(&key, &proj2, "rev2".to_string(), &storage).unwrap();

    let loaded = GraphProjectionStore::load(&key, &storage).unwrap().unwrap();
    assert_eq!(loaded.node_count(), 2);
}
