// SPDX-License-Identifier: BSL-1.1

//! Statement and clause parsing: MATCH, CREATE, WHERE, RETURN, etc.

use super::common::{any_identifier, comma_sep1, keyword, ws0, ws_token, PResult, Span};
use super::expr::expr;
use super::pattern::graph_pattern;
use crate::ast::{Clause, Order, OrderBy, Query, RemoveItem, ReturnItem, SetItem, Statement};
use nom::{
    branch::alt,
    character::complete::char,
    combinator::{map, opt},
    multi::many1,
    sequence::{pair, preceded},
    Parser,
};

/// Parse a complete query
pub fn query(input: Span) -> PResult<Query> {
    let (input, _) = ws0(input)?;
    let (input, clauses) = many1(clause).parse(input)?;
    let (input, _) = ws0(input)?;

    Ok((input, Query::new(clauses)))
}

/// Parse a statement
pub fn statement(input: Span) -> PResult<Statement> {
    map(query, Statement::Query).parse(input)
}

/// Parse any clause
fn clause(input: Span) -> PResult<Clause> {
    let (input, _) = ws0(input)?;
    alt((
        match_clause,
        create_clause,
        merge_clause,
        delete_clause,
        set_clause,
        remove_clause,
        unwind_clause,
        with_clause,
        return_clause,
        where_clause,
    ))
    .parse(input)
}

/// Parse MATCH or OPTIONAL MATCH clause
fn match_clause(input: Span) -> PResult<Clause> {
    let (input, optional) = opt(ws_token(keyword("OPTIONAL"))).parse(input)?;
    let (input, _) = ws_token(keyword("MATCH")).parse(input)?;
    let (input, pattern) = graph_pattern(input)?;

    Ok((
        input,
        Clause::Match {
            optional: optional.is_some(),
            pattern,
        },
    ))
}

/// Parse CREATE clause
fn create_clause(input: Span) -> PResult<Clause> {
    let (input, _) = ws_token(keyword("CREATE")).parse(input)?;
    let (input, pattern) = graph_pattern(input)?;

    Ok((input, Clause::Create { pattern }))
}

/// Parse MERGE clause
fn merge_clause(input: Span) -> PResult<Clause> {
    let (input, _) = ws_token(keyword("MERGE")).parse(input)?;
    let (input, pattern) = graph_pattern(input)?;

    Ok((input, Clause::Merge { pattern }))
}

/// Parse WHERE clause (standalone, not part of pattern)
fn where_clause(input: Span) -> PResult<Clause> {
    let (input, _) = ws_token(keyword("WHERE")).parse(input)?;
    let (input, condition) = expr(input)?;

    Ok((input, Clause::Where { condition }))
}

/// Parse RETURN clause
fn return_clause(input: Span) -> PResult<Clause> {
    let (input, _) = ws_token(keyword("RETURN")).parse(input)?;

    // Optional DISTINCT
    let (input, distinct) = opt(ws_token(keyword("DISTINCT"))).parse(input)?;
    let distinct = distinct.is_some();

    // Return items
    let (input, items) = comma_sep1(return_item).parse(input)?;

    // Optional ORDER BY
    let (input, order_by) = opt(order_by_clause).parse(input)?;
    let order_by = order_by.unwrap_or_default();

    // Optional SKIP
    let (input, skip) = opt(preceded(ws_token(keyword("SKIP")), expr)).parse(input)?;

    // Optional LIMIT
    let (input, limit) = opt(preceded(ws_token(keyword("LIMIT")), expr)).parse(input)?;

    Ok((
        input,
        Clause::Return {
            distinct,
            items,
            order_by,
            skip,
            limit,
        },
    ))
}

/// Parse WITH clause (similar to RETURN but can have WHERE)
fn with_clause(input: Span) -> PResult<Clause> {
    let (input, _) = ws_token(keyword("WITH")).parse(input)?;

    // Optional DISTINCT
    let (input, distinct) = opt(ws_token(keyword("DISTINCT"))).parse(input)?;
    let distinct = distinct.is_some();

    // Items
    let (input, items) = comma_sep1(return_item).parse(input)?;

    // Optional ORDER BY
    let (input, order_by) = opt(order_by_clause).parse(input)?;
    let order_by = order_by.unwrap_or_default();

    // Optional SKIP
    let (input, skip) = opt(preceded(ws_token(keyword("SKIP")), expr)).parse(input)?;

    // Optional LIMIT
    let (input, limit) = opt(preceded(ws_token(keyword("LIMIT")), expr)).parse(input)?;

    // Optional WHERE (specific to WITH)
    let (input, where_clause) = opt(preceded(ws_token(keyword("WHERE")), expr)).parse(input)?;

    Ok((
        input,
        Clause::With {
            distinct,
            items,
            order_by,
            skip,
            limit,
            where_clause,
        },
    ))
}

/// Parse return item: expr [AS alias]
fn return_item(input: Span) -> PResult<ReturnItem> {
    let (input, e) = expr(input)?;
    let (input, alias) = opt(preceded(ws_token(keyword("AS")), any_identifier)).parse(input)?;

    Ok((input, ReturnItem { expr: e, alias }))
}

/// Parse ORDER BY clause
fn order_by_clause(input: Span) -> PResult<Vec<OrderBy>> {
    let (input, _) = ws_token(keyword("ORDER")).parse(input)?;
    let (input, _) = ws_token(keyword("BY")).parse(input)?;
    comma_sep1(order_by_item).parse(input)
}

/// Parse single ORDER BY item: expr [ASC|DESC]
fn order_by_item(input: Span) -> PResult<OrderBy> {
    let (input, e) = expr(input)?;
    let (input, order) = opt(alt((
        map(ws_token(keyword("ASC")), |_| Order::Asc),
        map(ws_token(keyword("DESC")), |_| Order::Desc),
    )))
    .parse(input)?;

    Ok((
        input,
        OrderBy {
            expr: e,
            order: order.unwrap_or(Order::Asc),
        },
    ))
}

/// Parse DELETE clause
fn delete_clause(input: Span) -> PResult<Clause> {
    let (input, detach) = opt(ws_token(keyword("DETACH"))).parse(input)?;
    let (input, _) = ws_token(keyword("DELETE")).parse(input)?;
    let (input, items) = comma_sep1(expr).parse(input)?;

    Ok((
        input,
        Clause::Delete {
            detach: detach.is_some(),
            items,
        },
    ))
}

/// Parse SET clause
fn set_clause(input: Span) -> PResult<Clause> {
    let (input, _) = ws_token(keyword("SET")).parse(input)?;
    let (input, items) = comma_sep1(set_item).parse(input)?;

    Ok((input, Clause::Set { items }))
}

/// Parse SET item
fn set_item(input: Span) -> PResult<SetItem> {
    alt((
        // var.prop = value
        map(
            (
                any_identifier,
                ws_token(char('.')),
                any_identifier,
                ws_token(char('=')),
                expr,
            ),
            |(variable, _, property, _, value)| SetItem::Property {
                variable,
                property,
                value,
            },
        ),
        // var += {props}
        map(
            (
                any_identifier,
                ws_token(nom::bytes::complete::tag("+=")),
                expr,
            ),
            |(variable, _, properties)| SetItem::AddProperties {
                variable,
                properties,
            },
        ),
        // var:Label or var:Label1:Label2
        map(
            pair(any_identifier, many1(preceded(char(':'), any_identifier))),
            |(variable, labels)| SetItem::Labels { variable, labels },
        ),
        // var = value
        map(
            (any_identifier, ws_token(char('=')), expr),
            |(variable, _, value)| SetItem::Variable { variable, value },
        ),
    ))
    .parse(input)
}

/// Parse REMOVE clause
fn remove_clause(input: Span) -> PResult<Clause> {
    let (input, _) = ws_token(keyword("REMOVE")).parse(input)?;
    let (input, items) = comma_sep1(remove_item).parse(input)?;

    Ok((input, Clause::Remove { items }))
}

/// Parse REMOVE item
fn remove_item(input: Span) -> PResult<RemoveItem> {
    alt((
        // var.prop
        map(
            (any_identifier, ws_token(char('.')), any_identifier),
            |(variable, _, property)| RemoveItem::Property { variable, property },
        ),
        // var:Label or var:Label1:Label2
        map(
            pair(any_identifier, many1(preceded(char(':'), any_identifier))),
            |(variable, labels)| RemoveItem::Labels { variable, labels },
        ),
    ))
    .parse(input)
}

/// Parse UNWIND clause
fn unwind_clause(input: Span) -> PResult<Clause> {
    let (input, _) = ws_token(keyword("UNWIND")).parse(input)?;
    let (input, e) = expr(input)?;
    let (input, _) = ws_token(keyword("AS")).parse(input)?;
    let (input, alias) = any_identifier(input)?;

    Ok((input, Clause::Unwind { expr: e, alias }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(s: &str) -> Span {
        Span::new(s)
    }

    #[test]
    fn test_match_clause() {
        let result = match_clause(span("MATCH (n)")).unwrap();
        match result.1 {
            Clause::Match { optional, .. } => {
                assert!(!optional);
            }
            _ => panic!("Expected MATCH clause"),
        }
    }

    #[test]
    fn test_optional_match() {
        let result = match_clause(span("OPTIONAL MATCH (n)")).unwrap();
        match result.1 {
            Clause::Match { optional, .. } => {
                assert!(optional);
            }
            _ => panic!("Expected OPTIONAL MATCH clause"),
        }
    }

    #[test]
    fn test_return_clause() {
        let result = return_clause(span("RETURN n.name")).unwrap();
        match result.1 {
            Clause::Return { items, .. } => {
                assert_eq!(items.len(), 1);
            }
            _ => panic!("Expected RETURN clause"),
        }
    }

    #[test]
    fn test_return_with_alias() {
        let result = return_clause(span("RETURN n.name AS name")).unwrap();
        match result.1 {
            Clause::Return { items, .. } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].alias, Some("name".to_string()));
            }
            _ => panic!("Expected RETURN clause"),
        }
    }

    #[test]
    fn test_return_distinct() {
        let result = return_clause(span("RETURN DISTINCT n")).unwrap();
        match result.1 {
            Clause::Return { distinct, .. } => {
                assert!(distinct);
            }
            _ => panic!("Expected RETURN DISTINCT"),
        }
    }

    #[test]
    fn test_order_by() {
        let result = return_clause(span("RETURN n ORDER BY n.name DESC")).unwrap();
        match result.1 {
            Clause::Return { order_by, .. } => {
                assert_eq!(order_by.len(), 1);
                assert_eq!(order_by[0].order, Order::Desc);
            }
            _ => panic!("Expected ORDER BY"),
        }
    }

    #[test]
    fn test_limit_skip() {
        let result = return_clause(span("RETURN n SKIP 10 LIMIT 5")).unwrap();
        match result.1 {
            Clause::Return { skip, limit, .. } => {
                assert!(skip.is_some());
                assert!(limit.is_some());
            }
            _ => panic!("Expected SKIP/LIMIT"),
        }
    }

    #[test]
    fn test_create_clause() {
        let result = create_clause(span("CREATE (n:Person)")).unwrap();
        assert!(matches!(result.1, Clause::Create { .. }));
    }

    #[test]
    fn test_where_clause() {
        let result = where_clause(span("WHERE n.age > 18")).unwrap();
        assert!(matches!(result.1, Clause::Where { .. }));
    }

    #[test]
    fn test_complete_query() {
        let result = query(span("MATCH (n:Person) WHERE n.age > 18 RETURN n.name")).unwrap();
        assert_eq!(result.1.clauses.len(), 3);
    }
}
