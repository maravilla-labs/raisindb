// SPDX-License-Identifier: BSL-1.1

//! Serverless function management and invocation handlers.
//!
//! HTTP endpoints for listing functions, retrieving function metadata,
//! invoking functions, executing standalone files, running flows,
//! and inspecting execution history. The implementation bridges
//! RaisinDB's node storage with the `raisin-functions` runtime and
//! the RocksDB job system when available.

mod api_factory;
mod file_helpers;
mod flow_events;
mod helpers;
mod invoke;
mod list;
mod run_file;
mod run_flow;
pub mod types;

const TENANT_ID: &str = "default";
const DEFAULT_BRANCH: &str = "main";
const FUNCTIONS_WORKSPACE: &str = "functions";
const SYSTEM_WORKSPACE: &str = "raisin:system";

// Re-export public handler functions
pub use invoke::invoke_function;
pub use list::{get_execution, get_function, list_executions, list_functions};
pub use run_file::run_file;
pub use flow_events::stream_flow_events;
pub use run_flow::{
    cancel_flow_instance, delete_flow_instance, get_flow_instance, resume_flow, run_flow,
    run_flow_test,
};

// Re-export types used by other crates
pub use types::*;

// Re-export helpers used by webhooks, SQL, and other handlers
pub(crate) use api_factory::build_function_api;
pub(crate) use helpers::{build_loaded_function, find_function_node, load_function_code};
