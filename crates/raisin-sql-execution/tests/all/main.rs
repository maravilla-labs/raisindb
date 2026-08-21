//! All `raisin-sql-execution` integration tests, compiled into ONE binary.
//!
//! Each former `tests/<name>.rs` is now `tests/all/<name>.rs` and appears below
//! as a module. Cargo links one test binary per target, and every binary
//! statically links the whole dependency graph — so N targets cost N link steps
//! and N copies in `target/`. One target costs one.
//!
//! Running a single test now names the module:
//!
//! ```bash
//! cargo test -p raisin-sql-execution --test all <module>
//! cargo test -p raisin-sql-execution --test all <module>::<test_name>
//! ```
//!
//! Adding a test file? Drop it in `tests/all/` and add a `mod` line here.

// Helpers are shared per-module, so unused ones in a given module are expected.
#![allow(dead_code)]

mod bulk_sql_rls;
mod compound_index_hierarchy;
mod created_event_nonsystem;
mod current_user_gating;
mod dml_index_and_predicate_maintenance;
mod editorial_order_tests;
mod group_by_json_extraction;
mod hash_join_integration_tests;
mod is_distinct_from;
mod join_property_tests;
mod limit_pushdown_tests;
mod locks_sql_test;
mod move_copy_order_at_root;
mod pagination_navigation_tests;
mod path_like_prefix_scan;
mod pgq_paths_rocksdb;
mod pgq_rls;
mod profiling_test;
mod references_compose_tests;
mod references_integration_tests;
mod restore_workspace;
mod rocksdb_integration_tests;
mod spatial_pushdown_tests;
mod throughput_sql;
mod translation_blocks;
mod translation_roundtrip;
mod translation_throughput;
