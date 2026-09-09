// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The OIDC provider section of the tenant auth config.
//!
//! Providers live in `TenantAuthConfig::providers` next to `local` and
//! `magic_link`, distinguished by a `strategy_id` of `oidc:{slug}`. This module
//! converts between that storage shape and the API shape, and is the ONE place
//! that seals a client secret: the plaintext arrives in a `PUT`, is sealed
//! under the deployment master key bound to the tenant, and only the sealed
//! bytes are stored. Reads report `has_client_secret` and nothing more.

use axum::http::StatusCode;
use raisin_crypto::{SecretBox, SecretContext};
use raisin_models::auth::{AuthProviderConfig, TenantAuthConfig};

use crate::error::ApiError;

use super::config_types::{OidcProviderInput, OidcProviderView};

/// Where a browser starts a login with a provider.
pub fn authorize_url(provider_id: &str) -> String {
    format!("/auth/oidc/{provider_id}")
}

/// Every OIDC provider in a config, in priority order, secrets omitted.
pub fn oidc_provider_views(config: &TenantAuthConfig) -> Vec<OidcProviderView> {
    let mut providers: Vec<&AuthProviderConfig> =
        config.providers.iter().filter(|p| p.is_oidc()).collect();
    providers.sort_by_key(|p| (p.priority, p.provider_id.clone()));
    providers.into_iter().map(view).collect()
}

fn view(p: &AuthProviderConfig) -> OidcProviderView {
    OidcProviderView {
        provider_id: p.provider_id.clone(),
        display_name: p.display_name.clone(),
        icon: p.icon.clone(),
        enabled: p.enabled,
        priority: p.priority,
        issuer_url: p.issuer_url.clone(),
        client_id: p.client_id.clone(),
        has_client_secret: p
            .client_secret_encrypted
            .as_ref()
            .is_some_and(|b| !b.is_empty()),
        redirect_uri: p.redirect_uri.clone(),
        scopes: p.scopes.clone(),
        attribute_mapping: p.attribute_mapping.clone(),
        groups_claim: p.groups_claim.clone(),
        allowed_email_domains: p.allowed_email_domains.clone(),
        authorization_url: p.authorization_url.clone(),
        token_url: p.token_url.clone(),
        userinfo_url: p.userinfo_url.clone(),
        jwks_url: p.jwks_url.clone(),
        authorize_url: authorize_url(&p.provider_id),
    }
}

/// Seal a client secret for storage in `client_secret_encrypted`.
pub fn seal_client_secret(
    master_key: &[u8; 32],
    tenant_id: &str,
    plaintext: &str,
) -> Result<Vec<u8>, ApiError> {
    let ctx = SecretContext::oidc_client_secret(tenant_id)
        .map_err(|e| ApiError::internal(format!("secret context: {e}")))?;
    SecretBox::new(master_key)
        .seal(&ctx, plaintext.as_bytes())
        .map_err(|e| ApiError::internal(format!("could not seal client secret: {e}")))
}

/// Open a stored client secret. The inverse of [`seal_client_secret`]; the
/// login handlers call this once per login.
pub fn open_client_secret(
    master_key: &[u8; 32],
    tenant_id: &str,
    sealed: &[u8],
) -> Result<String, ApiError> {
    let ctx = SecretContext::oidc_client_secret(tenant_id)
        .map_err(|e| ApiError::internal(format!("secret context: {e}")))?;
    let bytes = SecretBox::new(master_key)
        .open(&ctx, sealed)
        .map_err(|e| ApiError::internal(format!("could not open client secret: {e}")))?;
    String::from_utf8(bytes)
        .map_err(|_| ApiError::internal("stored client secret is not UTF-8".to_string()))
}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "INVALID_OIDC_PROVIDER", msg)
}

/// A provider slug is a URL path segment and a `strategy_id` suffix, so it is
/// restricted to lower-case letters, digits, `-` and `_`.
fn validate_provider_id(id: &str) -> Result<(), ApiError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && id.starts_with(|c: char| c.is_ascii_alphanumeric());
    if !ok {
        return Err(bad(format!(
            "provider_id '{id}' must be 1-64 lower-case letters, digits, '-' or '_'"
        )));
    }
    if id == "local" || id == "magic_link" {
        return Err(bad(format!("provider_id '{id}' is reserved")));
    }
    Ok(())
}

fn validate_http_url(field: &str, value: &str) -> Result<(), ApiError> {
    match url::Url::parse(value) {
        Ok(u) if u.scheme() == "http" || u.scheme() == "https" => Ok(()),
        _ => Err(bad(format!("{field} must be an absolute http(s) URL"))),
    }
}

/// Replace the OIDC providers of `config` with `inputs`.
///
/// `seal` turns a plaintext secret into stored bytes; it is a parameter so the
/// merge logic is testable without a key. Existing sealed secrets survive an
/// entry that omits `client_secret`.
pub fn apply_oidc_providers(
    config: &mut TenantAuthConfig,
    inputs: &[OidcProviderInput],
    seal: impl Fn(&str) -> Result<Vec<u8>, ApiError>,
) -> Result<(), ApiError> {
    let mut seen = std::collections::HashSet::new();
    let mut next: Vec<AuthProviderConfig> = Vec::with_capacity(inputs.len());

    for input in inputs {
        validate_provider_id(&input.provider_id)?;
        if !seen.insert(input.provider_id.clone()) {
            return Err(bad(format!(
                "provider_id '{}' appears twice",
                input.provider_id
            )));
        }

        let existing = config
            .providers
            .iter()
            .find(|p| p.is_oidc() && p.provider_id == input.provider_id)
            .cloned();
        let mut p = existing.clone().unwrap_or_else(|| {
            AuthProviderConfig::oidc(
                input.provider_id.clone(),
                input
                    .display_name
                    .clone()
                    .unwrap_or_else(|| input.provider_id.clone()),
            )
        });

        if let Some(v) = &input.display_name {
            p.display_name = v.clone();
        }
        if let Some(v) = &input.icon {
            p.icon = v.clone();
        }
        if let Some(v) = input.enabled {
            p.enabled = v;
        }
        if let Some(v) = input.priority {
            p.priority = v;
        }
        if let Some(v) = &input.client_id {
            p.client_id = Some(v.clone());
        }
        if let Some(v) = &input.issuer_url {
            validate_http_url("issuer_url", v)?;
            p.issuer_url = Some(v.trim_end_matches('/').to_string());
        }
        if let Some(v) = &input.redirect_uri {
            validate_http_url("redirect_uri", v)?;
            p.redirect_uri = Some(v.clone());
        }
        for (name, from, to) in [
            (
                "authorization_url",
                &input.authorization_url,
                &mut p.authorization_url,
            ),
            ("token_url", &input.token_url, &mut p.token_url),
            ("userinfo_url", &input.userinfo_url, &mut p.userinfo_url),
            ("jwks_url", &input.jwks_url, &mut p.jwks_url),
        ] {
            if let Some(v) = from {
                validate_http_url(name, v)?;
                *to = Some(v.clone());
            }
        }
        if let Some(v) = &input.scopes {
            p.scopes = v.clone();
        }
        if let Some(v) = &input.attribute_mapping {
            p.attribute_mapping = v.clone();
        }
        if let Some(v) = &input.groups_claim {
            p.groups_claim = Some(v.clone()).filter(|s| !s.is_empty());
        }
        if let Some(v) = &input.allowed_email_domains {
            p.allowed_email_domains = v
                .iter()
                .map(|d| d.trim().trim_start_matches('@').to_ascii_lowercase())
                .filter(|d| !d.is_empty())
                .collect();
        }
        if let Some(secret) = &input.client_secret {
            p.client_secret_encrypted = if secret.is_empty() {
                None
            } else {
                Some(seal(secret)?)
            };
        }

        if p.client_id.as_deref().unwrap_or("").is_empty() {
            return Err(bad(format!(
                "provider '{}' needs a client_id",
                input.provider_id
            )));
        }
        if p.issuer_url.is_none()
            && (p.authorization_url.is_none() || p.token_url.is_none() || p.jwks_url.is_none())
        {
            return Err(bad(format!(
                "provider '{}' needs an issuer_url for discovery, or authorization_url, \
                 token_url and jwks_url",
                input.provider_id
            )));
        }
        if p.scopes.is_empty() {
            p.scopes = vec!["openid".into(), "email".into(), "profile".into()];
        }
        next.push(p);
    }

    config.providers.retain(|p| !p.is_oidc());
    config.providers.extend(next);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_seal(s: &str) -> Result<Vec<u8>, ApiError> {
        Ok(format!("sealed:{s}").into_bytes())
    }

    fn input(id: &str) -> OidcProviderInput {
        OidcProviderInput {
            provider_id: id.to_string(),
            client_id: Some("cid".to_string()),
            issuer_url: Some("https://idp.example.com/".to_string()),
            client_secret: Some("shh".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn a_provider_is_stored_with_a_sealed_secret_and_defaults() {
        let mut cfg = TenantAuthConfig::new("t".into());
        apply_oidc_providers(&mut cfg, &[input("keycloak")], fake_seal).unwrap();
        let p = cfg.get_provider("oidc:keycloak").unwrap();
        assert_eq!(p.provider_id, "keycloak");
        assert_eq!(
            p.client_secret_encrypted.as_deref(),
            Some(b"sealed:shh".as_slice())
        );
        assert_eq!(p.issuer_url.as_deref(), Some("https://idp.example.com"));
        assert_eq!(p.scopes, vec!["openid", "email", "profile"]);

        let views = oidc_provider_views(&cfg);
        assert_eq!(views.len(), 1);
        assert!(views[0].has_client_secret);
        assert_eq!(views[0].authorize_url, "/auth/oidc/keycloak");
    }

    /// The reason the secret is optional on update: editing scopes must not
    /// require re-entering the secret, and must not silently drop it either.
    #[test]
    fn an_update_without_a_secret_keeps_the_stored_one() {
        let mut cfg = TenantAuthConfig::new("t".into());
        apply_oidc_providers(&mut cfg, &[input("g")], fake_seal).unwrap();
        let mut again = input("g");
        again.client_secret = None;
        again.scopes = Some(vec!["openid".into()]);
        apply_oidc_providers(&mut cfg, &[again], fake_seal).unwrap();
        let p = cfg.get_provider("oidc:g").unwrap();
        assert_eq!(
            p.client_secret_encrypted.as_deref(),
            Some(b"sealed:shh".as_slice())
        );
        assert_eq!(p.scopes, vec!["openid"]);
    }

    /// The list is a full replacement, but only of the OIDC entries: local and
    /// magic-link providers in the same vector are untouched.
    #[test]
    fn the_list_replaces_oidc_entries_only() {
        let mut cfg = TenantAuthConfig::new("t".into());
        cfg.providers.push(AuthProviderConfig::local());
        apply_oidc_providers(&mut cfg, &[input("a"), input("b")], fake_seal).unwrap();
        apply_oidc_providers(&mut cfg, &[input("b")], fake_seal).unwrap();
        assert!(cfg.get_provider("oidc:a").is_none());
        assert!(cfg.get_provider("oidc:b").is_some());
        assert!(cfg.local_auth_enabled());
    }

    #[test]
    fn a_bad_slug_a_duplicate_and_a_missing_client_id_are_refused() {
        let mut cfg = TenantAuthConfig::new("t".into());
        assert!(apply_oidc_providers(&mut cfg, &[input("Bad Slug")], fake_seal).is_err());
        assert!(apply_oidc_providers(&mut cfg, &[input("local")], fake_seal).is_err());
        assert!(apply_oidc_providers(&mut cfg, &[input("x"), input("x")], fake_seal).is_err());
        let mut no_client = input("x");
        no_client.client_id = None;
        assert!(apply_oidc_providers(&mut cfg, &[no_client], fake_seal).is_err());
        let mut no_endpoints = input("x");
        no_endpoints.issuer_url = None;
        assert!(apply_oidc_providers(&mut cfg, &[no_endpoints], fake_seal).is_err());
        assert!(
            cfg.providers.is_empty(),
            "a refused PUT must not half-apply"
        );
    }

    #[test]
    fn email_domains_are_normalised() {
        let mut cfg = TenantAuthConfig::new("t".into());
        let mut i = input("g");
        i.allowed_email_domains = Some(vec![" @Example.COM ".into(), "".into()]);
        apply_oidc_providers(&mut cfg, &[i], fake_seal).unwrap();
        assert_eq!(
            cfg.get_provider("oidc:g").unwrap().allowed_email_domains,
            vec!["example.com"]
        );
    }

    #[test]
    fn seal_and_open_round_trip_under_the_tenant_binding() {
        // `oidc_client_secret` is `V1Policy::Reject`, which only makes the
        // tenant binding load-bearing against a v2 blob — `seal()` emits v1
        // (no AAD at all) unless `RAISIN_CRYPTO_EMIT_V2` is set, so this test
        // has to turn the gate on itself rather than rely on ambient process
        // state. No cross-crate `ENV_LOCK` is available here (raisin-crypto's
        // is private), but no other test in this crate touches this var.
        std::env::set_var("RAISIN_CRYPTO_EMIT_V2", "1");

        let key = [7u8; 32];
        let sealed = seal_client_secret(&key, "acme", "top-secret").unwrap();
        assert_ne!(sealed, b"top-secret");
        assert_eq!(
            open_client_secret(&key, "acme", &sealed).unwrap(),
            "top-secret"
        );
        assert!(open_client_secret(&key, "othercorp", &sealed).is_err());

        std::env::remove_var("RAISIN_CRYPTO_EMIT_V2");
    }
}
