//! Expression Evaluator
//!
//! Evaluates typed expressions against a row of data at runtime.
//! This is the core of filter and projection execution.
//!
//! # Module Structure
//!
//! - `core`: Main eval_expr function
//! - `async_eval`: Async expression evaluation (EMBEDDING, RESOLVE, INVOKE, locks)
//! - `resolve_eval`: The SQL binding of RESOLVE()
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
pub(crate) mod functions;
mod helpers;
mod json_ops;
mod pattern;
mod regex_ops;
mod resolve_eval;
mod resolve_path_eval;
mod vector_ops;

// Public API - re-export the main functions
pub use self::async_eval::{eval_expr_async, generate_embedding_cached};
pub(crate) use self::casting::cast_literal;
pub use self::core::eval_expr;
pub(crate) use self::resolve_eval::eval_resolve_rows;

// Re-export function context for system functions (CURRENT_USER, etc.)
pub use self::functions::{clear_function_context, set_function_context, FunctionContext};
