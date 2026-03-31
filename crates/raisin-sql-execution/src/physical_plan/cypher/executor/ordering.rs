//! Cypher ORDER BY and property value comparison
//!
//! Provides sorting for Cypher result rows and helper functions for
//! attaching WHERE predicates to MATCH clauses.

use raisin_cypher_parser::{Clause, Expr};
use raisin_models::nodes::properties::PropertyValue;
use std::cmp::Ordering;

use super::CypherRow;
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Apply ORDER BY to Cypher result rows
pub(super) fn apply_order_by(
    rows: &mut [CypherRow],
    order_by: &[raisin_cypher_parser::OrderBy],
    return_items: &[raisin_cypher_parser::ReturnItem],
) -> Result<()> {
    use raisin_cypher_parser::Order;

    rows.sort_by(|a, b| {
        for order_item in order_by {
            let col_name = match &order_item.expr {
                Expr::Variable(name) => name.clone(),
                Expr::Property { expr, property } => {
                    if let Expr::Variable(var) = expr.as_ref() {
                        format!("{}_{}", var, property)
                    } else {
                        property.clone()
                    }
                }
                Expr::FunctionCall { name, args, .. } => {
                    if let Some(item) = return_items.iter().find(|item| {
                        matches!(&item.expr, Expr::FunctionCall { name: n, args: a, .. } if n == name && a.len() == args.len())
                    }) {
                        if let Some(alias) = &item.alias {
                            alias.clone()
                        } else {
                            format!("{}_{}", name, args.len())
                        }
                    } else {
                        format!("{}_{}", name, args.len())
                    }
                }
                _ => continue,
            };

            let a_idx = a.columns.iter().position(|c| c == &col_name);
            let b_idx = b.columns.iter().position(|c| c == &col_name);

            if let (Some(a_idx), Some(b_idx)) = (a_idx, b_idx) {
                let a_val = &a.values[a_idx];
                let b_val = &b.values[b_idx];

                let cmp = compare_property_values(a_val, b_val);
                let cmp = match order_item.order {
                    Order::Desc => cmp.reverse(),
                    Order::Asc => cmp,
                };

                if cmp != Ordering::Equal {
                    return cmp;
                }
            }
        }
        Ordering::Equal
    });

    Ok(())
}

/// Attach WHERE predicates to their preceding MATCH clause
///
/// Cypher grammar allows WHERE to follow MATCH as a separate clause.
/// This function merges them so the match executor can use WHERE for
/// early filtering.
pub(super) fn attach_match_where_predicates(clauses: &mut [Clause]) {
    let mut last_match_index: Option<usize> = None;

    for idx in 0..clauses.len() {
        match clauses[idx].clone() {
            Clause::Match { .. } => {
                last_match_index = Some(idx);
            }
            Clause::Where { condition } => {
                if let Some(match_idx) = last_match_index.take() {
                    if let Clause::Match { pattern, .. } = &mut clauses[match_idx] {
                        if pattern.where_clause.is_none() {
                            pattern.where_clause = Some(condition);
                        }
                    }
                }
            }
            _ => {
                last_match_index = None;
            }
        }
    }
}

/// Compare two PropertyValue values for ordering
pub(super) fn compare_property_values(a: &PropertyValue, b: &PropertyValue) -> Ordering {
    match (a, b) {
        (PropertyValue::Null, PropertyValue::Null) => Ordering::Equal,
        (PropertyValue::Integer(a), PropertyValue::Integer(b)) => a.cmp(b),
        (PropertyValue::Float(a), PropertyValue::Float(b)) => {
            a.partial_cmp(b).unwrap_or(Ordering::Equal)
        }
        (PropertyValue::Integer(a), PropertyValue::Float(b)) => {
            (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal)
        }
        (PropertyValue::Float(a), PropertyValue::Integer(b)) => {
            a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal)
        }
        (PropertyValue::Decimal(a), PropertyValue::Decimal(b)) => a.cmp(b),
        (PropertyValue::String(a), PropertyValue::String(b)) => a.cmp(b),
        (PropertyValue::Boolean(a), PropertyValue::Boolean(b)) => a.cmp(b),
        (PropertyValue::Date(a), PropertyValue::Date(b)) => a.cmp(b),
        _ => {
            fn type_order(v: &PropertyValue) -> u8 {
                match v {
                    PropertyValue::Null => 0,
                    PropertyValue::Boolean(_) => 1,
                    PropertyValue::Integer(_) => 2,
                    PropertyValue::Float(_) => 3,
                    PropertyValue::Decimal(_) => 4,
                    PropertyValue::Date(_) => 5,
                    PropertyValue::String(_) => 6,
                    PropertyValue::Reference(_) => 7,
                    PropertyValue::Url(_) => 8,
                    PropertyValue::Resource(_) => 9,
                    PropertyValue::Composite(_) => 10,
                    PropertyValue::Element(_) => 11,
                    PropertyValue::Vector(_) => 12,
                    PropertyValue::Geometry(_) => 13,
                    PropertyValue::Array(_) => 14,
                    PropertyValue::Object(_) => 15,
                }
            }
            type_order(a).cmp(&type_order(b))
        }
    }
}
