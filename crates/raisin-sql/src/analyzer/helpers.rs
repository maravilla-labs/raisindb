//! Helper functions for the Analyzer
//!
//! This module contains utility functions used by the analyzer,
//! such as SQL statement splitting.

/// Split SQL string by semicolons, respecting string literals
///
/// This function correctly handles semicolons inside single-quoted strings,
/// so `'hello; world'` is not split.
pub(super) fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_string => {
                in_string = true;
                current.push(c);
            }
            '\'' if in_string => {
                current.push(c);
                // Check for escaped quote ('')
                if chars.peek() == Some(&'\'') {
                    current.push(chars.next().unwrap());
                } else {
                    in_string = false;
                }
            }
            ';' if !in_string => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    statements.push(trimmed.to_string());
                }
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }

    // Don't forget the last statement (may not end with semicolon)
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        statements.push(trimmed.to_string());
    }

    statements
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_sql_statements_simple() {
        let statements = split_sql_statements("SELECT 1; SELECT 2; SELECT 3");
        assert_eq!(statements.len(), 3);
        assert_eq!(statements[0], "SELECT 1");
        assert_eq!(statements[1], "SELECT 2");
        assert_eq!(statements[2], "SELECT 3");
    }

    #[test]
    fn test_split_sql_statements_with_strings() {
        // Semicolons inside strings should not split
        let statements = split_sql_statements("SELECT 'hello; world'; SELECT 2");
        assert_eq!(statements.len(), 2);
        assert_eq!(statements[0], "SELECT 'hello; world'");
        assert_eq!(statements[1], "SELECT 2");
    }

    #[test]
    fn test_split_sql_statements_escaped_quotes() {
        // Escaped single quotes should not end the string
        let statements = split_sql_statements("SELECT 'it''s ok'; SELECT 2");
        assert_eq!(statements.len(), 2);
        assert_eq!(statements[0], "SELECT 'it''s ok'");
        assert_eq!(statements[1], "SELECT 2");
    }

    #[test]
    fn test_split_sql_statements_no_trailing_semicolon() {
        let statements = split_sql_statements("SELECT 1; SELECT 2");
        assert_eq!(statements.len(), 2);
    }
}
