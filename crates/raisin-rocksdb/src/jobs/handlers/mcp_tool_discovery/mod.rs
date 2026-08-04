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

//! MCP tool discovery: turn a remote server's tools into local proxy functions.
//!
//! Handles:
//! - [`JobType::McpDiscoveryCheck`] — periodic scan enqueuing due discoveries.
//! - [`JobType::McpToolDiscovery`] — discover ONE connection and reconcile it.
//!
//! For each enabled `raisin:McpConnection`, `tools/list` is called and the
//! result reconciled into one proxy `raisin:Function` per remote tool, under
//! `/mcp/{slug}/` in the functions workspace. Agents then reference those paths
//! from their `tools:` array exactly like any local function.
//!
//! Two properties this module exists to protect:
//!
//! - **A steady-state refresh writes nothing.** Discovery may run hourly
//!   forever; without the schema-hash guard in `reconcile_plan` each connection
//!   would mint thousands of function revisions a year for no reason.
//! - **Proxy paths never change.** An agent holds a path. A rename makes the
//!   tool silently vanish from that agent, with no error anywhere.
//!
//! Cluster safety comes from the per-connection lease, NOT from the job's dedup
//! key: `JobRegistry`'s dedup map is an in-memory `HashMap` with no storage
//! behind it, so every node runs its own scan. Without the lease, N nodes would
//! each write the same proxy nodes.

mod apply;
mod check;

use std::sync::Arc;
use std::time::Duration;

use raisin_error::Result;
use raisin_locks::LockManagerHandle;
use raisin_mcp_protocol::client::{
    reconcile_plan, EgressPolicy, McpClientSession, McpConnectionDescriptor, RemoteToolError,
    StreamableHttpTransport,
};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use serde_json::Value;

use crate::jobs::KeyedMutex;
use crate::RocksDBStorage;

pub use check::run_check;

/// Workspace holding connection configuration.
pub(crate) const SYSTEM_WORKSPACE: &str = "raisin:system";
/// Workspace holding generated proxy functions.
pub(crate) const FUNCTIONS_WORKSPACE: &str = "functions";
/// Folder holding connection nodes.
pub(crate) const CONNECTIONS_ROOT: &str = "/mcp-connections";
/// NodeType of a connection.
pub(crate) const CONNECTION_NODE_TYPE: &str = "raisin:McpConnection";
/// Actor stamped on discovery writes, distinct from any user or sync actor.
pub(crate) const DISCOVERY_ACTOR: &str = "mcp-tool-discovery";

/// How long one connection's discovery lease is held.
const DISCOVERY_LEASE_TTL: Duration = Duration::from_secs(300);

/// What the discovery handler needs from the server binary.
///
/// Bundled so `init_job_system` gains one optional parameter rather than three,
/// and so "MCP discovery is configured" is a single `Option` rather than a set
/// of independently-defaultable knobs.
#[derive(Clone)]
pub struct McpDiscoveryDeps {
    /// Operator egress policy from `[mcp_client]`.
    pub egress: EgressPolicy,
    /// Cap on a remote response body.
    pub max_response_bytes: usize,
    /// The process-wide HTTP client.
    pub http: reqwest::Client,
}

/// Handler for the MCP discovery job types.
pub struct McpToolDiscoveryHandler {
    storage: Arc<RocksDBStorage>,
    /// Cluster-wide single-fire per connection. `None` = single-node only.
    lock_manager: Option<LockManagerHandle>,
    /// In-process serialization, so a manual refresh racing the scheduled one
    /// QUEUES rather than being dropped — the lease would reject it outright.
    keyed: Arc<KeyedMutex<String>>,
    /// Operator egress policy, re-checked before every dial.
    egress: EgressPolicy,
    /// Cap on a remote response body.
    max_response_bytes: usize,
    http: reqwest::Client,
    instance_id: String,
}

impl McpToolDiscoveryHandler {
    /// Create a discovery handler.
    pub fn new(
        storage: Arc<RocksDBStorage>,
        lock_manager: Option<LockManagerHandle>,
        deps: McpDiscoveryDeps,
    ) -> Self {
        Self {
            storage,
            lock_manager,
            keyed: Arc::new(KeyedMutex::new()),
            egress: deps.egress,
            max_response_bytes: deps.max_response_bytes,
            http: deps.http,
            instance_id: nanoid::nanoid!(8),
        }
    }

    /// Dispatch a discovery job.
    pub async fn handle(&self, job: &JobInfo, _context: &JobContext) -> Result<Option<Value>> {
        match &job.job_type {
            JobType::McpDiscoveryCheck { tenant_id } => {
                check::run_check(&self.storage, tenant_id.clone())
                    .await
                    .map(|n| Some(serde_json::json!({ "enqueued": n })))
            }
            JobType::McpToolDiscovery {
                connection_slug,
                tenant_id,
                repo_id,
            } => self
                .discover(tenant_id, repo_id, connection_slug)
                .await
                .map(Some),
            other => Err(raisin_error::Error::Validation(format!(
                "McpToolDiscoveryHandler received an unsupported job type: {other}"
            ))),
        }
    }

    /// Discover and reconcile one connection.
    async fn discover(&self, tenant: &str, repo: &str, slug: &str) -> Result<Value> {
        let branch = apply::default_branch(&self.storage, tenant, repo).await;

        // In-process first (queues), then the cluster lease (rejects). Taking
        // them in this order means two local callers do not both burn a lease
        // acquisition attempt against Redis.
        let _local = self
            .keyed
            .lock(format!("{tenant}\0{repo}\0{branch}\0{slug}"))
            .await;

        let lock_key =
            raisin_locks::scoped_key(tenant, repo, &branch, &format!("mcp-discovery:{slug}"));
        let lease = match &self.lock_manager {
            Some(lm) => {
                let owner = format!("{}:{}", self.instance_id, slug);
                match lm
                    .try_acquire(&lock_key, &owner, DISCOVERY_LEASE_TTL)
                    .await?
                {
                    Some(guard) => Some(guard.token),
                    None => {
                        tracing::debug!(slug, "connection is being discovered elsewhere; skipping");
                        return Ok(serde_json::json!({ "skipped": "locked" }));
                    }
                }
            }
            None => {
                if self.storage.config.replication_enabled {
                    tracing::warn!(
                        slug,
                        "locks subsystem disabled while replication is active: MCP tool \
                         discovery is NOT cluster-safe — every node will write the same proxy \
                         function nodes (configure [locks] backend=redis)"
                    );
                }
                None
            }
        };

        let outcome = self.discover_locked(tenant, repo, &branch, slug).await;

        if let (Some(lm), Some(token)) = (&self.lock_manager, lease) {
            let _ = lm.release(&lock_key, token).await;
        }
        outcome
    }

    /// Discovery proper, with both locks held.
    async fn discover_locked(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
        slug: &str,
    ) -> Result<Value> {
        let svc = apply::system_service(&self.storage, tenant, repo, branch, SYSTEM_WORKSPACE);
        let path = format!("{CONNECTIONS_ROOT}/{slug}");
        let Some(node) = svc.get_by_path(&path).await? else {
            // Deleted between enqueue and execution: not an error.
            return Ok(serde_json::json!({ "skipped": "connection not found" }));
        };

        let descriptor = match McpConnectionDescriptor::from_node(&node) {
            Ok(d) => d,
            Err(e) => {
                apply::record_health(&svc, node, "config_error", &e).await?;
                return Ok(serde_json::json!({ "skipped": "unparseable connection" }));
            }
        };
        if !descriptor.is_callable() {
            return Ok(serde_json::json!({ "skipped": "disabled" }));
        }

        // Re-checked here, not merely at save time: the stored URL may predate a
        // tightened allowlist, and a host can be re-pointed after it was saved.
        if let Err(e) = self.egress.guard(&descriptor.url).await {
            apply::record_health(&svc, node, e.code(), &e).await?;
            return Ok(serde_json::json!({ "skipped": "egress denied" }));
        }

        let credential = apply::decrypt_credential(&node, &descriptor);
        let headers = descriptor.auth_headers(credential.as_deref());

        let session = McpClientSession::new(
            StreamableHttpTransport::new(
                self.http.clone(),
                descriptor.url.clone(),
                descriptor.refresh_policy.call_timeout(),
            )
            .with_max_response_bytes(self.max_response_bytes),
        );

        let handshake = match session.handshake(&headers).await {
            Ok(h) => h,
            Err(e) => {
                apply::record_health(&svc, node, e.code(), &e).await?;
                return Ok(serde_json::json!({ "error": e.to_string() }));
            }
        };
        let remote = match session.list_tools(&headers).await {
            Ok(t) => t,
            Err(e) => {
                apply::record_health(&svc, node, e.code(), &e).await?;
                return Ok(serde_json::json!({ "error": e.to_string() }));
            }
        };

        let existing = apply::existing_proxies(&node);
        let actions = reconcile_plan(
            &descriptor.slug,
            &existing,
            &remote,
            &descriptor.tool_filter,
        );

        apply::apply_actions(
            &self.storage,
            tenant,
            repo,
            branch,
            &descriptor,
            &node,
            &actions,
            &handshake,
            remote.len(),
        )
        .await
    }
}

/// Classify a failure for the connection's `health.status`.
pub(crate) fn health_status_for(code: &str) -> &'static str {
    match code {
        "auth_expired" => "auth_expired",
        "config_error" => "config_error",
        "rate_limited" => "rate_limited",
        _ => "unreachable",
    }
}

/// Shorthand for the error detail recorded on a connection.
pub(crate) fn error_detail(err: &RemoteToolError) -> String {
    err.to_string()
}
