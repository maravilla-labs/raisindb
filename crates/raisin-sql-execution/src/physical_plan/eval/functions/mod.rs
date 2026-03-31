//! Function evaluation module
//!
//! This module handles evaluation of SQL functions including:
//! - Hierarchy functions (DEPTH, PARENT, ANCESTOR, PATH_STARTS_WITH)
//! - String functions (UPPER, LOWER, COALESCE)
//! - Numeric functions (ROUND)
//! - JSON functions (JSON_VALUE, JSON_EXISTS, JSON_GET_*)
//! - Aggregate function lookup (COUNT, SUM, AVG, MIN, MAX, ARRAY_AGG)
//! - Full-text functions (TS_RANK)
//! - Temporal functions (NOW)
//! - System functions (CURRENT_USER, VERSION, etc.)
//!
//! # Architecture
//! This module uses a trait-based extensible architecture:
//! - `SqlFunction` trait defines the interface for all functions
//! - `FunctionRegistry` provides centralized, case-insensitive function lookup
//! - Functions are organized by category in separate modules
//! - Each function is a self-contained struct implementing `SqlFunction`

mod aggregate;
mod fulltext;
mod geospatial;
mod hierarchy;
mod json;
mod numeric;
mod registry;
mod string;
mod system;
mod temporal;
mod traits;

// Re-export public API
pub use registry::FunctionRegistry;
pub use traits::SqlFunction;

use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};
use std::sync::LazyLock;

/// Context for function evaluation containing authentication info
///
/// This is passed to functions that need access to the current user
/// or other session-level information not available in the row.
#[derive(Debug, Clone, Default)]
pub struct FunctionContext {
    /// The current authenticated user's ID (from AuthContext)
    pub user_id: Option<String>,
    /// The current authenticated user's node (pre-fetched from repository)
    /// CURRENT_USER() returns this as JSON
    pub user_node: Option<serde_json::Value>,
}

// Thread-local storage for function context
// This allows system functions to access user info without changing trait signatures
thread_local! {
    static FUNCTION_CONTEXT: std::cell::RefCell<Option<FunctionContext>> = const { std::cell::RefCell::new(None) };
}

/// Set the function context for the current thread
///
/// This should be called before evaluating expressions that may use
/// system functions like CURRENT_USER.
pub fn set_function_context(ctx: FunctionContext) {
    FUNCTION_CONTEXT.with(|c| {
        *c.borrow_mut() = Some(ctx);
    });
}

/// Clear the function context for the current thread
pub fn clear_function_context() {
    FUNCTION_CONTEXT.with(|c| {
        *c.borrow_mut() = None;
    });
}

/// Get a clone of the current function context
pub(crate) fn get_function_context() -> Option<FunctionContext> {
    FUNCTION_CONTEXT.with(|c| c.borrow().clone())
}

// Re-export naming functions for backward compatibility
pub use aggregate::naming::generate_function_column_name;

/// Global function registry initialized on first access
///
/// This registry is populated with all built-in SQL functions during initialization.
/// It provides O(1) case-insensitive function lookup.
///
/// # Thread Safety
/// LazyLock ensures thread-safe initialization - the registry is created exactly once
/// on first access, even if multiple threads try to access it simultaneously.
static FUNCTIONS: LazyLock<FunctionRegistry> = LazyLock::new(|| {
    let mut registry = FunctionRegistry::new();

    // Register functions by category
    string::register_functions(&mut registry);
    numeric::register_functions(&mut registry);
    json::register_functions(&mut registry);
    hierarchy::register_functions(&mut registry);
    aggregate::register_functions(&mut registry);
    fulltext::register_functions(&mut registry);
    temporal::register_functions(&mut registry);
    system::register_functions(&mut registry);
    geospatial::register_functions(&mut registry);

    tracing::info!(
        "Initialized SQL function registry with {} functions",
        registry.len()
    );

    registry
});

/// Evaluate a function call using the registry-based system
///
/// This function looks up and evaluates functions from the global function registry.
/// All built-in SQL functions are registered during registry initialization.
///
/// # Arguments
/// * `name` - Function name (case-insensitive)
/// * `args` - Typed expressions representing function arguments
/// * `row` - Current row context for column value lookup
///
/// # Returns
/// * `Ok(Literal)` - The computed function result
/// * `Err(Error)` - Validation or runtime error (including unknown function)
pub(super) fn eval_function(name: &str, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
    // Look up function in the registry
    if let Some(func) = FUNCTIONS.get(name) {
        tracing::trace!(
            "Evaluating function {} via registry (category: {})",
            name,
            func.category().as_str()
        );
        func.evaluate(args, row)
    } else {
        // Function not found in registry
        Err(Error::Validation(format!("Unknown function: {}", name)))
    }
}
