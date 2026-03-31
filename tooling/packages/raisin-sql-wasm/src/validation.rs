//! SQL validation logic for DDL, ORDER, BRANCH, and standard SQL statements

use raisin_sql::analyzer::{Analyzer, Catalog, StaticCatalog};

use crate::cypher::{extract_cypher_blocks, validate_cypher_blocks};
use crate::types::{ValidationError, ValidationResult, TABLE_CATALOG};

/// Validate a SQL string internally (dispatches to appropriate validator)
pub(crate) fn validate_sql_internal(sql: &str) -> ValidationResult {
    let trimmed = sql.trim();

    if trimmed.is_empty() {
        return ValidationResult {
            success: true,
            errors: vec![],
        };
    }

    // Check if this is a DDL statement (CREATE, ALTER, DROP for NODETYPE/ARCHETYPE/ELEMENTTYPE)
    if is_ddl_statement(trimmed) {
        return validate_ddl(sql);
    }

    // Check if this is an ORDER statement
    if is_order_statement(trimmed) {
        return validate_order(sql);
    }

    // Check if this is a BRANCH statement
    if is_branch_statement(trimmed) {
        return validate_branch(sql);
    }

    // Otherwise, validate as standard SQL
    validate_standard_sql(sql)
}

/// Check if the SQL is a DDL statement (CREATE/ALTER/DROP NODETYPE/ARCHETYPE/ELEMENTTYPE)
fn is_ddl_statement(sql: &str) -> bool {
    let upper = sql.to_uppercase();
    let words: Vec<&str> = upper.split_whitespace().collect();

    if words.len() < 2 {
        return false;
    }

    matches!(words[0], "CREATE" | "ALTER" | "DROP")
        && matches!(words[1], "NODETYPE" | "ARCHETYPE" | "ELEMENTTYPE")
}

/// Check if the SQL is an ORDER statement
fn is_order_statement(sql: &str) -> bool {
    raisin_sql::ast::order_parser::is_order_statement(sql)
}

/// Check if the SQL is a BRANCH statement
fn is_branch_statement(sql: &str) -> bool {
    raisin_sql::ast::branch_parser::is_branch_statement(sql)
}

/// Build a catalog with registered workspaces from the table catalog
fn build_catalog() -> Box<dyn Catalog> {
    TABLE_CATALOG.with(|c| {
        let mut catalog = StaticCatalog::default_nodes_schema();
        for workspace_name in c.borrow().keys() {
            catalog.register_workspace(workspace_name.clone());
        }
        Box::new(catalog) as Box<dyn Catalog>
    })
}

/// Validate BRANCH statements using the Analyzer with catalog
fn validate_branch(sql: &str) -> ValidationResult {
    let catalog = build_catalog();
    let analyzer = Analyzer::with_catalog(catalog);
    match analyzer.analyze(sql) {
        Ok(_) => ValidationResult {
            success: true,
            errors: vec![],
        },
        Err(e) => {
            let (line, column) = extract_position_from_error(&e.to_string(), sql);
            ValidationResult {
                success: false,
                errors: vec![ValidationError {
                    line,
                    column,
                    end_line: line,
                    end_column: column + 10,
                    message: e.to_string(),
                    severity: "error".to_string(),
                }],
            }
        }
    }
}

/// Validate ORDER statements using the Analyzer with catalog
fn validate_order(sql: &str) -> ValidationResult {
    let catalog = build_catalog();
    let analyzer = Analyzer::with_catalog(catalog);
    match analyzer.analyze(sql) {
        Ok(_) => ValidationResult {
            success: true,
            errors: vec![],
        },
        Err(e) => {
            let (line, column) = extract_position_from_error(&e.to_string(), sql);
            ValidationResult {
                success: false,
                errors: vec![ValidationError {
                    line,
                    column,
                    end_line: line,
                    end_column: column + 10,
                    message: e.to_string(),
                    severity: "error".to_string(),
                }],
            }
        }
    }
}

/// Validate DDL statements (CREATE NODETYPE, etc.)
fn validate_ddl(sql: &str) -> ValidationResult {
    match raisin_sql::ast::ddl_parser::parse_ddl(sql) {
        Ok(_) => ValidationResult {
            success: true,
            errors: vec![],
        },
        Err(e) => {
            let pos = e.position.unwrap_or(0);
            let (line, column) = position_to_line_column(sql, pos);

            // Calculate end_column by finding the end of the problematic token
            let remaining = if pos < sql.len() { &sql[pos..] } else { "" };
            let token_len = remaining
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .count()
                .max(1);

            ValidationResult {
                success: false,
                errors: vec![ValidationError {
                    line,
                    column,
                    end_line: line,
                    end_column: column + token_len,
                    message: e.message,
                    severity: "error".to_string(),
                }],
            }
        }
    }
}

/// Validate standard SQL statements (SELECT, INSERT, UPDATE, DELETE)
/// Also validates embedded Cypher queries in CYPHER() function calls
/// Supports multi-statement SQL (e.g., BEGIN; UPDATE...; COMMIT;)
fn validate_standard_sql(sql: &str) -> ValidationResult {
    let catalog = build_catalog();

    // Collect all errors
    let mut all_errors = Vec::new();

    // Use the Analyzer for proper SQL validation with workspace support
    let analyzer = Analyzer::with_catalog(catalog);
    if let Err(e) = analyzer.analyze_batch(sql) {
        let (line, column) = extract_position_from_error(&e.to_string(), sql);
        all_errors.push(ValidationError {
            line,
            column,
            end_line: line,
            end_column: column + 10,
            message: e.to_string(),
            severity: "error".to_string(),
        });
    }

    // Validate embedded Cypher blocks in CYPHER() function calls
    let cypher_blocks = extract_cypher_blocks(sql);
    let cypher_errors = validate_cypher_blocks(&cypher_blocks);
    all_errors.extend(cypher_errors);

    ValidationResult {
        success: all_errors.is_empty(),
        errors: all_errors,
    }
}

/// Convert a byte offset to line and column numbers (1-based)
pub(crate) fn position_to_line_column(text: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    let mut current_offset = 0;

    for c in text.chars() {
        if current_offset >= byte_offset {
            break;
        }
        if c == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
        current_offset += c.len_utf8();
    }

    (line, column)
}

/// Try to extract position information from sqlparser error message
///
/// sqlparser errors look like: "sql parser error: Expected ..., found: ... at Line: X, Column: Y"
pub(crate) fn extract_position_from_error(error_msg: &str, sql: &str) -> (usize, usize) {
    // Try to find "at Line: X, Column: Y" pattern
    if let Some(line_idx) = error_msg.find("Line: ") {
        let after_line = &error_msg[line_idx + 6..];
        if let Some(comma_idx) = after_line.find(',') {
            if let Ok(line) = after_line[..comma_idx].trim().parse::<usize>() {
                // Find column
                if let Some(col_idx) = after_line.find("Column: ") {
                    let after_col = &after_line[col_idx + 8..];
                    let col_end = after_col
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(after_col.len());
                    if let Ok(column) = after_col[..col_end].trim().parse::<usize>() {
                        return (line, column);
                    }
                }
            }
        }
    }

    // Fallback: try to find the problematic token in the SQL
    if let Some(found_idx) = error_msg.to_lowercase().find("found:") {
        let after_found = &error_msg[found_idx + 6..];
        let token = after_found.trim().split_whitespace().next().unwrap_or("");
        let clean_token = token.trim_matches(|c| c == '\'' || c == '"' || c == '`');
        if !clean_token.is_empty() {
            if let Some(pos) = sql.to_lowercase().find(&clean_token.to_lowercase()) {
                return position_to_line_column(sql, pos);
            }
        }
    }

    // Default to first line
    (1, 1)
}
