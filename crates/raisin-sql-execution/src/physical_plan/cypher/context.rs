//! Execution context for Cypher queries
//!
//! This module provides a centralized execution context that encapsulates
//! all the necessary information for executing Cypher queries, including
//! storage access and workspace metadata.

use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;
use std::collections::HashMap;
use std::sync::Arc;

use super::evaluation::FunctionContext;

/// Execution context for Cypher queries
///
/// This struct centralizes all context information needed during query execution,
/// making it easier to pass context between different execution components.
pub struct ExecutionContext<S: Storage> {
    /// Storage backend
    pub storage: Arc<S>,
    /// Tenant identifier
    pub tenant_id: String,
    /// Repository identifier
    pub repo_id: String,
    /// Branch name
    pub branch: String,
    /// Workspace identifier
    pub workspace_id: String,
    /// Optional revision for time-travel queries
    pub revision: Option<HLC>,
    /// Query parameters provided by the caller
    pub parameters: Arc<HashMap<String, PropertyValue>>,
}

impl<S: Storage> ExecutionContext<S> {
    /// Create a new execution context
    pub fn new(
        storage: Arc<S>,
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace_id: String,
        revision: Option<HLC>,
    ) -> Self {
        Self {
            storage,
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            revision,
            parameters: Arc::new(HashMap::new()),
        }
    }

    /// Create a FunctionContext for expression evaluation
    ///
    /// This is a convenience method that creates a FunctionContext reference
    /// from this ExecutionContext, which is used for evaluating expressions
    /// and function calls.
    pub fn function_context(&self) -> FunctionContext<'_, S> {
        FunctionContext {
            storage: &self.storage,
            tenant_id: &self.tenant_id,
            repo_id: &self.repo_id,
            branch: &self.branch,
            workspace_id: &self.workspace_id,
            revision: self.revision.as_ref(),
            parameters: &self.parameters,
        }
    }
}
