//! All `raisin-rocksdb` integration tests, compiled into ONE binary.
//!
//! Each former `tests/<name>.rs` is now `tests/all/<name>.rs` and appears
//! below as a module. Cargo links one test binary per target, and each binary
//! statically links the whole dependency graph (rocksdb, tantivy, candle,
//! tesseract) — so 37 separate targets cost ~6 GB of `target/` and 37 link
//! steps. One target costs one.
//!
//! Running a single test now names the module:
//!
//! ```bash
//! cargo test -p raisin-rocksdb --test all ordered_children_keyset_test
//! cargo test -p raisin-rocksdb --test all ordered_children_keyset_test::forward_paging
//! ```
//!
//! Adding a test file? Drop it in `tests/all/` and add a `mod` line here.

// Helpers are shared per-module, so unused ones in a given module are expected.
#![allow(dead_code)]

mod apply_revision_capture_test;
mod apply_revision_test;
mod branch_fork_publish_test;
mod branch_head_monotonic_test;
mod checkpoint_network_test;
mod cluster_integration_test;
mod cluster_move_node_test;
mod compare_put_vs_add;
mod create_path_uniqueness_test;
mod debug_msgpack_issue;
mod fulltext_job_store_integration;
mod hnsw_integration_test;
mod integration_tests;
mod multi_node_crdt_integration;
mod oplog_write_read_test;
mod ordered_children_keyset_test;
mod permission_resolution_tests;
mod profile_breakdown;
mod profile_detailed;
mod profile_node_creation;
mod profile_put_detailed;
mod profile_put_operation;
mod quick_100node_test;
mod reference_index_consistency_test;
mod reference_retarget_test;
mod rename_updates_node_identity_test;
mod reorder_persists_order_key_test;
mod replication_performance_test;
mod rls_integration_tests;
mod scheduled_invocation_test;
mod simple_operation_capture_test;
mod storage_5ktest;
mod subtree_document_order_test;
mod test_graph_algorithms;
mod test_nodetype_operation_msgpack;
mod test_relationships;
mod test_tombstone_handling;
mod two_node_replication_test;
