//! Real fulltext index rebuild + reconcile.
//!
//! The legacy `TantivyManagement::rebuild_index` was a stub that
//! `remove_dir_all`'d the index and returned `items_processed: 0`
//! without re-indexing anything. This module replaces it with a
//! rebuild that mirrors the shape of `vector::HnswManagement::rebuild_index`:
//!
//! 1. Acquire the per-(tenant,repo,branch) `IndexLockManager` lock so
//!    in-flight batch indexing serializes against the rebuild instead
//!    of racing the directory.
//! 2. Invalidate the Tantivy engine's cached `Index`/`IndexReader`
//!    handles for the key.
//! 3. `remove_dir_all` the on-disk index directory.
//! 4. Iterate every workspace under `(tenant, repo, branch)`, scan
//!    nodes, and call `engine.do_batch_index` in chunks.
//! 5. Return honest `RebuildStats` with the real `items_processed`.
//!
//! Reconcile (see `reconcile_fulltext_index`) is the v0.1.29 recovery
//! lever for tenants whose indexes were corrupted by v0.1.28's
//! aggregator-AND-gate bug. It does the same end-to-end re-index but
//! without first deleting the directory, so it's safe to run on a
//! healthy index too — Tantivy's `delete_term + add_document` makes
//! the per-node ops idempotent.

use crate::jobs::IndexKey;
use crate::management::async_indexing::helpers::scan_nodes;
use crate::management::helpers::{list_branches, list_workspaces};
use crate::storage::RocksDBStorage;
use raisin_error::{Error, Result};
use raisin_indexer::tantivy_engine::{BatchIndexContext, TantivyIndexingEngine};
use raisin_storage::{IndexType, RebuildStats};
use std::sync::Arc;

/// Conservative chunk size for `do_batch_index`. Tantivy's writer
/// holds an in-memory heap (50 MB by default in `get_writer`) and
/// commits per call, so very large chunks hurt commit latency without
/// improving throughput. 500 mirrors what the worker's batch handler
/// already uses for paging through bulk jobs.
const REBUILD_CHUNK_SIZE: usize = 500;

/// Default language / supported languages used when we don't have
/// repository metadata at the rebuild site. The fulltext schema is
/// language-aware (see `tantivy_engine::language::register_language_tokenizer`),
/// so we register "en" as the default; per-language translations on
/// each node are still indexed via `do_batch_index`'s built-in
/// translation loop.
const DEFAULT_LANGUAGE: &str = "en";

fn batch_context(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> BatchIndexContext {
    BatchIndexContext {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch.to_string(),
        workspace_id: workspace.to_string(),
        default_language: DEFAULT_LANGUAGE.to_string(),
        supported_languages: vec![DEFAULT_LANGUAGE.to_string()],
    }
}

/// Rebuild the Tantivy index for a single `(tenant, repo, branch)`.
///
/// Holds the `IndexLockManager` lock for the whole rebuild so the
/// batch worker blocks instead of racing the directory. See module
/// docs for the full sequence.
pub async fn rebuild_fulltext_index(
    storage: &Arc<RocksDBStorage>,
    engine: &Arc<TantivyIndexingEngine>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RebuildStats> {
    let start = std::time::Instant::now();

    let key = IndexKey::new(tenant_id, repo_id, branch);
    let lock = storage.index_lock_manager().get_lock(&key).await;
    let _guard = lock.lock().await;

    tracing::info!(
        tenant_id, repo_id, branch,
        "Acquired index lock; starting fulltext rebuild"
    );

    // Tear down the cached handles BEFORE removing the directory so
    // no thread tries to read from the half-deleted dir via the
    // moka cache.
    engine.invalidate_cached_index(tenant_id, repo_id, branch);

    let index_path = engine
        .base_path()
        .join(tenant_id)
        .join(repo_id)
        .join(branch);
    if index_path.exists() {
        std::fs::remove_dir_all(&index_path).map_err(|e| {
            Error::storage(format!(
                "Failed to remove index directory {}: {}",
                index_path.display(),
                e
            ))
        })?;
    }

    let (items_processed, errors) =
        reindex_all_workspaces(storage, engine, tenant_id, repo_id, branch).await;

    Ok(RebuildStats {
        index_type: IndexType::FullText,
        items_processed: items_processed as u64,
        errors: errors as u64,
        duration_ms: start.elapsed().as_millis() as u64,
        success: errors == 0,
    })
}

/// Reconcile the fulltext index against the canonical node store
/// without first deleting the on-disk index.
///
/// This is the v0.1.29 recovery path for tenants whose indexes were
/// corrupted by v0.1.28's aggregator-AND-gate bug. Tantivy's
/// `delete_term + add_document` is idempotent, so running this on a
/// healthy index has no effect beyond some wasted I/O.
///
/// Unlike `rebuild_fulltext_index`, this does not invalidate the
/// engine cache or remove the directory — it just re-issues
/// `do_batch_index` for every node so any missing entries get added.
pub async fn reconcile_fulltext_index(
    storage: &Arc<RocksDBStorage>,
    engine: &Arc<TantivyIndexingEngine>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<RebuildStats> {
    let start = std::time::Instant::now();

    // Hold the same lock the worker uses, so reconcile can't
    // interleave Tantivy commits with the live batch handler.
    let key = IndexKey::new(tenant_id, repo_id, branch);
    let lock = storage.index_lock_manager().get_lock(&key).await;
    let _guard = lock.lock().await;

    tracing::info!(
        tenant_id, repo_id, branch,
        "Acquired index lock; starting fulltext reconcile"
    );

    let (items_processed, errors) =
        reindex_all_workspaces(storage, engine, tenant_id, repo_id, branch).await;

    Ok(RebuildStats {
        index_type: IndexType::FullText,
        items_processed: items_processed as u64,
        errors: errors as u64,
        duration_ms: start.elapsed().as_millis() as u64,
        success: errors == 0,
    })
}

/// Inner helper: iterate every workspace under `(tenant, repo, branch)`,
/// scan its nodes, and submit them to `engine.do_batch_index` in
/// `REBUILD_CHUNK_SIZE`-sized chunks. Returns
/// `(items_processed, errors)`.
///
/// Caller is responsible for any locking, cache invalidation, and
/// directory teardown.
async fn reindex_all_workspaces(
    storage: &Arc<RocksDBStorage>,
    engine: &Arc<TantivyIndexingEngine>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> (usize, usize) {
    // Defensively confirm the branch exists. Avoids silently
    // succeeding on a typo with `items_processed: 0`.
    match list_branches(storage, tenant_id, repo_id).await {
        Ok(branches) if branches.iter().any(|b| b == branch) => {}
        Ok(_) => {
            tracing::warn!(
                tenant_id,
                repo_id,
                branch,
                "Branch not found during fulltext rebuild — returning empty stats"
            );
            return (0, 0);
        }
        Err(e) => {
            tracing::error!(error = %e, "list_branches failed during rebuild");
            return (0, 1);
        }
    }

    let workspaces = match list_workspaces(storage, tenant_id, repo_id).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::error!(error = %e, "list_workspaces failed during rebuild");
            return (0, 1);
        }
    };

    let mut total_processed = 0usize;
    let mut total_errors = 0usize;

    for workspace in &workspaces {
        let nodes = match scan_nodes(storage, tenant_id, repo_id, branch, workspace).await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!(
                    workspace, error = %e,
                    "scan_nodes failed for workspace during rebuild"
                );
                total_errors += 1;
                continue;
            }
        };

        if nodes.is_empty() {
            continue;
        }

        // do_batch_index is blocking and CPU-bound (Tantivy commit
        // and tokenizer work). Hop to the blocking pool so we don't
        // tie up the worker runtime for the duration of a large
        // rebuild.
        let ctx = batch_context(tenant_id, repo_id, branch, workspace);
        let engine_clone = engine.clone();
        for chunk in nodes.chunks(REBUILD_CHUNK_SIZE) {
            let chunk_vec = chunk.to_vec();
            let ctx_clone = batch_context(&ctx.tenant_id, &ctx.repo_id, &ctx.branch, &ctx.workspace_id);
            let engine_for_task = engine_clone.clone();
            let result = tokio::task::spawn_blocking(move || {
                engine_for_task.do_batch_index(&ctx_clone, chunk_vec, Vec::new())
            })
            .await;

            match result {
                Ok(Ok(n)) => total_processed += n,
                Ok(Err(e)) => {
                    tracing::error!(workspace, error = %e, "do_batch_index chunk failed");
                    total_errors += 1;
                }
                Err(e) => {
                    tracing::error!(workspace, error = %e, "blocking task panicked");
                    total_errors += 1;
                }
            }
        }
    }

    tracing::info!(
        tenant_id, repo_id, branch,
        items_processed = total_processed,
        errors = total_errors,
        workspaces = workspaces.len(),
        "Fulltext rebuild finished"
    );

    (total_processed, total_errors)
}
