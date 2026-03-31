use raisin_sql::Analyzer;

fn analyze_query(sql: &str) -> Result<(), String> {
    let analyzer = Analyzer::new();
    analyzer.analyze(sql).map_err(|e| e.to_string())?;
    Ok(())
}

#[test]
fn test_row_number_basic() {
    let sql = r#"
        SELECT path, ROW_NUMBER() OVER (ORDER BY path) as rn
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_row_number_partition_by_parent() {
    let sql = r#"
        SELECT
            path,
            PARENT(path) as parent,
            ROW_NUMBER() OVER (PARTITION BY PARENT(path) ORDER BY path) as sibling_rank
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_rank_functions() {
    let sql = r#"
        SELECT
            path,
            RANK() OVER (ORDER BY version DESC) as rank,
            DENSE_RANK() OVER (ORDER BY version DESC) as dense_rank
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_count_over() {
    let sql = r#"
        SELECT
            path,
            COUNT(*) OVER (PARTITION BY PARENT(path)) as sibling_count
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_multiple_windows() {
    let sql = r#"
        SELECT
            path,
            ROW_NUMBER() OVER (ORDER BY path) as rn,
            COUNT(*) OVER (PARTITION BY node_type) as type_count
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_window_with_subquery() {
    let sql = r#"
        SELECT
            parent,
            total,
            ROW_NUMBER() OVER (ORDER BY total DESC) as rank
        FROM (
            SELECT PARENT(path) as parent, COUNT(*) as total
            FROM nodes
            GROUP BY PARENT(path)
        ) AS sub
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_sum_over_window() {
    let sql = r#"
        SELECT
            path,
            version,
            SUM(version) OVER (PARTITION BY PARENT(path) ORDER BY path) as running_sum
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_avg_over_window() {
    let sql = r#"
        SELECT
            path,
            version,
            AVG(version) OVER (PARTITION BY node_type) as avg_version
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_min_max_over_window() {
    let sql = r#"
        SELECT
            path,
            version,
            MIN(version) OVER (PARTITION BY PARENT(path)) as min_version,
            MAX(version) OVER (PARTITION BY PARENT(path)) as max_version
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_window_no_partition() {
    let sql = r#"
        SELECT
            path,
            ROW_NUMBER() OVER (ORDER BY created_at DESC) as global_order
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_window_no_order() {
    let sql = r#"
        SELECT
            path,
            COUNT(*) OVER (PARTITION BY node_type) as type_count
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_window_with_filter() {
    let sql = r#"
        SELECT
            path,
            ROW_NUMBER() OVER (PARTITION BY PARENT(path) ORDER BY version DESC) as rank
        FROM nodes
        WHERE node_type = 'DOCUMENT'
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_window_with_limit() {
    let sql = r#"
        SELECT
            path,
            ROW_NUMBER() OVER (ORDER BY created_at DESC) as rank
        FROM nodes
        LIMIT 10
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_window_partition_by_multiple_columns() {
    let sql = r#"
        SELECT
            path,
            node_type,
            ROW_NUMBER() OVER (PARTITION BY node_type, PARENT(path) ORDER BY path) as rank
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_window_order_by_multiple_columns() {
    let sql = r#"
        SELECT
            path,
            ROW_NUMBER() OVER (ORDER BY node_type ASC, created_at DESC) as rank
        FROM nodes
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}

#[test]
fn test_window_mixed_with_regular_aggregates_subquery() {
    // Window functions cannot be mixed with regular aggregates in same SELECT
    // But can be used in subquery results
    let sql = r#"
        SELECT
            parent,
            child_count,
            ROW_NUMBER() OVER (ORDER BY child_count DESC) as rank
        FROM (
            SELECT PARENT(path) as parent, COUNT(*) as child_count
            FROM nodes
            GROUP BY PARENT(path)
        ) AS grouped
    "#;
    let result = analyze_query(sql);
    assert!(result.is_ok(), "Error: {:?}", result.err());
}
