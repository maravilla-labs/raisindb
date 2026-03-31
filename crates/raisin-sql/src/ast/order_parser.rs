//! ORDER statement parser using nom combinators
//!
//! Parses ORDER statements for node sibling positioning:
//! - ORDER Page SET path='/content/pagea' ABOVE path='/content/pageb'
//! - ORDER BlogPost SET id='abc123' BELOW path='/content/target'
//! - ORDER Article SET path='/source' BELOW id='xyz789'

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_until, take_while1},
    character::complete::{char, multispace0, multispace1},
    combinator::{map, opt},
    sequence::{preceded, tuple},
    IResult, Parser,
};

use super::order::{NodeReference, OrderPosition, OrderStatement};

/// Error type for ORDER statement parsing
#[derive(Debug, Clone, PartialEq)]
pub struct OrderParseError {
    pub message: String,
    pub position: Option<usize>,
}

impl std::fmt::Display for OrderParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(pos) = self.position {
            write!(f, "ORDER parse error at position {}: {}", pos, self.message)
        } else {
            write!(f, "ORDER parse error: {}", self.message)
        }
    }
}

impl std::error::Error for OrderParseError {}

/// Check if a SQL statement is an ORDER statement (not ORDER BY)
///
/// ORDER statement must start with "ORDER" followed by a table name (optionally IN BRANCH) and "SET"
/// ORDER BY is a different construct used in SELECT statements
pub fn is_order_statement(sql: &str) -> bool {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();

    // Must start with ORDER
    if !upper.starts_with("ORDER") {
        return false;
    }

    // Get what comes after "ORDER"
    let after_order = upper.get(5..).unwrap_or("").trim_start();

    // Must NOT be ORDER BY (which is a clause in SELECT)
    if after_order.starts_with("BY") {
        return false;
    }

    // For ORDER Table [IN BRANCH 'x'] SET ... syntax, look for SET keyword
    // Table name is an identifier (alphanumeric + underscore)
    let words: Vec<&str> = after_order.split_whitespace().collect();
    if words.len() >= 2 {
        // Second word should be SET or IN (for IN BRANCH)
        if words[1] == "SET" {
            return true;
        }
        // Check for IN BRANCH syntax: ORDER Table IN BRANCH 'x' SET ...
        if words.len() >= 5 && words[1] == "IN" && words[2] == "BRANCH" {
            // words[3] is the quoted branch name, words[4] should be SET
            return words.get(4).map(|w| *w == "SET").unwrap_or(false);
        }
    }
    false
}

/// Parse an ORDER statement from SQL string
///
/// Returns `Some(OrderStatement)` if the input is a valid ORDER statement,
/// `None` if it's not an ORDER statement (should be handled by other parsers).
pub fn parse_order(sql: &str) -> Result<Option<OrderStatement>, OrderParseError> {
    let trimmed = sql.trim();

    // Strip leading SQL comments
    let statement_start = super::ddl_parser::strip_leading_comments(trimmed);

    // Check if this is an ORDER statement (not ORDER BY)
    if !is_order_statement(statement_start) {
        return Ok(None);
    }

    // Calculate offset for error position mapping
    let offset_to_statement_start = statement_start.as_ptr() as usize - sql.as_ptr() as usize;

    match order_statement(statement_start) {
        Ok((remaining, stmt)) => {
            // Verify we consumed all input (except whitespace and semicolon)
            let remaining_trimmed = remaining.trim().trim_end_matches(';').trim();
            if remaining_trimmed.is_empty() {
                Ok(Some(stmt))
            } else {
                let position_in_statement = statement_start.len() - remaining.len();
                let position = offset_to_statement_start + position_in_statement;
                Err(OrderParseError {
                    message: format!("Unexpected trailing content: '{}'", remaining_trimmed),
                    position: Some(position),
                })
            }
        }
        Err(e) => {
            let (position, message) = match &e {
                nom::Err::Failure(err) | nom::Err::Error(err) => {
                    let pos_in_statement = statement_start.len() - err.input.len();
                    let remaining = err.input.trim();
                    let problematic: String = remaining
                        .chars()
                        .take(30)
                        .take_while(|c| *c != '\n')
                        .collect();
                    (
                        Some(offset_to_statement_start + pos_in_statement),
                        format!("Parse error near: '{}'", problematic.trim()),
                    )
                }
                nom::Err::Incomplete(_) => (None, "Incomplete ORDER statement".to_string()),
            };
            Err(OrderParseError { message, position })
        }
    }
}

/// Parse the full ORDER statement: ORDER Table [IN BRANCH 'x'] SET source ABOVE|BELOW target
fn order_statement(input: &str) -> IResult<&str, OrderStatement> {
    let (input, _) = tag_no_case("ORDER").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse table name (identifier)
    let (input, table) = identifier(input)?;

    // Parse optional IN BRANCH 'xx' clause
    let (input, branch) = opt(preceded(
        tuple((
            multispace1,
            tag_no_case("IN"),
            multispace1,
            tag_no_case("BRANCH"),
            multispace1,
        )),
        quoted_string,
    ))
    .parse(input)?;

    let (input, _) = multispace1.parse(input)?;

    // Parse SET keyword
    let (input, _) = tag_no_case("SET").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse source reference (path='...' or id='...')
    let (input, source) = node_reference(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse position (ABOVE or BELOW)
    let (input, position) = order_position(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse target reference (path='...' or id='...')
    let (input, target) = node_reference(input)?;

    Ok((
        input,
        OrderStatement::with_branch(
            table,
            branch.map(|s| s.to_string()),
            source,
            position,
            target,
        ),
    ))
}

/// Parse an identifier (table name): alphanumeric + underscore, must start with letter
fn identifier(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphanumeric() || c == '_').parse(input)
}

/// Parse a node reference: path='...' or id='...'
fn node_reference(input: &str) -> IResult<&str, NodeReference> {
    alt((
        map(
            preceded(
                (tag_no_case("path"), multispace0, char('='), multispace0),
                quoted_string,
            ),
            |s: &str| NodeReference::Path(s.to_string()),
        ),
        map(
            preceded(
                (tag_no_case("id"), multispace0, char('='), multispace0),
                quoted_string,
            ),
            |s: &str| NodeReference::Id(s.to_string()),
        ),
    ))
    .parse(input)
}

/// Parse position keyword: ABOVE or BELOW
fn order_position(input: &str) -> IResult<&str, OrderPosition> {
    alt((
        map(tag_no_case("ABOVE"), |_| OrderPosition::Above),
        map(tag_no_case("BELOW"), |_| OrderPosition::Below),
    ))
    .parse(input)
}

/// Parse a quoted string: 'content' or "content"
fn quoted_string(input: &str) -> IResult<&str, &str> {
    alt((
        nom::sequence::delimited(char('\''), take_until("'"), char('\'')),
        nom::sequence::delimited(char('"'), take_until("\""), char('"')),
    ))
    .parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_order_statement() {
        assert!(is_order_statement(
            "ORDER Page SET path='/a' ABOVE path='/b'"
        ));
        assert!(is_order_statement(
            "ORDER BlogPost SET PATH='/a' ABOVE PATH='/b'"
        ));
        assert!(is_order_statement("ORDER Article SET id='x' BELOW id='y'"));
        assert!(is_order_statement("ORDER MyTable SET ID='x' BELOW ID='y'"));
        assert!(is_order_statement(
            "  ORDER Page SET path='/a' ABOVE path='/b'  "
        ));
    }

    #[test]
    fn test_is_not_order_statement() {
        assert!(!is_order_statement("ORDER BY name"));
        assert!(!is_order_statement("SELECT * FROM nodes ORDER BY name"));
        assert!(!is_order_statement("SELECT * FROM nodes"));
        assert!(!is_order_statement("UPDATE nodes SET name = 'test'"));
        // Old syntax without SET should not match
        assert!(!is_order_statement("ORDER path='/a' ABOVE path='/b'"));
    }

    #[test]
    fn test_parse_order_path_above_path() {
        let sql = "ORDER Page SET path='/content/pagea' ABOVE path='/content/pageb'";
        let result = parse_order(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.source, NodeReference::Path("/content/pagea".into()));
        assert_eq!(result.position, OrderPosition::Above);
        assert_eq!(result.target, NodeReference::Path("/content/pageb".into()));
    }

    #[test]
    fn test_parse_order_path_below_path() {
        let sql = "ORDER BlogPost SET path='/content/page1' BELOW path='/content/page2'";
        let result = parse_order(sql).unwrap().unwrap();
        assert_eq!(result.table, "BlogPost");
        assert_eq!(result.source, NodeReference::Path("/content/page1".into()));
        assert_eq!(result.position, OrderPosition::Below);
        assert_eq!(result.target, NodeReference::Path("/content/page2".into()));
    }

    #[test]
    fn test_parse_order_id_above_path() {
        let sql = "ORDER Article SET id='abc123' ABOVE path='/content/target'";
        let result = parse_order(sql).unwrap().unwrap();
        assert_eq!(result.table, "Article");
        assert_eq!(result.source, NodeReference::Id("abc123".into()));
        assert_eq!(result.position, OrderPosition::Above);
        assert_eq!(result.target, NodeReference::Path("/content/target".into()));
    }

    #[test]
    fn test_parse_order_path_below_id() {
        let sql = "ORDER Page SET path='/source' BELOW id='xyz789'";
        let result = parse_order(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.source, NodeReference::Path("/source".into()));
        assert_eq!(result.position, OrderPosition::Below);
        assert_eq!(result.target, NodeReference::Id("xyz789".into()));
    }

    #[test]
    fn test_parse_order_id_above_id() {
        let sql = "ORDER Page SET id='node1' ABOVE id='node2'";
        let result = parse_order(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.source, NodeReference::Id("node1".into()));
        assert_eq!(result.position, OrderPosition::Above);
        assert_eq!(result.target, NodeReference::Id("node2".into()));
    }

    #[test]
    fn test_parse_order_case_insensitive() {
        let sql = "order Page set PATH='/a' above PATH='/b'";
        let result = parse_order(sql).unwrap().unwrap();
        assert_eq!(result.position, OrderPosition::Above);

        let sql2 = "OrDeR MyTable SeT pAtH='/x' BeLow Id='y'";
        let result2 = parse_order(sql2).unwrap().unwrap();
        assert_eq!(result2.position, OrderPosition::Below);
    }

    #[test]
    fn test_parse_order_with_semicolon() {
        let sql = "ORDER Page SET path='/a' ABOVE path='/b';";
        let result = parse_order(sql);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_parse_order_with_double_quotes() {
        let sql = r#"ORDER Page SET path="/content/page1" ABOVE path="/content/page2""#;
        let result = parse_order(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.source, NodeReference::Path("/content/page1".into()));
        assert_eq!(result.target, NodeReference::Path("/content/page2".into()));
    }

    #[test]
    fn test_parse_order_with_whitespace() {
        let sql = "  ORDER   Page   SET   path = '/a'   ABOVE   path = '/b'  ";
        let result = parse_order(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.source, NodeReference::Path("/a".into()));
        assert_eq!(result.target, NodeReference::Path("/b".into()));
    }

    #[test]
    fn test_parse_non_order_statement() {
        let sql = "SELECT * FROM nodes";
        let result = parse_order(sql).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_order_by_not_matched() {
        let sql = "ORDER BY name ASC";
        let result = parse_order(sql).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_order_statement_display() {
        let stmt = OrderStatement::new(
            "Page",
            NodeReference::path("/content/page1"),
            OrderPosition::Above,
            NodeReference::id("target-id"),
        );
        assert_eq!(
            stmt.to_string(),
            "ORDER Page SET path='/content/page1' ABOVE id='target-id'"
        );
    }

    #[test]
    fn test_parse_order_underscore_table_name() {
        let sql = "ORDER blog_post SET path='/a' ABOVE path='/b'";
        let result = parse_order(sql).unwrap().unwrap();
        assert_eq!(result.table, "blog_post");
    }
}
