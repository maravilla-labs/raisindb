// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tenant-identity operations for `RaisinFunctionApi` — and the policy gate.
//!
//! # The gate is here, and only here
//!
//! Both operations call [`RaisinFunctionApi::authorize_identities`] BEFORE
//! they touch a callback, mirroring `secrets.rs` and `email.rs`. The callbacks
//! themselves do no checking, so the one branch here is the whole gate.
//!
//! # What a function can never do: mark an address verified
//!
//! `email_verified` is not in the patch, and a caller that sends it is refused
//! rather than ignored. The flag means "the account holder has demonstrated
//! possession of this mailbox", and the ONLY thing that demonstrates it is the
//! magic-link verify step (a token that arrived in that mailbox came back).
//! A rename through this binding proves that somebody could type an address —
//! a buyer correcting a typo, an admin acting on a support ticket — and
//! carrying the old flag over to the new address would let an unverified
//! address inherit a verified one's trust. So an email change always goes out
//! with `email_verified: false`, and nothing here can flip it back.
//!
//! # Denials are loud
//!
//! A denied call is a `PermissionDenied` naming the block that would grant it
//! — never `None`, which would read exactly like "no such account".

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;
use crate::api::callbacks::IdentityPatch;

/// Trim and lowercase an address. Mail addresses are matched
/// case-insensitively everywhere in the identity store (the email index is
/// keyed lowercased), so the binding normalises once, at the edge, and every
/// layer below sees one spelling.
pub(crate) fn normalise_email(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Parse the JSON `patch` a function passed to `raisin.identities.update`.
///
/// Refuses — rather than ignores — keys the binding does not accept, so an
/// author who writes `email_verified: true` learns that it is not a thing this
/// API does instead of believing it silently worked.
pub(crate) fn parse_patch(patch: Value) -> Result<IdentityPatch> {
    let Value::Object(map) = patch else {
        return Err(raisin_error::Error::Validation(
            "[identities:invalid_patch] patch must be an object \
             { email?, password?, display_name? }"
                .to_string(),
        ));
    };

    let mut out = IdentityPatch::default();
    for (key, value) in map {
        match key.as_str() {
            "email" => {
                let email = normalise_email(expect_string(&key, &value)?);
                // The index is keyed by the part after '@'; an address without
                // one can never be found again, so refuse it here.
                if email.is_empty() || !email.contains('@') {
                    return Err(raisin_error::Error::Validation(format!(
                        "[identities:invalid_patch] email {email:?} is not an address"
                    )));
                }
                out.email = Some(email);
            }
            "password" => {
                let password = expect_string(&key, &value)?;
                if password.is_empty() {
                    return Err(raisin_error::Error::Validation(
                        "[identities:invalid_patch] password must not be empty".to_string(),
                    ));
                }
                out.password = Some(password.to_string());
            }
            "display_name" => {
                out.display_name = Some(expect_string(&key, &value)?.trim().to_string());
            }
            "email_verified" => {
                return Err(raisin_error::Error::Validation(
                    "[identities:invalid_patch] email_verified cannot be set by a function: \
                     a rename proves the address was typed, not that it is possessed. \
                     Only the magic-link verify step marks an address verified."
                        .to_string(),
                ));
            }
            other => {
                return Err(raisin_error::Error::Validation(format!(
                    "[identities:invalid_patch] unknown field {other:?}; \
                     accepted: email, password, display_name"
                )));
            }
        }
    }

    if out.is_empty() {
        return Err(raisin_error::Error::Validation(
            "[identities:invalid_patch] patch changes nothing".to_string(),
        ));
    }
    Ok(out)
}

fn expect_string<'a>(key: &str, value: &'a Value) -> Result<&'a str> {
    value.as_str().ok_or_else(|| {
        raisin_error::Error::Validation(format!(
            "[identities:invalid_patch] {key} must be a string"
        ))
    })
}

fn not_configured(op: &str) -> raisin_error::Error {
    raisin_error::Error::Validation(format!(
        "Identity store not available on this execution path (cannot {op})"
    ))
}

impl RaisinFunctionApi {
    /// The policy gate for `raisin.identities.*`.
    fn authorize_identities(&self, op: &str) -> Result<()> {
        if self.identity_policy.enabled {
            return Ok(());
        }
        tracing::warn!(op, "raisin.identities denied by identity policy");
        Err(raisin_error::Error::PermissionDenied(format!(
            "[identities:policy_denied] cannot {op}: this function has no identity_policy \
             (identity access is denied by default). Grant it by adding \
             `identity_policy: {{ enabled: true }}` to the function's .node.yaml."
        )))
    }

    /// Back `raisin.identities.findByEmail(email)`.
    pub(crate) async fn impl_identity_find_by_email(&self, email: &str) -> Result<Option<Value>> {
        self.authorize_identities("findByEmail")?;
        let email = normalise_email(email);
        if email.is_empty() {
            return Err(raisin_error::Error::Validation(
                "[identities:invalid_email] email must not be empty".to_string(),
            ));
        }
        let cb = self
            .callbacks
            .identity_find_by_email
            .as_ref()
            .ok_or_else(|| not_configured("findByEmail"))?;
        cb(email).await
    }

    /// Back `raisin.identities.update(id, patch)`.
    pub(crate) async fn impl_identity_update(&self, id: &str, patch: Value) -> Result<Value> {
        self.authorize_identities("update")?;
        let id = id.trim();
        if id.is_empty() {
            return Err(raisin_error::Error::Validation(
                "[identities:invalid_id] identity id must not be empty".to_string(),
            ));
        }
        let patch = parse_patch(patch)?;
        let cb = self
            .callbacks
            .identity_update
            .as_ref()
            .ok_or_else(|| not_configured("update"))?;
        cb(id.to_string(), patch).await
    }
}
