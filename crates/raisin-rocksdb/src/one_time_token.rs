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

//! One-time tokens, stored as `raisin:OneTimeToken` NODES.
//!
//! # Why nodes and not a column family
//!
//! Every other short-lived auth record in this crate ([`crate::repositories::SessionRepository`],
//! [`crate::repositories::IdentityRepository`]) is a column family with its own
//! key builders, its own replication OpType and its own GC. A one-time token
//! needs none of that: `raisin:OneTimeToken` already exists as a global
//! nodetype, `raisin:system` already allows it, and node writes already
//! replicate through `CreateNode`/`DeleteNode`. Adding a fourth storage shape
//! for a record that lives fifteen minutes would be four more places for the
//! auth surface to drift apart.
//!
//! The nodes live at `/tokens/{token_id}` in the `raisin:system` workspace on
//! the repo's `main` branch — the `/tokens` folder is part of that workspace's
//! `initial_structure`, so it is there on a freshly created repo.
//!
//! # Lookup: prefix is indexed, hash deliberately is not
//!
//! `token_prefix` carries `index: [Property]`; `token_hash` does not, and that
//! asymmetry is the point. Indexing the hash would put a lookup key derived
//! from the secret into a scannable index, and would invite a "find the token
//! whose hash is X" call whose latency leaks how close a guess was. So
//! [`OneTimeTokenStore::find_by_token`] narrows on the (non-secret, 8-char)
//! prefix and then compares the full hash in constant time, byte by byte,
//! across every candidate — no early return, so a partial match costs exactly
//! what a total mismatch costs.
//!
//! # This store never decides whether a token is *acceptable*
//!
//! It returns what is stored — including expired and already-used tokens.
//! Validity (`OneTimeToken::is_valid`), purpose matching and the mark-used
//! write are the caller's, because only the caller knows what it is
//! authenticating and what "fail closed" means for that flow.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_core::services::node_service::NodeService;
use raisin_error::{Error, Result};
use raisin_models::auth::{AuthContext, OneTimeToken, TokenPurpose};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::timestamp::StorageTimestamp;

use crate::RocksDBStorage;

/// Workspace holding per-tenant auth records.
const TOKEN_WORKSPACE: &str = "raisin:system";
/// Branch the auth records live on. Pinned, exactly as the `RepoAuthConfig`
/// read in the HTTP auth middleware is: a token minted against `main` must be
/// verifiable from a request that carries no branch at all.
const TOKEN_BRANCH: &str = "main";
/// Parent folder of the token nodes (seeded by `raisin:system`'s
/// `initial_structure`).
const TOKENS_PATH: &str = "/tokens";
/// The nodetype this store reads and writes.
const TOKEN_NODE_TYPE: &str = "raisin:OneTimeToken";
/// Reserved `context` key carrying the full serialized [`TokenPurpose`].
///
/// The indexed `purpose` property holds only the variant tag, because that is
/// what a query filters on. Data-carrying variants (`WorkspaceInvite`,
/// `ApiAccess`) would lose their payload to that flattening, so the complete
/// value rides along here and wins on read.
const PURPOSE_DETAIL_KEY: &str = "raisin:purpose";

/// Sentinel prepended to `token_prefix` before it is stored or queried.
///
/// WORKAROUND for a storage-layer coercion, not a design choice. A String
/// property whose value is entirely digits is written back as a
/// `PropertyValue::Decimal`, not the `String` that was handed in — confirmed by
/// reading a stored node back and finding `Decimal(11112222333344445)` where a
/// `String` was written. Two things then break at once: `find_by_property`
/// double-checks `properties.get(name) == Some(String(..))` and rejects the
/// node, and any `match ... Some(PropertyValue::String(h))` arm falls through.
///
/// That matters here more than almost anywhere else. A token prefix is the first
/// 8 characters of `hex::encode(32 random bytes)`, so it is all-digits with
/// probability (10/16)^8 — roughly one magic link in 43. The failure is silent
/// and total: the user clicks a valid, unexpired link and is told it is invalid,
/// with nothing in the logs to say why.
///
/// A single non-digit character in front sidesteps it, because the stored value
/// is then never entirely numeric. The sentinel is an internal storage detail:
/// `token_from_node` strips it, so `OneTimeToken.token_prefix` still round-trips
/// as the real prefix and no caller has to know.
///
/// `token_hash` is deliberately NOT given the same treatment: it is a full
/// sha256 in hex, so all-digits has probability (10/16)^64 and guarding it would
/// be ceremony. If the coercion is ever fixed, remove this — but note that links
/// minted before the change carry the sentinel, so the read side has to keep
/// tolerating it (which `strip_sentinel` already does) through at least one
/// token lifetime.
const PREFIX_SENTINEL: &str = "p:";

/// The value actually stored in, and queried against, the property index.
fn indexed_prefix(token_prefix: &str) -> String {
    format!("{PREFIX_SENTINEL}{token_prefix}")
}

/// Undo [`indexed_prefix`], tolerating a value written before the sentinel
/// existed so an in-flight link is not invalidated by deploying this.
fn strip_sentinel(stored: &str) -> String {
    stored
        .strip_prefix(PREFIX_SENTINEL)
        .unwrap_or(stored)
        .to_string()
}

/// Node-backed create / find / mark-used / prune for one-time tokens.
///
/// Scoped to one tenant+repo at construction: a token minted for one repo must
/// never be findable from another, and threading the scope through every call
/// is how that gets forgotten.
pub struct OneTimeTokenStore {
    storage: Arc<RocksDBStorage>,
    tenant_id: String,
    repo_id: String,
}

impl OneTimeTokenStore {
    /// Bind a store to one tenant/repo pair.
    pub fn new(
        storage: Arc<RocksDBStorage>,
        tenant_id: impl Into<String>,
        repo_id: impl Into<String>,
    ) -> Self {
        Self {
            storage,
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
        }
    }

    /// The system-auth `NodeService` every operation goes through.
    ///
    /// System context because token nodes are infrastructure: the principal
    /// asking to verify a magic link is by definition not yet authenticated,
    /// so there is no user context that could pass RLS on `/tokens`.
    fn node_service(&self) -> NodeService<RocksDBStorage> {
        NodeService::new_with_context(
            self.storage.clone(),
            self.tenant_id.clone(),
            self.repo_id.clone(),
            TOKEN_BRANCH.to_string(),
            TOKEN_WORKSPACE.to_string(),
        )
        .with_auth(AuthContext::system())
    }

    /// Persist a freshly minted token.
    ///
    /// The caller has already hashed it; `token.token_hash` is what is stored
    /// and the plaintext never reaches this module.
    pub async fn create(&self, token: &OneTimeToken) -> Result<Node> {
        let node = Node {
            id: String::new(),   // assigned by add_deep_node
            path: String::new(), // derived from parent + name
            node_type: TOKEN_NODE_TYPE.to_string(),
            name: token.token_id.clone(),
            workspace: Some(TOKEN_WORKSPACE.to_string()),
            properties: token_to_properties(token)?,
            ..Default::default()
        };

        self.node_service().add_deep_node(TOKENS_PATH, node).await
    }

    /// Find the stored token matching `token_hash`, narrowing on `token_prefix`.
    ///
    /// Returns the record as stored — expired and used tokens included. See the
    /// module docs for why validity is not decided here.
    pub async fn find_by_token(
        &self,
        token_prefix: &str,
        token_hash: &str,
    ) -> Result<Option<OneTimeToken>> {
        use raisin_storage::{NodeRepository, Storage};

        let scope = raisin_storage::StorageScope::new(
            &self.tenant_id,
            &self.repo_id,
            TOKEN_BRANCH,
            TOKEN_WORKSPACE,
        );

        let candidates = self
            .storage
            .nodes()
            .find_by_property(
                scope,
                "token_prefix",
                &PropertyValue::String(indexed_prefix(token_prefix)),
            )
            .await?;

        // Every candidate is compared, and the comparison is constant-time, so
        // neither the number of prefix collisions nor how far a wrong hash got
        // before diverging is observable in the response time.
        let mut matched: Option<Node> = None;
        for node in candidates {
            if node.node_type != TOKEN_NODE_TYPE {
                continue;
            }
            let stored_hash = match node.properties.get("token_hash") {
                Some(PropertyValue::String(h)) => h.clone(),
                _ => continue,
            };
            if constant_time_eq(stored_hash.as_bytes(), token_hash.as_bytes()) && matched.is_none()
            {
                matched = Some(node);
            }
        }

        matched.map(|node| token_from_node(&node)).transpose()
    }

    /// Mark a token consumed.
    ///
    /// Returns `false` if the node is gone (already pruned). Callers that treat
    /// a token as spent MUST fail closed when this errors: a login that
    /// proceeded on a token the store could not mark used is a replayable link.
    pub async fn mark_used(&self, token_id: &str) -> Result<bool> {
        let service = self.node_service();
        let path = format!("{TOKENS_PATH}/{token_id}");

        let Some(mut node) = service.get_by_path(&path).await? else {
            return Ok(false);
        };

        node.properties
            .insert("used".to_string(), PropertyValue::Boolean(true));
        node.properties.insert(
            "used_at".to_string(),
            PropertyValue::Date(StorageTimestamp::now()),
        );

        service.update_node(node).await?;
        Ok(true)
    }

    /// Delete every token whose `expires_at` has passed.
    ///
    /// Returns how many were removed. A used-but-unexpired token is kept until
    /// it expires: it is the only record that the link was consumed, and the
    /// audit trail for a single-use credential is worth its remaining minutes.
    pub async fn prune_expired(&self) -> Result<usize> {
        let service = self.node_service();
        let now = StorageTimestamp::now().timestamp_nanos();

        let mut pruned = 0;
        for node in service.list_by_type(TOKEN_NODE_TYPE).await? {
            let expired = matches!(
                node.properties.get("expires_at"),
                Some(PropertyValue::Date(ts)) if ts.timestamp_nanos() <= now
            );
            if expired && service.delete(&node.id).await? {
                pruned += 1;
            }
        }

        Ok(pruned)
    }
}

/// Compare two byte strings without an early return.
///
/// Length is compared first and unequal lengths short-circuit — that only
/// reveals the length of a hex hash, which is fixed and public. The bytes
/// themselves are always walked in full.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Project a [`OneTimeToken`] onto the nodetype's properties.
fn token_to_properties(token: &OneTimeToken) -> Result<HashMap<String, PropertyValue>> {
    let purpose_json = serde_json::to_value(&token.purpose)
        .map_err(|e| Error::internal(format!("failed to serialize token purpose: {e}")))?;
    let purpose_tag = purpose_json
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::internal("token purpose serialized without a type tag".to_string()))?
        .to_string();

    let mut context: HashMap<String, serde_json::Value> = token.context.clone();
    context.insert(PURPOSE_DETAIL_KEY.to_string(), purpose_json);

    let mut properties = HashMap::new();
    properties.insert(
        "token_id".to_string(),
        PropertyValue::String(token.token_id.clone()),
    );
    properties.insert(
        "tenant_id".to_string(),
        PropertyValue::String(token.tenant_id.clone()),
    );
    properties.insert(
        "token_hash".to_string(),
        PropertyValue::String(token.token_hash.clone()),
    );
    properties.insert(
        "token_prefix".to_string(),
        PropertyValue::String(indexed_prefix(&token.token_prefix)),
    );
    properties.insert("purpose".to_string(), PropertyValue::String(purpose_tag));
    if let Some(identity_id) = &token.identity_id {
        properties.insert(
            "identity_id".to_string(),
            PropertyValue::String(identity_id.clone()),
        );
    }
    properties.insert(
        "email".to_string(),
        PropertyValue::String(token.email.clone()),
    );
    properties.insert(
        "created_at".to_string(),
        PropertyValue::Date(token.created_at),
    );
    properties.insert(
        "expires_at".to_string(),
        PropertyValue::Date(token.expires_at),
    );
    properties.insert("used".to_string(), PropertyValue::Boolean(token.used));
    if let Some(used_at) = token.used_at {
        properties.insert("used_at".to_string(), PropertyValue::Date(used_at));
    }
    // `PropertyValue` is `#[serde(untagged)]`, so a JSON object deserializes
    // straight into the `Object` variant — no hand-rolled JSON-to-property
    // walk to keep in step with the enum.
    let context_value = serde_json::Value::Object(context.into_iter().collect());
    properties.insert(
        "context".to_string(),
        serde_json::from_value(context_value)
            .map_err(|e| Error::internal(format!("failed to encode token context: {e}")))?,
    );

    Ok(properties)
}

/// Rebuild a [`OneTimeToken`] from its node.
///
/// A node missing a required property is an error rather than a default: these
/// records authenticate people, and a token that read back with an empty hash
/// or a zero expiry would be a token that matches nothing — or everything.
fn token_from_node(node: &Node) -> Result<OneTimeToken> {
    let string_prop = |name: &str| -> Option<String> {
        match node.properties.get(name) {
            Some(PropertyValue::String(s)) => Some(s.clone()),
            _ => None,
        }
    };
    let date_prop = |name: &str| -> Option<StorageTimestamp> {
        match node.properties.get(name) {
            Some(PropertyValue::Date(ts)) => Some(*ts),
            _ => None,
        }
    };
    let required = |name: &str, value: Option<String>| -> Result<String> {
        value.ok_or_else(|| {
            Error::internal(format!(
                "{TOKEN_NODE_TYPE} node {} has no usable '{name}' property",
                node.path
            ))
        })
    };

    let context: HashMap<String, serde_json::Value> = match node.properties.get("context") {
        Some(PropertyValue::Object(map)) => map
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or_default()))
            .collect(),
        _ => HashMap::new(),
    };

    // The full purpose if it was stored (it always is on tokens this store
    // wrote); otherwise reconstruct the payload-free variants from the tag, so
    // a token written by an older build still reads.
    let purpose = match context.get(PURPOSE_DETAIL_KEY) {
        Some(detail) => serde_json::from_value::<TokenPurpose>(detail.clone())
            .map_err(|e| Error::internal(format!("unreadable token purpose: {e}")))?,
        None => {
            let tag = required("purpose", string_prop("purpose"))?;
            serde_json::from_value::<TokenPurpose>(serde_json::json!({ "type": tag }))
                .map_err(|e| Error::internal(format!("unreadable token purpose: {e}")))?
        }
    };

    let mut context = context;
    context.remove(PURPOSE_DETAIL_KEY);

    Ok(OneTimeToken {
        token_id: required("token_id", string_prop("token_id"))?,
        tenant_id: required("tenant_id", string_prop("tenant_id"))?,
        token_hash: required("token_hash", string_prop("token_hash"))?,
        token_prefix: strip_sentinel(&required("token_prefix", string_prop("token_prefix"))?),
        purpose,
        identity_id: string_prop("identity_id"),
        email: required("email", string_prop("email"))?,
        created_at: date_prop("created_at").unwrap_or_else(StorageTimestamp::now),
        expires_at: date_prop("expires_at").ok_or_else(|| {
            Error::internal(format!(
                "{TOKEN_NODE_TYPE} node {} has no usable 'expires_at' property",
                node.path
            ))
        })?,
        used: matches!(
            node.properties.get("used"),
            Some(PropertyValue::Boolean(true))
        ),
        used_at: date_prop("used_at"),
        context,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_token() -> OneTimeToken {
        OneTimeToken::new(
            "tok-1".to_string(),
            "tenant-1".to_string(),
            "a".repeat(64),
            "abc12345".to_string(),
            TokenPurpose::MagicLink,
            "user@example.com".to_string(),
            StorageTimestamp::now(),
        )
    }

    fn node_for(token: &OneTimeToken) -> Node {
        Node {
            node_type: TOKEN_NODE_TYPE.to_string(),
            name: token.token_id.clone(),
            path: format!("{TOKENS_PATH}/{}", token.token_id),
            properties: token_to_properties(token).expect("projectable"),
            ..Default::default()
        }
    }

    #[test]
    fn a_token_survives_the_node_round_trip() {
        let mut token = sample_token();
        token.identity_id = Some("identity-9".to_string());
        token
            .context
            .insert("redirect_url".to_string(), serde_json::json!("/app"));

        let restored = token_from_node(&node_for(&token)).expect("readable");

        assert_eq!(restored, token);
        // The reserved key is an implementation detail of the projection and
        // must not leak back into the caller's context map.
        assert!(!restored.context.contains_key(PURPOSE_DETAIL_KEY));
    }

    #[test]
    fn a_payload_carrying_purpose_is_not_flattened_to_its_tag() {
        // The indexed `purpose` property can only hold the tag; losing the
        // workspace and roles would turn an invitation into a bare magic link.
        let mut token = sample_token();
        token.purpose = TokenPurpose::WorkspaceInvite {
            workspace_id: "content".to_string(),
            roles: vec!["editor".to_string()],
        };

        let node = node_for(&token);
        assert_eq!(
            node.properties.get("purpose"),
            Some(&PropertyValue::String("workspace_invite".to_string()))
        );
        assert_eq!(
            token_from_node(&node).expect("readable").purpose,
            token.purpose
        );
    }

    #[test]
    fn a_node_without_a_hash_is_an_error_not_an_empty_match() {
        let token = sample_token();
        let mut node = node_for(&token);
        node.properties.remove("token_hash");

        assert!(token_from_node(&node).is_err());
    }

    #[test]
    fn constant_time_eq_still_answers_the_question() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
