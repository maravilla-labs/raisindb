//! PGQ function call evaluation (e.g., CARDINALITY).

use raisin_sql::ast::Expr;

use super::Result;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::pgq::types::{SqlValue, VariableBinding};

/// Evaluate a function call expression
///
/// Handles PGQ-specific functions like CARDINALITY for path length.
pub(super) fn evaluate_function(
    name: &str,
    args: &[Expr],
    binding: &VariableBinding,
) -> Result<SqlValue> {
    let name_lower = name.to_lowercase();
    match name_lower.as_str() {
        "cardinality" => evaluate_cardinality(args, binding),
        _ => Err(ExecutionError::Validation(format!(
            "Unsupported function in PGQ expression: {}",
            name
        ))),
    }
}

/// `CARDINALITY(r)` - hop count of the path bound to `r`.
///
/// # Migration note
///
/// This used to parse the hop count back out of a mangled relation type: a
/// variable-length match rewrote `relation_type` to `"knows[3]"` purely so this
/// function could read the `3`. The real path is now bound under the same
/// variable, so the count is read directly and `relation_type` stays verbatim.
/// A single-hop relationship has no bound path and is still cardinality 1.
fn evaluate_cardinality(args: &[Expr], binding: &VariableBinding) -> Result<SqlValue> {
    if args.len() != 1 {
        return Err(ExecutionError::Validation(
            "CARDINALITY requires exactly one argument".into(),
        ));
    }

    let var_name = get_variable_name(&args[0]).ok_or_else(|| {
        ExecutionError::Validation(
            "CARDINALITY requires a relationship or path variable as argument".into(),
        )
    })?;

    if let Some(path) = binding.get_path(&var_name) {
        return Ok(SqlValue::Integer(path.length() as i64));
    }

    if binding.get_relation(&var_name).is_some() {
        // Single-hop relationship: exactly one edge.
        return Ok(SqlValue::Integer(1));
    }

    Err(ExecutionError::Validation(format!(
        "CARDINALITY argument '{}' is not a relationship or path variable",
        var_name
    )))
}

/// Extract variable name from an expression
///
/// Returns the variable name if the expression is a simple variable reference.
fn get_variable_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::PropertyAccess {
            variable,
            properties,
            ..
        } if properties.is_empty() => Some(variable.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_plan::graph_algo::GraphEdge;
    use crate::physical_plan::pgq::matching::{GraphPath, PathNode};
    use crate::physical_plan::pgq::types::RelationInfo;
    use raisin_sql::ast::SourceSpan;

    fn var(name: &str) -> Expr {
        Expr::PropertyAccess {
            variable: name.into(),
            properties: vec![],
            span: SourceSpan::empty(),
        }
    }

    #[test]
    fn cardinality_reads_the_bound_path_not_a_mangled_relation_type() {
        let mut path = GraphPath::start(PathNode::new("a", "ws", "T"));
        path.push(&GraphEdge::new("ws", "b", "T", "knows", None));

        let mut binding = VariableBinding::new();
        binding.bind_path("r".into(), path);
        binding.bind_relation(
            "r".into(),
            RelationInfo::new("knows".into(), None, "a".into(), "b".into()),
        );

        assert_eq!(
            evaluate_function("CARDINALITY", &[var("r")], &binding).unwrap(),
            SqlValue::Integer(1)
        );
    }

    #[test]
    fn cardinality_of_a_single_hop_relationship_is_one() {
        let mut binding = VariableBinding::new();
        binding.bind_relation(
            "r".into(),
            RelationInfo::new("knows".into(), None, "a".into(), "b".into()),
        );
        assert_eq!(
            evaluate_function("cardinality", &[var("r")], &binding).unwrap(),
            SqlValue::Integer(1)
        );
    }

    #[test]
    fn cardinality_of_an_unknown_variable_errors() {
        let binding = VariableBinding::new();
        assert!(evaluate_function("cardinality", &[var("nope")], &binding).is_err());
    }
}
