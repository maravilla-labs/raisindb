//! PATH_STARTS_WITH function - check if path starts with prefix

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Check if a path starts with a given prefix
///
/// # SQL Signature
/// `PATH_STARTS_WITH(path, prefix) -> BOOLEAN`
///
/// # Arguments
/// * `path` - Hierarchical path to check (PATH or TEXT type)
/// * `prefix` - Prefix path to match against (PATH or TEXT type)
///
/// # Returns
/// * TRUE if path starts with prefix
/// * FALSE otherwise
/// * NULL if either argument is NULL
///
/// # Examples
/// ```sql
/// SELECT PATH_STARTS_WITH('/a/b/c', '/a') -> TRUE
/// SELECT PATH_STARTS_WITH('/a/b/c', '/a/b') -> TRUE
/// SELECT PATH_STARTS_WITH('/a/b/c', '/x') -> FALSE
/// SELECT PATH_STARTS_WITH(NULL, '/a') -> NULL
/// ```
///
/// # Notes
/// - Useful for hierarchical queries and filtering
/// - Can be used with indexes for efficient path-based queries
pub struct PathStartsWithFunction;

impl SqlFunction for PathStartsWithFunction {
    fn name(&self) -> &str {
        "PATH_STARTS_WITH"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Hierarchy
    }

    fn signature(&self) -> &str {
        "PATH_STARTS_WITH(path, prefix) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 2 {
            return Err(Error::Validation(
                "PATH_STARTS_WITH requires exactly 2 arguments".to_string(),
            ));
        }

        let path_lit = eval_expr(&args[0], row)?;
        let prefix_lit = eval_expr(&args[1], row)?;

        tracing::debug!(
            "PATH_STARTS_WITH: path_lit={:?}, prefix_lit={:?}",
            path_lit,
            prefix_lit
        );

        // Handle NULL propagation
        match (&path_lit, &prefix_lit) {
            (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
            (Literal::Path(path), Literal::Path(prefix))
            | (Literal::Text(path), Literal::Text(prefix))
            | (Literal::Path(path), Literal::Text(prefix))
            | (Literal::Text(path), Literal::Path(prefix)) => {
                Ok(Literal::Boolean(path.starts_with(prefix)))
            }
            _ => Err(Error::Validation(format!(
                "PATH_STARTS_WITH requires path/text arguments, got {:?} and {:?}",
                path_lit, prefix_lit
            ))),
        }
    }
}
