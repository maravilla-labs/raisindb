//! Loading half of "which embedding provider does this tenant use?".
//!
//! [`raisin_embeddings::resolve`] is the single place that *decides*; this is
//! the single place that *reads storage* for it — the tenant embedding config
//! plus, only when a `ai_provider_ref` actually points at one, the tenant AI
//! config.
//!
//! Every server surface that embeds text calls one of these two functions: the
//! embedding job handler (write path), the SQL query engine, HTTP hybrid
//! search, the MCP `search` tool and the HTTP "test connection" endpoint. They
//! used to derive it five different ways, and only the job handler honoured
//! `ai_provider_ref` — so a tenant configured through the console (which writes
//! the unified ref) embedded correctly on write and read back as "no embedding
//! configuration for tenant". Adding a sixth derivation is the bug; call this
//! instead.

use raisin_ai::config::TenantAIConfig;
use raisin_ai::storage::TenantAIConfigStore;
use raisin_embeddings::config::TenantEmbeddingConfig;
use raisin_embeddings::provider::EmbeddingProvider as EmbeddingProviderTrait;
use raisin_embeddings::resolve::ResolvedEmbeddingProvider;
use raisin_embeddings::TenantEmbeddingConfigStore;
use raisin_error::{Error, Result};

use crate::RocksDBStorage;

/// Load the tenant AI config, but only when the embedding config needs it.
///
/// A legacy config resolves entirely from its own fields, and this is on the
/// per-query path for SQL, so the read is skipped rather than made
/// unconditional.
pub async fn load_ai_config_if_referenced(
    storage: &RocksDBStorage,
    tenant_id: &str,
    config: &TenantEmbeddingConfig,
) -> Result<Option<TenantAIConfig>> {
    if !config.uses_unified_provider() {
        return Ok(None);
    }

    let ai_config = storage
        .tenant_ai_config_repository()
        .get_config(tenant_id)
        .await
        .map_err(|e| Error::Backend(format!("Failed to get AI config for embeddings: {e}")))?;

    Ok(Some(ai_config))
}

/// Resolve a tenant's embedding settings, reading whatever config it references.
pub async fn resolve_settings(
    storage: &RocksDBStorage,
    tenant_id: &str,
    config: &TenantEmbeddingConfig,
    master_key: &[u8; 32],
) -> Result<ResolvedEmbeddingProvider> {
    let ai_config = load_ai_config_if_referenced(storage, tenant_id, config).await?;
    raisin_embeddings::resolve_settings(config, ai_config.as_ref(), master_key)
}

/// Resolve a tenant's embedding provider and build the client.
///
/// Returns a built provider rather than its parts, so `base_url` and
/// `dimensions` cannot be dropped on the way to the constructor again — the
/// failure mode that made three read-path call sites work only on a host where
/// Ollama happened to sit at its default `localhost:11434`.
pub async fn resolve_provider(
    storage: &RocksDBStorage,
    tenant_id: &str,
    config: &TenantEmbeddingConfig,
    master_key: &[u8; 32],
) -> Result<Box<dyn EmbeddingProviderTrait>> {
    resolve_settings(storage, tenant_id, config, master_key)
        .await?
        .build()
}

/// Load the tenant's embedding config, requiring it to exist and be enabled.
///
/// Shared so "not configured" and "configured but switched off" stay two
/// distinguishable errors on every surface.
pub fn require_enabled_config(
    storage: &RocksDBStorage,
    tenant_id: &str,
) -> Result<TenantEmbeddingConfig> {
    let config = storage
        .tenant_embedding_config_repository()
        .get_config(tenant_id)
        .map_err(|e| Error::storage(e.to_string()))?
        .ok_or_else(|| Error::NotFound("No embedding configuration for tenant".to_string()))?;

    if !config.enabled {
        return Err(Error::Validation(
            "Embeddings are not enabled for this tenant".to_string(),
        ));
    }

    Ok(config)
}

/// The server's [`raisin_embeddings::TenantQueryEmbedder`]: resolves a QUERY
/// embedder from the same tenant config every other embedding path reads.
///
/// Installed once at startup via
/// [`raisin_embeddings::configure_query_embedder`], so every SQL surface — HTTP,
/// pgwire simple and extended, the WebSocket `sql_query` request, and
/// `raisin.sql()` inside a QuickJS or Starlark function — embeds query text
/// through this one object. Before it, only the HTTP handler had an embedder at
/// all; the other four wired the HNSW engine, found no provider beside it, and
/// silently ran `HYBRID_SEARCH` as a plain fulltext search.
///
/// It is a thin shell on purpose: the decision is [`resolve_provider`]'s, which
/// is also what the `EmbeddingGenerate` job handler uses to embed the DOCUMENT
/// side. Queries and documents therefore cannot be embedded by two different
/// models — a failure with no error message anywhere, since two same-width
/// models produce vectors that simply occupy unrelated regions of the space and
/// every resulting ranking is confident noise.
pub struct TenantQueryEmbedderResolver {
    storage: std::sync::Arc<RocksDBStorage>,
}

impl TenantQueryEmbedderResolver {
    /// Build from the shared storage handle.
    ///
    /// The master key is deliberately NOT captured here. It is read per
    /// resolution, exactly as the HTTP path already reads it, because
    /// `main.rs` settles `RAISIN_MASTER_KEY` (propagating the legacy
    /// `EMBEDDING_MASTER_KEY`, and installing the dev zero key) *inside* the
    /// job-system block, which is itself conditional on
    /// `background_jobs_enabled`. Capturing the key at construction would make
    /// this resolver silently depend on being built after that block — a
    /// startup-ordering trap whose only symptom is vector search being off on
    /// servers with background jobs disabled.
    pub fn new(storage: std::sync::Arc<RocksDBStorage>) -> Self {
        Self { storage }
    }
}

#[async_trait::async_trait]
impl raisin_embeddings::TenantQueryEmbedder for TenantQueryEmbedderResolver {
    async fn embedder_for(
        &self,
        tenant_id: &str,
    ) -> Result<Option<std::sync::Arc<dyn EmbeddingProviderTrait>>> {
        // "Never configured" and "switched off" are both Ok(None): vector search
        // is legitimately unavailable, and the caller degrades to fulltext.
        // Anything else — an undecryptable key, an unknown model, a dangling
        // `ai_provider_ref` — is an Err, because a tenant that ASKED for
        // embeddings and got none must not be indistinguishable from one that
        // never asked. That is exactly the confusion this whole module exists to
        // end, so it must not be reintroduced one layer up.
        let config = match self
            .storage
            .tenant_embedding_config_repository()
            .get_config(tenant_id)
            .map_err(|e| Error::storage(e.to_string()))?
        {
            Some(c) if c.enabled => c,
            _ => return Ok(None),
        };

        let master_key = raisin_crypto::master_key_with_embedding_fallback()
            .map_err(|e| Error::Validation(format!("Invalid master key: {e}")))?
            .ok_or_else(|| {
                Error::Validation(
                    "RAISIN_MASTER_KEY is not set, so the tenant's embedding API key cannot be \
                     decrypted"
                        .to_string(),
                )
            })?;

        let provider = resolve_provider(&self.storage, tenant_id, &config, &master_key).await?;

        Ok(Some(std::sync::Arc::from(provider)))
    }
}
