// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! SQL passthrough runtime implementation

use async_trait::async_trait;
use raisin_error::{Error, Result};
use std::collections::HashMap;
use std::sync::Arc;

use super::FunctionRuntime;
use crate::api::FunctionApi;
use crate::types::{
    ExecutionContext, ExecutionError, ExecutionResult, ExecutionStats, FunctionLanguage,
    FunctionMetadata, LogEntry,
};

/// SQL passthrough runtime
///
/// Executes SQL code directly via the RaisinDB SQL engine.
/// This is useful for simple data transformation functions written in SQL.
pub struct SqlRuntime {
    // Configuration options can be added here
}

impl SqlRuntime {
    /// Create a new SQL runtime
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for SqlRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FunctionRuntime for SqlRuntime {
    async fn execute(
        &self,
        code: &str,
        _entrypoint: &str,
        context: ExecutionContext,
        _metadata: &FunctionMetadata,
        api: Arc<dyn FunctionApi>,
        _files: HashMap<String, String>,
    ) -> Result<ExecutionResult> {
        let start = std::time::Instant::now();
        let execution_id = context.execution_id.clone();

        tracing::debug!(
            execution_id = %execution_id,
            "Executing SQL function"
        );

        // Extract parameters from input
        let params: Vec<serde_json::Value> = if let Some(arr) = context.input.as_array() {
            arr.clone()
        } else if context.input.is_object() {
            // Convert object to array of values if needed
            vec![context.input.clone()]
        } else {
            vec![]
        };

        // Execute the SQL
        match api.sql_query(code, params).await {
            Ok(result) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                let stats = ExecutionStats {
                    duration_ms,
                    sql_queries: 1,
                    ..Default::default()
                };

                Ok(
                    ExecutionResult::success(execution_id, result, stats).with_logs(vec![
                        LogEntry::info(format!("SQL executed in {}ms", duration_ms)),
                    ]),
                )
            }
            Err(e) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                Ok(ExecutionResult::failure(
                    execution_id,
                    ExecutionError::runtime(format!("SQL execution failed: {}", e)),
                    ExecutionStats {
                        duration_ms,
                        sql_queries: 1,
                        ..Default::default()
                    },
                ))
            }
        }
    }

    fn validate(&self, code: &str) -> Result<()> {
        if code.trim().is_empty() {
            return Err(Error::Validation("SQL code cannot be empty".to_string()));
        }

        // Basic SQL validation - check for dangerous operations
        let code_upper = code.to_uppercase();

        // Disallow certain dangerous operations in functions
        let dangerous = ["DROP DATABASE", "DROP SCHEMA", "TRUNCATE"];
        for op in dangerous {
            if code_upper.contains(op) {
                return Err(Error::Validation(format!(
                    "SQL function cannot contain '{}' operation",
                    op
                )));
            }
        }

        Ok(())
    }

    fn language(&self) -> FunctionLanguage {
        FunctionLanguage::Sql
    }

    fn name(&self) -> &'static str {
        "SQL"
    }
}
