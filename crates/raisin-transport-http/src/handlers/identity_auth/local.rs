// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Local (email/password) authentication handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};

use crate::error::ApiError;
use crate::state::AppState;

#[cfg(feature = "storage-rocksdb")]
use raisin_models::auth::{Identity, LocalCredentials, Session};
#[cfg(feature = "storage-rocksdb")]
use raisin_models::timestamp::StorageTimestamp;
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::repositories::IdentityRepository;

use super::helpers::{
    build_auth_response, create_session, extract_repos, generate_tokens, validate_email,
    validate_password, AuthRepositories,
};
use super::types::{AuthTokensResponse, LocalLoginRequest, RegisterRequest};
use super::user_node::ensure_user_node;

// ============================================================================
// Core Registration Logic
// ============================================================================

/// Core registration logic - creates identity and session.
/// Shared by both `register()` and `register_for_repo()`.
#[cfg(feature = "storage-rocksdb")]
async fn create_identity_and_session(
    repos: &AuthRepositories,
    tenant_id: &str,
    req: &RegisterRequest,
) -> Result<(Identity, Session, StorageTimestamp), ApiError> {
    use uuid::Uuid;

    // Validate inputs
    validate_email(&req.email)?;
    validate_password(&req.password)?;

    // Check if email already exists
    if repos
        .identity
        .find_by_email(tenant_id, &req.email)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to check existing identity: {}", e),
            )
        })?
        .is_some()
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "EMAIL_EXISTS",
            "An account with this email already exists",
        ));
    }

    // Create new identity
    let identity_id = Uuid::new_v4().to_string();
    let mut identity = Identity::new(
        identity_id.clone(),
        tenant_id.to_string(),
        req.email.clone(),
    );
    identity.display_name = req.display_name.clone();

    // Hash password and set local credentials
    let password_hash = IdentityRepository::hash_password(&req.password).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "PASSWORD_HASH_ERROR",
            format!("Failed to hash password: {}", e),
        )
    })?;
    identity.local_credentials = Some(LocalCredentials::new(password_hash));

    // Save identity
    repos
        .identity
        .upsert(tenant_id, &identity, "system:registration")
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to create identity: {}", e),
            )
        })?;

    // Create session
    let (session, expires_at) = create_session(
        &repos.session,
        tenant_id,
        &identity_id,
        "local",
        false, // Default: no remember_me for registration
        "system:registration",
    )
    .await?;

    Ok((identity, session, expires_at))
}

// ============================================================================
// Core Login Logic
// ============================================================================

/// Core login logic - verifies credentials and creates session.
/// Shared by both `login()` and `login_for_repo()`.
#[cfg(feature = "storage-rocksdb")]
async fn verify_credentials_and_create_session(
    repos: &AuthRepositories,
    tenant_id: &str,
    email: &str,
    password: &str,
    remember_me: bool,
) -> Result<(Identity, Session, StorageTimestamp), ApiError> {
    use raisin_auth::strategies::LocalStrategy;

    // Find identity by email
    let identity = repos
        .identity
        .find_by_email(tenant_id, email)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                format!("Failed to query identity: {}", e),
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "INVALID_CREDENTIALS",
                "Invalid email or password",
            )
        })?;

    // Check if identity is active
    if !identity.is_active {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "ACCOUNT_DISABLED",
            "This account has been disabled",
        ));
    }

    // Verify local credentials exist
    let credentials = identity.local_credentials.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "INVALID_CREDENTIALS",
            "Invalid email or password",
        )
    })?;

    // Check if account is locked
    if credentials.is_locked() {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "ACCOUNT_LOCKED",
            "Account is temporarily locked due to too many failed login attempts",
        ));
    }

    // Verify password
    if !LocalStrategy::verify_password(password, &credentials.password_hash) {
        // Record failed login attempt
        let _ = repos
            .identity
            .record_failed_login(
                tenant_id,
                &identity.identity_id,
                5,  // lockout after 5 attempts
                15, // 15 minutes lockout
            )
            .await;

        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "INVALID_CREDENTIALS",
            "Invalid email or password",
        ));
    }

    // Record successful login
    repos
        .identity
        .record_successful_login(tenant_id, &identity.identity_id, "system:login")
        .await
        .map_err(|e| {
            tracing::warn!("Failed to record successful login: {}", e);
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                "Login succeeded but failed to update records",
            )
        })?;

    // Create session
    let (session, expires_at) = create_session(
        &repos.session,
        tenant_id,
        &identity.identity_id,
        "local",
        remember_me,
        "system:login",
    )
    .await?;

    Ok((identity, session, expires_at))
}

// ============================================================================
// Handlers
// ============================================================================

/// Register a new user identity (generic, no repo context).
///
/// # Endpoint
/// POST /auth/register
#[cfg(feature = "storage-rocksdb")]
pub async fn register(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthTokensResponse>, ApiError> {
    let repos = extract_repos(&state)?;
    let tenant_id = &tenant_info.tenant_id;

    let (identity, session, expires_at) =
        create_identity_and_session(&repos, tenant_id, &req).await?;

    // Generate tokens (no repo context, no home)
    let tokens = generate_tokens(&state, &identity, &session, None, None)?;

    Ok(Json(build_auth_response(
        &identity, tokens, expires_at, None,
    )))
}

/// Register a new user for a specific repository.
///
/// # Endpoint
/// POST /auth/{repo}/register
///
/// This creates a new identity and also creates a raisin:User node in the
/// repository's access_control workspace.
#[cfg(feature = "storage-rocksdb")]
pub async fn register_for_repo(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Path(repo): Path<String>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthTokensResponse>, ApiError> {
    let repos = extract_repos(&state)?;
    let tenant_id = &tenant_info.tenant_id;

    let (identity, session, expires_at) =
        create_identity_and_session(&repos, tenant_id, &req).await?;

    // Create user node inline in the repository's access_control workspace
    let home = match ensure_user_node(
        &repos.storage,
        tenant_id,
        &repo,
        &identity.identity_id,
        &req.email,
        req.display_name.as_deref(),
        &["viewer".to_string(), "authenticated_user".to_string()], // Default roles
    )
    .await
    {
        Ok(path) => {
            tracing::info!(
                identity_id = %identity.identity_id,
                home = %path,
                "User node created/found during registration"
            );
            Some(path)
        }
        Err(e) => {
            // Log but don't fail registration - user can still log in
            tracing::warn!(
                identity_id = %identity.identity_id,
                error = %e,
                "Failed to create user node during registration"
            );
            None
        }
    };

    // Generate tokens with repo context and home path
    let tokens = generate_tokens(&state, &identity, &session, Some(&repo), home.as_deref())?;

    Ok(Json(build_auth_response(
        &identity, tokens, expires_at, home,
    )))
}

/// Authenticate with email and password (generic, no repo context).
///
/// # Endpoint
/// POST /auth/login
#[cfg(feature = "storage-rocksdb")]
pub async fn login(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Json(req): Json<LocalLoginRequest>,
) -> Result<Json<AuthTokensResponse>, ApiError> {
    let repos = extract_repos(&state)?;
    let tenant_id = &tenant_info.tenant_id;

    let (identity, session, expires_at) = verify_credentials_and_create_session(
        &repos,
        tenant_id,
        &req.email,
        &req.password,
        req.remember_me,
    )
    .await?;

    // Generate tokens (no repo context, no home)
    let tokens = generate_tokens(&state, &identity, &session, None, None)?;

    Ok(Json(build_auth_response(
        &identity, tokens, expires_at, None,
    )))
}

/// Login to a specific repository.
///
/// # Endpoint
/// POST /auth/{repo}/login
///
/// This authenticates an existing user and ensures a raisin:User node exists
/// (just-in-time provisioning).
#[cfg(feature = "storage-rocksdb")]
pub async fn login_for_repo(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Path(repo): Path<String>,
    Json(req): Json<LocalLoginRequest>,
) -> Result<Json<AuthTokensResponse>, ApiError> {
    let repos = extract_repos(&state)?;
    let tenant_id = &tenant_info.tenant_id;

    let (identity, session, expires_at) = verify_credentials_and_create_session(
        &repos,
        tenant_id,
        &req.email,
        &req.password,
        req.remember_me,
    )
    .await?;

    // Just-in-time user provisioning: ensure user node exists in repository
    let home = match ensure_user_node(
        &repos.storage,
        tenant_id,
        &repo,
        &identity.identity_id,
        &identity.email,
        identity.display_name.as_deref(),
        &["viewer".to_string(), "authenticated_user".to_string()], // Default roles for JIT provisioned users
    )
    .await
    {
        Ok(path) => {
            tracing::info!(
                identity_id = %identity.identity_id,
                home = %path,
                repo = %repo,
                "User node found/created during login"
            );
            Some(path)
        }
        Err(e) => {
            // Log but don't fail login - user can still access
            tracing::warn!(
                identity_id = %identity.identity_id,
                repo = %repo,
                error = %e,
                "Failed to ensure user node during login"
            );
            None
        }
    };

    // Generate tokens with repo context and home path
    let tokens = generate_tokens(&state, &identity, &session, Some(&repo), home.as_deref())?;

    tracing::info!(
        identity_id = %identity.identity_id,
        repo = %repo,
        "User logged in via repo-scoped endpoint"
    );

    Ok(Json(build_auth_response(
        &identity, tokens, expires_at, home,
    )))
}
