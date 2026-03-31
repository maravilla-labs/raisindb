//! CHILD_OF function - check if current path is a direct child of parent

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use crate::physical_plan::types::from_property_value;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Check if the current row's path is a direct child of the given parent path
///
/// # SQL Signature
/// `CHILD_OF(parent_path) -> BOOLEAN`
///
/// # Arguments
/// * `parent_path` - Parent path to check against (PATH or TEXT type)
///
/// # Returns
/// * TRUE if current row's path is a direct child of parent_path
/// * FALSE otherwise
/// * NULL if either the path column or parent_path is NULL
///
/// # Examples
/// ```sql
/// -- Given table with paths: '/', '/a', '/b', '/a/b', '/a/b/c'
/// SELECT * FROM social WHERE CHILD_OF('/')
/// -- Returns: '/a', '/b' (direct children of root only)
///
/// SELECT * FROM social WHERE CHILD_OF('/a')
/// -- Returns: '/a/b' (direct children of '/a' only, not '/a/b/c')
/// ```
///
/// # Notes
/// - A direct child has exactly depth(parent) + 1 levels
/// - Useful for hierarchical queries with natural ordering
/// - Can be optimized by query planner to use ordered children index
/// - For recursive descendants, use PATH_STARTS_WITH instead
pub struct ChildOfFunction;

impl SqlFunction for ChildOfFunction {
    fn name(&self) -> &str {
        "CHILD_OF"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Hierarchy
    }

    fn signature(&self) -> &str {
        "CHILD_OF(parent_path) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 1 {
            return Err(Error::Validation(
                "CHILD_OF requires exactly 1 argument".to_string(),
            ));
        }

        // Evaluate the parent path argument
        let parent_lit = eval_expr(&args[0], row)?;

        // Handle NULL propagation
        if matches!(parent_lit, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Extract parent path value (supports both Path and Text types)
        let parent_path = match parent_lit {
            Literal::Path(p) | Literal::Text(p) => p,
            _ => {
                return Err(Error::Validation(
                    "CHILD_OF requires a path argument".to_string(),
                ))
            }
        };

        // Get the current row's path from the 'path' column
        // Use get_by_unqualified to handle both qualified (e.g., "social.path")
        // and unqualified (e.g., "path") column names
        let path_property = row.get_by_unqualified("path").ok_or_else(|| {
            Error::Validation("CHILD_OF requires row to have a 'path' column".to_string())
        })?;

        let path_lit = from_property_value(path_property).map_err(|e| {
            Error::Validation(format!(
                "CHILD_OF could not convert path property value: {}",
                e
            ))
        })?;

        let row_path = match path_lit {
            Literal::Path(p) | Literal::Text(p) => p,
            Literal::Null => return Ok(Literal::Null),
            _ => {
                return Err(Error::Validation(format!(
                    "CHILD_OF requires row to have a 'path' column with path/text type, got {:?}",
                    path_lit
                )))
            }
        };

        tracing::debug!(
            "CHILD_OF: checking if '{}' is direct child of '{}'",
            row_path,
            parent_path
        );

        // Check if row_path starts with parent_path
        if !row_path.starts_with(&parent_path) {
            return Ok(Literal::Boolean(false));
        }

        // Calculate depths
        let parent_depth = parent_path.split('/').filter(|s| !s.is_empty()).count();
        let row_depth = row_path.split('/').filter(|s| !s.is_empty()).count();

        // Direct child means exactly one level deeper
        let is_direct_child = row_depth == parent_depth + 1;

        Ok(Literal::Boolean(is_direct_child))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_plan::executor::Row;
    use indexmap::IndexMap;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_sql::analyzer::{DataType, TypedExpr};

    fn make_test_row(path: &str) -> Row {
        let mut columns = IndexMap::new();
        columns.insert("path".to_string(), PropertyValue::String(path.to_string()));
        Row::from_map(columns)
    }

    fn make_test_row_with_null_path() -> Row {
        let columns = IndexMap::new();
        // Empty map - path column doesn't exist, should be treated as NULL
        Row::from_map(columns)
    }

    #[test]
    fn test_child_of_root() {
        let func = ChildOfFunction;
        let args = vec![TypedExpr::literal(Literal::Path("/".to_string()))];

        // Direct children of root
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a")).unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/b")).unwrap(),
            Literal::Boolean(true)
        );

        // Not direct children
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b")).unwrap(),
            Literal::Boolean(false)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b/c")).unwrap(),
            Literal::Boolean(false)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/")).unwrap(),
            Literal::Boolean(false)
        );
    }

    #[test]
    fn test_child_of_nested() {
        let func = ChildOfFunction;
        let args = vec![TypedExpr::literal(Literal::Path("/a/b".to_string()))];

        // Direct children of /a/b
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b/c")).unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b/d")).unwrap(),
            Literal::Boolean(true)
        );

        // Not direct children
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a")).unwrap(),
            Literal::Boolean(false)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b")).unwrap(),
            Literal::Boolean(false)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b/c/d")).unwrap(),
            Literal::Boolean(false)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/x/y")).unwrap(),
            Literal::Boolean(false)
        );
    }

    #[test]
    fn test_child_of_null_handling() {
        let func = ChildOfFunction;

        // NULL parent path
        let args_null = vec![TypedExpr::literal(Literal::Null)];
        assert_eq!(
            func.evaluate(&args_null, &make_test_row("/a")).unwrap(),
            Literal::Null
        );

        // NULL/missing row path
        let args = vec![TypedExpr::literal(Literal::Path("/".to_string()))];
        assert!(func
            .evaluate(&args, &make_test_row_with_null_path())
            .is_err());
    }
}
