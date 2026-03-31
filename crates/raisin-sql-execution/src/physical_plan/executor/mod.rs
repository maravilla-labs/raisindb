//! Physical Plan Execution Engine
//!
//! Provides the execution context and streaming execution for physical plans.
//! The execution model is async streaming using the Volcano-style iterator model.
//!
//! This module is split into focused submodules:
//! - `row`: Core `Row` type, `RowStream`, `ExecutionError`, and embedding cache
//! - `context`: `ExecutionContext` with storage, indexes, and query parameters
//! - `plan_dispatch`: Top-level `execute_plan` / `execute_plan_batch` dispatch
//! - `cte`: Common Table Expression materialization and scanning

mod cte;
mod plan_dispatch;

pub mod context;
pub(crate) mod row;

// Re-export the public API (unchanged from before the split)
pub use context::ExecutionContext;
pub use plan_dispatch::{execute_plan, execute_plan_batch};
pub use row::{ExecutionError, Row, RowStream};

// Re-export crate-visible items
pub(crate) use row::CachedEmbedding;

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use raisin_models::nodes::properties::PropertyValue;

    #[test]
    fn test_row_creation() {
        let row = Row::new();
        assert_eq!(row.columns.len(), 0);
    }

    #[test]
    fn test_row_insert_get() {
        let mut row = Row::new();
        row.insert("id".to_string(), PropertyValue::String("123".to_string()));
        row.insert("count".to_string(), PropertyValue::Integer(42));

        assert_eq!(
            row.get("id"),
            Some(&PropertyValue::String("123".to_string()))
        );
        assert_eq!(row.get("count"), Some(&PropertyValue::Integer(42)));
        assert_eq!(row.get("missing"), None);
    }

    #[test]
    fn test_row_contains() {
        let mut row = Row::new();
        row.insert("id".to_string(), PropertyValue::String("123".to_string()));

        assert!(row.contains("id"));
        assert!(!row.contains("missing"));
    }

    #[test]
    fn test_row_column_names() {
        let mut row = Row::new();
        row.insert("id".to_string(), PropertyValue::String("123".to_string()));
        row.insert(
            "name".to_string(),
            PropertyValue::String("test".to_string()),
        );

        let mut names = row.column_names();
        names.sort();
        assert_eq!(names, vec!["id", "name"]);
    }

    #[test]
    fn test_row_from_map() {
        let mut map = IndexMap::new();
        map.insert("id".to_string(), PropertyValue::String("123".to_string()));

        let row = Row::from_map(map);
        assert_eq!(
            row.get("id"),
            Some(&PropertyValue::String("123".to_string()))
        );
    }
}
