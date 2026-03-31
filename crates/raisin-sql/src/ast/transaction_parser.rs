//! Transaction statement parser using nom combinators
//!
//! Parses transaction control statements:
//! - BEGIN / BEGIN TRANSACTION
//! - COMMIT [WITH MESSAGE 'msg'] [ACTOR 'actor']

use nom::{
    bytes::complete::{tag_no_case, take_until},
    character::complete::{char, multispace0, multispace1},
    combinator::opt,
    sequence::preceded,
    IResult, Parser,
};

use super::transaction::TransactionStatement;

/// Parse a transaction statement (BEGIN, COMMIT, or SET)
pub fn parse_transaction(input: &str) -> IResult<&str, TransactionStatement> {
    // Strip leading whitespace and comments
    let trimmed = input.trim();
    let statement_start = super::ddl_parser::strip_leading_comments(trimmed);
    let upper = statement_start.to_uppercase();

    // Determine which transaction statement type
    if upper.starts_with("BEGIN") {
        parse_begin(statement_start)
    } else if upper.starts_with("COMMIT") {
        parse_commit(statement_start)
    } else if upper.starts_with("SET ") {
        parse_set(statement_start)
    } else {
        Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )))
    }
}

/// Parse BEGIN [TRANSACTION]
fn parse_begin(input: &str) -> IResult<&str, TransactionStatement> {
    let (input, _) = tag_no_case("BEGIN").parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Optional TRANSACTION keyword
    let (input, _) = opt(tag_no_case("TRANSACTION")).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    Ok((input, TransactionStatement::Begin))
}

/// Parse COMMIT [WITH MESSAGE 'msg'] [ACTOR 'actor']
fn parse_commit(input: &str) -> IResult<&str, TransactionStatement> {
    let (input, _) = tag_no_case("COMMIT").parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Optional MESSAGE clause
    let (input, message) = opt(preceded(
        (
            tag_no_case("WITH"),
            multispace1,
            tag_no_case("MESSAGE"),
            multispace0,
        ),
        quoted_string,
    ))
    .parse(input)?;

    let (input, _) = multispace0.parse(input)?;

    // Optional ACTOR clause
    let (input, actor) =
        opt(preceded((tag_no_case("ACTOR"), multispace0), quoted_string)).parse(input)?;

    Ok((
        input,
        TransactionStatement::Commit {
            message: message.map(|s| s.to_string()),
            actor: actor.map(|s| s.to_string()),
        },
    ))
}

/// Parse a quoted string: 'content' or "content"
fn quoted_string(input: &str) -> IResult<&str, &str> {
    nom::branch::alt((
        nom::sequence::delimited(char('\''), take_until("'"), char('\'')),
        nom::sequence::delimited(char('"'), take_until("\""), char('"')),
    ))
    .parse(input)
}

/// Parse SET variable = value
/// Supports: SET variable = value, SET variable = 'value', SET variable = true/false
fn parse_set(input: &str) -> IResult<&str, TransactionStatement> {
    let (input, _) = tag_no_case("SET").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse variable name (alphanumeric with underscores)
    let (input, variable) =
        nom::bytes::complete::take_while1(|c: char| c.is_alphanumeric() || c == '_')
            .parse(input)?;

    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char('=').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Parse value - can be quoted string or unquoted identifier
    let (input, value) = nom::branch::alt((
        // Quoted string
        quoted_string,
        // Unquoted value (true, false, numbers, identifiers)
        nom::bytes::complete::take_while1(|c: char| {
            c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
        }),
    ))
    .parse(input)?;

    Ok((
        input,
        TransactionStatement::Set {
            variable: variable.to_string(),
            value: value.to_string(),
        },
    ))
}

/// Check if a SQL statement is a transaction statement
pub fn is_transaction_statement(sql: &str) -> bool {
    let trimmed = sql.trim().to_uppercase();
    trimmed.starts_with("BEGIN") || trimmed.starts_with("COMMIT") || trimmed.starts_with("SET ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_begin() {
        let result = parse_transaction("BEGIN");
        assert!(result.is_ok());
        let (remaining, stmt) = result.unwrap();
        assert_eq!(stmt, TransactionStatement::Begin);
        assert_eq!(remaining.trim(), "");
    }

    #[test]
    fn test_parse_begin_transaction() {
        let result = parse_transaction("BEGIN TRANSACTION");
        assert!(result.is_ok());
        let (remaining, stmt) = result.unwrap();
        assert_eq!(stmt, TransactionStatement::Begin);
        assert_eq!(remaining.trim(), "");
    }

    #[test]
    fn test_parse_commit() {
        let result = parse_transaction("COMMIT");
        assert!(result.is_ok());
        let (remaining, stmt) = result.unwrap();
        assert_eq!(
            stmt,
            TransactionStatement::Commit {
                message: None,
                actor: None
            }
        );
        assert_eq!(remaining.trim(), "");
    }

    #[test]
    fn test_parse_commit_with_message() {
        let result = parse_transaction("COMMIT WITH MESSAGE 'Created new article'");
        assert!(result.is_ok());
        let (remaining, stmt) = result.unwrap();
        assert_eq!(
            stmt,
            TransactionStatement::Commit {
                message: Some("Created new article".to_string()),
                actor: None
            }
        );
        assert_eq!(remaining.trim(), "");
    }

    #[test]
    fn test_parse_commit_with_message_and_actor() {
        let result = parse_transaction("COMMIT WITH MESSAGE 'Updated profile' ACTOR 'user123'");
        assert!(result.is_ok());
        let (remaining, stmt) = result.unwrap();
        assert_eq!(
            stmt,
            TransactionStatement::Commit {
                message: Some("Updated profile".to_string()),
                actor: Some("user123".to_string())
            }
        );
        assert_eq!(remaining.trim(), "");
    }

    #[test]
    fn test_parse_commit_with_actor_only() {
        let result = parse_transaction("COMMIT ACTOR 'admin'");
        assert!(result.is_ok());
        let (remaining, stmt) = result.unwrap();
        assert_eq!(
            stmt,
            TransactionStatement::Commit {
                message: None,
                actor: Some("admin".to_string())
            }
        );
        assert_eq!(remaining.trim(), "");
    }

    #[test]
    fn test_is_transaction_statement() {
        assert!(is_transaction_statement("BEGIN"));
        assert!(is_transaction_statement("BEGIN TRANSACTION"));
        assert!(is_transaction_statement("COMMIT"));
        assert!(is_transaction_statement("COMMIT WITH MESSAGE 'test'"));
        assert!(!is_transaction_statement("SELECT * FROM nodes"));
        assert!(!is_transaction_statement("INSERT INTO nodes VALUES (...)"));
    }

    #[test]
    fn test_case_insensitive() {
        assert!(parse_transaction("begin").is_ok());
        assert!(parse_transaction("BeGiN TrAnSaCtIoN").is_ok());
        assert!(parse_transaction("commit").is_ok());
        assert!(parse_transaction("CoMmIt WiTh MeSsAgE 'test'").is_ok());
    }
}
