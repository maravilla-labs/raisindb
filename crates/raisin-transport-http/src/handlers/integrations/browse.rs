// SPDX-License-Identifier: BSL-1.1

//! `POST /api/integrations/{repo}/browse` — remote discovery for the mount
//! editor.
//!
//! Lists what a connector can see on the provider side — mail folders,
//! calendars, SharePoint sites, document libraries, drive folders — so an
//! operator *picks* a mount's remote root instead of pasting a provider id into
//! a text box. Backed by the optional `browse` adapter operation (§2.10 of
//! `docs/reference/virtual-node-adapters.md`); adapters that do not implement it
//! return `supported: false` and the console keeps its free-text input.
//!
//! Never called by the sync engine, and it changes nothing: this endpoint is a
//! read, invoked synchronously while a human waits.
//!
//! Security invariants, identical to the connection test it shares
//! [`adapter_invoke`] with: admin-gated, the credential handed to the adapter
//! has its `refresh_token` stripped, adapter error text is scrubbed before it
//! reaches the response, and a provider-side failure is a `200` with a
//! structured `error` rather than a 500.

#![cfg(feature = "storage-rocksdb")]

use std::time::Duration;

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::adapter_invoke::{invoke_adapter, load_adapter_node, AdapterResult};
use super::require_admin;
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Wall-clock ceiling so a hung adapter cannot hang the request. Shorter than
/// the connection test's 30s: a picker runs while someone watches a spinner, and
/// a directory listing that takes this long is not worth waiting for.
const BROWSE_TIMEOUT: Duration = Duration::from_secs(15);
/// Hard cap on items per page, whatever the caller or adapter asks for.
const MAX_LIMIT: u32 = 200;

#[derive(Debug, Deserialize)]
pub struct BrowseRequest {
    /// Path of the `raisin:Integration` node to browse.
    pub integration_path: String,
    /// Connected-account id to browse as.
    #[serde(default)]
    pub account_id: Option<String>,
    /// Adapter-defined container class (`folder`, `calendar`, `site`, `drive`,
    /// `driveItem`, `mailbox`, …). Passed through verbatim — deliberately not an
    /// enum here, so a connector can add a container type without a change in
    /// this crate.
    #[serde(default)]
    pub kind: Option<String>,
    /// List the children of this container. Absent = the top level.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Free-text filter, where the provider supports one.
    #[serde(default)]
    pub query: Option<String>,
    /// Opaque page cursor from a previous `next_cursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size hint; clamped to [`MAX_LIMIT`].
    #[serde(default)]
    pub limit: Option<u32>,
    /// Mount `sync_config` keys that select what to browse — ms-graph's
    /// `principal` (which mailbox's folders), `drive_scope`/`site_id` (which
    /// drive's folders). Without this the picker would always browse the
    /// connected account's own data while the mount syncs someone else's.
    #[serde(default)]
    pub sync_config: Option<Value>,
}

/// One browsable container. Mirrors `BrowseItem` in the adapter contract (§3.4).
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct BrowseItem {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub has_children: bool,
    #[serde(default)]
    pub hint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BrowseError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct BrowseResponse {
    pub ok: bool,
    /// False when this connector's adapter has no `browse` operation. The
    /// console uses it to fall back to manual entry rather than showing an
    /// error — an adapter without discovery is a normal, supported state.
    pub supported: bool,
    pub items: Vec<BrowseItem>,
    pub next_cursor: Option<String>,
    pub error: Option<BrowseError>,
}

impl BrowseResponse {
    fn empty(supported: bool, code: &str, message: &str) -> Self {
        Self {
            ok: false,
            supported,
            items: Vec::new(),
            next_cursor: None,
            error: Some(BrowseError {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }
}

/// Browse a connector's remote containers.
pub async fn browse(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<BrowseRequest>,
) -> Result<Json<BrowseResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let svc = super::config_service(&state, &tenant.tenant_id, &repo, "integration-browse");
    let node = svc
        .get_by_path(&req.integration_path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(req.integration_path.clone()))?;

    let provider_type = string_prop(&node, "provider_type").unwrap_or_default();
    let adapter_path = match string_prop(&node, "adapter_function") {
        Some(p) => p,
        None => {
            return Ok(Json(BrowseResponse::empty(
                false,
                "adapter_not_found",
                "this connector has no adapter function",
            )))
        }
    };

    // Browsing is inherently per-account: it is the account's own view of the
    // provider. Without one there is nothing to list.
    let account_id = match req.account_id.as_deref() {
        Some(id) => id,
        None => {
            return Ok(Json(BrowseResponse::empty(
                true,
                "missing_credential",
                "select a connected account to browse",
            )))
        }
    };
    let credential =
        match super::test_connection::resolve_credential(&state, &node, &provider_type, account_id)
        {
            Some(c) => Some(c),
            None => {
                return Ok(Json(BrowseResponse::empty(
                    true,
                    "missing_credential",
                    "no decryptable token for the requested account",
                )))
            }
        };

    let adapter_node = match load_adapter_node(
        &state,
        &tenant.tenant_id,
        &repo,
        &adapter_path,
        "integration-browse",
    )
    .await?
    {
        Some(n) => n,
        None => {
            return Ok(Json(BrowseResponse::empty(
                false,
                "adapter_not_found",
                &format!("adapter function `{adapter_path}` not found"),
            )))
        }
    };

    // Same merged mount snapshot the sync engine and the connection test build,
    // so the picker browses the mailbox/drive the mount will actually sync.
    let mount = super::test_connection::mount_snapshot(
        &req.parent_id,
        &node,
        Some(account_id),
        req.sync_config.as_ref(),
    );

    let params = serde_json::json!({
        "kind": req.kind,
        "parent_id": req.parent_id,
        "query": req.query,
        "cursor": req.cursor,
        "limit": req.limit.unwrap_or(MAX_LIMIT).min(MAX_LIMIT),
    });

    let outcome = tokio::time::timeout(
        BROWSE_TIMEOUT,
        invoke_adapter(
            &state,
            &tenant.tenant_id,
            &repo,
            &adapter_node,
            "browse",
            params,
            &credential,
            &mount,
        ),
    )
    .await;

    let result = match outcome {
        Ok(r) => r?,
        Err(_) => {
            return Ok(Json(BrowseResponse::empty(
                true,
                "timeout",
                &format!(
                    "adapter did not respond within {}s",
                    BROWSE_TIMEOUT.as_secs()
                ),
            )))
        }
    };

    match result {
        AdapterResult::Ok(value) => Ok(Json(parse_browse(&value))),
        AdapterResult::Failed(msg) => {
            let code = super::test_connection::adapter_error_code(&msg);
            // An adapter with no `browse` case throws "Unsupported operation".
            // That is not a failure to report — it is a connector without
            // discovery, and the console must fall back to manual entry.
            let unsupported = msg.contains("Unsupported operation");
            Ok(Json(BrowseResponse::empty(
                !unsupported,
                if unsupported { "unsupported" } else { code },
                &super::test_connection::sanitize(&msg),
            )))
        }
    }
}

/// Read an adapter's `browse` return value. A malformed item is skipped rather
/// than failing the whole listing — a picker showing four of five folders is
/// more useful than one showing an error.
fn parse_browse(value: &Value) -> BrowseResponse {
    let items = value
        .get("items")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|it| serde_json::from_value::<BrowseItem>(it.clone()).ok())
                .filter(|it| !it.id.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    BrowseResponse {
        ok: true,
        supported: true,
        items,
        next_cursor: value
            .get("next_cursor")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        error: None,
    }
}

/// Read a non-empty string property.
fn string_prop(node: &raisin_models::nodes::Node, key: &str) -> Option<String> {
    use raisin_models::nodes::properties::PropertyValue;
    match node.properties.get(key)? {
        PropertyValue::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_browse_reads_items_and_cursor() {
        let out = parse_browse(&json!({
            "items": [
                { "id": "F1", "name": "Inbox", "kind": "folder", "has_children": true, "hint": "17 items" },
                { "id": "F2", "name": "Archive", "kind": "folder" }
            ],
            "next_cursor": "https://graph.microsoft.com/v1.0/next"
        }));
        assert!(out.ok);
        assert!(out.supported);
        assert_eq!(out.items.len(), 2);
        assert_eq!(out.items[0].id, "F1");
        assert!(out.items[0].has_children);
        assert_eq!(out.items[0].hint.as_deref(), Some("17 items"));
        // Absent fields default rather than dropping the item.
        assert!(!out.items[1].has_children);
        assert_eq!(
            out.next_cursor.as_deref(),
            Some("https://graph.microsoft.com/v1.0/next")
        );
    }

    #[test]
    fn malformed_items_are_skipped_not_fatal() {
        let out = parse_browse(&json!({
            "items": [
                { "id": "ok-1", "name": "Fine" },
                { "name": "no id at all" },
                { "id": "", "name": "empty id" },
                "not even an object"
            ]
        }));
        assert!(out.ok);
        assert_eq!(out.items.len(), 1);
        assert_eq!(out.items[0].id, "ok-1");
    }

    #[test]
    fn an_empty_listing_is_a_success_not_an_error() {
        let out = parse_browse(&json!({ "items": [], "next_cursor": null }));
        assert!(out.ok);
        assert!(out.items.is_empty());
        assert!(out.next_cursor.is_none());
        assert!(out.error.is_none());
    }

    #[test]
    fn an_empty_cursor_string_is_treated_as_no_more_pages() {
        let out = parse_browse(&json!({ "items": [{ "id": "a" }], "next_cursor": "" }));
        assert!(out.next_cursor.is_none());
    }
}
