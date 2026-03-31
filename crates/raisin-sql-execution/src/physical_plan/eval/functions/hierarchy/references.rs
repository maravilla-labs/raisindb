//! REFERENCES function - check if current node references a target
//!
//! This function is primarily used during plan optimization. The actual query execution
//! leverages the reverse reference index for efficient lookups. However, this function
//! provides a fallback for post-filter evaluation when index access is not possible.

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Check if the current row references the target 'workspace:/path'
///
/// # SQL Signature
/// `REFERENCES(target) -> BOOLEAN`
///
/// # Arguments
/// * `target` - Target reference in 'workspace:/path' format (TEXT type)
///
/// # Returns
/// * TRUE if current row has a reference property pointing to the target
/// * FALSE otherwise
/// * NULL if target is NULL
///
/// # Examples
/// ```sql
/// -- Find articles that reference a specific tag
/// SELECT * FROM social
/// WHERE DESCENDANT_OF('/demonews/articles')
///   AND node_type = 'news:Article'
///   AND REFERENCES('social:/demonews/tags/tech-stack/rust')
///
/// -- Find articles with any tech-stack tag (using optimizer)
/// SELECT * FROM social
/// WHERE REFERENCES('social:/demonews/tags/tech-stack/rust')
///   OR REFERENCES('social:/demonews/tags/tech-stack/typescript')
/// ```
///
/// # Performance Notes
/// This function performs a linear scan of the properties looking for matching references.
/// For efficient queries, the optimizer will convert REFERENCES predicates to use the
/// reverse reference index (ref_rev CF) which provides O(k) lookup where k is the number
/// of references to the target.
///
/// The function is primarily used:
/// 1. As a post-filter when combined with other predicates
/// 2. For runtime evaluation when the optimizer cannot use the index
pub struct ReferencesFunction;

impl SqlFunction for ReferencesFunction {
    fn name(&self) -> &str {
        "REFERENCES"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Hierarchy
    }

    fn signature(&self) -> &str {
        "REFERENCES(target) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 1 {
            return Err(Error::Validation(
                "REFERENCES requires exactly 1 argument: (target)".to_string(),
            ));
        }

        // Evaluate the target argument
        let target_lit = eval_expr(&args[0], row)?;

        // Handle NULL propagation
        if matches!(target_lit, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Extract target value (must be Text in 'workspace:/path' format)
        let target = match target_lit {
            Literal::Text(t) => t,
            _ => {
                return Err(Error::Validation(
                    "REFERENCES argument must be a text value in 'workspace:/path' format"
                        .to_string(),
                ))
            }
        };

        // Parse 'workspace:/path' format
        let (target_workspace, target_path) = match target.split_once(':') {
            Some((ws, path)) => (ws, path),
            None => {
                return Err(Error::Validation(format!(
                    "REFERENCES target '{}' must be in 'workspace:/path' format",
                    target
                )))
            }
        };

        // Get the properties from the row
        let properties = row.get_by_unqualified("properties");

        if properties.is_none() {
            // No properties column, cannot have references
            return Ok(Literal::Boolean(false));
        }

        let properties = properties.unwrap();

        // Check if any reference in properties matches the target
        let has_reference = check_for_reference(properties, target_workspace, target_path);

        Ok(Literal::Boolean(has_reference))
    }
}

/// Recursively check if a property value contains a reference to the target
fn check_for_reference(value: &PropertyValue, target_workspace: &str, target_path: &str) -> bool {
    match value {
        PropertyValue::Reference(r) => r.workspace == target_workspace && r.path == target_path,
        PropertyValue::Array(items) => items
            .iter()
            .any(|item| check_for_reference(item, target_workspace, target_path)),
        PropertyValue::Object(obj) => obj
            .values()
            .any(|val| check_for_reference(val, target_workspace, target_path)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_plan::executor::Row;
    use indexmap::IndexMap;
    use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
    use raisin_sql::analyzer::TypedExpr;
    use std::collections::HashMap;

    fn make_test_row_with_tags(tags: Vec<(&str, &str)>) -> Row {
        let mut columns = IndexMap::new();
        columns.insert(
            "id".to_string(),
            PropertyValue::String("test-id".to_string()),
        );
        columns.insert(
            "path".to_string(),
            PropertyValue::String("/demonews/articles/tech/test-article".to_string()),
        );

        // Create properties with tags as references
        let tag_refs: Vec<PropertyValue> = tags
            .into_iter()
            .map(|(ws, path)| {
                PropertyValue::Reference(RaisinReference {
                    id: format!("tag-{}", path.split('/').last().unwrap_or("unknown")),
                    workspace: ws.to_string(),
                    path: path.to_string(),
                })
            })
            .collect();

        let mut props = HashMap::new();
        props.insert("tags".to_string(), PropertyValue::Array(tag_refs));
        props.insert(
            "title".to_string(),
            PropertyValue::String("Test Article".to_string()),
        );

        columns.insert("properties".to_string(), PropertyValue::Object(props));

        Row::from_map(columns)
    }

    fn make_test_row_without_tags() -> Row {
        let mut columns = IndexMap::new();
        columns.insert(
            "id".to_string(),
            PropertyValue::String("test-id".to_string()),
        );
        columns.insert(
            "path".to_string(),
            PropertyValue::String("/demonews/articles/tech/test-article".to_string()),
        );

        let mut props = HashMap::new();
        props.insert(
            "title".to_string(),
            PropertyValue::String("Test Article".to_string()),
        );

        columns.insert("properties".to_string(), PropertyValue::Object(props));

        Row::from_map(columns)
    }

    #[test]
    fn test_references_match() {
        let func = ReferencesFunction;
        let row = make_test_row_with_tags(vec![
            ("social", "/demonews/tags/tech-stack/rust"),
            ("social", "/demonews/tags/topics/trending"),
        ]);

        // Should match the rust tag
        let args = vec![TypedExpr::literal(Literal::Text(
            "social:/demonews/tags/tech-stack/rust".to_string(),
        ))];
        assert_eq!(func.evaluate(&args, &row).unwrap(), Literal::Boolean(true));

        // Should match the trending tag
        let args = vec![TypedExpr::literal(Literal::Text(
            "social:/demonews/tags/topics/trending".to_string(),
        ))];
        assert_eq!(func.evaluate(&args, &row).unwrap(), Literal::Boolean(true));
    }

    #[test]
    fn test_references_no_match() {
        let func = ReferencesFunction;
        let row = make_test_row_with_tags(vec![("social", "/demonews/tags/tech-stack/rust")]);

        // Should not match typescript tag
        let args = vec![TypedExpr::literal(Literal::Text(
            "social:/demonews/tags/tech-stack/typescript".to_string(),
        ))];
        assert_eq!(func.evaluate(&args, &row).unwrap(), Literal::Boolean(false));

        // Should not match different workspace
        let args = vec![TypedExpr::literal(Literal::Text(
            "other:/demonews/tags/tech-stack/rust".to_string(),
        ))];
        assert_eq!(func.evaluate(&args, &row).unwrap(), Literal::Boolean(false));
    }

    #[test]
    fn test_references_no_tags() {
        let func = ReferencesFunction;
        let row = make_test_row_without_tags();

        let args = vec![TypedExpr::literal(Literal::Text(
            "social:/demonews/tags/tech-stack/rust".to_string(),
        ))];
        assert_eq!(func.evaluate(&args, &row).unwrap(), Literal::Boolean(false));
    }

    #[test]
    fn test_references_null_target() {
        let func = ReferencesFunction;
        let row = make_test_row_with_tags(vec![("social", "/demonews/tags/tech-stack/rust")]);

        let args = vec![TypedExpr::literal(Literal::Null)];
        assert_eq!(func.evaluate(&args, &row).unwrap(), Literal::Null);
    }

    #[test]
    fn test_references_invalid_format() {
        let func = ReferencesFunction;
        let row = make_test_row_with_tags(vec![("social", "/demonews/tags/tech-stack/rust")]);

        // Missing colon separator
        let args = vec![TypedExpr::literal(Literal::Text(
            "/demonews/tags/tech-stack/rust".to_string(),
        ))];
        assert!(func.evaluate(&args, &row).is_err());
    }
}
