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

//! Turning `notifications/tools/list_changed` into a discovery run.
//!
//! The notification carries **no params** — it means "re-list", nothing more.
//! So the whole job here is to schedule a re-list for the right connection and
//! get out of the way.
//!
//! Two rules this module exists to enforce:
//!
//! 1. **Enqueue, never work inline.** This runs inside the decode path of an
//!    in-flight request, which on the POST route is an agent's `tools/call`. A
//!    re-list on that path would put a third party's bookkeeping on the latency
//!    of a tool the model is waiting for.
//! 2. **Rate-limit per connection.** The job registry's dedup key collapses
//!    duplicates *within one process only* — it is a plain in-memory `HashMap`
//!    with no storage behind it — so a chatty server could otherwise mint a job
//!    per notification. The floor below is what makes a misbehaving peer cost a
//!    bounded amount of work.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use raisin_mcp_protocol::client::{NotificationSink, TOOLS_LIST_CHANGED};
use raisin_storage::jobs::{JobContext, JobId, JobRegistry, JobType, McpDiscoverySource};
use serde_json::Value;

/// Branch the connection nodes live on. Discovery is a default-branch concern.
const CONFIG_BRANCH: &str = "main";
/// Workspace holding connection configuration.
const CONFIG_WORKSPACE: &str = "raisin:system";

/// Shortest gap between two notification-driven discovery runs for one
/// connection.
///
/// Not a debounce of the notification — it is a floor on the *work*. A server
/// that announces a change every second still gets re-listed, just no more than
/// once a minute, and the interval refresh remains the backstop for anything
/// that floor swallows.
const MIN_INTERVAL: Duration = Duration::from_secs(60);

/// Enqueues a discovery run when a connection announces its tools changed.
pub struct ToolChangeSink {
    registry: Arc<JobRegistry>,
    data_store: Arc<raisin_rocksdb::JobDataStore>,
    tenant_id: String,
    repo_id: String,
    slug: String,
    /// Last enqueue per connection key, for the rate limit.
    last: Arc<Mutex<HashMap<String, Instant>>>,
}

impl ToolChangeSink {
    /// Build a sink bound to one connection.
    ///
    /// The binding is safe against the session cache reusing this sink: the
    /// cache is keyed by `(tenant, repo, branch, slug, url)`, so a cached
    /// session can only ever be handed back for the same connection these
    /// fields name.
    pub fn new(
        registry: Arc<JobRegistry>,
        data_store: Arc<raisin_rocksdb::JobDataStore>,
        tenant_id: impl Into<String>,
        repo_id: impl Into<String>,
        slug: impl Into<String>,
    ) -> Self {
        Self {
            registry,
            data_store,
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            slug: slug.into(),
            last: shared_rate_limiter(),
        }
    }

    /// Whether enough time has passed to enqueue again for this connection.
    fn may_enqueue(&self, now: Instant) -> bool {
        let key = format!("{}\0{}\0{}", self.tenant_id, self.repo_id, self.slug);
        let mut guard = self.last.lock().expect("poisoned");
        match guard.get(&key) {
            Some(previous) if now.duration_since(*previous) < MIN_INTERVAL => false,
            _ => {
                guard.insert(key, now);
                true
            }
        }
    }

    /// Register the discovery job, off this thread.
    ///
    /// Registration is async, and this is called from the synchronous decode of
    /// an in-flight `tools/call` — so it is spawned rather than awaited. That is
    /// not a workaround for the signature; it is the requirement the signature
    /// encodes. Failures are logged and never propagated: a notification is
    /// advisory, and the refresh interval still covers anything lost here.
    fn enqueue(&self) {
        let registry = self.registry.clone();
        let data_store = self.data_store.clone();
        let tenant_id = self.tenant_id.clone();
        let repo_id = self.repo_id.clone();
        let slug = self.slug.clone();

        tokio::spawn(async move {
            let job_id = JobId::new();
            let context = JobContext {
                tenant_id: tenant_id.clone(),
                repo_id: repo_id.clone(),
                branch: CONFIG_BRANCH.to_string(),
                workspace_id: CONFIG_WORKSPACE.to_string(),
                revision: raisin_hlc::HLC::now(),
                metadata: HashMap::new(),
            };

            // Context before registration: the dispatcher may claim the job the
            // instant it is registered.
            if let Err(e) = data_store.put(&job_id, &context) {
                tracing::warn!(
                    %slug,
                    error = %e,
                    "could not store context for a notification-driven MCP discovery"
                );
                return;
            }

            let dedup = format!("mcp-discovery:{tenant_id}:{repo_id}:{slug}");
            let job_type = JobType::McpToolDiscovery {
                connection_slug: slug.clone(),
                tenant_id: tenant_id.clone(),
                repo_id: repo_id.clone(),
                source: McpDiscoverySource::Notification,
            };
            if let Err(e) = registry
                .register_job_with_id_idempotent(job_id, job_type, tenant_id, dedup, None)
                .await
            {
                tracing::warn!(
                    %slug,
                    error = %e,
                    "could not enqueue a notification-driven MCP discovery"
                );
            }
        });
    }
}

impl NotificationSink for ToolChangeSink {
    fn on_notification(&self, method: &str, _params: &Value) {
        // Every other notification type is somebody else's concern; ignoring
        // one is correct, not a gap.
        if method != TOOLS_LIST_CHANGED {
            return;
        }
        if !self.may_enqueue(Instant::now()) {
            tracing::debug!(
                slug = %self.slug,
                "tools/list_changed inside the rate-limit window; not re-listing"
            );
            return;
        }
        tracing::info!(slug = %self.slug, "remote server announced a tool change; re-listing");
        self.enqueue();
    }
}

/// One rate-limit table for the process.
///
/// Shared rather than per-sink because the session cache can evict and rebuild
/// a session — and with it the sink — at any time. A per-sink table would reset
/// its window on every rebuild, which is exactly the case the limit exists for.
fn shared_rate_limiter() -> Arc<Mutex<HashMap<String, Instant>>> {
    static LIMITER: std::sync::OnceLock<Arc<Mutex<HashMap<String, Instant>>>> =
        std::sync::OnceLock::new();
    LIMITER
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rate limit is the only part testable without a live job registry,
    /// and it is the part that bounds a hostile peer.
    fn limiter() -> Arc<Mutex<HashMap<String, Instant>>> {
        Arc::new(Mutex::new(HashMap::new()))
    }

    fn check(last: &Arc<Mutex<HashMap<String, Instant>>>, slug: &str, now: Instant) -> bool {
        let key = format!("t\0r\0{slug}");
        let mut guard = last.lock().unwrap();
        match guard.get(&key) {
            Some(previous) if now.duration_since(*previous) < MIN_INTERVAL => false,
            _ => {
                guard.insert(key, now);
                true
            }
        }
    }

    #[test]
    fn a_burst_from_one_connection_enqueues_once() {
        let last = limiter();
        let now = Instant::now();
        assert!(check(&last, "linear", now));
        for _ in 0..100 {
            assert!(!check(&last, "linear", now));
        }
    }

    #[test]
    fn the_window_reopens_after_the_floor() {
        let last = limiter();
        let now = Instant::now();
        assert!(check(&last, "linear", now));
        assert!(check(&last, "linear", now + MIN_INTERVAL));
    }

    /// One noisy connection must not silence a quiet one.
    #[test]
    fn connections_are_limited_independently() {
        let last = limiter();
        let now = Instant::now();
        assert!(check(&last, "linear", now));
        assert!(check(&last, "sentry", now));
    }
}
