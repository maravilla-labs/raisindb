//! All `raisin-agent-runtime` integration tests, compiled into ONE binary.
//!
//! ```bash
//! cargo test -p raisin-agent-runtime --test all
//! cargo test -p raisin-agent-runtime --test all <module>
//! ```

// Helpers are shared per-module, so unused ones in a given module are expected.
#![allow(dead_code)]

mod common;

mod admission;
mod cancellation;
mod checkpoints;
mod children;
mod children_ctl;
mod domain;
mod domain_finalize;
mod fencing;
mod host_api;
mod invariants;
mod lease;
mod resume;
mod sequencing;
mod steering;
mod waiter;
