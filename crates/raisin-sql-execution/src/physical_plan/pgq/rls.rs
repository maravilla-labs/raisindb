//! Row-level security for `GRAPH_TABLE`.
//!
//! # Policy: ENDPOINTS ONLY
//!
//! Only the nodes a query actually RETURNS are permission-checked — that is,
//! the pattern variables referenced by the `COLUMNS(...)` clause. Intermediate
//! hops that are merely traversed stay traversable:
//!
//! ```text
//! MATCH (a)-[]->(b)-[]->(c) COLUMNS(a.name, c.name)
//! ```
//!
//! `b` is a stepping stone. A caller who may read `a` and `c` gets the row even
//! if `b` is invisible to them. The alternative (prune the whole path when any
//! node on it is denied) is a *stricter* policy that was explicitly not chosen,
//! and it is easy to implement by accident: a binding holds every pattern
//! variable, and `ensure_loaded` may have materialised `b` to evaluate a WHERE
//! clause, so "nodes present in the binding" is NOT "nodes returned". The whole
//! point of [`ProjectedVars`] is to keep those two sets apart.
//!
//! If ANY projected node is denied, the whole ROW is dropped — a row with a
//! hole in it would leak the existence of the denied node.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::permissions::PermissionScope;
use raisin_sql::ast::{ColumnsClause, Expr};
use raisin_storage::{NodeRepository, Storage, StorageScope};

use super::context::PgqContext;
use super::types::VariableBinding;
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Which pattern variables a `COLUMNS(...)` clause returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProjectedVars {
    /// An unqualified `*` is present: every variable bound in a row is emitted,
    /// so every node variable of that binding is a projected endpoint.
    All,
    /// Only these variable names are emitted. Names that turn out to be
    /// relation or path variables are ignored by the gate — RLS filters nodes.
    Named(HashSet<String>),
}

/// Compute the set of pattern variables referenced by the COLUMNS clause.
///
/// Every expression form that can name a variable is walked, not just the
/// simple `a.name` case: an alias, a nested expression, an aggregate argument
/// (`COUNT(a)`), a CASE arm and a JSONPath access all put a node's data in the
/// output and therefore make that node an endpoint.
///
/// A bare variable (`COLUMNS(a)`) parses as `PropertyAccess` with an empty
/// property path, so it is covered by the same arm.
pub(super) fn projected_variables(columns: &ColumnsClause) -> ProjectedVars {
    let mut named = HashSet::new();
    for col in &columns.columns {
        if collect_expr_vars(&col.expr, &mut named) {
            return ProjectedVars::All;
        }
    }
    ProjectedVars::Named(named)
}

/// Collect variables referenced by `expr`. Returns true if an unqualified
/// wildcard was found, in which case the caller must widen to [`ProjectedVars::All`].
fn collect_expr_vars(expr: &Expr, out: &mut HashSet<String>) -> bool {
    match expr {
        Expr::PropertyAccess { variable, .. } | Expr::JsonPathAccess { variable, .. } => {
            out.insert(variable.clone());
            false
        }
        Expr::Wildcard { qualifier, .. } => match qualifier {
            // `a.*` — expands every field of `a`, so `a` is projected.
            Some(var) => {
                out.insert(var.clone());
                false
            }
            // `*` — expands every variable of the binding, known only at runtime.
            None => true,
        },
        Expr::Literal(_) => false,
        Expr::BinaryOp { left, right, .. } => {
            // Both sides are walked; `||` short-circuits nothing at plan time.
            let l = collect_expr_vars(left, out);
            let r = collect_expr_vars(right, out);
            l || r
        }
        Expr::UnaryOp { expr, .. }
        | Expr::IsNull { expr, .. }
        | Expr::Like { expr, .. }
        | Expr::Nested(expr)
        | Expr::JsonAccess { expr, .. } => collect_expr_vars(expr, out),
        Expr::FunctionCall { args, .. } => {
            let mut all = false;
            for arg in args {
                all |= collect_expr_vars(arg, out);
            }
            all
        }
        Expr::InList { expr, list, .. } => {
            let mut all = collect_expr_vars(expr, out);
            for item in list {
                all |= collect_expr_vars(item, out);
            }
            all
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            let mut all = collect_expr_vars(expr, out);
            all |= collect_expr_vars(low, out);
            all |= collect_expr_vars(high, out);
            all
        }
        Expr::Case {
            operand,
            when_clauses,
            else_clause,
            ..
        } => {
            let mut all = false;
            if let Some(operand) = operand {
                all |= collect_expr_vars(operand, out);
            }
            for (when, then) in when_clauses {
                all |= collect_expr_vars(when, out);
                all |= collect_expr_vars(then, out);
            }
            if let Some(else_clause) = else_clause {
                all |= collect_expr_vars(else_clause, out);
            }
            all
        }
    }
}

/// Permission gate for projected endpoints.
///
/// Built once per `GRAPH_TABLE` projection and reused across every binding, so
/// a node reachable from many rows is fetched and evaluated once. The graph
/// resolver is likewise built at most once per node inside
/// [`rls_filter_node_graph`], which short-circuits entirely when no permission
/// uses a `RELATES` condition.
pub(super) struct RlsGate<'a> {
    auth: &'a AuthContext,
    vars: ProjectedVars,
    /// (workspace, node_id) → filtered node, or None when access is denied.
    decisions: HashMap<(String, String), Option<Arc<Node>>>,
}

impl<'a> RlsGate<'a> {
    /// Build a gate, or `None` when there is no auth context.
    ///
    /// `None` means "system/internal caller" and must stay completely
    /// unfiltered — that is how every scan executor behaves, and GRAPH_TABLE
    /// running inside jobs, replication and function bindings depends on it.
    pub(super) fn for_query(context: &'a PgqContext, columns: &ColumnsClause) -> Option<Self> {
        let auth = context.auth_context.as_ref()?;
        Some(Self {
            auth,
            vars: projected_variables(columns),
            decisions: HashMap::new(),
        })
    }

    /// Decide whether this binding may produce a row.
    ///
    /// Checks ONLY the projected variables. On success the binding's node data
    /// is replaced with the permission-filtered node, so field-level filtering
    /// (a permission that grants a subset of fields) also applies to the
    /// columns GRAPH_TABLE emits.
    pub(super) async fn admit<S: Storage>(
        &mut self,
        binding: &mut VariableBinding,
        storage: &Arc<S>,
        context: &PgqContext,
    ) -> Result<bool> {
        // Resolve the projected variables against THIS binding. A variable that
        // is not a node variable (a relation or a path) has no node to check.
        let targets: Vec<(String, String, String)> = match &self.vars {
            ProjectedVars::All => binding
                .node_vars()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|var| {
                    binding
                        .get_node(&var)
                        .map(|n| (var.clone(), n.workspace.clone(), n.id.clone()))
                })
                .collect(),
            ProjectedVars::Named(names) => names
                .iter()
                .filter_map(|var| {
                    binding
                        .get_node(var)
                        .map(|n| (var.clone(), n.workspace.clone(), n.id.clone()))
                })
                .collect(),
        };

        for (var, workspace, node_id) in targets {
            let decision = self.decide(storage, context, &workspace, &node_id).await?;
            match decision {
                Some(filtered) => {
                    if let Some(info) = binding.get_node_mut(&var) {
                        info.apply_loaded_node(&filtered);
                    }
                }
                None => return Ok(false),
            }
        }

        Ok(true)
    }

    /// Fetch + permission-check a node, memoised per (workspace, node id).
    async fn decide<S: Storage>(
        &mut self,
        storage: &Arc<S>,
        context: &PgqContext,
        workspace: &str,
        node_id: &str,
    ) -> Result<Option<Arc<Node>>> {
        let key = (workspace.to_string(), node_id.to_string());
        if let Some(cached) = self.decisions.get(&key) {
            return Ok(cached.clone());
        }

        let node = storage
            .nodes()
            .get(
                StorageScope::new(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    workspace,
                ),
                node_id,
                context.revision.as_ref(),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;

        let decision = match node {
            Some(node) => {
                let scope = PermissionScope::new(workspace, &context.branch);
                crate::physical_plan::scan_executors::helpers::rls_filter_node_graph(
                    &**storage,
                    node,
                    self.auth,
                    &scope,
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    context.revision.as_ref(),
                )
                .await
                .map(Arc::new)
            }
            // Missing/tombstoned: no row. Same outcome as the orphaned-relation
            // check in the projection, and it must not be an allow.
            None => None,
        };

        self.decisions.insert(key, decision.clone());
        Ok(decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_sql::ast::{ColumnExpr, SourceSpan};

    fn col(expr: Expr) -> ColumnExpr {
        ColumnExpr {
            expr,
            alias: None,
            span: SourceSpan::empty(),
        }
    }

    fn prop(var: &str, props: &[&str]) -> Expr {
        Expr::PropertyAccess {
            variable: var.into(),
            properties: props.iter().map(|s| s.to_string()).collect(),
            span: SourceSpan::empty(),
        }
    }

    fn named(columns: Vec<ColumnExpr>) -> Vec<String> {
        match projected_variables(&ColumnsClause {
            columns,
            span: SourceSpan::empty(),
        }) {
            ProjectedVars::Named(mut v) => {
                let mut out: Vec<String> = v.drain().collect();
                out.sort();
                out
            }
            ProjectedVars::All => panic!("expected named"),
        }
    }

    #[test]
    fn only_projected_variables_are_collected() {
        // MATCH (a)-[]->(b)-[]->(c) COLUMNS(a.name, c.name): `b` is a stepping
        // stone and must NOT appear. This is the endpoints-only invariant.
        assert_eq!(
            named(vec![col(prop("a", &["name"])), col(prop("c", &["name"]))]),
            vec!["a", "c"]
        );
    }

    #[test]
    fn qualified_wildcard_projects_its_variable() {
        assert_eq!(
            named(vec![col(Expr::Wildcard {
                qualifier: Some("a".into()),
                span: SourceSpan::empty(),
            })]),
            vec!["a"]
        );
    }

    #[test]
    fn bare_wildcard_widens_to_all() {
        let clause = ColumnsClause {
            columns: vec![col(Expr::Wildcard {
                qualifier: None,
                span: SourceSpan::empty(),
            })],
            span: SourceSpan::empty(),
        };
        assert_eq!(projected_variables(&clause), ProjectedVars::All);
    }

    #[test]
    fn nested_and_aggregate_arguments_are_collected() {
        let expr = Expr::FunctionCall {
            name: "count".into(),
            args: vec![Expr::Nested(Box::new(Expr::BinaryOp {
                left: Box::new(prop("a", &["x"])),
                op: raisin_sql::ast::BinaryOperator::Plus,
                right: Box::new(prop("b", &["y"])),
                span: SourceSpan::empty(),
            }))],
            distinct: false,
            span: SourceSpan::empty(),
        };
        assert_eq!(named(vec![col(expr)]), vec!["a", "b"]);
    }

    #[test]
    fn a_wildcard_anywhere_widens_to_all() {
        let clause = ColumnsClause {
            columns: vec![
                col(prop("a", &["name"])),
                col(Expr::FunctionCall {
                    name: "count".into(),
                    args: vec![Expr::Wildcard {
                        qualifier: None,
                        span: SourceSpan::empty(),
                    }],
                    distinct: false,
                    span: SourceSpan::empty(),
                }),
            ],
            span: SourceSpan::empty(),
        };
        assert_eq!(projected_variables(&clause), ProjectedVars::All);
    }
}
