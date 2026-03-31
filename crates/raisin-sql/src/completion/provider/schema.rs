//! Schema-based completion methods (tables, columns, functions)
//!
//! Provides suggestions from the catalog and function registry based on
//! the current SQL context.

use super::helpers::{format_data_type, is_type_compatible};
use super::CompletionProvider;
use crate::analyzer::functions::{FunctionCategory, FunctionSignature};
use crate::analyzer::types::DataType;
use crate::completion::context::AnalyzedContext;
use crate::completion::types::{CompletionItem, CompletionResult};

impl<'a> CompletionProvider<'a> {
    // =========================================================================
    // Table completions
    // =========================================================================

    pub(super) fn add_tables(&self, result: &mut CompletionResult) {
        // Add all tables from catalog
        for table_name in self.catalog.list_tables() {
            let is_workspace = self.catalog.is_workspace(table_name);
            result.add(CompletionItem::table(table_name, is_workspace).with_sort_priority(0));
        }
    }

    pub(super) fn add_table_functions(&self, result: &mut CompletionResult) {
        // Table-valued functions that return rows
        let table_funcs = [
            ("CHILDREN", &["parent_path"][..], "Child nodes of parent"),
            ("DESCENDANTS", &["ancestor_path"][..], "All descendants"),
            ("CYPHER", &["query"][..], "Execute Cypher graph query"),
            (
                "KNN",
                &["query_vector", "k"][..],
                "K-nearest neighbors search",
            ),
            (
                "NEIGHBORS",
                &["node_id", "direction", "type"][..],
                "Graph neighbors",
            ),
            (
                "FULLTEXT_SEARCH",
                &["query", "language"][..],
                "Full-text search",
            ),
        ];

        for (name, params, desc) in table_funcs {
            let params_str: Vec<String> = params.iter().map(|s| s.to_string()).collect();
            let mut item = CompletionItem::function(name, &params_str, "Table", "TableFunction");
            item.detail = Some(desc.to_string());
            item.sort_text = Some(format!("10{}", name));
            result.add(item);
        }
    }

    // =========================================================================
    // Column completions
    // =========================================================================

    pub(super) fn add_columns_from_context(
        &self,
        ctx: &AnalyzedContext,
        result: &mut CompletionResult,
    ) {
        // If we have specific tables, show their columns
        if !ctx.from_tables.is_empty() {
            for table_name in &ctx.from_tables {
                self.add_columns_for_table(table_name, None, result);
            }
        } else {
            // Default: show columns from 'nodes' table
            self.add_columns_for_table("nodes", None, result);
        }
    }

    pub(super) fn add_columns_for_qualifier(
        &self,
        qualifier: &str,
        ctx: &AnalyzedContext,
        result: &mut CompletionResult,
    ) {
        // Resolve qualifier to table name
        let table_name = ctx
            .aliases
            .get(qualifier)
            .map(|s| s.as_str())
            .unwrap_or(qualifier);

        self.add_columns_for_table(table_name, Some(qualifier), result);
    }

    fn add_columns_for_table(
        &self,
        table_name: &str,
        _qualifier: Option<&str>,
        result: &mut CompletionResult,
    ) {
        // Try to get table definition
        let table_def = if let Some(def) = self.catalog.get_table(table_name) {
            Some(def.clone())
        } else if self.catalog.is_workspace(table_name) {
            self.catalog.get_workspace_table(table_name)
        } else {
            None
        };

        if let Some(def) = table_def {
            for col in &def.columns {
                let data_type = format_data_type(&col.data_type);
                result.add(
                    CompletionItem::column(&col.name, data_type, col.nullable)
                        .with_sort_priority(0),
                );
            }
        }
    }

    // =========================================================================
    // Function completions
    // =========================================================================

    pub(super) fn add_functions(&self, result: &mut CompletionResult) {
        // Add all functions from registry
        let function_names = [
            // Hierarchy
            "PATH_STARTS_WITH",
            "PARENT",
            "DEPTH",
            "ANCESTOR",
            "CHILD_OF",
            "DESCENDANT_OF",
            // JSON
            "JSON_VALUE",
            "JSON_QUERY",
            "JSON_EXISTS",
            "JSON_GET_TEXT",
            "JSON_GET_DOUBLE",
            "JSON_GET_INT",
            "JSON_GET_BOOL",
            // Aggregate
            "COUNT",
            "SUM",
            "AVG",
            "MIN",
            "MAX",
            "ARRAY_AGG",
            // Scalar
            "LOWER",
            "UPPER",
            "LENGTH",
            "ROUND",
            "COALESCE",
            // Temporal
            "NOW",
            // Full-text
            "to_tsvector",
            "to_tsquery",
            "FULLTEXT_MATCH",
            // Vector
            "EMBEDDING",
            "VECTOR_L2_DISTANCE",
            "VECTOR_COSINE_DISTANCE",
            "VECTOR_INNER_PRODUCT",
        ];

        for name in function_names {
            if let Some(signatures) = self.functions.get_signatures(name) {
                if let Some(sig) = signatures.first() {
                    let item = self.signature_to_completion(sig);
                    result.add(item);
                }
            }
        }
    }

    pub(super) fn add_function_argument_completions(
        &self,
        function_name: &str,
        arg_index: usize,
        ctx: &AnalyzedContext,
        result: &mut CompletionResult,
    ) {
        // Get function signature to determine expected type
        if let Some(signatures) = self.functions.get_signatures(function_name) {
            for sig in signatures {
                if arg_index < sig.params.len() {
                    let expected_type = &sig.params[arg_index];
                    // Add columns that match the expected type
                    self.add_typed_columns(expected_type, ctx, result);
                }
            }
        }

        // Always show columns as fallback
        if result.items.is_empty() {
            self.add_columns_from_context(ctx, result);
        }
    }

    fn add_typed_columns(
        &self,
        expected_type: &DataType,
        ctx: &AnalyzedContext,
        result: &mut CompletionResult,
    ) {
        // Get columns from context tables
        let tables: Vec<&str> = if ctx.from_tables.is_empty() {
            vec!["nodes"]
        } else {
            ctx.from_tables.iter().map(|s| s.as_str()).collect()
        };

        for table_name in tables {
            let table_def = if let Some(def) = self.catalog.get_table(table_name) {
                Some(def.clone())
            } else if self.catalog.is_workspace(table_name) {
                self.catalog.get_workspace_table(table_name)
            } else {
                None
            };

            if let Some(def) = table_def {
                for col in &def.columns {
                    // Check if column type is compatible
                    if is_type_compatible(&col.data_type, expected_type) {
                        let data_type = format_data_type(&col.data_type);
                        result.add(
                            CompletionItem::column(&col.name, data_type, col.nullable)
                                .with_sort_priority(0),
                        );
                    }
                }
            }
        }
    }

    pub(super) fn signature_to_completion(&self, sig: &FunctionSignature) -> CompletionItem {
        let params: Vec<String> = sig.params.iter().map(format_data_type).collect();
        let return_type = format_data_type(&sig.return_type);
        let category = format!("{:?}", sig.category);

        let mut item = if matches!(sig.category, FunctionCategory::Aggregate) {
            CompletionItem::aggregate(&sig.name, &params, return_type)
        } else {
            CompletionItem::function(&sig.name, &params, return_type, &category)
        };

        // Set sort priority based on category
        let priority = match sig.category {
            FunctionCategory::Aggregate => 5,
            FunctionCategory::Hierarchy => 6,
            FunctionCategory::Json => 7,
            FunctionCategory::Scalar => 8,
            FunctionCategory::FullText => 9,
            FunctionCategory::Vector => 10,
            FunctionCategory::Temporal => 11,
            FunctionCategory::System => 12,
            FunctionCategory::Geospatial => 13,
            FunctionCategory::Auth => 14,
        };
        item = item.with_sort_priority(priority);

        item
    }
}
