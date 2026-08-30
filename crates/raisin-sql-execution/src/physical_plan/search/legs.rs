//! The two index legs, and what gets pushed into them.
//!
//! Both legs FAIL LOUDLY. They used to be called as `if let Ok(results) = ...`,
//! so an engine that errored produced an empty rank map and the statement
//! returned a plausible-looking half-search with nothing logged: the exact shape
//! of the dead-vector-leg bug, where every row came back with a NULL
//! `vector_rank` and no operator could tell that from "the vector index matched
//! nothing". Per CLAUDE.md, fail closed rather than open.

use std::sync::Arc;

use raisin_embeddings::provider::EmbeddingProvider;
use raisin_hnsw::{HnswIndexingEngine, PartitionId, SearchResult};
use raisin_indexer::TantivyIndexingEngine;
use raisin_sql::analyzer::{BinaryOperator, Expr, Literal, TypedExpr};
use raisin_storage::fulltext::{FullTextSearchQuery, FullTextSearchResult};
use raisin_storage::IndexingEngine;

use crate::physical_plan::executor::ExecutionError;

use super::args::EmbeddingKindFilter;
use super::scope::WorkspaceSet;

/// Where a leg runs, minus the query itself.
pub struct LegContext {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
    pub max_revision: Option<raisin_hlc::HLC>,
}

/// Run the full-text leg.
///
/// The workspace set reaches Tantivy natively: `add_workspace_filter` already
/// builds a `TermQuery` for one workspace and a `Should` boolean for several, so
/// pushing the resolved scope down here is a call-site change and no new indexer
/// code. It is a rank-CORRECTNESS change, not only a performance one: RRF
/// consumes ranks, and a rank computed over a pool containing rows the caller
/// can never see is a wrong rank printed in a column named `fulltext_rank`.
pub fn run_fulltext_leg(
    engine: &Arc<TantivyIndexingEngine>,
    ctx: &LegContext,
    scope: &WorkspaceSet,
    query_text: &str,
    language: &str,
    shape_types: Option<Vec<String>>,
    k: usize,
) -> Result<Vec<FullTextSearchResult>, ExecutionError> {
    let search_query = FullTextSearchQuery {
        tenant_id: ctx.tenant_id.clone(),
        repo_id: ctx.repo_id.clone(),
        // NOTE for the next reader: a pushed-down workspace set does NOT mean
        // the rows coming back are authorised. Workspace is one of four RLS
        // dimensions (workspace, path, node_type, REL condition) plus field
        // filtering. Every hit still goes through `rls_filter_search_hit`.
        workspace_ids: scope.as_filter().map(|w| w.to_vec()),
        branch: ctx.branch.clone(),
        language: language.to_string(),
        query: crate::physical_plan::fulltext::convert_postgres_query(query_text),
        limit: k,
        revision: ctx.max_revision,
        shape_types,
    };

    engine.search(&search_query).map_err(|e| {
        ExecutionError::Backend(format!(
            "full-text leg failed: {e}. The result would have been a vector-only \
             search reported as a hybrid one, so the statement fails instead."
        ))
    })
}

/// Run the vector leg.
///
/// `query_vector` is already normalised.
#[allow(clippy::too_many_arguments)]
pub fn run_vector_leg(
    engine: &Arc<HnswIndexingEngine>,
    ctx: &LegContext,
    scope: &WorkspaceSet,
    partition: &PartitionId,
    query_vector: &[f32],
    k: usize,
    max_distance: f32,
) -> Result<Vec<SearchResult>, ExecutionError> {
    // Empty slice = no filter. `WorkspaceSet::Empty` never reaches here; the
    // emit loop returns zero rows without touching either engine.
    let workspaces: Vec<String> = scope.as_filter().map(|w| w.to_vec()).unwrap_or_default();

    engine
        .search_with_threshold(
            &ctx.tenant_id,
            &ctx.repo_id,
            &ctx.branch,
            partition,
            &workspaces,
            query_vector,
            k,
            Some(max_distance),
        )
        .map_err(|e| {
            ExecutionError::Backend(format!(
                "vector leg over partition {partition} failed: {e}. The result would \
                 have been a full-text search reported as a hybrid one, so the \
                 statement fails instead."
            ))
        })
}

/// Which vector partitions the `kind =>` selector resolves to.
///
/// One partition becomes one leg, and N legs fuse by rank with no new parameter
/// (see [`super::fusion`]). Resolution is deliberately ASYMMETRIC between the
/// two kinds, and the asymmetry is not an oversight:
///
/// * **Text** comes from `default_text_partition`, i.e. from the tenant's
///   embedding config row -- the SAME row the write path resolves against. That
///   agreement is the whole invariant `PartitionId` exists to protect: a read
///   side that discovered its text partition by listing the directory could
///   pick a stale model's index during a re-embedding and answer every query
///   confidently from the wrong vector space.
/// * **Image** is DISCOVERED, by listing the branch's partitions and keeping
///   the tokens that end in `'I'`, because there is no configured image
///   embedder to resolve against yet: `TenantEmbeddingConfig` names one model.
///
/// # The seam
///
/// When an image embedder becomes configurable, `IndexSpecResolver` gains a
/// `default_image_partition` beside `default_text_partition`, and the `Image`
/// arm below switches to it — one function, one arm, no call site. Until then
/// discovery is the honest answer rather than a second config concept invented
/// here.
pub fn resolve_vector_partitions(
    engine: &Arc<HnswIndexingEngine>,
    ctx: &LegContext,
    kind: EmbeddingKindFilter,
) -> Result<Vec<PartitionId>, ExecutionError> {
    let configured_text = engine.default_text_partition(&ctx.tenant_id, &ctx.repo_id, &ctx.branch);

    let mut out: Vec<PartitionId> = Vec::new();

    if kind.admits('T') {
        match configured_text {
            Some(p) => out.push(p),
            None if kind == EmbeddingKindFilter::Text => {
                // Unchanged from before `kind` existed: asking for text vectors
                // on a tenant with no embedding config is a hard error, not an
                // empty leg. A vector leg that silently does nothing is the
                // production shape of the last bug in this area.
                return Err(ExecutionError::Backend(
                    "the vector leg cannot run: this tenant has no resolvable \
                     embedding configuration, so there is no index partition to \
                     search. The result would have been a full-text search \
                     reported as a hybrid one, so the statement fails instead."
                        .to_string(),
                ));
            }
            // `kind => 'all'` with no text config still has the image half to
            // try; failing here would make 'all' STRICTER than 'image'.
            None => {}
        }
    }

    if kind.admits('I') {
        let listed = engine
            .list_partitions(&ctx.tenant_id, &ctx.repo_id, &ctx.branch)
            .map_err(|e| {
                ExecutionError::Backend(format!(
                    "cannot list this branch's vector partitions to resolve \
                     kind => '{}': {e}",
                    kind.as_str()
                ))
            })?;
        for p in listed {
            if p.kind_char() == Some('I') && !out.contains(&p) {
                out.push(p);
            }
        }
    }

    if out.is_empty() {
        // Reached only for 'image' (and for 'all' on a tenant with neither), so
        // the message names the cause rather than leaving an empty leg to look
        // like an empty corpus.
        return Err(ExecutionError::Backend(format!(
            "kind => '{}' selects no vector index: this branch has no {} \
             embedding partition. An empty leg reported as a search is \
             indistinguishable from a corpus that matched nothing, so the \
             statement fails instead.",
            kind.as_str(),
            match kind {
                EmbeddingKindFilter::Image => "image",
                _ => "usable",
            }
        )));
    }

    Ok(out)
}

/// Embed the query text with the tenant's provider and normalise it.
pub async fn embed_query(
    provider: &Arc<dyn EmbeddingProvider>,
    text: &str,
) -> Result<Vec<f32>, ExecutionError> {
    let raw = provider.generate_embedding(text).await.map_err(|e| {
        ExecutionError::Backend(format!("failed to generate the query embedding: {e}"))
    })?;
    Ok(raisin_hnsw::normalize_vector(&raw))
}

/// Which shape-type terms, if any, can be pushed into the full-text leg from the
/// advisory residual filter.
///
/// Recognises ONLY two predicate shapes, both as TOP-LEVEL conjuncts:
///
/// * `node_type = 'X'` -- `BinaryOp{Eq}` over `Column{node_type}` and a text
///   literal;
/// * `node_type IN ('X', 'Y')` -- a non-negated `InList` of text literals.
///
/// Anything disjunctive, negated, or referencing a joined table gets no
/// push-down and is left entirely to the residual filter. Several recognised
/// conjuncts are INTERSECTED, and an empty intersection yields `None` (no
/// push-down) rather than an empty term list: `node_type = 'A' AND
/// node_type = 'B'` matches nothing, and the residual filter is what must say
/// so -- pushing an unsatisfiable filter down would be correct here but would
/// establish "the push-down can eliminate rows", which is exactly the property
/// this design does not want.
pub fn shape_type_pushdown(filter: Option<&TypedExpr>) -> Option<Vec<String>> {
    let filter = filter?;
    let mut sets: Vec<Vec<String>> = Vec::new();
    collect_conjuncts(filter, &mut sets);

    let mut acc: Option<Vec<String>> = None;
    for set in sets {
        acc = Some(match acc {
            None => set,
            Some(prev) => prev.into_iter().filter(|t| set.contains(t)).collect(),
        });
    }

    match acc {
        Some(types) if !types.is_empty() => Some(types),
        _ => None,
    }
}

fn collect_conjuncts(expr: &TypedExpr, out: &mut Vec<Vec<String>>) {
    match &expr.expr {
        Expr::BinaryOp { left, op, right } if *op == BinaryOperator::And => {
            collect_conjuncts(left, out);
            collect_conjuncts(right, out);
        }
        Expr::BinaryOp { left, op, right } if *op == BinaryOperator::Eq => {
            if let (Some(_), Some(text)) = (node_type_column(left), text_literal(right)) {
                out.push(vec![text]);
            } else if let (Some(_), Some(text)) = (node_type_column(right), text_literal(left)) {
                out.push(vec![text]);
            }
        }
        Expr::InList {
            expr: inner,
            list,
            negated: false,
        } => {
            if node_type_column(inner).is_some() {
                let values: Option<Vec<String>> = list.iter().map(text_literal).collect();
                if let Some(values) = values {
                    if !values.is_empty() {
                        out.push(values);
                    }
                }
            }
        }
        _ => {}
    }
}

fn node_type_column(expr: &TypedExpr) -> Option<()> {
    match &expr.expr {
        Expr::Column { column, .. } if column == "node_type" => Some(()),
        _ => None,
    }
}

fn text_literal(expr: &TypedExpr) -> Option<String> {
    match &expr.expr {
        Expr::Literal(Literal::Text(v)) | Expr::Literal(Literal::Path(v)) => Some(v.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_sql::analyzer::types::DataType;

    fn col(name: &str) -> TypedExpr {
        TypedExpr::column(
            "hybrid_search".to_string(),
            name.to_string(),
            DataType::Text,
        )
    }
    fn text(v: &str) -> TypedExpr {
        TypedExpr::new(Expr::Literal(Literal::Text(v.to_string())), DataType::Text)
    }
    fn binop(l: TypedExpr, op: BinaryOperator, r: TypedExpr) -> TypedExpr {
        TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(l),
                op,
                right: Box::new(r),
            },
            DataType::Boolean,
        )
    }

    #[test]
    fn equality_on_node_type_is_pushed() {
        let f = binop(col("node_type"), BinaryOperator::Eq, text("ns:Doc"));
        assert_eq!(
            shape_type_pushdown(Some(&f)),
            Some(vec!["ns:Doc".to_string()])
        );
    }

    #[test]
    fn a_conjunct_buried_under_and_is_found() {
        let f = binop(
            binop(col("path"), BinaryOperator::Eq, text("/x")),
            BinaryOperator::And,
            binop(col("node_type"), BinaryOperator::Eq, text("ns:Doc")),
        );
        assert_eq!(
            shape_type_pushdown(Some(&f)),
            Some(vec!["ns:Doc".to_string()])
        );
    }

    #[test]
    fn in_list_is_pushed_and_negation_is_not() {
        let list = TypedExpr::new(
            Expr::InList {
                expr: Box::new(col("node_type")),
                list: vec![text("a:One"), text("b:Two")],
                negated: false,
            },
            DataType::Boolean,
        );
        assert_eq!(
            shape_type_pushdown(Some(&list)),
            Some(vec!["a:One".to_string(), "b:Two".to_string()])
        );

        let negated = TypedExpr::new(
            Expr::InList {
                expr: Box::new(col("node_type")),
                list: vec![text("a:One")],
                negated: true,
            },
            DataType::Boolean,
        );
        assert_eq!(shape_type_pushdown(Some(&negated)), None);
    }

    /// A disjunction must not be pushed: `OR` widens, and treating one arm as a
    /// mandatory term would DROP rows the residual filter would have kept.
    #[test]
    fn disjunction_is_not_pushed() {
        let f = binop(
            binop(col("node_type"), BinaryOperator::Eq, text("a:One")),
            BinaryOperator::Or,
            binop(col("node_type"), BinaryOperator::Eq, text("b:Two")),
        );
        assert_eq!(shape_type_pushdown(Some(&f)), None);
    }

    #[test]
    fn other_columns_are_ignored() {
        let f = binop(col("path"), BinaryOperator::Eq, text("/x"));
        assert_eq!(shape_type_pushdown(Some(&f)), None);
        assert_eq!(shape_type_pushdown(None), None);
    }
}
