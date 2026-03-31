//! Expression and function evaluation module for Cypher
//!
//! This module provides a clean, enum-based architecture for evaluating Cypher expressions
//! and functions. It separates evaluation logic from execution coordination, making the
//! codebase more maintainable and extensible.
//!
//! # Architecture
//!
//! - **expr**: Expression evaluation (literals, variables, properties, binary ops)
//! - **condition**: WHERE clause evaluation (boolean conditions)
//! - **functions**: Enum-based function dispatch with all Cypher functions
//!
//! # Usage
//!
//! ```ignore
//! use evaluation::{evaluate_expr, evaluate_condition, FunctionContext};
//!
//! // Evaluate an expression
//! let value = evaluate_expr(&expr, &binding, &context)?;
//!
//! // Filter bindings with WHERE clause
//! let filtered = execute_where(&condition, bindings, &context).await?;
//!
//! // Call a function
//! let result = evaluate_function("pageRank", &args, &binding, &context).await?;
//! ```

mod condition;
mod expr;
pub mod functions;

// Re-export main evaluation functions
pub use condition::execute_where;
pub use expr::evaluate_expr_async_impl;

// Re-export function-related types and functions
pub use functions::FunctionContext;
