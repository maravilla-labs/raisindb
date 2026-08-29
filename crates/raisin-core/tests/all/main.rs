//! All `raisin-core` integration tests, compiled into ONE binary.
//!
//! Each former `tests/<name>.rs` is now `tests/all/<name>.rs` and appears below
//! as a module. Cargo links one test binary per target, and every binary
//! statically links the whole dependency graph — so N targets cost N link steps
//! and N copies in `target/`. One target costs one.
//!
//! Running a single test now names the module:
//!
//! ```bash
//! cargo test -p raisin-core --test all <module>
//! cargo test -p raisin-core --test all <module>::<test_name>
//! ```
//!
//! Adding a test file? Drop it in `tests/all/` and add a `mod` line here.

// Helpers are shared per-module, so unused ones in a given module are expected.
#![allow(dead_code)]

mod agent_auth_test;
mod branch_isolation_tests;
mod child_listing_keyset_test;
mod engine_owned_property_shield;
mod indexed_validation_test;
mod multi_tenant_integration;
mod node_service_integration;
mod phase4_tree_queries;
mod phase53_pagination;
mod throughput_baseline;
