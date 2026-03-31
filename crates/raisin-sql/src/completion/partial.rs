// TODO(v0.2): Partial SQL completion support
#![allow(dead_code)]

//! Partial SQL Handling
//!
//! Strategies for analyzing incomplete SQL that wouldn't normally parse.
//! This enables completions while the user is typing.

/// SQL patching result
#[derive(Debug, Clone)]
pub enum PatchResult {
    /// SQL was already complete, no patching needed
    Complete,
    /// SQL was patched to be parseable
    Patched(String),
    /// SQL could not be patched, use token-based analysis
    Unparseable,
}

/// Attempt to patch incomplete SQL to make it parseable
///
/// This is used to enable semantic analysis on incomplete queries
/// for better completion suggestions.
pub fn patch_incomplete_sql(sql: &str) -> PatchResult {
    // Don't trim - we need to check trailing patterns
    // Use trim_start() only to check for empty input
    if sql.trim().is_empty() {
        return PatchResult::Unparseable;
    }

    // Try parsing as-is first (might already be complete)
    // We don't actually parse here - that's done by the caller
    // This function just patches obvious incomplete patterns

    // Work with original (or right-trimmed) for pattern matching
    let trimmed = sql.trim_end();
    let upper = sql.to_uppercase();
    let upper_trimmed = trimmed.to_uppercase();

    // Pattern: Function call with open paren "FUNC(" -> add placeholder and close
    // Check this EARLY before other patterns that might match partial function calls
    if trimmed.ends_with('(') {
        return PatchResult::Patched(format!("{}__arg__)", trimmed));
    }

    // Pattern: Function call with partial args "FUNC(a, " -> add placeholder
    if sql.ends_with(", ") || trimmed.ends_with(',') {
        return PatchResult::Patched(format!("{}__arg__)", trimmed));
    }

    // Pattern: "SELECT " -> "SELECT * FROM __placeholder__"
    if upper_trimmed == "SELECT" || upper_trimmed.ends_with(" SELECT") {
        return PatchResult::Patched(format!("{} * FROM __placeholder__", trimmed));
    }

    // Pattern: "SELECT x" (no FROM) -> "SELECT x FROM __placeholder__"
    if upper_trimmed.starts_with("SELECT ") && !upper_trimmed.contains(" FROM ") {
        return PatchResult::Patched(format!("{} FROM __placeholder__", trimmed));
    }

    // Pattern: "SELECT x FROM " -> "SELECT x FROM __placeholder__"
    if upper.ends_with(" FROM ") || upper_trimmed.ends_with(" FROM") {
        return PatchResult::Patched(format!("{}__placeholder__", trimmed));
    }

    // Pattern: "SELECT x FROM table WHERE " -> add "TRUE"
    if upper.ends_with(" WHERE ") || upper_trimmed.ends_with(" WHERE") {
        return PatchResult::Patched(format!("{} TRUE", trimmed));
    }

    // Pattern: "SELECT x FROM table WHERE a = " -> add placeholder
    // Check both trimmed and untrimmed versions
    if upper.ends_with(" = ")
        || upper.ends_with(" < ")
        || upper.ends_with(" > ")
        || upper.ends_with(" != ")
        || upper.ends_with(" <> ")
        || upper.ends_with(" <= ")
        || upper.ends_with(" >= ")
        || upper_trimmed.ends_with(" =")
        || upper_trimmed.ends_with(" <")
        || upper_trimmed.ends_with(" >")
        || upper_trimmed.ends_with(" !=")
        || upper_trimmed.ends_with(" <>")
        || upper_trimmed.ends_with(" <=")
        || upper_trimmed.ends_with(" >=")
    {
        return PatchResult::Patched(format!("{} 1", trimmed));
    }

    // Pattern: "SELECT x FROM table WHERE a AND " -> add "TRUE"
    if upper.ends_with(" AND ") || upper_trimmed.ends_with(" AND") {
        return PatchResult::Patched(format!("{} TRUE", trimmed));
    }

    // Pattern: "SELECT x FROM table WHERE a OR " -> add "TRUE"
    if upper.ends_with(" OR ") || upper_trimmed.ends_with(" OR") {
        return PatchResult::Patched(format!("{} TRUE", trimmed));
    }

    // Pattern: "SELECT x FROM table JOIN " -> add placeholder table
    if upper.ends_with(" JOIN ") || upper_trimmed.ends_with(" JOIN") {
        return PatchResult::Patched(format!("{} __placeholder__ ON TRUE", trimmed));
    }

    // Pattern: "SELECT x FROM table JOIN t ON " -> add "TRUE"
    if upper.ends_with(" ON ") || upper_trimmed.ends_with(" ON") {
        return PatchResult::Patched(format!("{} TRUE", trimmed));
    }

    // Pattern: "SELECT x FROM table GROUP BY " -> add placeholder column
    if (upper.ends_with(" BY ") || upper_trimmed.ends_with(" BY")) && upper.contains("GROUP") {
        return PatchResult::Patched(format!("{} __col__", trimmed));
    }

    // Pattern: "SELECT x FROM table ORDER BY " -> add placeholder column
    if (upper.ends_with(" BY ") || upper_trimmed.ends_with(" BY")) && upper.contains("ORDER") {
        return PatchResult::Patched(format!("{} __col__", trimmed));
    }

    // Pattern: "INSERT INTO " -> add placeholder
    if upper.ends_with(" INTO ") || upper_trimmed.ends_with(" INTO") {
        return PatchResult::Patched(format!("{} __placeholder__ VALUES (1)", trimmed));
    }

    // Pattern: "UPDATE " -> add placeholder
    if upper_trimmed == "UPDATE"
        || upper_trimmed.ends_with(" UPDATE")
        || upper.ends_with(" UPDATE ")
    {
        return PatchResult::Patched(format!("{} __placeholder__ SET __col__ = 1", trimmed));
    }

    // Pattern: "UPDATE table SET " -> add placeholder assignment
    if upper.ends_with(" SET ") || upper_trimmed.ends_with(" SET") {
        return PatchResult::Patched(format!("{} __col__ = 1", trimmed));
    }

    // Pattern: "DELETE FROM " -> add placeholder
    if (upper.ends_with(" FROM ") || upper_trimmed.ends_with(" FROM")) && upper.contains("DELETE") {
        return PatchResult::Patched(format!("{}__placeholder__", trimmed));
    }

    // For everything else, try adding a semicolon if missing
    if !trimmed.ends_with(';') {
        // This might help with some parsers
        return PatchResult::Patched(format!("{};", trimmed));
    }

    PatchResult::Complete
}

/// Check if SQL ends in an incomplete state for a specific clause
pub fn is_incomplete_clause(sql: &str) -> bool {
    let upper = sql.trim().to_uppercase();

    // Check for trailing keywords that expect more input
    upper.ends_with(" SELECT")
        || upper.ends_with(" FROM")
        || upper.ends_with(" WHERE")
        || upper.ends_with(" AND")
        || upper.ends_with(" OR")
        || upper.ends_with(" ON")
        || upper.ends_with(" JOIN")
        || upper.ends_with(" BY")
        || upper.ends_with(" SET")
        || upper.ends_with(" INTO")
        || upper.ends_with(" =")
        || upper.ends_with(" <")
        || upper.ends_with(" >")
        || upper.ends_with(" !=")
        || upper.ends_with(" <>")
        || upper.ends_with(" <=")
        || upper.ends_with(" >=")
        || upper.ends_with(" IN")
        || upper.ends_with(" LIKE")
        || upper.ends_with(" BETWEEN")
        || upper.ends_with(" AS")
        || upper.ends_with(" NOT")
        || sql.trim().ends_with('(')
        || sql.trim().ends_with(',')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_patch_select_only() {
        match patch_incomplete_sql("SELECT") {
            PatchResult::Patched(s) => {
                assert!(s.contains("FROM"));
                assert!(s.contains("__placeholder__"));
            }
            _ => panic!("Expected Patched"),
        }
    }

    #[test]
    fn test_patch_select_columns() {
        match patch_incomplete_sql("SELECT id, name") {
            PatchResult::Patched(s) => {
                assert!(s.contains("FROM"));
            }
            _ => panic!("Expected Patched"),
        }
    }

    #[test]
    fn test_patch_from_incomplete() {
        match patch_incomplete_sql("SELECT * FROM ") {
            PatchResult::Patched(s) => {
                assert!(s.contains("__placeholder__"));
            }
            _ => panic!("Expected Patched"),
        }
    }

    #[test]
    fn test_patch_where_incomplete() {
        match patch_incomplete_sql("SELECT * FROM nodes WHERE ") {
            PatchResult::Patched(s) => {
                assert!(s.contains("TRUE"));
            }
            _ => panic!("Expected Patched"),
        }
    }

    #[test]
    fn test_patch_comparison_incomplete() {
        match patch_incomplete_sql("SELECT * FROM nodes WHERE id = ") {
            PatchResult::Patched(s) => {
                assert!(s.ends_with("1"));
            }
            _ => panic!("Expected Patched"),
        }
    }

    #[test]
    fn test_patch_function_open() {
        match patch_incomplete_sql("SELECT JSON_VALUE(") {
            PatchResult::Patched(s) => {
                assert!(s.ends_with(")"));
            }
            _ => panic!("Expected Patched"),
        }
    }

    #[test]
    fn test_is_incomplete() {
        assert!(is_incomplete_clause("SELECT * FROM "));
        assert!(is_incomplete_clause("SELECT * FROM nodes WHERE "));
        assert!(is_incomplete_clause("SELECT * FROM nodes WHERE id = "));
        assert!(is_incomplete_clause("SELECT JSON_VALUE("));
        assert!(!is_incomplete_clause("SELECT * FROM nodes"));
    }
}
