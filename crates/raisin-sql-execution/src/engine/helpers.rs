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
    COMPOUND_INDEX_CACHE
        .get_or_init(|| raisin_core::TtlCache::new(std::time::Duration::from_secs(30)))
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
