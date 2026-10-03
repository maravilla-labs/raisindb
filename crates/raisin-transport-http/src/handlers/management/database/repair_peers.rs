// SPDX-License-Identifier: BSL-1.1

//! Forwarding an index-repair request to the other nodes of the cluster.
//!
//! Derived indexes are rebuilt locally on every node and a repair job is
//! process-local, so one admin call has to reach every node. The peers are the
//! HTTP base URLs in `RocksDBConfig::replication_peers`; a forwarded request
//! carries `local_only=true` so the peer acts on itself and never fans out
//! again.

use std::sync::OnceLock;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;

/// A peer this node forwards repairs to.
#[derive(Debug, Clone)]
pub(super) struct RepairPeer {
    pub node_id: String,
    pub url: String,
}

/// Why a peer did not answer with a result.
#[derive(Debug)]
pub(super) enum PeerFailure {
    /// No connection, or no answer in time: the node may simply be down.
    Unreachable(String),
    /// The node answered, but not with a result (refused, failed, garbled).
    Error(String),
}

/// This node's id, exactly as `run_repair` stamps it into the state key.
pub(super) fn local_node_id(storage: &raisin_rocksdb::RocksDBStorage) -> String {
    storage
        .config()
        .cluster_node_id
        .clone()
        .unwrap_or_else(|| "local".to_string())
}

/// The enabled peers with an HTTP address.
pub(super) fn peers(storage: &raisin_rocksdb::RocksDBStorage) -> Vec<RepairPeer> {
    storage
        .config()
        .replication_peers
        .iter()
        .filter(|p| p.enabled && !p.url.is_empty())
        .map(|p| RepairPeer {
            node_id: p.peer_id.clone(),
            url: p.url.trim_end_matches('/').to_string(),
        })
        .collect()
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default()
    })
}

/// Send one request to a peer and decode its answer.
///
/// `path_and_query` must already carry `local_only=true`. The peer's
/// management routes take administrator credentials only; nodes of one cluster
/// share the operator's superadmin bearer, the same one the replication pull
/// client presents.
pub(super) async fn call<T: DeserializeOwned>(
    peer: &RepairPeer,
    method: reqwest::Method,
    path_and_query: &str,
    tenant_id: &str,
    body: Option<&Value>,
) -> Result<T, PeerFailure> {
    let url = format!("{}{}", peer.url, path_and_query);
    let mut request = client()
        .request(method, &url)
        .header("x-tenant-id", tenant_id);
    if let Some(token) = std::env::var("RAISIN_SUPERADMIN_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
    {
        request = request.bearer_auth(token);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.map_err(|e| {
        if e.is_connect() || e.is_timeout() {
            PeerFailure::Unreachable(e.to_string())
        } else {
            PeerFailure::Error(e.to_string())
        }
    })?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(PeerFailure::Error(format!("{status}: {text}")));
    }
    response
        .json::<T>()
        .await
        .map_err(|e| PeerFailure::Error(format!("undecodable answer: {e}")))
}

/// The path a peer is asked, always marked `local_only`.
pub(super) fn peer_path(repo: &str, repair: &str, suffix: &str, branch: Option<&str>) -> String {
    let mut path = format!(
        "/api/management/{}/repairs/{}{suffix}?local_only=true",
        urlencoding::encode(repo),
        urlencoding::encode(repair)
    );
    if let Some(branch) = branch {
        path.push_str(&format!("&branch={}", urlencoding::encode(branch)));
    }
    path
}

/// `(node_id, status, error)` for a peer that gave no result.
pub(super) fn failure_parts(
    peer: &RepairPeer,
    failure: PeerFailure,
) -> (String, String, Option<String>) {
    match failure {
        PeerFailure::Unreachable(e) => (peer.node_id.clone(), "unreachable".into(), Some(e)),
        PeerFailure::Error(e) => (peer.node_id.clone(), "error".into(), Some(e)),
    }
}
