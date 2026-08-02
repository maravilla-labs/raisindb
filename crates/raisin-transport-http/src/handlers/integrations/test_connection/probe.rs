// SPDX-License-Identifier: BSL-1.1

//! The connection-test probe: `capabilities` then a bounded `list`.
//!
//! The adapter invocation itself lives in
//! [`crate::handlers::integrations::adapter_invoke`], shared with the browse
//! endpoint — this module only decides what to ask and how to read the answer.

#![cfg(feature = "storage-rocksdb")]

use raisin_models::nodes::Node;
use serde_json::{json, Value};

use super::support::{adapter_error_code, error_is_auth_expired, probe_from_list, sanitize};
use super::{Capabilities, ProbeOutcome, PROBE_LIMIT};
use crate::error::ApiError;
use crate::handlers::integrations::adapter_invoke::{invoke_adapter, AdapterResult};
use crate::state::AppState;

/// Run `capabilities` then a bounded `list`. Capabilities failures are
/// non-fatal (fallback) unless they signal expired auth; the `list` probe is
/// the real connectivity signal.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_probe(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    adapter_node: &Node,
    credential: &Option<Value>,
    mount: &Value,
    remote_root: &Option<String>,
    base_auth: &str,
) -> Result<ProbeOutcome, ApiError> {
    // 1. capabilities
    let caps = match invoke_adapter(
        state,
        tenant_id,
        repo,
        adapter_node,
        "capabilities",
        json!({}),
        credential,
        mount,
    )
    .await?
    {
        AdapterResult::Ok(value) => Capabilities::from_value(&value),
        AdapterResult::Failed(msg) => {
            if error_is_auth_expired(&msg) {
                return Ok(expired(Some(Capabilities::fallback())));
            }
            // Missing/soft-failing capabilities is tolerated — keep probing.
            Capabilities::fallback()
        }
    };

    // 2. bounded list
    let params = json!({ "folder_id": remote_root, "limit": PROBE_LIMIT });
    match invoke_adapter(
        state,
        tenant_id,
        repo,
        adapter_node,
        "list",
        params,
        credential,
        mount,
    )
    .await?
    {
        AdapterResult::Ok(value) => Ok(ProbeOutcome {
            ok: true,
            auth: base_auth.to_string(),
            capabilities: Some(caps),
            probe: Some(probe_from_list(&value)),
            error: None,
        }),
        AdapterResult::Failed(msg) => {
            if error_is_auth_expired(&msg) {
                return Ok(expired(Some(caps)));
            }
            Ok(ProbeOutcome {
                ok: false,
                auth: base_auth.to_string(),
                capabilities: Some(caps),
                probe: None,
                error: Some(super::TestError {
                    code: adapter_error_code(&msg).to_string(),
                    message: sanitize(&msg),
                }),
            })
        }
    }
}

/// Build the "expired credential" outcome.
pub(super) fn expired(capabilities: Option<Capabilities>) -> ProbeOutcome {
    ProbeOutcome {
        ok: false,
        auth: "expired".to_string(),
        capabilities,
        probe: None,
        error: Some(super::TestError {
            code: "auth_expired".to_string(),
            message: "the account credential is expired or was rejected".to_string(),
        }),
    }
}

/// Load the adapter function node for the connection test.
pub(super) async fn load_adapter_node(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    adapter_path: &str,
) -> Result<Option<Node>, ApiError> {
    crate::handlers::integrations::adapter_invoke::load_adapter_node(
        state,
        tenant_id,
        repo,
        adapter_path,
        "integration-test",
    )
    .await
}
