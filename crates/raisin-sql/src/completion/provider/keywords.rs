//! Keyword completion methods
//!
//! Provides keyword suggestions for different SQL contexts (statement start,
//! SELECT clause, WHERE expressions, DDL, property types, etc.).

use super::CompletionProvider;
use crate::completion::types::{CompletionItem, CompletionKind, CompletionResult};

impl<'a> CompletionProvider<'a> {
    pub(super) fn add_statement_keywords(&self, result: &mut CompletionResult) {
        let keywords = [
            ("SELECT", "Query rows from tables"),
            ("INSERT", "Insert rows into a table"),
            ("UPDATE", "Update rows in a table"),
            ("DELETE", "Delete rows from a table"),
            (
                "CREATE",
                "Create schema objects (NODETYPE, ARCHETYPE, ELEMENTTYPE)",
            ),
            ("ALTER", "Modify schema objects"),
            ("DROP", "Remove schema objects"),
            ("BEGIN", "Start a transaction block"),
            ("EXPLAIN", "Show query execution plan"),
            ("ORDER", "Reorder sibling nodes"),
            ("MOVE", "Move node subtree to new parent"),
            ("WITH", "Common Table Expression (CTE)"),
        ];

        for (kw, desc) in keywords {
            result.add(
                CompletionItem::keyword(kw)
                    .with_detail(desc)
                    .with_sort_priority(0),
            );
        }
    }

    pub(super) fn add_select_keywords(&self, result: &mut CompletionResult) {
        let keywords = [
            ("DISTINCT", "Remove duplicate rows"),
            ("AS", "Alias for column"),
            ("CASE", "Conditional expression"),
            ("FROM", "Specify source table"),
        ];

        for (kw, desc) in keywords {
            result.add(
                CompletionItem::keyword(kw)
                    .with_detail(desc)
                    .with_sort_priority(5),
            );
        }
    }

    pub(super) fn add_expression_keywords(&self, result: &mut CompletionResult) {
        let keywords = [
            ("AND", "Logical AND"),
            ("OR", "Logical OR"),
            ("NOT", "Logical NOT"),
            ("IS NULL", "Check for NULL"),
            ("IS NOT NULL", "Check for non-NULL"),
            ("IN", "Match against list"),
            ("LIKE", "Pattern matching"),
            ("BETWEEN", "Range comparison"),
            ("TRUE", "Boolean true"),
            ("FALSE", "Boolean false"),
            ("NULL", "NULL value"),
        ];

        for (kw, desc) in keywords {
            result.add(
                CompletionItem::keyword(kw)
                    .with_detail(desc)
                    .with_sort_priority(10),
            );
        }
    }

    pub(super) fn add_common_keywords(&self, result: &mut CompletionResult) {
        self.add_statement_keywords(result);
        let keywords = [
            "FROM",
            "WHERE",
            "JOIN",
            "LEFT JOIN",
            "GROUP BY",
            "ORDER BY",
            "HAVING",
            "LIMIT",
            "OFFSET",
            "AND",
            "OR",
            "AS",
            "ON",
            "COMMIT",
            "ROLLBACK",
        ];
        for kw in keywords {
            result.add(CompletionItem::keyword(kw).with_sort_priority(20));
        }
    }

    pub(super) fn add_schema_object_keywords(&self, result: &mut CompletionResult) {
        let keywords = [
            ("NODETYPE", "Define a node type with properties"),
            ("ARCHETYPE", "Define an archetype for content editing"),
            ("ELEMENTTYPE", "Define an element type for nested content"),
        ];

        for (kw, desc) in keywords {
            result.add(
                CompletionItem::keyword(kw)
                    .with_detail(desc)
                    .with_sort_priority(0),
            );
        }
    }

    pub(super) fn add_property_types(&self, result: &mut CompletionResult) {
        let types = [
            ("String", "Text value"),
            ("Number", "Numeric value (integer or decimal)"),
            ("Boolean", "True/false value"),
            ("Date", "Date without time"),
            ("DateTime", "Date with time"),
            ("Timestamp", "Unix timestamp"),
            ("URL", "URL string"),
            ("Email", "Email address"),
            ("Phone", "Phone number"),
            ("Reference", "Reference to another node"),
            ("Media", "Media attachment"),
            ("RichText", "Rich text content"),
            ("Json", "JSON object"),
            ("Array", "Array of values"),
        ];

        for (t, desc) in types {
            let mut item = CompletionItem::keyword(t);
            item.kind = CompletionKind::Type;
            item.detail = Some(desc.to_string());
            item.sort_text = Some(format!("00{}", t));
            result.add(item);
        }
    }

    pub(super) fn add_order_position_keywords(&self, result: &mut CompletionResult) {
        result.add(
            CompletionItem::keyword("ABOVE")
                .with_detail("Position node before sibling")
                .with_sort_priority(0),
        );
        result.add(
            CompletionItem::keyword("BELOW")
                .with_detail("Position node after sibling")
                .with_sort_priority(0),
        );
    }

    pub(super) fn add_move_keywords(&self, result: &mut CompletionResult) {
        result.add(
            CompletionItem::keyword("TO")
                .with_detail("Target parent path")
                .with_sort_priority(0),
        );
    }
}
