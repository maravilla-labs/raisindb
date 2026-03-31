//! Function evaluation context for Cypher
//!
//! Provides the FunctionContext struct that contains all necessary information
//! for evaluating Cypher functions, including storage access and query execution
//! context (tenant, repo, branch, etc.).

use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;
use std::collections::HashMap;

/// Context needed for function evaluation
///
/// Contains all necessary context information for evaluating Cypher functions,
/// including storage access and query execution context (tenant, repo, branch, etc.)
///
/// # Type Parameters
///
/// * `S` - Storage backend implementing the Storage trait
///
/// # Lifetimes
///
/// * `'a` - Lifetime of the borrowed context data
///
/// # Example
///
/// ```ignore
/// let context = FunctionContext {
///     storage: &storage,
///     tenant_id: "tenant1",
///     repo_id: "repo1",
///     branch: "main",
///     workspace_id: "workspace1",
///     revision: None,
///     parameters: &params,
/// };
/// ```
pub struct FunctionContext<'a, S: Storage> {
    /// Storage backend for accessing nodes and relationships
    pub storage: &'a S,
    /// Tenant identifier
    pub tenant_id: &'a str,
    /// Repository identifier
    pub repo_id: &'a str,
    /// Branch name
    pub branch: &'a str,
    /// Workspace identifier
    pub workspace_id: &'a str,
    /// Optional revision for time-travel queries
    pub revision: Option<&'a raisin_hlc::HLC>,
    /// Parameter map provided by SQL caller
    pub parameters: &'a HashMap<String, PropertyValue>,
}
