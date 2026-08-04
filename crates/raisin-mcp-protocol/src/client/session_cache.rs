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

//! Reusing a negotiated session across tool calls.
//!
//! Without this, every proxy tool call costs TWO round trips to the remote
//! server: an `initialize` handshake and then the `tools/call`. An agent turn
//! that calls three tools pays six. The handshake result — the agreed protocol
//! revision and the server's `Mcp-Session-Id` — is stable for the life of a
//! connection, so it is exactly what a cache is for.
//!
//! Two things the key must carry, and why:
//!
//! - **Tenant, repo and branch.** A cache keyed only by connection slug would
//!   serve one tenant's session to another. Scoping is not an optimization
//!   detail here; it is the isolation boundary.
//! - **The URL.** Editing a connection's endpoint must not keep dialling the
//!   old host. Including it means a URL change simply misses, with no
//!   invalidation hook to forget to call.
//!
//! Credentials are deliberately NOT part of the key: they travel per-request as
//! headers and are never stored in a session, so rotating one cannot leave a
//! cached session presenting a stale token.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::session::McpClientSession;

/// How long an idle session stays cached.
///
/// A backstop, not the correctness mechanism: the client already re-initializes
/// when a server drops its session (404 on a request carrying an
/// `Mcp-Session-Id`). This just stops a connection nobody uses from pinning a
/// session forever.
const SESSION_TTL: Duration = Duration::from_secs(15 * 60);

/// Maximum cached sessions before the oldest are dropped.
///
/// Bounded because the key includes a per-connection URL: a tenant that churns
/// connections would otherwise grow this without limit.
const MAX_SESSIONS: usize = 256;

/// Identity of a cached session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SessionKey {
    tenant: String,
    repo: String,
    branch: String,
    slug: String,
    url: String,
}

struct Entry {
    session: Arc<McpClientSession>,
    last_used: Instant,
}

/// Process-wide cache of negotiated sessions.
#[derive(Default)]
pub struct SessionCache {
    entries: Mutex<HashMap<SessionKey, Entry>>,
}

impl SessionCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Fetch the cached session for this connection, or build one with `make`.
    ///
    /// `make` is only called on a miss. It is NOT run under the lock: building a
    /// transport is cheap, and holding a mutex across it would serialize every
    /// tool call in the process behind one connection's setup.
    pub fn get_or_insert<F>(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
        slug: &str,
        url: &str,
        make: F,
    ) -> Arc<McpClientSession>
    where
        F: FnOnce() -> McpClientSession,
    {
        let key = SessionKey {
            tenant: tenant.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
            slug: slug.to_string(),
            url: url.to_string(),
        };

        if let Some(session) = self.hit(&key) {
            return session;
        }

        let session = Arc::new(make());
        let mut guard = match self.entries.lock() {
            Ok(guard) => guard,
            // A poisoned cache must not take the call down with it; losing the
            // cache costs a handshake, nothing more.
            Err(_) => return session,
        };
        prune(&mut guard);
        // Another caller may have raced us here. Prefer whatever landed first
        // so concurrent callers converge on ONE session per connection rather
        // than each holding a private handshake.
        if let Some(existing) = guard.get_mut(&key) {
            existing.last_used = Instant::now();
            return Arc::clone(&existing.session);
        }
        guard.insert(
            key,
            Entry {
                session: Arc::clone(&session),
                last_used: Instant::now(),
            },
        );
        session
    }

    /// A live entry for `key`, refreshing its idle timer.
    fn hit(&self, key: &SessionKey) -> Option<Arc<McpClientSession>> {
        let mut guard = self.entries.lock().ok()?;
        let entry = guard.get_mut(key)?;
        if entry.last_used.elapsed() > SESSION_TTL {
            guard.remove(key);
            return None;
        }
        entry.last_used = Instant::now();
        Some(Arc::clone(&entry.session))
    }

    /// Drop every cached session.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.entries.lock() {
            guard.clear();
        }
    }

    /// Number of cached sessions (test/diagnostic).
    pub fn len(&self) -> usize {
        self.entries.lock().map(|g| g.len()).unwrap_or(0)
    }

    /// Whether the cache holds nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Evict expired entries, then the least recently used until under the cap.
fn prune(entries: &mut HashMap<SessionKey, Entry>) {
    entries.retain(|_, e| e.last_used.elapsed() <= SESSION_TTL);
    while entries.len() >= MAX_SESSIONS {
        let Some(oldest) = entries
            .iter()
            .min_by_key(|(_, e)| e.last_used)
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        entries.remove(&oldest);
    }
}

/// The process-wide session cache.
///
/// Shared rather than per-`ExecutionDependencies`: that struct is rebuilt per
/// request on the HTTP, WS and MCP paths, so a cache living on it would be
/// discarded before its second use — which is the entire point of having one.
pub fn shared_session_cache() -> &'static SessionCache {
    static CACHE: OnceLock<SessionCache> = OnceLock::new();
    CACHE.get_or_init(SessionCache::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::StreamableHttpTransport;
    use url::Url;

    fn session(url: &str) -> McpClientSession {
        McpClientSession::new(StreamableHttpTransport::new(
            reqwest::Client::new(),
            Url::parse(url).unwrap(),
            Duration::from_secs(5),
        ))
    }

    const URL: &str = "https://a.test/mcp";

    #[test]
    fn a_second_call_reuses_the_session() {
        let cache = SessionCache::new();
        let first = cache.get_or_insert("t", "r", "main", "s", URL, || session(URL));
        let second = cache.get_or_insert("t", "r", "main", "s", URL, || {
            panic!("must not re-handshake on a hit")
        });
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn tenants_never_share_a_session() {
        let cache = SessionCache::new();
        let a = cache.get_or_insert("t1", "r", "main", "s", URL, || session(URL));
        let b = cache.get_or_insert("t2", "r", "main", "s", URL, || session(URL));
        // Sharing here would hand one tenant another's negotiated session.
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn repo_and_branch_also_scope_the_key() {
        let cache = SessionCache::new();
        cache.get_or_insert("t", "r1", "main", "s", URL, || session(URL));
        cache.get_or_insert("t", "r2", "main", "s", URL, || session(URL));
        cache.get_or_insert("t", "r1", "dev", "s", URL, || session(URL));
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn changing_the_url_misses_rather_than_dialling_the_old_host() {
        let cache = SessionCache::new();
        let old = cache.get_or_insert("t", "r", "main", "s", URL, || session(URL));
        let new_url = "https://b.test/mcp";
        let new = cache.get_or_insert("t", "r", "main", "s", new_url, || session(new_url));
        assert!(!Arc::ptr_eq(&old, &new));
    }

    #[test]
    fn clear_drops_everything() {
        let cache = SessionCache::new();
        cache.get_or_insert("t", "r", "main", "s", URL, || session(URL));
        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn the_cache_stays_bounded() {
        let cache = SessionCache::new();
        for i in 0..(MAX_SESSIONS + 20) {
            let slug = format!("s{i}");
            cache.get_or_insert("t", "r", "main", &slug, URL, || session(URL));
        }
        // `prune` evicts down to MAX_SESSIONS - 1 before each insert, so the
        // cap is the exact ceiling, never exceeded.
        assert!(
            cache.len() <= MAX_SESSIONS,
            "cache must stay bounded, got {}",
            cache.len()
        );
    }
}
