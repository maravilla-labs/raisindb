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

//! The listener supervisor loop.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use raisin_locks::{LeaseElection, LockManagerHandle};
use raisin_mcp_protocol::client::{
    installed_config, open_subscription, Backoff, McpClientSession, McpConnectionDescriptor,
    StreamEnd, StreamableHttpTransport, TOOLS_LIST_CHANGED,
};
use raisin_storage::jobs::{JobContext, JobId, JobType, McpDiscoverySource};

use crate::jobs::handlers::mcp_tool_discovery::{decrypt_credential, enumerate};
use crate::RocksDBStorage;

/// How long a listener's lease is held before it must be renewed.
///
/// `LeaseElection` renews at a third of this, so a node that dies is replaced
/// within roughly a minute rather than leaving the connection dark for an hour.
const LISTEN_LEASE_TTL: Duration = Duration::from_secs(90);

/// How often the supervisor re-reads which connections want a listener.
const RESCAN_INTERVAL: Duration = Duration::from_secs(60);

/// What the supervisor needs from the server binary.
pub struct ListenerConfig {
    pub storage: Arc<RocksDBStorage>,
    pub lock_manager: Option<LockManagerHandle>,
    /// Whether this deployment replicates. Decides whether missing locks are
    /// merely single-node or actively unsafe.
    pub replication_enabled: bool,
    /// Stable-ish identity for lease ownership, purely diagnostic.
    pub node_id: String,
    pub http: reqwest::Client,
}

/// Start the supervisor. Returns its join handle so shutdown can wait on it.
///
/// Refuses — rather than merely warning — when locks cannot coordinate across a
/// replicated cluster. Every node would win its own election and hold a
/// duplicate stream against somebody else's server, which is worse than having
/// no listeners at all. Discovery's interval refresh still covers freshness.
pub fn spawn(
    config: ListenerConfig,
    cancel: tokio_util::sync::CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    if config.lock_manager.is_none() && config.replication_enabled {
        tracing::warn!(
            "MCP tool-change listeners are DISABLED: replication is on but the locks subsystem \
             is not configured, so every node would hold a duplicate notification stream. \
             Configure [locks] with backend=redis to enable them."
        );
        return None;
    }
    Some(tokio::spawn(async move { run(config, cancel).await }))
}

/// Re-scan periodically, electing a holder for each connection that wants one.
async fn run(config: ListenerConfig, cancel: tokio_util::sync::CancellationToken) {
    let config = Arc::new(config);
    // Connections we already have a listener task for, so a rescan does not
    // spawn a second one for the same connection every minute.
    let mut running: HashSet<String> = HashSet::new();
    let mut ticker = tokio::time::interval(RESCAN_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::debug!("MCP listener supervisor shutting down");
                return;
            }
            _ = ticker.tick() => {}
        }

        let entries = match enumerate::all_connections(&config.storage, None).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(error = %e, "MCP listener: could not enumerate connections");
                continue;
            }
        };

        for entry in entries {
            if !wants_listener(&entry.descriptor) {
                continue;
            }
            let key = format!(
                "{}\0{}\0{}",
                entry.tenant, entry.repo, entry.descriptor.slug
            );
            if !running.insert(key.clone()) {
                continue;
            }

            let config = config.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                listen_for(config, entry, cancel).await;
            });
        }
    }
}

/// Whether this connection has opted in. The server-side half of the gate is
/// checked after the handshake, which is the first point it is knowable.
fn wants_listener(descriptor: &McpConnectionDescriptor) -> bool {
    descriptor.is_callable() && descriptor.refresh_policy.notifications
}

/// Stand for election on one connection, and hold its stream while elected.
async fn listen_for(
    config: Arc<ListenerConfig>,
    entry: enumerate::ConnectionEntry,
    cancel: tokio_util::sync::CancellationToken,
) {
    let slug = entry.descriptor.slug.clone();
    let entry = Arc::new(entry);

    let Some(lock_manager) = config.lock_manager.clone() else {
        // Single node, no locks: no election to run, just hold the stream.
        hold_stream(&config, &entry, &cancel).await;
        return;
    };

    let key = raisin_locks::scoped_key(
        &entry.tenant,
        &entry.repo,
        &entry.branch,
        &format!("mcp-listen:{slug}"),
    );
    let election = LeaseElection::new(
        lock_manager,
        key,
        format!("{}:{slug}", config.node_id),
        LISTEN_LEASE_TTL,
    );

    let result = election
        .run(cancel.clone().cancelled_owned(), || {
            let config = config.clone();
            let entry = entry.clone();
            let cancel = cancel.clone();
            // Dropped on demotion, which closes the stream — that is the whole
            // demotion mechanism, and why no signal is threaded into it.
            async move { hold_stream(&config, &entry, &cancel).await }
        })
        .await;

    if let Err(e) = result {
        tracing::warn!(%slug, error = %e, "MCP listener election failed");
    }
}

/// Keep a notification stream open, reconnecting until cancelled.
async fn hold_stream(
    config: &ListenerConfig,
    entry: &enumerate::ConnectionEntry,
    cancel: &tokio_util::sync::CancellationToken,
) {
    let slug = &entry.descriptor.slug;
    let mut backoff = Backoff::default();
    let mut last_event_id: Option<String> = None;

    loop {
        if cancel.is_cancelled() {
            return;
        }

        match connect_and_read(config, entry, last_event_id.as_deref(), cancel).await {
            Ok((end, resume)) => {
                last_event_id = resume;
                match end {
                    // A clean close is the server saying "shutting down", not a
                    // fault. Do not escalate the backoff for it.
                    StreamEnd::Graceful => {
                        backoff.reset();
                        tracing::debug!(%slug, "notification stream closed gracefully");
                    }
                    StreamEnd::Dropped => {
                        tracing::debug!(%slug, "notification stream dropped");
                    }
                }
            }
            Err(e) => {
                tracing::debug!(%slug, error = %e, "notification stream failed");
            }
        }

        let delay = backoff.next_delay();
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

/// One connection attempt: handshake, open the stream, read until it ends.
async fn connect_and_read(
    config: &ListenerConfig,
    entry: &enumerate::ConnectionEntry,
    last_event_id: Option<&str>,
    cancel: &tokio_util::sync::CancellationToken,
) -> raisin_mcp_protocol::client::Result<(StreamEnd, Option<String>)> {
    let descriptor = &entry.descriptor;
    let settings = installed_config();

    // Re-checked here, against the RESOLVED addresses, because a listener
    // reconnects for as long as the process lives and a host can be re-pointed
    // at any point in that window.
    settings.egress_policy().guard(&descriptor.url).await?;

    // The same resolver discovery uses, so a master-key mismatch is diagnosed
    // identically on both paths instead of twice, differently.
    let credential = decrypt_credential(&entry.node, descriptor);
    let headers = descriptor.auth_headers(credential.as_deref());

    let transport = StreamableHttpTransport::new(
        config.http.clone(),
        descriptor.url.clone(),
        descriptor.refresh_policy.call_timeout(),
    )
    .with_max_response_bytes(settings.max_response_bytes);
    let session = McpClientSession::new(transport);

    // The handshake decides the dialect AND whether there is anything to listen
    // for. It also sets the protocol version the stream request must carry.
    let handshake = session.handshake(&headers).await?;
    let modern = handshake.uses_listen_subscriptions();
    if !modern && !handshake.announces_tool_changes() {
        tracing::info!(
            slug = %descriptor.slug,
            "server does not advertise tools.listChanged; not holding a stream open"
        );
        // Not an error, and not worth retrying in a tight loop: report a
        // graceful end so the backoff stays calm.
        return Ok((StreamEnd::Graceful, None));
    }

    let mut stream =
        open_subscription(session.transport(), &headers, modern, last_event_id).await?;

    if !stream.tools_granted() {
        tracing::warn!(
            slug = %descriptor.slug,
            "server declined toolsListChanged on the subscription; \
             falling back to the refresh interval"
        );
        return Ok((StreamEnd::Graceful, None));
    }

    tracing::info!(slug = %descriptor.slug, modern, "MCP notification stream open");

    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => return Ok((StreamEnd::Graceful, None)),
            next = stream.next_notification() => next,
        };
        match next? {
            Ok((method, _params)) => {
                if method == TOOLS_LIST_CHANGED {
                    enqueue_discovery(config, entry).await;
                }
            }
            Err(end) => {
                return Ok((end, stream.last_event_id().map(str::to_string)));
            }
        }
    }
}

/// Schedule a re-list. Failures are logged; the interval refresh is the backstop.
async fn enqueue_discovery(config: &ListenerConfig, entry: &enumerate::ConnectionEntry) {
    let slug = &entry.descriptor.slug;
    tracing::info!(%slug, "remote server announced a tool change; re-listing");

    let job_id = JobId::new();
    let context = JobContext {
        tenant_id: entry.tenant.clone(),
        repo_id: entry.repo.clone(),
        branch: entry.branch.clone(),
        workspace_id: crate::jobs::handlers::mcp_tool_discovery::SYSTEM_WORKSPACE.to_string(),
        revision: raisin_hlc::HLC::now(),
        metadata: std::collections::HashMap::new(),
    };
    if let Err(e) = config.storage.job_data_store().put(&job_id, &context) {
        tracing::warn!(%slug, error = %e, "could not store discovery job context");
        return;
    }

    let dedup = format!("mcp-discovery:{}:{}:{slug}", entry.tenant, entry.repo);
    let job_type = JobType::McpToolDiscovery {
        connection_slug: slug.clone(),
        tenant_id: entry.tenant.clone(),
        repo_id: entry.repo.clone(),
        source: McpDiscoverySource::Notification,
    };
    if let Err(e) = config
        .storage
        .job_registry()
        .register_job_with_id_idempotent(job_id, job_type, entry.tenant.clone(), dedup, None)
        .await
    {
        tracing::warn!(%slug, error = %e, "could not enqueue discovery");
    }
}
