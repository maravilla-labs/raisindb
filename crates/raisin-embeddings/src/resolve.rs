//! The one place that answers "which embedding provider does this tenant use?".
//!
//! # Why this module exists
//!
//! Five call sites used to answer that question independently — the embedding
//! job handler, the SQL query engine, HTTP hybrid search, the MCP search tool
//! and the "test connection" endpoint — and all five drifted:
//!
//! * only the job handler honoured `ai_provider_ref` / `ai_model_ref`, so a
//!   tenant configured through the console (which writes the unified ref)
//!   embedded fine on **write** and was silently unconfigured on **read**;
//! * only the test endpoint exempted Ollama from needing an API key, so
//!   "Test connection" returned a green 768-dimension success for a config
//!   under which every subsequent job failed with *No API key configured for
//!   embeddings*, and nothing surfaced the contradiction;
//! * three of them called [`crate::provider::create_provider`], which drops
//!   `base_url` **and** `dimensions` — working only by accident on a host
//!   where Ollama happens to sit at its default `localhost:11434`.
//!
//! So: one resolver, and it hands back a **built provider**, not a tuple of
//! parts a caller can forget to pass on. That is the same reasoning that
//! produced [`ResolvedEmbeddingProvider`] inside the job handler, taken one
//! level up so every surface inherits it.
//!
//! # The API-key rule lives here, once
//!
//! Whether a key is required is a property of the provider variant
//! ([`EmbeddingProvider::requires_api_key`] /
//! [`AIProvider::requires_api_key`]), asked in exactly one place. Ollama and
//! any self-hosted OpenAI-compatible endpoint are therefore exempt everywhere
//! or nowhere — the test endpoint and the job can no longer disagree.

use raisin_ai::config::{AIProvider, EmbedderId};

/// Re-exported so a caller can name the resolver's inputs without taking its
/// own `raisin-ai` dependency — one crate owns the shape, one crate owns the
/// rule.
pub use raisin_ai::config::TenantAIConfig;
pub use raisin_ai::storage::TenantAIConfigStore;
use raisin_crypto::ApiKeyEncryptor;
use raisin_error::{Error, Result};

use crate::config::{EmbeddingProvider, TenantEmbeddingConfig};
use crate::provider::{create_provider_full, EmbeddingProvider as EmbeddingProviderTrait};

/// Everything needed to build an embedding client for a tenant.
///
/// A struct rather than a tuple because `base_url` and `dimensions` are the
/// fields that used to get lost: a 3-tuple made "provider, key, model" look
/// complete when it was not. Prefer [`resolve_provider`], which builds the
/// client for you and makes dropping a field impossible.
#[derive(Debug, Clone)]
pub struct ResolvedEmbeddingProvider {
    /// Which wire protocol to speak.
    pub provider: EmbeddingProvider,
    /// Decrypted API key. Empty string when the provider requires none.
    pub api_key: String,
    /// Model identifier to request.
    pub model: String,
    /// Endpoint override for self-hosted / gateway deployments.
    pub base_url: Option<String>,
    /// Vector width. A one-way door: it is hashed into the embedder identity
    /// and the HNSW index is built at this width.
    pub dimensions: usize,
}

impl ResolvedEmbeddingProvider {
    /// Build the client. Always `create_provider_full` — the shorter factories
    /// drop `base_url` and `dimensions`.
    pub fn build(&self) -> Result<Box<dyn EmbeddingProviderTrait>> {
        create_provider_full(
            &self.provider,
            &self.api_key,
            &self.model,
            self.base_url.as_deref(),
            Some(self.dimensions),
        )
    }

    /// Which embedder produced (or will produce) these vectors.
    ///
    /// [`EmbedderId::to_key_hash`] is the `{embedder_hash}` segment of every
    /// `cf::EMBEDDINGS` key, so this value is what partitions one model's
    /// vectors from another's. It must therefore describe the provider that
    /// **actually ran**, which is the resolution — not the config row it was
    /// resolved from.
    ///
    /// That distinction is the whole reason this method exists. The job handler
    /// used to build the identity straight from `TenantEmbeddingConfig`'s
    /// *legacy* fields while embedding through the *resolved* provider. In
    /// unified mode (`ai_provider_ref` set) those legacy fields are whatever
    /// `TenantEmbeddingConfig::new` left behind — `OpenAI` /
    /// `text-embedding-3-small` — so a tenant whose vectors came from Ollama
    /// `bge-m3` stored every one of them labelled as OpenAI's model, under
    /// OpenAI's key hash. Two consequences, both silent:
    ///
    /// * repointing `ai_provider_ref` at a different model of the **same
    ///   width** did not change the hash, so the old model's vectors and the
    ///   new model's vectors landed in one partition and were ranked against
    ///   each other — confident nonsense, no error anywhere;
    /// * the identity stored in the record, the one thing that could ever have
    ///   detected that, agreed with the lie.
    ///
    /// # Stability
    ///
    /// The provider component stays the lowercased `Debug` name of the
    /// [`EmbeddingProvider`] **variant** (`"ollama"`, `"openai"`, …), never a
    /// tenant's free-form provider slug. The hash is a storage key: feeding it
    /// a slug would move every existing vector out from under its own key. In
    /// legacy mode the resolution is a copy of the config fields, so this is
    /// byte-identical to what the handler computed before — legacy tenants see
    /// no re-partitioning.
    pub fn embedder_id(&self) -> EmbedderId {
        EmbedderId::new(
            format!("{:?}", self.provider).to_lowercase(),
            self.model.clone(),
            self.dimensions,
        )
    }
}

/// Resolve a tenant's embedding provider and build the client.
///
/// This is the single public entry point; every surface that needs to embed
/// text (write path or read path) goes through it.
///
/// `ai_config` must be supplied whenever the embedding config sets
/// `ai_provider_ref` (see [`TenantEmbeddingConfig::uses_unified_provider`]).
/// Callers that cannot cheaply load it may pass `None` and will get a clear
/// error rather than a silent fall back to the legacy fields — falling back
/// is precisely how the read path used to embed with the wrong provider.
pub fn resolve_provider(
    config: &TenantEmbeddingConfig,
    ai_config: Option<&TenantAIConfig>,
    master_key: &[u8; 32],
) -> Result<Box<dyn EmbeddingProviderTrait>> {
    resolve_settings(config, ai_config, master_key)?.build()
}

/// Resolve the parts without building a client.
///
/// Only for callers that genuinely need to inspect the resolution (logging,
/// the config surface). Anything that is going to embed should call
/// [`resolve_provider`].
pub fn resolve_settings(
    config: &TenantEmbeddingConfig,
    ai_config: Option<&TenantAIConfig>,
    master_key: &[u8; 32],
) -> Result<ResolvedEmbeddingProvider> {
    let encryptor = ApiKeyEncryptor::new(master_key);
    let shape = resolve_shape(config, ai_config)?;

    let api_key = match &shape.key_source {
        // The provider variant decides whether a key is required — Ollama,
        // Custom and Local need none — and a vendor kind pointed at someone
        // else's host is no longer a vendor request, so it does not need the
        // vendor's key either.
        KeySource::Unified {
            encrypted,
            required,
            label,
        } => decrypt_key(&encryptor, encrypted.as_deref(), *required, label)?,
        KeySource::Legacy => decrypt_key(
            &encryptor,
            config.api_key_encrypted.as_deref(),
            config.provider.requires_api_key(config.base_url.as_deref()),
            "embeddings",
        )?,
    };

    Ok(ResolvedEmbeddingProvider {
        provider: shape.provider,
        api_key,
        model: shape.model,
        base_url: shape.base_url,
        dimensions: config.dimensions,
    })
}

/// Where the API key for a resolution comes from.
enum KeySource {
    Unified {
        encrypted: Option<Vec<u8>>,
        required: bool,
        label: String,
    },
    Legacy,
}

/// Everything a resolution decides that is NOT the API key.
struct ResolvedShape {
    provider: EmbeddingProvider,
    model: String,
    base_url: Option<String>,
    key_source: KeySource,
}

/// Which provider, which model, which endpoint — the half of resolution that
/// needs no master key and can therefore run on a synchronous, key-less path.
///
/// Split out so that [`resolve_settings`] and [`resolve_embedder_id`] cannot
/// disagree about which model a tenant is on. They used to be able to: the job
/// handler built the embedder identity straight from `TenantEmbeddingConfig`'s
/// *legacy* fields while embedding through the *resolved* provider, so a tenant
/// on Ollama `bge-m3` stored every vector labelled as OpenAI's model, under
/// OpenAI's key hash — and the partition an index is filed under is derived from
/// exactly that hash.
fn resolve_shape(
    config: &TenantEmbeddingConfig,
    ai_config: Option<&TenantAIConfig>,
) -> Result<ResolvedShape> {
    if config.uses_unified_provider() {
        // ── Unified mode: the provider lives in TenantAIConfig ──────────────
        let provider_ref = config
            .ai_provider_ref
            .as_ref()
            .expect("uses_unified_provider() implies ai_provider_ref is Some");

        let ai_config = ai_config.ok_or_else(|| {
            Error::Validation(format!(
                "embedding config references AI provider '{}' but no tenant AI config was supplied",
                provider_ref
            ))
        })?;

        // Match on SLUG. `format!("{:?}", kind).to_lowercase()` used to be the
        // comparison and produced `azureopenai`, which can never equal the
        // `azure_openai` a caller would write — an Azure ref never resolved.
        // Legacy refs keep working: entries stored before slugs existed are
        // slugged after their kind's serde name.
        let ai_provider = ai_config.get_provider(provider_ref).ok_or_else(|| {
            Error::Validation(format!(
                "AI provider '{}' not found in tenant config",
                provider_ref
            ))
        })?;

        // Which wire protocol to speak is a property of the KIND; the slug is
        // free-form and a tenant may call its OpenAI entry anything.
        let provider = embedding_provider_for_kind(ai_provider.kind, provider_ref)?;

        // The default model likewise follows the kind, not the slug.
        let model = config
            .ai_model_ref
            .clone()
            .unwrap_or_else(|| default_model_for_kind(ai_provider.kind).to_string());

        // Previously dropped on the floor, which is why a tenant could
        // configure a custom endpoint, see it accepted, and still have every
        // embedding go to the vendor's default host.
        let base_url = ai_provider.api_endpoint.clone();
        let required = ai_provider.kind.requires_api_key() && base_url.is_none();

        Ok(ResolvedShape {
            provider,
            model,
            base_url,
            key_source: KeySource::Unified {
                encrypted: ai_provider.api_key_encrypted.clone(),
                required,
                label: format!("AI provider '{}'", provider_ref),
            },
        })
    } else {
        // ── Legacy mode: the fields on TenantEmbeddingConfig ────────────────
        Ok(ResolvedShape {
            provider: config.provider.clone(),
            model: config.model.clone(),
            base_url: config.base_url.clone(),
            key_source: KeySource::Legacy,
        })
    }
}

/// Which embedder a tenant is on, WITHOUT decrypting anything.
///
/// The partition an HNSW index is filed under is `{embedder_hash}{kind}`, and
/// the hash is over `provider:model:dimensions:tokenizer`. None of that is
/// secret, and the caller that needs it most — the engine's index-spec resolver,
/// consulted synchronously on a cache miss — has no master key and cannot be
/// made async. So this is the key-less half of [`resolve_settings`], sharing its
/// one decision via [`resolve_shape`].
pub fn resolve_embedder_id(
    config: &TenantEmbeddingConfig,
    ai_config: Option<&TenantAIConfig>,
) -> Result<EmbedderId> {
    let shape = resolve_shape(config, ai_config)?;
    Ok(EmbedderId::new(
        format!("{:?}", shape.provider).to_lowercase(),
        shape.model,
        config.dimensions,
    ))
}

/// Decrypt a stored key, enforcing the requirement in exactly one place.
///
/// A key that is *present* is always decrypted even when not required — a
/// self-hosted gateway may still want one — but its absence is only an error
/// when the provider actually needs it.
fn decrypt_key(
    encryptor: &ApiKeyEncryptor,
    encrypted: Option<&[u8]>,
    required: bool,
    who: &str,
) -> Result<String> {
    match encrypted {
        // Context-free decryption is deliberate here: these are the existing
        // AI/embedding key blobs, sealed context-free by every writer. Binding
        // them to a `SecretContext` is a format change for stored data, not a
        // change this resolver can make on the read side alone.
        #[allow(deprecated)]
        Some(bytes) => encryptor
            .decrypt(bytes)
            .map_err(|e| Error::Backend(format!("Failed to decrypt API key for {}: {}", who, e))),
        None if required => Err(Error::Validation(format!(
            "No API key configured for {}",
            who
        ))),
        None => Ok(String::new()),
    }
}

/// Map a unified AI provider kind onto the embedding wire protocol.
fn embedding_provider_for_kind(kind: AIProvider, slug: &str) -> Result<EmbeddingProvider> {
    match kind {
        // Everything that speaks OpenAI's `/embeddings` shape shares one
        // client; they differ only in host, which `api_endpoint` supplies.
        AIProvider::OpenAI
        | AIProvider::AzureOpenAI
        | AIProvider::Groq
        | AIProvider::OpenRouter
        | AIProvider::Custom => Ok(EmbeddingProvider::OpenAI),
        AIProvider::Anthropic => Ok(EmbeddingProvider::Claude),
        AIProvider::Ollama => Ok(EmbeddingProvider::Ollama),
        _ => Err(Error::Validation(format!(
            "Provider '{}' does not support embeddings",
            slug
        ))),
    }
}

/// Default embedding model per provider kind, used when `ai_model_ref` is unset.
fn default_model_for_kind(kind: AIProvider) -> &'static str {
    match kind {
        AIProvider::Ollama => "nomic-embed-text",
        _ => "text-embedding-3-small",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_ai::config::{AIProviderConfig, TenantAIConfig};

    const KEY: [u8; 32] = [7u8; 32];

    fn base_config() -> TenantEmbeddingConfig {
        let mut c = TenantEmbeddingConfig::new("t".to_string());
        c.enabled = true;
        c
    }

    fn ai_entry(slug: &str, kind: AIProvider) -> AIProviderConfig {
        AIProviderConfig {
            slug: slug.to_string(),
            kind,
            display_name: None,
            icon_url: None,
            api_key_encrypted: None,
            api_endpoint: None,
            enabled: true,
            models: Vec::new(),
        }
    }

    // ── The key requirement is asked once, of the provider variant ──────────
    //
    // The regression this replaces: the test endpoint exempted Ollama while
    // the job handler did not, so "Test connection" went green for a config
    // under which every job then failed.

    #[test]
    fn ollama_needs_no_api_key_in_legacy_mode() {
        let mut c = base_config();
        c.provider = EmbeddingProvider::Ollama;
        c.model = "nomic-embed-text".to_string();
        c.dimensions = 768;
        c.api_key_encrypted = None;

        let r = resolve_settings(&c, None, &KEY).expect("ollama must resolve without a key");
        assert_eq!(r.provider, EmbeddingProvider::Ollama);
        assert!(r.api_key.is_empty());
        assert_eq!(r.dimensions, 768);
    }

    #[test]
    fn openai_still_demands_an_api_key() {
        let mut c = base_config();
        c.provider = EmbeddingProvider::OpenAI;
        c.api_key_encrypted = None;

        assert!(resolve_settings(&c, None, &KEY).is_err());
    }

    #[test]
    fn ollama_needs_no_api_key_in_unified_mode_either() {
        let mut c = base_config();
        c.ai_provider_ref = Some("local-ollama".to_string());
        c.dimensions = 768;

        let mut ai = TenantAIConfig::new("t".to_string());
        let mut entry = ai_entry("local-ollama", AIProvider::Ollama);
        entry.api_endpoint = Some("http://127.0.0.1:11434".to_string());
        ai.providers.push(entry);

        let r = resolve_settings(&c, Some(&ai), &KEY).expect("ollama ref must resolve");
        assert_eq!(r.provider, EmbeddingProvider::Ollama);
        assert!(r.api_key.is_empty());
        // Defaults follow the KIND, not the slug.
        assert_eq!(r.model, "nomic-embed-text");
        assert_eq!(r.base_url.as_deref(), Some("http://127.0.0.1:11434"));
    }

    // ── base_url and dimensions survive the round trip ──────────────────────
    //
    // Three read-path call sites used `create_provider`, which drops both.

    #[test]
    fn base_url_and_dimensions_reach_the_built_provider() {
        let mut c = base_config();
        c.provider = EmbeddingProvider::Ollama;
        c.model = "nomic-embed-text".to_string();
        c.dimensions = 768;
        c.base_url = Some("http://127.0.0.1:11434".to_string());

        let p = resolve_provider(&c, None, &KEY).expect("must build");
        assert_eq!(p.dimensions(), 768);
    }

    #[test]
    fn a_custom_openai_compatible_endpoint_keeps_its_host() {
        let mut c = base_config();
        c.ai_provider_ref = Some("gateway".to_string());
        c.ai_model_ref = Some("bge-m3".to_string());
        c.dimensions = 1024;

        let mut ai = TenantAIConfig::new("t".to_string());
        let mut entry = ai_entry("gateway", AIProvider::Custom);
        entry.api_endpoint = Some("https://gw.example.test/v1".to_string());
        ai.providers.push(entry);

        let r = resolve_settings(&c, Some(&ai), &KEY).expect("custom ref must resolve");
        assert_eq!(r.provider, EmbeddingProvider::OpenAI);
        assert_eq!(r.base_url.as_deref(), Some("https://gw.example.test/v1"));
        // A model outside the vendor's built-in name table only works because
        // the dimension override is carried through.
        assert_eq!(r.dimensions, 1024);
        assert_eq!(r.build().expect("must build").dimensions(), 1024);
    }

    // ── A unified ref must never silently fall back to the legacy fields ────
    //
    // That fallback is exactly how a console-configured tenant embedded on
    // write and was "unconfigured" on read.

    #[test]
    fn a_unified_ref_without_ai_config_errors_instead_of_falling_back() {
        let mut c = base_config();
        c.ai_provider_ref = Some("openai".to_string());
        // Legacy fields are populated and would happily resolve — the point is
        // that they must NOT be used when a ref is set.
        c.provider = EmbeddingProvider::Ollama;
        c.api_key_encrypted = None;

        let err = resolve_settings(&c, None, &KEY).expect_err("must not fall back");
        assert!(
            format!("{err}").contains("no tenant AI config"),
            "unexpected error: {err}"
        );
    }

    /// The exact configuration that used to send a self-hosted tenant's queries
    /// to `api.openai.com`.
    ///
    /// Measured before the fix, on a live server against real Ollama: the
    /// legacy fields below (`OpenAI` / `text-embedding-3-small`) are what the
    /// SQL engine used, and the query came back
    /// `OpenAI API error 401 Unauthorized: You didn't provide an API key` — for
    /// a config whose `ai_provider_ref` names an Ollama entry on
    /// `127.0.0.1:11434`. The legacy fields must lose to the ref, always.
    #[test]
    fn the_ref_wins_over_stale_legacy_fields() {
        let mut c = base_config();
        c.ai_provider_ref = Some("local-ollama".to_string());
        c.ai_model_ref = Some("nomic-embed-text".to_string());
        c.dimensions = 768;
        // Exactly what the console leaves behind in the legacy fields.
        c.provider = EmbeddingProvider::OpenAI;
        c.model = "text-embedding-3-small".to_string();
        c.base_url = None;

        let mut ai = TenantAIConfig::new("t".to_string());
        let mut entry = ai_entry("local-ollama", AIProvider::Ollama);
        entry.api_endpoint = Some("http://127.0.0.1:11434".to_string());
        ai.providers.push(entry);

        let r = resolve_settings(&c, Some(&ai), &KEY).expect("must resolve");
        assert_eq!(r.provider, EmbeddingProvider::Ollama);
        assert_eq!(r.model, "nomic-embed-text");
        assert_eq!(r.base_url.as_deref(), Some("http://127.0.0.1:11434"));
        assert!(r.api_key.is_empty());
    }

    #[test]
    fn a_dangling_ref_names_the_slug() {
        let mut c = base_config();
        c.ai_provider_ref = Some("gone".to_string());
        let ai = TenantAIConfig::new("t".to_string());

        let err = resolve_settings(&c, Some(&ai), &KEY).expect_err("must not resolve");
        assert!(format!("{err}").contains("gone"), "unexpected error: {err}");
    }

    #[test]
    fn a_non_embedding_kind_is_rejected_by_name() {
        let mut c = base_config();
        c.ai_provider_ref = Some("g".to_string());
        let mut ai = TenantAIConfig::new("t".to_string());
        ai.providers.push(ai_entry("g", AIProvider::Google));

        let err = resolve_settings(&c, Some(&ai), &KEY).expect_err("google has no embeddings");
        assert!(
            format!("{err}").contains("does not support embeddings"),
            "unexpected error: {err}"
        );
    }

    // ── The embedder identity describes what RAN, not the config row ────────
    //
    // `EmbedderId::to_key_hash()` is the `{embedder_hash}` segment of every
    // `cf::EMBEDDINGS` key. Deriving it from the legacy fields while embedding
    // through the resolved provider is how a unified-mode tenant's Ollama
    // vectors ended up stored under OpenAI's hash.

    #[test]
    fn the_identity_follows_the_resolution_not_the_legacy_fields() {
        let mut c = base_config();
        c.ai_provider_ref = Some("local-ollama".to_string());
        c.ai_model_ref = Some("bge-m3".to_string());
        c.dimensions = 1024;
        // Exactly what `TenantEmbeddingConfig::new` leaves behind for a tenant
        // configured only through the unified console.
        c.provider = EmbeddingProvider::OpenAI;
        c.model = "text-embedding-3-small".to_string();

        let mut ai = TenantAIConfig::new("t".to_string());
        let mut entry = ai_entry("local-ollama", AIProvider::Ollama);
        entry.api_endpoint = Some("http://127.0.0.1:11434".to_string());
        ai.providers.push(entry);

        let id = resolve_settings(&c, Some(&ai), &KEY)
            .expect("must resolve")
            .embedder_id();

        assert_eq!(id.provider, "ollama");
        assert_eq!(id.model, "bge-m3");
        assert_eq!(id.dimensions, 1024);

        // And it must NOT be what the old derivation produced.
        let old = EmbedderId::new("openai", "text-embedding-3-small", 1024);
        assert_ne!(
            id.to_key_hash(),
            old.to_key_hash(),
            "unified-mode vectors must not be stored under the legacy fields' hash"
        );
    }

    /// Two same-width models must land in different key partitions.
    ///
    /// This is the case nothing anywhere catches: the width check in
    /// `HnswIndex` only fires when the widths differ, so two 1024-wide models
    /// produce vectors that occupy unrelated regions of the space and are
    /// ranked against each other with no error. Separating them at the key
    /// hash is the mechanism the storage layout was designed around, and the
    /// legacy-field derivation defeated it.
    #[test]
    fn two_same_width_models_do_not_share_a_key_partition() {
        let ai = {
            let mut ai = TenantAIConfig::new("t".to_string());
            let mut entry = ai_entry("local-ollama", AIProvider::Ollama);
            entry.api_endpoint = Some("http://127.0.0.1:11434".to_string());
            ai.providers.push(entry);
            ai
        };

        let id_for = |model: &str| {
            let mut c = base_config();
            c.ai_provider_ref = Some("local-ollama".to_string());
            c.ai_model_ref = Some(model.to_string());
            c.dimensions = 1024;
            resolve_settings(&c, Some(&ai), &KEY)
                .expect("must resolve")
                .embedder_id()
                .to_key_hash()
        };

        assert_ne!(id_for("bge-m3"), id_for("mxbai-embed-large"));
    }

    /// A legacy tenant's hash must not move — it addresses vectors already on
    /// disk.
    #[test]
    fn legacy_mode_keeps_the_hash_it_always_had() {
        let mut c = base_config();
        c.provider = EmbeddingProvider::Ollama;
        c.model = "nomic-embed-text".to_string();
        c.dimensions = 768;

        let id = resolve_settings(&c, None, &KEY)
            .expect("must resolve")
            .embedder_id();

        // Byte-for-byte the pre-change derivation:
        // `EmbedderId::new(format!("{:?}", config.provider).to_lowercase(),
        //                  config.model.clone(), config.dimensions)`
        let previous = EmbedderId::new(
            format!("{:?}", c.provider).to_lowercase(),
            c.model.clone(),
            c.dimensions,
        );
        assert_eq!(id.to_key_hash(), previous.to_key_hash());
    }

    // ── An encrypted key round-trips through the shared encryptor ───────────

    #[test]
    fn a_stored_key_is_decrypted_once_here() {
        let enc = ApiKeyEncryptor::new(&KEY);
        let mut c = base_config();
        c.provider = EmbeddingProvider::OpenAI;
        c.api_key_encrypted = Some(enc.encrypt("sk-secret").unwrap());

        let r = resolve_settings(&c, None, &KEY).expect("must resolve");
        assert_eq!(r.api_key, "sk-secret");
    }
}
