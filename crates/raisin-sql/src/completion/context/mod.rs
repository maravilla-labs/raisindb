//! SQL Context Analyzer
//!
//! Determines the semantic context at a cursor position in SQL text.
//! This enables context-aware completions (e.g., tables after FROM,
//! columns after SELECT or table alias dot notation).

mod analysis;
pub(crate) mod tokenizer;
mod types;

pub use analysis::analyze_context;
pub use types::{AnalyzedContext, SqlContext, TableAlias};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_statement_start() {
        let ctx = analyze_context("", 0);
        assert_eq!(ctx.context, SqlContext::StatementStart);
    }

    #[test]
    fn test_select_clause() {
        let ctx = analyze_context("SELECT ", 7);
        assert_eq!(ctx.context, SqlContext::SelectClause);
    }

    #[test]
    fn test_from_clause() {
        let ctx = analyze_context("SELECT * FROM ", 14);
        assert_eq!(ctx.context, SqlContext::FromClause);
    }

    #[test]
    fn test_where_clause() {
        let ctx = analyze_context("SELECT * FROM nodes WHERE ", 26);
        assert_eq!(ctx.context, SqlContext::WhereClause);
    }

    #[test]
    fn test_after_dot() {
        let ctx = analyze_context("SELECT n.", 9);
        assert_eq!(
            ctx.context,
            SqlContext::AfterDot {
                qualifier: "n".to_string()
            }
        );
    }

    #[test]
    fn test_alias_extraction() {
        let ctx = analyze_context("SELECT * FROM nodes n WHERE n.", 30);
        assert!(ctx.aliases.contains_key("n"));
        assert_eq!(ctx.aliases.get("n"), Some(&"nodes".to_string()));
    }

    #[test]
    fn test_function_argument() {
        let ctx = analyze_context("SELECT JSON_VALUE(properties, ", 30);
        match ctx.context {
            SqlContext::FunctionArgument {
                function_name,
                arg_index,
            } => {
                assert_eq!(function_name, "JSON_VALUE");
                assert_eq!(arg_index, 1);
            }
            _ => panic!("Expected FunctionArgument context"),
        }
    }

    #[test]
    fn test_join_clause() {
        let ctx = analyze_context("SELECT * FROM nodes n JOIN ", 27);
        assert_eq!(ctx.context, SqlContext::JoinClause);
    }

    #[test]
    fn test_group_by() {
        let ctx = analyze_context("SELECT type, COUNT(*) FROM nodes GROUP BY ", 42);
        assert_eq!(ctx.context, SqlContext::GroupByClause);
    }

    #[test]
    fn test_order_by() {
        let ctx = analyze_context("SELECT * FROM nodes ORDER BY ", 29);
        assert_eq!(ctx.context, SqlContext::OrderByClause);
    }

    #[test]
    fn test_partial_word() {
        let ctx = analyze_context("SELECT na", 9);
        assert_eq!(ctx.partial_word, "na");
    }

    #[test]
    fn test_in_transaction() {
        let ctx = analyze_context("BEGIN; SELECT * FROM nodes; ", 28);
        assert!(ctx.in_transaction);

        let ctx2 = analyze_context("BEGIN; SELECT * FROM nodes; COMMIT; ", 36);
        assert!(!ctx2.in_transaction);
    }
}
