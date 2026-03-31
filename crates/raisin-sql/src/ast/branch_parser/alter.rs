//\! ALTER BRANCH statement parser

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    character::complete::multispace1,
    combinator::{map, value},
    sequence::{preceded, tuple},
    IResult, Parser,
};

use super::super::branch::{AlterBranch, BranchAlteration};
use super::helpers::{boolean_value, branch_name, quoted_string};

/// Parse ALTER BRANCH statement
pub(crate) fn alter_branch(input: &str) -> IResult<&str, AlterBranch> {
    let (input, _) = tag_no_case("ALTER").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("BRANCH").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse branch name
    let (input, name) = branch_name(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse alteration
    let (input, alteration) = branch_alteration(input)?;

    Ok((input, AlterBranch { name, alteration }))
}

/// Parse branch alteration: SET UPSTREAM, UNSET UPSTREAM, SET PROTECTED, SET DESCRIPTION, RENAME TO
fn branch_alteration(input: &str) -> IResult<&str, BranchAlteration> {
    alt((
        // SET UPSTREAM 'branch'
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("UPSTREAM"),
                    multispace1,
                ),
                branch_name,
            ),
            BranchAlteration::SetUpstream,
        ),
        // UNSET UPSTREAM
        value(
            BranchAlteration::UnsetUpstream,
            tuple((tag_no_case("UNSET"), multispace1, tag_no_case("UPSTREAM"))),
        ),
        // SET PROTECTED TRUE/FALSE
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("PROTECTED"),
                    multispace1,
                ),
                boolean_value,
            ),
            BranchAlteration::SetProtected,
        ),
        // SET DESCRIPTION 'description'
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("DESCRIPTION"),
                    multispace1,
                ),
                quoted_string,
            ),
            |s| BranchAlteration::SetDescription(s.to_string()),
        ),
        // RENAME TO 'new_name'
        map(
            preceded(
                (
                    tag_no_case("RENAME"),
                    multispace1,
                    tag_no_case("TO"),
                    multispace1,
                ),
                branch_name,
            ),
            BranchAlteration::RenameTo,
        ),
    ))
    .parse(input)
}
