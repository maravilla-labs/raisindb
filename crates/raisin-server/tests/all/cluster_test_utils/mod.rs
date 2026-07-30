// Test utilities for cluster integration testing
//
// This module provides utilities for testing a 3-node RaisinDB cluster via REST and WebSocket APIs.
// All tests use the public API endpoints exposed by raisin-server binaries.

pub mod config;
pub mod fixture;
pub mod ports;
pub mod process;
pub mod rest_client;
pub mod social_feed;
pub mod verification;
pub mod websocket_client;

// Re-export commonly used types and functions
pub use config::ClusterConfig;
pub use fixture::ClusterTestFixture;
pub use ports::unique_ports;
pub use process::ClusterProcess;
pub use rest_client::RestClient;
pub use social_feed::{create_comment, create_post};
pub use verification::{
    explain_on_node, verify_all_nodes_agree, verify_child_order_via_rest,
    verify_child_order_via_sql, verify_comment_exists_on_all_nodes,
    verify_node_exists_on_all_nodes, verify_node_properties_match,
    verify_plan_contains_on_all_nodes, verify_post_at_same_position,
    verify_relation_deleted_on_all_nodes, verify_relation_exists_on_all_nodes,
    verify_relations_match, verify_spatial_query_on_all_nodes, wait_for_nodetype_replication,
    wait_for_replication, wait_for_replication_by_id,
};
