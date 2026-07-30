//! Path accessors for GRAPH_TABLE `COLUMNS` and `WHERE`.
//!
//! Path accessors are **not** standardised — DuckPGQ, Spanner and Oracle each
//! spell them differently. RaisinDB uses DuckPGQ-style lowercase names, plus the
//! four Spanner names DuckPGQ has no equivalent for. Uppercase spellings work
//! for free because dispatch lowercases first, as the graph-algorithm dispatcher
//! already does.
//!
//! | accessor | result | `SqlValue` |
//! |---|---|---|
//! | `path_length(p)` | hop count (`edges.len()`) | `Integer` |
//! | `nodes(p)` | nodes in path order | `Json` array |
//! | `edges(p)` | edges in path order | `Json` array |
//! | `element_id(p)` | opaque stable path identity | `String` |
//! | `path_first(p)` / `path_last(p)` | one node identity | `Json` object |
//! | `is_trail(p)` / `is_acyclic(p)` | distinctness | `Boolean` |
//!
//! # There is no PATH column type
//!
//! `COLUMNS (p)` is a deliberate compile error. `SqlValue` has no PATH variant
//! and adding one would mean teaching three transports a new type — PGWire in
//! particular has nothing to borrow, since PostgreSQL OID 602 `path` is a
//! *geometric* type. Every accessor above lands on a `SqlValue` that already
//! crosses HTTP, WS and PGWire, so path support needs zero transport changes.

use raisin_sql::ast::Expr;
use serde_json::{json, Map, Value};

use super::Result;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::pgq::matching::{GraphPath, PathEdge, PathNode};
use crate::physical_plan::pgq::types::{SqlValue, VariableBinding};

/// Every accessor name, for diagnostics.
const ACCESSOR_NAMES: &str =
    "path_length, nodes, edges, element_id, path_first, path_last, is_trail, is_acyclic";

/// True when `name` is a path accessor.
pub(super) fn is_path_accessor(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "path_length"
            | "nodes"
            | "edges"
            | "element_id"
            | "path_first"
            | "path_last"
            | "is_trail"
            | "is_acyclic"
    )
}

/// Evaluate a path accessor against a binding.
pub(super) fn evaluate_path_accessor(
    name: &str,
    args: &[Expr],
    binding: &VariableBinding,
) -> Result<SqlValue> {
    let lower = name.to_lowercase();

    if args.len() != 1 {
        return Err(ExecutionError::Validation(format!(
            "{lower} requires exactly one argument: a path variable"
        )));
    }

    let var = path_variable_name(&args[0]).ok_or_else(|| {
        ExecutionError::Validation(format!("{lower} requires a path variable as its argument"))
    })?;

    let path = binding.get_path(&var).ok_or_else(|| {
        ExecutionError::Validation(format!(
            "{lower} argument '{var}' is not a path variable. Path values come from a \
             variable-length pattern; declare one with MATCH {var} = (a)-[e]->{{1,3}}(b)."
        ))
    })?;

    Ok(match lower.as_str() {
        "path_length" => SqlValue::Integer(path.length() as i64),
        "nodes" => SqlValue::Json(nodes_json(path)),
        "edges" => SqlValue::Json(edges_json(path)),
        "element_id" => SqlValue::String(path.element_id()),
        "path_first" => json_or_null(path.first().map(node_json)),
        "path_last" => json_or_null(path.last().map(node_json)),
        "is_trail" => SqlValue::Boolean(path.is_trail()),
        "is_acyclic" => SqlValue::Boolean(path.is_acyclic()),
        other => {
            return Err(ExecutionError::Validation(format!(
                "Unknown path accessor: {other}. Available: {ACCESSOR_NAMES}"
            )))
        }
    })
}

fn json_or_null(value: Option<Value>) -> SqlValue {
    value.map(SqlValue::Json).unwrap_or(SqlValue::Null)
}

/// Fixed shape: `{"id":…, "workspace":…, "node_type":…}`.
///
/// This shape crosses HTTP, WS and PGWire verbatim; changing it is a breaking
/// change.
fn node_json(node: &PathNode) -> Value {
    let mut map = Map::new();
    map.insert("id".into(), json!(node.id));
    map.insert("workspace".into(), json!(node.workspace));
    map.insert("node_type".into(), json!(node.node_type));
    Value::Object(map)
}

/// Fixed shape, with `weight` taken verbatim from `RelationRef::weight` and
/// rendered as JSON `null` when unset — never defaulted to 1.0.
fn edge_json(edge: &PathEdge) -> Value {
    let mut map = Map::new();
    map.insert("relation_type".into(), json!(edge.relation_type));
    map.insert("source_id".into(), json!(edge.source_id));
    map.insert("target_id".into(), json!(edge.target_id));
    map.insert("source_workspace".into(), json!(edge.source_workspace));
    map.insert("target_workspace".into(), json!(edge.target_workspace));
    map.insert(
        "weight".into(),
        match edge.weight {
            Some(w) => json!(w),
            None => Value::Null,
        },
    );
    Value::Object(map)
}

/// `nodes(p)` — JSON array in path order, length `path_length + 1`.
fn nodes_json(path: &GraphPath) -> Value {
    Value::Array(path.nodes.iter().map(node_json).collect())
}

/// `edges(p)` — JSON array in path order, length `path_length`.
fn edges_json(path: &GraphPath) -> Value {
    Value::Array(path.edges.iter().map(edge_json).collect())
}

/// Extract a bare variable name from an accessor argument.
fn path_variable_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::PropertyAccess {
            variable,
            properties,
            ..
        } if properties.is_empty() => Some(variable.clone()),
        _ => None,
    }
}

/// The error produced when a path variable is selected directly.
///
/// `COLUMNS (p)` cannot work: there is no PATH column type (see module docs), so
/// the message names the accessors instead of returning something lossy.
pub(super) fn path_is_not_selectable(variable: &str) -> ExecutionError {
    ExecutionError::Validation(format!(
        "'{variable}' is a path and has no SQL column type, so it cannot be selected directly. \
         Use a path accessor: {ACCESSOR_NAMES}. For example COLUMNS (path_length({variable}), \
         nodes({variable}))."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_plan::graph_algo::GraphEdge;
    use crate::physical_plan::pgq::matching::PathNode;
    use raisin_sql::ast::SourceSpan;

    fn var(name: &str) -> Expr {
        Expr::PropertyAccess {
            variable: name.into(),
            properties: vec![],
            span: SourceSpan::empty(),
        }
    }

    fn binding_with_path() -> VariableBinding {
        let mut path = GraphPath::start(PathNode::new("a", "ws", "User"));
        path.push(&GraphEdge::new("ws", "b", "User", "knows", Some(2.0)));
        path.push(&GraphEdge::new("ws", "c", "Admin", "knows", None));

        let mut binding = VariableBinding::new();
        binding.bind_path("p".into(), path);
        binding
    }

    #[test]
    fn path_length_is_the_hop_count() {
        let b = binding_with_path();
        assert_eq!(
            evaluate_path_accessor("path_length", &[var("p")], &b).unwrap(),
            SqlValue::Integer(2)
        );
    }

    #[test]
    fn uppercase_spellings_work_too() {
        let b = binding_with_path();
        assert_eq!(
            evaluate_path_accessor("PATH_LENGTH", &[var("p")], &b).unwrap(),
            SqlValue::Integer(2)
        );
    }

    #[test]
    fn nodes_and_edges_are_json_arrays_in_path_order() {
        let b = binding_with_path();
        let SqlValue::Json(nodes) = evaluate_path_accessor("nodes", &[var("p")], &b).unwrap()
        else {
            panic!("nodes(p) must be JSON");
        };
        assert_eq!(nodes.as_array().unwrap().len(), 3);
        assert_eq!(nodes[0]["id"], serde_json::json!("a"));
        assert_eq!(nodes[2]["node_type"], serde_json::json!("Admin"));

        let SqlValue::Json(edges) = evaluate_path_accessor("edges", &[var("p")], &b).unwrap()
        else {
            panic!("edges(p) must be JSON");
        };
        assert_eq!(edges.as_array().unwrap().len(), 2);
        // weight is RelationRef.weight verbatim, null when unset.
        assert_eq!(edges[0]["weight"], serde_json::json!(2.0));
        assert_eq!(edges[1]["weight"], serde_json::Value::Null);
    }

    #[test]
    fn endpoints_and_identity() {
        let b = binding_with_path();
        let SqlValue::Json(first) = evaluate_path_accessor("path_first", &[var("p")], &b).unwrap()
        else {
            panic!("path_first(p) must be JSON");
        };
        assert_eq!(first["id"], serde_json::json!("a"));

        assert_eq!(
            evaluate_path_accessor("element_id", &[var("p")], &b).unwrap(),
            SqlValue::String("ws:a|knows|ws:b|knows|ws:c".into())
        );
    }

    #[test]
    fn distinctness_predicates() {
        let b = binding_with_path();
        assert_eq!(
            evaluate_path_accessor("is_trail", &[var("p")], &b).unwrap(),
            SqlValue::Boolean(true)
        );
        assert_eq!(
            evaluate_path_accessor("is_acyclic", &[var("p")], &b).unwrap(),
            SqlValue::Boolean(true)
        );
    }

    #[test]
    fn an_unbound_variable_names_the_remedy() {
        let b = binding_with_path();
        let err = evaluate_path_accessor("path_length", &[var("q")], &b).unwrap_err();
        assert!(err.to_string().contains("is not a path variable"), "{err}");
    }

    #[test]
    fn selecting_a_path_directly_names_the_accessors() {
        let err = path_is_not_selectable("p");
        assert!(err.to_string().contains("path_length"), "{err}");
        assert!(err.to_string().contains("no SQL column type"), "{err}");
    }
}
