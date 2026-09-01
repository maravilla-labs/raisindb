// SPDX-License-Identifier: BSL-1.1

//! Search operations for the HNSW indexing engine.
//!
//! Provides nearest-neighbor search with workspace filtering, distance thresholds,
//! chunk-aware search modes, document deduplication, and position-based scoring.

use crate::index::{HnswIndex, ScopeFilterMode, ScopedSearch};
use crate::partition::PartitionId;
use crate::types::{
    deduplicate_by_document, ChunkSearchResult, DocumentSearchResult, ScoringConfig, SearchMode,
    SearchRequest, SearchResult, DEFAULT_MAX_DISTANCE, MAX_FETCH_K,
};
use raisin_error::Result;

use super::HnswIndexingEngine;

/// Size of the FIRST draw when the result set will be collapsed to documents.
///
/// The index holds one entry per CHUNK, so several entries can belong to the
/// same document and collapse into one row. Fetching exactly `k` would then
/// return fewer than `k` DOCUMENTS whenever any of them is chunked.
///
/// It is an opening bid, NOT a guarantee. No fixed multiplier can promise `k`
/// distinct documents: whatever the factor, a document with more chunks than
/// that defeats it and `LIMIT k` comes back holding one document. See
/// `search_documents_adaptive`, which escalates from here — the escalation is
/// the correctness mechanism; this constant only decides where it starts.
///
/// This is the only over-draw the scoped paths need for the collapse. The old
/// workspace over-draw (a flat 5x/10x on top of this) existed to compensate for
/// post-filtering an unfiltered walk; the walk is now filtered inside the index,
/// so drawing more candidates buys nothing but work. See
/// `HnswIndex::search_scoped`.
pub(super) const CHUNK_COLLAPSE_HEADROOM: usize = 2;

/// How many times the adaptive draw may double before it gives up.
///
/// Bounds the work one pathological document (chunked into far more pieces than
/// the caller's limit, all of them nearer the query than anything else) can
/// cost: at most `MAX_FETCH_ESCALATIONS + 1` graph walks, and never a draw past
/// `MAX_FETCH_K`. Six draws span `k * 2` to `k * 64`; past that the honest
/// answer is that the neighbourhood really is that lopsided, and returning short
/// beats walking the whole graph on every query.
pub(super) const MAX_FETCH_ESCALATIONS: usize = 5;

/// How many candidates the raw-candidate diagnostic dump prints before it
/// summarises the rest.
///
/// The dump is ONE LINE PER CANDIDATE of the final draw. That was survivable
/// while the draw was a flat `k * CHUNK_COLLAPSE_HEADROOM`; it is not now that
/// the draw escalates, because the final `fetch_k` can be 64x the opening bid
/// and is bounded only by `MAX_FETCH_K` — 2000 lines for ONE user query, on a
/// hot path. Dropping the dump to `debug!` (which is what it always was in
/// kind: an operator diagnostic, not an event) fixes the LEVEL; this fixes the
/// VOLUME, which a level alone does not — an operator who turns debug on to
/// investigate one slow query should not be handed 2000 lines per query.
///
/// The head is the part that carries information: candidates arrive
/// distance-ascending, so the nearest few are exactly the ones the collapse and
/// the distance filter act on. The tail is uniform and farther, and the elision
/// line reports how much of it there was.
pub(super) const CANDIDATE_DUMP_LIMIT: usize = 20;

/// Fetch candidates for one workspace-scoped search.
///
/// Both of this crate's search paths (`search_with_threshold` and
/// `search_chunks`) go through here. They previously carried mirrored copies of
/// the fetch sizing and the workspace post-filter, which is precisely how the
/// two drifted: one grew a distance-threshold override and a diagnostic dump,
/// the other did not.
///
/// The short-result accounting lives next door in `log_fetch_shortfall` rather
/// than inside this function, because a document search now calls this SEVERAL
/// times for one logical query (see `search_documents_adaptive`) and the
/// post-filter warning must not be shouted once per escalation. Callers draw as
/// often as they need and log exactly once, for the final draw.
/// Draw the k nearest CHUNKS, without collapsing them to documents.
///
/// The document draw escalates because a collapse can turn many chunks into few
/// documents and it still owes the caller `k` of them. Nothing collapses here,
/// so one draw of exactly `k` is right and over-drawing would only fetch rows
/// the truncate throws away.
///
/// Row-level security is NOT applied here — it is applied above, per node, by
/// the emit loop. A caller that needs headroom for it must ask for more.
fn draw_chunks(
    index: &HnswIndex,
    query: &[f32],
    k: usize,
    workspaces: &[String],
    partition: &PartitionId,
    max_distance: f32,
) -> Result<Vec<SearchResult>> {
    let fetch_k = k.min(MAX_FETCH_K);
    let scoped = fetch_scoped(index, query, fetch_k, workspaces)?;
    log_fetch_shortfall(&scoped, fetch_k, workspaces, partition);

    let mut results = scoped.results;
    results.retain(|r| r.distance < max_distance);
    results.truncate(k);
    Ok(results)
}

fn fetch_scoped(
    index: &HnswIndex,
    query: &[f32],
    fetch_k: usize,
    workspaces: &[String],
) -> Result<ScopedSearch> {
    index.search_scoped(query, fetch_k, workspaces)
}

/// Account for a short result IN ONE PLACE, once per logical query.
///
/// `requested` is the LARGEST draw the query ended up making, so an escalated
/// document search reports the size it actually gave up at rather than its
/// opening bid.
fn log_fetch_shortfall(
    scoped: &ScopedSearch,
    requested: usize,
    workspaces: &[String],
    partition: &PartitionId,
) {
    if scoped.results.len() >= requested {
        return;
    }

    match scoped.mode {
        // The misattribution this whole path exists to prevent. An operator
        // reading "returned 0 of 10" next to a workspace list concludes
        // "permissions dropped my rows"; on this branch the truth is that
        // the ANN walk never visited the workspace's region of the graph.
        ScopeFilterMode::PostFilter => tracing::warn!(
            filter_mode = "post_filter",
            partition = %partition,
            requested,
            returned = scoped.results.len(),
            drawn = scoped.drawn,
            in_scope_vectors = scoped.in_scope_total,
            workspaces = ?workspaces,
            "vector search came back SHORT after a post-filter: the graph walk was \
             UNFILTERED and the workspace restriction was applied to its output, so this \
             is INDEX SELECTIVITY (the walk never reached this scope), NOT a permission \
             drop and NOT an empty index — the scope holds `in_scope_vectors` vectors"
        ),
        // Same shortfall, opposite meaning: the walk WAS scoped, so this is
        // the index's honest answer.
        ScopeFilterMode::IndexSide => tracing::debug!(
            filter_mode = "index_side",
            partition = %partition,
            requested,
            returned = scoped.results.len(),
            in_scope_vectors = scoped.in_scope_total,
            workspaces = ?workspaces,
            "vector search came back short from a workspace-filtered walk: the scope holds \
             `in_scope_vectors` vectors in total, so this is the index's complete answer, \
             not starvation"
        ),
        ScopeFilterMode::Unrestricted => tracing::debug!(
            filter_mode = "unrestricted",
            partition = %partition,
            requested,
            returned = scoped.results.len(),
            index_vectors = scoped.in_scope_total,
            "vector search came back short over the whole index"
        ),
    }
}

/// Which of the adaptive loop's four exits held when it stopped.
///
/// Recorded because THE RETURNED ROWS CANNOT TELL THEM APART. A query that
/// stops at `threshold_cut` on its opening draw and one that grinds through six
/// escalating draws to `exhausted` return byte-identical results, so a test
/// asserting only on rows stays green with the early exit DELETED — which is
/// exactly what
/// `a_threshold_cut_stops_the_escalation_instead_of_grinding_to_the_cap` did
/// before this struct existed (its only escalation assertion was a wall-clock
/// smoke alarm a six-draw walk over 27 vectors passes trivially). The work done
/// is the observable; so make it observable.
///
/// More than one flag can be true at once — an exhausted draw that also cut on
/// distance, say — so all four are recorded rather than one collapsed "reason".
/// A test that means to exercise ONE exit can then assert it fired ALONE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FetchExit {
    /// The collapse produced `k` documents.
    pub(crate) enough: bool,
    /// The draw came back shorter than it asked for: no more in-scope
    /// neighbours exist.
    pub(crate) exhausted: bool,
    /// The distance filter removed at least one candidate, so every candidate a
    /// larger draw could add is farther and would be cut too.
    pub(crate) threshold_cut: bool,
    /// The work bound bit: `MAX_FETCH_ESCALATIONS` doublings, or `MAX_FETCH_K`.
    pub(crate) capped: bool,
}

/// The outcome of one logical document search.
pub(crate) struct DocumentFetch {
    /// One row per source document, nearest first, at most `k` of them.
    pub(crate) documents: Vec<SearchResult>,
    /// Candidates the final (largest) draw produced, before the distance filter.
    pub(crate) drawn: usize,
    /// How many of those the distance filter removed.
    pub(crate) filtered_out: usize,
    /// How many times the draw doubled. `0` means the opening bid ended it.
    pub(crate) escalations: usize,
    /// Why the loop stopped. See `FetchExit`.
    pub(crate) exit: FetchExit,
}

/// Draw, filter, collapse — escalating the draw until the collapse actually
/// yields `k` DOCUMENTS, or there is provably nothing more to gain.
///
/// # Why this is a loop and not a bigger constant
///
/// The index holds one entry per chunk, and `deduplicate_by_document` collapses
/// every chunk of a document into a single row. A fixed over-draw of `k * N` is
/// therefore defeated by any document with more than `N` chunks near the query:
/// its chunks fill the whole draw, collapse to ONE row, and a `LIMIT k` returns
/// one document. Raising `N` moves the threshold; it does not remove it. Only
/// escalating until the COLLAPSED count reaches `k` — or until the index is out
/// of candidates — makes `k` mean "k documents".
///
/// # The four ways out, and why each is sound
///
/// * `enough` — the collapse produced `k` documents. Done.
/// * `exhausted` — the draw came back with fewer rows than it asked for, so the
///   index has no more in-scope neighbours to give. (In the `PostFilter`
///   fallback a short draw can also mean the post-filter ate rows that a larger
///   over-draw might have replaced. Stopping there is deliberate: that
///   fallback's starvation is a known, documented and separately reported
///   property — see `ScopeFilterMode` — and escalating against it would spend
///   six graph walks to still come back short.)
/// * `threshold_cut` — LOAD-BEARING. Candidates arrive distance-ascending, so
///   once the distance filter has removed anything, every candidate a larger
///   draw could add is strictly farther and would be cut too. Without this
///   check a query whose neighbours are genuinely all beyond the threshold
///   escalates to `MAX_FETCH_K` on EVERY call, turning a correctness fix into a
///   latency bug.
/// * `capped` — the bound on total work: at most `MAX_FETCH_ESCALATIONS`
///   doublings, and never a draw past `MAX_FETCH_K`.
fn search_documents_adaptive(
    index: &HnswIndex,
    query: &[f32],
    k: usize,
    workspaces: &[String],
    partition: &PartitionId,
    threshold: f32,
) -> Result<DocumentFetch> {
    let mut fetch_k = k.saturating_mul(CHUNK_COLLAPSE_HEADROOM).min(MAX_FETCH_K);
    let mut escalations = 0usize;

    loop {
        let scoped = fetch_scoped(index, query, fetch_k, workspaces)?;

        // THE ORDERING INVARIANT the `threshold_cut` exit below rests on, made
        // explicit and checked.
        //
        // `HnswIndex::search_scoped` returns candidates non-decreasing by
        // distance on all three of its paths — usearch orders its own matches,
        // `hydrate` preserves that order, and the post-filter fallback only
        // `retain`s and truncates, neither of which reorders. But NOTHING in
        // the crate enforced it, and neighbouring code deliberately does not
        // rely on it. A future post-filter, re-rank or merge step could quietly
        // break the order, and the failure would be SILENT: `threshold_cut`
        // would stop the escalation on a candidate that merely happened to be
        // far, truncating correct nearer answers with no error and no log
        // line — just missing rows.
        //
        // So: asserted on every draw, in every debug and test build, free in
        // release. Pinned from the other side by
        // `search_scoped_returns_distance_ordered_candidates`, which asserts the
        // order for the index-side path, the post-filter fallback and the
        // unrestricted walk.
        debug_assert!(
            scoped
                .results
                .windows(2)
                .all(|w| w[0].distance <= w[1].distance),
            "search_scoped returned candidates OUT OF DISTANCE ORDER. The threshold_cut \
             early exit in search_documents_adaptive is sound only while they arrive \
             non-decreasing; unordered, it silently truncates correct answers. \
             distances: {:?}",
            scoped
                .results
                .iter()
                .map(|r| r.distance)
                .collect::<Vec<_>>()
        );

        // Asked for `fetch_k` and got fewer: the index has nothing more in scope.
        let exhausted = scoped.results.len() < fetch_k;

        let drawn = scoped.results.len();
        // Filtered OUT OF PLACE so the raw candidate list stays available for
        // the diagnostic dump below, which has to show the entries before the
        // collapse — that is the only view in which a chunked document looks
        // like the several index rows it really occupies.
        let kept: Vec<SearchResult> = scoped
            .results
            .iter()
            .filter(|r| r.distance < threshold)
            .cloned()
            .collect();
        let filtered_out = drawn - kept.len();
        let threshold_cut = filtered_out > 0;

        let documents = deduplicate_by_document(kept, k);

        let enough = documents.len() >= k;
        let capped = fetch_k >= MAX_FETCH_K || escalations >= MAX_FETCH_ESCALATIONS;

        // `threshold_cut` is sound ONLY under the non-decreasing distance order
        // asserted at the top of this loop: once the filter has cut one
        // candidate, every candidate a larger draw could add is strictly
        // farther and would be cut too, so escalating buys nothing. Take that
        // ordering away and this stops being an optimisation and becomes silent
        // truncation. The invariant itself is pinned by
        // `search_scoped_returns_distance_ordered_candidates`; this exit's
        // behaviour is pinned by
        // `a_threshold_cut_stops_the_escalation_instead_of_grinding_to_the_cap`,
        // which asserts on the ESCALATION COUNT — the returned rows cannot
        // distinguish this exit from `exhausted`.
        if enough || exhausted || threshold_cut || capped {
            // Log the candidates before filtering, for debugging.
            //
            // `chunk_id` as well as `node_id`: this line is the operator's view
            // of what the index actually holds, and after the chunk collapse
            // several consecutive entries share one `node_id`. Printing only the
            // source id would make five distinct chunk vectors look like one
            // entry repeated five times.
            //
            // Emitted for the FINAL draw only. The escalating draws are ONE
            // logical query, and each is a superset of the last, so a dump per
            // attempt would say the same thing at ever greater length.
            //
            // At `debug!`, and capped at `CANDIDATE_DUMP_LIMIT`. This was one
            // `info!` per candidate: bounded by `k * CHUNK_COLLAPSE_HEADROOM`
            // before the adaptive draw landed, and measured at 206 INFO lines
            // for a single `LIMIT 5` query over a 200-chunk fixture after it,
            // with a worst case of `MAX_FETCH_K`. It is an operator diagnostic,
            // not an event. The SHORTFALL line below IS the event — once per
            // logical query, and it keeps its levels.
            tracing::debug!(
                fetch_k,
                escalations,
                candidates = scoped.results.len(),
                "Vector search raw results (before distance filtering):"
            );
            for (i, result) in scoped.results.iter().take(CANDIDATE_DUMP_LIMIT).enumerate() {
                tracing::debug!(
                    "  [{}] entry={} node={} workspace={} distance={:.4}",
                    i + 1,
                    result.chunk_id,
                    result.node_id,
                    result.workspace_id,
                    result.distance
                );
            }
            let elided = scoped.results.len().saturating_sub(CANDIDATE_DUMP_LIMIT);
            if elided > 0 {
                tracing::debug!(
                    "  ... {} further candidates elided (dump capped at {}; every one of them \
                     is farther than the lines above)",
                    elided,
                    CANDIDATE_DUMP_LIMIT
                );
            }

            // Once, for the draw the query actually gave up at — not once per
            // escalation. An operator reading the post-filter warning must see
            // one line per logical query or the count itself becomes misleading.
            log_fetch_shortfall(&scoped, fetch_k, workspaces, partition);

            // The one short result `log_fetch_shortfall` CANNOT report. At the
            // cap the draw came back FULL (`fetch_k` of `fetch_k`), so that
            // function returns early and says nothing at all — leaving a
            // `LIMIT k` that returned fewer than k documents with no
            // explanation anywhere. That is the same misattribution the
            // post_filter/index_side split exists to prevent, one level up:
            // the index is neither empty nor filtering by permission, the
            // neighbourhood is simply dominated by chunks of a few documents.
            if capped && !enough {
                tracing::warn!(
                    partition = %partition,
                    requested_documents = k,
                    returned_documents = documents.len(),
                    drawn,
                    escalations,
                    workspaces = ?workspaces,
                    "vector search hit the draw cap before collecting `requested_documents` \
                     distinct documents: `drawn` candidates collapsed to `returned_documents`, \
                     so this is CHUNK CROWDING — not an empty index, not a permission drop. \
                     A document with more chunks near the query than the cap can draw will \
                     crowd out others; raising MAX_FETCH_ESCALATIONS trades latency for recall"
                );
            }

            if filtered_out > 0 {
                tracing::info!(
                    "Filtered out {} results with distance >= {:.2}",
                    filtered_out,
                    threshold
                );
            }

            return Ok(DocumentFetch {
                documents,
                drawn,
                filtered_out,
                escalations,
                exit: FetchExit {
                    enough,
                    exhausted,
                    threshold_cut,
                    capped,
                },
            });
        }

        escalations += 1;
        fetch_k = fetch_k.saturating_mul(2).min(MAX_FETCH_K);
    }
}

impl HnswIndexingEngine {
    /// Search for nearest neighbors.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspaces` - Workspace filter. EMPTY means "every workspace"; a
    ///   non-empty slice restricts to exactly those.
    /// * `query` - Query vector
    /// * `k` - Number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results ordered by distance (closest first)
    ///
    /// # Workspace filtering happens INSIDE the graph walk
    ///
    /// usearch 2.24 takes a per-candidate predicate (`filtered_search`), so the
    /// workspace restriction is applied during the walk, not to its output. The
    /// walk keeps navigating through out-of-scope nodes and keeps going until it
    /// has `k` IN-SCOPE neighbours. A narrow scope inside a large index is
    /// therefore answered in full.
    ///
    /// It used to be a POST-filter over a fixed 10x over-draw, and a narrow
    /// scope came back short — often empty — because the unfiltered walk never
    /// visited that region of the graph. From the outside that was
    /// indistinguishable from permission filtering, which is what made it so
    /// expensive to diagnose. See `HnswIndex::search_scoped`, and
    /// `ScopeFilterMode` for how the one remaining fallback reports itself.
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        workspaces: &[String],
        query: &[f32],
        k: usize,
    ) -> Result<Vec<SearchResult>> {
        self.search_with_threshold(
            tenant_id, repo_id, branch, partition, workspaces, query, k, None,
        )
    }

    /// The k nearest CHUNKS, NOT collapsed to one row per document.
    ///
    /// Same arguments and same threshold semantics as
    /// [`Self::search_with_threshold`]; the difference is only that several
    /// results may share a `node_id`, each being a different passage of that
    /// document. That is what `granularity => 'chunk'` needs and what a RAG
    /// caller filling a context window wants.
    ///
    /// Returns `SearchResult` rather than `ChunkSearchResult` deliberately: the
    /// latter drops `spec` and flattens `total_chunks` to 1, and the caller
    /// needs the spec to address the chunk's stored row.
    #[allow(clippy::too_many_arguments)]
    pub fn search_chunks_with_threshold(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        workspaces: &[String],
        query: &[f32],
        k: usize,
        max_distance: Option<f32>,
    ) -> Result<Vec<SearchResult>> {
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch, partition)?;
        let index = index_arc.read().unwrap();
        draw_chunks(
            &index,
            query,
            k,
            workspaces,
            partition,
            max_distance.unwrap_or(DEFAULT_MAX_DISTANCE),
        )
    }

    /// Search for nearest neighbors with an optional distance threshold override.
    ///
    /// If `max_distance` is `None`, uses the default threshold (0.6 for cosine).
    /// Pass `Some(threshold)` to override per-query (e.g., from SQL WHERE clause
    /// or tenant configuration).
    #[allow(clippy::too_many_arguments)]
    pub fn search_with_threshold(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        workspaces: &[String],
        query: &[f32],
        k: usize,
        max_distance: Option<f32>,
    ) -> Result<Vec<SearchResult>> {
        Ok(self
            .search_with_threshold_stats(
                tenant_id,
                repo_id,
                branch,
                partition,
                workspaces,
                query,
                k,
                max_distance,
            )?
            .documents)
    }

    /// `search_with_threshold`, keeping the draw statistics the public form
    /// drops.
    ///
    /// Exists for the tests, and deliberately: the adaptive loop's four exits
    /// are INDISTINGUISHABLE from the rows it returns (see `FetchExit`), so a
    /// test that can only see `documents` cannot tell an early `threshold_cut`
    /// from a six-draw grind to `exhausted` — and therefore cannot fail when
    /// the early exit is deleted. Everything callers actually need is in
    /// `documents`; the counters are how the behaviour is pinned.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn search_with_threshold_stats(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        workspaces: &[String],
        query: &[f32],
        k: usize,
        max_distance: Option<f32>,
    ) -> Result<DocumentFetch> {
        let start = std::time::Instant::now();
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch, partition)?;

        let index = index_arc.read().unwrap();

        // The cutoff is `crate::types::DEFAULT_MAX_DISTANCE` — one declaration,
        // shared with `search_chunks` below.
        let threshold = max_distance.unwrap_or(DEFAULT_MAX_DISTANCE);

        // Draw, distance-filter and collapse chunks to source documents, keeping
        // each document's best chunk and escalating the draw until there really
        // are k DOCUMENTS. The workspace restriction is applied inside the walk,
        // so it needs no over-draw of its own. An EMPTY `workspaces` slice means
        // "no filter"; it is never "match nothing" — a caller that resolved to
        // "nothing readable" must not reach this function at all.
        //
        // The collapse is the whole reason the chunk split lives in the engine
        // rather than in each caller. The index files a chunked document under
        // `{node_id}#{chunk_index}`, an id that exists nowhere outside the
        // index: `storage.nodes().get(scope, "abc#3")` is `None`. Every caller
        // of this method fetches the node by `SearchResult::node_id`, so before
        // this every vector hit on a long document produced no row at all while
        // still consuming a result slot — the vector half of HYBRID_SEARCH was
        // silently dead for exactly the documents RAG exists to retrieve, and
        // an unchunked document's `abc` could never fuse with a chunked one's
        // `abc#0`. `node_id` is now the source id (see `SearchResult`), so the
        // fix reaches all four consumers without any of them knowing about
        // chunking.
        let fetched =
            search_documents_adaptive(&index, query, k, workspaces, partition, threshold)?;

        // Record metrics
        self.metrics
            .record_search(start.elapsed(), fetched.documents.len());

        tracing::info!(
            drawn = fetched.drawn,
            filtered_out = fetched.filtered_out,
            escalations = fetched.escalations,
            "Returning {} vector search results (after filtering and limit)",
            fetched.documents.len()
        );

        Ok(fetched)
    }

    /// Search for nearest neighbors using a SearchRequest.
    ///
    /// This is the chunk-aware search API that supports both:
    /// - `SearchMode::Chunks`: Returns all matching chunks
    /// - `SearchMode::Documents`: Returns best chunk per document (deduplicated)
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `request` - Search request with mode and filters
    ///
    /// # Returns
    ///
    /// Vector of chunk search results ordered by distance or adjusted_score (closest first)
    pub fn search_chunks(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        request: &SearchRequest,
    ) -> Result<Vec<ChunkSearchResult>> {
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch, partition)?;
        let index = index_arc.read().unwrap();

        // Apply distance threshold (use custom or the shared default)
        let max_distance = request.max_distance.unwrap_or(DEFAULT_MAX_DISTANCE);

        // Sizing depends on the MODE, and only Documents mode collapses. The
        // workspace restriction costs nothing extra now that it is applied
        // inside the walk.
        let final_results = match request.mode {
            SearchMode::Chunks => draw_chunks(
                &index,
                &request.query_vector,
                request.k,
                &request.workspace_filters,
                partition,
                max_distance,
            )?,
            SearchMode::Documents => {
                // The SAME adaptive draw as `search_with_threshold`, including
                // its short-result accounting and its escalation. These two are
                // this crate's mirrored search paths; they share the one
                // implementation so they cannot drift again.
                search_documents_adaptive(
                    &index,
                    &request.query_vector,
                    request.k,
                    &request.workspace_filters,
                    partition,
                    max_distance,
                )?
                .documents
            }
        };

        // Convert to ChunkSearchResult
        let mut chunk_results: Vec<ChunkSearchResult> = final_results
            .into_iter()
            .map(|r| ChunkSearchResult::from_search_result(r, 1, None))
            .collect();

        // Apply scoring if configured
        if let Some(scoring_config) = &request.scoring {
            apply_scoring(&mut chunk_results, scoring_config);
        }

        Ok(chunk_results)
    }

    /// Search for nearest neighbors and return document results (deduplicated).
    ///
    /// This is a convenience method that always uses `SearchMode::Documents`.
    /// It returns one result per source document, choosing the best matching chunk.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `request` - Search request (mode will be overridden to Documents)
    ///
    /// # Returns
    ///
    /// Vector of document search results ordered by distance (closest first)
    pub fn search_documents(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        request: &SearchRequest,
    ) -> Result<Vec<DocumentSearchResult>> {
        // Force Documents mode
        let mut doc_request = request.clone();
        doc_request.mode = SearchMode::Documents;

        // Get chunk results
        let chunk_results =
            self.search_chunks(tenant_id, repo_id, branch, partition, &doc_request)?;

        // Convert to DocumentSearchResult
        let doc_results = chunk_results
            .into_iter()
            .map(DocumentSearchResult::from_chunk_result)
            .collect();

        Ok(doc_results)
    }
}

/// Apply scoring configuration to chunk search results.
///
/// This function adjusts the similarity scores based on chunk position and other factors,
/// then re-sorts results by the adjusted score instead of raw distance.
///
/// # Arguments
///
/// * `results` - Mutable reference to chunk search results
/// * `config` - Scoring configuration
fn apply_scoring(results: &mut [ChunkSearchResult], config: &ScoringConfig) {
    for result in results.iter_mut() {
        // Start with base similarity score (convert distance to similarity)
        let mut score = result.similarity();

        // Apply position decay: earlier chunks score higher
        // position_factor decreases linearly with chunk_index
        let position_factor = 1.0 - (config.position_decay * result.chunk_index as f32);
        score *= position_factor.max(0.5); // Don't decay below 50%

        // Apply first chunk boost
        if result.chunk_index == 0 {
            score *= config.first_chunk_boost;
        }

        // Store adjusted score
        result.adjusted_score = Some(score);
    }

    // Re-sort by adjusted score (higher is better)
    results.sort_by(|a, b| {
        let score_a = a.adjusted_score.unwrap_or(a.similarity());
        let score_b = b.adjusted_score.unwrap_or(b.similarity());
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}
