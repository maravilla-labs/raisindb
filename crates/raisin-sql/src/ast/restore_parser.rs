//! RESTORE statement parser using nom combinators
//!
//! Parses RESTORE statements for restoring nodes to previous revision states:
//! - RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2
//! - RESTORE TREE NODE path='/products/category' TO REVISION HEAD~5
//! - RESTORE NODE id='uuid' TO REVISION HEAD~2 TRANSLATIONS ('en', 'de')

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_until, take_while1},
    character::complete::{char, digit1, multispace0, multispace1},
    combinator::{map, opt},
    multi::separated_list0,
    sequence::preceded,
    IResult, Parser,
};

use super::branch::RevisionRef;
use super::order::NodeReference;
use super::restore::RestoreStatement;

/// Error type for RESTORE statement parsing
#[derive(Debug, Clone, PartialEq)]
pub struct RestoreParseError {
    pub message: String,
    pub position: Option<usize>,
}

impl std::fmt::Display for RestoreParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(pos) = self.position {
            write!(
                f,
                "RESTORE parse error at position {}: {}",
                pos, self.message
            )
        } else {
            write!(f, "RESTORE parse error: {}", self.message)
        }
    }
}

impl std::error::Error for RestoreParseError {}

/// Check if a SQL statement is a RESTORE statement
///
/// RESTORE statement must start with "RESTORE" followed by optional "TREE" and "NODE"
pub fn is_restore_statement(sql: &str) -> bool {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();

    // Must start with RESTORE
    if !upper.starts_with("RESTORE") {
        return false;
    }

    // Get what comes after "RESTORE"
    let after_restore = upper.get(7..).unwrap_or("").trim_start();

    // Check for RESTORE TREE NODE or RESTORE NODE
    if after_restore.starts_with("TREE") {
        let after_tree = after_restore.get(4..).unwrap_or("").trim_start();
        after_tree.starts_with("NODE")
    } else {
        after_restore.starts_with("NODE")
    }
}

/// Parse a RESTORE statement from SQL string
///
/// Returns `Some(RestoreStatement)` if the input is a valid RESTORE statement,
/// `None` if it's not a RESTORE statement (should be handled by other parsers).
pub fn parse_restore(sql: &str) -> Result<Option<RestoreStatement>, RestoreParseError> {
    let trimmed = sql.trim();

    // Strip leading SQL comments
    let statement_start = super::ddl_parser::strip_leading_comments(trimmed);

    // Check if this is a RESTORE statement
    if !is_restore_statement(statement_start) {
        return Ok(None);
    }

    // Calculate offset for error position mapping
    let offset_to_statement_start = statement_start.as_ptr() as usize - sql.as_ptr() as usize;

    match restore_statement(statement_start) {
        Ok((remaining, stmt)) => {
            // Verify we consumed all input (except whitespace and semicolon)
            let remaining_trimmed = remaining.trim().trim_end_matches(';').trim();
            if remaining_trimmed.is_empty() {
                Ok(Some(stmt))
            } else {
                let position_in_statement = statement_start.len() - remaining.len();
                let position = offset_to_statement_start + position_in_statement;
                Err(RestoreParseError {
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
                nom::Err::Incomplete(_) => (None, "Incomplete RESTORE statement".to_string()),
            };
            Err(RestoreParseError { message, position })
        }
    }
}

/// Parse the full RESTORE statement: RESTORE [TREE] NODE <ref> TO REVISION <rev> [TRANSLATIONS (...)]
fn restore_statement(input: &str) -> IResult<&str, RestoreStatement> {
    let (input, _) = tag_no_case("RESTORE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse optional TREE keyword
    let (input, is_tree) = opt(preceded(tag_no_case("TREE"), multispace1)).parse(input)?;
    let recursive = is_tree.is_some();

    // Parse NODE keyword
    let (input, _) = tag_no_case("NODE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse node reference (path='...' or id='...')
    let (input, node) = node_reference(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse TO REVISION
    let (input, _) = tag_no_case("TO").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("REVISION").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, revision) = revision_ref(input)?;

    // Parse optional TRANSLATIONS clause
    let (input, translations) = opt(parse_translations_clause).parse(input)?;

    Ok((
        input,
        RestoreStatement::with_options(node, revision, recursive, translations),
    ))
}

/// Parse TRANSLATIONS ('en', 'de') clause
fn parse_translations_clause(input: &str) -> IResult<&str, Vec<String>> {
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("TRANSLATIONS").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char('(').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Parse comma-separated quoted strings
    let (input, translations) = separated_list0(
        (multispace0, char(','), multispace0),
        map(quoted_string, |s: &str| s.to_string()),
    )
    .parse(input)?;

    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char(')').parse(input)?;

    Ok((input, translations))
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

/// Parse a revision reference: HEAD~N, branch~N, or HLC timestamp
fn revision_ref(input: &str) -> IResult<&str, RevisionRef> {
    alt((
        // HEAD~N
        map(
            preceded(
                (tag_no_case("HEAD"), char('~')),
                map(digit1, |s: &str| s.parse::<u32>().unwrap_or(0)),
            ),
            RevisionRef::HeadRelative,
        ),
        // branch~N (unquoted identifier followed by ~N)
        map(
            (
                identifier,
                char('~'),
                map(digit1, |s: &str| s.parse::<u32>().unwrap_or(0)),
            ),
            |(branch, _, offset)| RevisionRef::BranchRelative {
                branch: branch.to_string(),
                offset,
            },
        ),
        // HLC: timestamp_counter format or just a numeric string
        map(hlc_string, |s| RevisionRef::Hlc(s.to_string())),
    ))
    .parse(input)
}

/// Parse an HLC string: numeric with optional underscore (e.g., 1734567890123_42 or 1734567890123)
fn hlc_string(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_ascii_digit() || c == '_').parse(input)
}

/// Parse an identifier: alphanumeric + underscore
fn identifier(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphanumeric() || c == '_').parse(input)
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
    fn test_is_restore_statement() {
        assert!(is_restore_statement(
            "RESTORE NODE path='/a' TO REVISION HEAD~2"
        ));
        assert!(is_restore_statement(
            "RESTORE TREE NODE path='/a' TO REVISION HEAD~2"
        ));
        assert!(is_restore_statement(
            "  RESTORE NODE id='x' TO REVISION HEAD~5  "
        ));
        assert!(is_restore_statement(
            "restore node PATH='/a' to revision head~2"
        ));
    }

    #[test]
    fn test_is_not_restore_statement() {
        assert!(!is_restore_statement("SELECT * FROM nodes"));
        assert!(!is_restore_statement(
            "COPY Page SET path='/a' TO path='/b'"
        ));
        assert!(!is_restore_statement("RESTORE something else"));
        assert!(!is_restore_statement("RESTORE without NODE keyword"));
    }

    #[test]
    fn test_parse_restore_by_path() {
        let sql = "RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2";
        let result = parse_restore(sql).unwrap().unwrap();
        assert_eq!(
            result.node,
            NodeReference::Path("/articles/my-article".into())
        );
        assert_eq!(result.revision, RevisionRef::HeadRelative(2));
        assert!(!result.recursive);
        assert!(result.translations.is_none());
    }

    #[test]
    fn test_parse_restore_by_id() {
        let sql = "RESTORE NODE id='uuid-123' TO REVISION HEAD~3";
        let result = parse_restore(sql).unwrap().unwrap();
        assert_eq!(result.node, NodeReference::Id("uuid-123".into()));
        assert_eq!(result.revision, RevisionRef::HeadRelative(3));
        assert!(!result.recursive);
    }

    #[test]
    fn test_parse_restore_tree() {
        let sql = "RESTORE TREE NODE path='/products/category' TO REVISION HEAD~5";
        let result = parse_restore(sql).unwrap().unwrap();
        assert_eq!(
            result.node,
            NodeReference::Path("/products/category".into())
        );
        assert_eq!(result.revision, RevisionRef::HeadRelative(5));
        assert!(result.recursive);
    }

    #[test]
    fn test_parse_restore_with_translations() {
        let sql =
            "RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2 TRANSLATIONS ('en', 'de')";
        let result = parse_restore(sql).unwrap().unwrap();
        assert_eq!(
            result.node,
            NodeReference::Path("/articles/my-article".into())
        );
        assert_eq!(
            result.translations,
            Some(vec!["en".to_string(), "de".to_string()])
        );
    }

    #[test]
    fn test_parse_restore_with_single_translation() {
        let sql = "RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2 TRANSLATIONS ('fr')";
        let result = parse_restore(sql).unwrap().unwrap();
        assert_eq!(result.translations, Some(vec!["fr".to_string()]));
    }

    #[test]
    fn test_parse_restore_tree_with_translations() {
        let sql = "RESTORE TREE NODE path='/products' TO REVISION HEAD~10 TRANSLATIONS ('en')";
        let result = parse_restore(sql).unwrap().unwrap();
        assert!(result.recursive);
        assert_eq!(result.translations, Some(vec!["en".to_string()]));
    }

    #[test]
    fn test_parse_restore_with_hlc() {
        let sql = "RESTORE NODE path='/articles/my-article' TO REVISION 1734567890123_42";
        let result = parse_restore(sql).unwrap().unwrap();
        assert_eq!(
            result.revision,
            RevisionRef::Hlc("1734567890123_42".to_string())
        );
    }

    #[test]
    fn test_parse_restore_with_branch_relative() {
        let sql = "RESTORE NODE path='/articles/my-article' TO REVISION main~3";
        let result = parse_restore(sql).unwrap().unwrap();
        assert_eq!(
            result.revision,
            RevisionRef::BranchRelative {
                branch: "main".to_string(),
                offset: 3
            }
        );
    }

    #[test]
    fn test_parse_restore_case_insensitive() {
        let sql = "restore tree node PATH='/a' to revision HEAD~2";
        let result = parse_restore(sql).unwrap().unwrap();
        assert_eq!(result.node, NodeReference::Path("/a".into()));
        assert!(result.recursive);
    }

    #[test]
    fn test_parse_restore_with_semicolon() {
        let sql = "RESTORE NODE path='/a' TO REVISION HEAD~1;";
        let result = parse_restore(sql);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_parse_restore_with_double_quotes() {
        let sql = r#"RESTORE NODE path="/articles/my-article" TO REVISION HEAD~2"#;
        let result = parse_restore(sql).unwrap().unwrap();
        assert_eq!(
            result.node,
            NodeReference::Path("/articles/my-article".into())
        );
    }

    #[test]
    fn test_parse_restore_with_whitespace() {
        let sql = "  RESTORE   NODE   path = '/a'   TO   REVISION   HEAD~2  ";
        let result = parse_restore(sql).unwrap().unwrap();
        assert_eq!(result.node, NodeReference::Path("/a".into()));
        assert_eq!(result.revision, RevisionRef::HeadRelative(2));
    }

    #[test]
    fn test_parse_non_restore_statement() {
        let sql = "SELECT * FROM nodes";
        let result = parse_restore(sql).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_restore_translations_double_quotes() {
        let sql = r#"RESTORE NODE path='/a' TO REVISION HEAD~2 TRANSLATIONS ("en", "de")"#;
        let result = parse_restore(sql).unwrap().unwrap();
        assert_eq!(
            result.translations,
            Some(vec!["en".to_string(), "de".to_string()])
        );
    }

    #[test]
    fn test_restore_statement_display() {
        let stmt = RestoreStatement::new(
            NodeReference::path("/articles/my-article"),
            RevisionRef::HeadRelative(2),
        );
        assert_eq!(
            stmt.to_string(),
            "RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2"
        );
    }
}
