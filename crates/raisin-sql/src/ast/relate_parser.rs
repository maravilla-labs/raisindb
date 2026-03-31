//! RELATE/UNRELATE statement parser
//!
//! Parses RELATE and UNRELATE statements for managing node relationships.
//!
//! # Grammar
//!
//! ```text
//! RELATE [IN BRANCH 'branch']
//!   FROM path|id='value' [IN WORKSPACE 'ws']
//!   TO path|id='value' [IN WORKSPACE 'ws']
//!   [TYPE 'relation_type']
//!   [WEIGHT number]
//! ;
//!
//! UNRELATE [IN BRANCH 'branch']
//!   FROM path|id='value' [IN WORKSPACE 'ws']
//!   TO path|id='value' [IN WORKSPACE 'ws']
//!   [TYPE 'relation_type']
//! ;
//! ```

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while1},
    character::complete::{char, multispace0, multispace1},
    combinator::opt,
    number::complete::double,
    IResult, Parser,
};

use super::relate::{RelateEndpoint, RelateNodeReference, RelateStatement, UnrelateStatement};

/// Check if a SQL string is a RELATE statement
pub fn is_relate_statement(sql: &str) -> bool {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();
    upper.starts_with("RELATE ")
        || upper.starts_with("RELATE\n")
        || upper.starts_with("RELATE\t")
        || upper == "RELATE"
}

/// Check if a SQL string is an UNRELATE statement
pub fn is_unrelate_statement(sql: &str) -> bool {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();
    upper.starts_with("UNRELATE ")
        || upper.starts_with("UNRELATE\n")
        || upper.starts_with("UNRELATE\t")
        || upper == "UNRELATE"
}

/// Parse a RELATE statement
pub fn parse_relate(sql: &str) -> Result<Option<RelateStatement>, String> {
    let trimmed = sql.trim();

    // Remove trailing semicolon if present
    let input = trimmed.strip_suffix(';').unwrap_or(trimmed).trim();

    match parse_relate_internal(input) {
        Ok((remaining, stmt)) => {
            if remaining.trim().is_empty() {
                Ok(Some(stmt))
            } else {
                Err(format!(
                    "Unexpected trailing content after RELATE statement: '{}'",
                    remaining.trim()
                ))
            }
        }
        Err(e) => Err(format!("Failed to parse RELATE statement: {:?}", e)),
    }
}

/// Parse an UNRELATE statement
pub fn parse_unrelate(sql: &str) -> Result<Option<UnrelateStatement>, String> {
    let trimmed = sql.trim();

    // Remove trailing semicolon if present
    let input = trimmed.strip_suffix(';').unwrap_or(trimmed).trim();

    match parse_unrelate_internal(input) {
        Ok((remaining, stmt)) => {
            if remaining.trim().is_empty() {
                Ok(Some(stmt))
            } else {
                Err(format!(
                    "Unexpected trailing content after UNRELATE statement: '{}'",
                    remaining.trim()
                ))
            }
        }
        Err(e) => Err(format!("Failed to parse UNRELATE statement: {:?}", e)),
    }
}

// ============================================================================
// Internal Parsers (nom combinators)
// ============================================================================

/// Parse RELATE statement
fn parse_relate_internal(input: &str) -> IResult<&str, RelateStatement> {
    let (input, _) = tag_no_case("RELATE").parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Optional: IN BRANCH 'branch_name'
    let (input, branch) = opt(parse_in_branch).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Required: FROM ...
    let (input, source) = parse_from_clause(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Required: TO ...
    let (input, target) = parse_to_clause(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Optional: TYPE 'relation_type'
    let (input, relation_type) = opt(parse_type_clause).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Optional: WEIGHT number
    let (input, weight) = opt(parse_weight_clause).parse(input)?;

    Ok((
        input,
        RelateStatement {
            branch,
            source,
            target,
            relation_type,
            weight,
        },
    ))
}

/// Parse UNRELATE statement
fn parse_unrelate_internal(input: &str) -> IResult<&str, UnrelateStatement> {
    let (input, _) = tag_no_case("UNRELATE").parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Optional: IN BRANCH 'branch_name'
    let (input, branch) = opt(parse_in_branch).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Required: FROM ...
    let (input, source) = parse_from_clause(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Required: TO ...
    let (input, target) = parse_to_clause(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Optional: TYPE 'relation_type'
    let (input, relation_type) = opt(parse_type_clause).parse(input)?;

    Ok((
        input,
        UnrelateStatement {
            branch,
            source,
            target,
            relation_type,
        },
    ))
}

/// Parse: IN BRANCH 'branch_name'
fn parse_in_branch(input: &str) -> IResult<&str, String> {
    let (input, _) = tag_no_case("IN").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("BRANCH").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, branch) = parse_string_literal(input)?;
    Ok((input, branch))
}

/// Parse: FROM path|id='value' [IN WORKSPACE 'ws']
fn parse_from_clause(input: &str) -> IResult<&str, RelateEndpoint> {
    let (input, _) = tag_no_case("FROM").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    parse_endpoint(input)
}

/// Parse: TO path|id='value' [IN WORKSPACE 'ws']
fn parse_to_clause(input: &str) -> IResult<&str, RelateEndpoint> {
    let (input, _) = tag_no_case("TO").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    parse_endpoint(input)
}

/// Parse: path|id='value' [IN WORKSPACE 'ws']
fn parse_endpoint(input: &str) -> IResult<&str, RelateEndpoint> {
    let (input, node_ref) = parse_node_reference(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, workspace) = opt(parse_in_workspace).parse(input)?;
    Ok((
        input,
        RelateEndpoint {
            node_ref,
            workspace,
        },
    ))
}

/// Parse: path='value' or id='value'
fn parse_node_reference(input: &str) -> IResult<&str, RelateNodeReference> {
    alt((parse_path_reference, parse_id_reference)).parse(input)
}

/// Parse: path='value'
fn parse_path_reference(input: &str) -> IResult<&str, RelateNodeReference> {
    let (input, _) = tag_no_case("path").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char('=').parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, value) = parse_string_literal(input)?;
    Ok((input, RelateNodeReference::Path(value)))
}

/// Parse: id='value'
fn parse_id_reference(input: &str) -> IResult<&str, RelateNodeReference> {
    let (input, _) = tag_no_case("id").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char('=').parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, value) = parse_string_literal(input)?;
    Ok((input, RelateNodeReference::Id(value)))
}

/// Parse: IN WORKSPACE 'workspace_name'
fn parse_in_workspace(input: &str) -> IResult<&str, String> {
    let (input, _) = tag_no_case("IN").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("WORKSPACE").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, ws) = parse_string_literal(input)?;
    Ok((input, ws))
}

/// Parse: TYPE 'relation_type'
fn parse_type_clause(input: &str) -> IResult<&str, String> {
    let (input, _) = tag_no_case("TYPE").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, rel_type) = parse_string_literal(input)?;
    Ok((input, rel_type))
}

/// Parse: WEIGHT number
fn parse_weight_clause(input: &str) -> IResult<&str, f64> {
    let (input, _) = tag_no_case("WEIGHT").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, weight) = parse_number(input)?;
    Ok((input, weight))
}

/// Parse a quoted string literal (single quotes)
fn parse_string_literal(input: &str) -> IResult<&str, String> {
    let (input, _) = char('\'').parse(input)?;
    let (input, content) = take_while1(|c| c != '\'').parse(input)?;
    let (input, _) = char('\'').parse(input)?;
    Ok((input, content.to_string()))
}

/// Parse a number (integer or float)
fn parse_number(input: &str) -> IResult<&str, f64> {
    double.parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_relate_statement() {
        assert!(is_relate_statement("RELATE FROM path='/a' TO path='/b'"));
        assert!(is_relate_statement("relate from path='/a' to path='/b'"));
        assert!(is_relate_statement("  RELATE FROM path='/a' TO path='/b'"));
        assert!(!is_relate_statement("SELECT * FROM table"));
        assert!(!is_relate_statement("UNRELATE FROM path='/a' TO path='/b'"));
    }

    #[test]
    fn test_is_unrelate_statement() {
        assert!(is_unrelate_statement(
            "UNRELATE FROM path='/a' TO path='/b'"
        ));
        assert!(is_unrelate_statement(
            "unrelate from path='/a' to path='/b'"
        ));
        assert!(!is_unrelate_statement("RELATE FROM path='/a' TO path='/b'"));
    }

    #[test]
    fn test_parse_simple_relate() {
        let sql = "RELATE FROM path='/content/page1' TO path='/assets/image'";
        let result = parse_relate(sql).unwrap().unwrap();

        assert!(result.branch.is_none());
        assert!(matches!(
            result.source.node_ref,
            RelateNodeReference::Path(p) if p == "/content/page1"
        ));
        assert!(matches!(
            result.target.node_ref,
            RelateNodeReference::Path(p) if p == "/assets/image"
        ));
        assert!(result.relation_type.is_none());
        assert!(result.weight.is_none());
    }

    #[test]
    fn test_parse_relate_with_type_and_weight() {
        let sql = "RELATE FROM path='/a' TO path='/b' TYPE 'references' WEIGHT 1.5";
        let result = parse_relate(sql).unwrap().unwrap();

        assert_eq!(result.relation_type, Some("references".to_string()));
        assert_eq!(result.weight, Some(1.5));
    }

    #[test]
    fn test_parse_relate_with_branch() {
        let sql = "RELATE IN BRANCH 'feature/new' FROM path='/a' TO path='/b'";
        let result = parse_relate(sql).unwrap().unwrap();

        assert_eq!(result.branch, Some("feature/new".to_string()));
    }

    #[test]
    fn test_parse_relate_with_workspaces() {
        let sql =
            "RELATE FROM path='/page' IN WORKSPACE 'main' TO path='/asset' IN WORKSPACE 'media'";
        let result = parse_relate(sql).unwrap().unwrap();

        assert_eq!(result.source.workspace, Some("main".to_string()));
        assert_eq!(result.target.workspace, Some("media".to_string()));
    }

    #[test]
    fn test_parse_relate_with_id_references() {
        let sql = "RELATE FROM id='node-123' TO id='node-456' TYPE 'tagged'";
        let result = parse_relate(sql).unwrap().unwrap();

        assert!(matches!(
            result.source.node_ref,
            RelateNodeReference::Id(id) if id == "node-123"
        ));
        assert!(matches!(
            result.target.node_ref,
            RelateNodeReference::Id(id) if id == "node-456"
        ));
    }

    #[test]
    fn test_parse_relate_full() {
        let sql = r#"
            RELATE IN BRANCH 'feature/relations'
            FROM path='/content/blog/post-1' IN WORKSPACE 'content'
            TO path='/tags/rust' IN WORKSPACE 'tags'
            TYPE 'tagged_with'
            WEIGHT 2.5
        "#;
        let result = parse_relate(sql).unwrap().unwrap();

        assert_eq!(result.branch, Some("feature/relations".to_string()));
        assert!(matches!(
            result.source.node_ref,
            RelateNodeReference::Path(p) if p == "/content/blog/post-1"
        ));
        assert_eq!(result.source.workspace, Some("content".to_string()));
        assert!(matches!(
            result.target.node_ref,
            RelateNodeReference::Path(p) if p == "/tags/rust"
        ));
        assert_eq!(result.target.workspace, Some("tags".to_string()));
        assert_eq!(result.relation_type, Some("tagged_with".to_string()));
        assert_eq!(result.weight, Some(2.5));
    }

    #[test]
    fn test_parse_simple_unrelate() {
        let sql = "UNRELATE FROM path='/a' TO path='/b'";
        let result = parse_unrelate(sql).unwrap().unwrap();

        assert!(result.branch.is_none());
        assert!(matches!(
            result.source.node_ref,
            RelateNodeReference::Path(p) if p == "/a"
        ));
        assert!(matches!(
            result.target.node_ref,
            RelateNodeReference::Path(p) if p == "/b"
        ));
    }

    #[test]
    fn test_parse_unrelate_with_type() {
        let sql = "UNRELATE FROM path='/a' TO path='/b' TYPE 'tagged'";
        let result = parse_unrelate(sql).unwrap().unwrap();

        assert_eq!(result.relation_type, Some("tagged".to_string()));
    }

    #[test]
    fn test_parse_relate_with_semicolon() {
        let sql = "RELATE FROM path='/a' TO path='/b';";
        let result = parse_relate(sql).unwrap().unwrap();
        assert!(matches!(
            result.source.node_ref,
            RelateNodeReference::Path(_)
        ));
    }

    #[test]
    fn test_parse_relate_case_insensitive() {
        let sql = "relate FROM path='/a' TO path='/b' type 'ref' weight 1.0";
        let result = parse_relate(sql).unwrap().unwrap();
        assert_eq!(result.relation_type, Some("ref".to_string()));
        assert_eq!(result.weight, Some(1.0));
    }
}
