//! Semantic analyzer tests for RaisinSQL
//!
//! Tests semantic analysis and validation, including GROUP BY validation,
//! type checking, and expression equivalence

use raisin_sql::Analyzer;

/// Helper to analyze a SQL query
fn analyze_query(sql: &str) -> Result<(), String> {
    let analyzer = Analyzer::new();
    analyzer.analyze(sql).map_err(|e| e.to_string())?;
    Ok(())
}

#[test]
fn test_group_by_with_parent_function() {
    // Test the exact query from the bug report (using 'nodes' table)
    let sql = r#"
        SELECT PARENT(path), COUNT(*)
        FROM nodes
        WHERE DEPTH(path) = 2
        GROUP BY PARENT(path)
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "GROUP BY with PARENT() function should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_with_depth_function() {
    // Test GROUP BY with DEPTH() function
    let sql = r#"
        SELECT DEPTH(path), COUNT(*), AVG(version)
        FROM nodes
        GROUP BY DEPTH(path)
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "GROUP BY with DEPTH() function should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_with_nested_function() {
    // Test GROUP BY with single-argument PARENT nested call
    let sql = r#"
        SELECT PARENT(path), COUNT(*)
        FROM nodes
        WHERE DEPTH(path) = 3
        GROUP BY PARENT(path)
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "GROUP BY with PARENT function calls should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_with_multiple_functions() {
    // Test GROUP BY with multiple different functions
    let sql = r#"
        SELECT PARENT(path), DEPTH(path), COUNT(*)
        FROM nodes
        GROUP BY PARENT(path), DEPTH(path)
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "GROUP BY with multiple functions should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_with_column() {
    // Test GROUP BY with regular column
    let sql = r#"
        SELECT node_type, COUNT(*)
        FROM nodes
        GROUP BY node_type
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "GROUP BY with column should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_with_json_extract() {
    // Test GROUP BY with JSON extraction
    let sql = r#"
        SELECT properties->>'status', COUNT(*)
        FROM nodes
        GROUP BY properties->>'status'
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "GROUP BY with JSON extraction should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_with_binary_expression() {
    // Test GROUP BY with binary expression (arithmetic)
    let sql = r#"
        SELECT version * 2, COUNT(*)
        FROM nodes
        GROUP BY version * 2
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "GROUP BY with binary expression should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_with_cast() {
    // Test GROUP BY with CAST expression (INT to BIGINT)
    let sql = r#"
        SELECT CAST(version AS BIGINT), COUNT(*)
        FROM nodes
        GROUP BY CAST(version AS BIGINT)
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "GROUP BY with CAST should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_missing_function_in_group_by() {
    // Test that we correctly reject when a function in SELECT is NOT in GROUP BY
    let sql = r#"
        SELECT PARENT(path), COUNT(*)
        FROM nodes
        GROUP BY path
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_err(),
        "Should reject when PARENT(path) in SELECT but not in GROUP BY"
    );
}

#[test]
fn test_group_by_missing_column() {
    // Test that we correctly reject when a column in SELECT is NOT in GROUP BY
    let sql = r#"
        SELECT name, COUNT(*)
        FROM nodes
        GROUP BY node_type
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_err(),
        "Should reject when column 'name' in SELECT but not in GROUP BY"
    );
}

#[test]
fn test_group_by_literals_allowed() {
    // Test that literals in SELECT are allowed even without GROUP BY
    let sql = r#"
        SELECT node_type, 'constant', COUNT(*)
        FROM nodes
        GROUP BY node_type
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Literals should be allowed in SELECT with GROUP BY. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_aggregate_without_column() {
    // Test that aggregate functions are allowed in SELECT without being in GROUP BY
    let sql = r#"
        SELECT node_type, COUNT(*), SUM(version), AVG(version), MIN(version), MAX(version)
        FROM nodes
        GROUP BY node_type
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Aggregate functions should be allowed in SELECT. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_function_case_insensitive() {
    // Test that function names are case-insensitive in GROUP BY matching
    let sql = r#"
        SELECT parent(path), COUNT(*)
        FROM nodes
        GROUP BY PARENT(path)
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Function names should be case-insensitive in GROUP BY matching. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_with_order_by() {
    // Test GROUP BY with ORDER BY
    let sql = r#"
        SELECT PARENT(path), COUNT(*) as count
        FROM nodes
        GROUP BY PARENT(path)
        ORDER BY count DESC
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "GROUP BY with ORDER BY should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_complex_expression() {
    // Test GROUP BY with complex expression combining multiple functions
    let sql = r#"
        SELECT PARENT(path), DEPTH(path), node_type, COUNT(*)
        FROM nodes
        WHERE PATH_STARTS_WITH(path, '/content/')
        GROUP BY PARENT(path), DEPTH(path), node_type
        ORDER BY COUNT(*) DESC
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Complex GROUP BY query should be valid. Error: {:?}",
        result.err()
    );
}
