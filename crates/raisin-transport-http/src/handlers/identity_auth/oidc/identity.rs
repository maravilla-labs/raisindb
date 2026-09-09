// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Turning a verified OIDC assertion into a RaisinDB identity.
//!
//! This module is the persistence half. The rules deciding WHICH account an
//! assertion may reach, and the tests for them, live in
//! [`super::linking`]; keeping them apart is what lets those rules be tested
//! without a database.

use axum::http::StatusCode;
use raisin_auth::AuthenticationResult;
use raisin_models::auth::{Identity, LinkedProvider};
use raisin_models::timestamp::StorageTimestamp;

use crate::error::ApiError;

use super::super::helpers::AuthRepositories;
use super::linking::{decide, LinkDecision};

/// An identity resolved from a provider assertion.
pub struct ResolvedIdentity {
    pub identity: Identity,
    /// True when this login created the account. Used only for logging.
    pub created: bool,
}

/// Find, link, or create the identity behind a verified OIDC assertion.
pub async fn resolve_identity(
    repos: &AuthRepositories,
    tenant_id: &str,
    strategy_id: &str,
    result: &AuthenticationResult,
) -> Result<ResolvedIdentity, ApiError> {
    let external_id = result.external_id.as_deref().ok_or_else(|| {
        // Every OIDC provider issues `sub`; its absence means the token was not
        // what we think it was.
        unauthorized("the provider returned no subject claim, so the account cannot be resolved")
    })?;

    let email = result
        .email
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .ok_or_else(|| {
            unauthorized(
                "the provider returned no email address; RaisinDB identities are keyed by email",
            )
        })?
        .to_ascii_lowercase();

    let existing = repos
        .identity
        .find_by_email(tenant_id, &email)
        .await
        .map_err(|e| database("look up the identity", e))?;

    if let Some(identity) = &existing {
        if !identity.is_active {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "ACCOUNT_DISABLED",
                "This account has been disabled",
            ));
        }
    }

    // Cloned rather than borrowed from `existing`, which is moved into the
    // Link arm below. One small allocation per login buys a function that does
    // not depend on the borrow checker's view of a match scrutinee.
    let existing_subject: Option<String> = existing.as_ref().and_then(|i| {
        i.linked_providers
            .iter()
            .find(|p| p.strategy_id == strategy_id)
            .map(|p| p.external_id.clone())
    });

    let account_exists = existing.is_some();

    match decide(
        account_exists,
        existing_subject.as_deref(),
        external_id,
        result.email_verified,
    ) {
        LinkDecision::Refuse(message) => {
            tracing::warn!(
                strategy = %strategy_id,
                email_verified = result.email_verified,
                account_exists,
                "refused an OIDC login: {}",
                message
            );
            Err(unauthorized(message))
        }
        LinkDecision::Link => {
            let identity = existing.expect("Link is only returned when an account exists");
            let identity = write_identity(
                repos,
                tenant_id,
                identity,
                strategy_id,
                external_id,
                result,
                false,
            )
            .await?;
            Ok(ResolvedIdentity {
                identity,
                created: false,
            })
        }
        LinkDecision::Create => {
            let identity = Identity::new(
                uuid::Uuid::new_v4().to_string(),
                tenant_id.to_string(),
                email.clone(),
            );
            // No local_credentials: the account has no password and cannot be
            // reached by the local login path until someone sets one.
            let identity = write_identity(
                repos,
                tenant_id,
                identity,
                strategy_id,
                external_id,
                result,
                true,
            )
            .await?;
            Ok(ResolvedIdentity {
                identity,
                created: true,
            })
        }
    }
}

/// Attach the provider, copy the profile, and persist.
async fn write_identity(
    repos: &AuthRepositories,
    tenant_id: &str,
    mut identity: Identity,
    strategy_id: &str,
    external_id: &str,
    result: &AuthenticationResult,
    created: bool,
) -> Result<Identity, ApiError> {
    let mut link = LinkedProvider::new(strategy_id.to_string(), external_id.to_string());
    link.last_auth_at = Some(StorageTimestamp::now());
    link.claims = result.provider_claims.clone();
    identity.link_provider(link);

    // Only ever set, never cleared: a later assertion that omits the claim must
    // not undo an earlier verified one. Reaching here at all means the address
    // was verified, or that this provider was already linked.
    if result.email_verified {
        identity.email_verified = true;
    }

    // Fill blanks only. A display name someone set in RaisinDB should not be
    // overwritten by their corporate directory on every login.
    if identity.display_name.is_none() {
        identity.display_name = result.display_name.clone();
    }
    if identity.avatar_url.is_none() {
        identity.avatar_url = result.avatar_url.clone();
    }

    let actor = if created {
        "system:oidc_registration"
    } else {
        "system:oidc_login"
    };
    repos
        .identity
        .upsert(tenant_id, &identity, actor)
        .await
        .map_err(|e| database("save the identity", e))?;

    if !created {
        repos
            .identity
            .record_successful_login(tenant_id, &identity.identity_id, actor)
            .await
            .map_err(|e| database("record the login", e))?;
    }

    Ok(identity)
}

fn unauthorized(message: &str) -> ApiError {
    ApiError::new(StatusCode::UNAUTHORIZED, "OIDC_LOGIN_REFUSED", message)
}

fn database(what: &str, e: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %e, "OIDC login could not {}", what);
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "DATABASE_ERROR",
        format!("Could not {what}"),
    )
}
