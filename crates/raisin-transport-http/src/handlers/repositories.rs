//! Repository management HTTP handlers
//!
//! These endpoints manage repositories within a tenant's context.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use raisin_context::{RepositoryConfig, RepositoryInfo};
use raisin_storage::{
    BranchRepository, RegistryRepository, RepositoryManagementRepository, Storage,
};
use serde::Deserialize;

use crate::middleware::TenantInfo;
use crate::{error::ApiError, state::AppState};

/// Request to create a new repository
#[derive(Debug, Deserialize)]
pub struct CreateRepositoryRequest {
    /// Repository identifier (e.g., "website", "blog")
    pub repo_id: String,

    /// Repository description
    #[serde(default)]
    pub description: Option<String>,

    /// Default branch name (defaults to "main")
    #[serde(default)]
    pub default_branch: Option<String>,

    /// Default language: the language base content is stored in (defaults to
    /// "en"). Normalized as a BCP-47 tag (`EN-us` -> `en-US`). Can be changed
    /// later with `PATCH /api/repositories/{repo}/translation-config`.
    #[serde(default)]
    pub default_language: Option<String>,

    /// Supported languages for translations (defaults to [default_language])
    #[serde(default)]
    pub supported_languages: Option<Vec<String>>,
}

/// Request to update repository configuration
#[derive(Debug, Deserialize)]
pub struct UpdateRepositoryRequest {
    /// Repository description
    #[serde(default)]
    pub description: Option<String>,

    /// Default branch name
    #[serde(default)]
    pub default_branch: Option<String>,

    /// Supported languages for translations. The default language is kept as it
    /// is; change it with `PATCH /api/repositories/{repo}/translation-config`.
    #[serde(default)]
    pub supported_languages: Option<Vec<String>>,

    /// Settings of the localized name index (plan Phase 12):
    /// `{ "enforce_unique": false }`. Omitted: kept as it is.
    #[serde(default)]
    pub localized_names: Option<raisin_context::LocalizedNameConfig>,
}

/// Request to update translation configuration
#[derive(Debug, Deserialize)]
pub struct UpdateTranslationConfigRequest {
    /// New default language (BCP-47, normalized like repository create).
    ///
    /// Changing it is refused with 409 while translation overlays in the new
    /// language exist, and queues a full-text rebuild of every branch.
    #[serde(default)]
    pub default_language: Option<String>,

    /// Supported languages for translations.
    /// The default language is always added when missing.
    #[serde(default)]
    pub supported_languages: Option<Vec<String>>,

    /// Locale fallback chains for translation resolution
    /// Maps a locale to its fallback sequence
    /// Example: {"fr-CA": ["fr", "en"], "de-CH": ["de", "en"]}
    #[serde(default)]
    pub locale_fallback_chains: Option<std::collections::HashMap<String, Vec<String>>>,
}

/// List all repositories for a tenant
///
/// # Endpoint
/// GET /api/repositories
///
/// # Headers
/// X-Tenant-ID: {tenant_id} (defaults to "default" in single-tenant mode)
pub async fn list_repositories(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<Vec<RepositoryInfo>>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let storage = state.storage();
    let repo_mgmt = storage.repository_management();

    let repos = repo_mgmt.list_repositories_for_tenant(tenant_id).await?;
    Ok(Json(repos))
}

/// Get repository information
///
/// # Endpoint
/// GET /api/repositories/{repo_id}
///
/// # Headers
/// X-Tenant-ID: {tenant_id}
pub async fn get_repository(
    State(state): State<AppState>,
    Path(repo_id): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<RepositoryInfo>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let storage = state.storage();
    let repo_mgmt = storage.repository_management();

    let repo = repo_mgmt
        .get_repository(tenant_id, &repo_id)
        .await?
        .ok_or_else(|| ApiError::repository_not_found(&repo_id))?;

    Ok(Json(repo))
}

/// Create a new repository
///
/// # Endpoint
/// POST /api/repositories
///
/// # Headers
/// X-Tenant-ID: {tenant_id}
///
/// # Body
/// ```json
/// {
///   "repo_id": "website",
///   "name": "Corporate Website",
///   "description": "Main corporate website content",
///   "default_branch": "main"
/// }
/// ```
pub async fn create_repository(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<RepositoryInfo>), ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let storage = state.storage();
    let repo_mgmt = storage.repository_management();
    let branches = storage.branches();

    // Ensure tenant is registered (will emit TenantCreated event if new)
    // This triggers admin user initialization for new tenants
    let registry = storage.registry();
    registry
        .register_tenant(tenant_id, std::collections::HashMap::new())
        .await?;

    // The desired default branch — we need this before the repo-exists
    // check so we can decide whether to fall through to branch-create.
    let requested_default_branch = req
        .default_branch
        .clone()
        .unwrap_or_else(|| "main".to_string());

    // Check whether this is a brand-new repo or a self-heal call (the repo
    // already exists but its default branch is missing — usually because an
    // earlier create raced with branch creation and silently dropped the
    // branch error in pre-v0.1.20 builds).
    let repo_exists = repo_mgmt.repository_exists(tenant_id, &req.repo_id).await?;

    let repo = if repo_exists {
        let branch_exists = branches
            .get_branch(tenant_id, &req.repo_id, &requested_default_branch)
            .await
            .is_ok();
        if branch_exists {
            return Err(ApiError::repository_already_exists(&req.repo_id));
        }
        // Repo exists but default branch is missing — fall through to the
        // branch-create step so the caller can self-heal the half-created
        // state. Return the existing repo metadata for the response.
        tracing::warn!(
            tenant_id = %tenant_id,
            repo_id = %req.repo_id,
            default_branch = %requested_default_branch,
            "Repository exists but default branch is missing; completing branch creation"
        );
        repo_mgmt
            .get_repository(tenant_id, &req.repo_id)
            .await?
            .ok_or_else(|| {
                ApiError::internal(format!(
                    "Repository '{}' existed during check but get_repository returned None",
                    req.repo_id
                ))
            })?
    } else {
        // Determine default language, normalized as a BCP-47 tag
        let default_language = match req.default_language.as_deref() {
            Some(code) => normalize_language(code)?,
            None => "en".to_string(),
        };

        // Ensure supported languages includes default language
        let mut supported_languages = req
            .supported_languages
            .unwrap_or_else(|| vec![default_language.clone()]);
        if !supported_languages.contains(&default_language) {
            supported_languages.push(default_language.clone());
        }

        let config = RepositoryConfig {
            default_branch: requested_default_branch.clone(),
            description: req.description,
            tags: std::collections::HashMap::new(),
            default_language,
            supported_languages,
            locale_fallback_chains: std::collections::HashMap::new(),
            localized_names: Default::default(),
        };

        repo_mgmt
            .create_repository(tenant_id, &req.repo_id, config)
            .await?
    };

    // Create the default branch. Errors here are NOT swallowed — a half-
    // created repo (with no main branch) is unusable downstream and was the
    // root cause of v0.1.19's customer-facing system-updates 500s.
    branches
        .create_branch(
            tenant_id,
            &req.repo_id,
            &requested_default_branch,
            "system", // created_by
            None,     // from_revision - start from scratch
            None,     // upstream_branch - main has no upstream
            false,    // protected
            false,    // include_revision_history - not applicable for new repo
        )
        .await
        .map_err(|e| {
            tracing::error!(
                tenant_id = %tenant_id,
                repo_id = %req.repo_id,
                default_branch = %requested_default_branch,
                error = %format!("{:#}", e),
                "Failed to create default branch for repository"
            );
            ApiError::internal(format!(
                "Repository '{}' created but default branch '{}' creation failed: {}. \
                 Retry the create call to complete branch initialization.",
                req.repo_id, requested_default_branch, e
            ))
        })?;

    tracing::info!(
        tenant_id = %tenant_id,
        repo_id = %req.repo_id,
        default_branch = %requested_default_branch,
        "Created default branch for repository"
    );

    Ok((StatusCode::CREATED, Json(repo)))
}

/// Update repository configuration
///
/// # Endpoint
/// PUT /api/repositories/{repo_id}
///
/// # Headers
/// X-Tenant-ID: {tenant_id}
pub async fn update_repository(
    State(state): State<AppState>,
    Path(repo_id): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<UpdateRepositoryRequest>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let storage = state.storage();
    let repo_mgmt = storage.repository_management();

    // Get existing repository to preserve unchanged fields
    let existing = repo_mgmt
        .get_repository(tenant_id, &repo_id)
        .await?
        .ok_or_else(|| ApiError::repository_not_found(&repo_id))?;

    // Validate that supported languages includes default language if being updated
    let supported_languages = if let Some(mut langs) = req.supported_languages {
        if !langs.contains(&existing.config.default_language) {
            langs.push(existing.config.default_language.clone());
        }
        langs
    } else {
        existing.config.supported_languages
    };

    let config = RepositoryConfig {
        default_branch: req.default_branch.unwrap_or(existing.config.default_branch),
        description: req.description.or(existing.config.description),
        tags: existing.config.tags, // Preserve existing tags
        // Kept as it is: changing it is a re-index, done by PATCH translation-config
        default_language: existing.config.default_language,
        supported_languages,
        locale_fallback_chains: existing.config.locale_fallback_chains, // Preserve existing fallback chains,
        localized_names: req
            .localized_names
            .unwrap_or(existing.config.localized_names),
    };

    repo_mgmt
        .update_repository_config(tenant_id, &repo_id, config)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Update translation configuration for a repository
///
/// # Endpoint
/// PATCH /api/repositories/{repo_id}/translation-config
///
/// # Headers
/// X-Tenant-ID: {tenant_id}
///
/// # Body
/// Every field is optional.
/// ```json
/// {
///   "default_language": "de",
///   "supported_languages": ["en", "fr", "fr-CA", "de", "de-CH"],
///   "locale_fallback_chains": {
///     "fr-CA": ["fr", "en"],
///     "de-CH": ["de", "en"]
///   }
/// }
/// ```
///
/// # Response
/// 200 with the stored repository configuration, plus `reindex_jobs` (the
/// full-text rebuilds queued by a default-language change, one per branch;
/// empty otherwise) and `previous_default_language` (only when it changed).
///
/// # Changing the default language
/// - `default_language` is normalized as a BCP-47 tag, like repository create.
/// - Without `supported_languages`, the new default is added to the existing
///   list and the old default stays in it; with it, the new default is added
///   when missing.
/// - While translation overlays in the NEW default exist, the change is refused
///   with 409 (`DEFAULT_LANGUAGE_CONFLICT`, naming `language` and
///   `overlay_count`) and nothing is changed: those overlays would collide with
///   the base content, which is now in that language.
/// - On success a full-text rebuild of every branch is queued, so base content
///   moves from the old language's index to the new one's. Vector embeddings
///   are language-agnostic and are left alone.
/// - The change replicates as an ordinary repository update; peers rebuild
///   their own full-text indexes.
/// - Sending the current default is a no-op.
///
/// # Validation
/// - All locales in fallback chains must exist in supported_languages
/// - supported_languages must always include default_language
/// - No circular references in fallback chains
pub async fn update_translation_config(
    State(state): State<AppState>,
    Path(repo_id): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<UpdateTranslationConfigRequest>,
) -> Result<Response, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let storage = state.storage();
    let repo_mgmt = storage.repository_management();

    // Get existing repository
    let existing = repo_mgmt
        .get_repository(tenant_id, &repo_id)
        .await?
        .ok_or_else(|| ApiError::repository_not_found(&repo_id))?;

    let mut config = existing.config.clone();
    if let Some(langs) = req.supported_languages {
        config.supported_languages = langs;
    }
    if let Some(chains) = req.locale_fallback_chains {
        config.locale_fallback_chains = chains;
    }

    // Switching the default adds the new one to the supported languages; not
    // switching still makes sure the current one is in them.
    let previous_default_language = match req.default_language.as_deref() {
        Some(code) => config.set_default_language(&normalize_language(code)?),
        None => {
            let current = config.default_language.clone();
            config.set_default_language(&current)
        }
    };

    // Validate the configuration
    if let Err(validation_error) = config.validate_locale_fallback_chains() {
        return Err(ApiError::validation_failed(validation_error));
    }

    let reindex_jobs = match &previous_default_language {
        None => {
            repo_mgmt
                .update_repository_config(tenant_id, &repo_id, config.clone())
                .await?;
            Vec::new()
        }
        Some(previous) => {
            match change_default_language(&state, tenant_id, &repo_id, previous, &config).await? {
                Ok(jobs) => jobs,
                Err(conflict) => return Ok(conflict),
            }
        }
    };

    Ok(Json(UpdateTranslationConfigResponse {
        config,
        previous_default_language,
        reindex_jobs,
    })
    .into_response())
}

/// Store a config whose default language differs from the stored one: refuse
/// while overlays in the new default exist (the inner `Err` is the 409 to send),
/// then store it and queue a full-text rebuild of every branch.
#[cfg(feature = "storage-rocksdb")]
async fn change_default_language(
    state: &AppState,
    tenant_id: &str,
    repo_id: &str,
    previous: &str,
    config: &RepositoryConfig,
) -> Result<Result<Vec<ReindexJobResponse>, Response>, ApiError> {
    use raisin_rocksdb::management::default_language as dl;

    let storage = state.storage();
    let language = config.default_language.as_str();

    let overlays = dl::count_translation_overlays(storage, tenant_id, repo_id, language).await?;
    if overlays.total() > 0 {
        let body = serde_json::json!({
            "code": "DEFAULT_LANGUAGE_CONFLICT",
            "message": format!(
                "Cannot make '{language}' the default language of '{repo_id}': {} translation \
                 overlay(s) in '{language}' exist and would collide with the base content. \
                 Delete those translations first.",
                overlays.total()
            ),
            "language": language,
            "overlay_count": overlays.total(),
            "node_overlays": overlays.node_overlays,
            "block_overlays": overlays.block_overlays,
            "current_default_language": previous,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        return Ok(Err((StatusCode::CONFLICT, Json(body)).into_response()));
    }

    storage
        .repository_management()
        .update_repository_config(tenant_id, repo_id, config.clone())
        .await?;
    tracing::warn!(
        tenant_id,
        repo_id,
        previous,
        current = language,
        "Repository default language changed; queueing full-text rebuilds"
    );

    let jobs = dl::enqueue_fulltext_rebuild_all_branches(storage, tenant_id, repo_id)
        .await
        .map_err(|e| {
            ApiError::internal(format!(
                "The default language of '{repo_id}' is now '{language}', but queueing the \
                 full-text rebuild failed: {e}. Rebuild each branch with \
                 POST /api/admin/management/database/{tenant_id}/{repo_id}/fulltext/rebuild"
            ))
        })?;
    Ok(Ok(jobs
        .into_iter()
        .map(|j| ReindexJobResponse {
            branch: j.branch,
            job_id: j.job_id,
        })
        .collect()))
}

/// Without RocksDB there is no full-text index and no overlay store to check.
#[cfg(not(feature = "storage-rocksdb"))]
async fn change_default_language(
    _state: &AppState,
    _tenant_id: &str,
    _repo_id: &str,
    _previous: &str,
    _config: &RepositoryConfig,
) -> Result<Result<Vec<ReindexJobResponse>, Response>, ApiError> {
    Err(ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "NOT_IMPLEMENTED",
        "Changing the default language requires the RocksDB storage backend",
    ))
}

/// Validate and normalize a language code the way repository create does:
/// as a BCP-47 tag, language lowercased and region uppercased (`DE-ch` -> `de-CH`).
fn normalize_language(code: &str) -> Result<String, ApiError> {
    raisin_models::translations::LocaleCode::parse(code.trim())
        .map(|locale| locale.as_str().to_string())
        .map_err(|e| {
            let mut err =
                ApiError::validation_failed(format!("Invalid language code '{code}': {e}"));
            err.field = Some("default_language".to_string());
            err
        })
}

/// Response of `PATCH /api/repositories/{repo}/translation-config`
#[derive(Debug, serde::Serialize)]
pub struct UpdateTranslationConfigResponse {
    /// The repository configuration as stored after the update
    #[serde(flatten)]
    pub config: RepositoryConfig,
    /// The default language before this request, when it changed it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_default_language: Option<String>,
    /// Full-text rebuilds queued by a default-language change (one per branch)
    pub reindex_jobs: Vec<ReindexJobResponse>,
}

/// A full-text rebuild queued for one branch
#[derive(Debug, serde::Serialize)]
pub struct ReindexJobResponse {
    pub branch: String,
    pub job_id: String,
}

/// Get translation configuration for a repository
///
/// # Endpoint
/// GET /api/repositories/{repo_id}/translation-config
///
/// # Headers
/// X-Tenant-ID: {tenant_id}
///
/// # Response
/// ```json
/// {
///   "default_language": "en",
///   "supported_languages": ["en", "fr", "fr-CA", "de"],
///   "locale_fallback_chains": {
///     "fr-CA": ["fr", "en"]
///   }
/// }
/// ```
pub async fn get_translation_config(
    State(state): State<AppState>,
    Path(repo_id): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<TranslationConfigResponse>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let storage = state.storage();
    let repo_mgmt = storage.repository_management();

    let repo = repo_mgmt
        .get_repository(tenant_id, &repo_id)
        .await?
        .ok_or_else(|| ApiError::repository_not_found(&repo_id))?;

    Ok(Json(TranslationConfigResponse {
        default_language: repo.config.default_language,
        supported_languages: repo.config.supported_languages,
        locale_fallback_chains: repo.config.locale_fallback_chains,
    }))
}

/// Response for translation configuration
#[derive(Debug, serde::Serialize)]
pub struct TranslationConfigResponse {
    /// Default language (change it with PATCH translation-config)
    pub default_language: String,
    /// List of supported languages
    pub supported_languages: Vec<String>,
    /// Locale fallback chains
    pub locale_fallback_chains: std::collections::HashMap<String, Vec<String>>,
}

/// Delete a repository
///
/// # Endpoint
/// DELETE /api/repositories/{repo_id}
///
/// # Headers
/// X-Tenant-ID: {tenant_id}
///
/// # Warning
/// Irreversible. Removes ALL of the repository's data — branches, revisions,
/// nodes, translations, embeddings, every index, its jobs and its index
/// directories — so a repository recreated under the same id starts empty.
/// See `docs/API_REPOSITORIES.md`.
pub async fn delete_repository(
    State(state): State<AppState>,
    Path(repo_id): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let storage = state.storage();
    let repo_mgmt = storage.repository_management();

    let deleted = repo_mgmt.delete_repository(tenant_id, &repo_id).await?;

    if deleted {
        purge_repository_indexes(&state, tenant_id, &repo_id);
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::repository_not_found(&repo_id))
    }
}

/// Remove a deleted repository's full-text and vector indexes from disk.
///
/// They live outside RocksDB, so deleting the registry entry left them behind:
/// a repository recreated under the same id found the old repository's index
/// directories — including branches it does not have — and zero-byte vector
/// index files that failed every embedding job until they were removed by
/// hand. A failure here is logged, not returned: the repository IS deleted,
/// and a leftover directory is a disk-space matter the next rebuild replaces.
#[cfg(feature = "storage-rocksdb")]
fn purge_repository_indexes(state: &AppState, tenant_id: &str, repo_id: &str) {
    if let Some(engine) = state.indexing_engine.as_ref() {
        if let Err(e) = engine.purge_repository(tenant_id, repo_id) {
            tracing::warn!(tenant_id, repo_id, error = %e, "could not remove the deleted repository's full-text indexes");
        }
    }
    if let Some(engine) = state.hnsw_engine.as_ref() {
        if let Err(e) = engine.purge_repository(tenant_id, repo_id) {
            tracing::warn!(tenant_id, repo_id, error = %e, "could not remove the deleted repository's vector indexes");
        }
    }
}

#[cfg(not(feature = "storage-rocksdb"))]
fn purge_repository_indexes(_state: &AppState, _tenant_id: &str, _repo_id: &str) {}
