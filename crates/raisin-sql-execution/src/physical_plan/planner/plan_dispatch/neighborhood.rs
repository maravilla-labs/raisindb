//! The argument grammar of `NEIGHBORS` in its set form.
//!
//! ```text
//! NEIGHBORS ( start, direction [, relation_type]
//!             [, depth => n] [, min_depth => n] [, types => 'A,B'] [, max_nodes => n] )
//! ```
//!
//! Without any named argument `NEIGHBORS` keeps its original meaning: one row
//! per RELATION of the node, with `relation_type` and `weight`. A named
//! argument selects the set form: one row per distinct NODE within `depth` hops,
//! with `depth` and `via_count` (see [`crate::physical_plan::graph_neighborhood`]).
//!
//! A named argument that nobody reads is the defect the table-function argument
//! work exists to close, so an unknown name is an error that lists the valid ones.

use raisin_sql::analyzer::{Expr, Literal, TableFunctionArg, TypedExpr};

use super::super::super::graph_neighborhood::{DEFAULT_MAX_NODES, MAX_DEPTH, MAX_MAX_NODES};
use super::super::Error;

/// The named arguments of the set form, in the order the error lists them.
pub(super) const NAMED: &[&str] = &["depth", "min_depth", "types", "max_nodes"];

/// A validated set-form call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NeighborhoodArgs {
    pub relation_types: Vec<String>,
    pub min_depth: u32,
    pub max_depth: u32,
    pub max_nodes: usize,
}

/// Whether any argument is named, which is what selects the set form.
pub(super) fn is_set_form(args: &[TableFunctionArg]) -> bool {
    args.iter().any(|a| a.name.is_some())
}

fn int_arg(expr: &TypedExpr, name: &str) -> Result<i64, Error> {
    match &expr.expr {
        Expr::Literal(Literal::Int(n)) => Ok(i64::from(*n)),
        Expr::Literal(Literal::BigInt(n)) => Ok(*n),
        Expr::Literal(Literal::Parameter(p)) => Err(Error::Validation(format!(
            "NEIGHBORS: parameter {p} for '{name}' was not bound."
        ))),
        other => Err(Error::Validation(format!(
            "NEIGHBORS: '{name}' must be an integer literal, got {other:?}."
        ))),
    }
}

fn types_arg(expr: &TypedExpr) -> Result<Vec<String>, Error> {
    match &expr.expr {
        Expr::Literal(Literal::Text(v)) | Expr::Literal(Literal::Path(v)) => {
            let list: Vec<String> = v
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect();
            if list.is_empty() {
                return Err(Error::Validation(
                    "NEIGHBORS: types => '' names no relation type. Omit it to follow every type."
                        .to_string(),
                ));
            }
            Ok(list)
        }
        Expr::Literal(Literal::Null) => Ok(vec![]),
        other => Err(Error::Validation(format!(
            "NEIGHBORS: 'types' must be a text literal such as 'CONNECTED' or \
             'CONNECTED,FOLLOWS', got {other:?}."
        ))),
    }
}

/// Parse the named arguments, and the positional relation type, of a set-form
/// call. `positional_type` is the third positional argument, already decoded.
pub(super) fn parse(
    args: &[TableFunctionArg],
    positional_type: Option<String>,
) -> Result<NeighborhoodArgs, Error> {
    let mut seen_named = false;
    for a in args {
        match &a.name {
            Some(_) => seen_named = true,
            None if seen_named => {
                return Err(Error::Validation(
                    "NEIGHBORS: positional arguments must come before named ones.".to_string(),
                ))
            }
            None => {}
        }
    }
    if args.iter().filter(|a| a.name.is_none()).count() > 3 {
        return Err(Error::Validation(
            "NEIGHBORS takes at most three positional arguments (start, direction, \
             relation_type); use named arguments for the rest."
                .to_string(),
        ));
    }

    let mut depth: Option<i64> = None;
    let mut min_depth: Option<i64> = None;
    let mut types: Option<Vec<String>> = None;
    let mut max_nodes: Option<i64> = None;

    for a in args.iter().filter(|a| a.name.is_some()) {
        let key = a.name.as_deref().unwrap_or_default();
        let lower = key.to_ascii_lowercase();
        let duplicate = |taken: bool| {
            if taken {
                Err(Error::Validation(format!(
                    "NEIGHBORS: argument '{lower}' was given twice."
                )))
            } else {
                Ok(())
            }
        };
        match lower.as_str() {
            "depth" => {
                duplicate(depth.is_some())?;
                depth = Some(int_arg(&a.value, "depth")?);
            }
            "min_depth" => {
                duplicate(min_depth.is_some())?;
                min_depth = Some(int_arg(&a.value, "min_depth")?);
            }
            "types" => {
                duplicate(types.is_some())?;
                types = Some(types_arg(&a.value)?);
            }
            "max_nodes" => {
                duplicate(max_nodes.is_some())?;
                max_nodes = Some(int_arg(&a.value, "max_nodes")?);
            }
            _ => {
                return Err(Error::Validation(format!(
                    "unknown argument '{key}' for NEIGHBORS. Valid: {}.",
                    NAMED.join(", ")
                )))
            }
        }
    }

    let depth = depth.unwrap_or(1);
    if !(1..=i64::from(MAX_DEPTH)).contains(&depth) {
        return Err(Error::Validation(format!(
            "NEIGHBORS: depth must be between 1 and {MAX_DEPTH}, got {depth}."
        )));
    }
    let min_depth = min_depth.unwrap_or(1);
    if min_depth < 1 || min_depth > depth {
        return Err(Error::Validation(format!(
            "NEIGHBORS: min_depth must be between 1 and depth ({depth}), got {min_depth}."
        )));
    }
    let max_nodes = max_nodes.unwrap_or(DEFAULT_MAX_NODES as i64);
    if max_nodes < 1 || max_nodes > MAX_MAX_NODES as i64 {
        return Err(Error::Validation(format!(
            "NEIGHBORS: max_nodes must be between 1 and {MAX_MAX_NODES}, got {max_nodes}."
        )));
    }

    let relation_types = match (types, positional_type) {
        (Some(named), None) => named,
        (None, Some(single)) => vec![single],
        (None, None) => vec![],
        (Some(named), Some(single)) => {
            if named.len() == 1 && named[0] == single {
                named
            } else {
                return Err(Error::Validation(
                    "NEIGHBORS: the relation type was given twice, positionally and as \
                     types =>. Use one."
                        .to_string(),
                ));
            }
        }
    };

    Ok(NeighborhoodArgs {
        relation_types,
        min_depth: min_depth as u32,
        max_depth: depth as u32,
        max_nodes: max_nodes as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_sql::analyzer::types::DataType;

    fn lit(l: Literal) -> TypedExpr {
        let ty = match &l {
            Literal::Text(_) => DataType::Text,
            _ => DataType::BigInt,
        };
        TypedExpr::new(Expr::Literal(l), ty)
    }
    fn named(n: &str, l: Literal) -> TableFunctionArg {
        TableFunctionArg::named(n, lit(l))
    }
    fn text(v: &str) -> TableFunctionArg {
        TableFunctionArg::positional(lit(Literal::Text(v.into())))
    }

    #[test]
    fn named_arguments_select_the_set_form() {
        assert!(!is_set_form(&[text("a"), text("OUT")]));
        assert!(is_set_form(&[
            text("a"),
            text("OUT"),
            named("depth", Literal::BigInt(2))
        ]));
    }

    #[test]
    fn defaults_are_one_hop_and_the_documented_budget() {
        let a = parse(&[named("types", Literal::Text("K".into()))], None).unwrap();
        assert_eq!(
            a,
            NeighborhoodArgs {
                relation_types: vec!["K".into()],
                min_depth: 1,
                max_depth: 1,
                max_nodes: DEFAULT_MAX_NODES
            }
        );
    }

    #[test]
    fn types_take_a_comma_list_and_the_positional_type_is_the_same_thing() {
        let a = parse(&[named("types", Literal::Text("A, B".into()))], None).unwrap();
        assert_eq!(a.relation_types, vec!["A".to_string(), "B".to_string()]);
        let a = parse(&[named("depth", Literal::BigInt(2))], Some("K".into())).unwrap();
        assert_eq!(a.relation_types, vec!["K".to_string()]);
        let err = parse(
            &[named("types", Literal::Text("A".into()))],
            Some("K".into()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("given twice"), "{err}");
    }

    #[test]
    fn depth_bounds_are_checked() {
        for bad in [0, -1, i64::from(MAX_DEPTH) + 1] {
            let err = parse(&[named("depth", Literal::BigInt(bad))], None).unwrap_err();
            assert!(err.to_string().contains("depth must be between"), "{err}");
        }
        let err = parse(
            &[
                named("depth", Literal::BigInt(2)),
                named("min_depth", Literal::BigInt(3)),
            ],
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("min_depth"), "{err}");
    }

    #[test]
    fn max_nodes_bounds_are_checked() {
        assert!(parse(&[named("max_nodes", Literal::BigInt(0))], None).is_err());
        assert!(parse(
            &[named(
                "max_nodes",
                Literal::BigInt(MAX_MAX_NODES as i64 + 1)
            )],
            None
        )
        .is_err());
        assert_eq!(
            parse(&[named("max_nodes", Literal::BigInt(10))], None)
                .unwrap()
                .max_nodes,
            10
        );
    }

    #[test]
    fn an_unknown_name_lists_the_valid_ones() {
        let err = parse(&[named("deep", Literal::BigInt(2))], None).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("unknown argument 'deep'"), "{text}");
        assert!(
            text.contains("depth, min_depth, types, max_nodes"),
            "{text}"
        );
    }

    #[test]
    fn a_duplicate_or_misordered_argument_is_an_error() {
        assert!(parse(
            &[
                named("depth", Literal::BigInt(2)),
                named("DEPTH", Literal::BigInt(3))
            ],
            None
        )
        .is_err());
        assert!(parse(&[named("depth", Literal::BigInt(2)), text("x")], None).is_err());
    }

    #[test]
    fn a_non_integer_depth_is_an_error_not_a_default() {
        let err = parse(&[named("depth", Literal::Text("2".into()))], None).unwrap_err();
        assert!(err.to_string().contains("integer literal"), "{err}");
    }
}
