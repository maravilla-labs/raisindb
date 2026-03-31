//\! CREATE BRANCH statement parser

use nom::{
    bytes::complete::tag_no_case,
    character::complete::{multispace0, multispace1},
    sequence::{preceded, tuple},
    IResult, Parser,
};

use super::super::branch::CreateBranch;
use super::helpers::{branch_name, quoted_string, revision_ref};

/// Parse CREATE BRANCH statement
pub(crate) fn create_branch(input: &str) -> IResult<&str, CreateBranch> {
    let (input, _) = tag_no_case("CREATE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("BRANCH").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse branch name (quoted or unquoted)
    let (input, name) = branch_name(input)?;

    // Parse optional clauses in any order
    let (input, stmt) = parse_create_branch_clauses(input, CreateBranch::new(name))?;

    // Finalize the statement
    Ok((input, stmt))
}

/// Parse optional clauses for CREATE BRANCH in any order
fn parse_create_branch_clauses(
    mut input: &str,
    mut result: CreateBranch,
) -> IResult<&str, CreateBranch> {
    loop {
        let (remaining, _) = multispace0.parse(input)?;

        // Try FROM clause
        if let Ok((new_input, from)) =
            preceded((tag_no_case("FROM"), multispace1), branch_name).parse(remaining)
        {
            result.from_branch = Some(from);
            input = new_input;
            continue;
        }

        // Try AT REVISION clause
        if let Ok((new_input, rev)) = preceded(
            (
                tag_no_case("AT"),
                multispace1,
                tag_no_case("REVISION"),
                multispace1,
            ),
            revision_ref,
        )
        .parse(remaining)
        {
            result.at_revision = Some(rev);
            input = new_input;
            continue;
        }

        // Try DESCRIPTION clause
        if let Ok((new_input, desc)) =
            preceded((tag_no_case("DESCRIPTION"), multispace1), quoted_string).parse(remaining)
        {
            result.description = Some(desc.to_string());
            input = new_input;
            continue;
        }

        // Try PROTECTED flag
        if let Ok((new_input, _)) =
            tag_no_case::<&str, &str, nom::error::Error<&str>>("PROTECTED").parse(remaining)
        {
            result.protected = true;
            input = new_input;
            continue;
        }

        // Try UPSTREAM clause
        if let Ok((new_input, upstream)) =
            preceded((tag_no_case("UPSTREAM"), multispace1), branch_name).parse(remaining)
        {
            result.upstream = Some(upstream);
            input = new_input;
            continue;
        }

        // Try WITH HISTORY clause
        if let Ok((new_input, _)) = tuple((
            tag_no_case::<&str, &str, nom::error::Error<&str>>("WITH"),
            multispace1,
            tag_no_case("HISTORY"),
        ))
        .parse(remaining)
        {
            result.with_history = true;
            input = new_input;
            continue;
        }

        break;
    }

    Ok((input, result))
}
