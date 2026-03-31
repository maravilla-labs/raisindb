//! Cypher query extraction and validation for embedded CYPHER() function calls

use raisin_cypher_parser::{parse_query as parse_cypher, ParseError as CypherParseError};

use crate::types::{ValidationError, ValidationResult};
use crate::validation::position_to_line_column;

/// Validate a standalone Cypher query string
pub(crate) fn validate_cypher_internal(cypher: &str) -> ValidationResult {
    let trimmed = cypher.trim();
    if trimmed.is_empty() {
        return ValidationResult {
            success: true,
            errors: vec![],
        };
    }

    match parse_cypher(cypher) {
        Ok(_) => ValidationResult {
            success: true,
            errors: vec![],
        },
        Err(e) => {
            let (line, column) = match &e {
                CypherParseError::SyntaxError { line, column, .. }
                | CypherParseError::UnexpectedToken { line, column, .. }
                | CypherParseError::InvalidSyntax { line, column, .. }
                | CypherParseError::UnexpectedEof { line, column, .. } => (*line, *column),
                CypherParseError::Incomplete(_) => (1, 1),
            };

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

/// Information about an embedded Cypher query in SQL
pub(crate) struct CypherBlock {
    /// The Cypher query content (without quotes)
    pub content: String,
    /// Line number in SQL where string content starts (1-based)
    pub start_line: usize,
    /// Column number in SQL where string content starts (1-based)
    pub start_column: usize,
}

/// Extract CYPHER() function string arguments from SQL
pub(crate) fn extract_cypher_blocks(sql: &str) -> Vec<CypherBlock> {
    let mut blocks = Vec::new();
    let upper = sql.to_uppercase();

    // Find all CYPHER( occurrences
    let mut search_start = 0;
    while let Some(pos) = upper[search_start..].find("CYPHER") {
        let abs_pos = search_start + pos;

        // Skip if CYPHER is part of a larger identifier
        if abs_pos > 0 {
            let prev_char = sql[..abs_pos].chars().last();
            if let Some(c) = prev_char {
                if c.is_alphanumeric() || c == '_' {
                    search_start = abs_pos + 6;
                    continue;
                }
            }
        }

        // Find opening parenthesis
        let after_cypher = &sql[abs_pos + 6..];
        let paren_pos = after_cypher
            .chars()
            .position(|c| !c.is_whitespace())
            .and_then(|p| {
                if after_cypher.chars().nth(p) == Some('(') {
                    Some(p)
                } else {
                    None
                }
            });

        if let Some(paren_offset) = paren_pos {
            let paren_abs = abs_pos + 6 + paren_offset;

            // Find string start (skip whitespace after paren)
            let after_paren = &sql[paren_abs + 1..];
            let ws_len = after_paren.len() - after_paren.trim_start().len();
            let string_start = paren_abs + 1 + ws_len;

            if string_start < sql.len() {
                let quote_char = sql.chars().nth(string_start);
                if quote_char == Some('\'') || quote_char == Some('"') {
                    let quote = quote_char.unwrap();
                    let content_start = string_start + 1;
                    if let Some((content, end_pos)) =
                        extract_string_content(&sql[content_start..], quote)
                    {
                        let (line, column) = position_to_line_column(sql, content_start);
                        blocks.push(CypherBlock {
                            content,
                            start_line: line,
                            start_column: column,
                        });
                        search_start = content_start + end_pos;
                        continue;
                    }
                }
            }
        }
        search_start = abs_pos + 6;
    }

    blocks
}

/// Extract string content until closing quote, handling escapes
pub(crate) fn extract_string_content(s: &str, quote: char) -> Option<(String, usize)> {
    let mut content = String::new();
    let mut chars = s.chars().peekable();
    let mut pos = 0;

    while let Some(c) = chars.next() {
        pos += c.len_utf8();
        if c == quote {
            // Check for escaped quote (doubled)
            if chars.peek() == Some(&quote) {
                chars.next();
                pos += quote.len_utf8();
                content.push(quote);
            } else {
                return Some((content, pos));
            }
        } else if c == '\\' {
            // Handle backslash escapes
            if let Some(&next) = chars.peek() {
                chars.next();
                pos += next.len_utf8();
                content.push(match next {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    _ => next,
                });
            }
        } else {
            content.push(c);
        }
    }

    None // Unclosed string
}

/// Validate Cypher blocks and map errors to SQL positions
pub(crate) fn validate_cypher_blocks(blocks: &[CypherBlock]) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for block in blocks {
        let trimmed = block.content.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Err(e) = parse_cypher(&block.content) {
            // Map Cypher error position to SQL document position
            let (cypher_line, cypher_col) = match &e {
                CypherParseError::SyntaxError { line, column, .. }
                | CypherParseError::UnexpectedToken { line, column, .. }
                | CypherParseError::InvalidSyntax { line, column, .. }
                | CypherParseError::UnexpectedEof { line, column, .. } => (*line, *column),
                CypherParseError::Incomplete(_) => (1, 1),
            };

            // Calculate absolute position in SQL
            let sql_line = block.start_line + cypher_line - 1;
            let sql_column = if cypher_line == 1 {
                block.start_column + cypher_col - 1
            } else {
                cypher_col
            };

            errors.push(ValidationError {
                line: sql_line,
                column: sql_column,
                end_line: sql_line,
                end_column: sql_column + 10,
                message: format!("Cypher: {}", e),
                severity: "error".to_string(),
            });
        }
    }

    errors
}
