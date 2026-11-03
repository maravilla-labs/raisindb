//! What a `GRAPH_TABLE` `WHERE` clause pins down before any relation is read.
//!
//! `MATCH (a)-[:knows]->(b) WHERE a.id = 'x'` names ONE start node. The matcher
//! used to scan every `knows` relation in the repository and let the filter
//! throw all but a handful away. [`SeekHints`] extracts the equalities that make
//! the start node known, so the matcher can read the per-node relation index
//! (O(degree)) instead of the global one (O(edges)).
//!
//! # Soundness
//!
//! A hint is only ever a NECESSARY condition: the `WHERE` clause is still
//! evaluated, unchanged, against every binding the narrowed read produces. A
//! hint that is too weak costs time; a hint that is wrong would drop rows. So
//! only shapes whose meaning is unambiguous are extracted:
//!
//! * top-level `AND` conjuncts (through parentheses), never anything under `OR`,
//!   `NOT` or a function;
//! * `var.id = 'text'`, `var.path = 'text'`, `var.workspace = 'text'`, in either
//!   operand order, and `var.id|path|workspace IN ('a', 'b', ...)`;
//! * text literals only. `a.id = 5` compares a string with an integer and is
//!   never true; it is left to the filter rather than guessed at here.
//!
//! Two conjuncts on the same property intersect: `a.id = 'x' AND a.id = 'y'`
//! pins an empty set, which is the right answer (no row can satisfy both).

use std::collections::HashMap;

use raisin_sql::ast::{BinaryOperator, Expr, Literal};

/// Values a variable's system property is restricted to. `None` = unrestricted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeSeek {
    /// Allowed node ids.
    pub ids: Option<Vec<String>>,
    /// Allowed node paths.
    pub paths: Option<Vec<String>>,
    /// Allowed workspaces.
    pub workspaces: Option<Vec<String>>,
}

impl NodeSeek {
    /// Whether the node can be located without scanning: an id or a path.
    pub fn is_seekable(&self) -> bool {
        self.ids.is_some() || self.paths.is_some()
    }

    fn restrict(slot: &mut Option<Vec<String>>, values: Vec<String>) {
        *slot = Some(match slot.take() {
            None => values,
            Some(prev) => prev.into_iter().filter(|v| values.contains(v)).collect(),
        });
    }
}

/// Per-variable seeks extracted from one `WHERE` clause.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeekHints {
    vars: HashMap<String, NodeSeek>,
}

impl SeekHints {
    /// No hints: every pattern reads the global index, exactly as before.
    pub fn none() -> Self {
        Self::default()
    }

    /// Extract the hints a `WHERE` expression provides.
    pub fn from_where(expr: Option<&Expr>) -> Self {
        let mut hints = Self::default();
        if let Some(expr) = expr {
            hints.collect(expr);
        }
        hints
    }

    /// The seek for a pattern variable, when it has a locatable one.
    pub fn seek_for(&self, variable: Option<&String>) -> Option<&NodeSeek> {
        let seek = self.vars.get(variable?)?;
        seek.is_seekable().then_some(seek)
    }

    /// The restriction (possibly workspace-only) recorded for a variable.
    pub fn raw(&self, variable: &str) -> Option<&NodeSeek> {
        self.vars.get(variable)
    }

    fn collect(&mut self, expr: &Expr) {
        match expr {
            Expr::Nested(inner) => self.collect(inner),
            Expr::BinaryOp {
                left,
                op: BinaryOperator::And,
                right,
                ..
            } => {
                self.collect(left);
                self.collect(right);
            }
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Eq,
                right,
                ..
            } => {
                if let Some((var, prop, value)) =
                    eq_parts(left, right).or_else(|| eq_parts(right, left))
                {
                    self.pin(var, prop, vec![value]);
                }
            }
            Expr::InList {
                expr,
                list,
                negated: false,
                ..
            } => {
                let Some((var, prop)) = system_property(expr) else {
                    return;
                };
                let mut values = Vec::with_capacity(list.len());
                for item in list {
                    match text_literal(item) {
                        Some(v) => values.push(v),
                        // A non-text item (or an expression) could match
                        // something this cannot predict: no hint at all.
                        None => return,
                    }
                }
                self.pin(var, prop, values);
            }
            _ => {}
        }
    }

    fn pin(&mut self, var: &str, prop: SystemProp, values: Vec<String>) {
        let seek = self.vars.entry(var.to_string()).or_default();
        match prop {
            SystemProp::Id => NodeSeek::restrict(&mut seek.ids, values),
            SystemProp::Path => NodeSeek::restrict(&mut seek.paths, values),
            SystemProp::Workspace => NodeSeek::restrict(&mut seek.workspaces, values),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum SystemProp {
    Id,
    Path,
    Workspace,
}

fn system_property(expr: &Expr) -> Option<(&str, SystemProp)> {
    match expr {
        Expr::Nested(inner) => system_property(inner),
        Expr::PropertyAccess {
            variable,
            properties,
            ..
        } if properties.len() == 1 => {
            let prop = match properties[0].as_str() {
                "id" => SystemProp::Id,
                "path" => SystemProp::Path,
                "workspace" => SystemProp::Workspace,
                _ => return None,
            };
            Some((variable.as_str(), prop))
        }
        _ => None,
    }
}

fn text_literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Nested(inner) => text_literal(inner),
        Expr::Literal(Literal::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn eq_parts<'a>(prop_side: &'a Expr, value_side: &Expr) -> Option<(&'a str, SystemProp, String)> {
    let (var, prop) = system_property(prop_side)?;
    Some((var, prop, text_literal(value_side)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_sql::ast::pgq_parser::parse_graph_table;

    fn hints(where_sql: &str) -> SeekHints {
        let q = parse_graph_table(&format!(
            "GRAPH_TABLE(MATCH (a)-[:knows]->(b) WHERE {where_sql} COLUMNS (a.id AS x))"
        ))
        .expect("parse");
        SeekHints::from_where(q.where_clause.as_ref().map(|w| &w.expression))
    }

    fn ids(h: &SeekHints, v: &str) -> Option<Vec<String>> {
        h.raw(v).and_then(|s| s.ids.clone())
    }

    #[test]
    fn an_id_equality_is_a_seek() {
        let h = hints("a.id = 'x'");
        assert_eq!(ids(&h, "a"), Some(vec!["x".into()]));
        assert!(h.seek_for(Some(&"a".to_string())).is_some());
        assert!(h.seek_for(Some(&"b".to_string())).is_none());
    }

    #[test]
    fn operand_order_does_not_matter() {
        assert_eq!(ids(&hints("'x' = a.id"), "a"), Some(vec!["x".into()]));
    }

    #[test]
    fn conjuncts_are_found_through_and_and_parentheses() {
        let h = hints("(a.id = 'x' AND b.name = 'n') AND b.id = 'y'");
        assert_eq!(ids(&h, "a"), Some(vec!["x".into()]));
        assert_eq!(ids(&h, "b"), Some(vec!["y".into()]));
    }

    #[test]
    fn nothing_under_or_is_a_hint() {
        let h = hints("a.id = 'x' OR a.id = 'y'");
        assert!(h.raw("a").is_none());
        let h = hints("a.id = 'x' AND (b.id = 'p' OR b.id = 'q')");
        assert!(h.raw("b").is_none());
        assert_eq!(ids(&h, "a"), Some(vec!["x".into()]));
    }

    #[test]
    fn negation_and_inequality_are_not_hints() {
        assert!(hints("NOT a.id = 'x'").raw("a").is_none());
        assert!(hints("a.id <> 'x'").raw("a").is_none());
    }

    #[test]
    fn a_non_text_literal_is_left_to_the_filter() {
        assert!(hints("a.id = 5").raw("a").is_none());
    }

    #[test]
    fn other_properties_are_not_hints() {
        assert!(hints("a.name = 'x'").raw("a").is_none());
        assert!(hints("a.properties.id = 'x'").raw("a").is_none());
    }

    fn in_list(items: Vec<Expr>, negated: bool) -> Expr {
        Expr::InList {
            expr: Box::new(Expr::PropertyAccess {
                variable: "a".into(),
                properties: vec!["id".into()],
                span: Default::default(),
            }),
            list: items,
            negated,
            span: Default::default(),
        }
    }

    fn text(v: &str) -> Expr {
        Expr::Literal(Literal::String(v.into()))
    }

    // The PGQ grammar has no IN yet; the evaluator does, and an AST built
    // another way (or a future grammar) must not silently lose the hint.
    #[test]
    fn in_list_pins_a_set() {
        let h = SeekHints::from_where(Some(&in_list(vec![text("x"), text("y")], false)));
        assert_eq!(ids(&h, "a"), Some(vec!["x".into(), "y".into()]));
    }

    #[test]
    fn in_list_with_a_non_literal_item_or_negated_is_not_a_hint() {
        let item = Expr::PropertyAccess {
            variable: "b".into(),
            properties: vec!["id".into()],
            span: Default::default(),
        };
        let h = SeekHints::from_where(Some(&in_list(vec![text("x"), item], false)));
        assert!(h.raw("a").is_none());
        let h = SeekHints::from_where(Some(&in_list(vec![text("x")], true)));
        assert!(h.raw("a").is_none());
    }

    #[test]
    fn two_equalities_intersect() {
        let h = hints("a.id = 'x' AND a.id = 'y'");
        assert_eq!(ids(&h, "a"), Some(vec![]));
        let both = Expr::BinaryOp {
            left: Box::new(in_list(vec![text("x"), text("y")], false)),
            op: BinaryOperator::And,
            right: Box::new(Expr::BinaryOp {
                left: Box::new(Expr::PropertyAccess {
                    variable: "a".into(),
                    properties: vec!["id".into()],
                    span: Default::default(),
                }),
                op: BinaryOperator::Eq,
                right: Box::new(text("y")),
                span: Default::default(),
            }),
            span: Default::default(),
        };
        let h = SeekHints::from_where(Some(&both));
        assert_eq!(ids(&h, "a"), Some(vec!["y".into()]));
    }

    #[test]
    fn a_path_is_a_seek_and_a_workspace_alone_is_not() {
        let h = hints("a.path = '/people/ann'");
        assert!(h.seek_for(Some(&"a".to_string())).is_some());
        let h = hints("a.workspace = 'people'");
        assert!(h.seek_for(Some(&"a".to_string())).is_none());
        assert!(h.raw("a").is_some());
    }
}
