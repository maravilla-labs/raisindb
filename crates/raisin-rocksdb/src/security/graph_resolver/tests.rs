//! Tests for the graph resolver module.

use super::*;
use raisin_models::nodes::{FullRelation, RelationRef};
use std::sync::Arc;

/// Mock relation repository for testing
struct MockRelationRepo {
    relations: Vec<(String, String, String, String, FullRelation)>,
}

impl RelationRepository for MockRelationRepo {
    fn add_relation(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _source_workspace: &str,
        _source_node_id: &str,
        _source_node_type: &str,
        _relation: RelationRef,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async { unimplemented!() }
    }

    fn remove_relation(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _source_workspace: &str,
        _source_node_id: &str,
        _target_workspace: &str,
        _target_node_id: &str,
    ) -> impl std::future::Future<Output = Result<bool>> + Send {
        async { unimplemented!() }
    }

    fn get_outgoing_relations(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<RelationRef>>> + Send {
        async { unimplemented!() }
    }

    fn get_incoming_relations(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<(String, String, RelationRef)>>> + Send {
        async { unimplemented!() }
    }

    fn get_relations_by_type(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _target_node_type: &str,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<RelationRef>>> + Send {
        async { unimplemented!() }
    }

    fn remove_all_relations_for_node(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        async { unimplemented!() }
    }

    fn scan_relations_global(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _relation_type_filter: Option<&str>,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<(String, String, String, String, FullRelation)>>>
           + Send {
        let relations = self.relations.clone();
        async move { Ok(relations) }
    }
}

fn make_relation(
    rel_type: &str,
    source_id: &str,
    source_ws: &str,
    target_id: &str,
    target_ws: &str,
) -> FullRelation {
    FullRelation {
        source_id: source_id.to_string(),
        source_workspace: source_ws.to_string(),
        source_node_type: "Node".to_string(),
        target_id: target_id.to_string(),
        target_workspace: target_ws.to_string(),
        target_node_type: "Node".to_string(),
        relation_type: rel_type.to_string(),
        weight: None,
    }
}

#[tokio::test]
async fn test_direct_path() {
    let repo = MockRelationRepo {
        relations: vec![(
            "ws".to_string(),
            "a".to_string(),
            "ws".to_string(),
            "b".to_string(),
            make_relation("owns", "a", "ws", "b", "ws"),
        )],
    };

    let revision = HLC::now();
    let resolver = RocksDBGraphResolver::new(&repo, "t1", "r1", "main", &revision);

    // Direct path with depth 1
    assert!(resolver
        .has_path(
            "a",
            "b",
            &["owns".to_string()],
            1,
            1,
            RelDirection::Outgoing
        )
        .await
        .unwrap());

    // No path with depth 2
    assert!(!resolver
        .has_path(
            "a",
            "b",
            &["owns".to_string()],
            2,
            2,
            RelDirection::Outgoing
        )
        .await
        .unwrap());
}

#[tokio::test]
async fn test_multi_hop_path() {
    // a -> b -> c
    let repo = MockRelationRepo {
        relations: vec![
            (
                "ws".to_string(),
                "a".to_string(),
                "ws".to_string(),
                "b".to_string(),
                make_relation("owns", "a", "ws", "b", "ws"),
            ),
            (
                "ws".to_string(),
                "b".to_string(),
                "ws".to_string(),
                "c".to_string(),
                make_relation("owns", "b", "ws", "c", "ws"),
            ),
        ],
    };

    let revision = HLC::now();
    let resolver = RocksDBGraphResolver::new(&repo, "t1", "r1", "main", &revision);

    // Path a -> c with depth 2
    assert!(resolver
        .has_path(
            "a",
            "c",
            &["owns".to_string()],
            2,
            2,
            RelDirection::Outgoing
        )
        .await
        .unwrap());

    // Path a -> c with depth range 1-3
    assert!(resolver
        .has_path(
            "a",
            "c",
            &["owns".to_string()],
            1,
            3,
            RelDirection::Outgoing
        )
        .await
        .unwrap());

    // No path with depth 1
    assert!(!resolver
        .has_path(
            "a",
            "c",
            &["owns".to_string()],
            1,
            1,
            RelDirection::Outgoing
        )
        .await
        .unwrap());
}

#[tokio::test]
async fn test_incoming_direction() {
    // a -> b
    let repo = MockRelationRepo {
        relations: vec![(
            "ws".to_string(),
            "a".to_string(),
            "ws".to_string(),
            "b".to_string(),
            make_relation("owns", "a", "ws", "b", "ws"),
        )],
    };

    let revision = HLC::now();
    let resolver = RocksDBGraphResolver::new(&repo, "t1", "r1", "main", &revision);

    // Incoming: b <- a
    assert!(resolver
        .has_path(
            "b",
            "a",
            &["owns".to_string()],
            1,
            1,
            RelDirection::Incoming
        )
        .await
        .unwrap());

    // Outgoing: b -> a (doesn't exist)
    assert!(!resolver
        .has_path(
            "b",
            "a",
            &["owns".to_string()],
            1,
            1,
            RelDirection::Outgoing
        )
        .await
        .unwrap());
}

#[tokio::test]
async fn test_cycle_detection() {
    // a -> b -> c -> a (cycle)
    let repo = MockRelationRepo {
        relations: vec![
            (
                "ws".to_string(),
                "a".to_string(),
                "ws".to_string(),
                "b".to_string(),
                make_relation("link", "a", "ws", "b", "ws"),
            ),
            (
                "ws".to_string(),
                "b".to_string(),
                "ws".to_string(),
                "c".to_string(),
                make_relation("link", "b", "ws", "c", "ws"),
            ),
            (
                "ws".to_string(),
                "c".to_string(),
                "ws".to_string(),
                "a".to_string(),
                make_relation("link", "c", "ws", "a", "ws"),
            ),
        ],
    };

    let revision = HLC::now();
    let resolver = RocksDBGraphResolver::new(&repo, "t1", "r1", "main", &revision);

    // Should handle cycles gracefully
    assert!(resolver
        .has_path(
            "a",
            "c",
            &["link".to_string()],
            1,
            5,
            RelDirection::Outgoing
        )
        .await
        .unwrap());
}
