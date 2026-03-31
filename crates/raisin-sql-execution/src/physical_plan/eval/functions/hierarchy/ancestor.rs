//! ANCESTOR function - get ancestor at absolute depth

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_ancestor;

/// Get ancestor at a specific absolute depth from root
///
/// # SQL Signature
/// `ANCESTOR(path, depth) -> PATH`
///
/// # Arguments
/// * `path` - Hierarchical path (PATH or TEXT type)
/// * `depth` - Absolute depth from root (0 = root, 1 = first level, etc.)
///
/// # Returns
/// * Path truncated to the specified depth
/// * Empty string if depth exceeds path depth
///
/// # Examples
/// ```sql
/// SELECT ANCESTOR('/a/b/c/d', 2) -> '/a/b'
/// SELECT ANCESTOR('/a/b/c/d', 1) -> '/a'
/// SELECT ANCESTOR('/a/b/c/d', 0) -> ''
/// SELECT ANCESTOR('/a/b/c/d', 10) -> ''  -- depth too high
/// ```
///
/// # Notes
/// - Counts from the root (absolute depth)
/// - Different from PARENT which counts backwards from current position
pub struct AncestorFunction;

impl SqlFunction for AncestorFunction {
    fn name(&self) -> &str {
        "ANCESTOR"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Hierarchy
    }

    fn signature(&self) -> &str {
        "ANCESTOR(path, depth) -> PATH"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 2 {
            return Err(Error::Validation(
                "ANCESTOR requires exactly 2 arguments".to_string(),
            ));
        }

        let path_lit = eval_expr(&args[0], row)?;
        let depth_lit = eval_expr(&args[1], row)?;

        // Extract path value (supports both Path and Text types)
        let path = match path_lit {
            Literal::Path(p) | Literal::Text(p) => p,
            _ => {
                return Err(Error::Validation(
                    "ANCESTOR requires a path argument".to_string(),
                ))
            }
        };

        // Extract depth value
        let Literal::Int(depth) = depth_lit else {
            return Err(Error::Validation(
                "ANCESTOR depth must be an integer".to_string(),
            ));
        };

        // Get ancestor at specified depth
        let result = get_ancestor(&path, depth);
        Ok(Literal::Path(result))
    }
}
