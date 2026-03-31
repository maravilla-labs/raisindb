//! RaisinDB SQL Dialect
//!
//! Defines the RaisinDialect which extends PostgreSQL syntax with
//! RaisinDB-specific functions and operators.

use sqlparser::dialect::Dialect;

/// RaisinDB SQL dialect
///
/// Based on PostgreSQL dialect with additional support for:
/// - Hierarchical path functions (PATH_STARTS_WITH, PARENT, DEPTH)
/// - JSON operators and functions (->>, @>, JSON_VALUE, JSON_EXISTS)
/// - Vector search (KNN table-valued function)
/// - Graph traversal (NEIGHBORS table-valued function)
#[derive(Debug, Default)]
pub struct RaisinDialect;

impl Dialect for RaisinDialect {
    fn is_identifier_start(&self, ch: char) -> bool {
        ch.is_ascii_lowercase() || ch.is_ascii_uppercase() || ch == '_' || ch == '$' || ch == '@'
    }

    fn is_identifier_part(&self, ch: char) -> bool {
        ch.is_ascii_lowercase()
            || ch.is_ascii_uppercase()
            || ch.is_ascii_digit()
            || ch == '_'
            || ch == '$'
            || ch == '@'
            || ch == ':'
    }

    fn supports_filter_during_aggregation(&self) -> bool {
        true
    }

    fn supports_within_after_array_aggregation(&self) -> bool {
        true
    }

    fn supports_group_by_expr(&self) -> bool {
        true
    }

    fn supports_parenthesized_set_variables(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identifier_start() {
        let dialect = RaisinDialect;
        assert!(dialect.is_identifier_start('a'));
        assert!(dialect.is_identifier_start('Z'));
        assert!(dialect.is_identifier_start('_'));
        assert!(dialect.is_identifier_start('$'));
        assert!(!dialect.is_identifier_start('1'));
    }

    #[test]
    fn test_identifier_part() {
        let dialect = RaisinDialect;
        assert!(dialect.is_identifier_part('a'));
        assert!(dialect.is_identifier_part('Z'));
        assert!(dialect.is_identifier_part('_'));
        assert!(dialect.is_identifier_part('$'));
        assert!(dialect.is_identifier_part('1'));
        assert!(dialect.is_identifier_part(':'));
    }
}
