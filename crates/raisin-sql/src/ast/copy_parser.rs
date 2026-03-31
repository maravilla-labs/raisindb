//! COPY statement parser using nom combinators
//!
//! Parses COPY statements for duplicating nodes:
//! - COPY Page SET path='/content/pagea' TO path='/new/parent'
//! - COPY Page SET id='abc123' TO path='/target/parent' AS 'new-name'
//! - COPY TREE Article SET path='/source' TO id='target-parent-id'

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_until},
    character::complete::{char, multispace0, multispace1},
    combinator::{map, opt},
    sequence::{preceded, tuple},
    IResult, Parser,
};

use super::copy_stmt::CopyStatement;
use super::order::NodeReference;

/// Error type for COPY statement parsing
#[derive(Debug, Clone, PartialEq)]
pub struct CopyParseError {
    pub message: String,
    pub position: Option<usize>,
}

impl std::fmt::Display for CopyParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(pos) = self.position {
            write!(f, "COPY parse error at position {}: {}", pos, self.message)
        } else {
            write!(f, "COPY parse error: {}", self.message)
        }
    }
}

impl std::error::Error for CopyParseError {}

/// Check if a SQL statement is a COPY statement
///
/// COPY statement must start with "COPY" (optionally "COPY TREE") followed by a table name
/// (optionally IN BRANCH) and "SET"
pub fn is_copy_statement(sql: &str) -> bool {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();

    // Must start with COPY
    if !upper.starts_with("COPY") {
        return false;
    }

    // Get what comes after "COPY"
    let after_copy = upper.get(4..).unwrap_or("").trim_start();

    // Check for COPY TREE syntax first
    let (after_tree, _is_tree) = if after_copy.starts_with("TREE") {
        (after_copy.get(4..).unwrap_or("").trim_start(), true)
    } else {
        (after_copy, false)
    };

    // For COPY [TREE] Table [IN BRANCH 'x'] SET ... syntax, look for SET keyword
    // Table name is an identifier (alphanumeric + underscore)
    let words: Vec<&str> = after_tree.split_whitespace().collect();
    if words.len() >= 2 {
        // Second word should be SET or IN (for IN BRANCH)
        if words[1] == "SET" {
            return true;
        }
        // Check for IN BRANCH syntax: COPY [TREE] Table IN BRANCH 'x' SET ...
        if words.len() >= 5 && words[1] == "IN" && words[2] == "BRANCH" {
            // words[3] is the quoted branch name, words[4] should be SET
            return words.get(4).map(|w| *w == "SET").unwrap_or(false);
        }
    }
    false
}

/// Parse a COPY statement from SQL string
///
/// Returns `Some(CopyStatement)` if the input is a valid COPY statement,
/// `None` if it's not a COPY statement (should be handled by other parsers).
pub fn parse_copy(sql: &str) -> Result<Option<CopyStatement>, CopyParseError> {
    let trimmed = sql.trim();

    // Strip leading SQL comments
    let statement_start = super::ddl_parser::strip_leading_comments(trimmed);

    // Check if this is a COPY statement
    if !is_copy_statement(statement_start) {
        return Ok(None);
    }

    // Calculate offset for error position mapping
    let offset_to_statement_start = statement_start.as_ptr() as usize - sql.as_ptr() as usize;

    match copy_statement(statement_start) {
        Ok((remaining, stmt)) => {
            // Verify we consumed all input (except whitespace and semicolon)
            let remaining_trimmed = remaining.trim().trim_end_matches(';').trim();
            if remaining_trimmed.is_empty() {
                Ok(Some(stmt))
            } else {
                let position_in_statement = statement_start.len() - remaining.len();
                let position = offset_to_statement_start + position_in_statement;
                Err(CopyParseError {
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
                nom::Err::Incomplete(_) => (None, "Incomplete COPY statement".to_string()),
            };
            Err(CopyParseError { message, position })
        }
    }
}

/// Parse the full COPY statement: COPY [TREE] Table [IN BRANCH 'x'] SET source TO target [AS 'name']
fn copy_statement(input: &str) -> IResult<&str, CopyStatement> {
    let (input, _) = tag_no_case("COPY").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse optional TREE keyword
    let (input, is_tree) = opt(tuple((tag_no_case("TREE"), multispace1))).parse(input)?;
    let recursive = is_tree.is_some();

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

    // Parse TO keyword
    let (input, _) = tag_no_case("TO").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse target parent reference (path='...' or id='...')
    let (input, target_parent) = node_reference(input)?;

    // Parse optional AS 'new-name' clause
    let (input, new_name) = opt(preceded(
        tuple((multispace1, tag_no_case("AS"), multispace1)),
        quoted_string,
    ))
    .parse(input)?;

    Ok((
        input,
        CopyStatement::with_options(
            table,
            branch.map(|s| s.to_string()),
            source,
            target_parent,
            new_name.map(|s| s.to_string()),
            recursive,
        ),
    ))
}

/// Parse an identifier (table name): alphanumeric + underscore
fn identifier(input: &str) -> IResult<&str, &str> {
    nom::bytes::complete::take_while1(|c: char| c.is_alphanumeric() || c == '_').parse(input)
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
    fn test_is_copy_statement() {
        assert!(is_copy_statement("COPY Page SET path='/a' TO path='/b'"));
        assert!(is_copy_statement(
            "COPY BlogPost SET PATH='/a' TO PATH='/b'"
        ));
        assert!(is_copy_statement("COPY Article SET id='x' TO id='y'"));
        assert!(is_copy_statement("COPY MyTable SET ID='x' TO ID='y'"));
        assert!(is_copy_statement(
            "  COPY Page SET path='/a' TO path='/b'  "
        ));
    }

    #[test]
    fn test_is_copy_tree_statement() {
        assert!(is_copy_statement(
            "COPY TREE Page SET path='/a' TO path='/b'"
        ));
        assert!(is_copy_statement(
            "COPY TREE BlogPost SET PATH='/a' TO PATH='/b'"
        ));
        assert!(is_copy_statement(
            "  COPY TREE Page SET path='/a' TO path='/b'  "
        ));
    }

    #[test]
    fn test_is_not_copy_statement() {
        assert!(!is_copy_statement("SELECT * FROM nodes"));
        assert!(!is_copy_statement("UPDATE nodes SET name = 'test'"));
        assert!(!is_copy_statement("MOVE Page SET path='/a' TO path='/b'"));
        assert!(!is_copy_statement(
            "ORDER Page SET path='/a' ABOVE path='/b'"
        ));
        // Old syntax without SET should not match
        assert!(!is_copy_statement("COPY path='/a' TO path='/b'"));
    }

    #[test]
    fn test_parse_copy_path_to_path() {
        let sql = "COPY Page SET path='/content/pagea' TO path='/content/newparent'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.source, NodeReference::Path("/content/pagea".into()));
        assert_eq!(
            result.target_parent,
            NodeReference::Path("/content/newparent".into())
        );
        assert!(!result.recursive);
        assert!(result.new_name.is_none());
    }

    #[test]
    fn test_parse_copy_tree() {
        let sql = "COPY TREE Page SET path='/content/pagea' TO path='/content/newparent'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.source, NodeReference::Path("/content/pagea".into()));
        assert_eq!(
            result.target_parent,
            NodeReference::Path("/content/newparent".into())
        );
        assert!(result.recursive);
    }

    #[test]
    fn test_parse_copy_with_new_name() {
        let sql =
            "COPY Page SET path='/content/pagea' TO path='/content/newparent' AS 'copied-page'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.new_name, Some("copied-page".to_string()));
        assert!(!result.recursive);
    }

    #[test]
    fn test_parse_copy_tree_with_new_name() {
        let sql =
            "COPY TREE Page SET path='/content/pagea' TO path='/content/newparent' AS 'copied-tree'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.new_name, Some("copied-tree".to_string()));
        assert!(result.recursive);
    }

    #[test]
    fn test_parse_copy_id_to_path() {
        let sql = "COPY BlogPost SET id='abc123' TO path='/content/target'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "BlogPost");
        assert_eq!(result.source, NodeReference::Id("abc123".into()));
        assert_eq!(
            result.target_parent,
            NodeReference::Path("/content/target".into())
        );
    }

    #[test]
    fn test_parse_copy_path_to_id() {
        let sql = "COPY Article SET path='/source' TO id='xyz789'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Article");
        assert_eq!(result.source, NodeReference::Path("/source".into()));
        assert_eq!(result.target_parent, NodeReference::Id("xyz789".into()));
    }

    #[test]
    fn test_parse_copy_id_to_id() {
        let sql = "COPY Page SET id='node1' TO id='parent1'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.source, NodeReference::Id("node1".into()));
        assert_eq!(result.target_parent, NodeReference::Id("parent1".into()));
    }

    #[test]
    fn test_parse_copy_case_insensitive() {
        let sql = "copy Page set PATH='/a' to PATH='/b'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.source, NodeReference::Path("/a".into()));
        assert_eq!(result.target_parent, NodeReference::Path("/b".into()));

        let sql2 = "CoPy TrEe MyTable SeT pAtH='/x' To Id='y'";
        let result2 = parse_copy(sql2).unwrap().unwrap();
        assert_eq!(result2.source, NodeReference::Path("/x".into()));
        assert_eq!(result2.target_parent, NodeReference::Id("y".into()));
        assert!(result2.recursive);
    }

    #[test]
    fn test_parse_copy_with_semicolon() {
        let sql = "COPY Page SET path='/a' TO path='/b';";
        let result = parse_copy(sql);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_parse_copy_with_double_quotes() {
        let sql = r#"COPY Page SET path="/content/page1" TO path="/content/newparent""#;
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.source, NodeReference::Path("/content/page1".into()));
        assert_eq!(
            result.target_parent,
            NodeReference::Path("/content/newparent".into())
        );
    }

    #[test]
    fn test_parse_copy_with_whitespace() {
        let sql = "  COPY   Page   SET   path = '/a'   TO   path = '/b'  ";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.source, NodeReference::Path("/a".into()));
        assert_eq!(result.target_parent, NodeReference::Path("/b".into()));
    }

    #[test]
    fn test_parse_non_copy_statement() {
        let sql = "SELECT * FROM nodes";
        let result = parse_copy(sql).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_copy_statement_display() {
        let stmt = CopyStatement::new(
            "Page",
            NodeReference::path("/content/page1"),
            NodeReference::id("target-id"),
        );
        assert_eq!(
            stmt.to_string(),
            "COPY Page SET path='/content/page1' TO id='target-id'"
        );
    }

    #[test]
    fn test_parse_copy_underscore_table_name() {
        let sql = "COPY blog_post SET path='/a' TO path='/b'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "blog_post");
    }

    #[test]
    fn test_is_copy_statement_with_branch() {
        assert!(is_copy_statement(
            "COPY Page IN BRANCH 'feature-x' SET path='/a' TO path='/b'"
        ));
        assert!(is_copy_statement(
            "COPY TREE BlogPost IN BRANCH 'test' SET id='x' TO id='y'"
        ));
    }

    #[test]
    fn test_parse_copy_with_branch() {
        let sql = "COPY Page IN BRANCH 'feature-x' SET path='/a' TO path='/b'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.branch, Some("feature-x".to_string()));
        assert_eq!(result.source, NodeReference::Path("/a".into()));
        assert_eq!(result.target_parent, NodeReference::Path("/b".into()));
    }

    #[test]
    fn test_parse_copy_tree_with_branch() {
        let sql = "COPY TREE Page IN BRANCH 'feature-x' SET path='/a' TO path='/b'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.branch, Some("feature-x".to_string()));
        assert!(result.recursive);
    }

    #[test]
    fn test_parse_copy_without_branch() {
        let sql = "COPY Page SET path='/a' TO path='/b'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.branch, None);
    }

    #[test]
    fn test_parse_copy_with_branch_double_quotes() {
        let sql = r#"COPY Page IN BRANCH "my-branch" SET path='/a' TO path='/b'"#;
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.branch, Some("my-branch".to_string()));
    }

    #[test]
    fn test_copy_statement_display_with_branch() {
        let stmt = CopyStatement::with_options(
            "Page",
            Some("feature-x".to_string()),
            NodeReference::path("/content/page1"),
            NodeReference::id("target-id"),
            None,
            false,
        );
        assert_eq!(
            stmt.to_string(),
            "COPY Page IN BRANCH 'feature-x' SET path='/content/page1' TO id='target-id'"
        );
    }

    #[test]
    fn test_parse_copy_full_options() {
        let sql = "COPY TREE Page IN BRANCH 'feature-x' SET path='/a' TO path='/b' AS 'new-page'";
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.table, "Page");
        assert_eq!(result.branch, Some("feature-x".to_string()));
        assert_eq!(result.source, NodeReference::Path("/a".into()));
        assert_eq!(result.target_parent, NodeReference::Path("/b".into()));
        assert_eq!(result.new_name, Some("new-page".to_string()));
        assert!(result.recursive);
    }

    #[test]
    fn test_parse_copy_as_with_double_quotes() {
        let sql = r#"COPY Page SET path='/a' TO path='/b' AS "my-copy""#;
        let result = parse_copy(sql).unwrap().unwrap();
        assert_eq!(result.new_name, Some("my-copy".to_string()));
    }
}
