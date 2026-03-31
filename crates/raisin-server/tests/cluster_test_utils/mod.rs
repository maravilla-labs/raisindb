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
pub use config::{ClusterConfig, NodeConfig};
pub use fixture::ClusterTestFixture;
pub use ports::{free_port, unique_ports};
pub use process::{ClusterProcess, NodeLogs};
pub use rest_client::RestClient;
pub use social_feed::{
    add_follow_relationships, create_comment, create_demo_users, create_initial_posts, create_post,
    init_social_feed_schema, SOCIAL_FEED_BRANCH, SOCIAL_FEED_NODE_TYPES, SOCIAL_FEED_REPO,
    SOCIAL_FEED_WORKSPACE,
};
pub use verification::{
    dump_children_order, verify_child_order_via_rest, verify_child_order_via_sql,
    verify_comment_exists_on_all_nodes, verify_node_exists_on_all_nodes,
    verify_node_properties_match, verify_post_at_same_position,
    verify_relation_deleted_on_all_nodes, verify_relation_exists_on_all_nodes,
    verify_relations_match, wait_for_nodetype_replication, wait_for_replication,
    wait_for_replication_by_id,
};
pub use websocket_client::WebSocketClient;
