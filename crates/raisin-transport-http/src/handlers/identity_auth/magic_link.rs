// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Magic link (passwordless) authentication handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};

use crate::error::ApiError;
use crate::state::AppState;

use super::helpers::mask_email;
use super::types::{
    AuthTokensResponse, MagicLinkRequest, MagicLinkSentResponse, MagicLinkVerifyQuery,
};

/// Request a magic link for passwordless authentication.
///
/// # Endpoint
/// POST /auth/magic-link
#[cfg(feature = "storage-rocksdb")]
pub async fn request_magic_link(
    State(_state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Json(req): Json<MagicLinkRequest>,
) -> Result<Json<MagicLinkSentResponse>, ApiError> {
    // TODO: Implement via IdentityAuthService + MagicLinkStrategy
    // 1. Find or create identity by email
    // 2. Generate magic link token
    // 3. Queue email job via JobRegistry
    // 4. Return masked confirmation

    let masked_email = mask_email(&req.email);

    Err(ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "NOT_IMPLEMENTED",
        "Magic link authentication not yet implemented",
    ))
}

/// Verify a magic link token.
///
/// # Endpoint
/// GET /auth/magic-link/verify?token={token}
///
/// This endpoint verifies the magic link token and returns auth tokens.
/// If a redirect_url was specified when requesting the magic link, the
/// browser will be redirected there with the token as a query parameter.
#[cfg(feature = "storage-rocksdb")]
pub async fn verify_magic_link(
    State(_state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Query(query): Query<MagicLinkVerifyQuery>,
) -> Result<Json<AuthTokensResponse>, ApiError> {
    // TODO: Implement via IdentityAuthService + MagicLinkStrategy
    // 1. Verify token (check hash, expiration, single-use)
    // 2. Get identity from token
    // 3. Mark email as verified
    // 4. Create session
    // 5. Generate tokens
    // 6. Delete/mark token as used

    Err(ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "NOT_IMPLEMENTED",
        "Magic link verification not yet implemented",
    ))
}

/// Request a magic link for a specific repository.
///
/// # Endpoint
/// POST /auth/{repo}/magic-link
#[cfg(feature = "storage-rocksdb")]
pub async fn request_magic_link_for_repo(
    State(_state): State<AppState>,
    Extension(_tenant_info): Extension<crate::middleware::TenantInfo>,
    Path(_repo): Path<String>,
    Json(req): Json<MagicLinkRequest>,
) -> Result<Json<MagicLinkSentResponse>, ApiError> {
    // TODO: Implement - similar to request_magic_link but with repo context
    let _masked_email = mask_email(&req.email);

    Err(ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "NOT_IMPLEMENTED",
        "Magic link authentication not yet implemented",
    ))
}
