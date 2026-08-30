//! The one fetch / fuse / emit loop, shared by `HYBRID_SEARCH`,
//! `FULLTEXT_SEARCH` and `KNN`.
//!
//! # Why one loop
//!
//! There were two, over two index legs, and the RLS test file already says why
//! that is a trap: *"a fix applied to one and not the other leaves the hole
//! open."* It had already happened twice in this area (a hard-coded `"default"`
//! workspace on one side; `language: "en"` hard-coded on one side while the
//! other took the language as an argument). Three functions with three loops
//! would be the same bug with more surface.
//!
//! # What the loop fixes
//!
//! Before: fuse, `truncate(limit)`, fetch, RLS-filter, drop. A caller who may
//! read one workspace of twelve asked for 10 and received 1 -- indistinguishable
//! from "only one document matches". Truncation now happens on rows actually
//! EMITTED, and RLS drops and residual-predicate drops go through the same pass,
//! because they are the same phenomenon: a candidate that did not survive.

use std::collections::{HashMap, HashSet};

use async_stream::try_stream;
use indexmap::IndexMap;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::PermissionScope;
use raisin_sql::analyzer::{Literal, TypedExpr};
use raisin_storage::{NodeRepository, Storage, StorageScope};

use crate::physical_plan::executor::{ExecutionContext, ExecutionError, Row, RowStream};

use super::args::{parse_search_args, QueryInput, SearchArgs, SearchFunction};
use super::fusion::{fuse, FusedHit, HitKey, LegId, LegResult, VectorDetail, VectorDetails};
use super::legs::{
    embed_query, resolve_vector_partitions, run_fulltext_leg, run_vector_leg, shape_type_pushdown,
    LegContext,
};
use super::scope::{resolve_scope, WorkspaceSet};
use super::vector_of::{resolve_source, stored_vector_for_partition};
use super::{SEARCH_LEG_CAP, SEARCH_OVERFETCH};

/// Row-level security for the search table functions.
///
/// Neither the full-text index nor the HNSW vector index is permission-aware --
/// they answer "which nodes match these terms/this vector", not "which of them
/// may this caller read". A hit is therefore filtered exactly as the scan
/// executors filter theirs, through the SAME helper
/// (`scan_executors::helpers::rls_filter_node_graph`), so that graph
/// (`RELATES ... VIA`) conditions and per-permission field filtering behave
/// identically here and in `table_scan` / `vector_scan` / `FullTextScan`.
/// Returns `None` when the caller may not read the node.
///
/// `auth == None` means a system caller with no identity to filter against --
/// the convention every scan executor and `GRAPH_TABLE` already use. The
/// fail-closed behaviour lives in `rls_filter::filter_node`, which DENIES a node
/// no matching permission allowed.
///
/// This is the sole authority. A workspace set pushed into the index legs is an
/// upper bound on which workspaces could contribute; it says nothing about
/// whether any particular node in them is readable, because workspace is one of
/// four RLS dimensions (workspace, path, node_type, REL condition) plus field
/// filtering.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn rls_filter_search_hit<S: Storage>(
    storage: &S,
    node: Node,
    auth: Option<&AuthContext>,
    workspace_id: &str,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    max_revision: Option<&HLC>,
) -> Option<Node> {
    match auth {
        Some(auth) => {
            let scope = PermissionScope::new(workspace_id, branch);
            crate::physical_plan::scan_executors::helpers::rls_filter_node_graph(
                storage,
                node,
                auth,
                &scope,
                tenant_id,
                repo_id,
                branch,
                max_revision,
            )
            .await
        }
        None => Some(node),
    }
}

/// Everything an operator needs to tell "nothing matched" from "94 matches were
/// unreadable".
///
/// None of it reaches the caller. `emitted < limit` because nothing matched and
/// `emitted < limit` because the matches were unreadable MUST be
/// indistinguishable client-side, or the function becomes a differential oracle:
/// hold the scope, vary the query, count rows, enumerate documents you may not
/// read. That would be a worse leak than the one the per-hit RLS filter closed,
/// delivered through the fix's own diagnostics. This is the one place
/// least-privilege beats least-surprise on purpose.
#[derive(Debug, Default, Clone)]
pub struct SearchCounters {
    pub requested: usize,
    pub emitted: usize,
    pub leg_k: usize,
    pub redraws: usize,
    pub candidates: usize,
    pub dropped_permission: usize,
    pub dropped_residual: usize,
    pub dropped_missing_node: usize,
    pub dropped_no_workspace: usize,
    /// The reference node of a `VECTOR_OF(...)` query, dropped from its own
    /// results. Counted rather than silently skipped so a short answer can be
    /// explained: `LIMIT 10` over a corpus of exactly 10 assets returns 9.
    pub dropped_self: usize,
    pub legs_exhausted: bool,
}

/// Plan-time facts about a search, rendered by `EXPLAIN` and logged at INFO on
/// every execution.
#[derive(Debug, Clone)]
pub struct SearchPlanNote {
    pub function: &'static str,
    pub scope_spec: String,
    pub scope: String,
    pub catalog: usize,
    pub readable: usize,
    pub leg_k: usize,
    pub fulltext_weight: f64,
    pub vector_weight: f64,
    pub max_distance: f32,
    pub language: String,
    pub shape_types: Option<Vec<String>>,
    pub has_residual: bool,
    /// What the caller asked for: `text`, `image` or `all`.
    pub kind: &'static str,
    /// The partition tokens `kind` resolved to -- one vector leg each. Empty
    /// when the vector leg does not run at all.
    pub partitions: Vec<String>,
}

impl std::fmt::Display for SearchPlanNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} scope={} spec='{}' (catalog={}, readable={}) leg_k={} \
             weights=(ft {:.2}, vec {:.2}) max_distance={:.2} language='{}' \
             pushed_shape_types={:?} residual={} kind='{}' partitions={:?}",
            self.function,
            self.scope,
            self.scope_spec,
            self.catalog,
            self.readable,
            self.leg_k,
            self.fulltext_weight,
            self.vector_weight,
            self.max_distance,
            self.language,
            self.shape_types,
            if self.has_residual { "yes" } else { "no" },
            self.kind,
            self.partitions,
        )
    }
}

/// Analyse the arguments and resolve the corpus, without running anything.
///
/// Split out so `EXPLAIN` can print the resolved universe -- "which workspaces
/// did this search cover" used to be discoverable only as the ABSENCE of a
/// filter, which is exactly why it stayed unnoticed.
pub async fn plan_search<S: Storage + 'static>(
    function: SearchFunction,
    args: &[raisin_sql::analyzer::TableFunctionArg],
    residual: Option<&TypedExpr>,
    ctx: &ExecutionContext<S>,
) -> Result<(SearchArgs, WorkspaceSet, SearchPlanNote), ExecutionError> {
    let parsed = parse_search_args(function, args, &ctx.default_language)?;
    let (scope, note) = plan_resolved(&parsed, residual, ctx).await?;
    Ok((parsed, scope, note))
}

/// As [`plan_search`], but for a caller that already has a [`SearchArgs`] --
/// the HTTP and MCP surfaces, which never see SQL text.
pub async fn plan_resolved<S: Storage + 'static>(
    parsed: &SearchArgs,
    residual: Option<&TypedExpr>,
    ctx: &ExecutionContext<S>,
) -> Result<(WorkspaceSet, SearchPlanNote), ExecutionError> {
    let (scope, stats) = resolve_scope(
        &parsed.scope_spec,
        ctx.storage.as_ref(),
        &ctx.tenant_id,
        &ctx.repo_id,
        &ctx.branch,
        ctx.auth_context.as_ref(),
    )
    .await?;

    let shape_types = shape_type_pushdown(residual);

    // BEST EFFORT, for the note only. The authoritative resolution happens in
    // `execute_parsed`, at the point where the embedding provider is resolved,
    // so that a tenant with no embedding config still gets the error that names
    // `ALTER EMBEDDING CONFIG` and `vector_weight => 0` rather than the lower
    // level "no index partition to search". Same function, called twice; not a
    // second resolver.
    let partitions: Vec<String> = match (&ctx.hnsw_engine, parsed.runs_vector()) {
        (Some(engine), true) => resolve_vector_partitions(
            engine,
            &LegContext {
                tenant_id: ctx.tenant_id.to_string(),
                repo_id: ctx.repo_id.to_string(),
                branch: ctx.branch.to_string(),
                max_revision: ctx.max_revision,
            },
            parsed.kind,
        )
        .map(|ps| ps.iter().map(|p| p.to_string()).collect())
        .unwrap_or_default(),
        _ => Vec::new(),
    };

    let note = SearchPlanNote {
        function: parsed.function.name(),
        scope_spec: parsed.scope_spec_raw.clone(),
        scope: scope.describe(),
        catalog: stats.catalog,
        readable: stats.readable,
        leg_k: leg_k(parsed, &scope, residual),
        fulltext_weight: parsed.fulltext_weight,
        vector_weight: parsed.vector_weight,
        max_distance: parsed.max_distance,
        language: parsed.language.clone(),
        shape_types,
        has_residual: residual.is_some(),
        kind: parsed.kind.as_str(),
        partitions,
    };
    Ok((scope, note))
}

/// How wide to draw each leg.
///
/// The unfiltered fast path matters: `rls_filter_search_hit` returns everything
/// for `auth == None` / `is_system` / `is_system_admin`, so over-fetching for
/// those callers is pure waste. `limit` is validated `1..=1000` at parse time,
/// so this cannot exceed `SEARCH_LEG_CAP`.
fn leg_k(args: &SearchArgs, scope: &WorkspaceSet, residual: Option<&TypedExpr>) -> usize {
    let filtered = !(matches!(scope, WorkspaceSet::All) && residual.is_none());
    if filtered {
        (args.limit * SEARCH_OVERFETCH).min(SEARCH_LEG_CAP)
    } else {
        // Chunk-collapse headroom only.
        args.limit * 2
    }
}

/// Run a search and stream its rows.
pub async fn execute_search<S: Storage + 'static>(
    function: SearchFunction,
    args: &[raisin_sql::analyzer::TableFunctionArg],
    residual: Option<&TypedExpr>,
    table_name: String,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let parsed = parse_search_args(function, args, &ctx.default_language)?;
    execute_parsed(parsed, residual, table_name, ctx).await
}

/// Run a search whose arguments were built programmatically rather than parsed
/// from SQL. Same loop, same scope resolver, same RLS pass, same columns.
pub async fn execute_parsed<S: Storage + 'static>(
    parsed: SearchArgs,
    residual: Option<&TypedExpr>,
    table_name: String,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let function = parsed.function;
    let (scope, note) = plan_resolved(&parsed, residual, ctx).await?;
    tracing::info!(search = %note, "search plan");

    let tenant_id = ctx.tenant_id.to_string();
    let repo_id = ctx.repo_id.to_string();
    let branch = ctx.branch.to_string();
    let user_id = ctx
        .auth_context
        .as_ref()
        .and_then(|a| a.permissions().map(|p| p.user_id.clone()))
        .unwrap_or_else(|| "<system>".to_string());

    // Resolving to nothing means zero rows, one INFO line, and NO call into
    // either index. `Empty` is not `All`; getting that backwards would turn "may
    // read nothing" into "search everything".
    if scope.is_empty() {
        tracing::info!(
            function = note.function,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            user_id = %user_id,
            scope_spec = %note.scope_spec,
            catalog = note.catalog,
            readable = note.readable,
            "search scope resolved to no workspace; returning zero rows without \
             querying either index"
        );
        return Ok(Box::pin(futures::stream::empty()));
    }

    let indexing_engine = ctx.indexing_engine.clone();
    let hnsw_engine = ctx.hnsw_engine.clone();

    let wants_fulltext = parsed.runs_fulltext();
    let wants_vector = parsed.runs_vector();

    // A weight of 0 skips the leg ENTIRELY, and that includes embedding-provider
    // resolution. Otherwise a tenant with no embedder could not run a
    // deliberately full-text-only hybrid query -- strictly worse than before
    // this argument existed.
    let embedding_provider = if wants_vector && hnsw_engine.is_some() {
        match &parsed.query {
            // A supplied vector, and a stored one addressed by VECTOR_OF, both
            // need no provider at all. ONE predicate, shared with the
            // does-the-vector-leg-run guard below.
            q if q.needs_no_provider() => None,
            _ => {
                match ctx.resolve_embedding_provider().await {
                    Ok(Some(p)) => Some(p),
                    Ok(None) => {
                        // A vector index but no embedder is the PRODUCTION shape
                        // of the last bug here: rows came back with NULL
                        // vector_rank on every one and nothing was logged,
                        // indistinguishable from a working hybrid query whose
                        // vector leg matched nothing.
                        return Err(ExecutionError::Validation(format!(
                            "{} requires an embedding provider to embed the query, \
                             but this tenant has no enabled embedding \
                             configuration. Without it only the full-text half \
                             would run and the result would be reported as \
                             hybrid. Enable embeddings for the tenant (ALTER \
                             EMBEDDING CONFIG), set vector_weight => 0 to ask for \
                             keyword search deliberately, or use \
                             FULLTEXT_SEARCH.",
                            function.name()
                        )));
                    }
                    Err(e) => {
                        return Err(ExecutionError::Validation(format!(
                            "{} cannot embed the query: {e}. The vector half of \
                             this search is unavailable, so the result would be a \
                             plain full-text search reported as a hybrid one.",
                            function.name()
                        )));
                    }
                }
            }
        }
    } else {
        None
    };

    if wants_vector && hnsw_engine.is_none() && function == SearchFunction::Knn {
        return Err(ExecutionError::Validation(
            "KNN requires a vector index, and this server has none configured.".to_string(),
        ));
    }
    if wants_fulltext && indexing_engine.is_none() && function == SearchFunction::Fulltext {
        return Err(ExecutionError::Validation(
            "FULLTEXT_SEARCH requires a full-text indexing engine.".to_string(),
        ));
    }

    let storage = ctx.storage.clone();
    // Cloned out of the context BEFORE the stream, like every other dependency
    // the loop uses. VECTOR_OF reads `cf::EMBEDDINGS` through the trait object,
    // not through a second key builder on this side.
    let embedding_storage = ctx.embedding_storage.clone();
    let auth_context = ctx.auth_context.clone();
    let max_revision = ctx.max_revision;
    let residual_conjuncts: Vec<TypedExpr> = residual.map(split_and).unwrap_or_default();
    let shape_types = note.shape_types.clone();
    let leg_context = LegContext {
        tenant_id: tenant_id.clone(),
        repo_id: repo_id.clone(),
        branch: branch.clone(),
        max_revision,
    };

    // WHICH vector partitions -- resolved ONCE, here, and never per re-draw. One
    // partition is one leg; `kind => 'all'` is what makes that plural. Resolved
    // AFTER the provider check above so that a tenant with no embedding config
    // gets the error naming its own fix.
    //
    // Skipped entirely when the vector leg does not run, because resolution
    // reads the tenant's config, and `vector_weight => 0` must not trip a
    // configuration error for a leg the caller deliberately switched off.
    let vector_leg_runs = wants_vector
        && hnsw_engine.is_some()
        // Either the caller handed us a vector (or named a node holding one),
        // or we resolved a provider to make one. Neither means there is nothing
        // to search with.
        && (parsed.query.needs_no_provider() || embedding_provider.is_some());
    let vector_partitions: Vec<raisin_hnsw::PartitionId> = match (&hnsw_engine, vector_leg_runs) {
        (Some(engine), true) => resolve_vector_partitions(engine, &leg_context, parsed.kind)?,
        _ => Vec::new(),
    };

    let start_k = note.leg_k;
    let limit = parsed.limit;
    let language = parsed.language.clone();
    let max_distance = parsed.max_distance;
    let fulltext_weight = parsed.fulltext_weight;
    let vector_weight = parsed.vector_weight;
    let query = parsed.query.clone();
    let note_for_stream = note.clone();

    // WHICH vector each partition is searched with. Resolved once, not per
    // re-draw: a wider search is the same query.
    //
    // Resolved BEFORE the stream, not inside it, so that everything it can
    // refuse -- a reference naming no node, a node the caller may not read, a
    // source with several stored chunks -- is a plan-time error the caller sees
    // as a bad request. Raised from inside the stream it surfaced as a failure
    // to fetch a row, i.e. an HTTP 500 for what is a mistake in the query text.
    //
    // A LIST rather than one vector, because `VECTOR_OF` is genuinely
    // per-partition. `kind => 'all'` over a node with both a caption and an
    // image vector must search the text index with the text vector and the
    // image index with the image one; reusing a single vector across two
    // embedding spaces is precisely the failure `PartitionId` exists to
    // prevent -- every distance finite, every ranking plausible, nothing
    // logged. Text and supplied-literal queries repeat the same vector for
    // every partition, which is exactly the previous behaviour.
    let mut vector_legs: Vec<(raisin_hnsw::PartitionId, Vec<f32>)> = Vec::new();
    // The reference node of a VECTOR_OF query, so it can be kept out of its
    // own results. Held even when a partition had no vector for it: under
    // `kind => 'all'` the node can still surface from the OTHER leg.
    let mut exclude_key: Option<HitKey> = None;

    if hnsw_engine.is_some() && wants_vector {
        match &query {
            QueryInput::Vector(v) => {
                let normalised = raisin_hnsw::normalize_vector(v);
                for partition in &vector_partitions {
                    vector_legs.push((partition.clone(), normalised.clone()));
                }
            }
            QueryInput::Text(text) => {
                if let Some(provider) = &embedding_provider {
                    let embedded = embed_query(provider, text).await?;
                    for partition in &vector_partitions {
                        vector_legs.push((partition.clone(), embedded.clone()));
                    }
                }
            }
            QueryInput::StoredVector(reference) => {
                let embedding_storage = embedding_storage.clone().ok_or_else(|| {
                    ExecutionError::Validation(format!(
                        "{}: this server has no embedding store, so a node's \
                         stored vector cannot be read.",
                        reference.describe()
                    ))
                })?;
                let source = resolve_source(
                    reference,
                    &storage,
                    auth_context.as_ref(),
                    &tenant_id,
                    &repo_id,
                    &branch,
                    max_revision.as_ref(),
                )
                .await?;
                // SELF-EXCLUSION is armed from the RESOLVED identity, before
                // any vector is read. A reference whose vector is missing in
                // one partition must still not come back as its own nearest
                // neighbour through another.
                exclude_key = Some((source.workspace.clone(), source.node_id.clone()));

                for partition in &vector_partitions {
                    match stored_vector_for_partition(
                        reference,
                        &source,
                        &embedding_storage,
                        partition,
                        &tenant_id,
                        &repo_id,
                        &branch,
                    )? {
                        Some(v) => vector_legs.push((partition.clone(), v)),
                        None => tracing::info!(
                            reference = %reference.raw,
                            node_id = %source.node_id,
                            source_id = %source.source_id,
                            partition = %partition,
                            "VECTOR_OF: this source has no stored vector in this \
                             partition; that partition contributes no leg"
                        ),
                    }
                }

                if vector_legs.is_empty() {
                    // Every partition came back empty. An empty leg reported
                    // as a search is indistinguishable from a corpus that
                    // matched nothing, so say which node and which spaces.
                    return Err(ExecutionError::Validation(format!(
                        "{}: node '{}' has no stored vector in any of the \
                         partitions selected by kind => '{}' ({}). It may not \
                         have been embedded yet, or its embedding may live in \
                         a different space -- an image-only asset has no text \
                         vector.",
                        reference.describe(),
                        source.node_id,
                        note.kind,
                        note.partitions
                            .iter()
                            .map(|p| p.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    )));
                }
            }
        }
    }

    let stream = try_stream! {
        let mut counters = SearchCounters {
            requested: limit,
            leg_k: start_k,
            ..Default::default()
        };

        // Memoised verdicts. This is what makes the single permitted re-draw
        // cheap: the wider run re-returns the same prefix, and every hit in it
        // has already been decided.
        let mut decided: HashMap<HitKey, bool> = HashMap::new();
        let mut emitted_keys: HashSet<HitKey> = HashSet::new();
        // Bound on fetch + permission evaluations, so a hostile or pathological
        // corpus cannot turn one statement into an unbounded scan.
        let mut checks_left = limit.saturating_mul(10);
        let mut k = start_k;
        let mut redraws = 0usize;
        let mut emitted = 0usize;

        loop {
            // Every leg is rebuilt from scratch on every pass, and so is the
            // fusion. HNSW is approximate, so a wider search can reorder;
            // stitching ranks across two runs would make `vector_rank` a lie in
            // a column that says otherwise.
            let mut legs: Vec<LegResult> = Vec::with_capacity(1 + vector_partitions.len());
            let mut details = VectorDetails::new();

            if wants_fulltext {
                if let (Some(engine), Some(text)) = (&indexing_engine, query.as_text()) {
                    let results = run_fulltext_leg(
                        engine,
                        &leg_context,
                        &scope,
                        text,
                        &language,
                        shape_types.clone(),
                        k,
                    )?;
                    legs.push(LegResult {
                        leg: LegId::Fulltext,
                        weight: fulltext_weight,
                        ordered: results
                            .iter()
                            .map(|r| (r.workspace_id.clone(), r.node_id.clone()))
                            .collect(),
                        requested: k,
                    });
                }
            }

            // ONE leg per partition. `kind => 'text'` gives one and reproduces
            // the previous behaviour exactly; `kind => 'all'` gives several and
            // needs no new code path, which is the whole reason fusion takes a
            // list.
            if let Some(engine) = &hnsw_engine {
                for (partition, vector) in &vector_legs {
                    let results = run_vector_leg(
                        engine,
                        &leg_context,
                        &scope,
                        partition,
                        vector,
                        k,
                        max_distance,
                    )?;
                    let leg_id = LegId::Vector {
                        kind: partition.kind_char().unwrap_or('?'),
                        partition: partition.to_string(),
                    };
                    let mut ordered = Vec::with_capacity(results.len());
                    for result in &results {
                        let key = (result.workspace_id.clone(), result.node_id.clone());
                        details.insert(
                            (leg_id.clone(), key.clone()),
                            VectorDetail {
                                distance: result.distance,
                                chunk_index: result.chunk_index,
                            },
                        );
                        ordered.push(key);
                    }
                    legs.push(LegResult {
                        leg: leg_id,
                        weight: vector_weight,
                        ordered,
                        requested: k,
                    });
                }
            }

            let fused = fuse(&legs, &details);
            counters.candidates = counters.candidates.max(fused.len());

            for hit in fused {
                if emitted >= limit || checks_left == 0 {
                    break;
                }
                if decided.contains_key(&hit.key) {
                    continue;
                }
                // An index entry with no workspace cannot be fetched in the
                // right scope and cannot be permission-checked in the right
                // scope either, so it is DROPPED rather than fetched somewhere
                // arbitrary. Guessing a scope is how a node gets checked against
                // the wrong workspace's permissions.
                // SELF-EXCLUSION. "Find things similar to X" must not answer
                // with X. Its own vector is at distance 0 from itself, so
                // without this it is rank 1 in EVERY such query and a
                // `LIMIT 10` spends a slot restating the question.
                //
                // Applied HERE, in the same pass as the RLS and residual drops,
                // rather than by subtracting a row from the leg results: the
                // legs are re-run wider on a redraw, and a filter applied to
                // one draw and not the other is this codebase's signature bug.
                // `SEARCH_OVERFETCH` already draws wider than `limit`, so
                // dropping one candidate does not cost a row.
                if exclude_key.as_ref() == Some(&hit.key) {
                    counters.dropped_self += 1;
                    decided.insert(hit.key.clone(), false);
                    continue;
                }
                if hit.key.0.is_empty() {
                    counters.dropped_no_workspace += 1;
                    tracing::warn!(
                        node_id = %hit.key.1,
                        "search: index hit carries no workspace; dropping"
                    );
                    decided.insert(hit.key.clone(), false);
                    continue;
                }

                checks_left -= 1;
                let fetched = storage
                    .nodes()
                    .get(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &hit.key.0),
                        &hit.key.1,
                        max_revision.as_ref(),
                    )
                    .await?;

                let Some(node) = fetched else {
                    counters.dropped_missing_node += 1;
                    decided.insert(hit.key.clone(), false);
                    continue;
                };
                if node.path == "/" {
                    decided.insert(hit.key.clone(), false);
                    continue;
                }

                let node = match rls_filter_search_hit(
                    &*storage,
                    node,
                    auth_context.as_ref(),
                    &hit.key.0,
                    &tenant_id,
                    &repo_id,
                    &branch,
                    max_revision.as_ref(),
                )
                .await
                {
                    Some(node) => node,
                    None => {
                        counters.dropped_permission += 1;
                        decided.insert(hit.key.clone(), false);
                        continue;
                    }
                };

                let row = build_row(&table_name, &hit, &node);

                // The SAME loop. An RLS drop and a residual-predicate drop are
                // the same phenomenon -- a candidate that did not survive -- and
                // two loops over them would be this codebase's signature bug.
                if !residual_matches(&residual_conjuncts, &row) {
                    counters.dropped_residual += 1;
                    decided.insert(hit.key.clone(), false);
                    continue;
                }

                decided.insert(hit.key.clone(), true);
                emitted_keys.insert(hit.key.clone());
                emitted += 1;
                counters.emitted = emitted;
                yield row;
            }

            if emitted >= limit {
                break;
            }
            // "Both legs came back short" is the only evidence available that
            // there is nothing more to find, and it is now honest about BOTH
            // legs. The vector leg used to be optimistic: its workspace
            // restriction was a post-filter over an unfiltered walk, so a narrow
            // scope could starve while the index still held matches the walk
            // never visited. The walk is workspace-filtered inside usearch now
            // (`HnswIndex::search_scoped`), so a short vector leg means the
            // index really is out of in-scope neighbours. The one remaining
            // fallback logs itself as `filter_mode="post_filter"` — if you are
            // reading a short result and that line is present, selectivity is
            // back on the table.
            //
            // EVERY leg, not two named ones. A leg that did not run is not
            // evidence either way and simply is not in the list, so "all legs
            // came back short" is `all()` over what actually ran -- and stays
            // correct when a third leg joins.
            let legs_exhausted = legs.iter().all(|leg| leg.exhausted());
            counters.legs_exhausted = legs_exhausted;
            if legs_exhausted || redraws >= 1 || k >= SEARCH_LEG_CAP || checks_left == 0 {
                break;
            }
            // At most ONE re-draw. A second search at a larger k re-returns the
            // same prefix, so a third round buys almost nothing while costing a
            // third full graph walk.
            k = (k * 4).min(SEARCH_LEG_CAP);
            redraws += 1;
            counters.redraws = redraws;
            counters.leg_k = k;
        }

        // Exactly one line per statement, and only when short. Everything the
        // caller is deliberately NOT told lives here.
        if emitted < limit {
            tracing::warn!(
                function = note_for_stream.function,
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                branch = %branch,
                user_id = %user_id,
                scope_spec = %note_for_stream.scope_spec,
                scope = %note_for_stream.scope,
                catalog = note_for_stream.catalog,
                readable = note_for_stream.readable,
                kind = note_for_stream.kind,
                partitions = ?note_for_stream.partitions,
                requested = counters.requested,
                emitted = counters.emitted,
                leg_k = counters.leg_k,
                redraws = counters.redraws,
                candidates = counters.candidates,
                dropped_permission = counters.dropped_permission,
                dropped_residual = counters.dropped_residual,
                dropped_missing_node = counters.dropped_missing_node,
                dropped_no_workspace = counters.dropped_no_workspace,
                dropped_self = counters.dropped_self,
                legs_exhausted = counters.legs_exhausted,
                "search returned fewer rows than requested"
            );
        }
    };

    Ok(Box::pin(stream))
}

/// Split a predicate into top-level `AND` conjuncts.
fn split_and(expr: &TypedExpr) -> Vec<TypedExpr> {
    use raisin_sql::analyzer::{BinaryOperator, Expr};
    match &expr.expr {
        Expr::BinaryOp { left, op, right } if *op == BinaryOperator::And => {
            let mut out = split_and(left);
            out.extend(split_and(right));
            out
        }
        _ => vec![expr.clone()],
    }
}

/// Does this row satisfy every residual conjunct?
///
/// NULL counts as "no", matching `execute_filter`. This is the SAME predicate
/// the authoritative `Filter` above the table function evaluates; running it
/// here as well is what makes `limit` mean "rows delivered" instead of
/// "candidates budgeted", and it is idempotent.
fn residual_matches(conjuncts: &[TypedExpr], row: &Row) -> bool {
    conjuncts.iter().all(|predicate| {
        matches!(
            crate::physical_plan::eval::eval_expr(predicate, row),
            Ok(Literal::Boolean(true))
        )
    })
}

/// The unified column set, in the fixed order the table definitions declare.
fn build_row(table_name: &str, hit: &FusedHit, node: &Node) -> Row {
    let mut columns: IndexMap<String, PropertyValue> = IndexMap::new();
    let mut put = |name: &str, value: PropertyValue| {
        columns.insert(format!("{table_name}.{name}"), value);
    };

    put("node_id", PropertyValue::String(hit.key.1.clone()));
    put("workspace_id", PropertyValue::String(hit.key.0.clone()));
    put("name", PropertyValue::String(node.name.clone()));
    put("path", PropertyValue::String(node.path.clone()));
    put("node_type", PropertyValue::String(node.node_type.clone()));
    put("score", PropertyValue::Float(hit.score));
    put("fulltext_rank", opt_int(hit.fulltext_rank()));
    put("vector_rank", opt_int(hit.vector_rank()));
    put(
        "vector_distance",
        hit.vector_distance
            .map(|d| PropertyValue::Float(d as f64))
            .unwrap_or(PropertyValue::Null),
    );
    // NULL = no vector hit; 0 = a document that was never chunked. A RAG caller
    // needs it to know WHERE in a long document the answer lives.
    put("chunk_index", opt_int(hit.chunk_index));
    // Which embedding space produced the vector hit. NULL exactly when
    // `vector_rank` is NULL. Without it `kind => 'all'` is uninterpretable: two
    // towers fused into one ranking with nothing saying which one matched, and
    // a `vector_distance` whose scale silently depends on the answer.
    put(
        "embedding_kind",
        match hit.embedding_kind {
            Some('T') => PropertyValue::String("text".to_string()),
            Some('I') => PropertyValue::String("image".to_string()),
            Some(other) => PropertyValue::String(other.to_string()),
            None => PropertyValue::Null,
        },
    );
    put("revision", PropertyValue::Integer(node.version as i64));
    put(
        "created_at",
        node.created_at
            .map(|t| PropertyValue::String(t.to_rfc3339()))
            .unwrap_or(PropertyValue::Null),
    );
    put(
        "updated_at",
        node.updated_at
            .map(|t| PropertyValue::String(t.to_rfc3339()))
            .unwrap_or(PropertyValue::Null),
    );
    // Post-RLS: the field filter of the permission that granted access has
    // already been applied to this bag by `rls_filter_search_hit`.
    put("properties", PropertyValue::Object(node.properties.clone()));

    Row { columns }
}

fn opt_int(value: Option<usize>) -> PropertyValue {
    value
        .map(|v| PropertyValue::Integer(v as i64))
        .unwrap_or(PropertyValue::Null)
}
