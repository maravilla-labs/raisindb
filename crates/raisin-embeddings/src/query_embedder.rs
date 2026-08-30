// SPDX-License-Identifier: BSL-1.1

//! Where a *query's* embedding vector comes from, on every SQL surface.
//!
//! # Why this module exists
//!
//! [`crate::resolve`] answers "which embedding provider does this tenant use?".
//! It does not answer "does the SQL engine executing this statement actually
//! have one?" — and that second question had five different answers, four of
//! them "no".
//!
//! `QueryEngine::with_embedding_provider` had exactly ONE call site in the
//! whole workspace outside its own definition: the HTTP `/api/sql` handler.
//! Every other SQL surface — pgwire simple query, pgwire extended query, the
//! WebSocket `sql_query` request, and the three engine constructions inside
//! `raisin-functions` that back `raisin.sql()` — wired the fulltext engine and
//! the HNSW engine but not the embedder. `HYBRID_SEARCH` then took its
//! `if let (Some(hnsw), Some(provider))` branch, found `provider` empty, and
//! skipped the vector half with no else arm: rows came back, `vector_rank` and
//! `vector_distance` were NULL on every one of them, and nothing was logged.
//! An agent built on `raisin.sql('SELECT * FROM HYBRID_SEARCH(...)')` got
//! keyword search and believed it got hybrid.
//!
//! # Why a process-wide installation and not another builder method
//!
//! Because an eighth surface will be added, and adding a
//! `.with_embedding_provider(...)` line to seven call sites is precisely the
//! step that was already forgotten seven times. The provider is not per-query
//! state; it is a property of the *server*, exactly like the outbound MCP
//! client policy (`raisin_functions::configure_mcp_client`) and the platform
//! hook table. So it is installed once at startup and every surface inherits
//! it, including surfaces that do not exist yet.
//!
//! An explicitly wired provider still wins: `with_embedding_provider` remains,
//! it is what tests use, and `ExecutionContext` consults it before falling back
//! to the installed embedder.
//!
//! # Why resolution is lazy, and uncached
//!
//! Resolving reads the tenant embedding config and, when it references one, the
//! tenant AI config — two point reads. Doing that eagerly at engine
//! construction would put them on *every* SQL statement including plain
//! `SELECT`s, and, worse, would let a misconfigured embedder fail the engine
//! construction itself: a tenant with an unrecognised model name took a 500 on
//! every statement it issued, `SELECT 1` included.
//!
//! Resolving at the point of use costs those two reads only on statements that
//! are about to make an HTTP round-trip to an embedding model anyway, where
//! they are noise. It also makes a bad embedding config break vector search and
//! nothing else, and it means `ALTER EMBEDDING CONFIG` takes effect on the next
//! query rather than the next restart — no cache to invalidate, so no stale
//! embedder silently answering with the wrong model's geometry.

use std::sync::{Arc, OnceLock};

use crate::provider::EmbeddingProvider as EmbeddingProviderTrait;
use raisin_error::Result;

/// Resolves the embedding provider a tenant's *queries* should be embedded with.
///
/// `Ok(None)` means "this tenant has deliberately not configured embeddings" —
/// no config row, or a config with `enabled = false`. That is a legitimate
/// state and callers should treat it as "vector search is off here".
///
/// `Err` means a configuration exists and is broken (undecryptable key,
/// unknown model, unreachable reference). That is NOT the same thing, and the
/// distinction is the whole point of the return type: the old code could only
/// express "no provider", so a tenant whose embedder was misconfigured and a
/// tenant who had never enabled embeddings produced byte-identical results.
#[async_trait::async_trait]
pub trait TenantQueryEmbedder: Send + Sync {
    /// The provider to embed this tenant's query text with.
    async fn embedder_for(
        &self,
        tenant_id: &str,
    ) -> Result<Option<Arc<dyn EmbeddingProviderTrait>>>;
}

static QUERY_EMBEDDER: OnceLock<Arc<dyn TenantQueryEmbedder>> = OnceLock::new();

/// Install the process-wide query embedder. Called once, from server startup.
///
/// Returns `false` if one was already installed, which is left to the caller to
/// log rather than panicked on: a second install is a startup-ordering bug, not
/// a reason to refuse to boot.
pub fn configure_query_embedder(embedder: Arc<dyn TenantQueryEmbedder>) -> bool {
    QUERY_EMBEDDER.set(embedder).is_ok()
}

/// The installed query embedder, if server startup installed one.
///
/// `None` in unit tests, in embedded uses of the SQL engine, and in any binary
/// that never calls [`configure_query_embedder`].
pub fn query_embedder() -> Option<&'static Arc<dyn TenantQueryEmbedder>> {
    QUERY_EMBEDDER.get()
}
