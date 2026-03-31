//! WASM bindings for RaisinDB SQL parser validation and completion
//!
//! Provides real-time SQL validation and intelligent completions in the browser
//! by exposing the Rust parser via WebAssembly. Includes support for embedded
//! Cypher query validation in CYPHER() function calls.
//!
//! Submodules:
//! - `types` - Type definitions (ValidationError, ValidationResult, catalog types)
//! - `validation` - SQL validation (DDL, ORDER, BRANCH, standard SQL)
//! - `cypher` - Cypher extraction and validation
//! - `completion` - Completion and function signature APIs

mod completion;
mod cypher;
mod types;
mod validation;

use types::{TableDef, TABLE_CATALOG};
use wasm_bindgen::prelude::*;

// =============================================================================
// WASM Exports
// =============================================================================

/// Initialize the WASM module (sets up panic hook for better error messages)
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Set the table catalog from JavaScript
///
/// @param catalog - Array of TableDef objects: [{ name: string, columns: [...] }, ...]
#[wasm_bindgen]
pub fn set_table_catalog(catalog: JsValue) -> Result<(), JsValue> {
    let tables: Vec<TableDef> = serde_wasm_bindgen::from_value(catalog)
        .map_err(|e| JsValue::from_str(&format!("Invalid catalog format: {}", e)))?;

    TABLE_CATALOG.with(|c| {
        let mut cat = c.borrow_mut();
        cat.clear();
        for table in tables {
            cat.insert(table.name.clone(), table);
        }
    });

    Ok(())
}

/// Clear the table catalog
#[wasm_bindgen]
pub fn clear_table_catalog() {
    TABLE_CATALOG.with(|c| c.borrow_mut().clear());
}

/// Get current table names (for autocomplete)
#[wasm_bindgen]
pub fn get_table_names() -> JsValue {
    TABLE_CATALOG.with(|c| {
        let names: Vec<String> = c.borrow().keys().cloned().collect();
        serde_wasm_bindgen::to_value(&names).unwrap_or(JsValue::NULL)
    })
}

/// Get columns for a specific table (for autocomplete)
#[wasm_bindgen]
pub fn get_table_columns(table_name: &str) -> JsValue {
    TABLE_CATALOG.with(|c| {
        let cat = c.borrow();
        if let Some(table) = cat.get(table_name) {
            serde_wasm_bindgen::to_value(&table.columns).unwrap_or(JsValue::NULL)
        } else {
            JsValue::NULL
        }
    })
}

/// Validate a SQL string and return errors with positions
///
/// This function attempts to parse the SQL and returns a ValidationResult
/// containing any errors with their line/column positions.
/// Also validates embedded Cypher queries in CYPHER() function calls.
#[wasm_bindgen]
pub fn validate_sql(sql: &str) -> JsValue {
    let result = validation::validate_sql_internal(sql);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Validate a standalone Cypher query string
///
/// This function validates a Cypher query independent of SQL context.
/// Useful for testing or validating Cypher queries before embedding in SQL.
#[wasm_bindgen]
pub fn validate_cypher(cypher: &str) -> JsValue {
    let result = cypher::validate_cypher_internal(cypher);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

#[cfg(test)]
mod tests {
    use super::cypher::{extract_cypher_blocks, extract_string_content, validate_cypher_blocks};
    use super::cypher::validate_cypher_internal;
    use super::types::{TableDef, TABLE_CATALOG};
    use super::validation::{position_to_line_column, validate_sql_internal};

    #[test]
    fn test_valid_sql() {
        let result = validate_sql_internal("SELECT * FROM nodes");
        assert!(result.success);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_invalid_sql() {
        let result = validate_sql_internal("SELEC * FROM nodes");
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_valid_ddl() {
        let result =
            validate_sql_internal("CREATE NODETYPE 'test:Article' PROPERTIES (title String)");
        assert!(result.success);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_empty_sql() {
        let result = validate_sql_internal("");
        assert!(result.success);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_position_to_line_column() {
        let text = "line1\nline2\nline3";
        assert_eq!(position_to_line_column(text, 0), (1, 1));
        assert_eq!(position_to_line_column(text, 5), (1, 6));
        assert_eq!(position_to_line_column(text, 6), (2, 1));
        assert_eq!(position_to_line_column(text, 12), (3, 1));
    }

    #[test]
    fn test_ddl_error_position_for_typo() {
        let sql = "CREATE NODETYPE 'test:Page' PROPERTIES (views Numer DEFAULT 0)";
        let result = validate_sql_internal(sql);

        assert!(!result.success, "Invalid DDL should fail");
        assert_eq!(result.errors.len(), 1);

        let err = &result.errors[0];
        assert_eq!(err.line, 1, "Error should be on line 1");
        assert!(
            err.column > 1,
            "Error should have a position, got column {}. Message: {}",
            err.column,
            err.message
        );
        assert!(
            err.message.contains("Numer")
                || err.message.contains("Invalid")
                || err.message.contains("error"),
            "Error message should mention the issue. Got: {}",
            err.message
        );
    }

    #[test]
    fn test_ddl_valid_property_type() {
        let sql = "CREATE NODETYPE 'test:Page' PROPERTIES (views Number DEFAULT 0)";
        let result = validate_sql_internal(sql);
        assert!(result.success, "Valid DDL should parse successfully");
    }

    // =========================================================================
    // Cypher Validation Tests
    // =========================================================================

    #[test]
    fn test_valid_cypher() {
        let result = validate_cypher_internal("MATCH (n:Person) RETURN n.name");
        assert!(result.success, "Valid Cypher should pass");
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_invalid_cypher() {
        let result = validate_cypher_internal("MATC (n) RETURN n");
        assert!(!result.success, "Invalid Cypher should fail");
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_empty_cypher() {
        let result = validate_cypher_internal("");
        assert!(result.success, "Empty Cypher should pass");
    }

    #[test]
    fn test_extract_cypher_blocks_single() {
        let sql = "SELECT * FROM CYPHER('MATCH (n) RETURN n')";
        let blocks = extract_cypher_blocks(sql);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, "MATCH (n) RETURN n");
    }

    #[test]
    fn test_extract_cypher_blocks_double_quoted() {
        let sql = r#"SELECT * FROM CYPHER("MATCH (n) RETURN n")"#;
        let blocks = extract_cypher_blocks(sql);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, "MATCH (n) RETURN n");
    }

    #[test]
    fn test_extract_cypher_blocks_multiline() {
        let sql = "SELECT * FROM CYPHER('\n  MATCH (n:Person)\n  RETURN n.name\n')";
        let blocks = extract_cypher_blocks(sql);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].content.contains("MATCH (n:Person)"));
        assert!(blocks[0].content.contains("RETURN n.name"));
    }

    #[test]
    fn test_extract_cypher_blocks_with_whitespace() {
        let sql = "SELECT * FROM CYPHER  (  'MATCH (n) RETURN n'  )";
        let blocks = extract_cypher_blocks(sql);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, "MATCH (n) RETURN n");
    }

    #[test]
    fn test_sql_with_embedded_cypher_valid() {
        let cypher = "MATCH (n:Person) RETURN n.name";
        let result = validate_cypher_internal(cypher);
        assert!(result.success, "Valid embedded Cypher should pass");
    }

    #[test]
    fn test_cypher_error_position_mapping() {
        let sql = "SELECT * FROM CYPHER('MATC (n) RETURN n')";
        let blocks = extract_cypher_blocks(sql);
        assert_eq!(blocks.len(), 1);

        let errors = validate_cypher_blocks(&blocks);
        assert!(!errors.is_empty(), "Invalid Cypher should produce errors");
    }

    #[test]
    fn test_extract_string_content() {
        let (content, _) = extract_string_content("hello world'", '\'').unwrap();
        assert_eq!(content, "hello world");

        let (content, _) = extract_string_content("it''s here'", '\'').unwrap();
        assert_eq!(content, "it's here");

        let (content, _) = extract_string_content("line1\\nline2'", '\'').unwrap();
        assert_eq!(content, "line1\nline2");
    }

    // =========================================================================
    // Multi-Statement SQL Tests
    // =========================================================================

    #[test]
    fn test_multi_statement_transaction() {
        let sql = "BEGIN; SELECT id FROM nodes; COMMIT";
        let result = validate_sql_internal(sql);
        assert!(result.success, "Multi-statement transaction should be valid: {:?}", result.errors);
    }

    #[test]
    fn test_multi_statement_with_multiple_selects() {
        let sql = "BEGIN; SELECT id FROM nodes; SELECT name FROM nodes; COMMIT";
        let result = validate_sql_internal(sql);
        assert!(result.success, "Multi-statement with multiple SELECTs should be valid: {:?}", result.errors);
    }

    #[test]
    fn test_single_statement_still_works() {
        let result = validate_sql_internal("SELECT id, name FROM nodes WHERE version > 1");
        assert!(result.success, "Single statement should still work: {:?}", result.errors);
    }

    #[test]
    fn test_catalog_with_workspace() {
        TABLE_CATALOG.with(|c| {
            let mut cat = c.borrow_mut();
            cat.insert("my_workspace".to_string(), TableDef {
                name: "my_workspace".to_string(),
                columns: vec![],
            });
        });

        let result = validate_sql_internal("SELECT id, name FROM my_workspace");

        TABLE_CATALOG.with(|c| c.borrow_mut().clear());

        assert!(result.success, "Query against registered workspace should succeed: {:?}", result.errors);
    }

    #[test]
    fn test_catalog_with_workspace_batch() {
        TABLE_CATALOG.with(|c| {
            let mut cat = c.borrow_mut();
            cat.insert("social".to_string(), TableDef {
                name: "social".to_string(),
                columns: vec![],
            });
        });

        let result = validate_sql_internal("BEGIN; SELECT id FROM social; COMMIT");

        TABLE_CATALOG.with(|c| c.borrow_mut().clear());

        assert!(result.success, "Multi-statement with workspace should succeed: {:?}", result.errors);
    }

    // =========================================================================
    // UPSERT Statement Tests
    // =========================================================================

    #[test]
    fn test_upsert_syntax_valid() {
        TABLE_CATALOG.with(|c| {
            let mut cat = c.borrow_mut();
            cat.insert("content".to_string(), TableDef {
                name: "content".to_string(),
                columns: vec![],
            });
        });

        let result = validate_sql_internal("UPSERT INTO content (path, node_type) VALUES ('/articles/new', 'cms:Article')");

        TABLE_CATALOG.with(|c| c.borrow_mut().clear());

        assert!(result.success, "UPSERT should be valid like INSERT: {:?}", result.errors);
    }

    #[test]
    fn test_upsert_invalid_table() {
        let result = validate_sql_internal("UPSERT INTO nonexistent (path, node_type) VALUES ('/path', 'cms:Page')");
        assert!(!result.success, "UPSERT to non-existent workspace should fail");
    }

    // =========================================================================
    // ORDER Statement Tests
    // =========================================================================

    #[test]
    fn test_valid_order_above_with_paths() {
        let result = validate_sql_internal("ORDER Page SET path='/content/page1' ABOVE path='/content/page2'");
        assert!(result.success, "Valid ORDER ABOVE should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_order_below_with_paths() {
        let result = validate_sql_internal("ORDER Page SET path='/content/page1' BELOW path='/content/page2'");
        assert!(result.success, "Valid ORDER BELOW should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_order_with_ids() {
        let result = validate_sql_internal("ORDER Page SET id='abc123' ABOVE id='def456'");
        assert!(result.success, "Valid ORDER with IDs should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_order_mixed_path_id() {
        let result = validate_sql_internal("ORDER Page SET id='abc123' BELOW path='/content/page'");
        assert!(result.success, "Valid ORDER with mixed path/id should pass: {:?}", result.errors);
    }

    #[test]
    fn test_invalid_order_missing_target() {
        let result = validate_sql_internal("ORDER Page SET path='/content/page' ABOVE");
        assert!(!result.success, "ORDER without target should fail");
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_invalid_order_wrong_position() {
        let result = validate_sql_internal("ORDER Page SET path='/a' BESIDE path='/b'");
        assert!(!result.success, "ORDER with invalid position should fail");
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_invalid_order_old_syntax() {
        let result = validate_sql_internal("ORDER path='/a' ABOVE path='/b'");
        assert!(!result.success, "Old ORDER syntax without Table SET should fail");
    }

    // =========================================================================
    // BRANCH Statement Tests
    // =========================================================================

    #[test]
    fn test_valid_create_branch_basic() {
        let result = validate_sql_internal("CREATE BRANCH 'feature/new-feature' FROM 'main'");
        assert!(result.success, "Basic CREATE BRANCH should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_create_branch_unquoted() {
        let result = validate_sql_internal("CREATE BRANCH feature_branch FROM main");
        assert!(result.success, "CREATE BRANCH with unquoted names should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_create_branch_with_revision() {
        let result = validate_sql_internal("CREATE BRANCH 'hotfix/urgent' FROM 'production' AT REVISION 1734567890123_42");
        assert!(result.success, "CREATE BRANCH with AT REVISION should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_create_branch_with_head_relative() {
        let result = validate_sql_internal("CREATE BRANCH 'hotfix/urgent' FROM 'production' AT REVISION HEAD~5");
        assert!(result.success, "CREATE BRANCH with HEAD~N should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_create_branch_full_options() {
        let result = validate_sql_internal(
            "CREATE BRANCH 'develop' FROM 'main' AT REVISION HEAD~2 DESCRIPTION 'Development branch' PROTECTED UPSTREAM 'main' WITH HISTORY"
        );
        assert!(result.success, "CREATE BRANCH with all options should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_drop_branch() {
        let result = validate_sql_internal("DROP BRANCH 'feature/old-feature'");
        assert!(result.success, "DROP BRANCH should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_drop_branch_if_exists() {
        let result = validate_sql_internal("DROP BRANCH IF EXISTS 'feature/maybe-exists'");
        assert!(result.success, "DROP BRANCH IF EXISTS should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_alter_branch_set_upstream() {
        let result = validate_sql_internal("ALTER BRANCH 'feature/x' SET UPSTREAM 'main'");
        assert!(result.success, "ALTER BRANCH SET UPSTREAM should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_alter_branch_unset_upstream() {
        let result = validate_sql_internal("ALTER BRANCH 'feature/x' UNSET UPSTREAM");
        assert!(result.success, "ALTER BRANCH UNSET UPSTREAM should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_alter_branch_protected() {
        let result = validate_sql_internal("ALTER BRANCH 'production' SET PROTECTED TRUE");
        assert!(result.success, "ALTER BRANCH SET PROTECTED should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_alter_branch_rename() {
        let result = validate_sql_internal("ALTER BRANCH 'old-name' RENAME TO 'new-name'");
        assert!(result.success, "ALTER BRANCH RENAME TO should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_merge_branch_basic() {
        let result = validate_sql_internal("MERGE BRANCH 'feature/complete' INTO 'main'");
        assert!(result.success, "Basic MERGE BRANCH should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_merge_branch_with_strategy() {
        let result = validate_sql_internal("MERGE BRANCH 'hotfix/urgent' INTO 'production' USING FAST_FORWARD");
        assert!(result.success, "MERGE BRANCH with strategy should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_merge_branch_with_message() {
        let result = validate_sql_internal("MERGE BRANCH 'feature/x' INTO 'develop' USING THREE_WAY MESSAGE 'Merge feature X'");
        assert!(result.success, "MERGE BRANCH with message should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_use_branch() {
        let result = validate_sql_internal("USE BRANCH 'develop'");
        assert!(result.success, "USE BRANCH should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_checkout_branch() {
        let result = validate_sql_internal("CHECKOUT BRANCH develop");
        assert!(result.success, "CHECKOUT BRANCH should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_show_branches() {
        let result = validate_sql_internal("SHOW BRANCHES");
        assert!(result.success, "SHOW BRANCHES should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_show_current_branch() {
        let result = validate_sql_internal("SHOW CURRENT BRANCH");
        assert!(result.success, "SHOW CURRENT BRANCH should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_describe_branch() {
        let result = validate_sql_internal("DESCRIBE BRANCH 'main'");
        assert!(result.success, "DESCRIBE BRANCH should pass: {:?}", result.errors);
    }

    #[test]
    fn test_valid_show_divergence() {
        let result = validate_sql_internal("SHOW DIVERGENCE 'feature/x' FROM 'main'");
        assert!(result.success, "SHOW DIVERGENCE should pass: {:?}", result.errors);
    }

    #[test]
    fn test_invalid_create_branch_missing_from() {
        let result = validate_sql_internal("CREATE BRANCH FROM 'main'");
        assert!(!result.success, "CREATE BRANCH without name should fail");
    }

    #[test]
    fn test_invalid_merge_branch_missing_into() {
        let result = validate_sql_internal("MERGE BRANCH 'feature/x'");
        assert!(!result.success, "MERGE BRANCH without INTO should fail");
    }
}
