//! Expression Evaluator
//!
//! Evaluates typed expressions against a row of data at runtime.
//! This is the core of filter and projection execution.
//!
//! # Module Structure
//!
//! - `core`: Main eval_expr function
//! - `async_eval`: Async expression evaluation (for EMBEDDING function)
//! - `binary_ops`: Binary and unary operations
//! - `helpers`: Helper functions (arithmetic, comparison, logical)
//! - `vector_ops`: Vector operations (L2 distance, dot product)
//! - `pattern`: SQL LIKE pattern matching
//! - `casting`: Type casting operations
//! - `json_ops`: JSON containment operations
//! - `functions`: Function evaluation (hierarchy, string, numeric, JSON, aggregate)

// Module declarations
mod async_eval;
mod binary_ops;
mod casting;
pub(crate) mod core;
mod functions;
mod helpers;
mod json_ops;
mod pattern;
mod vector_ops;

// Public API - re-export the main functions
pub use self::async_eval::{eval_expr_async, generate_embedding_cached};
pub use self::core::eval_expr;

// Re-export function context for system functions (CURRENT_USER, etc.)
pub use self::functions::{clear_function_context, set_function_context, FunctionContext};
