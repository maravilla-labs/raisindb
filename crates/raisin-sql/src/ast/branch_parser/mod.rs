//\! BRANCH statement parser using nom combinators
//\!
//\! Parses branch management statements:
//\! - CREATE BRANCH 'name' FROM 'main' [AT REVISION ...] [DESCRIPTION '...'] [PROTECTED] [UPSTREAM '...'] [WITH HISTORY]
//\! - DROP BRANCH [IF EXISTS] 'name'
//\! - ALTER BRANCH 'name' SET UPSTREAM 'main' / UNSET UPSTREAM / SET PROTECTED TRUE/FALSE / RENAME TO 'new'
//\! - MERGE BRANCH 'source' INTO 'target' [USING FAST_FORWARD|THREE_WAY] [MESSAGE '...']
//\! - USE BRANCH 'name' / CHECKOUT BRANCH 'name' / SET app.branch = 'name'
//\! - USE LOCAL BRANCH 'name' / SET LOCAL app.branch = 'name'
//\! - SHOW BRANCHES / SHOW CURRENT BRANCH / SHOW app.branch / DESCRIBE BRANCH 'name' / SHOW DIVERGENCE 'x' FROM 'y'

mod alter;
mod create;
mod drop;
mod helpers;
mod merge;
mod use_branch;

#[cfg(test)]
mod tests;

use nom::{branch::alt, combinator::map, IResult, Parser};

use super::branch::BranchStatement;

use alter::alter_branch;
use create::create_branch;
use drop::drop_branch;
use merge::merge_branch;
use use_branch::{
    describe_branch, set_app_branch, show_app_branch, show_branches, show_conflicts_for_merge,
    show_current_branch, show_divergence, use_branch, use_local_branch,
};

/// Error type for BRANCH statement parsing
#[derive(Debug, Clone, PartialEq)]
pub struct BranchParseError {
    pub message: String,
    pub position: Option<usize>,
}

impl std::fmt::Display for BranchParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(pos) = self.position {
            write!(
                f,
                "BRANCH parse error at position {}: {}",
                pos, self.message
            )
        } else {
            write!(f, "BRANCH parse error: {}", self.message)
        }
    }
}

impl std::error::Error for BranchParseError {}

/// Check if a SQL statement is a BRANCH-related statement
pub fn is_branch_statement(sql: &str) -> bool {
    let upper = sql.trim().to_uppercase();

    upper.starts_with("CREATE BRANCH")
        || upper.starts_with("DROP BRANCH")
        || upper.starts_with("ALTER BRANCH")
        || upper.starts_with("MERGE BRANCH")
        || upper.starts_with("USE BRANCH")
        || upper.starts_with("USE LOCAL BRANCH")
        || upper.starts_with("CHECKOUT BRANCH")
        || upper.starts_with("SHOW BRANCHES")
        || upper.starts_with("SHOW CURRENT BRANCH")
        || upper.starts_with("DESCRIBE BRANCH")
        || upper.starts_with("SHOW DIVERGENCE")
        || upper.starts_with("SHOW CONFLICTS")
        // PostgreSQL-compatible SET app.branch syntax
        || upper.starts_with("SET APP.BRANCH")
        || upper.starts_with("SET LOCAL APP.BRANCH")
        || upper.starts_with("SHOW APP.BRANCH")
}

/// Parse a BRANCH statement from SQL string
///
/// Returns `Some(BranchStatement)` if the input is a valid branch statement,
/// `None` if it's not a branch statement (should be handled by other parsers).
pub fn parse_branch(sql: &str) -> Result<Option<BranchStatement>, BranchParseError> {
    let trimmed = sql.trim();

    // Strip leading SQL comments
    let statement_start = super::ddl_parser::strip_leading_comments(trimmed);

    // Check if this is a branch statement
    if !is_branch_statement(statement_start) {
        return Ok(None);
    }

    // Calculate offset for error position mapping
    let offset_to_statement_start = statement_start.as_ptr() as usize - sql.as_ptr() as usize;

    match branch_statement(statement_start) {
        Ok((remaining, stmt)) => {
            // Verify we consumed all input (except whitespace and semicolon)
            let remaining_trimmed = remaining.trim().trim_end_matches(';').trim();
            if remaining_trimmed.is_empty() {
                Ok(Some(stmt))
            } else {
                let position_in_statement = statement_start.len() - remaining.len();
                let position = offset_to_statement_start + position_in_statement;
                Err(BranchParseError {
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
                nom::Err::Incomplete(_) => (None, "Incomplete BRANCH statement".to_string()),
            };
            Err(BranchParseError { message, position })
        }
    }
}

/// Parse any branch statement
fn branch_statement(input: &str) -> IResult<&str, BranchStatement> {
    alt((
        map(create_branch, BranchStatement::Create),
        map(drop_branch, BranchStatement::Drop),
        map(alter_branch, BranchStatement::Alter),
        map(merge_branch, BranchStatement::Merge),
        use_local_branch, // Must come before use_branch to match LOCAL first
        use_branch,
        set_app_branch,  // SET [LOCAL] app.branch = 'x'
        show_app_branch, // SHOW app.branch (alias for SHOW CURRENT BRANCH)
        show_current_branch,
        show_branches,
        describe_branch,
        show_conflicts_for_merge, // Must come before show_divergence (more specific)
        show_divergence,
    ))
    .parse(input)
}
