use super::*;
use crate::keys::{graph_cache_key, graph_cache_meta_key};
use std::time::Duration;

#[test]
fn test_default_config() {
    let config = GraphComputeConfig::default();
    assert!(config.enabled);
    assert_eq!(config.check_interval, Duration::from_secs(60));
    assert_eq!(config.max_configs_per_tick, 10);
}

#[test]
fn test_meta_key_format() {
    let key = graph_cache_meta_key("tenant1", "repo1", "main", "pagerank-config");
    let key_str = String::from_utf8_lossy(&key);
    assert!(key_str.contains("tenant1"));
    assert!(key_str.contains("repo1"));
    assert!(key_str.contains("main"));
    assert!(key_str.contains("pagerank-config"));
    assert!(key_str.contains("_meta"));
}

#[test]
fn test_value_key_format() {
    let key = graph_cache_key("tenant1", "repo1", "main", "pagerank-config", "node123");
    let key_str = String::from_utf8_lossy(&key);
    assert!(key_str.contains("tenant1"));
    assert!(key_str.contains("repo1"));
    assert!(key_str.contains("main"));
    assert!(key_str.contains("pagerank-config"));
    assert!(key_str.contains("node123"));
}
