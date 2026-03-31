//! SQL/PGQ GRAPH_TABLE parser
//!
//! Parses `GRAPH_TABLE(...)` expressions using nom combinators with
//! precise error location tracking.
//!
//! # Grammar
//!
//! ```text
//! graph_table_expr ::=
//!     GRAPH_TABLE '(' [graph_name] MATCH match_pattern [WHERE expr] COLUMNS '(' column_list ')' ')'
//!
//! match_pattern ::=
//!     path_pattern (',' path_pattern)*
//!
//! path_pattern ::=
//!     node_pattern (relationship_pattern node_pattern)*
//!
//! node_pattern ::=
//!     '(' [variable] [':' label ('|' label)*] [WHERE expr] ')'
//!
//! relationship_pattern ::=
//!     '-[' [variable] [':' type ('|' type)*] [quantifier] ']->'
//!   | '<-[' [variable] [':' type ('|' type)*] [quantifier] ']-'
//!   | '-[' [variable] [':' type ('|' type)*] [quantifier] ']-'
//!
//! quantifier ::=
//!     '*' [min] ['..' [max]]
//! ```

mod clauses;
mod error;
mod expression;
mod graph_table;
mod patterns;
mod primitives;

#[cfg(test)]
mod tests;

pub use error::PgqParseError;
use error::{describe_error_kind, make_error};
use graph_table::parse_graph_table_internal;

use super::pgq::GraphTableQuery;

/// Check if SQL contains GRAPH_TABLE
pub fn is_graph_table_expression(sql: &str) -> bool {
    let upper = sql.to_uppercase();
    upper.contains("GRAPH_TABLE")
}

/// Extract the GRAPH_TABLE content from a SQL string argument
///
/// When GRAPH_TABLE is used as a table-valued function like `GRAPH_TABLE('...')`,
/// this extracts the inner content for parsing.
pub fn extract_graph_table_arg(arg: &str) -> Option<String> {
    let trimmed = arg.trim();
    if (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('"') && trimmed.ends_with('"'))
    {
        Some(trimmed[1..trimmed.len() - 1].to_string())
    } else {
        Some(trimmed.to_string())
    }
}

/// Preprocess SQL to transform GRAPH_TABLE(PGQ) into GRAPH_TABLE('PGQ') for sqlparser compatibility
///
/// GRAPH_TABLE content isn't valid SQL, so we convert it to a string literal that sqlparser
/// can handle. At execution time, we parse the string content with our PGQ parser.
///
/// # Example
///
/// Input:  `SELECT * FROM GRAPH_TABLE(MATCH (a:User) COLUMNS (a.id))`
/// Output: `SELECT * FROM GRAPH_TABLE('GRAPH_TABLE(MATCH (a:User) COLUMNS (a.id))')`
///
/// The full GRAPH_TABLE(...) is wrapped as the string so the PGQ parser can parse it directly.
pub fn preprocess_graph_tables(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len() + 100);
    let mut last_end = 0;

    for (start, end, _) in find_graph_tables(sql) {
        result.push_str(&sql[last_end..start]);
        let graph_table_content = &sql[start..end];
        let escaped = graph_table_content.replace('\'', "''");
        result.push_str("GRAPH_TABLE('");
        result.push_str(&escaped);
        result.push_str("')");
        last_end = end;
    }

    result.push_str(&sql[last_end..]);
    result
}

/// Find and extract GRAPH_TABLE expressions from a SQL string
///
/// Returns a list of (start_offset, end_offset, parsed_query) for each GRAPH_TABLE found.
pub fn find_graph_tables(sql: &str) -> Vec<(usize, usize, Result<GraphTableQuery, PgqParseError>)> {
    let mut results = Vec::new();
    let upper = sql.to_uppercase();

    let pattern = "GRAPH_TABLE(";
    let mut search_start = 0;

    while let Some(pos) = upper[search_start..].find(pattern) {
        let start = search_start + pos;
        let content_start = start + pattern.len();

        if let Some(end) = find_matching_paren(sql, content_start - 1) {
            let graph_table_str = &sql[start..=end];
            let result = parse_graph_table(graph_table_str);
            results.push((start, end + 1, result));
            search_start = end + 1;
        } else {
            search_start = content_start;
        }
    }

    results
}

/// Find the matching closing parenthesis
fn find_matching_paren(sql: &str, start: usize) -> Option<usize> {
    let chars: Vec<char> = sql.chars().collect();
    if chars.get(start) != Some(&'(') {
        return None;
    }

    let mut depth = 1;
    let mut in_string = false;
    let mut string_char = '"';

    for i in (start + 1)..chars.len() {
        let c = chars[i];

        if in_string {
            if c == string_char && chars.get(i.wrapping_sub(1)) != Some(&'\\') {
                in_string = false;
            }
        } else {
            match c {
                '\'' | '"' => {
                    in_string = true;
                    string_char = c;
                }
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
    }

    None
}

/// Parse a GRAPH_TABLE expression from within a larger SQL statement
///
/// This finds and parses the GRAPH_TABLE(...) portion.
pub fn parse_graph_table(sql: &str) -> Result<GraphTableQuery, PgqParseError> {
    let trimmed = sql.trim();

    match parse_graph_table_internal(trimmed) {
        Ok((remaining, query)) => {
            if remaining.trim().is_empty() {
                Ok(query)
            } else {
                Err(make_error(
                    remaining,
                    trimmed,
                    format!("Unexpected trailing content: '{}'", remaining.trim()),
                ))
            }
        }
        Err(nom::Err::Error(e) | nom::Err::Failure(e)) => Err(make_error(
            e.input,
            trimmed,
            format!("Expected {}", describe_error_kind(&e.code)),
        )),
        Err(nom::Err::Incomplete(_)) => Err(PgqParseError {
            message: "Incomplete input".into(),
            line: 1,
            column: 1,
            offset: 0,
            context: None,
        }),
    }
}
