// SPDX-License-Identifier: BSL-1.1

//! Short-lived store for the OAuth `state` parameter.
//!
//! The `state` value ties an outbound-OAuth `/start` request to its eventual
//! `/callback`, carrying the tenant/repo/integration it belongs to and guarding
//! against CSRF. Entries expire after a short TTL and are single-use (consumed
//! on the first successful `/callback`).
//!
//! The backing store is an in-process map. That is correct for a single-node
//! server; a multi-node deployment behind a load balancer must pin the OAuth
//! start/callback pair to the same node (the flow is a one-shot admin action, so
//! this is an acceptable v1 constraint — noted in the plan).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// Default lifetime of an OAuth `state` entry.
pub const STATE_TTL_SECS: u64 = 600;

/// Which flow minted a `state`, so the shared callback can route it.
///
/// The connector flow and the MCP-connection flow both redirect a browser back
/// to their own path, but they store state in ONE place: two stores would mean
/// two TTLs, two eviction policies and two chances to get single-use wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKind {
    /// `raisin:Integration` connector OAuth.
    Integration,
    /// `raisin:McpConnection` outbound MCP OAuth.
    McpConnection,
}

/// What a `state` value resolves to at callback time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateEntry {
    pub tenant_id: String,
    pub repo: String,
    /// Path of the node this flow is authorizing — a `raisin:Integration` or a
    /// `raisin:McpConnection`, per [`kind`](StateEntry::kind).
    pub target_path: String,
    /// Which flow minted this entry.
    pub kind: StateKind,
    /// PKCE verifier, held until the token exchange. `None` for flows that do
    /// not use PKCE (the legacy connector flow).
    ///
    /// Kept HERE rather than in a cookie or the redirect: it is the secret half
    /// of the challenge, and the whole point of PKCE is that it never leaves
    /// the server that started the flow.
    pub pkce_verifier: Option<String>,
    /// Unix epoch seconds after which the entry is no longer valid.
    pub expires_at: u64,
}

/// In-process, TTL'd, single-use store for OAuth `state` values.
#[derive(Default)]
pub struct OAuthStateStore {
    entries: Mutex<HashMap<String, StateEntry>>,
}

impl OAuthStateStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Insert a `state` entry that expires `ttl_secs` after `now_secs`.
    pub fn insert(
        &self,
        state: String,
        tenant_id: String,
        repo: String,
        target_path: String,
        now_secs: u64,
        ttl_secs: u64,
    ) {
        self.insert_kind(
            state,
            tenant_id,
            repo,
            target_path,
            StateKind::Integration,
            None,
            now_secs,
            ttl_secs,
        )
    }

    /// Insert an entry for a specific flow, optionally carrying a PKCE verifier.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_kind(
        &self,
        state: String,
        tenant_id: String,
        repo: String,
        target_path: String,
        kind: StateKind,
        pkce_verifier: Option<String>,
        now_secs: u64,
        ttl_secs: u64,
    ) {
        let mut guard = self.entries.lock().unwrap();
        // Opportunistically evict anything already expired so the map cannot
        // grow without bound on abandoned flows.
        guard.retain(|_, e| e.expires_at > now_secs);
        guard.insert(
            state,
            StateEntry {
                tenant_id,
                repo,
                target_path,
                kind,
                pkce_verifier,
                expires_at: now_secs.saturating_add(ttl_secs),
            },
        );
    }

    /// Consume a `state` entry: remove it and return it only if it exists, has
    /// not expired at `now_secs`, and belongs to `expected`.
    ///
    /// The kind is a REQUIRED argument rather than something the caller checks
    /// afterwards. One store serves several OAuth flows, so without this a
    /// state minted by one flow could be redeemed at another flow's callback,
    /// handing it a `target_path` that points at a node of the wrong type.
    /// Making it a parameter means a future callback cannot forget the check:
    /// it cannot call `consume` without stating what it expects.
    ///
    /// Unknown, expired and wrong-kind are all `None` — indistinguishable to
    /// the caller, by design.
    pub fn consume(&self, state: &str, expected: StateKind, now_secs: u64) -> Option<StateEntry> {
        let mut guard = self.entries.lock().unwrap();
        let entry = guard.remove(state)?;
        if entry.expires_at <= now_secs || entry.kind != expected {
            None
        } else {
            Some(entry)
        }
    }
}

/// Process-global OAuth state store shared by the `/start` and `/callback`
/// handlers.
pub static OAUTH_STATE_STORE: LazyLock<OAuthStateStore> = LazyLock::new(OAuthStateStore::new);

/// Current wall-clock time in unix epoch seconds.
pub fn now_secs() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_valid_state() {
        let store = OAuthStateStore::new();
        store.insert(
            "abc".into(),
            "t1".into(),
            "repo1".into(),
            "/integrations/gdrive".into(),
            1000,
            STATE_TTL_SECS,
        );
        let got = store
            .consume("abc", StateKind::Integration, 1100)
            .expect("should resolve");
        assert_eq!(got.tenant_id, "t1");
        assert_eq!(got.repo, "repo1");
        assert_eq!(got.target_path, "/integrations/gdrive");
        assert_eq!(got.kind, StateKind::Integration);
    }

    #[test]
    fn an_mcp_entry_carries_its_pkce_verifier_and_kind() {
        let store = OAuthStateStore::new();
        store.insert_kind(
            "abc".into(),
            "t".into(),
            "r".into(),
            "/mcp-connections/linear".into(),
            StateKind::McpConnection,
            Some("verifier".into()),
            0,
            600,
        );
        let got = store
            .consume("abc", StateKind::McpConnection, 1)
            .expect("should resolve");
        assert_eq!(got.kind, StateKind::McpConnection);
        // The verifier must survive to the token exchange; losing it makes
        // every PKCE code exchange fail with invalid_grant.
        assert_eq!(got.pkce_verifier.as_deref(), Some("verifier"));
    }

    #[test]
    fn state_is_single_use() {
        let store = OAuthStateStore::new();
        store.insert("abc".into(), "t".into(), "r".into(), "/i".into(), 0, 600);
        assert!(store.consume("abc", StateKind::Integration, 1).is_some());
        // Second consume finds nothing.
        assert!(store.consume("abc", StateKind::Integration, 1).is_none());
    }

    #[test]
    fn expired_state_is_rejected() {
        let store = OAuthStateStore::new();
        store.insert("abc".into(), "t".into(), "r".into(), "/i".into(), 0, 600);
        // now == expires_at (600) -> expired (exclusive upper bound).
        assert!(store.consume("abc", StateKind::Integration, 600).is_none());
    }

    /// A state minted by one flow must not be redeemable at another's
    /// callback: that callback would treat `target_path` as a node of its own
    /// type and act on the wrong object.
    #[test]
    fn a_state_cannot_be_redeemed_by_the_wrong_flow() {
        let store = OAuthStateStore::new();
        store.insert_kind(
            "abc".into(),
            "t".into(),
            "r".into(),
            "/mcp-connections/linear".into(),
            StateKind::McpConnection,
            Some("verifier".into()),
            0,
            600,
        );
        assert!(store.consume("abc", StateKind::Integration, 1).is_none());
    }

    #[test]
    fn unknown_state_is_rejected() {
        let store = OAuthStateStore::new();
        assert!(store.consume("nope", StateKind::Integration, 0).is_none());
    }
}
