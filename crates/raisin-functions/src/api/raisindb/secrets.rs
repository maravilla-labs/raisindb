// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Secret store operations for `RaisinFunctionApi` — and the policy gate.
//!
//! # The gate is here, and only here
//!
//! Every one of the five operations calls [`authorize_secret`] (or, for `list`,
//! [`authorize_secret_subsystem`]) BEFORE it touches a callback, mirroring how
//! `imap.rs` calls `authorize_imap` before opening a socket. The callbacks
//! themselves do no checking, so a future caller that reaches for one directly
//! would bypass the policy — which is exactly why there is one branch, not five
//! scattered checks.
//!
//! # Why deny-by-default is not merely tidy here
//!
//! Virtual-node adapters run privileged (row-level security bypassed) and hold
//! `raisin.http`. An unrestricted secrets binding would therefore hand any
//! adapter a one-line exfiltration path for every credential in the repo. The
//! [`SecretPolicy`](crate::types::SecretPolicy) default is "no access", and an
//! enabled-but-empty allow-list still denies, so "opted in" can never
//! accidentally mean "unrestricted".
//!
//! # Denials are loud
//!
//! A denied read is a [`PermissionDenied`](raisin_error::Error::PermissionDenied)
//! naming the secret and the policy — never `None`, never an empty list. A
//! silent empty result reads exactly like "that secret does not exist", which
//! sends the function author chasing a missing secret instead of a missing
//! grant. `list` is the one shaped differently: it is not a denial to omit
//! names the caller may not read, so it FILTERS (and still errors loudly when
//! the subsystem is off entirely).

use raisin_error::Result;
use raisin_models::secret_ref::SecretRef;
use serde_json::Value;

use super::RaisinFunctionApi;
use crate::api::secret_spec::{parse_read_spec, parse_write_spec};

fn disabled(op: &str) -> raisin_error::Error {
    raisin_error::Error::Validation(format!(
        "Secret store not configured (cannot {op}). It needs a master keyring: \
         set RAISIN_MASTER_KEYS (or RAISIN_MASTER_KEY) on every node."
    ))
}

impl RaisinFunctionApi {
    /// The policy gate for one named secret.
    ///
    /// Returns the reason as a `PermissionDenied` error, naming both the secret
    /// and what would grant it, so the fix is visible from the message alone.
    fn authorize_secret(&self, op: &str, name: &str) -> Result<()> {
        if self.secret_policy.is_name_allowed(name) {
            return Ok(());
        }

        // Deliberately no secret VALUE anywhere in this message — only the name,
        // which the caller already supplied.
        let detail = if !self.secret_policy.enabled {
            "this function has no secret_policy (secret access is denied by default)".to_string()
        } else if self.secret_policy.allowed_names.is_empty() {
            "this function's secret_policy has an empty allowed_names list".to_string()
        } else {
            format!(
                "not matched by this function's secret_policy.allowed_names {:?}",
                self.secret_policy.allowed_names
            )
        };

        tracing::warn!(
            secret_name = %name,
            operation = %op,
            policy_enabled = self.secret_policy.enabled,
            "raisin.secrets call denied by secret policy"
        );

        Err(raisin_error::Error::PermissionDenied(format!(
            "[secrets:policy_denied] cannot {op} secret '{name}': {detail}. \
             Grant it by adding a matching pattern to the function's \
             secret_policy.allowed_names."
        )))
    }

    /// The gate for operations that name no single secret (`list`).
    fn authorize_secret_subsystem(&self, op: &str) -> Result<()> {
        if self.secret_policy.enabled && !self.secret_policy.allowed_names.is_empty() {
            return Ok(());
        }
        let detail = if !self.secret_policy.enabled {
            "this function has no secret_policy (secret access is denied by default)"
        } else {
            "this function's secret_policy has an empty allowed_names list"
        };
        Err(raisin_error::Error::PermissionDenied(format!(
            "[secrets:policy_denied] cannot {op}: {detail}. Grant access by adding \
             patterns to the function's secret_policy.allowed_names."
        )))
    }

    /// Read a secret, named EITHER as a bare name or as a full `secret://`
    /// reference.
    ///
    /// The reference form is the one the primary use case produces: a node
    /// property in an `encrypted: true` field holds
    /// `secret://node/{id}/{field}@N`, node reads never resolve, so this is the
    /// exact string the function was handed.
    ///
    /// Parsing happens BEFORE the policy gate, so both spellings of one secret
    /// get one allow/deny answer, and a pinned version in the reference is
    /// honoured — an older node revision resolves to what it actually held.
    pub(crate) async fn impl_secret_get(&self, spec: &str, version: Option<i64>) -> Result<String> {
        let parsed = parse_read_spec(spec, version)?;
        self.authorize_secret("read", &parsed.name)?;
        let callback = self
            .callbacks
            .secret_get
            .as_ref()
            .ok_or_else(|| disabled("read a secret"))?;
        callback(parsed.name, parsed.version).await
    }

    /// Resolve a property value that MAY be a `secret://` reference.
    ///
    /// - not a reference → returned unchanged, and NO policy check runs,
    ///   because nothing secret was touched. That is what lets an adapter read
    ///   a config field that is a literal password on one deployment and a
    ///   vaulted reference on another with the same line of code.
    /// - a reference → parsed, gated, resolved to plaintext.
    /// - a reference that fails to resolve → an ERROR.
    ///
    /// That last case is the one worth being careful about, and it mirrors why
    /// `SecretStore::resolve_if_reference` returns `Result<Option<_>>` rather
    /// than `Option<_>`: if a failed resolve fell back to returning the input,
    /// the caller would send the literal text `secret://stripe_key` to a
    /// payment provider as an API key. Unchanged-literal, plaintext, and
    /// failure stay three distinct outcomes.
    pub(crate) async fn impl_secret_resolve(&self, value: &str) -> Result<String> {
        if !SecretRef::is_secret_ref(value) {
            return Ok(value.to_string());
        }
        self.impl_secret_get(value, None).await
    }

    pub(crate) async fn impl_secret_put(&self, spec: &str, value: &str) -> Result<Value> {
        let name = parse_write_spec(spec, "write")?;
        self.authorize_secret("write", &name)?;
        let callback = self
            .callbacks
            .secret_put
            .as_ref()
            .ok_or_else(|| disabled("write a secret"))?;
        callback(name, value.to_string()).await
    }

    pub(crate) async fn impl_secret_list(&self) -> Result<Vec<Value>> {
        self.authorize_secret_subsystem("list secrets")?;
        let callback = self
            .callbacks
            .secret_list
            .as_ref()
            .ok_or_else(|| disabled("list secrets"))?;
        let all = callback().await?;

        // Filter rather than deny: a listing is not a read, and a caller that
        // may read `stripe/*` should see those and nothing else. Anything whose
        // name we cannot parse is dropped — showing a name the policy could not
        // be evaluated against would leak the very enumeration this prevents.
        Ok(all
            .into_iter()
            .filter(|entry| {
                entry
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| self.secret_policy.is_name_allowed(n))
                    .unwrap_or(false)
            })
            .collect())
    }

    pub(crate) async fn impl_secret_rotate(&self, spec: &str, value: &str) -> Result<Value> {
        let name = parse_write_spec(spec, "rotate")?;
        self.authorize_secret("rotate", &name)?;
        let callback = self
            .callbacks
            .secret_rotate
            .as_ref()
            .ok_or_else(|| disabled("rotate a secret"))?;
        callback(name, value.to_string()).await
    }

    pub(crate) async fn impl_secret_delete(&self, spec: &str) -> Result<Value> {
        let name = parse_write_spec(spec, "delete")?;
        self.authorize_secret("delete", &name)?;
        let callback = self
            .callbacks
            .secret_delete
            .as_ref()
            .ok_or_else(|| disabled("delete a secret"))?;
        callback(name).await
    }
}
