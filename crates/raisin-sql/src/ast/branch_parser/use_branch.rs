//\! USE/CHECKOUT BRANCH and SHOW statement parsers
//\!
//\! Handles parsing of:
//\! - USE BRANCH / CHECKOUT BRANCH (session scope)
//\! - USE LOCAL BRANCH (local scope)
//\! - SET [LOCAL] app.branch = 'name' / SET [LOCAL] app.branch TO 'name'
//\! - SHOW app.branch / SHOW CURRENT BRANCH / SHOW BRANCHES
//\! - DESCRIBE BRANCH / SHOW DIVERGENCE / SHOW CONFLICTS

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    character::complete::{char, multispace0, multispace1},
    combinator::{map, opt},
    sequence::tuple,
    IResult, Parser,
};

use super::super::branch::{BranchScope, BranchStatement};
use super::helpers::branch_name;

/// Parse USE BRANCH or CHECKOUT BRANCH statement (Session scope)
pub(crate) fn use_branch(input: &str) -> IResult<&str, BranchStatement> {
    let (input, _) = alt((tag_no_case("USE"), tag_no_case("CHECKOUT"))).parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("BRANCH").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    let (input, name) = branch_name(input)?;

    Ok((
        input,
        BranchStatement::UseBranch {
            name,
            scope: BranchScope::Session,
        },
    ))
}

/// Parse USE LOCAL BRANCH statement (Local scope - single statement only)
pub(crate) fn use_local_branch(input: &str) -> IResult<&str, BranchStatement> {
    let (input, _) = tag_no_case("USE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("LOCAL").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("BRANCH").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    let (input, name) = branch_name(input)?;

    Ok((
        input,
        BranchStatement::UseBranch {
            name,
            scope: BranchScope::Local,
        },
    ))
}

/// Parse SET [LOCAL] app.branch = 'name' or SET [LOCAL] app.branch TO 'name'
pub(crate) fn set_app_branch(input: &str) -> IResult<&str, BranchStatement> {
    let (input, _) = tag_no_case("SET").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Check for optional LOCAL keyword
    let (input, is_local) = opt(tuple((tag_no_case("LOCAL"), multispace1))).parse(input)?;
    let scope = if is_local.is_some() {
        BranchScope::Local
    } else {
        BranchScope::Session
    };

    // Parse app.branch
    let (input, _) = tag_no_case("APP.BRANCH").parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Parse = or TO
    let (input, _) = alt((
        map(tuple((char('='), multispace0)), |_| ()),
        map(tuple((tag_no_case("TO"), multispace1)), |_| ()),
    ))
    .parse(input)?;

    // Parse branch name
    let (input, name) = branch_name(input)?;

    Ok((input, BranchStatement::UseBranch { name, scope }))
}

/// Parse SHOW app.branch (alias for SHOW CURRENT BRANCH)
pub(crate) fn show_app_branch(input: &str) -> IResult<&str, BranchStatement> {
    let (input, _) = tag_no_case("SHOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("APP.BRANCH").parse(input)?;

    Ok((input, BranchStatement::ShowCurrentBranch))
}

// ============================================================================
// SHOW statements parser
// ============================================================================

/// Parse SHOW CURRENT BRANCH statement
pub(crate) fn show_current_branch(input: &str) -> IResult<&str, BranchStatement> {
    let (input, _) = tag_no_case("SHOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("CURRENT").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("BRANCH").parse(input)?;

    Ok((input, BranchStatement::ShowCurrentBranch))
}

/// Parse SHOW BRANCHES statement
pub(crate) fn show_branches(input: &str) -> IResult<&str, BranchStatement> {
    let (input, _) = tag_no_case("SHOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("BRANCHES").parse(input)?;

    Ok((input, BranchStatement::ShowBranches))
}

/// Parse DESCRIBE BRANCH 'name' statement
pub(crate) fn describe_branch(input: &str) -> IResult<&str, BranchStatement> {
    let (input, _) = tag_no_case("DESCRIBE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("BRANCH").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    let (input, name) = branch_name(input)?;

    Ok((input, BranchStatement::DescribeBranch(name)))
}

/// Parse SHOW CONFLICTS FOR MERGE 'source' INTO 'target' statement
///
/// This allows previewing conflicts before performing a merge.
pub(crate) fn show_conflicts_for_merge(input: &str) -> IResult<&str, BranchStatement> {
    let (input, _) = tag_no_case("SHOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("CONFLICTS").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("FOR").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("MERGE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    let (input, source) = branch_name(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("INTO").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, target) = branch_name(input)?;

    Ok((input, BranchStatement::ShowConflicts { source, target }))
}

/// Parse SHOW DIVERGENCE 'branch' FROM 'base' statement
pub(crate) fn show_divergence(input: &str) -> IResult<&str, BranchStatement> {
    let (input, _) = tag_no_case("SHOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("DIVERGENCE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    let (input, branch) = branch_name(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("FROM").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, from) = branch_name(input)?;

    Ok((input, BranchStatement::ShowDivergence { branch, from }))
}
