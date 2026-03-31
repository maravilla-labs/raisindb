//! PARENT function - get parent path

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_parent_at_level;

/// Get the parent path by going N levels up
///
/// # SQL Signature
/// `PARENT(path, levels?) -> PATH`
///
/// # Arguments
/// * `path` - Hierarchical path (PATH or TEXT type)
/// * `levels` - Optional number of levels to go up (default: 1)
///
/// # Returns
/// * Parent path N levels up from the current path
/// * Empty string if levels exceed path depth
/// * "/" if going up reaches the root
///
/// # Examples
/// ```sql
/// SELECT PARENT('/a/b/c') -> '/a/b'  -- default 1 level
/// SELECT PARENT('/a/b/c', 1) -> '/a/b'  -- immediate parent
/// SELECT PARENT('/a/b/c', 2) -> '/a'  -- grandparent
/// SELECT PARENT('/a', 1) -> '/'  -- root
/// ```
///
/// # Notes
/// - Works backwards from the end of the path
/// - Useful for navigating hierarchies in queries
pub struct ParentFunction;

impl SqlFunction for ParentFunction {
    fn name(&self) -> &str {
        "PARENT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Hierarchy
    }

    fn signature(&self) -> &str {
        "PARENT(path, levels?) -> PATH"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count (1 or 2)
        if args.is_empty() || args.len() > 2 {
            return Err(Error::Validation(
                "PARENT requires 1 or 2 arguments".to_string(),
            ));
        }

        // Evaluate the path argument
        let path_lit = eval_expr(&args[0], row)?;

        // Extract path value (supports both Path and Text types)
        let path = match path_lit {
            Literal::Path(p) | Literal::Text(p) => p,
            _ => {
                return Err(Error::Validation(
                    "PARENT requires a path argument".to_string(),
                ))
            }
        };

        // Determine how many levels to go up
        let levels = if args.len() == 2 {
            let levels_lit = eval_expr(&args[1], row)?;
            match levels_lit {
                Literal::Int(n) if n >= 0 => n as usize,
                Literal::Int(n) => {
                    return Err(Error::Validation(format!(
                        "PARENT levels must be non-negative, got {}",
                        n
                    )))
                }
                _ => {
                    return Err(Error::Validation(
                        "PARENT levels must be an integer".to_string(),
                    ))
                }
            }
        } else {
            1 // Default: immediate parent
        };

        // Get parent at specified level
        let result = get_parent_at_level(&path, levels);
        Ok(Literal::Path(result))
    }
}
