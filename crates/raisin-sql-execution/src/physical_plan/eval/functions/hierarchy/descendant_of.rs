//! DESCENDANT_OF function - check if current path is a descendant of parent

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use crate::physical_plan::types::from_property_value;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Check if the current row's path is a descendant of the given parent path
///
/// # SQL Signature
/// `DESCENDANT_OF(parent_path [, max_depth]) -> BOOLEAN`
///
/// # Arguments
/// * `parent_path` - Parent path to check against (PATH or TEXT type)
/// * `max_depth` - Optional maximum depth of descendants to include (INT type)
///
/// # Returns
/// * TRUE if current row's path is a descendant of parent_path (within max_depth if specified)
/// * FALSE otherwise
/// * NULL if either the path column or parent_path is NULL
///
/// # Examples
/// ```sql
/// -- Given table with paths: '/', '/a', '/a/b', '/a/b/c', '/a/b/c/d'
///
/// -- Get all descendants (any depth)
/// SELECT * FROM social WHERE DESCENDANT_OF('/a')
/// -- Returns: '/a/b', '/a/b/c', '/a/b/c/d'
///
/// -- Get descendants up to 2 levels deep
/// SELECT * FROM social WHERE DESCENDANT_OF('/a', 2)
/// -- Returns: '/a/b', '/a/b/c' (only 1 and 2 levels deep)
///
/// -- Direct children only (equivalent to CHILD_OF)
/// SELECT * FROM social WHERE DESCENDANT_OF('/a', 1)
/// -- Returns: '/a/b' (only 1 level deep, same as CHILD_OF('/a'))
/// ```
///
/// # Notes
/// - A descendant must start with parent_path + "/"
/// - The parent itself is NOT considered a descendant
/// - max_depth is relative to the parent:
///   - 1 = direct children only (like CHILD_OF)
///   - 2 = children and grandchildren
///   - NULL or omitted = all descendants (unlimited)
/// - Useful for recursive hierarchical queries
pub struct DescendantOfFunction;

impl SqlFunction for DescendantOfFunction {
    fn name(&self) -> &str {
        "DESCENDANT_OF"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Hierarchy
    }

    fn signature(&self) -> &str {
        "DESCENDANT_OF(parent_path [, max_depth]) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count (1 or 2)
        if args.is_empty() || args.len() > 2 {
            return Err(Error::Validation(
                "DESCENDANT_OF requires 1 or 2 arguments: (parent_path [, max_depth])".to_string(),
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
                    "DESCENDANT_OF first argument must be a path".to_string(),
                ))
            }
        };

        // Evaluate optional max_depth argument
        let max_depth: Option<i64> = if args.len() == 2 {
            let depth_lit = eval_expr(&args[1], row)?;
            match depth_lit {
                Literal::Null => None, // NULL means unlimited
                Literal::Int(n) => {
                    if n < 1 {
                        return Err(Error::Validation(
                            "DESCENDANT_OF max_depth must be >= 1".to_string(),
                        ));
                    }
                    Some(n as i64)
                }
                _ => {
                    return Err(Error::Validation(
                        "DESCENDANT_OF second argument must be an integer".to_string(),
                    ))
                }
            }
        } else {
            None // No limit specified
        };

        // Get the current row's path from the 'path' column
        // Use get_by_unqualified to handle both qualified (e.g., "social.path")
        // and unqualified (e.g., "path") column names
        let path_property = row.get_by_unqualified("path").ok_or_else(|| {
            Error::Validation("DESCENDANT_OF requires row to have a 'path' column".to_string())
        })?;

        let path_lit = from_property_value(path_property).map_err(|e| {
            Error::Validation(format!(
                "DESCENDANT_OF could not convert path property value: {}",
                e
            ))
        })?;

        let row_path = match path_lit {
            Literal::Path(p) | Literal::Text(p) => p,
            Literal::Null => return Ok(Literal::Null),
            _ => {
                return Err(Error::Validation(format!(
                "DESCENDANT_OF requires row to have a 'path' column with path/text type, got {:?}",
                path_lit
            )))
            }
        };

        tracing::debug!(
            "DESCENDANT_OF: checking if '{}' is descendant of '{}' (max_depth: {:?})",
            row_path,
            parent_path,
            max_depth
        );

        // A path is never its own descendant
        if row_path == parent_path {
            return Ok(Literal::Boolean(false));
        }

        // Build the prefix to check: parent_path + "/"
        let prefix = if parent_path.ends_with('/') {
            parent_path.clone()
        } else {
            format!("{}/", parent_path)
        };

        // Check if row_path is a descendant (must start with prefix)
        if !row_path.starts_with(&prefix) {
            return Ok(Literal::Boolean(false));
        }

        // If max_depth is specified, check the depth constraint
        if let Some(max) = max_depth {
            let parent_depth = parent_path.split('/').filter(|s| !s.is_empty()).count();
            let row_depth = row_path.split('/').filter(|s| !s.is_empty()).count();
            let relative_depth = (row_depth - parent_depth) as i64;

            if relative_depth > max {
                return Ok(Literal::Boolean(false));
            }
        }

        Ok(Literal::Boolean(true))
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
    fn test_descendant_of_root() {
        let func = DescendantOfFunction;
        let args = vec![TypedExpr::literal(Literal::Path("/".to_string()))];

        // All descendants of root
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a")).unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b")).unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b/c")).unwrap(),
            Literal::Boolean(true)
        );

        // Root is not its own descendant
        assert_eq!(
            func.evaluate(&args, &make_test_row("/")).unwrap(),
            Literal::Boolean(false)
        );
    }

    #[test]
    fn test_descendant_of_nested() {
        let func = DescendantOfFunction;
        let args = vec![TypedExpr::literal(Literal::Path("/a/b".to_string()))];

        // All descendants of /a/b
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b/c")).unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b/c/d")).unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b/x/y/z")).unwrap(),
            Literal::Boolean(true)
        );

        // Not descendants
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a")).unwrap(),
            Literal::Boolean(false)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b")).unwrap(),
            Literal::Boolean(false) // parent is not its own descendant
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/x/y")).unwrap(),
            Literal::Boolean(false)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/bx")).unwrap(),
            Literal::Boolean(false) // /a/bx is not under /a/b
        );
    }

    #[test]
    fn test_descendant_of_excludes_self() {
        let func = DescendantOfFunction;

        // /a is not a descendant of /a
        let args = vec![TypedExpr::literal(Literal::Path("/a".to_string()))];
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a")).unwrap(),
            Literal::Boolean(false)
        );

        // /a/b is not a descendant of /a/b
        let args = vec![TypedExpr::literal(Literal::Path("/a/b".to_string()))];
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b")).unwrap(),
            Literal::Boolean(false)
        );
    }

    #[test]
    fn test_descendant_of_null_handling() {
        let func = DescendantOfFunction;

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

    #[test]
    fn test_descendant_of_with_max_depth() {
        let func = DescendantOfFunction;

        // Test with max_depth = 1 (direct children only, like CHILD_OF)
        let args_depth_1 = vec![
            TypedExpr::literal(Literal::Path("/a".to_string())),
            TypedExpr::literal(Literal::Int(1)),
        ];
        assert_eq!(
            func.evaluate(&args_depth_1, &make_test_row("/a/b"))
                .unwrap(),
            Literal::Boolean(true) // 1 level deep - included
        );
        assert_eq!(
            func.evaluate(&args_depth_1, &make_test_row("/a/b/c"))
                .unwrap(),
            Literal::Boolean(false) // 2 levels deep - excluded
        );

        // Test with max_depth = 2 (children and grandchildren)
        let args_depth_2 = vec![
            TypedExpr::literal(Literal::Path("/a".to_string())),
            TypedExpr::literal(Literal::Int(2)),
        ];
        assert_eq!(
            func.evaluate(&args_depth_2, &make_test_row("/a/b"))
                .unwrap(),
            Literal::Boolean(true) // 1 level deep
        );
        assert_eq!(
            func.evaluate(&args_depth_2, &make_test_row("/a/b/c"))
                .unwrap(),
            Literal::Boolean(true) // 2 levels deep
        );
        assert_eq!(
            func.evaluate(&args_depth_2, &make_test_row("/a/b/c/d"))
                .unwrap(),
            Literal::Boolean(false) // 3 levels deep - excluded
        );

        // Test with max_depth = 3
        let args_depth_3 = vec![
            TypedExpr::literal(Literal::Path("/a".to_string())),
            TypedExpr::literal(Literal::Int(3)),
        ];
        assert_eq!(
            func.evaluate(&args_depth_3, &make_test_row("/a/b/c/d"))
                .unwrap(),
            Literal::Boolean(true) // 3 levels deep - included
        );
        assert_eq!(
            func.evaluate(&args_depth_3, &make_test_row("/a/b/c/d/e"))
                .unwrap(),
            Literal::Boolean(false) // 4 levels deep - excluded
        );
    }

    #[test]
    fn test_descendant_of_depth_1_equals_child_of() {
        let func = DescendantOfFunction;

        // DESCENDANT_OF(path, 1) should behave exactly like CHILD_OF(path)
        let args = vec![
            TypedExpr::literal(Literal::Path("/content".to_string())),
            TypedExpr::literal(Literal::Int(1)),
        ];

        // Direct children - should be true
        assert_eq!(
            func.evaluate(&args, &make_test_row("/content/blog"))
                .unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/content/about"))
                .unwrap(),
            Literal::Boolean(true)
        );

        // Grandchildren - should be false (depth > 1)
        assert_eq!(
            func.evaluate(&args, &make_test_row("/content/blog/post1"))
                .unwrap(),
            Literal::Boolean(false)
        );

        // Parent itself - should be false
        assert_eq!(
            func.evaluate(&args, &make_test_row("/content")).unwrap(),
            Literal::Boolean(false)
        );

        // Unrelated path - should be false
        assert_eq!(
            func.evaluate(&args, &make_test_row("/other")).unwrap(),
            Literal::Boolean(false)
        );
    }

    #[test]
    fn test_descendant_of_null_max_depth() {
        let func = DescendantOfFunction;

        // NULL max_depth means unlimited
        let args = vec![
            TypedExpr::literal(Literal::Path("/a".to_string())),
            TypedExpr::literal(Literal::Null),
        ];

        // All descendants should be included
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b")).unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            func.evaluate(&args, &make_test_row("/a/b/c/d/e/f"))
                .unwrap(),
            Literal::Boolean(true)
        );
    }

    #[test]
    fn test_descendant_of_invalid_max_depth() {
        let func = DescendantOfFunction;

        // max_depth = 0 should error
        let args_zero = vec![
            TypedExpr::literal(Literal::Path("/a".to_string())),
            TypedExpr::literal(Literal::Int(0)),
        ];
        assert!(func.evaluate(&args_zero, &make_test_row("/a/b")).is_err());

        // max_depth = -1 should error
        let args_neg = vec![
            TypedExpr::literal(Literal::Path("/a".to_string())),
            TypedExpr::literal(Literal::Int(-1)),
        ];
        assert!(func.evaluate(&args_neg, &make_test_row("/a/b")).is_err());
    }
}
