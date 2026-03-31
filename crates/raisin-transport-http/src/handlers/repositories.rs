//! Repository management HTTP handlers
//!
//! These endpoints manage repositories within a tenant's context.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use raisin_context::{RepositoryConfig, RepositoryInfo};
use raisin_storage::{
    BranchRepository, RegistryRepository, RepositoryManagementRepository, Storage,
};
use serde::Deserialize;

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

    /// Default language (IMMUTABLE after creation, defaults to "en")
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

    /// Supported languages for translations (default_language is IMMUTABLE and cannot be changed)
    #[serde(default)]
    pub supported_languages: Option<Vec<String>>,
}

/// Request to update translation configuration
#[derive(Debug, Deserialize)]
pub struct UpdateTranslationConfigRequest {
    /// Supported languages for translations
    /// Must always include the default_language (which is immutable)
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
    // TODO: Extract tenant_id from headers when multi-tenant auth is implemented
) -> Result<Json<Vec<RepositoryInfo>>, ApiError> {
    let tenant_id = "default"; // Single-tenant mode
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
) -> Result<Json<RepositoryInfo>, ApiError> {
    let tenant_id = "default";
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
    Json(req): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<RepositoryInfo>), ApiError> {
    let tenant_id = "default";
    let storage = state.storage();
    let repo_mgmt = storage.repository_management();

    // Ensure tenant is registered (will emit TenantCreated event if new)
    // This triggers admin user initialization for new tenants
    let registry = storage.registry();
    registry
        .register_tenant(tenant_id, std::collections::HashMap::new())
        .await?;

    // Check if repository already exists
    if repo_mgmt.repository_exists(tenant_id, &req.repo_id).await? {
        return Err(ApiError::repository_already_exists(&req.repo_id));
    }

    // Determine default language (IMMUTABLE after creation)
    let default_language = req.default_language.unwrap_or_else(|| "en".to_string());

    // Ensure supported languages includes default language
    let mut supported_languages = req
        .supported_languages
        .unwrap_or_else(|| vec![default_language.clone()]);
    if !supported_languages.contains(&default_language) {
        supported_languages.push(default_language.clone());
    }

    let config = RepositoryConfig {
        default_branch: req.default_branch.unwrap_or_else(|| "main".to_string()),
        description: req.description,
        tags: std::collections::HashMap::new(),
        default_language,
        supported_languages,
        locale_fallback_chains: std::collections::HashMap::new(),
    };

    let default_branch = config.default_branch.clone();

    // Create the repository
    let repo = repo_mgmt
        .create_repository(tenant_id, &req.repo_id, config)
        .await?;

    // Create the default branch
    let branches = storage.branches();
    if let Err(e) = branches
        .create_branch(
            tenant_id,
            &req.repo_id,
            &default_branch,
            "system", // created_by
            None,     // from_revision - start from scratch
            None,     // upstream_branch - main has no upstream
            false,    // protected
            false,    // include_revision_history - not applicable for new repo
        )
        .await
    {
        tracing::warn!(
            "Failed to create default branch '{}' for repository '{}': {}",
            default_branch,
            req.repo_id,
            e
        );
        // Don't fail the repository creation if branch creation fails
        // The repository is still valid, just needs manual branch creation
    } else {
        tracing::info!(
            "Created default branch '{}' for repository '{}'",
            default_branch,
            req.repo_id
        );
    }

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
    Json(req): Json<UpdateRepositoryRequest>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default";
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
        default_language: existing.config.default_language, // IMMUTABLE - always preserve
        supported_languages,
        locale_fallback_chains: existing.config.locale_fallback_chains, // Preserve existing fallback chains
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
/// ```json
/// {
///   "supported_languages": ["en", "fr", "fr-CA", "de", "de-CH"],
///   "locale_fallback_chains": {
///     "fr-CA": ["fr", "en"],
///     "de-CH": ["de", "en"]
///   }
/// }
/// ```
///
/// # Validation
/// - All locales in fallback chains must exist in supported_languages
/// - default_language is immutable and cannot be changed
/// - supported_languages must always include default_language
/// - No circular references in fallback chains
pub async fn update_translation_config(
    State(state): State<AppState>,
    Path(repo_id): Path<String>,
    Json(req): Json<UpdateTranslationConfigRequest>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default";
    let storage = state.storage();
    let repo_mgmt = storage.repository_management();

    // Get existing repository
    let existing = repo_mgmt
        .get_repository(tenant_id, &repo_id)
        .await?
        .ok_or_else(|| ApiError::repository_not_found(&repo_id))?;

    // Update supported_languages if provided
    let supported_languages = if let Some(mut langs) = req.supported_languages {
        // Ensure default_language is always included
        if !langs.contains(&existing.config.default_language) {
            langs.push(existing.config.default_language.clone());
        }
        langs
    } else {
        existing.config.supported_languages
    };

    // Update locale_fallback_chains if provided
    let locale_fallback_chains = req
        .locale_fallback_chains
        .unwrap_or(existing.config.locale_fallback_chains);

    // Build new config
    let config = RepositoryConfig {
        default_branch: existing.config.default_branch,
        description: existing.config.description,
        tags: existing.config.tags,
        default_language: existing.config.default_language, // IMMUTABLE
        supported_languages,
        locale_fallback_chains,
    };

    // Validate the configuration
    if let Err(validation_error) = config.validate_locale_fallback_chains() {
        return Err(ApiError::validation_failed(validation_error));
    }

    // Apply the update
    repo_mgmt
        .update_repository_config(tenant_id, &repo_id, config)
        .await?;

    Ok(StatusCode::NO_CONTENT)
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
) -> Result<Json<TranslationConfigResponse>, ApiError> {
    let tenant_id = "default";
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
    /// Default language (immutable)
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
/// This will delete all branches, tags, revisions, and nodes in the repository.
/// This operation cannot be undone.
pub async fn delete_repository(
    State(state): State<AppState>,
    Path(repo_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default";
    let storage = state.storage();
    let repo_mgmt = storage.repository_management();

    let deleted = repo_mgmt.delete_repository(tenant_id, &repo_id).await?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::repository_not_found(&repo_id))
    }
}
