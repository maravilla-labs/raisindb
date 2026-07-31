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

//! One connection on a connector, and the rules for choosing between several.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How a connection authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    /// Tokens obtained through the OAuth authorization-code flow.
    #[default]
    Oauth,
    /// Credentials entered directly (an IMAP username + app password).
    Config,
}

/// One entry in a `raisin:Integration.connected_accounts` array — i.e. one
/// *connection*.
///
/// Every field past `id` is `#[serde(default)]` so entries written before
/// per-connection config existed (OAuth-only: `id`/`subject`/`expires_at`/
/// `tokens_encrypted`) keep deserializing untouched.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectedAccount {
    pub id: String,
    /// Operator-supplied name. Falls back to `subject` in UIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Verified account identity from the provider (the OIDC `email` claim).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    #[serde(default)]
    pub auth_kind: AuthKind,
    /// Unix seconds at which `access_token` expires. Drives the refresh job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// base64 AES-256-GCM of `{ access_token, refresh_token }`. OAuth only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_encrypted: Option<String>,
    /// Per-connection NON-SECRET config, validated against the connector's
    /// `connection_config_type` (host/port/tenant_id/...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    /// base64 AES-256-GCM of a `{ field: value }` map of this connection's
    /// secret-flagged fields (an IMAP app password).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets_encrypted: Option<String>,
    /// Names **only** of the fields inside `secrets_encrypted`, so a UI can show
    /// "is set" without decrypting.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

impl ConnectedAccount {
    /// Best-effort human label: explicit name, else the verified identity, else
    /// the opaque id.
    pub fn display_label(&self) -> &str {
        self.label
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(self.subject.as_deref().filter(|s| !s.is_empty()))
            .unwrap_or(&self.id)
    }

    /// This connection's config object (empty when unset).
    pub fn config_or_empty(&self) -> Value {
        match &self.config {
            Some(v) if v.is_object() => v.clone(),
            _ => Value::Object(serde_json::Map::new()),
        }
    }
}

/// Why a connection could not be chosen.
///
/// These are surfaced as a `misconfigured` mount status rather than being
/// silently papered over — see [`AccountSelection::resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountSelectionError {
    /// The connector has no connections at all.
    NoAccounts,
    /// `account_ref` names a connection that no longer exists (it was
    /// disconnected, or the mount was copied from another repo).
    NotFound { account_ref: String },
    /// Several connections exist and the mount does not say which to use.
    /// Guessing here would sync the wrong mailbox.
    Ambiguous { candidates: Vec<String> },
}

impl std::fmt::Display for AccountSelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAccounts => write!(
                f,
                "the connector has no connected account — connect one before syncing"
            ),
            Self::NotFound { account_ref } => write!(
                f,
                "connected account `{account_ref}` no longer exists on this connector"
            ),
            Self::Ambiguous { candidates } => write!(
                f,
                "this connector has {} connections ({}) — set the mount's account so it \
                 syncs a known one",
                candidates.len(),
                candidates.join(", ")
            ),
        }
    }
}

/// Choosing which connection a mount uses.
pub struct AccountSelection;

impl AccountSelection {
    /// Resolve `account_ref` against a connector's connections.
    ///
    /// | `account_ref` | connections | result |
    /// |---|---|---|
    /// | set | matching | that connection |
    /// | set | no match | [`AccountSelectionError::NotFound`] |
    /// | unset | exactly 1 | that connection |
    /// | unset | 0 | [`AccountSelectionError::NoAccounts`] |
    /// | unset | 2+ | [`AccountSelectionError::Ambiguous`] |
    ///
    /// The single-connection fallback is kept deliberately: it is what every
    /// existing deployment relies on, and with one connection there is nothing
    /// to get wrong. What is *removed* is the old "just take the first" rule,
    /// which silently synced an arbitrary mailbox as soon as a second
    /// connection appeared — a wrong-data bug with no error anywhere. Failing
    /// loudly is the point.
    pub fn resolve<'a>(
        accounts: &'a [ConnectedAccount],
        account_ref: Option<&str>,
    ) -> Result<&'a ConnectedAccount, AccountSelectionError> {
        match account_ref.filter(|s| !s.is_empty()) {
            Some(id) => accounts.iter().find(|a| a.id == id).ok_or_else(|| {
                AccountSelectionError::NotFound {
                    account_ref: id.to_string(),
                }
            }),
            None => match accounts.len() {
                0 => Err(AccountSelectionError::NoAccounts),
                1 => Ok(&accounts[0]),
                _ => Err(AccountSelectionError::Ambiguous {
                    candidates: accounts
                        .iter()
                        .map(|a| a.display_label().to_string())
                        .collect(),
                }),
            },
        }
    }
}
