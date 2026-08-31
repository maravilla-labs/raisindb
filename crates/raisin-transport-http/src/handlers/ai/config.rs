// SPDX-License-Identifier: BSL-1.1

//! AI configuration CRUD handlers.
//!
//! GET/PUT endpoints for tenant AI configuration and provider listing.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use std::collections::HashSet;

use raisin_ai::{
    config::{validate_slug, AIProviderConfig, TenantAIConfig},
    crypto::ApiKeyEncryptor,
    storage::TenantAIConfigStore,
};

use crate::state::AppState;

use super::types::{
    ConfigResponse, ErrorResponse, Patch, ProviderConfigRequest, ProviderConfigResponse,
    ProviderSummary, ProvidersListResponse, SetConfigRequest, SuccessResponse,
};

/// Get full tenant AI configuration.
///
/// GET /api/tenants/{tenant_id}/ai/config
#[axum::debug_handler]
pub async fn get_ai_config(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<ConfigResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = &tenant_id;

    let repo_impl = state.storage().tenant_ai_config_repository();

    // Fetch config
    match repo_impl.get_config(tenant_id).await {
        Ok(config) => {
            let providers = config
                .providers
                .into_iter()
                .map(|p| ProviderConfigResponse {
                    slug: p.slug,
                    provider: p.kind,
                    display_name: p.display_name,
                    icon_url: p.icon_url,
                    has_api_key: p.api_key_encrypted.is_some(),
                    api_endpoint: p.api_endpoint,
                    enabled: p.enabled,
                    models: p.models,
                })
                .collect();

            Ok(Json(ConfigResponse {
                tenant_id: config.tenant_id,
                providers,
                embedding_settings: config.embedding_settings,
            }))
        }
        Err(raisin_ai::storage::StorageError::NotFound(_)) => {
            // Return empty config for tenant if not found
            Ok(Json(ConfigResponse {
                tenant_id: tenant_id.to_string(),
                providers: Vec::new(),
                embedding_settings: None,
            }))
        }
        Err(e) => {
            tracing::error!("Failed to get AI config for {}: {}", tenant_id, e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Storage error: {}", e),
                }),
            ))
        }
    }
}

/// Set or update tenant AI configuration.
///
/// PUT /api/tenants/{tenant_id}/ai/config
///
/// The payload is a MERGE, not a document replace. Entries are matched against
/// storage by SLUG:
///
/// - slug present in storage -> update (an omitted `api_key_plain` keeps the
///   stored key)
/// - slug absent from storage -> create (the slug is validated here, and only here)
/// - stored slug absent from the payload -> left untouched
///
/// That last rule is the whole point. Every shipped client predates slugs and sends
/// the fixed list of kind-named entries it knows about; under delete-by-absence such
/// a payload wiped a tenant's self-named gateway and the encrypted key with it, on an
/// ordinary "save settings" click. Deletion is therefore explicit — see
/// [`delete_ai_provider`].
///
/// There is deliberately no rename either: an `ai_provider_ref` in an embedding
/// config, an agent node's `provider` property and every `<slug>:<model>` id are plain
/// strings with no referential integrity behind them, so a renamed slug would leave
/// them pointing at nothing and fail at call time rather than at save time.
#[axum::debug_handler]
pub async fn set_ai_config(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<SetConfigRequest>,
) -> Result<Json<SuccessResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = &tenant_id;
    let repo_impl = state.storage().tenant_ai_config_repository();

    let master_key = state.get_master_key().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Master key not configured: {}", e),
            }),
        )
    })?;
    let encryptor = ApiKeyEncryptor::new(&master_key);

    let config = merge_and_store(&repo_impl, tenant_id, &req, |plain| {
        encryptor.encrypt(plain).map_err(|e| {
            tracing::error!("Failed to encrypt API key: {}", e);
            MergeError::Internal(format!("Encryption failed: {}", e))
        })
    })
    .await?;

    tracing::info!(
        "Updated AI config for tenant: {} ({} providers)",
        tenant_id,
        config.providers.len()
    );

    mirror_embedding_settings(&state, tenant_id, &config);

    Ok(Json(SuccessResponse {
        success: true,
        message: format!("AI configuration saved for tenant {}", tenant_id),
    }))
}

/// Project the AI config's `embedding_settings` onto the tenant's
/// `TenantEmbeddingConfig`, which is the row every read path actually consults.
///
/// There are two stores. `TenantAIConfig.embedding_settings` is what the routed
/// admin-console page (`TenantAiSettings.tsx`) writes; `cf::TENANT_EMBEDDING_CONFIG`
/// is what the SQL catalog reads for `embedding_dimensions()`, what the HNSW spec
/// resolver reads for the index width, metric and quantization, and what the
/// embedding job reads to pick a provider. Nothing joined them, so a tenant could
/// configure embeddings in the UI, see them enabled, and get
/// *"EMBEDDING() function requires embeddings to be enabled"* from every query --
/// a config that is stored, displayed, and invisible to the only code that needs it.
///
/// Mirroring here rather than teaching the read path a fallback keeps ONE row
/// authoritative: a fallback would mean two documents that disagree, and the
/// embedder identity (provider, model, width) is hashed into the key of every
/// vector already written -- the wrong one wins silently and re-partitions the
/// index.
///
/// The projection is deliberately partial. `distance_metric`, `quantization`,
/// `base_url`, `default_max_distance` and the legacy API key have no counterpart
/// in `EmbeddingSettings`, so the stored row's values are carried forward rather
/// than reset to defaults -- resetting `quantization` alone would make the existing
/// graph unloadable.
///
/// A mirror failure is logged, not returned: the AI config write has already
/// committed, and answering 500 to a save that succeeded would send the operator
/// back to re-save a form that is already correct.
fn mirror_embedding_settings(state: &AppState, tenant_id: &str, config: &TenantAIConfig) {
    use raisin_embeddings::TenantEmbeddingConfigStore;

    let Some(settings) = config.embedding_settings.as_ref() else {
        return;
    };

    let Some(rocksdb_storage) = state.rocksdb_storage.as_ref() else {
        tracing::warn!(
            tenant_id = %tenant_id,
            "AI config saved with embedding settings, but there is no RocksDB storage to \
             mirror them into - vector search will not see this configuration"
        );
        return;
    };

    let repo = rocksdb_storage.tenant_embedding_config_repository();

    let mut target = match repo.get_config(tenant_id) {
        Ok(Some(existing)) => existing,
        Ok(None) => raisin_embeddings::config::TenantEmbeddingConfig::new(tenant_id.to_string()),
        Err(e) => {
            tracing::error!(
                tenant_id = %tenant_id,
                error = %e,
                "Could not read the embedding config to mirror the saved AI settings into it"
            );
            return;
        }
    };

    target.enabled = settings.enabled;
    target.dimensions = settings.dimensions;
    target.include_name = settings.include_name;
    target.include_path = settings.include_path;
    target.max_embeddings_per_repo = settings.max_embeddings_per_repo;
    target.chunking = settings.chunking.clone();
    if settings.default_max_distance.is_some() {
        target.default_max_distance = settings.default_max_distance;
    }
    // Parsed against the canonical enums rather than re-spelled here, and a
    // value that does not parse is refused rather than defaulted: defaulting
    // `quantization` would silently rebuild the graph at a different scalar
    // width from the one the operator picked.
    if let Some(metric) = settings.distance_metric.as_deref() {
        match serde_json::from_value(serde_json::Value::String(metric.to_string())) {
            Ok(parsed) => target.distance_metric = parsed,
            Err(e) => tracing::warn!(
                tenant_id = %tenant_id,
                metric = %metric,
                error = %e,
                "Ignoring an unrecognised embedding distance metric; the stored one stands"
            ),
        }
    }
    if let Some(quantization) = settings.quantization.as_deref() {
        match serde_json::from_value(serde_json::Value::String(quantization.to_string())) {
            Ok(parsed) => target.quantization = parsed,
            Err(e) => tracing::warn!(
                tenant_id = %tenant_id,
                quantization = %quantization,
                error = %e,
                "Ignoring an unrecognised embedding quantization; the stored one stands"
            ),
        }
    }
    // Only overwrite the refs when the payload names one. A save that omits the
    // provider must not unbind an embedder whose hash is already in the keys of
    // every vector the tenant has written.
    if settings.ai_provider_ref.is_some() {
        target.ai_provider_ref = settings.ai_provider_ref.clone();
    }
    if settings.ai_model_ref.is_some() {
        target.ai_model_ref = settings.ai_model_ref.clone();
    }
    if let Some(model) = settings.ai_model_ref.as_deref() {
        // The legacy `model` field is what `ResolvedEmbeddingProvider::embedder_id`
        // falls back to, and `TenantEmbeddingConfig::new` leaves it saying
        // `text-embedding-3-small`. Keeping it in step costs nothing and stops a
        // legacy-mode read from labelling this tenant's vectors as OpenAI's.
        target.model = model.to_string();
    }

    if let Err(e) = repo.set_config(&target) {
        tracing::error!(
            tenant_id = %tenant_id,
            error = %e,
            "Saved the AI config but failed to mirror its embedding settings - vector search \
             will keep using the previous configuration"
        );
        return;
    }

    tracing::info!(
        tenant_id = %tenant_id,
        enabled = target.enabled,
        dimensions = target.dimensions,
        provider = ?target.ai_provider_ref,
        model = ?target.ai_model_ref,
        "Mirrored the AI config's embedding settings into the tenant embedding config"
    );
}

/// Reads the stored config, folds the request into it and writes the result back.
///
/// Split out of the handler so the read/merge/write sequence can be exercised against
/// a store that fails — the failure mode that matters here is not "the merge is wrong"
/// but "the merge ran against the wrong base and the write went ahead anyway".
async fn merge_and_store(
    store: &dyn TenantAIConfigStore,
    tenant_id: &str,
    req: &SetConfigRequest,
    encrypt: impl Fn(&str) -> Result<Vec<u8>, MergeError>,
) -> Result<TenantAIConfig, (StatusCode, Json<ErrorResponse>)> {
    // Read the stored config once, before the merge. It answers four questions per
    // entry — is this a create or an update, what kind is this slug already bound to,
    // what API key should be carried forward, and what should the fields the caller
    // omitted keep saying.
    //
    // "Nothing stored yet" and "the read failed" must not be conflated. A corrupt or
    // partially-migrated record deserializes to a `DeserializationError`, and treating
    // that as an empty base would merge the payload onto nothing and write it back —
    // deleting every entry the caller did not know about, which is the exact data loss
    // merge semantics exist to prevent, and doing it precisely when the stored data is
    // already in trouble. A failed read is a 500 with no write at all.
    let stored = match store.get_config(tenant_id).await {
        Ok(config) => Some(config),
        Err(raisin_ai::storage::StorageError::NotFound(_)) => None,
        Err(e) => {
            tracing::error!(
                tenant_id = %tenant_id,
                error = %e,
                "Refusing to save AI config: the stored config could not be read, so a merge \
                 would silently drop whatever it holds"
            );
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Storage error: {}", e),
                }),
            ));
        }
    };

    let providers = merge_providers(stored.as_ref(), &req.providers, encrypt)
        .map_err(MergeError::into_response_parts)?;

    let config = TenantAIConfig {
        tenant_id: tenant_id.to_string(),
        providers,
        // Both of these merge the same way the providers do: whatever is stored
        // survives a save that does not mention them. `processing_defaults` is not
        // part of this request body at all; `embedding_settings` is, but omitting it
        // is what every pre-slug client does, and honouring that as a delete drops
        // `dimensions` — which is hashed into the embedder identity and baked into
        // the key of every vector the tenant has already written.
        embedding_settings: req
            .embedding_settings
            .clone()
            .or_else(|| stored.as_ref().and_then(|c| c.embedding_settings.clone())),
        processing_defaults: stored.as_ref().and_then(|c| c.processing_defaults.clone()),
    };

    store.set_config(&config).await.map_err(|e| {
        tracing::error!("Failed to store AI config for {}: {}", tenant_id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Storage error: {}", e),
            }),
        )
    })?;

    Ok(config)
}

/// Why a merge was refused, and with which status.
enum MergeError {
    /// The payload itself is wrong — a duplicate slug, an unusable new slug, or an
    /// attempt to repoint an existing slug at a different kind.
    BadRequest(String),
    /// The server could not do its half of the work (encryption).
    Internal(String),
}

impl MergeError {
    fn into_response_parts(self) -> (StatusCode, Json<ErrorResponse>) {
        let (status, error) = match self {
            MergeError::BadRequest(error) => (StatusCode::BAD_REQUEST, error),
            MergeError::Internal(error) => (StatusCode::INTERNAL_SERVER_ERROR, error),
        };
        (status, Json(ErrorResponse { error }))
    }
}

/// Folds an incoming provider list into the stored one and returns the merged result.
///
/// Kept separate from the handler — and from storage — because the invariants worth
/// testing are all here: no stored entry is ever dropped, a slug keeps its kind, and
/// an omitted `api_key_plain` carries the stored secret forward.
///
/// `encrypt` turns a plaintext key into its stored form; it is a parameter so the
/// merge does not need a master key to be exercised.
fn merge_providers(
    stored: Option<&TenantAIConfig>,
    incoming: &[ProviderConfigRequest],
    encrypt: impl Fn(&str) -> Result<Vec<u8>, MergeError>,
) -> Result<Vec<AIProviderConfig>, MergeError> {
    // Start from storage, not from an empty vec: a slug the payload never mentions has
    // to come out the other side unchanged.
    let mut merged: Vec<AIProviderConfig> = stored.map(|c| c.providers.clone()).unwrap_or_default();

    let mut seen_slugs: HashSet<&str> = HashSet::new();

    for provider_req in incoming {
        let slug = provider_req.slug();

        let stored_provider =
            resolve_entry(stored, &mut seen_slugs, provider_req).map_err(MergeError::BadRequest)?;

        let api_key_encrypted = match provider_req.api_key_plain.as_deref() {
            // An empty string is a form field the user left alone, not a credential.
            // Encrypting it would store a key that authenticates as nothing while
            // `has_api_key` reports true — strictly worse than having none, because every
            // surface then says the provider is configured while every request 401s.
            Some("") => stored_provider.and_then(|existing| existing.api_key_encrypted.clone()),
            Some(plain_key) => Some(encrypt(plain_key)?),
            // Keep the existing API key when the caller did not resend it — matched by
            // slug, so an update to one of two same-kind entries cannot pick up the
            // other's key.
            None => stored_provider.and_then(|existing| existing.api_key_encrypted.clone()),
        };

        let provider_config = AIProviderConfig {
            slug: slug.to_string(),
            kind: provider_req.provider,
            // Merge the optional descriptive fields rather than taking them verbatim:
            // a client that does not know a field exists must not be able to erase it.
            // The admin console has never sent `display_name` or `icon_url`, so taking
            // them from the payload wiped the gateway's name and logo on its first
            // save; `api_endpoint` had the same exposure, and losing that unroutes the
            // entry outright.
            display_name: patch(
                &provider_req.display_name,
                stored_provider.and_then(|e| e.display_name.as_ref()),
            ),
            icon_url: patch(
                &provider_req.icon_url,
                stored_provider.and_then(|e| e.icon_url.as_ref()),
            ),
            api_key_encrypted,
            api_endpoint: patch(
                &provider_req.api_endpoint,
                stored_provider.and_then(|e| e.api_endpoint.as_ref()),
            ),
            enabled: provider_req.enabled,
            models: match &provider_req.models {
                Some(models) => models.clone().unwrap_or_default(),
                None => stored_provider
                    .map(|e| e.models.clone())
                    .unwrap_or_default(),
            },
        };

        match merged.iter_mut().find(|p| p.slug == slug) {
            Some(existing) => *existing = provider_config,
            None => merged.push(provider_config),
        }
    }

    Ok(merged)
}

/// Resolves one three-state payload field against the stored value.
///
/// Absent keeps the stored value, an explicit `null` clears it, a value sets it —
/// the same shape `api_key_plain` has had all along, made explicit in the type so
/// that "the client didn't mention it" can no longer read as "the client cleared it".
/// On a CREATE there is nothing stored, so absent and `null` coincide.
fn patch<T: Clone>(incoming: &Patch<T>, stored: Option<&T>) -> Option<T> {
    match incoming {
        Some(value) => value.clone(),
        None => stored.cloned(),
    }
}

/// Decides how one incoming entry relates to what is stored, and rejects anything
/// that would stop a slug being an address.
///
/// Returns the stored entry this request updates, or `None` when it is a create.
/// `Err` carries the message for a 400.
fn resolve_entry<'a, 'r>(
    stored: Option<&'a TenantAIConfig>,
    seen_slugs: &mut HashSet<&'r str>,
    provider_req: &'r ProviderConfigRequest,
) -> Result<Option<&'a AIProviderConfig>, String> {
    let slug = provider_req.slug();

    // Uniqueness within the payload is what makes a slug an address. Two entries
    // sharing one would make `<slug>:<model>` ambiguous, and `get_provider` would
    // silently answer with whichever was stored first.
    if !seen_slugs.insert(slug) {
        return Err(format!("Duplicate provider slug '{}'", slug));
    }

    let stored_provider = stored.and_then(|c| c.get_provider(slug));

    match stored_provider {
        // A slug's kind is as immutable as the slug itself, and for a sharper reason:
        // the stored API key is carried forward on any PUT that omits it, so a kind
        // switch would hand a gateway's credential to a client that speaks a different
        // wire protocol — every call under that slug then 404s upstream, with nothing
        // at save time to say why.
        Some(existing) if existing.kind != provider_req.provider => Err(format!(
            "Provider slug '{}' is configured as kind '{}' and cannot be changed to '{}'; \
             delete it and create it again under a new slug",
            slug,
            existing.kind.serde_name(),
            provider_req.provider.serde_name()
        )),
        Some(_) => Ok(stored_provider),
        // Validate on CREATE only. An entry already in storage may carry a slug this
        // predicate would not mint today, and slugs are immutable — validating on
        // update would make such an entry impossible to re-save, with no way for the
        // tenant to fix it.
        None => {
            // The kind goes in with the slug: the legacy kind-name exemption is only
            // an exemption for the entry's OWN kind, or an Anthropic entry could be
            // created under slug `openai` and take every legacy `openai:<model>` id
            // with it.
            validate_slug(slug, provider_req.provider)
                .map_err(|e| format!("Invalid provider slug '{}': {}", slug, e))?;
            Ok(None)
        }
    }
}

/// Drops the entry addressed by `slug`, reporting whether there was one.
///
/// Split out so the "exactly one entry, matched by slug" invariant is testable without
/// storage and a master key behind it.
fn remove_provider(providers: &mut Vec<AIProviderConfig>, slug: &str) -> bool {
    let before = providers.len();
    providers.retain(|p| p.slug != slug);
    providers.len() != before
}

/// Remove one provider entry from a tenant's configuration.
///
/// DELETE /api/tenants/{tenant_id}/ai/providers/{slug}
///
/// PUT is a merge and never deletes, so this is the only way an entry goes away —
/// which is deliberate: nothing holds a foreign key to a slug, so every
/// `<slug>:<model>` id, agent node and `ai_provider_ref` naming it simply stops
/// resolving, at call time, with no error at save time. Making that an explicit verb
/// means it can only happen because somebody asked for it.
///
/// The removal is by slug, so deleting one of two same-kind gateways leaves the other
/// — and its encrypted key — alone.
#[axum::debug_handler]
pub async fn delete_ai_provider(
    Path((tenant_id, slug)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<SuccessResponse>, (StatusCode, Json<ErrorResponse>)> {
    let repo_impl = state.storage().tenant_ai_config_repository();

    let not_configured = || {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Provider '{}' not configured", slug),
            }),
        )
    };

    let mut config = match repo_impl.get_config(&tenant_id).await {
        Ok(config) => config,
        Err(raisin_ai::storage::StorageError::NotFound(_)) => return Err(not_configured()),
        Err(e) => {
            tracing::error!("Failed to get AI config for {}: {}", tenant_id, e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Storage error: {}", e),
                }),
            ));
        }
    };

    if !remove_provider(&mut config.providers, &slug) {
        // 404 rather than a silent success: a caller that mistyped the slug would
        // otherwise be told its provider is gone while it is still live.
        return Err(not_configured());
    }

    repo_impl.set_config(&config).await.map_err(|e| {
        tracing::error!("Failed to store AI config for {}: {}", tenant_id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Storage error: {}", e),
            }),
        )
    })?;

    tracing::warn!(
        tenant_id = %tenant_id,
        slug = %slug,
        "Removed AI provider entry; any model id, agent node or ai_provider_ref \
         naming this slug will stop resolving"
    );

    Ok(Json(SuccessResponse {
        success: true,
        message: format!("Provider '{}' removed from tenant {}", slug, tenant_id),
    }))
}

/// List all configured providers.
///
/// GET /api/tenants/{tenant_id}/ai/providers
#[axum::debug_handler]
pub async fn list_providers(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<ProvidersListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = &tenant_id;
    let repo_impl = state.storage().tenant_ai_config_repository();

    match repo_impl.get_config(tenant_id).await {
        Ok(config) => {
            let providers = config
                .providers
                .into_iter()
                .map(|p| ProviderSummary {
                    slug: p.slug,
                    provider: p.kind,
                    display_name: p.display_name,
                    icon_url: p.icon_url,
                    // Not a secret, and the only field that tells two `custom`-kind
                    // gateways apart in a list — which is what slugs exist for.
                    api_endpoint: p.api_endpoint,
                    enabled: p.enabled,
                    has_api_key: p.api_key_encrypted.is_some(),
                    model_count: p.models.len(),
                })
                .collect();

            Ok(Json(ProvidersListResponse { providers }))
        }
        Err(raisin_ai::storage::StorageError::NotFound(_)) => {
            // Return empty list if not configured
            Ok(Json(ProvidersListResponse {
                providers: Vec::new(),
            }))
        }
        Err(e) => {
            tracing::error!("Failed to get AI config for {}: {}", tenant_id, e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Storage error: {}", e),
                }),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_ai::config::{AIModelConfig, AIProvider};

    /// A payload entry as a pre-slug client sends one: the kind, `enabled`, and
    /// nothing else. Every optional field is ABSENT, which under merge semantics
    /// means "keep what is stored" — building these with `Some(None)` instead would
    /// quietly test the clearing path and miss the bug this file is full of.
    fn request(slug: Option<&str>, kind: AIProvider) -> ProviderConfigRequest {
        ProviderConfigRequest {
            slug: slug.map(str::to_string),
            provider: kind,
            display_name: None,
            icon_url: None,
            api_key_plain: None,
            api_endpoint: None,
            enabled: true,
            models: None,
        }
    }

    fn stored(entries: Vec<AIProviderConfig>) -> TenantAIConfig {
        TenantAIConfig {
            providers: entries,
            ..TenantAIConfig::new("t".to_string())
        }
    }

    /// A client written before slugs existed PUTs the config it just GET'd, with no
    /// `slug` anywhere. Every entry has to land on the stored entry of the same kind —
    /// otherwise the save reads as "delete everything, create everything", and every
    /// API key the caller did not resend is lost.
    #[test]
    fn a_payload_without_slugs_updates_the_entries_it_meant_to_update() {
        let cfg = stored(vec![AIProviderConfig::new(AIProvider::OpenAI)]);
        let req = request(None, AIProvider::OpenAI);
        let mut seen = HashSet::new();

        let resolved = resolve_entry(Some(&cfg), &mut seen, &req).expect("valid");
        assert_eq!(resolved.map(|p| p.slug.as_str()), Some("openai"));
    }

    /// The api-key-preservation lookup runs through here, so this is what stops an
    /// edit to one gateway from silently adopting the other's credentials.
    #[test]
    fn two_entries_of_the_same_kind_resolve_to_their_own_stored_entry() {
        let cfg = stored(vec![
            AIProviderConfig::with_slug("marvel", AIProvider::Custom),
            AIProviderConfig::with_slug("my-vllm", AIProvider::Custom),
        ]);
        let mut seen = HashSet::new();

        let marvel = request(Some("marvel"), AIProvider::Custom);
        let vllm = request(Some("my-vllm"), AIProvider::Custom);

        assert_eq!(
            resolve_entry(Some(&cfg), &mut seen, &marvel)
                .unwrap()
                .map(|p| p.slug.as_str()),
            Some("marvel")
        );
        assert_eq!(
            resolve_entry(Some(&cfg), &mut seen, &vllm)
                .unwrap()
                .map(|p| p.slug.as_str()),
            Some("my-vllm")
        );
    }

    /// Two entries under one slug make `<slug>:<model>` ambiguous — the second would
    /// simply be unreachable, with no error at read time to say so.
    #[test]
    fn a_duplicate_slug_in_one_payload_is_rejected() {
        let mut seen = HashSet::new();
        let first = request(Some("marvel"), AIProvider::Custom);
        let second = request(Some("marvel"), AIProvider::Ollama);

        assert!(resolve_entry(None, &mut seen, &first).is_ok());
        let err = resolve_entry(None, &mut seen, &second).expect_err("duplicate");
        assert!(err.contains("Duplicate"), "got: {err}");
    }

    /// A slug is the prefix half of a model id, so a colon inside one would make
    /// `parse_model_id` split in the wrong place and resolve to the wrong provider —
    /// or to none.
    #[test]
    fn a_new_slug_that_would_break_a_model_id_is_rejected() {
        for bad in ["has:colon", "Upper", "-leading", "has space", ""] {
            let mut seen = HashSet::new();
            let req = request(Some(bad), AIProvider::Custom);
            assert!(
                resolve_entry(None, &mut seen, &req).is_err(),
                "'{bad}' should be rejected"
            );
        }
    }

    /// `azure_openai` is the legacy default slug for the Azure kind and does not match
    /// the slug pattern — `_` is not in the character class. It still has to be
    /// accepted on CREATE, not just on update: the admin console sends the ten kind
    /// names on every save, for kinds the tenant has never configured, so rejecting it
    /// 400s the whole PUT and no AI setting can be saved by anyone.
    #[test]
    fn a_kind_named_slug_is_accepted_even_when_nothing_is_stored_under_it() {
        let mut seen = HashSet::new();
        let req = request(Some("azure_openai"), AIProvider::AzureOpenAI);
        assert!(resolve_entry(None, &mut seen, &req).is_ok());

        // And it re-saves once it exists.
        let cfg = stored(vec![AIProviderConfig::new(AIProvider::AzureOpenAI)]);
        assert_eq!(cfg.providers[0].slug, "azure_openai");
        let mut seen = HashSet::new();
        assert!(resolve_entry(Some(&cfg), &mut seen, &req).is_ok());
    }

    /// The stored API key is carried forward whenever the caller omits it, so a kind
    /// switch would hand a gateway's credential to a client speaking a different wire
    /// protocol: `marvel` repointed at `openai` keeps the gateway key and then POSTs
    /// `{base}/responses` on every call, 404ing forever with nothing at save time to
    /// explain it.
    #[test]
    fn the_kind_behind_an_existing_slug_cannot_be_changed() {
        let cfg = stored(vec![AIProviderConfig::with_slug(
            "marvel",
            AIProvider::Custom,
        )]);
        let mut seen = HashSet::new();
        let repoint = request(Some("marvel"), AIProvider::OpenAI);

        let err = resolve_entry(Some(&cfg), &mut seen, &repoint).expect_err("kind change");
        assert!(err.contains("custom"), "should name the stored kind: {err}");
        assert!(err.contains("openai"), "should name the new kind: {err}");
    }

    // ------------------------------------------------------------------
    // Merge semantics
    // ------------------------------------------------------------------

    /// Encryption stand-in: the merge only has to move opaque bytes around.
    fn fake_encrypt(plain: &str) -> Result<Vec<u8>, MergeError> {
        Ok(format!("enc:{plain}").into_bytes())
    }

    fn merge(
        stored_cfg: Option<&TenantAIConfig>,
        incoming: &[ProviderConfigRequest],
    ) -> Result<Vec<AIProviderConfig>, String> {
        merge_providers(stored_cfg, incoming, fake_encrypt).map_err(|e| match e {
            MergeError::BadRequest(m) | MergeError::Internal(m) => m,
        })
    }

    fn find<'a>(providers: &'a [AIProviderConfig], slug: &str) -> &'a AIProviderConfig {
        providers
            .iter()
            .find(|p| p.slug == slug)
            .unwrap_or_else(|| panic!("'{slug}' should still be configured"))
    }

    /// PUT is a merge. A stored slug the payload never mentions survives untouched,
    /// encrypted key included — the delete-by-absence version of this handler wiped a
    /// tenant's gateway, and its only copy of the credential, on a routine save.
    #[test]
    fn a_put_that_omits_a_stored_slug_leaves_that_entry_and_its_key_intact() {
        let mut gateway = AIProviderConfig::with_slug("marvel", AIProvider::Custom);
        gateway.api_key_encrypted = Some(b"gateway-secret".to_vec());
        gateway.api_endpoint = Some("https://marvel.internal/v1".to_string());
        let cfg = stored(vec![gateway, AIProviderConfig::new(AIProvider::OpenAI)]);

        let merged = merge(Some(&cfg), &[request(None, AIProvider::OpenAI)]).expect("merged");

        assert_eq!(merged.len(), 2, "nothing may be dropped: {merged:#?}");
        let marvel = find(&merged, "marvel");
        assert_eq!(marvel.kind, AIProvider::Custom);
        assert_eq!(
            marvel.api_key_encrypted.as_deref(),
            Some(&b"gateway-secret"[..])
        );
        assert_eq!(
            marvel.api_endpoint.as_deref(),
            Some("https://marvel.internal/v1")
        );
    }

    /// The payload the admin console actually sends (TenantAiSettings.tsx, `handleSave`):
    /// all ten kinds, no `slug` on any of them, `local` appended last. It has to save,
    /// and it has to leave a self-named gateway alone — that combination is what makes
    /// this change shippable ahead of the console rewrite.
    #[test]
    fn the_consoles_ten_kind_slugless_payload_saves_and_preserves_a_self_named_gateway() {
        let mut gateway = AIProviderConfig::with_slug("marvel", AIProvider::Custom);
        gateway.api_key_encrypted = Some(b"gateway-secret".to_vec());
        let cfg = stored(vec![
            gateway,
            AIProviderConfig::new(AIProvider::OpenAI),
            AIProviderConfig::new(AIProvider::Custom),
        ]);

        // Every kind except `local`, then `local` — the console's exact ordering.
        let payload: Vec<ProviderConfigRequest> = [
            AIProvider::OpenAI,
            AIProvider::Anthropic,
            AIProvider::Google,
            AIProvider::AzureOpenAI,
            AIProvider::Ollama,
            AIProvider::Groq,
            AIProvider::OpenRouter,
            AIProvider::Bedrock,
            AIProvider::Custom,
            AIProvider::Local,
        ]
        .into_iter()
        .map(|kind| request(None, kind))
        .collect();

        let merged = merge(Some(&cfg), &payload).expect("the console's payload must save");

        // Ten kind-named entries plus the gateway the console knows nothing about.
        assert_eq!(merged.len(), 11, "{merged:#?}");
        assert_eq!(
            find(&merged, "marvel").api_key_encrypted.as_deref(),
            Some(&b"gateway-secret"[..])
        );
        // `azure_openai` had to be created, underscore and all.
        assert_eq!(find(&merged, "azure_openai").kind, AIProvider::AzureOpenAI);
        // The kind-named `custom` entry is a different entry from `marvel`.
        assert_eq!(find(&merged, "custom").kind, AIProvider::Custom);
    }

    /// A genuinely new slug is still held to the pattern: it is the prefix half of a
    /// model id, and a `:` or a space in it makes `<slug>:<model>` resolve to the wrong
    /// provider, or to none.
    #[test]
    fn a_new_slug_that_is_not_a_kind_name_must_still_match_the_pattern() {
        for bad in ["Not A Slug!", "has:colon"] {
            let err = merge(None, &[request(Some(bad), AIProvider::Custom)])
                .expect_err("'{bad}' should be rejected");
            assert!(err.contains("Invalid provider slug"), "got: {err}");
        }

        assert!(merge(None, &[request(Some("marvel"), AIProvider::Custom)]).is_ok());
    }

    /// An omitted `api_key_plain` keeps the stored secret; a supplied one replaces it.
    /// This is how a client saves an endpoint edit without holding the credential.
    #[test]
    fn an_omitted_api_key_is_carried_forward_and_a_supplied_one_replaces_it() {
        let mut gateway = AIProviderConfig::with_slug("marvel", AIProvider::Custom);
        gateway.api_key_encrypted = Some(b"old".to_vec());
        let cfg = stored(vec![gateway]);

        let merged = merge(Some(&cfg), &[request(Some("marvel"), AIProvider::Custom)]).unwrap();
        assert_eq!(
            find(&merged, "marvel").api_key_encrypted.as_deref(),
            Some(&b"old"[..])
        );

        let mut rotate = request(Some("marvel"), AIProvider::Custom);
        rotate.api_key_plain = Some("new".to_string());
        let merged = merge(Some(&cfg), &[rotate]).unwrap();
        assert_eq!(
            find(&merged, "marvel").api_key_encrypted.as_deref(),
            Some(&b"enc:new"[..])
        );
    }

    /// A form that serializes its untouched password field as `""` must not be able to
    /// replace a working credential with one that authenticates as nothing — which is
    /// worse than no key at all, because `has_api_key` would still report true and every
    /// surface would claim the provider is configured while every request 401s.
    #[test]
    fn an_empty_api_key_string_is_not_a_credential() {
        let mut gateway = AIProviderConfig::with_slug("marvel", AIProvider::Custom);
        gateway.api_key_encrypted = Some(b"old".to_vec());
        let cfg = stored(vec![gateway]);

        let mut blank = request(Some("marvel"), AIProvider::Custom);
        blank.api_key_plain = Some(String::new());
        let merged = merge(Some(&cfg), &[blank]).unwrap();

        assert_eq!(
            find(&merged, "marvel").api_key_encrypted.as_deref(),
            Some(&b"old"[..]),
            "an empty string means the field was left alone, not that the key should go"
        );
    }

    /// `models` was the last provider field taking the payload verbatim. The gateway's
    /// catalogue is synced into it by flightdeck, and the admin console does not manage
    /// it — so a console save that simply does not mention models was blanking it, and
    /// every `marvel:<alias>` id stopped resolving on a deployment that still needs the
    /// strict lookup.
    #[test]
    fn an_omitted_model_list_is_kept_and_an_explicit_one_replaces_it() {
        let mut gateway = AIProviderConfig::with_slug("marvel", AIProvider::Custom);
        gateway.models = vec![AIModelConfig::new(
            "maravilla/smart".to_string(),
            "maravilla/smart".to_string(),
        )];
        let cfg = stored(vec![gateway]);

        let merged = merge(Some(&cfg), &[request(Some("marvel"), AIProvider::Custom)]).unwrap();
        assert_eq!(
            find(&merged, "marvel").models.len(),
            1,
            "absent means keep the synced catalogue"
        );

        let mut clear = request(Some("marvel"), AIProvider::Custom);
        clear.models = Some(Some(Vec::new()));
        let merged = merge(Some(&cfg), &[clear]).unwrap();
        assert!(
            find(&merged, "marvel").models.is_empty(),
            "an explicit empty list still clears it — that is the caller saying so"
        );
    }

    /// `openai` passes the slug pattern on its own merits, so only the kind-aware
    /// check stands between an Anthropic entry and the slug that every legacy
    /// `openai:gpt-4o` id in the tenant's data points at. Slugs are immutable, so
    /// accepting it once mis-routes those ids for good.
    #[test]
    fn a_slug_naming_a_kind_other_than_its_own_is_rejected_on_create() {
        let mut anthropic_under_openai = request(Some("openai"), AIProvider::Anthropic);
        anthropic_under_openai.api_key_plain = Some("sk-ant".to_string());

        let err = merge(None, &[anthropic_under_openai]).expect_err("wrong kind name");
        assert!(err.contains("Invalid provider slug 'openai'"), "got: {err}");

        // The same slug for the kind it actually names is the legacy default, and has
        // to keep working — it is what every pre-slug client sends.
        let merged = merge(None, &[request(Some("openai"), AIProvider::OpenAI)])
            .expect("the legacy default slug must still create");
        assert_eq!(find(&merged, "openai").kind, AIProvider::OpenAI);
    }

    /// The admin console sends neither `display_name` nor `icon_url`, and sends
    /// `api_endpoint` only for the entries it has fields for. Taking those verbatim
    /// erased the gateway's "Maravilla" name, its logo and its endpoint on the first
    /// ordinary save — an update must merge fields, not replace the entry.
    #[test]
    fn a_save_that_does_not_mention_a_field_keeps_the_stored_value() {
        let mut gateway = AIProviderConfig::with_slug("marvel", AIProvider::Custom);
        gateway.display_name = Some("Maravilla".to_string());
        gateway.icon_url = Some("https://maravilla.cloud/logo.svg".to_string());
        gateway.api_endpoint = Some("https://marvel.maravilla.cloud/v1".to_string());
        gateway.api_key_encrypted = Some(b"gateway-secret".to_vec());
        // Stored under the kind name, which is what the console addresses — this is the
        // entry a pre-rewrite console actually overwrites on save.
        gateway.slug = "custom".to_string();
        let cfg = stored(vec![gateway]);

        // A console-shaped entry: kind, `enabled`, no slug, and none of the fields it
        // has no input for.
        let merged = merge(Some(&cfg), &[request(None, AIProvider::Custom)]).unwrap();

        let marvel = find(&merged, "custom");
        assert_eq!(marvel.display_name.as_deref(), Some("Maravilla"));
        assert_eq!(
            marvel.icon_url.as_deref(),
            Some("https://maravilla.cloud/logo.svg")
        );
        assert_eq!(
            marvel.api_endpoint.as_deref(),
            Some("https://marvel.maravilla.cloud/v1")
        );
        assert_eq!(
            marvel.api_key_encrypted.as_deref(),
            Some(&b"gateway-secret"[..])
        );
    }

    /// The other half of the three-state rule: a client that means to clear a field
    /// still can, by sending `null` — otherwise a display name could never be removed
    /// once set. `Some(None)` is what an explicit `null` deserializes to.
    #[test]
    fn an_explicitly_null_field_clears_the_stored_value() {
        let mut gateway = AIProviderConfig::with_slug("marvel", AIProvider::Custom);
        gateway.display_name = Some("Maravilla".to_string());
        gateway.icon_url = Some("https://maravilla.cloud/logo.svg".to_string());
        let cfg = stored(vec![gateway]);

        let mut clear = request(Some("marvel"), AIProvider::Custom);
        clear.display_name = Some(None);
        clear.icon_url = Some(Some("https://maravilla.cloud/new.svg".to_string()));

        let merged = merge(Some(&cfg), &[clear]).unwrap();
        let marvel = find(&merged, "marvel");
        assert_eq!(marvel.display_name, None, "an explicit null clears");
        assert_eq!(
            marvel.icon_url.as_deref(),
            Some("https://maravilla.cloud/new.svg"),
            "a value sets"
        );
    }

    /// Deletion is explicit, and it removes exactly the one entry named — the other
    /// gateway, and its key, must be untouched.
    #[test]
    fn deleting_one_slug_leaves_every_other_entry_in_place() {
        let mut keep = AIProviderConfig::with_slug("my-vllm", AIProvider::Custom);
        keep.api_key_encrypted = Some(b"vllm-secret".to_vec());
        let mut providers = vec![
            AIProviderConfig::with_slug("marvel", AIProvider::Custom),
            keep,
            AIProviderConfig::new(AIProvider::OpenAI),
        ];

        // A slug nobody configured is not a delete — the caller mistyped.
        assert!(!remove_provider(&mut providers, "marvle"));
        assert_eq!(providers.len(), 3);

        assert!(remove_provider(&mut providers, "marvel"));

        assert_eq!(providers.len(), 2);
        assert!(providers.iter().all(|p| p.slug != "marvel"));
        assert_eq!(
            find(&providers, "my-vllm").api_key_encrypted.as_deref(),
            Some(&b"vllm-secret"[..])
        );
        assert_eq!(find(&providers, "openai").kind, AIProvider::OpenAI);
    }

    // ------------------------------------------------------------------
    // Read/merge/write sequencing
    // ------------------------------------------------------------------

    use raisin_ai::config::EmbeddingSettings;
    use raisin_ai::storage::{Result as StorageResult, StorageError};
    use std::sync::Mutex;

    /// A store whose read outcome is dictated by the test, and which records every
    /// write. The point of the recording is negative: some failures have to produce
    /// no write at all, and only the store can testify to that.
    struct ScriptedStore {
        read: Box<dyn Fn() -> StorageResult<TenantAIConfig> + Send + Sync>,
        writes: Mutex<Vec<TenantAIConfig>>,
    }

    impl ScriptedStore {
        fn reading(
            read: impl Fn() -> StorageResult<TenantAIConfig> + Send + Sync + 'static,
        ) -> Self {
            Self {
                read: Box::new(read),
                writes: Mutex::new(Vec::new()),
            }
        }

        fn writes(&self) -> Vec<TenantAIConfig> {
            self.writes.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl TenantAIConfigStore for ScriptedStore {
        async fn get_config(&self, _tenant_id: &str) -> StorageResult<TenantAIConfig> {
            (self.read)()
        }

        async fn set_config(&self, config: &TenantAIConfig) -> StorageResult<()> {
            self.writes.lock().unwrap().push(config.clone());
            Ok(())
        }

        async fn delete_config(&self, _tenant_id: &str) -> StorageResult<()> {
            unimplemented!("not exercised by these tests")
        }

        async fn list_tenant_ids(&self) -> StorageResult<Vec<String>> {
            unimplemented!("not exercised by these tests")
        }
    }

    fn set_config_request(
        providers: Vec<ProviderConfigRequest>,
        embedding_settings: Option<EmbeddingSettings>,
    ) -> SetConfigRequest {
        SetConfigRequest {
            providers,
            embedding_settings,
        }
    }

    /// A read that fails is not an empty config. Collapsing the two — which is what
    /// `.ok()` did — merges the payload onto nothing and writes it back, deleting every
    /// entry the caller never knew about: the precise data loss merge semantics exist to
    /// prevent, triggered by a corrupt or partially-migrated record. Nothing may be
    /// written on a failed read.
    #[tokio::test]
    async fn a_stored_config_that_cannot_be_read_fails_the_save_without_writing_anything() {
        // The two ways the repository actually fails a read: a record it cannot
        // deserialize, and a backend that would not hand one over.
        let cases: [(&str, fn() -> StorageError); 2] = [
            ("a corrupt or partially-migrated record", || {
                StorageError::DeserializationError("truncated record".to_string())
            }),
            ("a backend read failure", || {
                StorageError::BackendError("Failed to read from storage: IO error".to_string())
            }),
        ];

        for (label, failure) in cases {
            let store = ScriptedStore::reading(move || Err(failure()));

            let req = set_config_request(vec![request(None, AIProvider::OpenAI)], None);
            let (status, _) = merge_and_store(&store, "t", &req, fake_encrypt)
                .await
                .expect_err("a failed read must not be treated as an empty config");

            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "for {label}");
            assert!(
                store.writes().is_empty(),
                "nothing may be written when the base is unknown ({label})"
            );
        }
    }

    /// A tenant with no config yet is legitimately empty, and must still be able to
    /// save one — the 500 above is for a read that failed, not for a read that found
    /// nothing.
    #[tokio::test]
    async fn a_tenant_with_nothing_stored_can_still_save_a_first_config() {
        let store = ScriptedStore::reading(|| Err(StorageError::NotFound("t".to_string())));

        let req = set_config_request(vec![request(None, AIProvider::OpenAI)], None);
        merge_and_store(&store, "t", &req, fake_encrypt)
            .await
            .expect("a first save must work");

        let writes = store.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].providers[0].slug, "openai");
    }

    /// `embedding_settings` is `#[serde(default)]`, so any client that PUTs a config
    /// without it — every pre-slug one — arrives here with `None`. Honouring that as a
    /// delete drops `dimensions`, which is hashed into the embedder identity and baked
    /// into the key of every vector the tenant has written: the settings go, and every
    /// stored embedding is orphaned.
    #[tokio::test]
    async fn a_save_that_omits_embedding_settings_keeps_the_stored_ones() {
        let settings = EmbeddingSettings {
            enabled: true,
            dimensions: 3072,
            ..EmbeddingSettings::default()
        };
        let base = TenantAIConfig {
            embedding_settings: Some(settings),
            ..TenantAIConfig::new("t".to_string())
        };
        let store = ScriptedStore::reading(move || Ok(base.clone()));

        let req = set_config_request(vec![request(None, AIProvider::OpenAI)], None);
        let saved = merge_and_store(&store, "t", &req, fake_encrypt)
            .await
            .expect("saved");

        let kept = saved
            .embedding_settings
            .as_ref()
            .expect("embedding settings must survive a payload that omits them");
        assert!(kept.enabled);
        assert_eq!(
            kept.dimensions, 3072,
            "dimensions is the vector index's identity; losing it orphans every embedding"
        );
        // And what was written is what was returned.
        assert_eq!(
            store.writes()[0]
                .embedding_settings
                .as_ref()
                .map(|s| s.dimensions),
            Some(3072)
        );
    }

    /// Supplied settings still win, or the endpoint could never change them.
    #[tokio::test]
    async fn supplied_embedding_settings_replace_the_stored_ones() {
        let base = TenantAIConfig {
            embedding_settings: Some(EmbeddingSettings {
                dimensions: 3072,
                ..EmbeddingSettings::default()
            }),
            ..TenantAIConfig::new("t".to_string())
        };
        let store = ScriptedStore::reading(move || Ok(base.clone()));

        let req = set_config_request(
            Vec::new(),
            Some(EmbeddingSettings {
                dimensions: 1536,
                ..EmbeddingSettings::default()
            }),
        );
        let saved = merge_and_store(&store, "t", &req, fake_encrypt)
            .await
            .expect("saved");

        assert_eq!(saved.embedding_settings.map(|s| s.dimensions), Some(1536));
    }
}
