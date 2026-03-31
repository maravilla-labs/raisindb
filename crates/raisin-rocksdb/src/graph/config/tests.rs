//! Tests for graph algorithm configuration parsing.

use super::*;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use std::collections::HashMap;

fn make_test_node() -> Node {
    let mut properties = HashMap::new();

    properties.insert(
        "algorithm".to_string(),
        PropertyValue::String("pagerank".to_string()),
    );
    properties.insert("enabled".to_string(), PropertyValue::Boolean(true));

    // Target
    let mut target = HashMap::new();
    target.insert(
        "mode".to_string(),
        PropertyValue::String("branch".to_string()),
    );
    target.insert(
        "branches".to_string(),
        PropertyValue::Array(vec![PropertyValue::String("main".to_string())]),
    );
    properties.insert("target".to_string(), PropertyValue::Object(target));

    // Scope
    let mut scope = HashMap::new();
    scope.insert(
        "node_types".to_string(),
        PropertyValue::Array(vec![PropertyValue::String("raisin:User".to_string())]),
    );
    properties.insert("scope".to_string(), PropertyValue::Object(scope));

    // Config
    let mut config = HashMap::new();
    config.insert("damping_factor".to_string(), PropertyValue::Float(0.85));
    config.insert("max_iterations".to_string(), PropertyValue::Integer(100));
    properties.insert("config".to_string(), PropertyValue::Object(config));

    // Refresh
    let mut refresh = HashMap::new();
    refresh.insert("ttl_seconds".to_string(), PropertyValue::Integer(300));
    refresh.insert("on_branch_change".to_string(), PropertyValue::Boolean(true));
    properties.insert("refresh".to_string(), PropertyValue::Object(refresh));

    Node {
        id: "test-config-id".to_string(),
        name: "pagerank-social".to_string(),
        node_type: "raisin:GraphAlgorithmConfig".to_string(),
        properties,
        ..Default::default()
    }
}

#[test]
fn test_parse_config_from_node() {
    let node = make_test_node();
    let config = GraphAlgorithmConfig::from_node(&node).unwrap();

    assert_eq!(config.id, "pagerank-social");
    assert_eq!(config.algorithm, GraphAlgorithm::PageRank);
    assert!(config.enabled);
    assert_eq!(config.target.mode, TargetMode::Branch);
    assert_eq!(config.target.branches, vec!["main".to_string()]);
    assert_eq!(config.scope.node_types, vec!["raisin:User".to_string()]);
    assert_eq!(config.get_config_f64("damping_factor"), Some(0.85));
    assert_eq!(config.get_config_i64("max_iterations"), Some(100));
    assert_eq!(config.refresh.ttl_seconds, 300);
    assert!(config.refresh.on_branch_change);
}

#[test]
fn test_targets_branch() {
    let node = make_test_node();
    let config = GraphAlgorithmConfig::from_node(&node).unwrap();

    assert!(config.targets_branch("main"));
    assert!(!config.targets_branch("develop"));
}

#[test]
fn test_glob_match() {
    assert!(parsers::glob_match("release/*", "release/v1.0"));
    assert!(parsers::glob_match("release/*", "release/v2.0"));
    assert!(!parsers::glob_match("release/*", "develop"));
    assert!(parsers::glob_match("feature/**", "feature/auth/login"));
}
