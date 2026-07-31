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

//! Assembling the `credential` object handed to an adapter.

use serde_json::{Map, Value};

use super::account::ConnectedAccount;

/// Build the adapter `credential` object for one connection.
///
/// # The refresh-token guarantee
///
/// **Nothing from the OAuth token blob reaches the adapter except
/// `access_token`.** That is enforced *structurally*: the blob is read through
/// a one-key allowlist rather than cloned-then-stripped. A blocklist would leak
/// the moment a provider added a field, and "strip after clone" leaks whenever
/// someone forgets a path. This matters more than usual here because adapters
/// run privileged with RLS bypassed and hold `raisin.http`, so a leaked refresh
/// token is exfiltratable. Refresh tokens are decrypted *only* by the Rust
/// token-refresh job.
///
/// The `secrets` map is merged wholesale, because a per-connection secret only
/// exists because the adapter needs it to authenticate. An operator who names
/// one of their own secret fields `refresh_token` therefore does get it — that
/// is their field, not the provider's, and it is covered by a test so the
/// distinction stays deliberate rather than accidental.
///
/// # Shape
///
/// ```jsonc
/// { "account_id": "…", "provider_type": "imap",  // engine-owned, always win
///   "username": "ops@example.com",  // account.config.username ?? subject
///   "access_token": "…",            // present only for OAuth connections
///   "password": "…" }               // from credential-flagged secrets
/// ```
///
/// Assembly order — later entries overwrite earlier ones, except that
/// engine-owned keys are written last and unconditionally:
///
/// 1. credential-flagged fields from the connection's `config`
/// 2. the connection's decrypted secrets (all of them: a secret is only stored
///    because something needs it to authenticate)
/// 3. `access_token` from the decrypted token blob, when present
/// 4. `username`, defaulted from the verified `subject`
/// 5. engine-owned keys
///
/// `tokens` and `secrets` are plaintext maps the caller has already decrypted —
/// this module stays free of `raisin-crypto`. Either may be absent: an OAuth
/// connection has tokens and no secrets, an IMAP app-password connection has
/// secrets and **no tokens**. That second case is why the old code was broken —
/// it bailed out whenever a token blob was missing.
pub fn build_credential(
    provider_type: &str,
    account: &ConnectedAccount,
    tokens: Option<&Value>,
    secrets: Option<&Value>,
    credential_fields: &[String],
) -> Value {
    let mut cred = Map::new();

    // 1. Credential-flagged non-secret config (an IMAP username).
    if let Value::Object(cfg) = account.config_or_empty() {
        for field in credential_fields {
            if let Some(v) = cfg.get(field) {
                cred.insert(field.clone(), v.clone());
            }
        }
    }

    // 2. Decrypted per-connection secrets (the app password).
    if let Some(Value::Object(map)) = secrets {
        for (k, v) in map {
            cred.insert(k.clone(), v.clone());
        }
    }

    // 3. OAuth access token — allowlisted, so no sibling of it can ride along.
    if let Some(tok) = tokens
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
    {
        cred.insert("access_token".into(), Value::String(tok.to_string()));
    }

    // 4. Login identity. An explicit per-connection username wins; otherwise the
    //    provider-verified subject, which is what makes IMAP XOAUTH2 work
    //    without the operator retyping their address.
    if !cred.contains_key("username") {
        if let Some(subject) = account.subject.as_deref().filter(|s| !s.is_empty()) {
            cred.insert("username".into(), Value::String(subject.to_string()));
        }
    }

    // 5. Engine-owned identity, written last so neither a connection's config
    //    nor its secrets can shadow it and misattribute the call.
    cred.insert("account_id".into(), Value::String(account.id.clone()));
    cred.insert(
        "provider_type".into(),
        Value::String(
            account
                .provider_type
                .clone()
                .unwrap_or_else(|| provider_type.to_string()),
        ),
    );

    Value::Object(cred)
}
