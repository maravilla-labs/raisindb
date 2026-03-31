//! Trait definitions for the extensible SQL function system
//!
//! This module defines the core traits and types for the function registry:
//! - `SqlFunction`: The main trait that all SQL functions must implement
//! - `FunctionCategory`: Categorization for better organization and debugging
//!
//! # Design Principles
//! - Functions are stateless and thread-safe (trait objects are `Send + Sync`)
//! - Each function is responsible for its own argument validation
//! - Functions follow SQL NULL propagation semantics
//! - Error messages should be clear and actionable

use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Category of SQL function for organization and debugging
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FunctionCategory {
    /// String manipulation functions (UPPER, LOWER, COALESCE, etc.)
    String,
    /// Numeric operations (ROUND, ABS, etc.)
    Numeric,
    /// JSON/JSONB operations (JSON_VALUE, JSON_EXISTS, etc.)
    Json,
    /// Aggregate functions (COUNT, SUM, AVG, MIN, MAX, ARRAY_AGG)
    Aggregate,
    /// Full-text search functions (TS_RANK, etc.)
    FullText,
    /// Hierarchical path operations (DEPTH, PARENT, ANCESTOR, etc.)
    Hierarchy,
    /// Temporal/datetime functions (NOW, CURRENT_TIMESTAMP, etc.)
    Temporal,
    /// System information functions (VERSION, CURRENT_SCHEMA, CURRENT_USER, etc.)
    System,
    /// Geospatial functions (ST_DISTANCE, ST_DWITHIN, ST_CONTAINS, etc.)
    Geospatial,
}

impl FunctionCategory {
    /// Get the category name as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Numeric => "numeric",
            Self::Json => "json",
            Self::Aggregate => "aggregate",
            Self::FullText => "fulltext",
            Self::Hierarchy => "hierarchy",
            Self::Temporal => "temporal",
            Self::System => "system",
            Self::Geospatial => "geospatial",
        }
    }
}

/// Core trait for all SQL functions in the evaluation engine
///
/// This trait defines the interface for evaluating SQL functions during query execution.
/// Each function implementation is responsible for:
/// - Argument validation (count and types)
/// - NULL propagation handling
/// - Type conversions and coercions
/// - Returning descriptive errors
///
/// # Thread Safety
/// Implementations must be thread-safe (`Send + Sync`) as they may be accessed
/// from multiple threads during parallel query execution.
///
/// # Example Implementation
/// ```rust,ignore
/// pub struct UpperFunction;
///
/// impl SqlFunction for UpperFunction {
///     fn name(&self) -> &str {
///         "UPPER"
///     }
///
///     fn category(&self) -> FunctionCategory {
///         FunctionCategory::String
///     }
///
///     fn signature(&self) -> &str {
///         "UPPER(text) -> TEXT"
///     }
///
///     #[inline]
///     fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
///         // Implementation...
///     }
/// }
/// ```
pub trait SqlFunction: Send + Sync {
    /// Return the function name (case-insensitive, typically uppercase)
    ///
    /// This name is used for function lookup in the registry.
    /// Should be a constant string in uppercase (e.g., "UPPER", "JSON_VALUE").
    fn name(&self) -> &str;

    /// Return the function category for organization
    ///
    /// Used for debugging, introspection, and potential optimizations.
    fn category(&self) -> FunctionCategory;

    /// Return a human-readable function signature
    ///
    /// Used for error messages and documentation.
    /// Format: "FUNCTION_NAME(arg1_type, arg2_type, ...) -> return_type"
    ///
    /// Examples:
    /// - "UPPER(text) -> TEXT"
    /// - "ROUND(numeric, decimals?) -> DOUBLE"
    /// - "JSON_VALUE(jsonb, path) -> TEXT"
    fn signature(&self) -> &str;

    /// Evaluate the function with given arguments in the context of a row
    ///
    /// # Arguments
    /// * `args` - Typed expressions representing function arguments
    /// * `row` - Current row context for column value lookup
    ///
    /// # Returns
    /// * `Ok(Literal)` - The computed function result
    /// * `Err(Error)` - Validation or runtime error
    ///
    /// # Responsibilities
    /// - Validate argument count and types
    /// - Evaluate argument expressions using `eval_expr` from parent module
    /// - Handle NULL propagation according to SQL semantics
    /// - Return descriptive errors for invalid inputs
    ///
    /// # Performance
    /// Mark implementations with `#[inline]` when beneficial for small functions.
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error>;
}
