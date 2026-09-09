// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! The authorization-code flow, both halves.
//!
//! [`OidcStrategy::begin_login`] mints the PKCE verifier and nonce, seals them
//! into the `state` parameter, and returns the URL to redirect the browser to.
//! [`OidcStrategy::complete_login`] takes that state back, exchanges the code
//! and returns a verified identity. Nothing is stored between the two.

use raisin_error::{Error, Result};
use std::collections::HashMap;

use crate::strategy::AuthenticationResult;

use super::login_state::{OidcLoginState, LOGIN_STATE_TTL_SECONDS};
use super::OidcStrategy;

/// What a caller needs to send a browser to the provider.
#[derive(Debug, Clone)]
pub struct LoginRedirect {
    /// The provider's authorization URL, fully parameterised.
    pub authorization_url: String,
    /// The sealed state embedded in that URL, returned so a caller can log or
    /// set it in a cookie as an extra CSRF binding.
    pub state: String,
}

impl OidcStrategy {
    /// Start a login.
    ///
    /// `redirect_uri` is where the *application* wants the browser after login
    /// succeeds, which is not the provider's callback: the callback URL is
    /// fixed configuration, registered with the provider, while this varies per
    /// login. Passing `None` makes the callback answer with JSON instead.
    pub fn begin_login(
        &self,
        tenant_id: &str,
        master_key: &[u8; 32],
        redirect_uri: Option<String>,
        repo: Option<String>,
    ) -> Result<LoginRedirect> {
        let config = self.get_config()?;
        if config.redirect_uri.is_empty() {
            return Err(Error::Validation(format!(
                "OIDC provider '{}' has no redirect_uri configured; it must match the callback \
                 URL registered with the provider exactly",
                self.strategy_id
            )));
        }

        let code_verifier = Self::generate_code_verifier();
        let code_challenge = Self::generate_code_challenge(&code_verifier);
        let nonce = Self::generate_code_verifier();

        let state = OidcLoginState {
            provider: self.provider_name().to_string(),
            tenant_id: tenant_id.to_string(),
            code_verifier,
            nonce: nonce.clone(),
            redirect_uri,
            repo,
            expires_at: chrono::Utc::now().timestamp() + LOGIN_STATE_TTL_SECONDS,
        };
        let sealed = state.seal(master_key)?;

        let authorization_url = self.build_authorization_url(
            &config.redirect_uri.clone(),
            &sealed,
            &code_challenge,
            Some(&nonce),
        )?;

        Ok(LoginRedirect {
            authorization_url,
            state: sealed,
        })
    }

    /// Finish a login: redeem the code, verify the `id_token`, map the claims.
    ///
    /// The returned [`AuthenticationResult`] carries the subject and email the
    /// signed token asserted. A caller resolves those to an account; this
    /// function does no account lookup of its own.
    pub async fn complete_login(
        &self,
        state: &OidcLoginState,
        code: &str,
    ) -> Result<AuthenticationResult> {
        let tokens = self
            .exchange_code_for_tokens(code, &state.code_verifier)
            .await?;

        let id_token = tokens.id_token.as_deref().ok_or_else(|| {
            // Without an id_token there is nothing signed to trust. An OAuth2
            // provider that returns only an access token is not an OIDC
            // provider, and treating its userinfo response as proof of identity
            // would accept any access token the endpoint happens to honour.
            Error::Unauthorized(
                "the provider returned no id_token, so the login cannot be verified".to_string(),
            )
        })?;

        let mut claims = self.verify_id_token(id_token, &state.nonce).await?;

        // Some providers keep `name`, `picture` and group claims out of the
        // token and only serve them from userinfo. Fetch them when we can, but
        // treat them as decoration: an id_token claim always wins, so nothing
        // the signature covers can be overwritten by an unsigned response.
        if let Some(access_token) = tokens.access_token.as_deref() {
            if self.get_config()?.userinfo_endpoint.is_some() {
                match self.fetch_user_info(access_token).await {
                    Ok(extra) => merge_userinfo(&mut claims, extra)?,
                    Err(e) => {
                        tracing::debug!(
                            provider = %self.strategy_id,
                            error = %e,
                            "userinfo fetch failed; continuing with id_token claims only"
                        );
                    }
                }
            }
        }

        let result = self.map_user_info(claims)?;
        self.check_email_domain(result.email.as_deref())?;
        Ok(result)
    }
}

/// Fold a userinfo response into the verified claim set.
///
/// Two rules. The subject must agree, per OpenID Connect Core §5.3.2 — a
/// userinfo document for a different subject means the access token and the
/// id_token describe different people, which is a mix-up, not a profile. And
/// only claims absent from the token are taken, so the signed set is never
/// overwritten by an unsigned one.
fn merge_userinfo(
    claims: &mut HashMap<String, serde_json::Value>,
    extra: HashMap<String, serde_json::Value>,
) -> Result<()> {
    let token_sub = claims.get("sub").and_then(|v| v.as_str());
    let info_sub = extra.get("sub").and_then(|v| v.as_str());
    if let (Some(a), Some(b)) = (token_sub, info_sub) {
        if a != b {
            return Err(Error::Unauthorized(
                "the userinfo response describes a different subject than the id_token".to_string(),
            ));
        }
    }

    for (k, v) in extra {
        claims.entry(k).or_insert(v);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claims(sub: &str) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert("sub".to_string(), json!(sub));
        m
    }

    #[test]
    fn userinfo_fills_gaps_but_never_overwrites_the_signed_set() {
        let mut signed = claims("user-1");
        signed.insert("email".to_string(), json!("real@example.com"));

        let mut extra = claims("user-1");
        // A hostile or merely stale userinfo response tries to change the email.
        extra.insert("email".to_string(), json!("attacker@example.com"));
        extra.insert("name".to_string(), json!("Real Name"));

        merge_userinfo(&mut signed, extra).unwrap();

        assert_eq!(signed["email"], json!("real@example.com"));
        assert_eq!(signed["name"], json!("Real Name"));
    }

    #[test]
    fn a_userinfo_response_for_another_subject_is_refused() {
        let mut signed = claims("user-1");
        let err = merge_userinfo(&mut signed, claims("user-2"));
        assert!(err.is_err());
    }

    /// A provider that omits `sub` from userinfo is tolerated: the id_token
    /// already carries the authoritative one.
    #[test]
    fn a_userinfo_response_without_a_subject_is_accepted() {
        let mut signed = claims("user-1");
        let mut extra = HashMap::new();
        extra.insert("name".to_string(), json!("N"));
        merge_userinfo(&mut signed, extra).unwrap();
        assert_eq!(signed["sub"], json!("user-1"));
    }
}
