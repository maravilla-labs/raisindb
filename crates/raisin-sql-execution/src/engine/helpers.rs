//! Compound index loading and node_type extraction helpers.
//!
//! Provides utility functions for extracting node_type from WHERE clauses
//! and loading compound index definitions from NodeType storage.

use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_sql::analyzer::{AnalyzedStatement, BinaryOperator, Expr, Literal, TypedExpr};
use raisin_storage::{NodeTypeRepository, Storage};

/// Extract node_type value from analyzed query's WHERE clause
///
/// Searches the WHERE clause for `node_type = 'typename'` conditions
/// so we can load compound indexes for the matching NodeType.
pub(super) fn extract_node_type_from_analyzed(analyzed: &AnalyzedStatement) -> Option<String> {
    if let AnalyzedStatement::Query(ref q) = analyzed {
        if let Some(ref filter) = q.selection {
            return extract_node_type_from_expr(filter);
        }
    }
    None
}

/// The WHERE clause of an analyzed statement, if it has one.
///
/// Exists so every `apply_schema_stats` call site can hand it the same
/// `Option<&TypedExpr>` regardless of whether it holds a raw query or an
/// `AnalyzedStatement` — the gate then lives in one place instead of being
/// re-derived per site. Mirrors `extract_node_type_from_analyzed`'s shape.
pub(super) fn analyzed_selection(analyzed: &AnalyzedStatement) -> Option<&TypedExpr> {
    if let AnalyzedStatement::Query(ref q) = analyzed {
        return q.selection.as_ref();
    }
    None
}

/// The WHERE clause of any analyzed statement that HAS one, DML included.
///
/// Separate from [`analyzed_selection`] on purpose: that one feeds
/// `apply_schema_stats`, whose gate is deliberately query-only. This one feeds
/// index selection, where an `UPDATE … WHERE` deserves the same access path its
/// equivalent `SELECT … WHERE` would get — which it did not, because the DML
/// planner never loaded compound indexes at all.
pub(super) fn statement_filter(analyzed: &AnalyzedStatement) -> Option<&TypedExpr> {
    match analyzed {
        AnalyzedStatement::Query(q) => q.selection.as_ref(),
        AnalyzedStatement::Update(u) => u.filter.as_ref(),
        AnalyzedStatement::Delete(d) => d.filter.as_ref(),
        _ => None,
    }
}

/// Recursively search a TypedExpr for `node_type = 'value'` pattern
pub(crate) fn extract_node_type_from_expr(expr: &TypedExpr) -> Option<String> {
    match &expr.expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            // Check: node_type = 'X'
            if let Expr::Column { column, .. } = &left.expr {
                if column.eq_ignore_ascii_case("node_type") {
                    if let Expr::Literal(Literal::Text(s)) = &right.expr {
                        return Some(s.clone());
                    }
                }
            }
            // Check reversed: 'X' = node_type
            if let Expr::Column { column, .. } = &right.expr {
                if column.eq_ignore_ascii_case("node_type") {
                    if let Expr::Literal(Literal::Text(s)) = &left.expr {
                        return Some(s.clone());
                    }
                }
            }
            None
        }
        // Recurse into AND/OR expressions
        Expr::BinaryOp { left, right, .. } => {
            extract_node_type_from_expr(left).or_else(|| extract_node_type_from_expr(right))
        }
        _ => None,
    }
}

/// Does this WHERE clause contain a `node_type = '...'` or `archetype = '...'`
/// equality — the ONLY thing `SchemaStats` can change the plan for?
///
/// `schema_stats` is read in exactly one place,
/// `physical_plan/planner/scan_planning/selectivity.rs::estimate_selectivity`,
/// and only to refine `ColumnEq` on those two columns from the flat 0.05
/// heuristic to `1/count`. Every other predicate ignores it entirely. So for a
/// query without such an equality the stats are computed, cached and then never
/// read — and computing them deserializes EVERY NodeType and Archetype on the
/// branch (recursive `PropertyValueSchema` trees) just to take two `.len()`s.
/// In production that was ~33% of server CPU and a large share of the 41% spent
/// in `PropertyValue::deserialize` (2026-08 investigation).
///
/// BOTH columns must be checked. Gating on `node_type` alone silently degrades
/// archetype-equality plans back to the 0.05 heuristic — a planner regression
/// with no error anywhere.
///
/// Kept next to `extract_node_type_from_expr` and mirroring its traversal
/// (including the reversed `'X' = column` form) so the two cannot drift.
pub(crate) fn selection_uses_schema_stats(expr: &TypedExpr) -> bool {
    fn is_stats_column(name: &str) -> bool {
        name.eq_ignore_ascii_case("node_type") || name.eq_ignore_ascii_case("archetype")
    }

    match &expr.expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            if let Expr::Column { column, .. } = &left.expr {
                if is_stats_column(column) && matches!(right.expr, Expr::Literal(Literal::Text(_)))
                {
                    return true;
                }
            }
            if let Expr::Column { column, .. } = &right.expr {
                if is_stats_column(column) && matches!(left.expr, Expr::Literal(Literal::Text(_))) {
                    return true;
                }
            }
            false
        }
        Expr::BinaryOp { left, right, .. } => {
            selection_uses_schema_stats(left) || selection_uses_schema_stats(right)
        }
        _ => false,
    }
}

/// Every compound index declared by ANY NodeType on this branch, cached.
///
/// Needed because a hierarchy query usually does not name a node type:
/// `CHILD_OF('/inbox/2026/05') ORDER BY created_at LIMIT 50` is a complete,
/// ordinary query, and gating index loading on a literal `node_type =` meant a
/// `(__parent_path, ...)` index could never be found by the queries it exists
/// to serve.
///
/// Loading every type's definitions is consistent with how the entries are
/// actually keyed: a compound index key is
/// `…cidx\0{index_name}\0{column values}…` — it carries the index NAME and no
/// node type, so an index name already addresses a workspace-global keyspace.
/// Scoping the lookup by node type was a fiction the storage layer never shared.
///
/// (Corollary worth knowing: two NodeTypes declaring the same index name with
/// different columns share one keyspace and interleave incompatible entries.
/// That is pre-existing and wants a validation check at NodeType upsert.)
///
/// Cached per branch with a short TTL. A per-query NodeType listing would be a
/// fixed cost on every statement — the exact shape of overhead that made these
/// queries slow in the first place. The TTL is deliberately short rather than
/// the 5 minutes schema stats use: a definition removed from a NodeType stops
/// being maintained immediately, so continuing to plan against it reads a
/// keyspace nobody is writing.
static COMPOUND_INDEX_CACHE: std::sync::OnceLock<
    raisin_core::TtlCache<std::sync::Arc<Vec<CompoundIndexDefinition>>>,
> = std::sync::OnceLock::new();

fn compound_index_cache(
) -> &'static raisin_core::TtlCache<std::sync::Arc<Vec<CompoundIndexDefinition>>> {
    COMPOUND_INDEX_CACHE.get_or_init(|| {
        let cache = raisin_core::TtlCache::new(std::time::Duration::from_secs(30));
        // Replication checkpoint ingest copies whole SSTs in and emits NO
        // events, so an event-driven hook alone would leave this cache serving
        // definitions for a schema that has already been replaced. Registering
        // here — inside the initializer, so registration cannot get out of
        // order with population — is the registry's documented contract.
        //
        // Registered even though the TTL is only 30s: a stale entry here plans
        // reads against a keyspace nobody is writing any more, and 30s of that
        // is 30s of silently wrong index selection.
        raisin_core::register_invalidator(|| {
            if let Some(c) = COMPOUND_INDEX_CACHE.get() {
                c.invalidate_all();
            }
        });
        cache
    })
}

pub(crate) async fn load_all_compound_indexes<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Option<Vec<CompoundIndexDefinition>> {
    let scope_key = format!("{tenant_id}:{repo_id}:{branch}");

    if let Some(cached) = compound_index_cache().get(&scope_key) {
        return if cached.is_empty() {
            None
        } else {
            Some((*cached).clone())
        };
    }

    let node_types = match storage
        .node_types()
        .list(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            None,
        )
        .await
    {
        Ok(types) => types,
        Err(e) => {
            tracing::warn!("   Failed to list NodeTypes for compound indexes: {}", e);
            return None;
        }
    };

    // Deduplicate by index name: the name IS the keyspace, so two types
    // declaring the same one describe the same entries. First declaration wins,
    // which keeps planning deterministic across list order.
    let mut seen = std::collections::HashSet::new();
    let mut all: Vec<CompoundIndexDefinition> = Vec::new();
    for node_type in node_types {
        if let Some(indexes) = node_type.compound_indexes {
            for index in indexes {
                if seen.insert(index.name.clone()) {
                    all.push(index);
                }
            }
        }
    }

    tracing::debug!(
        "   Loaded {} compound indexes across all NodeTypes on '{}'",
        all.len(),
        branch
    );

    compound_index_cache().put(&scope_key, std::sync::Arc::new(all.clone()));

    if all.is_empty() {
        None
    } else {
        Some(all)
    }
}

/// Load compound indexes from NodeType storage
///
/// Fetches the NodeType definition and extracts its compound_indexes field.
/// Shares `COMPOUND_INDEX_CACHE` with the all-types variant above, under a key
/// that carries the node type — without this, a `node_type =` in the WHERE
/// clause meant a NodeType read on every statement while the *unpinned* path
/// (the more general one) was already cached.
pub(crate) async fn load_compound_indexes<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_type_name: &str,
) -> Option<Vec<CompoundIndexDefinition>> {
    let scope_key = format!("{tenant_id}:{repo_id}:{branch}:{node_type_name}");

    if let Some(cached) = compound_index_cache().get(&scope_key) {
        return if cached.is_empty() {
            None
        } else {
            Some((*cached).clone())
        };
    }

    let loaded =
        load_compound_indexes_uncached(storage, tenant_id, repo_id, branch, node_type_name)
            .await
            .unwrap_or_default();

    compound_index_cache().put(&scope_key, std::sync::Arc::new(loaded.clone()));

    if loaded.is_empty() {
        None
    } else {
        Some(loaded)
    }
}

async fn load_compound_indexes_uncached<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_type_name: &str,
) -> Option<Vec<CompoundIndexDefinition>> {
    match storage
        .node_types()
        .get(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            node_type_name,
            None,
        )
        .await
    {
        Ok(Some(node_type)) => {
            if let Some(ref indexes) = node_type.compound_indexes {
                if !indexes.is_empty() {
                    tracing::debug!(
                        "   Loaded {} compound indexes for '{}'",
                        indexes.len(),
                        node_type_name
                    );
                    return Some(indexes.clone());
                }
            }
            None
        }
        Ok(None) => {
            tracing::debug!("   NodeType '{}' not found", node_type_name);
            None
        }
        Err(e) => {
            tracing::warn!("   Failed to load NodeType '{}': {}", node_type_name, e);
            None
        }
    }
}

#[cfg(test)]
mod schema_stats_gate_tests {
    use super::*;
    use raisin_sql::analyzer::DataType;

    fn col(name: &str) -> TypedExpr {
        TypedExpr::new(
            Expr::Column {
                table: "nodes".to_string(),
                column: name.to_string(),
            },
            DataType::Text,
        )
    }

    fn lit(v: &str) -> TypedExpr {
        TypedExpr::new(Expr::Literal(Literal::Text(v.to_string())), DataType::Text)
    }

    fn eq(l: TypedExpr, r: TypedExpr) -> TypedExpr {
        TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(l),
                op: BinaryOperator::Eq,
                right: Box::new(r),
            },
            DataType::Boolean,
        )
    }

    fn and(l: TypedExpr, r: TypedExpr) -> TypedExpr {
        TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(l),
                op: BinaryOperator::And,
                right: Box::new(r),
            },
            DataType::Boolean,
        )
    }

    #[test]
    fn node_type_equality_needs_the_stats() {
        assert!(selection_uses_schema_stats(&eq(
            col("node_type"),
            lit("raisin:Page")
        )));
    }

    /// The regression this gate is most likely to cause. `estimate_selectivity`
    /// refines `archetype =` from `1/archetype_count` exactly as it does
    /// `node_type =`, so a gate that only recognised `node_type` would silently
    /// drop archetype-equality plans back to the flat 0.05 heuristic — a worse
    /// plan with no error anywhere.
    #[test]
    fn archetype_equality_needs_the_stats_too() {
        assert!(selection_uses_schema_stats(&eq(
            col("archetype"),
            lit("article")
        )));
    }

    #[test]
    fn reversed_literal_first_form_is_recognised() {
        assert!(selection_uses_schema_stats(&eq(
            lit("raisin:Page"),
            col("node_type")
        )));
    }

    #[test]
    fn column_matching_is_case_insensitive() {
        assert!(selection_uses_schema_stats(&eq(
            col("NODE_TYPE"),
            lit("raisin:Page")
        )));
    }

    #[test]
    fn nested_under_and_is_found() {
        let e = and(
            eq(col("path"), lit("/docs")),
            eq(col("node_type"), lit("raisin:Page")),
        );
        assert!(selection_uses_schema_stats(&e));
    }

    /// The whole point: an ordinary path/id lookup must NOT drag in a full
    /// NodeType + Archetype deserialize just to compute two counts nothing reads.
    #[test]
    fn a_predicate_the_stats_cannot_affect_skips_them() {
        assert!(!selection_uses_schema_stats(&eq(col("path"), lit("/docs"))));
        assert!(!selection_uses_schema_stats(&and(
            eq(col("path"), lit("/docs")),
            eq(col("name"), lit("index")),
        )));
    }

    /// `node_type > 'x'` is not an equality, so `estimate_selectivity` never
    /// consults the stats for it.
    #[test]
    fn non_equality_on_a_stats_column_does_not_qualify() {
        let gt = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(col("node_type")),
                op: BinaryOperator::Gt,
                right: Box::new(lit("raisin:Page")),
            },
            DataType::Boolean,
        );
        assert!(!selection_uses_schema_stats(&gt));
    }
}
