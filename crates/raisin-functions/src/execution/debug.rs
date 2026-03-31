// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Debug handlers that print job events in color.
//!
//! These handlers are used during development and refactoring to verify
//! that the job system is correctly routing events. They print colored
//! output to stderr and return stub results.
//!
//! # Color Coding
//!
//! - **Cyan**: SQL execution events
//! - **Green**: Function execution events
//! - **Yellow**: Function enabled checks
//! - **Magenta**: Binary retrieval events
//! - **Blue**: FunctionContext info
//! - **Red**: Errors

use super::types::{
    BinaryRetrievalCallback, FunctionContext, FunctionEnabledChecker, FunctionExecutionResult,
    FunctionExecutorCallback, SqlExecutorCallback,
};
use crate::types::NodeEventKind;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::sync::Arc;

// ANSI escape codes for colored output (no external dependency needed)
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const DIM: &str = "\x1b[2m";

/// Helper to parse event type from input
fn parse_event_type(event_type_str: &str) -> Option<NodeEventKind> {
    match event_type_str {
        "Created" => Some(NodeEventKind::Created),
        "Updated" => Some(NodeEventKind::Updated),
        "Deleted" => Some(NodeEventKind::Deleted),
        "Published" => Some(NodeEventKind::Published),
        "Unpublished" => Some(NodeEventKind::Unpublished),
        "Moved" => Some(NodeEventKind::Moved),
        "Renamed" => Some(NodeEventKind::Renamed),
        _ => None,
    }
}

/// Parse flow_input from the function input to extract event information
/// Supports both flow_input wrapper format and direct input format for backwards compatibility
fn parse_flow_input(input: &serde_json::Value) -> Option<(NodeEventKind, String, String, String)> {
    // Try flow_input wrapper first (new standardized format)
    let event_source = input.get("flow_input").or(Some(input))?; // Fall back to direct input format

    let event = event_source.get("event")?;
    let workspace = event_source.get("workspace")?.as_str()?;

    let event_type_str = event.get("type")?.as_str()?;
    let node_id = event.get("node_id")?.as_str()?;
    let node_type = event.get("node_type")?.as_str()?;

    let event_type = parse_event_type(event_type_str)?;

    Some((
        event_type,
        node_id.to_string(),
        node_type.to_string(),
        workspace.to_string(),
    ))
}

/// Parse entry_file property and resolve the full path to the entry file.
///
/// Entry file format: `"filename:function_name"` (e.g., `"index.js:handler"`)
///
/// # Arguments
/// * `function_path` - The path of the function node (e.g., `/lib/raisin/agent-handler`)
/// * `entry_file` - The entry_file property value (e.g., `"index.js:handleUserMessage"`)
///
/// # Returns
/// A tuple of (resolved_path, function_name)
/// e.g., (`"/lib/raisin/agent-handler/index.js"`, `"handleUserMessage"`)
fn resolve_entry_file(function_path: &str, entry_file: &str) -> (String, String) {
    use std::path::{Component, Path, PathBuf};

    // Parse entry_file into file path and function name
    let (file_path, function_name) = if let Some((path, func)) = entry_file.rsplit_once(':') {
        (path.trim(), func.trim())
    } else {
        // Backward compat: if no colon, assume "index.js" and entry_file is function name
        ("index.js", entry_file.trim())
    };

    // Remove legacy "functions:" prefix if present
    let file_path = file_path.strip_prefix("functions:").unwrap_or(file_path);

    // Resolve path
    let resolved_path = if file_path.starts_with('/') {
        // Absolute path - use as-is
        file_path.to_string()
    } else {
        // Relative path - join with function directory and normalize
        let joined = Path::new(function_path).join(file_path);

        // Normalize path (resolve . and .. components)
        let normalized: PathBuf = joined.components().fold(PathBuf::new(), |mut acc, comp| {
            match comp {
                Component::ParentDir => {
                    acc.pop();
                }
                Component::CurDir => {}
                c => acc.push(c),
            }
            acc
        });

        normalized.to_string_lossy().to_string()
    };

    (resolved_path, function_name.to_string())
}

/// Create a debug SQL executor callback
///
/// Prints SQL execution events in cyan and returns 0 affected rows.
pub fn create_debug_sql_executor() -> SqlExecutorCallback {
    Arc::new(
        move |sql: String, tenant_id: String, repo_id: String, branch: String, actor: String| {
            Box::pin(async move {
                eprintln!(
                    "\n{BOLD}{CYAN}[SQL_EXEC]{RESET} {CYAN}tenant={} repo={} branch={}{RESET}",
                    tenant_id, repo_id, branch
                );
                eprintln!("  {CYAN}actor={}{RESET}", actor);
                eprintln!("  {DIM}{CYAN}sql={}{RESET}", truncate_string(&sql, 200));
                eprintln!("  {DIM}{CYAN}(debug stub - returning 0 affected rows){RESET}\n");

                Ok(0i64)
            })
        },
    )
}

/// The workspace where functions are stored
const FUNCTIONS_WORKSPACE: &str = "functions";

/// Create a debug function executor callback with storage access
///
/// Prints function execution events in green and returns a stub result.
/// Fetches the function node and triggering node from storage.
pub fn create_debug_function_executor_with_storage<S>(storage: Arc<S>) -> FunctionExecutorCallback
where
    S: Storage + 'static,
{
    Arc::new(
        move |function_path: String,
              execution_id: String,
              input: serde_json::Value,
              tenant_id: String,
              repo_id: String,
              branch: String,
              workspace: String,
              auth_context: Option<raisin_models::auth::AuthContext>,
              _log_emitter: Option<raisin_storage::LogEmitter>| {
            let storage = storage.clone();
            Box::pin(async move {
                // Log auth context info
                if let Some(ref auth) = auth_context {
                    eprintln!(
                        "  {GREEN}auth_context=user_id:{:?}, roles:{:?}{RESET}",
                        auth.user_id, auth.roles
                    );
                } else {
                    eprintln!("  {GREEN}auth_context=system (no RLS){RESET}");
                }
                eprintln!(
                    "\n{BOLD}{GREEN}[FUNCTION_EXEC]{RESET} {GREEN}tenant={} repo={} branch={}{RESET}",
                    tenant_id, repo_id, branch
                );
                eprintln!("  {GREEN}workspace={}{RESET}", workspace);
                eprintln!("  {GREEN}path={}{RESET}", function_path);
                eprintln!("  {GREEN}execution_id={}{RESET}", execution_id);
                eprintln!(
                    "  {GREEN}input={}{RESET}",
                    serde_json::to_string_pretty(&input).unwrap_or_else(|_| "null".to_string())
                );

                // Fetch the function node from the functions workspace
                eprintln!("\n{BOLD}{MAGENTA}[FUNCTION_NODE]{RESET}");
                let function_node = storage
                    .nodes()
                    .get_by_path(
                        StorageScope::new(&tenant_id, &repo_id, &branch, FUNCTIONS_WORKSPACE),
                        &function_path,
                        None,
                    )
                    .await;

                match &function_node {
                    Ok(Some(node)) => {
                        eprintln!("  {MAGENTA}id={}{RESET}", node.id);
                        eprintln!("  {MAGENTA}name={}{RESET}", node.name);
                        eprintln!("  {MAGENTA}path={}{RESET}", node.path);
                        eprintln!("  {MAGENTA}node_type={}{RESET}", node.node_type);
                        eprintln!(
                            "  {MAGENTA}properties={}{RESET}",
                            serde_json::to_string_pretty(&node.properties)
                                .unwrap_or_else(|_| "{}".to_string())
                        );

                        // Parse and resolve entry_file path
                        if let Some(raisin_models::nodes::properties::PropertyValue::String(
                            entry_file,
                        )) = node.properties.get("entry_file")
                        {
                            let (entry_path, entry_function) =
                                resolve_entry_file(&function_path, entry_file);
                            eprintln!("  {MAGENTA}entry_file={}{RESET}", entry_file);
                            eprintln!("  {MAGENTA}entry_path={}{RESET}", entry_path);
                            eprintln!("  {MAGENTA}entry_function={}{RESET}", entry_function);
                        }
                    }
                    Ok(None) => {
                        eprintln!(
                            "  {DIM}{MAGENTA}function_node=<not found at path: {}>{RESET}",
                            function_path
                        );
                    }
                    Err(e) => {
                        eprintln!("  {DIM}{MAGENTA}function_node=<error: {}>{RESET}", e);
                    }
                }

                // Parse flow_input and build FunctionContext
                if let Some((event_type, node_id, node_type, event_workspace)) =
                    parse_flow_input(&input)
                {
                    // Fetch the triggering node from storage
                    let node = storage
                        .nodes()
                        .get(
                            StorageScope::new(&tenant_id, &repo_id, &branch, &event_workspace),
                            &node_id,
                            None,
                        )
                        .await
                        .ok()
                        .flatten();

                    let context = FunctionContext {
                        event_type,
                        node_id: node_id.clone(),
                        node_type,
                        workspace: event_workspace,
                        execution_id: execution_id.clone(),
                        node,
                    };

                    eprintln!("\n{BOLD}{BLUE}[FUNCTION_CONTEXT]{RESET}");
                    eprintln!("  {BLUE}event_type={}{RESET}", context.event_type);
                    eprintln!("  {BLUE}node_id={}{RESET}", context.node_id);
                    eprintln!("  {BLUE}node_type={}{RESET}", context.node_type);
                    eprintln!("  {BLUE}workspace={}{RESET}", context.workspace);
                    eprintln!("  {BLUE}execution_id={}{RESET}", context.execution_id);
                    if let Some(ref node) = context.node {
                        eprintln!("  {BLUE}node.name={}{RESET}", node.name);
                        eprintln!("  {BLUE}node.path={}{RESET}", node.path);
                        eprintln!(
                            "  {BLUE}node.properties={}{RESET}",
                            serde_json::to_string_pretty(&node.properties)
                                .unwrap_or_else(|_| "{}".to_string())
                        );
                    } else {
                        eprintln!("  {DIM}{BLUE}node=<not found>{RESET}");
                    }
                } else {
                    eprintln!(
                        "  {DIM}{GREEN}(no flow_input in input - cannot build FunctionContext){RESET}"
                    );
                }

                eprintln!("  {DIM}{GREEN}(debug stub - returning success){RESET}\n");

                Ok(FunctionExecutionResult {
                    execution_id,
                    success: true,
                    result: Some(serde_json::json!({
                        "debug": true,
                        "message": "Stub function execution - debug mode enabled"
                    })),
                    error: None,
                    duration_ms: 0,
                    logs: vec!["[debug] Stub function execution".to_string()],
                })
            })
        },
    )
}

/// Create a debug function executor callback (without storage access)
///
/// Prints function execution events in green and returns a stub result.
/// Does not fetch node data - use `create_debug_function_executor_with_storage` for full context.
pub fn create_debug_function_executor() -> FunctionExecutorCallback {
    Arc::new(
        move |function_path: String,
              execution_id: String,
              input: serde_json::Value,
              tenant_id: String,
              repo_id: String,
              branch: String,
              workspace: String,
              auth_context: Option<raisin_models::auth::AuthContext>,
              _log_emitter: Option<raisin_storage::LogEmitter>| {
            Box::pin(async move {
                eprintln!(
                    "\n{BOLD}{GREEN}[FUNCTION_EXEC]{RESET} {GREEN}tenant={} repo={} branch={}{RESET}",
                    tenant_id, repo_id, branch
                );
                eprintln!("  {GREEN}workspace={}{RESET}", workspace);
                eprintln!("  {GREEN}path={}{RESET}", function_path);
                eprintln!("  {GREEN}execution_id={}{RESET}", execution_id);
                // Log auth context info
                if let Some(ref auth) = auth_context {
                    eprintln!(
                        "  {GREEN}auth_context=user_id:{:?}, roles:{:?}{RESET}",
                        auth.user_id, auth.roles
                    );
                } else {
                    eprintln!("  {GREEN}auth_context=system (no RLS){RESET}");
                }
                eprintln!(
                    "  {GREEN}input={}{RESET}",
                    serde_json::to_string_pretty(&input).unwrap_or_else(|_| "null".to_string())
                );
                eprintln!("  {DIM}{GREEN}(debug stub - returning success){RESET}\n");

                Ok(FunctionExecutionResult {
                    execution_id,
                    success: true,
                    result: Some(serde_json::json!({
                        "debug": true,
                        "message": "Stub function execution - debug mode enabled"
                    })),
                    error: None,
                    duration_ms: 0,
                    logs: vec!["[debug] Stub function execution".to_string()],
                })
            })
        },
    )
}

/// Create a debug function enabled checker callback
///
/// Prints function enabled check events in yellow and returns true (enabled).
pub fn create_debug_enabled_checker() -> FunctionEnabledChecker {
    Arc::new(
        move |function_path: String,
              tenant_id: String,
              repo_id: String,
              branch: String,
              workspace: String| {
            Box::pin(async move {
                eprintln!(
                    "\n{BOLD}{YELLOW}[FUNCTION_CHECK]{RESET} {YELLOW}tenant={} repo={} branch={}{RESET}",
                    tenant_id, repo_id, branch
                );
                eprintln!("  {YELLOW}workspace={}{RESET}", workspace);
                eprintln!("  {YELLOW}path={}{RESET}", function_path);
                eprintln!("  {DIM}{YELLOW}(debug stub - returning enabled=true){RESET}\n");

                Ok(true)
            })
        },
    )
}

/// Create a debug binary retrieval callback
///
/// Prints binary retrieval events in magenta and returns empty bytes.
pub fn create_debug_binary_retrieval() -> BinaryRetrievalCallback {
    Arc::new(move |key: String| {
        Box::pin(async move {
            eprintln!(
                "\n{BOLD}{MAGENTA}[BINARY_GET]{RESET} {MAGENTA}key={}{RESET}",
                key
            );
            eprintln!("  {DIM}{MAGENTA}(debug stub - returning empty bytes){RESET}\n");

            Ok(Vec::new())
        })
    })
}

/// Truncate a string to a maximum length with ellipsis
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_debug_sql_executor() {
        let executor = create_debug_sql_executor();
        let result = executor(
            "SELECT * FROM nodes".to_string(),
            "default".to_string(),
            "myrepo".to_string(),
            "main".to_string(),
            "system".to_string(),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_debug_function_executor() {
        let executor = create_debug_function_executor();
        let result = executor(
            "/my-function".to_string(),
            "exec-123".to_string(),
            serde_json::json!({"key": "value"}),
            "default".to_string(),
            "myrepo".to_string(),
            "main".to_string(),
            "functions".to_string(),
            None, // auth_context - system context for tests
            None, // log_emitter
        )
        .await;
        assert!(result.is_ok());
        let exec_result = result.unwrap();
        assert!(exec_result.success);
        assert_eq!(exec_result.execution_id, "exec-123");
    }

    #[tokio::test]
    async fn test_debug_enabled_checker() {
        let checker = create_debug_enabled_checker();
        let result = checker(
            "/my-function".to_string(),
            "default".to_string(),
            "myrepo".to_string(),
            "main".to_string(),
            "functions".to_string(),
        )
        .await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_debug_binary_retrieval() {
        let retriever = create_debug_binary_retrieval();
        let result = retriever("some-key".to_string()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
