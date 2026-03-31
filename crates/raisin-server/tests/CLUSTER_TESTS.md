# RaisinDB 3-Node Cluster Integration Test Suite

> **TODO**: Review and update this documentation to ensure accuracy with current implementation.

Comprehensive, modular integration test suite for testing RaisinDB cluster replication, consistency, and child ordering.

## Quick Start

### Build the Server

```bash
cargo build --package raisin-server --features storage-rocksdb
```

### Run All Tests

```bash
cargo test --package raisin-server --test cluster_social_feed_test -- --ignored --nocapture
```

### Run a Specific Test

```bash
cargo test --package raisin-server --test cluster_social_feed_test test_add_post_node1 -- --ignored --nocapture
```

## What This Test Suite Does

This test suite:
1. Spawns 3 fresh RaisinDB server processes
2. Configures them as a replication cluster
3. Initializes a social feed schema (users, posts, comments)
4. Tests CRUD operations across nodes
5. Verifies natural child order consistency (from fragmented index)
6. Tests replication correctness
7. Cleans up or preserves data based on test results

## Module Overview

```
tests/cluster_test_utils/
├── ports.rs           - Port allocation utilities
├── config.rs          - Node and cluster configuration
├── process.rs         - Process lifecycle management
├── rest_client.rs     - REST API client
├── websocket_client.rs - WebSocket client (placeholder)
├── verification.rs    - Consistency verification functions
├── social_feed.rs     - Social feed schema initialization
└── fixture.rs         - Complete test fixture setup
```

## Test Cases

### 1. Cluster Initialization
Verifies basic cluster setup and schema replication.

### 2-4. Natural Order Tests (Node 1, 2, 3)
Creates posts on each node and verifies child order consistency across the cluster.

**Key Principle**: The "natural order" comes from the **fragmented index**, NOT from ORDER BY clauses.

### 5. Cross-Node Updates
Updates a node on a different cluster node than where it was created, verifies property consistency.

### 6. Relations Replication
Adds relations between nodes and verifies they replicate correctly.

### 7. Deletion and Order Updates
Deletes a node and verifies order remains consistent after replication.

### 8. Stress Test
Creates 9 posts rapidly across all nodes, verifies:
- REST API order consistency
- SQL query (without ORDER BY) matches REST order

### 9. WebSocket Events (Placeholder)
Placeholder for WebSocket event streaming tests.

## Key Verification Functions

### `verify_child_order_via_rest()`
Fetches children from all 3 nodes and verifies identical ID sequences.

### `verify_child_order_via_sql()`
Executes SQL query WITHOUT ORDER BY and verifies it matches expected order.

### `verify_node_exists_on_all_nodes()`
Checks that a node has replicated to all cluster members.

### `verify_node_properties_match()`
Verifies node properties are identical across all nodes.

### `wait_for_replication()`
Polls until a node appears on all cluster members (with timeout).

## ClusterTestFixture

The `ClusterTestFixture` provides a ready-to-use cluster:

```rust
let fixture = ClusterTestFixture::setup().await?;

// fixture provides:
// - fixture.cluster (3 running processes)
// - fixture.client (REST client)
// - fixture.tokens (JWT tokens for all nodes)
// - fixture.user_ids (demo user IDs)
// - fixture.user_paths (demo user paths)
// - fixture.post_ids (demo post IDs)
// - fixture.post_paths (demo post paths)

// Run your tests...

fixture.teardown(); // Cleanup on success
```

## Data Management

### Success Case
- `teardown()` cleans up all data directories
- Processes are gracefully shutdown

### Failure Case
- `teardown_on_failure()` preserves data directories
- Prints paths for debugging:
  ```
  Preserving data directories for debugging:
    node1: /tmp/.tmpXYZ/node1_data
    node2: /tmp/.tmpABC/node2_data
    node3: /tmp/.tmpDEF/node3_data
  ```

## Adding New Tests

```rust
#[tokio::test]
#[ignore]
async fn test_my_feature() {
    println!("\n=== Test: My Feature ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    // Your test logic here
    // Use fixture.client, fixture.tokens, etc.

    // Verify consistency
    verify_child_order_via_rest(
        &fixture.client,
        &fixture.tokens,
        fixture.repo(),
        fixture.branch(),
        fixture.workspace(),
        "parent_path",
    )
    .await
    .expect("Order verification failed");

    println!("\n✅ Test passed\n");
    fixture.teardown();
}
```

## Troubleshooting

### "Binary not found"
Build the server first:
```bash
cargo build --package raisin-server --features storage-rocksdb
```

### "Address already in use"
- Tests allocate random ports to avoid conflicts
- Wait a moment for ports to be released
- Kill any stale processes: `pkill raisin-server`

### Timeout waiting for health
- Check for stale processes: `ps aux | grep raisin-server`
- Increase timeout in cluster setup
- Check logs with: `RUST_LOG=debug cargo test ...`

### Replication not working
- Enable replication logs: `RUST_LOG=raisin_replication=debug`
- Check firewall settings
- Verify data directories are writable

### Order mismatches
- Check debug output from `dump_children_order()`
- Ensure all operations have fully replicated
- Look for concurrent update races

## Architecture Highlights

### Modular Design
Each utility module is independent and testable:
- Can be used in other test suites
- Clear single responsibility
- Well-documented public APIs

### Error Handling
- Uses `anyhow::Result` throughout
- Provides context with `.context()`
- Detailed error messages for debugging

### Process Management
- Graceful shutdown with `Drop` trait
- Automatic cleanup on test completion
- Preserve-on-failure for debugging

### Natural Order Testing
- Verifies fragmented index ordering
- Compares REST API vs SQL results
- Tests consistency across all nodes

## Dependencies Added

```toml
[dev-dependencies]
reqwest = { workspace = true, features = ["json"] }
tempfile = "3.8"
chrono = { workspace = true }
tokio-tungstenite = "0.21"
serde_json = { workspace = true }
anyhow = { workspace = true }
```

## Files Created

1. `cluster_test_utils/mod.rs` - Module declarations and exports
2. `cluster_test_utils/ports.rs` - Port allocation
3. `cluster_test_utils/config.rs` - Configuration management
4. `cluster_test_utils/process.rs` - Process lifecycle
5. `cluster_test_utils/rest_client.rs` - REST API client
6. `cluster_test_utils/websocket_client.rs` - WebSocket client (placeholder)
7. `cluster_test_utils/verification.rs` - Verification utilities
8. `cluster_test_utils/social_feed.rs` - Schema initialization
9. `cluster_test_utils/fixture.rs` - Test fixture
10. `cluster_social_feed_test.rs` - Test cases

Total: ~2000+ lines of well-documented, idiomatic Rust code.

## Future Enhancements

- [ ] Full WebSocket client implementation
- [ ] Network partition simulation
- [ ] Snapshot/restore testing
- [ ] Conflict resolution scenarios
- [ ] Performance benchmarks
- [ ] Large dataset stress tests (10K+ nodes)
- [ ] Node failure and recovery testing

## Best Practices

1. **Always use the fixture**: `ClusterTestFixture::setup()` provides a consistent starting point
2. **Wait for replication**: Use `wait_for_replication()` before verification
3. **Verify on all nodes**: Don't just check one node - verify consistency
4. **Clean up properly**: Call `teardown()` on success, `teardown_on_failure()` on error
5. **Print progress**: Use `println!()` to show test progress (helpful with `--nocapture`)
6. **Test on different nodes**: Create/update on different nodes to test cross-node operations

## License

Licensed under the Business Source License 1.1 (BSL-1.1).
