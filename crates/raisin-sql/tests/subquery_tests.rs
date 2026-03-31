//! Subquery tests for RaisinSQL
//!
//! Tests subqueries in FROM clause (derived tables)

use raisin_sql::Analyzer;

/// Helper to analyze a SQL query
fn analyze_query(sql: &str) -> Result<(), String> {
    let analyzer = Analyzer::new();
    analyzer.analyze(sql).map_err(|e| e.to_string())?;
    Ok(())
}

#[test]
fn test_simple_subquery() {
    // Test simple subquery in FROM clause
    let sql = r#"
        SELECT parent, COUNT(*)
        FROM (SELECT PARENT(path) as parent FROM nodes) AS sub
        GROUP BY parent
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Simple subquery should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_subquery_with_where() {
    // Test subquery with WHERE clause
    let sql = r#"
        SELECT parent, count
        FROM (
            SELECT PARENT(path) as parent, COUNT(*) as count
            FROM nodes
            WHERE DEPTH(path) = 2
            GROUP BY PARENT(path)
        ) AS sub
        WHERE count > 5
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Subquery with WHERE should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_subquery_with_join() {
    // Test subquery with JOIN
    let sql = r#"
        SELECT n.name, sub.child_count
        FROM nodes n
        JOIN (
            SELECT PARENT(path) as parent, COUNT(*) as child_count
            FROM nodes
            GROUP BY PARENT(path)
        ) AS sub ON sub.parent = n.path
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Subquery with JOIN should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_nested_subquery() {
    // Test nested subqueries
    let sql = r#"
        SELECT parent, max_count
        FROM (
            SELECT parent, MAX(child_count) as max_count
            FROM (
                SELECT PARENT(path) as parent, COUNT(*) as child_count
                FROM nodes
                GROUP BY PARENT(path)
            ) AS inner_sub
            GROUP BY parent
        ) AS outer_sub
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Nested subqueries should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_subquery_with_multiple_columns() {
    // Test subquery with multiple columns
    let sql = r#"
        SELECT parent, depth_val, total
        FROM (
            SELECT
                PARENT(path) as parent,
                DEPTH(path) as depth_val,
                COUNT(*) as total
            FROM nodes
            GROUP BY PARENT(path), DEPTH(path)
        ) AS sub
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Subquery with multiple columns should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_subquery_with_order_by() {
    // Test subquery with ORDER BY in outer query
    let sql = r#"
        SELECT parent, total
        FROM (
            SELECT PARENT(path) as parent, COUNT(*) as total
            FROM nodes
            GROUP BY PARENT(path)
        ) AS sub
        ORDER BY total DESC
        LIMIT 10
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Subquery with ORDER BY should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_subquery_without_alias_should_fail() {
    // Test that subqueries require an alias
    let sql = r#"
        SELECT parent, COUNT(*)
        FROM (SELECT PARENT(path) as parent FROM nodes)
        GROUP BY parent
    "#;

    let result = analyze_query(sql);
    assert!(result.is_err(), "Subquery without alias should fail");
}

#[test]
fn test_subquery_column_reference() {
    // Test referencing subquery columns
    let sql = r#"
        SELECT sub.parent, sub.total * 2 as doubled
        FROM (
            SELECT PARENT(path) as parent, COUNT(*) as total
            FROM nodes
            GROUP BY PARENT(path)
        ) AS sub
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Subquery column reference should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_subquery_with_json_extract() {
    // Test subquery with JSON extraction
    let sql = r#"
        SELECT status, total
        FROM (
            SELECT properties->>'status' as status, COUNT(*) as total
            FROM nodes
            GROUP BY properties->>'status'
        ) AS sub
        WHERE status = 'active'
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Subquery with JSON extraction should be valid. Error: {:?}",
        result.err()
    );
}

#[test]
fn test_subquery_explicit_columns() {
    // Test subquery with explicit column selection (SELECT * not yet supported for subqueries)
    let sql = r#"
        SELECT parent, total
        FROM (
            SELECT PARENT(path) as parent, COUNT(*) as total
            FROM nodes
            GROUP BY PARENT(path)
        ) AS sub
    "#;

    let result = analyze_query(sql);
    assert!(
        result.is_ok(),
        "Subquery with explicit columns should be valid. Error: {:?}",
        result.err()
    );
}
