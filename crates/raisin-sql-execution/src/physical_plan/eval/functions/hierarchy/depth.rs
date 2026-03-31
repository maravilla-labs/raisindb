//! DEPTH function - calculate hierarchy depth

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Calculate the depth of a hierarchical path
///
/// # SQL Signature
/// `DEPTH(path) -> INT`
///
/// # Arguments
/// * `path` - Hierarchical path (PATH or TEXT type)
///
/// # Returns
/// * Number of path segments (depth level)
/// * 0 for root path "/"
/// * Error if argument is not a path
///
/// # Examples
/// ```sql
/// SELECT DEPTH('/') -> 0
/// SELECT DEPTH('/a') -> 1
/// SELECT DEPTH('/a/b/c') -> 3
/// ```
///
/// # Notes
/// - Counts non-empty segments separated by '/'
/// - Compatible with hierarchical queries and path-based indexes
pub struct DepthFunction;

impl SqlFunction for DepthFunction {
    fn name(&self) -> &str {
        "DEPTH"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Hierarchy
    }

    fn signature(&self) -> &str {
        "DEPTH(path) -> INT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 1 {
            return Err(Error::Validation(
                "DEPTH requires exactly 1 argument".to_string(),
            ));
        }

        // Evaluate the path argument
        let path_lit = eval_expr(&args[0], row)?;

        // Extract path value (supports both Path and Text types)
        let path = match path_lit {
            Literal::Path(p) | Literal::Text(p) => p,
            _ => {
                return Err(Error::Validation(
                    "DEPTH requires a path argument".to_string(),
                ))
            }
        };

        // Calculate depth by counting non-empty segments
        let depth = path.split('/').filter(|s| !s.is_empty()).count();
        Ok(Literal::Int(depth as i32))
    }
}
