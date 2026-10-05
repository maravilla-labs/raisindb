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

mod batched_fetch_tests;
mod bulk_sql_rls;
mod compound_index_hierarchy;
mod count_scan_rls;
mod created_event_nonsystem;
mod current_user_gating;
mod delta_restore_tests;
mod dml_index_and_predicate_maintenance;
mod editorial_order_tests;
mod group_by_json_extraction;
mod hash_join_integration_tests;
mod hybrid_search_query_embedder;
mod hybrid_search_workspace;
mod immutable_nodetype_dml_test;
mod index_read_bench;
mod index_read_bench_paths;
mod index_read_bench_point;
mod index_read_bench_point_resolve;
mod index_read_bench_resolve;
mod index_read_bench_writes;
mod is_distinct_from;
mod join_property_tests;
mod limit_pushdown_tests;
mod localized_name_uniqueness;
mod localized_paths;
mod locks_sql_test;
mod move_copy_order_at_root;
mod mvcc_index_oracle;
mod pagination_navigation_tests;
mod param_plan_cache_invalidation;
mod param_plan_cache_tests;
mod path_like_prefix_scan;
mod pgq_paths_rocksdb;
mod pgq_rls;
mod plan_cache_tests;
mod point_read_shape_tests;
mod profiling_test;
mod property_index_bounded_tests;
mod property_index_limit_tests;
mod rag_retrieval_sql;
mod references_compose_tests;
mod references_integration_tests;
mod regex_and_quantified_ops;
mod resolve_frontier_tests;
mod resolve_json_tests;
mod resolve_rls_tests;
mod restore_workspace;
mod rocksdb_integration_tests;
mod scalar_select_clauses;
mod search_table_function_rls;
mod spatial_pushdown_tests;
mod throughput_sql;
mod translation_blocks;
mod translation_roundtrip;
mod translation_throughput;
mod vector_of_similarity;
