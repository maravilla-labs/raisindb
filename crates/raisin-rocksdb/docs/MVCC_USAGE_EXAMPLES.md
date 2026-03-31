# MVCC Time-Travel Query Usage Examples

## Overview

This document provides practical examples of using the Phase 2 MVCC time-travel features in the RocksDB storage backend.

## Basic Time-Travel Queries

### Get Node at Specific Revision

```rust
use raisin_rocksdb::RocksDBStorage;

async fn view_historical_content(storage: &RocksDBStorage) -> Result<()> {
    let nodes_repo = storage.nodes();

    // Get a node as it existed at revision 42
    let node = nodes_repo.get_at_revision(
        "tenant1",
        "repo1",
        "main",
        "draft",
        "node123",
        42
    ).await?;

    match node {
        Some(n) => println!("Node at revision 42: {:?}", n),
        None => println!("Node didn't exist or was deleted at revision 42"),
    }

    Ok(())
}
```

### Get Complete Node History

```rust
async fn view_node_history(storage: &RocksDBStorage) -> Result<()> {
    let nodes_repo = storage.nodes();

    // Get all revisions of a node (newest first)
    let history = nodes_repo.get_history(
        "tenant1",
        "repo1",
        "main",
        "draft",
        "node123",
        Some(10) // Limit to 10 most recent revisions
    ).await?;

    println!("Node History:");
    for (revision, node_opt) in history {
        match node_opt {
            Some(node) => {
                println!("  Rev {}: {} (path: {})",
                    revision, node.name, node.path);
            }
            None => {
                println!("  Rev {}: DELETED", revision);
            }
        }
    }

    Ok(())
}
```

### List Workspace at Specific Revision

```rust
async fn view_workspace_snapshot(storage: &RocksDBStorage) -> Result<()> {
    let nodes_repo = storage.nodes();

    // Get all nodes as they existed at revision 100
    let snapshot = nodes_repo.list_at_revision(
        "tenant1",
        "repo1",
        "main",
        "draft",
        100
    ).await?;

    println!("Workspace snapshot at revision 100:");
    println!("Total nodes: {}", snapshot.len());

    for node in snapshot {
        println!("  - {} ({})", node.name, node.path);
    }

    Ok(())
}
```

## Practical Use Cases

### 1. Audit Trail / Change Log

```rust
async fn generate_audit_log(
    storage: &RocksDBStorage,
    node_id: &str
) -> Result<Vec<AuditEntry>> {
    let nodes_repo = storage.nodes();
    let revisions_repo = storage.revisions();

    let history = nodes_repo.get_history(
        "tenant1",
        "repo1",
        "main",
        "draft",
        node_id,
        None // Get all revisions
    ).await?;

    let mut audit_log = Vec::new();

    for (revision, node_opt) in history {
        // Get revision metadata (author, timestamp, message)
        if let Some(meta) = revisions_repo.get_revision_meta("tenant1", "repo1", revision).await? {
            let entry = AuditEntry {
                revision,
                timestamp: meta.created_at,
                author: meta.author,
                action: match node_opt {
                    Some(node) => format!("Modified: {}", node.name),
                    None => "Deleted".to_string(),
                },
            };
            audit_log.push(entry);
        }
    }

    Ok(audit_log)
}

struct AuditEntry {
    revision: u64,
    timestamp: chrono::DateTime<chrono::Utc>,
    author: String,
    action: String,
}
```

### 2. Diff Between Revisions

```rust
async fn show_changes_between_revisions(
    storage: &RocksDBStorage,
    node_id: &str,
    old_rev: u64,
    new_rev: u64,
) -> Result<NodeDiff> {
    let nodes_repo = storage.nodes();

    let old_node = nodes_repo.get_at_revision(
        "tenant1", "repo1", "main", "draft", node_id, old_rev
    ).await?;

    let new_node = nodes_repo.get_at_revision(
        "tenant1", "repo1", "main", "draft", node_id, new_rev
    ).await?;

    match (old_node, new_node) {
        (Some(old), Some(new)) => {
            let diff = NodeDiff {
                name_changed: old.name != new.name,
                path_changed: old.path != new.path,
                properties_changed: compute_property_diff(&old.properties, &new.properties),
            };
            Ok(diff)
        }
        (Some(_), None) => {
            Ok(NodeDiff { deleted: true, ..Default::default() })
        }
        (None, Some(_)) => {
            Ok(NodeDiff { created: true, ..Default::default() })
        }
        (None, None) => {
            Err(raisin_error::Error::NotFound("Node not found at either revision".to_string()))
        }
    }
}

#[derive(Default)]
struct NodeDiff {
    created: bool,
    deleted: bool,
    name_changed: bool,
    path_changed: bool,
    properties_changed: Vec<String>,
}

fn compute_property_diff(
    old_props: &HashMap<String, PropertyValue>,
    new_props: &HashMap<String, PropertyValue>,
) -> Vec<String> {
    let mut changes = Vec::new();

    // Check for changed or removed properties
    for (key, old_val) in old_props {
        if let Some(new_val) = new_props.get(key) {
            if old_val != new_val {
                changes.push(format!("Modified: {}", key));
            }
        } else {
            changes.push(format!("Removed: {}", key));
        }
    }

    // Check for new properties
    for key in new_props.keys() {
        if !old_props.contains_key(key) {
            changes.push(format!("Added: {}", key));
        }
    }

    changes
}
```

### 3. Restore Deleted Content

```rust
async fn restore_deleted_node(
    storage: &RocksDBStorage,
    node_id: &str,
    restore_from_revision: u64,
) -> Result<()> {
    let nodes_repo = storage.nodes();

    // Get the node as it existed before deletion
    let deleted_node = nodes_repo.get_at_revision(
        "tenant1", "repo1", "main", "draft",
        node_id,
        restore_from_revision
    ).await?
        .ok_or_else(|| raisin_error::Error::NotFound(
            format!("Node {} not found at revision {}", node_id, restore_from_revision)
        ))?;

    // Re-create the node (creates new revision)
    nodes_repo.put("tenant1", "repo1", "main", "draft", deleted_node).await?;

    println!("Restored node {} from revision {}", node_id, restore_from_revision);

    Ok(())
}
```

### 4. Rollback to Previous Version

```rust
async fn rollback_node_to_revision(
    storage: &RocksDBStorage,
    node_id: &str,
    target_revision: u64,
) -> Result<()> {
    let nodes_repo = storage.nodes();

    // Get the historical version
    let historical_node = nodes_repo.get_at_revision(
        "tenant1", "repo1", "main", "draft",
        node_id,
        target_revision
    ).await?
        .ok_or_else(|| raisin_error::Error::NotFound(
            format!("Node {} not found at revision {}", node_id, target_revision)
        ))?;

    // Save it as current (creates new revision with old content)
    nodes_repo.put("tenant1", "repo1", "main", "draft", historical_node).await?;

    println!("Rolled back node {} to state at revision {}", node_id, target_revision);

    Ok(())
}
```

### 5. Point-in-Time Content Export

```rust
async fn export_workspace_at_date(
    storage: &RocksDBStorage,
    target_date: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<Node>> {
    let nodes_repo = storage.nodes();
    let revisions_repo = storage.revisions();

    // Find the revision closest to the target date
    let all_revisions = revisions_repo.list_revisions(
        "tenant1", "repo1", 1000, 0
    ).await?;

    let target_revision = all_revisions
        .iter()
        .filter(|meta| meta.created_at <= target_date)
        .max_by_key(|meta| meta.created_at)
        .map(|meta| meta.revision)
        .ok_or_else(|| raisin_error::Error::NotFound(
            "No revisions found before target date".to_string()
        ))?;

    // Get workspace snapshot at that revision
    let snapshot = nodes_repo.list_at_revision(
        "tenant1", "repo1", "main", "draft",
        target_revision
    ).await?;

    println!("Exported {} nodes as of {} (revision {})",
        snapshot.len(), target_date, target_revision);

    Ok(snapshot)
}
```

### 6. Compare Two Workspace Snapshots

```rust
async fn compare_workspace_states(
    storage: &RocksDBStorage,
    revision_a: u64,
    revision_b: u64,
) -> Result<WorkspaceDiff> {
    let nodes_repo = storage.nodes();

    let snapshot_a = nodes_repo.list_at_revision(
        "tenant1", "repo1", "main", "draft", revision_a
    ).await?;

    let snapshot_b = nodes_repo.list_at_revision(
        "tenant1", "repo1", "main", "draft", revision_b
    ).await?;

    let nodes_a: HashMap<String, Node> = snapshot_a
        .into_iter()
        .map(|n| (n.id.clone(), n))
        .collect();

    let nodes_b: HashMap<String, Node> = snapshot_b
        .into_iter()
        .map(|n| (n.id.clone(), n))
        .collect();

    let mut diff = WorkspaceDiff::default();

    // Find added nodes
    for (id, node) in &nodes_b {
        if !nodes_a.contains_key(id) {
            diff.added.push(node.clone());
        }
    }

    // Find removed nodes
    for (id, node) in &nodes_a {
        if !nodes_b.contains_key(id) {
            diff.removed.push(node.clone());
        }
    }

    // Find modified nodes
    for (id, node_a) in &nodes_a {
        if let Some(node_b) = nodes_b.get(id) {
            if node_a != node_b {
                diff.modified.push((node_a.clone(), node_b.clone()));
            }
        }
    }

    Ok(diff)
}

#[derive(Default)]
struct WorkspaceDiff {
    added: Vec<Node>,
    removed: Vec<Node>,
    modified: Vec<(Node, Node)>, // (old, new)
}
```

## Best Practices

### 1. Limit History Queries
```rust
// Good: Use limit to avoid loading entire history
let recent_history = nodes_repo.get_history(
    tenant_id, repo_id, branch, workspace,
    node_id,
    Some(20) // Last 20 revisions
).await?;

// Avoid: Loading unbounded history for long-lived nodes
let all_history = nodes_repo.get_history(
    tenant_id, repo_id, branch, workspace,
    node_id,
    None // Could be thousands of revisions!
).await?;
```

### 2. Cache Revision Metadata
```rust
// Cache revision metadata to avoid repeated lookups
let mut revision_cache: HashMap<u64, RevisionMeta> = HashMap::new();

for (revision, _) in history {
    if !revision_cache.contains_key(&revision) {
        if let Some(meta) = revisions_repo.get_revision_meta(
            tenant_id, repo_id, revision
        ).await? {
            revision_cache.insert(revision, meta);
        }
    }
}
```

### 3. Use Tombstones Appropriately
```rust
// History queries include tombstones to show deletions
let history = nodes_repo.get_history(...).await?;
for (rev, node_opt) in history {
    match node_opt {
        Some(node) => {
            // Node existed at this revision
        }
        None => {
            // Node was deleted at this revision - important for audit!
        }
    }
}

// Regular queries filter tombstones
let current = nodes_repo.get(...).await?; // Returns None for deleted nodes
```

### 4. Batch Operations for Snapshots
```rust
// Efficient: Single query for workspace snapshot
let snapshot = nodes_repo.list_at_revision(
    tenant_id, repo_id, branch, workspace,
    target_revision
).await?;

// Inefficient: Individual queries per node
for node_id in all_node_ids {
    let node = nodes_repo.get_at_revision(
        tenant_id, repo_id, branch, workspace,
        &node_id,
        target_revision
    ).await?;
}
```

## Error Handling

```rust
async fn safe_time_travel_query(
    storage: &RocksDBStorage,
    node_id: &str,
    revision: u64,
) -> Result<Option<Node>> {
    let nodes_repo = storage.nodes();

    match nodes_repo.get_at_revision(
        "tenant1", "repo1", "main", "draft",
        node_id,
        revision
    ).await {
        Ok(node) => Ok(node),
        Err(e) if e.is_not_found() => {
            // Node never existed at any revision
            tracing::warn!("Node {} not found in any revision", node_id);
            Ok(None)
        }
        Err(e) => {
            // Storage error
            tracing::error!("Failed to query node {}: {}", node_id, e);
            Err(e)
        }
    }
}
```

## Performance Considerations

### Query Complexity

- `get_at_revision()`: O(log N) where N = revisions of node
- `get_history()`: O(N) where N = revisions (with limit)
- `list_at_revision()`: O(M * R) where M = total nodes, R = avg revisions

### Optimization Tips

1. **Use limits** on history queries
2. **Cache revision metadata** for repeated access
3. **Batch snapshot queries** instead of individual node queries
4. **Consider workspace snapshots** for periodic archiving
5. **Index revision numbers** for date-based queries

### Storage Impact

Each revision creates:
- 1 versioned node entry
- 1 versioned path index entry
- N property index entries (N = number of properties)
- M reference index entries (M = number of references)

Plan for ~3-4x storage overhead for revision history.