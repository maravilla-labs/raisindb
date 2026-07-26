//! Integration tests for RaisinSQL AST parser
//!
//! Tests parse real SQL files to ensure the parser works correctly

use anyhow::Result;
use raisin_sql::ast::parse_sql;
use std::fs;
use std::path::PathBuf;

/// Helper function to get the path to SQL test files
fn get_sql_test_file(filename: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("sql")
        .join(filename)
}

/// Helper function to parse a SQL file and return the number of statements
fn parse_sql_file(filename: &str) -> Result<usize> {
    let path = get_sql_test_file(filename);
    let sql = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path.display(), e))?;

    // Split by semicolons and parse each statement separately
    // This handles multi-statement SQL files
    let mut total_statements = 0;

    for (idx, chunk) in sql.split(';').enumerate() {
        let trimmed = chunk.trim();
        // Skip empty statements and comments-only lines
        if trimmed.is_empty()
            || trimmed
                .lines()
                .all(|l| l.trim().is_empty() || l.trim().starts_with("--"))
        {
            continue;
        }

        match parse_sql(trimmed) {
            Ok(statements) => {
                total_statements += statements.len();
            }
            Err(e) => {
                eprintln!(
                    "Error parsing SQL from {} (statement #{}):",
                    filename,
                    idx + 1
                );
                eprintln!("Error: {}", e);
                eprintln!("Statement: {}", trimmed);
                return Err(e.into());
            }
        }
    }

    if total_statements == 0 {
        eprintln!("Warning: No valid statements found in {}", filename);
    }

    Ok(total_statements)
}

#[test]
fn test_01_basic_select() -> Result<()> {
    let count = parse_sql_file("01_basic_select.sql")?;
    assert!(count > 0, "Should parse at least one SELECT statement");
    println!("✓ Parsed {} basic SELECT statements", count);
    Ok(())
}

#[test]
fn test_02_hierarchy_functions() -> Result<()> {
    let count = parse_sql_file("02_hierarchy_functions.sql")?;
    assert!(count > 0, "Should parse hierarchy function statements");
    println!("✓ Parsed {} hierarchy function statements", count);
    Ok(())
}

#[test]
fn test_03_json_operations() -> Result<()> {
    let count = parse_sql_file("03_json_operations.sql")?;
    assert!(count > 0, "Should parse JSON operation statements");
    println!("✓ Parsed {} JSON operation statements", count);
    Ok(())
}

#[test]
fn test_04_vector_graph() -> Result<()> {
    let count = parse_sql_file("04_vector_graph.sql")?;
    assert!(count > 0, "Should parse vector/graph query statements");
    println!("✓ Parsed {} vector/graph statements", count);
    Ok(())
}

#[test]
fn test_05_insert() -> Result<()> {
    let count = parse_sql_file("05_insert.sql")?;
    assert!(count > 0, "Should parse INSERT statements");
    println!("✓ Parsed {} INSERT statements", count);
    Ok(())
}

#[test]
fn test_06_update() -> Result<()> {
    let count = parse_sql_file("06_update.sql")?;
    assert!(count > 0, "Should parse UPDATE statements");
    println!("✓ Parsed {} UPDATE statements", count);
    Ok(())
}

#[test]
fn test_07_delete() -> Result<()> {
    let count = parse_sql_file("07_delete.sql")?;
    assert!(count > 0, "Should parse DELETE statements");
    println!("✓ Parsed {} DELETE statements", count);
    Ok(())
}

#[test]
fn test_08_list_children() -> Result<()> {
    let count = parse_sql_file("08_list_children.sql")?;
    assert!(count > 0, "Should parse list children statements");
    println!("✓ Parsed {} list children statements", count);
    Ok(())
}

#[test]
fn test_09_pagination() -> Result<()> {
    let count = parse_sql_file("09_pagination.sql")?;
    assert!(count > 0, "Should parse pagination statements");
    println!("✓ Parsed {} pagination statements", count);
    Ok(())
}

#[test]
fn test_10_fulltext_search() -> Result<()> {
    let count = parse_sql_file("10_fulltext_search.sql")?;
    assert!(count > 0, "Should parse full-text search statements");
    println!("✓ Parsed {} full-text search statements", count);
    Ok(())
}

#[test]
fn test_all_sql_files() -> Result<()> {
    let sql_files = vec![
        "01_basic_select.sql",
        "02_hierarchy_functions.sql",
        "03_json_operations.sql",
        "04_vector_graph.sql",
        "05_insert.sql",
        "06_update.sql",
        "07_delete.sql",
        "08_list_children.sql",
        "09_pagination.sql",
        "10_fulltext_search.sql",
    ];

    let mut total = 0;
    for file in sql_files {
        let count = parse_sql_file(file)?;
        total += count;
        println!("  {} -> {} statements", file, count);
    }

    println!("\n✓ Total: {} SQL statements parsed successfully", total);
    Ok(())
}

/// Test specific RaisinDB function validation
#[test]
fn test_path_starts_with_validation() -> Result<()> {
    // Valid: 2 arguments
    let sql = "SELECT * FROM nodes WHERE PATH_STARTS_WITH(path, '/content/')";
    assert!(parse_sql(sql).is_ok());

    // Invalid: 1 argument (should fail)
    let sql_invalid = "SELECT * FROM nodes WHERE PATH_STARTS_WITH(path)";
    assert!(parse_sql(sql_invalid).is_err());

    Ok(())
}

#[test]
fn test_json_value_validation() -> Result<()> {
    // Valid: 2 arguments
    let sql = "SELECT JSON_VALUE(properties, '$.title') FROM nodes";
    assert!(parse_sql(sql).is_ok());

    // Valid: with RETURNING clause
    let sql_returning = "SELECT JSON_VALUE(properties, '$.price' RETURNING DOUBLE) FROM nodes";
    assert!(parse_sql(sql_returning).is_ok());

    Ok(())
}

#[test]
fn test_parent_function_validation() -> Result<()> {
    // Valid: 1 argument
    let sql = "SELECT * FROM nodes WHERE PARENT(path) = '/content'";
    assert!(parse_sql(sql).is_ok());

    // Invalid: 2 arguments (should fail)
    let sql_invalid = "SELECT * FROM nodes WHERE PARENT(path, '/content')";
    assert!(parse_sql(sql_invalid).is_err());

    Ok(())
}

#[test]
fn test_depth_function_validation() -> Result<()> {
    // Valid: 1 argument
    let sql = "SELECT DEPTH(path) FROM nodes";
    assert!(parse_sql(sql).is_ok());

    // Invalid: no arguments (should fail)
    let sql_invalid = "SELECT DEPTH() FROM nodes";
    assert!(parse_sql(sql_invalid).is_err());

    Ok(())
}

#[test]
fn test_invalid_table_name() -> Result<()> {
    // Should reject non-'nodes' table
    let sql = "SELECT * FROM users";
    assert!(parse_sql(sql).is_err());

    let sql = "INSERT INTO posts (id) VALUES ('123')";
    assert!(parse_sql(sql).is_err());

    Ok(())
}

#[test]
fn test_complex_query() -> Result<()> {
    let sql = r#"
        SELECT
            id,
            name,
            path,
            DEPTH(path) as depth,
            properties ->> 'title' AS title,
            JSON_VALUE(properties, '$.status') AS status
        FROM nodes
        WHERE PATH_STARTS_WITH(path, '/content/')
        AND DEPTH(path) BETWEEN 2 AND 4
        AND properties @> '{"status": "published"}'
        AND JSON_EXISTS(properties, '$.seo')
        ORDER BY created_at DESC
        LIMIT 100
    "#;

    let result = parse_sql(sql)?;
    assert_eq!(result.len(), 1);

    Ok(())
}

#[test]
fn test_json_operators() -> Result<()> {
    // Test ->> operator
    let sql = "SELECT properties ->> 'title' FROM nodes";
    assert!(parse_sql(sql).is_ok());

    // Test @> operator
    let sql = "SELECT * FROM nodes WHERE properties @> '{\"status\": \"active\"}'";
    assert!(parse_sql(sql).is_ok());

    Ok(())
}

#[test]
fn test_json_column_selection() -> Result<()> {
    // Postgres-style: select JSONB column directly without TO_JSON
    let sql = "SELECT properties FROM nodes WHERE id = 'node-123'";
    assert!(parse_sql(sql).is_ok());

    // Select multiple columns including JSONB
    let sql = "SELECT id, name, properties, translations FROM nodes";
    assert!(parse_sql(sql).is_ok());

    Ok(())
}
