//! Canonical column naming for GROUP BY key expressions.
//!
//! When a query groups by a computed expression (e.g. `properties ->> 'status'`
//! or `DEPTH(path)`), the aggregation operator materializes the group key into
//! its output row under a canonical column name, and the projection above it is
//! rewritten (see `PlanBuilder::exprs_match`) to reference that column instead
//! of re-evaluating the expression against the aggregated row — where the source
//! columns no longer exist.
//!
//! This module is the SINGLE source of truth for that name. It is used both by
//! the logical plan builder (projection/window rewrites) and by the hash
//! aggregate executor in the execution crate. Keeping them in one function is
//! what guarantees the producer (aggregate output row) and the consumer
//! (rewritten column reference) agree; they were previously duplicated and
//! drifted, which silently produced NULL group keys.

use crate::analyzer::{Expr, Literal, TypedExpr};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Generate the canonical output column name for a GROUP BY key expression.
///
/// Every expression gets a deterministic name:
/// - `Column` → `table.column`
/// - `Function` → `NAME(arg)` (recursive)
/// - `properties ->> 'key'` → `table.properties_key`
/// - `Cast` → the name of the inner expression (a cast changes the value's
///   type, not its identity as a group key; this also lets an uncast
///   projection of the same extraction find the materialized key)
/// - anything else → a stable hash of the expression structure, so even
///   arbitrary expressions (CASE, arithmetic, …) round-trip through the
///   aggregate output row instead of silently evaluating to NULL.
pub fn group_key_column_name(expr: &TypedExpr) -> String {
    match &expr.expr {
        Expr::Column { table, column } => {
            format!("{}.{}", table, column)
        }
        Expr::Function { name, args, .. } => {
            let func_name_upper = name.to_uppercase();
            if args.is_empty() {
                format!("{}()", func_name_upper)
            } else if args.len() == 1 {
                format!("{}({})", func_name_upper, group_key_column_name(&args[0]))
            } else {
                format!("{}(...)", func_name_upper)
            }
        }
        Expr::JsonExtractText { object, key } => {
            if let Expr::Column { table, column } = &object.expr {
                if let Expr::Literal(Literal::Text(key_str)) = &key.expr {
                    return format!("{}.{}_{}", table, column, key_str);
                }
            }
            structural_hash_name(expr)
        }
        // The analyzer turns `properties ->> 'k'::Type` into
        // `Cast { expr: JsonExtractText { .. }, target_type: Type }` — the cast
        // wraps the extraction. Name the group key after the inner expression.
        Expr::Cast { expr: inner, .. } => group_key_column_name(inner),
        _ => structural_hash_name(expr),
    }
}

/// Deterministic fallback name derived from the expression structure.
fn structural_hash_name(expr: &TypedExpr) -> String {
    let mut hasher = DefaultHasher::new();
    format!("{:?}", expr.expr).hash(&mut hasher);
    format!("group_{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::DataType;

    fn column(table: &str, name: &str) -> TypedExpr {
        TypedExpr::new(
            Expr::Column {
                table: table.to_string(),
                column: name.to_string(),
            },
            DataType::Text,
        )
    }

    fn json_extract_text(table: &str, column_name: &str, key: &str) -> TypedExpr {
        TypedExpr::new(
            Expr::JsonExtractText {
                object: Box::new(column(table, column_name)),
                key: Box::new(TypedExpr::new(
                    Expr::Literal(Literal::Text(key.to_string())),
                    DataType::Text,
                )),
            },
            DataType::Nullable(Box::new(DataType::Text)),
        )
    }

    #[test]
    fn column_name_is_qualified() {
        assert_eq!(group_key_column_name(&column("items", "path")), "items.path");
    }

    #[test]
    fn json_extract_text_gets_synthetic_name() {
        assert_eq!(
            group_key_column_name(&json_extract_text("items", "properties", "status")),
            "items.properties_status"
        );
    }

    #[test]
    fn cast_is_transparent() {
        // `properties ->> 'status'::String` analyzes to Cast{JsonExtractText};
        // the group-key name must be the inner extraction's name so an uncast
        // projection of the same extraction still finds the materialized key.
        let cast = TypedExpr::new(
            Expr::Cast {
                expr: Box::new(json_extract_text("items", "properties", "status")),
                target_type: DataType::Text,
            },
            DataType::Nullable(Box::new(DataType::Text)),
        );
        assert_eq!(group_key_column_name(&cast), "items.properties_status");
    }

    #[test]
    fn unknown_shapes_get_stable_names() {
        let case = TypedExpr::new(
            Expr::IsNull {
                expr: Box::new(column("items", "path")),
            },
            DataType::Boolean,
        );
        let name1 = group_key_column_name(&case);
        let name2 = group_key_column_name(&case);
        assert_eq!(name1, name2, "fallback name must be deterministic");
        assert!(name1.starts_with("group_"), "fallback name: {name1}");
    }
}
