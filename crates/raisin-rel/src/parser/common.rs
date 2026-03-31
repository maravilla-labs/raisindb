//! Common parsing utilities and types

use nom::{
    branch::alt,
    bytes::complete::{tag, take_while, take_while1},
    character::complete::{char, multispace0, multispace1},
    combinator::{map, recognize},
    sequence::{delimited, pair, preceded},
    IResult, Parser,
};
use nom_locate::LocatedSpan;

/// Input type with position tracking
pub type Span<'a> = LocatedSpan<&'a str>;

/// Parser result type
pub type PResult<'a, O> = IResult<Span<'a>, O>;

/// Get position information from a Span
pub fn get_position(span: &Span) -> (usize, usize) {
    (span.location_line() as usize, span.get_column())
}

/// Check if a character is valid for the start of an identifier
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// Check if a character is valid for the rest of an identifier
fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Reserved keywords that cannot be used as identifiers
pub const KEYWORDS: &[&str] = &[
    "true",
    "false",
    "null",
    "contains",
    "startsWith",
    "endsWith",
    "RELATES",
    "VIA",
    "DEPTH",
    "DIRECTION",
    "OUTGOING",
    "INCOMING",
    "ANY",
];

/// Parse optional whitespace (including newlines)
pub fn ws0(input: Span) -> PResult<Span> {
    multispace0(input)
}

/// Parse required whitespace
pub fn ws1(input: Span) -> PResult<Span> {
    multispace1(input)
}

/// Wrap a parser to consume surrounding whitespace
pub fn ws<'a, O, F>(mut parser: F) -> impl FnMut(Span<'a>) -> PResult<'a, O>
where
    F: Parser<Span<'a>, Output = O, Error = nom::error::Error<Span<'a>>>,
{
    move |input| delimited(ws0, |i| parser.parse(i), ws0).parse(input)
}

/// Wrap a parser to consume leading whitespace
pub fn ws_before<'a, O, F>(mut parser: F) -> impl FnMut(Span<'a>) -> PResult<'a, O>
where
    F: Parser<Span<'a>, Output = O, Error = nom::error::Error<Span<'a>>>,
{
    move |input| preceded(ws0, |i| parser.parse(i)).parse(input)
}

/// Parse a token with surrounding whitespace
#[allow(dead_code)] // Reserved for future parser extensions
pub fn ws_token<'a>(t: &'a str) -> impl FnMut(Span<'a>) -> PResult<'a, Span<'a>> {
    move |input| delimited(ws0, tag(t), ws0).parse(input)
}

/// Parse an identifier (variable name)
/// Must start with letter or underscore, followed by letters, digits, or underscores
pub fn identifier(input: Span) -> PResult<String> {
    let (input, ident) =
        recognize(pair(take_while1(is_ident_start), take_while(is_ident_char))).parse(input)?;

    let ident_str = ident.fragment().to_string();

    // Check it's not a reserved keyword
    if KEYWORDS.contains(&ident_str.as_str()) {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }

    Ok((input, ident_str))
}

/// Parse a keyword (case-sensitive)
pub fn keyword(kw: &'static str) -> impl Fn(Span) -> PResult<()> {
    move |input: Span| {
        let (input, _) = tag(kw).parse(input)?;
        // Make sure keyword is not part of a longer identifier
        if let Ok((_, c)) =
            nom::character::complete::anychar::<Span, nom::error::Error<Span>>(input)
        {
            if is_ident_char(c) {
                return Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Tag,
                )));
            }
        }
        Ok((input, ()))
    }
}

/// Parse 'true' keyword
pub fn kw_true(input: Span) -> PResult<()> {
    keyword("true")(input)
}

/// Parse 'false' keyword
pub fn kw_false(input: Span) -> PResult<()> {
    keyword("false")(input)
}

/// Parse 'null' keyword
pub fn kw_null(input: Span) -> PResult<()> {
    keyword("null")(input)
}

/// Parse comparison operators
pub fn comparison_op(input: Span) -> PResult<&str> {
    alt((
        map(tag("=="), |_| "=="),
        map(tag("!="), |_| "!="),
        map(tag("<="), |_| "<="),
        map(tag(">="), |_| ">="),
        map(tag("<"), |_| "<"),
        map(tag(">"), |_| ">"),
    ))
    .parse(input)
}

/// Parse logical AND operator
pub fn and_op(input: Span) -> PResult<()> {
    map(tag("&&"), |_| ()).parse(input)
}

/// Parse logical OR operator
pub fn or_op(input: Span) -> PResult<()> {
    map(tag("||"), |_| ()).parse(input)
}

/// Parse unary NOT operator
pub fn not_op(input: Span) -> PResult<()> {
    map(char('!'), |_| ()).parse(input)
}

/// Parse unary minus operator
pub fn neg_op(input: Span) -> PResult<()> {
    map(char('-'), |_| ()).parse(input)
}

/// Parse addition operator
pub fn add_op(input: Span) -> PResult<()> {
    map(char('+'), |_| ()).parse(input)
}

/// Parse subtraction operator (binary)
pub fn sub_op(input: Span) -> PResult<()> {
    map(char('-'), |_| ()).parse(input)
}

/// Parse multiplication operator
pub fn mul_op(input: Span) -> PResult<()> {
    map(char('*'), |_| ()).parse(input)
}

/// Parse division operator
pub fn div_op(input: Span) -> PResult<()> {
    map(char('/'), |_| ()).parse(input)
}

/// Parse modulo operator
pub fn mod_op(input: Span) -> PResult<()> {
    map(char('%'), |_| ()).parse(input)
}

/// Parse a comma separator with optional whitespace
pub fn comma(input: Span) -> PResult<()> {
    map(delimited(ws0, char(','), ws0), |_| ()).parse(input)
}

/// Parse opening parenthesis
pub fn open_paren(input: Span) -> PResult<()> {
    map(preceded(ws0, char('(')), |_| ()).parse(input)
}

/// Parse closing parenthesis
pub fn close_paren(input: Span) -> PResult<()> {
    map(preceded(ws0, char(')')), |_| ()).parse(input)
}

/// Parse opening bracket
pub fn open_bracket(input: Span) -> PResult<()> {
    map(preceded(ws0, char('[')), |_| ()).parse(input)
}

/// Parse closing bracket
pub fn close_bracket(input: Span) -> PResult<()> {
    map(preceded(ws0, char(']')), |_| ()).parse(input)
}

/// Parse opening brace
pub fn open_brace(input: Span) -> PResult<()> {
    map(preceded(ws0, char('{')), |_| ()).parse(input)
}

/// Parse closing brace
pub fn close_brace(input: Span) -> PResult<()> {
    map(preceded(ws0, char('}')), |_| ()).parse(input)
}

/// Parse a dot for property access
pub fn dot(input: Span) -> PResult<()> {
    map(char('.'), |_| ()).parse(input)
}

/// Parse a colon for object literals
pub fn colon(input: Span) -> PResult<()> {
    map(delimited(ws0, char(':'), ws0), |_| ()).parse(input)
}

/// Parse '..' for range syntax
pub fn dotdot(input: Span) -> PResult<()> {
    map(tag(".."), |_| ()).parse(input)
}

/// Parse 'RELATES' keyword
pub fn kw_relates(input: Span) -> PResult<()> {
    keyword("RELATES")(input)
}

/// Parse 'VIA' keyword
pub fn kw_via(input: Span) -> PResult<()> {
    keyword("VIA")(input)
}

/// Parse 'DEPTH' keyword
pub fn kw_depth(input: Span) -> PResult<()> {
    keyword("DEPTH")(input)
}

/// Parse 'DIRECTION' keyword
pub fn kw_direction(input: Span) -> PResult<()> {
    keyword("DIRECTION")(input)
}

/// Parse 'OUTGOING' keyword
pub fn kw_outgoing(input: Span) -> PResult<()> {
    keyword("OUTGOING")(input)
}

/// Parse 'INCOMING' keyword
pub fn kw_incoming(input: Span) -> PResult<()> {
    keyword("INCOMING")(input)
}

/// Parse 'ANY' keyword
pub fn kw_any(input: Span) -> PResult<()> {
    keyword("ANY")(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(s: &str) -> Span {
        Span::new(s)
    }

    #[test]
    fn test_identifier() {
        let (_, id) = identifier(span("foo")).unwrap();
        assert_eq!(id, "foo");

        let (_, id) = identifier(span("_bar123")).unwrap();
        assert_eq!(id, "_bar123");

        let (rem, id) = identifier(span("myVar.something")).unwrap();
        assert_eq!(id, "myVar");
        assert_eq!(*rem.fragment(), ".something");
    }

    #[test]
    fn test_identifier_rejects_keywords() {
        assert!(identifier(span("true")).is_err());
        assert!(identifier(span("false")).is_err());
        assert!(identifier(span("null")).is_err());
    }

    #[test]
    fn test_keyword() {
        assert!(kw_true(span("true")).is_ok());
        assert!(kw_true(span("trueish")).is_err());
        assert!(kw_false(span("false")).is_ok());
        assert!(kw_null(span("null")).is_ok());
    }

    #[test]
    fn test_comparison_op() {
        let (_, op) = comparison_op(span("==")).unwrap();
        assert_eq!(op, "==");

        let (_, op) = comparison_op(span("!=")).unwrap();
        assert_eq!(op, "!=");

        let (_, op) = comparison_op(span("<=")).unwrap();
        assert_eq!(op, "<=");

        let (_, op) = comparison_op(span("<")).unwrap();
        assert_eq!(op, "<");
    }
}
