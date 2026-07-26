//! All `raisin-server` integration tests, compiled into ONE binary.
//!
//! Each former `tests/<name>.rs` is now `tests/all/<name>.rs` and appears below
//! as a module. Cargo links one test binary per target, and every binary
//! statically links the whole dependency graph — so N targets cost N link steps
//! and N copies in `target/`. One target costs one.
//!
//! Running a single test now names the module:
//!
//! ```bash
//! cargo test -p raisin-server --test all <module>
//! cargo test -p raisin-server --test all <module>::<test_name>
//! ```
//!
//! Adding a test file? Drop it in `tests/all/` and add a `mod` line here.

// Helpers are shared per-module, so unused ones in a given module are expected.
#![allow(dead_code)]

mod cluster_test_utils;
mod helpers;

mod cluster_social_feed_test;
mod flow_e2e_test;
mod geospatial_test;
mod integration_node_operations;
mod mgmt_jobs_tenant_isolation_test;
mod mgmt_superadmin_test;
mod package_upload_test;
mod single_node_crud_test;
mod three_node_mesh_test;
mod two_node_replication_test;
