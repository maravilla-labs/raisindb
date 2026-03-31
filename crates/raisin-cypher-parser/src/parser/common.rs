// SPDX-License-Identifier: BSL-1.1

//! Common parsing utilities: whitespace, identifiers, keywords, parameters

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while, take_while1},
    character::complete::{char, multispace1},
    combinator::{map, opt, recognize},
    multi::{many0, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded},
    IResult, Parser,
};
use nom_locate::LocatedSpan;

/// Input type with position tracking
pub type Span<'a> = LocatedSpan<&'a str>;

/// Parser result type
pub type PResult<'a, O> = IResult<Span<'a>, O>;

/// Parse optional whitespace (spaces, tabs, newlines, comments)
pub fn ws0(input: Span) -> PResult<Span> {
    recognize(many0(alt((multispace1, line_comment, block_comment)))).parse(input)
}

/// Parse required whitespace
#[allow(dead_code)] // Reserved for future parser extensions
pub fn ws1(input: Span) -> PResult<Span> {
    recognize(alt((multispace1, line_comment, block_comment))).parse(input)
}

/// Parse line comment: // ... \n
fn line_comment(input: Span) -> PResult<Span> {
    recognize((tag("//"), take_while(|c| c != '\n'), opt(char('\n')))).parse(input)
}

/// Parse block comment: /* ... */
fn block_comment(input: Span) -> PResult<Span> {
    recognize((
        tag("/*"),
        take_while(|c| c != '*'),
        many0((
            take_while1(|c| c == '*'),
            take_while(|c| c != '/' && c != '*'),
        )),
        tag("*/"),
    ))
    .parse(input)
}

/// Parse identifier: letter or underscore followed by alphanumeric or underscore
pub fn identifier(input: Span) -> PResult<String> {
    let (input, name) = recognize(pair(
        alt((take_while1(|c: char| c.is_alphabetic()), tag("_"))),
        take_while(|c: char| c.is_alphanumeric() || c == '_'),
    ))
    .parse(input)?;

    let name_str = name.fragment().to_string();

    // Check if it's a reserved keyword - if so, fail
    if is_keyword(&name_str) {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }

    Ok((input, name_str))
}

/// Parse escaped identifier: `name with spaces`
pub fn escaped_identifier(input: Span) -> PResult<String> {
    delimited(
        char('`'),
        map(take_while(|c| c != '`'), |s: Span| s.fragment().to_string()),
        char('`'),
    )
    .parse(input)
}

/// Parse any identifier (normal or escaped)
pub fn any_identifier(input: Span) -> PResult<String> {
    alt((escaped_identifier, identifier)).parse(input)
}

/// Parse a keyword (case-insensitive)
pub fn keyword(kw: &'static str) -> impl Fn(Span) -> PResult<Span> {
    move |input: Span| {
        let (input, matched) = tag_no_case(kw).parse(input)?;

        // Ensure keyword is not followed by alphanumeric (word boundary)
        if let Ok((_, next_char)) =
            take_while1::<_, _, nom::error::Error<Span>>(|c: char| c.is_alphanumeric() || c == '_')
                .parse(input)
        {
            if !next_char.fragment().is_empty() {
                return Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Tag,
                )));
            }
        }

        Ok((input, matched))
    }
}

/// Check if a string is a reserved keyword
fn is_keyword(s: &str) -> bool {
    matches!(
        s.to_uppercase().as_str(),
        "MATCH"
            | "OPTIONAL"
            | "WHERE"
            | "CREATE"
            | "MERGE"
            | "DELETE"
            | "DETACH"
            | "REMOVE"
            | "SET"
            | "RETURN"
            | "WITH"
            | "DISTINCT"
            | "ORDER"
            | "BY"
            | "ASC"
            | "DESC"
            | "SKIP"
            | "LIMIT"
            | "AS"
            | "AND"
            | "OR"
            | "XOR"
            | "NOT"
            | "IN"
            | "STARTS"
            | "ENDS"
            | "CONTAINS"
            | "IS"
            | "NULL"
            | "TRUE"
            | "FALSE"
            | "UNWIND"
            | "CASE"
            | "WHEN"
            | "THEN"
            | "ELSE"
            | "END"
    )
}

/// Parse a parameter: $name or $123
pub fn parameter(input: Span) -> PResult<String> {
    preceded(
        char('$'),
        map(
            alt((
                recognize(take_while1(|c: char| c.is_alphanumeric() || c == '_')),
                recognize(take_while1(|c: char| c.is_numeric())),
            )),
            |s: Span| s.fragment().to_string(),
        ),
    )
    .parse(input)
}

/// Parse token with optional whitespace on both sides
pub fn ws_token<'a, O, F>(mut f: F) -> impl FnMut(Span<'a>) -> PResult<'a, O>
where
    F: Parser<Span<'a>, Output = O, Error = nom::error::Error<Span<'a>>>,
{
    move |input| delimited(ws0, |i| f.parse(i), ws0).parse(input)
}

/// Parse comma-separated list (0 or more)
pub fn comma_sep0<'a, O, F>(mut f: F) -> impl FnMut(Span<'a>) -> PResult<'a, Vec<O>>
where
    F: Parser<Span<'a>, Output = O, Error = nom::error::Error<Span<'a>>>,
{
    move |input| separated_list0(delimited(ws0, char(','), ws0), |i| f.parse(i)).parse(input)
}

/// Parse comma-separated list (1 or more)
pub fn comma_sep1<'a, O, F>(mut f: F) -> impl FnMut(Span<'a>) -> PResult<'a, Vec<O>>
where
    F: Parser<Span<'a>, Output = O, Error = nom::error::Error<Span<'a>>>,
{
    move |input| separated_list1(delimited(ws0, char(','), ws0), |i| f.parse(i)).parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(s: &str) -> Span {
        Span::new(s)
    }

    #[test]
    fn test_identifier() {
        assert_eq!(identifier(span("abc")).unwrap().1, "abc");
        assert_eq!(identifier(span("_test")).unwrap().1, "_test");
        assert_eq!(identifier(span("var123")).unwrap().1, "var123");
        assert!(identifier(span("MATCH")).is_err()); // keyword
    }

    #[test]
    fn test_escaped_identifier() {
        assert_eq!(
            escaped_identifier(span("`name with spaces`")).unwrap().1,
            "name with spaces"
        );
    }

    #[test]
    fn test_keyword() {
        assert!(keyword("MATCH")(span("MATCH")).is_ok());
        assert!(keyword("MATCH")(span("match")).is_ok());
        assert!(keyword("MATCH")(span("Match")).is_ok());
        assert!(keyword("MATCH")(span("MATCHES")).is_err()); // not a word boundary
    }

    #[test]
    fn test_parameter() {
        assert_eq!(parameter(span("$param")).unwrap().1, "param");
        assert_eq!(parameter(span("$123")).unwrap().1, "123");
    }

    #[test]
    fn test_ws0() {
        assert!(ws0(span("  \n\t")).is_ok());
        assert!(ws0(span("// comment\n")).is_ok());
        assert!(ws0(span("/* block */")).is_ok());
    }
}
