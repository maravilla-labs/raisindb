// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! The one place a workspace-aware SQL catalog is built.
//!
//! Every transport needs the same thing before it can plan a query: the default
//! nodes schema plus one table per workspace in the repo. That was written out
//! six times — the HTTP, WS and both pgwire paths, and twice in the function SQL
//! callbacks — and each copy performed a **full workspace listing per query**.
//! For a function issuing 20 `raisin.sql.query()` calls, that is 20 listings and
//! 20 catalogs for a value that changes only when someone creates or deletes a
//! workspace.
//!
//! Two things follow from that observation, and the order matters:
//!
//! 1. **One implementation.** Six copies of a derivation is the drift pattern
//!    that CLAUDE.md calls this codebase's most common bug class. Callers get
//!    [`workspace_catalog`] and nothing else.
//! 2. **Derived once per repo, not per query.** The result is cached under
//!    `{tenant}:{repo}`.
//!
//! # Invalidation
//!
//! The correctness mechanism is the **event**, not the TTL. `Event::Workspace`
//! is already published by the storage layer on create/update/delete; a handler
//! in `raisin-server` calls [`invalidate_workspace_catalog`]. The TTL is a
//! self-healing floor for the case where no event bus is wired (tests, embedded
//! use, the CLI) — without it, such a deployment would never see a new
//! workspace at all.
//!
//! Note the asymmetry that makes a stale entry cheap: a **missing** workspace is
//! a plan-time "unknown table" error, while an **extra** one that has since been
//! deleted plans fine and scans nothing. Only the first direction is
//! user-visible, and that is the direction the create event covers.

use std::sync::Arc;
use std::time::Duration;

use raisin_error::Result;
use raisin_sql::analyzer::StaticCatalog;
use raisin_storage::scope::RepoScope;
use raisin_storage::{Storage, WorkspaceRepository};

/// Floor for how long a repo's workspace set may be stale when no event bus is
/// wired. Events are the real invalidation path; see the module docs.
const CATALOG_TTL: Duration = Duration::from_secs(60);

static CATALOG_CACHE: std::sync::OnceLock<raisin_core::TtlCache<Arc<StaticCatalog>>> =
    std::sync::OnceLock::new();

fn catalog_cache() -> &'static raisin_core::TtlCache<Arc<StaticCatalog>> {
    CATALOG_CACHE.get_or_init(|| {
        // Replication checkpoint ingestion copies the workspaces column family
        // straight into the live database and emits no events by design, so the
        // per-repo invalidation below never fires for it. Without this hook a
        // replica that catches up by checkpoint answers "unknown table" for
        // every workspace it just ingested, until the TTL floor expires.
        raisin_core::register_invalidator(invalidate_all_workspace_catalogs);
        raisin_core::TtlCache::new(CATALOG_TTL)
    })
}

/// Embedding dimensions are part of the key, not something layered on top of a
/// shared base: `with_embedding_column` pushes a column onto the `nodes` table,
/// so a catalog built for a tenant with embeddings enabled is a *different*
/// schema, and handing it to a tenant without them would invent a column.
///
/// The trailing separator is load-bearing: [`invalidate_workspace_catalog`]
/// drops entries by prefix, and without it invalidating repo `r1` would also
/// drop `r10`, `r1_archive` and every other repo whose name it prefixes.
fn cache_key(tenant_id: &str, repo_id: &str, embedding_dimensions: Option<usize>) -> String {
    match embedding_dimensions {
        Some(dims) => format!("{tenant_id}:{repo_id}:v{dims}"),
        None => format!("{tenant_id}:{repo_id}:"),
    }
}

/// Prefix covering every embedding variant of one repo — and nothing else.
fn scope_prefix(tenant_id: &str, repo_id: &str) -> String {
    format!("{tenant_id}:{repo_id}:")
}

/// Build (or reuse) the SQL catalog for a repo: default nodes schema plus one
/// table per workspace.
///
/// This is the only supported way to obtain a catalog for query planning. It is
/// cheap to call per query — on a hit it is a hash lookup and an `Arc` bump.
pub async fn workspace_catalog<S: Storage + ?Sized>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Arc<StaticCatalog>> {
    workspace_catalog_with_embeddings(storage, tenant_id, repo_id, None).await
}

/// As [`workspace_catalog`], but also declaring the `embedding` vector column
/// when the tenant has embeddings enabled.
pub async fn workspace_catalog_with_embeddings<S: Storage + ?Sized>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    embedding_dimensions: Option<usize>,
) -> Result<Arc<StaticCatalog>> {
    let key = cache_key(tenant_id, repo_id, embedding_dimensions);

    if let Some(cached) = catalog_cache().get(&key) {
        return Ok(cached);
    }

    let workspaces = storage
        .workspaces()
        .list(RepoScope::new(tenant_id, repo_id))
        .await?;

    let mut catalog = StaticCatalog::default_nodes_schema();
    if let Some(dims) = embedding_dimensions {
        catalog = catalog.with_embedding_column(dims);
    }
    for ws in &workspaces {
        catalog.register_workspace(ws.name.clone());
    }

    let catalog = Arc::new(catalog);
    catalog_cache().put(&key, catalog.clone());

    tracing::debug!(
        tenant = %tenant_id,
        repo = %repo_id,
        workspaces = workspaces.len(),
        embedding_dimensions = ?embedding_dimensions,
        "Built workspace catalog"
    );

    Ok(catalog)
}

/// Drop a repo's cached catalogs. Call this whenever a workspace is created,
/// renamed or deleted.
///
/// Exported as a free function rather than a handle threaded through
/// `ExecutionDependencies` so the event handler in `raisin-server` needs no
/// plumbing through the six transports.
///
/// Drops every embedding variant for the repo, not just the plain one — the
/// caller reporting the workspace change has no idea which variants exist.
pub fn invalidate_workspace_catalog(tenant_id: &str, repo_id: &str) {
    let prefix = scope_prefix(tenant_id, repo_id);
    catalog_cache().invalidate_prefix(&prefix);
    tracing::debug!(scope = %prefix, "Workspace catalog cache invalidated");
}

/// Drop every cached catalog. Intended for tests and administrative resets.
pub fn invalidate_all_workspace_catalogs() {
    catalog_cache().invalidate_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_variants_get_distinct_keys() {
        assert_ne!(
            cache_key("t", "r", None),
            cache_key("t", "r", Some(1536)),
            "a catalog carrying the embedding column is a different schema and \
             must never be served to a tenant without embeddings"
        );
        assert_ne!(
            cache_key("t", "r", Some(768)),
            cache_key("t", "r", Some(1536))
        );
    }

    #[test]
    fn the_scope_prefix_covers_variants_without_bleeding_into_sibling_repos() {
        let prefix = scope_prefix("t", "r1");

        assert!(cache_key("t", "r1", None).starts_with(&prefix));
        assert!(cache_key("t", "r1", Some(1536)).starts_with(&prefix));

        // The trailing separator is what stops `r1` from also invalidating these.
        assert!(!cache_key("t", "r10", None).starts_with(&prefix));
        assert!(!cache_key("t", "r1_archive", None).starts_with(&prefix));
        assert!(!cache_key("t2", "r1", None).starts_with(&prefix));
    }
}
