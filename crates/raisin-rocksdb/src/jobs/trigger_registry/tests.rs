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
fn test_scoped_quick_reject_fails_open_for_other_scopes() {
    // Regression for the lost-continuation hang (production chat-5494d4d7):
    // the global snapshot is loaded from ONE (tenant, repo, branch). After an
    // invalidation from repo B, events from repo A were quick-rejected against
    // B's trigger shapes and silently dropped — the agent-continue trigger
    // never fired and the conversation hung forever. Events from a scope other
    // than the snapshot's must fail OPEN.
    let triggers = vec![CachedTrigger {
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
    }];

    let mut snapshot = TriggerRegistrySnapshot::build_indexes(triggers, 1);
    snapshot.scope = Some((
        "tenant-b".to_string(),
        "repo-b".to_string(),
        "main".to_string(),
    ));

    // Same scope: quick-reject is authoritative.
    assert!(snapshot.could_have_matches_scoped(
        "tenant-b",
        "repo-b",
        "main",
        "workspace1",
        "test:Node"
    ));
    assert!(!snapshot.could_have_matches_scoped(
        "tenant-b",
        "repo-b",
        "main",
        "ai",
        "raisin:AIToolResult"
    ));

    // Different repo: MUST fail open even though the shape is not indexed.
    assert!(snapshot.could_have_matches_scoped(
        "tenant-b",
        "repo-a",
        "main",
        "ai",
        "raisin:AIToolResult"
    ));
    // Different tenant and branch likewise.
    assert!(snapshot.could_have_matches_scoped(
        "tenant-a",
        "repo-b",
        "main",
        "ai",
        "raisin:AIToolResult"
    ));
    assert!(snapshot.could_have_matches_scoped(
        "tenant-b",
        "repo-b",
        "dev",
        "ai",
        "raisin:AIToolResult"
    ));

    // Unscoped snapshot (never loaded): fail open for everything.
    let empty = TriggerRegistrySnapshot::empty();
    assert!(empty.could_have_matches_scoped("t", "r", "b", "ai", "raisin:AIToolResult"));
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

// ───────────────────────── Per-scope snapshot cache ─────────────────────────

mod snapshot_cache {
    use super::super::registry::{ScopeKey, SnapshotCache};
    use super::super::snapshot::TriggerRegistrySnapshot;
    use std::sync::Arc;
    use std::time::Duration;

    fn key(t: &str, r: &str, b: &str) -> ScopeKey {
        (t.to_string(), r.to_string(), b.to_string())
    }

    fn snapshot(version: u64) -> Arc<TriggerRegistrySnapshot> {
        Arc::new(TriggerRegistrySnapshot::build_indexes(vec![], version))
    }

    #[test]
    fn fresh_snapshot_is_served_per_scope() {
        let cache = SnapshotCache::new(Duration::from_secs(300), 16);
        cache.insert(key("t1", "repo-a", "main"), snapshot(1));
        cache.insert(key("t1", "repo-b", "main"), snapshot(7));

        // Each scope sees only its own snapshot.
        assert_eq!(
            cache
                .get_fresh(&key("t1", "repo-a", "main"))
                .unwrap()
                .version,
            1
        );
        assert_eq!(
            cache
                .get_fresh(&key("t1", "repo-b", "main"))
                .unwrap()
                .version,
            7
        );
        // Inserting repo-b must NOT evict or replace repo-a (the old global
        // single-snapshot design did exactly that).
        assert!(cache.get_fresh(&key("t1", "repo-a", "main")).is_some());

        // Unknown scope: no snapshot -> caller fails open.
        assert!(cache.get_fresh(&key("t2", "repo-a", "main")).is_none());
    }

    #[test]
    fn stale_snapshot_is_not_served() {
        // TTL of zero: everything is immediately stale.
        let cache = SnapshotCache::new(Duration::from_millis(0), 16);
        cache.insert(key("t1", "repo-a", "main"), snapshot(1));
        std::thread::sleep(Duration::from_millis(5));
        assert!(
            cache.get_fresh(&key("t1", "repo-a", "main")).is_none(),
            "stale snapshot must not be served (caller fails open)"
        );
        // version_for still sees the stored entry so reloads stay monotonic.
        assert_eq!(cache.version_for(&key("t1", "repo-a", "main")), 1);
    }

    #[test]
    fn version_is_monotonic_per_scope() {
        let cache = SnapshotCache::new(Duration::from_secs(300), 16);
        assert_eq!(cache.version_for(&key("t1", "r", "main")), 0);
        cache.insert(key("t1", "r", "main"), snapshot(1));
        assert_eq!(cache.version_for(&key("t1", "r", "main")), 1);
        cache.insert(key("t1", "r", "main"), snapshot(2));
        assert_eq!(cache.version_for(&key("t1", "r", "main")), 2);
        // Other scopes keep independent version counters.
        assert_eq!(cache.version_for(&key("t2", "r", "main")), 0);
    }

    #[test]
    fn eviction_respects_bound_and_keeps_newest() {
        let cache = SnapshotCache::new(Duration::from_secs(300), 3);
        cache.insert(key("t", "r1", "main"), snapshot(1));
        std::thread::sleep(Duration::from_millis(2));
        cache.insert(key("t", "r2", "main"), snapshot(1));
        std::thread::sleep(Duration::from_millis(2));
        cache.insert(key("t", "r3", "main"), snapshot(1));
        std::thread::sleep(Duration::from_millis(2));

        // Fourth scope: bound is 3, the oldest (r1) must be evicted.
        cache.insert(key("t", "r4", "main"), snapshot(1));
        assert_eq!(cache.len(), 3);
        assert!(!cache.contains(&key("t", "r1", "main")), "oldest evicted");
        assert!(cache.contains(&key("t", "r4", "main")), "newest kept");

        // Replacing an existing scope never evicts others.
        cache.insert(key("t", "r4", "main"), snapshot(2));
        assert_eq!(cache.len(), 3);
        assert!(cache.contains(&key("t", "r2", "main")));
        assert!(cache.contains(&key("t", "r3", "main")));
    }
}
