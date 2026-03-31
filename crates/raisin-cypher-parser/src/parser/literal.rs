// SPDX-License-Identifier: BSL-1.1

//! Literal value parsing: null, booleans, numbers, strings

use super::common::{keyword, PResult, Span};
use crate::ast::Literal;
use nom::{
    branch::alt,
    bytes::complete::{take_while, take_while1},
    character::complete::{char, one_of},
    combinator::{map, opt, recognize, value},
    multi::many0,
    sequence::{delimited, preceded},
    Parser,
};

/// Parse any literal value
pub fn literal(input: Span) -> PResult<Literal> {
    alt((
        null_literal,
        boolean_literal,
        float_literal,
        integer_literal,
        string_literal,
    ))
    .parse(input)
}

/// Parse null literal
fn null_literal(input: Span) -> PResult<Literal> {
    value(Literal::Null, keyword("NULL")).parse(input)
}

/// Parse boolean literal
fn boolean_literal(input: Span) -> PResult<Literal> {
    alt((
        value(Literal::Boolean(true), keyword("TRUE")),
        value(Literal::Boolean(false), keyword("FALSE")),
    ))
    .parse(input)
}

/// Parse integer literal
fn integer_literal(input: Span) -> PResult<Literal> {
    map(
        recognize((opt(char('-')), take_while1(|c: char| c.is_ascii_digit()))),
        |s: Span| {
            let num = s.fragment().parse::<i64>().unwrap();
            Literal::Integer(num)
        },
    )
    .parse(input)
}

/// Parse float literal
fn float_literal(input: Span) -> PResult<Literal> {
    map(
        recognize((
            opt(char('-')),
            alt((
                // 123.456
                recognize((
                    take_while1(|c: char| c.is_ascii_digit()),
                    char('.'),
                    take_while(|c: char| c.is_ascii_digit()),
                    opt(exponent),
                )),
                // 123e10
                recognize((take_while1(|c: char| c.is_ascii_digit()), exponent)),
                // .456
                recognize((
                    char('.'),
                    take_while1(|c: char| c.is_ascii_digit()),
                    opt(exponent),
                )),
            )),
        )),
        |s: Span| {
            let num = s.fragment().parse::<f64>().unwrap();
            Literal::Float(num)
        },
    )
    .parse(input)
}

/// Parse exponent part: e+10, E-5, e2
fn exponent(input: Span) -> PResult<Span> {
    recognize((
        one_of("eE"),
        opt(one_of("+-")),
        take_while1(|c: char| c.is_ascii_digit()),
    ))
    .parse(input)
}

/// Parse string literal with escape sequences
fn string_literal(input: Span) -> PResult<Literal> {
    alt((
        delimited(
            char('"'),
            map(many0(string_char('"')), |chars| {
                Literal::String(chars.into_iter().collect())
            }),
            char('"'),
        ),
        delimited(
            char('\''),
            map(many0(string_char('\'')), |chars| {
                Literal::String(chars.into_iter().collect())
            }),
            char('\''),
        ),
    ))
    .parse(input)
}

/// Parse a single character in a string (with escapes)
fn string_char(quote: char) -> impl Fn(Span) -> PResult<char> {
    move |input: Span| {
        alt((
            // Escaped characters
            preceded(
                char('\\'),
                alt((
                    value('\n', char('n')),
                    value('\r', char('r')),
                    value('\t', char('t')),
                    value('\\', char('\\')),
                    value('"', char('"')),
                    value('\'', char('\'')),
                    value('\0', char('0')),
                    map(unicode_escape, |c| c),
                )),
            ),
            // Regular character (anything except quote and backslash)
            nom::character::complete::satisfy(move |c| c != quote && c != '\\'),
        ))
        .parse(input)
    }
}

/// Parse Unicode escape: \uXXXX or \u{XXXX}
fn unicode_escape(input: Span) -> PResult<char> {
    preceded(
        char('u'),
        alt((
            // \u{XXXX}
            delimited(
                char('{'),
                map(take_while1(|c: char| c.is_ascii_hexdigit()), |s: Span| {
                    let code = u32::from_str_radix(s.fragment(), 16).unwrap();
                    char::from_u32(code).unwrap_or('\u{FFFD}')
                }),
                char('}'),
            ),
            // \uXXXX (exactly 4 hex digits)
            map(
                recognize((
                    one_of("0123456789abcdefABCDEF"),
                    one_of("0123456789abcdefABCDEF"),
                    one_of("0123456789abcdefABCDEF"),
                    one_of("0123456789abcdefABCDEF"),
                )),
                |s: Span| {
                    let code = u32::from_str_radix(s.fragment(), 16).unwrap();
                    char::from_u32(code).unwrap_or('\u{FFFD}')
                },
            ),
        )),
    )
    .parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(s: &str) -> Span {
        Span::new(s)
    }

    #[test]
    fn test_null() {
        assert_eq!(null_literal(span("null")).unwrap().1, Literal::Null);
        assert_eq!(null_literal(span("NULL")).unwrap().1, Literal::Null);
    }

    #[test]
    fn test_boolean() {
        assert_eq!(
            boolean_literal(span("true")).unwrap().1,
            Literal::Boolean(true)
        );
        assert_eq!(
            boolean_literal(span("FALSE")).unwrap().1,
            Literal::Boolean(false)
        );
    }

    #[test]
    fn test_integer() {
        assert_eq!(
            integer_literal(span("123")).unwrap().1,
            Literal::Integer(123)
        );
        assert_eq!(
            integer_literal(span("-456")).unwrap().1,
            Literal::Integer(-456)
        );
    }

    #[test]
    fn test_float() {
        assert_eq!(
            float_literal(span("123.456")).unwrap().1,
            Literal::Float(123.456)
        );
        assert_eq!(
            float_literal(span("-1.5e10")).unwrap().1,
            Literal::Float(-1.5e10)
        );
        assert_eq!(float_literal(span(".5")).unwrap().1, Literal::Float(0.5));
    }

    #[test]
    fn test_string() {
        assert_eq!(
            string_literal(span("\"hello\"")).unwrap().1,
            Literal::String("hello".to_string())
        );
        assert_eq!(
            string_literal(span("'world'")).unwrap().1,
            Literal::String("world".to_string())
        );
    }

    #[test]
    fn test_string_escapes() {
        assert_eq!(
            string_literal(span(r#""line1\nline2""#)).unwrap().1,
            Literal::String("line1\nline2".to_string())
        );
        assert_eq!(
            string_literal(span(r#""quote: \"hi\"""#)).unwrap().1,
            Literal::String("quote: \"hi\"".to_string())
        );
    }
}
