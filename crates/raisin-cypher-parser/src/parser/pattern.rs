// SPDX-License-Identifier: BSL-1.1

//! Graph pattern parsing: nodes, relationships, paths

use super::common::{any_identifier, comma_sep0, keyword, ws0, ws_token, PResult, Span};
use super::expr::expr;
use crate::ast::{
    Direction, Expr, GraphPattern, NodePattern, PathPattern, PatternElement, Range, RelPattern,
};
use nom::{
    bytes::complete::tag,
    character::complete::char,
    combinator::opt,
    sequence::{delimited, pair, preceded, separated_pair},
    Parser,
};

/// Parse a complete graph pattern with optional WHERE
pub fn graph_pattern(input: Span) -> PResult<GraphPattern> {
    let (input, patterns) = comma_sep0(path_pattern).parse(input)?;

    Ok((input, GraphPattern::new(patterns)))
}

/// Parse a single path pattern: [p =] (a)-[:REL]->(b)
pub fn path_pattern(input: Span) -> PResult<PathPattern> {
    // Optional path variable
    let (input, variable) = opt(pair(any_identifier, ws_token(char('=')))).parse(input)?;

    let variable = variable.map(|(name, _)| name);

    // Parse the actual path
    let (input, elements) = path_elements(input)?;

    Ok((input, PathPattern { variable, elements }))
}

/// Parse path elements: alternating nodes and relationships
fn path_elements(input: Span) -> PResult<Vec<PatternElement>> {
    let (input, first_node) = node_pattern(input)?;
    let (input, _) = ws0(input)?;

    let mut elements = vec![PatternElement::Node(first_node)];
    let mut current = input;

    // Parse relationship-node pairs
    while let Ok((rest, rel)) = rel_pattern(current) {
        let (rest, _) = ws0(rest)?;
        let (rest, node) = node_pattern(rest)?;
        let (rest, _) = ws0(rest)?;

        elements.push(PatternElement::Relationship(rel));
        elements.push(PatternElement::Node(node));
        current = rest;
    }

    Ok((current, elements))
}

/// Parse a node pattern: (variable:Label {properties})
pub fn node_pattern(input: Span) -> PResult<NodePattern> {
    let (input, _) = ws_token(char('(')).parse(input)?;

    // Optional variable
    let (input, variable) = opt(any_identifier).parse(input)?;
    let (input, _) = ws0(input)?;

    // Optional labels (can have multiple)
    let (input, labels) = nom::multi::many0(preceded(char(':'), any_identifier)).parse(input)?;
    let (input, _) = ws0(input)?;

    // Optional properties
    let (input, properties) = opt(properties_map).parse(input)?;
    let (input, _) = ws0(input)?;

    // Optional WHERE clause (inline)
    let (input, where_clause) = opt(preceded(ws_token(keyword("WHERE")), expr)).parse(input)?;
    let (input, _) = ws0(input)?;

    let (input, _) = char(')').parse(input)?;

    Ok((
        input,
        NodePattern {
            variable,
            labels,
            properties,
            where_clause,
        },
    ))
}

/// Parse properties map: {key: value, ...}
fn properties_map(input: Span) -> PResult<Vec<(String, Expr)>> {
    delimited(
        ws_token(char('{')),
        comma_sep0(separated_pair(any_identifier, ws_token(char(':')), expr)),
        ws_token(char('}')),
    )
    .parse(input)
}

/// Parse a relationship pattern: -[:TYPE {props}]->
pub fn rel_pattern(input: Span) -> PResult<RelPattern> {
    // Parse left arrow (optional)
    let (input, left_arrow) = opt(ws_token(char('<'))).parse(input)?;
    let (input, _) = ws_token(char('-')).parse(input)?;

    // Optional bracket section with details
    let (input, details) = opt(delimited(char('['), rel_details, char(']'))).parse(input)?;

    let (input, _) = ws_token(char('-')).parse(input)?;
    let (input, right_arrow) = opt(ws_token(char('>'))).parse(input)?;

    // Determine direction
    let direction = match (left_arrow.is_some(), right_arrow.is_some()) {
        (true, true) => Direction::Both,
        (true, false) => Direction::Left,
        (false, true) => Direction::Right,
        (false, false) => Direction::None,
    };

    // Extract details or use defaults
    let (variable, types, properties, range, where_clause) = match details {
        Some((v, t, p, r, w)) => (v, t, p, r, w),
        None => (None, Vec::new(), None, None, None),
    };

    Ok((
        input,
        RelPattern {
            variable,
            types,
            properties,
            direction,
            range,
            where_clause,
        },
    ))
}

/// Parsed relationship details: (variable, types, properties, range, where_clause)
type RelDetails = (
    Option<String>,
    Vec<String>,
    Option<Vec<(String, Expr)>>,
    Option<Range>,
    Option<Expr>,
);

/// Parse relationship details inside brackets: [variable:TYPE*1..5 {props} WHERE ...]
fn rel_details(input: Span) -> PResult<RelDetails> {
    let (input, _) = ws0(input)?;

    // Optional variable
    let (input, variable) = opt(any_identifier).parse(input)?;
    let (input, _) = ws0(input)?;

    // Optional types (can have multiple with |)
    let (input, types) = if variable.is_some() || opt(char(':')).parse(input)?.1.is_some() {
        let (input, first_type) = opt(preceded(opt(char(':')), any_identifier)).parse(input)?;

        if let Some(first) = first_type {
            let (input, rest) =
                nom::multi::many0(preceded(ws_token(char('|')), any_identifier)).parse(input)?;

            let mut all_types = vec![first];
            all_types.extend(rest);
            (input, all_types)
        } else {
            (input, Vec::new())
        }
    } else {
        (input, Vec::new())
    };

    let (input, _) = ws0(input)?;

    // Optional range: *1..5, *1.., *.., *
    let (input, range) = opt(range_spec).parse(input)?;
    let (input, _) = ws0(input)?;

    // Optional properties
    let (input, properties) = opt(properties_map).parse(input)?;
    let (input, _) = ws0(input)?;

    // Optional WHERE clause
    let (input, where_clause) = opt(preceded(ws_token(keyword("WHERE")), expr)).parse(input)?;
    let (input, _) = ws0(input)?;

    Ok((input, (variable, types, properties, range, where_clause)))
}

/// Parse range specification: *1..5, *1.., *.., *, *..5
fn range_spec(input: Span) -> PResult<Range> {
    let (input, _) = char('*').parse(input)?;

    // Try to parse min
    let (input, min) = opt(nom::character::complete::u32).parse(input)?;

    // Try to parse ..
    let (input, has_dots) = opt(tag("..")).parse(input)?;

    if has_dots.is_some() {
        // Parse max
        let (input, max) = opt(nom::character::complete::u32).parse(input)?;
        Ok((input, Range { min, max }))
    } else if min.is_some() {
        // Just a single number: *5 means exactly 5
        Ok((input, Range { min, max: min }))
    } else {
        // Just * means unbounded
        Ok((input, Range::unbounded()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(s: &str) -> Span {
        Span::new(s)
    }

    #[test]
    fn test_simple_node() {
        let result = node_pattern(span("(n)")).unwrap();
        assert_eq!(result.1.variable, Some("n".to_string()));
        assert!(result.1.labels.is_empty());
    }

    #[test]
    fn test_node_with_label() {
        let result = node_pattern(span("(n:Person)")).unwrap();
        assert_eq!(result.1.variable, Some("n".to_string()));
        assert_eq!(result.1.labels, vec!["Person"]);
    }

    #[test]
    fn test_node_with_multiple_labels() {
        let result = node_pattern(span("(n:Person:Employee)")).unwrap();
        assert_eq!(result.1.labels, vec!["Person", "Employee"]);
    }

    #[test]
    fn test_node_with_properties() {
        let result = node_pattern(span("(n {name: 'Alice'})")).unwrap();
        assert!(result.1.properties.is_some());
    }

    #[test]
    fn test_simple_relationship() {
        let result = rel_pattern(span("-[:KNOWS]->")).unwrap();
        assert_eq!(result.1.direction, Direction::Right);
        assert_eq!(result.1.types, vec!["KNOWS"]);
    }

    #[test]
    fn test_relationship_left() {
        let result = rel_pattern(span("<-[:KNOWS]-")).unwrap();
        assert_eq!(result.1.direction, Direction::Left);
    }

    #[test]
    fn test_relationship_both() {
        let result = rel_pattern(span("<-[:KNOWS]->")).unwrap();
        assert_eq!(result.1.direction, Direction::Both);
    }

    #[test]
    fn test_relationship_undirected() {
        let result = rel_pattern(span("-[:KNOWS]-")).unwrap();
        assert_eq!(result.1.direction, Direction::None);
    }

    #[test]
    fn test_relationship_with_variable() {
        let result = rel_pattern(span("-[r:KNOWS]->")).unwrap();
        assert_eq!(result.1.variable, Some("r".to_string()));
    }

    #[test]
    fn test_variable_length() {
        let result = rel_pattern(span("-[:KNOWS*1..5]->")).unwrap();
        assert_eq!(result.1.range, Some(Range::bounded(1, 5)));
    }

    #[test]
    fn test_path_pattern() {
        let result = path_pattern(span("(a)-[:KNOWS]->(b)")).unwrap();
        assert_eq!(result.1.elements.len(), 3); // node, rel, node
    }

    #[test]
    fn test_path_with_variable() {
        let result = path_pattern(span("p = (a)-[:KNOWS]->(b)")).unwrap();
        assert_eq!(result.1.variable, Some("p".to_string()));
    }
}
