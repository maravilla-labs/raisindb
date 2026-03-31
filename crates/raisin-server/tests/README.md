# RaisinDB Cluster Integration Tests

> **TODO**: Review and update this documentation to ensure accuracy with current implementation.

Comprehensive integration test suite for testing a 3-node RaisinDB cluster using public REST and WebSocket APIs.

## Overview

This test suite provides modular, reusable utilities for testing RaisinDB cluster replication, consistency, and child ordering across multiple nodes. All tests interact with the cluster through the public HTTP REST API exposed by `raisin-server` binaries.

## Architecture

### Module Structure

```
tests/
├── cluster_test_utils/           # Reusable test utilities
│   ├── mod.rs                    # Module exports
│   ├── ports.rs                  # Port allocation (50 lines)
│   ├── config.rs                 # Node & cluster configuration (150 lines)
│   ├── process.rs                # Process management (200 lines)
│   ├── rest_client.rs            # REST API client (300 lines)
│   ├── websocket_client.rs       # WebSocket client (placeholder)
│   ├── verification.rs           # Consistency checks (350 lines)
│   ├── social_feed.rs            # Demo schema initialization (250 lines)
│   └── fixture.rs                # Test fixture setup (150 lines)
└── cluster_social_feed_test.rs   # Comprehensive test cases (700 lines)
```

See full content in the file for complete documentation.
