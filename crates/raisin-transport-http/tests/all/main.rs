//! All `raisin-transport-http` integration tests, compiled into ONE binary.
//!
//! Each former `tests/<name>.rs` is now `tests/all/<name>.rs` and appears below
//! as a module. Cargo links one test binary per target, and every binary
//! statically links the whole dependency graph — so N targets cost N link steps
//! and N copies in `target/`. One target costs one.
//!
//! Running a single test now names the module:
//!
//! ```bash
//! cargo test -p raisin-transport-http --test all <module>
//! cargo test -p raisin-transport-http --test all <module>::<test_name>
//! ```
//!
//! Adding a test file? Drop it in `tests/all/` and add a `mod` line here.

// Helpers are shared per-module, so unused ones in a given module are expected.
#![allow(dead_code)]

mod http_audit_logs;
mod http_branches_tags;
mod http_collisions;
mod http_comprehensive_e2e;
mod http_edges;
mod http_error_handling;
mod http_node_creation;
mod http_pagination;
mod http_reorder_copy;
mod http_revision_queries;
mod http_revisions;
mod http_sanitization;
mod http_smoke;
mod http_snapshot_branches;
mod mcp_oauth_flow;
