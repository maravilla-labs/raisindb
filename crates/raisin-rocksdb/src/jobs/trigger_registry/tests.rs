//! Tests for trigger registry

use super::snapshot::TriggerRegistrySnapshot;
use super::types::{CachedTrigger, TriggerFilters};

#[test]
fn test_snapshot_quick_reject() {
    let mut triggers = vec![];

    // Add trigger for specific workspace and node_type
    triggers.push(CachedTrigger {
        id: "trigger1".to_string(),
        function_path: Some("/functions/test".to_string()),
        trigger_name: "test-trigger".to_string(),
        trigger_path: None,
        priority: 0,
        enabled: true,
        event_kinds: vec!["Created".to_string()],
        filters: TriggerFilters {
            workspaces: Some(vec!["workspace1".to_string()]),
            node_types: Some(vec!["test:Node".to_string()]),
            paths: None,
            property_filters: None,
        },
        max_retries: None,
        workflow_data: None,
    });

    let snapshot = TriggerRegistrySnapshot::build_indexes(triggers, 1);

    // Should match indexed workspace and type
    assert!(snapshot.could_have_matches("workspace1", "test:Node"));

    // Should not match unindexed workspace and type
    assert!(!snapshot.could_have_matches("other_workspace", "other:Type"));
}

#[test]
fn test_snapshot_wildcards() {
    let mut triggers = vec![];

    // Add trigger with no workspace filter (wildcard)
    triggers.push(CachedTrigger {
        id: "trigger1".to_string(),
        function_path: Some("/functions/test".to_string()),
        trigger_name: "wildcard-trigger".to_string(),
        trigger_path: None,
        priority: 0,
        enabled: true,
        event_kinds: vec!["Created".to_string()],
        filters: TriggerFilters {
            workspaces: None, // Matches all workspaces
            node_types: Some(vec!["test:Node".to_string()]),
            paths: None,
            property_filters: None,
        },
        max_retries: None,
        workflow_data: None,
    });

    let snapshot = TriggerRegistrySnapshot::build_indexes(triggers, 1);

    // Should match any workspace due to wildcard
    assert!(snapshot.could_have_matches("any_workspace", "test:Node"));
    assert!(snapshot.could_have_matches("another_workspace", "test:Node"));
}

#[test]
fn test_get_candidates_filtering() {
    let mut triggers = vec![];

    // Trigger 1: workspace1, test:Node, Created
    triggers.push(CachedTrigger {
        id: "trigger1".to_string(),
        function_path: Some("/functions/test1".to_string()),
        trigger_name: "trigger1".to_string(),
        trigger_path: None,
        priority: 10,
        enabled: true,
        event_kinds: vec!["Created".to_string()],
        filters: TriggerFilters {
            workspaces: Some(vec!["workspace1".to_string()]),
            node_types: Some(vec!["test:Node".to_string()]),
            paths: None,
            property_filters: None,
        },
        max_retries: None,
        workflow_data: None,
    });

    // Trigger 2: workspace1, test:Node, Updated (different event)
    triggers.push(CachedTrigger {
        id: "trigger2".to_string(),
        function_path: Some("/functions/test2".to_string()),
        trigger_name: "trigger2".to_string(),
        trigger_path: None,
        priority: 5,
        enabled: true,
        event_kinds: vec!["Updated".to_string()],
        filters: TriggerFilters {
            workspaces: Some(vec!["workspace1".to_string()]),
            node_types: Some(vec!["test:Node".to_string()]),
            paths: None,
            property_filters: None,
        },
        max_retries: None,
        workflow_data: None,
    });

    let snapshot = TriggerRegistrySnapshot::build_indexes(triggers, 1);

    // Should only get trigger1 for Created event
    let candidates = snapshot.get_candidates("workspace1", "test:Node", "Created");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].trigger_name, "trigger1");

    // Should only get trigger2 for Updated event
    let candidates = snapshot.get_candidates("workspace1", "test:Node", "Updated");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].trigger_name, "trigger2");

    // Should get no candidates for Deleted event
    let candidates = snapshot.get_candidates("workspace1", "test:Node", "Deleted");
    assert_eq!(candidates.len(), 0);
}
