//! Completion Provider
//!
//! Generates context-aware completion suggestions based on SQL context,
//! table catalog, and function registry.

mod helpers;
mod keywords;
mod schema;

use super::context::{analyze_context, SqlContext};
use super::types::CompletionResult;
use crate::analyzer::catalog::Catalog;
use crate::analyzer::functions::FunctionRegistry;

/// Completion provider with access to schema and functions
pub struct CompletionProvider<'a> {
    catalog: &'a dyn Catalog,
    functions: &'a FunctionRegistry,
}

impl<'a> CompletionProvider<'a> {
    /// Create a new completion provider
    pub fn new(catalog: &'a dyn Catalog, functions: &'a FunctionRegistry) -> Self {
        Self { catalog, functions }
    }

    /// Generate completions for SQL at cursor position
    pub fn provide_completions(&self, sql: &str, cursor_offset: usize) -> CompletionResult {
        let ctx = analyze_context(sql, cursor_offset);
        let mut result = CompletionResult::default();

        match &ctx.context {
            SqlContext::StatementStart => {
                self.add_statement_keywords(&mut result);
            }
            SqlContext::SelectClause => {
                self.add_columns_from_context(&ctx, &mut result);
                self.add_functions(&mut result);
                self.add_select_keywords(&mut result);
            }
            SqlContext::FromClause | SqlContext::JoinClause => {
                self.add_tables(&mut result);
                self.add_table_functions(&mut result);
            }
            SqlContext::WhereClause | SqlContext::JoinCondition | SqlContext::HavingClause => {
                self.add_columns_from_context(&ctx, &mut result);
                self.add_functions(&mut result);
                self.add_expression_keywords(&mut result);
            }
            SqlContext::GroupByClause | SqlContext::OrderByClause => {
                self.add_columns_from_context(&ctx, &mut result);
            }
            SqlContext::SetClause => {
                self.add_columns_from_context(&ctx, &mut result);
            }
            SqlContext::AfterDot { qualifier } => {
                self.add_columns_for_qualifier(qualifier, &ctx, &mut result);
            }
            SqlContext::FunctionArgument {
                function_name,
                arg_index,
            } => {
                self.add_function_argument_completions(
                    function_name,
                    *arg_index,
                    &ctx,
                    &mut result,
                );
            }
            SqlContext::AfterCreate | SqlContext::AfterAlter | SqlContext::AfterDrop => {
                self.add_schema_object_keywords(&mut result);
            }
            SqlContext::PropertyDefinition | SqlContext::PropertyType => {
                self.add_property_types(&mut result);
            }
            SqlContext::InsertClause | SqlContext::ValuesClause => {
                self.add_columns_from_context(&ctx, &mut result);
            }
            SqlContext::OrderPosition => {
                self.add_order_position_keywords(&mut result);
            }
            SqlContext::MoveTarget => {
                self.add_move_keywords(&mut result);
            }
            SqlContext::Expression => {
                self.add_columns_from_context(&ctx, &mut result);
                self.add_functions(&mut result);
                self.add_expression_keywords(&mut result);
            }
            SqlContext::Unknown => {
                // Fallback: show keywords
                self.add_common_keywords(&mut result);
            }
        }

        // Filter by partial word
        if !ctx.partial_word.is_empty() {
            let filter = ctx.partial_word.to_uppercase();
            result.items.retain(|item| {
                let label = item.filter_text.as_ref().unwrap_or(&item.label);
                label.to_uppercase().starts_with(&filter)
            });
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::catalog::StaticCatalog;

    fn test_catalog() -> StaticCatalog {
        let mut catalog = StaticCatalog::default_nodes_schema();
        catalog.register_workspace("social".to_string());
        catalog
    }

    #[test]
    fn test_statement_start_completions() {
        let catalog = test_catalog();
        let functions = FunctionRegistry::default();
        let provider = CompletionProvider::new(&catalog, &functions);

        let result = provider.provide_completions("", 0);
        assert!(!result.items.is_empty());

        let labels: Vec<_> = result.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"SELECT"));
        assert!(labels.contains(&"INSERT"));
        assert!(labels.contains(&"CREATE"));
    }

    #[test]
    fn test_from_clause_shows_tables() {
        let catalog = test_catalog();
        let functions = FunctionRegistry::default();
        let provider = CompletionProvider::new(&catalog, &functions);

        let result = provider.provide_completions("SELECT * FROM ", 14);

        let labels: Vec<_> = result.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"nodes"));
        assert!(labels.contains(&"social"));
    }

    #[test]
    fn test_after_dot_shows_columns() {
        let catalog = test_catalog();
        let functions = FunctionRegistry::default();
        let provider = CompletionProvider::new(&catalog, &functions);

        let result = provider.provide_completions("SELECT n. FROM nodes n", 9);

        let labels: Vec<_> = result.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"id"));
        assert!(labels.contains(&"path"));
        assert!(labels.contains(&"name"));
        assert!(labels.contains(&"properties"));
    }

    #[test]
    fn test_select_shows_functions() {
        let catalog = test_catalog();
        let functions = FunctionRegistry::default();
        let provider = CompletionProvider::new(&catalog, &functions);

        let result = provider.provide_completions("SELECT ", 7);

        let labels: Vec<_> = result.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"COUNT"));
        assert!(labels.contains(&"JSON_VALUE"));
    }

    #[test]
    fn test_partial_word_filtering() {
        let catalog = test_catalog();
        let functions = FunctionRegistry::default();
        let provider = CompletionProvider::new(&catalog, &functions);

        let result = provider.provide_completions("SELECT na", 9);

        // Should only show items starting with "na"
        for item in &result.items {
            let label = item.filter_text.as_ref().unwrap_or(&item.label);
            assert!(label.to_uppercase().starts_with("NA"));
        }
    }

    #[test]
    fn test_ddl_context() {
        let catalog = test_catalog();
        let functions = FunctionRegistry::default();
        let provider = CompletionProvider::new(&catalog, &functions);

        let result = provider.provide_completions("CREATE ", 7);

        let labels: Vec<_> = result.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"NODETYPE"));
        assert!(labels.contains(&"ARCHETYPE"));
        assert!(!labels.contains(&"SELECT"));
    }
}
